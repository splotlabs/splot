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

/// One parsed diagnostic catalog entry. `severity` and `section` are stored
/// verbatim from the doc (so a dual `error/warning` and non-AV2 section labels like
/// `IVF` / `varies` survive); only `summary` and surrounding whitespace are trimmed.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    rule_id: String,
    /// The doc's `Severity` cell, e.g. `error` / `warning`; a dual cell is rendered
    /// with a comma (`error, warning`) rather than a slash so the value cannot be
    /// mistaken for a `ns/id` rule id by the registry scanners.
    severity: String,
    /// The doc's `Section` cell verbatim (e.g. `§ 6.2.2`, `§ A.4`, `IVF`,
    /// `varies`). `None` when the cell is empty or a dash.
    section: Option<String>,
    summary: String,
}

/// `true` when `token` (trimmed) is one of the doc's severity words.
fn is_severity_token(token: &str) -> bool {
    matches!(token.trim(), "error" | "warning" | "info")
}

/// `true` when a severity cell is a non-empty `/`-separated list of severity words
/// (e.g. `error`, `warning`, or a dual `error/warning`).
fn is_severity_cell(cell: &str) -> bool {
    !cell.trim().is_empty() && cell.split('/').all(is_severity_token)
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
        // Split a `| id | severity | section | condition |` row into columns. Drop the
        // leading empty (the row starts with `|`) and an optional trailing empty (a
        // closing `|`), so rows with OR without a trailing pipe — both valid GitHub
        // Markdown — parse to the same columns. A diagnostics row then has >= 4
        // columns; the 3-column `*/syntax` table is shorter and stays excluded.
        let mut cells: Vec<&str> = trimmed.split('|').collect();
        cells.remove(0); // the empty before the opening `|` (row is `starts_with('|')`)
        if cells.last().is_some_and(|cell| cell.trim().is_empty()) {
            cells.pop(); // the empty after a closing `|`, when present
        }
        if cells.len() < 4 {
            continue; // separator, prose, or the 3-column `*/syntax` table
        }
        let Some(rule_id) = cells[0]
            .trim()
            .strip_prefix('`')
            .and_then(|s| s.strip_suffix('`'))
        else {
            continue; // header row / separator / prose, not a diagnostics row
        };
        if !is_rule_id(rule_id) {
            continue; // a backtick token that is not a rule id
        }
        // From here the row IS a diagnostics row (a valid backtick rule id in the
        // first column), so its severity MUST be valid — a silent skip would drop a
        // real diagnostic (e.g. a dual `error/warning` cell).
        let severity_cell = cells[1].trim();
        if !is_severity_cell(severity_cell) {
            bail!(
                "gen-explain: rule `{rule_id}` in {DOC_REL} has an unparsable severity `{severity_cell}`"
            );
        }
        // Render a dual `error/warning` cell with a comma so the stored value is not
        // a single-slash token a rule-id scanner would flag.
        let severity = severity_cell
            .split('/')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(", ");
        let section = parse_section(cells[2].trim());
        // Rejoin any `|` that appeared inside the condition cell (defensive: the doc
        // has none today) so an embedded pipe never silently drops the row.
        let summary = cells[3..].join("|").trim().to_owned();
        if summary.is_empty() {
            bail!("gen-explain: empty condition for `{rule_id}` in {DOC_REL}");
        }
        entries.push(Entry {
            rule_id: rule_id.to_owned(),
            severity,
            section,
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

/// Returns the `Section` cell verbatim (trimmed), or `None` for an empty / dash
/// cell. Kept verbatim — including the leading `§` for AV2 sections and non-AV2
/// labels like `IVF` / `varies` — so `explain` never mis-labels a section.
fn parse_section(cell: &str) -> Option<String> {
    let section = cell.trim();
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
        let section = match &entry.section {
            Some(section) => format!("Some({section:?})"),
            None => "None".to_owned(),
        };
        out.push_str(&format!(
            "    super::DiagnosticInfo {{ rule_id: {:?}, severity: {:?}, section: {}, summary: {:?} }},\n",
            entry.rule_id, entry.severity, section, entry.summary,
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
| `ops/dual` | error/warning | § 6.1 | emitted as either severity |
| `ops/piped` | info | § 6.2 | left | right pipe in condition |
| `ops/notrail` | warning | § 6.3 | a valid row without a trailing pipe

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
        assert_eq!(
            ids,
            ["brt/bar", "ops/dual", "ops/foo", "ops/notrail", "ops/piped"]
        );
    }

    #[test]
    fn row_without_a_trailing_pipe_is_parsed() {
        // GitHub Markdown allows a table row to omit the closing `|`; such a row must
        // still be captured (and the 3-col `*/syntax` table still excluded by count).
        let entries = parse_entries(DOC).unwrap();
        let notrail = entries.iter().find(|e| e.rule_id == "ops/notrail").unwrap();
        assert_eq!(notrail.severity, "warning");
        assert_eq!(notrail.section.as_deref(), Some("§ 6.3"));
        assert_eq!(notrail.summary, "a valid row without a trailing pipe");
    }

    #[test]
    fn dual_severity_preserved_as_comma_list() {
        let entries = parse_entries(DOC).unwrap();
        let dual = entries.iter().find(|e| e.rule_id == "ops/dual").unwrap();
        // Both severities survive; the slash is normalized to a comma.
        assert_eq!(dual.severity, "error, warning");
    }

    #[test]
    fn pipe_inside_a_condition_is_rejoined_not_dropped() {
        let entries = parse_entries(DOC).unwrap();
        let piped = entries.iter().find(|e| e.rule_id == "ops/piped").unwrap();
        assert_eq!(piped.summary, "left | right pipe in condition");
    }

    #[test]
    fn valid_id_row_with_unparsable_severity_bails() {
        let bad = "\
<!-- diagnostics-registry:begin -->
| `ops/foo` | notasev | § 6.1 | x |
<!-- diagnostics-registry:end -->
";
        assert!(parse_entries(bad).is_err());
    }

    #[test]
    fn extracts_severity_section_and_summary() {
        let entries = parse_entries(DOC).unwrap();
        let foo = entries.iter().find(|e| e.rule_id == "ops/foo").unwrap();
        assert_eq!(foo.severity, "error");
        // The section is kept verbatim, including the leading `§`.
        assert_eq!(foo.section.as_deref(), Some("§ 6.10.2"));
        assert_eq!(foo.summary, "a thing is wrong with \"quoted\" text");
        let bar = entries.iter().find(|e| e.rule_id == "brt/bar").unwrap();
        assert_eq!(bar.severity, "warning");
        assert_eq!(bar.section, None);
    }

    #[test]
    fn generated_output_escapes_and_is_stable() {
        let entries = parse_entries(DOC).unwrap();
        let generated = render_generated(&entries);
        // Quotes in the summary are escaped into a valid Rust literal.
        assert!(generated.contains(r#"summary: "a thing is wrong with \"quoted\" text""#));
        assert!(generated.contains(r#"severity: "error""#));
        assert!(generated.contains("section: None"));
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
