use bytes::Bytes;
use futures_util::StreamExt;
use rig::{
    http_client::{
        self, sse::BoxedStream, HttpClientExt, LazyBody, MultipartForm, Request, ReqwestClient,
        Response, StreamingResponse,
    },
    wasm_compat::WasmCompatSend,
};
use serde_json::{Map, Value};
use std::{
    collections::hash_map::DefaultHasher,
    future::Future,
    hash::{Hash, Hasher},
};
use tracing::warn;

#[derive(Clone, Debug)]
pub struct ResponsesRectifierHttpClient<H = ReqwestClient> {
    inner: H,
    enabled: bool,
}

impl ResponsesRectifierHttpClient<ReqwestClient> {
    pub fn new(enabled: bool) -> Self {
        Self::from_inner(ReqwestClient::default(), enabled)
    }
}

impl<H> ResponsesRectifierHttpClient<H> {
    pub fn from_inner(inner: H, enabled: bool) -> Self {
        Self { inner, enabled }
    }
}

impl Default for ResponsesRectifierHttpClient<ReqwestClient> {
    fn default() -> Self {
        Self::new(false)
    }
}

impl<H> HttpClientExt for ResponsesRectifierHttpClient<H>
where
    H: HttpClientExt + Clone + Default + std::fmt::Debug + WasmCompatSend + 'static,
{
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes>,
        T: WasmCompatSend,
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        let should_rectify = self.enabled && is_responses_request(&req);
        let req = req.map(Into::into);
        let inner = self.inner.clone();

        async move {
            if !should_rectify {
                return inner.send::<Bytes, U>(req).await;
            }

            let response = inner.send::<Bytes, Bytes>(req).await?;
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.into_body().await?;
            let rectified = rectify_response_bytes(bytes);
            let body: LazyBody<U> = Box::pin(async move { Ok(U::from(rectified)) });

            let mut builder = Response::builder().status(status);
            if let Some(response_headers) = builder.headers_mut() {
                *response_headers = headers;
            }

            builder.body(body).map_err(http_client::Error::Protocol)
        }
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        self.inner.send_multipart(req)
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let should_rectify = self.enabled && is_responses_request(&req);
        let req = req.map(Into::into);
        let inner = self.inner.clone();

        async move {
            let response = inner.send_streaming(req).await?;
            if !should_rectify {
                return Ok(response);
            }

            Ok(rectify_streaming_response(response))
        }
    }
}

fn is_responses_request<T>(req: &Request<T>) -> bool {
    req.method() == rig::http_client::Method::POST && req.uri().path().ends_with("/responses")
}

fn rectify_response_bytes(bytes: Bytes) -> Bytes {
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return bytes;
    };

    let seed = String::from_utf8_lossy(&bytes);
    rectify_response_value(&mut value, &seed);

    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(bytes)
}

fn rectify_response_value(value: &mut Value, seed: &str) {
    if !value.is_object() {
        return;
    }

    let response_id = ensure_response_id(value, seed);
    ensure_response_status(value);

    let Some(output) = value.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };

    for (index, item) in output.iter_mut().enumerate() {
        rectify_output_item(item, &response_id, index);
    }
}

fn rectify_output_item(item: &mut Value, response_id: &str, output_index: usize) {
    let Some(object) = item.as_object_mut() else {
        return;
    };

    match object.get("type").and_then(Value::as_str) {
        Some("message") => rectify_message_item(object, response_id, output_index),
        Some("function_call") => rectify_function_call_item(object, response_id, output_index),
        _ => {}
    }
}

fn rectify_message_item(object: &mut Map<String, Value>, response_id: &str, output_index: usize) {
    ensure_string_field(
        object,
        "id",
        format!("msg_rect_{response_id}_{output_index}"),
        "OpenAI Responses rectifier synthesized missing output message id",
    );
    ensure_string_field(
        object,
        "role",
        "assistant",
        "OpenAI Responses rectifier synthesized missing output message role",
    );
    ensure_string_field(
        object,
        "status",
        "completed",
        "OpenAI Responses rectifier synthesized missing output message status",
    );

    if !object.contains_key("content") || object.get("content").is_some_and(Value::is_null) {
        warn!("OpenAI Responses rectifier synthesized missing output message content");
        object.insert("content".to_string(), Value::Array(Vec::new()));
    }
}

fn rectify_function_call_item(
    object: &mut Map<String, Value>,
    response_id: &str,
    output_index: usize,
) {
    ensure_string_field(
        object,
        "id",
        format!("fc_rect_{response_id}_{output_index}"),
        "OpenAI Responses rectifier synthesized missing function_call id",
    );
    ensure_string_field(
        object,
        "call_id",
        format!("call_rect_{response_id}_{output_index}"),
        "OpenAI Responses rectifier synthesized missing function_call call_id",
    );
    ensure_string_field(
        object,
        "status",
        "completed",
        "OpenAI Responses rectifier synthesized missing function_call status",
    );
}

fn ensure_response_id(value: &mut Value, seed: &str) -> String {
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        if !id.is_empty() {
            return id.to_string();
        }
    }

    let id = format!("resp_rect_{}", stable_hash(seed));
    warn!("OpenAI Responses rectifier synthesized missing top-level response id");
    if let Some(object) = value.as_object_mut() {
        object.insert("id".to_string(), Value::String(id.clone()));
    }
    id
}

fn ensure_response_status(value: &mut Value) {
    let has_status = value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| !status.is_empty());

    if !has_status {
        warn!("OpenAI Responses rectifier synthesized missing top-level response status");
        if let Some(object) = value.as_object_mut() {
            object.insert("status".to_string(), Value::String("completed".to_string()));
        }
    }
}

fn ensure_string_field(
    object: &mut Map<String, Value>,
    field: &str,
    value: impl Into<String>,
    warning: &str,
) {
    let has_value = object
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|item| !item.is_empty());

    if !has_value {
        warn!("{warning}");
        object.insert(field.to_string(), Value::String(value.into()));
    }
}

fn stable_hash(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn rectify_streaming_response(response: StreamingResponse) -> StreamingResponse {
    let (parts, mut body) = response.into_parts();
    let stream: BoxedStream = Box::pin(async_stream::try_stream! {
        let mut buffer = SseRectifierBuffer::default();

        while let Some(chunk) = body.next().await {
            let bytes = chunk?;
            let text = String::from_utf8_lossy(&bytes);

            for rectified in buffer.push_text(&text) {
                yield Bytes::from(rectified);
            }
        }

        if let Some(rectified) = buffer.finish() {
            yield Bytes::from(rectified);
        }
    });

    Response::from_parts(parts, stream)
}

#[derive(Default)]
struct SseRectifierBuffer {
    buffer: String,
}

impl SseRectifierBuffer {
    fn push_text(&mut self, text: &str) -> Vec<String> {
        self.buffer.push_str(text);
        let mut output = Vec::new();

        while let Some((index, separator)) = find_event_separator(&self.buffer) {
            let event = self.buffer[..index].to_string();
            self.buffer.drain(..index + separator.len());

            let mut rectified = rectify_sse_event(&event);
            rectified.push_str(separator);
            output.push(rectified);
        }

        output
    }

    fn finish(self) -> Option<String> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(rectify_sse_event(&self.buffer))
        }
    }
}

fn find_event_separator(input: &str) -> Option<(usize, &'static str)> {
    match (input.find("\r\n\r\n"), input.find("\n\n")) {
        (Some(crlf), Some(lf)) if crlf < lf => Some((crlf, "\r\n\r\n")),
        (Some(_), Some(lf)) => Some((lf, "\n\n")),
        (Some(crlf), None) => Some((crlf, "\r\n\r\n")),
        (None, Some(lf)) => Some((lf, "\n\n")),
        (None, None) => None,
    }
}

fn rectify_sse_event(event: &str) -> String {
    let mut output = String::new();

    for line in event.split_inclusive('\n') {
        output.push_str(&rectify_sse_line(line));
    }

    if !event.ends_with('\n') {
        return output;
    }

    output
}

fn rectify_sse_line(line: &str) -> String {
    let (line_body, line_ending) = split_line_ending(line);
    let Some(data) = line_body.strip_prefix("data:") else {
        return line.to_string();
    };

    let leading_space = if data.starts_with(' ') { " " } else { "" };
    let payload = data.trim_start();
    if payload.is_empty() || payload == "[DONE]" {
        return line.to_string();
    }

    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return line.to_string();
    };

    rectify_sse_value(&mut value, payload);
    let Ok(serialized) = serde_json::to_string(&value) else {
        return line.to_string();
    };

    format!("data:{leading_space}{serialized}{line_ending}")
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

fn rectify_sse_value(value: &mut Value, seed: &str) {
    match value.get("type").and_then(Value::as_str) {
        Some("response.output_item.added") | Some("response.output_item.done") => {
            let response_id = value
                .get("response_id")
                .and_then(Value::as_str)
                .unwrap_or("stream")
                .to_string();
            let output_index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;

            if let Some(item) = value.get_mut("item") {
                rectify_output_item(item, &response_id, output_index);
            }
        }
        Some("response.completed") | Some("response.incomplete") | Some("response.failed") => {
            if let Some(response) = value.get_mut("response") {
                rectify_response_value(response, seed);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::providers::openai::responses_api::CompletionResponse;
    use serde_json::json;

    fn valid_response_with_output(output: Value) -> Value {
        json!({
            "id": "resp_test",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "error": null,
            "incomplete_details": null,
            "instructions": null,
            "max_output_tokens": null,
            "model": "gpt-test",
            "usage": null,
            "output": [output]
        })
    }

    #[test]
    fn rectifies_missing_message_fields() {
        let mut response = valid_response_with_output(json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "hello"}]
        }));

        rectify_response_value(&mut response, "seed");

        let message = &response["output"][0];
        assert_eq!(message["id"], "msg_rect_resp_test_0");
        assert_eq!(message["status"], "completed");
        serde_json::from_value::<CompletionResponse>(response)
            .expect("rectified response should deserialize");
    }

    #[test]
    fn rectifies_missing_function_call_fields() {
        let mut response = valid_response_with_output(json!({
            "type": "function_call",
            "name": "getCurrentDateTime",
            "arguments": "{}"
        }));

        rectify_response_value(&mut response, "seed");

        let call = &response["output"][0];
        assert_eq!(call["id"], "fc_rect_resp_test_0");
        assert_eq!(call["call_id"], "call_rect_resp_test_0");
        assert_eq!(call["status"], "completed");
        serde_json::from_value::<CompletionResponse>(response)
            .expect("rectified function call should deserialize");
    }

    #[test]
    fn leaves_compliant_response_unchanged() {
        let original = valid_response_with_output(json!({
            "type": "message",
            "id": "msg_real",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "hello"}]
        }));
        let mut response = original.clone();

        rectify_response_value(&mut response, "seed");

        assert_eq!(response, original);
    }

    #[test]
    fn rectifies_missing_top_level_fields() {
        let mut response = valid_response_with_output(json!({
            "type": "message",
            "content": []
        }));
        response.as_object_mut().unwrap().remove("id");
        response.as_object_mut().unwrap().remove("status");

        rectify_response_value(&mut response, "top-seed");

        assert!(response["id"].as_str().unwrap().starts_with("resp_rect_"));
        assert_eq!(response["status"], "completed");
        assert!(response["output"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("msg_rect_resp_rect_"));
    }

    #[test]
    fn rectifies_sse_output_item_done() {
        let event = concat!(
            "event: message\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,",
            "\"item\":{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"hi\"}]}}\n"
        );

        let rectified = rectify_sse_event(event);
        let data = rectified
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap();
        let value: Value = serde_json::from_str(data).unwrap();

        assert_eq!(value["item"]["id"], "msg_rect_stream_0");
        assert_eq!(value["item"]["status"], "completed");
    }

    #[test]
    fn rectifies_sse_completed_response() {
        let event = concat!(
            "data: {\"type\":\"response.completed\",\"response\":{",
            "\"object\":\"response\",\"created_at\":0,\"model\":\"gpt-test\",",
            "\"error\":null,\"incomplete_details\":null,\"instructions\":null,",
            "\"max_output_tokens\":null,\"usage\":null,",
            "\"output\":[{\"type\":\"message\",\"content\":[]}]}}\n"
        );

        let rectified = rectify_sse_event(event);
        let data = rectified.strip_prefix("data: ").unwrap().trim();
        let value: Value = serde_json::from_str(data).unwrap();

        assert_eq!(value["response"]["status"], "completed");
        assert!(value["response"]["output"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("msg_rect_resp_rect_"));
    }

    #[test]
    fn sse_buffer_handles_cross_chunk_events() {
        let mut buffer = SseRectifierBuffer::default();

        assert!(buffer
            .push_text("data: {\"type\":\"response.output_item.done\",")
            .is_empty());
        let output = buffer.push_text(
            "\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"tool\",\"arguments\":\"{}\"}}\n\n",
        );

        assert_eq!(output.len(), 1);
        assert!(output[0].contains("\"call_id\":\"call_rect_stream_0\""));
        assert!(output[0].ends_with("\n\n"));
    }

    #[test]
    fn sse_done_and_non_json_data_pass_through() {
        assert_eq!(rectify_sse_line("data: [DONE]\n"), "data: [DONE]\n");
        assert_eq!(rectify_sse_line("data: not-json\n"), "data: not-json\n");
    }
}
