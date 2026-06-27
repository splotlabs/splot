// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scan-type metadata consistency checks.

use super::*;

/// Table 6.18 picture-output group of a defined `mps_pic_struct_type` value
/// (AV2 § 6.16.10, `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10`).
///
/// The three groups mirror the § 6.16.10 bitstream-conformance requirement: "It is
/// a requirement of bitstream conformance that when mps_pic_struct_type is present
/// that only one of the following conditions, for all pictures in the current CVS,
/// is true: – The value of mps_pic_struct_type is equal to 0, 7 or 8. – The value
/// of mps_pic_struct_type is equal to 1, 2, 9, 10, 11 or 12. – The value of
/// mps_pic_struct_type is equal to 3, 4, 5 or 6."
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

/// One defined-`mps_pic_struct_type` scan-type metadata observation within its
/// coded-video-sequence scope (AV2 § 6.16.10).
///
/// The Table 6.18 CI cross-checks pair each observation with each in-scope
/// content-interpretation record exactly once per distinct decisive CI content:
/// the metadata-time pass ([`ValidatorContext::check_scan_type_consistency`])
/// pairs a new observation against every record already in scope, and the
/// CI-time pass ([`ValidatorContext::recheck_scan_type_after_ci`]) runs only
/// when the new CI's Table 6.18-decisive content differs from the record it
/// replaces — so a repeated identical CI (the only legal repeat, § 6.14) never
/// re-reports, while a CI for a new embedded layer or with changed content is
/// evaluated against every stored observation.
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
    /// The content-interpretation identities `(obu_xlayer_id, obu_mlayer_id)` whose
    /// Table 6.18 restriction this observation already paired-and-emitted *eagerly*
    /// against, in its OWN temporal unit, at observation time (the scan-type analogue of
    /// the round-7 timecode finding 2). A CI key lands here when, at
    /// [`ValidatorContext::check_scan_type_consistency`], that already-recorded in-scope
    /// CI in this temporal unit decided a Table 6.18 restriction and the diagnostic was
    /// emitted (not deferred) — i.e. an identical CI was re-sent BEFORE the scan-type
    /// metadata in the same § 7.3.8.11 RAP temporal unit. The § 7.3.8.11 RAP re-pair
    /// ([`ValidatorContext::repair_post_rap_ci_pairings`]) skips only the
    /// `(observation, CI)` *pairs* recorded here, not the whole observation: a multi-layer
    /// stream can pair one observation with several CIs in opposite orderings relative to
    /// the metadata, so an eager emission against one CI must not suppress the re-pair of
    /// a different CI whose eager pairing was DEFERRED against a stale pre-RAP record (and
    /// dropped at the RAP). The set is empty for an observation that emitted nothing
    /// eagerly, and re-pairing covers every not-yet-emitted post-epoch pairing.
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
    /// Folds one non-cancel `metadata_scan_type()` unit into the § 6.16.10 CVS
    /// consistency state and runs the Table 6.18 checks (AV2 § 6.16.10,
    /// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10`):
    ///
    /// - **Group consistency**: "It is a requirement of bitstream conformance that
    ///   when mps_pic_struct_type is present that only one of the following
    ///   conditions, for all pictures in the current CVS, is true" (the three
    ///   [`PicStructGroup`] value sets). The new value's group is compared against
    ///   each in-scope group baseline (the scope's first observation in the coded
    ///   video sequence); a global-bucket unit is also compared against every
    ///   concrete extended-layer scope and vice versa, since global metadata
    ///   describes every layer's pictures.
    /// - **CI cross-check**: the Table 6.18 "Restrictions" column
    ///   ("ci_scan_type_idc shall be equal to" 1 / 2 / 3 per group) against every
    ///   in-scope content-interpretation record with an established non-zero
    ///   `ci_scan_type_idc`; an established value of 0 is Unspecified and decides
    ///   nothing (the scope-level absence is the
    ///   `metadata/scan-type-ci-scan-type-unestablished` warning at the CVS flush
    ///   instead). For values 7 and 8 additionally "equal_picture_interval shall
    ///   be equal to 1", checked against records carrying `timing_info()`; a
    ///   record without `timing_info()` is silently skipped for this half — the
    ///   mirror attaches the restriction to the signaled element and states no
    ///   absent-timing rule. A record from before its extended layer's most
    ///   recent random access point is skipped: § 7.3.8.11 re-initializes the
    ///   content interpretation parameters to defaults at each CLK / OLK temporal
    ///   unit, so a pre-epoch record no longer establishes the parameters this
    ///   picture sees (a record re-sent at or after the random access point
    ///   refreshes its temporal unit and re-enters pairing).
    ///
    /// Reserved values above 12 never enter the state ("Decoders shall ignore
    /// reserved values of mps_pic_struct_type", § 6.16.10). Comparisons against a
    /// baseline from an earlier temporal unit are routed through
    /// [`CvsTracker::defer_or_emit`], tagged with the baseline's owning scope, so
    /// the exact § 7.3.6 CVS boundary applies.
    ///
    /// `mps_source_scan_type_idc` is deliberately NOT cross-checked against
    /// `ci_scan_type_idc`: the mirror's complete semantics are
    /// "mps_source_scan_type_idc specifies the scan type with the same semantics
    /// as for ci_scan_type_idc" (§ 6.16.10) — no consistency requirement exists.
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

        // Group consistency against the unit's own scope plus the paired
        // global / concrete scopes.
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

        // Table 6.18 CI cross-check against the in-scope content-interpretation
        // records already observed (a CI arriving later re-evaluates instead; see
        // recheck_scan_type_after_ci). Each record applies its own extended
        // layer's § 7.3.8.11 epoch (for the global bucket too — an epoch only
        // resets the CI parameters of its own extended layer).
        //
        // `eagerly_emitted` collects the CI identities whose same-temporal-unit in-scope
        // Table 6.18 restriction was decided HERE and emitted (not deferred) — i.e. an
        // identical CI was re-sent BEFORE this scan-type metadata in the same § 7.3.8.11
        // RAP temporal unit. The RAP re-pair (repair_post_rap_ci_pairings) skips exactly
        // those `(observation, CI)` pairs so the diagnostic is not emitted twice (the
        // scan-type analogue of the round-7 timecode finding 2), while still re-pairing
        // any OTHER CI for this observation. A pairing DEFERRED against an
        // earlier-temporal-unit (stale pre-RAP) CI does NOT enter the set: that deferred
        // diagnostic is dropped at the RAP, so the re-pair must still cover it.
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
            // defer_or_emit emits eagerly iff the CI is in this temporal unit; a
            // same-temporal-unit emission is the case to skip in the RAP re-pair, keyed
            // by the CI's identity so only this exact pairing is skipped.
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

        // Push the observation after the loop so `eagerly_emitted` is final, tagged with
        // whether its Table 6.18 restriction was already emitted eagerly above (the
        // scan-type analogue of the round-7 timecode finding 2).
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

    /// Re-evaluates the § 6.16.10 Table 6.18 restrictions of the stored scan-type
    /// observations against a newly observed content-interpretation record — the
    /// CI may arrive after the scan-type metadata it constrains. The caller
    /// ([`ValidatorContext::observe_content_interpretation`]) invokes this only
    /// when the CI's Table 6.18-decisive content differs from the record it
    /// replaces, so a repeated identical CI never re-reports while every
    /// genuinely new (observation, CI-content) pair is evaluated exactly once
    /// (see [`ScanTypeObservation`]). Observations from a temporal unit before
    /// the CI extended layer's most recent random access point are skipped —
    /// their pictures' content interpretation parameters belong to the previous
    /// § 7.3.8.11 epoch, the same epoch mismatch in the other direction as the
    /// pre-epoch-record skip in
    /// [`ValidatorContext::check_scan_type_consistency`]. The CI's own
    /// extended-layer scope and the global bucket are re-evaluated (global
    /// scan-type metadata describes every layer's pictures); the baseline of
    /// each comparison is the metadata observation, so
    /// [`CvsTracker::defer_or_emit`] routes on its temporal unit.
    ///
    /// `repair` flags the call as the § 7.3.8.11 RAP re-pair from
    /// [`Self::repair_post_rap_ci_pairings`] (the scan-type analogue of the round-7
    /// timecode finding 2). The eager CI-after-metadata caller passes `false`; the RAP
    /// re-pair passes `true`, which skips an `(observation, CI)` pair that already
    /// paired-and-emitted eagerly against this in-scope same-temporal-unit CI at
    /// observation time (the [`ScanTypeObservation::eagerly_emitted`] set contains the
    /// CI's identity — populated when an identical CI was already recorded BEFORE the
    /// observation in the same RAP temporal unit, so the eager observation-time pairing
    /// emitted directly). Re-pairing such a pair would duplicate the diagnostic; the skip
    /// is per-CI, so a DIFFERENT CI for the same observation — whose eager pairing was
    /// instead DEFERRED against a stale pre-RAP CI (and dropped by `observe_ci_rap` at the
    /// RAP) — still gets re-paired.
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
                // The RAP re-pair additionally skips a `(observation, CI)` pair already
                // paired-and-emitted eagerly at observation time (the scan-type analogue
                // of the round-7 timecode finding 2). The skip is keyed by THIS CI's
                // identity, so an eager emission against a different CI does not suppress
                // re-pairing this one (the multi-layer opposite-ordering case).
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

    /// Returns whether any in-scope content-interpretation record established a
    /// non-zero `ci_scan_type_idc` for `scope_key`: a concrete extended layer
    /// matches its own records, the global bucket matches every record (global
    /// scan-type metadata describes every layer's pictures).
    ///
    /// The § 7.3.8.11 random-access epoch is deliberately NOT applied here: a
    /// pre-OLK record keeps suppressing the
    /// `metadata/scan-type-ci-scan-type-unestablished` warning after an OLK
    /// re-initializes the parameters to `ci_scan_type_idc` 0 — a documented
    /// lenient false-negative approximation in the conservative direction for a
    /// warning-severity diagnostic derived from a literal Table 6.18 reading
    /// (tightening it would make the derived warning fire more often).
    pub(super) fn scan_type_ci_established(&self, scope_key: ExtendedLayerId) -> bool {
        self.content_interpretations
            .iter()
            .any(|((ci_xlayer, _), record)| {
                (scope_key.is_global() || *ci_xlayer == scope_key)
                    && record.content.scan_type_idc.get() != 0
            })
    }

    /// Ends the coded video sequence of `scope_key`'s scan-type scope: emits the
    /// `metadata/scan-type-ci-scan-type-unestablished` warning when observations
    /// are being retired and no in-scope content-interpretation record established
    /// a non-zero `ci_scan_type_idc`, then drops observations with
    /// `tu_index < keep_from_tu` (pass `u64::MAX` to retire the whole scope at the
    /// end of the bitstream). One warning per scope, citing the first retiring
    /// observation.
    ///
    /// The warning is a **derived** diagnostic from a literal reading of
    /// Table 6.18 (AV2 § 6.16.10): every defined `mps_pic_struct_type` row
    /// restricts `ci_scan_type_idc` to 1, 2 or 3, while the default content
    /// interpretation parameter — in effect when no content interpretation OBU
    /// establishes one — is "ci_scan_type_idc = 0 (unspecified)" (AV2 § 7.3.8.11),
    /// which satisfies no row. The mirror states no explicit
    /// presence requirement for the content interpretation OBU, so this is a
    /// warning, never an error.
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
