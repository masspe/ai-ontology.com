use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
    pub fn system(s: impl Into<String>) -> Self { Self { role: Role::System, content: s.into() } }
    pub fn user(s: impl Into<String>) -> Self { Self { role: Role::User, content: s.into() } }
    pub fn assistant(s: impl Into<String>) -> Self { Self { role: Role::Assistant, content: s.into() } }
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
#[async_trait]
pub trait LanguageModel: Send + Sync + 'static {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError>;
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
pub struct AnthropicModel {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicModel {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "claude-opus-4-7".into(),
            base_url: "https://api.anthropic.com".into(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into(); self
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into(); self
    }
}

#[derive(Serialize)]
struct AnthropicMessage<'a> { role: &'a str, content: &'a str }

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
                    blocks.push(SystemBlock { ty: "text", text: s, cache_control: None });
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
        };

        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("{status}: {text}")));
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Decode(e.to_string()))?;
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
        let r = EchoModel.generate(&LlmRequest {
            messages: vec![Message::user("hi")],
            ..Default::default()
        }).await.unwrap();
        assert_eq!(r.content, "[echo] hi");
        assert_eq!(r.usage.input_tokens, 0);
    }
}
