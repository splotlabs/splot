// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Validation options, including caller-supplied external high-level-syntax
//! availability (AV2 v1.0.0 § 7.3.8).
//!
//! AV2 § 7.3.8.1 requires each referenced HLS OBU to be available "by inclusion in
//! the bitstream or by provision through external means". The validator models
//! in-band availability automatically; external availability must be declared
//! explicitly by the caller, so the default assumes none.

use std::collections::BTreeSet;

use splot_core::headers::sequence::MAX_SEQ_NUM;

/// Caller-declared external HLS objects (AV2 § 7.3.8): objects provided "through
/// external means" rather than in-band in the bitstream.
///
/// Only declared availability *keys* are modeled (not the object contents), which is
/// enough to resolve in-band-vs-external availability of a reference without
/// fabricating the external object's syntax.
///
/// **Partial-declaration semantics (important for validator suppression).** This set
/// enumerates only the external HLS object kinds the caller can describe — sequence
/// headers ([`Self::with_sequence_header_id`]) and operating point sets
/// ([`Self::with_operating_point_set`]). It is **not** an exhaustive declaration of all
/// external HLS: other external HLS OBU kinds the API cannot express — notably layer
/// configuration records (LCRs) — MAY still exist externally even when the set does not
/// (and cannot) enumerate them. The set is "what the caller knows", not "the complete
/// external HLS". See [`ExternalHlsMode::Provided`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalHlsSet {
    sequence_header_ids: BTreeSet<u32>,
    /// `(obu_xlayer_id, ops_id)` of operating point sets declared available externally
    /// (AV2 § 7.3.8.5).
    operating_point_sets: BTreeSet<(u8, u8)>,
}

impl ExternalHlsSet {
    /// Creates an empty external-HLS set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that a sequence header with `seq_header_id` is available externally
    /// (AV2 § 7.3.8.6).
    ///
    /// An out-of-range id (`>= MAX_SEQ_NUM`) cannot be a valid `seq_header_id`
    /// (AV2 § 6.4.1), so it is ignored — declaring one must not make the set act as
    /// though a valid external sequence header were available.
    #[must_use]
    pub fn with_sequence_header_id(mut self, seq_header_id: u32) -> Self {
        if seq_header_id < MAX_SEQ_NUM {
            self.sequence_header_ids.insert(seq_header_id);
        }
        self
    }

    /// Returns `true` if a sequence header with `seq_header_id` was declared.
    #[must_use]
    pub(crate) fn has_sequence_header(&self, seq_header_id: u32) -> bool {
        self.sequence_header_ids.contains(&seq_header_id)
    }

    /// Returns `true` if any sequence header at all was declared externally. Used to
    /// decide whether an externally-provided sequence header could be the active one
    /// for an extended layer (AV2 § 7.3.8.1).
    #[must_use]
    pub(crate) fn declares_any_sequence_header(&self) -> bool {
        !self.sequence_header_ids.is_empty()
    }

    /// Declares that an operating point set with `ops_id` for `obu_xlayer_id` is
    /// available externally (AV2 § 7.3.8.5).
    ///
    /// An out-of-range key (`obu_xlayer_id > 31` or `ops_id > 15`) cannot identify a
    /// valid OPS (`obu_xlayer_id` is `f(5)`, `ops_id` is `f(4)`), so it is ignored.
    #[must_use]
    pub fn with_operating_point_set(mut self, obu_xlayer_id: u8, ops_id: u8) -> Self {
        if obu_xlayer_id <= 31 && ops_id <= 15 {
            self.operating_point_sets.insert((obu_xlayer_id, ops_id));
        }
        self
    }

    /// Returns `true` if an operating point set with `ops_id` for `obu_xlayer_id` was
    /// declared available externally (AV2 § 7.3.8.5).
    #[must_use]
    pub(crate) fn has_operating_point_set(&self, obu_xlayer_id: u8, ops_id: u8) -> bool {
        self.operating_point_sets.contains(&(obu_xlayer_id, ops_id))
    }
}

/// Whether caller-provided external HLS is available during validation
/// (AV2 § 7.3.8). Defaults to [`ExternalHlsMode::Disabled`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExternalHlsMode {
    /// No external HLS: only objects included in the bitstream are available. This
    /// is the default — the validator never assumes external availability unless the
    /// caller supplies it.
    #[default]
    Disabled,
    /// External HLS exists, available in addition to in-band ones. The carried
    /// [`ExternalHlsSet`] is a **partial** declaration: it enumerates only the object
    /// kinds the caller can describe (sequence headers and operating point sets), so
    /// other external HLS OBUs the set cannot express — notably layer configuration
    /// records (LCRs) — MAY exist externally without being listed. The set is "what the
    /// caller knows", not "the complete external HLS".
    ///
    /// This drives validator suppression policy (zero-false-positive principle,
    /// AGENTS.md § 7): because an unenumerated external *local* LCR could win the
    /// local-first § 6.4.1 `seq_lcr_id` resolution ahead of an in-band record, *any*
    /// Provided mode suppresses the association-dependent § 6.4.1 / § 6.8.5 / § 6.8.8 /
    /// § 6.8.9 LCR checks — even an empty or OPS-only set, since the suppression is about
    /// possibly-unenumerated external LCRs, not about declared sequence headers. Checks
    /// that read no LCR association (e.g. the Annex A header value-space checks) stay
    /// active under Provided, as they remain locally decidable from the in-band header
    /// alone.
    Provided(ExternalHlsSet),
}

/// Options controlling a validation run.
///
/// The default disables external HLS, so [`crate::Validator::validate_bytes`] (which
/// uses the default) is unchanged. Use
/// [`crate::Validator::validate_bytes_with_options`] to supply non-default options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationOptions {
    /// Caller-provided external HLS availability (AV2 § 7.3.8).
    pub external_hls: ExternalHlsMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_does_not_assume_external_hls() {
        let options = ValidationOptions::default();
        assert_eq!(options.external_hls, ExternalHlsMode::Disabled);
    }

    #[test]
    fn external_hls_set_records_declared_ids() {
        let set = ExternalHlsSet::new()
            .with_sequence_header_id(3)
            .with_sequence_header_id(7);
        assert!(set.has_sequence_header(3));
        assert!(set.has_sequence_header(7));
        assert!(!set.has_sequence_header(5));
    }

    #[test]
    fn external_hls_set_ignores_out_of_range_ids() {
        // MAX_SEQ_NUM (16) and above cannot be a valid seq_header_id, so declaring
        // one is ignored and does not make the set non-empty.
        let set = ExternalHlsSet::new().with_sequence_header_id(MAX_SEQ_NUM);
        assert!(!set.has_sequence_header(MAX_SEQ_NUM));
        assert!(!set.declares_any_sequence_header());
    }

    #[test]
    fn external_hls_set_records_operating_point_sets() {
        let set = ExternalHlsSet::new()
            .with_operating_point_set(31, 5)
            .with_operating_point_set(2, 0);
        assert!(set.has_operating_point_set(31, 5));
        assert!(set.has_operating_point_set(2, 0));
        // A different xlayer or ops_id, and out-of-range keys, are not present.
        assert!(!set.has_operating_point_set(31, 4));
        assert!(!set.has_operating_point_set(3, 5));
        let out_of_range = ExternalHlsSet::new().with_operating_point_set(32, 16);
        assert!(!out_of_range.has_operating_point_set(32, 16));
    }
}
