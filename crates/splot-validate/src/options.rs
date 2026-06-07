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

/// Caller-declared external HLS objects (AV2 § 7.3.8): objects provided "through
/// external means" rather than in-band in the bitstream.
///
/// Only declared availability *keys* are modeled (not the object contents), which is
/// enough to resolve in-band-vs-external availability of a reference without
/// fabricating the external object's syntax.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalHlsSet {
    sequence_header_ids: BTreeSet<u32>,
}

impl ExternalHlsSet {
    /// Creates an empty external-HLS set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that a sequence header with `seq_header_id` is available externally
    /// (AV2 § 7.3.8.6).
    #[must_use]
    pub fn with_sequence_header_id(mut self, seq_header_id: u32) -> Self {
        self.sequence_header_ids.insert(seq_header_id);
        self
    }

    /// Returns `true` if a sequence header with `seq_header_id` was declared.
    #[must_use]
    pub(crate) fn has_sequence_header(&self, seq_header_id: u32) -> bool {
        self.sequence_header_ids.contains(&seq_header_id)
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
    /// Caller-declared external HLS objects, available in addition to in-band ones.
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
}
