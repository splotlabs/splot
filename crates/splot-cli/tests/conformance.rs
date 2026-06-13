// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed conformance-corpus runner (CONF-AVM-VALID-STREAMS).
//!
//! Loads `tests/conformance/manifest.toml`, reads each committed vector's bytes,
//! validates them with `splot_validate::Validator::validate_bytes` (the same
//! entry point the CLI `validate` command uses), and asserts the manifest's
//! expected outcome. This is the CI gate: it runs under `cargo test`, hence under
//! `cargo xtask ci`.
//!
//! There is NO AVM dependency: the runner only validates already-committed vector
//! bytes against the manifest, and never invokes AVM or touches the network. AVM
//! is the LOCAL generator of the committed vectors only (see docs/CONFORMANCE.md).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use splot_validate::Validator;

/// The manifest root: an array of `[[vector]]` entries.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    vector: Vec<VectorEntry>,
}

/// One conformance-corpus vector entry.
#[derive(Debug, Deserialize)]
struct VectorEntry {
    /// Path to the committed vector, relative to `tests/conformance/`.
    path: String,
    /// Human-readable note (unused by the runner, present for documentation).
    #[allow(dead_code)]
    description: String,
    /// Expected validation outcome.
    expect: Expect,
}

/// The expected validation outcome for a vector.
///
/// `expect = "clean"` deserializes from the bare string; `expect = { diagnostics
/// = [...] }` deserializes from the table. `#[serde(untagged)]` tries each arm in
/// order.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Expect {
    /// `expect = "clean"`: the validator must report zero errors.
    Clean(CleanTag),
    /// `expect = { diagnostics = [rule_id, ...] }`: the validator must emit
    /// exactly this set of error rule ids.
    Diagnostics { diagnostics: Vec<String> },
}

/// The literal `"clean"` string tag for [`Expect::Clean`].
#[derive(Debug, Deserialize)]
enum CleanTag {
    #[serde(rename = "clean")]
    Clean,
}

/// Locates `tests/conformance/` from the crate manifest dir (`crates/splot-cli`),
/// robustly relative to the workspace root.
fn conformance_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .canonicalize()
        .expect("tests/conformance/ exists")
}

#[test]
fn conformance_corpus_matches_manifest() {
    let root = conformance_root();
    let manifest_path = root.join("manifest.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path).expect("read manifest.toml");
    let manifest: Manifest = toml::from_str(&manifest_text).expect("parse manifest.toml");

    assert!(
        !manifest.vector.is_empty(),
        "manifest {} has no [[vector]] entries",
        manifest_path.display()
    );

    // Anti-vacuity: the manifest must exercise both arms, so a regression in the
    // diagnostics arm cannot hide behind an all-clean corpus.
    let mut saw_clean = false;
    let mut saw_diagnostics = false;

    // The validator is stateless across vectors; a non-strict validator is the
    // CLI default (errors fail, warnings do not).
    let validator = Validator::new(false);

    // Track which committed vector files the manifest references, so we can fail
    // on any committed vector that is NOT in the manifest (otherwise a committed
    // vector would never be validated and the committed-corpus guarantee leaks).
    let mut manifest_paths: BTreeSet<PathBuf> = BTreeSet::new();

    for entry in &manifest.vector {
        let vector_path = root.join(&entry.path);
        manifest_paths.insert(vector_path.clone());
        // `expect` takes a `&str`, so build the path-bearing message first to
        // surface which committed vector is missing or unreadable.
        let read_msg = format!("read committed vector {}", vector_path.display());
        let bytes = std::fs::read(&vector_path).expect(&read_msg);
        let report = validator.validate_bytes(&bytes);
        let got: BTreeSet<&str> = report.errors().map(|d| d.rule_id.as_str()).collect();

        match &entry.expect {
            Expect::Clean(CleanTag::Clean) => {
                saw_clean = true;
                assert!(
                    got.is_empty(),
                    "vector {} expected `clean` but the validator emitted error(s): {got:?}",
                    entry.path
                );
            }
            Expect::Diagnostics { diagnostics } => {
                saw_diagnostics = true;
                let want: BTreeSet<&str> = diagnostics.iter().map(String::as_str).collect();
                assert!(
                    !want.is_empty(),
                    "vector {} has an empty `diagnostics` set; use `expect = \"clean\"` instead",
                    entry.path
                );
                assert_eq!(
                    got, want,
                    "vector {} error rule ids mismatch: expected {want:?}, got {got:?}",
                    entry.path
                );
            }
        }
    }

    assert!(
        saw_clean,
        "manifest exercises no `clean` vector; the corpus must include at least one"
    );
    assert!(
        saw_diagnostics,
        "manifest exercises no `diagnostics` vector; the diagnostics arm would be vacuous"
    );

    // Every committed vector file must appear in the manifest — otherwise it is
    // never validated and the committed-corpus guarantee silently leaks.
    let mut committed = Vec::new();
    collect_vector_files(&root.join("vectors"), &mut committed);
    let orphans: Vec<String> = committed
        .iter()
        .filter(|p| !manifest_paths.contains(*p))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        orphans.is_empty(),
        "committed vector(s) missing from manifest.toml (never validated): {orphans:?}"
    );
}

/// Recursively collects committed vector files (`.ivf` / `.av2` / `.obu`) under
/// `dir` into `out`. A missing directory yields nothing.
fn collect_vector_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_vector_files(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "ivf" | "av2" | "obu"))
        {
            out.push(path);
        }
    }
}
