//! 网络层诊断：直接用 reqwest 复刻 call_agent 发往 OpenAI Responses 端点的请求，
//! 反复探测并把 reqwest::Error 彻底拆解，用于观测偶发的
//! "error sending request for url" 错误的真实底层原因。
//!
//! 运行：
//!   cargo run --example probe_responses
//! 可用环境变量覆盖：
//!   PROBE_BASE_URL / PROBE_API_KEY / PROBE_MODEL / PROBE_ROUNDS / PROBE_INTERVAL_MS

use std::collections::BTreeMap;
use std::error::Error;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let base =
        std::env::var("PROBE_BASE_URL").unwrap_or_else(|_| "https://anyrouter.top/v1".to_string());
    let key = std::env::var("PROBE_API_KEY")
        .or_else(|_| std::env::var("DASHSCOPE_API_KEY"))
        .expect("请设置 PROBE_API_KEY 或 DASHSCOPE_API_KEY 环境变量");
    let model = std::env::var("PROBE_MODEL").unwrap_or_else(|_| "gpt-5.5".to_string());
    let rounds: usize = std::env::var("PROBE_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let interval_ms: u64 = std::env::var("PROBE_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);

    let url = format!("{}/responses", base.trim_end_matches('/'));
    println!("[probe] target = {url}");
    println!("[probe] model  = {model}");
    println!("[probe] rounds = {rounds}, interval = {interval_ms}ms");

    // reqwest 默认会读取系统代理环境变量，这里把它们打印出来便于判断是否走代理。
    let mut saw_proxy = false;
    for k in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        if let Ok(v) = std::env::var(k) {
            println!("[env] {k}={v}");
            saw_proxy = true;
        }
    }
    if !saw_proxy {
        println!("[env] (未发现 *_PROXY 环境变量)");
    }
    println!();

    // 尽量贴近 rig ReqwestClient::default()：rustls-tls。为防止诊断进程卡死，
    // 额外加上连接/总超时（生产 client 可能没有，会把 timeout 类错误归到 send 错误里）。
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build reqwest client");

    let body = serde_json::json!({
        "model": model,
        "input": [{
            "role": "user",
            "type": "message",
            "content": [{ "type": "input_text", "text": "ping" }]
        }]
    });

    let mut ok = 0usize;
    let mut http_err: BTreeMap<u16, usize> = BTreeMap::new();
    let mut kind_err: BTreeMap<&'static str, usize> = BTreeMap::new();

    for i in 1..=rounds {
        let started = Instant::now();
        let resp = client.post(&url).bearer_auth(&key).json(&body).send().await;
        let ms = started.elapsed().as_millis();

        match resp {
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                let snippet: String = text.chars().take(200).collect();
                if status.is_success() {
                    ok += 1;
                    println!("#{i:>3} {ms:>6}ms  OK    {status}");
                } else {
                    *http_err.entry(status.as_u16()).or_default() += 1;
                    println!("#{i:>3} {ms:>6}ms  HTTP  {status}  body: {snippet}");
                }
            }
            Err(e) => {
                let kind = classify(&e);
                *kind_err.entry(kind).or_default() += 1;
                println!("#{i:>3} {ms:>6}ms  ERR   [{kind}] {e}");

                // 逐层展开 source / cause 链——这是 rig 字符串化时丢掉的关键信息。
                let mut src: Option<&(dyn Error + 'static)> = e.source();
                let mut depth = 1;
                while let Some(s) = src {
                    println!("        └─[cause {depth}] {s}");
                    src = s.source();
                    depth += 1;
                }
                // Debug 格式通常包含 kind/url/内部 hyper/io 错误，信息最全。
                println!("        debug: {e:?}");
            }
        }

        if i < rounds {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    println!("\n==== 汇总 (共 {rounds} 轮) ====");
    println!("成功 (2xx):            {ok}");
    if !kind_err.is_empty() {
        println!("-- 发送/传输层错误 (拿不到响应) --");
        for (kind, n) in &kind_err {
            println!("  {kind:<14} {n}");
        }
    }
    if !http_err.is_empty() {
        println!("-- 拿到响应但状态非 2xx --");
        for (code, n) in &http_err {
            println!("  HTTP {code:<9} {n}");
        }
    }
}

/// 把 reqwest::Error 归到一个粗类别，便于统计分布。
fn classify(e: &reqwest::Error) -> &'static str {
    if e.is_connect() {
        "connect"
    } else if e.is_timeout() {
        "timeout"
    } else if e.is_request() {
        "request/send"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else if e.is_builder() {
        "builder"
    } else if e.is_redirect() {
        "redirect"
    } else {
        "other"
    }
}
