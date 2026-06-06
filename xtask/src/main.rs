// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Project automation for the splot workspace. Run via `cargo xtask <command>`.
//!
//! `xtask` is standalone automation: it depends on no `splot-*` crate.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand};

/// SPDX identifier line every tracked `.rs` file must begin with.
const SPDX_LINE: &str = "// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0";

/// Prefix the second header line (the copyright line) must start with.
const SPDX_COPYRIGHT_PREFIX: &str = "// SPDX-FileCopyrightText: ";

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
    /// Verify every tracked `.rs` file starts with the SPDX license header.
    CheckLicenseHeaders,
    /// Verify member crates honor the one-way dependency direction.
    CheckDependencyDirection,
    /// (stub) Generate spec tables from the AV2 additional tables.
    GenTables,
    /// (stub) Fetch AV2/AOMedia conformance vectors.
    FetchVectors,
    /// (stub) Run AVM differential testing.
    Conformance,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Task::Ci => run_ci(),
        Task::CheckLicenseHeaders => check_license_headers(&workspace_root()?),
        Task::CheckDependencyDirection => check_dependency_direction(&workspace_root()?),
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

    let root = workspace_root()?;
    check_license_headers(&root)?;
    check_dependency_direction(&root)?;

    eprintln!("ci: all checks passed");
    Ok(())
}

/// Returns the `cargo` executable, honoring the `CARGO` env var when set.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn run_cargo(args: &[&str]) -> Result<()> {
    let display = format!("cargo {}", args.join(" "));
    eprintln!("> {display}");
    let status = Command::new(cargo())
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !status.success() {
        bail!("`{display}` failed with {status}");
    }
    Ok(())
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

fn check_dependency_direction(root: &Path) -> Result<()> {
    let mut violations = Vec::new();

    for member in workspace_members(root)? {
        let manifest_path = root.join(&member).join("Cargo.toml");
        let manifest = read_manifest(&manifest_path)?;
        let name = manifest_package_name(&manifest)
            .with_context(|| format!("{} has no [package].name", manifest_path.display()))?;
        let permitted = allowed_internal_deps(&name).unwrap_or(&[]);
        for dependency in internal_deps(&manifest) {
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

fn internal_deps(manifest: &toml::Table) -> Vec<String> {
    let mut deps = Vec::new();
    collect_internal_deps(manifest, &mut deps);
    // Also scan platform-specific `[target.'cfg(...)'.dependencies]` tables.
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                collect_internal_deps(table, &mut deps);
            }
        }
    }
    deps
}

fn collect_internal_deps(parent: &toml::Table, deps: &mut Vec<String>) {
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = parent.get(table_name).and_then(toml::Value::as_table) {
            for (key, value) in table {
                let name = resolved_dep_name(key, value);
                if is_internal_crate(&name) {
                    deps.push(name);
                }
            }
        }
    }
}

/// Resolves a dependency's real crate name, honoring a `package = "..."` rename.
fn resolved_dep_name(key: &str, value: &toml::Value) -> String {
    value
        .as_table()
        .and_then(|table| table.get("package"))
        .and_then(toml::Value::as_str)
        .map_or_else(|| key.to_owned(), str::to_owned)
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
