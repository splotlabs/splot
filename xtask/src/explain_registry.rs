// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `cargo xtask gen-explain [--check]`: generate the `splot explain` diagnostic
//! registry from `docs/VALIDATOR-DIAGNOSTICS.md`.
//!
//! The doc is the CI-enforced single source of truth for emitted validator rule ids
//! (see `check-diagnostic-registry`). This generator parses its 4-column emitted-
//! diagnostics tables (rule id, severity, `§ section`, condition) inside
//! the registry markers and emits
//! `crates/splot-validate/src/explain/generated.rs`, a sorted `DiagnosticInfo`
//! table the `explain` command reads. Every field is taken **directly** from the
//! doc; nothing is hand-transcribed or invented. The separate 3-column
//! `*/syntax` "Check registry identifiers" table is excluded (those ids are not
//! user-visible diagnostics — they route through `bitstream/parse-error`).
//!
//! `--check` regenerates into memory and diffs against the committed file, failing
//! on drift; it is wired into `cargo xtask ci`, so the registry can never silently
//! diverge from the doc.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// Source doc (the single source of truth) relative to the workspace root.
const DOC_REL: &str = "docs/VALIDATOR-DIAGNOSTICS.md";
/// Generated registry file relative to the workspace root.
const GENERATED_REL: &str = "crates/splot-validate/src/explain/generated.rs";
const BEGIN_MARKER: &str = "<!-- diagnostics-registry:begin -->";
const END_MARKER: &str = "<!-- diagnostics-registry:end -->";

/// One parsed diagnostic catalog entry.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    rule_id: String,
    severity: Severity,
    /// The spec section without its leading `§`, e.g. `A.4` / `6.2.2`. `None` when
    /// the doc cell is empty or a dash.
    spec_section: Option<String>,
    summary: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    fn rust_path(self) -> &'static str {
        match self {
            Severity::Error => "crate::Severity::Error",
            Severity::Warning => "crate::Severity::Warning",
            Severity::Info => "crate::Severity::Info",
        }
    }

    fn parse(s: &str) -> Option<Severity> {
        match s {
            "error" => Some(Severity::Error),
            "warning" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            _ => None,
        }
    }
}

/// Entry point for `cargo xtask gen-explain [--check]`.
pub(crate) fn run_gen_explain(root: &Path, check: bool) -> Result<()> {
    let doc_path = root.join(DOC_REL);
    let doc =
        std::fs::read_to_string(&doc_path).with_context(|| format!("failed to read {DOC_REL}"))?;
    let entries = parse_entries(&doc)?;
    let generated = render_generated(&entries);

    let generated_path = root.join(GENERATED_REL);
    if check {
        let committed = std::fs::read_to_string(&generated_path)
            .with_context(|| format!("failed to read {GENERATED_REL}"))?;
        if committed != generated {
            bail!(
                "gen-explain --check: {GENERATED_REL} is out of date ({} entries); run `cargo xtask gen-explain`",
                entries.len()
            );
        }
        eprintln!("gen-explain --check: ok ({} entries)", entries.len());
    } else {
        std::fs::write(&generated_path, &generated)
            .with_context(|| format!("failed to write {GENERATED_REL}"))?;
        eprintln!(
            "gen-explain: wrote {} entries to {GENERATED_REL}",
            entries.len()
        );
    }
    Ok(())
}

/// Parses the 4-column emitted-diagnostics tables inside the registry markers.
fn parse_entries(doc: &str) -> Result<Vec<Entry>> {
    let region = registry_region(doc)?;
    let mut entries: Vec<Entry> = Vec::new();
    for line in region.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        // Split a `| a | b | c | d |` row into its 4 cells (drop the outer empties).
        let cells: Vec<&str> = trimmed.split('|').collect();
        // 4 columns => 6 split parts (leading + 4 + trailing).
        if cells.len() != 6 {
            continue;
        }
        let id_cell = cells[1].trim();
        let Some(rule_id) = id_cell.strip_prefix('`').and_then(|s| s.strip_suffix('`')) else {
            continue; // header row / separator / prose
        };
        let Some(severity) = Severity::parse(cells[2].trim()) else {
            // A row whose 2nd column is not a severity is not a 4-col diagnostics
            // row (e.g. the separator `|---|` or the 3-col syntax table).
            continue;
        };
        if !is_rule_id(rule_id) {
            bail!("gen-explain: malformed rule id `{rule_id}` in {DOC_REL}");
        }
        let spec_section = parse_section(cells[3].trim());
        let summary = cells[4].trim().to_owned();
        if summary.is_empty() {
            bail!("gen-explain: empty condition for `{rule_id}` in {DOC_REL}");
        }
        entries.push(Entry {
            rule_id: rule_id.to_owned(),
            severity,
            spec_section,
            summary,
        });
    }

    if entries.is_empty() {
        bail!("gen-explain: parsed no diagnostics from {DOC_REL}");
    }
    entries.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    for pair in entries.windows(2) {
        if pair[0].rule_id == pair[1].rule_id {
            bail!(
                "gen-explain: duplicate rule id `{}` in {DOC_REL}",
                pair[0].rule_id
            );
        }
    }
    Ok(entries)
}

/// Normalizes a `Section` cell to the bare section (no leading `§`), or `None` for
/// an empty / dash cell.
fn parse_section(cell: &str) -> Option<String> {
    let section = cell.trim_start_matches('§').trim();
    if section.is_empty() || section == "—" || section == "-" {
        None
    } else {
        Some(section.to_owned())
    }
}

/// `true` for a `<ns>/<id>` rule id: lowercase ascii / digits / `-`, exactly one
/// `/`, non-empty segments.
fn is_rule_id(s: &str) -> bool {
    let mut segments = s.split('/');
    let (Some(ns), Some(id), None) = (segments.next(), segments.next(), segments.next()) else {
        return false;
    };
    [ns, id].into_iter().all(|seg| {
        !seg.is_empty()
            && seg
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// Returns the slice between the registry markers (exactly one of each required).
fn registry_region(text: &str) -> Result<&str> {
    if text.matches(BEGIN_MARKER).count() != 1 || text.matches(END_MARKER).count() != 1 {
        bail!("gen-explain: expected exactly one begin and one end registry marker in {DOC_REL}");
    }
    let begin = text.find(BEGIN_MARKER).context("missing begin marker")? + BEGIN_MARKER.len();
    let end_rel = text[begin..]
        .find(END_MARKER)
        .context("end marker precedes begin marker")?;
    Ok(&text[begin..begin + end_rel])
}

/// Renders the generated Rust file. `#[rustfmt::skip]` on the table plus
/// fully-qualified type paths (no `use` items) make the output rustfmt-stable, so
/// `--check` can diff it byte-for-byte.
fn render_generated(entries: &[Entry]) -> String {
    let mut out = String::new();
    out.push_str("// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0\n");
    out.push_str("// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>\n\n");
    out.push_str("//! @generated by `cargo xtask gen-explain` from docs/VALIDATOR-DIAGNOSTICS.md — DO NOT EDIT.\n");
    out.push_str("//! Regenerate with `cargo xtask gen-explain`; the drift check runs in `cargo xtask ci`.\n\n");
    out.push_str(
        "/// Every emitted validator diagnostic, sorted by rule id (for binary search).\n",
    );
    out.push_str("#[rustfmt::skip]\n");
    out.push_str("pub(super) const REGISTRY: &[super::DiagnosticInfo] = &[\n");
    for entry in entries {
        let section = match &entry.spec_section {
            Some(section) => format!("Some({section:?})"),
            None => "None".to_owned(),
        };
        out.push_str(&format!(
            "    super::DiagnosticInfo {{ rule_id: {:?}, severity: {}, spec_section: {}, summary: {:?} }},\n",
            entry.rule_id,
            entry.severity.rust_path(),
            section,
            entry.summary,
        ));
    }
    out.push_str("];\n");
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const DOC: &str = "\
intro `ns/before`
<!-- diagnostics-registry:begin -->
### `ops/`
| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ops/foo` | error | § 6.10.2 | a thing is wrong with \"quoted\" text |
| `brt/bar` | warning |  | no section here |

### check registry identifiers
| Registry ID | Parse § | Routed through |
|---|---|---|
| `atlas/syntax` | § 6.9 | bitstream/parse-error |
<!-- diagnostics-registry:end -->
outro `ns/after`
";

    #[test]
    fn parses_four_column_rows_only() {
        let entries = parse_entries(DOC).unwrap();
        let ids: Vec<&str> = entries.iter().map(|e| e.rule_id.as_str()).collect();
        // Sorted; the 3-col syntax row and prose ids are excluded.
        assert_eq!(ids, ["brt/bar", "ops/foo"]);
    }

    #[test]
    fn extracts_severity_section_and_summary() {
        let entries = parse_entries(DOC).unwrap();
        let foo = entries.iter().find(|e| e.rule_id == "ops/foo").unwrap();
        assert_eq!(foo.severity, Severity::Error);
        assert_eq!(foo.spec_section.as_deref(), Some("6.10.2"));
        assert_eq!(foo.summary, "a thing is wrong with \"quoted\" text");
        let bar = entries.iter().find(|e| e.rule_id == "brt/bar").unwrap();
        assert_eq!(bar.severity, Severity::Warning);
        assert_eq!(bar.spec_section, None);
    }

    #[test]
    fn generated_output_escapes_and_is_stable() {
        let entries = parse_entries(DOC).unwrap();
        let generated = render_generated(&entries);
        // Quotes in the summary are escaped into a valid Rust literal.
        assert!(generated.contains(r#"summary: "a thing is wrong with \"quoted\" text""#));
        assert!(generated.contains("spec_section: None"));
        assert!(generated.contains("#[rustfmt::skip]"));
        // Deterministic: rendering twice yields the same bytes.
        assert_eq!(generated, render_generated(&parse_entries(DOC).unwrap()));
    }

    #[test]
    fn rule_id_grammar() {
        assert!(is_rule_id("ops/foo"));
        assert!(is_rule_id("annex-a/frame-size-below-minimum"));
        assert!(!is_rule_id("parse-error"));
        assert!(!is_rule_id("a/b/c"));
        assert!(!is_rule_id("Ops/Foo"));
        assert!(!is_rule_id("ops//foo"));
    }
}
