// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CLI subcommand implementations and shared helpers.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

pub(crate) mod decode;
pub(crate) mod explain;
pub(crate) mod inspect;
pub(crate) mod validate;

/// Initializes `tracing` from verbosity flags. Logs are written to stderr so that
/// stdout stays clean for machine-readable output (`--json`).
pub(crate) fn init_tracing(verbose: u8, quiet: bool) {
    let max_level = if quiet {
        tracing::Level::ERROR
    } else {
        match verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(max_level)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Reads an input file into memory, attaching context on failure.
///
/// # Errors
/// Returns an error if the file cannot be read.
fn read_input(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("failed to read input file: {}", path.display()))
}
