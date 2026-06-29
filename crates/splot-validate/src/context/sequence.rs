// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sequence-header observation, activation, and per-CVS sequence state.

use super::*;

/// Per-extended-layer distinct-`obu_mlayer_id` accumulator for the § 6.4.1
/// `SeqMaxMlayerCnt` count (AV2 v1.0.0 § 6.4.1).
///
/// § 6.4.1 (mirror `06-syntax-structures-semantics.md` lines 445-447) requires "the
/// number of distinct values of obu_mlayer_id present in the coded video sequence
/// associated with this sequence header is less than or equal to SeqMaxMlayerCnt", and
/// the § 6.4.1 NOTE (lines 450-452) adds that "the counting applies to all OBUs, even
/// if they are not layer-specific".
///
/// **Attribution (design decision 4, conservative under-approximation).** The count is
/// scoped to *one extended layer's* coded video sequence (§ 7.3.6), but a global
/// (`GLOBAL_XLAYER_ID`) OBU belongs to no single extended layer's CVS, so attributing
/// its `obu_mlayer_id` to a specific extended layer is ambiguous. Only OBUs whose
/// `obu_xlayer_id` names a concrete extended layer are counted under that layer; the
/// `obu_mlayer_id` of a global OBU is left uncounted. This can only *under*-count, so a
/// reported exceedance is always real. The forced-`obu_mlayer_id == 0` of a per-layer
/// `OBU_SEQUENCE_HEADER` (§ 6.2.2) is itself a concrete-`obu_xlayer_id` OBU and is
/// counted, matching the NOTE's "a sequence containing only embedded layer 1 will count
/// as two layers" example.
/// TODO(spec: AV2-6.4-SEQUENCE-HEADER-SEMANTICS): attribute global-`obu_xlayer_id`
/// OBUs (e.g. a global LCR) to the per-extended-layer CVS counts once the § 6.2.2 /
/// § 7.3.6 global-OBU-to-CVS association is modeled.
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerTracker {
    /// For each extended layer, the distinct `obu_mlayer_id` values counted in its
    /// current coded video sequence, and whether the § 6.4.1 exceedance has already
    /// been reported for that coded video sequence (emit once per CVS).
    pub(super) per_xlayer: BTreeMap<ExtendedLayerId, DistinctMlayerState>,
    /// For each extended layer, the distinct `obu_mlayer_id` values observed so far in
    /// the *current temporal unit* (cleared at each temporal-unit advance, analogous to
    /// [`ValidatorContext::frames_seen_in_tu`]). § 7.3.6 (mirror
    /// `07-decoding-process.md` lines 604-606) starts a new coded video sequence AT the
    /// temporal unit containing the CLK, so every id of that temporal unit — including a
    /// § 7.3.8.1 resent-at-RAP sequence header observed before the CLK — belongs to the
    /// NEW coded video sequence. [`Self::reset_cvs`] re-seeds the new coded video
    /// sequence from this per-temporal-unit set so those pre-CLK ids are attributed to
    /// the new CVS (exact re-attribution, not the former whole-state drop).
    pub(super) per_xlayer_tu: BTreeMap<ExtendedLayerId, BTreeSet<EmbeddedLayerId>>,
}

/// One extended layer's distinct-`obu_mlayer_id` count state within its current coded
/// video sequence; see [`DistinctMlayerTracker`].
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerState {
    /// The distinct `obu_mlayer_id` values seen so far in this coded video sequence.
    pub(super) seen: BTreeSet<EmbeddedLayerId>,
    /// The temporal unit in which the first `obu_mlayer_id` of this coded video sequence
    /// was counted, or `None` until the first count. The exceedance baseline for
    /// [`CvsTracker::defer_or_emit`]: when the whole accumulated set was first observed in
    /// the temporal unit of the OBU that triggers the exceedance, all members share one
    /// coded video sequence regardless of a later same-temporal-unit CLK (§ 7.3.6: a CVS
    /// starts *at* a temporal unit, so same-TU OBUs join the same new CVS), and the
    /// diagnostic is emitted eagerly; when the set spans an earlier temporal unit, a CLK
    /// later in the current temporal unit could split it across two coded video sequences,
    /// so the diagnostic is deferred and dropped by [`CvsTracker::flush_completed_tu`] if
    /// such a CLK arrives.
    pub(super) first_tu: Option<u64>,
    /// `true` once `sequence-state/distinct-mlayer-count-exceeds-seq-max` has been
    /// emitted for this coded video sequence (the check emits once per CVS).
    pub(super) reported: bool,
}

impl DistinctMlayerTracker {
    /// Resets `xlayer`'s distinct-`obu_mlayer_id` count at a § 7.3.6 coded-video-sequence
    /// start (CLK), *re-attributing* the same-temporal-unit ids observed before the CLK
    /// to the new coded video sequence rather than dropping them.
    ///
    /// § 7.3.6 (mirror `07-decoding-process.md` lines 604-606): the new coded video
    /// sequence starts AT the temporal unit containing the CLK, so every id of that
    /// temporal unit — canonically the § 7.3.8.1 resent-at-RAP sequence header, forced to
    /// `obu_mlayer_id == 0` (the § 6.4.1 NOTE, mirror
    /// `06-syntax-structures-semantics.md` lines 450-452) — belongs to the new coded
    /// video sequence and must count toward `SeqMaxMlayerCnt`. The new state is therefore
    /// re-seeded from the current temporal unit's seen set (`per_xlayer_tu`) with
    /// `first_tu == tu_index` (the boundary temporal unit, so an exceedance within the new
    /// coded video sequence counted in this temporal unit emits eagerly). The once-per-CVS
    /// `reported` flag is carried only when the old set's first temporal unit *was* this
    /// boundary temporal unit — meaning the old set was entirely in the boundary temporal
    /// unit and thus is the same coded video sequence — so an exceedance already reported
    /// among the pre-CLK ids is not re-reported; when the old set spanned an earlier
    /// temporal unit, its (deferred) exceedance belonged to the ending coded video
    /// sequence and the new coded video sequence starts unreported.
    ///
    /// Only re-seeds the state; the § 6.4.1 exceedance comparison runs *after* the CLK's
    /// frame header activates the new coded video sequence's referenced sequence header
    /// (see [`ValidatorContext::observe_frame_bearing_obu`]'s activation path), where the
    /// correct `SeqMaxMlayerCnt` is available. A set whose pre-CLK members already exceed
    /// `SeqMaxMlayerCnt` cannot be re-surfaced by [`Self::observe`] (it never re-yields an
    /// already-seen id), so the activation path runs [`Self::current_count`] to read the
    /// re-seeded set back out.
    pub(super) fn reset_cvs(&mut self, xlayer: ExtendedLayerId, tu_index: u64) {
        let prior_first_tu = self.per_xlayer.get(&xlayer).and_then(|s| s.first_tu);
        let prior_reported = self.per_xlayer.get(&xlayer).is_some_and(|s| s.reported);
        let tu_seen = self.per_xlayer_tu.get(&xlayer).cloned().unwrap_or_default();
        if tu_seen.is_empty() {
            self.per_xlayer.remove(&xlayer);
            return;
        }
        let reported = prior_reported && prior_first_tu == Some(tu_index);
        let state = DistinctMlayerState {
            seen: tu_seen,
            first_tu: Some(tu_index),
            reported,
        };
        self.per_xlayer.insert(xlayer, state);
    }

    /// Clears the per-temporal-unit seen sets at a global `OBU_TEMPORAL_DELIMITER`
    /// (AV2 § 7.3.7), so the next temporal unit's re-attribution at a CLK
    /// ([`Self::reset_cvs`]) starts from an empty per-temporal-unit set.
    pub(super) fn advance_temporal_unit(&mut self) {
        self.per_xlayer_tu.clear();
    }

    /// Records `mlayer` under `xlayer`'s current coded video sequence at temporal unit
    /// `tu_index` and returns `(new_distinct_count, first_tu)` when this `obu_mlayer_id`
    /// was not already counted *and* the exceedance has not yet been reported for this
    /// coded video sequence; otherwise `None`. `first_tu` is the temporal unit of the
    /// set's first counted OBU (the [`CvsTracker::defer_or_emit`] baseline). The caller
    /// compares the returned count against `SeqMaxMlayerCnt` and, on the first exceedance,
    /// marks the coded video sequence reported via [`Self::mark_reported`].
    ///
    /// The id is always recorded in the per-temporal-unit set
    /// ([`DistinctMlayerTracker::per_xlayer_tu`]) regardless of the once-per-CVS report
    /// state, so [`Self::reset_cvs`] can re-attribute every pre-CLK id of the boundary
    /// temporal unit to the new coded video sequence.
    pub(super) fn observe(
        &mut self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        tu_index: u64,
    ) -> Option<(usize, u64)> {
        self.per_xlayer_tu.entry(xlayer).or_default().insert(mlayer);
        let state = self.per_xlayer.entry(xlayer).or_default();
        if state.reported {
            return None;
        }
        let first_tu = *state.first_tu.get_or_insert(tu_index);
        if state.seen.insert(mlayer) {
            Some((state.seen.len(), first_tu))
        } else {
            None
        }
    }

    /// Marks `xlayer`'s current coded video sequence as having reported the § 6.4.1
    /// exceedance, suppressing further reports until the next CVS reset.
    pub(super) fn mark_reported(&mut self, xlayer: ExtendedLayerId) {
        self.per_xlayer.entry(xlayer).or_default().reported = true;
    }

    /// Returns the distinct `obu_mlayer_id` count accumulated so far in `xlayer`'s
    /// current coded video sequence and the set's first-counted temporal unit, or `None`
    /// when nothing has been counted yet or the exceedance was already reported for this
    /// coded video sequence (emit once per CVS). Read-only: it does not record a new id,
    /// so it surfaces a count that [`Self::observe`] cannot re-yield (its already-seen ids
    /// return `None`). Used by the activation-path retroactive check, which compares a
    /// count accumulated *before* a sequence header became active for the extended layer.
    pub(super) fn current_count(&self, xlayer: ExtendedLayerId) -> Option<(usize, u64)> {
        let state = self.per_xlayer.get(&xlayer)?;
        if state.reported {
            return None;
        }
        let first_tu = state.first_tu?;
        if state.seen.is_empty() {
            return None;
        }
        Some((state.seen.len(), first_tu))
    }
}

impl ValidatorContext {
    /// Returns the activated in-band sequence header's general fields for `xlayer`,
    /// if any. The fields are copied out so callers can keep mutating `self`.
    pub(super) fn active_general_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        let id = *self.active_sequence_by_xlayer.get(&xlayer)?;
        let header = self.sequence_headers.get(&id)?;
        Some((id, header.general))
    }

    /// The frame-confirmed extended layers whose latest activation lies within the *current*
    /// CMVS window — i.e. whose most recent frame-confirmed sequence-header activation
    /// happened at a temporal unit at or after the CMVS-window start
    /// (`cmvs_start_tu_index`). Returns an empty vector when no CMVS window is open.
    ///
    /// The § 6.8.2 LCR DOH requirement and the § 6.6 MSDO DOH requirement scope their
    /// per-layer evaluation to this set instead of the whole-history `frame_confirmed_xlayers`
    /// accumulator, so a non-monotonic header left active from an earlier, already-ended
    /// coded video sequence outside the current CMVS does not trigger a diagnostic against
    /// this CMVS's MSDO / global LCR. The § 7.3.2 CMVS spans
    /// specific temporal units, so a temporal-unit lower bound is the right scope.
    pub(super) fn frame_confirmed_xlayers_in_current_cmvs(&self) -> Vec<ExtendedLayerId> {
        let Some(cmvs_start) = self.cmvs.current_cmvs_start_tu_index() else {
            return Vec::new();
        };
        self.frame_confirmed_xlayers
            .iter()
            .copied()
            .filter(|xlayer| {
                self.frame_confirmed_activation_tu
                    .get(xlayer)
                    .is_some_and(|&tu| tu >= cmvs_start)
            })
            .collect()
    }

    /// The activated sequence header usable for the § 6.10.7 / § 6.8.9 agreement
    /// checks: the in-band active header for `xlayer`, but only when the
    /// activation is *decidable* — confirmed by a parsed frame-header reference
    /// (§ 5.18.2 `load_sequence_header`), or the OBU-order fallback while it is
    /// the sole in-band sequence header (any frame must then reference it or
    /// trip the availability checks). With several in-band candidates and no
    /// frame yet, the first-seen fallback is a guess a later frame can
    /// contradict, and an agreement error emitted against the guess could not be
    /// retracted — so the checks defer to frame-driven activation instead.
    pub(super) fn agreement_activation_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        let resolved = self.active_general_for(xlayer)?;
        if self.frame_confirmed_xlayers.contains(&xlayer) || self.sequence_headers.len() == 1 {
            Some(resolved)
        } else {
            None
        }
    }

    /// The activated sequence header for `xlayer`, but *only* when a parsed
    /// frame-header reference confirmed it (§ 5.18.2 `load_sequence_header`) — the
    /// strict variant of [`Self::agreement_activation_for`] that does *not* admit the
    /// sole-in-band-header OBU-order fallback.
    ///
    /// The fallback (`sequence_headers.len() == 1`) is a guess: § 7.3.6 permits staging a
    /// header before any frame activates one, and with external HLS declared the *real*
    /// activated header could be the external one (the in-band staged header may never be
    /// referenced). Checks that fire unconditionally on a violation (the Annex A
    /// value-space check, and the § 6.8.5 / § 6.8.8 / § 6.8.9 LCR-agreement checks)
    /// therefore use this strict gate so they never emit against a fallback guess; they
    /// re-enter the moment a frame confirms the activation. Contrast the OPS / § 6.8.2
    /// resolutions that tolerate the fallback because they emit nothing without an
    /// OPS/global-LCR present and are otherwise suppressed under external HLS.
    pub(super) fn frame_confirmed_activation_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(SequenceHeaderId, SequenceHeaderGeneral)> {
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return None;
        }
        self.active_general_for(xlayer)
    }

    /// Runs the § 6.10.7 / § 6.8.9 agreement checks that become decidable when a
    /// sequence header is newly activated (or re-activated to a different id) for
    /// `xlayer`: the stored explicit maps of active OPS records describing the
    /// layer (its local bucket plus global-OPS entries), and the § 6.8.9 pairing
    /// through the activated header's `seq_lcr_id`. The dedup keys make repeated
    /// activation idempotent.
    pub(super) fn on_sequence_activation(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        self.check_annex_a_value_space(xlayer, report);
        self.check_lcr_dependency_agreement(xlayer, options, report);
        self.check_lcr_ptl_ceilings(xlayer, options, report);
        self.check_lcr_rep_info_agreement(xlayer, options, report);
        self.check_lcr_expected_dims_bounds(xlayer, options, report);
        if external_declares_sequence_header(options) {
            return;
        }
        let mut pending: Vec<(ByteOffset, u8, Vec<OpsExplicitEntry>)> = Vec::new();
        for bucket in [xlayer, GLOBAL_XLAYER_ID] {
            for record in self.ops.records_for(bucket) {
                let relevant: Vec<OpsExplicitEntry> = record
                    .explicit_entries
                    .iter()
                    .filter(|entry| entry.xlayer_id == xlayer)
                    .cloned()
                    .collect();
                if !relevant.is_empty() {
                    pending.push((record.offset, record.ops_id, relevant));
                }
            }
        }
        for (offset, ops_id, entries) in pending {
            self.check_ops_entries_against_active(offset, ops_id, &entries, options, report);
        }
        self.check_substream_max_ceilings(xlayer, options, report);
        self.note_annex_a_iop_activation(xlayer, options);
    }

    pub(super) fn observe_sequence_header(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(sequence_header) = parse_sequence_header(&mut reader) else {
            return;
        };
        if sequence_header.is_fully_parsed()
            && finish_obu_payload(
                &mut reader,
                obu.payload,
                obu.header.obu_type.is_extensible_obu(),
            )
            .is_err()
        {
            return;
        }
        let general = sequence_header.general;

        if !sequence_header_can_activate(obu) {
            return;
        }

        self.hls
            .record_sequence_header(u32::from(general.seq_header_id.get()));
        self.rap_replay.note_resend(
            RapHlsKey::SequenceHeader(u32::from(general.seq_header_id.get())),
            obu.header.extended_layer_id,
        );

        self.check_seq_lcr_reference(obu, general.seq_lcr_id.get(), options, report);
        self.note_seq_lcr_rap_reference(obu, general.seq_lcr_id.get());

        let seq_header_id = general.seq_header_id;
        let xlayer = obu.header.extended_layer_id;
        let fingerprint = payload_fingerprint(obu.payload);

        // TODO(spec: AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT): enforce cross-extended-layer
        let tu_index = self.cvs.tu_index;
        match self.sequence_fingerprints.entry((xlayer, seq_header_id)) {
            Entry::Vacant(slot) => {
                slot.insert((fingerprint, tu_index));
            }
            Entry::Occupied(mut slot) => {
                let (stored_fingerprint, stored_tu) = *slot.get();
                if stored_fingerprint != fingerprint {
                    let diagnostic = Diagnostic::error(
                        "hls/repeated-sequence-header-not-identical",
                        format!(
                            "activated sequence header seq_header_id {} for obu_xlayer_id {} \
                             is repeated with different payload bytes",
                            seq_header_id.get(),
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6")
                    .with_byte_offset(obu.offset);
                    self.cvs
                        .defer_or_emit(xlayer, stored_tu, diagnostic, report);
                }
                slot.insert((fingerprint, tu_index));
            }
        }

        let new_general = sequence_header.general;
        self.sequence_header_offsets
            .insert(seq_header_id, obu.offset);
        let previous_header = self.sequence_headers.insert(seq_header_id, sequence_header);
        self.active_sequence_by_xlayer
            .entry(xlayer)
            .or_insert(seq_header_id);

        self.snapshot_lcr_association(xlayer, seq_header_id, new_general.seq_lcr_id.get());

        let agreement_inputs_changed = previous_header.as_ref().is_some_and(|previous| {
            let old = previous.general;
            old.mlayer_dependency_map != new_general.mlayer_dependency_map
                || old.tlayer_dependency_map != new_general.tlayer_dependency_map
                || old.seq_lcr_id != new_general.seq_lcr_id
        });
        let annex_a_value_space_changed = previous_header.as_ref().is_some_and(|previous| {
            annex_a_value_space_fingerprint(&previous.general)
                != annex_a_value_space_fingerprint(&new_general)
        });
        let lcr_agreement_values_changed = previous_header.as_ref().is_some_and(|previous| {
            lcr_agreement_value_fingerprint(&previous.general)
                != lcr_agreement_value_fingerprint(&new_general)
        });
        let mut layers_to_check = BTreeSet::new();
        if self.active_sequence_by_xlayer.get(&xlayer) == Some(&seq_header_id) {
            layers_to_check.insert(xlayer);
        }
        if agreement_inputs_changed || annex_a_value_space_changed || lcr_agreement_values_changed {
            layers_to_check.extend(
                self.active_sequence_by_xlayer
                    .iter()
                    .filter(|(_, id)| **id == seq_header_id)
                    .map(|(layer, _)| *layer),
            );
        }
        if agreement_inputs_changed {
            self.emitted_dependency_findings
                .retain(|key| key.seq_header_id() != seq_header_id);
        }
        let external_hls_suppresses = matches!(
            &options.external_hls,
            ExternalHlsMode::Provided(set) if set.declares_any_sequence_header()
        );
        for layer in layers_to_check {
            self.on_sequence_activation(layer, options, report);
            self.check_monotonic_output_order_agreement(layer, obu.offset, options, report);
            if !external_hls_suppresses {
                let byte_offset = obu.offset.saturating_add(1);
                self.retroactive_distinct_mlayer_check(layer, byte_offset, report);
            }
        }
    }

    pub(super) fn validate_active_sequence_limits(
        &self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !requires_active_sequence(obu) {
            return;
        }

        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.declares_any_sequence_header()
        {
            return;
        }

        let Some(seq_header_id) = self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)
        else {
            report.push(sequence_state_error(
                "sequence-state/no-active-sequence-header",
                "7.3.8",
                obu,
                None,
                format!(
                    "{} uses obu_xlayer_id {} before an active sequence header is available",
                    obu.header.obu_type.spec_name(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        let Some(sequence_header) = self.sequence_headers.get(seq_header_id) else {
            report.push(sequence_state_error(
                "sequence-state/unknown-sequence-header-id",
                "7.3.8",
                obu,
                None,
                format!(
                    "active seq_header_id {} for obu_xlayer_id {} is unavailable",
                    seq_header_id.get(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        let (max_tlayer_id, max_mlayer_id) = self
            .frame_confirmed_activated_limits
            .get(&obu.header.extended_layer_id)
            .copied()
            .unwrap_or((
                sequence_header.general.max_tlayer_id,
                sequence_header.general.max_mlayer_id,
            ));

        if obu.header.temporal_layer_id > max_tlayer_id {
            report.push(sequence_state_error(
                "sequence-state/tlayer-exceeds-max",
                "6.2.2",
                obu,
                Some(BitOffset::from_bits(6)),
                format!(
                    "obu_tlayer_id {} exceeds active sequence max_tlayer_id {}",
                    obu.header.temporal_layer_id.get(),
                    max_tlayer_id.get()
                ),
            ));
        }

        if obu.header.embedded_layer_id > max_mlayer_id {
            let byte_offset = obu.offset.saturating_add(1);
            report.push(
                Diagnostic::error(
                    "sequence-state/mlayer-exceeds-max",
                    format!(
                        "obu_mlayer_id {} exceeds active sequence max_mlayer_id {}",
                        obu.header.embedded_layer_id.get(),
                        max_mlayer_id.get()
                    ),
                )
                .with_spec_section("6.2.2")
                .with_byte_offset(byte_offset)
                .with_bit_offset(BitOffset::from_bits(0)),
            );
        }
    }

    /// Counts the distinct `obu_mlayer_id` values present in `obu`'s extended layer's
    /// current coded video sequence and emits
    /// `sequence-state/distinct-mlayer-count-exceeds-seq-max` (AV2 § 6.4.1) the first
    /// time the count exceeds the active sequence header's `SeqMaxMlayerCnt`.
    ///
    /// § 6.4.1 requires "the number of distinct values of obu_mlayer_id present in the
    /// coded video sequence associated with this sequence header is less than or equal
    /// to SeqMaxMlayerCnt" (mirror `06-syntax-structures-semantics.md` lines 445-447).
    /// Only OBUs carrying a concrete `obu_xlayer_id` are counted (global OBUs cannot be
    /// unambiguously attributed to one extended layer's coded video sequence; see
    /// [`DistinctMlayerTracker`] for the conservative attribution reading and the
    /// associated spec TODO). The comparison uses the extended layer's active sequence
    /// header, so a layer with no active header yet — or whose active header is supplied
    /// out of band — is skipped rather than guessed.
    ///
    /// § 7.3.6 starts a new coded video sequence *at* the temporal unit containing the
    /// CLK, so an OBU of this extended layer observed earlier in the temporal unit that a
    /// later CLK begins a coded video sequence already belongs to the *new* coded video
    /// sequence. The validator cannot know a CLK is still coming when it counts that OBU,
    /// so an exceedance whose accumulated set spans an earlier temporal unit is routed
    /// through [`CvsTracker::defer_or_emit`]: deferred, then dropped by
    /// [`CvsTracker::flush_completed_tu`] when a CLK started a coded video sequence for
    /// this extended layer in the temporal unit (the set straddled the boundary), and
    /// emitted otherwise. An exceedance whose set is entirely within the current temporal
    /// unit is in one coded video sequence regardless of a later CLK and is emitted eagerly.
    pub(super) fn count_distinct_mlayer(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if obu.header.extended_layer_id.is_global() {
            return;
        }

        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.declares_any_sequence_header()
        {
            return;
        }

        let xlayer = obu.header.extended_layer_id;
        let tu_index = self.cvs.tu_index;
        let Some((new_count, first_tu)) =
            self.distinct_mlayer
                .observe(xlayer, obu.header.embedded_layer_id, tu_index)
        else {
            return;
        };

        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let byte_offset = obu.offset.saturating_add(1);
        self.emit_distinct_mlayer_exceedance(
            xlayer,
            (new_count, first_tu),
            (seq_header_id, general.seq_max_mlayer_count.get()),
            byte_offset,
            report,
        );
    }

    /// Retroactively compares `xlayer`'s already-accumulated distinct-`obu_mlayer_id`
    /// count against the `SeqMaxMlayerCnt` of the sequence header that just became active
    /// for it (AV2 § 6.4.1). OBUs arriving before any active sequence header for an
    /// extended layer accumulate a distinct count that [`Self::count_distinct_mlayer`]
    /// never compares (it has no header to associate the count with, and the activating
    /// sequence header's own already-seen `obu_mlayer_id == 0` makes [`DistinctMlayerTracker::observe`]
    /// yield `None`). Once a header activates, its `SeqMaxMlayerCnt` is available, so the
    /// pre-activation count is compared here. The diagnostic is anchored to the activating
    /// OBU's `byte_offset` and routed/deduplicated identically to the eager check
    /// (emit once per CVS via [`DistinctMlayerTracker::mark_reported`]). Called from the
    /// sequence-activation path; the external-HLS gate is applied by the caller.
    pub(super) fn retroactive_distinct_mlayer_check(
        &mut self,
        xlayer: ExtendedLayerId,
        byte_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        let Some((count, first_tu)) = self.distinct_mlayer.current_count(xlayer) else {
            return;
        };
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        self.emit_distinct_mlayer_exceedance(
            xlayer,
            (count, first_tu),
            (seq_header_id, general.seq_max_mlayer_count.get()),
            byte_offset,
            report,
        );
    }

    /// Emits `sequence-state/distinct-mlayer-count-exceeds-seq-max` (AV2 § 6.4.1) when the
    /// distinct count exceeds the active header's `SeqMaxMlayerCnt`, routing the diagnostic
    /// through the § 7.3.6 boundary logic and marking `xlayer`'s coded video sequence
    /// reported (emit once per CVS). Shared by the eager [`Self::count_distinct_mlayer`]
    /// and the activation-path [`Self::retroactive_distinct_mlayer_check`].
    /// `count_and_first_tu` is the distinct count and the set's first-counted temporal unit
    /// (the [`CvsTracker::defer_or_emit`] deferral baseline): a set confined to the current
    /// temporal unit emits eagerly; a set spanning an earlier temporal unit is deferred and
    /// dropped if a CLK begins a new coded video sequence for this extended layer in this
    /// temporal unit (the pre-CLK members then belong to the new coded video sequence, not
    /// the exceeding old one). `active_header` is the activated header's id and its
    /// `SeqMaxMlayerCnt`.
    pub(super) fn emit_distinct_mlayer_exceedance(
        &mut self,
        xlayer: ExtendedLayerId,
        count_and_first_tu: (usize, u64),
        active_header: (SequenceHeaderId, u8),
        byte_offset: ByteOffset,
        report: &mut ValidationReport,
    ) {
        let (count, first_tu) = count_and_first_tu;
        let (seq_header_id, max_mlayer_cnt) = active_header;
        let max = usize::from(max_mlayer_cnt);
        if count <= max {
            return;
        }
        let diagnostic = Diagnostic::error(
            "sequence-state/distinct-mlayer-count-exceeds-seq-max",
            format!(
                "the coded video sequence for obu_xlayer_id {} carries {} distinct \
                 obu_mlayer_id values, exceeding SeqMaxMlayerCnt {} of the active \
                 sequence header {}",
                xlayer.get(),
                count,
                max,
                seq_header_id.get()
            ),
        )
        .with_spec_section("6.4.1")
        .with_byte_offset(byte_offset)
        .with_bit_offset(BitOffset::from_bits(0));
        self.cvs.defer_or_emit(xlayer, first_tu, diagnostic, report);
        self.distinct_mlayer.mark_reported(xlayer);
    }
}
