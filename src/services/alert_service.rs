//! 告警报告内存存储
//! 对应 Java 版本 `AlertService` 中的报告存储部分（`storeReport` / `getReport`）。
//! AI Ops 流程在生成《告警分析报告》后，可按 alertId 关联存储，供后续查询。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tracing::info;

#[derive(Clone, Default)]
pub struct AlertService {
    reports: Arc<Mutex<HashMap<String, String>>>,
}

impl AlertService {
    pub fn new() -> Self {
        Self::default()
    }

    /// 存储分析报告
    pub fn store_report(&self, alert_id: &str, report: &str) {
        let length = report.chars().count();
        self.reports
            .lock()
            .expect("AlertService reports 锁不应被毒化")
            .insert(alert_id.to_string(), report.to_string());
        info!(
            "告警分析报告已存储, alertId: {}, report length: {}",
            alert_id, length
        );
    }

    /// 获取分析报告
    pub fn get_report(&self, alert_id: &str) -> Option<String> {
        self.reports
            .lock()
            .expect("AlertService reports 锁不应被毒化")
            .get(alert_id)
            .cloned()
    }
}
