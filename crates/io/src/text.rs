use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId};
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Ingester for free-text "documents" (think: contracts, policies, memos).
///
/// Each input becomes one [`Concept`] of the configured type:
/// * `name` is the file stem (e.g. `c-2025-001` for `c-2025-001.txt`),
/// * `description` is the full text of the file,
/// * no extra properties are emitted.
///
/// The [`HybridIndex`] then tokenizes that description for both lexical
/// (TF-IDF) and vector (cosine) retrieval, so questions about the document
/// content land on the right concept without any further wiring.
///
/// Two constructors:
/// * [`TextDocumentSource::from_files`] — explicit list of paths.
/// * [`TextDocumentSource::from_dir`] — every regular file under a directory
///   matching one of the given extensions.
///
/// [`HybridIndex`]: ontology_index::HybridIndex
pub struct TextDocumentSource {
    concept_type: String,
    queue: Vec<PathBuf>,
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
        })
    }
}

#[async_trait]
impl Source for TextDocumentSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        let path = match self.queue.pop() {
            Some(p) => p,
            None => return Ok(None),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.display().to_string());
        let body = fs::read_to_string(&path)
            .await
            .map_err(|e| IngestError::Source(format!("{}: {e}", path.display())))?;
        let mut concept = Concept::new(ConceptId(0), self.concept_type.clone(), name);
        concept.description = body;
        Ok(Some(Record::Concept(concept)))
    }
}
