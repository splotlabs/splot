// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The committed fixture-manifest gate (`cargo xtask check-fixtures`).
//!
//! Verifies that the hand-crafted `tests/fixtures/*.av2` corpus and its
//! `tests/fixtures/MANIFEST.toml` agree: every committed fixture is listed exactly
//! once, exists, and matches its recorded `sha256`, with unique `name`/`path` and a
//! `category` consistent with `expect`. The check is hermetic — it reads files and
//! computes SHA-256 only; it never spawns the validator, invokes a decoder, or
//! touches the network. The `expect` outcomes are separately verified against the
//! real validator in-process by `crates/splot-cli/tests/fixture_manifest.rs`.
//!
//! Mirrors the manifest schema of [`crate::conformance`] and the problem-count
//! reporting style of the spec-mirror gate.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::git_util::{run_git, sha256_hex};

/// The fixture conflict-zone error string used in the parse-error `expect` arm.
const PARSE_ERROR_RULE_ID: &str = "bitstream/parse-error";

/// The manifest root: an array of `[[fixture]]` entries.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    fixture: Vec<FixtureEntry>,
}

/// One fixture entry.
#[derive(Debug, Deserialize)]
struct FixtureEntry {
    /// Stable short id, unique across the manifest.
    name: String,
    /// Path relative to `tests/fixtures/`, unique across the manifest.
    path: String,
    /// Lowercase hex SHA-256 of the committed bytes.
    sha256: String,
    /// One-line human note.
    #[allow(dead_code)]
    description: String,
    /// Outcome category.
    category: Category,
    /// Expected validation outcome (verified for real by the in-process test).
    expect: Expect,
}

/// The outcome category. An unknown string fails the manifest parse loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Category {
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "validation-error")]
    ValidationError,
    #[serde(rename = "parse-error")]
    ParseError,
}

/// The expected validation outcome. `expect = "clean"` deserializes from the bare
/// string; `expect = { diagnostics = [...] }` from the table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Expect {
    /// `expect = "clean"`: the validator must report zero errors.
    Clean(CleanTag),
    /// `expect = { diagnostics = [rule_id, ...] }`: the exact error rule-id set.
    Diagnostics { diagnostics: Vec<String> },
}

/// The literal `"clean"` string tag for [`Expect::Clean`].
#[derive(Debug, Deserialize)]
enum CleanTag {
    #[serde(rename = "clean")]
    Clean,
}

/// Returns `true` when `s` is exactly 64 lowercase hex characters.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validates one entry's metadata (format + category/expect consistency) without
/// touching the filesystem. Returns a problem string per defect.
fn validate_metadata(entry: &FixtureEntry) -> Vec<String> {
    let mut problems = Vec::new();
    let label = if entry.name.is_empty() {
        entry.path.clone()
    } else {
        entry.name.clone()
    };
    if entry.name.is_empty() {
        problems.push(format!(
            "fixture with path `{}` has an empty name",
            entry.path
        ));
    }
    if entry.path.is_empty() {
        problems.push(format!("fixture `{label}` has an empty path"));
    }
    if !is_sha256_hex(&entry.sha256) {
        problems.push(format!(
            "fixture `{label}`: sha256 `{}` is not 64 lowercase hex chars",
            entry.sha256
        ));
    }
    if let Expect::Diagnostics { diagnostics } = &entry.expect
        && diagnostics.is_empty()
    {
        problems.push(format!(
            "fixture `{label}`: empty `diagnostics` set; use `expect = \"clean\"` instead"
        ));
    }
    if let Some(problem) = category_expect_consistency(&label, entry.category, &entry.expect) {
        problems.push(problem);
    }
    problems
}

/// Enforces that `category` matches the shape of `expect` (see the manifest schema
/// comment). Returns a problem string when they disagree.
fn category_expect_consistency(label: &str, category: Category, expect: &Expect) -> Option<String> {
    let is_parse_error_set = matches!(
        expect,
        Expect::Diagnostics { diagnostics }
            if diagnostics.len() == 1 && diagnostics[0] == PARSE_ERROR_RULE_ID
    );
    match (category, expect) {
        (Category::Valid, Expect::Clean(_)) => None,
        (Category::Valid, Expect::Diagnostics { .. }) => Some(format!(
            "fixture `{label}`: category `valid` requires `expect = \"clean\"`"
        )),
        (Category::ParseError, _) if is_parse_error_set => None,
        (Category::ParseError, _) => Some(format!(
            "fixture `{label}`: category `parse-error` requires \
             `expect = {{ diagnostics = [\"{PARSE_ERROR_RULE_ID}\"] }}`"
        )),
        (Category::ValidationError, Expect::Diagnostics { diagnostics })
            if !diagnostics.is_empty() && !is_parse_error_set =>
        {
            None
        }
        (Category::ValidationError, _) => Some(format!(
            "fixture `{label}`: category `validation-error` requires a non-empty \
             `diagnostics` set other than the bare `{PARSE_ERROR_RULE_ID}` set"
        )),
    }
}

/// Verifies `tests/fixtures/` against `tests/fixtures/MANIFEST.toml`: presence,
/// SHA-256, uniqueness, metadata/category consistency, and no orphan committed
/// `.av2` files. Hermetic (no validator, no decoder, no network).
pub(crate) fn check_fixtures(root: &Path) -> Result<()> {
    let fixtures_dir = root.join("tests").join("fixtures");
    let manifest_path = fixtures_dir.join("MANIFEST.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: Manifest = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

    if manifest.fixture.is_empty() {
        bail!(
            "manifest {} has no [[fixture]] entries",
            manifest_path.display()
        );
    }

    let mut problems: Vec<String> = Vec::new();
    let mut seen_names: BTreeSet<&str> = BTreeSet::new();
    let mut seen_paths: BTreeSet<&str> = BTreeSet::new();

    for entry in &manifest.fixture {
        problems.extend(validate_metadata(entry));
        if !entry.name.is_empty() && !seen_names.insert(entry.name.as_str()) {
            problems.push(format!("duplicate fixture name `{}`", entry.name));
        }
        if !entry.path.is_empty() && !seen_paths.insert(entry.path.as_str()) {
            problems.push(format!("duplicate fixture path `{}`", entry.path));
        }
        // Presence + content hash.
        let file_path = fixtures_dir.join(&entry.path);
        match std::fs::read(&file_path) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                if actual != entry.sha256 {
                    problems.push(format!(
                        "fixture `{}` ({}): sha256 mismatch — manifest {}… on-disk {}…",
                        entry.name,
                        entry.path,
                        short(&entry.sha256),
                        short(&actual),
                    ));
                }
            }
            Err(_) => problems.push(format!(
                "fixture `{}`: manifest path `{}` does not exist",
                entry.name, entry.path
            )),
        }
    }

    // Orphans: every committed `tests/fixtures/*.av2` must be in the manifest.
    for tracked in run_git(root, &["ls-files", "tests/fixtures"])?.lines() {
        let tracked = tracked.trim();
        let Some(rel) = tracked.strip_prefix("tests/fixtures/") else {
            continue;
        };
        if rel.ends_with(".av2") && !seen_paths.contains(rel) {
            problems.push(format!(
                "committed fixture `{rel}` is not in MANIFEST.toml (never hash-checked)"
            ));
        }
    }

    if problems.is_empty() {
        eprintln!("check-fixtures: ok ({} fixture(s))", manifest.fixture.len());
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("{problem}");
        }
        bail!("check-fixtures: {} problem(s)", problems.len())
    }
}

/// First 8 chars of a hash for compact mismatch reporting.
fn short(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(category: Category, expect: Expect) -> FixtureEntry {
        FixtureEntry {
            name: "x".to_owned(),
            path: "x.av2".to_owned(),
            sha256: "0".repeat(64),
            description: "d".to_owned(),
            category,
            expect,
        }
    }

    fn clean() -> Expect {
        Expect::Clean(CleanTag::Clean)
    }

    fn diags(ids: &[&str]) -> Expect {
        Expect::Diagnostics {
            diagnostics: ids.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn sha256_hex_format() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(is_sha256_hex(&"0123456789abcdef".repeat(4)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"A".repeat(64))); // uppercase rejected
        assert!(!is_sha256_hex(&"g".repeat(64))); // non-hex rejected
    }

    #[test]
    fn consistent_categories_pass() {
        assert!(validate_metadata(&entry(Category::Valid, clean())).is_empty());
        assert!(
            validate_metadata(&entry(
                Category::ParseError,
                diags(&["bitstream/parse-error"])
            ))
            .is_empty()
        );
        assert!(
            validate_metadata(&entry(
                Category::ValidationError,
                diags(&["celu/missing-output-frame-unit"])
            ))
            .is_empty()
        );
    }

    #[test]
    fn inconsistent_categories_are_flagged() {
        // valid must be clean.
        assert!(!validate_metadata(&entry(Category::Valid, diags(&["x/y"]))).is_empty());
        // parse-error must be exactly the bare parse-error set.
        assert!(!validate_metadata(&entry(Category::ParseError, clean())).is_empty());
        assert!(
            !validate_metadata(&entry(
                Category::ParseError,
                diags(&["bitstream/parse-error", "x/y"])
            ))
            .is_empty()
        );
        // validation-error must not be the bare parse-error set or clean.
        assert!(
            !validate_metadata(&entry(
                Category::ValidationError,
                diags(&["bitstream/parse-error"])
            ))
            .is_empty()
        );
        assert!(!validate_metadata(&entry(Category::ValidationError, clean())).is_empty());
    }

    #[test]
    fn empty_diagnostics_set_is_flagged() {
        assert!(!validate_metadata(&entry(Category::ValidationError, diags(&[]))).is_empty());
    }

    #[test]
    fn manifest_parses_both_expect_arms() -> Result<()> {
        let text = r#"
[[fixture]]
name = "a"
path = "a.av2"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
description = "clean one"
category = "valid"
expect = "clean"

[[fixture]]
name = "b"
path = "b.av2"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
description = "negative one"
category = "validation-error"
expect = { diagnostics = ["celu/missing-output-frame-unit"] }
"#;
        let manifest: Manifest = toml::from_str(text)?;
        assert_eq!(manifest.fixture.len(), 2);
        assert!(matches!(
            manifest.fixture[0].expect,
            Expect::Clean(CleanTag::Clean)
        ));
        assert_eq!(manifest.fixture[0].category, Category::Valid);
        Ok(())
    }
}
