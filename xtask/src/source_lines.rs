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
        path: "crates/splot-core/src/headers/sequence.rs",
        max_lines: 3328,
        reason: "large cohesive sequence-header parser (incl. the ProfileIdc Table A.1 enum + canonical-identity trait impls, and the §5.4.1 MLayerPresenceMap reflexive-transitive closure derivation + its closure test); split separately from this validator refactor",
    },
    SourceLineAllowance {
        path: "crates/splot-core/src/headers/frame/info.rs",
        max_lines: 5_340,
        reason: "large cohesive frame-header parser; split separately from this validator refactor. +16 (2026-06-15): the maintainer-approved frame-config discarded-bit model extension adds FrameHeaderCore.force_integer_mv / .intrabc so the byte-exact §5.18.3 frame-header writer can reproduce every read-and-discarded bit (frame-header-writer-size-config). +16 (2026-06-15): expose CoreSeqView / MfhFrameView / CoreSeqInterView (pub, crate-private fields) + their from_sequence/from_record constructors and pub(crate) init_core_from_prefix/parse_core_body so the composing intra frame-header writer (write_frame_header_core, frame-header-writer-compose) can take them as inputs and round-trip in its sibling tests. +409 (2026-06-16): the single-picture IsBridge parse fix adds parse_single_picture_bridge_tail (the spec-mirror prefix, the overwrite-gated refresh per §6.17.2+AVM, the decidable §5.18.10.1 film-grain tail, and the BruInactiveOrBridgeReturn stop) and its five positive/data-dependent/EOF tests, replacing the buggy intra-key-path test (frame-header-single-picture-bridge-fix). The CoreSeqInterView/CoreSeqView encoder writer-input constructors that briefly lived here were moved to frame/encoder_input.rs (encoder-input-submodule), so this file is back to the parser-only size. +198 (2026-06-21): the inter shared-tail completion (decode-inter-header-shared-tail) adds FrameHeaderParseStatus::InterHeaderComplete + its doc, the FrameHeaderCore.inter_tail field, the enable_bawp/enable_global_motion CoreSeqInterView fields + enable_df_sub_pu CoreSeqFilterView field with their from_sequence wiring, finish_inter_control_with_tail (the ReachedSharedTail continuation that lifts the reference-grounded frame size before parse_inter_shared_tail and converts a tail EOF to StoppedInsideInterControl), and the focused inter-shared-tail tests (asymmetric-value completion, segmentation-on / ccso-on honest stops) which reuse the in-file parse_body_with_ref seam. The shared-tail PARSER itself lives in the sibling crates/splot-core/src/headers/frame/inter_shared_tail.rs, keeping this growth to the FrameHeaderCore-owning wiring + tests. +40 (2026-06-22): the multi-reference runtime brick (decode-inter-multiref-runtime) adds FrameReferenceStateView.ref_base_q_idx (RefBaseQIdx, the §7.7 score input) plus the from_slots_with_base_q_idx constructor + its doc, so the §7.7 derive_implicit_ref_map can rank two valid reference slots exactly",
    },
    SourceLineAllowance {
        path: "crates/splot-validate/src/celu.rs",
        max_lines: 3_693,
        reason: "large CELU state machine; split separately from this validator refactor",
    },
    SourceLineAllowance {
        path: "crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs",
        max_lines: 2_610,
        reason: "cohesive ac0ej3 selectable transform-record walk. +~70 (2026-06-27, decode-ac0ej3-intra-recon-bridge): the reconstruction bridge threads an optional WienerNsLrReconSink<u16> + SelectableReconContext through the in-place selectable residual-chunk decode (decode_selectable_residual_chunks / decode_luma_records_for_chunk / decode_chroma_group / decode_chroma_residual_chunks and the SDP chroma-part branch) so each decoded LumaCoeffBlock and chroma group is reconstructed where it is parsed, plus the DeltaQState.qindex_u32 per-block dequant accessor. +~12 (2026-06-27, decode-ac0ej3-cardinal-hpred-recon): SelectableReconContext now also carries the leaf's resolved directional_luma (the supported_directional_luma() cardinal H/V mode), threaded into reconstruct_luma_transform at both the live and skipped-record sites so the SB-column-3 H_PRED IntrABC source reconstructs bit-exact. +2 (2026-06-27, codex review of decode-ac0ej3-cardinal-hpred-recon): SelectableReconContext.mrl_index (the §5.20.5.5 MrlIndex, read from the existing luma_transform_type_context via the new LumaTransformTypeContext::mrl_index accessor, NOT a new tuple element) lets the cardinal branch DEFER a multi-reference-line leaf — two irreducible field-assignment lines (recon_context + sdp_recon). +4 (2026-06-27, decode-ac0ej3-intrabc-recon): the §7.13.3.18 IntrABC reconstruction threads the optional sink through read_luma_shared_mode_info_prelude into read_intrabc_info (one new parameter + one new call-site arg at the prelude callsite) so an active IntrABC block copies its integer-vector CurrFrame predictor via the sink before the walk still fails closed at the currframe frontier. The reconstruction sink, SelectableReconContext, and the test driver live in the sibling wienerns_lr/recon.rs module; only the per-decode-site threading is here. +4 (2026-06-28, decode-ac0ej3-intrabc-walk-advance): SelectableReconContext.is_intrabc (two field-assignment lines at the recon_context + sdp_recon sites) exempts the now-continuing IntrABC skip leaf from the placeholder-DC skip-residual reconstruction. Split the chroma residual decoders into a submodule separately if this grows further.",
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

fn physical_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        // Plain newline count: the `bytecount` crate would be a new dependency
        // (gated) for a once-per-file count where speed is irrelevant.
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
            hard_violation_limit_label(&violation.path)
        );
    }
    for problem in &report.allowance_problems {
        eprintln!("source-line allowance problem: {problem}");
    }
}

fn hard_violation_limit_label(path: &str) -> String {
    let path = normalize_source_path(path);
    if let Some(allowance) = HARD_LINE_ALLOWANCES
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
        let allowance = &HARD_LINE_ALLOWANCES[0];
        let allowances = [*allowance];
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
            hard_violation_limit_label(&windows_path),
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
        let allowance = &HARD_LINE_ALLOWANCES[0];
        assert_eq!(
            hard_violation_limit_label(allowance.path),
            format!("allowance cap {}", allowance.max_lines)
        );
        assert_eq!(
            hard_violation_limit_label("src/lib.rs"),
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
