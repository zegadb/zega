//! Headless linked-graph host. Configuration contains URLs and identities;
//! SOURCE_SECRET is supplied only by the deployment environment.
use serde::Deserialize;
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    data: String,
    listen: std::net::SocketAddr,
    linked: zega_server::sync::Config,
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: zega-linked-host <config.json>")?;
    let host: Host = serde_json::from_reader(std::fs::File::open(path)?)?;
    // This host exposes public sources only. The existing authenticated host
    // remains available through `zega-server start` for private graphs.
    if !host.listen.ip().is_loopback() {
        return Err(
            "linked-graph host requires a loopback listener behind an authenticated gateway".into(),
        );
    }
    let runtime =
        zega_server::sync::Runtime::new(host.linked, std::env::var("SOURCE_SECRET").ok())?;
    let db = zega::Zega::open(&host.data).build()?;
    let mut state = zega_server::AppState::new(db, None);
    state.sync = Some(Arc::new(runtime));
    let listener = tokio::net::TcpListener::bind(host.listen).await?;
    let worker = zega_server::sync::start(state.clone()).await?;
    let result = zega_server::server::serve(listener, state).await;
    worker.abort();
    result?;
    Ok(())
}
