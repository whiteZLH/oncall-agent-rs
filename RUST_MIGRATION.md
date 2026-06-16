# oncall-agent Rust 技术选型与迁移草案

这不是一次 Java 到 Rust 的等价移植，更像是围绕现有能力重新组装一套 Rust 服务。Web、异步运行时、Redis、HTTP、指标、日志、向量库在 Rust 生态里都很成熟；真正需要谨慎设计的是 Spring AI 风格的一站式 Agent 编排体验，尤其是当前项目里的 planner -> executor -> replan -> finish 诊断循环。

## 迁移判断

最稳路线不是寻找一个 Rust 全家桶框架，而是先把项目的确定性基础能力迁过去，再逐步接入 Agent/RAG 抽象：

1. 用 axum + tokio + serde + reqwest + redis 跑通聊天、会话、Incident、Webhook、SSE。
2. 接入 rig 作为 Agent、Tool、Embedding、Vector Store 的基础抽象。
3. 对 AIOps 事故诊断 loop 手写显式状态机，保留对工具调用、重试、证据记录、诊断报告的控制。

## Java 组件到 Rust crate 对位

| 当前 Java/Spring 组件                    | Rust 建议                        | 说明                                                                                                                       |
| ---------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| Spring Boot Web                          | axum + tokio + tower-http        | REST、SSE、文件上传、中间件。axum 负责路由、extractor、response；tower-http 负责 CORS、trace、request-id、timeout 等能力。 |
| Spring async / executor                  | tokio                            | 用 `tokio::spawn`、channel、time、runtime 管后台任务、SSE 推流、诊断异步执行。                                             |
| Jackson / DTO                            | serde + serde_json               | Rust 类型建模更紧，适合将 request/response、Incident、Evidence、DiagnosisRun 等结构显式化。                                |
| RedisTemplate / Redis client             | redis                            | 支持异步连接、connection manager、pubsub，可承接会话、状态缓存、轻量分布式协调。                                           |
| OkHttp / RestTemplate / WebClient        | reqwest                          | 用于调用 DashScope、Prometheus、日志服务、Webhook、内部依赖检查。                                                          |
| Milvus Java SDK                          | rig-milvus 或 Qdrant             | 保守迁移可继续 Milvus；如果优先 Rust 体验，建议评估 Qdrant。                                                               |
| Spring AI / ReactAgent / SupervisorAgent | rig + 手写状态机                 | 简单聊天可用 `rig::Agent`；事故诊断建议手写 planner/executor/replan/finish loop。                                          |
| Micrometer                               | prometheus 或 metrics            | 直接暴露 `/metrics` 用 prometheus；想保留 facade 层可用 metrics。                                                          |
| SLF4J / Logback                          | tracing + tracing-subscriber     | 适合记录一次诊断 run、工具调用、外部依赖耗时、错误上下文。                                                                 |
| Spring configuration properties          | config + serde                   | 用 TOML/YAML/env 组合配置 DashScope、Redis、向量库、安全、限流、上传等参数。                                               |
| JUnit / Mockito                          | cargo test + mockito/wiremock-rs | 服务层逻辑用单元测试；外部 HTTP 依赖用 mock server。                                                                       |

## 能直接稳定替代的部分

- Web API
- SSE 推流
- 文件上传
- Redis 会话与缓存
- HTTP 调第三方服务
- JSON 序列化与 DTO 建模
- 指标与日志
- 本地 JSON 文件持久化
- Webhook 接入
- 依赖健康检查

这些能力在 Rust 里都比较成熟，迁移风险主要来自接口细节和行为兼容，而不是生态缺口。

## 需要自己多搭的部分

- Agent loop
- Tool calling 协议
- 多 Agent 编排
- RAG 管线 glue code
- DashScope 专用集成
- 诊断证据采集与报告生成的生命周期控制

Rust 里没有 Spring AI 那种“框架帮你串好”的舒适层。更现实的做法是把 Agent、工具、RAG、状态机拆开，显式定义上下文、状态迁移、工具结果、失败策略和最终报告。

## 推荐目录结构

```text
oncall-agent-rs/
  Cargo.toml
  config/
    default.toml
    dev.toml
    prod.toml
  src/
    main.rs
    app.rs
    config.rs
    error.rs
    http/
      mod.rs
      routes.rs
      state.rs
      middleware.rs
      chat.rs
      incidents.rs
      knowledge.rs
      webhooks.rs
      sse.rs
      uploads.rs
      health.rs
      metrics.rs
    domain/
      mod.rs
      chat.rs
      incident.rs
      diagnosis.rs
      evidence.rs
      document.rs
      dependency.rs
    services/
      mod.rs
      chat_service.rs
      session_manager.rs
      incident_service.rs
      incident_store.rs
      diagnosis_service.rs
      diagnosis_report_service.rs
      evidence_recorder.rs
      rag_service.rs
      vector_index_service.rs
      vector_search_service.rs
      embedding_service.rs
      metric_trend_prefetch.rs
    agent/
      mod.rs
      loop_state.rs
      planner.rs
      executor.rs
      replanner.rs
      tools.rs
      tool_result.rs
    clients/
      mod.rs
      dashscope.rs
      prometheus.rs
      logs.rs
      milvus.rs
      qdrant.rs
      redis.rs
    observability/
      mod.rs
      tracing.rs
      prometheus.rs
    storage/
      mod.rs
      json_store.rs
      redis_store.rs
  tests/
    chat_api.rs
    incident_api.rs
    diagnosis_loop.rs
    rag_service.rs
```

## Agent loop 建议

事故诊断不要强依赖框架式 agent orchestration。建议把诊断过程建模成显式状态机：

```rust
enum DiagnosisState {
    Plan,
    Execute,
    Replan,
    Finish,
    Failed,
}
```

核心对象建议拆成：

- `DiagnosisContext`: 一次诊断 run 的输入、Incident、历史步骤、证据、工具结果、预算、trace id。
- `Planner`: 基于告警和上下文生成步骤计划。
- `Executor`: 调用 metrics/logs/docs/time 等工具并结构化保存结果。
- `Replanner`: 根据已有证据决定继续、改计划或结束。
- `Reporter`: 生成最终诊断报告。
- `EvidenceRecorder`: 负责每一步证据落盘和审计。

这样可以保留当前 Java 版 AIOpsService、DiagnosisEvidenceRecorder、DiagnosisReportService 的清晰职责，同时避免把业务状态藏进某个 Agent 框架内部。

## Vector DB 选择

### 保守迁移: 继续 Milvus

适合目标是尽量复用现有 collection schema、部署文件和向量检索行为。Rust 侧可以优先评估 rig 文档中列出的 Milvus companion crate，必要时用 HTTP/gRPC 客户端薄封装项目内部接口。

### Rust 体验优先: 迁到 Qdrant

适合目标是减少 Rust 集成摩擦。Qdrant 的 Rust 客户端和部署体验通常更顺，向量 collection、payload filter、upsert/search 管线也适合当前 RAG 场景。

## 推荐 Cargo 依赖起点

```toml
[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.7", features = ["cors", "trace", "request-id", "timeout"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"] }
redis = { version = "0.32", features = ["tokio-comp", "connection-manager"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
prometheus = "0.14"
config = "0.15"
thiserror = "2"
anyhow = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
rig-core = "0.13"
```

版本号后续应以实际 `cargo update` 和 crate 兼容性为准。先把依赖面压在基础服务能力上，等 API 和服务骨架稳定后再接向量库、DashScope provider、工具调用协议。

## 分阶段落地计划

### Phase 1: 服务骨架

- 初始化 Cargo workspace。
- 建立 axum 路由、统一错误、配置加载、tracing。
- 实现 `/health`、`/metrics`、CORS、request id、timeout。
- 定义核心 domain model: Chat、Incident、DiagnosisRun、Evidence、DocumentChunk。

### Phase 2: 现有 API 对齐

- 迁移 Chat API、Incident API、Webhook API。
- 实现 SSE alert stream。
- 实现文件上传入口和本地/对象存储抽象。
- 对齐 Java 版 DTO 字段，降低前端和调用方改动。

### Phase 3: 存储与外部依赖

- 接 Redis session/cache。
- 接 Prometheus query client。
- 接日志服务 client。
- 接 DashScope chat/embedding/rerank client。
- 接 Milvus 或 Qdrant vector store。

### Phase 4: RAG

- 文档切分、embedding、index task status。
- vector search、rerank、internal docs tool。
- 保留对索引任务、chunk 配置和依赖降级的测试。

### Phase 5: AIOps Agent Loop

- 实现 planner/executor/replanner/reporter 状态机。
- 实现 metrics/logs/internal-docs/date-time tools。
- 落 DiagnosisEvidenceRecorder。
- 增加诊断 run 的 trace、预算、超时、失败恢复。

## 参考资料

- axum: <https://docs.rs/axum/latest/axum/>
- tokio: <https://docs.rs/tokio/latest/tokio/>
- tower-http: <https://docs.rs/tower-http/latest/tower_http/>
- serde: <https://docs.rs/serde/latest/serde/>
- redis: <https://docs.rs/redis/latest/redis/>
- reqwest: <https://docs.rs/reqwest/latest/reqwest/>
- sqlx: <https://docs.rs/sqlx/latest/sqlx/>
- prometheus: <https://docs.rs/prometheus/latest/prometheus/>
- metrics: <https://docs.rs/metrics/latest/metrics/>
- rig: <https://docs.rs/rig-core/latest/rig/>
- async-openai: <https://docs.rs/async-openai/latest/async_openai/>
