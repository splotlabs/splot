// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Manual documentation budget gate (`cargo xtask check-doc-budget`).

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::feature_status::display_path;
use crate::git_util::run_git;

const BUDGET_PATH: &str = "tools/docs/budget.toml";

#[derive(Debug, Deserialize)]
struct Budget {
    max_manual_markdown_files: usize,
    max_manual_markdown_lines: usize,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    allowed_manual_docs: Vec<String>,
    #[serde(default)]
    banned_paths: Vec<String>,
    #[serde(default)]
    generated_status_docs: Vec<String>,
}

#[derive(Debug, Clone)]
struct MarkdownEntry {
    path: String,
    lines: usize,
}

#[derive(Debug)]
struct BudgetReport {
    manual_files: usize,
    manual_lines: usize,
    largest: Vec<MarkdownEntry>,
    problems: Vec<String>,
}

/// Verifies committed manual markdown stays within the documentation budget.
pub(crate) fn check_doc_budget(root: &Path) -> Result<()> {
    let budget_path = root.join(BUDGET_PATH);
    let budget_text = std::fs::read_to_string(&budget_path)
        .with_context(|| format!("failed to read {}", budget_path.display()))?;
    let budget: Budget = toml::from_str(&budget_text)
        .with_context(|| format!("failed to parse {}", budget_path.display()))?;
    let entries = markdown_entries(root)?;
    let report = evaluate_doc_budget(&budget, &entries);

    if report.problems.is_empty() {
        eprintln!(
            "check-doc-budget: ok ({} manual markdown file(s), {} line(s))",
            report.manual_files, report.manual_lines
        );
        return Ok(());
    }

    for problem in &report.problems {
        eprintln!("error: {problem}");
    }
    if !report.largest.is_empty() {
        eprintln!("largest counted manual docs:");
        for entry in &report.largest {
            eprintln!("{:6} {}", entry.lines, entry.path);
        }
    }
    bail!("check-doc-budget: documentation budget failed")
}

fn markdown_entries(root: &Path) -> Result<Vec<MarkdownEntry>> {
    let output = run_git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.md",
        ],
    )?;
    let mut entries = Vec::new();
    for rel in output.lines() {
        let path = root.join(rel);
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", display_path(root, &path)))?;
        entries.push(MarkdownEntry {
            path: rel.to_owned(),
            lines: line_count(&text),
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    Ok(entries)
}

fn evaluate_doc_budget(budget: &Budget, entries: &[MarkdownEntry]) -> BudgetReport {
    let mut manual = Vec::new();
    let mut problems = Vec::new();

    for entry in entries {
        if matches_any(&budget.generated_status_docs, &entry.path) {
            problems.push(format!(
                "{} is generated status markdown; generate it on demand instead",
                entry.path
            ));
        }
        if matches_any(&budget.banned_paths, &entry.path) {
            problems.push(format!(
                "{} matches a banned documentation path",
                entry.path
            ));
        }
        if matches_any(&budget.exclude, &entry.path) {
            continue;
        }
        manual.push(entry.clone());
        if !budget.allowed_manual_docs.is_empty()
            && !matches_any(&budget.allowed_manual_docs, &entry.path)
        {
            problems.push(format!(
                "{} is counted manual markdown but is not in allowed_manual_docs",
                entry.path
            ));
        }
    }

    let manual_files = manual.len();
    let manual_lines = manual.iter().map(|entry| entry.lines).sum::<usize>();
    if manual_files > budget.max_manual_markdown_files {
        problems.push(format!(
            "{manual_files} manual markdown files exceed the budget of {}",
            budget.max_manual_markdown_files
        ));
    }
    if manual_lines > budget.max_manual_markdown_lines {
        problems.push(format!(
            "{manual_lines} manual markdown lines exceed the budget of {}",
            budget.max_manual_markdown_lines
        ));
    }

    manual.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
    manual.truncate(10);
    BudgetReport {
        manual_files,
        manual_lines,
        largest: manual,
        problems,
    }
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_pattern(pattern, path))
}

fn matches_pattern(pattern: &str, path: &str) -> bool {
    let pattern = pattern.replace('\\', "/").to_ascii_lowercase();
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let basename = path.rsplit('/').next().unwrap_or(path.as_str());
    if let Some(prefix) = pattern.strip_suffix("/**") {
        let prefix = prefix.trim_end_matches('/');
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    if let Some(prefix) = pattern.strip_suffix('*')
        && !prefix.contains('*')
    {
        return path.starts_with(prefix) || basename.starts_with(prefix);
    }
    if let Some((prefix, rest)) = pattern.split_once("**/*")
        && let Some(marker) = rest.strip_suffix("*.md")
    {
        return path.starts_with(prefix) && basename.contains(marker);
    }
    if let Some((prefix, suffix)) = pattern.split_once('*')
        && !suffix.contains('*')
    {
        return (path.starts_with(prefix) && path.ends_with(suffix))
            || (basename.starts_with(prefix) && basename.ends_with(suffix));
    }
    path == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget {
            max_manual_markdown_files: 2,
            max_manual_markdown_lines: 8,
            exclude: vec!["docs/spec/av2/1.0.0/**".to_owned(), "LICENSE*".to_owned()],
            allowed_manual_docs: vec!["README.md".to_owned(), "docs/README.md".to_owned()],
            banned_paths: vec![
                ".codex/skills/**".to_owned(),
                "docs/archive/**".to_owned(),
                "openspec/changes/archive/**".to_owned(),
                "openspec/**".to_owned(),
                "docs/**/*roadmap*.md".to_owned(),
            ],
            generated_status_docs: vec!["docs/feature-status.md".to_owned()],
        }
    }

    fn entry(path: &str, lines: usize) -> MarkdownEntry {
        MarkdownEntry {
            path: path.to_owned(),
            lines,
        }
    }

    #[test]
    fn excludes_spec_legal_and_openspec_paths() {
        let budget = budget();
        for path in ["docs/spec/av2/1.0.0/index.md", "LICENSE.md"] {
            assert!(matches_any(&budget.exclude, path));
        }
        assert!(!matches_any(
            &budget.exclude,
            "openspec/specs/process/spec.md"
        ));
        assert!(matches_any(
            &budget.banned_paths,
            "openspec/specs/process/spec.md"
        ));
    }

    #[test]
    fn rejects_unallowed_and_over_budget_manual_docs() {
        let report = evaluate_doc_budget(
            &budget(),
            &[
                entry("README.md", 4),
                entry("docs/README.md", 4),
                entry("docs/EXTRA.md", 1),
            ],
        );
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.contains("not in allowed_manual_docs"))
        );
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.contains("manual markdown files exceed"))
        );
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.contains("manual markdown lines exceed"))
        );
    }

    #[test]
    fn rejects_generated_status_and_banned_paths_even_when_excluded() {
        let report = evaluate_doc_budget(
            &budget(),
            &[
                entry("docs/FEATURE-STATUS.md", 4),
                entry("docs/ROADMAP.md", 4),
                entry("docs/archive/old.md", 4),
                entry(".codex/skills/splot-doc-audit/SKILL.md", 4),
                entry("openspec/changes/archive/old/proposal.md", 4),
            ],
        );
        assert!(report.problems.len() >= 5, "{:?}", report.problems);
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.contains("generated status markdown"))
        );
        assert!(
            report
                .problems
                .iter()
                .filter(|problem| problem.contains("banned documentation path"))
                .count()
                >= 4
        );
    }

    #[test]
    fn pattern_matching_handles_targeted_globs() {
        assert!(matches_pattern(
            "docs/spec/av2/1.0.0/**",
            "docs/spec/av2/1.0.0/index.md"
        ));
        assert!(matches_pattern("LICENSE*", "LICENSE.md"));
        assert!(matches_pattern("docs/**/*roadmap*.md", "docs/ROADMAP.md"));
        assert!(matches_pattern("docs/archive/**", "docs/archive/a.md"));
        assert!(matches_pattern(
            "openspec/changes/archive/**",
            "openspec/changes/archive/old/proposal.md"
        ));
        assert!(matches_pattern(
            ".codex/skills/**",
            ".codex/skills/splot-doc-audit/SKILL.md"
        ));
        assert!(!matches_pattern("docs/archive/**", "docs/ARCHITECTURE.md"));
    }
}
