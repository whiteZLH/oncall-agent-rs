use crate::{
    config::AppConfig,
    domain::chat::{ChatMessage, ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    services::{
        chat_history_store::ChatHistoryStore, memory_extraction_service::MemoryExtractionService,
    },
};
use redis::Commands;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};
use uuid::Uuid;

const MAX_WINDOW_SIZE: usize = 6;

pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionInfo>>,
    redis_url: Option<String>,
    chat_history_store: ChatHistoryStore,
    session_ttl_secs: u64,
    memory_extraction_service: MemoryExtractionService,
}

impl SessionManager {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            redis_url: config.redis_url.clone(),
            chat_history_store: ChatHistoryStore::new(config.chat_history_path.clone()),
            session_ttl_secs: config.session_ttl_secs,
            memory_extraction_service: MemoryExtractionService::new(config),
        }
    }

    pub(crate) fn get_or_create_session(&self, session_id: Option<&str>) -> SessionInfo {
        let session_id = session_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let mut sessions = self.sessions.lock().expect("会话存储锁已损坏");
        if let Some(existing) = sessions.get(&session_id) {
            self.refresh_session_ttl(&session_id);
            return existing.clone();
        }

        if let Some(from_redis) = self.load_from_redis(&session_id) {
            sessions.insert(session_id.clone(), from_redis.clone());
            return from_redis;
        }

        if let Some(from_history_store) = self.load_from_history_store(&session_id) {
            sessions.insert(session_id.clone(), from_history_store.clone());
            self.save_to_redis(&from_history_store);
            return from_history_store;
        }

        let session = SessionInfo::new(session_id.clone());
        sessions.insert(session_id, session.clone());
        self.save_to_redis(&session);
        session
    }

    pub fn save_session(&self, session: &SessionInfo) {
        self.sessions
            .lock()
            .expect("会话存储锁已损坏")
            .insert(session.session_id.clone(), session.clone());
        self.save_to_redis(session);
    }

    pub fn session_info(&self, session_id: &str) -> Option<SessionInfoResponse> {
        let session = self.get_session(session_id)?;
        Some(SessionInfoResponse {
            session_id: session.session_id.clone(),
            message_pair_count: session.message_history.len() / 2,
            create_time: session.create_time,
        })
    }

    pub fn list_sessions(&self) -> Vec<ChatSessionSummary> {
        self.chat_history_store.list_sessions()
    }

    pub fn session_messages(&self, session_id: &str) -> Option<ChatSessionRecord> {
        if let Some(record) = self.chat_history_store.load(session_id) {
            return Some(record);
        }

        let session = self.get_session(session_id)?;
        Some(session.as_record())
    }

    pub fn delete_session(&self, session_id: &str) -> bool {
        if session_id.trim().is_empty() {
            return false;
        }
        let removed = self
            .sessions
            .lock()
            .expect("会话存储锁已损坏")
            .remove(session_id)
            .is_some();
        self.delete_from_redis(session_id);
        let removed_from_history = self.chat_history_store.delete(session_id);
        removed || removed_from_history
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Option<SessionInfo> {
        if session_id.trim().is_empty() {
            return None;
        }

        if let Some(existing) = self
            .sessions
            .lock()
            .expect("会话存储锁已损坏")
            .get(session_id)
            .cloned()
        {
            self.refresh_session_ttl(session_id);
            return Some(existing);
        }

        if let Some(from_redis) = self.load_from_redis(session_id) {
            self.sessions
                .lock()
                .expect("会话存储锁已损坏")
                .insert(session_id.to_string(), from_redis.clone());
            return Some(from_redis);
        }

        if let Some(from_history_store) = self.load_from_history_store(session_id) {
            self.sessions
                .lock()
                .expect("会话存储锁已损坏")
                .insert(session_id.to_string(), from_history_store.clone());
            self.save_to_redis(&from_history_store);
            return Some(from_history_store);
        }

        None
    }

    fn load_from_redis(&self, session_id: &str) -> Option<SessionInfo> {
        let redis_url = self.redis_url.as_ref()?;
        let client = redis::Client::open(redis_url.as_str()).ok()?;
        let mut connection = client.get_connection().ok()?;
        let key = redis_key(session_id);
        let json: String = connection.get(&key).ok()?;
        let data: RedisSessionData = serde_json::from_str(&json).ok()?;
        Some(SessionInfo::with_parts(
            session_id.to_string(),
            if data.create_time > 0 {
                data.create_time
            } else {
                now_millis()
            },
            recent_window(data.message_history),
        ))
    }

    fn save_to_redis(&self, session: &SessionInfo) {
        let Some(redis_url) = self.redis_url.as_ref() else {
            return;
        };
        let Ok(client) = redis::Client::open(redis_url.as_str()) else {
            return;
        };
        let Ok(mut connection) = client.get_connection() else {
            return;
        };
        let Ok(json) = serde_json::to_string(&RedisSessionData::from_session(session)) else {
            return;
        };

        let key = redis_key(&session.session_id);
        let ttl = self.session_ttl_secs as usize;
        let _: redis::RedisResult<()> = redis::cmd("SET")
            .arg(&key)
            .arg(json)
            .arg("EX")
            .arg(ttl)
            .query(&mut connection);
    }

    fn refresh_session_ttl(&self, session_id: &str) {
        if self.redis_url.is_none() {
            return;
        }
        if let Some(redis_url) = self.redis_url.as_ref() {
            if let Ok(client) = redis::Client::open(redis_url.as_str()) {
                if let Ok(mut connection) = client.get_connection() {
                    let _: redis::RedisResult<()> = redis::cmd("EXPIRE")
                        .arg(redis_key(session_id))
                        .arg(self.session_ttl_secs as usize)
                        .query(&mut connection);
                }
            }
        }
    }

    fn load_from_history_store(&self, session_id: &str) -> Option<SessionInfo> {
        let mut record = self.chat_history_store.load(session_id)?;
        record.message_history = recent_window(record.message_history);
        Some(SessionInfo::from_record(record))
    }

    fn append_to_history_store(
        &self,
        session_id: &str,
        create_time: i64,
        question: &str,
        answer: &str,
    ) {
        if !self
            .chat_history_store
            .append_message_pair(session_id, create_time, question, answer)
        {
            warn!("保存完整聊天历史失败: {}", session_id);
        }
    }

    // 触发异步的记忆提炼
    pub fn trigger_memory_extraction(
        &self,
        session_id: &str,
        history_to_archive: Vec<ChatMessage>,
    ) {
        if history_to_archive.is_empty() {
            return;
        }

        let memory_extraction_service = self.memory_extraction_service.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = memory_extraction_service
                .extract_and_store(&session_id, &history_to_archive)
                .await
            {
                warn!(
                    "提炼长期记忆失败: session_id={}, error={}",
                    session_id, error
                );
            }
        });
    }

    fn delete_persisted_history(&self, session_id: &str) {
        self.delete_from_redis(session_id);
        self.chat_history_store.delete(session_id);
    }

    fn delete_from_redis(&self, session_id: &str) {
        if let Some(redis_url) = self.redis_url.as_ref() {
            let client = match redis::Client::open(redis_url.as_str()) {
                Ok(client) => client,
                Err(error) => {
                    warn!("Failed to delete session from Redis: {}", error);
                    return;
                }
            };
            let mut connection = match client.get_connection() {
                Ok(connection) => connection,
                Err(error) => {
                    warn!("Failed to delete session from Redis: {}", error);
                    return;
                }
            };
            if let Err(error) = redis::cmd("DEL")
                .arg(redis_key(session_id))
                .query::<()>(&mut connection)
            {
                warn!("Failed to delete session from Redis: {}", error);
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RedisSessionData {
    session_id: String,
    create_time: i64,
    #[serde(default)]
    message_history: Vec<ChatMessage>,
}

impl RedisSessionData {
    fn from_session(session: &SessionInfo) -> Self {
        Self {
            session_id: session.session_id.clone(),
            create_time: session.create_time,
            message_history: session.message_history.clone(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    session_id: String,
    create_time: i64,
    update_time: i64,
    message_history: Vec<ChatMessage>,
}

impl SessionInfo {
    fn new(session_id: String) -> Self {
        let now = now_millis();
        Self {
            session_id,
            create_time: now,
            update_time: now,
            message_history: Vec::new(),
        }
    }

    fn with_parts(session_id: String, create_time: i64, message_history: Vec<ChatMessage>) -> Self {
        Self {
            session_id,
            create_time,
            update_time: create_time,
            message_history,
        }
    }

    fn from_record(record: ChatSessionRecord) -> Self {
        let mut session = Self::with_parts(
            record.session_id,
            record.create_time,
            record.message_history,
        );
        session.update_time = record.update_time;
        session
    }

    pub(crate) fn as_record(&self) -> ChatSessionRecord {
        ChatSessionRecord {
            session_id: self.session_id.clone(),
            create_time: self.create_time,
            update_time: self.update_time,
            message_history: self.message_history.clone(),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn get_history(&self) -> Vec<ChatMessage> {
        self.message_history.clone()
    }

    pub fn clear_history(&mut self, manager: &SessionManager) {
        self.message_history.clear();
        self.update_time = now_millis();
        debug!("会话 {} 历史消息已清空", self.session_id);
        manager
            .sessions
            .lock()
            .expect("会话存储锁已损坏")
            .insert(self.session_id.clone(), self.clone());
        manager.delete_persisted_history(&self.session_id);
    }

    pub fn add_message(&mut self, user_question: &str, ai_answer: &str, manager: &SessionManager) {
        let mut evicted_messages = Vec::new();

        // 添加用户消息
        self.message_history.push(ChatMessage {
            role: "user".to_string(),
            content: user_question.to_string(),
        });
        // 添加AI回复
        self.message_history.push(ChatMessage {
            role: "assistant".to_string(),
            content: ai_answer.to_string(),
        });

        // 自动清理：保持最多 MAX_WINDOW_SIZE 对消息
        let max_messages = MAX_WINDOW_SIZE * 2;
        while self.message_history.len() > max_messages {
            evicted_messages.push(self.message_history.remove(0)); // 删除最旧的用户消息
            if !self.message_history.is_empty() {
                evicted_messages.push(self.message_history.remove(0)); // 删除对应的AI回复
            }
        }

        self.update_time = now_millis();
        debug!(
            "会话 {} 更新历史消息，当前消息对数: {}",
            self.session_id,
            self.message_history.len() / 2
        );

        manager.append_to_history_store(
            &self.session_id,
            self.create_time,
            user_question,
            ai_answer,
        );
        manager.save_session(self);

        if !evicted_messages.is_empty() {
            manager.trigger_memory_extraction(&self.session_id, evicted_messages);
        }
    }

    pub fn get_message_pair_count(&self) -> usize {
        self.message_history.len() / 2
    }
}

fn redis_key(session_id: &str) -> String {
    format!("session:{session_id}")
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}

fn recent_window(history: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let max_messages = MAX_WINDOW_SIZE * 2;
    let history_len = history.len();
    if history_len <= max_messages {
        return history;
    }
    history
        .into_iter()
        .skip(history_len - max_messages)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::SessionManager;
    use crate::{
        config::AppConfig,
        domain::chat::{ChatMessage, ChatSessionRecord},
    };
    use std::{
        net::Ipv4Addr,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn fresh_manager_should_not_include_seed_session() {
        let manager = SessionManager::new(&test_config("fresh"));

        assert!(manager.list_sessions().is_empty());
        assert!(manager.get_session("session-1").is_none());
    }

    #[tokio::test]
    async fn should_persist_complete_history_and_restore_recent_window() {
        let config = test_config("persist");
        let manager = SessionManager::new(&config);
        let mut session = manager.get_or_create_session(Some("persist-test"));

        for index in 1..=8 {
            session.add_message(&format!("q{index}"), &format!("a{index}"), &manager);
        }

        assert_eq!(session.get_message_pair_count(), 6);
        let full_record = manager
            .session_messages("persist-test")
            .expect("完整历史应存在");
        assert_eq!(full_record.message_history.len(), 16);
        assert_eq!(full_record.message_history[0].content, "q1");

        let restarted = SessionManager::new(&config);
        let restored = restarted.get_or_create_session(Some("persist-test"));

        assert_eq!(restored.get_message_pair_count(), 6);
        assert_eq!(restored.get_history()[0].content, "q3");
    }

    #[test]
    fn delete_session_should_remove_disk_only_history() {
        let config = test_config("delete-disk-only");
        let manager = SessionManager::new(&config);
        assert!(manager.chat_history_store.save(record("disk-only")));

        assert!(manager.delete_session("disk-only"));
        assert!(manager.chat_history_store.load("disk-only").is_none());
    }

    fn record(session_id: &str) -> ChatSessionRecord {
        ChatSessionRecord {
            session_id: session_id.to_string(),
            create_time: 100,
            update_time: 200,
            message_history: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: "hello".to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "hi".to_string(),
                },
            ],
        }
    }

    fn test_config(name: &str) -> AppConfig {
        AppConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 3000,
            allowed_origin: "*".to_string(),
            request_timeout: Duration::from_secs(30),
            log_filter: "info".to_string(),
            redis_url: None,
            chat_history_path: unique_path(name).display().to_string(),
            session_ttl_secs: 3600,
            dashscope_api_key: None,
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            dashscope_api_base_url: "https://dashscope.aliyuncs.com/api/v1".to_string(),
            dashscope_chat_model: "qwen-plus".to_string(),
            chat_agent_max_turns: 6,
            dashscope_embedding_model: "text-embedding-v4".to_string(),
            dashscope_rerank_model: "gte-rerank".to_string(),
            dashscope_rerank_url:
                "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                    .to_string(),
            milvus_host: "localhost".to_string(),
            milvus_port: 19530,
            milvus_username: String::new(),
            milvus_password: String::new(),
            milvus_database: "default".to_string(),
            milvus_timeout_ms: 10_000,
            rag_candidate_k: 10,
            rag_search_ef: 64,
            upload_path: "./target/uploads".to_string(),
            upload_allowed_extensions: vec!["txt".to_string(), "md".to_string()],
            document_chunk_max_size: 800,
            document_chunk_overlap: 100,
            private_memory_recall_enabled: true,
            private_memory_recall_top_k: 3,
            private_memory_store_path: "./target/test-private-memories".to_string(),
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间早于 Unix 纪元")
            .as_nanos();
        PathBuf::from(format!("./target/test-session-manager-{name}-{suffix}"))
    }
}
