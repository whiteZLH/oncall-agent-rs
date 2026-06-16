use std::sync::Arc;

use axum::{response::IntoResponse, routing::get, Router};

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/metrics", get(metrics))
}

async fn metrics() -> impl IntoResponse {
    concat!(
        "# HELP oncall_agent_up Whether the service is running.\n",
        "# TYPE oncall_agent_up gauge\n",
        "oncall_agent_up 1\n"
    )
}
