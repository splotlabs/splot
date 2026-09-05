// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scan-type metadata consistency checks.

use super::*;

/// Table 6.18 output groups (AV2 § 6.16.10,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10`).
/// All defined picture-structure values in one CVS must belong to one group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PicStructGroup {
    /// `mps_pic_struct_type` 0, 7 or 8 — frame output. Table 6.18 requires
    /// "ci_scan_type_idc shall be equal to 1" (and, for values 7 and 8,
    /// "equal_picture_interval shall be equal to 1").
    Frame,
    /// `mps_pic_struct_type` 1, 2, 9, 10, 11 or 12 — single-field output.
    /// Table 6.18 requires "ci_scan_type_idc shall be equal to 2".
    SingleField,
    /// `mps_pic_struct_type` 3, 4, 5 or 6 — field-pair output. Table 6.18 requires
    /// "ci_scan_type_idc shall be equal to 3".
    FieldPair,
}

impl PicStructGroup {
    /// Classifies a `mps_pic_struct_type` value into its Table 6.18 group; `None`
    /// for the reserved values above 12, which are excluded from the group state
    /// ("Decoders shall ignore reserved values of mps_pic_struct_type",
    /// AV2 § 6.16.10; the stateless `metadata/scan-type-pic-struct-reserved` check
    /// reports the reserved value itself).
    pub(super) fn from_pic_struct(value: u8) -> Option<Self> {
        match value {
            0 | 7 | 8 => Some(Self::Frame),
            1 | 2 | 9..=12 => Some(Self::SingleField),
            3..=6 => Some(Self::FieldPair),
            _ => None,
        }
    }

    /// The `ci_scan_type_idc` value the Table 6.18 "Restrictions" column requires
    /// for this group (AV2 § 6.16.10): "ci_scan_type_idc shall be equal to" 1, 2,
    /// or 3 respectively.
    pub(super) fn required_ci_scan_type_idc(self) -> u8 {
        match self {
            Self::Frame => 1,
            Self::SingleField => 2,
            Self::FieldPair => 3,
        }
    }

    /// The group's `mps_pic_struct_type` values, worded as in the § 6.16.10
    /// conformance requirement, for diagnostic messages.
    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::Frame => "0, 7 or 8",
            Self::SingleField => "1, 2, 9, 10, 11 or 12",
            Self::FieldPair => "3, 4, 5 or 6",
        }
    }
}

/// One defined picture-structure observation in its § 6.16.10 CVS scope.
/// Metadata-time and changed-CI-time checks cover both arrival orders; identical
/// CI repeats do not re-report.
#[derive(Debug)]
pub(super) struct ScanTypeObservation {
    /// The observed `mps_pic_struct_type` (defined values 0..=12 only; reserved
    /// values never enter the state).
    pub(super) mps_pic_struct_type: u8,
    /// Source byte offset of the carrying metadata OBU.
    pub(super) offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of the observation, used by the
    /// exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions) and by the
    /// § 7.3.8.11 CI-parameter epoch checks.
    pub(super) tu_index: u64,
    /// CI identities already emitted against eagerly in this observation's own TU.
    /// RAP repair skips these pairs only; another CI may still need re-pairing after
    /// its stale pre-RAP comparison was deferred and dropped.
    pub(super) eagerly_emitted: BTreeSet<ContentInterpretationKey>,
}

/// Per-scope scan-type observations (AV2 § 6.16.10). Append-only within the coded
/// video sequence: the group requirement binds the values *present* "for all
/// pictures in the current CVS", so neither § 6.16.3 cancellation nor persistence
/// expiry removes an observation — only the § 7.3.6 CVS boundary does (see
/// [`ValidatorContext::flush_scan_type_scope`]).
#[derive(Debug, Default)]
pub(super) struct ScanTypeScope {
    pub(super) observations: Vec<ScanTypeObservation>,
}

impl ScanTypeScope {
    /// The scope's group baseline: its first (oldest surviving) observation and
    /// that observation's Table 6.18 group. Stored observations carry only defined
    /// values, so the classification always succeeds; it is still expressed as a
    /// filter to keep the path panic-free.
    pub(super) fn group_baseline(&self) -> Option<(&ScanTypeObservation, PicStructGroup)> {
        self.observations.first().and_then(|observation| {
            PicStructGroup::from_pic_struct(observation.mps_pic_struct_type)
                .map(|group| (observation, group))
        })
    }
}

/// Scan-type metadata CVS-consistency state (AV2 § 6.16.10), keyed by the carrying
/// OBU's `obu_xlayer_id`; [`GLOBAL_XLAYER_ID`] keys the global bucket. Global
/// scan-type metadata describes every layer's pictures, so the group rule compares
/// the global bucket against each concrete extended-layer scope and vice versa,
/// and the global bucket's Table 6.18 CI cross-checks consider every layer's
/// content-interpretation records.
#[derive(Debug, Default)]
pub(super) struct ScanTypeCvsState {
    pub(super) scopes: BTreeMap<ExtendedLayerId, ScanTypeScope>,
}

/// The pairing context of a § 6.16.10 Table 6.18 scan-type / content-interpretation
/// diagnostic: the content-interpretation side's layer ids, both OBUs' byte
/// offsets, and the byte offset to attach the diagnostic at (`at` — whichever OBU
/// completed the violating pair, the scan-type metadata OBU or the
/// content-interpretation OBU, whichever came second).
pub(super) struct ScanTypeCiPair {
    pub(super) ci_xlayer: ExtendedLayerId,
    pub(super) ci_mlayer: EmbeddedLayerId,
    pub(super) metadata_offset: ByteOffset,
    pub(super) ci_offset: ByteOffset,
    pub(super) at: ByteOffset,
}

/// The Table 6.18-decisive content of a content interpretation (AV2 § 6.16.10):
/// the established `ci_scan_type_idc` ("ci_scan_type_idc shall be equal to"
/// 1 / 2 / 3 per group) and whether a present `timing_info()` signals
/// `equal_picture_interval` 0 (the "equal_picture_interval shall be equal to 1"
/// half binding `mps_pic_struct_type` 7 / 8). Two content interpretations with
/// equal decisive content decide every Table 6.18 restriction identically.
pub(super) fn scan_type_decisive_content(content: &ContentInterpretation) -> (u8, bool) {
    (
        content.scan_type_idc.get(),
        content
            .timing_info
            .is_some_and(|timing| !timing.equal_picture_interval),
    )
}

/// Builds the § 6.16.10 Table 6.18 `ci_scan_type_idc` mismatch diagnostic
/// (`metadata/scan-type-ci-scan-type-mismatch`).
pub(super) fn scan_type_ci_mismatch_error(
    pic_struct: u8,
    required: u8,
    established: u8,
    pair: &ScanTypeCiPair,
) -> Diagnostic {
    Diagnostic::error(
        "metadata/scan-type-ci-scan-type-mismatch",
        format!(
            "mps_pic_struct_type {pic_struct} (scan-type metadata at byte {}) requires \
             ci_scan_type_idc equal to {required} per Table 6.18, but the content \
             interpretation for obu_xlayer_id {} / obu_mlayer_id {} (at byte {}) establishes \
             ci_scan_type_idc {established} within the coded video sequence",
            pair.metadata_offset,
            pair.ci_xlayer.get(),
            pair.ci_mlayer.get(),
            pair.ci_offset,
        ),
    )
    .with_spec_section("6.16.10")
    .with_byte_offset(pair.at)
}

/// Builds the § 6.16.10 Table 6.18 equal-picture-interval diagnostic
/// (`metadata/scan-type-equal-picture-interval-required`) for `mps_pic_struct_type`
/// 7 / 8 ("equal_picture_interval shall be equal to 1").
pub(super) fn scan_type_equal_picture_interval_error(
    pic_struct: u8,
    pair: &ScanTypeCiPair,
) -> Diagnostic {
    Diagnostic::error(
        "metadata/scan-type-equal-picture-interval-required",
        format!(
            "mps_pic_struct_type {pic_struct} (scan-type metadata at byte {}) requires \
             equal_picture_interval equal to 1 per Table 6.18, but the content interpretation \
             timing_info() for obu_xlayer_id {} / obu_mlayer_id {} (at byte {}) signals \
             equal_picture_interval 0",
            pair.metadata_offset,
            pair.ci_xlayer.get(),
            pair.ci_mlayer.get(),
            pair.ci_offset,
        ),
    )
    .with_spec_section("6.16.10")
    .with_byte_offset(pair.at)
}

impl ValidatorContext {
    /// Checks § 6.16.10 group consistency and Table 6.18 CI restrictions.
    /// Global metadata compares with every layer; concrete metadata with its own
    /// and global group baseline. Reserved values are ignored. CI scan type 0 is
    /// undecided, and absent timing does not establish an interval restriction.
    /// Pre-RAP CI records are excluded by the § 7.3.8.11 epoch. Earlier-TU baselines
    /// defer through CvsTracker to respect a later CLK in this TU.
    /// No rule relates mps_source_scan_type_idc to ci_scan_type_idc; do not compare it.
    pub(super) fn check_scan_type_consistency(
        &mut self,
        obu: &ObuEnvelope<'_>,
        scan: MetadataScanType,
        report: &mut ValidationReport,
    ) {
        let value = scan.mps_pic_struct_type;
        let Some(group) = PicStructGroup::from_pic_struct(value) else {
            return;
        };
        let scope_key = obu.header.extended_layer_id;
        let tu_index = self.cvs.tu_index;

        for (key, scope) in &self.scan_type.scopes {
            if !(*key == scope_key || key.is_global() || scope_key.is_global()) {
                continue;
            }
            let Some((baseline, baseline_group)) = scope.group_baseline() else {
                continue;
            };
            if baseline_group != group {
                let diagnostic = Diagnostic::error(
                    "metadata/scan-type-pic-struct-group-inconsistent",
                    format!(
                        "mps_pic_struct_type {value} falls into Table 6.18 group {{{}}} but \
                         mps_pic_struct_type {} (at byte {}) established group {{{}}}; only one \
                         group is allowed for all pictures in the coded video sequence",
                        group.describe(),
                        baseline.mps_pic_struct_type,
                        baseline.offset,
                        baseline_group.describe(),
                    ),
                )
                .with_spec_section("6.16.10")
                .with_byte_offset(obu.offset);
                self.cvs
                    .defer_or_emit(*key, baseline.tu_index, diagnostic, report);
            }
        }

        let required = group.required_ci_scan_type_idc();
        let mut eagerly_emitted = BTreeSet::new();
        for ((ci_xlayer, ci_mlayer), record) in &self.content_interpretations {
            if !(scope_key.is_global() || *ci_xlayer == scope_key) {
                continue;
            }
            if record.tu_index < self.ci_rap_epoch(*ci_xlayer) {
                continue;
            }
            let pair = ScanTypeCiPair {
                ci_xlayer: *ci_xlayer,
                ci_mlayer: *ci_mlayer,
                metadata_offset: obu.offset,
                ci_offset: record.offset,
                at: obu.offset,
            };
            let same_tu = record.tu_index == tu_index;
            let established = record.content.scan_type_idc.get();
            if established != 0 && established != required {
                let diagnostic = scan_type_ci_mismatch_error(value, required, established, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
                if same_tu {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
            }
            if matches!(value, 7 | 8)
                && let Some(timing) = record.content.timing_info
                && !timing.equal_picture_interval
            {
                let diagnostic = scan_type_equal_picture_interval_error(value, &pair);
                self.cvs
                    .defer_or_emit(*ci_xlayer, record.tu_index, diagnostic, report);
                if same_tu {
                    eagerly_emitted.insert((*ci_xlayer, *ci_mlayer));
                }
            }
        }

        self.scan_type
            .scopes
            .entry(scope_key)
            .or_default()
            .observations
            .push(ScanTypeObservation {
                mps_pic_struct_type: value,
                offset: obu.offset,
                tu_index,
                eagerly_emitted,
            });
    }

    /// Rechecks stored observations for changed Table 6.18-decisive CI content.
    /// The caller excludes identical repeats. Only observations in the CI's current
    /// § 7.3.8.11 epoch participate; global metadata also applies to concrete layers.
    /// RAP repair skips only pairs in eagerly_emitted, preserving other pairings
    /// whose stale pre-RAP diagnostics were deferred and dropped.
    pub(super) fn recheck_scan_type_after_ci(
        &mut self,
        ci_xlayer: ExtendedLayerId,
        ci_mlayer: EmbeddedLayerId,
        content: &ContentInterpretation,
        ci_offset: ByteOffset,
        repair: bool,
        report: &mut ValidationReport,
    ) {
        let (established, bad_interval) = scan_type_decisive_content(content);
        if established == 0 && !bad_interval {
            return;
        }
        let epoch = self.ci_rap_epoch(ci_xlayer);
        let scope_keys: &[ExtendedLayerId] = if ci_xlayer.is_global() {
            &[GLOBAL_XLAYER_ID]
        } else {
            &[ci_xlayer, GLOBAL_XLAYER_ID]
        };
        for &scope_key in scope_keys {
            let Some(scope) = self.scan_type.scopes.get(&scope_key) else {
                continue;
            };
            for observation in &scope.observations {
                if observation.tu_index < epoch {
                    continue;
                }
                if repair
                    && observation
                        .eagerly_emitted
                        .contains(&(ci_xlayer, ci_mlayer))
                {
                    continue;
                }
                let value = observation.mps_pic_struct_type;
                let Some(group) = PicStructGroup::from_pic_struct(value) else {
                    continue;
                };
                let pair = ScanTypeCiPair {
                    ci_xlayer,
                    ci_mlayer,
                    metadata_offset: observation.offset,
                    ci_offset,
                    at: ci_offset,
                };
                if established != 0 {
                    let required = group.required_ci_scan_type_idc();
                    if established != required {
                        let diagnostic =
                            scan_type_ci_mismatch_error(value, required, established, &pair);
                        self.cvs
                            .defer_or_emit(scope_key, observation.tu_index, diagnostic, report);
                    }
                }
                if matches!(value, 7 | 8) && bad_interval {
                    let diagnostic = scan_type_equal_picture_interval_error(value, &pair);
                    self.cvs
                        .defer_or_emit(scope_key, observation.tu_index, diagnostic, report);
                }
            }
        }
    }

    /// Whether an in-scope CI establishes nonzero scan type. The warning deliberately
    /// ignores RAP epochs: a pre-OLK record still suppresses it, a conservative
    /// false-negative approximation. Global scope considers all layers.
    pub(super) fn scan_type_ci_established(&self, scope_key: ExtendedLayerId) -> bool {
        self.content_interpretations
            .iter()
            .any(|((ci_xlayer, _), record)| {
                (scope_key.is_global() || *ci_xlayer == scope_key)
                    && record.content.scan_type_idc.get() != 0
            })
    }

    /// Retires observations before keep_from_tu (u64::MAX at EOF), warning once
    /// at the first retiring observation if no CI established nonzero scan type.
    /// This warning is derived from Table 6.18 (§ 6.16.10) versus default scan type 0
    /// (§ 7.3.8.11); the spec does not explicitly require a CI OBU, so it is no error.
    pub(super) fn flush_scan_type_scope(
        &mut self,
        scope_key: ExtendedLayerId,
        keep_from_tu: u64,
        report: &mut ValidationReport,
    ) {
        let established = self.scan_type_ci_established(scope_key);
        let Some(scope) = self.scan_type.scopes.get_mut(&scope_key) else {
            return;
        };
        if !established
            && let Some(first) = scope
                .observations
                .iter()
                .find(|observation| observation.tu_index < keep_from_tu)
        {
            report.push(
                Diagnostic::warning(
                    "metadata/scan-type-ci-scan-type-unestablished",
                    format!(
                        "scan-type metadata with mps_pic_struct_type {} (first at byte {}) was \
                         signaled, but no content interpretation in scope established a non-zero \
                         ci_scan_type_idc within the coded video sequence; the default is \
                         ci_scan_type_idc = 0 (unspecified) per AV2 § 7.3.8.11, which satisfies \
                         no Table 6.18 restriction — a diagnostic derived from a literal reading \
                         of Table 6.18",
                        first.mps_pic_struct_type, first.offset,
                    ),
                )
                .with_spec_section("6.16.10")
                .with_byte_offset(first.offset),
            );
        }
        scope
            .observations
            .retain(|observation| observation.tu_index >= keep_from_tu);
        if scope.observations.is_empty() {
            self.scan_type.scopes.remove(&scope_key);
        }
    }
}
