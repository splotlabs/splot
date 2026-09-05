// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decoder/reconstruction support matrix automation.
//!
//! - `cargo xtask decoder-support` renders decoder support status on demand.
//! - `cargo xtask check-decoder-support` validates the matrix and any committed render.
//!
//! The matrix records local reference evidence as portable metadata only. This
//! module never probes for, locates, or invokes AVM, dav2d, or any other decoder.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

#[cfg(test)]
use crate::util::temp_root;
use crate::util::{is_valid_feature_id, is_windows_absolute_path, tokenized};

/// Repo-relative path of the canonical decoder support matrix.
const MATRIX_PATH: &str = "docs/DECODER-SUPPORT-MATRIX.toml";
/// Repo-relative path of the optional generated decoder support status document.
const STATUS_DOC_PATH: &str = "docs/DECODER-SUPPORT-STATUS.md";
/// Regeneration command printed by the drift check.
const REGEN_COMMAND: &str =
    "cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md";
/// The only decoder support matrix version this tool understands.
const SUPPORTED_MATRIX_VERSION: u32 = 1;
/// Allowed support statuses, in report order.
const STATUSES: &[&str] = &[
    "todo",
    "partial",
    "supported",
    "unsupported-intentional",
    "blocked",
];
/// Decoder/tool names that must not appear in self-contained test or fixture proof.
const EXTERNAL_DECODER_TOKENS: &[&str] = &[
    "aomdec",
    "aomenc",
    "avm",
    "avmdec",
    "avmenc",
    "dav1d",
    "dav2d",
    "decode_to_md5",
    "dump_obu",
    "ffmpeg",
    "ffprobe",
];

/// Output format for `decoder-support`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum DecoderSupportFormat {
    /// GitHub-flavored markdown status document.
    Markdown,
}

/// Implements `cargo xtask decoder-support`.
pub(crate) fn run_decoder_support(
    root: &Path,
    format: DecoderSupportFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    let matrix = load_matrix(root)?;
    let checked = validate_matrix(matrix)?;
    let rendered = match format {
        DecoderSupportFormat::Markdown => render_markdown(&checked),
    };

    if let Some(path) = output {
        std::fs::write(&path, &rendered)
            .with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!(
            "decoder-support: wrote {} row(s) to {}",
            checked.rows.len(),
            path.display()
        );
    } else {
        print!("{rendered}");
        if !rendered.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

/// Implements `cargo xtask check-decoder-support`.
pub(crate) fn run_check_decoder_support(root: &Path) -> Result<()> {
    let status_path = root.join(STATUS_DOC_PATH);
    let matrix = load_matrix(root)?;
    let checked = validate_matrix(matrix)?;
    validate_local_reference_evidence_links(root, &checked)?;
    if !status_path.exists() {
        eprintln!(
            "check-decoder-support: ok ({} row(s); {STATUS_DOC_PATH} is generated on demand)",
            checked.rows.len()
        );
        return Ok(());
    }

    let expected = render_markdown(&checked);
    let actual = std::fs::read_to_string(&status_path)
        .with_context(|| format!("failed to read {}", status_path.display()))?;
    if actual.trim_end() != expected.trim_end() {
        bail!("{STATUS_DOC_PATH} is out of date; regenerate with `{REGEN_COMMAND}`");
    }

    eprintln!("check-decoder-support: ok ({} row(s))", checked.rows.len());
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Matrix {
    #[serde(rename = "matrix_version")]
    version: Option<u32>,
    #[serde(default)]
    last_reviewed: Option<String>,
    #[serde(default)]
    row: Vec<Row>,
}

#[derive(Debug, Deserialize)]
struct Row {
    id: Option<String>,
    name: Option<String>,
    feature_id: Option<String>,
    spec_sections: Option<Vec<String>>,
    parser_source: Option<String>,
    decode_module: Option<String>,
    tier: Option<String>,
    status: Option<String>,
    self_contained_tests: Option<Vec<String>>,
    #[serde(default)]
    fixtures: Vec<String>,
    diagnostics: Option<Vec<String>>,
    local_reference_evidence: Option<Vec<String>>,
    notes: Option<String>,
}

#[derive(Debug)]
struct CheckedMatrix {
    last_reviewed: Option<String>,
    rows: Vec<CheckedRow>,
}

#[derive(Debug)]
struct CheckedRow {
    id: String,
    name: String,
    feature_id: String,
    spec_sections: Vec<String>,
    decode_module: String,
    tier: String,
    status: String,
    tests: Vec<String>,
    fixtures: Vec<String>,
    diagnostics: Vec<String>,
    reference_evidence: Vec<String>,
}

fn load_matrix(root: &Path) -> Result<Matrix> {
    let path = root.join(MATRIX_PATH);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_matrix(&text)
}

fn parse_matrix(text: &str) -> Result<Matrix> {
    toml::from_str::<Matrix>(text).context("failed to parse the decoder support matrix")
}

fn validate_matrix(matrix: Matrix) -> Result<CheckedMatrix> {
    let mut problems = Vec::new();

    match matrix.version {
        Some(SUPPORTED_MATRIX_VERSION) => {}
        Some(other) => {
            problems.push(format!(
                "unsupported matrix_version {other} (this tool supports {SUPPORTED_MATRIX_VERSION})"
            ));
        }
        None => {
            problems.push("missing required field `matrix_version`".to_owned());
        }
    }

    if matrix.row.is_empty() {
        problems.push("matrix has no [[row]] entries".to_owned());
    }
    if let Some(last_reviewed) = matrix.last_reviewed.as_deref()
        && let Some(path) = local_absolute_path(last_reviewed)
    {
        problems.push(format!(
            "matrix last_reviewed contains local absolute path `{path}`"
        ));
    }

    let mut seen = BTreeSet::new();
    let mut checked = Vec::new();
    for (index, row) in matrix.row.into_iter().enumerate() {
        let label = row
            .id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .map_or_else(|| format!("row {}", index + 1), |id| format!("row `{id}`"));

        let id = required_string(&mut problems, &label, "id", row.id);
        if let Some(id) = id.as_deref() {
            if !seen.insert(id.to_owned()) {
                problems.push(format!("{label}: duplicate row id `{id}`"));
            }
            if !is_valid_row_id(id) {
                problems.push(format!(
                    "{label}: row id `{id}` must be lowercase kebab-case or an uppercase Feature ID"
                ));
            }
        }

        let name = required_string(&mut problems, &label, "name", row.name);
        let feature_id =
            required_string_allow_empty(&mut problems, &label, "feature_id", row.feature_id);
        if let Some(feature_id) = feature_id.as_deref()
            && !feature_id.is_empty()
            && !is_valid_feature_id(feature_id)
        {
            problems.push(format!(
                "{label}: feature_id `{feature_id}` does not match ^[A-Z0-9]+(-[A-Z0-9.]+)+$"
            ));
        }

        let spec_sections =
            required_string_list(&mut problems, &label, "spec_sections", row.spec_sections);
        let parser_source =
            required_string(&mut problems, &label, "parser_source", row.parser_source);
        let decode_module =
            required_string(&mut problems, &label, "decode_module", row.decode_module);
        let tier = required_string(&mut problems, &label, "tier", row.tier);
        let status = required_string(&mut problems, &label, "status", row.status);
        if let Some(status) = status.as_deref()
            && !STATUSES.contains(&status)
        {
            problems.push(format!(
                "{label}: status `{status}` is invalid (allowed: {})",
                STATUSES.join(", ")
            ));
        }

        let tests = required_string_list(
            &mut problems,
            &label,
            "self_contained_tests",
            row.self_contained_tests,
        );
        let fixtures = validate_string_list(&mut problems, &label, "fixtures", row.fixtures);
        let diagnostics =
            required_string_list(&mut problems, &label, "diagnostics", row.diagnostics);
        let reference_evidence = required_string_list(
            &mut problems,
            &label,
            "local_reference_evidence",
            row.local_reference_evidence,
        );
        let notes = required_string(&mut problems, &label, "notes", row.notes);

        if status.as_deref() == Some("supported")
            && tests.as_ref().is_none_or(Vec::is_empty)
            && fixtures.is_empty()
        {
            problems.push(format!(
                "{label}: status `supported` requires at least one self-contained test or fixture"
            ));
        }

        for (field, values) in [
            ("tests", tests.as_deref().unwrap_or(&[])),
            ("fixtures", fixtures.as_slice()),
            ("spec_sections", spec_sections.as_deref().unwrap_or(&[])),
            ("diagnostics", diagnostics.as_deref().unwrap_or(&[])),
            (
                "local_reference_evidence",
                reference_evidence.as_deref().unwrap_or(&[]),
            ),
        ] {
            for value in values {
                if let Some(path) = local_absolute_path(value) {
                    problems.push(format!(
                        "{label}: {field} entry `{value}` contains local absolute path `{path}`"
                    ));
                }
            }
        }

        for (field, values) in [
            ("tests", tests.as_deref().unwrap_or(&[])),
            ("fixtures", fixtures.as_slice()),
        ] {
            for value in values {
                if mentions_external_decoder(value) {
                    problems.push(format!(
                        "{label}: {field} entry `{value}` must be self-contained and not require an external decoder"
                    ));
                }
            }
        }

        for (field, value) in [
            ("name", name.as_deref()),
            ("parser_source", parser_source.as_deref()),
            ("decode_module", decode_module.as_deref()),
            ("tier", tier.as_deref()),
            ("notes", notes.as_deref()),
        ] {
            if let Some(value) = value
                && let Some(path) = local_absolute_path(value)
            {
                problems.push(format!(
                    "{label}: {field} contains local absolute path `{path}`"
                ));
            }
            if let Some(value) = value
                && let Some(path) = reference_decoder_path(value)
            {
                problems.push(format!(
                    "{label}: {field} mentions executable reference decoder path `{path}`"
                ));
            }
        }

        if let (
            Some(id),
            Some(name),
            Some(feature_id),
            Some(spec_sections),
            Some(_),
            Some(decode_module),
            Some(tier),
            Some(status),
            Some(tests),
            Some(diagnostics),
            Some(reference_evidence),
            Some(_),
        ) = (
            id,
            name,
            feature_id,
            spec_sections,
            parser_source,
            decode_module,
            tier,
            status,
            tests,
            diagnostics,
            reference_evidence,
            notes,
        ) {
            checked.push(CheckedRow {
                id,
                name,
                feature_id,
                spec_sections,
                decode_module,
                tier,
                status,
                tests,
                fixtures,
                diagnostics,
                reference_evidence,
            });
        }
    }

    if problems.is_empty() {
        Ok(CheckedMatrix {
            last_reviewed: matrix.last_reviewed,
            rows: checked,
        })
    } else {
        for problem in &problems {
            eprintln!("error: {problem}");
        }
        bail!(
            "{} decoder support matrix problem(s); fix {MATRIX_PATH}",
            problems.len()
        )
    }
}

fn required_string(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    value: Option<String>,
) -> Option<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Some(value),
        Some(_) => {
            problems.push(format!(
                "{label}: required field `{field}` must not be empty"
            ));
            None
        }
        None => {
            problems.push(format!("{label}: missing required field `{field}`"));
            None
        }
    }
}

fn required_string_allow_empty(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    value: Option<String>,
) -> Option<String> {
    if let Some(value) = value {
        Some(value)
    } else {
        problems.push(format!("{label}: missing required field `{field}`"));
        None
    }
}

fn required_string_list(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    value: Option<Vec<String>>,
) -> Option<Vec<String>> {
    if let Some(values) = value {
        Some(validate_string_list(problems, label, field, values))
    } else {
        problems.push(format!("{label}: missing required field `{field}`"));
        None
    }
}

fn validate_string_list(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    values: Vec<String>,
) -> Vec<String> {
    for (index, value) in values.iter().enumerate() {
        if value.trim().is_empty() {
            problems.push(format!(
                "{label}: `{field}` entry {} must not be empty",
                index + 1
            ));
        }
    }
    values
}

fn is_valid_row_id(s: &str) -> bool {
    is_valid_feature_id(s)
        || s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn validate_local_reference_evidence_links(root: &Path, checked: &CheckedMatrix) -> Result<()> {
    let evidence_index = crate::reference_evidence::load_checked_reference_evidence_index(root)?;
    validate_reference_evidence_links(&checked.rows, &evidence_index)?;
    eprintln!(
        "check-reference-evidence: ok ({} evidence entr{})",
        evidence_index.evidence_count(),
        if evidence_index.evidence_count() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    Ok(())
}

fn validate_reference_evidence_links(
    rows: &[CheckedRow],
    evidence_index: &crate::reference_evidence::ReferenceEvidenceIndex,
) -> Result<()> {
    let mut problems = Vec::new();
    for row in rows {
        for evidence in &row.reference_evidence {
            let Some(evidence_id) =
                crate::reference_evidence::canonical_evidence_pointer_id(evidence)
            else {
                continue;
            };
            if evidence_id.trim().is_empty() {
                problems.push(format!(
                    "row `{}`: local_reference_evidence pointer `{evidence}` is missing an evidence id",
                    row.id
                ));
                continue;
            }
            let Some(rows_for_evidence) = evidence_index.rows_for(evidence_id) else {
                problems.push(format!(
                    "row `{}`: local_reference_evidence pointer `{evidence}` references unknown evidence id `{evidence_id}`",
                    row.id
                ));
                continue;
            };
            if !rows_for_evidence.contains(&row.id) {
                problems.push(format!(
                    "row `{}`: local_reference_evidence pointer `{evidence}` is not reciprocal; evidence `{evidence_id}` does not list row `{}` in decoder_support_rows",
                    row.id, row.id
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("error: {problem}");
        }
        bail!(
            "{} decoder support reference evidence link problem(s); fix {MATRIX_PATH} or {}",
            problems.len(),
            crate::reference_evidence::MANIFEST_PATH
        )
    }
}

fn render_markdown(matrix: &CheckedMatrix) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Decoder Support Status");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Generated from `{MATRIX_PATH}` by `cargo xtask decoder-support --format markdown`. Do not edit by hand."
    );
    let _ = writeln!(out);
    let reviewed = matrix.last_reviewed.as_deref().unwrap_or("unknown");
    let _ = writeln!(
        out,
        "Matrix version {}. Last reviewed {}. {} row(s).",
        SUPPORTED_MATRIX_VERSION,
        reviewed,
        matrix.rows.len()
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## Status Counts");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Status | Rows |");
    let _ = writeln!(out, "|---|---:|");
    let counts = counts_by_status(&matrix.rows);
    for status in STATUSES {
        let _ = writeln!(out, "| `{status}` | {} |", counts.get(status).unwrap_or(&0));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Tier Counts");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Tier | Rows |");
    let _ = writeln!(out, "|---|---:|");
    for (tier, count) in counts_by_tier(&matrix.rows) {
        let _ = writeln!(out, "| `{}` | {count} |", md_escape(&tier));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Rows");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| ID | Name | Feature | Tier | Status | Spec Sections | Tests | Diagnostics | Local Evidence | Module |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|");
    for row in &matrix.rows {
        let tests = tests_and_fixtures(row);
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | `{}` | `{}` | {} | {} | {} | {} | `{}` |",
            md_escape(&row.id),
            md_escape(&row.name),
            optional_code_cell(&row.feature_id),
            md_escape(&row.tier),
            row.status,
            list_cell(&row.spec_sections),
            list_cell(&tests),
            list_cell(&row.diagnostics),
            list_cell(&row.reference_evidence),
            md_escape(&row.decode_module),
        );
    }
    out
}

fn counts_by_status(rows: &[CheckedRow]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.status.as_str()).or_insert(0) += 1;
    }
    counts
}

fn counts_by_tier(rows: &[CheckedRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.tier.clone()).or_insert(0) += 1;
    }
    counts
}

fn tests_and_fixtures(row: &CheckedRow) -> Vec<String> {
    row.tests
        .iter()
        .cloned()
        .chain(
            row.fixtures
                .iter()
                .map(|fixture| format!("fixture: {fixture}")),
        )
        .collect()
}

fn list_cell(values: &[String]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|value| md_escape(value))
        .collect::<Vec<_>>()
        .join("<br>")
}

fn optional_code_cell(value: &str) -> String {
    if value.is_empty() {
        "none".to_owned()
    } else {
        format!("`{}`", md_escape(value))
    }
}

fn md_escape(cell: &str) -> String {
    cell.replace('|', "\\|")
}

fn mentions_external_decoder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    EXTERNAL_DECODER_TOKENS
        .iter()
        .any(|token| lower.contains(token))
}

fn reference_decoder_path(value: &str) -> Option<String> {
    path_fragments(value).into_iter().find(|fragment| {
        let lower = fragment.to_ascii_lowercase();
        looks_absolute_path(fragment)
            && EXTERNAL_DECODER_TOKENS
                .iter()
                .any(|decoder| lower.contains(decoder))
    })
}

fn local_absolute_path(value: &str) -> Option<String> {
    path_fragments(value)
        .into_iter()
        .find(|fragment| looks_local_absolute_path(fragment))
}

fn path_fragments(value: &str) -> Vec<String> {
    tokenized(value)
        .into_iter()
        .flat_map(|token| {
            let mut fragments = vec![token.clone()];
            fragments.extend(
                token
                    .split('=')
                    .skip(1)
                    .map(str::to_owned)
                    .filter(|fragment| !fragment.is_empty()),
            );
            fragments
        })
        .collect()
}

fn looks_local_absolute_path(token: &str) -> bool {
    token.starts_with("file://") || looks_absolute_path(token)
}

fn looks_absolute_path(token: &str) -> bool {
    if token.contains("://") {
        return false;
    }
    token.starts_with('/') || token.starts_with("~/") || is_windows_absolute_path(token)
}

#[cfg(test)]
const SAMPLE: &str = r#"
matrix_version = 1
last_reviewed = "2026-06-13"

[[row]]
id = "dec-b-row"
name = "B row"
feature_id = ""
spec_sections = ["7.1"]
parser_source = "crates/splot-core/src/stream.rs"
decode_module = "crates/splot-decode/src/context.rs"
tier = "tier-0"
status = "todo"
self_contained_tests = []
diagnostics = ["decode/unsupported"]
local_reference_evidence = ["AVM commit f6f0b9c89 raw hash metadata"]
notes = "planned"

[[row]]
id = "dec-a-row"
name = "A row"
feature_id = "DEC-A-ROW"
spec_sections = []
parser_source = "crates/splot-core/src/obu.rs"
decode_module = "crates/splot-decode/src/context.rs"
tier = "tier-1"
status = "supported"
self_contained_tests = ["cargo test -p xtask decoder_support"]
fixtures = ["tests/conformance/vectors/valid/syn-key-intra-64x64.ivf"]
diagnostics = []
local_reference_evidence = []
notes = "done"
"#;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod link_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn valid_matrix_renders_rows_deterministically() -> Result<()> {
        let matrix = validate_matrix(parse_matrix(SAMPLE)?)?;
        let rendered = render_markdown(&matrix);
        assert!(rendered.contains("| `todo` | 1 |"));
        assert!(rendered.contains("| `supported` | 1 |"));
        assert!(rendered.contains("| `tier-0` | 1 |"));
        assert!(
            rendered.contains("fixture: tests/conformance/vectors/valid/syn-key-intra-64x64.ivf")
        );
        assert!(rendered.contains("| `dec-b-row` | B row | none |"));
        let a = rendered.find("`dec-a-row`").expect("A row is rendered");
        let b = rendered.find("`dec-b-row`").expect("B row is rendered");
        assert!(b < a, "rows preserve matrix order for deterministic output");
        Ok(())
    }

    #[test]
    fn invalid_status_and_missing_required_field_are_rejected() -> Result<()> {
        let bad = SAMPLE
            .replace(r#"status = "todo""#, r#"status = "done""#)
            .replace(r#"parser_source = "crates/splot-core/src/stream.rs""#, "");
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        let message = err.to_string();
        assert!(message.contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn supported_row_requires_test_or_fixture() -> Result<()> {
        let bad = SAMPLE
            .replace(
                r#"self_contained_tests = ["cargo test -p xtask decoder_support"]"#,
                "self_contained_tests = []",
            )
            .replace(
                r#"fixtures = ["tests/conformance/vectors/valid/syn-key-intra-64x64.ivf"]"#,
                "fixtures = []",
            );
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        assert!(err.to_string().contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn supported_proof_rejects_external_decoders() -> Result<()> {
        let bad = SAMPLE.replace(
            r#"self_contained_tests = ["cargo test -p xtask decoder_support"]"#,
            r#"self_contained_tests = ["ffmpeg -i fixture.ivf -f md5 -"]"#,
        );
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        assert!(err.to_string().contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn reference_evidence_rejects_local_absolute_paths() -> Result<()> {
        let bad = SAMPLE.replace(
            r#"local_reference_evidence = ["AVM commit f6f0b9c89 raw hash metadata"]"#,
            r#"local_reference_evidence = ["/Users/me/Devel/avm/build/avmdec --rawvideo"]"#,
        );
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        assert!(err.to_string().contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn reference_evidence_rejects_prefixed_local_absolute_paths() -> Result<()> {
        for evidence in [
            r#"local_reference_evidence = ["AVM=/Users/me/Devel/avm/build/avmdec"]"#,
            r#"local_reference_evidence = ["--decoder=/home/me/dav2d/build/dav2d"]"#,
        ] {
            let bad = SAMPLE.replace(
                r#"local_reference_evidence = ["AVM commit f6f0b9c89 raw hash metadata"]"#,
                evidence,
            );
            let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
            assert!(err.to_string().contains("decoder support matrix problem"));
        }
        Ok(())
    }

    #[test]
    fn rendered_fields_reject_local_absolute_paths() -> Result<()> {
        let bad = SAMPLE.replace(
            r#"decode_module = "crates/splot-decode/src/context.rs""#,
            r#"decode_module = "file:///Users/me/scratch/context.rs""#,
        );
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        assert!(err.to_string().contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn metadata_rejects_local_absolute_paths() -> Result<()> {
        let bad = SAMPLE.replace(
            r#"last_reviewed = "2026-06-13""#,
            r#"last_reviewed = "file:///Users/me/review.txt""#,
        );
        let err = validate_matrix(parse_matrix(&bad)?).expect_err("matrix should be rejected");
        assert!(err.to_string().contains("decoder support matrix problem"));
        Ok(())
    }

    #[test]
    fn check_decoder_support_detects_drift() -> Result<()> {
        let root = temp_root("decoder-support-drift")?;
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs)?;
        std::fs::write(docs.join("DECODER-SUPPORT-MATRIX.toml"), SAMPLE)?;
        let expected = render_markdown(&validate_matrix(parse_matrix(SAMPLE)?)?);
        std::fs::write(docs.join("DECODER-SUPPORT-STATUS.md"), expected)?;
        std::fs::write(
            docs.join("LOCAL-REFERENCE-EVIDENCE.toml"),
            "manifest_version = 1\nlast_reviewed = \"2026-06-13\"\n",
        )?;

        run_check_decoder_support(&root)?;

        std::fs::write(docs.join("DECODER-SUPPORT-STATUS.md"), "stale\n")?;
        let err = run_check_decoder_support(&root).expect_err("drift should fail");
        assert!(err.to_string().contains(REGEN_COMMAND));

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn check_decoder_support_skips_when_generated_status_is_absent() -> Result<()> {
        let root = temp_root("decoder-support-absent")?;
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs)?;
        std::fs::write(docs.join("DECODER-SUPPORT-MATRIX.toml"), SAMPLE)?;
        std::fs::write(
            docs.join("LOCAL-REFERENCE-EVIDENCE.toml"),
            "manifest_version = 1\nlast_reviewed = \"2026-06-13\"\n",
        )?;
        run_check_decoder_support(&root)?;
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
