//! Channel-set loader for the Telegram run task.
//!
//! Per SPEC-001 §17.5 line 885 the canonical source of truth is the JSON
//! file `data/telegram-channels.json`, with the env var
//! `TELEGRAM_CHANNEL_SET` providing a comma-separated override. The
//! rebuild plan T4.5.0 line 1079 ("Channel set persisted in vault") is
//! superseded by Q2 = "data/telegram-channels.json + env override" per
//! the resolved decisions in
//! `~/.claude/plans/streamed-roaming-pond.md`.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default channel basket used when the JSON file is missing AND the
/// env override is unset. Mirrors the basket the legacy
/// `seed_telegram_intel_min` seeder shipped — every channel here has a
/// public preview, so MTProto will resolve `*Channel` access without
/// the user having explicitly subscribed.
pub const DEFAULT_CHANNELS: &[&str] = &[
    "bbcbreaking",
    "rianru",
    "tassagency_en",
    "voxgamma",
    "iaeaorg",
];

/// JSON file shape (`data/telegram-channels.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelSetFile {
    /// One channel slug per array entry — no leading `@`, no trailing
    /// whitespace.
    pub channels: Vec<String>,
}

/// Errors `load_channel_set` can produce.
#[derive(Debug, Error)]
pub enum ChannelLoadError {
    /// Reading the JSON file failed for a reason other than "not found".
    #[error("read {path}: {source}")]
    Read {
        /// File path that was being read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// JSON parse failed.
    #[error("parse {path}: {source}")]
    Parse {
        /// File path that was being parsed.
        path: String,
        /// `serde_json` error.
        #[source]
        source: serde_json::Error,
    },
}

/// Load the channel set, applying the env override if present.
///
/// Resolution order (later entries override earlier ones):
/// 1. [`DEFAULT_CHANNELS`] — used only when both the file and env are absent.
/// 2. JSON file at `path` if it exists.
/// 3. Env override `env_override` (comma-separated) if non-empty.
///
/// All slugs are trimmed, `@` prefixes stripped, deduplicated, and
/// returned in stable order.
///
/// # Errors
/// [`ChannelLoadError::Read`] if the file exists but cannot be opened.
/// [`ChannelLoadError::Parse`] if the file exists and parses but does
/// not match the [`ChannelSetFile`] schema.
pub fn load_channel_set(
    path: &Path,
    env_override: Option<&str>,
) -> Result<Vec<String>, ChannelLoadError> {
    let mut channels: Vec<String> = match std::fs::read_to_string(path) {
        Ok(s) => {
            let parsed: ChannelSetFile =
                serde_json::from_str(&s).map_err(|e| ChannelLoadError::Parse {
                    path: path.display().to_string(),
                    source: e,
                })?;
            parsed.channels
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            DEFAULT_CHANNELS.iter().map(|s| (*s).to_string()).collect()
        }
        Err(e) => {
            return Err(ChannelLoadError::Read {
                path: path.display().to_string(),
                source: e,
            });
        }
    };

    if let Some(raw) = env_override {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            channels = trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    Ok(normalise(channels))
}

fn normalise(input: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::with_capacity(input.len());
    for raw in input {
        let cleaned = raw.trim().trim_start_matches('@').to_string();
        if cleaned.is_empty() {
            continue;
        }
        if seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempfile_with(content: &str) -> tempfile::NamedTempFile {
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        tf.write_all(content.as_bytes()).unwrap();
        tf
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = std::path::Path::new("/tmp/this-does-not-exist-pellucid-test-zzzz-9876.json");
        let out = load_channel_set(path, None).unwrap();
        let expected: Vec<String> = DEFAULT_CHANNELS.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn json_file_overrides_defaults() {
        let tf = tempfile_with(r#"{"channels": ["a", "b", "@c", "  d  "]}"#);
        let out = load_channel_set(tf.path(), None).unwrap();
        assert_eq!(out, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn env_override_replaces_file() {
        let tf = tempfile_with(r#"{"channels": ["fromfile"]}"#);
        let out = load_channel_set(tf.path(), Some("env1, env2 ,@env3")).unwrap();
        assert_eq!(out, vec!["env1", "env2", "env3"]);
    }

    #[test]
    fn empty_env_override_does_not_replace() {
        let tf = tempfile_with(r#"{"channels": ["a", "b"]}"#);
        let out = load_channel_set(tf.path(), Some("   ")).unwrap();
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn duplicates_are_deduped_keeping_first() {
        let tf = tempfile_with(r#"{"channels": ["x", "y", "x", "@y"]}"#);
        let out = load_channel_set(tf.path(), None).unwrap();
        assert_eq!(out, vec!["x", "y"]);
    }

    #[test]
    fn invalid_json_returns_parse_error() {
        let tf = tempfile_with("not valid json");
        let err = load_channel_set(tf.path(), None).unwrap_err();
        assert!(matches!(err, ChannelLoadError::Parse { .. }));
    }
}
