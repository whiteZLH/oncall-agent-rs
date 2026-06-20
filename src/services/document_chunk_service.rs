use crate::{config::AppConfig, domain::document::DocumentChunk};

#[derive(Clone)]
pub struct DocumentChunkService {
    max_size: usize,
    overlap: usize,
}

#[derive(Debug)]
struct Section {
    title: String,
    content: String,
    start_index: usize,
}

impl DocumentChunkService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            max_size: config.document_chunk_max_size,
            overlap: config.document_chunk_overlap,
        }
    }

    pub fn chunk_document(&self, content: &str, _file_path: &str) -> Vec<DocumentChunk> {
        if content.trim().is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let mut global_chunk_index = 0;
        for section in split_by_headings(content) {
            let section_chunks = self.chunk_section(&section, global_chunk_index);
            global_chunk_index += section_chunks.len() as i32;
            chunks.extend(section_chunks);
        }
        chunks
    }

    fn chunk_section(&self, section: &Section, start_chunk_index: i32) -> Vec<DocumentChunk> {
        let content = section.content.as_str();
        if content.chars().count() <= self.max_size {
            return vec![DocumentChunk {
                content: content.to_string(),
                start_index: section.start_index as i32,
                end_index: (section.start_index + content.len()) as i32,
                chunk_index: start_chunk_index,
                title: section.title.clone(),
            }];
        }

        let mut chunks = Vec::new();
        let paragraphs = split_by_paragraphs(content);
        let mut current_chunk = String::new();
        let mut current_start_index = section.start_index;
        let mut chunk_index = start_chunk_index;

        for paragraph in paragraphs {
            if !current_chunk.is_empty()
                && current_chunk.chars().count() + paragraph.chars().count() > self.max_size
            {
                let chunk_content = current_chunk.trim().to_string();
                chunks.push(DocumentChunk {
                    content: chunk_content.clone(),
                    start_index: current_start_index as i32,
                    end_index: (current_start_index + chunk_content.len()) as i32,
                    chunk_index,
                    title: section.title.clone(),
                });
                chunk_index += 1;

                let overlap = self.get_overlap_text(&chunk_content);
                current_start_index = current_start_index + chunk_content.len() - overlap.len();
                current_chunk = overlap;
            }

            current_chunk.push_str(&paragraph);
            current_chunk.push_str("\n\n");
        }

        if !current_chunk.is_empty() {
            let chunk_content = current_chunk.trim().to_string();
            chunks.push(DocumentChunk {
                content: chunk_content.clone(),
                start_index: current_start_index as i32,
                end_index: (current_start_index + chunk_content.len()) as i32,
                chunk_index,
                title: section.title.clone(),
            });
        }

        chunks
    }

    fn get_overlap_text(&self, text: &str) -> String {
        let chars = text.chars().collect::<Vec<_>>();
        let overlap_size = self.overlap.min(chars.len());
        if overlap_size == 0 {
            return String::new();
        }
        let overlap = chars[chars.len() - overlap_size..]
            .iter()
            .collect::<String>();
        let mut last_sentence_end = None;
        for (index, ch) in overlap.char_indices() {
            if matches!(ch, '。' | '？' | '！') {
                last_sentence_end = Some(index + ch.len_utf8());
            }
        }
        if let Some(index) = last_sentence_end {
            if index > overlap.len() / 2 {
                return overlap[index..].trim().to_string();
            }
        }
        overlap.trim().to_string()
    }
}

fn split_by_headings(content: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_title = String::new();
    let mut current_start = 0usize;
    let mut current = String::new();
    let mut byte_offset = 0usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if let Some(title) = markdown_heading_title(trimmed) {
            if !current.trim().is_empty() {
                sections.push(Section {
                    title: current_title.clone(),
                    content: current.trim().to_string(),
                    start_index: current_start,
                });
            }
            current_title = title.to_string();
            current_start = byte_offset;
            current.clear();
        }
        current.push_str(line);
        byte_offset += line.len();
    }

    if !current.trim().is_empty() {
        sections.push(Section {
            title: current_title,
            content: current.trim().to_string(),
            start_index: current_start,
        });
    }

    if sections.is_empty() {
        sections.push(Section {
            title: String::new(),
            content: content.to_string(),
            start_index: 0,
        });
    }
    sections
}

fn markdown_heading_title(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        Some(line[hashes + 1..].trim())
    } else {
        None
    }
}

fn split_by_paragraphs(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::DocumentChunkService;
    use crate::config::AppConfig;
    use std::{net::Ipv4Addr, time::Duration};

    fn config() -> AppConfig {
        AppConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 3000,
            allowed_origin: "*".to_string(),
            request_timeout: Duration::from_secs(30),
            log_filter: "info".to_string(),
            static_dir: "./static".to_string(),
            redis_url: None,
            chat_history_path: "./target/test-chat-history".to_string(),
            session_ttl_secs: 3600,
            dashscope_api_key: None,
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            dashscope_api_base_url: "https://dashscope.aliyuncs.com/api/v1".to_string(),
            dashscope_responses_rectifier_enabled: false,
            dashscope_chat_model: "qwen-plus".to_string(),
            chat_agent_max_turns: 6,
            dashscope_embedding_model: "text-embedding-v4".to_string(),
            dashscope_rerank_model: "gte-rerank".to_string(),
            dashscope_rerank_url:
                "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                    .to_string(),
            milvus_host: "localhost".to_string(),
            milvus_port: 19530,
            milvus_username: String::new(),
            milvus_password: String::new(),
            milvus_database: "default".to_string(),
            milvus_timeout_ms: 10_000,
            rag_candidate_k: 10,
            rag_search_ef: 64,
            upload_path: "./target/uploads".to_string(),
            upload_allowed_extensions: vec!["txt".to_string(), "md".to_string()],
            document_chunk_max_size: 20,
            document_chunk_overlap: 5,
            private_memory_recall_enabled: true,
            private_memory_recall_top_k: 3,
            private_memory_store_path: "./target/test-private-memories".to_string(),
            prometheus_base_url: "http://localhost:9090".to_string(),
            prometheus_timeout_secs: 10,
            prometheus_mock_enabled: true,
            cls_mock_enabled: true,
            ai_ops_chat_model: "qwen-plus".to_string(),
            ai_ops_agent_max_turns: 12,
            ai_ops_max_rounds: 8,
        }
    }

    #[test]
    fn chunk_document_keeps_markdown_title() {
        let service = DocumentChunkService::new(&config());
        let chunks = service.chunk_document("# CPU\n\n第一段\n\n第二段", "runbook.md");

        assert_eq!(chunks[0].title, "CPU");
        assert!(chunks[0].content.contains("# CPU"));
    }
}
