use crate::{
    domain::rag::{FileUploadRes, IndexTaskStatus},
    http::dto::ApiResponse,
    state::AppState,
};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::{path::PathBuf, sync::Arc};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/upload", post(upload))
        .route("/api/upload/status/{task_id}", get(get_upload_status))
}

async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> (StatusCode, Json<ApiResponse<FileUploadRes>>) {
    let mut file_name = None;
    let mut file_bytes = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        file_name = field.file_name().map(ToOwned::to_owned);
        match field.bytes().await {
            Ok(bytes) => file_bytes = Some(bytes),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::error(
                        400,
                        format!("读取上传文件失败: {error}"),
                    )),
                );
            }
        }
        break;
    }

    let Some(original_name) = file_name else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(400, "文件名不能为空")),
        );
    };
    let clean_name = clean_file_name(&original_name);
    if clean_name.is_empty() || clean_name.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(
                400,
                format!("非法的路径格式: {clean_name}"),
            )),
        );
    }
    if !is_allowed_extension(&clean_name, &state.config.upload_allowed_extensions) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(
                400,
                format!(
                    "不支持的文件格式，仅支持: {}",
                    state.config.upload_allowed_extensions.join(",")
                ),
            )),
        );
    }

    let Some(bytes) = file_bytes else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(400, "文件不能为空")),
        );
    };
    if bytes.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(400, "文件不能为空")),
        );
    }

    let upload_dir = PathBuf::from(&state.config.upload_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&state.config.upload_path));
    if let Err(error) = tokio::fs::create_dir_all(&upload_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(
                500,
                format!("创建上传目录失败: {error}"),
            )),
        );
    }

    let file_path = upload_dir.join(&clean_name);
    let absolute_upload_dir = upload_dir
        .canonicalize()
        .unwrap_or_else(|_| upload_dir.clone());
    let absolute_file_path = file_path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .unwrap_or_else(|| absolute_upload_dir.clone())
        .join(&clean_name);
    if !absolute_file_path.starts_with(&absolute_upload_dir) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(400, "不允许将文件上传到指定目录外")),
        );
    }

    if let Err(error) = tokio::fs::write(&absolute_file_path, &bytes).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(
                500,
                format!("保存上传文件失败: {error}"),
            )),
        );
    }

    let index_task = state
        .index_task_status_service
        .create_task(&clean_name, &absolute_file_path.to_string_lossy());
    let task_id = index_task.task_id.clone();
    let index_service = state.vector_index_service.clone();
    let status_service = state.index_task_status_service.clone();
    let index_path = absolute_file_path.clone();
    tokio::spawn(async move {
        status_service.mark_running(&task_id);
        match index_service.index_single_file(&index_path).await {
            Ok(()) => status_service.mark_completed(&task_id),
            Err(error) => status_service.mark_failed(&task_id, error.to_string()),
        }
    });

    (
        StatusCode::OK,
        Json(ApiResponse::success(FileUploadRes {
            file_name: clean_name,
            file_path: absolute_file_path.to_string_lossy().to_string(),
            size: bytes.len() as u64,
            task_id: index_task.task_id,
            status: "INDEXING".to_string(),
            message: "文件已接收，索引处理中".to_string(),
        })),
    )
}

async fn get_upload_status(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<IndexTaskStatus>>) {
    match state.index_task_status_service.get_status(&task_id) {
        Some(status) => (StatusCode::OK, Json(ApiResponse::success(status))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "索引任务不存在")),
        ),
    }
}

fn clean_file_name(value: &str) -> String {
    value
        .replace('\\', "/")
        .split('/')
        .next_back()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn is_allowed_extension(file_name: &str, allowed_extensions: &[String]) -> bool {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    allowed_extensions
        .iter()
        .any(|allowed| allowed == &extension)
}
