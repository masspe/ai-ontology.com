// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::extract::extract_from_text;
use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Ingester for free-text "documents" (think: contracts, policies, memos).
///
/// Each input becomes one [`Concept`] of the configured type (`name` =
/// file stem, `description` = full body). The body is additionally fed to
/// [`extract_from_text`] so any inline `@concept`, `@relation`, `@rule`
/// and `@action` directives lift their declarations into the ontology and
/// the graph automatically.
///
/// Two constructors:
/// * [`TextDocumentSource::from_files`] — explicit list of paths.
/// * [`TextDocumentSource::from_dir`] — every regular file under a directory
///   matching one of the given extensions.
///
/// [`Concept`]: ontology_graph::Concept
/// [`HybridIndex`]: ontology_index::HybridIndex
pub struct TextDocumentSource {
    concept_type: String,
    queue: Vec<PathBuf>,
    pending: Vec<Record>,
}

impl TextDocumentSource {
    pub fn from_files<I, P>(concept_type: impl Into<String>, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        Self {
            concept_type: concept_type.into(),
            queue: paths
                .into_iter()
                .map(|p| p.as_ref().to_path_buf())
                .collect(),
            pending: Vec::new(),
        }
    }

    pub async fn from_dir(
        concept_type: impl Into<String>,
        dir: impl AsRef<Path>,
        extensions: &[&str],
    ) -> Result<Self, IngestError> {
        let dir = dir.as_ref();
        let mut entries = fs::read_dir(dir).await?;
        let mut queue = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !entry.file_type().await?.is_file() {
                continue;
            }
            let ext_ok = match path.extension().and_then(|e| e.to_str()) {
                Some(ext) => extensions.iter().any(|w| w.eq_ignore_ascii_case(ext)),
                None => extensions.is_empty(),
            };
            if ext_ok {
                queue.push(path);
            }
        }
        // Stable order across runs — predictable for caching and debugging.
        queue.sort();
        Ok(Self {
            concept_type: concept_type.into(),
            queue,
            pending: Vec::new(),
        })
    }
}

/// Decode arbitrary file bytes into a `String` for text ingestion.
///
/// Delegates to [`crate::charset::decode_to_utf8`] which auto-detects
/// UTF-8 (with or without BOM), UTF-16 LE/BE, and legacy single-byte
/// encodings (Windows-1252, ISO-8859-*) via `chardetng`. Result is NFC
/// normalized; decoding never fails (malformed bytes become U+FFFD).
fn decode_text(raw: &[u8]) -> String {
    crate::charset::decode_to_utf8(raw).text
}

#[async_trait]
impl Source for TextDocumentSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        loop {
            if let Some(r) = self.pending.pop() {
                return Ok(Some(r));
            }
            let path = match self.queue.pop() {
                Some(p) => p,
                None => return Ok(None),
            };
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| path.display().to_string());
            let raw = fs::read(&path)
                .await
                .map_err(|e| IngestError::Source(format!("{}: {e}", path.display())))?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase());
            let is_docx = ext.as_deref() == Some("docx")
                || (crate::docx::is_zip(&raw)
                    && ext.as_deref().map_or(false, |e| {
                        matches!(e, "docx" | "docm" | "dotx" | "dotm")
                    }));
            let body = if is_docx {
                crate::docx::extract_docx_text(&raw).map_err(|e| {
                    IngestError::Source(format!("{}: docx extract: {e}", path.display()))
                })?
            } else {
                decode_text(&raw)
            };
            // Extract emits records in dependency-friendly order; we drain
            // via `pop`, so reverse to preserve that order to the consumer.
            let mut recs = extract_from_text(&self.concept_type, &name, &body);
            recs.reverse();
            self.pending = recs;
        }
    }
}
