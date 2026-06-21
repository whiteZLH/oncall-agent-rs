use crate::{
    domain::{
        diagnosis::DiagnosisRun,
        incident::{ArchiveResult, IncidentRecord, IncidentSummary, SearchResult},
    },
    http::dto::ApiResponse,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/incidents", get(list_incidents))
        .route("/api/incidents/{incident_id}", get(get_incident))
        .route("/api/incidents/{incident_id}/runs", get(get_diagnosis_runs))
        .route(
            "/api/incidents/{incident_id}/diagnose",
            post(diagnose_incident),
        )
        .route(
            "/api/incidents/{incident_id}/archive-case",
            post(archive_case),
        )
        .route(
            "/api/incidents/{incident_id}/similar-cases",
            get(similar_cases),
        )
}

async fn list_incidents(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<IncidentSummary>>> {
    Json(ApiResponse::success(state.incident_service.list()))
}

async fn get_incident(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<IncidentRecord>>) {
    match state.incident_service.get(&incident_id) {
        Some(record) => (StatusCode::OK, Json(ApiResponse::success(record))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        ),
    }
}

async fn get_diagnosis_runs(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DiagnosisRun>>>) {
    match state.incident_service.runs(&incident_id) {
        Some(runs) => (StatusCode::OK, Json(ApiResponse::success(runs))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        ),
    }
}

async fn diagnose_incident(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<DiagnosisRun>>) {
    // 创建并入队诊断 run（对齐 Java createDiagnosisRun）
    let Some(run) = state.incident_service.diagnose(&incident_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        );
    };

    // 异步执行诊断，立即返回入队的 run（对齐 Java IncidentController 的 executor.execute）
    tokio::spawn(execute_diagnosis_run(
        Arc::clone(&state),
        incident_id,
        run.run_id.clone(),
        run.alert_context.clone(),
    ));

    (StatusCode::OK, Json(ApiResponse::success(run)))
}

/// 异步执行单次 Incident 诊断（对齐 Java `IncidentController.executeDiagnosisRun`）：
/// 标记运行中 → 调用带证据记录的 AI Ops → 写入经守卫的报告 / 失败。
/// 相似案例与指标趋势预取属后续批次，暂留 TODO，不阻塞 evidence 闭环。
async fn execute_diagnosis_run(
    state: Arc<AppState>,
    incident_id: String,
    run_id: String,
    alert_context: String,
) {
    info!(
        "开始执行 Incident 诊断, incidentId: {}, runId: {}",
        incident_id, run_id
    );
    state.incident_service.mark_running(&incident_id, &run_id);

    // TODO(后续批次): prefetchSimilarCasesContext / prefetchMetricTrendContext

    let collector = Arc::new(
        state
            .incident_service
            .evidence_collector(&incident_id, &run_id),
    );
    let context = if alert_context.trim().is_empty() {
        None
    } else {
        Some(alert_context.as_str())
    };
    let chat_model = state.ai_ops_service.chat_model();
    let tool_callbacks = state.ai_ops_service.tool_callbacks();

    match state
        .ai_ops_service
        .execute_ai_ops_analysis_with_run(
            &chat_model,
            &tool_callbacks,
            context,
            &incident_id,
            &run_id,
            Some(collector),
        )
        .await
    {
        Ok(Some(over_all_state)) => {
            let Some(report) = state.ai_ops_service.extract_final_report(&over_all_state) else {
                state.incident_service.fail_run(
                    &incident_id,
                    &run_id,
                    "多 Agent 流程完成，但未能提取最终报告",
                );
                return;
            };
            state
                .incident_service
                .complete_run(&incident_id, &run_id, &report);
            info!(
                "Incident 诊断完成, incidentId: {}, runId: {}",
                incident_id, run_id
            );
        }
        Ok(None) => {
            state.incident_service.fail_run(
                &incident_id,
                &run_id,
                "多 Agent 流程完成，但未能提取最终报告",
            );
        }
        Err(error) => {
            warn!(
                "Incident 诊断执行异常, incidentId: {}, runId: {}, error: {}",
                incident_id, run_id, error
            );
            state.incident_service.fail_run(
                &incident_id,
                &run_id,
                &format!("告警分析异常: {error}"),
            );
        }
    }
}

async fn archive_case(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<ArchiveResult>>) {
    match state.incident_service.archive_case(&incident_id).await {
        Ok(result) => (StatusCode::OK, Json(ApiResponse::success(result))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(500, error.to_string())),
        ),
    }
}

async fn similar_cases(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
    Query(params): Query<SimilarCasesQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<SearchResult>>>) {
    match state
        .incident_service
        .similar_cases(&incident_id, params.top_k.unwrap_or(3))
        .await
    {
        Ok(results) => (StatusCode::OK, Json(ApiResponse::success(results))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(500, error.to_string())),
        ),
    }
}

#[derive(Deserialize)]
struct SimilarCasesQuery {
    #[serde(rename = "topK", alias = "top_k")]
    top_k: Option<usize>,
}
