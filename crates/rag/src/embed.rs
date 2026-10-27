// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Embeddings from a provider's `POST /v1/embeddings` (OpenAI's shape:
//! OpenAI, Infomaniak AI Tools, any compatible gateway). Multilingual
//! models make « salaire » and « paie » neighbours — what the hashing
//! embedder cannot. The index calls [`Embedder`] synchronously, from a
//! parallel reindex or an HTTP handler, so the client owns a runtime of
//! its own and blocks on a thread that is not tokio's.

use ontology_index::Embedder;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::model::v1_api_url;

/// Texts per request: well under every provider's input limit, and a
/// unit of retry.
pub const BATCH: usize = 64;

pub struct RemoteEmbedder {
    http: reqwest::Client,
    rt: &'static tokio::runtime::Runtime,
    api_key: String,
    base_url: String,
    model: String,
    dim: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embeddings: {0}")]
    Http(String),
    #[error("embeddings: the provider answered without a vector for every text")]
    Shape,
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct Response {
    data: Vec<Datum>,
}

#[derive(Deserialize)]
struct Datum {
    index: usize,
    embedding: Vec<f32>,
}

impl RemoteEmbedder {
    /// Connects and embeds one probe text: the vector's length is the
    /// dimension every row of the index will have. A provider that does
    /// not answer is an error here, not later in a reindex.
    pub fn connect(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, EmbedError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| EmbedError::Http(e.to_string()))?;
        // Dropped inside an async context a runtime panics; this one lives
        // as long as the process, like the index it serves.
        let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(rt));
        let mut me = Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| EmbedError::Http(e.to_string()))?,
            rt,
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
            dim: 0,
        };
        let probe = me.call(&["dimension"])?;
        me.dim = probe.first().map(Vec::len).ok_or(EmbedError::Shape)?;
        if me.dim == 0 {
            return Err(EmbedError::Shape);
        }
        Ok(me)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// One request; the texts come back in their order.
    fn call(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let url = v1_api_url(&self.base_url, "embeddings");
        let body = serde_json::to_string(&Request {
            model: &self.model,
            input: texts,
        })
        .map_err(|e| EmbedError::Http(e.to_string()))?;
        let http = self.http.clone();
        let key = self.api_key.clone();
        let fut = async move {
            let mut attempt = 0u32;
            loop {
                let res = http
                    .post(&url)
                    .bearer_auth(&key)
                    .header("content-type", "application/json")
                    .body(body.clone())
                    .send()
                    .await;
                match res {
                    Ok(r) if r.status().is_success() => {
                        return r
                            .json::<Response>()
                            .await
                            .map_err(|e| EmbedError::Http(e.to_string()));
                    }
                    Ok(r)
                        if attempt < 2
                            && (r.status().as_u16() == 429 || r.status().is_server_error()) =>
                    {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        return Err(EmbedError::Http(format!(
                            "{status}: {}",
                            text.chars().take(200).collect::<String>()
                        )));
                    }
                    Err(e) if attempt < 2 && (e.is_connect() || e.is_timeout()) => {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                    }
                    Err(e) => return Err(EmbedError::Http(e.to_string())),
                }
            }
        };
        // `block_on` on a tokio worker thread panics; a scoped thread is not one.
        let resp = std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(fut))
                .join()
                .unwrap_or_else(|_| Err(EmbedError::Http("embedding thread panicked".into())))
        })?;
        if resp.data.len() != texts.len() {
            return Err(EmbedError::Shape);
        }
        let mut out = vec![Vec::new(); texts.len()];
        for d in resp.data {
            let slot = out.get_mut(d.index).ok_or(EmbedError::Shape)?;
            *slot = d.embedding;
        }
        if out.iter().any(Vec::is_empty) {
            return Err(EmbedError::Shape);
        }
        Ok(out)
    }

    /// Every text embedded, `BATCH` per request. A failed request leaves
    /// its texts as zero vectors — no score, never a wrong one — and is
    /// logged; the next reindex retries them (the cache keeps no zeros).
    pub fn embed_all(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH) {
            match self.call(chunk) {
                Ok(v) => out.extend(v),
                Err(e) => {
                    tracing::warn!(error = %e, texts = chunk.len(), "embeddings request failed");
                    out.extend(chunk.iter().map(|_| vec![0.0; self.dim]));
                }
            }
        }
        out
    }
}

impl Embedder for RemoteEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> String {
        format!("{}@{}", self.model, self.base_url)
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        self.embed_all(&[text]).pop().unwrap_or_default()
    }

    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        self.embed_all(texts)
    }
}
