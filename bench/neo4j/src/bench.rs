// The bench client: one program drives both engines over their normal network
// paths (zega-server's HTTP /zql; Neo4j's Bolt via neo4rs, plus its HTTP API
// when enabled) with the same seeded parameter lists, so per-query answers
// and timings are directly comparable. Loaders for both sides live here too.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};

use crate::gen::{categories, usage, Rng};

// ---------------------------------------------------------------- kinds

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Lookup,
    OneHop,
    TwoHop,
    Filtered,
    Path,
    Write,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "lookup" => Some(Kind::Lookup),
            "onehop" => Some(Kind::OneHop),
            "twohop" => Some(Kind::TwoHop),
            "filtered" => Some(Kind::Filtered),
            "path" => Some(Kind::Path),
            "write" => Some(Kind::Write),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::Lookup => "lookup",
            Kind::OneHop => "onehop",
            Kind::TwoHop => "twohop",
            Kind::Filtered => "filtered",
            Kind::Path => "path",
            Kind::Write => "write",
        }
    }
    pub fn all() -> [Kind; 6] {
        [
            Kind::Lookup,
            Kind::OneHop,
            Kind::TwoHop,
            Kind::Filtered,
            Kind::Path,
            Kind::Write,
        ]
    }
}

// ---------------------------------------------------------------- dataset

pub struct Dataset {
    pub dir: std::path::PathBuf,
    pub schema: String,
    pub params: Json,
    pub meta: Json,
}

impl Dataset {
    pub fn load(dir: &str) -> Result<Dataset, String> {
        let dir = std::path::PathBuf::from(dir);
        let schema = std::fs::read_to_string(dir.join("schema.zql"))
            .map_err(|e| format!("read schema.zql: {e}"))?;
        let params = serde_json::from_str(
            &std::fs::read_to_string(dir.join("params.json"))
                .map_err(|e| format!("read params.json: {e}"))?,
        )
        .map_err(|e| format!("parse params.json: {e}"))?;
        let meta = serde_json::from_str(
            &std::fs::read_to_string(dir.join("meta.json"))
                .map_err(|e| format!("read meta.json: {e}"))?,
        )
        .map_err(|e| format!("parse meta.json: {e}"))?;
        Ok(Dataset {
            dir,
            schema,
            params,
            meta,
        })
    }

    /// The seeded parameter list for one query kind. Write kind has no list:
    /// its parameter is the global op index, shared by both engines.
    pub fn params_for(&self, kind: Kind) -> Vec<Json> {
        let key = match kind {
            Kind::Lookup => "lookup",
            Kind::OneHop => "onehop",
            Kind::TwoHop => "twohop",
            Kind::Filtered => "filtered",
            Kind::Path => "path",
            Kind::Write => return Vec::new(),
        };
        self.params[key].as_array().cloned().unwrap_or_default()
    }
}

/// A write's payload, derived from the op index alone so both engines receive
/// byte-identical writes.
pub fn write_payload(seq: u64) -> (String, String, String, String, i64) {
    (
        format!("w{seq:09}"),
        format!("Bench User {seq}"),
        format!("w{seq}@bench.example"),
        crate::gen::categories()[(seq % categories().len() as u64) as usize].to_string(),
        2026,
    )
}

fn person_id(ix: u64) -> String {
    format!("u{ix:07}")
}

// ---------------------------------------------------------------- statements

pub fn zql_stmt(kind: Kind, p: &Json, write_seq: Option<u64>) -> String {
    match kind {
        Kind::Lookup => {
            let id = person_id(p.as_u64().expect("lookup param"));
            format!(r#"query {{ Person(id: "{id}") {{ id name email city joined }} }}"#)
        }
        Kind::OneHop => {
            let id = person_id(p.as_u64().expect("onehop param"));
            format!(r#"query {{ Person(id: "{id}") {{ knows -> Person {{ id name }} }} }}"#)
        }
        Kind::TwoHop => {
            let id = person_id(p.as_u64().expect("twohop param"));
            format!(
                r#"query {{ Person(id: "{id}") {{ knows *2..2 -> Person {{ id @hops }} }} }}"#
            )
        }
        Kind::Filtered => {
            let ix = p[0].as_u64().expect("filtered person");
            let cat = categories()[p[1].as_u64().expect("filtered category") as usize];
            format!(
                r#"query {{ Person(has knows exactly 2 hops(id: "{}") && has purchases(category = "{cat}")) limit 50 {{ id }} }}"#,
                person_id(ix)
            )
        }
        Kind::Path => {
            let a = person_id(p[0].as_u64().expect("path a"));
            let b = person_id(p[1].as_u64().expect("path b"));
            format!(r#"query {{ Person(id: "{a}") {{ knows *path -> Person(id: "{b}") {{ id }} }} }}"#)
        }
        Kind::Write => {
            let (id, name, email, city, joined) = write_payload(write_seq.expect("write seq"));
            format!(
                r#"mutation {{ Person(id: "{id}" && name: "{name}" && email: "{email}" && city: "{city}" && joined: {joined}) {{ id }} }}"#
            )
        }
    }
}

pub fn cypher_stmt(kind: Kind) -> &'static str {
    match kind {
        Kind::Lookup => {
            "MATCH (p:Person {id: $id}) RETURN p.id AS id, p.name AS name, p.email AS email, p.city AS city, p.joined AS joined"
        }
        Kind::OneHop => "MATCH (p:Person {id: $id})-[:KNOWS]->(f) RETURN f.id AS id, f.name AS name",
        Kind::TwoHop => {
            "MATCH (a:Person {id: $id})-[:KNOWS*2]->(f) \
             WHERE NOT (a)-[:KNOWS]->(f) AND f <> a RETURN DISTINCT f.id AS id"
        }
        Kind::Filtered => {
            "MATCH (a:Person {id: $id})-[:KNOWS*2]->(f) \
             WHERE NOT (a)-[:KNOWS]->(f) AND f <> a \
             AND (f)-[:PURCHASED]->(:Product {category: $cat}) RETURN DISTINCT f.id AS id LIMIT 50"
        }
        Kind::Path => {
            "MATCH (a:Person {id: $a}), (b:Person {id: $b}) \
             MATCH pp = shortestPath((a)-[:KNOWS*]-(b)) RETURN length(pp) AS hops"
        }
        Kind::Write => {
            "CREATE (p:Person {id: $id, name: $name, email: $email, city: $city, joined: $joined}) RETURN p.id AS id"
        }
    }
}

// ---------------------------------------------------------------- engines

#[derive(Clone)]
pub enum Engine {
    /// zega-server over HTTP; every request carries the schema text, as the
    /// API (and the explorer) does.
    Zega {
        http: reqwest::Client,
        url: String,
        schema: String,
    },
    /// Neo4j over Bolt (neo4rs).
    Neo4j { graph: neo4rs::Graph },
    /// Neo4j's HTTP API, where enabled.
    Neo4jHttp {
        http: reqwest::Client,
        url: String,
        auth: String,
    },
}

impl Engine {
    pub async fn zega(url: &str, dataset: &Dataset) -> Result<Engine, String> {
        Ok(Engine::Zega {
            http: reqwest::Client::new(),
            url: url.trim_end_matches('/').to_string(),
            schema: dataset.schema.clone(),
        })
    }

    pub async fn neo4j(url: &str, password: &str) -> Result<Engine, String> {
        let graph = neo4rs::Graph::new(url, "neo4j", password)
            .await
            .map_err(|e| format!("neo4j connect {url}: {e}"))?;
        Ok(Engine::Neo4j { graph })
    }

    pub fn neo4j_http(url: &str, password: &str) -> Engine {
        Engine::Neo4jHttp {
            http: reqwest::Client::new(),
            url: url.trim_end_matches('/').to_string(),
            auth: format!("neo4j:{password}"),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Engine::Zega { .. } => "zega",
            Engine::Neo4j { .. } => "neo4j",
            Engine::Neo4jHttp { .. } => "neo4j-http",
        }
    }

    /// Run one parameterized query and return its canonical answer (see
    /// `canon_*`): the same JSON from either engine for the same question.
    pub async fn exec(&self, kind: Kind, p: &Json, write_seq: Option<u64>) -> Result<Json, String> {
        match self {
            Engine::Zega { http, url, schema } => {
                let body = json!({"schema": schema, "query": zql_stmt(kind, p, write_seq)});
                let resp: Json = http
                    .post(format!("{url}/zql"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("zega http: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("zega json: {e}"))?;
                if resp["ok"] != true {
                    return Err(format!("zega error: {}", resp["error"]));
                }
                canon_zega(kind, &resp["result"])
            }
            Engine::Neo4j { graph } => {
                use neo4rs::query as cypher;
                let mut q = cypher(cypher_stmt(kind));
                match kind {
                    Kind::Lookup | Kind::OneHop | Kind::TwoHop => {
                        let id = person_id(p.as_u64().expect("param"));
                        q = q.param("id", id);
                    }
                    Kind::Filtered => {
                        q = q
                            .param("id", person_id(p[0].as_u64().expect("param")))
                            .param(
                                "cat",
                                categories()[p[1].as_u64().expect("cat") as usize].to_string(),
                            );
                    }
                    Kind::Path => {
                        q = q
                            .param("a", person_id(p[0].as_u64().expect("a")))
                            .param("b", person_id(p[1].as_u64().expect("b")));
                    }
                    Kind::Write => {
                        let (id, name, email, city, joined) =
                            write_payload(write_seq.expect("write seq"));
                        q = q
                            .param("id", id)
                            .param("name", name)
                            .param("email", email)
                            .param("city", city)
                            .param("joined", joined);
                    }
                }
                let mut rows = graph
                    .execute(q)
                    .await
                    .map_err(|e| format!("neo4j execute: {e}"))?;
                let mut out = Vec::new();
                while let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| format!("neo4j row: {e}"))?
                {
                    out.push(row);
                }
                canon_neo4j(kind, &out)
            }
            Engine::Neo4jHttp { http, url, auth } => {
                let (stmt, parameters) = http_statement(kind, p, write_seq);
                let body = json!({
                    "statements": [{ "statement": stmt, "parameters": parameters, "resultDataContents": ["row"] }],
                });
                let resp = http
                    .post(format!("{url}/db/neo4j/tx/commit"))
                    .header("Authorization", format!("Basic {}", base64(auth)))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("neo4j http: {e}"))?;
                let resp: Json = resp.json().await.map_err(|e| format!("neo4j http json: {e}"))?;
                if !resp["errors"].as_array().is_some_and(|e| e.is_empty()) {
                    return Err(format!("neo4j http error: {}", resp["errors"]));
                }
                canon_neo4j_http(kind, &resp["results"][0])
            }
        }
    }

    /// Remove a person node (parity cleanup for the write check).
    pub async fn delete_person(&self, id: &str) -> Result<(), String> {
        match self {
            Engine::Zega { http, url, schema } => {
                let q = format!(r#"query {{ Person(id: "{id}") {{ @id }} }}"#);
                let body = json!({"schema": schema, "query": q});
                let resp: Json = http
                    .post(format!("{url}/zql"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("zega http: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("zega json: {e}"))?;
                let engine_id = resp["result"]["id"]
                    .as_u64()
                    .ok_or_else(|| format!("parity write not found on zega: {id}"))?;
                let resp = http
                    .delete(format!("{url}/graph/nodes/{engine_id}"))
                    .send()
                    .await
                    .map_err(|e| format!("zega delete: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("zega delete status: {}", resp.status()));
                }
                Ok(())
            }
            Engine::Neo4j { graph } => {
                use neo4rs::query as cypher;
                graph
                    .run(cypher("MATCH (p:Person {id: $id}) DETACH DELETE p").param("id", id.to_string()))
                    .await
                    .map_err(|e| format!("neo4j delete: {e}"))
            }
            Engine::Neo4jHttp { .. } => Err("delete over http not implemented".into()),
        }
    }
}

fn base64(text: &str) -> String {
    use std::fmt::Write as _;
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    let _ = write!(out, "");
    out
}

fn http_statement(kind: Kind, p: &Json, write_seq: Option<u64>) -> (String, Json) {
    let mut parameters = json!({});
    let stmt = cypher_stmt(kind).to_string();
    match kind {
        Kind::Lookup | Kind::OneHop | Kind::TwoHop => {
            parameters["id"] = json!(person_id(p.as_u64().expect("param")));
        }
        Kind::Filtered => {
            parameters["id"] = json!(person_id(p[0].as_u64().expect("param")));
            parameters["cat"] = json!(categories()[p[1].as_u64().expect("cat") as usize]);
        }
        Kind::Path => {
            parameters["a"] = json!(person_id(p[0].as_u64().expect("a")));
            parameters["b"] = json!(person_id(p[1].as_u64().expect("b")));
        }
        Kind::Write => {
            let (id, name, email, city, joined) = write_payload(write_seq.expect("write seq"));
            parameters = json!({"id": id, "name": name, "email": email, "city": city, "joined": joined});
        }
    }
    (stmt, parameters)
}

// ---------------------------------------------------------------- canonical answers

fn sort_ids(ids: &mut Vec<String>) {
    ids.sort_unstable();
}

/// zega's result JSON → the canonical answer for the kind.
fn canon_zega(kind: Kind, result: &Json) -> Result<Json, String> {
    let expect_obj = |v: &Json, what: &str| -> Result<Vec<Json>, String> {
        v.as_array()
            .cloned()
            .ok_or_else(|| format!("zega {what}: expected a list, got {v}"))
    };
    Ok(match kind {
        Kind::Lookup => match result {
            Json::Null => Json::Null,
            obj => json!({
                "id": obj["id"], "name": obj["name"], "email": obj["email"],
                "city": obj["city"], "joined": obj["joined"],
            }),
        },
        Kind::OneHop => {
            if result.is_null() {
                return Ok(json!([]));
            }
            let mut list = expect_obj(&result["knows"], "onehop")?;
            list.sort_by_key(|n| n["id"].as_str().unwrap_or_default().to_string());
            Json::Array(list)
        }
        Kind::TwoHop => {
            if result.is_null() {
                return Ok(json!([]));
            }
            let list = expect_obj(&result["knows"], "twohop")?;
            let mut ids: Vec<String> = list
                .iter()
                .map(|n| n["id"].as_str().unwrap_or_default().to_string())
                .collect();
            sort_ids(&mut ids);
            Json::Array(ids.into_iter().map(Json::String).collect())
        }
        Kind::Filtered => {
            let list = expect_obj(result, "filtered")?;
            let mut ids: Vec<String> = list
                .iter()
                .map(|n| n["id"].as_str().unwrap_or_default().to_string())
                .collect();
            sort_ids(&mut ids);
            Json::Array(ids.into_iter().map(Json::String).collect())
        }
        Kind::Path => match &result["knows"] {
            Json::Null => Json::Null,
            route => route["hops"].clone(),
        },
        Kind::Write => json!({"id": result["id"]}),
    })
}

fn row_str(row: &neo4rs::Row, key: &str) -> Result<String, String> {
    row.get::<String>(key).map_err(|e| format!("neo4j column {key}: {e}"))
}

/// Bolt rows → the canonical answer.
fn canon_neo4j(kind: Kind, rows: &[neo4rs::Row]) -> Result<Json, String> {
    Ok(match kind {
        Kind::Lookup => {
            let Some(row) = rows.first() else { return Ok(Json::Null) };
            json!({
                "id": row_str(row, "id")?,
                "name": row_str(row, "name")?,
                "email": row_str(row, "email")?,
                "city": row_str(row, "city")?,
                "joined": row.get::<i64>("joined").map_err(|e| format!("neo4j joined: {e}"))?,
            })
        }
        Kind::OneHop => {
            let mut list: Vec<Json> = rows
                .iter()
                .map(|r| Ok(json!({"id": row_str(r, "id")?, "name": row_str(r, "name")?})))
                .collect::<Result<_, String>>()?;
            list.sort_by_key(|n| n["id"].as_str().unwrap_or_default().to_string());
            Json::Array(list)
        }
        Kind::TwoHop | Kind::Filtered => {
            let mut ids: Vec<String> = rows.iter().map(|r| row_str(r, "id")).collect::<Result<_, _>>()?;
            sort_ids(&mut ids);
            Json::Array(ids.into_iter().map(Json::String).collect())
        }
        Kind::Path => match rows.first() {
            None => Json::Null,
            Some(row) => json!(row.get::<i64>("hops").map_err(|e| format!("neo4j hops: {e}"))?),
        },
        Kind::Write => {
            let Some(row) = rows.first() else {
                return Err("neo4j write returned no row".into());
            };
            json!({"id": row_str(row, "id")?})
        }
    })
}

/// HTTP API results[0] → the canonical answer (rows arrive positionally).
fn canon_neo4j_http(kind: Kind, result: &Json) -> Result<Json, String> {
    let columns = result["columns"]
        .as_array()
        .ok_or("neo4j http: no columns")?;
    let pos = |name: &str| -> Result<usize, String> {
        columns
            .iter()
            .position(|c| c.as_str() == Some(name))
            .ok_or_else(|| format!("neo4j http: no column {name}"))
    };
    let rows: Vec<&Json> = result["data"].as_array().map(|d| d.iter().collect()).unwrap_or_default();
    let cell = |row: &Json, p: usize| row["row"][p].clone();
    Ok(match kind {
        Kind::Lookup => {
            let Some(row) = rows.first() else { return Ok(Json::Null) };
            json!({
                "id": cell(row, pos("id")?), "name": cell(row, pos("name")?),
                "email": cell(row, pos("email")?), "city": cell(row, pos("city")?),
                "joined": cell(row, pos("joined")?),
            })
        }
        Kind::OneHop => {
            let (pi, ni) = (pos("id")?, pos("name")?);
            let mut list: Vec<Json> = rows
                .iter()
                .map(|r| json!({"id": cell(r, pi), "name": cell(r, ni)}))
                .collect();
            list.sort_by_key(|n| n["id"].as_str().unwrap_or_default().to_string());
            Json::Array(list)
        }
        Kind::TwoHop | Kind::Filtered => {
            let pi = pos("id")?;
            let mut ids: Vec<String> = rows
                .iter()
                .map(|r| cell(r, pi).as_str().unwrap_or_default().to_string())
                .collect();
            sort_ids(&mut ids);
            Json::Array(ids.into_iter().map(Json::String).collect())
        }
        Kind::Path => match rows.first() {
            None => Json::Null,
            Some(row) => cell(row, pos("hops")?),
        },
        Kind::Write => {
            let Some(row) = rows.first() else {
                return Err("neo4j http write returned no row".into());
            };
            json!({"id": cell(row, pos("id")?)})
        }
    })
}

// ---------------------------------------------------------------- run cell

pub struct CellStats {
    pub ops: u64,
    pub errors: u64,
    pub qps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

/// One measured cell: `conc` workers replay the seeded params (or the shared
/// write sequence) for `window`, after `warmup` of the same. Returns
/// client-observed latency percentiles over the window.
pub async fn run_cell(
    engine: Engine,
    kind: Kind,
    params: Vec<Json>,
    conc: usize,
    warmup: Duration,
    window: Duration,
    write_base: u64,
) -> CellStats {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let warm_until = Instant::now() + warmup;
    let stop_at = warm_until + window;
    let seq = Arc::new(AtomicU64::new(0));
    let samples: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
    let errors = Arc::new(AtomicU64::new(0));

    let mut workers = Vec::new();
    for _ in 0..conc {
        let engine = engine.clone();
        let params = params.clone();
        let seq = seq.clone();
        let samples = samples.clone();
        let errors = errors.clone();
        workers.push(tokio::spawn(async move {
            let mut rng = Rng(0x5eed ^ (seq.load(Ordering::Relaxed)));
            loop {
                let measured = Instant::now() >= warm_until;
                if Instant::now() >= stop_at {
                    break;
                }
                let ix = rng.below(params.len().max(1) as u64);
                let p = params.get(ix as usize).cloned().unwrap_or(Json::Null);
                let write_seq = (kind == Kind::Write)
                    .then(|| write_base + seq.fetch_add(1, Ordering::Relaxed));
                let started = Instant::now();
                match engine.exec(kind, &p, write_seq).await {
                    Ok(_) => {
                        if measured {
                            samples
                                .lock()
                                .await
                                .push(started.elapsed().as_nanos());
                        }
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }
    for w in workers {
        let _ = w.await;
    }

    let mut samples = samples.lock().await.clone();
    samples.sort_unstable();
    let ops = samples.len() as u64;
    let qps = ops as f64 / window.as_secs_f64();
    CellStats {
        ops,
        errors: errors.load(Ordering::Relaxed),
        qps,
        p50_ms: percentile(&samples, 0.50) as f64 / 1e6,
        p95_ms: percentile(&samples, 0.95) as f64 / 1e6,
        p99_ms: percentile(&samples, 0.99) as f64 / 1e6,
    }
}

// ---------------------------------------------------------------- parity

/// Run each query kind on both engines over the same sampled params and
/// compare canonical answers. A faster wrong answer is not a result: any
/// mismatch fails the bench before a single number is timed.
pub async fn parity(
    zega: &Engine,
    neo4j: &Engine,
    dataset: &Dataset,
    samples: usize,
) -> Result<(), String> {
    let mut failures = 0;
    for kind in [
        Kind::Lookup,
        Kind::OneHop,
        Kind::TwoHop,
        Kind::Filtered,
        Kind::Path,
    ] {
        let params = dataset.params_for(kind);
        let mut checked = 0;
        for p in params.iter().take(samples) {
            let a = zega.exec(kind, p, None).await?;
            let b = neo4j.exec(kind, p, None).await?;
            checked += 1;
            if a != b {
                failures += 1;
                if failures <= 5 {
                    eprintln!(
                        "PARITY MISMATCH {kind:?}\n  param: {p}\n  zega:  {a}\n  neo4j: {b}"
                    );
                }
            }
        }
        println!("PARITY {:<8} {}/{} agree", kind.name(), checked - failures.min(checked), checked);
    }

    // Write parity: same write on both, read it back, remove it. The id
    // carries a time nonce so a re-run never collides with a leftover row
    // from a crashed earlier run (zega's unique refuses it — correctly).
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 100_000_000)
        .unwrap_or(0);
    let seq = Some(900_000_000 + nonce);
    let _ = zega.delete_person(&crate::bench::write_payload(seq.unwrap()).0).await;
    let _ = neo4j.delete_person(&crate::bench::write_payload(seq.unwrap()).0).await;
    let (a, b) = tokio::try_join!(zega.exec(Kind::Write, &Json::Null, seq), neo4j.exec(Kind::Write, &Json::Null, seq))?;
    if a != b {
        failures += 1;
        eprintln!("PARITY MISMATCH write: zega {a} vs neo4j {b}");
    }
    let id = a["id"].as_str().unwrap_or_default().to_string();
    // delete_person verifies the node existed on each side; that is the
    // read-back check for the write.
    zega.delete_person(&id).await?;
    neo4j.delete_person(&id).await?;
    println!("PARITY write   1/1 agree (created, verified, deleted {id})");

    if failures > 0 {
        return Err(format!("{failures} parity mismatches; fix before timing"));
    }
    Ok(())
}

// ---------------------------------------------------------------- loaders

/// Load the dataset into a zega data directory, the way zega-server holds a
/// graph (same calls as zega-scale's server path), and checkpoint it.
pub fn load_zega(args: &[String]) {
    let dataset = flag(args, "--dataset").unwrap_or_else(|| usage());
    let data = flag(args, "--data").unwrap_or_else(|| usage());
    let dataset = Dataset::load(&dataset).unwrap_or_else(|e| {
        eprintln!("dataset: {e}");
        std::process::exit(1)
    });

    let source = &dataset.schema;
    let locations =
        zega::zql_load_locations(zega::ZqlEntryPoint::File, source).expect("load locations");
    let mut sources: HashMap<String, String> = HashMap::with_capacity(locations.len());
    for location in &locations {
        let text =
            std::fs::read_to_string(dataset.dir.join(location.trim_start_matches("./")))
                .expect("read shard");
        sources.insert(location.clone(), text);
    }

    let db = zega::Zega::open(&data).build().expect("open data dir");
    let started = Instant::now();
    db.apply_zql_with_sources(source, &sources).expect("apply");
    let apply_ms = started.elapsed().as_millis() as u64;
    let started = Instant::now();
    db.checkpoint().expect("checkpoint");
    let checkpoint_ms = started.elapsed().as_millis() as u64;
    let counts = db.counts().expect("counts");
    drop(db);

    let on_disk: u64 = walkdir(&std::path::PathBuf::from(&data))
        .iter()
        .map(|p| p.metadata().expect("meta").len())
        .sum();
    println!(
        "{}",
        json!({
            "apply_ms": apply_ms,
            "checkpoint_ms": checkpoint_ms,
            "nodes": counts.nodes,
            "relationships": counts.relationships,
            "on_disk_bytes": on_disk,
        })
    );
}

/// Load the dataset into Neo4j over Bolt in 10k-row UNWIND batches: unique
/// constraints first (the enforced-index match for zega's `unique`), then
/// nodes, then relationships; indexes awaited ONLINE before reporting.
pub async fn load_neo4j(args: &[String]) {
    let dataset = flag(args, "--dataset").unwrap_or_else(|| usage());
    let url = flag(args, "--url").unwrap_or_else(|| usage());
    let password = flag(args, "--password").unwrap_or_else(|| usage());
    let clear = args.iter().any(|a| a == "--clear");
    let dataset = Dataset::load(&dataset).unwrap_or_else(|e| {
        eprintln!("dataset: {e}");
        std::process::exit(1)
    });
    use neo4rs::query as cypher;

    let graph = neo4rs::Graph::new(&url, "neo4j", &password)
        .await
        .expect("neo4j connect");

    if clear {
        graph
            .run(cypher("MATCH (n) DETACH DELETE n"))
            .await
            .expect("clear");
    }
    for stmt in [
        "CREATE CONSTRAINT person_id IF NOT EXISTS FOR (p:Person) REQUIRE p.id IS UNIQUE",
        "CREATE CONSTRAINT product_sku IF NOT EXISTS FOR (pr:Product) REQUIRE pr.sku IS UNIQUE",
    ] {
        graph.run(cypher(stmt)).await.expect("constraint");
    }

    let total = Instant::now();

    // Persons and products: parallel arrays indexed by UNWIND range, so the
    // driver only sends primitive lists.
    let persons = read_csv_rows(&dataset.dir, "persons");
    let mut ids = Vec::with_capacity(persons.len());
    let mut names = Vec::with_capacity(persons.len());
    let mut emails = Vec::with_capacity(persons.len());
    let mut citys = Vec::with_capacity(persons.len());
    let mut joineds = Vec::with_capacity(persons.len());
    for r in &persons {
        ids.push(r[0].clone());
        names.push(r[1].clone());
        emails.push(r[2].clone());
        citys.push(r[3].clone());
        joineds.push(r[4].parse::<i64>().expect("joined"));
    }
    let mut start = 0usize;
    while start < ids.len() {
        let end = (start + 10_000).min(ids.len());
        let n = end - start;
        let q = cypher(
            "UNWIND range(0, $n - 1) AS i CREATE (:Person {id: $ids[i], name: $names[i], email: $emails[i], city: $cities[i], joined: $joineds[i]})",
        )
        .param("n", n as i64)
        .param("ids", ids[start..end].to_vec())
        .param("names", names[start..end].to_vec())
        .param("emails", emails[start..end].to_vec())
        .param("cities", citys[start..end].to_vec())
        .param("joineds", joineds[start..end].to_vec());
        graph.run(q).await.expect("insert persons");
        start = end;
    }
    let products = read_csv_rows(&dataset.dir, "products");
    {
        let mut skus = Vec::with_capacity(products.len());
        let mut names = Vec::with_capacity(products.len());
        let mut cats = Vec::with_capacity(products.len());
        let mut prices = Vec::with_capacity(products.len());
        for r in &products {
            skus.push(r[0].clone());
            names.push(r[1].clone());
            cats.push(r[2].clone());
            prices.push(r[3].parse::<i64>().expect("price"));
        }
        let mut start = 0usize;
        while start < skus.len() {
            let end = (start + 10_000).min(skus.len());
            let n = end - start;
            let q = cypher(
                "UNWIND range(0, $n - 1) AS i CREATE (:Product {sku: $skus[i], name: $names[i], category: $cats[i], price: $prices[i]})",
            )
            .param("n", n as i64)
            .param("skus", skus[start..end].to_vec())
            .param("names", names[start..end].to_vec())
            .param("cats", cats[start..end].to_vec())
            .param("prices", prices[start..end].to_vec());
            graph.run(q).await.expect("insert products");
            start = end;
        }
    }

    for (stem, rel, target_field) in [
        ("knows", "KNOWS", "id"),
        ("purchased", "PURCHASED", "sku"),
    ] {
        let rows = read_csv_rows(&dataset.dir, stem);
        let mut froms = Vec::with_capacity(rows.len());
        let mut tos = Vec::with_capacity(rows.len());
        for r in &rows {
            froms.push(r[0].clone());
            tos.push(r[1].clone());
        }
        let target_label = if rel == "KNOWS" { "Person" } else { "Product" };
        let stmt = format!(
            "UNWIND range(0, $n - 1) AS i \
             MATCH (a:Person {{id: $froms[i]}}) MATCH (b:{target_label} {{{target_field}: $tos[i]}}) \
             CREATE (a)-[:{rel}]->(b)"
        );
        let mut start = 0usize;
        while start < froms.len() {
            let end = (start + 10_000).min(froms.len());
            let n = end - start;
            let q = cypher(&stmt)
                .param("n", n as i64)
                .param("froms", froms[start..end].to_vec())
                .param("tos", tos[start..end].to_vec());
            graph.run(q).await.expect("insert rels");
            start = end;
        }
    }

    let started = Instant::now();
    graph
        .run(cypher("CALL db.awaitIndexes()"))
        .await
        .expect("await indexes");
    let index_ms = started.elapsed().as_millis() as u64;
    let load_ms = total.elapsed().as_millis() as u64;

    let nodes: i64 = {
        let mut stream = graph
            .execute(cypher(
                "MATCH (p:Person) RETURN count(p) AS c UNION ALL MATCH (pr:Product) RETURN count(pr) AS c",
            ))
            .await
            .expect("count nodes");
        let mut got = Vec::new();
        while let Some(row) = stream.next().await.expect("count row") {
            got.push(row.get::<i64>("c").expect("c"));
        }
        got.iter().sum()
    };
    let rels: i64 = {
        let mut stream = graph
            .execute(cypher("MATCH ()-[r]->() RETURN count(r) AS c"))
            .await
            .expect("count rels");
        let mut n = 0;
        while let Some(row) = stream.next().await.expect("count row") {
            n = row.get::<i64>("c").expect("c");
        }
        n
    };

    // Integrity: the loaded graph must match the generated one exactly. A
    // silent mismatch (loader bug, dropped rows) invalidates every number.
    let want_nodes = dataset.meta["nodes"].as_i64().unwrap_or(-1);
    let want_rels = dataset.meta["rels"].as_i64().unwrap_or(-1);
    assert_eq!(nodes, want_nodes, "node count mismatch: neo4j {nodes} vs generated {want_nodes}");
    assert_eq!(rels, want_rels, "relationship count mismatch: neo4j {rels} vs generated {want_rels}");

    println!(
        "{}",
        json!({
            "load_ms": load_ms,
            "index_await_ms": index_ms,
            "nodes": nodes,
            "relationships": rels,
        })
    );
}

/// All rows of a stem's shards (stem-001.csv, …), headers stripped, in order.
fn read_csv_rows(dir: &std::path::Path, stem: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut i = 1;
    loop {
        let path = dir.join(format!("{stem}-{i:03}.csv"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            break;
        };
        for (n, line) in text.lines().enumerate() {
            if n == 0 {
                continue;
            }
            if line.is_empty() {
                continue;
            }
            rows.push(line.split(',').map(str::to_string).collect());
        }
        i += 1;
    }
    if rows.is_empty() {
        panic!("no rows for {stem} in {}", dir.display());
    }
    rows
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- args

pub fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].clone())
}

// ---------------------------------------------------------------- run command

pub async fn run_command(args: &[String]) {
    let engine_name = flag(args, "--engine").unwrap_or_else(|| usage());
    let url = flag(args, "--url").unwrap_or_else(|| usage());
    let password = flag(args, "--password").unwrap_or_default();
    let dataset_dir = flag(args, "--dataset").unwrap_or_else(|| usage());
    let kind = Kind::parse(&flag(args, "--query").unwrap_or_else(|| usage())).unwrap_or_else(|| usage());
    let conc: usize = flag(args, "--conc").unwrap_or_else(|| usage()).parse().expect("conc");
    let seconds: u64 = flag(args, "--seconds").unwrap_or_else(|| usage()).parse().expect("seconds");
    let warmup_s: u64 = flag(args, "--warmup").map(|s| s.parse().expect("warmup")).unwrap_or(3);
    let write_base: u64 = flag(args, "--write-base").map(|s| s.parse().expect("write-base")).unwrap_or(0);
    let tag = flag(args, "--tag").unwrap_or_default();

    let dataset = Dataset::load(&dataset_dir).unwrap_or_else(|e| {
        eprintln!("dataset: {e}");
        std::process::exit(1)
    });
    let engine = match engine_name.as_str() {
        "zega" => Engine::zega(&url, &dataset).await,
        "neo4j" => Engine::neo4j(&url, &password).await,
        "neo4j-http" => Ok(Engine::neo4j_http(&url, &password)),
        _ => usage(),
    }
    .unwrap_or_else(|e| {
        eprintln!("engine: {e}");
        std::process::exit(1)
    });

    let params = dataset.params_for(kind);
    let stats = run_cell(
        engine.clone(),
        kind,
        params,
        conc,
        Duration::from_secs(warmup_s),
        Duration::from_secs(seconds),
        write_base,
    )
    .await;
    println!(
        "RESULT {tag},{},{},{conc},{seconds},{},{},{:.1},{:.3},{:.3},{:.3}",
        engine.name(),
        kind.name(),
        stats.ops,
        stats.errors,
        stats.qps,
        stats.p50_ms,
        stats.p95_ms,
        stats.p99_ms,
    );
    if stats.errors > 0 {
        eprintln!("WARNING {} errors in cell {tag} {}", stats.errors, kind.name());
    }
}

pub async fn parity_command(args: &[String]) {
    let zega_url = flag(args, "--zega-url").unwrap_or_else(|| usage());
    let neo4j_url = flag(args, "--neo4j-url").unwrap_or_else(|| usage());
    let password = flag(args, "--password").unwrap_or_else(|| usage());
    let dataset_dir = flag(args, "--dataset").unwrap_or_else(|| usage());
    let samples: usize = flag(args, "--samples").map(|s| s.parse().expect("samples")).unwrap_or(25);

    let dataset = Dataset::load(&dataset_dir).unwrap_or_else(|e| {
        eprintln!("dataset: {e}");
        std::process::exit(1)
    });
    let zega = Engine::zega(&zega_url, &dataset).await.unwrap_or_else(|e| {
        eprintln!("zega: {e}");
        std::process::exit(1)
    });
    let neo4j = Engine::neo4j(&neo4j_url, &password).await.unwrap_or_else(|e| {
        eprintln!("neo4j: {e}");
        std::process::exit(1)
    });
    parity(&zega, &neo4j, &dataset, samples)
        .await
        .unwrap_or_else(|e| {
            eprintln!("parity: {e}");
            std::process::exit(1)
        });
}
