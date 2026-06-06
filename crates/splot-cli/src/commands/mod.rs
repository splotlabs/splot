// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CLI subcommand implementations and shared helpers.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

pub mod decode;
pub mod encode;
pub mod inspect;
pub mod validate;

/// Initializes `tracing` from verbosity flags. Logs are written to stderr so that
/// stdout stays clean for machine-readable output (`--json`).
pub fn init_tracing(verbose: u8, quiet: bool) {
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Reads an input file into memory, attaching context on failure.
///
/// # Errors
/// Returns an error if the file cannot be read.
pub fn read_input(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("failed to read input file: {}", path.display()))
}
