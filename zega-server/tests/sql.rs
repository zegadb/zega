//! APS 54 phase 2 over HTTP: `POST /sql` and `POST /transaction`.

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use zega::{Storage, Zega};
use zega_server::{server, AppState};

const TOKEN: &str = "test-token";
const P: &str = "schema { type P { name: String } } unique { P { name } }";

struct Server {
    url: String,
    _data: tempfile::TempDir,
}

async fn start(storage: Storage) -> Server {
    let data = tempfile::tempdir().unwrap();
    let zega = Zega::open(data.path().to_str().unwrap()).storage(storage).build().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let state = AppState::new(zega, Some(TOKEN));
    tokio::spawn(async move { server::serve(listener, state).await.unwrap() });
    Server { url, _data: data }
}

async fn call(server: &Server, path: &str, body: Value) -> (StatusCode, Value) {
    let response = Client::new()
        .post(format!("{}{path}", server.url))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap();
    (response.status(), response.json().await.unwrap())
}

#[tokio::test]
async fn sql_round_trips_every_value_kind() {
    let server = start(Storage::Sqlite).await;
    let (status, _) = call(&server, "/sql", json!({"sql": "CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n)"})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &server,
        "/sql",
        json!({"sql": "INSERT INTO t VALUES (?1, ?2, ?3, ?4, ?5), (?6, 0.5, 'x', NULL, NULL)",
               "params": [{"int": "9007199254740993"}, 1.5, "ü ✈", {"base64": "AAEC"}, null, 7]}),
    )
    .await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({"ok": true, "result": {"columns": [], "rows": [], "changes": 2}})));
    let (_, body) = call(&server, "/sql", json!({"sql": "SELECT i, r, s, b, n FROM t ORDER BY r DESC"})).await;
    assert_eq!(
        body["result"],
        json!({
            "columns": ["i", "r", "s", "b", "n"],
            "rows": [[{"int": "9007199254740993"}, 1.5, "ü ✈", {"base64": "AAEC"}, null], [7, 0.5, "x", null, null]],
            "changes": 0
        })
    );
}

#[tokio::test]
async fn a_transaction_keeps_all_steps_or_none_and_names_the_failing_step() {
    let server = start(Storage::Sqlite).await;
    call(&server, "/sql", json!({"sql": "CREATE TABLE orders (id INTEGER PRIMARY KEY)"})).await;
    let (status, body) = call(
        &server,
        "/transaction",
        json!({"steps": [
            {"sql": "INSERT INTO orders (id) VALUES (?1)", "params": [1]},
            {"schema": P, "zql": "mutation { P(name: \"a\") { name } }"}
        ]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["results"][0]["changes"], 1);
    assert_eq!(body["result"]["results"][1], json!({"name": "a"}));
    // Step 1 conflicts with order 1: step 0's write is taken back too.
    let (status, body) = call(
        &server,
        "/transaction",
        json!({"steps": [
            {"schema": P, "zql": "mutation { P(name: \"b\") { name } }"},
            {"sql": "INSERT INTO orders (id) VALUES (1)"}
        ]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!((body["code"].clone(), body["step"].clone()), (json!("constraint"), json!(1)));
    let (_, body) = call(&server, "/zql", json!({"schema": P, "query": "query { P { name } }"})).await;
    assert_eq!(body["result"], json!([{"name": "a"}]));
    let (_, body) = call(&server, "/sql", json!({"sql": "SELECT count(*) AS n FROM orders"})).await;
    assert_eq!(body["result"]["rows"], json!([[1]]));
}

#[tokio::test]
async fn refusals_carry_their_code() {
    let server = start(Storage::Sqlite).await;
    for (sql, status, code) in [
        ("DELETE FROM zega_log", StatusCode::BAD_REQUEST, "reserved_table"),
        ("SELECT 1; SELECT 2", StatusCode::BAD_REQUEST, "sql_error"),
        ("SELECT * FROM missing", StatusCode::BAD_REQUEST, "sql_error"),
    ] {
        let (got, body) = call(&server, "/sql", json!({ "sql": sql })).await;
        assert_eq!((got, body["code"].clone()), (status, json!(code)), "{sql}: {body}");
    }
    let (got, body) = call(&server, "/sql", json!({"sql": "SELECT ?1", "params": [{"nope": 1}]})).await;
    assert_eq!((got, body["code"].clone()), (StatusCode::BAD_REQUEST, json!("sql_error")), "{body}");
    let file = start(Storage::File).await;
    let (got, body) = call(&file, "/sql", json!({"sql": "SELECT 1"})).await;
    assert_eq!((got, body["code"].clone()), (StatusCode::UNPROCESSABLE_ENTITY, json!("sql_unavailable")));
    let unauthorized = Client::new().post(format!("{}/sql", server.url)).json(&json!({"sql": "SELECT 1"})).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_delivery_is_applied_once_on_either_storage() {
    for storage in [Storage::File, Storage::Sqlite] {
        let server = start(storage).await;
        let doc = format!("{P}\nmutation {{ P(name: \"a\") {{ name }} }}");
        let request = json!({"source_id": "pg:1", "txid": "42", "document": doc});
        let (status, first) = call(&server, "/deliveries", request.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["result"]["result"], json!({"name": "a"}));
        assert_eq!(first["result"]["payload_sha256"].as_str().unwrap().len(), 64);
        // A lost response, then the identical retry: the same receipt.
        let (status, again) = call(&server, "/deliveries", request).await;
        assert_eq!((status, &again), (StatusCode::OK, &first));
        let (status, body) = call(&server, "/deliveries", json!({"source_id": "pg:1", "txid": "42", "document": format!("{P}\nmutation {{ P(name: \"b\") }}")})).await;
        assert_eq!((status, body["code"].clone()), (StatusCode::CONFLICT, json!("delivery_conflict")));
        let (status, body) = call(&server, "/deliveries", json!({"source_id": "pg 1", "txid": "1", "document": P})).await;
        assert_eq!((status, body["code"].clone()), (StatusCode::UNPROCESSABLE_ENTITY, json!("invalid_identity")));
        let (_, body) = call(&server, "/zql", json!({"schema": P, "query": "query { P { name } }"})).await;
        assert_eq!(body["result"], json!([{"name": "a"}]), "{storage:?}");
    }
}
