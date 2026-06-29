// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header observation and activation checks.

use super::*;

impl ValidatorContext {
    /// Resets the §6.12/§6.13 coded-frame windows for quantizer-matrix and film-grain
    /// state at any coded frame, including a SEF (the § 7.3.3 grammar makes a SEF its
    /// own coded frame unit and calls it a frame, so it is a coded-frame boundary for
    /// both the QM between-coded-frames window and the film-grain coded-frame-unit
    /// window — see [`is_frame_bearing`]).
    pub(super) fn reset_coded_frame_window(&mut self) {
        self.qm.reset_coded_frame_window();
        self.film_grain.reset_coded_frame_window();
    }

    /// Observes a frame-bearing OBU — a tile-group OBU (tile group, switch, RAS
    /// frame) or a SEF / TIP / bridge frame — by parsing its frame-header prefix
    /// (best-effort) and running the HLS reference and sequence-activation checks
    /// (AV2 § 5.18.2 / § 7.3.8).
    pub(super) fn observe_frame_bearing_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !is_frame_bearing(obu.header.obu_type) {
            return;
        }

        let first_picture_in_tu = self.first_picture_in_tu(obu.header.extended_layer_id);
        self.frames_seen_in_tu.insert(obu.header.extended_layer_id);

        let Some(prefix) = parse_frame_prefix(obu, first_picture_in_tu) else {
            return;
        };

        let resolved = self.resolve_frame_header_reference(&prefix, obu, options, report);

        self.note_frame_rap_references(&prefix, resolved, obu.header.extended_layer_id, obu.offset);

        self.apply_qm_reset_for_frame(obu, first_picture_in_tu, resolved);

        if let Some(seq_id) = resolved {
            let xlayer = obu.header.extended_layer_id;
            let prior_seq = self.active_sequence_by_xlayer.get(&xlayer).copied();
            let prior_frame_confirmed = self.frame_confirmed_xlayers.contains(&xlayer);
            let prior_activation_cvs = self.frame_confirmed_activation_cvs.get(&xlayer).copied();
            self.check_single_active_sequence_header(
                obu,
                seq_id,
                prior_seq,
                prior_frame_confirmed,
                prior_activation_cvs,
                options,
                report,
            );

            let previous = self.active_sequence_by_xlayer.insert(xlayer, seq_id);
            let newly_confirmed = self.frame_confirmed_xlayers.insert(xlayer);
            self.frame_confirmed_activation_cvs
                .insert(xlayer, self.cvs.cvs_epoch(xlayer));
            self.frame_confirmed_activation_tu
                .insert(xlayer, self.cvs.tu_index);
            if let Some(activated) = self.sequence_headers.get(&seq_id) {
                self.frame_confirmed_activated_limits.insert(
                    xlayer,
                    (
                        activated.general.max_tlayer_id,
                        activated.general.max_mlayer_id,
                    ),
                );
            }
            if previous != Some(seq_id) || newly_confirmed {
                self.on_sequence_activation(xlayer, options, report);
            } else if obu.header.obu_type == ObuType::ClosedLoopKey {
                self.note_annex_a_iop_activation(xlayer, options);
            }
            let is_clk = obu.header.obu_type == ObuType::ClosedLoopKey;
            if previous != Some(seq_id) || newly_confirmed || is_clk {
                let external_hls_suppresses = matches!(
                    &options.external_hls,
                    ExternalHlsMode::Provided(set) if set.declares_any_sequence_header()
                );
                if !external_hls_suppresses {
                    let byte_offset = obu.offset.saturating_add(1);
                    self.retroactive_distinct_mlayer_check(xlayer, byte_offset, report);
                }
            }
            self.check_monotonic_output_order_agreement(xlayer, obu.offset, options, report);

            self.check_seq_buffer_delay_sum(
                obu.header.extended_layer_id,
                obu.offset,
                options,
                report,
            );

            let role = self.seg_role_for(obu, first_picture_in_tu);
            if self.frame_unit.commits_pending_ref_update(obu, role) {
                self.commit_pending_ref_update();
            }
            let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);
            let mut rap_refs = FrameRapReferences::default();
            if let Some(active_sequence) = self.sequence_headers.get(&seq_id) {
                let mut ref_valid = [false; NUM_REF_FRAMES];
                let mut ref_oh = [0u32; NUM_REF_FRAMES];
                let mut ref_w = [0u32; NUM_REF_FRAMES];
                let mut ref_h = [0u32; NUM_REF_FRAMES];
                let reference_buffer = if self
                    .reference_state
                    .view_into(
                        obu.header.extended_layer_id,
                        &mut ref_valid,
                        &mut ref_oh,
                        &mut ref_w,
                        &mut ref_h,
                    )
                    .is_some()
                {
                    FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h)
                } else {
                    FrameReferenceStateView::unknown()
                };
                rap_refs = frame_header_core_checks(
                    obu,
                    first_picture_in_tu,
                    active_sequence,
                    mfh_record,
                    FrameReferenceAvailability {
                        qm: &self.qm,
                        film_grain: &self.film_grain,
                        reference_buffer,
                    },
                    options,
                    report,
                );
            }
            let xlayer = obu.header.extended_layer_id;
            if let Some(slot) = rap_refs.film_grain_slot {
                self.note_rap_reference(RapHlsKey::FilmGrain { slot }, xlayer, obu.offset);
            }
            for level in rap_refs.qm_levels {
                self.note_rap_reference(RapHlsKey::QmLevel { level }, xlayer, obu.offset);
            }
        }
    }

    /// Emits `hls/multiple-active-sequence-headers` (AV2 § 7.3.6) when a frame-confirmed
    /// activation of `new_seq` for `obu`'s extended layer follows an earlier
    /// frame-confirmed activation of a *different* sequence header within the *same*
    /// coded video sequence.
    ///
    /// AV2 § 7.3.6 (mirror `07-decoding-process.md` lines 613-616): "Within each
    /// extended layer, only one sequence header shall remain active for the duration of
    /// a coded video sequence, i.e., until a CLK is encountered for that extended layer.
    /// Additional sequence header OBUs with a different seq_header_id can be present in
    /// the bitstream but are not activated and have no effect on the decoding process
    /// until referenced by a subsequent CLK frame header."
    ///
    /// The four gates (design decision 5):
    /// 1. the prior activation for this extended layer was *frame-confirmed*
    ///    (`prior_frame_confirmed`) — an OBU-order fallback guess never fires the check,
    ///    because a guess a later frame can contradict could not be retracted;
    /// 2. no § 7.3.6 coded-video-sequence start intervened — the prior frame-confirmed
    ///    activation shares this activation's coded video sequence epoch
    ///    ([`CvsTracker::cvs_epoch`]); a CLK advances the epoch, so a re-activation
    ///    across a CLK (a legal new coded video sequence) does not match;
    /// 3. the newly activated `seq_header_id` differs from the prior one; and
    /// 4. caller-provided external HLS does not declare any sequence header — only a
    ///    *declared* external sequence header can be the out-of-band active header that
    ///    makes the in-band activation history unreliable. An external channel that
    ///    declares no sequence header (`Provided(ExternalHlsSet::new())`, or one
    ///    declaring only operating point sets) cannot supply an active header, so it does
    ///    not suppress (precedent: [`ValidatorContext::validate_active_sequence_limits`]).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::option_option)]
    pub(super) fn check_single_active_sequence_header(
        &self,
        obu: &ObuEnvelope<'_>,
        new_seq: SequenceHeaderId,
        prior_seq: Option<SequenceHeaderId>,
        prior_frame_confirmed: bool,
        prior_activation_cvs: Option<Option<u64>>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        let xlayer = obu.header.extended_layer_id;
        let (Some(prior_seq), true, Some(prior_epoch)) =
            (prior_seq, prior_frame_confirmed, prior_activation_cvs)
        else {
            return;
        };
        if prior_seq == new_seq {
            return;
        }
        if prior_epoch != self.cvs.cvs_epoch(xlayer) {
            return;
        }
        report.push(
            Diagnostic::error(
                "hls/multiple-active-sequence-headers",
                format!(
                    "obu_xlayer_id {} activates sequence header {} while sequence header {} is \
                     still active for the same coded video sequence; only one sequence header \
                     may remain active until a CLK starts a new coded video sequence",
                    xlayer.get(),
                    new_seq.get(),
                    prior_seq.get()
                ),
            )
            .with_spec_section("7.3.6")
            .with_byte_offset(obu.offset),
        );
    }

    /// Resolves a parsed frame header's sequence-header reference, emitting range and
    /// availability diagnostics (AV2 § 5.18.2 / § 7.3.8.6 / § 7.3.8.7). Returns the
    /// in-band-resolved `seq_header_id` for activation, or `None` when it is out of
    /// range, resolved only externally, or unavailable.
    pub(super) fn resolve_frame_header_reference(
        &self,
        prefix: &FrameHeaderPrefix,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) -> Option<SequenceHeaderId> {
        if prefix.cur_mfh_id.is_zero() {
            let raw = prefix.seq_header_id_in_frame_header?;
            if raw >= MAX_SEQ_NUM {
                report.push(frame_header_error(
                    "frame-header/seq-header-id-out-of-range",
                    "6.17",
                    obu,
                    format!(
                        "seq_header_id_in_frame_header {raw} must be less than MAX_SEQ_NUM \
                         ({MAX_SEQ_NUM})"
                    ),
                ));
                return None;
            }
            self.resolve_referenced_sequence_header(raw, obu, options, report)
        } else {
            let cur = prefix.cur_mfh_id;
            if !cur.in_range() {
                report.push(frame_header_error(
                    "frame-header/cur-mfh-id-out-of-range",
                    "6.17",
                    obu,
                    format!(
                        "cur_mfh_id {} must be less than MAX_MFH_NUM ({MAX_MFH_NUM})",
                        cur.get()
                    ),
                ));
                return None;
            }
            let Some(record) = self.hls.multi_frame_header(cur) else {
                // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): declare external multi-frame
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(frame_header_unavailable_mfh(cur, obu));
                }
                return None;
            };
            let mfh_mlayer_id = record.mfh_mlayer_id;
            let mfh_tlayer_id = record.mfh_tlayer_id;
            let seq_raw = u32::from(record.mfh_seq_header_id.get());
            let resolved = self.resolve_referenced_sequence_header(seq_raw, obu, options, report);

            if let Some(seq_id) = resolved
                && let Some(header) = self.sequence_headers.get(&seq_id)
            {
                let general = header.general;
                let frame_mlayer = obu.header.embedded_layer_id;
                let frame_tlayer = obu.header.temporal_layer_id;
                if !general
                    .mlayer_dependency_map
                    .depends_on(frame_mlayer, mfh_mlayer_id)
                {
                    report.push(frame_header_error(
                        "frame-header/mfh-mlayer-dependency-missing",
                        "7.3.8.7",
                        obu,
                        format!(
                            "frame header at obu_mlayer_id {} references multi-frame header {} \
                             recorded at obu_mlayer_id {}, but the loaded sequence header {}'s \
                             MLayerDependencyMap[{}][{}] is 0 (§ 6.17.2)",
                            frame_mlayer.get(),
                            cur.get(),
                            mfh_mlayer_id.get(),
                            seq_id.get(),
                            frame_mlayer.get(),
                            mfh_mlayer_id.get(),
                        ),
                    ));
                }
                if !general.tlayer_dependency_map.depends_on(
                    frame_mlayer,
                    frame_tlayer,
                    mfh_tlayer_id,
                ) {
                    report.push(frame_header_error(
                        "frame-header/mfh-tlayer-dependency-missing",
                        "7.3.8.7",
                        obu,
                        format!(
                            "frame header at obu_tlayer_id {} references multi-frame header {} \
                             recorded at obu_tlayer_id {}, but the loaded sequence header {}'s \
                             TLayerDependencyMap[{}][{}][{}] is 0 (§ 6.17.2)",
                            frame_tlayer.get(),
                            cur.get(),
                            mfh_tlayer_id.get(),
                            seq_id.get(),
                            frame_mlayer.get(),
                            frame_tlayer.get(),
                            mfh_tlayer_id.get(),
                        ),
                    ));
                }
            }
            resolved
        }
    }

    /// Resolves an in-range `seq_header_id` referenced by a frame header against the
    /// HLS store, emitting `hls/unavailable-sequence-header` (§ 7.3.8.6) — and the
    /// external-HLS advisory under the default — when unavailable. Returns the id only
    /// for in-band availability (so it can be activated; an external reference has no
    /// modeled layer limits).
    pub(super) fn resolve_referenced_sequence_header(
        &self,
        seq_header_id: u32,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) -> Option<SequenceHeaderId> {
        match self.hls.resolve_sequence_header(seq_header_id, options) {
            HlsResolution::InBand => SequenceHeaderId::try_new(seq_header_id),
            HlsResolution::External => None,
            HlsResolution::Unavailable => {
                let external_note = if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    " (external HLS is disabled)"
                } else {
                    " in-band or through the supplied external HLS"
                };
                report.push(
                    Diagnostic::error(
                        "hls/unavailable-sequence-header",
                        format!(
                            "frame header references sequence header {seq_header_id}, but no \
                             sequence header with that id is available{external_note}"
                        ),
                    )
                    .with_spec_section("7.3.8.6")
                    .with_byte_offset(obu.offset),
                );
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(external_hls_disabled_advisory(seq_header_id, obu));
                }
                None
            }
        }
    }
}
