// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! Secrets at rest. `settings.json` is the credential store (provider
//! keys, later OIDC client secrets and data-source passwords); every
//! secret-named field in it is sealed with one tenant master key,
//! AES-256-GCM, so a copy of the data directory without the key reveals
//! nothing. The key is `ONTOLOGY_MASTER_KEY` (64 hex chars) when set, else
//! the file `master.key` beside the settings file, generated owner-only on
//! first use — the same convention as `jwt.secret`.
//!
//! A sealed value reads `enc:v1:<base64 of nonce ‖ ciphertext>`; a value
//! without the prefix is plain text (a file written before this existed)
//! and is sealed on the next save.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;

/// Environment variable holding the key as 64 hex characters.
pub const ENV_VAR: &str = "ONTOLOGY_MASTER_KEY";
/// Key file beside the settings file.
pub const KEY_FILE: &str = "master.key";
const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

/// The tenant master key.
pub struct MasterKey([u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("sealed value is not valid base64")]
    Encoding,
    #[error("sealed value is too short")]
    Truncated,
    #[error("sealed value does not open with this key (wrong key or tampered)")]
    Rejected,
}

impl MasterKey {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        Self(raw)
    }

    /// 64 hex characters, as the environment variable and the file hold it.
    pub fn from_hex(hex: &str) -> std::io::Result<Self> {
        let hex = hex.trim();
        let bad = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "master key must be 64 hex characters",
            )
        };
        if hex.len() != 64 {
            return Err(bad());
        }
        let mut raw = [0u8; 32];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
        Ok(Self(raw))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Where the key file of `settings_path` lives.
    pub fn file_for(settings_path: &Path) -> PathBuf {
        settings_path.with_file_name(KEY_FILE)
    }

    /// The key for `settings_path`: the environment variable when set,
    /// else the key file, generated on first use.
    pub fn for_settings(settings_path: &Path) -> std::io::Result<Self> {
        if let Ok(hex) = std::env::var(ENV_VAR) {
            return Self::from_hex(&hex);
        }
        let path = Self::file_for(settings_path);
        match std::fs::read_to_string(&path) {
            Ok(hex) => Self::from_hex(&hex),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate();
                key.write(&path)?;
                tracing::info!(file = %path.display(), "generated the master key for secrets at rest");
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }

    /// Write the key, refusing to overwrite; owner-only on Unix (Windows
    /// ACLs are not touched: the file inherits the data directory's).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(path)?.write_all(self.to_hex().as_bytes())
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("32-byte key")
    }

    /// `plain` as a sealed value; a fresh nonce every time.
    pub fn seal(&self, plain: &str) -> String {
        use rand::RngCore;
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .expect("AES-GCM encryption cannot fail");
        let mut raw = nonce.to_vec();
        raw.extend(ct);
        format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    /// The plain text of `stored`: itself when it is not sealed.
    pub fn open(&self, stored: &str) -> Result<String, SecretError> {
        let Some(b64) = stored.strip_prefix(PREFIX) else {
            return Ok(stored.to_string());
        };
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| SecretError::Encoding)?;
        if raw.len() < NONCE_LEN {
            return Err(SecretError::Truncated);
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let plain = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| SecretError::Rejected)?;
        String::from_utf8(plain).map_err(|_| SecretError::Rejected)
    }
}

pub fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

/// Seal every non-empty string under a key `is_secret` accepts, at any
/// depth of `v`. Already-sealed values are left as they are.
pub fn seal_fields(v: &mut serde_json::Value, key: &MasterKey, is_secret: &dyn Fn(&str) -> bool) {
    walk(v, &mut |name, s| {
        if is_secret(name) && !s.is_empty() && !is_sealed(s) {
            *s = key.seal(s);
        }
        true
    });
}

/// Open every sealed string under a key `is_secret` accepts. A value the
/// key does not open is emptied (never served sealed to a provider) and
/// counted: the operator re-enters it.
pub fn open_fields(
    v: &mut serde_json::Value,
    key: &MasterKey,
    is_secret: &dyn Fn(&str) -> bool,
) -> usize {
    let mut failed = 0;
    walk(v, &mut |name, s| {
        if is_secret(name) && is_sealed(s) {
            match key.open(s) {
                Ok(plain) => *s = plain,
                Err(e) => {
                    tracing::warn!(field = name, error = %e, "secret does not open; cleared");
                    s.clear();
                    failed += 1;
                }
            }
        }
        true
    });
    failed
}

fn walk(v: &mut serde_json::Value, f: &mut dyn FnMut(&str, &mut String) -> bool) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if let serde_json::Value::String(s) = child {
                    f(k, s);
                } else {
                    walk(child, f);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn secret(name: &str) -> bool {
        name.ends_with("_api_key")
    }

    #[test]
    fn seal_and_open_round_trip_with_a_fresh_nonce_each_time() {
        let k = MasterKey::generate();
        let a = k.seal("sk-1234");
        let b = k.seal("sk-1234");
        assert!(is_sealed(&a) && a != b, "a fresh nonce every time");
        assert_eq!(k.open(&a).unwrap(), "sk-1234");
        assert_eq!(k.open(&b).unwrap(), "sk-1234");
        assert_eq!(
            k.open("plain").unwrap(),
            "plain",
            "unsealed values pass through"
        );
        assert_eq!(k.open("").unwrap(), "");
    }

    #[test]
    fn another_key_or_a_tampered_value_is_rejected() {
        let k = MasterKey::generate();
        let sealed = k.seal("sk-1234");
        assert!(matches!(
            MasterKey::generate().open(&sealed),
            Err(SecretError::Rejected)
        ));
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(&sealed[PREFIX.len()..])
            .unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        let tampered = format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        assert!(matches!(k.open(&tampered), Err(SecretError::Rejected)));
        assert!(matches!(k.open("enc:v1:%%%"), Err(SecretError::Encoding)));
        assert!(matches!(k.open("enc:v1:AAAA"), Err(SecretError::Truncated)));
    }

    #[test]
    fn hex_form_round_trips_and_rejects_bad_input() {
        let k = MasterKey::generate();
        let hex = k.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(MasterKey::from_hex(&format!(" {hex}\n")).unwrap().0, k.0);
        assert!(MasterKey::from_hex("abc").is_err());
        assert!(MasterKey::from_hex(&"zz".repeat(32)).is_err());
        assert!(format!("{k:?}").contains(".."), "never printed");
    }

    #[test]
    fn fields_are_sealed_and_opened_at_any_depth() {
        let k = MasterKey::generate();
        let mut v = json!({
            "llm": { "openai_api_key": "sk-1", "openai_api_key_hint": "sk-...1", "model": "x" },
            "ocr": { "google_api_key": "" },
            "list": [{ "nested": { "x_api_key": "deep" } }],
            "n": 3
        });
        seal_fields(&mut v, &k, &secret);
        assert!(is_sealed(v["llm"]["openai_api_key"].as_str().unwrap()));
        assert_eq!(v["llm"]["openai_api_key_hint"], "sk-...1", "not a secret");
        assert_eq!(v["llm"]["model"], "x");
        assert_eq!(v["ocr"]["google_api_key"], "", "empty stays empty");
        assert!(is_sealed(
            v["list"][0]["nested"]["x_api_key"].as_str().unwrap()
        ));
        let once = v.clone();
        seal_fields(&mut v, &k, &secret);
        assert_eq!(v, once, "sealing twice changes nothing");

        assert_eq!(open_fields(&mut v, &k, &secret), 0);
        assert_eq!(v["llm"]["openai_api_key"], "sk-1");
        assert_eq!(v["list"][0]["nested"]["x_api_key"], "deep");

        // The wrong key clears what it cannot open and counts it.
        let mut sealed = once;
        assert_eq!(open_fields(&mut sealed, &MasterKey::generate(), &secret), 2);
        assert_eq!(sealed["llm"]["openai_api_key"], "");
        assert_eq!(sealed["llm"]["model"], "x");
    }

    #[test]
    fn the_key_file_is_generated_once_beside_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("sub").join("settings.json");
        let first = MasterKey::for_settings(&settings).unwrap();
        let file = MasterKey::file_for(&settings);
        assert!(file.ends_with("sub/master.key") || file.ends_with("sub\\master.key"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), first.to_hex());
        let again = MasterKey::for_settings(&settings).unwrap();
        assert_eq!(again.0, first.0, "the same key is read back");
        assert!(first.write(&file).is_err(), "never overwritten");
        std::fs::write(&file, "garbage").unwrap();
        assert!(MasterKey::for_settings(&settings).is_err());
    }
}
