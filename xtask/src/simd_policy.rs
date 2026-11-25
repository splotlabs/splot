// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Startup-only SIMD policy check.
//!
//! Decoder runtime crates must resolve CPU SIMD capabilities once at session or
//! decoder startup and propagate the chosen concrete `SimdBackend` statically.
//! Runtime ISA probing (`Level::new`, `is_x86_feature_detected!`,
//! `is_aarch64_feature_detected!`) is restricted to `crates/splot-core/src/simd.rs`
//! in decoder crates. Shared reconstruction kernels cannot select a new `Level`.
//! Ambient or per-call dispatch in decoder crates
//! (`Level::baseline`, `baseline_simd!`, `fearless_simd::dispatch!`) is banned
//! outside tests. The symbol helper can use the compile-time baseline token.

use std::fmt;
use std::path::Path;

use anyhow::{Result, bail};

use crate::concurrency_policy::{SourceLine, collect_source_lines};

/// Crate paths governed by the startup-only decoder SIMD policy.
const DECODER_RUNTIME_CRATE_PREFIXES: &[&str] = &[
    "crates/splot-core/",
    "crates/splot-decode/",
    "crates/splot-capi/",
    "crates/splot-cli/",
];

/// File permitted to perform runtime CPU capability detection once and cache it.
const RUNTIME_PROBE_ALLOWLIST: &str = "crates/splot-core/src/simd.rs";

/// File permitted to read compile-time baseline tokens on its symbol read path.
const BASELINE_DISPATCH_ALLOWLIST: &str = "crates/splot-core/src/symbol.rs";

/// Needles indicating runtime ISA probing.
const RUNTIME_PROBE_NEEDLES: &[&str] = &[
    "Level::new",
    "is_x86_feature_detected!",
    "is_aarch64_feature_detected!",
];

/// Needles indicating ambient baseline or per-call dynamic SIMD dispatch.
const AMBIENT_DISPATCH_NEEDLES: &[&str] = &[
    "Level::baseline",
    "baseline_simd!",
    "fearless_simd::dispatch!",
    "use fearless_simd::dispatch",
];

/// A single policy violation reported by [`evaluate_simd_policy`].
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct SimdViolation {
    /// Workspace-relative file path where the violation occurred.
    pub path: String,
    /// 1-based line number of the offending code.
    pub line_no: usize,
    /// Banned token matched on the line.
    pub needle: &'static str,
    /// Actionable remediation guidance.
    pub message: &'static str,
}

impl fmt::Display for SimdViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: `{}` is banned here; {}",
            self.path, self.line_no, self.needle, self.message
        )
    }
}

/// Evaluates startup-only SIMD rules against pre-collected source lines.
pub(crate) fn evaluate_simd_policy(sources: &[SourceLine]) -> Vec<SimdViolation> {
    let mut violations = Vec::new();
    for line in sources {
        if line.in_test {
            continue;
        }
        let decoder_runtime = DECODER_RUNTIME_CRATE_PREFIXES
            .iter()
            .any(|prefix| line.path.starts_with(prefix));
        let shared_recon = line.path.starts_with("crates/splot-recon/");
        if !decoder_runtime && !shared_recon {
            continue;
        }
        let code = line.text.split("//").next().unwrap_or(line.text.as_str());
        if line.path != RUNTIME_PROBE_ALLOWLIST {
            for &needle in RUNTIME_PROBE_NEEDLES {
                if (decoder_runtime || needle == "Level::new") && code.contains(needle) {
                    violations.push(SimdViolation {
                        path: line.path.clone(),
                        line_no: line.line_no,
                        needle,
                        message: "probe CPU features only in crates/splot-core/src/simd.rs and propagate `SimdBackend` statically",
                    });
                }
            }
        }
        if decoder_runtime {
            for &needle in AMBIENT_DISPATCH_NEEDLES {
                if code.contains(needle)
                    && !(line.path == BASELINE_DISPATCH_ALLOWLIST && needle == "Level::baseline")
                {
                    violations.push(SimdViolation {
                        path: line.path.clone(),
                        line_no: line.line_no,
                        needle,
                        message: "accept a generic `SimdBackend` or explicit token from startup instead of ambient/per-call dispatch",
                    });
                }
            }
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

/// Verifies decoder crates perform SIMD probing and dispatch only at designated startup boundaries.
///
/// # Errors
/// Returns an error if workspace sources cannot be collected or any violation is found.
pub(crate) fn check_simd_policy(root: &Path) -> Result<()> {
    let sources = collect_source_lines(root)?;
    let violations = evaluate_simd_policy(&sources);
    if violations.is_empty() {
        eprintln!("check-simd-policy: ok");
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("{violation}");
        }
        bail!("check-simd-policy: {} violation(s)", violations.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(path: &str, line_no: usize, text: &str, in_test: bool) -> SourceLine {
        SourceLine {
            path: path.to_owned(),
            line_no,
            text: text.to_owned(),
            in_test,
        }
    }

    #[test]
    fn allows_clean_decoder_code_allowlisted_files_and_tests() {
        let lines = vec![
            src(
                "crates/splot-decode/src/filters/gdf_simd.rs",
                10,
                "fn run<B: SimdBackend>(backend: B) {}",
                false,
            ),
            src(
                "crates/splot-core/src/simd.rs",
                561,
                "*CACHED_LEVEL.get_or_init(Level::new)",
                false,
            ),
            src(
                "crates/splot-core/src/symbol.rs",
                222,
                "Level::baseline().as_avx2()",
                false,
            ),
            src(
                "crates/splot-decode/src/filters/gdf_simd.rs",
                1273,
                "baseline_simd!(simd => band_classes_with(simd))",
                true,
            ),
            src(
                "crates/splot-decode/src/prediction/chroma/cfl_luma.rs",
                28,
                "fearless_simd::Level::baseline().as_avx2()",
                true,
            ),
        ];
        assert!(evaluate_simd_policy(&lines).is_empty());
    }

    #[test]
    fn flags_imported_dispatch_and_shared_kernel_probes() {
        let lines = [
            src(
                "crates/splot-decode/src/example.rs",
                1,
                "use fearless_simd::dispatch as select_backend;",
                false,
            ),
            src(
                "crates/splot-decode/src/example.rs",
                2,
                "dispatch!(stage, frame);",
                false,
            ),
            src(
                "crates/splot-recon/src/example.rs",
                3,
                "let level = Level::new();",
                false,
            ),
            src(
                "crates/splot-recon/src/example.rs",
                4,
                "baseline_simd!(simd => encoder_kernel(simd));",
                false,
            ),
            src(
                "crates/splot-core/src/symbol.rs",
                5,
                "fearless_simd::dispatch!(level, simd => read_symbol(simd));",
                false,
            ),
        ];
        let violations = evaluate_simd_policy(&lines);
        assert_eq!(violations.len(), 3);
        assert_eq!(violations[0].needle, "fearless_simd::dispatch!");
        assert_eq!(violations[1].needle, "use fearless_simd::dispatch");
        assert_eq!(violations[2].needle, "Level::new");
    }

    #[test]
    fn flags_runtime_probing_and_ambient_dispatch_in_decode_and_capi() {
        let lines = vec![
            src(
                "crates/splot-decode/src/filters/example.rs",
                12,
                "let lvl = Level::new();",
                false,
            ),
            src(
                "crates/splot-decode/src/filters/example.rs",
                13,
                "let tok = Level::baseline().as_avx2();",
                false,
            ),
            src(
                "crates/splot-decode/src/filters/example.rs",
                14,
                "baseline_simd!(simd => run(simd));",
                false,
            ),
            src(
                "crates/splot-decode/src/filters/example.rs",
                15,
                "fearless_simd::dispatch!(lvl, simd => run(simd));",
                false,
            ),
            src(
                "crates/splot-decode/src/filters/example.rs",
                16,
                "let has = is_x86_feature_detected!(\"avx2\");",
                false,
            ),
            src(
                "crates/splot-capi/src/lib.rs",
                99,
                "let has = is_aarch64_feature_detected!(\"neon\");",
                false,
            ),
        ];
        let violations = evaluate_simd_policy(&lines);
        assert_eq!(violations.len(), 6);
        let rendered: Vec<String> = violations.iter().map(ToString::to_string).collect();
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("example.rs:12") && s.contains("Level::new"))
        );
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("example.rs:13") && s.contains("Level::baseline"))
        );
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("example.rs:14") && s.contains("baseline_simd!"))
        );
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("example.rs:15") && s.contains("fearless_simd::dispatch!"))
        );
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("example.rs:16") && s.contains("is_x86_feature_detected!"))
        );
        assert!(
            rendered
                .iter()
                .any(|s| s.contains("splot-capi/src/lib.rs:99")
                    && s.contains("is_aarch64_feature_detected!"))
        );
    }
}
