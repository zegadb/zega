//! APS 54 phase 2: `POST /sql` and `POST /transaction`, the HTTP face of
//! `Zega::sql` and `Zega::transaction`. They need a database started with
//! `--storage sqlite`; otherwise they answer 422 `sql_unavailable`.
//! APS 52: `POST /deliveries`, `Zega::deliver`, on either storage.
//!
//! JSON values: `null`, numbers, strings; an integer beyond ±2^53 is
//! `{"int": "…"}` (JavaScript numbers lose it otherwise), a non-finite real
//! `{"real": "Infinity"}`, a blob `{"base64": "…"}`. `true`/`false` are 1/0.

use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use zega::receipts::Payload;
use zega::{SqlResult, SqlValue, ZegaError};

use crate::handlers::{authorized, error, refusal};
use crate::AppState;

/// ±2^53: past it, a JSON number is no longer exact in JavaScript.
const EXACT: i64 = 1 << 53;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqlRequest {
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionRequest {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Step {
    Sql {
        sql: String,
        #[serde(default)]
        params: Vec<Value>,
    },
    Zql {
        /// The stored schema when absent.
        #[serde(default)]
        schema: String,
        zql: String,
    },
}

fn to_param(value: &Value) -> Result<SqlValue, String> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64().ok_or_else(|| format!("{n} is out of range"))?),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Object(o) if o.len() == 1 => match o.iter().next().expect("one field") {
            (key, Value::String(s)) if key == "int" => SqlValue::Integer(s.parse().map_err(|_| format!("{s:?} is not an integer"))?),
            (key, Value::String(s)) if key == "real" => SqlValue::Real(s.parse().map_err(|_| format!("{s:?} is not a number"))?),
            (key, Value::String(s)) if key == "base64" => SqlValue::Blob(
                base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| format!("base64: {e}"))?,
            ),
            _ => return Err(r#"an object parameter is {"int": "…"}, {"real": "…"} or {"base64": "…"}"#.into()),
        },
        other => return Err(format!("{other} is not a SQL value")),
    })
}

fn from_value(value: &SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(i) if (-EXACT..=EXACT).contains(i) => json!(i),
        SqlValue::Integer(i) => json!({"int": i.to_string()}),
        SqlValue::Real(f) if f.is_finite() => json!(f),
        SqlValue::Real(f) => json!({"real": f.to_string()}),
        SqlValue::Text(s) => json!(s),
        SqlValue::Blob(b) => json!({"base64": base64::engine::general_purpose::STANDARD.encode(b)}),
    }
}

fn result_json(result: &SqlResult) -> Value {
    json!({
        "columns": result.columns,
        "rows": result.rows.iter().map(|row| row.iter().map(from_value).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "changes": result.changes,
    })
}

fn params(values: &[Value]) -> Result<Vec<SqlValue>, ZegaError> {
    values
        .iter()
        .map(to_param)
        .collect::<Result<_, _>>()
        .map_err(|message| ZegaError::Sql { code: "sql_error", message })
}

/// Run `action` on the database's worker thread; a refusal names `step`.
async fn run(
    state: AppState,
    action: impl FnOnce(&zega::Zega, &mut Option<usize>) -> Result<Value, ZegaError> + Send + 'static,
) -> Response {
    let ran = tokio::task::spawn_blocking(move || {
        let mut step = None;
        let result = state
            .zega
            .lock()
            .map_err(|_| ZegaError::Execution("database lock poisoned".into()))
            .and_then(|db| action(&db, &mut step));
        (result, step)
    })
    .await;
    match ran {
        Ok((Ok(result), _)) => Json(json!({"ok": true, "result": result})).into_response(),
        Ok((Err(cause), step)) => refusal(cause, StatusCode::BAD_REQUEST, step),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed"),
    }
}

pub async fn sql(State(state): State<AppState>, headers: HeaderMap, request: Result<Json<SqlRequest>, JsonRejection>) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    run(state, move |db, _| {
        let params = params(&request.params)?;
        db.sql(&request.sql, &params).map(|result| result_json(&result))
    })
    .await
}

pub async fn transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<TransactionRequest>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    run(state, move |db, failed| {
        db.transaction(|tx| {
            let mut results = Vec::with_capacity(request.steps.len());
            for (index, step) in request.steps.iter().enumerate() {
                *failed = Some(index);
                results.push(match step {
                    Step::Sql { sql, params: values } => tx.sql(sql, &params(values)?).map(|r| result_json(&r))?,
                    Step::Zql { schema, zql } => tx.zql(schema, zql)?,
                });
            }
            // Every step ran; a failure from here on is the commit's.
            *failed = None;
            Ok(json!({"results": results}))
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRequest {
    source_id: String,
    txid: String,
    /// Deleted before the schema is stored: `{"all": true}`, or `labels` and
    /// `relationships` to empty.
    #[serde(default)]
    flush: zega::receipts::Flush,
    /// Stored as the graph's schema before the document runs (as `PUT /schema`).
    #[serde(default)]
    schema: Option<String>,
    /// A `.zql` document: schema, then its blocks.
    document: String,
}

/// `POST /deliveries`: 200 with the receipt (also for an identical retry),
/// 409 `delivery_conflict`, 422 `invalid_identity`, 400 for a document the
/// engine refuses, and 503 `outcome_unknown` when the write may or may not
/// have reached the disk: retry the identical request.
pub async fn deliver(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<DeliveryRequest>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    let ran = tokio::task::spawn_blocking(move || {
        state
            .zega
            .lock()
            .map_err(|_| ZegaError::Execution("database lock poisoned".into()))
            .and_then(|db| {
                let payload = Payload { flush: request.flush, schema: request.schema.as_deref(), document: &request.document };
                db.deliver(&request.source_id, &request.txid, &payload)
            })
    })
    .await;
    match ran {
        Ok(Ok(receipt)) => Json(json!({"ok": true, "result": {
            "source_id": receipt.source_id,
            "txid": receipt.txid,
            "payload_sha256": receipt.payload_sha256.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "result": serde_json::from_str::<Value>(&receipt.result).unwrap_or(Value::Null),
        }}))
        .into_response(),
        Ok(Err(cause @ ZegaError::Wal(_))) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ok": false, "error": cause.to_string(), "code": "outcome_unknown"})),
        )
            .into_response(),
        Ok(Err(cause)) => refusal(cause, StatusCode::BAD_REQUEST, None),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed"),
    }
}
