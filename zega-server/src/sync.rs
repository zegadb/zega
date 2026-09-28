//! APS 39 transport. A single worker reads committed WAL-derived history;
//! network I/O and subscriber lookup never run on an engine write path.
use crate::{
    handlers::{authorized, error},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zega::linked::*;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub graph: String,
    #[serde(default)]
    pub mailbox: Option<String>,
    #[serde(default)]
    pub subscriber: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
}
// No Debug: mailbox credentials must never enter diagnostics.
pub struct Runtime {
    config: Config,
    secret: Option<String>,
    tokens: Mutex<HashMap<String, String>>,
    client: reqwest::Client,
    ready: std::sync::atomic::AtomicBool,
    wake: Arc<tokio::sync::Notify>,
}
impl Runtime {
    /// The host supplies SOURCE_SECRET from its secret manager/environment.
    /// No secret is serialized into graph files, configuration, or logs.
    pub fn new(config: Config, source_secret: Option<String>) -> Result<Self, String> {
        Reference::new(&config.graph, "validate").map_err(|e| e.to_string())?;
        if config.mailbox.is_some() && config.sources.len() > 1 {
            return Err("one mailbox subscriber identity supports one source; use separate identities for separate sources".into());
        }
        if let Some(subscriber) = &config.subscriber {
            Reference::new("subscriber", subscriber).map_err(|e| e.to_string())?;
        }
        for url in config
            .sources
            .values()
            .chain(config.mailbox.iter())
            .chain(config.endpoint.iter())
        {
            validate_url(url)?;
        }
        if config.mailbox.is_some() && source_secret.is_none() && config.sources.is_empty() {
            return Err("mailbox source requires SOURCE_SECRET".into());
        }
        Ok(Self {
            config,
            secret: source_secret,
            tokens: Mutex::new(HashMap::new()),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?,
            ready: false.into(),
            wake: Arc::default(),
        })
    }
    fn token(&self, subscriber: &str) -> String {
        let Some(secret) = &self.secret else {
            return String::new();
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts every key length");
        mac.update(subscriber.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }
}
fn validate_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "invalid sync URL")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("sync URLs must use HTTP(S), without credentials, query or fragment".into());
    }
    Ok(())
}
fn mailbox_url(base: &str, subscriber: &str, version: Option<u64>) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "invalid mailbox URL")?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| "invalid mailbox URL")?;
        segments.pop_if_empty().extend(["mb", subscriber]);
        if let Some(version) = version {
            segments.push(&version.to_string());
        }
    }
    Ok(url)
}
fn runtime(state: &AppState) -> Result<Arc<Runtime>, &'static str> {
    state.sync.clone().ok_or("linked graphs are not configured")
}
fn transport_error() -> Response {
    error(StatusCode::BAD_GATEWAY, "sync transport failed")
}
async fn engine<T: Send + 'static>(
    state: &AppState,
    op: impl FnOnce(&zega::Zega) -> zega::Result<T> + Send + 'static,
) -> Result<T, String> {
    let db = state.zega.clone();
    tokio::task::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "engine lock poisoned".to_owned())?;
        op(&db).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "engine task failed".to_owned())?
}
#[derive(Deserialize)]
pub struct Hops {
    hops: u8,
}
pub async fn node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(hops): Query<Hops>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if runtime(&state).is_err() {
        return error(StatusCode::NOT_FOUND, "linked graphs are not configured");
    }
    if hops.hops != 1 {
        return error(StatusCode::BAD_REQUEST, "linked graphs require hops=1");
    }
    match engine(&state, move |db| db.sync_node(&id)).await {
        Ok(node) => Json(node).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, e),
    }
}
#[derive(Deserialize)]
pub struct MirrorSource {
    source: String,
}
/// Provenance is local data, including the source-gone state; never a fetch.
pub async fn mirrors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(source): Query<MirrorSource>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match engine(&state, move |db| db.mirrors(&source.source)).await {
        Ok(mirrors) => Json(mirrors).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
pub async fn check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CheckRequest>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if let Err(message) = request.validate() {
        return error(StatusCode::BAD_REQUEST, message);
    }
    match engine(&state, move |db| db.sync_check(&request)).await {
        Ok(reply) => Json(reply).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
pub async fn subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SubscribeRequest>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let runtime = match runtime(&state) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    if let Some(endpoint) = &request.endpoint {
        if let Err(e) = validate_url(endpoint) {
            return error(StatusCode::BAD_REQUEST, e);
        }
    }
    let mailbox_token = runtime.token(&request.subscriber);
    match engine(&state, move |db| db.subscribe(&request)).await {
        Ok(graph_version) => Json(SubscribeResponse {
            mailbox_token,
            graph_version,
        })
        .into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
#[derive(Deserialize)]
pub struct LinkRequest {
    reference: Reference,
}
pub async fn link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LinkRequest>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let runtime = match runtime(&state) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    match fetch(&state, &runtime, &request.reference).await {
        Ok(id) => match renew(&state, &runtime, request.reference.graph()).await {
            Ok(()) => Json(serde_json::json!({"id":id})).into_response(),
            Err(_) => transport_error(),
        },
        Err(_) => transport_error(),
    }
}
async fn fetch(state: &AppState, runtime: &Runtime, reference: &Reference) -> Result<u64, String> {
    let base = runtime
        .config
        .sources
        .get(reference.graph())
        .ok_or("source is not configured")?;
    let mut url = reqwest::Url::parse(base).map_err(|_| "invalid source URL")?;
    url.path_segments_mut()
        .map_err(|_| "invalid source URL")?
        .pop_if_empty()
        .extend(["sync", "node", reference.id()]);
    url.query_pairs_mut().append_pair("hops", "1");
    let response = runtime
        .client
        .get(url)
        .send()
        .await
        .map_err(|_| "source fetch failed")?
        .error_for_status()
        .map_err(|_| "source fetch failed")?;
    // /sync endpoints return the contract directly.
    let snapshot: NodeSnapshot = response.json().await.map_err(|_| "invalid source node")?;
    let reference = reference.clone();
    engine(state, move |db| db.link(&reference, &snapshot)).await
}
async fn renew(state: &AppState, runtime: &Runtime, source: &str) -> Result<(), String> {
    let name = source.to_owned();
    let mirrors = engine(state, move |db| db.mirrors(&name)).await?;
    let ids: Vec<_> = mirrors
        .into_iter()
        .filter(|(_, m)| !m.stub)
        .map(|(_, m)| m.id)
        .collect();
    let subscriber = runtime
        .config
        .subscriber
        .as_ref()
        .ok_or("subscriber name is required")?;
    for batch in ids.chunks(MAX_CHECK_ITEMS) {
        let reply: SubscribeResponse = runtime
            .client
            .post(format!(
                "{}/sync/subscribe",
                runtime.config.sources[source].trim_end_matches('/')
            ))
            .json(&SubscribeRequest {
                subscriber: subscriber.clone(),
                endpoint: runtime.config.endpoint.clone(),
                ids: batch.to_vec(),
                lease_secs: MAILBOX_TTL.as_secs(),
            })
            .send()
            .await
            .map_err(|_| "subscribe failed")?
            .error_for_status()
            .map_err(|_| "subscribe failed")?
            .json()
            .await
            .map_err(|_| "invalid subscription")?;
        runtime
            .tokens
            .lock()
            .map_err(|_| "token lock poisoned")?
            .insert(source.into(), reply.mailbox_token);
    }
    Ok(())
}
pub async fn push(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let runtime = match runtime(&state) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    if !runtime.ready.load(std::sync::atomic::Ordering::Acquire) {
        return error(StatusCode::SERVICE_UNAVAILABLE, "mailbox drain in progress");
    }
    let bytes = if headers.get("content-encoding").is_some_and(|v| v == "gzip") {
        let mut decoded = Vec::new();
        if flate2::read::GzDecoder::new(body.as_ref())
            .take(16_000_001)
            .read_to_end(&mut decoded)
            .is_err()
            || decoded.len() > 16_000_000
        {
            return error(StatusCode::BAD_REQUEST, "invalid compressed diff");
        }
        decoded
    } else {
        body.to_vec()
    };
    let diff: Diff = match serde_json::from_slice(&bytes) {
        Ok(d) => d,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid diff"),
    };
    let expected = runtime
        .tokens
        .lock()
        .ok()
        .and_then(|t| t.get(&diff.source).cloned());
    let Some(expected) = expected else {
        return error(StatusCode::UNAUTHORIZED, "unconfigured source");
    };
    if !expected.is_empty() {
        if !crate::auth::authorized(&headers, &crate::auth::hash_token(&expected)) {
            return error(StatusCode::UNAUTHORIZED, "unauthorized source");
        }
    } else if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match receive(&state, &runtime, diff).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
async fn receive(state: &AppState, runtime: &Runtime, diff: Diff) -> Result<(), String> {
    let source = diff.source.clone();
    let mirrors = engine(state, move |db| db.mirrors(&source)).await?;
    for change in &diff.changes {
        if mirrors.iter().any(|(_, m)| {
            m.id == change.id && !m.stub && change.version > m.version && (
                change.version > m.version.saturating_add(1) ||
                matches!(&change.change, Change::Upsert { fields, rels } if fields.values().any(|v|v.is_array() || v.is_object()) || rels.add.iter().any(|r| !mirrors.iter().any(|(_,target)| target.id==r.to)))
            )
        }) {
            fetch(
                state,
                runtime,
                &Reference::new(&diff.source, &change.id).map_err(|e| e.to_string())?,
            )
            .await?;
        }
    }
    engine(state, move |db| db.apply_diff(&diff)).await
}
pub async fn repair(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let runtime = match runtime(&state) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    match wake_subscriber(&state, &runtime).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => transport_error(),
    }
}
async fn wake_subscriber(state: &AppState, runtime: &Runtime) -> Result<(), String> {
    runtime
        .ready
        .store(false, std::sync::atomic::Ordering::Release);
    for source in runtime.config.sources.keys() {
        renew(state, runtime, source).await?;
    }
    drain(state, runtime).await?;
    repair_all(state, runtime).await?;
    runtime
        .ready
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}
async fn repair_all(state: &AppState, runtime: &Runtime) -> Result<(), String> {
    for (source, base) in &runtime.config.sources {
        let name = source.clone();
        let mirrors = engine(state, move |db| db.mirrors(&name)).await?;
        let items: Vec<_> = mirrors
            .into_iter()
            .filter(|(_, m)| !m.stub)
            .map(|(_, m)| (m.id, m.version))
            .collect();
        for batch in items.chunks(MAX_CHECK_ITEMS) {
            let response: CheckResponse = runtime
                .client
                .post(format!("{}/sync/check", base.trim_end_matches('/')))
                .json(&CheckRequest {
                    items: batch.to_vec(),
                })
                .send()
                .await
                .map_err(|_| "check failed")?
                .error_for_status()
                .map_err(|_| "check failed")?
                .json()
                .await
                .map_err(|_| "invalid check response")?;
            for id in response.stale {
                fetch(
                    state,
                    runtime,
                    &Reference::new(source, &id).map_err(|e| e.to_string())?,
                )
                .await?;
            }
        }
    }
    Ok(())
}
async fn drain(state: &AppState, runtime: &Runtime) -> Result<(), String> {
    if runtime.config.sources.is_empty() {
        return Ok(());
    }
    let Some(mailbox) = &runtime.config.mailbox else {
        return Ok(());
    };
    let subscriber = runtime
        .config
        .subscriber
        .as_ref()
        .ok_or("subscriber name is required")?;
    // One mailbox identity belongs to one source: the token is source-minted.
    for source in runtime.config.sources.keys() {
        let token = runtime
            .tokens
            .lock()
            .map_err(|_| "token lock poisoned")?
            .get(source)
            .cloned();
        let Some(token) = token else { continue };
        let mut cursor = String::new();
        loop {
            let page: MailboxPage = runtime
                .client
                .get(mailbox_url(mailbox, subscriber, None)?)
                .query(&[("cursor", cursor.as_str()), ("limit", "100")])
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| "mailbox read failed")?
                .error_for_status()
                .map_err(|_| "mailbox read failed")?
                .json()
                .await
                .map_err(|_| "invalid mailbox page")?;
            let mut items = page.items;
            items.sort_by(|a, b| a.key.cmp(&b.key));
            for item in items {
                if item.diff.source != *source {
                    return Err("mailbox source mismatch".into());
                }
                let version = item.diff.graph_version;
                receive(state, runtime, item.diff).await?;
                runtime
                    .client
                    .delete(mailbox_url(mailbox, subscriber, Some(version))?)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|_| "mailbox delete failed")?
                    .error_for_status()
                    .map_err(|_| "mailbox delete failed")?;
            }
            match page.cursor {
                Some(next) if !next.is_empty() && next != cursor => cursor = next,
                Some(next) if next == cursor => return Err("mailbox cursor did not advance".into()),
                _ => break,
            }
        }
    }
    Ok(())
}
/// Start after binding the subscriber's socket. Until wake completes, pushes
/// receive 503 and fall back to the mailbox. Dropping the returned task aborts
/// no other process; the host owns its lifecycle.
pub async fn start(state: AppState) -> Result<tokio::task::JoinHandle<()>, String> {
    let runtime = state
        .sync
        .clone()
        .ok_or("linked graphs are not configured")?;
    engine(&state, |db| db.prepare_linked()).await?;
    // A source outage must not stop local queries from opening. Recovery is
    // retried by the explicit wake endpoint or daily maintenance, never by a
    // query. Live pushes remain gated until the mailbox has drained.
    if wake_subscriber(&state, &runtime).await.is_err() {
        eprintln!("zega: sync wake deferred; local queries remain available");
    }
    let wake = runtime.wake.clone();
    let cursor = engine(&state, move |db| {
        db.on_commit(Arc::new(move || wake.notify_one()))?;
        db.graph_version()
    })
    .await?;
    Ok(tokio::spawn(worker(state, runtime, cursor)))
}
struct Pending {
    started: Instant,
    lease: Lease,
    record: Record,
}
async fn worker(state: AppState, runtime: Arc<Runtime>, mut cursor: u64) {
    let mut pending = HashMap::<String, Pending>::new();
    let mut daily = Instant::now();
    loop {
        let deadline = pending
            .values()
            .map(|p| p.started + DEFAULT_COALESCE_WINDOW)
            .min()
            .unwrap_or(daily + Duration::from_secs(86400));
        tokio::select! { _=runtime.wake.notified()=>{}, _=tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))=>{} }
        let records = match engine(&state, move |db| db.changes_since(cursor)).await {
            Ok(r) => r,
            Err(_) => return,
        };
        for record in records {
            cursor = record.graph_version;
            let ids = record
                .changes
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
            // O(changed ids) shared snapshots under the lock. Iterating leases
            // and building subscriber batches happens after releasing it.
            let leases = match engine(&state, move |db| db.subscribers(&ids)).await {
                Ok(l) => l,
                Err(_) => return,
            };
            let mut recipients = HashMap::<String, (Lease, Vec<NodeChange>)>::new();
            let now = now_secs();
            for change in &record.changes {
                if let Some(subscribers) = leases.get(&change.id) {
                    for (subscriber, lease) in
                        subscribers.iter().filter(|(_, l)| l.expires_at > now)
                    {
                        if lease.endpoint.is_none() && runtime.config.mailbox.is_none() {
                            continue;
                        }
                        recipients
                            .entry(subscriber.clone())
                            .or_insert_with(|| (lease.clone(), Vec::new()))
                            .1
                            .push(change.clone());
                    }
                }
            }
            for (subscriber, (lease, changes)) in recipients {
                let scoped = Record::scoped(record.graph_version, record.committed_at, changes);
                match pending.entry(subscriber) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(Pending {
                            started: Instant::now(),
                            lease,
                            record: scoped,
                        });
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let pending = entry.get_mut();
                        let previous = std::mem::replace(&mut pending.record, scoped);
                        let merged =
                            coalesce(&runtime.config.graph, &[previous, pending.record.clone()]);
                        pending.record.changes = merged.changes;
                        pending.lease = lease;
                    }
                }
            }
        }
        let due = pending
            .iter()
            .filter(|(_, p)| p.started.elapsed() >= DEFAULT_COALESCE_WINDOW)
            .map(|(s, _)| s.clone())
            .collect::<Vec<_>>();
        for subscriber in due {
            if let Some(batch) = pending.remove(&subscriber) {
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    deliver(
                        &runtime,
                        &batch.lease,
                        &coalesce(&runtime.config.graph, &[batch.record]),
                    )
                    .await;
                });
            }
        }
        if daily.elapsed() >= Duration::from_secs(86400) {
            let _ = wake_subscriber(&state, &runtime).await;
            daily = Instant::now();
        }
    }
}
async fn deliver(runtime: &Runtime, lease: &Lease, diff: &Diff) {
    let Ok(bytes) = serde_json::to_vec(diff) else {
        return;
    };
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    if gzip.write_all(&bytes).is_err() {
        return;
    }
    let Ok(compressed) = gzip.finish() else {
        return;
    };
    let pushed = if let Some(endpoint) = &lease.endpoint {
        runtime
            .client
            .post(format!("{}/zega/sync/push", endpoint.trim_end_matches('/')))
            .header("content-type", "application/json")
            .header("content-encoding", "gzip")
            .bearer_auth(runtime.token(&lease.subscriber))
            .timeout(DEFAULT_PUSH_TIMEOUT)
            .body(compressed)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    } else {
        false
    };
    if !pushed {
        if let (Some(mailbox), Some(secret)) = (&runtime.config.mailbox, &runtime.secret) {
            let Ok(url) = mailbox_url(mailbox, &lease.subscriber, Some(diff.graph_version)) else {
                return;
            };
            let _ = runtime
                .client
                .put(url)
                .bearer_auth(secret)
                .json(diff)
                .send()
                .await;
        }
    }
}
