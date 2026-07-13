// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier AV2 § 7.23 reference-frame buffer state.
//!
//! The buffer tracks decoded-frame indices plus the per-slot metadata consumed by
//! the next inter frame's [`super::inter::InterReferenceState`].
//!
//! Feature tracking: `DECODE-INTER-MULTIREF-RUNTIME`.

use splot_core::headers::frame::{
    CcsoParams, GlobalMotionRef, SavedGlobalMotionOrderHints, SavedGlobalMotionParams,
};
use splot_core::types::{EmbeddedLayerId, ObuType};
use splot_recon::{DecodedFrame, ReferenceFrameStore, ReferenceSlot};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::error::{DecodeReferenceStateError, Result};
use crate::filters::ccso::CcsoUnitGrid;
use crate::pipeline::ActiveFilmGrain;
use crate::pipeline::PipelineFrame;
use crate::prediction::inter::TemporalMotionField;

#[derive(Clone, Debug)]
struct Slot {
    valid: bool,
    order_hint: u32,
    order_hint_lsb: u32,
    implicit_output_frame: bool,
    immediate_output_frame: bool,
    width: u32,
    height: u32,
    base_q_idx: u32,
    counter: u32,
    delta_q_u_ac: i32,
    delta_q_v_ac: i32,
    is_inter: bool,
    adapted: bool,
    num_total_refs: u32,
    saved_order_hints: SavedGlobalMotionOrderHints,
    saved_gm_params: SavedGlobalMotionParams,
    lr_frame_filter_class_counts: [u8; 3],
    lr_frame_filter_taps: [Vec<Vec<i16>>; 3],
    frame_index: Option<usize>,
    frame_cdfs: Option<FrameCdfSubset>,
    ccso_params: Option<CcsoParams>,
    ccso_grid: Option<CcsoUnitGrid>,
    motion_field: Option<TemporalMotionField>,
    long_term_id: Option<u32>,
    display_grain: Option<ActiveFilmGrain>,
    embedded_layer_id: EmbeddedLayerId,
    output_done: bool,
}

impl Slot {
    const EMPTY: Self = Self {
        valid: false,
        order_hint: 0,
        order_hint_lsb: 0,
        implicit_output_frame: false,
        immediate_output_frame: false,
        width: 0,
        height: 0,
        base_q_idx: 0,
        counter: 0,
        delta_q_u_ac: 0,
        delta_q_v_ac: 0,
        is_inter: false,
        adapted: false,
        num_total_refs: 0,
        saved_order_hints: [0; 7],
        saved_gm_params: [GlobalMotionRef::identity().gm_params; 7],
        lr_frame_filter_class_counts: [0; 3],
        lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
        frame_index: None,
        frame_cdfs: None,
        ccso_params: None,
        ccso_grid: None,
        motion_field: None,
        long_term_id: None,
        display_grain: None,
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        output_done: false,
    };

    fn refresh(&mut self, frame_index: usize, update: &FrameRefUpdate, valid: bool, counter: u32) {
        *self = Self {
            valid,
            order_hint: update.order_hint,
            order_hint_lsb: update.order_hint_lsb,
            implicit_output_frame: update.implicit_output_frame,
            immediate_output_frame: update.immediate_output_frame,
            width: update.width,
            height: update.height,
            base_q_idx: update.base_q_idx,
            counter,
            delta_q_u_ac: update.delta_q_u_ac,
            delta_q_v_ac: update.delta_q_v_ac,
            is_inter: update.is_inter,
            adapted: update.adapted,
            num_total_refs: update.num_total_refs,
            saved_order_hints: update.saved_order_hints,
            saved_gm_params: update.saved_gm_params,
            lr_frame_filter_class_counts: update.lr_frame_filter_class_counts,
            lr_frame_filter_taps: update.lr_frame_filter_taps.clone(),
            frame_index: Some(frame_index),
            frame_cdfs: Some(update.frame_cdfs.clone()),
            ccso_params: update.ccso_params.clone(),
            ccso_grid: update.ccso_grid.clone(),
            motion_field: Some(update.motion_field.clone()),
            long_term_id: update.long_term_id,
            display_grain: None,
            embedded_layer_id: update.embedded_layer_id,
            output_done: false,
        };
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRefUpdate {
    pub(crate) refresh_frame_flags: u32,
    pub(crate) order_hint: u32,
    pub(crate) order_hint_lsb: u32,
    pub(crate) implicit_output_frame: bool,
    pub(crate) immediate_output_frame: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) base_q_idx: u32,
    pub(crate) delta_q_u_ac: i32,
    pub(crate) delta_q_v_ac: i32,
    pub(crate) is_key_or_switch: bool,
    pub(crate) is_inter: bool,
    pub(crate) adapted: bool,
    pub(crate) num_total_refs: u32,
    pub(crate) saved_order_hints: SavedGlobalMotionOrderHints,
    pub(crate) saved_gm_params: SavedGlobalMotionParams,
    pub(crate) lr_frame_filter_class_counts: [u8; 3],
    pub(crate) lr_frame_filter_taps: [Vec<Vec<i16>>; 3],
    pub(crate) frame_cdfs: FrameCdfSubset,
    pub(crate) ccso_params: Option<CcsoParams>,
    pub(crate) ccso_grid: Option<CcsoUnitGrid>,
    pub(crate) motion_field: TemporalMotionField,
    pub(crate) long_term_id: Option<u32>,
    pub(crate) embedded_layer_id: EmbeddedLayerId,
}

#[derive(Clone, Debug)]
struct OpenLoopState {
    refresh_frame_flags: u32,
    co_vcl_refresh_frame_flags: u32,
    ref_long_term_ids: Vec<u32>,
}

pub(crate) struct RuntimeReferenceBuffer {
    slots: Vec<Slot>,
    frame_counter: u32,
    started: bool,
    open_loop: Option<OpenLoopState>,
}

impl RuntimeReferenceBuffer {
    pub(crate) fn new(num_ref_frames: usize) -> Result<Self> {
        ReferenceFrameStore::<()>::with_capacity(num_ref_frames)?;
        Ok(Self {
            slots: vec![Slot::EMPTY; num_ref_frames],
            frame_counter: 0,
            started: false,
            open_loop: None,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn prepare_for_frame(&mut self, obu_type: ObuType, first_picture_in_tu: bool) {
        if first_picture_in_tu && (obu_type == ObuType::OpenLoopKey || is_regular_non_olk(obu_type))
        {
            self.finish_open_loop();
        }
    }

    pub(crate) fn note_frame(
        &mut self,
        obu_type: ObuType,
        first_picture_in_tu: bool,
        update: &FrameRefUpdate,
        ref_long_term_ids: &[u32],
    ) {
        if obu_type == ObuType::OpenLoopKey {
            self.open_loop = Some(OpenLoopState {
                refresh_frame_flags: update.refresh_frame_flags,
                co_vcl_refresh_frame_flags: 0,
                ref_long_term_ids: ref_long_term_ids.to_vec(),
            });
        } else if !first_picture_in_tu
            && is_regular_non_olk(obu_type)
            && let Some(open_loop) = self.open_loop.as_mut()
        {
            open_loop.co_vcl_refresh_frame_flags |= update.refresh_frame_flags;
        }
    }

    fn finish_open_loop(&mut self) {
        let Some(open_loop) = self.open_loop.take() else {
            return;
        };
        let refreshed = open_loop.refresh_frame_flags | open_loop.co_vcl_refresh_frame_flags;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let retained_by_refresh = (refreshed >> index) & 1 != 0;
            let retained_long_term = slot
                .long_term_id
                .is_some_and(|id| open_loop.ref_long_term_ids.contains(&id));
            if !retained_by_refresh && !retained_long_term {
                slot.valid = false;
            }
        }
    }

    pub(crate) fn update(&mut self, frame_index: usize, update: &FrameRefUpdate) {
        self.advance_frame_counter();
        let mut first = true;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if (update.refresh_frame_flags >> i) & 1 == 0 {
                continue;
            }
            let valid = !update.is_key_or_switch || first;
            first = false;
            slot.refresh(frame_index, update, valid, self.frame_counter);
        }
    }

    pub(crate) fn note_show_existing(&mut self) {
        self.advance_frame_counter();
    }

    fn advance_frame_counter(&mut self) {
        if self.started {
            self.frame_counter = self.frame_counter.wrapping_add(1);
        }
        self.started = true;
    }

    pub(crate) fn save_grain_for_refreshed_slots(
        &mut self,
        refresh_frame_flags: u32,
        display_grain: Option<&ActiveFilmGrain>,
    ) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if (refresh_frame_flags >> index) & 1 != 0 {
                slot.display_grain = display_grain.cloned();
            }
        }
    }

    pub(crate) fn save_grain_for_slot(
        &mut self,
        slot: u32,
        display_grain: Option<ActiveFilmGrain>,
    ) -> Result<()> {
        let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
        let slot_count = self.slots.len();
        let stored =
            self.slots
                .get_mut(slot_index)
                .ok_or(DecodeReferenceStateError::SlotOutOfRange {
                    slot: slot_index,
                    slot_count,
                })?;
        if !stored.valid {
            return Err(DecodeReferenceStateError::MissingFrame { slot: slot_index }.into());
        }
        stored.display_grain = display_grain;
        Ok(())
    }

    pub(crate) fn restrict_references_for_switch(
        &mut self,
        current_layer: EmbeddedLayerId,
        presence: &splot_core::headers::sequence::MLayerPresenceMap,
    ) -> Vec<usize> {
        let mut restricted = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if presence.is_present(current_layer, slot.embedded_layer_id) {
                slot.order_hint = u32::MAX;
                restricted.push(index);
            }
        }
        restricted
    }

    pub(crate) fn retains_hidden_long_term_reference(
        &self,
        long_term_id: u32,
        frame_index: usize,
    ) -> bool {
        self.slots.iter().any(|slot| {
            slot.valid
                && slot.long_term_id == Some(long_term_id)
                && slot.frame_index == Some(frame_index)
                && !slot.immediate_output_frame
                && !slot.implicit_output_frame
        })
    }

    pub(crate) fn build_store_eight<'a>(
        &self,
        frames: &'a [Option<PipelineFrame>],
    ) -> Result<(ReferenceFrameStore<&'a DecodedFrame<u8>>, ReferenceMetadata)> {
        self.build_store(frames, PipelineFrame::frame_eight)
    }

    pub(crate) fn build_store_ten<'a>(
        &self,
        frames: &'a [Option<PipelineFrame>],
    ) -> Result<(
        ReferenceFrameStore<&'a DecodedFrame<u16>>,
        ReferenceMetadata,
    )> {
        self.build_store(frames, PipelineFrame::frame_ten)
    }

    fn build_store<'a, T: splot_recon::ReconSample>(
        &self,
        frames: &'a [Option<PipelineFrame>],
        frame_view: impl Fn(&'a PipelineFrame) -> Result<&'a DecodedFrame<T>>,
    ) -> Result<(ReferenceFrameStore<&'a DecodedFrame<T>>, ReferenceMetadata)> {
        let num = self.slots.len();
        let mut store: ReferenceFrameStore<&'a DecodedFrame<T>> =
            ReferenceFrameStore::with_capacity(num)?;
        let mut meta = ReferenceMetadata::with_capacity(num);
        for (i, slot) in self.slots.iter().enumerate() {
            meta.push_slot(slot);
            if !slot.valid {
                continue;
            }
            let frame_index = slot
                .frame_index
                .ok_or(DecodeReferenceStateError::MissingFrame { slot: i })?;
            let frame = frames
                .get(frame_index)
                .ok_or(DecodeReferenceStateError::FrameIndexOutOfRange {
                    slot: i,
                    frame_index,
                    frame_count: frames.len(),
                })?
                .as_ref()
                .ok_or(DecodeReferenceStateError::MissingFrame { slot: i })?;
            let reference_slot = ReferenceSlot::new(i)?;
            let frame = frame_view(frame)?;
            ensure_slot_matches_frame(i, slot, frame_index, frame)?;
            store.put(reference_slot, frame)?;
        }
        Ok((store, meta))
    }

    pub(crate) fn retains(&self, frame_index: usize) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.valid && slot.frame_index == Some(frame_index))
    }

    pub(crate) fn frame_index_for_slot(&self, slot: u32) -> Result<usize> {
        let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
        if slot_index >= self.slots.len() {
            return Err(DecodeReferenceStateError::SlotOutOfRange {
                slot: slot_index,
                slot_count: self.slots.len(),
            }
            .into());
        }
        let slot = &self.slots[slot_index];
        if !slot.valid {
            return Err(DecodeReferenceStateError::MissingFrame { slot: slot_index }.into());
        }
        slot.frame_index
            .ok_or_else(|| DecodeReferenceStateError::MissingFrame { slot: slot_index }.into())
    }

    pub(crate) fn mark_sef_derive_output(
        &mut self,
        slot: u32,
        source_already_output: bool,
    ) -> Result<()> {
        let slot_index = usize::try_from(slot).unwrap_or(usize::MAX);
        let slot_count = self.slots.len();
        let stored =
            self.slots
                .get_mut(slot_index)
                .ok_or(DecodeReferenceStateError::SlotOutOfRange {
                    slot: slot_index,
                    slot_count,
                })?;
        if !stored.valid {
            return Err(DecodeReferenceStateError::MissingFrame { slot: slot_index }.into());
        }
        if source_already_output
            || stored.output_done
            || stored.implicit_output_frame
            || stored.immediate_output_frame
        {
            return Err(DecodeReferenceStateError::ShowExistingFrameIneligible {
                slot: slot_index,
            }
            .into());
        }
        stored.output_done = true;
        Ok(())
    }
}

fn is_regular_non_olk(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::RegularSef
            | ObuType::RegularTip
            | ObuType::Switch
            | ObuType::RasFrame
            | ObuType::BridgeFrame
            | ObuType::RegularTileGroup
    )
}

fn ensure_slot_matches_frame<T: splot_recon::ReconSample>(
    slot_index: usize,
    slot: &Slot,
    frame_index: usize,
    frame: &DecodedFrame<T>,
) -> core::result::Result<(), DecodeReferenceStateError> {
    let size = frame.coded_luma_size();
    let expected_width = slot.width;
    let expected_height = slot.height;
    let actual_width = size.width();
    let actual_height = size.height();
    if u32::try_from(actual_width) != Ok(expected_width)
        || u32::try_from(actual_height) != Ok(expected_height)
    {
        return Err(DecodeReferenceStateError::FrameSizeMismatch {
            slot: slot_index,
            frame_index,
            expected_width,
            expected_height,
            actual_width,
            actual_height,
        });
    }
    Ok(())
}

#[allow(clippy::struct_field_names)]
pub(crate) struct ReferenceMetadata {
    pub(crate) ref_valid: Vec<bool>,
    pub(crate) ref_order_hint: Vec<u32>,
    pub(crate) ref_order_hint_lsbs: Vec<u32>,
    pub(crate) ref_implicit_output_frame: Vec<bool>,
    pub(crate) ref_immediate_output_frame: Vec<bool>,
    pub(crate) ref_frame_width: Vec<u32>,
    pub(crate) ref_frame_height: Vec<u32>,
    pub(crate) ref_base_q_idx: Vec<u32>,
    pub(crate) ref_counter: Vec<u32>,
    pub(crate) ref_delta_q_u_ac: Vec<i32>,
    pub(crate) ref_delta_q_v_ac: Vec<i32>,
    pub(crate) ref_is_inter: Vec<bool>,
    pub(crate) ref_long_term_id: Vec<Option<u32>>,
    pub(crate) ref_adapted: Vec<bool>,
    pub(crate) ref_num_total_refs: Vec<u32>,
    pub(crate) saved_global_motion_order_hints: Vec<SavedGlobalMotionOrderHints>,
    pub(crate) saved_global_motion_params: Vec<SavedGlobalMotionParams>,
    pub(crate) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    pub(crate) lr_frame_filter_taps: Vec<[Vec<Vec<i16>>; 3]>,
    pub(crate) ref_frame_cdfs: Vec<Option<FrameCdfSubset>>,
    pub(crate) ref_ccso_params: Vec<Option<CcsoParams>>,
    pub(crate) ref_ccso_unit_grids: Vec<Option<CcsoUnitGrid>>,
    pub(crate) ref_motion_fields: Vec<Option<TemporalMotionField>>,
}

impl ReferenceMetadata {
    fn with_capacity(num: usize) -> Self {
        Self {
            ref_valid: Vec::with_capacity(num),
            ref_order_hint: Vec::with_capacity(num),
            ref_order_hint_lsbs: Vec::with_capacity(num),
            ref_implicit_output_frame: Vec::with_capacity(num),
            ref_immediate_output_frame: Vec::with_capacity(num),
            ref_frame_width: Vec::with_capacity(num),
            ref_frame_height: Vec::with_capacity(num),
            ref_base_q_idx: Vec::with_capacity(num),
            ref_counter: Vec::with_capacity(num),
            ref_delta_q_u_ac: Vec::with_capacity(num),
            ref_delta_q_v_ac: Vec::with_capacity(num),
            ref_is_inter: Vec::with_capacity(num),
            ref_long_term_id: Vec::with_capacity(num),
            ref_adapted: Vec::with_capacity(num),
            ref_num_total_refs: Vec::with_capacity(num),
            saved_global_motion_order_hints: Vec::with_capacity(num),
            saved_global_motion_params: Vec::with_capacity(num),
            lr_frame_filter_class_counts: Vec::with_capacity(num),
            lr_frame_filter_taps: Vec::with_capacity(num),
            ref_frame_cdfs: Vec::with_capacity(num),
            ref_ccso_params: Vec::with_capacity(num),
            ref_ccso_unit_grids: Vec::with_capacity(num),
            ref_motion_fields: Vec::with_capacity(num),
        }
    }

    fn push_slot(&mut self, slot: &Slot) {
        self.ref_valid.push(slot.valid);
        self.ref_order_hint.push(slot.order_hint);
        self.ref_order_hint_lsbs.push(slot.order_hint_lsb);
        self.ref_implicit_output_frame
            .push(slot.implicit_output_frame);
        self.ref_immediate_output_frame
            .push(slot.immediate_output_frame);
        self.ref_frame_width.push(slot.width);
        self.ref_frame_height.push(slot.height);
        self.ref_base_q_idx.push(slot.base_q_idx);
        self.ref_counter.push(slot.counter);
        self.ref_delta_q_u_ac.push(slot.delta_q_u_ac);
        self.ref_delta_q_v_ac.push(slot.delta_q_v_ac);
        self.ref_is_inter.push(slot.is_inter);
        self.ref_long_term_id.push(slot.long_term_id);
        self.ref_adapted.push(slot.adapted);
        self.ref_num_total_refs.push(slot.num_total_refs);
        self.saved_global_motion_order_hints
            .push(slot.saved_order_hints);
        self.saved_global_motion_params.push(slot.saved_gm_params);
        self.lr_frame_filter_class_counts
            .push(slot.lr_frame_filter_class_counts);
        self.lr_frame_filter_taps
            .push(slot.lr_frame_filter_taps.clone());
        self.ref_frame_cdfs.push(slot.frame_cdfs.clone());
        self.ref_ccso_params.push(slot.ccso_params.clone());
        self.ref_ccso_unit_grids.push(slot.ccso_grid.clone());
        self.ref_motion_fields.push(slot.motion_field.clone());
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
