// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The diagnostic registry backing `splot explain <rule-id>`.
//!
//! A read-only catalog of every diagnostic the validator emits — `rule_id`,
//! `severity`, the AV2 spec section, and a one-line summary — keyed by rule id.
//! The backing table (`generated::REGISTRY`) is **generated** from the CI-enforced
//! `docs/DIAGNOSTICS.md` by `cargo xtask gen-explain`; nothing here is
//! hand-authored or invented, and `cargo xtask ci` fails if the generated file
//! drifts from the doc. This module only *reads* that catalog — it changes no
//! validator behavior and emits no diagnostics.

use serde::Serialize;

mod generated;

/// Catalog entry describing one validator diagnostic rule id. All fields are taken
/// verbatim from `docs/DIAGNOSTICS.md`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DiagnosticInfo {
    /// The stable rule id (e.g. `"obu-header/global-xlayer-required"`).
    pub rule_id: &'static str,
    /// The diagnostic's severity as the registry records it — `error`, `warning`,
    /// `info`, or a comma-separated dual value (e.g. `error, warning`) for a rule
    /// emitted at more than one severity.
    pub severity: &'static str,
    /// The registry's `Section` for the rule, verbatim (e.g. `"§ 6.2.2"`, `"§ A.4"`,
    /// or a non-AV2 label like `"IVF"` / `"varies"`), when one is recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<&'static str>,
    /// A one-line description of the condition the rule flags.
    pub summary: &'static str,
}

/// Every catalogued diagnostic, sorted by rule id.
#[must_use]
pub fn all() -> &'static [DiagnosticInfo] {
    generated::REGISTRY
}

/// Looks up a diagnostic by its exact rule id, or `None` if unknown.
#[must_use]
pub fn explain(rule_id: &str) -> Option<&'static DiagnosticInfo> {
    generated::REGISTRY
        .binary_search_by(|info| info.rule_id.cmp(rule_id))
        .ok()
        .map(|index| &generated::REGISTRY[index])
}

/// Returns up to a few catalogued rule ids "closest" to `rule_id` for an unknown-id
/// hint: ids sharing its namespace (text before the first `/`), else ids sharing a
/// leading character. Purely advisory; never panics.
#[must_use]
pub fn did_you_mean(rule_id: &str) -> Vec<&'static str> {
    let namespace = rule_id.split('/').next().unwrap_or(rule_id);
    let mut hits: Vec<&'static str> = generated::REGISTRY
        .iter()
        .map(|info| info.rule_id)
        .filter(|id| id.split('/').next() == Some(namespace))
        .collect();
    if hits.is_empty() {
        let lead = rule_id.chars().next();
        hits = generated::REGISTRY
            .iter()
            .map(|info| info.rule_id)
            .filter(|id| id.chars().next() == lead)
            .collect();
    }
    hits.truncate(8);
    hits
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_sorted_and_unique() {
        let ids: Vec<&str> = all().iter().map(|info| info.rule_id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            ids, sorted,
            "REGISTRY must be sorted by rule_id for binary search"
        );
        sorted.dedup();
        assert_eq!(
            ids.len(),
            sorted.len(),
            "REGISTRY must have unique rule ids"
        );
    }

    #[test]
    fn registry_has_a_substantial_catalog() {
        assert!(
            all().len() >= 200,
            "expected >= 200 entries, got {}",
            all().len()
        );
    }

    #[test]
    fn explain_known_and_unknown() {
        let info = explain("bitstream/parse-error").expect("a known diagnostic");
        assert_eq!(info.rule_id, "bitstream/parse-error");
        assert!(!info.summary.is_empty());
        assert!(explain("obu-header/this-id-does-not-exist").is_none());
    }

    #[test]
    fn did_you_mean_prefers_same_namespace() {
        let hits = did_you_mean("obu-header/does-not-exist");
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|id| id.starts_with("obu-header/")));
        assert!(hits.len() <= 8);
    }
}
