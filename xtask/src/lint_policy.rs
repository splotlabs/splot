// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Workspace lint-policy check.
//!
//! The workspace enables `clippy::pedantic`, but a small set of pedantic lint
//! families is still allowed globally while the codec matures. This gate turns
//! that global allow-list into an explicit ratchet: existing exceptions may be
//! tightened or moved to narrower scopes, but adding a new workspace-level
//! `allow` requires updating this policy with a reviewed rationale.
//!
//! Feature tracking: `XTASK-LINT-POLICY`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

const FEATURE_ID: &str = "XTASK-LINT-POLICY";

/// Clippy lint groups that must stay enabled at workspace scope. These groups
/// need lower priority than individual lints so the intentional overrides below
/// can take effect without tripping Cargo's `lint_groups_priority` check.
const REQUIRED_CLIPPY_GROUPS: &[(&str, &str, i64)] =
    &[("all", "warn", -1), ("pedantic", "warn", -1)];

/// High-signal development failures that must stay denied at workspace scope.
const REQUIRED_CLIPPY_DENIES: &[&str] = &[
    "unwrap_used",
    "expect_used",
    "panic",
    "todo",
    "unimplemented",
    "dbg_macro",
];

/// The only Clippy lints currently approved as workspace-wide `allow`.
///
/// Tightening is always permitted by removing an entry from `Cargo.toml`; this
/// list only prevents new broad blind spots from being added silently.
const APPROVED_GLOBAL_CLIPPY_ALLOWS: &[&str] = &[
    // Cast-heavy bitstream and table transcription code. Future semantic code
    // should prefer named/checked conversion helpers over broad cast reliance.
    "cast_possible_truncation",
    "cast_possible_wrap",
    "cast_sign_loss",
    "cast_lossless",
    "cast_precision_loss",
    // Project prose uses AV2 section names and codec terms Clippy cannot parse.
    "doc_markdown",
    // Public API review debt: re-enable crate by crate or replace with a more
    // targeted public-API gate before treating it as correctness signal.
    "must_use_candidate",
    // Source-line length is governed by `check-source-lines`.
    "too_many_lines",
    // Style/noise families that are not correctness signals for this codebase.
    "similar_names",
    "struct_excessive_bools",
    "fn_params_excessive_bools",
    "wildcard_imports",
    "if_not_else",
];

/// Verifies the workspace lint policy.
///
/// # Errors
/// Returns an error if the root manifest cannot be read or parsed, or if the
/// workspace lint policy has drifted outside the approved ratchet.
pub(crate) fn check_lint_policy(root: &Path) -> Result<()> {
    let manifest_path = root.join("Cargo.toml");
    let manifest = crate::read_manifest(&manifest_path).with_context(|| {
        format!(
            "failed to load lint policy from {}",
            manifest_path.display()
        )
    })?;
    let violations = evaluate_lint_policy(&manifest);

    if violations.is_empty() {
        eprintln!("check-lint-policy: ok");
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("{violation}");
        }
        bail!("check-lint-policy: {} violation(s)", violations.len())
    }
}

fn evaluate_lint_policy(manifest: &toml::Table) -> Vec<String> {
    let Some(clippy) = workspace_clippy_lints(manifest) else {
        return vec!["workspace manifest has no [workspace.lints.clippy] table".to_owned()];
    };

    let mut violations = Vec::new();

    for (group, expected_level, expected_priority) in REQUIRED_CLIPPY_GROUPS {
        match clippy.get(*group) {
            Some(value) => {
                if lint_level(value) != Some(*expected_level) {
                    violations.push(format!(
                        "`clippy::{group}` must stay `{expected_level}` at workspace scope ({FEATURE_ID})"
                    ));
                }
                if lint_priority(value) != Some(*expected_priority) {
                    violations.push(format!(
                        "`clippy::{group}` must keep priority {expected_priority} so per-lint overrides are explicit ({FEATURE_ID})"
                    ));
                }
            }
            None => violations.push(format!(
                "`clippy::{group}` must be configured at workspace scope ({FEATURE_ID})"
            )),
        }
    }

    for lint in REQUIRED_CLIPPY_DENIES {
        if clippy.get(*lint).and_then(lint_level) != Some("deny") {
            violations.push(format!(
                "`clippy::{lint}` must stay `deny` at workspace scope ({FEATURE_ID})"
            ));
        }
    }

    let approved_allows: BTreeSet<&str> = APPROVED_GLOBAL_CLIPPY_ALLOWS.iter().copied().collect();
    for (lint, value) in clippy {
        if lint_level(value) == Some("allow") && !approved_allows.contains(lint.as_str()) {
            violations.push(format!(
                "`clippy::{lint}` is a workspace-level `allow` but is not in the approved global allow-list; move it to a narrower scope or update {FEATURE_ID} with rationale"
            ));
        }
    }

    violations.sort();
    violations
}

fn workspace_clippy_lints(manifest: &toml::Table) -> Option<&toml::Table> {
    manifest
        .get("workspace")?
        .get("lints")?
        .get("clippy")?
        .as_table()
}

fn lint_level(value: &toml::Value) -> Option<&str> {
    match value {
        toml::Value::String(level) => Some(level),
        toml::Value::Table(table) => table.get("level").and_then(toml::Value::as_str),
        _ => None,
    }
}

fn lint_priority(value: &toml::Value) -> Option<i64> {
    value
        .as_table()
        .and_then(|table| table.get("priority"))
        .and_then(toml::Value::as_integer)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const BASE_POLICY: &str = r#"
[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
"#;

    fn manifest(extra: &str) -> toml::Table {
        toml::from_str::<toml::Table>(&format!("{BASE_POLICY}{extra}")).expect("valid manifest")
    }

    #[test]
    fn approved_workspace_allows_pass() {
        let mut extra = String::new();
        for lint in APPROVED_GLOBAL_CLIPPY_ALLOWS {
            extra.push_str(lint);
            extra.push_str(" = \"allow\"\n");
        }
        assert!(evaluate_lint_policy(&manifest(&extra)).is_empty());
    }

    #[test]
    fn unknown_workspace_allow_is_rejected() {
        let violations = evaluate_lint_policy(&manifest("large_enum_variant = \"allow\"\n"));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("large_enum_variant")
                    && v.contains("approved global allow-list")),
            "got {violations:?}"
        );
    }

    #[test]
    fn removing_debt_allows_is_accepted() {
        assert!(evaluate_lint_policy(&manifest("")).is_empty());
    }

    #[test]
    fn required_deny_lints_are_enforced() {
        let mut root = manifest("");
        let clippy = root
            .get_mut("workspace")
            .unwrap()
            .get_mut("lints")
            .unwrap()
            .get_mut("clippy")
            .unwrap()
            .as_table_mut()
            .unwrap();
        clippy.insert(
            "unwrap_used".to_owned(),
            toml::Value::String("warn".to_owned()),
        );

        let violations = evaluate_lint_policy(&root);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("unwrap_used") && v.contains("deny")),
            "got {violations:?}"
        );
    }

    #[test]
    fn clippy_groups_must_keep_low_priority() {
        let mut root = manifest("");
        let clippy = root
            .get_mut("workspace")
            .unwrap()
            .get_mut("lints")
            .unwrap()
            .get_mut("clippy")
            .unwrap()
            .as_table_mut()
            .unwrap();
        clippy.insert(
            "pedantic".to_owned(),
            toml::Value::String("warn".to_owned()),
        );

        let violations = evaluate_lint_policy(&root);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("pedantic") && v.contains("priority")),
            "got {violations:?}"
        );
    }
}
