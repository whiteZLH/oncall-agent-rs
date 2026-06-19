use crate::{config::AppConfig, domain::rag::SearchResult, error::AppError};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, time::Duration};

pub const MILVUS_COLLECTION_NAME: &str = "biz";
pub const FIELD_ID: &str = "id";
pub const FIELD_CONTENT: &str = "content";
pub const FIELD_VECTOR: &str = "vector";
pub const FIELD_SPARSE_VECTOR: &str = "sparse_vector";
pub const FIELD_METADATA: &str = "metadata";

#[derive(Clone)]
pub struct MilvusService {
    endpoint: String,
    database: String,
    username: String,
    password: String,
    client: Client,
}

#[derive(Debug, Clone)]
pub struct MilvusDocument {
    pub id: String,
    pub content: String,
    pub vector: Vec<f32>,
    pub sparse_vector: BTreeMap<i64, f32>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct HybridSearchRequest {
    pub dense_vector: Vec<f32>,
    pub sparse_vector: BTreeMap<i64, f32>,
    pub filter_expr: String,
    pub top_k: usize,
    pub search_ef: usize,
}

#[derive(Debug, Deserialize)]
struct MilvusResponse {
    #[serde(default)]
    code: Value,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Value,
}

impl MilvusService {
    pub fn new(config: &AppConfig) -> Self {
        let scheme = if config.milvus_host.starts_with("http://")
            || config.milvus_host.starts_with("https://")
        {
            String::new()
        } else {
            "http://".to_string()
        };
        let endpoint = format!(
            "{}{}:{}",
            scheme,
            config.milvus_host.trim_end_matches('/'),
            config.milvus_port
        );
        let client = Client::builder()
            .timeout(Duration::from_millis(config.milvus_timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            endpoint,
            database: config.milvus_database.clone(),
            username: config.milvus_username.clone(),
            password: config.milvus_password.clone(),
            client,
        }
    }

    pub async fn load_collection(&self) -> Result<(), AppError> {
        let body = json!({
            "dbName": self.database,
            "collectionName": MILVUS_COLLECTION_NAME,
        });
        self.post_ok("/v2/vectordb/collections/load", &body).await
    }

    pub async fn delete_by_expr(&self, expr: &str) -> Result<(), AppError> {
        let body = json!({
            "dbName": self.database,
            "collectionName": MILVUS_COLLECTION_NAME,
            "filter": expr,
        });
        self.post_ok("/v2/vectordb/entities/delete", &body).await
    }

    pub async fn insert(&self, documents: &[MilvusDocument]) -> Result<(), AppError> {
        if documents.is_empty() {
            return Ok(());
        }

        let rows = documents
            .iter()
            .map(document_to_row)
            .collect::<Vec<Value>>();
        let body = json!({
            "dbName": self.database,
            "collectionName": MILVUS_COLLECTION_NAME,
            "data": rows,
        });
        self.post_ok("/v2/vectordb/entities/insert", &body).await
    }

    pub async fn hybrid_search(
        &self,
        request: &HybridSearchRequest,
    ) -> Result<Vec<SearchResult>, AppError> {
        let body = json!({
            "dbName": self.database,
            "collectionName": MILVUS_COLLECTION_NAME,
            "search": [
                {
                    "annsField": FIELD_VECTOR,
                    "data": [request.dense_vector],
                    "metricType": "COSINE",
                    "filter": request.filter_expr,
                    "limit": request.top_k,
                    "params": { "ef": request.search_ef },
                },
                {
                    "annsField": FIELD_SPARSE_VECTOR,
                    "data": [sparse_to_json(&request.sparse_vector)],
                    "metricType": "IP",
                    "filter": request.filter_expr,
                    "limit": request.top_k,
                }
            ],
            "ranker": { "strategy": "rrf", "params": { "k": 60 } },
            "limit": request.top_k,
            "outputFields": [FIELD_ID, FIELD_CONTENT, FIELD_METADATA],
        });

        let response = self
            .post_json("/v2/vectordb/entities/hybrid_search", &body)
            .await?;
        parse_search_results(&response.data)
    }

    async fn post_ok(&self, path: &str, body: &Value) -> Result<(), AppError> {
        let response = self.post_json(path, body).await?;
        if is_success_code(&response.code) {
            Ok(())
        } else {
            Err(AppError::internal(format!(
                "Milvus 请求失败: {}",
                response.message
            )))
        }
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<MilvusResponse, AppError> {
        let url = format!("{}{}", self.endpoint, path);
        let mut request = self.client.post(url).json(body);
        if !self.username.is_empty() || !self.password.is_empty() {
            request = request.basic_auth(&self.username, Some(&self.password));
        }
        let response = request
            .send()
            .await
            .map_err(|error| AppError::internal(format!("Milvus 请求失败: {error}")))?;

        let status = response.status();
        let payload: MilvusResponse = response
            .json()
            .await
            .map_err(|error| AppError::internal(format!("解析 Milvus 响应失败: {error}")))?;
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "Milvus 请求失败: HTTP {status}, {}",
                payload.message
            )));
        }
        Ok(payload)
    }
}

fn document_to_row(document: &MilvusDocument) -> Value {
    json!({
        FIELD_ID: document.id,
        FIELD_CONTENT: document.content,
        FIELD_VECTOR: document.vector,
        FIELD_SPARSE_VECTOR: sparse_to_json(&document.sparse_vector),
        FIELD_METADATA: document.metadata,
    })
}

fn sparse_to_json(sparse: &BTreeMap<i64, f32>) -> Value {
    let mut object = Map::new();
    for (key, value) in sparse {
        object.insert(key.to_string(), json!(value));
    }
    Value::Object(object)
}

fn parse_search_results(data: &Value) -> Result<Vec<SearchResult>, AppError> {
    let rows = if let Some(array) = data.as_array() {
        array
    } else if let Some(array) = data["results"].as_array() {
        array
    } else {
        return Err(AppError::internal("Milvus 搜索响应缺少结果数组"));
    };

    let mut results = Vec::new();
    for row in rows {
        let id = row[FIELD_ID]
            .as_str()
            .or_else(|| row["id"].as_str())
            .unwrap_or_default()
            .to_string();
        let content = row[FIELD_CONTENT].as_str().unwrap_or_default().to_string();
        let score = row["distance"]
            .as_f64()
            .or_else(|| row["score"].as_f64())
            .unwrap_or_default() as f32;
        let metadata_value = row.get(FIELD_METADATA).cloned().unwrap_or(Value::Null);
        let metadata = match metadata_value {
            Value::String(value) => value,
            Value::Null => "{}".to_string(),
            other => other.to_string(),
        };
        results.push(SearchResult {
            id,
            content,
            score,
            metadata,
        });
    }

    Ok(results)
}

fn is_success_code(code: &Value) -> bool {
    match code {
        Value::Number(number) => number.as_i64() == Some(0) || number.as_i64() == Some(200),
        Value::String(value) => value == "0" || value.eq_ignore_ascii_case("success"),
        Value::Null => true,
        _ => false,
    }
}
