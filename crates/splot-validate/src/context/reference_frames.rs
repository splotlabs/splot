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
            FrameBoundary::ContinuesUnit => {}
            FrameBoundary::OpensNewUnit => {
                self.commit_pending_ref_update();
                self.reference_state_checks(obu, first_picture_in_tu, report);
                let update = self.derive_ref_update(obu, first_picture_in_tu);
                self.pending_ref_update = Some((obu.header.extended_layer_id, update));
            }
            FrameBoundary::Ambiguous => {
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
    /// prefix ([`FrameHeaderParseStatus::UnsupportedUntilFeature`] or any truncated /
    /// coverage stop) may have read its `refresh_frame_flags` / dims correctly
    /// *or* mis-positioned them, and its decodability is unestablished, so the downstream
    /// slot facts could be wrong both ways. Recording a normal § 7.23 [`Refresh`] from such a
    /// frame would assert facts about the buffer the validator has not earned; the sound
    /// treatment is to poison (mask known is not enough — see the
    /// [`FrameRefUpdate::PoisonAll`] contract). The grounded completed statuses are
    /// [`FrameHeaderParseStatus::IntraHeaderComplete`] (an intra header read in full),
    /// [`FrameHeaderParseStatus::InterHeaderComplete`] (an inter or TIP-output header), and
    /// [`FrameHeaderParseStatus::ShowExistingFrameComplete`] (a SEF read in full).
    ///
    /// - A completed show-existing-frame sets `refresh_frame_flags = 0` (§ 5.18.2 :4180), so
    ///   it updates no slot ([`FrameRefUpdate::SefNoUpdate`]).
    /// - A completed CLK that starts a new CVS (`OBU_CLOSED_LOOP_KEY && FirstPictureInTU`)
    ///   resets `RefValid[i] = 0` over `0..NumRefFrames` (§ 5.18.2 :4449-4455) then applies
    ///   its own refresh ([`FrameRefUpdate::ClkReset`]).
    /// - Any other **completed** frame whose `refresh_frame_flags`, `frame_type`, dims, and
    ///   order hint all parsed applies the § 7.23 update with the key/switch `first` rule.
    /// - Otherwise (the core did not resolve, an incomplete / unsupported / truncated parse,
    ///   or any missing fact) the frame's effect on the buffer is unestablished, so poison all
    ///   ([`FrameRefUpdate::PoisonAll`]).
    pub(super) fn derive_ref_update(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> FrameRefUpdate {
        let Some(core) = self.frame_core_against_referenced_header(obu, first_picture_in_tu) else {
            return FrameRefUpdate::PoisonAll;
        };

        if !matches!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete
                | FrameHeaderParseStatus::InterHeaderComplete
                | FrameHeaderParseStatus::ShowExistingFrameComplete
        ) {
            return FrameRefUpdate::PoisonAll;
        }

        if core.show_existing_frame == Some(true) {
            return FrameRefUpdate::SefNoUpdate;
        }

        let (Some(refresh_frame_flags), Some(frame_type)) =
            (core.refresh_frame_flags, core.frame_type)
        else {
            return FrameRefUpdate::PoisonAll;
        };
        let quantizer = core
            .quantization_params
            .map(|quant| (quant.base_q_idx, quant.delta_q_u_ac, quant.delta_q_v_ac))
            .or_else(|| {
                infer_tip_output_quantizer(
                    &core,
                    obu.header.extended_layer_id,
                    &self.reference_state,
                )
            });
        let Some(facts) = slot_facts(
            (core.order_hint, core.order_hint_lsb),
            (
                core.frame_size.map(|size| size.width),
                core.frame_size.map(|size| size.height),
            ),
            quantizer.map(|values| values.0),
            (
                quantizer.map(|values| values.1),
                quantizer.map(|values| values.2),
            ),
            (core.implicit_output_frame, core.immediate_output_frame),
            core.frame_type,
            core.long_term_id,
        ) else {
            return FrameRefUpdate::PoisonAll;
        };

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
                    break;
                }
            }
        }

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
                    break;
                }
            }
        }

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

        if obu.header.obu_type == ObuType::RasFrame
            && let Some(inter) = core.inter.as_ref()
        {
            for &idx in &inter.ref_frame_idx {
                let Some(slot_long_term_id) = self
                    .reference_state
                    .slot_long_term_id(obu.header.extended_layer_id, idx as usize)
                else {
                    continue;
                };
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
                    break;
                }
            }
        }
    }
}

/// Infers `base_q_idx`, `DeltaQUAc`, and `DeltaQVAc` for a TIP-as-output frame when
/// `enable_tip_explicit_qp == 0`
/// (AV2 v1.0.0 § 5.18.2, mirror :5105-5112). The reference list indices follow the
/// closest-past/closest-future selection of § 7.8; when no future reference exists, the
/// second-closest past reference is used by the motion-field-estimation process (§ 7.9.1).
fn infer_tip_output_quantizer(
    core: &FrameHeaderCore,
    xlayer: ExtendedLayerId,
    reference_state: &ReferenceStateTracker,
) -> Option<(u32, i32, i32)> {
    let inter = core.inter.as_ref()?;
    if inter.tip_frame_mode != Some(TipFrameMode::AsOutput) {
        return None;
    }

    let current_order_hint = i32::try_from(core.order_hint?).ok()?;
    let mut closest_past: Option<(i32, SlotFacts)> = None;
    let mut second_past: Option<(i32, SlotFacts)> = None;
    let mut closest_future: Option<(i32, SlotFacts)> = None;

    for &slot in &inter.ref_frame_idx {
        let SlotState::Valid(facts) = reference_state.slot(xlayer, slot as usize) else {
            continue;
        };
        let distance = get_relative_dist(current_order_hint, i32::try_from(facts.order_hint).ok()?);
        if distance > 0 {
            let candidate = (distance, facts);
            if closest_past.is_none_or(|(old, _)| distance < old) {
                second_past = closest_past;
                closest_past = Some(candidate);
            } else if second_past.is_none_or(|(old, _)| distance < old) {
                second_past = Some(candidate);
            }
        } else if distance < 0 && closest_future.is_none_or(|(old, _)| distance > old) {
            closest_future = Some((distance, facts));
        }
    }

    let (_, past) = closest_past?;
    let (_, future) = closest_future.or(second_past)?;
    Some((
        u32::try_from((u64::from(past.base_q_idx) + u64::from(future.base_q_idx) + 1) >> 1).ok()?,
        ((i64::from(past.delta_q_u_ac) + i64::from(future.delta_q_u_ac) + 1) >> 1)
            .try_into()
            .ok()?,
        ((i64::from(past.delta_q_v_ac) + i64::from(future.delta_q_v_ac) + 1) >> 1)
            .try_into()
            .ok()?,
    ))
}
