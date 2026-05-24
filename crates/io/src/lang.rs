// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Lightweight language detection wrapper around [`whatlang`].
//!
//! We expose only what the ingest pipeline actually needs: a short ISO
//! 639-1 code (e.g. `"en"`, `"it"`), the script name, and a confidence
//! score. The result is attached to LLM-generated proposals so we can
//! prompt in the source language and tag the resulting concepts.

use serde::{Deserialize, Serialize};

/// Language tag attached to a document or concept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LangTag {
    /// ISO 639-1 two-letter code (e.g. `"en"`). Falls back to the
    /// `whatlang` 639-3 code (`"eng"`) when no two-letter form exists.
    pub code: String,
    /// Script name as reported by `whatlang` (e.g. `"Latin"`).
    pub script: String,
    /// Detector confidence in `[0.0, 1.0]`.
    pub confidence: f32,
}

/// Detect the dominant language of `text`. Returns `None` when the
/// detector cannot decide (typical for very short or mixed inputs).
pub fn detect_language(text: &str) -> Option<LangTag> {
    let info = whatlang::detect(text)?;
    let lang = info.lang();
    let code = iso_639_1(lang).unwrap_or_else(|| lang.code().to_string());
    Some(LangTag {
        code,
        script: format!("{:?}", info.script()),
        confidence: info.confidence() as f32,
    })
}

/// Map a `whatlang::Lang` to its ISO 639-1 two-letter code when one
/// exists. Returns `None` for languages without a 2-letter alias.
fn iso_639_1(lang: whatlang::Lang) -> Option<String> {
    use whatlang::Lang::*;
    let code = match lang {
        Eng => "en",
        Fra => "fr",
        Ita => "it",
        Spa => "es",
        Por => "pt",
        Deu => "de",
        Nld => "nl",
        Rus => "ru",
        Ukr => "uk",
        Pol => "pl",
        Ces => "cs",
        Slv => "sl",
        Hrv => "hr",
        Srp => "sr",
        Ron => "ro",
        Hun => "hu",
        Fin => "fi",
        Swe => "sv",
        Dan => "da",
        Nob => "no",
        Tur => "tr",
        Ell => "el",
        Bul => "bg",
        Cmn => "zh",
        Jpn => "ja",
        Kor => "ko",
        Vie => "vi",
        Tha => "th",
        Ara => "ar",
        Heb => "he",
        Hin => "hi",
        Ben => "bn",
        Urd => "ur",
        Pes => "fa",
        Ind => "id",
        _ => return None,
    };
    Some(code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_english() {
        let t = detect_language("The quick brown fox jumps over the lazy dog repeatedly.")
            .expect("detection");
        assert_eq!(t.code, "en");
    }

    #[test]
    fn detects_italian() {
        let t = detect_language(
            "Il contratto è stato firmato dalle parti in data odierna a Milano.",
        )
        .expect("detection");
        assert_eq!(t.code, "it");
    }

    #[test]
    fn detects_french() {
        let t = detect_language(
            "Le contrat a été signé par les parties à Paris le jour même de la livraison.",
        )
        .expect("detection");
        assert_eq!(t.code, "fr");
    }

    #[test]
    fn empty_returns_none() {
        assert!(detect_language("").is_none());
    }
}
