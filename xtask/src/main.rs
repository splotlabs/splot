// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Project automation for the splot workspace. Run via `cargo xtask <command>`.
//!
//! `xtask` is standalone automation: it depends on no `splot-*` crate.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand};

mod ai_slop;
mod audit_scope;
mod comment_density;
mod concurrency_policy;
mod conformance;
mod decoder_conformance_coverage;
mod decoder_fixtures;
mod decoder_support;
mod diagnostic_registry;
mod doc_budget;
mod dupehound;
mod explain_registry;
mod feature_status;
mod fixtures;
mod gen_tables;
mod git_util;
mod reference_evidence;
mod seed_fuzz_corpus;
mod source_lines;
mod util;
mod zero_copy;

use audit_scope::{AuditScopeFormat, AuditScopeOptions};
use decoder_conformance_coverage::DecoderConformanceCoverageFormat;
use decoder_support::DecoderSupportFormat;
use feature_status::{CoverageFormat, StatusFormat};
use git_util::{run_git, sha256_hex};

/// SPDX identifier line every tracked `.rs` file must begin with.
const SPDX_LINE: &str = "// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0";

/// Prefix the second header line (the copyright line) must start with.
const SPDX_COPYRIGHT_PREFIX: &str = "// SPDX-FileCopyrightText: ";

/// `--ignore-filename-regex` excluding every workspace member except
/// `crates/splot-validate/` from the coverage threshold. The regex is matched
/// against the full (absolute) path, so xtask/fuzz use a `(^|/)` boundary rather
/// than a string anchor. Kept in sync with the `coverage` job in
/// `.github/workflows/ci.yml`. Exclusion is by name: extend this regex when
/// adding a workspace crate that should not gate here, or it joins the
/// threshold scope.
const SPLOT_VALIDATE_COVERAGE_IGNORE_REGEX: &str =
    r"crates/splot-(core|parallel|recon|decode|encode|cli)/|(^|/)xtask/|(^|/)fuzz/";

/// Committed AV2 spec mirrors, each pinned to `(dir, pdf_sha256, checksums_sha256)`.
///
/// `pdf_sha256` is the source PDF the mirror was generated from; `checksums_sha256`
/// is the sha256 of the mirror's own `CHECKSUMS` manifest. Pinning the manifest
/// hash here — in source, outside the mirror — is what makes the gate reject
/// content drift: editing any mirror file requires editing its `CHECKSUMS` line,
/// which changes the manifest hash and fails this pin, so the only legitimate way
/// to change the mirror is to regenerate it AND update this constant in a reviewed
/// commit. Update both hashes when (re)generating a mirror (see
/// `scripts/spec/regenerate-av2-spec.sh`).
const SPEC_MIRRORS: &[(&str, &str, &str)] = &[(
    "docs/spec/av2/1.0.0",
    "e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f",
    "d56cf5c10d24c03c3de675ccc78c42e1d56482726631e7ade71e768638a57273",
)];

/// Verbatim spec attachments committed under a mirror's `attachments/`, pinned to
/// `(mirror_dir, attachment_relpath, sha256)`. These are non-PDF-derived files
/// (e.g. the § 9 `all_tables.h` additional-tables header consumed by
/// `cargo xtask gen-tables`) fetched verbatim from the spec site. The mirror's own
/// `CHECKSUMS` already covers their bytes; this extra pin asserts the attachment's
/// sha256 is *also* recorded in `provenance.toml [attachments]`, so the recorded
/// provenance cannot silently drift from the committed bytes.
const SPEC_MIRROR_ATTACHMENTS: &[(&str, &str, &str)] = &[(
    "docs/spec/av2/1.0.0",
    "attachments/all_tables.h",
    "c3837e1c3b333e9ed51885c642562b519e3c3ed2ab385557d296c30a29c04ca1",
)];

#[derive(Parser, Debug)]
#[command(name = "xtask", about = "splot project automation")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand, Debug)]
enum Task {
    /// Run the local acceptance pipeline (fmt, clippy, build, test, repo checks).
    Ci,
    /// Verify commit subjects follow Conventional Commits.
    CheckConventionalCommits {
        /// Git revision range to inspect. Defaults to the current HEAD commit.
        #[arg(value_name = "REV_RANGE")]
        rev_range: Option<String>,
    },
    /// Verify a PR title follows Conventional Commits.
    CheckConventionalTitle {
        /// Pull request title to inspect.
        #[arg(value_name = "TITLE")]
        title: String,
    },
    /// Verify every tracked `.rs` file starts with the SPDX license header.
    CheckLicenseHeaders,
    /// Verify Rust source files stay within the repository source-line budget.
    CheckSourceLines,
    /// Verify implementation comments stay within the repository comment budget.
    CheckCommentDensity,
    /// Verify tracked source comments carry no banned AI-slop history/diary phrase.
    CheckAiSlop,
    /// Verify member crates honor the one-way dependency direction.
    CheckDependencyDirection,
    /// Verify the workspace honors the Rayon + crossbeam-channel concurrency policy.
    CheckConcurrencyPolicy,
    /// Verify the workspace honors the zero-copy media-buffer policy.
    CheckZeroCopyPolicy,
    /// Verify the committed AV2 spec mirror matches its CHECKSUMS and provenance.
    CheckSpecMirror,
    /// Verify diagnostic registry docs list exactly the emitted diagnostic rule ids.
    CheckDiagnosticRegistry,
    /// Verify every fuzz_targets/*.rs file has a matching `[[bin]]` entry in fuzz/Cargo.toml.
    CheckFuzzTargets,
    /// Seed `fuzz/corpus/<target>/` from the committed fixtures and conformance vectors.
    SeedFuzzCorpus,
    /// Verify tests/fixtures hashes + metadata against tests/fixtures/MANIFEST.toml (no decoder).
    CheckFixtures,
    /// Verify duplicate-code stays within tools/dupehound/budget.toml (needs `dupehound`).
    CheckDuplication,
    /// Verify committed manual markdown stays within the documentation budget.
    CheckDocBudget,
    /// Render the implementation matrix (docs/IMPLEMENTATION-MATRIX.toml).
    FeatureStatus {
        /// Output format.
        #[arg(long, value_enum, default_value_t = StatusFormat::Table)]
        format: StatusFormat,
        /// Filter to a single category.
        #[arg(long)]
        category: Option<String>,
        /// Filter to a single kind.
        #[arg(long)]
        kind: Option<String>,
        /// Write the rendered output to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Validate the implementation matrix and fail on drift.
    CheckFeatureStatus,
    /// Summarize implementation coverage from the matrix.
    SpecCoverage {
        /// Output format.
        #[arg(long, value_enum, default_value_t = CoverageFormat::Text)]
        format: CoverageFormat,
        /// Write the rendered output to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Render the writer coverage matrix.
    WriterCoverage {
        /// Output format.
        #[arg(long, value_enum, default_value_t = CoverageFormat::Text)]
        format: CoverageFormat,
        /// Write the rendered output to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Render the decoder support matrix (docs/DECODER-SUPPORT-MATRIX.toml).
    DecoderSupport {
        /// Output format.
        #[arg(long, value_enum, default_value_t = DecoderSupportFormat::Markdown)]
        format: DecoderSupportFormat,
        /// Write the rendered output to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Verify the optional generated decoder support status render is up to date.
    CheckDecoderSupport,
    /// Render the full decoder conformance coverage matrix.
    DecoderConformanceCoverage {
        /// Output format.
        #[arg(long, value_enum, default_value_t = DecoderConformanceCoverageFormat::Markdown)]
        format: DecoderConformanceCoverageFormat,
        /// Write the rendered output to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Verify the optional generated decoder conformance coverage render is up to date.
    CheckDecoderConformanceCoverage,
    /// Decoder-output oracle harness over the committed corpus (CONF-AVM-DECODE-ORACLE; no AVM).
    DecoderFixtures {
        #[command(subcommand)]
        cmd: DecoderFixturesCmd,
    },
    /// Verify docs/LOCAL-REFERENCE-EVIDENCE.toml is portable metadata.
    CheckReferenceEvidence,
    /// Generate the AV2 § 9 additional tables into `crates/splot-core/src/tables/`.
    GenTables {
        /// Verify the committed generated tables are up to date instead of writing.
        #[arg(long)]
        check: bool,
    },
    /// Generate the `splot explain` diagnostic registry from docs/DIAGNOSTICS.md.
    GenExplain {
        /// Verify the committed generated registry is up to date instead of writing.
        #[arg(long)]
        check: bool,
    },
    /// (stub) Fetch AV2/AOMedia conformance vectors.
    FetchVectors,
    /// Validate the committed conformance corpus against its manifest (no AVM).
    Conformance,
    /// Generate a local HTML coverage report (requires `cargo-llvm-cov`).
    Coverage,
    /// Run a short local fuzz smoke session against every fuzz target.
    ///
    /// Requires a nightly toolchain and `cargo-fuzz`. Each target runs for the
    /// given time (default 30 seconds).
    Fuzz {
        /// Maximum fuzzing time in seconds, per target (default 30).
        #[arg(long, value_name = "SECS")]
        time: Option<u64>,
    },
    /// Run the networked cargo-deny advisory check (requires `cargo-deny`).
    Audit,
    /// Compute changed-file scope for the heavy AV2 conformance audit.
    AuditScope {
        /// Git base revision for PR/diff mode.
        #[arg(long)]
        base: Option<String>,
        /// Audit ledger path for scheduled mode.
        #[arg(long)]
        ledger: Option<PathBuf>,
        /// Write the computed ledger update.
        #[arg(long)]
        write_ledger: bool,
        /// Select every in-scope file.
        #[arg(long)]
        all: bool,
        /// Output format.
        #[arg(long, value_enum, default_value_t = AuditScopeFormat::Json)]
        format: AuditScopeFormat,
        /// Outcome recorded when writing the generated ledger.
        #[arg(long, default_value = "success")]
        outcome: String,
    },
}

/// Subcommands of `cargo xtask decoder-fixtures`.
#[derive(Subcommand, Debug)]
enum DecoderFixturesCmd {
    /// Metadata-integrity gate: manifest/taxonomy shape, hashes, feature ids,
    /// orphan `.ivf` (no decode, no AVM). Wired into `cargo xtask ci`.
    Verify,
    /// Generate optional docs/decoder/DECODER-ORACLE-COVERAGE.md.
    Coverage {
        /// Verify the committed coverage doc is up to date instead of writing.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Task::Ci => run_ci(),
        Task::CheckConventionalCommits { rev_range } => {
            check_conventional_commits(&workspace_root()?, rev_range.as_deref())
        }
        Task::CheckConventionalTitle { title } => check_conventional_title(&title),
        Task::CheckLicenseHeaders => check_license_headers(&workspace_root()?),
        Task::CheckSourceLines => source_lines::check_source_lines(&workspace_root()?),
        Task::CheckCommentDensity => comment_density::check_comment_density(&workspace_root()?),
        Task::CheckAiSlop => ai_slop::check_ai_slop(&workspace_root()?),
        Task::CheckDependencyDirection => check_dependency_direction(&workspace_root()?),
        Task::CheckConcurrencyPolicy => {
            concurrency_policy::check_concurrency_policy(&workspace_root()?)
        }
        Task::CheckZeroCopyPolicy => zero_copy::check_zero_copy_policy(&workspace_root()?),
        Task::CheckSpecMirror => check_spec_mirror(&workspace_root()?),
        Task::CheckDiagnosticRegistry => {
            diagnostic_registry::check_diagnostic_registry(&workspace_root()?)
        }
        Task::CheckFuzzTargets => check_fuzz_targets(&workspace_root()?),
        Task::SeedFuzzCorpus => seed_fuzz_corpus::run_seed_fuzz_corpus(&workspace_root()?),
        Task::CheckFixtures => fixtures::check_fixtures(&workspace_root()?),
        Task::CheckDuplication => dupehound::check_duplication(&workspace_root()?),
        Task::CheckDocBudget => doc_budget::check_doc_budget(&workspace_root()?),
        Task::FeatureStatus {
            format,
            category,
            kind,
            output,
        } => feature_status::run_feature_status(
            &workspace_root()?,
            format,
            category.as_deref(),
            kind.as_deref(),
            output,
        ),
        Task::CheckFeatureStatus => feature_status::run_check_feature_status(&workspace_root()?),
        Task::SpecCoverage { format, output } => {
            feature_status::run_spec_coverage(&workspace_root()?, format, output)
        }
        Task::WriterCoverage { format, output } => {
            feature_status::run_writer_coverage(&workspace_root()?, format, output)
        }
        Task::DecoderSupport { format, output } => {
            decoder_support::run_decoder_support(&workspace_root()?, format, output)
        }
        Task::CheckDecoderSupport => decoder_support::run_check_decoder_support(&workspace_root()?),
        Task::DecoderConformanceCoverage { format, output } => {
            decoder_conformance_coverage::run_decoder_conformance_coverage(
                &workspace_root()?,
                format,
                output,
            )
        }
        Task::CheckDecoderConformanceCoverage => {
            decoder_conformance_coverage::run_check_decoder_conformance_coverage(&workspace_root()?)
        }
        Task::DecoderFixtures { cmd } => {
            let root = workspace_root()?;
            match cmd {
                DecoderFixturesCmd::Verify => decoder_fixtures::run_verify(&root),
                DecoderFixturesCmd::Coverage { check } => {
                    decoder_fixtures::run_coverage(&root, check)
                }
            }
        }
        Task::CheckReferenceEvidence => {
            reference_evidence::run_check_reference_evidence(&workspace_root()?)
        }
        Task::GenTables { check } => gen_tables::run_gen_tables(&workspace_root()?, check),
        Task::GenExplain { check } => explain_registry::run_gen_explain(&workspace_root()?, check),
        Task::FetchVectors => {
            fetch_vectors_stub();
            Ok(())
        }
        Task::Conformance => conformance::run_conformance(&workspace_root()?),
        Task::Coverage => run_coverage(),
        Task::Fuzz { time } => run_fuzz(time),
        Task::Audit => run_audit(),
        Task::AuditScope {
            base,
            ledger,
            write_ledger,
            all,
            format,
            outcome,
        } => audit_scope::run_audit_scope(
            &workspace_root()?,
            &AuditScopeOptions {
                base,
                ledger,
                write_ledger,
                all,
                format,
                outcome,
            },
        ),
    }
}

fn run_ci() -> Result<()> {
    run_cargo(&["fmt", "--all", "--", "--check"])?;
    run_cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    run_cargo(&["test", "--workspace", "--all-targets", "--locked"])?;
    run_cargo(&["test", "--doc", "--workspace", "--locked"])?;
    run_cargo_with_env(
        &[("RUSTDOCFLAGS", "-D warnings")],
        &["doc", "--workspace", "--no-deps", "--locked"],
    )?;

    run_typos()?;
    run_cargo_machete()?;
    run_cargo_deny_offline()?;
    run_openspec_validate()?;

    let root = workspace_root()?;
    check_license_headers(&root)?;
    source_lines::check_source_lines(&root)?;
    comment_density::check_comment_density(&root)?;
    ai_slop::check_ai_slop(&root)?;
    check_dependency_direction(&root)?;
    concurrency_policy::check_concurrency_policy(&root)?;
    zero_copy::check_zero_copy_policy(&root)?;
    check_spec_mirror(&root)?;
    check_fuzz_targets(&root)?;
    gen_tables::run_gen_tables(&root, true)?;
    explain_registry::run_gen_explain(&root, true)?;
    feature_status::run_check_feature_status(&root)?;
    decoder_support::run_check_decoder_support(&root)?;
    decoder_conformance_coverage::run_check_decoder_conformance_coverage(&root)?;
    decoder_fixtures::run_verify(&root)?;
    decoder_fixtures::run_coverage(&root, true)?;
    diagnostic_registry::check_diagnostic_registry(&root)?;
    fixtures::check_fixtures(&root)?;
    dupehound::check_duplication(&root)?;
    doc_budget::check_doc_budget(&root)?;

    eprintln!("ci: all checks passed");
    Ok(())
}

/// Returns the `cargo` executable, honoring the `CARGO` env var when set.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// Runs `program` with `args`, echoing the command and failing on a non-zero exit.
fn run_program(program: &str, args: &[&str]) -> Result<()> {
    let display = format!("{program} {}", args.join(" "));
    eprintln!("> {display}");
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !status.success() {
        bail!("`{display}` failed with {status}");
    }
    Ok(())
}

/// Runs `cargo` (honoring the `CARGO` env var) with `args`.
fn run_cargo(args: &[&str]) -> Result<()> {
    run_program(&cargo(), args)
}

/// Runs `cargo` with `args` and `envs` set for the child process only, echoing the
/// command and failing on a non-zero exit. Used to scope `RUSTDOCFLAGS` to the docs
/// gate without mutating the parent environment.
fn run_cargo_with_env(envs: &[(&str, &str)], args: &[&str]) -> Result<()> {
    let cargo = cargo();
    let env_prefix: String = envs.iter().fold(String::new(), |mut out, (key, value)| {
        let _ = write!(out, "{key}='{value}' ");
        out
    });
    let display = format!("{env_prefix}{cargo} {}", args.join(" "));
    eprintln!("> {display}");
    let status = Command::new(&cargo)
        .args(args)
        .envs(envs.iter().copied())
        .status()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !status.success() {
        bail!("`{display}` failed with {status}");
    }
    Ok(())
}

/// Returns `true` if `bin --version` runs successfully. Used to gate optional
/// external checks so a fresh checkout without the tool can still run `xtask ci`.
pub(crate) fn tool_available(bin: &str) -> bool {
    tool_available_with_args(bin, &["--version"])
}

/// Returns `true` if `bin args...` runs successfully. Probes a tool's presence with
/// caller-supplied args because some tools reject a bare `--version`: `cargo-llvm-cov`,
/// for example, demands the `llvm-cov` subcommand first (`cargo-llvm-cov --version`
/// errors, `cargo-llvm-cov llvm-cov --version` succeeds), so `tool_available` would
/// wrongly report it absent.
fn tool_available_with_args(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Runs an optional external check. If `probe` is not installed, prints an
/// actionable hint and returns `Ok` so a local `xtask ci` still completes; CI
/// installs every tool, so CI always enforces these.
fn run_if_present(probe: &str, program: &str, args: &[&str], install_hint: &str) -> Result<()> {
    if tool_available(probe) {
        run_program(program, args)
    } else {
        eprintln!(
            "ci: `{probe}` not installed; skipping `{program} {}`.\n     install: {install_hint}",
            args.join(" ")
        );
        Ok(())
    }
}

/// typos spell-check (<https://github.com/crate-ci/typos>), configured by `_typos.toml`.
fn run_typos() -> Result<()> {
    run_if_present(
        "typos",
        "typos",
        &[],
        "`brew install typos-cli` or `cargo install typos-cli`",
    )
}

/// cargo-machete unused-dependency check (`--with-metadata` resolves feature usage).
/// Invoked as the `cargo-machete` binary directly so the call does not depend on
/// cargo's external-subcommand argument handling.
fn run_cargo_machete() -> Result<()> {
    run_if_present(
        "cargo-machete",
        "cargo-machete",
        &["--with-metadata"],
        "`cargo install cargo-machete`",
    )
}

/// cargo-deny deterministic policy (bans, licenses, sources). Advisories need the
/// network, so they are left to CI and `cargo xtask audit`, not the offline gate.
/// `--all-features` is a global option (before `check`) and matches the CI
/// supply-chain job so the local gate cannot pass where CI fails.
fn run_cargo_deny_offline() -> Result<()> {
    run_if_present(
        "cargo-deny",
        "cargo-deny",
        &["--all-features", "check", "bans", "licenses", "sources"],
        "`brew install cargo-deny` or `cargo install cargo-deny`",
    )
}

/// Returns `true` if the `nightly` toolchain is installed (resolved via rustup).
fn nightly_available() -> bool {
    Command::new("rustup")
        .args(["run", "nightly", "rustc", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Generates a local HTML coverage report and enforces the `splot-validate`
/// line-coverage threshold (>= 90%) the CI `coverage` job gates on. The `--html` run
/// instruments the workspace; the follow-up `report` re-renders the same profile data
/// scoped to `crates/splot-validate/` via `--ignore-filename-regex`.
fn run_coverage() -> Result<()> {
    if !tool_available_with_args("cargo-llvm-cov", &["llvm-cov", "--version"]) {
        eprintln!(
            "coverage: `cargo-llvm-cov` not installed; skipping.\n     \
             install: `brew install cargo-llvm-cov` (or `cargo install cargo-llvm-cov`), \
             then `rustup component add llvm-tools-preview`"
        );
        return Ok(());
    }
    run_cargo(&[
        "llvm-cov",
        "--workspace",
        "--all-features",
        "--locked",
        "--html",
    ])?;
    run_cargo(&[
        "llvm-cov",
        "report",
        "--fail-under-lines",
        "90",
        "--ignore-filename-regex",
        SPLOT_VALIDATE_COVERAGE_IGNORE_REGEX,
    ])
}

/// Runs a short local fuzz smoke session against every fuzz target (nightly +
/// cargo-fuzz). Targets are enumerated from `fuzz/fuzz_targets/`, so listing needs
/// no nightly toolchain; each target then runs for `--time` seconds.
fn run_fuzz(time: Option<u64>) -> Result<()> {
    if !tool_available("cargo-fuzz") || !nightly_available() {
        eprintln!(
            "fuzz: requires a nightly toolchain and cargo-fuzz; skipping.\n     \
             install: `rustup toolchain install nightly` and `cargo install cargo-fuzz --locked`"
        );
        return Ok(());
    }
    let root = workspace_root()?;
    let targets = fuzz_targets(&root)?;
    if targets.is_empty() {
        bail!("fuzz: no targets found under fuzz/fuzz_targets/");
    }
    let secs = time.unwrap_or(30);
    let max_total_time = format!("-max_total_time={secs}");
    for target in &targets {
        run_program(
            "cargo",
            &[
                "+nightly",
                "fuzz",
                "run",
                target,
                "--",
                &max_total_time,
                "-timeout=10",
                "-rss_limit_mb=2048",
            ],
        )?;
    }
    Ok(())
}

/// Returns the fuzz target names (file stems of `fuzz/fuzz_targets/*.rs`), sorted for
/// a deterministic run order. Reading the directory avoids depending on a nightly
/// `cargo fuzz list`. Fails when the directory and the `[[bin]]` entries in
/// `fuzz/Cargo.toml` disagree: `cargo fuzz list` (used by the CI smoke job) only sees
/// registered `[[bin]]` targets, so an unregistered `.rs` file would be fuzzed by
/// neither — drift must be loud, not silently skipped.
pub(crate) fn fuzz_targets(root: &Path) -> Result<Vec<String>> {
    let dir = root.join("fuzz").join("fuzz_targets");
    let mut targets = Vec::new();
    let entries =
        std::fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            targets.push(stem.to_string());
        }
    }
    targets.sort();

    let manifest_path = root.join("fuzz").join("Cargo.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: toml::Value = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let mut registered: Vec<String> = manifest
        .get("bin")
        .and_then(|bins| bins.as_array())
        .map(|bins| {
            bins.iter()
                .filter_map(|bin| bin.get("name").and_then(|name| name.as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    registered.sort();
    if targets != registered {
        bail!(
            "fuzz target drift: fuzz/fuzz_targets/*.rs has [{}] but fuzz/Cargo.toml \
             [[bin]] entries are [{}]; register every target so `cargo fuzz list` \
             (and the CI smoke job) sees it",
            targets.join(", "),
            registered.join(", ")
        );
    }
    Ok(targets)
}

/// OpenSpec validation over every main spec and active change. Mirrors the CI
/// workflow's conditional step (CI does not install `openspec`; both gates use
/// the same run-if-present policy).
fn run_openspec_validate() -> Result<()> {
    run_if_present(
        "openspec",
        "openspec",
        &["validate", "--all", "--no-interactive"],
        "`npm install -g @fission-ai/openspec` (https://github.com/Fission-AI/OpenSpec)",
    )
}

/// Checks fuzz-target registration drift, on stable: the CI fuzz-smoke matrix
/// enumerates registered `[[bin]]` targets only (the `fuzz-list` job derives the
/// matrix from `Cargo.toml`; each leg's seed step uses `cargo fuzz list`), so an
/// unregistered `fuzz_targets/*.rs` file would be silently skipped there. Run as
/// `cargo xtask check-fuzz-targets` in the CI `ci` job and inside `cargo xtask ci`.
fn check_fuzz_targets(root: &Path) -> Result<()> {
    let targets = fuzz_targets(root)?;
    eprintln!("check-fuzz-targets: ok ({} target(s))", targets.len());
    Ok(())
}

/// Runs the networked cargo-deny advisory check (separate from the offline gate).
/// `--all-features` matches the CI advisory job, like the offline gate.
fn run_audit() -> Result<()> {
    run_if_present(
        "cargo-deny",
        "cargo-deny",
        &["--all-features", "check", "advisories"],
        "`brew install cargo-deny` or `cargo install cargo-deny`",
    )
}

/// Checks XTASK-CONVENTIONAL-COMMITS: commit subjects follow Conventional Commits.
fn check_conventional_commits(root: &Path, rev_range: Option<&str>) -> Result<()> {
    let ListedCommits { commits, raw_count } = git_commit_subjects(root, rev_range)?;
    if raw_count == 0 {
        let target = rev_range.unwrap_or("HEAD");
        bail!("check-conventional-commits: no commits found for `{target}`");
    }
    if commits.is_empty() {
        eprintln!("check-conventional-commits: ok (only merge commit(s) in range)");
        return Ok(());
    }

    let offenders: Vec<&CommitSubject> = commits
        .iter()
        .filter(|commit| !is_conventional_commit_subject(&commit.subject))
        .collect();

    if offenders.is_empty() {
        eprintln!(
            "check-conventional-commits: ok ({} commit(s))",
            commits.len()
        );
        return Ok(());
    }

    for offender in &offenders {
        eprintln!(
            "non-conventional commit {}: {}",
            short_sha(&offender.sha),
            offender.subject
        );
    }
    eprintln!(
        "expected: <type>[optional scope][!]: <description>; allowed types: {}",
        CONVENTIONAL_COMMIT_TYPES.join(", ")
    );
    bail!(
        "{} commit(s) do not use Conventional Commits",
        offenders.len()
    )
}

fn check_conventional_title(title: &str) -> Result<()> {
    if is_conventional_commit_subject(title) {
        eprintln!("check-conventional-title: ok");
        return Ok(());
    }

    eprintln!("non-conventional PR title: {title}");
    eprintln!(
        "expected: <type>[optional scope][!]: <description>; allowed types: {}",
        CONVENTIONAL_COMMIT_TYPES.join(", ")
    );
    bail!("PR title does not use Conventional Commits")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitSubject {
    sha: String,
    subject: String,
}

const CONVENTIONAL_COMMIT_TYPES: &[&str] = &[
    "build", "chore", "ci", "docs", "feat", "fix", "perf", "refactor", "revert", "style", "test",
];

/// The result of listing commit subjects for the Conventional Commits check:
/// `commits` holds the non-merge commits to validate, while `raw_count` counts
/// every commit git listed (so an empty revision range can be distinguished
/// from a range containing only exempt merge commits).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListedCommits {
    commits: Vec<CommitSubject>,
    raw_count: usize,
}

fn git_commit_subjects(root: &Path, rev_range: Option<&str>) -> Result<ListedCommits> {
    if let Some(range) = rev_range {
        if range.trim().is_empty() {
            bail!("revision range must not be empty");
        }
        if range.starts_with('-') {
            bail!("revision range must not start with `-`");
        }
    }

    let output = if let Some(range) = rev_range {
        run_git(root, &["log", "--format=%H%x09%P%x09%s", range])?
    } else {
        run_git(root, &["log", "-1", "--format=%H%x09%P%x09%s"])?
    };
    parse_commit_subjects(&output)
}

fn parse_commit_subjects(output: &str) -> Result<ListedCommits> {
    let mut commits = Vec::new();
    let mut raw_count = 0usize;
    for line in output.lines().filter(|line| !line.is_empty()) {
        let Some((sha, rest)) = line.split_once('\t') else {
            bail!("git log output line did not contain a tab separator: {line}");
        };
        let Some((parents, subject)) = rest.split_once('\t') else {
            bail!("git log output line did not contain a parents field: {line}");
        };
        raw_count += 1;
        if parents.split_whitespace().count() >= 2 && subject.starts_with("Merge ") {
            continue;
        }
        commits.push(CommitSubject {
            sha: sha.to_owned(),
            subject: subject.to_owned(),
        });
    }
    Ok(ListedCommits { commits, raw_count })
}

fn is_conventional_commit_subject(subject: &str) -> bool {
    let Some((prefix, description)) = subject.split_once(": ") else {
        return false;
    };
    if description.trim().is_empty() {
        return false;
    }

    let type_and_scope = prefix.strip_suffix('!').unwrap_or(prefix);
    let Some((commit_type, scope)) = split_commit_type_and_scope(type_and_scope) else {
        return false;
    };

    CONVENTIONAL_COMMIT_TYPES.contains(&commit_type) && scope.is_none_or(is_valid_commit_scope)
}

fn split_commit_type_and_scope(type_and_scope: &str) -> Option<(&str, Option<&str>)> {
    if let Some((commit_type, scope_with_end)) = type_and_scope.split_once('(') {
        let scope = scope_with_end.strip_suffix(')')?;
        if commit_type.is_empty() || scope.is_empty() || scope.contains('(') || scope.contains(')')
        {
            return None;
        }
        return Some((commit_type, Some(scope)));
    }

    if type_and_scope.is_empty() || type_and_scope.contains(')') {
        return None;
    }
    Some((type_and_scope, None))
}

fn is_valid_commit_scope(scope: &str) -> bool {
    scope
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '/' | '.'))
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Returns the workspace root (the parent of this xtask crate).
pub(crate) fn workspace_root() -> Result<PathBuf> {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR is not set")?;
    let root = Path::new(&manifest_dir)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent directory"))?
        .to_path_buf();
    Ok(root)
}

fn check_license_headers(root: &Path) -> Result<()> {
    let mut offenders = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory {}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to stat {}", path.display()))?;
            if file_type.is_dir() {
                if !is_skipped_dir(&path) {
                    stack.push(path);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                && !has_spdx_header(&path)?
            {
                offenders.push(path);
            }
        }
    }

    if offenders.is_empty() {
        eprintln!("check-license-headers: ok");
        Ok(())
    } else {
        for path in &offenders {
            eprintln!("missing SPDX header: {}", path.display());
        }
        bail!(
            "{} file(s) missing the SPDX license header",
            offenders.len()
        )
    }
}

fn is_skipped_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("target" | ".git")
    )
}

fn has_spdx_header(path: &Path) -> Result<bool> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut lines = contents.lines();
    let identifier = lines.next().map(str::trim_end);
    let copyright = lines.next().map(str::trim_end);
    Ok(identifier == Some(SPDX_LINE)
        && copyright.is_some_and(|line| line.starts_with(SPDX_COPYRIGHT_PREFIX)))
}

/// Verifies every committed AV2 spec mirror is byte-for-byte consistent with its
/// `CHECKSUMS` manifest and that `provenance.toml` pins the expected PDF sha256.
///
/// Deterministic and offline: it recomputes sha256 over the committed files and
/// never re-runs `pdftotext`, so it is stable across poppler versions. Drift
/// (hand-edits, missing or extra files, a re-pointed PDF) fails the gate.
fn check_spec_mirror(root: &Path) -> Result<()> {
    for (rel_dir, pinned_pdf_sha, pinned_checksums_sha) in SPEC_MIRRORS {
        verify_spec_mirror_dir(
            &root.join(rel_dir),
            rel_dir,
            pinned_pdf_sha,
            pinned_checksums_sha,
        )?;
    }
    for (rel_dir, rel_attachment, pinned_sha) in SPEC_MIRROR_ATTACHMENTS {
        verify_spec_mirror_attachment(&root.join(rel_dir), rel_dir, rel_attachment, pinned_sha)?;
    }
    eprintln!("check-spec-mirror: ok");
    Ok(())
}

/// Verifies a committed verbatim attachment matches its pinned sha256 on disk and
/// that the same sha256 is recorded in the mirror's `provenance.toml [attachments]`
/// table. The mirror `CHECKSUMS` pin (step 3 of [`verify_spec_mirror_dir`]) already
/// guards the bytes; this additionally couples the recorded provenance to those
/// bytes so a re-pointed or stale provenance entry fails the gate.
fn verify_spec_mirror_attachment(
    dir: &Path,
    rel_dir: &str,
    rel_attachment: &str,
    pinned_sha: &str,
) -> Result<()> {
    let mut problems: Vec<String> = Vec::new();

    let attachment_path = dir.join(rel_attachment);
    let bytes = std::fs::read(&attachment_path)
        .with_context(|| format!("failed to read {}", attachment_path.display()))?;
    let got = sha256_hex(&bytes);
    if got != *pinned_sha {
        problems.push(format!(
            "attachment {rel_attachment} sha256 {got:?} does not match the pinned {pinned_sha:?}"
        ));
    }

    let provenance_path = dir.join("provenance.toml");
    let provenance = std::fs::read_to_string(&provenance_path)
        .with_context(|| format!("failed to read {}", provenance_path.display()))?;
    let table: toml::Table = toml::from_str(&provenance)
        .with_context(|| format!("failed to parse {}", provenance_path.display()))?;
    let recorded = table
        .get("attachments")
        .and_then(|v| v.as_table())
        .and_then(|attachments| {
            attachments.values().find_map(|entry| {
                let entry = entry.as_table()?;
                let path = entry.get("path").and_then(|v| v.as_str())?;
                (path == rel_attachment)
                    .then(|| entry.get("sha256").and_then(|v| v.as_str()))
                    .flatten()
            })
        });
    match recorded {
        None => problems.push(format!(
            "provenance.toml has no [attachments] entry with path = \"{rel_attachment}\""
        )),
        Some(sha) if sha != pinned_sha => problems.push(format!(
            "provenance.toml attachment sha256 {sha:?} does not match the pinned {pinned_sha:?}"
        )),
        Some(_) => {}
    }

    if problems.is_empty() {
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("spec mirror {rel_dir}: {problem}");
        }
        bail!(
            "spec mirror {rel_dir} attachment {rel_attachment} failed integrity check ({} problem(s))",
            problems.len()
        )
    }
}

fn verify_spec_mirror_dir(
    dir: &Path,
    rel_dir: &str,
    pinned_pdf_sha: &str,
    pinned_checksums_sha: &str,
) -> Result<()> {
    if !dir.is_dir() {
        bail!(
            "spec mirror {rel_dir} is missing (expected directory {})",
            dir.display()
        );
    }

    let checksums_path = dir.join("CHECKSUMS");
    let manifest = std::fs::read_to_string(&checksums_path)
        .with_context(|| format!("failed to read {}", checksums_path.display()))?;
    let mut expected: HashMap<String, String> = HashMap::new();
    for (lineno, line) in manifest.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, "  ");
        let hash = parts.next().unwrap_or_default().trim();
        let rel = parts.next().map(str::trim).unwrap_or_default();
        if hash.len() != 64 || rel.is_empty() {
            bail!(
                "{}: malformed manifest line {}",
                checksums_path.display(),
                lineno + 1
            );
        }
        expected.insert(rel.to_string(), hash.to_string());
    }

    let mut problems: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in
            std::fs::read_dir(&d).with_context(|| format!("failed to read {}", d.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|_| anyhow!("path escaped the mirror directory"))?
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "CHECKSUMS" {
                continue;
            }
            seen.insert(rel.clone());
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let got = sha256_hex(&bytes);
            match expected.get(&rel) {
                None => problems.push(format!("not listed in CHECKSUMS: {rel}")),
                Some(want) if *want != got => problems.push(format!("checksum mismatch: {rel}")),
                Some(_) => {}
            }
        }
    }
    for rel in expected.keys() {
        if !seen.contains(rel) {
            problems.push(format!("listed in CHECKSUMS but missing on disk: {rel}"));
        }
    }

    let manifest_sha = sha256_hex(manifest.as_bytes());
    if manifest_sha != pinned_checksums_sha {
        problems.push(format!(
            "CHECKSUMS sha256 {manifest_sha:?} does not match the pinned {pinned_checksums_sha:?} (regenerate the mirror and update SPEC_MIRRORS)"
        ));
    }

    let provenance_path = dir.join("provenance.toml");
    let provenance = std::fs::read_to_string(&provenance_path)
        .with_context(|| format!("failed to read {}", provenance_path.display()))?;
    let table: toml::Table = toml::from_str(&provenance)
        .with_context(|| format!("failed to parse {}", provenance_path.display()))?;
    let pdf_sha = table
        .get("pdf_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if pdf_sha != pinned_pdf_sha {
        problems.push(format!(
            "provenance.toml pdf_sha256 {pdf_sha:?} does not match the pinned {pinned_pdf_sha:?}"
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("spec mirror {rel_dir}: {problem}");
        }
        bail!(
            "spec mirror {rel_dir} failed integrity check ({} problem(s)); regenerate via scripts/spec/regenerate-av2-spec.sh",
            problems.len()
        )
    }
}

fn check_dependency_direction(root: &Path) -> Result<()> {
    let root_manifest = read_manifest(&root.join("Cargo.toml"))?;
    let workspace_deps = workspace_dep_names(&root_manifest);
    let mut violations = Vec::new();

    for member in workspace_members(root)? {
        let manifest_path = root.join(&member).join("Cargo.toml");
        let manifest = read_manifest(&manifest_path)?;
        let name = manifest_package_name(&manifest)
            .with_context(|| format!("{} has no [package].name", manifest_path.display()))?;
        let permitted = allowed_internal_deps(&name).unwrap_or(&[]);
        for dependency in internal_deps(&manifest, &workspace_deps) {
            if !permitted.contains(&dependency.as_str()) {
                violations.push(format!(
                    "{name} must not depend on internal crate {dependency}"
                ));
            }
        }
    }

    if violations.is_empty() {
        eprintln!("check-dependency-direction: ok");
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("{violation}");
        }
        bail!("{} dependency-direction violation(s)", violations.len())
    }
}

/// The one-way dependency rule: each internal crate and the internal crates it may
/// depend on. Single source of truth for both the allow-list and the set of names
/// recognized as internal.
const INTERNAL_DEP_RULES: &[(&str, &[&str])] = &[
    ("splot-core", &[]),
    ("splot-parallel", &[]),
    ("splot-tables", &[]),
    ("splot-recon", &["splot-core", "splot-tables"]),
    (
        "splot-decode",
        &["splot-core", "splot-recon", "splot-parallel"],
    ),
    ("splot-validate", &["splot-core"]),
    (
        "splot-encode",
        &[
            "splot-core",
            "splot-parallel",
            "splot-recon",
            "splot-tables",
        ],
    ),
    (
        "splot-cli",
        &[
            "splot-core",
            "splot-decode",
            "splot-validate",
            "splot-encode",
            "splot-parallel",
        ],
    ),
    ("xtask", &[]),
];

/// Internal crates `name` is permitted to depend on, or `None` if `name` is not a
/// known workspace member.
fn allowed_internal_deps(name: &str) -> Option<&'static [&'static str]> {
    INTERNAL_DEP_RULES
        .iter()
        .find(|(crate_name, _)| *crate_name == name)
        .map(|(_, deps)| *deps)
}

/// Returns `true` if `name` is one of this workspace's internal crates.
fn is_internal_crate(name: &str) -> bool {
    INTERNAL_DEP_RULES
        .iter()
        .any(|(crate_name, _)| *crate_name == name)
}

pub(crate) fn workspace_members(root: &Path) -> Result<Vec<String>> {
    let manifest = read_manifest(&root.join("Cargo.toml"))?;
    let members = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow!("workspace manifest has no members array"))?;
    let mut out = Vec::new();
    for value in members {
        let member = value
            .as_str()
            .ok_or_else(|| anyhow!("workspace member is not a string"))?;
        out.push(member.to_owned());
    }
    Ok(out)
}

pub(crate) fn read_manifest(path: &Path) -> Result<toml::Table> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str::<toml::Table>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn manifest_package_name(manifest: &toml::Table) -> Option<String> {
    manifest
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

/// Maps each `[workspace.dependencies]` alias to the real crate name it resolves
/// to (honoring a `package = "..."` rename), for resolving `x.workspace = true`.
pub(crate) fn workspace_dep_names(root_manifest: &toml::Table) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let deps = root_manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table);
    if let Some(deps) = deps {
        for (alias, value) in deps {
            let name = value
                .as_table()
                .and_then(|table| table.get("package"))
                .and_then(toml::Value::as_str)
                .map_or_else(|| alias.clone(), str::to_owned);
            map.insert(alias.clone(), name);
        }
    }
    map
}

fn internal_deps(manifest: &toml::Table, workspace_deps: &HashMap<String, String>) -> Vec<String> {
    let mut deps = Vec::new();
    collect_internal_deps(manifest, workspace_deps, &mut deps);
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                collect_internal_deps(table, workspace_deps, &mut deps);
            }
        }
    }
    deps.sort_unstable();
    deps.dedup();
    deps
}

fn collect_internal_deps(
    parent: &toml::Table,
    workspace_deps: &HashMap<String, String>,
    deps: &mut Vec<String>,
) {
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = parent.get(table_name).and_then(toml::Value::as_table) {
            for (key, value) in table {
                let name = resolved_dep_name(key, value, workspace_deps);
                if is_internal_crate(&name) {
                    deps.push(name);
                }
            }
        }
    }
}

/// Resolves a dependency's real crate name: a local `package = "..."` rename, then
/// a workspace-inherited alias (`x.workspace = true`, resolved via the root
/// `[workspace.dependencies]`), else the dependency key itself.
pub(crate) fn resolved_dep_name(
    key: &str,
    value: &toml::Value,
    workspace_deps: &HashMap<String, String>,
) -> String {
    let Some(table) = value.as_table() else {
        return key.to_owned();
    };
    if let Some(package) = table.get("package").and_then(toml::Value::as_str) {
        return package.to_owned();
    }
    if table.get("workspace").and_then(toml::Value::as_bool) == Some(true)
        && let Some(real) = workspace_deps.get(key)
    {
        return real.clone();
    }
    key.to_owned()
}

fn fetch_vectors_stub() {
    eprintln!("xtask fetch-vectors: not yet implemented.");
    eprintln!("Planned: fetch AV2/AOMedia conformance vectors into a gitignored tests/vectors/.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conventional_subjects_accept_expected_forms() {
        for subject in [
            "feat: add Annex B parser",
            "fix(parser): reject truncated OBU headers",
            "ci(github-actions): check commit subjects",
            "docs!: rewrite contribution rules",
            "refactor(core)!: split OBU types",
        ] {
            assert!(
                is_conventional_commit_subject(subject),
                "{subject} should be accepted"
            );
        }
    }

    #[test]
    fn conventional_subjects_reject_non_matching_forms() {
        for subject in [
            "add Annex B parser",
            "Feat: uppercase type",
            "feat: ",
            "feat(scope):",
            "feat(): empty scope",
            "feat(scope space): invalid scope",
            "feat(SCOPE): uppercase scope should be rejected",
            "merge pull request #1",
            "wip: unsupported type",
        ] {
            assert!(
                !is_conventional_commit_subject(subject),
                "{subject} should be rejected"
            );
        }
    }

    #[test]
    fn conventional_title_check_reuses_subject_rules() {
        assert!(check_conventional_title("ci: enforce conventional commits").is_ok());
        assert!(check_conventional_title("Enforce Conventional Commits in CI").is_err());
    }

    #[test]
    fn parse_commit_subjects_skips_git_generated_merge_commits() -> Result<()> {
        let output = "aaa\tp1\tfeat: real change\nbbb\tp1 p2\tMerge branch 'main' into feature\nccc\tp1\tchore: follow-up\n";
        let listed = parse_commit_subjects(output)?;
        assert_eq!(listed.raw_count, 3);
        let subjects: Vec<&str> = listed
            .commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect();
        assert_eq!(subjects, ["feat: real change", "chore: follow-up"]);
        Ok(())
    }

    #[test]
    fn parse_commit_subjects_keeps_custom_subject_merge_commits() -> Result<()> {
        let output = "ddd\tp1 p2\tsync with main\n";
        let listed = parse_commit_subjects(output)?;
        assert_eq!(listed.raw_count, 1);
        assert_eq!(listed.commits.len(), 1);
        assert_eq!(listed.commits[0].subject, "sync with main");
        Ok(())
    }

    #[test]
    fn parse_commit_subjects_merge_only_output_keeps_raw_count() -> Result<()> {
        let output = "bbb\tp1 p2\tMerge remote-tracking branch 'origin/main' into feature\n";
        let listed = parse_commit_subjects(output)?;
        assert_eq!(listed.raw_count, 1);
        assert!(listed.commits.is_empty());
        Ok(())
    }

    #[test]
    fn git_commit_subjects_rejects_option_like_revision_range() -> Result<()> {
        let Err(err) = git_commit_subjects(Path::new("."), Some("--format=%s")) else {
            bail!("option-like revision range should be rejected before git runs");
        };
        assert!(
            err.to_string()
                .contains("revision range must not start with `-`")
        );
        Ok(())
    }

    #[test]
    fn dependency_direction_allows_encoder_recon_edge_only() -> Result<()> {
        let Some(encoder_deps) = allowed_internal_deps("splot-encode") else {
            bail!("splot-encode should have dependency policy");
        };
        assert!(encoder_deps.contains(&"splot-core"));
        assert!(encoder_deps.contains(&"splot-parallel"));
        assert!(encoder_deps.contains(&"splot-recon"));
        assert!(!encoder_deps.contains(&"splot-decode"));
        assert!(!encoder_deps.contains(&"splot-validate"));
        assert!(!encoder_deps.contains(&"splot-cli"));
        Ok(())
    }

    #[test]
    fn dependency_direction_allows_recon_core_and_tables_only() -> Result<()> {
        let Some(recon_deps) = allowed_internal_deps("splot-recon") else {
            bail!("splot-recon should have dependency policy");
        };
        assert_eq!(recon_deps, ["splot-core", "splot-tables"]);
        Ok(())
    }

    #[test]
    fn spec_mirror_gate_detects_drift() -> Result<()> {
        let base = std::env::temp_dir().join(format!("xtask-spec-mirror-{}", std::process::id()));
        let dir = base.join("docs/spec/av2/1.0.0");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(dir.join("sub"))?;

        let body = "hello\n";
        let provenance = "pdf_sha256 = \"PIN\"\n";
        let write_mirror = |file_body: &str| -> Result<String> {
            std::fs::write(dir.join("01.md"), file_body)?;
            std::fs::write(dir.join("provenance.toml"), provenance)?;
            let manifest = format!(
                "{}  01.md\n{}  provenance.toml\n",
                sha256_hex(file_body.as_bytes()),
                sha256_hex(provenance.as_bytes()),
            );
            std::fs::write(dir.join("CHECKSUMS"), &manifest)?;
            Ok(sha256_hex(manifest.as_bytes()))
        };

        let rel = "docs/spec/av2/1.0.0";
        let manifest_sha = write_mirror(body)?;
        verify_spec_mirror_dir(&dir, rel, "PIN", &manifest_sha)?;
        std::fs::write(dir.join("01.md"), "tampered\n")?;
        assert!(verify_spec_mirror_dir(&dir, rel, "PIN", &manifest_sha).is_err());
        let laundered_sha = write_mirror("tampered\n")?;
        assert_ne!(laundered_sha, manifest_sha);
        assert!(verify_spec_mirror_dir(&dir, rel, "PIN", &manifest_sha).is_err());
        let manifest_sha = write_mirror(body)?;
        assert!(verify_spec_mirror_dir(&dir, rel, "DIFFERENT", &manifest_sha).is_err());
        std::fs::write(dir.join("sub/extra.md"), "x")?;
        assert!(verify_spec_mirror_dir(&dir, rel, "PIN", &manifest_sha).is_err());

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    #[test]
    fn spec_mirror_attachment_gate_couples_bytes_and_provenance() -> Result<()> {
        let base = std::env::temp_dir().join(format!("xtask-spec-att-{}", std::process::id()));
        let dir = base.join("docs/spec/av2/1.0.0");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(dir.join("attachments"))?;

        let body = b"Foo[1] = { 0 }\n";
        let sha = sha256_hex(body);
        std::fs::write(dir.join("attachments/all_tables.h"), body)?;
        let rel = "docs/spec/av2/1.0.0";
        let att_rel = "attachments/all_tables.h";

        let write_provenance = |recorded_sha: &str| -> Result<()> {
            let provenance = format!(
                "pdf_sha256 = \"PIN\"\n\n[attachments.all_tables_h]\npath = \"{att_rel}\"\nsha256 = \"{recorded_sha}\"\n"
            );
            std::fs::write(dir.join("provenance.toml"), provenance)?;
            Ok(())
        };

        write_provenance(&sha)?;
        verify_spec_mirror_attachment(&dir, rel, att_rel, &sha)?;

        write_provenance("0000")?;
        assert!(verify_spec_mirror_attachment(&dir, rel, att_rel, &sha).is_err());

        std::fs::write(dir.join("provenance.toml"), "pdf_sha256 = \"PIN\"\n")?;
        assert!(verify_spec_mirror_attachment(&dir, rel, att_rel, &sha).is_err());

        write_provenance(&sha)?;
        std::fs::write(dir.join("attachments/all_tables.h"), b"tampered\n")?;
        assert!(verify_spec_mirror_attachment(&dir, rel, att_rel, &sha).is_err());

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
