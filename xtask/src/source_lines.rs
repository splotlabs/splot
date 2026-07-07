// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source-file size budget check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::feature_status::display_path;
use crate::git_util::run_git;

const SOFT_LINE_LIMIT: usize = 1_000;
const HARD_LINE_LIMIT: usize = 2_500;

#[derive(Debug, Clone, Copy)]
struct SourceLineAllowance {
    path: &'static str,
    max_lines: usize,
    reason: &'static str,
}

const HARD_LINE_ALLOWANCES: &[SourceLineAllowance] = &[
    SourceLineAllowance {
        path: "crates/splot-decode/src/pipeline/general_intra.rs",
        max_lines: 3_119,
        reason: "temporary local decoder mission general-intra runtime frontier before module split",
    },
    SourceLineAllowance {
        path: "crates/splot-decode/src/pipeline/reconstruct.rs",
        max_lines: 3_500,
        reason: "temporary local decoder mission intra-reconstruction frontier before module split",
    },
    SourceLineAllowance {
        path: "crates/splot-decode/src/residual/pipeline.rs",
        max_lines: 2_520,
        reason: "temporary local decoder mission residual runtime frontier before module split",
    },
    SourceLineAllowance {
        path: "crates/splot-decode/src/bitstream/tile_payload/partition_traversal.rs",
        max_lines: 3_000,
        reason: "temporary local decoder mission partition-traversal frontier before module split",
    },
    SourceLineAllowance {
        path: "crates/splot-decode/src/prediction/inter/block.rs",
        max_lines: 2_760,
        reason: "temporary local decoder mission unified per-block decode engine before module split",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFileLineCount {
    path: String,
    lines: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SourceLineReport {
    soft_warnings: Vec<SourceFileLineCount>,
    hard_violations: Vec<SourceFileLineCount>,
    allowance_problems: Vec<String>,
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

    let report = evaluate_source_lines(&counts, HARD_LINE_ALLOWANCES);
    emit_report(&report);

    if !report.hard_violations.is_empty() || !report.allowance_problems.is_empty() {
        bail!(
            "check-source-lines: {} hard violation(s), {} allowance problem(s)",
            report.hard_violations.len(),
            report.allowance_problems.len()
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

fn evaluate_source_lines(
    files: &[SourceFileLineCount],
    allowances: &[SourceLineAllowance],
) -> SourceLineReport {
    let mut report = SourceLineReport::default();
    let files = files
        .iter()
        .map(|file| SourceFileLineCount {
            path: normalize_source_path(&file.path),
            lines: file.lines,
        })
        .collect::<Vec<_>>();
    let line_counts = files
        .iter()
        .map(|file| (file.path.as_str(), file.lines))
        .collect::<BTreeMap<_, _>>();
    let mut allowance_paths = BTreeSet::new();
    let mut duplicate_allowances = BTreeSet::new();

    for allowance in allowances {
        if !allowance_paths.insert(allowance.path) {
            duplicate_allowances.insert(allowance.path);
        }
        match line_counts.get(allowance.path).copied() {
            Some(lines) if lines > allowance.max_lines => {
                report.hard_violations.push(SourceFileLineCount {
                    path: allowance.path.to_owned(),
                    lines,
                });
            }
            Some(lines) if lines <= HARD_LINE_LIMIT => {
                report.allowance_problems.push(format!(
                    "{} is allowlisted but now has {lines} line(s), at or below the hard cap {HARD_LINE_LIMIT}",
                    allowance.path
                ));
            }
            Some(_) => {}
            None => report.allowance_problems.push(format!(
                "{} is allowlisted but is not a tracked Rust source file",
                allowance.path
            )),
        }
    }

    for path in duplicate_allowances {
        report
            .allowance_problems
            .push(format!("{path} has duplicate source-line allowances"));
    }

    for file in &files {
        if file.lines > SOFT_LINE_LIMIT {
            report.soft_warnings.push(file.clone());
        }
        if file.lines > HARD_LINE_LIMIT && !allowance_paths.contains(file.path.as_str()) {
            report.hard_violations.push(file.clone());
        }
    }

    report.soft_warnings.sort_by(|a, b| a.path.cmp(&b.path));
    report.hard_violations.sort_by(|a, b| a.path.cmp(&b.path));
    report.allowance_problems.sort();
    report
}

fn emit_report(report: &SourceLineReport) {
    for warning in &report.soft_warnings {
        if let Some(allowance) = HARD_LINE_ALLOWANCES
            .iter()
            .find(|allowance| allowance.path == warning.path)
        {
            eprintln!(
                "source-line advisory: {} has {} line(s), above soft limit {SOFT_LINE_LIMIT}; hard-cap allowance up to {} line(s): {}",
                warning.path, warning.lines, allowance.max_lines, allowance.reason
            );
        } else {
            eprintln!(
                "source-line advisory: {} has {} line(s), above soft limit {SOFT_LINE_LIMIT}",
                warning.path, warning.lines
            );
        }
    }
    for violation in &report.hard_violations {
        eprintln!(
            "source-line hard violation: {} has {} line(s), above {}",
            violation.path,
            violation.lines,
            hard_violation_limit_label(&violation.path, HARD_LINE_ALLOWANCES)
        );
    }
    for problem in &report.allowance_problems {
        eprintln!("source-line allowance problem: {problem}");
    }
}

fn hard_violation_limit_label(path: &str, allowances: &[SourceLineAllowance]) -> String {
    let path = normalize_source_path(path);
    if let Some(allowance) = allowances
        .iter()
        .find(|allowance| allowance.path == path.as_str())
    {
        format!("allowance cap {}", allowance.max_lines)
    } else {
        format!("hard cap {HARD_LINE_LIMIT}")
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
        let report = evaluate_source_lines(&[file("src/lib.rs", SOFT_LINE_LIMIT + 1)], &[]);
        assert_eq!(report.soft_warnings, vec![file("src/lib.rs", 1_001)]);
        assert!(report.hard_violations.is_empty());
        assert!(report.allowance_problems.is_empty());
    }

    #[test]
    fn hard_cap_fails_without_allowance() {
        let report = evaluate_source_lines(&[file("src/lib.rs", HARD_LINE_LIMIT + 1)], &[]);
        assert_eq!(report.hard_violations, vec![file("src/lib.rs", 2_501)]);
        assert!(report.allowance_problems.is_empty());
    }

    #[test]
    fn allowance_tolerates_existing_file_but_caps_growth() {
        let allowances = &[SourceLineAllowance {
            path: "src/large.rs",
            max_lines: 3_000,
            reason: "fixture",
        }];
        let tolerated = evaluate_source_lines(&[file("src/large.rs", 3_000)], allowances);
        assert!(tolerated.hard_violations.is_empty());
        assert!(tolerated.allowance_problems.is_empty());

        let grown = evaluate_source_lines(&[file("src/large.rs", 3_001)], allowances);
        assert_eq!(grown.hard_violations, vec![file("src/large.rs", 3_001)]);
    }

    #[test]
    fn allowance_lookup_normalizes_backslash_paths() {
        let allowance = SourceLineAllowance {
            path: "crates/foo/src/large.rs",
            max_lines: 2_700,
            reason: "fixture",
        };
        let allowances = [allowance];
        let windows_path = allowance.path.replace('/', "\\");
        let report =
            evaluate_source_lines(&[file(&windows_path, allowance.max_lines)], &allowances);

        assert!(report.hard_violations.is_empty());
        assert!(report.allowance_problems.is_empty());
        assert_eq!(
            report.soft_warnings,
            vec![file(allowance.path, allowance.max_lines)]
        );
        assert_eq!(
            hard_violation_limit_label(&windows_path, &allowances),
            format!("allowance cap {}", allowance.max_lines)
        );
    }

    #[test]
    fn allowance_hygiene_flags_missing_duplicate_and_obsolete_entries() {
        let allowances = &[
            SourceLineAllowance {
                path: "src/missing.rs",
                max_lines: 3_000,
                reason: "missing",
            },
            SourceLineAllowance {
                path: "src/small.rs",
                max_lines: 3_000,
                reason: "obsolete",
            },
            SourceLineAllowance {
                path: "src/small.rs",
                max_lines: 3_000,
                reason: "duplicate",
            },
        ];
        let report = evaluate_source_lines(&[file("src/small.rs", HARD_LINE_LIMIT)], allowances);
        assert!(report.hard_violations.is_empty());
        assert_eq!(
            report.allowance_problems,
            vec![
                "src/missing.rs is allowlisted but is not a tracked Rust source file",
                "src/small.rs has duplicate source-line allowances",
                "src/small.rs is allowlisted but now has 2500 line(s), at or below the hard cap 2500",
                "src/small.rs is allowlisted but now has 2500 line(s), at or below the hard cap 2500",
            ]
        );
    }

    #[test]
    fn hard_violation_limit_label_names_allowance_caps() {
        let allowance = SourceLineAllowance {
            path: "crates/foo/src/large.rs",
            max_lines: 2_700,
            reason: "fixture",
        };
        let allowances = [allowance];
        assert_eq!(
            hard_violation_limit_label(allowance.path, &allowances),
            format!("allowance cap {}", allowance.max_lines)
        );
        assert_eq!(
            hard_violation_limit_label("src/lib.rs", &allowances),
            "hard cap 2500".to_owned()
        );
    }

    #[test]
    fn physical_line_count_counts_final_unterminated_line() {
        assert_eq!(physical_line_count(""), 0);
        assert_eq!(physical_line_count("one"), 1);
        assert_eq!(physical_line_count("one\n"), 1);
        assert_eq!(physical_line_count("one\n\n"), 2);
    }
}
