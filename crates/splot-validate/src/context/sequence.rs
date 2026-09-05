// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sequence-header observation, activation, and per-CVS sequence state.

use super::*;

/// Per-layer distinct obu_mlayer_id count (§ 6.4.1,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md`). Count all concrete
/// layer OBUs, including sequence headers forced to mlayer 0. Global OBUs are
/// unattributed: this conservative undercount cannot create a false positive.
/// TODO(spec: AV2-6.4-SEQUENCE-HEADER-SEMANTICS): attribute global OBUs once their
/// § 6.2.2 / § 7.3.6 association to per-layer CVSs is modeled.
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerTracker {
    /// For each extended layer, the distinct `obu_mlayer_id` values counted in its
    /// current coded video sequence, and whether the § 6.4.1 exceedance has already
    /// been reported for that coded video sequence (emit once per CVS).
    pub(super) per_xlayer: BTreeMap<ExtendedLayerId, DistinctMlayerState>,
    /// Ids observed in the current TU. A CLK starts the new CVS at that TU
    /// (§ 7.3.6), so reset_cvs reattributes even ids observed before the CLK.
    pub(super) per_xlayer_tu: BTreeMap<ExtendedLayerId, BTreeSet<EmbeddedLayerId>>,
}

/// One extended layer's distinct-`obu_mlayer_id` count state within its current coded
/// video sequence; see [`DistinctMlayerTracker`].
#[derive(Debug, Default)]
pub(super) struct DistinctMlayerState {
    /// The distinct `obu_mlayer_id` values seen so far in this coded video sequence.
    pub(super) seen: BTreeSet<EmbeddedLayerId>,
    /// First counted TU, used as the CvsTracker deferral baseline. A count confined
    /// to this TU emits eagerly; one spanning earlier TUs may cross a later CLK.
    pub(super) first_tu: Option<u64>,
    /// `true` once `sequence-state/distinct-mlayer-count-exceeds-seq-max` has been
    /// emitted for this coded video sequence (the check emits once per CVS).
    pub(super) reported: bool,
}

impl DistinctMlayerTracker {
    /// Reseeds the new § 7.3.6 CVS with all ids observed in the CLK's TU.
    /// Carry reported only when the old set began in this same TU; otherwise its
    /// pending report belongs to the ending CVS. The activation path checks the
    /// reseeded count after the CLK resolves the correct sequence header; observe
    /// cannot re-yield ids already in the set.
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

    /// Records an id in both TU and CVS sets. Returns (new count, first TU) only
    /// for a new id before the first report. Always update the TU set so a later
    /// CLK can reattribute ids even after the old CVS reported an exceedance.
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

    /// Unreported count and first TU for activation-time checking. Unlike observe,
    /// this surfaces ids accumulated before a sequence header became active.
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

    /// Confirmed layers activated at or after the current CMVS start, or empty
    /// outside a CMVS. Old active non-monotonic headers must not constrain a later
    /// CMVS's MSDO/global LCR (§ 6.6 / § 6.8.2).
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

    /// Strict activation gate: requires a parsed frame-header reference (§ 5.18.2).
    /// Unlike agreement_activation_for, excludes the sole-header OBU-order guess,
    /// which may merely be staged (§ 7.3.6) or replaced by external HLS.
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
        let external_hls_suppresses = external_declares_sequence_header(options);
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

        if external_declares_sequence_header(options) {
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

    /// Counts concrete-layer ids and checks § 6.4.1 SeqMaxMlayerCnt once per CVS.
    /// Unknown/external active headers are not guessed. CvsTracker defers counts
    /// spanning earlier TUs because a later CLK begins the new CVS at this TU;
    /// a count wholly within this TU emits eagerly. See DistinctMlayerTracker for
    /// the conservative global-OBU attribution limit.
    pub(super) fn count_distinct_mlayer(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if obu.header.extended_layer_id.is_global() {
            return;
        }

        if external_declares_sequence_header(options) {
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

    /// Checks ids accumulated before activation against the now-known SeqMaxMlayerCnt.
    /// The activating OBU anchors the diagnostic; routing/dedup matches the eager
    /// check. The caller applies the external-HLS gate.
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

    /// Shared eager/activation-time § 6.4.1 exceedance check. count_and_first_tu
    /// carries the CvsTracker deferral baseline; active_header is (id, SeqMaxMlayerCnt).
    /// Marks the CVS reported only on exceedance.
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
