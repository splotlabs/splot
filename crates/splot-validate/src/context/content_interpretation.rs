// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Content-interpretation state and first-CELU presence checks.

use super::*;

/// Key identifying a content-interpretation record: `(obu_xlayer_id, obu_mlayer_id)`.
pub(super) type ContentInterpretationKey = (ExtendedLayerId, EmbeddedLayerId);

/// One observed content-interpretation OBU within its coded-video-sequence scope.
#[derive(Debug)]
pub(super) struct ContentInterpretationRecord {
    /// Parsed § 5.15 syntax, used for cross-embedded-layer timing consistency
    /// (AV2 § 6.4.12) and the repeated-CI "same information" check (AV2 § 6.14).
    pub(super) content: ContentInterpretation,
    /// Source byte offset of the OBU that produced this record.
    pub(super) offset: ByteOffset,
    /// Temporal unit ([`CvsTracker::tu_index`]) of this record's latest appearance,
    /// used by the exact § 7.3.6 CVS scoping (CLK pruning and deferral decisions).
    pub(super) tu_index: u64,
    /// Whether this record's latest appearance had its § 6.16.10 Table 6.18
    /// scan-type CI-time recheck SUPPRESSED by the epoch-aware identical-CI dedup
    /// guard (finding 1). A re-send whose scan-type-decisive content equalled the
    /// pre-RAP record's is suppressed at CI-time (the lagging epoch cannot tell it
    /// apart from an ordinary identical repeat); only such suppressed re-sends are
    /// re-paired by [`ValidatorContext::repair_post_rap_ci_pairings`] at the CLK/OLK.
    /// A re-send that CHANGED the decisive content already rechecked eagerly at
    /// CI-time, so re-pairing it would duplicate the diagnostic.
    pub(super) scan_type_recheck_suppressed: bool,
    /// The § 6.16.7 n_frames analogue of [`Self::scan_type_recheck_suppressed`]:
    /// whether this record's latest appearance had its timecode n_frames CI-time
    /// recheck suppressed by the epoch-aware identical-CI dedup guard (finding 1).
    pub(super) timecode_recheck_suppressed: bool,
}

/// Per extended layer, the § 7.3.6 first-CELU CI PRESENCE state (mirror lines 560-562,
/// `07-decoding-process.md#s-7-3-6`): "If an OBU_CONTENT_INTERPRETATION is present in any
/// coded extended layer unit, this OBU shall also be present in the first coded extended
/// layer unit of the sequence ... for a given embedded layer."
///
/// The "first coded extended layer unit of the sequence" for an extended layer is its CELU
/// in the coded video sequence's FIRST temporal unit (a CVS starts at the temporal unit
/// containing a CLK, § 7.3.6). This state records — scoped to the layer's CVS — the embedded
/// layers whose first CELU carried a CI, so a later CELU that adds a CI for an embedded layer
/// the first CELU lacked can be flagged. Reset per coded video sequence in
/// [`ValidatorContext::start_cvs_for_xlayer`].
///
/// **Unknown-first-CELU drop.** If the first CELU of the CVS was not observed — the stream
/// starts mid-CVS (no CLK seen for the layer, so the implicit CVS began before the first
/// observed OBU) — `first_celu_tu` is `None` and the presence judgment drops: the first
/// CELU's CI set is unknowable. An external-HLS `Provided` mode likewise drops the judgment
/// at the call site (an external CI in the first CELU cannot be enumerated by
/// [`crate::options::ExternalHlsSet`], which expresses only sequence headers and operating
/// point sets), consistent with the partial-declaration suppression policy.
#[derive(Debug, Default)]
pub(super) struct CiFirstCeluState {
    /// The temporal-unit index of the CVS's first temporal unit — the temporal unit whose
    /// CELU is the "first coded extended layer unit of the sequence" for this layer. `None`
    /// until a CLK establishes the CVS start for the layer (so a mid-CVS join, where no CLK
    /// has been observed, leaves it `None` and drops the judgment).
    pub(super) first_celu_tu: Option<u64>,
    /// The embedded layers (`obu_mlayer_id`) whose CI was observed in the first CELU of the
    /// CVS. A CI in a later CELU for an embedded layer absent from this set fires
    /// `celu/content-interpretation-not-in-first-celu`.
    pub(super) first_celu_ci_mlayers: BTreeSet<EmbeddedLayerId>,
    /// The embedded layers already reported, so the diagnostic dedups per
    /// `(xlayer, mlayer, CVS epoch)` — a repeated later CI for the same missing embedded
    /// layer fires once per coded video sequence.
    pub(super) reported: BTreeSet<EmbeddedLayerId>,
}

/// Compares two present `timing_info()` values from different embedded layers of
/// the same coded video sequence, returning a diagnostic per differing field
/// (AV2 § 6.4.12: these values, when present, shall be the same across all embedded
/// layers). `new` is located at `obu` (embedded layer `obu.header.embedded_layer_id`);
/// `existing` is the value previously seen for `existing_mlayer`. Both embedded-layer
/// ids are named in each message so the finding is self-contained. The caller routes
/// each diagnostic through [`CvsTracker::defer_or_emit`], since the comparison is
/// scoped to the coded video sequence (AV2 § 7.3.6).
pub(super) fn compare_timing_across_embedded_layers(
    existing_mlayer: EmbeddedLayerId,
    existing: &TimingInfo,
    new: &TimingInfo,
    obu: &ObuEnvelope<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let new_mlayer = obu.header.embedded_layer_id.get();
    let existing_mlayer = existing_mlayer.get();
    if existing.num_units_in_display_tick != new.num_units_in_display_tick {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-display-tick-mismatch",
            obu,
            format!(
                "num_units_in_display_tick {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) \
                 in the same coded video sequence",
                new.num_units_in_display_tick,
                new_mlayer,
                existing.num_units_in_display_tick,
                existing_mlayer
            ),
        ));
    }
    if existing.time_scale != new.time_scale {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-time-scale-mismatch",
            obu,
            format!(
                "time_scale {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in the same \
                 coded video sequence",
                new.time_scale, new_mlayer, existing.time_scale, existing_mlayer
            ),
        ));
    }
    if existing.equal_picture_interval != new.equal_picture_interval {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-equal-picture-interval-mismatch",
            obu,
            format!(
                "equal_picture_interval {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in \
                 the same coded video sequence",
                new.equal_picture_interval, new_mlayer, existing.equal_picture_interval, existing_mlayer
            ),
        ));
    }
    // num_ticks_per_picture_minus_1 is only present when equal_picture_interval is
    // set; compare it only when both layers carry it (AV2 § 6.4.12).
    if let (Some(existing_ticks), Some(new_ticks)) = (
        existing.num_ticks_per_picture_minus_1,
        new.num_ticks_per_picture_minus_1,
    ) && existing_ticks != new_ticks
    {
        diagnostics.push(timing_mismatch_error(
            "sequence-header/timing-num-ticks-mismatch",
            obu,
            format!(
                "num_ticks_per_picture_minus_1 {new_ticks} (obu_mlayer_id {new_mlayer}) differs \
                 from {existing_ticks} (obu_mlayer_id {existing_mlayer}) in the same coded video \
                 sequence"
            ),
        ));
    }
    diagnostics
}

/// Returns `true` if two content-interpretation OBUs carry different *information*
/// (AV2 § 6.14: a repeated CI OBU must "contain the same information").
///
/// `ci_reserved_2bit` is excluded — it is decoder-ignored (§ 6.14) and surfaced
/// separately as a warning. The color description and aspect ratio are compared by
/// their *derived* values (§ 6.14 Table 6.13 / the § 5.15 aspect tables), resolving
/// presets, reserved ids, and absence to their canonical (incl. unspecified)
/// values: an alias-equivalent re-encoding (a preset vs. the equivalent explicit
/// triple / SAR, or a reserved id vs. an explicit unspecified one) is not flagged,
/// while genuinely different color/aspect information is — including a present value
/// vs. an absent (unspecified) one. The aspect ratio is compared only when both
/// derived SARs are decidable; a reserved `ci_aspect_ratio_idc` (already an
/// out-of-range error) yields no derived SAR and is not double-reported here.
pub(super) fn content_interpretation_information_differs(
    a: &ContentInterpretation,
    b: &ContentInterpretation,
) -> bool {
    a.scan_type_idc != b.scan_type_idc
        || a.chroma_sample_position != b.chroma_sample_position
        || a.timing_info != b.timing_info
        || a.derived_color() != b.derived_color()
        || aspect_ratio_information_differs(a, b)
}

/// Compares the derived sample aspect ratios (§ 5.15), resolving absence to the
/// unspecified `(0, 0)`. Only flags when both SARs are decidable; a reserved
/// `ci_aspect_ratio_idc` yields no derived SAR (it is already an out-of-range error).
pub(super) fn aspect_ratio_information_differs(
    a: &ContentInterpretation,
    b: &ContentInterpretation,
) -> bool {
    match (
        a.derived_sample_aspect_ratio(),
        b.derived_sample_aspect_ratio(),
    ) {
        (Some(sar_a), Some(sar_b)) => sar_a != sar_b,
        _ => false,
    }
}

/// Builds a § 6.4.12 cross-embedded-layer timing-mismatch diagnostic located at `obu`.
pub(super) fn timing_mismatch_error(
    rule_id: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("6.4.12")
        .with_byte_offset(obu.offset)
}

impl ValidatorContext {
    /// Records a § 7.3.8.11 random access point (CLK or OLK) for `xlayer` at the
    /// current temporal unit: "The content interpretation parameters for each
    /// embedded layer in an extended layer are initialized to default values ...
    /// at each random access point of the extended layer (i.e., at each temporal
    /// unit containing an OBU in the extended layer with obu_type equal to
    /// OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY)". The § 6.16.10 Table 6.18
    /// scan-type / CI pairing epoch starts here. The epoch starts AT the temporal
    /// unit, so same-temporal-unit records and observations belong to the new
    /// epoch (a CI OBU in the random access point's own temporal unit
    /// re-establishes the parameters, § 7.3.8.11 step 2) — matching the
    /// `tu_index >= epoch` retention convention of the CVS stores. Pending
    /// deferred § 6.16.10 Table 6.18 pairing diagnostics and the § 6.16.7
    /// n_frames-bound pairing for the extended layer pair pre-epoch CI content
    /// (`ci_scan_type_idc` / `equal_picture_interval` / `ci_timing_info_present_flag`)
    /// with post-epoch pictures (or vice versa), so exactly those three rules are
    /// dropped; every other pending diagnostic (§ 6.14 repeated-CI identity,
    /// § 6.4.12 timing, group consistency) is CVS-scoped and survives an OLK.
    pub(super) fn observe_ci_rap(&mut self, xlayer: ExtendedLayerId) {
        self.ci_rap_started_in_tu.insert(xlayer, self.cvs.tu_index);
        self.cvs.drop_pending_for_rules(
            xlayer,
            &[
                "metadata/scan-type-ci-scan-type-mismatch",
                "metadata/scan-type-equal-picture-interval-required",
                // § 6.16.7 / § 7.3.8.11: the n_frames bound is established by a CI's
                // ci_timing_info_present_flag, which a random access point reinitializes
                // to 0 (finding 5). A deferred n_frames pairing against a prior-TU CI
                // pairs that pre-epoch timing with post-epoch pictures, so this reinit
                // invalidates it exactly as it does the scan-type pairings above.
                "metadata/timecode-n-frames-exceeds-rate",
            ],
        );
    }

    /// The temporal unit at which `xlayer`'s current § 7.3.8.11
    /// content-interpretation-parameter epoch started (its most recent CLK / OLK
    /// random access point), or 0 when none has been observed.
    pub(super) fn ci_rap_epoch(&self, xlayer: ExtendedLayerId) -> u64 {
        self.ci_rap_started_in_tu.get(&xlayer).copied().unwrap_or(0)
    }

    /// Re-pairs the § 6.16.7 n_frames bound and the § 6.16.10 Table 6.18 scan-type
    /// restrictions of the new coded video sequence's observations against the content
    /// interpretation OBUs re-sent IDENTICALLY in this CLK's temporal unit (finding 1,
    /// the CLK side of the epoch-aware dedup).
    ///
    /// A content interpretation re-sent in a § 7.3.8.11 random-access-point temporal
    /// unit re-establishes the parameters for the new coded video sequence (§ 7.3.8.11
    /// step 2). When it repeats the pre-RAP content **identically** the epoch-aware dedup
    /// ([`Self::observe_content_interpretation`]) skipped its CI-time recheck — at
    /// CI-time the epoch had not advanced past the still-present pre-RAP record, so the
    /// re-sent CI could not be told apart from an ordinary identical repeat. By the time
    /// the CLK runs, [`Self::observe_ci_rap`] has advanced the epoch to this temporal
    /// unit and dropped the stale pre-RAP pairings. Re-running the suppressed rechecks
    /// now pairs the new epoch's observations (`tu_index >= epoch`, i.e. this temporal
    /// unit's metadata, since the epoch filter inside the rechecks excludes the dropped
    /// previous-epoch observations) against the re-sent CI exactly once — the
    /// authoritative pairing, with no duplicate because the pre-RAP pairing was dropped
    /// rather than reported.
    ///
    /// The re-pair is filtered to the CIs whose CI-time recheck the dedup guard actually
    /// SUPPRESSED — i.e. an identical re-send of the pre-RAP record (finding 1). A CI
    /// re-sent in this RAP temporal unit with a CHANGED (different) decisive content
    /// defeats the dedup guard and rechecked EAGERLY at CI-time, already reporting any
    /// violation; re-pairing it here too would duplicate the diagnostic, so the
    /// per-recheck `*_recheck_suppressed` flags exclude it. The scan-type and timecode
    /// suppressions are filtered independently, since a re-send can change one decisive
    /// content while leaving the other identical.
    ///
    /// Only the content interpretations re-sent IN this temporal unit (at/after the
    /// epoch) for the CLK's extended layer (or a global-keyed CI, which describes every
    /// layer) drive the re-pair; a CI from an earlier temporal unit belongs to the
    /// ending coded video sequence and is excluded by the epoch.
    pub(super) fn repair_post_rap_ci_pairings(
        &mut self,
        clk_xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        let epoch = self.ci_rap_epoch(clk_xlayer);
        // Idempotent within a temporal unit: a malformed temporal unit with two CLK/OLK
        // random access points for the SAME extended layer calls this twice. The second
        // observe_ci_rap leaves the epoch at this temporal unit and drops nothing new, so
        // a second re-pair would replay the same post-epoch CI snapshot and duplicate
        // every repaired diagnostic. Run once per (extended layer, temporal unit).
        let tu_index = self.cvs.tu_index;
        if self.repaired_post_rap_in_tu.get(&clk_xlayer) == Some(&tu_index) {
            return;
        }
        self.repaired_post_rap_in_tu.insert(clk_xlayer, tu_index);
        // Snapshot the re-sent (post-epoch) CI records for this extended layer to avoid
        // holding the content_interpretations borrow across the rechecks (which mutate
        // the deferral state). ContentInterpretation is Copy, so this is cheap. The
        // two suppression flags select which recheck to replay (finding 1): only a
        // recheck the dedup guard skipped at CI-time is re-paired here, so a re-send
        // that changed the content (already rechecked eagerly) is not re-reported.
        let resent: Vec<(
            ExtendedLayerId,
            EmbeddedLayerId,
            ContentInterpretation,
            ByteOffset,
            bool,
            bool,
        )> = self
            .content_interpretations
            .iter()
            .filter(|((ci_xlayer, _), record)| {
                (*ci_xlayer == clk_xlayer || ci_xlayer.is_global()) && record.tu_index >= epoch
            })
            .map(|((ci_xlayer, ci_mlayer), record)| {
                (
                    *ci_xlayer,
                    *ci_mlayer,
                    record.content,
                    record.offset,
                    record.scan_type_recheck_suppressed,
                    record.timecode_recheck_suppressed,
                )
            })
            .collect();
        for (ci_xlayer, ci_mlayer, content, ci_offset, scan_suppressed, timecode_suppressed) in
            resent
        {
            if scan_suppressed {
                // Re-pair (`repair = true`): a `(observation, this CI)` pair already
                // paired-and-emitted eagerly against an in-scope same-RAP-TU CI (one
                // re-sent BEFORE the observation) is skipped to avoid duplicating the
                // diagnostic (the scan-type analogue of the round-7 timecode finding 2).
                // The skip is per-CI, so a pair whose eager pairing was deferred against
                // the stale pre-RAP CI — dropped by observe_ci_rap at the RAP — is
                // re-paired, even when the SAME observation already emitted eagerly
                // against a DIFFERENT CI.
                self.recheck_scan_type_after_ci(
                    ci_xlayer, ci_mlayer, &content, ci_offset, true, report,
                );
            }
            if timecode_suppressed {
                // Re-pair (`repair = true`): a `(observation, this CI)` pair already
                // paired-and-emitted eagerly against an in-scope same-RAP-TU CI (one
                // re-sent BEFORE the observation) is skipped to avoid duplicating the
                // diagnostic (round-7 finding 2). The skip is per-CI, so a pair whose
                // eager pairing was deferred against the stale pre-RAP CI — dropped by
                // observe_ci_rap at the RAP — is re-paired, even when the SAME observation
                // already emitted eagerly against a DIFFERENT CI.
                self.recheck_timecode_n_frames_after_ci(
                    ci_xlayer, ci_mlayer, &content, ci_offset, true, report,
                );
            }
        }
    }

    /// Observes a content-interpretation OBU: checks cross-embedded-layer timing
    /// consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14) within the
    /// coded video sequence of the OBU's extended layer (exact § 7.3.6 boundaries:
    /// a CLK boundary event drops earlier-temporal-unit records, and a comparison
    /// against an earlier temporal unit's record is deferred to the temporal-unit
    /// flush; see [`CvsTracker`]).
    ///
    /// Timing values are compared only between two present `timing_info()` values of
    /// different embedded layers within the same extended layer — exactly the
    /// § 6.4.12 "within a coded video sequence ... across all embedded layers"
    /// scope, since a coded video sequence belongs to one extended layer (AV2 § 2).
    ///
    /// Also re-evaluates the stored § 6.16.10 scan-type observations against the
    /// new record (see [`ValidatorContext::recheck_scan_type_after_ci`]).
    pub(super) fn observe_content_interpretation(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures are reported by the stateless ContentInterpretationSyntax
        // check; here we only act on a successful parse.
        let Ok(content_interpretation) = parse_content_interpretation(&mut reader) else {
            return;
        };

        let xlayer = obu.header.extended_layer_id;
        let mlayer = obu.header.embedded_layer_id;
        let tu_index = self.cvs.tu_index;

        // AV2 § 7.3.6 (mirror lines 560-562): record this CI's `(xlayer, mlayer)` presence in
        // the current temporal unit for the first-CELU-of-the-sequence presence judgment,
        // resolved at the temporal-unit boundary (round-6 F3). A CI belongs to a coded
        // extended layer unit, which is per non-global extended layer (§ 7.3.6); a global CI
        // is not part of any CELU, so it is excluded. The first appearance's offset anchors
        // the diagnostic.
        if !xlayer.is_global() {
            self.ci_observed_in_tu
                .entry((xlayer, mlayer))
                .or_insert(obu.offset);
        }

        // Cross-embedded-layer timing consistency: compare this layer's timing
        // against the first other embedded layer (same extended layer) that already
        // carries present timing within this CVS scope.
        if let Some(new_timing) = content_interpretation.timing_info
            && let Some((existing_mlayer, existing_timing, existing_tu)) = self
                .content_interpretations
                .iter()
                .find(|((x, m), record)| {
                    *x == xlayer && *m != mlayer && record.content.timing_info.is_some()
                })
                .and_then(|((_, m), record)| {
                    record.content.timing_info.map(|t| (*m, t, record.tu_index))
                })
        {
            for diagnostic in compare_timing_across_embedded_layers(
                existing_mlayer,
                &existing_timing,
                &new_timing,
                obu,
            ) {
                self.cvs
                    .defer_or_emit(xlayer, existing_tu, diagnostic, report);
            }
        }

        // A content interpretation can arrive after the scan-type metadata whose
        // Table 6.18 restrictions it decides (AV2 § 6.16.10); re-evaluate the stored
        // observations of this scope and the global bucket — unless an existing record
        // at the same (xlayer, mlayer) key already carries identical Table 6.18-decisive
        // content AND is still the post-epoch authority for it. In that case every
        // stored observation has already been paired against that content (at
        // metadata-observation time by check_scan_type_consistency, or by the recheck
        // that ran when the record's decisive content last changed), so re-evaluating
        // would only duplicate reports for the identical repeats § 6.14 explicitly
        // allows. A content interpretation for a NEW key, or one whose decisive content
        // changed (itself flagged by content-interpretation/repeated-ci-not-identical
        // below), forms genuinely new (observation, CI-content) pairs and is
        // re-evaluated.
        //
        // EPOCH-AWARE dedup (finding 1, unified model). The predicate is
        // content-identical AND no § 7.3.8.11 random-access-point reinit occurred
        // *after* the existing record's temporal unit (`existing.tu_index >=
        // ci_rap_epoch`). Two cases the prior temporal-unit-identity predicate got
        // wrong:
        //
        //   - An ORDINARY identical repeat in a LATER temporal unit with no intervening
        //     RAP: the existing record is still the post-epoch authority that already
        //     paired (and reported) the observations, so re-running the recheck would
        //     re-report them. `existing.tu_index >= ci_rap_epoch` holds (no RAP since),
        //     so this dedups — the over-correction the temporal-unit-only predicate
        //     caused is gone.
        //   - A RAP temporal unit re-sends an identical CI BEFORE its CLK: the pre-RAP
        //     record is stale (its observations belong to the ending epoch). At CI-time
        //     the epoch has not advanced yet (the CLK follows), so this predicate ALSO
        //     dedups here — but the re-pair of the new epoch's observations is then done
        //     at the CLK, once observe_ci_rap has advanced the epoch and dropped the
        //     stale deferred pairings (see repair_post_rap_ci_pairings). That keeps the
        //     RAP-resent re-pair correct without an eager re-pair that the lagging epoch
        //     cannot soundly authorize.
        let decisive_content_unchanged = self
            .content_interpretations
            .get(&(xlayer, mlayer))
            .is_some_and(|existing| {
                existing.tu_index >= self.ci_rap_epoch(xlayer)
                    && scan_type_decisive_content(&existing.content)
                        == scan_type_decisive_content(&content_interpretation)
            });
        if !decisive_content_unchanged {
            // Eager CI-after-metadata re-pair (`repair = false`): the eager-emission skip
            // (the scan-type analogue of the round-7 timecode finding 2) applies only to
            // the RAP re-pair.
            self.recheck_scan_type_after_ci(
                xlayer,
                mlayer,
                &content_interpretation,
                obu.offset,
                false,
                report,
            );
        }
        // Finding 1 (CLK re-pair filter): record whether the scan-type recheck was
        // SUPPRESSED here. Only a suppressed re-send (a pre-RAP-identical copy whose
        // recheck the lagging epoch skipped) is re-paired at the CLK/OLK by
        // repair_post_rap_ci_pairings; a re-send that CHANGED the decisive content
        // rechecked eagerly just above, so re-pairing it would duplicate the diagnostic.
        let scan_type_recheck_suppressed = decisive_content_unchanged;

        // AV2 § 6.16.7: a content interpretation establishing ci_timing_info_present_flag
        // / timing may arrive after the timecode metadata whose n_frames bound it
        // decides; re-evaluate the stored timecode observations — but only when the
        // n_frames-decisive content (the timing_info) differs from the record this CI
        // replaces OR the existing record is no longer the post-epoch authority,
        // mirroring the scan-type dedup so a repeated identical CI (the only legal
        // repeat, § 6.14) never re-reports.
        //
        // EPOCH-AWARE dedup (finding 1, unified model — identical to the scan-type guard
        // above). `existing.tu_index >= ci_rap_epoch` means no § 7.3.8.11 random access
        // point reinitialized the parameters after the existing record's temporal unit,
        // so the existing record is still the authority that already paired (and
        // reported) every observation against this timing: re-running the recheck would
        // duplicate those reports for an ordinary identical repeat in a later temporal
        // unit (the over-correction the temporal-unit-only predicate caused). When a RAP
        // re-sends an identical CI BEFORE its CLK the epoch has not advanced yet, so this
        // dedups at CI-time too; the new epoch's observations are re-paired at the CLK
        // (see repair_post_rap_ci_pairings) after observe_ci_rap advances the epoch and
        // drops the stale deferred pairings.
        let timing_unchanged = self
            .content_interpretations
            .get(&(xlayer, mlayer))
            .is_some_and(|existing| {
                existing.tu_index >= self.ci_rap_epoch(xlayer)
                    && existing.content.timing_info == content_interpretation.timing_info
            });
        if !timing_unchanged {
            // Eager CI-after-timecode re-pair (`repair = false`): the round-7 finding 2
            // eager-emission skip applies only to the RAP re-pair below.
            self.recheck_timecode_n_frames_after_ci(
                xlayer,
                mlayer,
                &content_interpretation,
                obu.offset,
                false,
                report,
            );
        }
        // Finding 1 (CLK re-pair filter, the n_frames analogue of
        // `scan_type_recheck_suppressed`): a re-send whose timing CHANGED rechecked
        // eagerly just above, so repair_post_rap_ci_pairings must NOT re-pair it (that
        // would duplicate the diagnostic); only a suppressed identical re-send is
        // re-paired at the CLK/OLK.
        let timecode_recheck_suppressed = timing_unchanged;

        match self.content_interpretations.entry((xlayer, mlayer)) {
            Entry::Vacant(slot) => {
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
                    scan_type_recheck_suppressed,
                    timecode_recheck_suppressed,
                });
            }
            Entry::Occupied(mut slot) => {
                let existing = slot.get();
                // AV2 § 6.14: a repeated CI OBU for the same embedded layer within a
                // CVS must carry the same *information* (a weaker requirement than the
                // sequence header's bit-identity in § 7.3.6). Each compared field
                // resolves to a canonical value (incl. unspecified defaults for absent
                // color/aspect), so any observation is a complete baseline; the
                // decoder-ignored ci_reserved_2bit is excluded.
                if content_interpretation_information_differs(
                    &existing.content,
                    &content_interpretation,
                ) {
                    let diagnostic = Diagnostic::error(
                        "content-interpretation/repeated-ci-not-identical",
                        format!(
                            "content interpretation OBU for obu_xlayer_id {} / obu_mlayer_id {} \
                             is repeated within the coded video sequence with different \
                             information (previous copy at byte {})",
                            xlayer.get(),
                            mlayer.get(),
                            existing.offset
                        ),
                    )
                    .with_spec_section("6.14")
                    .with_byte_offset(obu.offset);
                    self.cvs
                        .defer_or_emit(xlayer, existing.tu_index, diagnostic, report);
                }
                // Refresh the record to this latest appearance: § 7.3.6 starts a new
                // coded video sequence *at the temporal unit*, so a copy re-sent in a
                // CLK temporal unit must survive the CLK pruning as the new coded
                // video sequence's baseline (a differing repeat also becomes the new
                // baseline after being routed above). The suppression flags carry
                // whether THIS appearance's rechecks were skipped by the dedup guard
                // (finding 1), so the CLK/OLK re-pair touches only an identical re-send.
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                    tu_index,
                    scan_type_recheck_suppressed,
                    timecode_recheck_suppressed,
                });
            }
        }
    }

    /// Resolves the § 7.3.6 first-CELU-of-the-sequence CI PRESENCE judgment (mirror lines
    /// 560-562, round-6 F3) for the just-completed temporal unit `completed_tu_index`. Called
    /// at each global-temporal-delimiter boundary and at the end of the bitstream, after the
    /// CLK boundary events of the temporal unit have been applied (so the CVS the temporal
    /// unit belongs to is final — the whole temporal unit containing a CLK belongs to the new
    /// coded video sequence, § 7.3.6). Drains [`Self::ci_observed_in_tu`].
    ///
    /// For each `(xlayer, mlayer)` CI observed in the temporal unit:
    ///
    /// - Under an external-HLS `Provided` mode the judgment DROPS: an external CI in the first
    ///   CELU cannot be enumerated by [`crate::options::ExternalHlsSet`] (it expresses only
    ///   sequence headers and operating point sets), so the validator cannot prove the first
    ///   CELU lacked the CI — consistent with the partial-declaration suppression policy.
    /// - If the layer's coded video sequence start was not observed (`first_celu_tu` is `None`
    ///   — a mid-CVS join, no CLK seen) the judgment DROPS: the first CELU's CI set is
    ///   unknowable (documented Unknown-first-CELU drop, see [`CiFirstCeluState`]).
    /// - If this temporal unit IS the CVS's first temporal unit (`completed_tu_index ==
    ///   first_celu_tu`), the CI is in the first CELU — record `mlayer` as present there.
    /// - Otherwise the CI is in a LATER CELU: if `mlayer` was absent from the first CELU's CI
    ///   set (and not already reported this CVS), fire `celu/content-interpretation-not-in-
    ///   first-celu`, anchored at the offending CI, and dedup per `(xlayer, mlayer, CVS epoch)`.
    pub(super) fn resolve_ci_first_celu_for_tu(
        &mut self,
        completed_tu_index: u64,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let observed = std::mem::take(&mut self.ci_observed_in_tu);
        // External HLS (any Provided mode): an external CI the set cannot enumerate may be the
        // first CELU's CI, so the presence judgment is not decidable — drop wholesale. The
        // per-TU buffer is still drained above so it does not leak into the next temporal unit.
        if matches!(options.external_hls, ExternalHlsMode::Provided(_)) {
            return;
        }
        for ((xlayer, mlayer), offset) in observed {
            let state = self.ci_first_celu.entry(xlayer).or_default();
            let Some(first_celu_tu) = state.first_celu_tu else {
                // No CLK established the CVS start for this layer (mid-CVS join): the first
                // coded extended layer unit of the sequence was not observed, so the presence
                // judgment is undecidable — drop (documented Unknown-first-CELU drop).
                continue;
            };
            if completed_tu_index == first_celu_tu {
                // This temporal unit is the CVS's first temporal unit, so this CI is in the
                // first coded extended layer unit of the sequence — record the embedded layer.
                state.first_celu_ci_mlayers.insert(mlayer);
            } else if !state.first_celu_ci_mlayers.contains(&mlayer)
                && state.reported.insert(mlayer)
            {
                // A later coded extended layer unit carries a CI for an embedded layer the
                // sequence's first CELU lacked (§ 7.3.6 lines 560-562). Dedup per
                // (xlayer, mlayer, CVS epoch) via `reported`.
                report.push(
                    Diagnostic::error(
                        "celu/content-interpretation-not-in-first-celu",
                        format!(
                            "OBU_CONTENT_INTERPRETATION is present for obu_xlayer_id {} / \
                             obu_mlayer_id {} in a coded extended layer unit that is not the \
                             first coded extended layer unit of the coded video sequence, but \
                             the first coded extended layer unit of the sequence carried no \
                             content interpretation for that embedded layer; § 7.3.6 requires a \
                             CI present in any coded extended layer unit to also be present in \
                             the first coded extended layer unit of the sequence",
                            xlayer.get(),
                            mlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(offset),
                );
            }
        }
    }
}
