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

        // AV2 § 6.17.2: FirstPictureInTU is per extended layer ("the first frame
        // unit in a coded extended layer unit in a temporal unit"), so a frame in
        // another extended layer earlier in this temporal unit does not clear it.
        let first_picture_in_tu = self.first_picture_in_tu(obu.header.extended_layer_id);
        self.frames_seen_in_tu.insert(obu.header.extended_layer_id);

        // A parse failure is silent: a frame/tile-group payload the skeleton cannot
        // reach is not-yet-validated coverage, not a conformance error in this phase.
        let Some(prefix) = parse_frame_prefix(obu, first_picture_in_tu) else {
            return;
        };

        let resolved = self.resolve_frame_header_reference(&prefix, obu, options, report);

        // AV2 § 7.3.8.1: buffer this frame's in-band-resolved HLS references for the
        // random-access-point availability replay. Only in-band-resolved references are
        // buffered (so the replay predicate stays disjoint from the linear
        // `hls/unavailable-*` checks, and an externally-supplied reference is not
        // double-judged): `resolved` is the in-band sequence-header id, and a
        // `cur_mfh_id > 0` that resolves to an in-band multi-frame header is the frame's
        // § 7.3.8.7 MFH reference. The resolution captures each object's qualifying-resend
        // snapshot as of this reference (intra-temporal-unit order).
        self.note_frame_rap_references(&prefix, resolved, obu.header.extended_layer_id, obu.offset);

        // AV2 § 7.3.8.9 / § 5.18.2: apply the frame header's reset_qm() availability effect
        // BEFORE the sequence-resolution gate below, so a reset-bearing frame whose sequence
        // reference cannot be resolved still gets the right treatment instead of being skipped
        // (codex F1). The partition is by what `reset_qm()` needs:
        //   - CLK / OLK: the §5.18.2 `keyFrame && FirstPictureInTU` reset (mirror :4106 /
        //     :4279-4283) is decidable from `obu_type` + `FirstPictureInTU` ALONE, with no
        //     sequence-dependent read before it, so it clears regardless of resolution.
        //   - RAS / restricted SWITCH: the reset sits past sequence-dependent reads, so an
        //     unresolvable reference (no in-band sequence header, `resolved == None`) cannot
        //     prove the reset fired — it POISONS, never silently skips. A resolvable reference
        //     confirms the reset from the parsed `reached_qm_reset` fact (codex F2).
        // This only mutates `self.qm` availability, independent of the §7.23 reference-buffer
        // commit inside the gate, so running it here (before activation) is order-stable.
        self.apply_qm_reset_for_frame(obu, first_picture_in_tu, resolved);

        // AV2 § 5.18.2: frame_header_info() calls load_sequence_header() for EVERY
        // frame (both cur_mfh_id == 0 and cur_mfh_id > 0), before the `if (keyFrame)`
        // block — so any parsed frame header, not only a CLK/OLK key frame, activates
        // the referenced sequence header for its extended layer, overriding the
        // OBU-order fallback. Only an in-band reference is activated (its layer limits
        // are modeled); an external reference already suppresses the layer-limit
        // checks.
        if let Some(seq_id) = resolved {
            let xlayer = obu.header.extended_layer_id;
            // Snapshot the prior activation state *before* it is overwritten below, so
            // the § 7.3.6 single-active-sequence-header check can compare against the
            // previous frame-confirmed activation.
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
            // A frame-header reference is the § 5.18.2 load_sequence_header path:
            // it *confirms* the layer's activation (the OBU-order fallback was a
            // guess), so the deferred § 6.10.7 / § 6.8.9 agreement checks become
            // decidable on the first confirmation even when the id is unchanged,
            // and again whenever the id changes.
            let newly_confirmed = self.frame_confirmed_xlayers.insert(xlayer);
            // Record the coded video sequence epoch of this frame-confirmed activation,
            // so a later activation can tell whether a CLK intervened (AV2 § 7.3.6).
            self.frame_confirmed_activation_cvs
                .insert(xlayer, self.cvs.cvs_epoch(xlayer));
            // Record the temporal unit of this frame-confirmed activation, so the § 6.8.2 /
            // § 6.6 DOH loops can scope to the current CMVS window (codex finding 3393129745).
            self.frame_confirmed_activation_tu
                .insert(xlayer, self.cvs.tu_index);
            // AV2 § 6.2.2 NOTE (mirror lines 197-198): snapshot the limits of the header as
            // *activated* here, so the § 6.2.2 layer-id check follows the activation window
            // rather than the live store. A later § 7.3.6 redefinition (legal only at a
            // coded-video-sequence boundary) overwrites the store but does not re-activate
            // until its own confirming frame reaches this path again; until then an OBU in the
            // prior activation's window must be bounded by these limits, not the redefined
            // ones. `seq_id` is an in-band reference (`resolved` is `Some`), so its header is
            // stored; if a future eviction policy ever removes it, the snapshot is left stale
            // (sound: still the last activated limits) rather than reset.
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
                // AV2 § 7.3.6 / Annex A Table A.4: a CLK that re-references the
                // already-active header opens a new coded video sequence (§ 7.3.6) without
                // changing the activated id, so `on_sequence_activation` is skipped. Re-seed
                // the IOP window's pending facts from the active confirmed header so the new
                // coded video sequence's window is decidable from the header carried across
                // the boundary (lesson 9), matching the `is_clk` re-run of the distinct-mlayer
                // check below.
                self.note_annex_a_iop_activation(xlayer, options);
            }
            // AV2 § 6.4.1: compare this extended layer's accumulated distinct-obu_mlayer_id
            // count against the header that just activated, the moment it activates — the
            // § 5.18.2 load_sequence_header confirmation path. Two cases reach here:
            //   (1) a count accumulated before any header was active (the eager
            //       count_distinct_mlayer had no SeqMaxMlayerCnt to compare against), or
            //   (2) the re-seeded boundary-temporal-unit set this OBU's own CLK
            //       re-attributed to the new coded video sequence in observe_cvs_boundary_events
            //       (start_cvs_for_xlayer). Case (2) must run even when the CLK re-references
            //       the SAME already-frame-confirmed header (so the id is unchanged and
            //       `newly_confirmed` is false), because DistinctMlayerTracker::observe never
            //       re-yields an already-seen id and so the eager check cannot re-surface the
            //       re-seeded set — hence the `is_clk` term. Running here, after activation,
            //       compares against the CLK-activated header (the header "associated with"
            //       the new coded video sequence, mirror `06-syntax-structures-semantics.md`
            //       lines 445-447), not the outgoing header still active when the boundary
            //       event fired (PR #41 false positive). The activating frame's own
            //       obu_mlayer_id is counted afterward by observe_obu's count_distinct_mlayer,
            //       so an id already in the set yields nothing new and never triggers the eager
            //       comparison here. Suppressed under caller-provided external HLS for the same
            //       reason as the eager check: an out-of-band header may carry a SeqMaxMlayerCnt
            //       this validator does not model.
            let is_clk = obu.header.obu_type == ObuType::ClosedLoopKey;
            if previous != Some(seq_id) || newly_confirmed || is_clk {
                let external_hls_suppresses = matches!(
                    &options.external_hls,
                    ExternalHlsMode::Provided(set) if set.declares_any_sequence_header()
                );
                if !external_hls_suppresses {
                    // Anchor to the activating OBU's extension byte (obu.offset + 1,
                    // bit 0), the same idiom as the eager count_distinct_mlayer. For a CLK
                    // this is the same anchor the removed reset-time check used.
                    let byte_offset = obu.offset.saturating_add(1);
                    self.retroactive_distinct_mlayer_check(xlayer, byte_offset, report);
                }
            }
            // AV2 § 6.4.1: cross-extended-layer monotonic_output_order_flag agreement,
            // gated on the § 7.3.2 CMVS tracker being definitively inside a CMVS.
            self.check_monotonic_output_order_agreement(xlayer, obu.offset, options, report);

            // AV2 § 6.4.13 cross-CVS advisory: evaluate on EVERY frame-confirmed
            // activation, not only an id change or first confirmation. A same-id
            // reconfiguration across a coded-video-sequence boundary (legal at the
            // boundary, § 7.3.6) re-confirms the unchanged id, so the short-circuit above
            // would skip it; this check must still re-compare. A CLK starts the new coded
            // video sequence before its own frame header activates (boundary events run
            // first in `observe_obu`), so by here the CVS epoch is already the new one.
            // The comparison is idempotent within a coded video sequence (it overwrites
            // its baseline with the same sum at the same epoch).
            self.check_seq_buffer_delay_sum(
                obu.header.extended_layer_id,
                obu.offset,
                options,
                report,
            );

            // With the in-band active sequence header available, run the frame-header
            // core parser and emit the locally decidable § 6.17 diagnostics. Parsing
            // and the checks are silent on failure or on paths that need reference
            // state (AV2 § 6.17.2 / § 6.17.4 / § 6.4.6).
            //
            // A `cur_mfh_id > 0` frame derives FrameWidth/FrameHeight (and the
            // §5.18.7.1 segmentation arm) from its resolved multi-frame header on the
            // non-override path, so resolve that record with the shared §7.3.8.7
            // discipline and thread it in; without it the core parse stops before
            // frame_size() and the §6.17.2 MFH-dims / §6.17.7 tile / quant diagnostics
            // would be skipped for MFH-backed frames. An unresolvable MFH stays `None`,
            // preserving the early-stop (no guessing).
            // AV2 § 7.23: when this OBU closes the previous coded frame, that frame's
            // decode finished, so its deferred § 7.23 update must be committed BEFORE the
            // reference-buffer snapshot below — otherwise the inter parser's
            // frame_size_with_refs() / frame_size_with_bridge() would read the stale
            // pre-refresh buffer and poison, silently skipping the §6.17 frame-size
            // diagnostics that the prior frame's refresh makes decidable (codex F1). The
            // segmenter is the boundary authority; `commits_pending_ref_update` is a
            // non-mutating peek of the SAME commit decision `observe_reference_state` makes
            // later in stream order — it fires for both the `OpensNewUnit` boundary AND the
            // `Ambiguous` boundary (a same-type no-delimiter TIP / bridge opener, or an
            // unreadable tile-group delimiter), the exact set on which the prior frame's
            // update is committed. The earlier peek only matched `OpensNewUnit`, so a
            // same-type no-delimiter opener after a refresh snapshotted the stale buffer
            // (codex F1). A decided continuation (its own frame's update is pending) does
            // NOT commit here, so the committing OBU never sees its OWN update — preserving
            // the PR #62 deferral. The commit is idempotent (`commit_pending_ref_update`
            // takes the pending), so the later `observe_reference_state` re-commit at the
            // same boundary is a no-op.
            let role = self.seg_role_for(obu, first_picture_in_tu);
            if self.frame_unit.commits_pending_ref_update(obu, role) {
                self.commit_pending_ref_update();
            }
            let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);
            // The frame's linearly-available §7.3.8.1 random-access-point HLS references (film
            // grain + quantizer matrix), surfaced by the reference checks so they can be
            // buffered below in &mut self context (the checks borrow self immutably).
            let mut rap_refs = FrameRapReferences::default();
            if let Some(active_sequence) = self.sequence_headers.get(&seq_id) {
                // AV2 § 7.23: thread the modeled per-extended-layer reference-frame buffer
                // into the §6.17 frame-header checks so the §6.17.2 inter `ref_frame_idx`
                // validity check sees the same `RefValid[]` the celu/output decisions do.
                // The scratch arrays must outlive the check, so they are stack-local here.
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
            // AV2 § 7.3.8.1: buffer the frame's linearly-available film-grain and
            // quantizer-matrix references for the random-access-point availability replay
            // (disjoint from the linear `frame-header/film-grain-model-unavailable` /
            // `frame-header/qm-level-unavailable`, which own the unavailable cases). Governed
            // by the frame's own extended layer.
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
        // Gate 4: caller-provided external HLS that declares a sequence header may supply
        // the active one out of band, making the in-band activation history unreliable.
        // An external channel that declares no sequence header cannot, so it does not
        // suppress (mirrors validate_active_sequence_limits' narrow gate).
        if external_declares_sequence_header(options) {
            return;
        }
        let xlayer = obu.header.extended_layer_id;
        // Gates 1 + 3: a prior *frame-confirmed* activation of a different sequence
        // header. `prior_activation_cvs` is `Some(epoch)` exactly when a prior
        // frame-confirmed activation was recorded; pair it with the frame-confirmed flag
        // and a recorded prior id.
        let (Some(prior_seq), true, Some(prior_epoch)) =
            (prior_seq, prior_frame_confirmed, prior_activation_cvs)
        else {
            return;
        };
        if prior_seq == new_seq {
            return;
        }
        // Gate 2: both activations are in the same coded video sequence (no CLK between
        // them advanced the epoch). The prior activation's recorded epoch — `None` for
        // the implicit pre-first-CLK coded video sequence — must equal the epoch now in
        // effect for this extended layer. A first-temporal-unit CLK gives `Some(0)`,
        // distinct from the pre-CLK `None`, so a re-activation across it does not match.
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
            // cur_mfh_id == 0: the frame references a sequence header directly.
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
            // cur_mfh_id > 0: resolve the multi-frame header, then its sequence header.
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
                // AV2 § 7.3.8.7: a multi-frame header may be provided "by inclusion in
                // the bitstream or by provision through external means". The validator
                // models external sequence headers but does not yet model external
                // multi-frame headers, so under ExternalHlsMode::Provided an
                // out-of-band MFH could satisfy this reference — suppress the hard
                // error to avoid rejecting a conformant external-HLS stream. Under the
                // default (Disabled) there is no external means, so it is unavailable.
                // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): declare external multi-frame
                // headers in ValidationOptions instead of suppressing under Provided.
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(frame_header_unavailable_mfh(cur, obu));
                }
                return None;
            };
            let mfh_mlayer_id = record.mfh_mlayer_id;
            let mfh_tlayer_id = record.mfh_tlayer_id;
            let seq_raw = u32::from(record.mfh_seq_header_id.get());
            let resolved = self.resolve_referenced_sequence_header(seq_raw, obu, options, report);

            // AV2 § 7.3.8.7: "the layer dependency constraints TLayerDependencyMap
            // and MLayerDependencyMap are satisfied for the referenced multi-frame
            // header OBU", with the concrete predicate from § 6.17.2, evaluated
            // after the sequence header is loaded:
            // MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]] == 1 and
            // TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]]
            // == 1, where obu_{m,t}layer_id are the frame header's. Only an
            // in-band-resolved sequence header has modeled § 5.4.1 maps; an external
            // or unavailable resolution is skipped (the availability diagnostics own
            // those cases, and unmodeled maps must not produce false positives).
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
