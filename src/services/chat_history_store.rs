use crate::domain::chat::{ChatMessage, ChatSessionRecord, ChatSessionSummary};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::warn;
use uuid::Uuid;

const TITLE_MAX_LENGTH: usize = 30;

#[derive(Clone)]
pub struct ChatHistoryStore {
    root_path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl ChatHistoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let root_path = path.into();
        let root_path = if root_path.as_os_str().is_empty() {
            PathBuf::from("./data/chat-history")
        } else {
            root_path
        };

        Self {
            root_path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn append_message_pair(
        &self,
        session_id: &str,
        create_time: i64,
        user_question: &str,
        ai_answer: &str,
    ) -> bool {
        let _guard = self.lock.lock().expect("聊天历史存储锁已损坏");
        let now = now_millis();
        let mut record = self
            .load_unlocked(session_id)
            .unwrap_or_else(|| new_record(session_id, create_time, now));

        record.message_history.push(ChatMessage {
            role: "user".to_string(),
            content: user_question.to_string(),
        });
        record.message_history.push(ChatMessage {
            role: "assistant".to_string(),
            content: ai_answer.to_string(),
        });
        record.update_time = now;

        self.save_unlocked(record)
    }

    pub fn save(&self, record: ChatSessionRecord) -> bool {
        let _guard = self.lock.lock().expect("聊天历史存储锁已损坏");
        self.save_unlocked(record)
    }

    pub fn load(&self, session_id: &str) -> Option<ChatSessionRecord> {
        if session_id.trim().is_empty() {
            return None;
        }

        let _guard = self.lock.lock().expect("聊天历史存储锁已损坏");
        self.load_unlocked(session_id)
    }

    pub fn list_sessions(&self) -> Vec<ChatSessionSummary> {
        let _guard = self.lock.lock().expect("聊天历史存储锁已损坏");
        let Ok(entries) = fs::read_dir(&self.root_path) else {
            return Vec::new();
        };

        let mut summaries = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|path| self.read_record(&path))
            .map(|record| ChatSessionSummary {
                session_id: record.session_id.clone(),
                title: title_for(&record),
                message_pair_count: record.message_history.len() / 2,
                create_time: record.create_time,
                update_time: record.update_time,
            })
            .collect::<Vec<_>>();

        summaries.sort_by(|left, right| right.update_time.cmp(&left.update_time));
        summaries
    }

    pub fn delete(&self, session_id: &str) -> bool {
        if session_id.trim().is_empty() {
            return false;
        }

        let _guard = self.lock.lock().expect("聊天历史存储锁已损坏");
        match fs::remove_file(self.path_for_session(session_id)) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                warn!("删除聊天历史失败: {}", error);
                false
            }
        }
    }

    fn save_unlocked(&self, mut record: ChatSessionRecord) -> bool {
        if record.session_id.trim().is_empty() {
            warn!("保存聊天历史失败: sessionId 不能为空");
            return false;
        }

        let now = now_millis();
        if record.create_time <= 0 {
            record.create_time = now;
        }
        if record.update_time <= 0 {
            record.update_time = now;
        }

        if let Err(error) = fs::create_dir_all(&self.root_path) {
            warn!("保存聊天历史失败: {}", error);
            return false;
        }

        let Ok(json) = serde_json::to_string_pretty(&record) else {
            warn!("序列化聊天历史失败: {}", record.session_id);
            return false;
        };

        let target = self.path_for_session(&record.session_id);
        let temp = self
            .root_path
            .join(format!("chat-history-{}.tmp", Uuid::new_v4()));
        if let Err(error) = fs::write(&temp, json) {
            warn!("写入聊天历史临时文件失败: {}", error);
            return false;
        }

        if let Err(error) = move_replacing(&temp, &target) {
            warn!("保存聊天历史失败: {}", error);
            let _ = fs::remove_file(temp);
            return false;
        }

        true
    }

    fn load_unlocked(&self, session_id: &str) -> Option<ChatSessionRecord> {
        if session_id.trim().is_empty() {
            return None;
        }

        let path = self.path_for_session(session_id);
        if !path.exists() {
            return None;
        }
        self.read_record(&path)
    }

    fn read_record(&self, path: &Path) -> Option<ChatSessionRecord> {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(record) => Some(record),
                Err(error) => {
                    warn!("读取聊天历史文件失败: {}, {}", path.display(), error);
                    None
                }
            },
            Err(error) => {
                warn!("读取聊天历史文件失败: {}, {}", path.display(), error);
                None
            }
        }
    }

    fn path_for_session(&self, session_id: &str) -> PathBuf {
        let encoded = URL_SAFE_NO_PAD.encode(session_id.as_bytes());
        self.root_path.join(format!("{encoded}.json"))
    }
}

fn new_record(session_id: &str, create_time: i64, update_time: i64) -> ChatSessionRecord {
    ChatSessionRecord {
        session_id: session_id.to_string(),
        create_time: if create_time > 0 {
            create_time
        } else {
            update_time
        },
        update_time,
        message_history: Vec::new(),
    }
}

fn title_for(record: &ChatSessionRecord) -> String {
    record
        .message_history
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.content.trim())
        .find(|content| !content.is_empty())
        .map(|content| {
            if content.chars().count() > TITLE_MAX_LENGTH {
                format!(
                    "{}...",
                    content.chars().take(TITLE_MAX_LENGTH).collect::<String>()
                )
            } else {
                content.to_string()
            }
        })
        .unwrap_or_else(|| "新对话".to_string())
}

fn move_replacing(temp: &Path, target: &Path) -> io::Result<()> {
    match fs::rename(temp, target) {
        Ok(()) => Ok(()),
        Err(error) if target.exists() => {
            fs::remove_file(target)?;
            fs::rename(temp, target).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::ChatHistoryStore;
    use crate::domain::chat::{ChatMessage, ChatSessionRecord};
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn append_message_pair_should_persist_and_reload_complete_history() {
        let store = ChatHistoryStore::new(unique_path("append"));

        assert!(store.append_message_pair("session/with/path", 100, "hello", "hi"));
        assert!(store.append_message_pair("session/with/path", 100, "second", "answer"));

        let record = store.load("session/with/path").expect("历史应已写入");
        assert_eq!(record.session_id, "session/with/path");
        assert_eq!(record.create_time, 100);
        assert_eq!(record.message_history.len(), 4);
        assert_eq!(record.message_history[0].content, "hello");
        assert_eq!(record.message_history[3].content, "answer");
    }

    #[test]
    fn list_sessions_should_return_summaries_sorted_by_update_time_desc() {
        let store = ChatHistoryStore::new(unique_path("list"));
        assert!(store.save(record("older", 100, 200, "old question")));
        assert!(store.save(record("newer", 100, 300, "1234567890123456789012345678901")));

        let summaries = store.list_sessions();

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].session_id, "newer");
        assert_eq!(summaries[0].title, "123456789012345678901234567890...");
        assert_eq!(summaries[0].message_pair_count, 1);
        assert_eq!(summaries[1].session_id, "older");
    }

    #[test]
    fn delete_should_remove_persisted_session() {
        let store = ChatHistoryStore::new(unique_path("delete"));
        assert!(store.append_message_pair("to-delete", 100, "hello", "hi"));
        assert!(store.load("to-delete").is_some());

        assert!(store.delete("to-delete"));
        assert!(store.load("to-delete").is_none());
        assert!(!store.delete(""));
    }

    #[test]
    fn list_sessions_title_should_skip_blank_user_messages() {
        let store = ChatHistoryStore::new(unique_path("blank-title"));
        assert!(store.save(ChatSessionRecord {
            session_id: "blank-title".to_string(),
            create_time: 100,
            update_time: 200,
            message_history: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: "   ".to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "blank".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "real question".to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "answer".to_string(),
                },
            ],
        }));

        let summaries = store.list_sessions();

        assert_eq!(summaries[0].title, "real question");
    }

    fn record(
        session_id: &str,
        create_time: i64,
        update_time: i64,
        first_question: &str,
    ) -> ChatSessionRecord {
        ChatSessionRecord {
            session_id: session_id.to_string(),
            create_time,
            update_time,
            message_history: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: first_question.to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "answer".to_string(),
                },
            ],
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间早于 Unix 纪元")
            .as_nanos();
        PathBuf::from(format!("./target/test-chat-history-store-{name}-{suffix}"))
    }
}
