// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: s.into(),
        }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: s.into(),
        }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: s.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    /// Short, frozen instruction. Sent as the first system block.
    pub system: Option<String>,
    /// Stable, large per-knowledge-base context (e.g. ontology). When set,
    /// the Anthropic client renders it as a second `system` block with
    /// `cache_control: {type: "ephemeral"}` so repeated queries over the
    /// same KB pay ~10% input price for the cached prefix on subsequent
    /// requests within the TTL (default 5 minutes).
    ///
    /// Caching is a strict prefix match — keep this content byte-stable
    /// across requests. The minimum cacheable prefix is model-dependent
    /// (4096 tokens on Claude Opus 4.7); below that the breakpoint is
    /// silently ignored, no error.
    #[serde(default)]
    pub cached_context: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for LlmRequest {
    fn default() -> Self {
        Self {
            system: None,
            cached_context: None,
            messages: Vec::new(),
            max_tokens: 1024,
            temperature: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    /// Tokens written to the prompt cache this request (~1.25× base price).
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    /// Tokens served from the prompt cache (~0.1× base price). If this stays
    /// at zero across repeated identical-prefix requests, a silent cache
    /// invalidator is at work — diff the rendered prompt bytes between two
    /// calls to find it.
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: TokenUsage,
}

/// One frame of a streaming generation. Emit text deltas as `Text(_)`,
/// then a single trailing `End { usage, stop_reason }` to close the
/// stream cleanly. Implementations are free to also emit periodic
/// `KeepAlive` frames if there's a long gap with no token output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamChunk {
    /// Incremental text from the model.
    Text(String),
    /// Heartbeat with no payload — clients can use this to keep
    /// connections alive across loadbalancer idle timeouts.
    KeepAlive,
    /// Final frame with totals; the stream ends after this.
    End {
        #[serde(default)]
        usage: TokenUsage,
        #[serde(default)]
        stop_reason: Option<String>,
        #[serde(default)]
        model: String,
    },
}

/// Boxed async stream of model output. Produced by [`LanguageModel::generate_stream`].
pub type LlmStream = BoxStream<'static, Result<StreamChunk, LlmError>>;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http: {0}")]
    Http(String),
    #[error("api: {0}")]
    Api(String),
    #[error("decode: {0}")]
    Decode(String),
}

/// Abstract chat-completion endpoint. All RAG pipeline tests can use
/// [`EchoModel`]; production callers wire up [`AnthropicModel`] or their own.
///
/// Implementations should override `generate_stream` for true incremental
/// output. The default forwards to `generate` and emits a single text
/// chunk + `End`, which preserves correctness but loses the latency win.
#[async_trait]
pub trait LanguageModel: Send + Sync + 'static {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Stream text deltas as they're produced. The default fakes a stream
    /// from `generate` so every implementor works out of the box.
    async fn generate_stream(&self, req: &LlmRequest) -> Result<LlmStream, LlmError> {
        let resp = self.generate(req).await?;
        let chunks = vec![
            Ok::<_, LlmError>(StreamChunk::Text(resp.content)),
            Ok(StreamChunk::End {
                usage: resp.usage,
                stop_reason: resp.stop_reason,
                model: resp.model,
            }),
        ];
        Ok(futures::stream::iter(chunks).boxed())
    }
}

/// Deterministic fake: echoes the last user message, prefixed with a marker
/// derived from the system prompt. Useful for tests and offline demos.
#[derive(Debug, Default, Clone)]
pub struct EchoModel;

#[async_trait]
impl LanguageModel for EchoModel {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let last = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(LlmResponse {
            content: format!("[echo] {}", last),
            model: "echo".into(),
            stop_reason: Some("end_turn".into()),
            usage: TokenUsage::default(),
        })
    }
}

/// HTTP client for the Anthropic Messages API. The crate doesn't ship API
/// keys; pass them in via the builder. Default model is `claude-opus-4-7`.
///
/// Sends `cached_context` as a separately-cached `system` block; verify cache
/// hits via `LlmResponse::usage.cache_read_input_tokens`. Omits sampling
/// parameters on Claude Opus 4.7, where `temperature` / `top_p` / `top_k`
/// are removed and 400 if sent.
///
/// Retries 408 / 409 / 429 and any 5xx response with exponential backoff
/// (full jitter), up to `max_retries` times. The first retry honors a
/// `retry-after` header if the server sent one.
pub struct AnthropicModel {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    max_retries: u32,
    initial_backoff: Duration,
}

impl AnthropicModel {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "claude-opus-4-7".into(),
            base_url: "https://api.anthropic.com".into(),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn with_initial_backoff(mut self, d: Duration) -> Self {
        self.initial_backoff = d;
        self
    }
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    ty: &'static str,
}

#[derive(Serialize)]
struct SystemBlock<'a> {
    #[serde(rename = "type")]
    ty: &'static str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum SystemField<'a> {
    Text(&'a str),
    Blocks(Vec<SystemBlock<'a>>),
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<SystemField<'a>>,
    messages: Vec<AnthropicMessage<'a>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Deserialize)]
struct AnthropicResponseBlock {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicResponseBlock>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: AnthropicUsage,
}

/// `temperature`/`top_p`/`top_k` are removed on Claude Opus 4.7 (400 if sent).
fn supports_sampling_params(model: &str) -> bool {
    !model.starts_with("claude-opus-4-7")
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 409 | 429 | 500..=599)
}

fn jittered_backoff(base: Duration, attempt: u32) -> Duration {
    // Full-jitter exponential backoff: random in [0, base * 2^attempt].
    use rand_like::Lcg;
    let cap_ms = (base.as_millis() as u64).saturating_mul(1u64 << attempt.min(8));
    let jitter = Lcg::seed_from_time().next_u64() % cap_ms.max(1);
    Duration::from_millis(jitter)
}

/// Tiny, dependency-free LCG used only for backoff jitter — does not need
/// crypto strength, just enough to avoid retry storms across processes.
mod rand_like {
    use std::time::{SystemTime, UNIX_EPOCH};
    pub struct Lcg(u64);
    impl Lcg {
        pub fn seed_from_time() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xDEAD_BEEF_CAFE_BABE);
            Self(nanos | 1)
        }
        pub fn next_u64(&mut self) -> u64 {
            // Numerical Recipes constants.
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
    }
}

fn build_system_field<'a>(
    instruction: Option<&'a str>,
    cached: Option<&'a str>,
) -> Option<SystemField<'a>> {
    match (instruction, cached) {
        (None, None) => None,
        (Some(s), None) => Some(SystemField::Text(s)),
        (instruction, Some(cached)) => {
            let mut blocks = Vec::with_capacity(2);
            if let Some(s) = instruction {
                if !s.is_empty() {
                    blocks.push(SystemBlock {
                        ty: "text",
                        text: s,
                        cache_control: None,
                    });
                }
            }
            // cache_control on the LAST block also caches everything before it.
            blocks.push(SystemBlock {
                ty: "text",
                text: cached,
                cache_control: Some(CacheControl { ty: "ephemeral" }),
            });
            Some(SystemField::Blocks(blocks))
        }
    }
}

/// Take one complete SSE event (terminated by `\n\n`) off the front of
/// `buf`. Returns `(event_name, data_payload)` if one is available.
fn take_one_sse_event(buf: &mut Vec<u8>) -> Option<(String, String)> {
    let pos = buf.windows(2).position(|w| w == b"\n\n")?;
    let raw = std::str::from_utf8(&buf[..pos]).ok()?.to_string();
    buf.drain(..pos + 2);
    let mut event = String::new();
    let mut data = String::new();
    for line in raw.split('\n') {
        if let Some(rest) = line.strip_prefix("event: ") {
            event = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest);
        }
    }
    Some((event, data))
}

#[derive(Deserialize)]
struct DeltaEvent {
    delta: DeltaPayload,
}

#[derive(Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type", default)]
    ty: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct MessageStartEvent {
    message: MessageStartPayload,
}

#[derive(Deserialize)]
struct MessageStartPayload {
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct MessageDeltaEvent {
    #[serde(default)]
    delta: MessageStopDelta,
    #[serde(default)]
    usage: AnthropicUsage,
}

#[derive(Deserialize, Default)]
struct MessageStopDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

struct StreamState {
    body: Option<reqwest::Response>,
    buf: Vec<u8>,
    model: String,
    usage: TokenUsage,
    stop_reason: Option<String>,
    end_emitted: bool,
}

fn parse_anthropic_event(
    event: &str,
    data: &str,
    state: &mut StreamState,
) -> Option<Result<StreamChunk, LlmError>> {
    match event {
        "content_block_delta" => {
            let parsed: DeltaEvent = serde_json::from_str(data).ok()?;
            if parsed.delta.ty == "text_delta" && !parsed.delta.text.is_empty() {
                return Some(Ok(StreamChunk::Text(parsed.delta.text)));
            }
            None
        }
        "message_start" => {
            if let Ok(parsed) = serde_json::from_str::<MessageStartEvent>(data) {
                if !parsed.message.model.is_empty() {
                    state.model = parsed.message.model;
                }
                state.usage.input_tokens = parsed.message.usage.input_tokens;
                state.usage.cache_creation_input_tokens =
                    parsed.message.usage.cache_creation_input_tokens;
                state.usage.cache_read_input_tokens = parsed.message.usage.cache_read_input_tokens;
            }
            None
        }
        "message_delta" => {
            if let Ok(parsed) = serde_json::from_str::<MessageDeltaEvent>(data) {
                if let Some(sr) = parsed.delta.stop_reason {
                    state.stop_reason = Some(sr);
                }
                if parsed.usage.output_tokens > 0 {
                    state.usage.output_tokens = parsed.usage.output_tokens;
                }
            }
            None
        }
        "message_stop" => Some(Ok(StreamChunk::End {
            usage: state.usage.clone(),
            stop_reason: state.stop_reason.take(),
            model: std::mem::take(&mut state.model),
        })),
        "error" => Some(Err(LlmError::Api(data.to_string()))),
        _ => None,
    }
}

#[async_trait]
impl LanguageModel for AnthropicModel {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let messages: Vec<AnthropicMessage> = req
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "user",
                },
                content: &m.content,
            })
            .collect();

        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: req.max_tokens,
            temperature: if supports_sampling_params(&self.model) {
                Some(req.temperature)
            } else {
                None
            },
            system: build_system_field(req.system.as_deref(), req.cached_context.as_deref()),
            messages,
            stream: false,
        };

        let url = format!("{}/v1/messages", self.base_url);

        let mut attempt: u32 = 0;
        let parsed: AnthropicResponse = loop {
            let send_result = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            match send_result {
                Err(e) if e.is_connect() || e.is_timeout() => {
                    if attempt >= self.max_retries {
                        return Err(LlmError::Http(e.to_string()));
                    }
                    let delay = jittered_backoff(self.initial_backoff, attempt);
                    tracing::warn!(error=%e, attempt, ?delay, "transport error; retrying");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(LlmError::Http(e.to_string())),
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        break resp
                            .json::<AnthropicResponse>()
                            .await
                            .map_err(|e| LlmError::Decode(e.to_string()))?;
                    }
                    if is_retryable_status(status) && attempt < self.max_retries {
                        // Honor server-side retry-after if present.
                        let server_hint = resp
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .map(Duration::from_secs);
                        let delay = server_hint
                            .unwrap_or_else(|| jittered_backoff(self.initial_backoff, attempt));
                        tracing::warn!(%status, attempt, ?delay, "retryable api error");
                        // Drain the body to free the connection before sleeping.
                        let _ = resp.text().await;
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue;
                    }
                    let text = resp.text().await.unwrap_or_default();
                    return Err(LlmError::Api(format!("{status}: {text}")));
                }
            }
        };
        let content = parsed
            .content
            .into_iter()
            .map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");
        Ok(LlmResponse {
            content,
            model: parsed.model,
            stop_reason: parsed.stop_reason,
            usage: TokenUsage {
                input_tokens: parsed.usage.input_tokens,
                output_tokens: parsed.usage.output_tokens,
                cache_creation_input_tokens: parsed.usage.cache_creation_input_tokens,
                cache_read_input_tokens: parsed.usage.cache_read_input_tokens,
            },
        })
    }

    async fn generate_stream(&self, req: &LlmRequest) -> Result<LlmStream, LlmError> {
        let messages: Vec<AnthropicMessage> = req
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "user",
                },
                content: &m.content,
            })
            .collect();

        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: req.max_tokens,
            temperature: if supports_sampling_params(&self.model) {
                Some(req.temperature)
            } else {
                None
            },
            system: build_system_field(req.system.as_deref(), req.cached_context.as_deref()),
            messages,
            stream: true,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("{status}: {text}")));
        }

        let state = StreamState {
            body: Some(resp),
            buf: Vec::with_capacity(4096),
            model: String::new(),
            usage: TokenUsage::default(),
            stop_reason: None,
            end_emitted: false,
        };

        let stream = futures::stream::unfold(state, |mut state| async move {
            if state.end_emitted {
                return None;
            }
            loop {
                // Drain any complete SSE events already buffered.
                while let Some((event, data)) = take_one_sse_event(&mut state.buf) {
                    if let Some(chunk) = parse_anthropic_event(&event, &data, &mut state) {
                        if matches!(chunk, Ok(StreamChunk::End { .. })) {
                            state.end_emitted = true;
                        }
                        return Some((chunk, state));
                    }
                }
                // Read more bytes from the response body.
                let body = match state.body.as_mut() {
                    Some(b) => b,
                    None => return None,
                };
                match body.chunk().await {
                    Ok(Some(bytes)) => state.buf.extend_from_slice(&bytes),
                    Ok(None) => {
                        // Connection closed without an explicit message_stop —
                        // synthesize an End so the consumer can flush.
                        state.end_emitted = true;
                        let chunk = StreamChunk::End {
                            usage: state.usage.clone(),
                            stop_reason: state.stop_reason.take(),
                            model: std::mem::take(&mut state.model),
                        };
                        return Some((Ok(chunk), state));
                    }
                    Err(e) => {
                        return Some((Err(LlmError::Http(e.to_string())), state));
                    }
                }
            }
        })
        .boxed();
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_field_omitted_when_neither_set() {
        assert!(build_system_field(None, None).is_none());
    }

    #[test]
    fn system_field_plain_text_when_only_instruction() {
        let f = build_system_field(Some("hi"), None).unwrap();
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"hi\"");
    }

    #[test]
    fn system_field_blocks_with_cache_control_when_cached_set() {
        let f = build_system_field(Some("hi"), Some("big ontology")).unwrap();
        let v: serde_json::Value = serde_json::to_value(&f).unwrap();
        let arr = v.as_array().expect("expected array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "hi");
        assert!(arr[0].get("cache_control").is_none());
        assert_eq!(arr[1]["text"], "big ontology");
        assert_eq!(arr[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn opus_4_7_omits_temperature() {
        assert!(!supports_sampling_params("claude-opus-4-7"));
        assert!(!supports_sampling_params("claude-opus-4-7-20260101"));
        assert!(supports_sampling_params("claude-opus-4-6"));
        assert!(supports_sampling_params("claude-sonnet-4-6"));
    }

    #[tokio::test]
    async fn echo_model_returns_default_usage() {
        let r = EchoModel
            .generate(&LlmRequest {
                messages: vec![Message::user("hi")],
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(r.content, "[echo] hi");
        assert_eq!(r.usage.input_tokens, 0);
    }
}
