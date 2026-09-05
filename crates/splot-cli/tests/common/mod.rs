// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared helpers for the `splot-cli` integration tests.
//!
//! Each integration-test binary that needs these declares `mod common;`.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Returns the sorted file names directly under `path`.
pub(crate) fn read_dir_names(path: &Path) -> Vec<String> {
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

/// Returns the header-only IVF emitted by AVM when no input frames are encoded.
pub(crate) fn empty_avmenc_ivf() -> Vec<u8> {
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-empty-avmenc-64x64.ivf")
        .to_vec()
}

pub(crate) fn splot(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(args)
        .output()
        .expect("failed to run the splot binary")
}

pub(crate) fn temp_path(stem: &str, extension: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splot-decode-cli-test-{stem}-{}-{nanos}-{id}.{extension}",
        std::process::id()
    ))
}

pub(crate) fn temp_input(extension: &str, data: &[u8]) -> PathBuf {
    let path = temp_path("input", extension);
    std::fs::write(&path, data).expect("write temporary input");
    path
}

pub(crate) fn temp_output(extension: &str) -> PathBuf {
    temp_path("output", extension)
}

pub(crate) fn temp_dir(stem: &str) -> PathBuf {
    let path = temp_path(stem, "dir");
    std::fs::create_dir(&path).expect("create temporary directory");
    path
}

pub(crate) fn conformance_vector(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/vectors/valid")
        .join(name)
}
