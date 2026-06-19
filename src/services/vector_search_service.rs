use crate::{
    config::AppConfig,
    domain::rag::{SearchResult, SearchTrace},
    error::AppError,
    services::{
        milvus_service::{HybridSearchRequest, MilvusService},
        vector_embedding_service::{generate_sparse_vector, VectorEmbeddingService},
        vector_rerank_service::VectorRerankService,
    },
};

// 普通知识库文档过滤条件，对齐 Java 版 MilvusConstants.DOCUMENT_FILTER_EXPR。
const DOCUMENT_FILTER_EXPR: &str = "metadata[\"doc_type\"] == \"document\"";
// 历史故障案例过滤条件，对齐 Java 版 MilvusConstants.INCIDENT_CASE_FILTER_EXPR。
const INCIDENT_CASE_FILTER_EXPR: &str = "metadata[\"doc_type\"] == \"incident_case\"";

#[derive(Clone)]
pub struct VectorSearchService {
    // 粗排候选数量。实际检索时会取 max(top_k, candidate_k)，给 rerank 留足候选。
    candidate_k: usize,
    // HNSW ef 参数，对齐 Java 版 rag.search-ef。
    search_ef: usize,
    // 负责生成 DashScope dense embedding。
    embedding_service: VectorEmbeddingService,
    // 负责调用 GTE-Rerank；失败时由 rerank service 自行降级为粗排 topK。
    rerank_service: VectorRerankService,
    // Milvus 访问适配层，封装 load/search/insert/delete 等底层调用。
    milvus_service: MilvusService,
}

impl VectorSearchService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            candidate_k: config.rag_candidate_k,
            search_ef: config.rag_search_ef,
            embedding_service: VectorEmbeddingService::new(config),
            rerank_service: VectorRerankService::new(config),
            milvus_service: MilvusService::new(config),
        }
    }

    pub fn embedding_service(&self) -> &VectorEmbeddingService {
        &self.embedding_service
    }

    pub fn milvus_service(&self) -> &MilvusService {
        &self.milvus_service
    }

    /// 搜索普通内部知识库文档。
    ///
    /// 对齐 Java 版 `searchSimilarDocuments`：只召回 `doc_type=document` 的数据。
    pub async fn search_similar_documents(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        Ok(self
            .search_with_filter(query, top_k, DOCUMENT_FILTER_EXPR, "相似文档")
            .await?
            .results)
    }

    /// 搜索普通内部知识库文档，并返回粗排、精排和检索参数。
    ///
    /// 用于 `/api/knowledge/search` 排查 RAG 命中质量，对齐 Java 版
    /// `explainSimilarDocuments`。
    pub async fn explain_similar_documents(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<SearchTrace, AppError> {
        self.search_with_filter(query, top_k, DOCUMENT_FILTER_EXPR, "相似文档")
            .await
    }

    /// 搜索当前会话的私人长期记忆。
    ///
    /// 记忆数据同样存放在 Milvus，通过 `doc_type=chat_memory` 和 `session_id`
    /// 两个 metadata 条件隔离不同会话。
    pub async fn search_session_memories(
        &self,
        query: &str,
        session_id: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        if session_id.trim().is_empty() {
            return Err(AppError::bad_request("sessionId 不能为空"));
        }
        Ok(self
            .search_with_filter(
                query,
                top_k,
                &chat_memory_filter_expr(session_id),
                "会话私人记忆",
            )
            .await?
            .results)
    }

    /// 搜索相似历史故障案例。
    ///
    /// 对齐 Java 版 `searchIncidentCases`：只召回 `doc_type=incident_case` 的数据。
    pub async fn search_incident_cases(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        Ok(self
            .search_with_filter(query, top_k, INCIDENT_CASE_FILTER_EXPR, "相似历史故障案例")
            .await?
            .results)
    }

    async fn search_with_filter(
        &self,
        query: &str,
        top_k: usize,
        filter_expr: &str,
        search_label: &str,
    ) -> Result<SearchTrace, AppError> {
        if query.trim().is_empty() {
            return Err(AppError::bad_request("query 不能为空"));
        }

        // Java 版会先用 candidateK 做粗排，再把粗排候选交给 rerank。
        // 这里保留同样策略：即使调用方 topK 很小，也至少取 candidate_k 个候选。
        let requested_top_k = top_k.max(1);
        let search_k = requested_top_k.max(self.candidate_k);

        // 混合检索需要 dense + sparse 两路向量：
        // dense 来自 DashScope embedding，sparse 使用与 Java 写入/查询一致的字符哈希词频。
        let dense_vector = self.embedding_service.generate_query_vector(query).await?;
        let sparse_vector = generate_sparse_vector(query);

        // Milvus 粗排：dense COSINE + sparse IP + RRF ranker 的具体细节由 adapter 封装。
        let mut candidates = self
            .milvus_service
            .hybrid_search(&HybridSearchRequest {
                dense_vector,
                sparse_vector,
                filter_expr: filter_expr.to_string(),
                top_k: search_k,
                search_ef: self.search_ef,
            })
            .await?;

        // 精排：rerank service 内部负责失败降级，调用方始终拿到一个可用结果列表。
        let results = self
            .rerank_service
            .rerank(query, &candidates, requested_top_k)
            .await;
        candidates.truncate(search_k);

        // SearchTrace 是给知识库检索解释接口用的，保留粗排候选、精排结果和关键参数。
        Ok(SearchTrace {
            query: query.to_string(),
            search_label: search_label.to_string(),
            requested_top_k,
            search_k,
            search_ef: self.search_ef,
            filter_expr: filter_expr.to_string(),
            candidates,
            results,
        })
    }
}

/// 构造会话私有记忆过滤条件。
///
/// session_id 来自外部请求，必须转义后再拼到 Milvus 表达式里。
pub fn chat_memory_filter_expr(session_id: &str) -> String {
    format!(
        "metadata[\"doc_type\"] == \"chat_memory\" && metadata[\"session_id\"] == \"{}\"",
        escape_expr_value(session_id)
    )
}

// Milvus filter 表达式使用双引号包裹字符串值，反斜杠和双引号都需要转义。
fn escape_expr_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::chat_memory_filter_expr;

    #[test]
    fn chat_memory_filter_escapes_session_id() {
        assert_eq!(
            chat_memory_filter_expr("session-\"1\""),
            "metadata[\"doc_type\"] == \"chat_memory\" && metadata[\"session_id\"] == \"session-\\\"1\\\"\""
        );
    }
}
