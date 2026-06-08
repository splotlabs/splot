// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Project automation for the splot workspace. Run via `cargo xtask <command>`.
//!
//! `xtask` is standalone automation: it depends on no `splot-*` crate.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand};

mod feature_status;

use feature_status::{CoverageFormat, StatusFormat};
use sha2::{Digest, Sha256};

/// SPDX identifier line every tracked `.rs` file must begin with.
const SPDX_LINE: &str = "// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0";

/// Prefix the second header line (the copyright line) must start with.
const SPDX_COPYRIGHT_PREFIX: &str = "// SPDX-FileCopyrightText: ";

/// Committed AV2 spec mirrors and the PDF sha256 each one is pinned to.
///
/// The integrity gate checks every mirror's `provenance.toml` against the pinned
/// hash so the committed copy cannot be silently re-pointed at a different PDF.
/// Update this when adding a new mirrored spec version (see
/// `scripts/spec/regenerate-av2-spec.sh`).
const SPEC_MIRRORS: &[(&str, &str)] = &[(
    "docs/spec/av2/1.0.0",
    "e9916f091e4e83446aad6b4601641c5b292e569c144c4163b26a4497573b533f",
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
    /// Verify member crates honor the one-way dependency direction.
    CheckDependencyDirection,
    /// Verify the committed AV2 spec mirror matches its CHECKSUMS and provenance.
    CheckSpecMirror,
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
    },
    /// (stub) Generate spec tables from the AV2 additional tables.
    GenTables,
    /// (stub) Fetch AV2/AOMedia conformance vectors.
    FetchVectors,
    /// (stub) Run AVM differential testing.
    Conformance,
    /// Generate a local HTML coverage report (requires `cargo-llvm-cov`).
    Coverage,
    /// Run a short local fuzz smoke session against the `parse_obu` target.
    ///
    /// Requires a nightly toolchain and `cargo-fuzz`. Defaults to 30 seconds.
    Fuzz {
        /// Maximum fuzzing time in seconds (default 30).
        #[arg(long, value_name = "SECS")]
        time: Option<u64>,
    },
    /// Run the networked cargo-deny advisory check (requires `cargo-deny`).
    Audit,
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
        Task::CheckDependencyDirection => check_dependency_direction(&workspace_root()?),
        Task::CheckSpecMirror => check_spec_mirror(&workspace_root()?),
        Task::FeatureStatus {
            format,
            category,
            kind,
            output,
        } => feature_status::run_feature_status(&workspace_root()?, format, category, kind, output),
        Task::CheckFeatureStatus => feature_status::run_check_feature_status(&workspace_root()?),
        Task::SpecCoverage { format } => {
            feature_status::run_spec_coverage(&workspace_root()?, format)
        }
        Task::GenTables => {
            gen_tables_stub();
            Ok(())
        }
        Task::FetchVectors => {
            fetch_vectors_stub();
            Ok(())
        }
        Task::Conformance => {
            conformance_stub();
            Ok(())
        }
        Task::Coverage => run_coverage(),
        Task::Fuzz { time } => run_fuzz(time),
        Task::Audit => run_audit(),
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
    run_cargo(&["build", "--workspace", "--all-targets", "--locked"])?;
    run_cargo(&["test", "--workspace", "--all-targets", "--locked"])?;
    // `--all-targets` skips doctests, so run them explicitly: the workspace
    // `missing_docs` lint implies doc examples that must keep compiling.
    run_cargo(&["test", "--doc", "--workspace", "--locked"])?;

    // External-binary checks: mandatory in CI (the workflow installs each tool),
    // run-if-present locally so a fresh checkout can still run `cargo xtask ci`.
    run_typos()?;
    run_cargo_machete()?;
    run_cargo_deny_offline()?;

    let root = workspace_root()?;
    check_license_headers(&root)?;
    check_dependency_direction(&root)?;
    check_spec_mirror(&root)?;
    feature_status::run_check_feature_status(&root)?;

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

/// Returns `true` if `bin --version` runs successfully. Used to gate optional
/// external checks so a fresh checkout without the tool can still run `xtask ci`.
fn tool_available(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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
fn run_cargo_deny_offline() -> Result<()> {
    run_if_present(
        "cargo-deny",
        "cargo-deny",
        &["check", "bans", "licenses", "sources"],
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
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Generates a local HTML coverage report. Report-only: no threshold is enforced here.
// TODO: once a baseline exists, add `--lcov`/Codecov upload and a
// `--fail-under-lines N` threshold (mirrors the report-only `coverage` CI job).
fn run_coverage() -> Result<()> {
    if !tool_available("cargo-llvm-cov") {
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
    ])
}

/// Runs a short local fuzz smoke session against `parse_obu` (nightly + cargo-fuzz).
fn run_fuzz(time: Option<u64>) -> Result<()> {
    if !tool_available("cargo-fuzz") || !nightly_available() {
        eprintln!(
            "fuzz: requires a nightly toolchain and cargo-fuzz; skipping.\n     \
             install: `rustup toolchain install nightly` and `cargo install cargo-fuzz --locked`"
        );
        return Ok(());
    }
    let secs = time.unwrap_or(30);
    let max_total_time = format!("-max_total_time={secs}");
    // `+nightly` is resolved by the rustup cargo proxy, so invoke `cargo` by name.
    // Mirror the CI fuzz-smoke guard flags so a local smoke catches the same classes
    // of bug: `-timeout` flags a hanging input, `-rss_limit_mb` an allocation blowup.
    run_program(
        "cargo",
        &[
            "+nightly",
            "fuzz",
            "run",
            "parse_obu",
            "--",
            &max_total_time,
            "-timeout=10",
            "-rss_limit_mb=2048",
        ],
    )
}

/// Runs the networked cargo-deny advisory check (separate from the offline gate).
fn run_audit() -> Result<()> {
    run_if_present(
        "cargo-deny",
        "cargo-deny",
        &["check", "advisories"],
        "`brew install cargo-deny` or `cargo install cargo-deny`",
    )
}

/// Checks XTASK-CONVENTIONAL-COMMITS: commit subjects follow Conventional Commits.
fn check_conventional_commits(root: &Path, rev_range: Option<&str>) -> Result<()> {
    let commits = git_commit_subjects(root, rev_range)?;
    if commits.is_empty() {
        let target = rev_range.unwrap_or("HEAD");
        bail!("check-conventional-commits: no commits found for `{target}`");
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

fn git_commit_subjects(root: &Path, rev_range: Option<&str>) -> Result<Vec<CommitSubject>> {
    if let Some(range) = rev_range {
        if range.trim().is_empty() {
            bail!("revision range must not be empty");
        }
        if range.starts_with('-') {
            bail!("revision range must not start with `-`");
        }
    }

    let output = if let Some(range) = rev_range {
        run_git(root, &["log", "--format=%H%x09%s", range])?
    } else {
        run_git(root, &["log", "-1", "--format=%H%x09%s"])?
    };
    parse_commit_subjects(&output)
}

fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let display = format!("git -C {} {}", root.display(), args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{display}` failed with {}; stderr:\n{}",
            output.status,
            stderr.trim_end()
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("`{display}` emitted non-UTF-8"))
}

fn parse_commit_subjects(output: &str) -> Result<Vec<CommitSubject>> {
    let mut commits = Vec::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let Some((sha, subject)) = line.split_once('\t') else {
            bail!("git log output line did not contain a tab separator: {line}");
        };
        commits.push(CommitSubject {
            sha: sha.to_owned(),
            subject: subject.to_owned(),
        });
    }
    Ok(commits)
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
fn workspace_root() -> Result<PathBuf> {
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

/// Lowercase hex SHA-256 of `bytes` (via the `sha2` crate), as used in the
/// spec-mirror `CHECKSUMS` manifest.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Verifies every committed AV2 spec mirror is byte-for-byte consistent with its
/// `CHECKSUMS` manifest and that `provenance.toml` pins the expected PDF sha256.
///
/// Deterministic and offline: it recomputes sha256 over the committed files and
/// never re-runs `pdftotext`, so it is stable across poppler versions. Drift
/// (hand-edits, missing or extra files, a re-pointed PDF) fails the gate.
fn check_spec_mirror(root: &Path) -> Result<()> {
    for (rel_dir, pinned_pdf_sha) in SPEC_MIRRORS {
        verify_spec_mirror_dir(&root.join(rel_dir), rel_dir, pinned_pdf_sha)?;
    }
    eprintln!("check-spec-mirror: ok");
    Ok(())
}

fn verify_spec_mirror_dir(dir: &Path, rel_dir: &str, pinned_pdf_sha: &str) -> Result<()> {
    if !dir.is_dir() {
        bail!(
            "spec mirror {rel_dir} is missing (expected directory {})",
            dir.display()
        );
    }

    // 1. Parse the CHECKSUMS manifest: "<hex>  <relpath>" per line.
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

    // 2. Walk the mirror, hashing every file except CHECKSUMS itself.
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

    // 3. Provenance must pin the expected PDF sha256.
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
    ("splot-validate", &["splot-core"]),
    ("splot-encode", &["splot-core"]),
    (
        "splot-cli",
        &["splot-core", "splot-validate", "splot-encode"],
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

fn workspace_members(root: &Path) -> Result<Vec<String>> {
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

fn read_manifest(path: &Path) -> Result<toml::Table> {
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
fn workspace_dep_names(root_manifest: &toml::Table) -> HashMap<String, String> {
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
    // Also scan platform-specific `[target.'cfg(...)'.dependencies]` tables.
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                collect_internal_deps(table, workspace_deps, &mut deps);
            }
        }
    }
    // Report each internal dependency once even if it appears in several tables.
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
fn resolved_dep_name(
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

fn gen_tables_stub() {
    eprintln!("xtask gen-tables: not yet implemented.");
    eprintln!(
        "Planned: download the AV2 v1.0.0 additional tables (all_tables.h) and generate Rust"
    );
    eprintln!("modules under crates/splot-core/src/ (AV2 § 9). See docs/SPEC-MAPPING.md.");
}

fn fetch_vectors_stub() {
    eprintln!("xtask fetch-vectors: not yet implemented.");
    eprintln!("Planned: fetch AV2/AOMedia conformance vectors into a gitignored tests/vectors/.");
}

fn conformance_stub() {
    eprintln!("xtask conformance: not yet implemented.");
    eprintln!("Planned: differential testing against AVM (avm encode -> splot validate).");
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
    fn spec_mirror_gate_detects_drift() -> Result<()> {
        let base = std::env::temp_dir().join(format!("xtask-spec-mirror-{}", std::process::id()));
        let dir = base.join("docs/spec/av2/1.0.0");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(dir.join("sub"))?;

        let body = "hello\n";
        let provenance = "pdf_sha256 = \"PIN\"\n";
        std::fs::write(dir.join("01.md"), body)?;
        std::fs::write(dir.join("provenance.toml"), provenance)?;
        // CHECKSUMS lists every generated file except itself.
        let manifest = format!(
            "{}  01.md\n{}  provenance.toml\n",
            sha256_hex(body.as_bytes()),
            sha256_hex(provenance.as_bytes()),
        );
        std::fs::write(dir.join("CHECKSUMS"), manifest)?;

        let rel = "docs/spec/av2/1.0.0";
        // Clean mirror passes.
        verify_spec_mirror_dir(&dir, rel, "PIN")?;
        // Tampered content fails.
        std::fs::write(dir.join("01.md"), "tampered\n")?;
        assert!(verify_spec_mirror_dir(&dir, rel, "PIN").is_err());
        // Restored content, but a re-pointed PDF hash fails.
        std::fs::write(dir.join("01.md"), body)?;
        assert!(verify_spec_mirror_dir(&dir, rel, "DIFFERENT").is_err());
        // An extra file not listed in CHECKSUMS fails.
        std::fs::write(dir.join("sub/extra.md"), "x")?;
        assert!(verify_spec_mirror_dir(&dir, rel, "PIN").is_err());

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
