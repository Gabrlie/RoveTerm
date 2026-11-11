//! The model client of the built-in assistant: one streaming completion with function tools
//! against a provider's base URL, in one of three wire formats — OpenAI Chat Completions, OpenAI
//! Responses and Anthropic Messages. The API key goes only into a request header; it is never
//! logged, and error texts that could quote it are scrubbed.

use std::time::Duration;

use futures::StreamExt;
use rove_http::reqwest::{self, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

/// Largest response body read from one completion.
const MAX_BODY: usize = 8 * 1024 * 1024;
/// `max_tokens` an Anthropic request sends when none is configured (the format requires one).
pub const ANTHROPIC_MAX_TOKENS: u32 = 32_000;

/// How a provider is spoken to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// OpenAI Chat Completions (`/chat/completions`), which most providers also speak.
    #[default]
    Chat,
    /// OpenAI Responses (`/responses`).
    Responses,
    /// Anthropic Messages (`/messages`).
    Anthropic,
}

/// How much the model should reason, where the format can set it; each format takes the
/// nearest level it knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl Effort {
    /// OpenAI's `reasoning_effort` / `reasoning.effort`: `minimal` to `xhigh`.
    fn openai(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh | Self::Max => "xhigh",
        }
    }

    /// Anthropic's `output_config.effort`: `low` to `max`.
    fn anthropic(self) -> &'static str {
        match self {
            Self::Minimal | Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// Where and how to reach a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub format: Format,
    /// Base URL including the API version path, such as `https://api.openai.com/v1`.
    pub base_url: String,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub effort: Option<Effort>,
}

/// A function the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments.
    pub parameters: Value,
}

/// One function call the model made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as JSON text, exactly as the model produced them.
    pub arguments: String,
}

/// One message of a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        text: String,
    },
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
        /// What the provider needs back verbatim (Anthropic's signed thinking blocks).
        opaque: Option<Value>,
    },
    /// The result of `call_id`.
    Tool {
        call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
}

/// A piece of the reply as it streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Text(String),
    Thought(String),
}

/// The whole reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub opaque: Option<Value>,
    /// Why the model stopped, in the provider's words (`stop`, `end_turn`, `tool_use`, …).
    pub stop: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum LlmError {
    #[error("the provider address must be an http or https URL")]
    InvalidUrl,
    /// The provider refused the request.
    #[error("the provider answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not reach the provider: {0}")]
    Network(String),
    #[error("the provider stopped answering")]
    Timeout,
    #[error("the reply is too large")]
    TooLarge,
    #[error("the provider sent something unexpected: {0}")]
    Protocol(String),
}

/// A client for one endpoint.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: Endpoint,
    base: Url,
    key: Zeroizing<String>,
    /// How long to wait for the reply to start and between two pieces of it.
    idle: Duration,
    /// The model turned down the reasoning effort; later requests leave it out. Shared by the
    /// clones of one client, reset when the model or effort changes.
    effort_refused: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Whether a 400 reply turns down the reasoning settings (the model has none).
fn refuses_effort(status: u16, message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    status == 400 && ["reasoning", "effort", "thinking"].iter().any(|word| message.contains(word))
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

impl Client {
    /// A client for `endpoint` with API key `key` (empty for providers that need none).
    pub fn new(endpoint: Endpoint, key: &str, idle: Duration) -> Result<Self, LlmError> {
        let trimmed = endpoint.base_url.trim().trim_end_matches('/');
        let base = Url::parse(trimmed).map_err(|_| LlmError::InvalidUrl)?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || base.query().is_some()
            || !base.username().is_empty()
        {
            return Err(LlmError::InvalidUrl);
        }
        // Redirects only within the provider's origin, so the key never follows one elsewhere.
        let origin = base.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() < 5 && same_origin(attempt.url(), &origin) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let http = rove_http::builder(None)
            .map_err(|error| LlmError::Network(error.to_string()))?
            .redirect(policy)
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| LlmError::Network(error.to_string()))?;
        Ok(Self {
            http,
            endpoint,
            base,
            key: Zeroizing::new(key.to_owned()),
            idle,
            effort_refused: Default::default(),
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The same client reasoning at `effort` (`None`: the model's default).
    pub fn with_effort(&self, effort: Option<Effort>) -> Self {
        let mut client = self.clone();
        client.endpoint.effort = effort;
        client.effort_refused = Default::default();
        client
    }

    /// The same client asking `model` instead.
    pub fn with_model(&self, model: &str) -> Self {
        let mut client = self.clone();
        model.clone_into(&mut client.endpoint.model);
        client.effort_refused = Default::default();
        client
    }

    /// The model turned down the reasoning effort, which is left out since.
    pub fn effort_refused(&self) -> bool {
        self.effort_refused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Removes the key from text that may quote the request.
    fn scrub(&self, text: &str) -> String {
        let mut text = text.to_owned();
        if self.key.len() >= 4 {
            text = text.replace(self.key.as_str(), "***");
        }
        if text.len() > 2000 {
            let mut end = 2000;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        text
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base.as_str().trim_end_matches('/'))
    }

    /// `request` with the key and the format's headers.
    fn authed(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if !self.key.is_empty() {
            request = match self.endpoint.format {
                Format::Anthropic => request.header("x-api-key", self.key.as_str()),
                _ => request.bearer_auth(self.key.as_str()),
            };
        }
        if self.endpoint.format == Format::Anthropic {
            request = request.header("anthropic-version", "2023-06-01");
        }
        request
    }

    /// The models the provider offers (`GET /models`: `data[].id`, paged on Anthropic), sorted
    /// and without repeats.
    pub async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let mut models: Vec<String> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..20 {
            let mut url = Url::parse(&self.url("/models")).map_err(|_| LlmError::InvalidUrl)?;
            if self.endpoint.format == Format::Anthropic {
                let mut query = url.query_pairs_mut();
                query.append_pair("limit", "1000");
                if let Some(after) = &after {
                    query.append_pair("after_id", after);
                }
            }
            let response = tokio::time::timeout(self.idle, self.authed(self.http.get(url)).send())
                .await
                .map_err(|_| LlmError::Timeout)?
                .map_err(|error| LlmError::Network(self.scrub(&error.without_url().to_string())))?;
            let status = response.status();
            let bytes = tokio::time::timeout(self.idle, response.bytes())
                .await
                .map_err(|_| LlmError::Timeout)?
                .map_err(|error| LlmError::Network(self.scrub(&error.without_url().to_string())))?;
            if bytes.len() > MAX_BODY {
                return Err(LlmError::TooLarge);
            }
            let text = String::from_utf8_lossy(&bytes);
            if !status.is_success() {
                return Err(LlmError::Http {
                    status: status.as_u16(),
                    message: self.scrub(&error_message(&text)),
                });
            }
            let page: Value = serde_json::from_str(&text)
                .map_err(|_| LlmError::Protocol("the model list is not JSON".into()))?;
            let ids = page
                .get("data")
                .or_else(|| page.get("models"))
                .and_then(Value::as_array)
                .ok_or_else(|| LlmError::Protocol("the model list has no data".into()))?;
            models.extend(
                ids.iter()
                    .filter_map(|model| model.get("id").or_else(|| model.get("name")))
                    .filter_map(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 200)
                    .map(str::to_owned),
            );
            let more = page.get("has_more").and_then(Value::as_bool) == Some(true);
            after = page
                .get("last_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if !more || after.is_none() {
                break;
            }
        }
        models.sort();
        models.dedup();
        Ok(models)
    }

    /// Streams one completion of `messages` after `system`, offering `tools`; `on_delta` sees the
    /// text and reasoning as they arrive. Dropping the future cancels the request. A model that
    /// turns down the reasoning effort is asked again without it, and not given it again.
    pub async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Reply, LlmError> {
        let mut endpoint = self.endpoint.clone();
        if self.effort_refused() {
            endpoint.effort = None;
        }
        match self
            .complete_at(&endpoint, system, messages, tools, on_delta)
            .await
        {
            Err(LlmError::Http { status, message })
                if endpoint.effort.is_some() && refuses_effort(status, &message) =>
            {
                self.effort_refused
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                endpoint.effort = None;
                self.complete_at(&endpoint, system, messages, tools, on_delta)
                    .await
            }
            other => other,
        }
    }

    async fn complete_at(
        &self,
        endpoint: &Endpoint,
        system: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Reply, LlmError> {
        let (path, body) = match endpoint.format {
            Format::Chat => (
                "/chat/completions",
                chat_body(endpoint, system, messages, tools),
            ),
            Format::Responses => (
                "/responses",
                responses_body(endpoint, system, messages, tools),
            ),
            Format::Anthropic => (
                "/messages",
                anthropic_body(endpoint, system, messages, tools),
            ),
        };
        let request = self.authed(
            self.http
                .post(self.url(path))
                .header("content-type", "application/json")
                .body(body.to_string())
                .header("accept", "text/event-stream"),
        );
        let response = tokio::time::timeout(self.idle, request.send())
            .await
            .map_err(|_| LlmError::Timeout)?
            .map_err(|error| LlmError::Network(self.scrub(&error.without_url().to_string())))?;
        let status = response.status();
        if !status.is_success() {
            let text = tokio::time::timeout(self.idle, response.text())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            return Err(LlmError::Http {
                status: status.as_u16(),
                message: self.scrub(&error_message(&text)),
            });
        }
        let mut stream = response.bytes_stream();
        let mut parser = SseParser::default();
        let mut state = StreamState::new(self.endpoint.format);
        let mut total = 0usize;
        loop {
            let chunk = match tokio::time::timeout(self.idle, stream.next()).await {
                Err(_) => return Err(LlmError::Timeout),
                Ok(None) => break,
                Ok(Some(Err(error))) => {
                    return Err(LlmError::Network(
                        self.scrub(&error.without_url().to_string()),
                    ));
                }
                Ok(Some(Ok(chunk))) => chunk,
            };
            total += chunk.len();
            if total > MAX_BODY {
                return Err(LlmError::TooLarge);
            }
            for event in parser.push(&chunk) {
                if state.event(&event, on_delta).map_err(|error| match error {
                    LlmError::Protocol(text) => LlmError::Protocol(self.scrub(&text)),
                    other => other,
                })? {
                    return Ok(state.finish());
                }
            }
        }
        if !state.finished {
            return Err(LlmError::Protocol("the reply ended early".into()));
        }
        Ok(state.finish())
    }
}

/// The `message` of a provider's JSON error body, or the body itself.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.trim().to_owned())
}

// ---- request bodies ------------------------------------------------------------------------

fn chat_body(endpoint: &Endpoint, system: &str, messages: &[Message], tools: &[ToolSpec]) -> Value {
    let mut list = vec![json!({ "role": "system", "content": system })];
    for message in messages {
        list.push(match message {
            Message::User { text } => json!({ "role": "user", "content": text }),
            Message::Assistant { text, tool_calls, .. } => {
                let mut value = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { Value::String(text.clone()) } });
                if !tool_calls.is_empty() {
                    value["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| json!({ "id": call.id, "type": "function", "function": { "name": call.name, "arguments": call.arguments } }))
                        .collect();
                }
                value
            }
            Message::Tool { call_id, output, .. } => json!({ "role": "tool", "tool_call_id": call_id, "content": output }),
        });
    }
    let mut body = json!({ "model": endpoint.model, "messages": list, "stream": true });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|tool| json!({ "type": "function", "function": { "name": tool.name, "description": tool.description, "parameters": tool.parameters } }))
            .collect();
    }
    if let Some(max) = endpoint.max_tokens {
        body["max_completion_tokens"] = json!(max);
    }
    if let Some(effort) = endpoint.effort {
        body["reasoning_effort"] = json!(effort.openai());
    }
    body
}

fn responses_body(
    endpoint: &Endpoint,
    system: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    let mut input = Vec::new();
    for message in messages {
        match message {
            Message::User { text } => input.push(json!({ "role": "user", "content": text })),
            Message::Assistant {
                text, tool_calls, ..
            } => {
                if !text.is_empty() {
                    input.push(json!({ "role": "assistant", "content": text }));
                }
                for call in tool_calls {
                    input.push(json!({ "type": "function_call", "call_id": call.id, "name": call.name, "arguments": call.arguments }));
                }
            }
            Message::Tool {
                call_id, output, ..
            } => input.push(
                json!({ "type": "function_call_output", "call_id": call_id, "output": output }),
            ),
        }
    }
    let mut body = json!({ "model": endpoint.model, "instructions": system, "input": input, "stream": true, "store": false });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|tool| json!({ "type": "function", "name": tool.name, "description": tool.description, "parameters": tool.parameters }))
            .collect();
    }
    if let Some(max) = endpoint.max_tokens {
        body["max_output_tokens"] = json!(max);
    }
    if let Some(effort) = endpoint.effort {
        body["reasoning"] = json!({ "effort": effort.openai(), "summary": "auto" });
    }
    body
}

fn anthropic_body(
    endpoint: &Endpoint,
    system: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    let mut list: Vec<Value> = Vec::new();
    for message in messages {
        match message {
            Message::User { text } => {
                list.push(json!({ "role": "user", "content": [{ "type": "text", "text": text }] }))
            }
            Message::Assistant {
                text,
                tool_calls,
                opaque,
            } => {
                let mut content: Vec<Value> = opaque
                    .as_ref()
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for call in tool_calls {
                    let input: Value =
                        serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                    content.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": input }));
                }
                if content.is_empty() {
                    content.push(json!({ "type": "text", "text": "…" }));
                }
                list.push(json!({ "role": "assistant", "content": content }));
            }
            Message::Tool {
                call_id,
                output,
                is_error,
                ..
            } => {
                let block = json!({ "type": "tool_result", "tool_use_id": call_id, "content": output, "is_error": is_error });
                // Results of one turn go back together in one user message.
                match list.last_mut() {
                    Some(last)
                        if last["role"] == "user"
                            && last["content"].as_array().is_some_and(|blocks| {
                                blocks.iter().all(|block| block["type"] == "tool_result")
                            }) =>
                    {
                        if let Some(blocks) = last["content"].as_array_mut() {
                            blocks.push(block);
                        }
                    }
                    _ => list.push(json!({ "role": "user", "content": [block] })),
                }
            }
        }
    }
    let max_tokens = endpoint.max_tokens.unwrap_or(ANTHROPIC_MAX_TOKENS);
    let mut body = json!({ "model": endpoint.model, "system": system, "messages": list, "max_tokens": max_tokens, "stream": true });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|tool| json!({ "name": tool.name, "description": tool.description, "input_schema": tool.parameters }))
            .collect();
    }
    // Current models take adaptive thinking with an effort level; fixed thinking budgets are
    // refused by them. A summary of the reasoning is asked for, to show it while it streams.
    if let Some(effort) = endpoint.effort {
        body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
        body["output_config"] = json!({ "effort": effort.anthropic() });
    }
    body
}

// ---- streaming -----------------------------------------------------------------------------

/// One server-sent event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Splits a byte stream into server-sent events.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        // An event ends at a blank line. Searching the bytes keeps a character split across
        // chunks intact until its event is complete.
        while let Some((end, skip)) = blank_line(&self.buffer) {
            let block = String::from_utf8_lossy(&self.buffer[..end]).into_owned();
            self.buffer.drain(..end + skip);
            let mut event = SseEvent::default();
            let mut data: Vec<&str> = Vec::new();
            for line in block.lines() {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.strip_prefix(' ').unwrap_or(value));
                } else if let Some(value) = line.strip_prefix("event:") {
                    event.event = value.trim().to_owned();
                }
            }
            event.data = data.join("\n");
            if !event.data.is_empty() || !event.event.is_empty() {
                events.push(event);
            }
        }
        events
    }
}

/// Where the first blank line starts in `bytes`, and its length.
fn blank_line(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut at = 0;
    while at + 1 < bytes.len() {
        if bytes[at] == b'\n' {
            if bytes[at + 1] == b'\n' {
                return Some((at, 2));
            }
            if bytes[at + 1] == b'\r' && bytes.get(at + 2) == Some(&b'\n') {
                return Some((at, 3));
            }
        }
        at += 1;
    }
    None
}

/// A tool call being assembled from pieces.
#[derive(Default, Clone)]
struct Partial {
    id: String,
    name: String,
    arguments: String,
}

struct StreamState {
    format: Format,
    text: String,
    calls: Vec<Partial>,
    /// Anthropic content blocks by index: thinking text and signature.
    thinking: Vec<(usize, String, String)>,
    /// Anthropic block kinds by index.
    blocks: Vec<(usize, String)>,
    stop: String,
    finished: bool,
}

fn parse(data: &str) -> Result<Value, LlmError> {
    serde_json::from_str(data).map_err(|_| LlmError::Protocol("a stream event is not JSON".into()))
}

fn text_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

impl StreamState {
    fn new(format: Format) -> Self {
        Self {
            format,
            text: String::new(),
            calls: Vec::new(),
            thinking: Vec::new(),
            blocks: Vec::new(),
            stop: String::new(),
            finished: false,
        }
    }

    fn call_at(&mut self, index: usize) -> &mut Partial {
        if self.calls.len() <= index {
            self.calls.resize(index + 1, Partial::default());
        }
        &mut self.calls[index]
    }

    /// Takes one event; returns `true` when the reply is complete.
    fn event(
        &mut self,
        event: &SseEvent,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<bool, LlmError> {
        match self.format {
            Format::Chat => self.chat(event, on_delta),
            Format::Responses => self.responses(event, on_delta),
            Format::Anthropic => self.anthropic(event, on_delta),
        }
    }

    fn chat(
        &mut self,
        event: &SseEvent,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<bool, LlmError> {
        if event.data.trim() == "[DONE]" {
            self.finished = true;
            return Ok(true);
        }
        let value = parse(&event.data)?;
        if let Some(error) = value.get("error") {
            return Err(LlmError::Protocol(error_message(&error.to_string())));
        }
        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(false);
        };
        let delta = &choice["delta"];
        // DeepSeek and others stream reasoning beside the content.
        for key in ["reasoning_content", "reasoning"] {
            if let Some(thought) = delta
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                on_delta(Delta::Thought(thought.to_owned()));
            }
        }
        if let Some(text) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            self.text.push_str(text);
            on_delta(Delta::Text(text.to_owned()));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (position, call) in calls.iter().enumerate() {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .map_or(position, |index| index as usize);
                if index > 64 {
                    return Err(LlmError::Protocol("too many tool calls".into()));
                }
                let partial = self.call_at(index);
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    partial.id = id.to_owned();
                }
                partial.name.push_str(text_at(call, "/function/name"));
                partial
                    .arguments
                    .push_str(text_at(call, "/function/arguments"));
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop = reason.to_owned();
            // Some servers end without `[DONE]`; the finish reason is final either way.
            self.finished = true;
        }
        Ok(false)
    }

    fn responses(
        &mut self,
        event: &SseEvent,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<bool, LlmError> {
        let value = parse(&event.data)?;
        let kind = if event.event.is_empty() {
            text_at(&value, "/type").to_owned()
        } else {
            event.event.clone()
        };
        match kind.as_str() {
            "response.output_text.delta" => {
                let text = text_at(&value, "/delta");
                self.text.push_str(text);
                on_delta(Delta::Text(text.to_owned()));
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                on_delta(Delta::Thought(text_at(&value, "/delta").to_owned()))
            }
            "response.output_item.done" if text_at(&value, "/item/type") == "function_call" => {
                if self.calls.len() > 64 {
                    return Err(LlmError::Protocol("too many tool calls".into()));
                }
                self.calls.push(Partial {
                    id: text_at(&value, "/item/call_id").to_owned(),
                    name: text_at(&value, "/item/name").to_owned(),
                    arguments: text_at(&value, "/item/arguments").to_owned(),
                });
            }
            "response.completed" | "response.incomplete" => {
                self.stop = value
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or(if kind == "response.completed" {
                        "stop"
                    } else {
                        "incomplete"
                    })
                    .to_owned();
                self.finished = true;
                return Ok(true);
            }
            "response.failed" => {
                return Err(LlmError::Protocol(
                    text_at(&value, "/response/error/message").to_owned(),
                ));
            }
            "error" => return Err(LlmError::Protocol(text_at(&value, "/message").to_owned())),
            _ => {}
        }
        Ok(false)
    }

    fn anthropic(
        &mut self,
        event: &SseEvent,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<bool, LlmError> {
        let value = parse(&event.data)?;
        let kind = if event.event.is_empty() {
            text_at(&value, "/type").to_owned()
        } else {
            event.event.clone()
        };
        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        if index > 256 {
            return Err(LlmError::Protocol("too many content blocks".into()));
        }
        match kind.as_str() {
            "content_block_start" => {
                let block = &value["content_block"];
                let block_type = text_at(block, "/type").to_owned();
                match block_type.as_str() {
                    "tool_use" => {
                        let at = self.calls.len();
                        self.call_at(at).id = text_at(block, "/id").to_owned();
                        self.calls[at].name = text_at(block, "/name").to_owned();
                        self.blocks.push((index, format!("tool_use:{at}")));
                    }
                    "thinking" => {
                        self.thinking.push((index, String::new(), String::new()));
                        self.blocks.push((index, block_type));
                    }
                    _ => self.blocks.push((index, block_type)),
                }
            }
            "content_block_delta" => {
                let delta = &value["delta"];
                let kind_of = self
                    .blocks
                    .iter()
                    .rev()
                    .find(|(at, _)| *at == index)
                    .map(|(_, kind)| kind.clone())
                    .unwrap_or_default();
                match text_at(delta, "/type") {
                    "text_delta" => {
                        let text = text_at(delta, "/text");
                        self.text.push_str(text);
                        on_delta(Delta::Text(text.to_owned()));
                    }
                    "thinking_delta" => {
                        let text = text_at(delta, "/thinking");
                        if let Some(entry) = self
                            .thinking
                            .iter_mut()
                            .rev()
                            .find(|(at, _, _)| *at == index)
                        {
                            entry.1.push_str(text);
                        }
                        on_delta(Delta::Thought(text.to_owned()));
                    }
                    "signature_delta" => {
                        if let Some(entry) = self
                            .thinking
                            .iter_mut()
                            .rev()
                            .find(|(at, _, _)| *at == index)
                        {
                            entry.2.push_str(text_at(delta, "/signature"));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(at) = kind_of
                            .strip_prefix("tool_use:")
                            .and_then(|at| at.parse::<usize>().ok())
                        {
                            self.call_at(at)
                                .arguments
                                .push_str(text_at(delta, "/partial_json"));
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = reason.to_owned();
                }
            }
            "message_stop" => {
                self.finished = true;
                return Ok(true);
            }
            "error" => {
                return Err(LlmError::Protocol(
                    text_at(&value, "/error/message").to_owned(),
                ));
            }
            _ => {}
        }
        Ok(false)
    }

    fn finish(self) -> Reply {
        let tool_calls = self
            .calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .enumerate()
            .map(|(index, call)| ToolCall {
                // A call without an id still needs one to pair with its result.
                id: if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                },
                name: call.name,
                arguments: if call.arguments.trim().is_empty() {
                    "{}".into()
                } else {
                    call.arguments
                },
            })
            .collect();
        let opaque = (!self.thinking.is_empty()).then(|| {
            Value::Array(self.thinking.into_iter().map(|(_, thinking, signature)| json!({ "type": "thinking", "thinking": thinking, "signature": signature })).collect())
        });
        Reply {
            text: self.text,
            tool_calls,
            opaque,
            stop: self.stop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_server_sent_events() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"event: a\ndata: {\"x\"").is_empty());
        let events = parser.push(b":1}\n\ndata: one\r\ndata: two\r\n\r\n");
        // A character split between chunks survives.
        assert!(parser.push(&"data: 你".as_bytes()[..7]).is_empty());
        assert_eq!(parser.push(&"data: 你\n\n".as_bytes()[7..])[0].data, "你");
        assert_eq!(
            events,
            [
                SseEvent {
                    event: "a".into(),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: String::new(),
                    data: "one\ntwo".into()
                }
            ]
        );
    }

    #[test]
    fn rejects_unsafe_base_urls() {
        let endpoint = |url: &str| Endpoint {
            format: Format::Chat,
            base_url: url.into(),
            model: "m".into(),
            max_tokens: None,
            effort: None,
        };
        let idle = Duration::from_secs(5);
        assert!(Client::new(endpoint("https://api.example.com/v1/"), "k", idle).is_ok());
        assert!(Client::new(endpoint("http://localhost:11434/v1"), "", idle).is_ok());
        for bad in [
            "ftp://example.com",
            "api.example.com",
            "https://user:pw@example.com",
            "https://example.com/v1?x=1",
        ] {
            assert!(
                matches!(
                    Client::new(endpoint(bad), "k", idle),
                    Err(LlmError::InvalidUrl)
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn anthropic_groups_tool_results_and_replays_thinking() {
        let endpoint = Endpoint {
            format: Format::Anthropic,
            base_url: String::new(),
            model: "m".into(),
            max_tokens: Some(4096),
            effort: Some(Effort::High),
        };
        let messages = vec![
            Message::User { text: "hi".into() },
            Message::Assistant {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "run_command".into(),
                        arguments: "{\"command\":\"ls\"}".into(),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "read_file".into(),
                        arguments: "{}".into(),
                    },
                ],
                opaque: Some(
                    json!([{ "type": "thinking", "thinking": "hmm", "signature": "sig" }]),
                ),
            },
            Message::Tool {
                call_id: "a".into(),
                name: "run_command".into(),
                output: "x".into(),
                is_error: false,
            },
            Message::Tool {
                call_id: "b".into(),
                name: "read_file".into(),
                output: "denied".into(),
                is_error: true,
            },
        ];
        let body = anthropic_body(&endpoint, "sys", &messages, &[]);
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(body["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(body["messages"][1]["content"][1]["input"]["command"], "ls");
        assert_eq!(body["messages"][2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(
            body["thinking"],
            json!({ "type": "adaptive", "display": "summarized" })
        );
        assert_eq!(body["output_config"]["effort"], "high");
        let max = Endpoint {
            effort: Some(Effort::Max),
            ..endpoint.clone()
        };
        assert_eq!(
            anthropic_body(&max, "s", &messages, &[])["output_config"]["effort"],
            "max"
        );
        assert_eq!(
            chat_body(&max, "s", &messages, &[])["reasoning_effort"],
            "xhigh"
        );
        let minimal = Endpoint {
            effort: Some(Effort::Minimal),
            ..endpoint
        };
        assert_eq!(
            anthropic_body(&minimal, "s", &messages, &[])["output_config"]["effort"],
            "low"
        );
        assert_eq!(
            responses_body(&minimal, "s", &messages, &[])["reasoning"]["effort"],
            "minimal"
        );
    }
}
