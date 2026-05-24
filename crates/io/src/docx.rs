// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl

//! Minimal `.docx` text extraction.
//!
//! A `.docx` is a ZIP archive whose `word/document.xml` entry holds the body
//! as a stream of `<w:p>` paragraphs containing `<w:t>` text runs. We pull
//! out the text runs and join paragraphs with newlines — enough to feed the
//! plain-text ingest pipeline. Formatting, tables, headers/footers and
//! comments are intentionally dropped.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::{Cursor, Read};

/// Returns `true` if `bytes` look like a ZIP / Office Open XML container.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..4] == b"PK\x03\x04"
}

/// Extract plain text from a `.docx` byte buffer. Returns `Err` if the
/// archive is malformed or has no `word/document.xml`.
pub fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("not a valid docx (zip): {e}"))?;
    let mut xml = String::new();
    {
        let mut entry = zip
            .by_name("word/document.xml")
            .map_err(|e| format!("missing word/document.xml: {e}"))?;
        entry
            .read_to_string(&mut xml)
            .map_err(|e| format!("read document.xml: {e}"))?;
    }

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut buf = Vec::new();
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = name.as_ref();
                if local.ends_with(b"t") || local.ends_with(b":t") {
                    in_text = true;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let local = name.as_ref();
                if local.ends_with(b"t") || local.ends_with(b":t") {
                    in_text = false;
                } else if local.ends_with(b"p") || local.ends_with(b":p") {
                    out.push('\n');
                } else if local.ends_with(b"tab") || local.ends_with(b":tab") {
                    out.push('\t');
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let local = name.as_ref();
                if local.ends_with(b"br") || local.ends_with(b":br") {
                    out.push('\n');
                } else if local.ends_with(b"tab") || local.ends_with(b":tab") {
                    out.push('\t');
                }
            }
            Ok(Event::Text(t)) if in_text => {
                let s = t
                    .unescape()
                    .map_err(|e| format!("xml unescape: {e}"))?;
                out.push_str(&s);
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("xml parse: {e}")),
            _ => {}
        }
        buf.clear();
    }

    // Collapse runs of blank lines and trim trailing whitespace.
    let mut cleaned = String::with_capacity(out.len());
    let mut blank_run = 0u8;
    for line in out.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_run = blank_run.saturating_add(1);
            if blank_run <= 1 {
                cleaned.push('\n');
            }
        } else {
            blank_run = 0;
            cleaned.push_str(trimmed);
            cleaned.push('\n');
        }
    }
    Ok(cleaned.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_zip_magic() {
        assert!(is_zip(b"PK\x03\x04rest"));
        assert!(!is_zip(b"hello"));
    }
}
