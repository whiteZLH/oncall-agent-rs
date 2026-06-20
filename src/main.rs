use oncall_agent_rs::{app, config::AppConfig};
use std::net::SocketAddr;
use tracing::info;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env().unwrap_or_else(|error| panic!("加载配置失败: {}", error));
    init_tracing(&config);

    let app = app::build_router(config.clone());
    let addr = SocketAddr::from((config.host, config.port));

    info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("绑定监听地址 {} 失败: {}", addr, error));
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("服务异常退出: {}", error));
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
