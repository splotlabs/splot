// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source-file size budget check.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::feature_status::display_path;
use crate::git_util::run_git;

const SOFT_LINE_LIMIT: usize = 1_000;
const HARD_LINE_LIMIT: usize = 2_500;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFileLineCount {
    path: String,
    lines: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SourceLineReport {
    soft_warnings: Vec<SourceFileLineCount>,
    hard_violations: Vec<SourceFileLineCount>,
}

/// Checks Rust source files against the source-line budget.
pub(crate) fn check_source_lines(root: &Path) -> Result<()> {
    let files = rust_source_files(root)?;
    let mut counts = Vec::with_capacity(files.len());
    for path in files {
        let displayed_path = normalized_display_path(root, &path);
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {displayed_path}"))?;
        counts.push(SourceFileLineCount {
            path: displayed_path,
            lines: physical_line_count(&contents),
        });
    }

    let report = evaluate_source_lines(&counts);
    emit_report(&report);

    if !report.hard_violations.is_empty() {
        bail!(
            "check-source-lines: {} hard violation(s)",
            report.hard_violations.len()
        );
    }

    eprintln!(
        "check-source-lines: ok ({} file(s), {} advisory warning(s))",
        counts.len(),
        report.soft_warnings.len()
    );
    Ok(())
}

fn rust_source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = run_git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ],
    )?;
    let mut files = output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| root.join(line))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(files)
}

pub(crate) fn physical_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        #[allow(clippy::naive_bytecount)]
        let newlines = text.as_bytes().iter().filter(|&&b| b == b'\n').count();
        newlines + usize::from(!text.ends_with('\n'))
    }
}

fn evaluate_source_lines(files: &[SourceFileLineCount]) -> SourceLineReport {
    let mut report = SourceLineReport::default();
    for file in files {
        let file = SourceFileLineCount {
            path: normalize_source_path(&file.path),
            lines: file.lines,
        };
        if file.lines > SOFT_LINE_LIMIT {
            report.soft_warnings.push(file.clone());
        }
        if file.lines > HARD_LINE_LIMIT {
            report.hard_violations.push(file);
        }
    }
    report.soft_warnings.sort_by(|a, b| a.path.cmp(&b.path));
    report.hard_violations.sort_by(|a, b| a.path.cmp(&b.path));
    report
}

fn emit_report(report: &SourceLineReport) {
    for warning in &report.soft_warnings {
        eprintln!(
            "source-line advisory: {} has {} line(s), above soft limit {SOFT_LINE_LIMIT}",
            warning.path, warning.lines
        );
    }
    for violation in &report.hard_violations {
        eprintln!(
            "source-line hard violation: {} has {} line(s), above hard cap {HARD_LINE_LIMIT}",
            violation.path, violation.lines
        );
    }
}

fn normalized_display_path(root: &Path, path: &Path) -> String {
    normalize_source_path(&display_path(root, path))
}

fn normalize_source_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, lines: usize) -> SourceFileLineCount {
        SourceFileLineCount {
            path: path.to_owned(),
            lines,
        }
    }

    #[test]
    fn soft_warning_does_not_create_hard_violation() {
        let report = evaluate_source_lines(&[file("src/lib.rs", SOFT_LINE_LIMIT + 1)]);
        assert_eq!(report.soft_warnings, vec![file("src/lib.rs", 1_001)]);
        assert!(report.hard_violations.is_empty());
    }

    #[test]
    fn hard_cap_fails() {
        let report = evaluate_source_lines(&[file("src/lib.rs", HARD_LINE_LIMIT + 1)]);
        assert_eq!(report.hard_violations, vec![file("src/lib.rs", 2_501)]);
    }

    #[test]
    fn limits_are_inclusive_and_paths_are_normalized() {
        let report = evaluate_source_lines(&[
            file("src/soft.rs", SOFT_LINE_LIMIT),
            file(r"src\hard.rs", HARD_LINE_LIMIT),
        ]);
        assert_eq!(
            report.soft_warnings,
            vec![file("src/hard.rs", HARD_LINE_LIMIT)]
        );
        assert!(report.hard_violations.is_empty());
    }

    #[test]
    fn physical_line_count_counts_final_unterminated_line() {
        assert_eq!(physical_line_count(""), 0);
        assert_eq!(physical_line_count("one"), 1);
        assert_eq!(physical_line_count("one\n"), 1);
        assert_eq!(physical_line_count("one\n\n"), 2);
    }
}
