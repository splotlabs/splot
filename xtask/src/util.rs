// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Small helpers shared across `xtask` modules.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

/// Reads and parses a TOML file at `path` into `T`, with read/parse context.
pub(crate) fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// Returns `true` if `s` matches `^[A-Z0-9]+(-[A-Z0-9.]+)+$`.
pub(crate) fn is_valid_feature_id(s: &str) -> bool {
    let mut parts = s.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty()
        || !first
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return false;
    }
    let mut count = 0usize;
    for part in parts {
        count += 1;
        if part.is_empty()
            || !part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.')
        {
            return false;
        }
    }
    count >= 1
}

/// Splits `value` into whitespace- and delimiter-separated tokens, trimming
/// trailing punctuation and dropping empties.
pub(crate) fn tokenized(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '<' | '>'))
        .map(|token| token.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.')))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Returns `true` if `token` looks like a Windows drive-letter absolute path
/// (for example `C:\` or `C:/`).
pub(crate) fn is_windows_absolute_path(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Creates a fresh, uniquely named scratch directory under the system temp
/// directory for tests, removing any stale directory with the same name first.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn temp_root(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}
