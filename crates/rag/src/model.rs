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
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for LlmRequest {
    fn default() -> Self {
        Self {
            system: None,
            messages: Vec::new(),
            max_tokens: 1024,
            temperature: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub stop_reason: Option<String>,
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
        })
    }
}

/// HTTP client for the Anthropic Messages API. The crate doesn't ship API
/// keys; pass them in via the builder. Default model is `claude-opus-4-7`.
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
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Deserialize)]
struct AnthropicResponseBlock { #[serde(default)] text: String }

#[derive(Deserialize)]
struct AnthropicResponse {
    #[serde(default)] content: Vec<AnthropicResponseBlock>,
    #[serde(default)] model: String,
    #[serde(default)] stop_reason: Option<String>,
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
            temperature: req.temperature,
            system: req.system.as_deref(),
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
        })
    }
}
