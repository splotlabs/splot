// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CLI subcommand implementations and shared helpers.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

mod atlas;
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod explain;
pub(crate) mod inspect;
pub(crate) mod validate;

/// Reads an input file into memory, attaching context on failure.
///
/// # Errors
/// Returns an error if the file cannot be read.
fn read_input(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("failed to read input file: {}", path.display()))
}
