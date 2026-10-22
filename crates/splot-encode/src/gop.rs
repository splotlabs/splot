// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference structure: the coding order, frame types, order hints,
//! refreshed reference slots and show-existing frames of the stream, and the
//! frame header and OBUs each coded frame gives.
//!
//! The first frame, every frame `keyint` frames after the last key frame, and
//! every scene cut the lookahead reports are closed-loop key frames. The frames
//! between key frames form mini-GOPs of the preset's `mini_gop_size` frames;
//! the last mini-GOP before a key frame or the end of the input can be
//! shorter. A mini-GOP codes its last frame first, then the middle frame of
//! each open span, recursively. A frame coded before frames that precede it in
//! display order is hidden, and a show-existing frame outputs it once those
//! frames are out; every other frame is shown when coded. So size 1 is IPPP,
//! and size 4 codes `4 2 1 [2] 3 [4]` (brackets are show-existing frames).
//!
//! A temporal unit holds the hidden frames coded since the last output and
//! one output: a shown frame or a show-existing frame (§ 7.3.3, § 7.3.4,
//! § 7.3.6). A key frame refreshes every slot and keeps only slot 0 valid
//! (§ 7.23); any other frame refreshes one slot: an empty one, else the one
//! with the oldest display index that no show-existing frame waits for. The
//! § 7.7 implicit reference map ranks the valid slots by display distance and
//! `base_q_idx`, so at one quantizer the closest frames rank first.
//!
//! Encoder policy, fixed for now: the sequence sets
//! `monotonic_output_order_flag`, so a hidden frame is shown by a
//! show-existing frame with `derive_sef_order_hint = 1`; inter frames code
//! `primary_ref_frame = PRIMARY_REF_NONE` with `disable_cross_frame_cdf_init =
//! 1`, so no CDFs carry across frames; motion vectors are quarter-pel.

use core::num::NonZeroU32;

use splot_core::headers::frame::{
    CoreSeqView, FrameFilterParams, FrameHeaderCore, InterFrameParams, KeyFrameTxParams,
    build_inter_frame_header, build_key_frame_header, encode_inter_frame_obu,
    encode_key_frame_temporal_unit, encode_show_existing_frame_obu, encode_temporal_delimiter_obu,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_decode::encoder_syntax::EncoderReferences;

use crate::error::{Error, Result};
use crate::preset::PresetParams;
use crate::rate_control::FrameType;

/// `NumRefFrames` of the encoder's sequence header (`explicit_num_ref_frames = 0`).
const REF_SLOTS: usize = 8;

/// The plan of one coded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GopFrame {
    /// Zero-based coding-order index.
    pub(crate) frame_index: u64,
    /// Zero-based input (display) index.
    pub(crate) display_index: u64,
    /// Frame type.
    pub(crate) frame_type: FrameType,
    /// `OrderHint`: the distance from the last key frame.
    pub(crate) order_hint: u32,
    /// `immediate_output_frame`; a hidden frame waits for a show-existing frame.
    pub(crate) show_frame: bool,
    /// Depth in the mini-GOP hierarchy: `0` for key frames and the last frame
    /// of each mini-GOP, one more per bisection.
    pub(crate) layer: u8,
    /// `refresh_frame_flags`.
    pub(crate) refresh_frame_flags: u32,
    /// The slots that hold a frame when this frame is coded: the frames it may
    /// reference.
    pub(crate) ref_slots: u32,
    /// The frame starts its temporal unit, after the temporal delimiter.
    pub(crate) opens_temporal_unit: bool,
}

/// One step of the coding order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GopStep {
    /// Code a frame.
    Code(GopFrame),
    /// Output the hidden frame `display_index` from `slot`. It always starts
    /// its temporal unit.
    ShowExisting { slot: u32, display_index: u64 },
}

impl GopStep {
    /// The display index of the frame this step outputs; the output closes
    /// the temporal unit.
    pub(crate) fn output(&self) -> Option<u64> {
        match *self {
            Self::Code(frame) => frame.show_frame.then_some(frame.display_index),
            Self::ShowExisting { display_index, .. } => Some(display_index),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    display_index: u64,
    awaiting_show: bool,
}

/// Plans key frames and mini-GOPs over the input frames.
#[derive(Debug, Clone)]
pub(crate) struct GopPlanner {
    keyint: u64,
    mini_gop_size: u64,
    slots: [Option<Slot>; REF_SLOTS],
    next_display: u64,
    next_frame: u64,
    key_display: u64,
    /// Display index of the next key frame `keyint` frames after the last one.
    scheduled_key: u64,
    /// Scene cuts not planned yet, ascending.
    cuts: Vec<u64>,
    after_output: bool,
}

impl GopPlanner {
    /// A planner with key frames every `keyint` frames and mini-GOPs of
    /// `mini_gop_size` frames (`0` counts as `1`).
    pub(crate) fn new(keyint: NonZeroU32, mini_gop_size: u8) -> Self {
        Self {
            keyint: u64::from(keyint.get()),
            mini_gop_size: u64::from(mini_gop_size.max(1)),
            slots: [None; REF_SLOTS],
            next_display: 0,
            next_frame: 0,
            key_display: 0,
            scheduled_key: 0,
            cuts: Vec::new(),
            after_output: true,
        }
    }

    /// The display index of the first frame the next [`Self::plan`] takes.
    pub(crate) fn next_display(&self) -> u64 {
        self.next_display
    }

    /// Makes the scene cut `display_index` a key frame, unless it is already
    /// planned or it is less than `mini_gop_size` frames after the key frame
    /// before it. Cuts come in display order.
    pub(crate) fn force_key(&mut self, display_index: u64) {
        let last = self
            .cuts
            .last()
            .map_or(self.key_display, |&cut| cut.max(self.key_display));
        let last = display_index.saturating_sub(last) / self.keyint * self.keyint + last;
        if display_index >= self.next_display
            && display_index.saturating_sub(last) >= self.mini_gop_size
            && !self.cuts.contains(&display_index)
        {
            self.cuts.push(display_index);
            self.cuts.sort_unstable();
        }
    }

    /// The display index of the next key frame.
    fn next_key(&self) -> u64 {
        self.cuts
            .first()
            .map_or(self.scheduled_key, |&cut| cut.min(self.scheduled_key))
    }

    /// The number of frames the next [`Self::plan`] takes from `queued` input
    /// frames: a key frame alone, else a mini-GOP that stops before the next
    /// key frame. `None` while too few frames are queued, unless the input has
    /// `ended`.
    pub(crate) fn next_len(&self, queued: usize, ended: bool) -> Option<usize> {
        let to_key = self.next_key().saturating_sub(self.next_display);
        let wanted = if to_key == 0 {
            1
        } else {
            to_key.min(self.mini_gop_size)
        };
        let wanted = usize::try_from(wanted).unwrap_or(usize::MAX);
        (queued > 0 && (queued >= wanted || ended)).then_some(queued.min(wanted))
    }

    /// Plans the next `len` input frames, as [`Self::next_len`] returned.
    /// Display index 0 is a key frame, so a mini-GOP always follows a coded frame.
    ///
    /// # Errors
    /// Returns [`Error::Unsupported`] when a frame finds no slot to refresh:
    /// every slot holds a hidden frame that is not shown yet.
    pub(crate) fn plan(&mut self, len: usize) -> Result<Vec<GopStep>> {
        let mut steps = Vec::new();
        if len == 0 {
            return Ok(steps);
        }
        let first = self.next_display;
        let last = first + len as u64 - 1;
        let mut anchor = first.saturating_sub(1);
        if first == self.next_key() {
            self.cuts.retain(|&cut| cut > first);
            self.scheduled_key = first + self.keyint;
            steps.push(self.code(first, FrameType::Key, true, 0)?);
            anchor = first;
        }
        if last > anchor {
            self.place(anchor, last, 0, &mut steps)?;
        }
        self.next_display = last + 1;
        Ok(steps)
    }

    /// Codes `hi` at `layer` and the frames between the coded frame `lo` and `hi`.
    fn place(&mut self, lo: u64, hi: u64, layer: u8, steps: &mut Vec<GopStep>) -> Result<()> {
        if hi - lo == 1 {
            steps.push(self.code(hi, FrameType::Inter, true, layer)?);
            return Ok(());
        }
        steps.push(self.code(hi, FrameType::Inter, false, layer)?);
        self.fill(lo, hi, layer + 1, steps)?;
        let (slot, held) = self
            .slots
            .iter_mut()
            .enumerate()
            .find_map(|(slot, held)| {
                held.as_mut()
                    .filter(|held| held.display_index == hi && held.awaiting_show)
                    .map(|held| (slot as u32, held))
            })
            .ok_or(Error::Unsupported {
                what: "a show-existing frame of an evicted frame",
            })?;
        held.awaiting_show = false;
        steps.push(GopStep::ShowExisting {
            slot,
            display_index: hi,
        });
        self.after_output = true;
        Ok(())
    }

    /// Codes the frames between the coded frames `lo` and `hi` from `layer` on.
    fn fill(&mut self, lo: u64, hi: u64, layer: u8, steps: &mut Vec<GopStep>) -> Result<()> {
        if hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            self.place(lo, mid, layer, steps)?;
            self.fill(mid, hi, layer + 1, steps)?;
        }
        Ok(())
    }

    fn code(
        &mut self,
        display_index: u64,
        frame_type: FrameType,
        show_frame: bool,
        layer: u8,
    ) -> Result<GopStep> {
        let (slot, refresh_frame_flags, ref_slots) = if frame_type == FrameType::Key {
            self.slots = [None; REF_SLOTS];
            self.key_display = display_index;
            (0, (1 << REF_SLOTS) - 1, 0)
        } else {
            let ref_slots = (0..REF_SLOTS)
                .filter(|&slot| self.slots[slot].is_some())
                .fold(0, |mask, slot| mask | 1 << slot);
            let slot = self.refresh_slot()?;
            (slot, 1 << slot, ref_slots)
        };
        self.slots[slot] = Some(Slot {
            display_index,
            awaiting_show: !show_frame,
        });
        let frame = GopFrame {
            frame_index: self.next_frame,
            display_index,
            frame_type,
            order_hint: (display_index - self.key_display) as u32,
            show_frame,
            layer,
            refresh_frame_flags,
            ref_slots,
            opens_temporal_unit: self.after_output,
        };
        self.next_frame += 1;
        self.after_output = show_frame;
        Ok(GopStep::Code(frame))
    }

    /// An empty slot, else the slot of the oldest frame no show-existing
    /// frame waits for.
    fn refresh_slot(&self) -> Result<usize> {
        if let Some(empty) = self.slots.iter().position(Option::is_none) {
            return Ok(empty);
        }
        (0..REF_SLOTS)
            .filter_map(|slot| {
                self.slots[slot]
                    .filter(|held| !held.awaiting_show)
                    .map(|held| (held.display_index, slot))
            })
            .min()
            .map(|(_, slot)| slot)
            .ok_or(Error::Unsupported {
                what: "more hidden frames than reference slots",
            })
    }
}

/// Frame `frame_index` of an IPPP stream with key frames every `keyint` frames.
#[cfg(test)]
pub(crate) fn plan(frame_index: u64, keyint: NonZeroU32) -> GopFrame {
    let mut planner = GopPlanner::new(keyint, 1);
    let mut frame = None;
    for _ in 0..=frame_index {
        if let Ok([GopStep::Code(coded)]) = planner.plan(1).as_deref() {
            frame = Some(*coded);
        }
    }
    frame.unwrap_or_else(|| unreachable!("IPPP codes every input frame"))
}

/// The frame-level tool choices of `preset`: the transform parameters, and
/// whether inter frames code compound and skip-mode blocks.
pub(crate) fn frame_tools(preset: &PresetParams) -> (KeyFrameTxParams, bool) {
    (
        crate::tx_choice::key_frame_tx_params(preset),
        preset.compound_candidates > 0,
    )
}

/// A temporal delimiter and the `OBU_REGULAR_SEF` that outputs `slot`.
pub(crate) fn show_existing_unit(view: &CoreSeqView, slot: u32) -> Result<Vec<u8>> {
    let mut unit = encode_temporal_delimiter_obu().map_err(header_error)?;
    unit.extend(encode_show_existing_frame_obu(view, slot).map_err(header_error)?);
    Ok(unit)
}

impl GopFrame {
    /// The frame header at `qindex` with `filters`, parsed with `refs`; an
    /// inter frame with `compound` sets `reference_select` and
    /// `skip_mode_present`. The header codes the low bits of the planned
    /// `OrderHint`, so the `OrderHint` the decoder derives must equal it.
    pub(crate) fn header(
        &self,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        qindex: u32,
        filters: &FrameFilterParams,
        (tx, compound): (KeyFrameTxParams, bool),
    ) -> Result<FrameHeaderCore> {
        let core = match self.frame_type {
            FrameType::Key => build_key_frame_header(view, qindex, self.order_hint, filters, tx),
            FrameType::Inter => build_inter_frame_header(
                view,
                &refs.header_view(),
                &self.inter_params(qindex, filters, (tx, compound)),
            )
            .map(|header| header.core),
        }
        .map_err(header_error)?;
        match core.display_order_hint() {
            Some(derived) if derived == self.order_hint => Ok(core),
            derived => Err(Error::OrderHintMismatch {
                planned: self.order_hint,
                derived,
            }),
        }
    }

    /// The OBUs of the frame `core` built by [`Self::header`] with
    /// `(qindex, filters, (tx, compound))`, after a temporal delimiter when the frame opens
    /// its temporal unit; a key frame repeats the sequence header.
    pub(crate) fn temporal_unit(
        &self,
        seq: &SequenceHeader,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        (qindex, filters, tools): (u32, &FrameFilterParams, (KeyFrameTxParams, bool)),
        core: &FrameHeaderCore,
        tile: &[u8],
    ) -> Result<Vec<u8>> {
        if self.frame_type == FrameType::Key {
            if !self.opens_temporal_unit {
                return Err(Error::Unsupported {
                    what: "a key frame after another frame of its temporal unit",
                });
            }
            return encode_key_frame_temporal_unit(seq, core, true, tile).map_err(header_error);
        }
        let header = build_inter_frame_header(
            view,
            &refs.header_view(),
            &self.inter_params(qindex, filters, tools),
        )
        .map_err(header_error)?;
        if header.core != *core {
            return Err(Error::Unsupported {
                what: "an inter frame header that changed after coding",
            });
        }
        let mut unit = if self.opens_temporal_unit {
            encode_temporal_delimiter_obu().map_err(header_error)?
        } else {
            Vec::new()
        };
        unit.extend(encode_inter_frame_obu(&header, tile).map_err(header_error)?);
        Ok(unit)
    }

    fn inter_params(
        &self,
        qindex: u32,
        filters: &FrameFilterParams,
        (tx, compound): (KeyFrameTxParams, bool),
    ) -> InterFrameParams {
        InterFrameParams {
            base_q_idx: qindex,
            order_hint: self.order_hint,
            refresh_frame_flags: self.refresh_frame_flags,
            show_frame: self.show_frame,
            filters: *filters,
            tx,
            reference_select: compound,
            skip_mode_present: compound,
        }
    }
}

fn header_error<E: std::error::Error + Send + Sync + 'static>(source: E) -> Error {
    Error::Coding {
        stage: "frame header",
        source: Box::new(source),
    }
}

#[cfg(test)]
#[path = "gop_tests.rs"]
mod tests;
