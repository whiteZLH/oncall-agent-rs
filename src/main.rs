mod app;
mod config;
mod error;
mod models;
mod routes;
mod services;
mod state;

use crate::config::AppConfig;
use std::net::SocketAddr;
use tracing::info;

#[tokio::main]
async fn main() {
    let config = AppConfig::from_env();
    init_tracing(&config);

    let app = app::build_router(config.clone());
    let addr = SocketAddr::from((config.host, config.port));

    info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {}: {}", addr, error));
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("server exited with error: {}", error));
}

fn init_tracing(config: &AppConfig) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_filter)),
        )
        .with_target(false)
        .compact()
        .init();
}
