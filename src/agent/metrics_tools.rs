//! Prometheus 告警和指标查询工具
//! 对应 Java 版本的 `QueryMetricsTools`，用于查询 Prometheus 的活动告警与核心指标趋势。

use crate::services::chat_service::ChatToolError;
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

const SUPPORTED_TREND_METRICS: [&str; 5] = [
    "cpu_usage",
    "memory_usage",
    "error_rate",
    "p99_latency",
    "restart_count",
];
const SUPPORTED_TREND_WINDOWS: [&str; 3] = ["15m", "1h", "6h"];

/// 查询 Prometheus 活动告警
#[derive(Clone)]
pub struct QueryPrometheusAlertsTool {
    pub base_url: String,
    pub timeout_secs: u64,
    pub mock_enabled: bool,
}

#[derive(Clone, Deserialize)]
pub struct QueryPrometheusAlertsArgs {}

/// 查询核心运维指标趋势
#[derive(Clone)]
pub struct QueryMetricTrendTool {
    pub base_url: String,
    pub timeout_secs: u64,
    pub mock_enabled: bool,
}

#[derive(Clone, Deserialize)]
pub struct QueryMetricTrendArgs {
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    instance: Option<String>,
    #[serde(default)]
    window: Option<String>,
    #[serde(default)]
    step: Option<String>,
}

impl Tool for QueryPrometheusAlertsTool {
    const NAME: &'static str = "queryPrometheusAlerts";

    type Error = ChatToolError;
    type Args = QueryPrometheusAlertsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Query active alerts from Prometheus alerting system. \
This tool retrieves all currently active/firing alerts including their labels, annotations, state, and values. \
Use this tool when you need to check what alerts are currently firing, investigate alert conditions, or monitor alert status."
                .to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        info!("开始查询 Prometheus 活动告警, Mock模式: {}", self.mock_enabled);

        let alerts = if self.mock_enabled {
            let mock = build_mock_alerts();
            info!("使用 Mock 数据，返回 {} 个模拟告警", mock.len());
            mock
        } else {
            match self.fetch_prometheus_alerts().await {
                Ok(alerts) => alerts,
                Err(error) => {
                    warn!("查询 Prometheus 告警失败: {}", error);
                    return Ok(build_error_response(
                        "查询 Prometheus 失败，证据缺失",
                        &error,
                        "DEPENDENCY_ERROR",
                    ));
                }
            }
        };

        let output = json!({
            "success": true,
            "alerts": alerts,
            "message": format!("成功检索到 {} 个活动告警", alerts.len()),
        });
        info!("Prometheus 告警查询完成: 找到 {} 个告警", alerts.len());
        Ok(output.to_string())
    }
}

impl QueryPrometheusAlertsTool {
    async fn fetch_prometheus_alerts(&self) -> Result<Vec<Value>, String> {
        let base = self.base_url.trim().trim_end_matches('/');
        let url = format!("{base}/api/v1/alerts");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|error| format!("初始化 HTTP client 失败: {error}"))?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|error| format!("HTTP 请求失败: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP 请求失败: {}", response.status().as_u16()));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("解析 Prometheus 响应失败: {error}"))?;
        if body.get("status").and_then(Value::as_str) != Some("success") {
            return Err(format!(
                "Prometheus API 返回非成功状态: {}",
                body.get("status").and_then(Value::as_str).unwrap_or("unknown")
            ));
        }

        let now = Utc::now();
        let mut seen: Vec<String> = Vec::new();
        let mut simplified = Vec::new();
        if let Some(alerts) = body
            .get("data")
            .and_then(|data| data.get("alerts"))
            .and_then(Value::as_array)
        {
            for alert in alerts {
                let alert_name = alert
                    .get("labels")
                    .and_then(|labels| labels.get("alertname"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if seen.contains(&alert_name) {
                    continue;
                }
                seen.push(alert_name.clone());
                let description = alert
                    .get("annotations")
                    .and_then(|annotations| annotations.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let state = alert.get("state").and_then(Value::as_str).unwrap_or("").to_string();
                let active_at = alert.get("activeAt").and_then(Value::as_str).unwrap_or("").to_string();
                let duration = duration_from_active_at(&active_at, now);
                simplified.push(json!({
                    "alert_name": alert_name,
                    "description": description,
                    "state": state,
                    "active_at": active_at,
                    "duration": duration,
                }));
            }
        }
        Ok(simplified)
    }
}

impl Tool for QueryMetricTrendTool {
    const NAME: &'static str = "queryMetricTrend";

    type Error = ChatToolError;
    type Args = QueryMetricTrendArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Query Prometheus metric trend over a time window. \
Supported metrics: cpu_usage, memory_usage, error_rate, p99_latency, restart_count. \
Use this before diagnosing CPU, memory, latency, error-rate, or restart alerts. \
Returns points, PromQL query, min/max/avg/latest, direction, anomaly flag, and message."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "metric": {"type": "string", "description": "Metric name. Supported: cpu_usage, memory_usage, error_rate, p99_latency, restart_count"},
                    "service": {"type": "string", "description": "Service name, for example payment-service. Optional but recommended"},
                    "instance": {"type": "string", "description": "Instance or pod name, for example pod-payment-service-xxx. Optional"},
                    "window": {"type": "string", "description": "Trend window. Supported: 15m, 1h, 6h. Invalid or blank values default to 1h"},
                    "step": {"type": "string", "description": "Prometheus query_range step, for example 30s, 1m, 5m. Blank uses the default for the selected window"}
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let metric = normalize_metric(args.metric.as_deref());
        let window = normalize_window(args.window.as_deref());
        let step = normalize_step(&window, args.step.as_deref());
        let service = args.service.as_deref().unwrap_or("");
        let instance = args.instance.as_deref().unwrap_or("");
        info!(
            "开始查询指标趋势, metric: {}, service: {}, instance: {}, window: {}, step: {}, Mock模式: {}",
            metric, service, instance, window, step, self.mock_enabled
        );

        if !SUPPORTED_TREND_METRICS.contains(&metric.as_str()) {
            return Ok(build_metric_trend_error(
                &metric,
                &window,
                None,
                &format!(
                    "不支持的指标: {}。支持的指标: {}",
                    metric,
                    SUPPORTED_TREND_METRICS.join(", ")
                ),
                None,
                None,
            ));
        }

        let query = build_metric_trend_query(&metric, service, instance);
        let points = if self.mock_enabled {
            build_mock_trend_points(&metric, &window, &step)
        } else {
            match self.fetch_prometheus_trend(&query, &window, &step).await {
                Ok(points) => points,
                Err(error) => {
                    warn!("查询指标趋势失败, metric: {}, query: {}: {}", metric, query, error);
                    return Ok(build_metric_trend_error(
                        &metric,
                        &window,
                        Some(&query),
                        &error,
                        Some(&error),
                        Some("DEPENDENCY_ERROR"),
                    ));
                }
            }
        };

        let summary = summarize_trend(&metric, &points);
        let message = build_metric_trend_message(&metric, &window, &summary, &points);
        let anomalous = summary["anomalous"].as_bool().unwrap_or(false);
        let output = json!({
            "success": true,
            "metric": metric,
            "window": window,
            "step": step,
            "query": query,
            "points": points,
            "summary": summary,
            "message": message,
        });
        info!(
            "指标趋势查询完成, metric: {}, points: {}, anomalous: {}",
            metric,
            points.len(),
            anomalous
        );
        Ok(output.to_string())
    }
}

impl QueryMetricTrendTool {
    async fn fetch_prometheus_trend(
        &self,
        query: &str,
        window: &str,
        step: &str,
    ) -> Result<Vec<Value>, String> {
        let base = self.base_url.trim().trim_end_matches('/');
        let url = format!("{base}/api/v1/query_range");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|error| format!("初始化 HTTP client 失败: {error}"))?;

        let end = Utc::now();
        let start = end - ChronoDuration::seconds(window_seconds(window));
        let response = client
            .get(&url)
            .query(&[
                ("query", query),
                ("start", &start.timestamp().to_string()),
                ("end", &end.timestamp().to_string()),
                ("step", step),
            ])
            .send()
            .await
            .map_err(|error| format!("HTTP 请求失败: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP 请求失败: {}", response.status().as_u16()));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("解析 Prometheus 响应失败: {error}"))?;
        if body.get("status").and_then(Value::as_str) != Some("success") {
            return Err(body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Prometheus query_range 返回非成功状态")
                .to_string());
        }

        let mut points = Vec::new();
        if let Some(series) = body
            .get("data")
            .and_then(|data| data.get("result"))
            .and_then(Value::as_array)
        {
            for serie in series {
                let Some(values) = serie.get("values").and_then(Value::as_array) else {
                    continue;
                };
                for value in values {
                    let Some(pair) = value.as_array() else { continue };
                    if pair.len() < 2 {
                        continue;
                    }
                    let timestamp = pair[0].as_f64().unwrap_or(f64::NAN);
                    let metric_value = pair[1]
                        .as_str()
                        .and_then(|raw| raw.parse::<f64>().ok())
                        .unwrap_or(f64::NAN);
                    if metric_value.is_nan() || metric_value.is_infinite() || timestamp.is_nan() {
                        continue;
                    }
                    let ts = DateTime::<Utc>::from_timestamp_millis((timestamp * 1000.0) as i64)
                        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
                        .unwrap_or_default();
                    points.push(json!({ "timestamp": ts, "value": round(metric_value) }));
                }
            }
        }
        Ok(points)
    }
}

// ==================== Mock 数据 ====================

fn build_mock_alerts() -> Vec<Value> {
    let now = Utc::now();
    let mk = |name: &str, desc: &str, mins: i64| {
        let active = now - ChronoDuration::minutes(mins);
        json!({
            "alert_name": name,
            "description": desc,
            "state": "firing",
            "active_at": active.to_rfc3339_opts(SecondsFormat::Secs, true),
            "duration": duration_string(now.signed_duration_since(active)),
        })
    };
    vec![
        mk(
            "HighCPUUsage",
            "服务 payment-service 的 CPU 使用率持续超过 80%，当前值为 92%。实例: pod-payment-service-7d8f9c6b5-x2k4m，命名空间: production",
            25,
        ),
        mk(
            "HighMemoryUsage",
            "服务 order-service 的内存使用率持续超过 85%，当前值为 91%。JVM堆内存使用: 3.8GB/4GB，可能存在内存泄漏风险。实例: pod-order-service-5c7d8e9f1-m3n2p，命名空间: production",
            15,
        ),
        mk(
            "SlowResponse",
            "服务 user-service 的 P99 响应时间持续超过 3 秒，当前值为 4.2 秒。受影响接口: /api/v1/users/profile, /api/v1/users/orders。可能原因：数据库慢查询或下游服务延迟",
            10,
        ),
    ]
}

fn build_mock_trend_points(metric: &str, window: &str, step: &str) -> Vec<Value> {
    let count = mock_point_count(window, step);
    let start = Utc::now() - ChronoDuration::seconds(window_seconds(window));
    let total = window_seconds(window);
    let mut points = Vec::new();
    for i in 0..count {
        let ratio = if count <= 1 {
            1.0
        } else {
            i as f64 / (count - 1) as f64
        };
        let value = match metric {
            "cpu_usage" => 58.0 + ratio * 36.0,
            "memory_usage" => 64.0 + ratio * 28.0,
            "error_rate" => {
                if ratio < 0.75 {
                    0.2 + ratio * 0.8
                } else {
                    2.0 + (ratio - 0.75) * 40.0
                }
            }
            "p99_latency" => 0.8 + ratio * ratio * 3.6,
            "restart_count" => {
                if ratio < 0.55 {
                    0.0
                } else {
                    ((ratio - 0.55) * 8.0).floor()
                }
            }
            _ => 0.0,
        };
        let offset = if count <= 1 {
            0
        } else {
            total * i as i64 / (count - 1) as i64
        };
        let ts = (start + ChronoDuration::seconds(offset)).to_rfc3339_opts(SecondsFormat::Secs, true);
        points.push(json!({ "timestamp": ts, "value": round(value) }));
    }
    points
}

// ==================== 汇总与查询构造 ====================

fn summarize_trend(metric: &str, points: &[Value]) -> Value {
    if points.is_empty() {
        return json!({
            "min": 0.0, "max": 0.0, "avg": 0.0, "latest": 0.0,
            "direction": "stable", "anomalous": false,
        });
    }
    let values: Vec<f64> = points
        .iter()
        .map(|point| point["value"].as_f64().unwrap_or(0.0))
        .collect();
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    let max = values.iter().cloned().fold(-f64::MAX, f64::max);
    let total: f64 = values.iter().sum();
    let first = values[0];
    let latest = values[values.len() - 1];
    let avg = total / values.len() as f64;
    json!({
        "min": round(min),
        "max": round(max),
        "avg": round(avg),
        "latest": round(latest),
        "direction": detect_direction(first, latest, max, avg),
        "anomalous": is_anomalous(metric, latest, max),
    })
}

fn detect_direction(first: f64, latest: f64, max: f64, avg: f64) -> &'static str {
    let delta = latest - first;
    let base = first.abs().max(1.0);
    if max > (avg * 2.0).max(first + base * 0.8) && latest > avg * 1.3 {
        return "spiking";
    }
    if delta.abs() <= base * 0.08 {
        return "stable";
    }
    if delta > 0.0 {
        "increasing"
    } else {
        "decreasing"
    }
}

fn is_anomalous(metric: &str, latest: f64, max: f64) -> bool {
    match metric {
        "cpu_usage" => latest >= 80.0 || max >= 90.0,
        "memory_usage" => latest >= 85.0 || max >= 90.0,
        "error_rate" => latest >= 1.0 || max >= 5.0,
        "p99_latency" => latest >= 3.0 || max >= 3.0,
        "restart_count" => latest > 0.0 || max > 0.0,
        _ => false,
    }
}

fn build_metric_trend_message(metric: &str, window: &str, summary: &Value, points: &[Value]) -> String {
    if points.is_empty() {
        return format!("{metric} 最近 {window} 未查询到趋势点");
    }
    format!(
        "{} 最近 {} {}，latest={:.2}，max={:.2}，avg={:.2}，anomalous={}",
        metric,
        window,
        direction_text(summary["direction"].as_str().unwrap_or("stable")),
        summary["latest"].as_f64().unwrap_or(0.0),
        summary["max"].as_f64().unwrap_or(0.0),
        summary["avg"].as_f64().unwrap_or(0.0),
        summary["anomalous"].as_bool().unwrap_or(false),
    )
}

fn direction_text(direction: &str) -> &'static str {
    match direction {
        "increasing" => "持续上升",
        "decreasing" => "持续下降",
        "spiking" => "出现突增",
        _ => "整体平稳",
    }
}

fn build_metric_trend_query(metric: &str, service: &str, instance: &str) -> String {
    let app_selector = build_selector("job", service, "instance", instance, &[]);
    let app_error_selector =
        build_selector("job", service, "instance", instance, &[("status", "=~\"5..\"")]);
    let pod_value = first_non_blank(instance, service);
    let pod_selector = build_selector("pod", pod_value, "", "", &[("container", "!\"\"")]);
    match metric {
        "cpu_usage" => {
            format!("100 * avg(rate(container_cpu_usage_seconds_total{pod_selector}[5m]))")
        }
        "memory_usage" => format!(
            "100 * avg(container_memory_working_set_bytes{pod_selector}) / clamp_min(avg(container_spec_memory_limit_bytes{pod_selector}), 1)"
        ),
        "error_rate" => format!(
            "sum(rate(http_requests_total{app_error_selector}[5m])) / clamp_min(sum(rate(http_requests_total{app_selector}[5m])), 1) * 100"
        ),
        "p99_latency" => format!(
            "histogram_quantile(0.99, sum(rate(http_request_duration_seconds_bucket{app_selector}[5m])) by (le))"
        ),
        "restart_count" => {
            let restart_selector = build_selector("pod", pod_value, "", "", &[]);
            format!("sum(increase(kube_pod_container_status_restarts_total{restart_selector}[5m]))")
        }
        _ => metric.to_string(),
    }
}

fn build_selector(
    primary_label: &str,
    primary_value: &str,
    secondary_label: &str,
    secondary_value: &str,
    extra_matchers: &[(&str, &str)],
) -> String {
    let mut matchers: Vec<String> = Vec::new();
    if !primary_label.is_empty() && !primary_value.trim().is_empty() {
        matchers.push(format!("{primary_label}=~\"{}\"", regex_contains(primary_value)));
    }
    if !secondary_label.is_empty() && !secondary_value.trim().is_empty() {
        matchers.push(format!(
            "{secondary_label}=~\"{}\"",
            regex_contains(secondary_value)
        ));
    }
    for (label, value) in extra_matchers {
        matchers.push(format!("{label}{value}"));
    }
    if matchers.is_empty() {
        "{}".to_string()
    } else {
        format!("{{{}}}", matchers.join(","))
    }
}

fn regex_contains(value: &str) -> String {
    let escaped = value
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('.', "\\.")
        .replace('*', "\\*")
        .replace('+', "\\+")
        .replace('?', "\\?")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('|', "\\|")
        .replace('^', "\\^")
        .replace('$', "\\$");
    format!(".*{escaped}.*")
}

// ==================== 归一化与工具函数 ====================

fn normalize_metric(metric: Option<&str>) -> String {
    metric.unwrap_or("").trim().to_lowercase()
}

fn normalize_window(window: Option<&str>) -> String {
    let normalized = window.unwrap_or("").trim().to_lowercase();
    if SUPPORTED_TREND_WINDOWS.contains(&normalized.as_str()) {
        normalized
    } else {
        "1h".to_string()
    }
}

fn normalize_step(window: &str, step: Option<&str>) -> String {
    let normalized = step.unwrap_or("").trim().to_lowercase();
    if is_step(&normalized) {
        return normalized;
    }
    match window {
        "15m" => "30s",
        "6h" => "5m",
        _ => "1m",
    }
    .to_string()
}

fn is_step(step: &str) -> bool {
    let bytes = step.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    let (digits, unit) = bytes.split_at(bytes.len() - 1);
    digits.iter().all(u8::is_ascii_digit)
        && matches!(unit[0], b's' | b'm' | b'h')
}

fn window_seconds(window: &str) -> i64 {
    match normalize_window(Some(window)).as_str() {
        "15m" => 15 * 60,
        "6h" => 6 * 3600,
        _ => 3600,
    }
}

fn mock_point_count(window: &str, step: &str) -> usize {
    let window_secs = window_seconds(window);
    let step_secs = parse_step_seconds(step);
    let count = if step_secs <= 0 {
        20
    } else {
        window_secs / step_secs + 1
    };
    count.clamp(8, 80) as usize
}

fn parse_step_seconds(step: &str) -> i64 {
    if !is_step(step) {
        return 60;
    }
    let (digits, unit) = step.split_at(step.len() - 1);
    let value: i64 = digits.parse().unwrap_or(60);
    match unit {
        "s" => value,
        "h" => value * 3600,
        _ => value * 60,
    }
}

fn duration_from_active_at(active_at: &str, now: DateTime<Utc>) -> String {
    match DateTime::parse_from_rfc3339(active_at) {
        Ok(active) => duration_string(now.signed_duration_since(active.with_timezone(&Utc))),
        Err(_) => "unknown".to_string(),
    }
}

fn duration_string(duration: ChronoDuration) -> String {
    let seconds = duration.num_seconds().max(0);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes}m{secs}s")
    } else if minutes > 0 {
        format!("{minutes}m{secs}s")
    } else {
        format!("{secs}s")
    }
}

fn first_non_blank<'a>(first: &'a str, second: &'a str) -> &'a str {
    if !first.trim().is_empty() {
        first
    } else {
        second
    }
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
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

fn build_metric_trend_error(
    metric: &str,
    window: &str,
    query: Option<&str>,
    message: &str,
    error: Option<&str>,
    error_code: Option<&str>,
) -> String {
    json!({
        "success": false,
        "metric": metric,
        "window": window,
        "query": query,
        "points": [],
        "summary": summarize_trend(metric, &[]),
        "message": if message.trim().is_empty() { "查询指标趋势失败" } else { message },
        "error": error,
        "errorCode": error_code,
    })
    .to_string()
}
