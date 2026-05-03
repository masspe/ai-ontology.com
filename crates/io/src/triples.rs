// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// It is licensed under the GNU Lesser General Public License v3.0
// or (at your option) any later version. See LICENSE and LICENSE.GPL.

use async_trait::async_trait;
use ontology_graph::{Concept, ConceptId};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};

use crate::ingest::{IngestError, Source};
use crate::record::Record;

/// Minimal text triple format. Each non-empty, non-comment line is:
///
/// ```text
/// SourceType:SourceName  predicate  TargetType:TargetName
/// ```
///
/// Whitespace-separated; `#` starts a line comment. Concept records are
/// emitted before each relation that introduces them, so consumers don't
/// need to declare nodes upfront.
pub struct TripleSource {
    lines: Lines<BufReader<File>>,
    pending: Vec<Record>,
    seen: std::collections::HashSet<(String, String)>,
}

impl TripleSource {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, IngestError> {
        let f = File::open(path.as_ref()).await?;
        Ok(Self {
            lines: BufReader::new(f).lines(),
            pending: Vec::new(),
            seen: std::collections::HashSet::new(),
        })
    }

    fn parse_endpoint(s: &str) -> Result<(String, String), IngestError> {
        let mut parts = s.splitn(2, ':');
        let ty = parts.next().unwrap_or("");
        let name = parts.next().ok_or_else(|| {
            IngestError::Source(format!("malformed endpoint `{s}`: expected `Type:Name`"))
        })?;
        Ok((ty.to_string(), name.to_string()))
    }

    fn ensure_concept(&mut self, ty: &str, name: &str) {
        if self.seen.insert((ty.to_string(), name.to_string())) {
            let c = Concept::new(ConceptId(0), ty.to_string(), name.to_string());
            self.pending.push(Record::Concept(c));
        }
    }
}

#[async_trait]
impl Source for TripleSource {
    async fn next(&mut self) -> Result<Option<Record>, IngestError> {
        loop {
            if let Some(r) = self.pending.pop() {
                return Ok(Some(r));
            }
            let line = match self.lines.next_line().await? {
                Some(l) => l,
                None => return Ok(None),
            };
            let line = line.split('#').next().unwrap_or("").trim().to_string();
            if line.is_empty() {
                continue;
            }
            let toks: Vec<&str> = line.split_whitespace().collect();
            if toks.len() != 3 {
                return Err(IngestError::Source(format!(
                    "expected `Subj predicate Obj`, got `{line}`"
                )));
            }
            let (st, sn) = Self::parse_endpoint(toks[0])?;
            let (tt, tn) = Self::parse_endpoint(toks[2])?;
            let predicate = toks[1].to_string();

            // Emit endpoints first (popped from `pending` in reverse order),
            // and the relation is the value we return now.
            let rel = Record::NamedRelation {
                relation_type: predicate,
                source_type: st.clone(),
                source_name: sn.clone(),
                target_type: tt.clone(),
                target_name: tn.clone(),
                weight: 1.0,
            };
            // Only emit the relation immediately if both endpoints already
            // exist; otherwise emit the concepts now and the relation next.
            self.pending.push(rel);
            self.ensure_concept(&tt, &tn);
            self.ensure_concept(&st, &sn);
        }
    }
}
