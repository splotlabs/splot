// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Offline validation for the decoder local-reference evidence manifest.
//!
//! The checker parses committed TOML metadata and committed fixture bytes only.
//! It does not locate, build, spawn, or require AVM, dav2d, ffmpeg, the network,
//! or `splot decode`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::git_util::sha256_hex;
use crate::util::{is_windows_absolute_path, tokenized};

pub(crate) const MANIFEST_PATH: &str = "docs/LOCAL-REFERENCE-EVIDENCE.toml";
pub(crate) const MANIFEST_POINTER_PREFIX: &str = "docs/LOCAL-REFERENCE-EVIDENCE.toml::";
const IMPLEMENTATION_MATRIX_PATH: &str = "docs/IMPLEMENTATION-MATRIX.toml";
const DECODER_SUPPORT_MATRIX_PATH: &str = "docs/DECODER-SUPPORT-MATRIX.toml";
const SUPPORTED_MANIFEST_VERSION: u32 = 1;

/// Validates `docs/LOCAL-REFERENCE-EVIDENCE.toml`.
pub(crate) fn run_check_reference_evidence(root: &Path) -> Result<()> {
    let manifest = load_manifest(root)?;
    validate_manifest(root, &manifest)?;
    eprintln!(
        "check-reference-evidence: ok ({} evidence entr{})",
        manifest.evidence.len(),
        if manifest.evidence.len() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ReferenceEvidenceIndex {
    evidence_count: usize,
    rows_by_evidence: BTreeMap<String, BTreeSet<String>>,
}

impl ReferenceEvidenceIndex {
    pub(crate) const fn evidence_count(&self) -> usize {
        self.evidence_count
    }

    pub(crate) fn rows_for(&self, evidence_id: &str) -> Option<&BTreeSet<String>> {
        self.rows_by_evidence.get(evidence_id)
    }
}

pub(crate) fn canonical_evidence_pointer_id(value: &str) -> Option<&str> {
    value.strip_prefix(MANIFEST_POINTER_PREFIX)
}

pub(crate) fn load_checked_reference_evidence_index(root: &Path) -> Result<ReferenceEvidenceIndex> {
    let manifest = load_manifest(root)?;
    validate_manifest(root, &manifest)?;
    Ok(ReferenceEvidenceIndex::from_manifest(&manifest))
}

impl ReferenceEvidenceIndex {
    fn from_manifest(manifest: &Manifest) -> Self {
        let rows_by_evidence = manifest
            .evidence
            .iter()
            .filter_map(|evidence| {
                let id = evidence.id.as_ref()?;
                let rows = evidence.decoder_support_rows.as_ref()?;
                Some((id.clone(), rows.iter().cloned().collect()))
            })
            .collect();
        Self {
            evidence_count: manifest.evidence.len(),
            rows_by_evidence,
        }
    }
}

fn load_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join(MANIFEST_PATH);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {MANIFEST_PATH}"))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {MANIFEST_PATH}"))
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(rename = "manifest_version")]
    version: Option<u32>,
    last_reviewed: Option<String>,
    #[serde(default)]
    evidence: Vec<Evidence>,
}

#[derive(Debug, Deserialize)]
struct Evidence {
    id: Option<String>,
    feature_id: Option<String>,
    decoder_support_rows: Option<Vec<String>>,
    kind: Option<String>,
    summary: Option<String>,
    recorded_at: Option<String>,
    #[serde(default)]
    fixture: Vec<Fixture>,
    #[serde(default)]
    reference_run: Vec<ReferenceRun>,
    #[serde(default)]
    assertion: Vec<Assertion>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: Option<String>,
    role: Option<String>,
    path: Option<String>,
    sha256: Option<String>,
    size_bytes: Option<u64>,
    format: Option<String>,
    provenance_kind: Option<String>,
    provenance_summary: Option<String>,
    license: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReferenceRun {
    id: Option<String>,
    tool: Option<String>,
    executable_name: Option<String>,
    tool_role: Option<String>,
    source_url: Option<String>,
    revision: Option<String>,
    version_summary: Option<String>,
    command_summary: Option<String>,
    output_digest_id: Option<String>,
    output_digest_algorithm: Option<String>,
    output_digest: Option<String>,
    output_scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Assertion {
    kind: Option<String>,
    left: Option<String>,
    right: Option<String>,
}

#[derive(Debug)]
struct DigestRecord {
    digest: String,
    run_id: String,
}

fn validate_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    let mut problems = Vec::new();

    match manifest.version {
        Some(SUPPORTED_MANIFEST_VERSION) => {}
        Some(other) => problems.push(format!(
            "unsupported manifest_version {other} (this tool supports {SUPPORTED_MANIFEST_VERSION})"
        )),
        None => problems.push("missing required field `manifest_version`".to_owned()),
    }

    if let Some(value) = manifest.last_reviewed.as_deref() {
        check_text(&mut problems, "last_reviewed", value);
    } else {
        problems.push("missing required field `last_reviewed`".to_owned());
    }

    let known_features = if manifest.evidence.is_empty() {
        BTreeSet::new()
    } else {
        load_feature_ids(root, &mut problems)
    };
    let known_rows = if manifest.evidence.is_empty() {
        BTreeSet::new()
    } else {
        load_decoder_support_rows(root, &mut problems)
    };

    let mut evidence_ids = BTreeSet::new();
    for (index, evidence) in manifest.evidence.iter().enumerate() {
        validate_evidence(
            root,
            &mut problems,
            &known_features,
            &known_rows,
            &mut evidence_ids,
            index,
            evidence,
        );
    }

    if problems.is_empty() {
        Ok(())
    } else {
        bail!(
            "local reference evidence manifest problem(s):\n- {}",
            problems.join("\n- ")
        );
    }
}

fn validate_evidence(
    root: &Path,
    problems: &mut Vec<String>,
    known_features: &BTreeSet<String>,
    known_rows: &BTreeSet<String>,
    evidence_ids: &mut BTreeSet<String>,
    index: usize,
    evidence: &Evidence,
) {
    let label = evidence
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map_or_else(
            || format!("evidence {}", index + 1),
            |id| format!("evidence `{id}`"),
        );

    let id = required_text(problems, &label, "id", evidence.id.as_deref());
    if let Some(id) = id {
        if !is_lower_kebab(id) {
            problems.push(format!("{label}: id `{id}` must be lowercase kebab-case"));
        }
        if !evidence_ids.insert(id.to_owned()) {
            problems.push(format!("{label}: duplicate evidence id `{id}`"));
        }
    }

    let feature_id = required_text(
        problems,
        &label,
        "feature_id",
        evidence.feature_id.as_deref(),
    );
    if let Some(feature_id) = feature_id
        && !known_features.contains(feature_id)
    {
        problems.push(format!("{label}: unknown feature_id `{feature_id}`"));
    }

    match evidence.decoder_support_rows.as_deref() {
        Some([]) => problems.push(format!(
            "{label}: decoder_support_rows must contain at least one row id"
        )),
        Some(rows) => {
            for row in rows {
                check_text(problems, &format!("{label}: decoder_support_rows"), row);
                if !known_rows.contains(row) {
                    problems.push(format!("{label}: unknown decoder support row `{row}`"));
                }
            }
        }
        None => problems.push(format!(
            "{label}: missing required field `decoder_support_rows`"
        )),
    }

    required_checked_text(problems, &label, "kind", evidence.kind.as_deref());
    required_checked_text(problems, &label, "summary", evidence.summary.as_deref());
    required_checked_text(
        problems,
        &label,
        "recorded_at",
        evidence.recorded_at.as_deref(),
    );

    if evidence.fixture.is_empty() {
        problems.push(format!("{label}: at least one `fixture` entry is required"));
    }
    if evidence.reference_run.is_empty() {
        problems.push(format!(
            "{label}: at least one `reference_run` entry is required"
        ));
    }
    if evidence.assertion.is_empty() {
        problems.push(format!(
            "{label}: at least one `assertion` entry is required"
        ));
    }

    let mut fixture_ids = BTreeSet::new();
    for (fixture_index, fixture) in evidence.fixture.iter().enumerate() {
        validate_fixture(
            root,
            problems,
            &label,
            &mut fixture_ids,
            fixture_index,
            fixture,
        );
    }

    let mut digests = BTreeMap::new();
    let mut run_ids = BTreeSet::new();
    for (run_index, run) in evidence.reference_run.iter().enumerate() {
        validate_reference_run(problems, &label, &mut digests, &mut run_ids, run_index, run);
    }

    for (assertion_index, assertion) in evidence.assertion.iter().enumerate() {
        validate_assertion(problems, &label, &digests, assertion_index, assertion);
    }
}

fn validate_fixture(
    root: &Path,
    problems: &mut Vec<String>,
    evidence_label: &str,
    fixture_ids: &mut BTreeSet<String>,
    index: usize,
    fixture: &Fixture,
) {
    let label = fixture
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map_or_else(
            || format!("{evidence_label}: fixture {}", index + 1),
            |id| format!("{evidence_label}: fixture `{id}`"),
        );
    let id = required_text(problems, &label, "id", fixture.id.as_deref());
    if let Some(id) = id {
        if !is_lower_kebab(id) {
            problems.push(format!("{label}: id `{id}` must be lowercase kebab-case"));
        }
        if !fixture_ids.insert(id.to_owned()) {
            problems.push(format!("{label}: duplicate fixture id `{id}`"));
        }
    }

    required_checked_text(problems, &label, "role", fixture.role.as_deref());
    required_checked_text(problems, &label, "format", fixture.format.as_deref());
    required_checked_text(
        problems,
        &label,
        "provenance_kind",
        fixture.provenance_kind.as_deref(),
    );
    required_checked_text(
        problems,
        &label,
        "provenance_summary",
        fixture.provenance_summary.as_deref(),
    );
    required_checked_text(problems, &label, "license", fixture.license.as_deref());

    let Some(path) = required_text(problems, &label, "path", fixture.path.as_deref()) else {
        return;
    };
    let Some(relpath) = validate_repo_relative_path(problems, &label, path) else {
        return;
    };
    let Some((fixture_path, meta)) = fixture_metadata(root, &relpath, problems, &label, path)
    else {
        return;
    };
    if !meta.is_file() {
        problems.push(format!(
            "{label}: fixture path `{path}` must be a regular file"
        ));
        return;
    }

    match fixture.size_bytes {
        Some(size) if size == meta.len() => {}
        Some(size) => problems.push(format!(
            "{label}: size_bytes {size} does not match actual byte length {}",
            meta.len()
        )),
        None => problems.push(format!("{label}: missing required field `size_bytes`")),
    }

    let Some(expected_sha) = required_text(problems, &label, "sha256", fixture.sha256.as_deref())
    else {
        return;
    };
    if !is_hex(expected_sha, 64) {
        problems.push(format!(
            "{label}: sha256 must be 64 lowercase hex characters"
        ));
        return;
    }
    match std::fs::read(&fixture_path) {
        Ok(bytes) => {
            let actual = sha256_hex(&bytes);
            if actual != expected_sha {
                problems.push(format!(
                    "{label}: sha256 `{expected_sha}` does not match actual `{actual}`"
                ));
            }
        }
        Err(err) => problems.push(format!("{label}: failed to read fixture `{path}`: {err}")),
    }
}

fn validate_reference_run(
    problems: &mut Vec<String>,
    evidence_label: &str,
    digests: &mut BTreeMap<String, DigestRecord>,
    run_ids: &mut BTreeSet<String>,
    index: usize,
    run: &ReferenceRun,
) {
    let label = run
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map_or_else(
            || format!("{evidence_label}: reference_run {}", index + 1),
            |id| format!("{evidence_label}: reference_run `{id}`"),
        );
    let id = required_text(problems, &label, "id", run.id.as_deref());
    if let Some(id) = id
        && !is_lower_kebab(id)
    {
        problems.push(format!("{label}: id `{id}` must be lowercase kebab-case"));
    }
    if let Some(id) = id
        && !run_ids.insert(id.to_owned())
    {
        problems.push(format!("{label}: duplicate reference_run id `{id}`"));
    }

    required_checked_text(problems, &label, "tool", run.tool.as_deref());
    required_checked_text(problems, &label, "tool_role", run.tool_role.as_deref());
    required_checked_text(problems, &label, "source_url", run.source_url.as_deref());
    required_checked_text(problems, &label, "revision", run.revision.as_deref());
    required_checked_text(
        problems,
        &label,
        "version_summary",
        run.version_summary.as_deref(),
    );
    required_checked_text(
        problems,
        &label,
        "output_scope",
        run.output_scope.as_deref(),
    );

    if let Some(name) = required_text(
        problems,
        &label,
        "executable_name",
        run.executable_name.as_deref(),
    ) {
        check_text(problems, &format!("{label}: executable_name"), name);
        if name.contains('/') || name.contains('\\') || name.starts_with('.') {
            problems.push(format!(
                "{label}: executable_name `{name}` must be a bare executable name, not a path"
            ));
        }
    }

    if let Some(summary) = required_text(
        problems,
        &label,
        "command_summary",
        run.command_summary.as_deref(),
    ) {
        check_text(problems, &format!("{label}: command_summary"), summary);
        if let Some(fragment) = relative_path_fragment(summary) {
            problems.push(format!(
                "{label}: command_summary contains executable or filesystem path fragment `{fragment}`"
            ));
        }
        if contains_shell_composition(summary) {
            problems.push(format!(
                "{label}: command_summary must be descriptive metadata, not shell composition"
            ));
        }
    }

    let digest_id = required_text(
        problems,
        &label,
        "output_digest_id",
        run.output_digest_id.as_deref(),
    );
    let digest_algorithm = required_text(
        problems,
        &label,
        "output_digest_algorithm",
        run.output_digest_algorithm.as_deref(),
    );
    let digest = required_text(
        problems,
        &label,
        "output_digest",
        run.output_digest.as_deref(),
    );
    if let Some(digest_id) = digest_id {
        if !is_lower_kebab(digest_id) {
            problems.push(format!(
                "{label}: output_digest_id `{digest_id}` must be lowercase kebab-case"
            ));
        }
        if let Some(digest) = digest
            && digests
                .insert(
                    digest_id.to_owned(),
                    DigestRecord {
                        digest: digest.to_owned(),
                        run_id: id.unwrap_or("").to_owned(),
                    },
                )
                .is_some()
        {
            problems.push(format!("{label}: duplicate output_digest_id `{digest_id}`"));
        }
    }
    if let (Some(algorithm), Some(digest)) = (digest_algorithm, digest) {
        let expected_len = match algorithm {
            "md5" => Some(32),
            "sha256" => Some(64),
            _ => {
                problems.push(format!(
                    "{label}: output_digest_algorithm `{algorithm}` is unsupported"
                ));
                None
            }
        };
        if let Some(expected_len) = expected_len
            && !is_hex(digest, expected_len)
        {
            problems.push(format!(
                "{label}: output_digest must be {expected_len} lowercase hex characters for {algorithm}"
            ));
        }
    }
}

fn validate_assertion(
    problems: &mut Vec<String>,
    evidence_label: &str,
    digests: &BTreeMap<String, DigestRecord>,
    index: usize,
    assertion: &Assertion,
) {
    let label = format!("{evidence_label}: assertion {}", index + 1);
    let Some(kind) = required_text(problems, &label, "kind", assertion.kind.as_deref()) else {
        return;
    };
    if kind != "digest-equality" {
        problems.push(format!("{label}: unsupported assertion kind `{kind}`"));
        return;
    }
    let left = required_text(problems, &label, "left", assertion.left.as_deref());
    let right = required_text(problems, &label, "right", assertion.right.as_deref());
    if let (Some(left), Some(right)) = (left, right) {
        if left == right {
            problems.push(format!(
                "{label}: digest-equality assertion must compare distinct digest ids"
            ));
        }
        let Some(left_digest) = digests.get(left) else {
            problems.push(format!("{label}: unknown left digest id `{left}`"));
            return;
        };
        let Some(right_digest) = digests.get(right) else {
            problems.push(format!("{label}: unknown right digest id `{right}`"));
            return;
        };
        if left_digest.run_id == right_digest.run_id {
            problems.push(format!(
                "{label}: digest-equality assertion must compare distinct reference runs"
            ));
        }
        if left_digest.digest != right_digest.digest {
            problems.push(format!(
                "{label}: digest-equality assertion compares unequal digests `{left}` and `{right}`"
            ));
        }
    }
}

fn load_feature_ids(root: &Path, problems: &mut Vec<String>) -> BTreeSet<String> {
    let path = root.join(IMPLEMENTATION_MATRIX_PATH);
    let Ok(text) = std::fs::read_to_string(&path) else {
        problems.push(format!("failed to read {IMPLEMENTATION_MATRIX_PATH}"));
        return BTreeSet::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        problems.push(format!("failed to parse {IMPLEMENTATION_MATRIX_PATH}"));
        return BTreeSet::new();
    };
    value
        .get("feature")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|feature| feature.get("id").and_then(toml::Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn load_decoder_support_rows(root: &Path, problems: &mut Vec<String>) -> BTreeSet<String> {
    let path = root.join(DECODER_SUPPORT_MATRIX_PATH);
    let Ok(text) = std::fs::read_to_string(&path) else {
        problems.push(format!("failed to read {DECODER_SUPPORT_MATRIX_PATH}"));
        return BTreeSet::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        problems.push(format!("failed to parse {DECODER_SUPPORT_MATRIX_PATH}"));
        return BTreeSet::new();
    };
    value
        .get("row")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(toml::Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn required_checked_text<'a>(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    value: Option<&'a str>,
) -> Option<&'a str> {
    let value = required_text(problems, label, field, value)?;
    check_text(problems, &format!("{label}: {field}"), value);
    Some(value)
}

fn required_text<'a>(
    problems: &mut Vec<String>,
    label: &str,
    field: &str,
    value: Option<&'a str>,
) -> Option<&'a str> {
    match value {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => {
            problems.push(format!("{label}: missing required field `{field}`"));
            None
        }
    }
}

fn validate_repo_relative_path(
    problems: &mut Vec<String>,
    label: &str,
    value: &str,
) -> Option<PathBuf> {
    check_text(problems, &format!("{label}: path"), value);
    if value.contains('\\') {
        problems.push(format!("{label}: path `{value}` must use `/` separators"));
        return None;
    }
    if value.contains("://") || looks_absolute_path(value) || contains_local_env_token(value) {
        problems.push(format!(
            "{label}: path `{value}` must be repo-relative and portable"
        ));
        return None;
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        problems.push(format!(
            "{label}: path `{value}` must not contain `.` or `..` components"
        ));
        return None;
    }
    Some(path.to_path_buf())
}

fn fixture_metadata(
    root: &Path,
    relpath: &Path,
    problems: &mut Vec<String>,
    label: &str,
    display_path: &str,
) -> Option<(PathBuf, Metadata)> {
    if git_metadata_exists(root) {
        match git_tracks_path(root, relpath) {
            Ok(true) => {}
            Ok(false) => {
                problems.push(format!(
                    "{label}: fixture path `{display_path}` must be tracked by git"
                ));
                return None;
            }
            Err(err) => {
                problems.push(format!(
                    "{label}: failed to query git for fixture path `{display_path}`: {err}"
                ));
                return None;
            }
        }
    }

    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(err) => {
            problems.push(format!(
                "{label}: failed to canonicalize repository root: {err}"
            ));
            return None;
        }
    };
    let mut current = root.clone();
    let mut final_meta = None;
    for component in relpath.components() {
        let Component::Normal(component) = component else {
            problems.push(format!(
                "{label}: fixture path `{display_path}` must stay repo-relative"
            ));
            return None;
        };
        current.push(component);
        let Ok(meta) = std::fs::symlink_metadata(&current) else {
            problems.push(format!(
                "{label}: fixture path `{display_path}` does not exist"
            ));
            return None;
        };
        if meta.file_type().is_symlink() {
            problems.push(format!(
                "{label}: fixture path `{display_path}` must not contain symlinks"
            ));
            return None;
        }
        final_meta = Some(meta);
    }
    let Ok(canonical_fixture) = current.canonicalize() else {
        problems.push(format!(
            "{label}: fixture path `{display_path}` does not exist"
        ));
        return None;
    };
    if !canonical_fixture.starts_with(&root) {
        problems.push(format!(
            "{label}: fixture path `{display_path}` escapes the repository root"
        ));
        return None;
    }
    final_meta.map(|meta| (current, meta))
}

fn git_metadata_exists(root: &Path) -> bool {
    root.join(".git").exists()
}

fn git_tracks_path(root: &Path, relpath: &Path) -> Result<bool> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(relpath)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to spawn git ls-files")?;
    Ok(status.success())
}

fn check_text(problems: &mut Vec<String>, label: &str, value: &str) {
    if let Some(fragment) = local_path_fragment(value) {
        problems.push(format!(
            "{label} contains non-portable local path fragment `{fragment}`"
        ));
    }
}

fn local_path_fragment(value: &str) -> Option<String> {
    path_fragments(value).into_iter().find(|fragment| {
        fragment.starts_with("file://")
            || looks_absolute_path(fragment)
            || contains_local_env_token(fragment)
    })
}

fn relative_path_fragment(value: &str) -> Option<String> {
    path_fragments(value).into_iter().find(|fragment| {
        !fragment.contains("://")
            && (fragment.starts_with("./")
                || fragment.starts_with("../")
                || fragment.contains('/')
                || fragment.contains('\\'))
    })
}

fn path_fragments(value: &str) -> Vec<String> {
    tokenized(value)
        .into_iter()
        .flat_map(|token| {
            let mut fragments = vec![token.clone()];
            fragments.extend(
                token
                    .split('=')
                    .skip(1)
                    .map(str::to_owned)
                    .filter(|fragment| !fragment.is_empty()),
            );
            if !token.contains("://") {
                fragments.extend(
                    token
                        .split(':')
                        .skip(1)
                        .map(str::to_owned)
                        .filter(|fragment| !fragment.is_empty()),
                );
            }
            fragments
        })
        .collect()
}

fn looks_absolute_path(token: &str) -> bool {
    if token.contains("://") && !token.starts_with("file://") {
        return false;
    }
    token.starts_with('/')
        || token.starts_with("~/")
        || token.starts_with("\\\\")
        || is_windows_absolute_path(token)
}

fn contains_local_env_token(value: &str) -> bool {
    ["$HOME", "${HOME}", "$PWD", "${PWD}", "%USERPROFILE%"]
        .iter()
        .any(|needle| value.contains(needle))
}

fn contains_shell_composition(value: &str) -> bool {
    value.contains('|')
        || value.contains("&&")
        || value.contains(';')
        || value.contains('`')
        || value.contains("$(")
        || value.contains('<')
        || value.contains('>')
}

fn is_lower_kebab(value: &str) -> bool {
    let mut previous_dash = false;
    !value.is_empty()
        && value.bytes().all(|byte| {
            let valid = byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
            let current_dash = byte == b'-';
            let ok = valid && !(previous_dash && current_dash);
            previous_dash = current_dash;
            ok
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
