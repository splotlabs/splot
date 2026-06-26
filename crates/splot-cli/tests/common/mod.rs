// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared helpers for the `splot-cli` integration tests.
//!
//! Each integration-test binary that needs these declares `mod common;`. Only a
//! subset of the helpers is used per binary, so unused-code warnings are allowed.

#![allow(dead_code)]

use std::path::Path;

/// Returns the sorted file names directly under `path`.
pub fn read_dir_names(path: &Path) -> Vec<String> {
    let mut entries = std::fs::read_dir(path)
        .expect("read temporary directory")
        .map(|entry| {
            entry
                .expect("read temporary directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}
