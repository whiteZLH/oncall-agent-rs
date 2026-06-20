//! 日志查询工具
//! 对应 Java 版本的 `QueryLogsTools`，用于查询 CLS（云日志服务）的日志信息（仅 Mock 模式可用）。

use crate::services::chat_service::ChatToolError;
use chrono::{Duration as ChronoDuration, FixedOffset, Utc};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{error, info};

/// 获取可用的日志主题列表
#[derive(Clone)]
pub struct GetAvailableLogTopicsTool;

#[derive(Clone, Deserialize)]
pub struct GetAvailableLogTopicsArgs {}

/// 查询日志
#[derive(Clone)]
pub struct QueryLogsTool {
    pub mock_enabled: bool,
}

#[derive(Clone, Deserialize)]
pub struct QueryLogsArgs {
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    log_topic: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

impl Tool for GetAvailableLogTopicsTool {
    const NAME: &'static str = "getAvailableLogTopics";

    type Error = ChatToolError;
    type Args = GetAvailableLogTopicsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get all available log topics and their descriptions. \
Use this tool only when the log topic cannot be inferred from alert type or planner context. \
Returns a list of log topics with their names, descriptions, and example queries."
                .to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        info!("获取可用的日志主题列表");
        let topics = json!([
            {
                "topic_name": "system-metrics",
                "description": "系统指标日志，包含 CPU、内存、磁盘使用率等系统资源监控数据",
                "example_queries": ["cpu_usage:>80", "memory_usage:>85", "disk_usage:>90", "level:WARN AND service:payment-service"],
                "related_alerts": ["HighCPUUsage", "HighMemoryUsage", "HighDiskUsage"]
            },
            {
                "topic_name": "application-logs",
                "description": "应用日志，包含应用程序的错误日志、警告日志、慢请求日志、下游依赖调用日志等",
                "example_queries": ["level:ERROR", "level:FATAL", "http_status:500", "response_time:>3000", "slow", "downstream OR redis OR database OR mq"],
                "related_alerts": ["ServiceUnavailable", "SlowResponse", "HighMemoryUsage"]
            },
            {
                "topic_name": "database-slow-query",
                "description": "数据库慢查询日志，包含执行时间较长的 SQL 查询，可用于分析数据库性能问题",
                "example_queries": ["query_time:>2", "table:orders", "query_type:SELECT", "*"],
                "related_alerts": ["SlowResponse", "ServiceUnavailable"]
            },
            {
                "topic_name": "system-events",
                "description": "系统事件日志，包含 Kubernetes Pod 重启、OOM Kill、容器崩溃等系统级事件",
                "example_queries": ["restart OR crash", "oom_kill", "event_type:PodRestart", "reason:OOMKilled"],
                "related_alerts": ["ServiceUnavailable", "HighMemoryUsage"]
            }
        ]);
        let output = json!({
            "success": true,
            "topics": topics,
            "available_regions": ["ap-guangzhou", "ap-shanghai", "ap-beijing", "ap-chengdu"],
            "default_region": "ap-guangzhou",
            "message": "共有 4 个可用的日志主题。建议使用默认地域 'ap-guangzhou' 或省略 region 参数",
        });
        Ok(output.to_string())
    }
}

impl Tool for QueryLogsTool {
    const NAME: &'static str = "queryLogs";

    type Error = ChatToolError;
    type Args = QueryLogsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Query logs from Cloud Log Service (CLS). \
Use this tool to search application logs, system metrics, and other log data. \
Call getAvailableLogTopics only when the log topic cannot be inferred from the alert type or planner context. \
Available log topics: \
1) 'system-metrics' - System metrics logs (CPU, memory, disk usage, etc. Related to HighCPUUsage, HighMemoryUsage, HighDiskUsage alerts); \
2) 'application-logs' - Application logs (error logs, slow request logs, downstream dependency logs. Related to ServiceUnavailable, SlowResponse alerts); \
3) 'database-slow-query' - Database slow query logs (SQL queries with long execution time. Related to SlowResponse alerts); \
4) 'system-events' - System event logs (Pod restart, OOM Kill, container crash. Related to ServiceUnavailable, HighMemoryUsage alerts). \
logTopic (required, one of the above topics or their CLS topicId), \
query (optional, defaults to a curated search if empty), \
limit (optional, default 20, max 100)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "region": {"type": "string", "description": "地域，可选值: ap-guangzhou, ap-shanghai, ap-beijing, ap-chengdu。默认 ap-guangzhou"},
                    "log_topic": {"type": "string", "description": "日志主题，如 system-metrics, application-logs, database-slow-query, system-events，也支持 CLS TopicId"},
                    "query": {"type": "string", "description": "查询条件，支持 Lucene 语法，如 level:ERROR OR cpu_usage:>80；为空时返回该主题近 5 条核心日志"},
                    "limit": {"type": "integer", "description": "返回日志条数，默认20，最大100"}
                },
                "required": ["log_topic"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = match args.limit {
            Some(value) if value > 0 => value.min(100) as usize,
            _ => 20,
        };
        let region = args.region.unwrap_or_default();
        let log_topic = args.log_topic.unwrap_or_default();
        let safe_query = args.query.unwrap_or_default();

        if !self.mock_enabled {
            error!("CLS 真实查询尚未实现，请启用 mock 模式进行测试");
            return Ok(build_error_response(
                "查询日志失败: CLS 真实查询尚未实现，请启用 mock 模式进行测试",
                "CLS 真实查询尚未实现，请启用 mock 模式进行测试",
                "DEPENDENCY_ERROR",
            ));
        }

        let logs = build_mock_logs(&log_topic, &safe_query, limit);
        info!("使用 Mock 数据，返回 {} 条日志", logs.len());

        let output = json!({
            "success": !logs.is_empty(),
            "region": region,
            "log_topic": log_topic,
            "query": if safe_query.trim().is_empty() { "DEFAULT_QUERY".to_string() } else { safe_query.clone() },
            "logs": logs,
            "total": logs.len(),
            "message": if logs.is_empty() { "未找到匹配的日志".to_string() } else { format!("成功查询到 {} 条日志", logs.len()) },
        });
        info!("日志查询完成: 找到 {} 条日志", logs.len());
        Ok(output.to_string())
    }
}

fn now_shanghai_minus(minutes: i64) -> String {
    let offset = FixedOffset::east_opt(8 * 3600).expect("固定时区偏移合法");
    let instant = Utc::now() - ChronoDuration::minutes(minutes);
    instant
        .with_timezone(&offset)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn log_entry(timestamp: String, level: &str, service: &str, instance: &str, message: String, metrics: Value) -> Value {
    json!({
        "timestamp": timestamp,
        "level": level,
        "service": service,
        "instance": instance,
        "message": message,
        "metrics": metrics,
    })
}

fn build_mock_logs(log_topic: &str, query: &str, limit: usize) -> Vec<Value> {
    let safe_topic = if log_topic.is_empty() {
        "system-metrics".to_string()
    } else {
        log_topic.to_lowercase()
    };
    let normalized_query = query.to_lowercase();

    let mut logs = match safe_topic.as_str() {
        "system-metrics" => build_system_metrics_logs(&normalized_query),
        "application-logs" => build_application_logs(&normalized_query),
        "database-slow-query" => build_database_slow_query_logs(),
        "system-events" => build_system_events_logs(&normalized_query),
        _ => build_generic_logs(&normalized_query, limit),
    };

    if logs.is_empty() {
        logs = build_generic_logs(&normalized_query, limit);
    }
    if logs.len() > limit {
        logs.truncate(limit);
    }
    logs
}

fn build_system_metrics_logs(query: &str) -> Vec<Value> {
    let mut logs = Vec::new();
    if query.contains("cpu") || query.contains(">80") {
        for i in 0..5 {
            logs.push(log_entry(
                now_shanghai_minus(i as i64 * 2),
                "WARN",
                "payment-service",
                "pod-payment-service-7d8f9c6b5-x2k4m",
                format!("CPU使用率过高: {:.1}%, 进程: java (PID: 1), 线程数: 245", 92.0 - i as f64 * 1.5),
                json!({
                    "cpu_usage": format!("{:.1}", 92.0 - i as f64 * 1.5),
                    "cpu_cores": "4",
                    "load_average_1m": "3.82",
                    "load_average_5m": "3.65",
                    "top_process": "java",
                    "process_threads": "245"
                }),
            ));
        }
    }
    if query.contains("memory") || query.contains(">85") || query.contains("oom") {
        for i in 0..5 {
            logs.push(log_entry(
                now_shanghai_minus(i as i64 * 3),
                "WARN",
                "order-service",
                "pod-order-service-5c7d8e9f1-m3n2p",
                format!(
                    "内存使用率过高: {:.1}%, JVM堆内存: {:.1}GB/4GB, GC次数: {}",
                    91.0 - i as f64 * 1.2,
                    3.8 - i as f64 * 0.1,
                    128 - i * 5
                ),
                json!({
                    "memory_usage": format!("{:.1}", 91.0 - i as f64 * 1.2),
                    "jvm_heap_used": format!("{:.1}GB", 3.8 - i as f64 * 0.1),
                    "jvm_heap_max": "4GB",
                    "gc_count": (128 - i * 5).to_string(),
                    "gc_time_ms": (1250 + i * 50).to_string()
                }),
            ));
        }
        logs.push(log_entry(
            now_shanghai_minus(8),
            "WARN",
            "order-service",
            "pod-order-service-5c7d8e9f1-m3n2p",
            "频繁 Full GC 警告: 过去10分钟内发生 15 次 Full GC, 平均耗时 850ms, 建议检查内存泄漏".to_string(),
            json!({"full_gc_count": "15", "avg_gc_time_ms": "850", "survivor_space": "95%", "old_gen": "89%"}),
        ));
    }
    if query.contains("disk") || query.contains("filesystem") {
        for i in 0..3 {
            logs.push(log_entry(
                now_shanghai_minus(i as i64 * 5),
                "WARN",
                "log-collector",
                "node-worker-01",
                format!(
                    "磁盘使用率告警: /data 分区使用率 {:.1}%, 可用空间: {:.1}GB",
                    85.0 + i as f64 * 2.0,
                    15.0 - i as f64 * 2.0
                ),
                json!({
                    "disk_usage": format!("{:.1}%", 85.0 + i as f64 * 2.0),
                    "disk_available": format!("{:.1}GB", 15.0 - i as f64 * 2.0),
                    "disk_total": "100GB",
                    "mount_point": "/data",
                    "largest_dir": "/data/logs"
                }),
            ));
        }
    }
    logs
}

fn build_application_logs(query: &str) -> Vec<Value> {
    let mut logs = Vec::new();
    if query.contains("error") || query.contains("fatal") || query.contains("500") {
        logs.push(log_entry(
            now_shanghai_minus(5),
            "ERROR",
            "order-service",
            "pod-order-service-5c7d8e9f1-m3n2p",
            "数据库连接池耗尽: Cannot acquire connection from pool, active: 50/50, waiting: 23, timeout: 30000ms".to_string(),
            json!({"error_type": "ConnectionPoolExhaustedException", "pool_active": "50", "pool_max": "50", "waiting_threads": "23"}),
        ));
        logs.push(log_entry(
            now_shanghai_minus(12),
            "FATAL",
            "order-service",
            "pod-order-service-5c7d8e9f1-m3n2p",
            "java.lang.OutOfMemoryError: Java heap space at com.example.order.service.OrderService.processLargeOrder(OrderService.java:156)".to_string(),
            json!({
                "error_type": "OutOfMemoryError",
                "heap_used": "3.9GB",
                "heap_max": "4GB",
                "stack_trace": "OrderService.processLargeOrder -> OrderRepository.findByCondition -> HikariPool.getConnection"
            }),
        ));
        for i in 0..3 {
            logs.push(log_entry(
                now_shanghai_minus(3 + i as i64),
                "ERROR",
                "user-service",
                "pod-user-service-8e9f0a1b2-k5j6h",
                format!(
                    "HTTP 500 Internal Server Error: /api/v1/users/profile, 耗时: {}ms, 错误: Database query timeout",
                    5200 + i * 300
                ),
                json!({
                    "http_status": "500",
                    "uri": "/api/v1/users/profile",
                    "method": "GET",
                    "duration_ms": (5200 + i * 300).to_string(),
                    "error_cause": "QueryTimeoutException"
                }),
            ));
        }
    }
    if query.contains("response_time") || query.contains("slow") || query.contains(">3000") {
        for i in 0..5 {
            let uri = if i % 2 == 0 { "/api/v1/users/profile" } else { "/api/v1/users/orders" };
            logs.push(log_entry(
                now_shanghai_minus(i as i64 * 2),
                "WARN",
                "user-service",
                "pod-user-service-8e9f0a1b2-k5j6h",
                format!("慢请求警告: {}, 响应时间: {}ms, 阈值: 3000ms", uri, 4200 - i * 150),
                json!({
                    "uri": uri,
                    "response_time_ms": (4200 - i * 150).to_string(),
                    "threshold_ms": "3000",
                    "db_time_ms": (3800 - i * 100).to_string(),
                    "cache_hit": "false"
                }),
            ));
        }
    }
    if query.contains("downstream") || query.contains("redis") || query.contains("database") || query.contains("mq") {
        logs.push(log_entry(
            now_shanghai_minus(7),
            "ERROR",
            "payment-service",
            "pod-payment-service-7d8f9c6b5-x2k4m",
            "Redis 连接超时: 无法连接到 Redis 集群, 节点: redis-cluster-01:6379, 超时: 3000ms".to_string(),
            json!({"dependency": "redis", "host": "redis-cluster-01:6379", "timeout_ms": "3000", "retry_count": "3"}),
        ));
        logs.push(log_entry(
            now_shanghai_minus(9),
            "WARN",
            "order-service",
            "pod-order-service-5c7d8e9f1-m3n2p",
            "消息队列积压警告: 队列 order-process-queue 积压消息数: 15823, 消费速率下降".to_string(),
            json!({"dependency": "rabbitmq", "queue": "order-process-queue", "pending_messages": "15823", "consumer_count": "3"}),
        ));
    }
    logs
}

fn build_database_slow_query_logs() -> Vec<Value> {
    vec![
        log_entry(
            now_shanghai_minus(3),
            "WARN",
            "mysql",
            "mysql-primary-01",
            "慢查询: SELECT * FROM orders WHERE user_id = ? AND status IN (?, ?, ?) ORDER BY created_at DESC LIMIT 100, 执行时间: 3.2s, 扫描行数: 1,245,678".to_string(),
            json!({
                "query_time_sec": "3.2",
                "rows_examined": "1245678",
                "rows_returned": "100",
                "index_used": "idx_user_id",
                "table": "orders",
                "query_type": "SELECT"
            }),
        ),
        log_entry(
            now_shanghai_minus(6),
            "WARN",
            "mysql",
            "mysql-primary-01",
            "慢查询: SELECT u.*, p.* FROM users u LEFT JOIN user_profiles p ON u.id = p.user_id WHERE u.last_login > ?, 执行时间: 2.8s, 全表扫描".to_string(),
            json!({
                "query_time_sec": "2.8",
                "rows_examined": "856234",
                "rows_returned": "45678",
                "index_used": "NONE",
                "table": "users, user_profiles",
                "query_type": "SELECT",
                "warning": "Full table scan detected"
            }),
        ),
        log_entry(
            now_shanghai_minus(8),
            "WARN",
            "mysql",
            "mysql-primary-01",
            "慢查询: UPDATE orders SET status = ? WHERE created_at < ? AND status = ?, 执行时间: 4.5s, 锁等待时间: 2.1s".to_string(),
            json!({
                "query_time_sec": "4.5",
                "lock_time_sec": "2.1",
                "rows_affected": "23456",
                "table": "orders",
                "query_type": "UPDATE",
                "warning": "High lock contention"
            }),
        ),
    ]
}

fn build_system_events_logs(query: &str) -> Vec<Value> {
    let mut logs = Vec::new();
    if query.contains("restart") || query.contains("crash") || query.contains("oom_kill") {
        logs.push(log_entry(
            now_shanghai_minus(15),
            "WARN",
            "kubernetes",
            "kube-controller-manager",
            "Pod 重启事件: pod-order-service-5c7d8e9f1-m3n2p, 原因: OOMKilled, 容器退出码: 137, 重启次数: 3".to_string(),
            json!({
                "event_type": "PodRestart",
                "pod": "pod-order-service-5c7d8e9f1-m3n2p",
                "reason": "OOMKilled",
                "exit_code": "137",
                "restart_count": "3",
                "namespace": "production"
            }),
        ));
        logs.push(log_entry(
            now_shanghai_minus(16),
            "ERROR",
            "kernel",
            "node-worker-02",
            "OOM Killer 触发: 进程 java (PID: 12345) 被杀死, 内存使用: 3.9GB, 内存限制: 4GB".to_string(),
            json!({
                "event_type": "OOMKill",
                "process": "java",
                "pid": "12345",
                "memory_used": "3.9GB",
                "memory_limit": "4GB",
                "cgroup": "/kubepods/pod-order-service"
            }),
        ));
    }
    logs
}

fn build_generic_logs(query: &str, limit: usize) -> Vec<Value> {
    let mut logs = Vec::new();
    for i in 0..limit.min(10) {
        let level = match i % 3 {
            0 => "ERROR",
            1 => "WARN",
            _ => "INFO",
        };
        logs.push(log_entry(
            now_shanghai_minus(i as i64),
            level,
            "generic-service",
            &format!("instance-{i}"),
            format!("日志消息 #{i}, 查询条件: {query}"),
            json!({}),
        ));
    }
    logs
}

fn build_error_response(message: &str, error: &str, error_code: &str) -> String {
    json!({
        "success": false,
        "message": message,
        "error": error,
        "errorCode": error_code,
    })
    .to_string()
}
