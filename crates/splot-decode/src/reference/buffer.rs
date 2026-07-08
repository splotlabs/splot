// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier AV2 § 7.23 reference-frame buffer state.
//!
//! The buffer tracks decoded-frame indices plus the per-slot metadata consumed by
//! the next inter frame's [`super::inter::InterReferenceState`].
//!
//! Feature tracking: `DECODE-INTER-MULTIREF-RUNTIME`.

use splot_core::headers::frame::CcsoParams;
use splot_recon::{DecodedFrame, ReferenceFrameStore, ReferenceSlot};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::error::{DecodeReferenceStateError, Result};
use crate::filters::ccso::CcsoUnitGrid;
use crate::pipeline::PipelineFrame;
use crate::prediction::inter::TemporalMotionField;

#[derive(Clone, Debug)]
struct Slot {
    valid: bool,
    order_hint: u32,
    width: u32,
    height: u32,
    base_q_idx: u32,
    is_inter: bool,
    adapted: bool,
    lr_frame_filter_class_counts: [u8; 3],
    lr_frame_filter_taps: [Vec<Vec<i16>>; 3],
    frame_index: Option<usize>,
    frame_cdfs: Option<FrameCdfSubset>,
    ccso_params: Option<CcsoParams>,
    ccso_grid: Option<CcsoUnitGrid>,
    motion_field: Option<TemporalMotionField>,
}

impl Slot {
    const EMPTY: Self = Self {
        valid: false,
        order_hint: 0,
        width: 0,
        height: 0,
        base_q_idx: 0,
        is_inter: false,
        adapted: false,
        lr_frame_filter_class_counts: [0; 3],
        lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
        frame_index: None,
        frame_cdfs: None,
        ccso_params: None,
        ccso_grid: None,
        motion_field: None,
    };

    fn refresh(&mut self, frame_index: usize, update: &FrameRefUpdate, valid: bool) {
        *self = Self {
            valid,
            order_hint: update.order_hint,
            width: update.width,
            height: update.height,
            base_q_idx: update.base_q_idx,
            is_inter: update.is_inter,
            adapted: update.adapted,
            lr_frame_filter_class_counts: update.lr_frame_filter_class_counts,
            lr_frame_filter_taps: update.lr_frame_filter_taps.clone(),
            frame_index: Some(frame_index),
            frame_cdfs: Some(update.frame_cdfs.clone()),
            ccso_params: update.ccso_params.clone(),
            ccso_grid: update.ccso_grid.clone(),
            motion_field: Some(update.motion_field.clone()),
        };
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRefUpdate {
    pub(crate) refresh_frame_flags: u32,
    pub(crate) order_hint: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) base_q_idx: u32,
    pub(crate) is_key_or_switch: bool,
    pub(crate) is_inter: bool,
    pub(crate) adapted: bool,
    pub(crate) lr_frame_filter_class_counts: [u8; 3],
    pub(crate) lr_frame_filter_taps: [Vec<Vec<i16>>; 3],
    pub(crate) frame_cdfs: FrameCdfSubset,
    pub(crate) ccso_params: Option<CcsoParams>,
    pub(crate) ccso_grid: Option<CcsoUnitGrid>,
    pub(crate) motion_field: TemporalMotionField,
}

pub(crate) struct RuntimeReferenceBuffer {
    slots: Vec<Slot>,
    frame_counter: u32,
    started: bool,
}

impl RuntimeReferenceBuffer {
    pub(crate) fn new(num_ref_frames: usize) -> Result<Self> {
        ReferenceFrameStore::<()>::with_capacity(num_ref_frames)?;
        Ok(Self {
            slots: vec![Slot::EMPTY; num_ref_frames],
            frame_counter: 0,
            started: false,
        })
    }

    pub(crate) fn update(&mut self, frame_index: usize, update: &FrameRefUpdate) {
        if self.started {
            self.frame_counter = self.frame_counter.wrapping_add(1);
        }
        self.started = true;
        let mut first = true;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if (update.refresh_frame_flags >> i) & 1 == 0 {
                continue;
            }
            let valid = !update.is_key_or_switch || first;
            first = false;
            slot.refresh(frame_index, update, valid);
        }
        let _ = self.frame_counter;
    }

    pub(crate) fn build_store_eight<'a>(
        &self,
        frames: &'a [PipelineFrame],
    ) -> Result<(ReferenceFrameStore<&'a DecodedFrame<u8>>, ReferenceMetadata)> {
        self.build_store(frames, PipelineFrame::frame_eight)
    }

    pub(crate) fn build_store_ten<'a>(
        &self,
        frames: &'a [PipelineFrame],
    ) -> Result<(
        ReferenceFrameStore<&'a DecodedFrame<u16>>,
        ReferenceMetadata,
    )> {
        self.build_store(frames, PipelineFrame::frame_ten)
    }

    fn build_store<'a, T: splot_recon::ReconSample>(
        &self,
        frames: &'a [PipelineFrame],
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
            let frame =
                frames
                    .get(frame_index)
                    .ok_or(DecodeReferenceStateError::FrameIndexOutOfRange {
                        slot: i,
                        frame_index,
                        frame_count: frames.len(),
                    })?;
            let reference_slot = ReferenceSlot::new(i)?;
            let frame = frame_view(frame)?;
            ensure_slot_matches_frame(i, slot, frame_index, frame)?;
            store.put(reference_slot, frame)?;
        }
        Ok((store, meta))
    }
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
    pub(crate) ref_frame_width: Vec<u32>,
    pub(crate) ref_frame_height: Vec<u32>,
    pub(crate) ref_base_q_idx: Vec<u32>,
    pub(crate) ref_is_inter: Vec<bool>,
    pub(crate) ref_adapted: Vec<bool>,
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
            ref_frame_width: Vec::with_capacity(num),
            ref_frame_height: Vec::with_capacity(num),
            ref_base_q_idx: Vec::with_capacity(num),
            ref_is_inter: Vec::with_capacity(num),
            ref_adapted: Vec::with_capacity(num),
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
        self.ref_frame_width.push(slot.width);
        self.ref_frame_height.push(slot.height);
        self.ref_base_q_idx.push(slot.base_q_idx);
        self.ref_is_inter.push(slot.is_inter);
        self.ref_adapted.push(slot.adapted);
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
