use crate::domain::rag::IndexTaskStatus;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct IndexTaskStatusService {
    statuses: Arc<Mutex<HashMap<String, IndexTaskStatus>>>,
}

impl IndexTaskStatusService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_task(&self, file_name: &str, file_path: &str) -> IndexTaskStatus {
        let now = now_millis();
        let status = IndexTaskStatus {
            task_id: Uuid::new_v4().to_string(),
            file_name: file_name.to_string(),
            file_path: file_path.to_string(),
            status: "INDEXING".to_string(),
            message: "文件已接收，索引处理中".to_string(),
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        self.statuses
            .lock()
            .expect("索引任务状态锁已损坏")
            .insert(status.task_id.clone(), status.clone());
        status
    }

    pub fn mark_running(&self, task_id: &str) {
        self.update(task_id, "INDEXING", "索引处理中", None);
    }

    pub fn mark_completed(&self, task_id: &str) {
        self.update(task_id, "COMPLETED", "索引完成", None);
    }

    pub fn mark_failed(&self, task_id: &str, error_message: impl Into<String>) {
        self.update(task_id, "FAILED", "索引失败", Some(error_message.into()));
    }

    pub fn get_status(&self, task_id: &str) -> Option<IndexTaskStatus> {
        self.statuses
            .lock()
            .expect("索引任务状态锁已损坏")
            .get(task_id)
            .cloned()
    }

    pub fn list_statuses(&self) -> Vec<IndexTaskStatus> {
        let mut statuses = self
            .statuses
            .lock()
            .expect("索引任务状态锁已损坏")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        statuses
    }

    fn update(
        &self,
        task_id: &str,
        status_value: &str,
        message: &str,
        error_message: Option<String>,
    ) {
        if let Some(status) = self
            .statuses
            .lock()
            .expect("索引任务状态锁已损坏")
            .get_mut(task_id)
        {
            status.status = status_value.to_string();
            status.message = message.to_string();
            status.error_message = error_message;
            status.updated_at = now_millis();
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}
