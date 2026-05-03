// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::ingest::{IngestError, Sink, Source};
use crate::record::Record;

/// Reads newline-delimited JSON [`Record`]s. Backed by either a file or
/// stdin — the source is opaque to consumers.
pub struct JsonlSource {
    lines: LinesReader,
}

enum LinesReader {
    File(Lines<BufReader<File>>),
    Stdin(Lines<BufReader<tokio::io::Stdin>>),
}

impl JsonlSource {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, IngestError> {
        let file = File::open(path.as_ref()).await?;
        Ok(Self {
            lines: LinesReader::File(BufReader::new(file).lines()),
        })
    }

    /// Read JSONL from stdin. Useful for piping: `cat data.jsonl | ontology ingest -`.
    pub fn stdin() -> Self {
        Self {
            lines: LinesReader::Stdin(BufReader::new(tokio::io::stdin()).lines()),
        }
    }
}

#[async_trait]
impl Source for JsonlSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        loop {
            let line = match &mut self.lines {
                LinesReader::File(l) => l.next_line().await?,
                LinesReader::Stdin(l) => l.next_line().await?,
            };
            let line = match line {
                Some(l) => l,
                None => return Ok(None),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let rec: Record = serde_json::from_str(trimmed)
                .map_err(|e| IngestError::Source(format!("jsonl: {e}")))?;
            return Ok(Some(rec));
        }
    }
}

/// Writes [`Record`]s as newline-delimited JSON to a file.
pub struct JsonlSink {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl JsonlSink {
    pub async fn create(path: impl AsRef<Path>) -> Result<Self, IngestError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .await?;
        Ok(Self { file, path })
    }
}

#[async_trait]
impl Sink for JsonlSink {
    async fn write(&mut self, record: &Record) -> Result<(), IngestError> {
        let line = serde_json::to_string(record)
            .map_err(|e| IngestError::Source(format!("jsonl: {e}")))?;
        self.file.write_all(line.as_bytes()).await?;
        self.file.write_all(b"\n").await?;
        Ok(())
    }
    async fn finish(&mut self) -> Result<(), IngestError> {
        self.file.flush().await?;
        Ok(())
    }
}
