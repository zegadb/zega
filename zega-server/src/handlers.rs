use crate::{auth, AppState};
use axum::{
    body::{Body, Bytes},
    extract::{rejection::JsonRejection, Path, State},
    http::{header, header::HeaderName, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use std::io;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use zega::{Zega, ZegaError};

pub(crate) fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"ok": false, "error": message.into()}))).into_response()
}

pub(crate) fn authorized(headers: &HeaderMap, state: &AppState) -> bool {
    state
        .token_hash
        .as_ref()
        .is_none_or(|hash| auth::authorized(headers, hash))
}

pub async fn health(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    Json(json!({"ok": true})).into_response()
}

/// `GET /stats`: the graph's node and relationship counts, for Zega Cloud's
/// dashboard (a graph's size against its plan). Cheap: two map lengths.
pub async fn stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    execute(state, |db| db.counts().map(|c| json!({"nodes": c.nodes, "relationships": c.relationships}))).await
}

async fn execute(
    state: AppState,
    action: impl FnOnce(&Zega) -> Result<Value, ZegaError> + Send + 'static,
) -> Response {
    execute_refusing(state, StatusCode::BAD_REQUEST, action).await
}

/// [`execute`], with `refused` as the status of a statement the engine
/// refuses (a parse, check or execution error, or the time limit). `POST`
/// answers 400 for all of them; `QUERY` answers 422 (RFC 10008: the content
/// is consistent with its media type, but cannot be processed as a query).
async fn execute_refusing(
    state: AppState,
    refused: StatusCode,
    action: impl FnOnce(&Zega) -> Result<Value, ZegaError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        let db = state
            .zega
            .lock()
            .map_err(|_| ZegaError::Execution("database lock poisoned".into()))?;
        action(&db)
    })
    .await
    {
        Ok(Ok(result)) => Json(json!({"ok": true, "result": result})).into_response(),
        // Its own code, so a client can tell "this query is too slow" from
        // "this query is wrong". A 4xx, like any other refused query: a 5xx
        // or 408 invites an automatic retry of the same slow query.
        Ok(Err(cause @ ZegaError::QueryTimeLimit { .. })) => (
            refused,
            Json(json!({"ok": false, "error": cause.to_string(), "code": "query_time_limit"})),
        )
            .into_response(),
        // Only the read-only entry points (QUERY) can say this: a statement
        // that writes, refused before it ran. Always 422, whatever `refused` is.
        Ok(Err(ZegaError::NotARead(what))) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "error": format!("QUERY /zql runs reads only, and this is {what}. Send it with POST /zql."),
                "code": "not_a_read",
            })),
        )
            .into_response(),
        Ok(Err(cause)) => error(refused, cause.to_string()),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZqlRequest {
    #[serde(default)]
    schema: String,
    query: String,
    #[serde(default)]
    document: bool,
    /// Optional raw text from a browser file picker; native requests omit this
    /// and use the library's own file/HTTP transport.
    sources: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaDiffRequest {
    #[serde(default)]
    old: String,
    #[serde(default)]
    new: String,
}

pub async fn schema_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<SchemaDiffRequest>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    execute(state, move |db| {
        let report = db.schema_diff(&request.old, &request.new)?;
        serde_json::to_value(report).map_err(|error| ZegaError::Execution(error.to_string()))
    })
    .await
}

pub async fn zql(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<ZqlRequest>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    execute(state, move |db| match (request.document, request.sources) {
        (false, None) => db.run_lang(&request.schema, &request.query),
        (false, Some(sources)) => {
            db.run_lang_with_sources(&request.schema, &request.query, &sources)
        }
        (true, None) => db.apply_zql(&request.query),
        (true, Some(sources)) => db.apply_zql_with_sources(&request.query, &sources),
    })
    .await
}

/// The raw-text media type of `QUERY /zql`: the body is one `.zql` document
/// (a `schema { }` block, then `query { }` blocks), read-only. There is no
/// schema anywhere else to use: this server keeps none, so a bare query with
/// no `schema { }` would fail with "schema has no types".
pub const ZQL_MEDIA_TYPE: &str = "application/zql";

/// Every media type `QUERY /zql` takes as its content. The one list: the 415
/// check and the `Accept-Query` header both read it, so the server advertises
/// exactly what it accepts.
pub const QUERY_MEDIA_TYPES: [&str; 2] = ["application/json", ZQL_MEDIA_TYPE];

/// `Accept-Query` (RFC 10008 section 3): a Structured Fields list of media
/// ranges, here `application/json, application/zql`.
fn accept_query() -> String {
    QUERY_MEDIA_TYPES.join(", ")
}

/// The methods `/zql` answers, as `Allow` lists them.
const ZQL_ALLOW: &str = "OPTIONS, POST, QUERY";

fn with_allow(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::ALLOW, header::HeaderValue::from_static(ZQL_ALLOW));
    if let Ok(value) = header::HeaderValue::from_str(&accept_query()) {
        headers.insert(HeaderName::from_static("accept-query"), value);
    }
    response
}

/// Every method of `/zql` but `POST` (axum's method router calls this for
/// the rest; axum 0.7 has no filter for a method it does not know, so
/// `QUERY` is told apart here by name). `QUERY` runs a read; `OPTIONS` says
/// what the route takes; anything else is the same 405 it always was, now
/// listing `QUERY` in `Allow`.
pub async fn zql_other(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    use axum::extract::FromRequest;
    match request.method().as_str() {
        "QUERY" => {
            let headers = request.headers().clone();
            if !authorized(&headers, &state) {
                return error(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            // The same 16 MB DefaultBodyLimit as POST's `Json`: `Bytes` applies it.
            let body = match Bytes::from_request(request, &state).await {
                Ok(body) => body,
                Err(rejection) => return rejection.into_response(),
            };
            let mut response = query(state, &headers, &body).await;
            // A QUERY answer is cacheable by default (RFC 10008 section 2.8). This one is
            // authenticated and the graph changes under it: no cache may keep or share it.
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
            with_allow(response)
        }
        "OPTIONS" => {
            if !authorized(request.headers(), &state) {
                return error(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            with_allow(StatusCode::NO_CONTENT.into_response())
        }
        _ => with_allow(error(
            StatusCode::METHOD_NOT_ALLOWED,
            format!("{} is not supported on /zql; it takes {ZQL_ALLOW}", request.method()),
        )),
    }
}

/// `QUERY /zql`, after the bearer. Status codes, per RFC 10008 section 2:
/// 400 no `Content-Type`, or content that is not what its type says (JSON
/// that is not a request, bytes that are not UTF-8); 415 a type this does not
/// take (or a `charset` other than UTF-8); 406 an `Accept` that excludes
/// `application/json`, the only type it answers in; 422 anything the engine
/// refuses to run as a read: a mutation (nothing changes), ZQL that does not
/// parse or check, the time limit.
async fn query(state: AppState, headers: &HeaderMap, body: &[u8]) -> Response {
    let media = match content_type(headers) {
        Ok(media) => media,
        Err((status, message)) => return error(status, message),
    };
    if !acceptable(headers, "application/json") {
        return error(
            StatusCode::NOT_ACCEPTABLE,
            "QUERY /zql answers in application/json",
        );
    }
    let request = if media == ZQL_MEDIA_TYPE {
        match std::str::from_utf8(body) {
            Ok(text) => ZqlRequest {
                schema: String::new(),
                query: text.to_string(),
                document: true,
                sources: None,
            },
            Err(_) => return error(StatusCode::BAD_REQUEST, "the content is not valid UTF-8"),
        }
    } else {
        match serde_json::from_slice::<ZqlRequest>(body) {
            Ok(request) => request,
            Err(cause) => return error(StatusCode::BAD_REQUEST, format!("the content is not a ZQL request: {cause}")),
        }
    };
    // `sources` feeds only `mutation csv|json` loads, which a read never has:
    // a QUERY that sent them runs exactly as without.
    execute_refusing(state, StatusCode::UNPROCESSABLE_ENTITY, move |db| {
        if request.document {
            db.apply_zql_read(&request.query)
        } else {
            db.run_lang_read(&request.schema, &request.query)
        }
    })
    .await
}

/// The media type of a QUERY's content, lowercased, or the refusal: 400 when
/// the header is missing or not a media type, 415 when the type (or its
/// `charset`) is not one `QUERY_MEDIA_TYPES` takes.
fn content_type(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let Some(raw) = headers.get(header::CONTENT_TYPE) else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("QUERY needs a Content-Type that says what the content is: {}", accept_query()),
        ));
    };
    let Some((media, parameters)) = raw
        .to_str()
        .ok()
        .map(|raw| raw.split_once(';').unwrap_or((raw, "")))
        .filter(|(media, _)| media.trim().split_once('/').is_some_and(|(kind, sub)| !kind.is_empty() && !sub.is_empty()))
    else {
        return Err((StatusCode::BAD_REQUEST, "the Content-Type is not a media type".into()));
    };
    let media = media.trim().to_ascii_lowercase();
    let unsupported = || (StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("QUERY /zql takes {}", accept_query()));
    if !QUERY_MEDIA_TYPES.contains(&media.as_str()) {
        return Err(unsupported());
    }
    let charset = parameters.split(';').find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim().eq_ignore_ascii_case("charset").then(|| value.trim().trim_matches('"').to_ascii_lowercase())
    });
    if charset.is_some_and(|charset| charset != "utf-8") {
        return Err(unsupported());
    }
    Ok(media)
}

/// Whether `Accept` allows `media`: absent, empty, or a range that matches it
/// with a q-value above zero (RFC 9110 section 12.5.1).
fn acceptable(headers: &HeaderMap, media: &str) -> bool {
    let ranges = accept_ranges(headers);
    ranges.is_empty() || quality(&ranges, media) > 0.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorViewRequest {
    schema: String,
    result: Value,
    kind: String,
    selected: Option<u64>,
    k: usize,
    threshold: f64,
}

pub async fn vector_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<VectorViewRequest>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    let kind = match request.kind.as_str() {
        "vector2d" => zega::ViewKind::Vector2d,
        "vector3d" => zega::ViewKind::Vector3d,
        _ => return error(StatusCode::BAD_REQUEST, "expected vector2d or vector3d"),
    };
    execute(state, move |db| {
        db.vector_view(
            &request.schema,
            &request.result,
            kind,
            request.selected,
            request.k,
            request.threshold,
        )
    })
    .await
}

/// Chunks in flight between a file and the socket: what bounds a
/// transfer's memory.
const CHUNKS_IN_FLIGHT: usize = 4;
const CHUNK: usize = 64 * 1024;

type Chunk = Result<Bytes, io::Error>;

/// Deletes a staging file when the transfer that owns it ends, however it
/// ends.
struct Staged(std::path::PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Hands each write to the response body, blocking while the client is
/// behind. A client that went away fails the write.
struct BodyWriter(tokio::sync::mpsc::Sender<Chunk>);

impl io::Write for BodyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(buf)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the client went away"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How much a client wants each representation of `/graph`, from its
/// `Accept` header (RFC 9110 §12.5.1): `(json, graph)` q-values. The most
/// specific range that matches a type sets its q-value.
fn accepted(headers: &HeaderMap) -> (f32, f32) {
    if !headers.contains_key(header::ACCEPT) {
        return (0.0, 1.0);
    }
    let ranges = accept_ranges(headers);
    (quality(&ranges, "application/json"), quality(&ranges, zega::graph_file::MEDIA_TYPE))
}

/// The media ranges of an `Accept` header with their q-values, lowercased.
fn accept_ranges(headers: &HeaderMap) -> Vec<(String, f32)> {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|value| value.to_str().ok()) else {
        return Vec::new();
    };
    let mut ranges = Vec::new();
    for item in accept.split(',') {
        let mut parts = item.split(';');
        let range = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        if range.is_empty() {
            continue;
        }
        let q = parts
            .filter_map(|param| param.trim().strip_prefix("q=").or_else(|| param.trim().strip_prefix("Q=")))
            .find_map(|q| q.trim().parse::<f32>().ok())
            .map_or(1.0, |q| q.clamp(0.0, 1.0));
        ranges.push((range, q));
    }
    ranges
}

/// The q-value `ranges` give `media`: the most specific matching range sets it.
fn quality(ranges: &[(String, f32)], media: &str) -> f32 {
    let (kind, _) = media.split_once('/').unwrap_or((media, ""));
    let exact = ranges.iter().find(|(range, _)| range == media);
    let family = ranges.iter().find(|(range, _)| range.strip_suffix("/*") == Some(kind));
    let any = ranges.iter().find(|(range, _)| range == "*/*");
    exact.or(family).or(any).map_or(0.0, |(_, q)| *q)
}

fn with_vary(mut response: Response) -> Response {
    response
        .headers_mut()
        .append(header::VARY, header::HeaderValue::from_static("accept"));
    response
}

/// `GET /graph`: the whole graph as a `.graph` file (docs/graph-format.md).
/// A client that prefers `application/json` gets the JSON view the explorer
/// draws instead.
///
/// On a disk database the file is written to a staging file under the gate,
/// which is released before a byte goes to the client: a slow download
/// holds disk space, never the database. In memory there is no staging
/// directory, so the export streams under the gate.
pub async fn graph(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let (json, graph) = accepted(&headers);
    if json <= 0.0 && graph <= 0.0 {
        return with_vary(error(
            StatusCode::NOT_ACCEPTABLE,
            format!("GET /graph serves {} or application/json", zega::graph_file::MEDIA_TYPE),
        ));
    }
    if json > graph {
        return with_vary(execute(state, Zega::graph_json).await);
    }
    let (send, mut receive) = tokio::sync::mpsc::channel::<Chunk>(CHUNKS_IN_FLIGHT);
    let Some(slot) = transfer_slot(&state) else {
        return with_vary(busy());
    };
    let gate = state.zega.clone();
    let spooled = tokio::task::spawn_blocking(move || -> Result<Option<(Staged, u64)>, ZegaError> {
        let db = gate
            .lock()
            .map_err(|_| ZegaError::Execution("database lock poisoned".into()))?;
        let Some((file, path)) = db.staging_file("export")? else {
            return Ok(None);
        };
        let staged = Staged(path);
        let mut out = io::BufWriter::new(file);
        let summary = db.export(&mut out)?;
        drop(db);
        out.into_inner().map_err(|error| error.into_error())?;
        Ok(Some((staged, summary.bytes)))
    })
    .await;
    let (staged, length) = match spooled {
        Ok(Ok(Some(spooled))) => spooled,
        Ok(Ok(None)) => {
            // In memory there is nowhere to stage: stream under the gate.
            tokio::task::spawn_blocking(move || {
                let result = match state.zega.lock() {
                    Ok(db) => db
                        .export(&mut BodyWriter(send.clone()))
                        .map_err(|error| io::Error::other(error.to_string())),
                    Err(_) => Err(io::Error::other("database lock poisoned")),
                };
                // Headers are already sent: failing the body is how the
                // client learns the file is incomplete.
                if let Err(error) = result {
                    let _ = send.blocking_send(Err(error));
                }
            });
            let body = futures_util::stream::poll_fn(move |context| receive.poll_recv(context));
            return with_vary(graph_response(Body::from_stream(body), None));
        }
        Ok(Err(cause)) => return with_vary(error(StatusCode::INTERNAL_SERVER_ERROR, cause.to_string())),
        Err(_) => return with_vary(error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed")),
    };
    let idle = state.transfer_idle_timeout;
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let _slot = slot;
        let result: io::Result<()> = async {
            let mut file = tokio::fs::File::open(&staged.0).await?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    return Ok(());
                }
                let chunk = Ok(Bytes::copy_from_slice(&buf[..n]));
                match tokio::time::timeout(idle, send.send(chunk)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => return Ok(()), // the client went away
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "the download stalled"))
                    }
                }
            }
        }
        .await;
        if let Err(error) = result {
            let _ = send.try_send(Err(error));
        }
        // The file is closed by now; `staged` deletes it.
        drop(staged);
    });
    let body = futures_util::stream::poll_fn(move |context| receive.poll_recv(context));
    with_vary(graph_response(Body::from_stream(body), Some(length)))
}

/// A transfer slot, if one is free.
fn transfer_slot(state: &AppState) -> Option<tokio::sync::OwnedSemaphorePermit> {
    state.transfers.clone().try_acquire_owned().ok()
}

/// The answer when no transfer slot is free.
fn busy() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "too many .graph transfers are in progress; try again shortly",
    )
}

fn graph_response(body: Body, length: Option<u64>) -> Response {
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, zega::graph_file::MEDIA_TYPE)
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"graph.graph\"");
    if let Some(length) = length {
        response = response.header(header::CONTENT_LENGTH, length);
    }
    response
        .body(body)
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "response failed"))
}
fn too_large(state: &AppState) -> Response {
    error(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "the upload is larger than this server's {} byte limit (zega start --max-import-bytes)",
            state.max_import_bytes
        ),
    )
}

/// `PUT /graph`: replace the whole graph with the `.graph` file in the body.
/// Nothing changes unless all of it is a valid file.
///
/// The body is spooled to a staging file without the gate, capped at
/// `max_import_bytes` (413) and given up on after `transfer_idle_timeout`
/// without data (408). Only then is the gate taken, to import the file from
/// disk. In memory the body is collected under the same limits instead.
pub async fn import_graph(State(state): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    use tokio::io::AsyncWriteExt;
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > state.max_import_bytes) {
        return too_large(&state);
    }
    let Some(_slot) = transfer_slot(&state) else {
        return busy();
    };
    let gate = state.zega.clone();
    let staging = match tokio::task::spawn_blocking(move || match gate.lock() {
        Ok(db) => db.staging_file("upload"),
        Err(_) => Err(ZegaError::Execution("database lock poisoned".into())),
    })
    .await
    {
        Ok(Ok(staging)) => staging,
        Ok(Err(cause)) => return error(StatusCode::INTERNAL_SERVER_ERROR, cause.to_string()),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed"),
    };
    // `staged` is bound first so it is dropped last: on an early return the
    // file handle is closed before the guard deletes the file (Windows
    // refuses to delete an open file).
    let (staged, mut file, mut memory) = match staging {
        Some((file, path)) => (Some(Staged(path)), Some(tokio::fs::File::from_std(file)), None),
        None => (None, None, Some(Vec::new())),
    };
    let mut body = body.into_data_stream();
    let mut received: u64 = 0;
    loop {
        let chunk = match tokio::time::timeout(state.transfer_idle_timeout, body.next()).await {
            Err(_) => {
                return error(
                    StatusCode::REQUEST_TIMEOUT,
                    format!(
                        "the upload stalled: no data for {} s",
                        state.transfer_idle_timeout.as_secs_f64()
                    ),
                )
            }
            Ok(None) => break,
            Ok(Some(Err(cause))) => return error(StatusCode::BAD_REQUEST, cause.to_string()),
            Ok(Some(Ok(chunk))) => chunk,
        };
        received += chunk.len() as u64;
        if received > state.max_import_bytes {
            return too_large(&state);
        }
        let written = match (&mut file, &mut memory) {
            (Some(file), _) => file.write_all(&chunk).await,
            (None, Some(memory)) => {
                memory.extend_from_slice(&chunk);
                Ok(())
            }
            (None, None) => Ok(()),
        };
        if let Err(cause) = written {
            return error(StatusCode::INTERNAL_SERVER_ERROR, cause.to_string());
        }
    }
    if let Some(mut file) = file {
        if let Err(cause) = file.flush().await {
            return error(StatusCode::INTERNAL_SERVER_ERROR, cause.to_string());
        }
    }
    let import = tokio::task::spawn_blocking(move || {
        let db = state
            .zega
            .lock()
            .map_err(|_| ZegaError::Execution("database lock poisoned".into()))?;
        match (staged, memory) {
            (Some(staged), _) => db.import_staged(&staged.0),
            (None, Some(memory)) => db.import(&memory[..]),
            (None, None) => Err(ZegaError::Execution("nothing was uploaded".into())),
        }
    });
    match import.await {
        Ok(Ok(summary)) => Json(json!({"ok": true, "result": summary})).into_response(),
        Ok(Err(cause)) => error(StatusCode::BAD_REQUEST, cause.to_string()),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "database worker failed"),
    }
}

/// `DELETE /graph`: replace the graph with an empty one, durably and as one
/// WAL entry. What an import carried (schema text, declarations, metadata)
/// goes too, so a later export cannot claim the old licence or source.
pub async fn clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    execute(state, |db| {
        db.clear()?;
        Ok(Value::Null)
    })
    .await
}

pub async fn delete_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    execute(state, move |db| {
        db.delete_node(id)?;
        Ok(Value::Null)
    })
    .await
}

pub async fn delete_relationship(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    execute(state, move |db| {
        db.delete_relationship(id)?;
        Ok(Value::Null)
    })
    .await
}

#[derive(Deserialize)]
pub struct Connection {
    schema: String,
    from: u64,
    field: String,
    to: u64,
}

pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<Connection>, JsonRejection>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    execute(state, move |db| {
        db.connect_schema(&request.schema, request.from, &request.field, request.to)?;
        Ok(Value::Null)
    })
    .await
}
