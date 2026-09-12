//! Embedding backend: a minimal OpenAI-compatible `/embeddings` client plus
//! the cosine similarity used by the write pipeline and the extraction
//! triggers.
//!
//! The trait is the seam: callers (and tests) can plug a deterministic fake
//! instead of the HTTP backend ([`OpenAIEmbeddings`]). The HTTP client is
//! deliberately thin — one POST, no retries, no sidecar LLM judgement chain
//! (jcode `embedding_backend::OpenAiEmbeddingBackend` lineage, reduced to the
//! write-pipeline needs).

use std::fmt;
use std::time::Duration;

/// Errors from an [`Embedder`].
#[derive(Debug)]
pub enum EmbedError {
    /// The endpoint answered with a non-2xx status. `body` is truncated
    /// response text for diagnostics.
    Status { status: u16, body: String },
    /// Transport-level failure: connect/read timeout, DNS, connection reset.
    Transport(String),
    /// The response body was not the expected `{"data":[{"embedding":...}]}`
    /// shape.
    Parse(String),
    /// Contract violation: the backend returned a different number of
    /// vectors than inputs.
    Count { want: usize, got: usize },
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::Status { status, body } => {
                write!(f, "embeddings request failed ({status}): {body}")
            }
            EmbedError::Transport(msg) => write!(f, "embeddings transport error: {msg}"),
            EmbedError::Parse(msg) => write!(f, "parse embeddings response: {msg}"),
            EmbedError::Count { want, got } => {
                write!(f, "embeddings returned {got} vectors for {want} inputs")
            }
        }
    }
}

impl std::error::Error for EmbedError {}

/// Source of dense vectors for memory texts.
pub trait Embedder {
    /// Embed `texts` in one batch; the returned vectors are position-aligned
    /// with the inputs.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// OpenAI-compatible embeddings backend (`POST {base_url}/embeddings`).
///
/// Works against OpenAI proper and any gateway exposing the same schema
/// (jcode lineage). `input` is always sent as a string array — some
/// OpenAI-compatible implementations reject the bare-string form. No retry:
/// the caller decides whether a failure is worth repeating.
pub struct OpenAIEmbeddings {
    base_url: String,
    api_key: String,
    model: String,
    agent: ureq::Agent,
}

impl OpenAIEmbeddings {
    /// Backend for `model` at `base_url` (trailing `/` trimmed) with a
    /// 10s connect / 30s read timeout — a stalled endpoint fails the write
    /// instead of hanging the remember call.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        OpenAIEmbeddings {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .build(),
        }
    }
}

/// One item of the OpenAI embeddings response.
#[derive(Debug, serde::Deserialize)]
struct EmbeddingItem {
    /// Position of this vector in the request's `input` array. Some
    /// OpenAI-compatible implementations omit it; then the array position is
    /// used as a fallback so ordering is still request order.
    #[serde(default)]
    index: Option<usize>,
    embedding: Vec<f32>,
}

impl Embedder for OpenAIEmbeddings {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/embeddings", self.base_url);
        let response = self
            .agent
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::json!({
                "model": self.model,
                "input": texts,
            }))
            .map_err(|e| match e {
                ureq::Error::Status(status, resp) => {
                    let body = resp
                        .into_string()
                        .unwrap_or_default()
                        .chars()
                        .take(400)
                        .collect::<String>();
                    EmbedError::Status { status, body }
                }
                other => EmbedError::Transport(other.to_string()),
            })?;

        let parsed: Vec<EmbeddingItem> = serde_json::from_value(
            response
                .into_json::<serde_json::Value>()
                .map_err(|e| EmbedError::Parse(e.to_string()))?
                .get("data")
                .cloned()
                .ok_or_else(|| EmbedError::Parse("missing `data` array".to_string()))?,
        )
        .map_err(|e| EmbedError::Parse(e.to_string()))?;

        if parsed.len() != texts.len() {
            return Err(EmbedError::Count {
                want: texts.len(),
                got: parsed.len(),
            });
        }

        // The API returns items carrying their `index`; sort so a gateway
        // that shuffles the array cannot silently scramble the vectors.
        let mut items: Vec<(usize, Vec<f32>)> = parsed
            .into_iter()
            .enumerate()
            .map(|(pos, item)| (item.index.unwrap_or(pos), item.embedding))
            .collect();
        items.sort_by_key(|(index, _)| *index);

        Ok(items.into_iter().map(|(_, v)| v).collect())
    }
}

/// Cosine similarity of two vectors: `None` when their lengths differ or
/// either is a zero vector (similarity is undefined there, not 0).
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    let (dot, norm_a, norm_b) = a
        .iter()
        .zip(b)
        .fold((0.0f32, 0.0f32, 0.0f32), |(dot, na, nb), (&p, &q)| {
            (dot + p * q, na + p * p, nb + q * q)
        });
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some(dot / (norm_a.sqrt() * norm_b.sqrt()))
}
