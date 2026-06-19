use crate::{
    config::AppConfig,
    domain::chat::{ChatMessage, ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    services::memory_extraction_service::MemoryExtractionService,
};
use redis::Commands;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};
use uuid::Uuid;

const MAX_WINDOW_SIZE: usize = 6;

pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionInfo>>,
    redis_url: Option<String>,
    chat_history_path: PathBuf,
    session_ttl_secs: u64,
    memory_extraction_service: MemoryExtractionService,
}

impl SessionManager {
    pub fn new(config: &AppConfig) -> Self {
        let seed = ChatSessionRecord {
            session_id: "session-1".to_string(),
            create_time: 1_718_559_600_000,
            update_time: 1_718_559_960_000,
            message_history: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: "继续上次的话题".to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "这是 Rust 迁移服务的会话占位回复。".to_string(),
                },
            ],
        };

        Self {
            sessions: Mutex::new(HashMap::from([(
                seed.session_id.clone(),
                SessionInfo::from_record(seed),
            )])),
            redis_url: config.redis_url.clone(),
            chat_history_path: PathBuf::from(&config.chat_history_path),
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

    pub fn clear(&self, session_id: &str) -> ClearResult {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return ClearResult::MissingSessionId;
        }

        let mut sessions = self.sessions.lock().expect("会话存储锁已损坏");
        let Some(session) = sessions.get_mut(trimmed) else {
            return ClearResult::NotFound;
        };

        session.message_history.clear();
        session.update_time = now_millis();
        drop(sessions);
        self.delete_persisted_history(trimmed);
        ClearResult::Cleared
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
        let mut sessions = self
            .sessions
            .lock()
            .expect("会话存储锁已损坏")
            .values()
            .map(|session| ChatSessionSummary {
                session_id: session_id(session).to_string(),
                title: session_title(&session.as_record()),
                message_pair_count: message_history(session).len() / 2,
                create_time: create_time(session),
                update_time: update_time(session),
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.update_time.cmp(&left.update_time));
        sessions
    }

    pub fn session_messages(&self, session_id: &str) -> Option<ChatSessionRecord> {
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
        self.delete_persisted_history(session_id);
        removed
    }

    fn get_session(&self, session_id: &str) -> Option<SessionInfo> {
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
        let session: SessionInfo = serde_json::from_str(&json).ok()?;
        Some(session)
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
        let Ok(json) = serde_json::to_string(session) else {
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
        let path = self.chat_history_path.join(format!("{session_id}.json"));
        let content = fs::read_to_string(path).ok()?;
        let mut record: ChatSessionRecord = serde_json::from_str(&content).ok()?;
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
        if fs::create_dir_all(&self.chat_history_path).is_err() {
            return;
        }

        let path = self.chat_history_path.join(format!("{session_id}.json"));
        let mut record = self.load_from_history_store(session_id).unwrap_or_else(|| {
            SessionInfo::with_parts(session_id.to_string(), create_time, Vec::new())
        });

        record.message_history.push(ChatMessage {
            role: "user".to_string(),
            content: question.to_string(),
        });
        record.message_history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer.to_string(),
        });
        record.update_time = now_millis();

        let Ok(json) = serde_json::to_string_pretty(&record) else {
            return;
        };
        let _ = fs::write(path, json);
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
        if let Some(redis_url) = self.redis_url.as_ref() {
            if let Ok(client) = redis::Client::open(redis_url.as_str()) {
                if let Ok(mut connection) = client.get_connection() {
                    let _: redis::RedisResult<()> = redis::cmd("DEL")
                        .arg(redis_key(session_id))
                        .query(&mut connection);
                }
            }
        }

        let path = self.chat_history_path.join(format!("{session_id}.json"));
        let _ = fs::remove_file(path);
    }
}

pub enum ClearResult {
    Cleared,
    MissingSessionId,
    NotFound,
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

fn session_id(session: &SessionInfo) -> &str {
    &session.session_id
}

fn message_history(session: &SessionInfo) -> &[ChatMessage] {
    &session.message_history
}

fn create_time(session: &SessionInfo) -> i64 {
    session.create_time
}

fn update_time(session: &SessionInfo) -> i64 {
    session.update_time
}

fn session_title(session: &ChatSessionRecord) -> String {
    session
        .message_history
        .iter()
        .find(|message| message.role == "user")
        .map(|message| {
            let content = message.content.trim();
            if content.chars().count() > 30 {
                format!("{}...", content.chars().take(30).collect::<String>())
            } else {
                content.to_string()
            }
        })
        .unwrap_or_else(|| "新对话".to_string())
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
