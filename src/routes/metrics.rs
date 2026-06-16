use std::sync::Arc;

use axum::{response::IntoResponse, routing::get, Router};

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/metrics", get(metrics))
}

async fn metrics() -> impl IntoResponse {
    // 先暴露一份最小可用的 Prometheus 文本，后续再接真实 registry。
    concat!(
        "# HELP oncall_agent_up Whether the service is running.\n",
        "# TYPE oncall_agent_up gauge\n",
        "oncall_agent_up 1\n"
    )
}
