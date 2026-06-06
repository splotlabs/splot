// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 feature-tracking matrix: load, render, and validate
//! `docs/IMPLEMENTATION-MATRIX.toml`, the canonical source of truth.
//!
//! - `cargo xtask feature-status`       renders the matrix (table/json/markdown).
//! - `cargo xtask check-feature-status` fails the build on drift.
//! - `cargo xtask spec-coverage`        summarizes coverage.
//!
//! The schema and rules are documented in `docs/IMPLEMENTATION-MATRIX.schema.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

/// Repo-relative path of the canonical matrix.
const MATRIX_PATH: &str = "docs/IMPLEMENTATION-MATRIX.toml";
/// Repo-relative path of the generated status document.
const STATUS_DOC_PATH: &str = "docs/FEATURE-STATUS.md";
/// The only `matrix_version` this tool understands.
const SUPPORTED_MATRIX_VERSION: u32 = 1;

/// Allowed `category` values.
const CATEGORIES: &[&str] = &[
    "normative",
    "encoder",
    "conformance",
    "cli",
    "docs",
    "automation",
    "infrastructure",
];
/// Allowed `kind` values.
const KINDS: &[&str] = &[
    "bitstream-syntax",
    "bitstream-semantics",
    "validator-check",
    "writer",
    "encoder-api",
    "encoder-tool",
    "cli",
    "conformance",
    "docs",
    "automation",
    "infrastructure",
];
/// Allowed `risk` values.
const RISKS: &[&str] = &["low", "medium", "high", "unknown"];
/// Allowed `crate` values (workspace members plus the `docs` pseudo-crate).
const CRATES: &[&str] = &[
    "splot-core",
    "splot-validate",
    "splot-encode",
    "splot-cli",
    "xtask",
    "fuzz",
    "docs",
];
/// Allowed `owner` values.
const OWNERS: &[&str] = &[
    "core",
    "validator",
    "encoder",
    "conformance",
    "cli",
    "automation",
    "docs",
];
/// Allowed status values for every stage.
const STATUSES: &[&str] = &[
    "todo",
    "partial",
    "done",
    "blocked",
    "not-applicable",
    "experimental",
    "pending",
];

/// All ten status stages, in display order.
const STAGE_NAMES: &[&str] = &[
    "mapped",
    "types",
    "parse",
    "validate",
    "write",
    "encode",
    "decode_check",
    "tests",
    "avm_diff",
    "perf",
];
/// Stages whose `done` requires recorded proof.
const CODE_STAGES: &[&str] = &[
    "parse",
    "validate",
    "write",
    "encode",
    "decode_check",
    "tests",
    "avm_diff",
    "perf",
];
/// Stages whose `partial`/`done`/`experimental` status requires the module to exist.
const IMPL_STAGES: &[&str] = &[
    "types",
    "parse",
    "validate",
    "write",
    "encode",
    "decode_check",
    "tests",
    "avm_diff",
    "perf",
];

/// Columns rendered by the table and markdown formats: `(header, stage)`.
///
/// This is a curated projection of [`STAGE_NAMES`]: `perf` is omitted to keep the
/// table readable (it is uniformly low-signal today). `decode_check` is included
/// because, for a validator-first project, "the validator/inspector can check this"
/// is a primary proof stage. `cargo xtask feature-status --format json` always
/// emits all ten stages.
const TABLE_COLUMNS: &[(&str, &str)] = &[
    ("Mapped", "mapped"),
    ("Types", "types"),
    ("Parse", "parse"),
    ("Validate", "validate"),
    ("Write", "write"),
    ("Encode", "encode"),
    ("DecChk", "decode_check"),
    ("Tests", "tests"),
    ("AVM", "avm_diff"),
];

/// Feature-ID prefixes recognized by the source/docs token scanner.
const FEATURE_ID_PREFIXES: &[&str] = &["AV2", "ENC", "CONF", "CLI", "XTASK", "DOC"];

/// Documented allowlist of feature-ID-shaped tokens that are intentional
/// placeholders/examples in documentation and templates, not real features.
const ALLOWLISTED_TOKENS: &[&str] = &["AV2-SECTION-SLUG"];

/// Documented validator diagnostic rule-id prefixes. Diagnostic rule ids use a
/// kebab/slash namespace that is separate from Feature IDs (see FEATURE-TRACKING.md).
const DIAGNOSTIC_PREFIXES: &[&str] = &["obu-header/", "obu-reserved/", "bitstream/"];

/// Output format for `feature-status`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum StatusFormat {
    /// Aligned plain-text table (default).
    Table,
    /// JSON for tooling.
    Json,
    /// GitHub-flavored markdown table.
    Markdown,
}

/// Output format for `spec-coverage`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum CoverageFormat {
    /// Plain-text summary (default).
    Text,
    /// Markdown summary.
    Markdown,
}

/// The whole matrix file.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Matrix {
    matrix_version: u32,
    #[serde(default)]
    last_reviewed: Option<String>,
    #[serde(default)]
    feature: Vec<Feature>,
}

/// One feature row.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Feature {
    id: String,
    name: String,
    category: String,
    kind: String,
    spec_sections: Vec<String>,
    sources: Vec<String>,
    #[serde(rename = "crate")]
    krate: String,
    module: String,
    openspec_change: String,
    tracking_issue: String,
    owner: String,
    risk: String,
    notes: String,
    #[serde(default)]
    replaces: Vec<String>,
    status: Status,
    proof: Proof,
}

/// The ten maturity stages of a feature.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Status {
    mapped: String,
    types: String,
    parse: String,
    validate: String,
    write: String,
    encode: String,
    decode_check: String,
    tests: String,
    avm_diff: String,
    perf: String,
}

impl Status {
    /// Returns the status value for `stage`, or `None` for an unknown stage name.
    fn get(&self, stage: &str) -> Option<&str> {
        Some(match stage {
            "mapped" => self.mapped.as_str(),
            "types" => self.types.as_str(),
            "parse" => self.parse.as_str(),
            "validate" => self.validate.as_str(),
            "write" => self.write.as_str(),
            "encode" => self.encode.as_str(),
            "decode_check" => self.decode_check.as_str(),
            "tests" => self.tests.as_str(),
            "avm_diff" => self.avm_diff.as_str(),
            "perf" => self.perf.as_str(),
            _ => return None,
        })
    }
}

/// Recorded proof for a feature row.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Proof {
    tests: Vec<String>,
    commands: Vec<String>,
    fixtures: Vec<String>,
    diagnostics: Vec<String>,
}

impl Proof {
    /// Returns `true` if no proof of any kind is recorded.
    fn is_empty(&self) -> bool {
        self.tests.is_empty()
            && self.commands.is_empty()
            && self.fixtures.is_empty()
            && self.diagnostics.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Parses a matrix from TOML text.
fn parse_matrix(text: &str) -> Result<Matrix> {
    toml::from_str::<Matrix>(text).context("failed to parse the implementation matrix")
}

/// Loads the matrix from `<root>/docs/IMPLEMENTATION-MATRIX.toml`.
fn load_matrix(root: &Path) -> Result<Matrix> {
    let path = root.join(MATRIX_PATH);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_matrix(&text)
}

/// Returns the features matching the optional `category`/`kind` filters.
fn filtered<'a>(
    matrix: &'a Matrix,
    category: Option<&str>,
    kind: Option<&str>,
) -> Vec<&'a Feature> {
    matrix
        .feature
        .iter()
        .filter(|f| category.is_none_or(|c| f.category == c))
        .filter(|f| kind.is_none_or(|k| f.kind == k))
        .collect()
}

// ---------------------------------------------------------------------------
// `feature-status`
// ---------------------------------------------------------------------------

/// Implements `cargo xtask feature-status`.
pub(crate) fn run_feature_status(
    root: &Path,
    format: StatusFormat,
    category: Option<String>,
    kind: Option<String>,
    output: Option<PathBuf>,
) -> Result<()> {
    let matrix = load_matrix(root)?;
    let features = filtered(&matrix, category.as_deref(), kind.as_deref());
    let rendered = match format {
        StatusFormat::Table => render_table(&features),
        StatusFormat::Json => render_json(&matrix, &features)?,
        StatusFormat::Markdown => render_markdown(&matrix, &features),
    };
    if let Some(path) = output {
        std::fs::write(&path, &rendered)
            .with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!(
            "feature-status: wrote {} feature(s) to {}",
            features.len(),
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

/// Abbreviates a status for compact display.
fn abbrev(status: &str) -> &str {
    match status {
        "not-applicable" => "n/a",
        "experimental" => "exp",
        other => other,
    }
}

/// Renders the aligned plain-text table.
fn render_table(features: &[&Feature]) -> String {
    let mut header: Vec<String> = vec![
        "ID".to_owned(),
        "Name".to_owned(),
        "Category".to_owned(),
        "Kind".to_owned(),
    ];
    for (label, _) in TABLE_COLUMNS {
        header.push((*label).to_owned());
    }
    header.push("Module".to_owned());

    let mut rows: Vec<Vec<String>> = vec![header];
    for f in features {
        let mut row = vec![
            f.id.clone(),
            f.name.clone(),
            f.category.clone(),
            f.kind.clone(),
        ];
        for (_, stage) in TABLE_COLUMNS {
            row.push(abbrev(f.status.get(stage).unwrap_or("?")).to_owned());
        }
        row.push(f.module.clone());
        rows.push(row);
    }

    let columns = rows.first().map_or(0, Vec::len);
    let mut widths = vec![0usize; columns];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.chars().count() > widths[i] {
                widths[i] = cell.chars().count();
            }
        }
    }

    let mut out = String::new();
    for (r, row) in rows.iter().enumerate() {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                line.push_str(" | ");
            }
            let pad = widths[i].saturating_sub(cell.chars().count());
            line.push_str(cell);
            for _ in 0..pad {
                line.push(' ');
            }
        }
        let _ = writeln!(out, "{}", line.trim_end());
        if r == 0 {
            // Separator under the header.
            let mut sep = String::new();
            for (i, width) in widths.iter().enumerate() {
                if i > 0 {
                    sep.push_str("-+-");
                }
                for _ in 0..*width {
                    sep.push('-');
                }
            }
            let _ = writeln!(out, "{}", sep.trim_end());
        }
    }
    if features.is_empty() {
        let _ = writeln!(out, "(no matching features)");
    }
    out
}

/// Renders JSON for tooling.
fn render_json(matrix: &Matrix, features: &[&Feature]) -> Result<String> {
    #[derive(Serialize)]
    struct JsonOut<'a> {
        matrix_version: u32,
        last_reviewed: &'a Option<String>,
        count: usize,
        feature: &'a [&'a Feature],
    }
    let out = JsonOut {
        matrix_version: matrix.matrix_version,
        last_reviewed: &matrix.last_reviewed,
        count: features.len(),
        feature: features,
    };
    let mut json = serde_json::to_string_pretty(&out).context("failed to serialize matrix JSON")?;
    json.push('\n');
    Ok(json)
}

/// Escapes a cell for a markdown table.
fn md_escape(cell: &str) -> String {
    cell.replace('|', "\\|")
}

/// Renders the deterministic markdown table used for `docs/FEATURE-STATUS.md`.
fn render_markdown(matrix: &Matrix, features: &[&Feature]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Feature status");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Generated from `docs/IMPLEMENTATION-MATRIX.toml` by `cargo xtask \
         feature-status --format markdown`. Do not edit by hand."
    );
    let _ = writeln!(out);
    let reviewed = matrix.last_reviewed.as_deref().unwrap_or("unknown");
    let _ = writeln!(
        out,
        "Matrix version {}. Last reviewed {}. {} feature(s).",
        matrix.matrix_version,
        reviewed,
        features.len()
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Status legend: `done` complete and proven, `partial` in progress, `todo` \
         not started, `pending` waiting on external proof, `blocked` blocked, `exp` \
         experimental, `n/a` not-applicable."
    );
    let _ = writeln!(out);

    let mut header = String::from("| ID | Name | Category | Kind |");
    for (label, _) in TABLE_COLUMNS {
        let _ = write!(header, " {label} |");
    }
    header.push_str(" Module |");
    let _ = writeln!(out, "{header}");

    let mut sep = String::from("|---|---|---|---|");
    for _ in TABLE_COLUMNS {
        sep.push_str("---|");
    }
    sep.push_str("---|");
    let _ = writeln!(out, "{sep}");

    for f in features {
        let mut row = format!(
            "| `{}` | {} | {} | {} |",
            f.id,
            md_escape(&f.name),
            f.category,
            f.kind
        );
        for (_, stage) in TABLE_COLUMNS {
            let _ = write!(row, " {} |", abbrev(f.status.get(stage).unwrap_or("?")));
        }
        let _ = write!(row, " `{}` |", f.module);
        let _ = writeln!(out, "{row}");
    }
    out
}

// ---------------------------------------------------------------------------
// `spec-coverage`
// ---------------------------------------------------------------------------

/// Implements `cargo xtask spec-coverage`.
pub(crate) fn run_spec_coverage(root: &Path, format: CoverageFormat) -> Result<()> {
    let matrix = load_matrix(root)?;
    let report = match format {
        CoverageFormat::Text => coverage_text(&matrix),
        CoverageFormat::Markdown => coverage_markdown(&matrix),
    };
    print!("{report}");
    if !report.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Counts features by a key projection, sorted by key.
fn count_by<'a>(
    matrix: &'a Matrix,
    key: impl Fn(&'a Feature) -> &'a str,
) -> BTreeMap<&'a str, usize> {
    let mut map = BTreeMap::new();
    for f in &matrix.feature {
        *map.entry(key(f)).or_insert(0) += 1;
    }
    map
}

/// Counts, per stage, how many features have that stage `done`.
fn done_counts(matrix: &Matrix) -> Vec<(&'static str, usize)> {
    STAGE_NAMES
        .iter()
        .map(|stage| {
            let n = matrix
                .feature
                .iter()
                .filter(|f| f.status.get(stage) == Some("done"))
                .count();
            (*stage, n)
        })
        .collect()
}

/// Normative features grouped by spec section.
fn normative_by_section(matrix: &Matrix) -> BTreeMap<&str, Vec<&str>> {
    let mut map: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &matrix.feature {
        if f.category != "normative" {
            continue;
        }
        for section in &f.spec_sections {
            map.entry(section.as_str()).or_default().push(f.id.as_str());
        }
    }
    map
}

/// `(feature id, stage, status)` triples whose status is `blocked` or `pending`.
fn blocked_pending(matrix: &Matrix) -> Vec<(&str, &str, &str)> {
    let mut out = Vec::new();
    for f in &matrix.feature {
        for stage in STAGE_NAMES {
            if let Some(status) = f.status.get(stage)
                && (status == "blocked" || status == "pending")
            {
                out.push((f.id.as_str(), *stage, status));
            }
        }
    }
    out
}

/// Feature ids that have progressed on a code stage but record no proof.
fn missing_proof(matrix: &Matrix) -> Vec<&str> {
    matrix
        .feature
        .iter()
        .filter(|f| {
            f.proof.is_empty()
                && IMPL_STAGES.iter().any(|stage| {
                    matches!(
                        f.status.get(stage),
                        Some("partial" | "done" | "experimental")
                    )
                })
        })
        .map(|f| f.id.as_str())
        .collect()
}

/// Plain-text coverage summary.
fn coverage_text(matrix: &Matrix) -> String {
    let mut out = String::new();
    let reviewed = matrix.last_reviewed.as_deref().unwrap_or("unknown");
    let _ = writeln!(
        out,
        "splot implementation coverage (matrix_version {}, last reviewed {})",
        matrix.matrix_version, reviewed
    );
    let _ = writeln!(out, "{} feature(s) total.", matrix.feature.len());
    let _ = writeln!(out);

    let _ = writeln!(out, "By category:");
    for (key, n) in count_by(matrix, |f| f.category.as_str()) {
        let _ = writeln!(out, "  {key:<14} {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "By kind:");
    for (key, n) in count_by(matrix, |f| f.kind.as_str()) {
        let _ = writeln!(out, "  {key:<20} {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "Stage completion (rows with stage = done):");
    for (stage, n) in done_counts(matrix) {
        let _ = writeln!(out, "  {stage:<14} {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "Normative features by spec section:");
    for (section, ids) in normative_by_section(matrix) {
        let _ = writeln!(out, "  {section:<10} {}", ids.join(", "));
    }
    let _ = writeln!(out);

    let bp = blocked_pending(matrix);
    let _ = writeln!(out, "Blocked / pending ({}):", bp.len());
    for (id, stage, status) in bp {
        let _ = writeln!(out, "  - {id}: {stage} {status}");
    }
    let _ = writeln!(out);

    let mp = missing_proof(matrix);
    let _ = writeln!(
        out,
        "Rows that progressed but record no proof ({}):",
        mp.len()
    );
    for id in mp {
        let _ = writeln!(out, "  - {id}");
    }
    out
}

/// Markdown coverage summary.
fn coverage_markdown(matrix: &Matrix) -> String {
    let mut out = String::new();
    let reviewed = matrix.last_reviewed.as_deref().unwrap_or("unknown");
    let _ = writeln!(out, "# Spec coverage");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Matrix version {}. Last reviewed {}. {} feature(s).",
        matrix.matrix_version,
        reviewed,
        matrix.feature.len()
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## By category");
    let _ = writeln!(out);
    for (key, n) in count_by(matrix, |f| f.category.as_str()) {
        let _ = writeln!(out, "- {key}: {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## By kind");
    let _ = writeln!(out);
    for (key, n) in count_by(matrix, |f| f.kind.as_str()) {
        let _ = writeln!(out, "- {key}: {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Stage completion (rows with stage = done)");
    let _ = writeln!(out);
    for (stage, n) in done_counts(matrix) {
        let _ = writeln!(out, "- {stage}: {n}");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Normative features by spec section");
    let _ = writeln!(out);
    for (section, ids) in normative_by_section(matrix) {
        let _ = writeln!(out, "- `{section}`: {}", ids.join(", "));
    }
    let _ = writeln!(out);

    let bp = blocked_pending(matrix);
    let _ = writeln!(out, "## Blocked / pending");
    let _ = writeln!(out);
    for (id, stage, status) in bp {
        let _ = writeln!(out, "- `{id}`: {stage} {status}");
    }
    let _ = writeln!(out);

    let mp = missing_proof(matrix);
    let _ = writeln!(out, "## Rows that progressed but record no proof");
    let _ = writeln!(out);
    for id in mp {
        let _ = writeln!(out, "- `{id}`");
    }
    out
}

// ---------------------------------------------------------------------------
// `check-feature-status`
// ---------------------------------------------------------------------------

/// Implements `cargo xtask check-feature-status`.
pub(crate) fn run_check_feature_status(root: &Path) -> Result<()> {
    let matrix = load_matrix(root)?;
    let mut checker = Checker::new(root, &matrix);
    checker.intrinsic(&matrix);
    checker.module_paths(&matrix);
    checker.scan_todos()?;
    checker.scan_tokens()?;
    checker.scan_diagnostics()?;
    checker.check_status_doc(&matrix)?;

    if checker.problems.is_empty() {
        eprintln!(
            "check-feature-status: ok ({} feature(s))",
            matrix.feature.len()
        );
        Ok(())
    } else {
        for problem in &checker.problems {
            eprintln!("error: {problem}");
        }
        bail!(
            "{} feature-status problem(s); see docs/IMPLEMENTATION-MATRIX.schema.md",
            checker.problems.len()
        )
    }
}

/// Accumulates check problems and the known-id context.
struct Checker {
    root: PathBuf,
    known: BTreeSet<String>,
    replaced: BTreeSet<String>,
    problems: Vec<String>,
}

impl Checker {
    fn new(root: &Path, matrix: &Matrix) -> Self {
        let mut known = BTreeSet::new();
        let mut replaced = BTreeSet::new();
        for f in &matrix.feature {
            known.insert(f.id.clone());
            for old in &f.replaces {
                replaced.insert(old.clone());
            }
        }
        Self {
            root: root.to_path_buf(),
            known,
            replaced,
            problems: Vec::new(),
        }
    }

    /// Returns `true` if `token` resolves to a known id, a `<known-id>.suffix`
    /// diagnostic sub-rule, a replaced id, or an allowlisted placeholder.
    fn token_ok(&self, token: &str) -> bool {
        if self.known.contains(token)
            || self.replaced.contains(token)
            || ALLOWLISTED_TOKENS.contains(&token)
        {
            return true;
        }
        self.known.iter().any(|id| is_suffixed(token, id))
    }

    /// Filesystem-free structural checks (also used by unit tests).
    fn intrinsic(&mut self, matrix: &Matrix) {
        if matrix.matrix_version != SUPPORTED_MATRIX_VERSION {
            self.problems.push(format!(
                "unsupported matrix_version {} (this tool supports {SUPPORTED_MATRIX_VERSION})",
                matrix.matrix_version
            ));
        }

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for f in &matrix.feature {
            if !seen.insert(f.id.as_str()) {
                self.problems
                    .push(format!("duplicate feature id `{}`", f.id));
            }
            if !is_valid_feature_id(&f.id) {
                self.problems.push(format!(
                    "feature id `{}` does not match the id regex ^[A-Z0-9]+(-[A-Z0-9.]+)+$",
                    f.id
                ));
            }
            if !CATEGORIES.contains(&f.category.as_str()) {
                self.problems
                    .push(format!("{}: unknown category `{}`", f.id, f.category));
            }
            if !KINDS.contains(&f.kind.as_str()) {
                self.problems
                    .push(format!("{}: unknown kind `{}`", f.id, f.kind));
            }
            if !RISKS.contains(&f.risk.as_str()) {
                self.problems
                    .push(format!("{}: unknown risk `{}`", f.id, f.risk));
            }
            if !CRATES.contains(&f.krate.as_str()) {
                self.problems
                    .push(format!("{}: unknown crate `{}`", f.id, f.krate));
            }
            if !OWNERS.contains(&f.owner.as_str()) {
                self.problems
                    .push(format!("{}: unknown owner `{}`", f.id, f.owner));
            }
            for stage in STAGE_NAMES {
                match f.status.get(stage) {
                    Some(value) if STATUSES.contains(&value) => {}
                    Some(value) => self.problems.push(format!(
                        "{}: stage `{stage}` has invalid status `{value}`",
                        f.id
                    )),
                    None => {}
                }
            }
            let done_code_stage = CODE_STAGES
                .iter()
                .any(|stage| f.status.get(stage) == Some("done"));
            if done_code_stage && f.proof.is_empty() {
                self.problems.push(format!(
                    "{}: a code stage is `done` but [feature.proof] is empty (record a test/command/fixture/diagnostic)",
                    f.id
                ));
            }
        }
    }

    /// Checks that each row's `module` exists when an implementation stage is active.
    fn module_paths(&mut self, matrix: &Matrix) {
        for f in &matrix.feature {
            let active = IMPL_STAGES.iter().any(|stage| {
                matches!(
                    f.status.get(stage),
                    Some("partial" | "done" | "experimental")
                )
            });
            if active && !self.root.join(&f.module).exists() {
                self.problems.push(format!(
                    "{}: module `{}` does not exist but an implementation stage is active",
                    f.id, f.module
                ));
            }
        }
    }

    /// Scans Rust source for spec TODO markers and validates each referenced id.
    fn scan_todos(&mut self) -> Result<()> {
        let needle: String = ["TODO", "(spec"].concat();
        let files = collect_files(&self.root, &["rs"])?;
        for path in files {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let mut from = 0usize;
            while let Some(rel) = text[from..].find(&needle) {
                let marker = from + rel;
                let after = marker + needle.len();
                let line = line_of(&text, marker);
                // Bound the marker to its own line so a stray `)` later in the file
                // cannot be mistaken for the marker's close paren.
                let rest = &text[after..];
                let line_rest = &rest[..rest.find('\n').unwrap_or(rest.len())];
                let location = format!("{}:{line}", display_path(&self.root, &path));
                if let Some(remainder) = line_rest.strip_prefix(':') {
                    // The id is the leading feature-id run after the colon; any
                    // trailing `): note` or `, note` is ignored.
                    let id: String = remainder
                        .trim_start()
                        .chars()
                        .take_while(|&c| {
                            c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.' || c == '-'
                        })
                        .collect();
                    if id.is_empty() {
                        self.problems.push(format!(
                            "{location}: spec TODO has no feature id (use the matrix id)"
                        ));
                    } else if !self.token_ok(&id) {
                        self.problems.push(format!(
                            "{location}: unknown feature id `{id}` in spec TODO; add a row to {MATRIX_PATH} or fix the id"
                        ));
                    }
                } else {
                    self.problems.push(format!(
                        "{location}: bare spec TODO without a feature id (use the matrix id)"
                    ));
                }
                from = after;
            }
        }
        Ok(())
    }

    /// Scans source and docs for feature-ID-shaped tokens and validates each.
    fn scan_tokens(&mut self) -> Result<()> {
        let files = collect_files(&self.root, &["rs", "md", "toml", "yml", "yaml"])?;
        for path in files {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let mut reported: BTreeSet<String> = BTreeSet::new();
            for token in extract_candidate_tokens(&text) {
                if !self.token_ok(&token) && reported.insert(token.clone()) {
                    self.problems.push(format!(
                        "{}: unknown feature-id token `{token}`; add a row to {MATRIX_PATH}, fix the id, or allowlist it in xtask",
                        display_path(&self.root, &path)
                    ));
                }
            }
        }
        Ok(())
    }

    /// Scans validator source for diagnostic rule ids using an undocumented prefix.
    fn scan_diagnostics(&mut self) -> Result<()> {
        let dir = self.root.join("crates/splot-validate/src");
        if !dir.exists() {
            return Ok(());
        }
        let files = collect_files(&dir, &["rs"])?;
        for path in files {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            for literal in string_literals(&text) {
                if is_diagnostic_id(&literal)
                    && !DIAGNOSTIC_PREFIXES.iter().any(|p| literal.starts_with(p))
                    && !self.known.contains(literal.as_str())
                {
                    self.problems.push(format!(
                        "{}: diagnostic rule id `{literal}` uses an undocumented prefix (allowed: {})",
                        display_path(&self.root, &path),
                        DIAGNOSTIC_PREFIXES.join(", ")
                    ));
                }
            }
        }
        Ok(())
    }

    /// Verifies `docs/FEATURE-STATUS.md`, if present, matches the matrix.
    fn check_status_doc(&mut self, matrix: &Matrix) -> Result<()> {
        let path = self.root.join(STATUS_DOC_PATH);
        if !path.exists() {
            return Ok(());
        }
        let actual = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let all: Vec<&Feature> = matrix.feature.iter().collect();
        let expected = render_markdown(matrix, &all);
        if actual.trim_end() != expected.trim_end() {
            self.problems.push(format!(
                "{STATUS_DOC_PATH} is out of date; regenerate with `cargo xtask feature-status --format markdown --output {STATUS_DOC_PATH}`"
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `s` matches `^[A-Z0-9]+(-[A-Z0-9.]+)+$`.
fn is_valid_feature_id(s: &str) -> bool {
    let mut parts = s.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty()
        || !first
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return false;
    }
    let mut count = 0usize;
    for part in parts {
        count += 1;
        if part.is_empty()
            || !part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.')
        {
            return false;
        }
    }
    count >= 1
}

/// Returns `true` if `token` equals `id` followed by `.` and a non-empty suffix.
fn is_suffixed(token: &str, id: &str) -> bool {
    token.len() > id.len() + 1
        && token.starts_with(id)
        && token.as_bytes().get(id.len()) == Some(&b'.')
}

/// `true` for bytes that can appear inside a feature-ID-shaped token.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
}

/// Extracts feature-ID-shaped tokens (known prefix, valid form) from `text`.
fn extract_candidate_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if is_token_byte(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_token_byte(bytes[i]) {
                i += 1;
            }
            let raw = &text[start..i];
            let trimmed = raw.trim_matches(|c| c == '.' || c == '-');
            if trimmed.contains('-')
                && trimmed
                    .split('-')
                    .next()
                    .is_some_and(|p| FEATURE_ID_PREFIXES.contains(&p))
                && is_valid_feature_id(trimmed)
            {
                out.push(trimmed.to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Returns the 1-based line number of byte offset `idx` within `text`.
fn line_of(text: &str, idx: usize) -> usize {
    text[..idx].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Returns the path relative to `root` for display, or the full path.
fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Returns `true` if `s` looks like a validator diagnostic rule id: lowercase
/// kebab segments, optionally `/`-separated — e.g. `obu-header/foo` or `parse-error`.
///
/// Slash-less ids are recognized too, so an un-namespaced rule id is still checked
/// against the documented prefixes (and therefore rejected) rather than silently
/// skipped.
fn is_diagnostic_id(s: &str) -> bool {
    if !s.bytes().next().is_some_and(|b| b.is_ascii_lowercase()) {
        return false;
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'/')
    {
        return false;
    }
    // Must contain at least one separator and have no empty segments.
    if !(s.contains('-') || s.contains('/')) {
        return false;
    }
    if s.starts_with(['-', '/']) || s.ends_with(['-', '/']) {
        return false;
    }
    !s.split(['/', '-']).any(str::is_empty)
}

/// Extracts double-quoted substrings from `text`, honoring `\"` / `\\` escapes so
/// an escaped quote cannot desync the literal/code parity.
///
/// NOTE: raw strings (`r#"…"#`) are not specially handled; diagnostic rule ids
/// should use plain string literals so they are visible to this scanner.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    let mut current = String::new();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            match c {
                '\\' => {
                    // Skip the escaped character; exact content is irrelevant here.
                    let _ = chars.next();
                }
                '"' => {
                    out.push(std::mem::take(&mut current));
                    in_string = false;
                }
                _ => current.push(c),
            }
        } else if c == '"' {
            in_string = true;
        }
    }
    out
}

/// Recursively collects files under `dir` with one of `extensions`, skipping
/// `target`, `.git`, and `corpus` directories.
fn collect_files(dir: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .with_context(|| format!("failed to read directory {}", current.display()))?;
        for entry in entries {
            let entry = entry
                .with_context(|| format!("failed to read an entry in {}", current.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to stat {}", path.display()))?;
            if file_type.is_dir() {
                if !is_skipped_dir(&path) {
                    stack.push(path);
                }
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
                && extensions.contains(&ext)
            {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Directories the scanners skip.
fn is_skipped_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("target" | ".git" | "corpus")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A minimal but complete one-feature matrix for tests.
    const SAMPLE: &str = r#"
matrix_version = 1
last_reviewed = "2026-06-06"

[[feature]]
id = "AV2-5.2.2-OBU-HEADER"
name = "OBU header syntax"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.2.2"]
sources = []
crate = "splot-core"
module = "crates/splot-core/src/obu.rs"
openspec_change = ""
tracking_issue = ""
owner = "core"
risk = "high"
notes = "ok"

[feature.status]
mapped = "done"
types = "done"
parse = "done"
validate = "partial"
write = "todo"
encode = "not-applicable"
decode_check = "done"
tests = "done"
avm_diff = "pending"
perf = "not-applicable"

[feature.proof]
tests = ["crates/splot-core/src/obu.rs::tests"]
commands = ["cargo test -p splot-core obu"]
fixtures = []
diagnostics = []
"#;

    fn intrinsic_problems(text: &str) -> Vec<String> {
        let matrix = parse_matrix(text).expect("sample parses");
        let mut checker = Checker::new(Path::new("."), &matrix);
        checker.intrinsic(&matrix);
        checker.problems
    }

    #[test]
    fn sample_matrix_is_clean() {
        assert!(intrinsic_problems(SAMPLE).is_empty());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let dup = format!("{SAMPLE}{}", SAMPLE.trim_start_matches(|c| c != '['));
        let problems = intrinsic_problems(&dup);
        assert!(problems.iter().any(|p| p.contains("duplicate feature id")));
    }

    #[test]
    fn invalid_status_is_rejected() {
        let bad = SAMPLE.replace(r#"parse = "done""#, r#"parse = "almost""#);
        let problems = intrinsic_problems(&bad);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("invalid status `almost`"))
        );
    }

    #[test]
    fn invalid_category_is_rejected() {
        let bad = SAMPLE.replace(r#"category = "normative""#, r#"category = "made-up""#);
        let problems = intrinsic_problems(&bad);
        assert!(problems.iter().any(|p| p.contains("unknown category")));
    }

    #[test]
    fn invalid_crate_is_rejected() {
        let bad = SAMPLE.replace(r#"crate = "splot-core""#, r#"crate = "splot-validte""#);
        let problems = intrinsic_problems(&bad);
        assert!(problems.iter().any(|p| p.contains("unknown crate")));
    }

    #[test]
    fn invalid_owner_is_rejected() {
        let bad = SAMPLE.replace(r#"owner = "core""#, r#"owner = "codec""#);
        let problems = intrinsic_problems(&bad);
        assert!(problems.iter().any(|p| p.contains("unknown owner")));
    }

    #[test]
    fn invalid_id_is_rejected() {
        let bad = SAMPLE.replace("AV2-5.2.2-OBU-HEADER", "av2-bad");
        let problems = intrinsic_problems(&bad);
        assert!(problems.iter().any(|p| p.contains("id regex")));
    }

    #[test]
    fn done_without_proof_is_rejected() {
        let bad = SAMPLE
            .replace(
                r#"tests = ["crates/splot-core/src/obu.rs::tests"]"#,
                "tests = []",
            )
            .replace(
                r#"commands = ["cargo test -p splot-core obu"]"#,
                "commands = []",
            );
        let problems = intrinsic_problems(&bad);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("[feature.proof] is empty"))
        );
    }

    #[test]
    fn id_regex_accepts_and_rejects() {
        assert!(is_valid_feature_id("AV2-5.2.2-OBU-HEADER"));
        assert!(is_valid_feature_id("AV2-B-ANNEXB-OBU-ENVELOPE"));
        assert!(is_valid_feature_id("ENC-Y4M-INPUT"));
        assert!(is_valid_feature_id("XTASK-FEATURE-STATUS"));
        assert!(!is_valid_feature_id("AV2"));
        assert!(!is_valid_feature_id("AV2-"));
        assert!(!is_valid_feature_id("av2-lower"));
        assert!(!is_valid_feature_id("4.11.6"));
    }

    #[test]
    fn token_extraction_ignores_prose_and_keeps_ids() {
        let text = "See AV2-5.2.2-OBU-HEADER. This is AV2-specific and AV2-permitted. \
                    Diagnostic AV2-5.2.2-OBU-HEADER.MISSING-BYTE fires.";
        let tokens = extract_candidate_tokens(text);
        assert!(tokens.contains(&"AV2-5.2.2-OBU-HEADER".to_owned()));
        assert!(tokens.contains(&"AV2-5.2.2-OBU-HEADER.MISSING-BYTE".to_owned()));
        // "AV2-specific" / "AV2-permitted" have lowercase tails and are not tokens.
        assert!(!tokens.iter().any(|t| t.contains("specific")));
        assert!(!tokens.iter().any(|t| t.contains("permitted")));
    }

    #[test]
    fn suffix_rule_accepts_diagnostic_sub_rules() {
        assert!(is_suffixed(
            "AV2-5.2.2-OBU-HEADER.MISSING-BYTE",
            "AV2-5.2.2-OBU-HEADER"
        ));
        // Built at runtime so this non-id example does not trip the token scanner.
        let extended = format!("{}-X", "AV2-5.2.2-OBU-HEADER");
        assert!(!is_suffixed(&extended, "AV2-5.2.2-OBU-HEADER"));
        assert!(!is_suffixed("AV2-5.2.2-OBU-HEADER", "AV2-5.2.2-OBU-HEADER"));
    }

    #[test]
    fn diagnostic_id_grammar() {
        assert!(is_diagnostic_id("obu-header/global-xlayer-required"));
        assert!(is_diagnostic_id("bitstream/parse-error"));
        // Slash-less kebab ids are recognized so they get prefix-checked.
        assert!(is_diagnostic_id("parse-error"));
        assert!(!is_diagnostic_id("6.2.2"));
        assert!(!is_diagnostic_id("a message with spaces"));
        assert!(!is_diagnostic_id("NoSlashHere"));
        assert!(!is_diagnostic_id("plain"));
        assert!(!is_diagnostic_id("-leading"));
    }

    #[test]
    fn string_literals_handle_escapes() {
        let src = r#"let a = "obu-header/ok"; let b = "say \"hi\" then frame/sneaky";"#;
        let lits = string_literals(src);
        assert!(lits.iter().any(|s| s == "obu-header/ok"));
        // The escaped quotes do not split the second literal into pieces of code.
        assert_eq!(lits.iter().filter(|s| s.contains("say")).count(), 1);
        // ... so a bad-prefix token embedded in prose is not a standalone literal.
        assert!(!lits.iter().any(|s| s == "frame/sneaky"));
    }

    #[test]
    fn markdown_render_has_expected_shape() {
        let matrix = parse_matrix(SAMPLE).unwrap();
        let all: Vec<&Feature> = matrix.feature.iter().collect();
        let rendered = render_markdown(&matrix, &all);
        // Guard the exact header (including the curated stage projection) so a
        // change to column order/headers/separator is caught here.
        assert!(
            rendered.contains(
                "| ID | Name | Category | Kind | Mapped | Types | Parse | Validate | \
                 Write | Encode | DecChk | Tests | AVM | Module |"
            ),
            "header row changed:\n{rendered}"
        );
        // Guard a feature row and a status value, then confirm determinism.
        assert!(
            rendered.contains("`AV2-5.2.2-OBU-HEADER`"),
            "feature row missing"
        );
        assert!(rendered.contains(" done "), "status value missing");
        assert_eq!(
            rendered,
            render_markdown(&matrix, &all),
            "render not deterministic"
        );
    }
}
