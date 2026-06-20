use std::{env, net::Ipv4Addr, time::Duration};

#[derive(Clone)]
pub struct AppConfig {
    pub host: Ipv4Addr,
    pub port: u16,
    pub allowed_origin: String,
    pub request_timeout: Duration,
    pub log_filter: String,
    pub static_dir: String,
    pub redis_url: Option<String>,
    pub chat_history_path: String,
    pub session_ttl_secs: u64,
    pub dashscope_api_key: Option<String>,
    pub dashscope_base_url: String,
    pub dashscope_api_base_url: String,
    pub dashscope_responses_rectifier_enabled: bool,
    pub dashscope_chat_model: String,
    pub chat_agent_max_turns: usize,
    pub dashscope_embedding_model: String,
    pub dashscope_rerank_model: String,
    pub dashscope_rerank_url: String,
    pub milvus_host: String,
    pub milvus_port: u16,
    pub milvus_username: String,
    pub milvus_password: String,
    pub milvus_database: String,
    pub milvus_timeout_ms: u64,
    pub rag_candidate_k: usize,
    pub rag_search_ef: usize,
    pub upload_path: String,
    pub upload_allowed_extensions: Vec<String>,
    pub document_chunk_max_size: usize,
    pub document_chunk_overlap: usize,
    pub private_memory_recall_enabled: bool,
    pub private_memory_recall_top_k: usize,
    pub private_memory_store_path: String,
    pub prometheus_base_url: String,
    pub prometheus_timeout_secs: u64,
    pub prometheus_mock_enabled: bool,
    pub cls_mock_enabled: bool,
    pub ai_ops_chat_model: String,
    pub ai_ops_agent_max_turns: usize,
    pub ai_ops_max_rounds: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let host = read_host("APP_HOST")?.unwrap_or_else(|| Ipv4Addr::new(127, 0, 0, 1));
        let port = read_port("APP_PORT")?.unwrap_or(3000);
        let allowed_origin = read_string("APP_ALLOWED_ORIGIN")?.unwrap_or_else(|| "*".to_string());
        let request_timeout_secs = read_u64("APP_REQUEST_TIMEOUT_SECS")?.unwrap_or(30);
        let log_filter = read_string("APP_LOG_FILTER")?.unwrap_or_else(|| "info".to_string());
        let static_dir = read_string("APP_STATIC_DIR")?.unwrap_or_else(|| "./static".to_string());
        let redis_url = read_string("APP_REDIS_URL")?;
        let chat_history_path = read_string("APP_CHAT_HISTORY_PATH")?
            .unwrap_or_else(|| "./data/chat-history".to_string());
        let session_ttl_secs = read_u64("APP_SESSION_TTL_SECS")?.unwrap_or(3600);
        let dashscope_api_key = read_string("DASHSCOPE_API_KEY")?;
        let dashscope_base_url = read_string("DASHSCOPE_BASE_URL")?
            .unwrap_or_else(|| "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string());
        let dashscope_api_base_url = read_string("DASHSCOPE_API_BASE_URL")?
            .unwrap_or_else(|| "https://dashscope.aliyuncs.com/api/v1".to_string());
        let dashscope_responses_rectifier_enabled =
            read_bool("DASHSCOPE_RESPONSES_RECTIFIER_ENABLED")?.unwrap_or(false);
        let dashscope_chat_model =
            read_string("DASHSCOPE_CHAT_MODEL")?.unwrap_or_else(|| "qwen-plus".to_string());
        let chat_agent_max_turns = read_usize("APP_CHAT_AGENT_MAX_TURNS")?.unwrap_or(6);
        let dashscope_embedding_model = read_string("DASHSCOPE_EMBEDDING_MODEL")?
            .unwrap_or_else(|| "text-embedding-v4".to_string());
        let dashscope_rerank_model =
            read_string("DASHSCOPE_RERANK_MODEL")?.unwrap_or_else(|| "gte-rerank".to_string());
        let dashscope_rerank_url = read_string("DASHSCOPE_RERANK_URL")?.unwrap_or_else(|| {
            "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                .to_string()
        });
        let milvus_host = read_string("MILVUS_HOST")?.unwrap_or_else(|| "localhost".to_string());
        let milvus_port = read_port("MILVUS_PORT")?.unwrap_or(19530);
        let milvus_username = read_string("MILVUS_USERNAME")?.unwrap_or_default();
        let milvus_password = read_string("MILVUS_PASSWORD")?.unwrap_or_default();
        let milvus_database =
            read_string("MILVUS_DATABASE")?.unwrap_or_else(|| "default".to_string());
        let milvus_timeout_ms = read_u64("MILVUS_TIMEOUT_MS")?.unwrap_or(10_000);
        let rag_candidate_k = read_usize("RAG_CANDIDATE_K")?.unwrap_or(10);
        let rag_search_ef = read_usize("RAG_SEARCH_EF")?.unwrap_or(64);
        let upload_path =
            read_string("FILE_UPLOAD_PATH")?.unwrap_or_else(|| "./uploads".to_string());
        let upload_allowed_extensions = read_string("FILE_UPLOAD_ALLOWED_EXTENSIONS")?
            .unwrap_or_else(|| "txt,md".to_string())
            .split(',')
            .map(|item| item.trim().to_ascii_lowercase())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        let document_chunk_max_size = read_usize("DOCUMENT_CHUNK_MAX_SIZE")?.unwrap_or(800);
        let document_chunk_overlap = read_usize("DOCUMENT_CHUNK_OVERLAP")?.unwrap_or(100);
        let private_memory_recall_enabled =
            read_bool("APP_PRIVATE_MEMORY_RECALL_ENABLED")?.unwrap_or(true);
        let private_memory_recall_top_k =
            read_usize("APP_PRIVATE_MEMORY_RECALL_TOP_K")?.unwrap_or(3);
        let private_memory_store_path = read_string("APP_PRIVATE_MEMORY_STORE_PATH")?
            .unwrap_or_else(|| "./data/private-memories".to_string());
        let prometheus_base_url = read_string("PROMETHEUS_BASE_URL")?
            .unwrap_or_else(|| "http://localhost:9090".to_string());
        let prometheus_timeout_secs = read_u64("PROMETHEUS_TIMEOUT")?.unwrap_or(10);
        let prometheus_mock_enabled = read_bool("PROMETHEUS_MOCK_ENABLED")?.unwrap_or(false);
        let cls_mock_enabled = read_bool("CLS_MOCK_ENABLED")?.unwrap_or(false);
        let ai_ops_chat_model =
            read_string("DASHSCOPE_AI_OPS_CHAT_MODEL")?.unwrap_or_else(|| dashscope_chat_model.clone());
        let ai_ops_agent_max_turns = read_usize("APP_AI_OPS_AGENT_MAX_TURNS")?.unwrap_or(12);
        let ai_ops_max_rounds = read_usize("APP_AI_OPS_MAX_ROUNDS")?.unwrap_or(8);

        if request_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_REQUEST_TIMEOUT_SECS 必须大于 0".to_string(),
            ));
        }
        if session_ttl_secs == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_SESSION_TTL_SECS 必须大于 0".to_string(),
            ));
        }
        if private_memory_recall_top_k == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_PRIVATE_MEMORY_RECALL_TOP_K 必须大于 0".to_string(),
            ));
        }
        if chat_agent_max_turns == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_CHAT_AGENT_MAX_TURNS 必须大于 0".to_string(),
            ));
        }
        if ai_ops_agent_max_turns == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_AI_OPS_AGENT_MAX_TURNS 必须大于 0".to_string(),
            ));
        }
        if ai_ops_max_rounds == 0 {
            return Err(ConfigError::InvalidValue(
                "APP_AI_OPS_MAX_ROUNDS 必须大于 0".to_string(),
            ));
        }
        if milvus_timeout_ms == 0 {
            return Err(ConfigError::InvalidValue(
                "MILVUS_TIMEOUT_MS 必须大于 0".to_string(),
            ));
        }
        if rag_candidate_k == 0 {
            return Err(ConfigError::InvalidValue(
                "RAG_CANDIDATE_K 必须大于 0".to_string(),
            ));
        }
        if rag_search_ef == 0 {
            return Err(ConfigError::InvalidValue(
                "RAG_SEARCH_EF 必须大于 0".to_string(),
            ));
        }
        if upload_allowed_extensions.is_empty() {
            return Err(ConfigError::InvalidValue(
                "FILE_UPLOAD_ALLOWED_EXTENSIONS 不能为空".to_string(),
            ));
        }
        if document_chunk_max_size == 0 {
            return Err(ConfigError::InvalidValue(
                "DOCUMENT_CHUNK_MAX_SIZE 必须大于 0".to_string(),
            ));
        }

        Ok(Self {
            host,
            port,
            allowed_origin,
            request_timeout: Duration::from_secs(request_timeout_secs),
            log_filter,
            static_dir,
            redis_url,
            chat_history_path,
            session_ttl_secs,
            dashscope_api_key,
            dashscope_base_url,
            dashscope_api_base_url,
            dashscope_responses_rectifier_enabled,
            dashscope_chat_model,
            chat_agent_max_turns,
            dashscope_embedding_model,
            dashscope_rerank_model,
            dashscope_rerank_url,
            milvus_host,
            milvus_port,
            milvus_username,
            milvus_password,
            milvus_database,
            milvus_timeout_ms,
            rag_candidate_k,
            rag_search_ef,
            upload_path,
            upload_allowed_extensions,
            document_chunk_max_size,
            document_chunk_overlap,
            private_memory_recall_enabled,
            private_memory_recall_top_k,
            private_memory_store_path,
            prometheus_base_url,
            prometheus_timeout_secs,
            prometheus_mock_enabled,
            cls_mock_enabled,
            ai_ops_chat_model,
            ai_ops_agent_max_turns,
            ai_ops_max_rounds,
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

fn read_bool(key: &str) -> Result<Option<bool>, ConfigError> {
    match env::var(key) {
        Ok(value) => {
            let trimmed = value.trim().to_ascii_lowercase();
            match trimmed.as_str() {
                "true" | "1" | "yes" | "y" | "on" => Ok(Some(true)),
                "false" | "0" | "no" | "n" | "off" => Ok(Some(false)),
                _ => Err(ConfigError::InvalidValue(format!("{key} 必须是布尔值"))),
            }
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{key} 必须是合法的 Unicode 字符串"
        ))),
    }
}

fn read_usize(key: &str) -> Result<Option<usize>, ConfigError> {
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
