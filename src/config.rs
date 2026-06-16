use std::{env, net::Ipv4Addr, time::Duration};

#[derive(Clone)]
pub struct AppConfig {
    pub host: Ipv4Addr,
    pub port: u16,
    pub allowed_origin: String,
    pub request_timeout: Duration,
    pub log_filter: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            host: read_host("APP_HOST").unwrap_or_else(|| Ipv4Addr::new(127, 0, 0, 1)),
            port: read_port("APP_PORT").unwrap_or(3000),
            allowed_origin: read_string("APP_ALLOWED_ORIGIN")
                .unwrap_or_else(|| "*".to_string()),
            request_timeout: Duration::from_secs(read_u64("APP_REQUEST_TIMEOUT_SECS").unwrap_or(30)),
            log_filter: read_string("APP_LOG_FILTER").unwrap_or_else(|| "info".to_string()),
        }
    }
}

fn read_host(key: &str) -> Option<Ipv4Addr> {
    env::var(key).ok()?.parse().ok()
}

fn read_port(key: &str) -> Option<u16> {
    env::var(key).ok()?.parse().ok()
}

fn read_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.parse().ok()
}

fn read_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}
