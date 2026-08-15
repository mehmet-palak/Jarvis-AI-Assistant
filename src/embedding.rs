//! Local CPU-only embedding model adapter for hybrid FTS + semantic workspace retrieval.
//!
//! Mirrors `LlamaServerProvider`'s loopback-only raw-HTTP client pattern (no new HTTP client
//! dependency). The embedding service is entirely optional: `Runtime` only uses it when one has
//! been explicitly attached, and every hybrid retrieval path falls back to plain FTS the moment
//! embedding fails or is unavailable — this never becomes a hard dependency for RAG. See
//! ADR-0004 for the model decision (Qwen3-Embedding-0.6B, user-approved 15 Ağustos 2026).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

// `Send + Sync`: `Runtime` (which optionally holds a `Box<dyn EmbeddingProvider>`) is shared
// across threads via `Arc<Mutex<Runtime>>` in both the TUI and native desktop apps.
pub trait EmbeddingProvider: std::fmt::Debug + Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;

    /// Embeds many texts in as few round-trips as possible, in request order. Default
    /// implementation just calls `embed` once per text — correct for any implementor, just not
    /// necessarily fast; a provider whose backend actually supports batching (like
    /// `LlamaEmbeddingProvider`) overrides this for a real speed win on `/index-folder` with many
    /// files, and nothing else in the crate needs to know the difference (`persistence.rs` always
    /// calls this, never loops over `embed` itself).
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        texts.iter().map(|text| self.embed(text)).collect()
    }

    /// Identifies the model + output shape a stored embedding was produced with (for example
    /// `"Qwen3-Embedding-0.6B-Q8_0:1024"`). The storage layer keys its reuse cache on this in
    /// addition to content hash: swapping to a different model must never let an old vector from
    /// a different embedding space be silently reused as if it were comparable.
    fn embedding_model_id(&self) -> &str;
}

/// OpenAI-compatible local adapter for the persistent CPU-only embedding llama-server. Bound to
/// loopback only, never granted tool or policy authority — same posture as `LlamaServerProvider`.
#[derive(Debug, Clone)]
pub struct LlamaEmbeddingProvider {
    pub host: String,
    pub port: u16,
    pub timeout_seconds: u16,
    /// Identifies which model/quantization is expected to be behind `host:port`. Stored
    /// alongside every embedding this provider produces so a future model swap can never have
    /// its vectors silently confused with an incompatible embedding space. Change this the
    /// moment the underlying GGUF changes — see `embedding_model_id`.
    pub model_label: String,
}

/// The model this project decided on (ADR-0004, user-approved 15 Ağustos 2026). Bump this the
/// moment the deployed GGUF changes, even to a different quantization of the same model — a
/// different quantization is not guaranteed to produce numerically comparable vectors.
pub const DEFAULT_EMBEDDING_MODEL_LABEL: &str = "Qwen3-Embedding-0.6B-Q8_0";

impl LlamaEmbeddingProvider {
    pub fn local_default() -> Self {
        Self {
            host: std::env::var("JARVIS_EMBEDDING_SERVER_HOST")
                .unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("JARVIS_EMBEDDING_SERVER_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(8090),
            timeout_seconds: 30,
            model_label: std::env::var("JARVIS_EMBEDDING_MODEL_LABEL")
                .unwrap_or_else(|_| DEFAULT_EMBEDDING_MODEL_LABEL.into()),
        }
    }

    pub fn is_reachable(&self) -> bool {
        self.request(serde_json::json!({"input": "ping"})).is_ok()
    }

    fn request(&self, body: Value) -> Result<Value, String> {
        let address = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|error| format!("embedding server address resolution failed: {error}"))?
            .next()
            .ok_or_else(|| "embedding server address has no socket".to_string())?;
        let timeout = Duration::from_secs(self.timeout_seconds.into());
        let mut stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("embedding server is unavailable: {error}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("embedding read timeout setup failed: {error}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| format!("embedding write timeout setup failed: {error}"))?;
        let body = serde_json::to_vec(&body)
            .map_err(|error| format!("embedding request serialization failed: {error}"))?;
        let request = format!(
            "POST /v1/embeddings HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.host,
            self.port,
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body))
            .map_err(|error| format!("embedding request write failed: {error}"))?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|error| format!("embedding response read failed: {error}"))?;
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| "embedding server returned malformed HTTP response".to_string())?;
        let headers = std::str::from_utf8(&response[..header_end])
            .map_err(|error| format!("embedding response headers were not UTF-8: {error}"))?;
        if !headers.starts_with("HTTP/1.1 200") {
            return Err(format!(
                "embedding server returned: {}",
                headers.lines().next().unwrap_or("unknown")
            ));
        }
        serde_json::from_slice(&response[header_end + 4..])
            .map_err(|error| format!("embedding response was not valid JSON: {error}"))
    }
}

impl EmbeddingProvider for LlamaEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let response = self.request(serde_json::json!({ "input": text }))?;
        let values = response
            .pointer("/data/0/embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| "embedding response missing data[0].embedding".to_string())?;
        parse_embedding_values(values)
    }

    /// The OpenAI-compatible `/v1/embeddings` endpoint llama.cpp serves already accepts an array
    /// `input` and returns one entry per text — this is a real batched request, not `embed`
    /// called in a loop. Each response entry carries its own `index`; entries are placed back by
    /// that index rather than by array position, since the server is not documented to guarantee
    /// response order matches request order.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let response = self.request(serde_json::json!({ "input": texts }))?;
        let data = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "embedding batch response missing data[]".to_string())?;
        if data.len() != texts.len() {
            return Err(format!(
                "embedding batch response had {} entries, expected {}",
                data.len(),
                texts.len()
            ));
        }
        let mut indexed = Vec::with_capacity(data.len());
        for entry in data {
            let index = entry
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| "embedding batch entry missing index".to_string())?
                as usize;
            let values = entry
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| "embedding batch entry missing embedding".to_string())?;
            indexed.push((index, parse_embedding_values(values)?));
        }
        indexed.sort_by_key(|(index, _)| *index);
        Ok(indexed.into_iter().map(|(_, vector)| vector).collect())
    }

    fn embedding_model_id(&self) -> &str {
        &self.model_label
    }
}

fn parse_embedding_values(values: &[Value]) -> Result<Vec<f32>, String> {
    values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|number| number as f32)
                .ok_or_else(|| "embedding value was not a number".to_string())
        })
        .collect()
}

/// Cosine similarity between two equal-length vectors, in `[-1, 1]`. Returns `0.0` for a
/// zero-length or mismatched-length pair instead of panicking or dividing by zero.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Serializes an embedding vector to little-endian `f32` bytes for SQLite BLOB storage.
pub fn serialize_embedding(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Deserializes bytes back into an embedding vector. Ignores a trailing partial value instead of
/// panicking, so a truncated/corrupt BLOB degrades to a shorter vector rather than crashing.
pub fn deserialize_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_is_one_for_identical_and_zero_for_orthogonal_or_mismatched() {
        assert!((cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0, 2.0, 3.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn embedding_bytes_round_trip_and_tolerate_a_truncated_tail() {
        let vector = vec![-0.5_f32, 0.25, 1.0, -2.75];
        let bytes = serialize_embedding(&vector);
        assert_eq!(bytes.len(), vector.len() * 4);
        assert_eq!(deserialize_embedding(&bytes), vector);

        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 1);
        assert_eq!(deserialize_embedding(&truncated).len(), vector.len() - 1);
    }
}
