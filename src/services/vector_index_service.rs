use crate::{
    domain::document::DocumentChunk,
    error::AppError,
    services::{
        document_chunk_service::DocumentChunkService,
        milvus_service::{MilvusDocument, MilvusService},
        vector_embedding_service::{generate_sparse_vector, VectorEmbeddingService},
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct VectorIndexService {
    embedding_service: VectorEmbeddingService,
    chunk_service: DocumentChunkService,
    milvus_service: MilvusService,
}

impl VectorIndexService {
    pub fn new(
        embedding_service: VectorEmbeddingService,
        chunk_service: DocumentChunkService,
        milvus_service: MilvusService,
    ) -> Self {
        Self {
            embedding_service,
            chunk_service,
            milvus_service,
        }
    }

    pub async fn index_single_file(&self, file_path: impl AsRef<Path>) -> Result<(), AppError> {
        let path = file_path.as_ref();
        if !path.exists() || !path.is_file() {
            return Err(AppError::bad_request(format!(
                "文件不存在: {}",
                path.display()
            )));
        }

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| AppError::internal(format!("读取文件失败: {error}")))?;
        self.milvus_service.load_collection().await?;
        self.delete_existing_data(path).await?;

        let normalized_path = normalize_path(path);
        let chunks = self
            .chunk_service
            .chunk_document(&content, normalized_path.as_str());
        if chunks.is_empty() {
            return Ok(());
        }

        let contents = chunks
            .iter()
            .map(|chunk| chunk.content.clone())
            .collect::<Vec<_>>();
        let vectors = self
            .embedding_service
            .generate_embeddings(&contents)
            .await?;
        if vectors.len() != chunks.len() {
            return Err(AppError::internal(
                "批量 embedding 返回数量与分片数量不一致",
            ));
        }

        let documents = chunks
            .iter()
            .zip(vectors)
            .map(|(chunk, vector)| {
                let metadata = build_document_metadata(path, &normalized_path, chunk, chunks.len());
                MilvusDocument {
                    id: deterministic_id(&format!(
                        "{}_{}",
                        metadata["_source"].as_str().unwrap_or_default(),
                        chunk.chunk_index
                    )),
                    content: chunk.content.clone(),
                    vector,
                    sparse_vector: generate_sparse_vector(&chunk.content),
                    metadata,
                }
            })
            .collect::<Vec<_>>();

        self.milvus_service.insert(&documents).await
    }

    async fn delete_existing_data(&self, path: &Path) -> Result<(), AppError> {
        let normalized_path = normalize_path(path);
        let expr = format!(
            "metadata[\"_source\"] == \"{}\"",
            normalized_path.replace('\\', "\\\\").replace('"', "\\\"")
        );
        self.milvus_service.delete_by_expr(&expr).await
    }
}

fn build_document_metadata(
    path: &Path,
    normalized_path: &str,
    chunk: &DocumentChunk,
    total_chunks: usize,
) -> Value {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|name| name.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();

    let mut metadata = HashMap::new();
    metadata.insert("_source".to_string(), json!(normalized_path));
    metadata.insert("doc_type".to_string(), json!("document"));
    metadata.insert("_extension".to_string(), json!(extension));
    metadata.insert("_file_name".to_string(), json!(file_name));
    metadata.insert("chunkIndex".to_string(), json!(chunk.chunk_index));
    metadata.insert("totalChunks".to_string(), json!(total_chunks));
    if !chunk.title.is_empty() {
        metadata.insert("title".to_string(), json!(&chunk.title));
    }
    json!(metadata)
}

fn normalize_path(path: &Path) -> String {
    NormalizeLexically::normalize_lexically(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn deterministic_id(input: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes()).to_string()
}

trait NormalizeLexically {
    fn normalize_lexically(&self) -> PathBuf;
}

impl NormalizeLexically for Path {
    fn normalize_lexically(&self) -> PathBuf {
        let mut result = PathBuf::new();
        for component in self.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    result.pop();
                }
                other => result.push(other.as_os_str()),
            }
        }
        result
    }
}
