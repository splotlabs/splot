// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference-frame state integration and deferred frame updates.

use super::*;

impl ValidatorContext {
    /// Maintains the § 7.23 reference-frame buffer state for a frame-bearing `obu`.
    ///
    /// § 7.23 runs at `decode_frame_wrapup`, the final step of decoding a frame, AFTER the
    /// frame is decoded. To keep later frames' reference checks consistent with that
    /// ordering, each frame's § 7.23 update is *deferred* in [`Self::pending_ref_update`]
    /// and committed at the next frame's coded-frame boundary (or the end-of-bitstream
    /// flush). The segmenter's `boundary` is the coded-frame-unit authority:
    ///
    /// - [`FrameBoundary::OpensNewUnit`]: this OBU opens a NEW coded frame. The previous
    ///   frame is complete, so commit its pending update FIRST (its decode finished),
    ///   then run this frame's reference checks against the post-update buffer, then
    ///   record this frame's own pending update.
    /// - [`FrameBoundary::ContinuesUnit`]: a non-first tile group of the SAME coded
    ///   frame. The frame's update was already derived from its first tile group; nothing
    ///   to do (no double-update, no premature commit).
    /// - [`FrameBoundary::Ambiguous`]: an unreadable frame delimiter — the OBU may open a
    ///   new coded frame or continue one. Commit any pending update (the prior frame is
    ///   done either way) and poison ALL slots: an ambiguous boundary makes this frame's
    ///   refresh effect on the buffer unknowable (the Unknown invariant).
    ///
    /// A `None` boundary is a global frame-bearing OBU (the segmenter ignores globals);
    /// such an OBU is invalid (diagnosed elsewhere) and not part of any coded frame unit,
    /// so it neither commits nor produces a reference-state update.
    pub(super) fn observe_reference_state(
        &mut self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
        report: &mut ValidationReport,
    ) {
        let Some(boundary) = boundary else {
            return;
        };
        match boundary {
            FrameBoundary::ContinuesUnit => {
                // Same coded frame as its first tile group; its pending update already
                // captured the §7.23 effect. Nothing to commit or re-derive.
            }
            FrameBoundary::OpensNewUnit => {
                // The previous coded frame completed: commit its deferred §7.23 update so
                // this frame's reference checks see the post-decode buffer.
                self.commit_pending_ref_update();
                // Run this frame's reference-state checks against the committed buffer
                // (the §6.17.2 show-existing-frame slot-validity diagnostic).
                self.reference_state_checks(obu, first_picture_in_tu, report);
                // Derive and stage this frame's own §7.23 update (committed at the NEXT
                // frame boundary or the end-of-bitstream flush).
                let update = self.derive_ref_update(obu, first_picture_in_tu);
                self.pending_ref_update = Some((obu.header.extended_layer_id, update));
            }
            FrameBoundary::Ambiguous => {
                // The prior frame is done; commit its update. This frame's own refresh
                // effect is unknowable, so stage a poison-all (no reference checks fire —
                // a poisoned buffer proves nothing).
                self.commit_pending_ref_update();
                self.pending_ref_update =
                    Some((obu.header.extended_layer_id, FrameRefUpdate::PoisonAll));
            }
        }
    }

    /// Commits the deferred § 7.23 update (if any) into the reference-state tracker. Used
    /// at each frame boundary that closes the previous coded frame and at the
    /// end-of-bitstream flush (the final frame has no following delimiter).
    pub(super) fn commit_pending_ref_update(&mut self) {
        if let Some((xlayer, update)) = self.pending_ref_update.take() {
            self.reference_state.apply(xlayer, update);
        }
    }

    /// Derives the grounded § 7.23 [`FrameRefUpdate`] for a frame-bearing `obu` from its
    /// parsed core, honestly poisoning when the frame's § 7.23 effect on the buffer cannot
    /// be grounded.
    ///
    /// Only a frame whose `frame_header_info()` parse **completed** stages a § 7.23 update;
    /// everything else poisons all slots. Completion is what establishes the frame's
    /// decodability and the trustworthiness of its slot facts: a parse that stopped past the
    /// prefix ([`FrameHeaderParseStatus::UnsupportedUntilFeature`] — the inter / TIP / bridge
    /// reference-control region this phase does not parse to completion — or any
    /// truncated / coverage stop) may have read its `refresh_frame_flags` / dims correctly
    /// *or* mis-positioned them, and its decodability is unestablished, so the downstream
    /// slot facts could be wrong both ways. Recording a normal § 7.23 [`Refresh`] from such a
    /// frame would assert facts about the buffer the validator has not earned; the sound
    /// treatment is to poison (mask known is not enough — see the
    /// [`FrameRefUpdate::PoisonAll`] contract). The grounded completed statuses are
    /// [`FrameHeaderParseStatus::IntraHeaderComplete`] (an intra header read in full) and
    /// [`FrameHeaderParseStatus::ShowExistingFrameComplete`] (a SEF read in full).
    ///
    /// - A completed show-existing-frame sets `refresh_frame_flags = 0` (§ 5.18.2 :4180), so
    ///   it updates no slot ([`FrameRefUpdate::SefNoUpdate`]).
    /// - A completed CLK that starts a new CVS (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`)
    ///   resets `RefValid[i] = 0` over `0..NumRefFrames` (§ 5.18.2 :4449-4455) then applies
    ///   its own refresh ([`FrameRefUpdate::ClkReset`]).
    /// - Any other **completed** frame whose `refresh_frame_flags`, `frame_type`, dims, and
    ///   order hint all parsed applies the § 7.23 update with the key/switch `first` rule.
    /// - Otherwise (the core did not resolve, an incomplete / unsupported / truncated parse —
    ///   including every inter / TIP / bridge path, which never completes in this phase — or
    ///   any missing fact) the frame's effect on the buffer is unestablished, so poison all
    ///   ([`FrameRefUpdate::PoisonAll`]).
    pub(super) fn derive_ref_update(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> FrameRefUpdate {
        // The core must resolve to the active (== referenced) sequence header, exactly as
        // the output-class / order-hint derivation requires — otherwise the parsed fields
        // were read against a stale header and cannot be trusted (the same guard
        // `frame_celu_facts` uses).
        let Some(core) = self.frame_core_against_referenced_header(obu, first_picture_in_tu) else {
            return FrameRefUpdate::PoisonAll;
        };

        // Only a completed parse may stage a § 7.23 update (see the doc above). An
        // incomplete / unsupported / truncated parse — every inter / TIP / bridge path lands
        // on `UnsupportedUntilFeature` past the prefix — has unestablished decodability and
        // possibly mis-positioned `refresh_frame_flags` / dims, so it poisons even though
        // those fields may be `Some` on the core. This is the gate that stops a normal
        // §7.23 Refresh from an inter frame whose parse never completed.
        if !matches!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete
                | FrameHeaderParseStatus::ShowExistingFrameComplete
        ) {
            return FrameRefUpdate::PoisonAll;
        }

        // A completed show-existing-frame updates no slot (§ 5.18.2 :4180).
        if core.show_existing_frame == Some(true) {
            return FrameRefUpdate::SefNoUpdate;
        }

        // Every grounded update needs the refresh mask, the frame type (for the §7.23
        // RefValid `first` rule), and the stored facts (OrderHint + dims). Any missing
        // fact poisons (the mask could refresh any slot, and a partial store is a guess).
        let (Some(refresh_frame_flags), Some(frame_type)) =
            (core.refresh_frame_flags, core.frame_type)
        else {
            return FrameRefUpdate::PoisonAll;
        };
        let Some(facts) = slot_facts(
            core.order_hint_lsb,
            core.frame_size.map(|size| size.width),
            core.frame_size.map(|size| size.height),
            // AV2 § 7.23 (mirror :14113): RefLongTermId[i] = LongTermId. A KEY frame's
            // LongTermId comes from long_term_id_plus_1 (§5.18.2 mirror :4231-4239); every
            // other frame infers LongTermId == -1 (the "not a long-term frame" sentinel).
            // `core.long_term_id` is `Some(-1)` once the long-term field was reached, so a
            // completed-parse frame always grounds this (the `slot_facts` helper maps a
            // negative or `None` value to the `None` non-long-term sentinel without poisoning).
            core.long_term_id,
        ) else {
            return FrameRefUpdate::PoisonAll;
        };

        // The §5.18.2 CLK reset (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`) clears
        // RefValid[i] over 0..NumRefFrames before the refresh (mirror :4449-4455). The
        // core records `starts_cvs` for exactly this condition.
        if core.starts_cvs && obu.header.obu_type == ObuType::ClosedLoopKey {
            let num_ref_frames = self
                .active_sequence_by_xlayer
                .get(&obu.header.extended_layer_id)
                .and_then(|seq_id| self.sequence_headers.get(seq_id))
                .and_then(|seq| seq.inter.as_ref())
                .map_or(NUM_REF_FRAMES, |inter| usize::from(inter.num_ref_frames));
            return FrameRefUpdate::ClkReset {
                num_ref_frames,
                refresh_frame_flags,
                facts,
            };
        }

        FrameRefUpdate::Refresh {
            refresh_frame_flags,
            is_key_or_switch: is_key_or_switch(frame_type),
            facts,
        }
    }

    /// Emits the reference-state-gated frame-header diagnostics that the modeled § 7.23
    /// buffer makes locally decidable (AV2 § 6.17.2).
    ///
    /// Currently: a show-existing-frame whose `frame_to_show_map_idx` names a slot the
    /// modeled buffer **proves** invalid (`RefValid == 0`); an inter frame whose
    /// explicit-reference-map `ref_frame_idx[i]` names a proven-invalid slot; and a RAS
    /// frame whose explicit-map `ref_frame_idx[i]` selects a slot whose modeled
    /// `RefLongTermId` is not in the RAS frame's own `ref_long_term_id` list
    /// (`long_term_id_in_use(...) == 0`, § 6.17.2 mirror :4615-4616). A *poisoned* (Unknown)
    /// slot drops to silence in all three — the buffer cannot prove a violation there (the
    /// Unknown invariant). The check runs only when the frame's core resolved against its
    /// active sequence header (the parsed indices are trustworthy).
    pub(super) fn reference_state_checks(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        report: &mut ValidationReport,
    ) {
        let Some(core) = self.frame_core_against_referenced_header(obu, first_picture_in_tu) else {
            return;
        };
        // AV2 § 6.17.2 (mirror :4178-4179) / § 7.23: a show-existing-frame outputs the
        // frame stored at `frame_to_show_map_idx`; that reference frame must be valid
        // (`RefValid[ frame_to_show_map_idx ] == 1`). The buffer fires ONLY when it
        // PROVES the slot invalid (a CLK reset with no re-validating refresh since); a
        // poisoned (Unknown) slot stays silent.
        if core.show_existing_frame == Some(true)
            && let Some(idx) = core.frame_to_show_map_idx
            && self
                .reference_state
                .slot(obu.header.extended_layer_id, idx as usize)
                == SlotState::ProvenInvalid
        {
            report.push(frame_header_error(
                "frame-header/show-existing-frame-invalid-slot",
                "6.17.2",
                obu,
                format!(
                    "show-existing-frame references reference slot frame_to_show_map_idx {idx}, \
                     but the §7.23 reference-frame buffer state proves RefValid[{idx}] == 0 \
                     (the slot was invalidated by a CLK reset and not refreshed since)"
                ),
            ));
        }

        // AV2 § 6.17.2 / § 7.23: an inter frame's explicit-reference-map ref_frame_idx[i]
        // (§5.18.2 mirror :4611-4625) names a reference slot that must be valid
        // (`RefValid[ ref_frame_idx[i] ] == 1`). Fire ONLY where the §7.23 buffer PROVES
        // the slot invalid (a CLK reset with no re-validating refresh since); a poisoned
        // (Unknown) slot, or the implicit reference map (`get_ref_frames()`, unmodeled),
        // stays silent.
        if let Some(inter) = core.inter.as_ref() {
            for &idx in &inter.ref_frame_idx {
                if self
                    .reference_state
                    .slot(obu.header.extended_layer_id, idx as usize)
                    == SlotState::ProvenInvalid
                {
                    report.push(frame_header_error(
                        "frame-header/ref-frame-idx-invalid-slot",
                        "6.17.2",
                        obu,
                        format!(
                            "inter frame ref_frame_idx names reference slot {idx}, but the §7.23 \
                             reference-frame buffer state proves RefValid[{idx}] == 0 (the slot \
                             was invalidated by a CLK reset and not refreshed since)"
                        ),
                    ));
                    // One diagnostic per frame is enough to flag the defect.
                    break;
                }
            }
        }

        // AV2 § 6.17.2 (mirror :4638-4644): "Once the frame size has been determined, it is a
        // requirement of bitstream conformance that all the following conditions are satisfied
        // for i=0..NumTotalRefs-1:
        //   2 * FrameWidth  >= RefFrameWidth [ ref_frame_idx[ i ] ]
        //   2 * FrameHeight >= RefFrameHeight[ ref_frame_idx[ i ] ]
        //       FrameWidth  <= 16 * RefFrameWidth [ ref_frame_idx[ i ] ]
        //       FrameHeight <= 16 * RefFrameHeight[ ref_frame_idx[ i ] ]"
        // (§6.17.4.3 mirror :5251-5258 restates the same four inequalities over the full
        // 0..REFS_PER_FRAME-1 reference set; the validator models only the explicit map's
        // ref_frame_idx[0..NumTotalRefs], so the implicit-map slots beyond NumTotalRefs — which
        // need the unmodeled get_ref_frames() derivation — are a named residual.)
        //
        // Decidable once the frame size has been determined — §6.17.2 (mirror :4638) gates the
        // constraint on exactly that — so the resolved FrameWidth/FrameHeight on `core.frame_size`
        // (parsed via the §5.18.4 frame-size syntax) AND the referenced slot being PROVEN valid so
        // RefFrameWidth/RefFrameHeight are known (`SlotState::Valid`) are both required. An
        // Unknown / ProvenInvalid slot has no proven dims and drops to silence (the Unknown
        // invariant; a ProvenInvalid slot is already flagged by the ref-frame-idx check above).
        // The implicit reference map (get_ref_frames(), unmodeled) records no ref_frame_idx and
        // is silent. A reference used to *derive* the size satisfies the bounds trivially
        // (FrameWidth == RefFrameWidth there); the constraint bites the other references.
        if let Some(inter) = core.inter.as_ref()
            && let Some(size) = core.frame_size
        {
            for &idx in &inter.ref_frame_idx {
                let SlotState::Valid(facts) = self
                    .reference_state
                    .slot(obu.header.extended_layer_id, idx as usize)
                else {
                    continue;
                };
                // RefFrame*/FrameWidth are u32; the 2x and 16x products can overflow. Saturate:
                // a saturated lower bound (2*FrameWidth -> u32::MAX) still dominates any u32 ref
                // (no violation), and a saturated upper bound (16*RefFrame* -> u32::MAX) still
                // dominates any u32 frame dim (no violation) — so saturation never invents a
                // violation, it only suppresses one that overflow would otherwise misjudge.
                let two_width = size.width.saturating_mul(2);
                let two_height = size.height.saturating_mul(2);
                let max_width = facts.width.saturating_mul(16);
                let max_height = facts.height.saturating_mul(16);
                let violation = if two_width < facts.width {
                    Some(format!(
                        "2*FrameWidth ({two_width}) < RefFrameWidth[{idx}] ({}) — an inter frame \
                         may upscale a reference by at most 2x in width",
                        facts.width
                    ))
                } else if two_height < facts.height {
                    Some(format!(
                        "2*FrameHeight ({two_height}) < RefFrameHeight[{idx}] ({}) — an inter \
                         frame may upscale a reference by at most 2x in height",
                        facts.height
                    ))
                } else if size.width > max_width {
                    Some(format!(
                        "FrameWidth ({}) > 16*RefFrameWidth[{idx}] ({max_width}) — an inter frame \
                         may downscale a reference by at most 16x in width",
                        size.width
                    ))
                } else if size.height > max_height {
                    Some(format!(
                        "FrameHeight ({}) > 16*RefFrameHeight[{idx}] ({max_height}) — an inter \
                         frame may downscale a reference by at most 16x in height",
                        size.height
                    ))
                } else {
                    None
                };
                if let Some(detail) = violation {
                    report.push(frame_header_error(
                        "frame-header/ref-frame-scale-ratio",
                        "6.17.2",
                        obu,
                        format!(
                            "inter frame reference scaling is out of range: {detail} (§6.17.2 \
                             requires every ref_frame_idx[i] reference to satisfy \
                             FrameWidth/16 <= RefFrameWidth <= 2*FrameWidth and likewise for \
                             height, once the frame size is determined)"
                        ),
                    ));
                    // One diagnostic per frame is enough to flag the defect.
                    break;
                }
            }
        }

        // AV2 § 6.17.2 (mirror :4594-4595): when use_bru == 1, "RefFrameWidth[ ref_frame_idx[
        // bru_ref ] ] is equal to FrameWidth" and "RefFrameHeight[ ref_frame_idx[ bru_ref ] ] is
        // equal to FrameHeight" — a backward-reference-update frame must match the dimensions of
        // the reference it updates. Decidable from the same modeled state as the scaling ratio:
        // the resolved current size (`core.frame_size`) and the proven-valid bru_ref slot's stored
        // dims (`SlotState::Valid`). The other use_bru == 1 conformance items are either already
        // checked (immediate_output_frame == 1 -> frame-header/bru-without-immediate-output;
        // bru_ref < NumTotalRefs -> frame-header/bru-ref-out-of-range; the refresh-mask-bit ->
        // frame-header/bru-ref-refresh-flag-unset below) or need the unmodeled get_ref_frames()
        // RefOrderHint derivation (OrderHint >= RefOrderHint[i], the RESTRICTED_OH item) and stay
        // residual. An Unknown / ProvenInvalid bru_ref slot has no proven dims and drops to
        // silence; bru_ref is
        // bounds-checked against the recorded ref_frame_idx (a separate bru-ref-out-of-range home).
        if let Some(inter) = core.inter.as_ref()
            && inter.use_bru == Some(true)
            && let Some(size) = core.frame_size
            && let Some(bru_ref) = inter.bru_ref
            && let Some(&idx) = inter.ref_frame_idx.get(bru_ref as usize)
            && let SlotState::Valid(facts) = self
                .reference_state
                .slot(obu.header.extended_layer_id, idx as usize)
            && (facts.width != size.width || facts.height != size.height)
        {
            report.push(frame_header_error(
                "frame-header/bru-ref-frame-size-mismatch",
                "6.17.2",
                obu,
                format!(
                    "use_bru == 1 backward-reference-update frame size {}x{} does not match its \
                     bru_ref reference (ref_frame_idx[bru_ref={bru_ref}] -> slot {idx}) dimensions \
                     {}x{} (§6.17.2 requires RefFrameWidth/RefFrameHeight[ref_frame_idx[bru_ref]] \
                     == FrameWidth/FrameHeight — a BRU frame must match the reference it updates)",
                    size.width, size.height, facts.width, facts.height
                ),
            ));
        }

        // AV2 § 6.17.2 (mirror :4596): when use_bru == 1, "The value of refresh_frame_flags &
        // (1 << ref_frame_idx[ bru_ref ]) must be non-zero" — a backward-reference-update frame
        // must refresh the very slot it updates. Decidable from parsed header state alone:
        // refresh_frame_flags (read on the inter path, inter.rs:477), bru_ref, and
        // ref_frame_idx[bru_ref] — no reference-state lookup (unlike the dims / RefOrderHint
        // clauses). bru_ref is bounds-checked against the recorded ref_frame_idx, and the shift
        // is guarded against an out-of-range slot index (one already flagged by
        // frame-header/ref-frame-idx-invalid-slot), so it cannot panic.
        if let Some(inter) = core.inter.as_ref()
            && inter.use_bru == Some(true)
            && let Some(refresh_frame_flags) = inter.refresh_frame_flags
            && let Some(bru_ref) = inter.bru_ref
            && let Some(&idx) = inter.ref_frame_idx.get(bru_ref as usize)
            && idx < u32::BITS
            && (refresh_frame_flags & (1u32 << idx)) == 0
        {
            report.push(frame_header_error(
                "frame-header/bru-ref-refresh-flag-unset",
                "6.17.2",
                obu,
                format!(
                    "use_bru == 1 backward-reference-update frame does not refresh its bru_ref \
                     reference: refresh_frame_flags {refresh_frame_flags:#x} has the \
                     ref_frame_idx[bru_ref={bru_ref}] = slot {idx} bit clear (§6.17.2 requires \
                     refresh_frame_flags & (1 << ref_frame_idx[bru_ref]) != 0 — a BRU frame must \
                     refresh the reference it updates)"
                ),
            ));
        }

        // AV2 § 6.17.2 (mirror :4615-4616): "If obu_type is equal to OBU_RAS_FRAME, it is a
        // requirement of bitstream conformance that
        // long_term_id_in_use( RefLongTermId[ ref_frame_idx[ i ] ] ) is equal to 1." A RAS
        // frame may reference ONLY long-term reference frames whose RefLongTermId appears in
        // its own ref_long_term_id list (§7.4.5). `long_term_id_in_use(longTermId)` (mirror
        // :5529-5536) returns 1 iff `longTermId` equals some `ref_long_term_id[j]`.
        //
        // Decidable when: (1) this is a RAS frame; (2) the explicit reference map recorded
        // `ref_frame_idx`; (3) the selected slot is PROVEN valid in the §7.23 buffer so its
        // RefLongTermId is known. A slot the buffer cannot prove valid (Unknown /
        // ProvenInvalid) yields `None` from `slot_long_term_id` and DROPS to silence (the
        // Unknown invariant — and a ProvenInvalid slot is already flagged by the
        // ref-frame-idx check above).
        //
        // REACHABILITY RESIDUAL: condition (2) holds only for a RAS frame with
        // `max_mlayer_id != 0`. For `max_mlayer_id == 0` (a single-embedded-layer stream) the
        // §5.18.2 RAS `refresh_frame_flags` derivation reads RefValid/RefLongTermId (mirror
        // :4493), which the inter parser cannot ground, so it stops with
        // InterStop::UnmodeledDerivation BEFORE `ref_frame_idx` (inter.rs) — `core.inter`
        // then records no `ref_frame_idx` and this check is silent. So the rule fires only for
        // the multistream (`max_mlayer_id != 0`) RAS case; the single-layer case is an honest
        // under-report (no false positive — the loop simply has nothing to evaluate).
        //
        // A proven-valid slot whose RefLongTermId is the `-1`
        // sentinel (`Some(None)`: refreshed by a non-KEY frame, so not a long-term frame) is
        // never `long_term_id_in_use`, since every `ref_long_term_id[j]` is `>= 0` — a RAS
        // selecting it is a defect. A proven-valid long-term slot (`Some(Some(id))`) must
        // have its `id` listed. Anchored at the RAS frame's OBU, one diagnostic per frame.
        if obu.header.obu_type == ObuType::RasFrame
            && let Some(inter) = core.inter.as_ref()
        {
            for &idx in &inter.ref_frame_idx {
                // Only a PROVEN-valid slot's RefLongTermId is decidable; Unknown /
                // ProvenInvalid drops to silence.
                let Some(slot_long_term_id) = self
                    .reference_state
                    .slot_long_term_id(obu.header.extended_layer_id, idx as usize)
                else {
                    continue;
                };
                // long_term_id_in_use: the slot's RefLongTermId must equal some listed
                // ref_long_term_id[j]. A `-1` (None) sentinel is never in the list.
                let in_use =
                    slot_long_term_id.is_some_and(|id| core.ref_long_term_ids.contains(&id));
                if !in_use {
                    let slot_desc = match slot_long_term_id {
                        Some(id) => format!("RefLongTermId {id}"),
                        None => "RefLongTermId -1 (not a long-term reference frame)".to_owned(),
                    };
                    report.push(frame_header_error(
                        "frame-header/ras-ref-long-term-id-not-in-use",
                        "6.17.2",
                        obu,
                        format!(
                            "OBU_RAS_FRAME ref_frame_idx selects reference slot {idx} whose \
                             modeled {slot_desc} is not in the frame's ref_long_term_id list \
                             {:?} (§6.17.2: long_term_id_in_use(RefLongTermId[ref_frame_idx[i]]) \
                             must equal 1 — a RAS frame may reference only long-term frames it \
                             lists)",
                            core.ref_long_term_ids
                        ),
                    ));
                    // One diagnostic per frame is enough to flag the defect.
                    break;
                }
            }
        }
    }
}
