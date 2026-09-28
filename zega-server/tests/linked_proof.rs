//! End-to-end APS 39 proof. EARTH and FAN are separate OS processes running
//! zega-server. Parent-owned proxies count actual HTTP TCP-stream bytes and
//! cut transport; a fake Worker implements the shared mailbox contract.
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use zega::{linked::*, Zega};
use zega_server::{
    sync::{Config, Runtime},
    AppState,
};
const SCHEMA:&str="type Entity { id: String name: String wikidata?: String score: Int related -> Entity[] } unique { Entity { id } }";
const PUBLIC_FIXTURE_KEY: &str = "public-proof-fixture-not-a-deployment-secret";
#[derive(Serialize, Deserialize)]
struct Host {
    data: String,
    address: SocketAddr,
    linked: Config,
}
#[test]
#[ignore = "child process entry point, launched only by proof scenarios"]
fn proof_server_child() {
    let path = std::env::var("ZEGA_PROOF_CONFIG").unwrap();
    let host: Host = serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let db = Zega::open(&host.data).snapshot_every(0).build().unwrap();
            let mut state = AppState::new(db, None);
            state.sync = Some(Arc::new(
                Runtime::new(host.linked, Some(PUBLIC_FIXTURE_KEY.into())).unwrap(),
            ));
            let listener = TcpListener::bind(host.address).await.unwrap();
            let worker = zega_server::sync::start(state.clone()).await.unwrap();
            let app = zega_server::routes::app(state.clone())
                .route("/proof/write", post(timed_write).with_state(state));
            axum::serve(listener, app).await.unwrap();
            worker.abort();
        });
}
/// Test-only instrumentation in the server process. Time includes contention
/// for the same AppState gate, schema/query execution, WAL append and fsync.
/// The production engine and asynchronous sync worker are unchanged.
async fn timed_write(State(state): State<AppState>, Json(query): Json<String>) -> Json<u64> {
    Json(
        tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            state.zega.lock().unwrap().run_lang(SCHEMA, &query).unwrap();
            u64::try_from(start.elapsed().as_nanos()).unwrap()
        })
        .await
        .unwrap(),
    )
}
struct Process {
    child: Child,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
struct Wire {
    sent: AtomicU64,
    received: AtomicU64,
    connections: AtomicU64,
    cut: AtomicBool,
}
impl Wire {
    fn total(&self) -> u64 {
        self.sent.load(Ordering::Relaxed) + self.received.load(Ordering::Relaxed)
    }
}
struct Proxy {
    url: String,
    wire: Arc<Wire>,
    task: JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn proxy(target: SocketAddr) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let wire = Arc::new(Wire::default());
    let counts = wire.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((incoming, _)) = listener.accept().await else {
                break;
            };
            let counts = counts.clone();
            counts.connections.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                if counts.cut.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(outgoing) = TcpStream::connect(target).await else {
                    return;
                };
                let (mut ir, mut iw) = incoming.into_split();
                let (mut or, mut ow) = outgoing.into_split();
                let a = counts.clone();
                let b = counts.clone();
                let upstream = async move {
                    let mut buf = [0u8; 8192];
                    loop {
                        let n = ir.read(&mut buf).await?;
                        if n == 0 || a.cut.load(Ordering::Relaxed) {
                            break;
                        }
                        a.sent.fetch_add(n as u64, Ordering::Relaxed);
                        ow.write_all(&buf[..n]).await?;
                    }
                    ow.shutdown().await
                };
                let downstream = async move {
                    let mut buf = [0u8; 8192];
                    loop {
                        let n = or.read(&mut buf).await?;
                        if n == 0 || b.cut.load(Ordering::Relaxed) {
                            break;
                        }
                        b.received.fetch_add(n as u64, Ordering::Relaxed);
                        iw.write_all(&buf[..n]).await?;
                    }
                    iw.shutdown().await
                };
                let _: (std::io::Result<()>, std::io::Result<()>) =
                    tokio::join!(upstream, downstream);
            });
        }
    });
    Proxy { url, wire, task }
}
#[derive(Default)]
struct Observed {
    paths: Mutex<Vec<String>>,
    pushes: Mutex<Vec<Diff>>,
}
#[derive(Clone)]
struct Gateway {
    target: String,
    observed: Arc<Observed>,
    client: reqwest::Client,
}
async fn forward(State(g): State<Gateway>, request: Request<Body>) -> Response {
    let path = request.uri().to_string();
    g.observed.paths.lock().unwrap().push(path.clone());
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 16_000_000).await.unwrap();
    if path == "/zega/sync/push" {
        let raw = if parts
            .headers
            .get("content-encoding")
            .is_some_and(|h| h == "gzip")
        {
            let mut raw = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes.as_ref()), &mut raw)
                .unwrap();
            raw
        } else {
            bytes.to_vec()
        };
        g.observed
            .pushes
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&raw).unwrap());
    }
    let mut req = g
        .client
        .request(parts.method, format!("{}{path}", g.target))
        .body(bytes);
    for (name, value) in &parts.headers {
        if name != "host" && name != "content-length" {
            req = req.header(name, value);
        }
    }
    match req.send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.bytes().await.unwrap();
            let mut response = (status, bytes).into_response();
            *response.headers_mut() = headers;
            response
        }
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}
async fn gateway(target: SocketAddr) -> (Proxy, Arc<Observed>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let observed = Arc::new(Observed::default());
    let g = Gateway {
        target: format!("http://{target}"),
        observed: observed.clone(),
        client: reqwest::Client::builder()
            .no_gzip()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap(),
    };
    let task = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(forward).with_state(g))
            .await
            .unwrap();
    });
    (proxy(address).await, observed, task)
}
struct Queued {
    diff: Diff,
    expires: Instant,
}
#[derive(Default)]
struct Mailbox {
    entries: Mutex<BTreeMap<String, Queued>>,
    puts: AtomicU64,
    deletes: AtomicU64,
}
fn mailbox_authorized(headers: &HeaderMap, token: &str) -> bool {
    zega_server::auth::authorized(headers, &zega_server::auth::hash_token(token))
}
fn mailbox_token(subscriber: &str) -> String {
    use base64::Engine;
    use hmac::Mac;
    let mut mac =
        hmac::Hmac::<sha2::Sha256>::new_from_slice(PUBLIC_FIXTURE_KEY.as_bytes()).unwrap();
    mac.update(subscriber.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}
async fn mailbox_put(
    State(m): State<Arc<Mailbox>>,
    Path((subscriber, version)): Path<(String, u64)>,
    headers: HeaderMap,
    Json(diff): Json<Diff>,
) -> StatusCode {
    if !mailbox_authorized(&headers, PUBLIC_FIXTURE_KEY) {
        return StatusCode::UNAUTHORIZED;
    }
    assert_eq!(version, diff.graph_version);
    m.entries.lock().unwrap().insert(
        format!("mb:{subscriber}:{version:020}"),
        Queued {
            diff,
            expires: Instant::now() + MAILBOX_TTL,
        },
    );
    m.puts.fetch_add(1, Ordering::Relaxed);
    StatusCode::NO_CONTENT
}
async fn mailbox_get(
    State(m): State<Arc<Mailbox>>,
    Path(subscriber): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !mailbox_authorized(&headers, &mailbox_token(&subscriber)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let entries = m.entries.lock().unwrap();
    let prefix = format!("mb:{subscriber}:");
    let cursor = query.get("cursor").cloned().unwrap_or_default();
    let mut items: Vec<_> = entries
        .iter()
        .filter(|(key, queued)| {
            key.starts_with(&prefix) && **key > cursor && queued.expires > Instant::now()
        })
        .take(101)
        .map(|(key, queued)| MailboxItem {
            key: key.clone(),
            diff: queued.diff.clone(),
        })
        .collect();
    let cursor = if items.len() > 100 {
        items.pop();
        items.last().map(|i| i.key.clone())
    } else {
        None
    };
    Json(MailboxPage { items, cursor }).into_response()
}
async fn mailbox_delete(
    State(m): State<Arc<Mailbox>>,
    Path((subscriber, version)): Path<(String, u64)>,
    headers: HeaderMap,
) -> StatusCode {
    if !mailbox_authorized(&headers, &mailbox_token(&subscriber)) {
        return StatusCode::UNAUTHORIZED;
    }
    m.entries
        .lock()
        .unwrap()
        .remove(&format!("mb:{subscriber}:{version:020}"));
    m.deletes.fetch_add(1, Ordering::Relaxed);
    StatusCode::NO_CONTENT
}
struct Rig {
    dir: tempfile::TempDir,
    earth: Option<Process>,
    fan: Option<Process>,
    earth_address: SocketAddr,
    fan_address: SocketAddr,
    source: Proxy,
    push: Proxy,
    mail: Proxy,
    source_observed: Arc<Observed>,
    push_observed: Arc<Observed>,
    mailbox: Arc<Mailbox>,
    tasks: Vec<JoinHandle<()>>,
    client: reqwest::Client,
    ids: Vec<String>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.fan.take();
        self.earth.take();
        for task in &self.tasks {
            task.abort();
        }
    }
}
fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}
impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let earth_address = free_address();
        let fan_address = free_address();
        let (source, source_observed, source_task) = gateway(earth_address).await;
        let (push, push_observed, push_task) = gateway(fan_address).await;
        let mailbox = Arc::new(Mailbox::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let m = mailbox.clone();
        let mailbox_task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/mb/:subscriber", get(mailbox_get))
                    .route(
                        "/mb/:subscriber/:version",
                        put(mailbox_put).delete(mailbox_delete),
                    )
                    .with_state(m),
            )
            .await
            .unwrap();
        });
        let mail = proxy(address).await;
        let mut rig = Self {
            dir,
            earth: None,
            fan: None,
            earth_address,
            fan_address,
            source,
            push,
            mail,
            source_observed,
            push_observed,
            mailbox,
            tasks: vec![source_task, push_task, mailbox_task],
            client: reqwest::Client::new(),
            ids: Vec::new(),
        };
        let earth = Zega::open(rig.dir.path().join("earth").to_str().unwrap())
            .snapshot_every(0)
            .build()
            .unwrap();
        let mut fixture: Vec<Value> = serde_json::from_str(include_str!(
            "../../experiments/search-intake/entities.json"
        ))
        .unwrap();
        // The landed intake has 38 hockey entities. Complete the requested
        // 50-node slice with hand-written city entities. Identity is a ZID
        // allocated by earth in fixture order; the Wikidata QID is kept as an
        // external-id property, never as identity (APS 39 §1).
        let cities = [
            ("Q2096", "Edmonton"),
            ("Q36312", "Calgary"),
            ("Q340", "Montreal"),
            ("Q172", "Toronto"),
            ("Q1930", "Ottawa"),
            ("Q24639", "Vancouver"),
            ("Q2135", "Winnipeg"),
            ("Q2145", "Quebec City"),
            ("Q5083", "Seattle"),
            ("Q100", "Boston"),
            ("Q8652", "Miami"),
            ("Q12439", "Detroit"),
        ];
        for (qid, label) in &cities {
            fixture.push(json!({"qid":qid,"label":label}));
        }
        assert_eq!(fixture.len(), 50);
        for (n, row) in fixture.iter().take(50).enumerate() {
            let id = format!("Z{}", n + 1);
            let qid = row["qid"].as_str().unwrap();
            let name = row["label"].as_str().unwrap();
            earth
                .run_lang(
                    SCHEMA,
                    &format!(
                        "mutation {{ Entity(id: {} && name: {} && wikidata: {} && score: 0) {{ id }} }}",
                        json!(id),
                        json!(name),
                        json!(qid)
                    ),
                )
                .unwrap();
            rig.ids.push(id);
        }
        // One-hop source edge, plus local fan edges below.
        earth.connect_schema(SCHEMA, 1, "related", 2).unwrap();
        drop(earth);
        rig.write_config(
            "earth",
            earth_address,
            Config {
                graph: "earth".into(),
                mailbox: Some(rig.mail.url.clone()),
                subscriber: None,
                endpoint: None,
                sources: BTreeMap::new(),
            },
        );
        rig.write_config(
            "fan",
            fan_address,
            Config {
                graph: "fan".into(),
                mailbox: Some(rig.mail.url.clone()),
                subscriber: Some("fan".into()),
                endpoint: Some(rig.push.url.clone()),
                sources: BTreeMap::from([("earth".into(), rig.source.url.clone())]),
            },
        );
        rig.earth = Some(rig.launch("earth"));
        rig.wait(earth_address).await;
        rig.fan = Some(rig.launch("fan"));
        rig.wait(fan_address).await;
        for id in rig.ids.iter().take(20) {
            let response = rig
                .client
                .post(format!("http://{fan_address}/sync/link"))
                .json(&json!({"reference":format!("zega://earth/{id}")}))
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body = response.text().await.unwrap();
            assert!(status.is_success(), "link {status}: {body}");
        }
        rig.query(
            fan_address,
            &format!(
                "mutation {{ Entity(id: {} && name: \"Fan club\" && score: 0) {{ id }} }}",
                json!("fan-club")
            ),
        )
        .await;
        let response = rig
            .client
            .post(format!("http://{fan_address}/graph/relationships"))
            .json(&json!({"schema":SCHEMA,"from":21,"field":"related","to":1}))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
        rig
    }
    fn write_config(&self, name: &str, address: SocketAddr, linked: Config) {
        let host = Host {
            data: self.dir.path().join(name).to_str().unwrap().into(),
            address,
            linked,
        };
        serde_json::to_writer(
            std::fs::File::create(self.dir.path().join(format!("{name}.json"))).unwrap(),
            &host,
        )
        .unwrap();
    }
    fn launch(&self, name: &str) -> Process {
        let log = std::fs::File::create(self.dir.path().join(format!("{name}.log"))).unwrap();
        Process {
            child: Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "proof_server_child", "--ignored", "--nocapture"])
                .env(
                    "ZEGA_PROOF_CONFIG",
                    self.dir.path().join(format!("{name}.json")),
                )
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        }
    }
    async fn wait(&self, address: SocketAddr) {
        let start = Instant::now();
        loop {
            if self
                .client
                .get(format!("http://{address}/health"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            if start.elapsed() >= Duration::from_secs(10) {
                for name in ["earth", "fan"] {
                    if let Ok(log) =
                        std::fs::read_to_string(self.dir.path().join(format!("{name}.log")))
                    {
                        eprintln!("{name}: {log}");
                    }
                }
                panic!("server did not start");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    async fn query(&self, address: SocketAddr, query: &str) -> Value {
        let reply: Value = self
            .client
            .post(format!("http://{address}/zql"))
            .json(&json!({"schema":SCHEMA,"query":query}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    }
    async fn edit(&self, n: u64) {
        self.query(
            self.earth_address,
            &format!(
                "mutation {{ Entity(id: {}) set score: {n} {{ score }} }}",
                json!(self.ids[0])
            ),
        )
        .await;
    }
    async fn score(&self) -> u64 {
        self.query(
            self.fan_address,
            &format!("{{ Entity(id: {}) {{ score }} }}", json!(self.ids[0])),
        )
        .await["score"]
            .as_u64()
            .unwrap()
    }
    async fn until_score(&self, n: u64, limit: Duration) {
        let start = Instant::now();
        while self.score().await != n {
            assert!(start.elapsed() < limit, "mirror did not reach {n}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    async fn until_mail(&self) {
        let start = Instant::now();
        while self.mailbox.entries.lock().unwrap().is_empty() {
            assert!(
                start.elapsed() < Duration::from_secs(7),
                "mailbox stayed empty"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    fn bytes(&self) -> u64 {
        self.source.wire.total() + self.push.wire.total() + self.mail.wire.total()
    }
    fn report(&self, scenario: &str, before: u64, elapsed: Duration) {
        println!(
            "PROOF {scenario} exit=0 wire_bytes={} elapsed_ms={:.3}",
            self.bytes() - before,
            elapsed.as_secs_f64() * 1000.
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_offline_query_has_zero_network_calls() {
    let rig = Rig::new().await;
    rig.source.wire.cut.store(true, Ordering::Relaxed);
    let before = rig.source.wire.total();
    let connections = rig.source.wire.connections.load(Ordering::Relaxed);
    let rows = rig
        .query(rig.fan_address, "{ Entity { id name score } }")
        .await;
    assert_eq!(rows.as_array().unwrap().len(), 21);
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|v| v["id"].as_str().is_some_and(|id| id.starts_with('Z')))
            .count(),
        20
    );
    let edges = rig
        .query(
            rig.fan_address,
            "{ Entity(id: \"fan-club\") { related -> Entity { name } } }",
        )
        .await;
    assert!(edges.to_string().contains("Anaheim Ducks"));
    assert_eq!(rig.source.wire.total(), before);
    assert_eq!(
        rig.source.wire.connections.load(Ordering::Relaxed),
        connections
    );
    println!("PROOF a exit=0 network_calls=0");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b_update_arrives_by_push_within_six_seconds() {
    let rig = Rig::new().await;
    let before = rig.bytes();
    let started = Instant::now();
    rig.edit(1).await;
    rig.until_score(1, Duration::from_secs(6)).await;
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(rig.push_observed.pushes.lock().unwrap().len(), 1);
    assert_eq!(rig.mailbox.puts.load(Ordering::Relaxed), 0);
    rig.report("b", before, started.elapsed());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c_offline_subscriber_drains_mailbox_before_live_pushes() {
    let mut rig = Rig::new().await;
    rig.fan.take();
    let before = rig.bytes();
    let started = Instant::now();
    rig.edit(2).await;
    rig.until_mail().await;
    let fetches = rig
        .source_observed
        .paths
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("/sync/node/"))
        .count();
    rig.fan = Some(rig.launch("fan"));
    rig.wait(rig.fan_address).await;
    assert_eq!(rig.score().await, 2);
    assert!(rig.mailbox.entries.lock().unwrap().is_empty());
    assert_eq!(rig.mailbox.deletes.load(Ordering::Relaxed), 1);
    assert_eq!(
        rig.source_observed
            .paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with("/sync/node/"))
            .count(),
        fetches,
        "repair must not mask a broken drain"
    );
    rig.report("c", before, started.elapsed());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d_lost_mailbox_entry_is_repaired_by_batch_check() {
    let mut rig = Rig::new().await;
    rig.fan.take();
    let before = rig.bytes();
    let started = Instant::now();
    rig.edit(3).await;
    rig.until_mail().await;
    rig.mailbox.entries.lock().unwrap().clear();
    let fetches = rig
        .source_observed
        .paths
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("/sync/node/"))
        .count();
    rig.fan = Some(rig.launch("fan"));
    rig.wait(rig.fan_address).await;
    assert_eq!(rig.score().await, 3);
    assert_eq!(
        rig.source_observed
            .paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with("/sync/node/"))
            .count(),
        fetches + 1
    );
    rig.report("d", before, started.elapsed());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e_five_edits_send_exactly_one_field_level_diff() {
    let rig = Rig::new().await;
    let before = rig.bytes();
    let started = Instant::now();
    for n in 1..=5 {
        rig.edit(n).await;
    }
    rig.until_score(5, Duration::from_secs(6)).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let pushes = rig.push_observed.pushes.lock().unwrap();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].changes.len(), 1);
    let Change::Upsert { fields, .. } = &pushes[0].changes[0].change else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields["score"], json!(5));
    rig.report("e", before, started.elapsed());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_mirror_writes_are_rejected() {
    let rig = Rig::new().await;
    let reply:Value=rig.client.post(format!("http://{}/zql",rig.fan_address)).json(&json!({"schema":SCHEMA,"query":format!("mutation {{ Entity(id: {}) set score: 99 {{ score }} }}",json!(rig.ids[0]))})).send().await.unwrap().json().await.unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"]
        .as_str()
        .unwrap()
        .contains("mirror facts are read-only"));
    assert_eq!(rig.score().await, 0);
    println!("PROOF f exit=0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g_ten_thousand_leases_do_not_slow_source_writes() {
    // Compare two otherwise identical EARTH processes. Registration happens
    // before timing. Both run the production commit hook and delivery worker.
    let mut rig = Rig::new().await;
    rig.fan.take();
    rig.earth.take();
    let address = free_address();
    let data = rig.dir.path().join("earth-ten-thousand");
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(
        SCHEMA,
        &format!(
            "mutation {{ Entity(id: {} && name: \"Anaheim Ducks\" && score: 0) {{ id }} }}",
            json!(rig.ids[0])
        ),
    )
    .unwrap();
    for n in 0..10_000 {
        db.subscribe(&SubscribeRequest {
            subscriber: format!("subscriber-{n}"),
            endpoint: None,
            ids: vec![rig.ids[0].clone()],
            lease_secs: MAILBOX_TTL.as_secs(),
        })
        .unwrap();
    }
    assert_eq!(
        db.subscribers(&[rig.ids[0].clone()]).unwrap()[&rig.ids[0]].len(),
        10_000
    );
    let mut fixture = Vec::new();
    db.export(&mut fixture).unwrap();
    drop(db);
    let persisted = Zega::open(data.to_str().unwrap())
        .snapshot_every(0)
        .build()
        .unwrap();
    persisted.import(fixture.as_slice()).unwrap();
    drop(persisted);
    rig.write_config(
        "earth-ten-thousand",
        address,
        Config {
            graph: "earth".into(),
            mailbox: None,
            subscriber: None,
            endpoint: None,
            sources: BTreeMap::new(),
        },
    );
    let _many = rig.launch("earth-ten-thousand");
    rig.wait(address).await;
    let baseline_address = free_address();
    let baseline = Zega::open(rig.dir.path().join("earth-baseline").to_str().unwrap())
        .snapshot_every(0)
        .build()
        .unwrap();
    baseline
        .run_lang(
            SCHEMA,
            &format!(
                "mutation {{ Entity(id: {} && name: \"Anaheim Ducks\" && score: 0) {{ id }} }}",
                json!(rig.ids[0])
            ),
        )
        .unwrap();
    drop(baseline);
    rig.write_config(
        "earth-baseline",
        baseline_address,
        Config {
            graph: "earth".into(),
            mailbox: None,
            subscriber: None,
            endpoint: None,
            sources: BTreeMap::new(),
        },
    );
    let _baseline = rig.launch("earth-baseline");
    rig.wait(baseline_address).await;
    // Alternate order in paired blocks to reduce thermal/load and filesystem
    // drift. Median block means include lock contention, WAL durability, and worker load.
    let mut none = Vec::new();
    let mut many = Vec::new();
    for block in 0..11 {
        let addresses = if block % 2 == 0 {
            [baseline_address, address]
        } else {
            [address, baseline_address]
        };
        for target in addresses {
            let mut elapsed = 0u64;
            for n in 0..50 {
                let query = format!(
                    "mutation {{ Entity(id: {}) set score: {} {{ score }} }}",
                    json!(rig.ids[0]),
                    block * 50 + n
                );
                elapsed += rig
                    .client
                    .post(format!("http://{target}/proof/write"))
                    .json(&query)
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .json::<u64>()
                    .await
                    .unwrap();
            }
            if block > 0 {
                let sample = elapsed as f64 / 1e9 / 50.;
                if target == address {
                    many.push(sample);
                } else {
                    none.push(sample);
                }
            }
        }
    }
    none.sort_by(f64::total_cmp);
    many.sort_by(f64::total_cmp);
    let zero = (none[4] + none[5]) / 2.;
    let ten_k = (many[4] + many[5]) / 2.;
    let ratio = ten_k / zero;
    println!("PROOF g baseline_us={:.3} ten_thousand_us={:.3} delta_percent={:.3} samples_per_condition=500",zero*1e6,ten_k*1e6,(ratio-1.)*100.);
    assert!(
        ratio <= 1.05,
        "10k subscription write latency must stay within 5% of none: {ratio:.5}"
    );
    println!("PROOF g exit=0");
}
