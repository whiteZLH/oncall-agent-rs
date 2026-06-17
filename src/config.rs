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
    pub fn from_env() -> Result<Self, ConfigError> {
        let host = read_host("APP_HOST")?.unwrap_or_else(|| Ipv4Addr::new(127, 0, 0, 1));
        let port = read_port("APP_PORT")?.unwrap_or(3000);
        let allowed_origin = read_string("APP_ALLOWED_ORIGIN")?.unwrap_or_else(|| "*".to_string());
        let request_timeout_secs = read_u64("APP_REQUEST_TIMEOUT_SECS")?.unwrap_or(30);
        let log_filter = read_string("APP_LOG_FILTER")?.unwrap_or_else(|| "info".to_string());

        if request_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_REQUEST_TIMEOUT_SECS 必须大于 0".to_string(),
            ));
        }

        Ok(Self {
            host,
            port,
            allowed_origin,
            request_timeout: Duration::from_secs(request_timeout_secs),
            log_filter,
        })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidValue(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn read_host(key: &str) -> Result<Option<Ipv4Addr>, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::InvalidValue(format!("{key} 必须是合法的 IPv4 地址"))),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{key} 必须是合法的 Unicode 字符串"
        ))),
    }
}

fn read_port(key: &str) -> Result<Option<u16>, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::InvalidValue(format!("{key} 必须是合法的端口号"))),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{key} 必须是合法的 Unicode 字符串"
        ))),
    }
}

fn read_u64(key: &str) -> Result<Option<u64>, ConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::InvalidValue(format!("{key} 必须是正整数"))),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{key} 必须是合法的 Unicode 字符串"
        ))),
    }
}

fn read_string(key: &str) -> Result<Option<String>, ConfigError> {
    let value = match env::var(key) {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(ConfigError::InvalidValue(format!(
                "{key} 必须是合法的 Unicode 字符串"
            )));
        }
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed.to_string()))
}
