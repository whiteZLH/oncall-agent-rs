use crate::{
    domain::chat::{ChatMessage, ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    http::dto::{ChatRequest, ChatResponse},
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct ChatService {
    sessions: Mutex<HashMap<String, ChatSessionRecord>>,
}

impl ChatService {
    pub fn new() -> Self {
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
            sessions: Mutex::new(HashMap::from([(seed.session_id.clone(), seed)])),
        }
    }

    pub fn reply(&self, input: ChatRequest) -> ChatResponse {
        let Some(question) = input.question.as_deref().map(str::trim) else {
            return ChatResponse::error("问题内容不能为空");
        };

        if question.is_empty() {
            return ChatResponse::error("问题内容不能为空");
        }

        let session_id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or("default-session");
        let answer = format!("oncall-agent-rs received [{}]: {}", session_id, question);

        let mut sessions = self.sessions.lock().expect("会话存储锁已损坏");
        let now = now_millis();
        let session = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| ChatSessionRecord {
                session_id: session_id.to_string(),
                create_time: now,
                update_time: now,
                message_history: Vec::new(),
            });
        session.message_history.push(ChatMessage {
            role: "user".to_string(),
            content: question.to_string(),
        });
        session.message_history.push(ChatMessage {
            role: "assistant".to_string(),
            content: answer.clone(),
        });
        session.update_time = now;

        ChatResponse::success(answer)
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
        ClearResult::Cleared
    }

    pub fn session_info(&self, session_id: &str) -> Option<SessionInfoResponse> {
        self.sessions
            .lock()
            .expect("会话存储锁已损坏")
            .get(session_id)
            .map(|session| SessionInfoResponse {
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
                session_id: session.session_id.clone(),
                title: session_title(session),
                message_pair_count: session.message_history.len() / 2,
                create_time: session.create_time,
                update_time: session.update_time,
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.update_time.cmp(&left.update_time));
        sessions
    }

    pub fn session_messages(&self, session_id: &str) -> Option<ChatSessionRecord> {
        self.sessions
            .lock()
            .expect("会话存储锁已损坏")
            .get(session_id)
            .cloned()
    }

    pub fn delete_session(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .expect("会话存储锁已损坏")
            .remove(session_id)
            .is_some()
    }
}

pub enum ClearResult {
    Cleared,
    MissingSessionId,
    NotFound,
}

fn session_title(session: &ChatSessionRecord) -> String {
    session
        .message_history
        .iter()
        .find(|message| message.role == "user")
        .map(|message| message.content.clone())
        .unwrap_or_else(|| "新会话".to_string())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}
