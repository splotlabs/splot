// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared git and hashing helpers for xtask automation.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest, Sha256};

pub(crate) fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let display = format!("git -C {} {}", root.display(), args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{display}` failed with {}; stderr:\n{}",
            output.status,
            stderr.trim_end()
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("`{display}` emitted non-UTF-8"))
}

/// Lowercase hex SHA-256 of `bytes` (via the `sha2` crate), as used in the
/// spec-mirror `CHECKSUMS` manifest.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a `String` is infallible; discard the `Result` (the
        // workspace denies `unwrap`).
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
