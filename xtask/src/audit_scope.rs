// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deterministic changed-file selection for the heavy AV2 conformance audit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::git_util::{run_git, sha256_hex};

const PROTOCOL_VERSION: u32 = 1;
const DEFAULT_LEDGER_PATH: &str = "docs/audits/av2-conformance-ledger.json";

/// Output format for `audit-scope`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum AuditScopeFormat {
    /// JSON for audit coordinators and scheduled agents.
    Json,
    /// Human-readable text summary.
    Text,
}

/// Runtime options for `cargo xtask audit-scope`.
#[derive(Debug)]
pub(crate) struct AuditScopeOptions {
    /// Optional git base revision for PR/diff mode.
    pub(crate) base: Option<String>,
    /// Optional ledger path for scheduled mode.
    pub(crate) ledger: Option<PathBuf>,
    /// Whether to write the computed ledger update.
    pub(crate) write_ledger: bool,
    /// Select all in-scope files regardless of diff or ledger state.
    pub(crate) all: bool,
    /// Output format.
    pub(crate) format: AuditScopeFormat,
    /// Outcome recorded in the generated ledger.
    pub(crate) outcome: String,
}

/// Implements `cargo xtask audit-scope`.
pub(crate) fn run_audit_scope(root: &Path, options: AuditScopeOptions) -> Result<()> {
    if options.write_ledger && options.base.is_some() {
        bail!("audit-scope --write-ledger cannot be used with --base");
    }

    let report = build_report(root, &options)?;
    if options.write_ledger {
        write_ledger(root, &report.ledger_path, &report.ledger_update)?;
    }

    match options.format {
        AuditScopeFormat::Json => {
            let mut json = serde_json::to_string_pretty(&report)
                .context("failed to serialize audit-scope JSON")?;
            json.push('\n');
            print!("{json}");
        }
        AuditScopeFormat::Text => print!("{}", render_text(&report)),
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct AuditScopeReport {
    protocol_version: u32,
    mode: AuditScopeMode,
    audited_commit: String,
    ledger_path: String,
    workspace_members: Vec<WorkspaceMember>,
    force_wide_review_triggers: Vec<ForceWideReviewTrigger>,
    candidate_count: usize,
    candidates: Vec<AuditCandidate>,
    ledger_update: AuditLedger,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum AuditScopeMode {
    All,
    Diff,
    Ledger,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize)]
struct WorkspaceMember {
    package: String,
    path: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize)]
struct ForceWideReviewTrigger {
    path: String,
    reason: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct AuditCandidate {
    path: String,
    sha256: String,
    scope_kind: String,
    reasons: Vec<String>,
    feature_ids: Vec<String>,
    reviewer_lanes: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct AuditLedger {
    protocol_version: u32,
    audited_commit: String,
    outcome: String,
    files: Vec<AuditLedgerFile>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct AuditLedgerFile {
    path: String,
    sha256: String,
    feature_ids: Vec<String>,
    outcome: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TrackedAuditFile {
    path: String,
    sha256: String,
    scope_kind: String,
    feature_ids: Vec<String>,
    reviewer_lanes: Vec<String>,
}

struct FeatureIndex {
    known_ids: BTreeSet<String>,
    by_module: BTreeMap<String, BTreeSet<String>>,
}

fn build_report(root: &Path, options: &AuditScopeOptions) -> Result<AuditScopeReport> {
    let ledger_path = options
        .ledger
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LEDGER_PATH));
    let workspace_members = load_workspace_members(root)?;
    let feature_index = load_feature_index(root)?;
    let tracked_paths = git_lines(root, &["ls-files", "--cached"])?;
    let ledger_path_string = path_to_string(&ledger_path);
    let excluded_paths = BTreeSet::from([ledger_path_string.clone()]);
    let in_scope = tracked_audit_files(
        root,
        &tracked_paths,
        &workspace_members,
        &feature_index,
        &excluded_paths,
    )?;
    let audited_commit = git_single_line(root, &["rev-parse", "HEAD"])?;

    let changed_paths = if let Some(base) = &options.base {
        Some(changed_paths_from_base(root, base)?)
    } else {
        None
    };
    let existing_ledger = if options.base.is_none() && !options.all {
        read_ledger(root, &ledger_path)?
    } else {
        None
    };

    let mode = if options.all {
        AuditScopeMode::All
    } else if options.base.is_some() {
        AuditScopeMode::Diff
    } else {
        AuditScopeMode::Ledger
    };

    let force_wide_review_triggers =
        force_wide_review_triggers(changed_paths.as_ref(), existing_ledger.as_ref(), &in_scope);
    let candidates = select_candidates(
        &in_scope,
        changed_paths.as_ref(),
        existing_ledger.as_ref(),
        options.all,
        !force_wide_review_triggers.is_empty(),
    );
    let ledger_update = build_ledger(&audited_commit, &options.outcome, &in_scope);

    Ok(AuditScopeReport {
        protocol_version: PROTOCOL_VERSION,
        mode,
        audited_commit,
        ledger_path: ledger_path_string,
        workspace_members,
        candidate_count: candidates.len(),
        candidates,
        force_wide_review_triggers,
        ledger_update,
    })
}

fn render_text(report: &AuditScopeReport) -> String {
    let mut out = String::new();
    out.push_str("audit-scope\n");
    out.push_str(&format!(
        "  protocol_version: {}\n",
        report.protocol_version
    ));
    out.push_str(&format!("  mode: {:?}\n", report.mode));
    out.push_str(&format!("  audited_commit: {}\n", report.audited_commit));
    out.push_str(&format!("  candidate_count: {}\n", report.candidate_count));
    if !report.force_wide_review_triggers.is_empty() {
        out.push_str("  force_wide_review_triggers:\n");
        for trigger in &report.force_wide_review_triggers {
            out.push_str(&format!("    - {} ({})\n", trigger.path, trigger.reason));
        }
    }
    out.push_str("  candidates:\n");
    for candidate in &report.candidates {
        out.push_str(&format!(
            "    - {} [{}] reasons={} features={} lanes={}\n",
            candidate.path,
            candidate.scope_kind,
            candidate.reasons.join(","),
            candidate.feature_ids.join(","),
            candidate.reviewer_lanes.join(","),
        ));
    }
    out
}

fn write_ledger(root: &Path, ledger_path: &str, ledger: &AuditLedger) -> Result<()> {
    let path = root.join(ledger_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut json =
        serde_json::to_string_pretty(ledger).context("failed to serialize audit ledger JSON")?;
    json.push('\n');
    std::fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))
}

fn read_ledger(root: &Path, ledger_path: &Path) -> Result<Option<AuditLedger>> {
    let path = root.join(ledger_path);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let ledger = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(ledger))
}

fn build_ledger(audited_commit: &str, outcome: &str, files: &[TrackedAuditFile]) -> AuditLedger {
    let ledger_files = files
        .iter()
        .map(|file| AuditLedgerFile {
            path: file.path.clone(),
            sha256: file.sha256.clone(),
            feature_ids: file.feature_ids.clone(),
            outcome: outcome.to_owned(),
        })
        .collect();
    AuditLedger {
        protocol_version: PROTOCOL_VERSION,
        audited_commit: audited_commit.to_owned(),
        outcome: outcome.to_owned(),
        files: ledger_files,
    }
}

fn select_candidates(
    files: &[TrackedAuditFile],
    changed_paths: Option<&BTreeSet<String>>,
    ledger: Option<&AuditLedger>,
    all: bool,
    force_wide_review: bool,
) -> Vec<AuditCandidate> {
    let ledger_files = ledger_files_by_path(ledger);
    let ledger_outcome = ledger.map(|ledger| ledger.outcome.as_str());
    let mut out = Vec::new();
    for file in files {
        let mut reasons = BTreeSet::new();
        if all {
            reasons.insert("all".to_owned());
        }
        if let Some(changed) = changed_paths
            && changed.contains(&file.path)
        {
            reasons.insert("changed-in-diff".to_owned());
        }
        if changed_paths.is_none() && !all {
            match ledger_files
                .as_ref()
                .and_then(|entries| entries.get(&file.path))
            {
                None => {
                    reasons.insert("ledger-missing".to_owned());
                }
                Some(entry) if entry.sha256 != file.sha256 => {
                    reasons.insert("content-hash-changed".to_owned());
                }
                Some(_) if ledger_outcome != Some("success") => {
                    reasons.insert("ledger-outcome-not-success".to_owned());
                }
                Some(entry) if entry.outcome != "success" => {
                    reasons.insert("ledger-file-outcome-not-success".to_owned());
                }
                Some(_) => {}
            }
        }
        if force_wide_review {
            reasons.insert("wide-review-triggered".to_owned());
        }
        if !reasons.is_empty() {
            out.push(AuditCandidate {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
                scope_kind: file.scope_kind.clone(),
                reasons: reasons.into_iter().collect(),
                feature_ids: file.feature_ids.clone(),
                reviewer_lanes: file.reviewer_lanes.clone(),
            });
        }
    }
    out
}

fn ledger_files_by_path(ledger: Option<&AuditLedger>) -> Option<BTreeMap<String, AuditLedgerFile>> {
    ledger.map(|ledger| {
        ledger
            .files
            .iter()
            .map(|file| (file.path.clone(), file.clone()))
            .collect()
    })
}

fn force_wide_review_triggers(
    changed_paths: Option<&BTreeSet<String>>,
    ledger: Option<&AuditLedger>,
    files: &[TrackedAuditFile],
) -> Vec<ForceWideReviewTrigger> {
    let changed: BTreeSet<String> = if let Some(changed_paths) = changed_paths {
        changed_paths.clone()
    } else if let Some(ledger) = ledger {
        let ledger_files = ledger_files_by_path(Some(ledger)).unwrap_or_default();
        let mut changed = BTreeSet::new();
        let mut current_paths = BTreeSet::new();
        for file in files {
            current_paths.insert(file.path.clone());
            let Some(entry) = ledger_files.get(&file.path) else {
                changed.insert(file.path.clone());
                continue;
            };
            let file_changed = entry.sha256 != file.sha256
                || ledger.outcome != "success"
                || entry.outcome != "success";
            if file_changed {
                changed.insert(file.path.clone());
            }
        }
        for entry in &ledger.files {
            if !current_paths.contains(&entry.path) {
                changed.insert(entry.path.clone());
            }
        }
        changed
    } else {
        BTreeSet::new()
    };

    changed
        .iter()
        .filter_map(|path| {
            force_wide_review_reason(path).map(|reason| ForceWideReviewTrigger {
                path: path.clone(),
                reason,
            })
        })
        .collect()
}

fn force_wide_review_reason(path: &str) -> Option<String> {
    match path {
        "AGENTS.md" => Some("repository-agent-instructions".to_owned()),
        "CLAUDE.md" => Some("repository-agent-instructions".to_owned()),
        ".github/copilot-instructions.md" => Some("repository-agent-instructions".to_owned()),
        "Cargo.toml" => Some("workspace-membership".to_owned()),
        "docs/IMPLEMENTATION-MATRIX.toml" => Some("implementation-matrix".to_owned()),
        "docs/SPEC-MAPPING.md" => Some("spec-mapping".to_owned()),
        "docs/FEATURE-TRACKING.md" => Some("feature-tracking".to_owned()),
        _ if path.starts_with(".codex/skills/splot-") => Some("audit-skill".to_owned()),
        _ if path.starts_with(".claude/skills/splot-") => Some("audit-skill".to_owned()),
        _ if path.starts_with(".github/skills/splot-") => Some("audit-skill".to_owned()),
        _ if path.starts_with("xtask/src/audit_scope.rs") => Some("audit-scope-tooling".to_owned()),
        _ if path.starts_with("docs/spec/av2/") && path.ends_with("/CHECKSUMS") => {
            Some("spec-mirror-integrity".to_owned())
        }
        _ if path.starts_with("docs/spec/av2/") && path.ends_with("/provenance.toml") => {
            Some("spec-mirror-provenance".to_owned())
        }
        _ => None,
    }
}

fn tracked_audit_files(
    root: &Path,
    tracked_paths: &[String],
    workspace_members: &[WorkspaceMember],
    feature_index: &FeatureIndex,
    excluded_paths: &BTreeSet<String>,
) -> Result<Vec<TrackedAuditFile>> {
    let mut files = Vec::new();
    for path in tracked_paths {
        if excluded_paths.contains(path) {
            continue;
        }
        let Some(scope_kind) = scope_kind(path, workspace_members) else {
            continue;
        };
        let abs = root.join(path);
        if !abs.is_file() {
            continue;
        }
        let bytes =
            std::fs::read(&abs).with_context(|| format!("failed to read {}", abs.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut feature_ids = feature_ids_for_path(path, feature_index);
        feature_ids.extend(feature_ids_in_text(&text, &feature_index.known_ids));
        let reviewer_lanes = reviewer_lanes(path, &scope_kind);
        files.push(TrackedAuditFile {
            path: path.clone(),
            sha256: sha256_hex(&bytes),
            scope_kind,
            feature_ids: feature_ids.into_iter().collect(),
            reviewer_lanes: reviewer_lanes.into_iter().collect(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn scope_kind(path: &str, workspace_members: &[WorkspaceMember]) -> Option<String> {
    if path.starts_with("docs/spec/av2/") {
        if path.ends_with("/CHECKSUMS") || path.ends_with("/provenance.toml") {
            return Some("spec-mirror-integrity".to_owned());
        }
        return None;
    }
    if path == DEFAULT_LEDGER_PATH {
        return None;
    }
    for member in workspace_members {
        if path == member.path || path.starts_with(&format!("{}/", member.path)) {
            return Some(format!("workspace:{}", member.package));
        }
    }
    match path {
        "AGENTS.md" | "CLAUDE.md" | "Cargo.toml" | "Cargo.lock" | "_typos.toml" | "deny.toml" => {
            Some("repository-process".to_owned())
        }
        ".github/copilot-instructions.md" | ".github/PULL_REQUEST_TEMPLATE.md" => {
            Some("agent-guidance".to_owned())
        }
        _ if path.starts_with("docs/") => Some("docs".to_owned()),
        _ if path.starts_with("openspec/") => Some("openspec".to_owned()),
        _ if path.starts_with("fuzz/") => Some("fuzz".to_owned()),
        _ if path.starts_with("scripts/") => Some("automation".to_owned()),
        _ if path.starts_with(".github/workflows/") => Some("automation".to_owned()),
        _ if path.starts_with(".github/prompts/") => Some("agent-guidance".to_owned()),
        _ if path.starts_with(".github/skills/") => Some("agent-guidance".to_owned()),
        _ if path.starts_with(".codex/skills/") => Some("agent-guidance".to_owned()),
        _ if path.starts_with(".claude/skills/") => Some("agent-guidance".to_owned()),
        _ => None,
    }
}

fn reviewer_lanes(path: &str, scope_kind: &str) -> BTreeSet<String> {
    let mut lanes = BTreeSet::new();
    if scope_kind.starts_with("workspace:") {
        lanes.insert("safety-boundaries".to_owned());
    }
    if path.contains("splot-core") || path.contains("parser") || path.contains("parse") {
        lanes.insert("parser-safety".to_owned());
        lanes.insert("spec-citation".to_owned());
    }
    if path.contains("splot-validate") || path.contains("diagnostic") || path.contains("validator")
    {
        lanes.insert("validator-diagnostics".to_owned());
    }
    if contains_any(
        path,
        &[
            "encode", "encoder", "decode", "decoder", "writer", "inspect",
        ],
    ) {
        lanes.insert("encoder-decoder-writer-inspector".to_owned());
        lanes.insert("spec-citation".to_owned());
    }
    if contains_any(path, &["test", "tests", "fuzz", "conformance", "fixtures"]) {
        lanes.insert("tests-fuzz-conformance".to_owned());
    }
    if contains_any(
        path,
        &["IMPLEMENTATION-MATRIX", "FEATURE-TRACKING", "openspec/"],
    ) {
        lanes.insert("feature-matrix-openspec".to_owned());
    }
    if scope_kind == "agent-guidance" || path == "AGENTS.md" || path == "CLAUDE.md" {
        lanes.insert("agent-guidance".to_owned());
    }
    if scope_kind == "automation"
        || path.starts_with("xtask/")
        || path.starts_with(".github/workflows/")
    {
        lanes.insert("automation".to_owned());
    }
    if path.starts_with("docs/") || scope_kind == "spec-mirror-integrity" {
        lanes.insert("spec-citation".to_owned());
    }
    if lanes.is_empty() {
        lanes.insert("general-repo-rules".to_owned());
    }
    lanes
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn feature_ids_for_path(path: &str, feature_index: &FeatureIndex) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (module, ids) in &feature_index.by_module {
        if path == module || path_is_under_module_dir(path, module) {
            out.extend(ids.iter().cloned());
        }
    }
    out
}

fn path_is_under_module_dir(path: &str, module: &str) -> bool {
    let Some((_, name)) = module.rsplit_once('/') else {
        return false;
    };
    !name.contains('.') && path.starts_with(&format!("{module}/"))
}

fn feature_ids_in_text(text: &str, known_ids: &BTreeSet<String>) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
        .filter(|token| known_ids.contains(*token))
        .map(str::to_owned)
        .collect()
}

fn load_feature_index(root: &Path) -> Result<FeatureIndex> {
    let path = root.join("docs/IMPLEMENTATION-MATRIX.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    feature_index_from_matrix_text(&text)
}

fn feature_index_from_matrix_text(text: &str) -> Result<FeatureIndex> {
    let value: toml::Value =
        toml::from_str(text).context("failed to parse implementation matrix")?;
    let features = value
        .get("feature")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut known_ids = BTreeSet::new();
    let mut by_module: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for feature in features {
        let Some(table) = feature.as_table() else {
            continue;
        };
        let Some(id) = table.get("id").and_then(toml::Value::as_str) else {
            continue;
        };
        known_ids.insert(id.to_owned());
        if let Some(module) = table.get("module").and_then(toml::Value::as_str)
            && !module.is_empty()
        {
            by_module
                .entry(module.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
    }
    Ok(FeatureIndex {
        known_ids,
        by_module,
    })
}

fn load_workspace_members(root: &Path) -> Result<Vec<WorkspaceMember>> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    workspace_members_from_manifest_text(root, &text)
}

fn workspace_members_from_manifest_text(root: &Path, text: &str) -> Result<Vec<WorkspaceMember>> {
    let value: toml::Value = toml::from_str(text).context("failed to parse workspace manifest")?;
    let members = value
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut seen_paths = BTreeSet::new();
    for member in members {
        let Some(path) = member.as_str() else {
            continue;
        };
        for member_path in workspace_member_paths(root, path)? {
            if !seen_paths.insert(member_path.clone()) {
                continue;
            }
            let package = package_name_for_member(root, &member_path)?;
            out.push(WorkspaceMember {
                package,
                path: member_path,
            });
        }
    }
    out.sort();
    Ok(out)
}

fn workspace_member_paths(root: &Path, pattern: &str) -> Result<Vec<String>> {
    if !pattern.contains('*') {
        return Ok(vec![pattern.to_owned()]);
    }

    let segments: Vec<&str> = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut paths = Vec::new();
    expand_workspace_member_pattern(root, Path::new(""), &segments, &mut paths)?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn expand_workspace_member_pattern(
    root: &Path,
    prefix: &Path,
    segments: &[&str],
    paths: &mut Vec<String>,
) -> Result<()> {
    let Some((segment, rest)) = segments.split_first() else {
        if root.join(prefix).join("Cargo.toml").is_file() {
            paths.push(path_to_string(prefix));
        }
        return Ok(());
    };

    if segment.contains('*') {
        let dir = root.join(prefix);
        if !dir.is_dir() {
            return Ok(());
        }
        let mut children = Vec::new();
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", dir.display()))?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if matches_glob_segment(segment, name) {
                children.push(name.to_owned());
            }
        }
        children.sort();
        for child in children {
            expand_workspace_member_pattern(root, &prefix.join(child), rest, paths)?;
        }
        return Ok(());
    }

    expand_workspace_member_pattern(root, &prefix.join(segment), rest, paths)
}

fn matches_glob_segment(pattern: &str, candidate: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut cursor = candidate;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 && anchored_start {
            let Some(rest) = cursor.strip_prefix(part) else {
                return false;
            };
            cursor = rest;
        } else if let Some(position) = cursor.find(part) {
            cursor = &cursor[position + part.len()..];
        } else {
            return false;
        }
    }

    !anchored_end || parts.last().is_some_and(|part| candidate.ends_with(part))
}

fn package_name_for_member(root: &Path, member: &str) -> Result<String> {
    let manifest_path = root.join(member).join("Cargo.toml");
    if manifest_path.is_file() {
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let value: toml::Value = toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        if let Some(name) = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        {
            return Ok(name.to_owned());
        }
    }
    Ok(member
        .rsplit_once('/')
        .map_or(member, |(_, name)| name)
        .to_owned())
}

fn changed_paths_from_base(root: &Path, base: &str) -> Result<BTreeSet<String>> {
    if base.trim().is_empty() || base.starts_with('-') {
        bail!("audit-scope --base must not be empty or start with `-`");
    }
    let mut args = vec!["diff", "--name-only", "--diff-filter=ACDMRT", base];
    args.push("HEAD");
    args.push("--");
    Ok(git_lines(root, &args)?.into_iter().collect())
}

fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let output = run_git(root, args)?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect())
}

fn git_single_line(root: &Path, args: &[&str]) -> Result<String> {
    let lines = git_lines(root, args)?;
    lines
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("git command returned no output"))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_future_workspace_members_without_hardcoded_crate_names() -> Result<()> {
        let root = temp_root("audit-scope-workspace")?;
        let crate_dir = root.join("crates/splot-decode");
        std::fs::create_dir_all(crate_dir.join("src"))?;
        std::fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/splot-decode"]
"#,
        )?;
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            r#"
[package]
name = "splot-decode"
version = "0.0.0"
edition = "2024"
"#,
        )?;

        let members = load_workspace_members(&root)?;
        assert_eq!(
            scope_kind("crates/splot-decode/src/lib.rs", &members).as_deref(),
            Some("workspace:splot-decode")
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn workspace_member_globs_expand_before_classification() -> Result<()> {
        let root = temp_root("audit-scope-workspace-globs")?;
        let crate_dir = root.join("crates/splot-core");
        std::fs::create_dir_all(crate_dir.join("src"))?;
        std::fs::create_dir_all(root.join("crates/not-a-package"))?;
        std::fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/*"]
"#,
        )?;
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            r#"
[package]
name = "splot-core"
version = "0.0.0"
edition = "2024"
"#,
        )?;

        let members = load_workspace_members(&root)?;
        assert_eq!(
            members,
            vec![WorkspaceMember {
                package: "splot-core".to_owned(),
                path: "crates/splot-core".to_owned(),
            }]
        );
        assert_eq!(
            scope_kind("crates/splot-core/src/lib.rs", &members).as_deref(),
            Some("workspace:splot-core")
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn ledger_selection_skips_unchanged_and_selects_hash_changes() {
        let files = vec![
            tracked("crates/splot-core/src/obu.rs", "aaa"),
            tracked("crates/splot-core/src/types.rs", "bbb"),
        ];
        let ledger = AuditLedger {
            protocol_version: PROTOCOL_VERSION,
            audited_commit: "old".to_owned(),
            outcome: "success".to_owned(),
            files: vec![
                ledger_file("crates/splot-core/src/obu.rs", "aaa"),
                ledger_file("crates/splot-core/src/types.rs", "old"),
            ],
        };

        let candidates = select_candidates(&files, None, Some(&ledger), false, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, "crates/splot-core/src/types.rs");
        assert_eq!(candidates[0].reasons, vec!["content-hash-changed"]);
    }

    #[test]
    fn ledger_selection_reaudits_non_success_outcomes() {
        let files = vec![
            tracked("crates/splot-core/src/obu.rs", "aaa"),
            tracked("crates/splot-core/src/types.rs", "bbb"),
        ];
        let ledger = AuditLedger {
            protocol_version: PROTOCOL_VERSION,
            audited_commit: "old".to_owned(),
            outcome: "success".to_owned(),
            files: vec![
                ledger_file("crates/splot-core/src/obu.rs", "aaa"),
                AuditLedgerFile {
                    path: "crates/splot-core/src/types.rs".to_owned(),
                    sha256: "bbb".to_owned(),
                    feature_ids: Vec::new(),
                    outcome: "failure".to_owned(),
                },
            ],
        };

        let candidates = select_candidates(&files, None, Some(&ledger), false, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, "crates/splot-core/src/types.rs");
        assert_eq!(
            candidates[0].reasons,
            vec!["ledger-file-outcome-not-success"]
        );

        let failed_ledger = AuditLedger {
            outcome: "failure".to_owned(),
            files: vec![ledger_file("crates/splot-core/src/obu.rs", "aaa")],
            ..ledger
        };
        let candidates = select_candidates(&files[..1], None, Some(&failed_ledger), false, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].reasons, vec!["ledger-outcome-not-success"]);
    }

    #[test]
    fn force_wide_trigger_selects_otherwise_unchanged_files() {
        let files = vec![
            tracked("docs/IMPLEMENTATION-MATRIX.toml", "new"),
            tracked("crates/splot-core/src/obu.rs", "same"),
        ];
        let ledger = AuditLedger {
            protocol_version: PROTOCOL_VERSION,
            audited_commit: "old".to_owned(),
            outcome: "success".to_owned(),
            files: vec![
                ledger_file("docs/IMPLEMENTATION-MATRIX.toml", "old"),
                ledger_file("crates/splot-core/src/obu.rs", "same"),
            ],
        };
        let triggers = force_wide_review_triggers(None, Some(&ledger), &files);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].reason, "implementation-matrix");

        let candidates = select_candidates(&files, None, Some(&ledger), false, true);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| {
            candidate
                .reasons
                .iter()
                .any(|r| r == "wide-review-triggered")
        }));
    }

    #[test]
    fn force_wide_trigger_treats_non_success_ledger_as_changed() {
        let files = vec![tracked("docs/IMPLEMENTATION-MATRIX.toml", "same")];
        let ledger = AuditLedger {
            protocol_version: PROTOCOL_VERSION,
            audited_commit: "old".to_owned(),
            outcome: "failure".to_owned(),
            files: vec![ledger_file("docs/IMPLEMENTATION-MATRIX.toml", "same")],
        };

        let triggers = force_wide_review_triggers(None, Some(&ledger), &files);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].path, "docs/IMPLEMENTATION-MATRIX.toml");
        assert_eq!(triggers[0].reason, "implementation-matrix");
    }

    #[test]
    fn force_wide_trigger_detects_deleted_ledger_paths() {
        let ledger = AuditLedger {
            protocol_version: PROTOCOL_VERSION,
            audited_commit: "old".to_owned(),
            outcome: "success".to_owned(),
            files: vec![ledger_file("AGENTS.md", "old")],
        };

        let triggers = force_wide_review_triggers(None, Some(&ledger), &[]);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].path, "AGENTS.md");
        assert_eq!(triggers[0].reason, "repository-agent-instructions");
    }

    #[test]
    fn changed_paths_from_base_includes_deleted_force_wide_paths() -> Result<()> {
        let root = temp_git_repo("audit-scope-deleted-force-wide")?;
        std::fs::write(root.join("AGENTS.md"), "rules\n")?;
        git_commit_all(&root, "test: add agents")?;
        std::fs::remove_file(root.join("AGENTS.md"))?;
        git_commit_all(&root, "test: remove agents")?;

        let changed = changed_paths_from_base(&root, "HEAD~1")?;
        assert!(changed.contains("AGENTS.md"));
        let triggers = force_wide_review_triggers(Some(&changed), None, &[]);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].reason, "repository-agent-instructions");

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn run_audit_scope_rejects_diff_mode_ledger_writes() -> Result<()> {
        let Err(err) = run_audit_scope(
            Path::new("."),
            AuditScopeOptions {
                base: Some("HEAD~1".to_owned()),
                ledger: None,
                write_ledger: true,
                all: false,
                format: AuditScopeFormat::Json,
                outcome: "success".to_owned(),
            },
        ) else {
            bail!("diff-mode ledger writes should be rejected before report construction");
        };

        assert!(
            err.to_string()
                .contains("audit-scope --write-ledger cannot be used with --base")
        );
        Ok(())
    }

    #[test]
    fn scope_kind_excludes_default_audit_ledger() {
        assert_eq!(scope_kind(DEFAULT_LEDGER_PATH, &[]), None);
    }

    #[test]
    fn build_report_excludes_untracked_files_from_deterministic_scope() -> Result<()> {
        let root = temp_git_repo("audit-scope-tracked-only")?;
        std::fs::create_dir_all(root.join("docs"))?;
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n")?;
        std::fs::write(root.join("docs/IMPLEMENTATION-MATRIX.toml"), "")?;
        std::fs::write(root.join("docs/tracked.md"), "tracked\n")?;
        git_commit_all(&root, "test: add tracked docs")?;
        std::fs::write(root.join("docs/untracked.md"), "untracked\n")?;

        let report = build_report(
            &root,
            &AuditScopeOptions {
                base: None,
                ledger: None,
                write_ledger: false,
                all: false,
                format: AuditScopeFormat::Json,
                outcome: "success".to_owned(),
            },
        )?;
        let candidate_paths: BTreeSet<&str> = report
            .candidates
            .iter()
            .map(|candidate| candidate.path.as_str())
            .collect();
        assert!(candidate_paths.contains("docs/tracked.md"));
        assert!(!candidate_paths.contains("docs/untracked.md"));
        assert!(
            !report
                .ledger_update
                .files
                .iter()
                .any(|file| file.path == "docs/untracked.md")
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn force_wide_review_reason_covers_explicit_path_mappings() {
        let cases = [
            ("AGENTS.md", "repository-agent-instructions"),
            ("CLAUDE.md", "repository-agent-instructions"),
            (
                ".github/copilot-instructions.md",
                "repository-agent-instructions",
            ),
            ("Cargo.toml", "workspace-membership"),
            ("docs/IMPLEMENTATION-MATRIX.toml", "implementation-matrix"),
            ("docs/SPEC-MAPPING.md", "spec-mapping"),
            ("docs/FEATURE-TRACKING.md", "feature-tracking"),
            (".codex/skills/splot-doc-audit/SKILL.md", "audit-skill"),
            (".claude/skills/splot-doc-audit/SKILL.md", "audit-skill"),
            (".github/skills/splot-doc-audit/SKILL.md", "audit-skill"),
            ("xtask/src/audit_scope.rs", "audit-scope-tooling"),
            ("docs/spec/av2/1.0.0/CHECKSUMS", "spec-mirror-integrity"),
            (
                "docs/spec/av2/1.0.0/provenance.toml",
                "spec-mirror-provenance",
            ),
        ];
        for (path, reason) in cases {
            assert_eq!(force_wide_review_reason(path).as_deref(), Some(reason));
        }
        assert_eq!(
            force_wide_review_reason("docs/spec/av2/1.0.0/05-syntax-structures.md"),
            None
        );
    }

    #[test]
    fn changed_paths_from_base_rejects_option_like_base() -> Result<()> {
        let Err(err) = changed_paths_from_base(Path::new("."), "--name-only") else {
            bail!("option-like base should be rejected before git runs");
        };
        assert!(
            err.to_string()
                .contains("audit-scope --base must not be empty or start with `-`")
        );
        Ok(())
    }

    #[test]
    fn ledger_output_is_deterministic() -> Result<()> {
        let files = vec![tracked("b.rs", "bbb"), tracked("a.rs", "aaa")];
        let mut sorted = files.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let ledger = build_ledger("commit", "success", &sorted);
        let first = serde_json::to_string_pretty(&ledger)?;
        let second = serde_json::to_string_pretty(&ledger)?;
        assert_eq!(first, second);
        assert!(first.find("\"a.rs\"") < first.find("\"b.rs\""));
        Ok(())
    }

    #[test]
    fn feature_ids_are_collected_from_matrix_module_and_text() -> Result<()> {
        let matrix = r#"
[[feature]]
id = "AV2-5.2.2-OBU-HEADER"
module = "crates/splot-core/src/obu.rs"

[[feature]]
id = "XTASK-AUDIT-SCOPE"
module = "xtask/src/audit_scope.rs"
"#;
        let index = feature_index_from_matrix_text(matrix)?;
        let ids = feature_ids_for_path("crates/splot-core/src/obu.rs", &index);
        assert!(ids.contains("AV2-5.2.2-OBU-HEADER"));
        let text_ids = feature_ids_in_text("covered by XTASK-AUDIT-SCOPE", &index.known_ids);
        assert!(text_ids.contains("XTASK-AUDIT-SCOPE"));
        Ok(())
    }

    fn tracked(path: &str, sha256: &str) -> TrackedAuditFile {
        TrackedAuditFile {
            path: path.to_owned(),
            sha256: sha256.to_owned(),
            scope_kind: "workspace:splot-core".to_owned(),
            feature_ids: Vec::new(),
            reviewer_lanes: Vec::new(),
        }
    }

    fn ledger_file(path: &str, sha256: &str) -> AuditLedgerFile {
        AuditLedgerFile {
            path: path.to_owned(),
            sha256: sha256.to_owned(),
            feature_ids: Vec::new(),
            outcome: "success".to_owned(),
        }
    }

    fn temp_root(name: &str) -> Result<PathBuf> {
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn temp_git_repo(name: &str) -> Result<PathBuf> {
        let root = temp_root(name)?;
        run_git(&root, &["init"])?;
        run_git(&root, &["config", "user.email", "splot@example.test"])?;
        run_git(&root, &["config", "user.name", "splot tests"])?;
        Ok(root)
    }

    fn git_commit_all(root: &Path, subject: &str) -> Result<()> {
        run_git(root, &["add", "-A"])?;
        run_git(root, &["commit", "-m", subject])?;
        Ok(())
    }
}
