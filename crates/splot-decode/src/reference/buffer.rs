// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier AV2 § 7.23 reference-frame buffer state.
//!
//! The buffer tracks decoded-frame indices plus the per-slot metadata consumed by
//! the next inter frame's [`super::inter::InterReferenceState`].
//!
//! Feature tracking: `DECODE-INTER-MULTIREF-RUNTIME`.

use splot_recon::{DecodedFrame, ReferenceFrameStore, ReferenceSlot};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::error::Result;
use crate::pipeline::{PipelineFrame, unsupported};

/// One modeled § 7.23 reference slot.
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
        };
    }
}

/// Per-frame § 7.23 refresh inputs.
#[derive(Clone, Debug)]
pub(crate) struct FrameRefUpdate {
    /// Slot refresh bitmask.
    pub(crate) refresh_frame_flags: u32,
    /// Unwrapped display order hint.
    pub(crate) order_hint: u32,
    /// Frame width.
    pub(crate) width: u32,
    /// Frame height.
    pub(crate) height: u32,
    /// Frame `base_q_idx`.
    pub(crate) base_q_idx: u32,
    /// Whether § 7.23 should apply KEY/SWITCH first-slot validity.
    pub(crate) is_key_or_switch: bool,
    /// Whether the stored frame is `INTER_FRAME`.
    pub(crate) is_inter: bool,
    /// Whether the stored frame adapted its CDFs.
    pub(crate) adapted: bool,
    /// Per-plane retained frame-level Wiener-NS filter class counts.
    pub(crate) lr_frame_filter_class_counts: [u8; 3],
    /// Per-plane retained frame-level Wiener-NS filter taps (class-major).
    pub(crate) lr_frame_filter_taps: [Vec<Vec<i16>>; 3],
    /// Saved frame CDF context for later cross-frame CDF initialization.
    pub(crate) frame_cdfs: FrameCdfSubset,
}

/// The minimal-tier § 7.23 reference-frame buffer over `num_ref_frames` active slots.
pub(crate) struct RuntimeReferenceBuffer {
    slots: Vec<Slot>,
    /// Modeled § 7.23 `FrameCounter`; the current subset deduplicates by frame index.
    frame_counter: u32,
    /// Whether the first update has run.
    started: bool,
}

impl RuntimeReferenceBuffer {
    /// Creates an empty buffer with `num_ref_frames` active slots (1..=NUM_REF_FRAMES).
    pub(crate) fn new(num_ref_frames: usize) -> Result<Self> {
        if num_ref_frames == 0 || num_ref_frames > ReferenceSlot::MAX_SLOTS {
            return Err(unsupported(
                "unsupported_num_ref_frames",
                None,
                "minimal multi-frame decode requires 1..=16 active reference slots",
            ));
        }
        Ok(Self {
            slots: vec![Slot::EMPTY; num_ref_frames],
            frame_counter: 0,
            started: false,
        })
    }

    /// Applies the § 7.23 refresh for a decoded frame.
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

    /// The number of active slots that are currently `RefValid`.
    pub(crate) fn valid_count(&self) -> usize {
        self.slots.iter().filter(|s| s.valid).count()
    }

    /// Builds the borrowed reference store and metadata for the next inter frame.
    pub(crate) fn build_store_eight<'a>(
        &self,
        frames: &'a [PipelineFrame],
    ) -> Result<(ReferenceFrameStore<&'a DecodedFrame<u8>>, ReferenceMetadata)> {
        self.build_store(frames, PipelineFrame::frame_eight)
    }

    /// Builds the borrowed 10-bit reference store and metadata for the next inter frame.
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
            let frame_index = slot.frame_index.ok_or_else(|| {
                unsupported(
                    "reference_slot_missing_frame",
                    None,
                    "a §7.23 valid reference slot has no stored decoded frame",
                )
            })?;
            let frame = frames.get(frame_index).ok_or_else(|| {
                unsupported(
                    "reference_slot_frame_index_out_of_range",
                    None,
                    "a §7.23 reference slot points past the decoded-frame buffer",
                )
            })?;
            let reference_slot = ReferenceSlot::new(i)?;
            store.put(reference_slot, frame_view(frame)?)?;
        }
        Ok((store, meta))
    }
}

/// Reference metadata arrays borrowed by [`super::inter::InterReferenceState`].
#[allow(clippy::struct_field_names)]
pub(crate) struct ReferenceMetadata {
    /// `RefValid[i]` per slot.
    pub(crate) ref_valid: Vec<bool>,
    /// `RefOrderHint[i]` per slot.
    pub(crate) ref_order_hint: Vec<u32>,
    /// `RefFrameWidth[i]` per slot.
    pub(crate) ref_frame_width: Vec<u32>,
    /// `RefFrameHeight[i]` per slot.
    pub(crate) ref_frame_height: Vec<u32>,
    /// `RefBaseQIdx[i]` per slot.
    pub(crate) ref_base_q_idx: Vec<u32>,
    /// `RefFrameType[i] == INTER_FRAME` per slot (§ 7.23).
    pub(crate) ref_is_inter: Vec<bool>,
    /// Whether the frame stored in slot `i` adapted its CDFs.
    pub(crate) ref_adapted: Vec<bool>,
    /// Retained frame-level Wiener-NS filter class counts per slot and plane.
    pub(crate) lr_frame_filter_class_counts: Vec<[u8; 3]>,
    /// Retained frame-level Wiener-NS filter taps per slot (plane, class-major).
    pub(crate) lr_frame_filter_taps: Vec<[Vec<Vec<i16>>; 3]>,
    /// Saved frame CDF context per slot.
    pub(crate) ref_frame_cdfs: Vec<Option<FrameCdfSubset>>,
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
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn key_update() -> FrameRefUpdate {
        FrameRefUpdate {
            refresh_frame_flags: 0xFF,
            order_hint: 0,
            width: 64,
            height: 64,
            base_q_idx: 70,
            is_key_or_switch: true,
            is_inter: false,
            adapted: false,
            lr_frame_filter_class_counts: [1, 0, 0],
            lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
            frame_cdfs: FrameCdfSubset::from_defaults(),
        }
    }

    fn inter_update(adapted: bool) -> FrameRefUpdate {
        FrameRefUpdate {
            refresh_frame_flags: 1 << 1,
            order_hint: 1,
            width: 64,
            height: 64,
            base_q_idx: 109,
            is_key_or_switch: false,
            is_inter: true,
            adapted,
            lr_frame_filter_class_counts: [0, 0, 0],
            lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
            frame_cdfs: FrameCdfSubset::from_defaults(),
        }
    }

    #[test]
    fn key_refresh_marks_only_first_slot_valid() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, &key_update());
        assert_eq!(buf.valid_count(), 1);
        assert!(buf.slots[0].valid);
        assert!(!buf.slots[1].valid);
        assert_eq!(buf.slots[0].base_q_idx, 70);
        assert_eq!(buf.slots[0].frame_index, Some(0));
        assert_eq!(buf.slots[0].lr_frame_filter_class_counts, [1, 0, 0]);
    }

    #[test]
    fn inter_refresh_adds_a_second_valid_slot() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, &key_update());
        buf.update(1, &inter_update(false));
        assert_eq!(buf.valid_count(), 2);
        assert!(buf.slots[0].valid);
        assert!(buf.slots[1].valid);
        assert_eq!(buf.slots[1].order_hint, 1);
        assert_eq!(buf.slots[1].base_q_idx, 109);
        assert_eq!(buf.slots[1].frame_index, Some(1));
        assert!(buf.slots[1].is_inter);
        assert!(!buf.slots[1].adapted);
    }

    #[test]
    fn per_slot_adaptation_is_tracked_independently() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, &key_update());
        buf.update(1, &inter_update(true));
        assert!(!buf.slots[0].adapted);
        assert!(buf.slots[1].adapted);
        assert!(!buf.slots[0].is_inter);
        assert!(buf.slots[1].is_inter);
    }

    #[test]
    fn zero_slots_rejected() {
        assert!(RuntimeReferenceBuffer::new(0).is_err());
    }
}
