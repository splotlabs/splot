// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The minimal-tier multi-frame § 7.23 reference-frame buffer state.
//!
//! As each frame decodes, the AV2 § 7.23 reference frame update process refreshes
//! the reference-frame buffer slots named by the frame's `refresh_frame_flags`
//! (§ 7.20), storing the decoded planes plus the per-slot metadata later frames'
//! § 7.7 implicit reference-map ranking reads (`RefValid` / `RefOrderHint` /
//! `RefBaseQIdx` / dims / `RefCounter`). This module models that buffer for the
//! verified multi-reference runtime subset: it tracks, per slot, the metadata and
//! the index of the decoded frame stored there, and builds the borrowed
//! [`super::inter::InterReferenceState`] a following inter frame consumes.
//!
//! Feature tracking: `DECODE-INTER-MULTIREF-RUNTIME`.

use splot_recon::{DecodedFrame, ReferenceFrameStore, ReferenceSlot};

use crate::error::{DecodeError, Result};

use super::MinimalRuntimeFrame;

/// One reference-frame buffer slot's modeled § 7.23 state for the minimal tier.
///
/// `frame_index` is the index, into the runtime's decoded-frame vector, of the
/// frame stored in this slot; the borrowed [`DecodedFrame`] is recovered from that
/// vector when building the reference store (so the buffer holds no borrow itself).
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// `RefValid[i]` (§ 7.23): whether the slot holds a usable reference frame.
    valid: bool,
    /// `RefOrderHint[i]` (§ 7.23): the stored frame's display order hint (the UNWRAPPED
    /// `OrderHint` from `get_disp_order_hint()`, mirror :4375 / § 7.23 :14123 — NOT the
    /// raw `OrderHintLsbs`), so a § 7.7 / `choose_primary_secondary_ref_frame` ranking is
    /// wrap-correct when `OrderHintBits` would otherwise truncate it.
    order_hint: u32,
    /// `RefFrameWidth[i]` (§ 7.23).
    width: u32,
    /// `RefFrameHeight[i]` (§ 7.23).
    height: u32,
    /// `RefBaseQIdx[i]` (§ 7.23): the stored frame's `base_q_idx` (a § 7.7 score input).
    base_q_idx: u32,
    /// `RefFrameType[i] == INTER_FRAME` (§ 7.23 :14110): whether the stored frame is an
    /// inter frame. The § 5 `choose_primary_secondary_ref_frame` CHOOSE-resolution loop
    /// (mirror :5468-5495) scores ONLY inter-typed reference slots, so a key / intra-only
    /// reference can never be the resolved `primary_ref_frame` (and so never triggers the
    /// cross-frame CDF-load reject).
    is_inter: bool,
    /// Whether the frame stored in this slot ADAPTED its CDFs (`disable_cdf_update == 0`).
    /// A later frame that loads THIS slot's saved CDFs (§ 7.23 save / § 5 `load_cdfs`)
    /// would inherit an adapted entropy state the minimal decoder does not model, so the
    /// per-slot flag drives the cross-frame CDF-inheritance reject against the RESOLVED
    /// loaded slot (not a coarse "any prior frame adapted").
    adapted: bool,
    /// The index of the decoded frame stored in this slot, or `None` when empty.
    frame_index: Option<usize>,
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
        frame_index: None,
    };
}

/// The per-frame § 7.23 update inputs the reference buffer needs from a decoded frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameRefUpdate {
    /// `refresh_frame_flags` (§ 5.18.2): the bitmask of slots this frame refreshes.
    pub(super) refresh_frame_flags: u32,
    /// `OrderHint` (§ 5.18.2): the frame's display order hint.
    pub(super) order_hint: u32,
    /// `FrameWidth` (§ 5.18.4).
    pub(super) width: u32,
    /// `FrameHeight` (§ 5.18.4).
    pub(super) height: u32,
    /// `base_q_idx` (§ 5.18.6.1).
    pub(super) base_q_idx: u32,
    /// `true` for a KEY / SWITCH frame: § 7.23 sets `RefValid[i] = first` (only the
    /// first refreshed slot becomes valid, the rest invalid); `false` (an inter frame)
    /// sets every refreshed slot `RefValid[i] = 1`.
    pub(super) is_key_or_switch: bool,
    /// `FrameType == INTER_FRAME` for the stored frame (§ 7.23 :14110 `RefFrameType`).
    /// Drives the § 5 `choose_primary_secondary_ref_frame` inter-only candidate filter.
    pub(super) is_inter: bool,
    /// Whether the stored frame ADAPTED its CDFs (`disable_cdf_update == 0`). Recorded
    /// per slot so a later frame's cross-frame CDF-load reject fires only when the
    /// RESOLVED loaded slot's saved CDFs are adapted.
    pub(super) adapted: bool,
}

/// The minimal-tier § 7.23 reference-frame buffer over `num_ref_frames` active slots.
pub(super) struct RuntimeReferenceBuffer {
    slots: Vec<Slot>,
    /// `FrameCounter` (§ 7.23): incremented per frame, stored as each refreshed slot's
    /// `RefCounter`. Equal-counter slots (the same decoded frame stored in two slots)
    /// dedup to one distinct reference in § 7.7 `first_slot_with_ref`. For the verified
    /// subset each frame refreshes distinct slots so counters are naturally distinct.
    frame_counter: u32,
    /// Whether the first § 7.23 update has run (so `frame_counter` starts at 0).
    started: bool,
}

impl RuntimeReferenceBuffer {
    /// Creates an empty buffer with `num_ref_frames` active slots (1..=NUM_REF_FRAMES).
    pub(super) fn new(num_ref_frames: usize) -> Result<Self> {
        if num_ref_frames == 0 || num_ref_frames > ReferenceSlot::MAX_SLOTS {
            return Err(super::unsupported(
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

    /// Applies the AV2 § 7.23 reference frame update process for a just-decoded frame
    /// at `frame_index`, refreshing every slot named by `update.refresh_frame_flags`.
    ///
    /// A KEY / SWITCH frame sets `RefValid[i] = first` (so a `refresh_frame_flags ==
    /// allFrames` key marks ONLY the first refreshed slot valid, the rest invalid,
    /// mirror § 7.23 :14100); an inter frame sets every refreshed slot valid.
    pub(super) fn update(&mut self, frame_index: usize, update: FrameRefUpdate) {
        // §7.23: FrameCounter is 0 on the first update, incremented thereafter.
        if self.started {
            self.frame_counter = self.frame_counter.wrapping_add(1);
        }
        self.started = true;
        let mut first = true;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if (update.refresh_frame_flags >> i) & 1 == 0 {
                continue;
            }
            // §7.23 :14100: RefValid[i] = (KEY || SWITCH) ? first : 1.
            slot.valid = if update.is_key_or_switch { first } else { true };
            first = false;
            slot.order_hint = update.order_hint;
            slot.width = update.width;
            slot.height = update.height;
            slot.base_q_idx = update.base_q_idx;
            slot.is_inter = update.is_inter;
            slot.adapted = update.adapted;
            slot.frame_index = Some(frame_index);
        }
        let _ = self.frame_counter; // RefCounter is naturally distinct in this subset.
    }

    /// The number of active slots that are currently `RefValid`.
    pub(super) fn valid_count(&self) -> usize {
        self.slots.iter().filter(|s| s.valid).count()
    }

    /// Builds the borrowed § 7.23 [`super::inter::InterReferenceState`] for the next
    /// inter frame, recovering each valid slot's decoded frame from `frames`.
    ///
    /// The returned [`ReferenceFrameStore`] borrows the decoded frames in `frames`, so
    /// the caller must keep `frames` alive for the inter decode. `RefBaseQIdx` is
    /// modeled so the § 7.7 two-valid-slot ranking resolves exactly.
    pub(super) fn build_store<'a>(
        &self,
        frames: &'a [MinimalRuntimeFrame],
    ) -> Result<(ReferenceFrameStore<&'a DecodedFrame<u8>>, ReferenceMetadata)> {
        let num = self.slots.len();
        let mut store: ReferenceFrameStore<&'a DecodedFrame<u8>> =
            ReferenceFrameStore::with_capacity(num)
                .map_err(|source| DecodeError::Reconstruction { source })?;
        let mut meta = ReferenceMetadata::with_len(num);
        for (i, slot) in self.slots.iter().enumerate() {
            meta.ref_valid[i] = slot.valid;
            meta.ref_order_hint[i] = slot.order_hint;
            meta.ref_frame_width[i] = slot.width;
            meta.ref_frame_height[i] = slot.height;
            meta.ref_base_q_idx[i] = slot.base_q_idx;
            meta.ref_is_inter[i] = slot.is_inter;
            meta.ref_adapted[i] = slot.adapted;
            if !slot.valid {
                continue;
            }
            let frame_index = slot.frame_index.ok_or_else(|| {
                super::unsupported(
                    "reference_slot_missing_frame",
                    None,
                    "a §7.23 valid reference slot has no stored decoded frame",
                )
            })?;
            let frame = frames.get(frame_index).ok_or_else(|| {
                super::unsupported(
                    "reference_slot_frame_index_out_of_range",
                    None,
                    "a §7.23 reference slot points past the decoded-frame buffer",
                )
            })?;
            let reference_slot =
                ReferenceSlot::new(i).map_err(|source| DecodeError::Reconstruction { source })?;
            // §7.23 reference retention is 8-bit only; `frame_eight` rejects a
            // 10-bit frame with a structured diagnostic (the 10-bit subset is
            // single-frame intra, so an inter frame never references one).
            store
                .put(reference_slot, frame.frame_eight()?)
                .map_err(|source| DecodeError::Reconstruction { source })?;
        }
        Ok((store, meta))
    }
}

/// The parallel § 7.23 / § 7.7 reference metadata slices the [`super::inter`] decode
/// borrows into its [`super::inter::InterReferenceState`] (the owner of the backing
/// storage outlives the borrow).
pub(super) struct ReferenceMetadata {
    /// `RefValid[i]` per slot.
    pub(super) ref_valid: Vec<bool>,
    /// `RefOrderHint[i]` per slot.
    pub(super) ref_order_hint: Vec<u32>,
    /// `RefFrameWidth[i]` per slot.
    pub(super) ref_frame_width: Vec<u32>,
    /// `RefFrameHeight[i]` per slot.
    pub(super) ref_frame_height: Vec<u32>,
    /// `RefBaseQIdx[i]` per slot.
    pub(super) ref_base_q_idx: Vec<u32>,
    /// `RefFrameType[i] == INTER_FRAME` per slot (§ 7.23).
    pub(super) ref_is_inter: Vec<bool>,
    /// Whether the frame stored in slot `i` adapted its CDFs.
    pub(super) ref_adapted: Vec<bool>,
}

impl ReferenceMetadata {
    fn with_len(num: usize) -> Self {
        Self {
            ref_valid: vec![false; num],
            ref_order_hint: vec![0; num],
            ref_frame_width: vec![0; num],
            ref_frame_height: vec![0; num],
            ref_base_q_idx: vec![0; num],
            ref_is_inter: vec![false; num],
            ref_adapted: vec![false; num],
        }
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
        }
    }

    /// AV2 § 7.23 :14100 — a KEY frame's `refresh_frame_flags == 0xFF` marks ONLY the
    /// first refreshed slot valid (`first`), the rest invalid.
    #[test]
    fn key_refresh_marks_only_first_slot_valid() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, key_update());
        assert_eq!(buf.valid_count(), 1);
        assert!(buf.slots[0].valid);
        assert!(!buf.slots[1].valid);
        assert_eq!(buf.slots[0].base_q_idx, 70);
        assert_eq!(buf.slots[0].frame_index, Some(0));
    }

    /// AV2 § 7.23 — an inter frame's `refresh_frame_flags` marks every refreshed slot
    /// valid (`RefValid[i] = 1`), so after key (slot 0) + an inter refreshing slot 1
    /// there are TWO valid slots (the multi-reference precondition).
    #[test]
    fn inter_refresh_adds_a_second_valid_slot() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, key_update());
        buf.update(
            1,
            FrameRefUpdate {
                refresh_frame_flags: 1 << 1, // slot 1
                order_hint: 1,
                width: 64,
                height: 64,
                base_q_idx: 109,
                is_key_or_switch: false,
                is_inter: true,
                adapted: false,
            },
        );
        assert_eq!(buf.valid_count(), 2);
        assert!(buf.slots[0].valid);
        assert!(buf.slots[1].valid);
        assert_eq!(buf.slots[1].order_hint, 1);
        assert_eq!(buf.slots[1].base_q_idx, 109);
        assert_eq!(buf.slots[1].frame_index, Some(1));
        assert!(buf.slots[1].is_inter);
        assert!(!buf.slots[1].adapted);
    }

    /// AV2 § 7.23 — per-slot CDF adaptation: an inter frame refreshed with
    /// `disable_cdf_update == 0` records `adapted == true` only in ITS refreshed slot,
    /// leaving an earlier non-adapted slot's flag clear. This is the precise per-slot
    /// state the cross-frame CDF-load reject keys on (vs a coarse "any prior adapted").
    #[test]
    fn per_slot_adaptation_is_tracked_independently() {
        let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
        buf.update(0, key_update()); // slot 0: key, not adapted
        buf.update(
            1,
            FrameRefUpdate {
                refresh_frame_flags: 1 << 1, // slot 1
                order_hint: 1,
                width: 64,
                height: 64,
                base_q_idx: 109,
                is_key_or_switch: false,
                is_inter: true,
                adapted: true, // this inter frame adapted its CDFs
            },
        );
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
