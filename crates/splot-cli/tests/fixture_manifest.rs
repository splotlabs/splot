// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Verifies the `tests/fixtures/MANIFEST.toml` `expect` outcomes against the real
//! validator (XTASK-CHECK-FIXTURES).
//!
//! `cargo xtask check-fixtures` checks the manifest's hashes, presence, and
//! `category`/`expect` *shape* hermetically. This complementary test closes the
//! loop in-process: for every `[[fixture]]` it runs
//! `splot_validate::Validator::validate_bytes` (the same entry point the CLI
//! `validate` command uses — no external decoder) and asserts the recorded
//! `expect`, so the manifest's outcomes can never silently drift from the
//! validator. It also enforces the same orphan guard as the conformance corpus.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use splot_validate::Validator;

/// The manifest root: an array of `[[fixture]]` entries.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    fixture: Vec<FixtureEntry>,
}

/// One fixture entry (only the fields this outcome check needs).
#[derive(Debug, Deserialize)]
struct FixtureEntry {
    name: String,
    path: String,
    expect: Expect,
}

/// The expected validation outcome. `expect = "clean"` deserializes from the bare
/// string; `expect = { diagnostics = [...] }` from the table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Expect {
    Clean(CleanTag),
    Diagnostics { diagnostics: Vec<String> },
}

#[derive(Debug, Deserialize)]
enum CleanTag {
    #[serde(rename = "clean")]
    Clean,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("tests/fixtures/ exists")
}

#[test]
fn fixture_manifest_outcomes_match_validator() {
    let root = fixtures_root();
    let manifest_path = root.join("MANIFEST.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path).expect("read MANIFEST.toml");
    let manifest: Manifest = toml::from_str(&manifest_text).expect("parse MANIFEST.toml");

    assert!(
        !manifest.fixture.is_empty(),
        "MANIFEST.toml has no [[fixture]] entries"
    );

    // Anti-vacuity: the corpus must exercise both arms.
    let mut saw_clean = false;
    let mut saw_diagnostics = false;

    // The validator is stateless across fixtures; non-strict matches the CLI default.
    let validator = Validator::new(false);

    let mut manifest_paths: BTreeSet<String> = BTreeSet::new();

    for entry in &manifest.fixture {
        manifest_paths.insert(entry.path.clone());
        let file_path = root.join(&entry.path);
        let read_msg = format!("read fixture {}", file_path.display());
        let bytes = std::fs::read(&file_path).expect(&read_msg);
        let report = validator.validate_bytes(&bytes);
        let got: BTreeSet<&str> = report.errors().map(|d| d.rule_id.as_str()).collect();

        match &entry.expect {
            Expect::Clean(CleanTag::Clean) => {
                saw_clean = true;
                assert!(
                    got.is_empty(),
                    "fixture `{}` expected `clean` but the validator emitted error(s): {got:?}",
                    entry.name
                );
            }
            Expect::Diagnostics { diagnostics } => {
                saw_diagnostics = true;
                let want: BTreeSet<&str> = diagnostics.iter().map(String::as_str).collect();
                assert!(
                    !want.is_empty(),
                    "fixture `{}` has an empty `diagnostics` set; use `expect = \"clean\"`",
                    entry.name
                );
                assert_eq!(
                    got, want,
                    "fixture `{}` error rule ids mismatch: expected {want:?}, got {got:?}",
                    entry.name
                );
            }
        }
    }

    assert!(saw_clean, "manifest exercises no `clean` fixture");
    assert!(
        saw_diagnostics,
        "manifest exercises no `diagnostics` fixture; the diagnostics arm would be vacuous"
    );

    // Every committed `.av2` fixture must appear in the manifest (the same orphan
    // guard cargo xtask check-fixtures enforces, mirrored here under cargo test).
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(&root)
        .expect("read tests/fixtures/")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("av2") {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("fixture file name is UTF-8")
                .to_owned();
            if !manifest_paths.contains(&name) {
                orphans.push(name);
            }
        }
    }
    assert!(
        orphans.is_empty(),
        "committed fixture(s) missing from MANIFEST.toml: {orphans:?}"
    );
}
