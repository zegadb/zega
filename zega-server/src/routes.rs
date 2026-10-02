use crate::{handlers, AppState};
use axum::{
    extract::DefaultBodyLimit,
    handler::Handler,
    routing::{delete, get, post},
    Router,
};

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/sync/node/:id", get(crate::sync::node))
        .route("/sync/check", post(crate::sync::check))
        .route("/sync/mirrors", get(crate::sync::mirrors))
        .route("/sync/subscribe", post(crate::sync::subscribe))
        .route("/sync/link", post(crate::sync::link))
        .route("/sync/repair", post(crate::sync::repair))
        .route("/zega/sync/push", post(crate::sync::push))
        .route("/health", get(handlers::health))
        .route("/stats", get(handlers::stats))
        // `POST` is the original; every other method (QUERY, OPTIONS, and a 405 for
        // the rest) goes to `zql_other`.
        .route("/zql", post(handlers::zql).fallback(handlers::zql_other))
        .route("/vector-view", post(handlers::vector_view))
        // A `.graph` upload streams into the engine, so the JSON body limit
        // below does not apply to it.
        .route(
            "/graph",
            get(handlers::graph)
                .put(handlers::import_graph.layer(DefaultBodyLimit::disable()))
                .delete(handlers::clear),
        )
        .route("/graph/nodes/:id", delete(handlers::delete_node))
        .route(
            "/graph/relationships/:id",
            delete(handlers::delete_relationship),
        )
        .route("/graph/relationships", post(handlers::connect))
        .route("/schema/diff", post(handlers::schema_diff))
        // Raw source text needs room for JSON escaping. Per-source limits are
        // still enforced by the engine before parsing and insertion.
        .layer(DefaultBodyLimit::max(16_000_000))
        .with_state(state)
}
