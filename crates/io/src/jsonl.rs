use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::ingest::{IngestError, Sink, Source};
use crate::record::Record;

/// Reads newline-delimited JSON [`Record`]s from a file.
pub struct JsonlSource {
    lines: Lines<BufReader<File>>,
}

impl JsonlSource {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, IngestError> {
        let file = File::open(path.as_ref()).await?;
        Ok(Self { lines: BufReader::new(file).lines() })
    }
}

#[async_trait]
impl Source for JsonlSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        loop {
            let line = match self.lines.next_line().await? {
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
            .create(true).truncate(true).write(true)
            .open(&path).await?;
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
