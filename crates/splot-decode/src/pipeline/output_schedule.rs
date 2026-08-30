// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Display-order scheduling and output resource accounting.

use super::{PipelineFrame, unsupported, unsupported_at};
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::BitDepth;

use crate::error::Result;
use crate::support::pipeline_limits::{checked_add, decoded_frame_storage_budget};
use crate::{DecodeLimitName, DecodeOptions};

/// The frames the driver owes `emit`, in display order.
///
/// [`OutputScheduler`] alone decides which frames are emitted and in what order,
/// and it decides that from frame headers, before any of those frames' samples
/// exist. Queueing the handover therefore changes only *when* a frame reaches
/// `emit`, never which frame does or in what order — in particular
/// `scheduler.emitted.len()`, which drives the `--limit` early exit, still
/// advances at exactly the point it did before, so the same frames decode.
///
/// The driver drains the queue at every scheduling point, handing over the
/// longest prefix whose filter phases have already settled without ever
/// blocking, and [flushes](Self::flush) what is left once the frame loop ends.
#[derive(Default)]
pub(super) struct EmissionQueue {
    pending: std::collections::VecDeque<usize>,
}

impl EmissionQueue {
    /// Whether a frame is still owed to `emit`, which keeps it unreclaimed.
    pub(super) fn holds(&self, frame_index: usize) -> bool {
        self.pending.contains(&frame_index)
    }

    /// Queues one frame for emission.
    fn push(&mut self, frame_index: usize) {
        self.pending.push_back(frame_index);
    }

    /// Emits the queued prefix whose frames have settled, leaving the rest.
    fn drain_settled(
        &mut self,
        frames: &[Option<PipelineFrame>],
        emit: &mut impl FnMut(&PipelineFrame) -> Result<()>,
    ) -> Result<()> {
        self.drain(frames, emit, false)
    }

    /// Emits every queued frame, waiting for the filter phases still running.
    pub(super) fn flush(
        &mut self,
        frames: &[Option<PipelineFrame>],
        emit: &mut impl FnMut(&PipelineFrame) -> Result<()>,
    ) -> Result<()> {
        self.drain(frames, emit, true)
    }

    fn drain(
        &mut self,
        frames: &[Option<PipelineFrame>],
        emit: &mut impl FnMut(&PipelineFrame) -> Result<()>,
        wait: bool,
    ) -> Result<()> {
        while let Some(&frame_index) = self.pending.front() {
            let frame = frames
                .get(frame_index)
                .and_then(Option::as_ref)
                .ok_or_else(missing_display_frame)?;
            if !wait && !frame.frame.is_settled() {
                break;
            }
            frame.wait_settled()?;
            emit(frame)?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

fn missing_display_frame() -> crate::error::DecodeError {
    unsupported(
        "displayed_frame_index_unavailable",
        None,
        "decode pipeline output ordering references a decoded frame that is unavailable",
    )
}

/// Queues the frames one scheduling action newly ordered for output, charges
/// them against the output limits, and hands over what has settled.
///
/// Each frame is queued behind a drain of the ones already owed, so a frame
/// whose output-effect metadata is refused cannot suppress the valid frames
/// scheduled ahead of it — one action can order several frames at once, since
/// [`OutputScheduler::on_immediate`] flushes the older pending ones first.
///
/// # Errors
///
/// Returns the output-limit, output-effect, or `emit` diagnostic.
pub(super) fn charge_emitted_outputs(
    options: &DecodeOptions,
    frames: &[Option<PipelineFrame>],
    scheduler: &OutputScheduler,
    queue: &mut EmissionQueue,
    newly: &[usize],
    emit: &mut impl FnMut(&PipelineFrame) -> Result<()>,
) -> Result<()> {
    if !newly.is_empty() {
        let requested = options
            .output_frame_limit()
            .map_or(u64::MAX, std::num::NonZeroU64::get);
        let emitted_total = (scheduler.emitted.len() as u64).min(requested);
        ensure_output_frame_count_limit(options.limits(), emitted_total)?;
        let first_new = scheduler.emitted.len() - newly.len();
        for (offset, &frame_index) in newly.iter().enumerate() {
            if (first_new + offset) as u64 >= requested {
                break;
            }
            queue.drain_settled(frames, emit)?;
            let frame = frames
                .get(frame_index)
                .and_then(Option::as_ref)
                .ok_or_else(missing_display_frame)?;
            frame.validate_output_effects()?;
            queue.push(frame_index);
        }
    }
    queue.drain_settled(frames, emit)
}

pub(super) fn output_frame_limit_reached(
    options: &DecodeOptions,
    output_frame_count: usize,
) -> bool {
    options
        .output_frame_limit()
        .is_some_and(|limit| output_frame_count as u64 >= limit.get())
}

pub(super) fn frame_is_output(core: &FrameHeaderCore) -> bool {
    core.immediate_output_frame == Some(true) || core.implicit_output_frame == Some(true)
}

pub(super) struct OutputScheduler {
    pub(super) pending: Vec<Option<(usize, u32)>>,
    pub(super) emitted: Vec<usize>,
    open_loop_active: bool,
    open_loop_order_hint: Option<u32>,
}

impl OutputScheduler {
    pub(super) fn new(num_slots: usize) -> Self {
        Self {
            pending: vec![None; num_slots],
            emitted: Vec::new(),
            open_loop_active: false,
            open_loop_order_hint: None,
        }
    }

    pub(super) fn emit(&mut self, frame_index: usize, newly: &mut Vec<usize>) {
        if !self.emitted.contains(&frame_index) {
            self.emitted.push(frame_index);
            newly.push(frame_index);
        }
        for slot in &mut self.pending {
            if slot.is_some_and(|(held, _)| held == frame_index) {
                *slot = None;
            }
        }
    }

    pub(super) fn flush_lower_than(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        loop {
            let next = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held < ordering)
                .min_by_key(|(_, held)| *held)
                .copied();
            let Some((frame_index, _)) = next else {
                return;
            };
            self.emit(frame_index, newly);
        }
    }

    pub(super) fn output_successive(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        let mut target = ordering.saturating_add(1);
        loop {
            let matches: Vec<usize> = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held == target)
                .map(|(frame_index, _)| *frame_index)
                .collect();
            if matches.is_empty() {
                return;
            }
            for frame_index in matches {
                self.emit(frame_index, newly);
            }
            target = target.saturating_add(1);
        }
    }

    pub(super) fn on_immediate(&mut self, frame_index: usize, ordering: u32) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(ordering, &mut newly);
        self.emit(frame_index, &mut newly);
        self.output_successive(ordering, &mut newly);
        newly
    }

    pub(super) fn refresh(
        &mut self,
        refresh_frame_flags: u32,
        frame_index: usize,
        ordering: u32,
        implicit: bool,
        is_key_or_switch: bool,
    ) -> Vec<usize> {
        let mut newly = Vec::new();
        let mut first = true;
        for slot in 0..self.pending.len() {
            if (refresh_frame_flags >> slot) & 1 == 0 {
                continue;
            }
            if let Some((held_index, held_ordering)) = self.pending[slot] {
                self.flush_lower_than(held_ordering, &mut newly);
                self.emit(held_index, &mut newly);
                self.output_successive(held_ordering, &mut newly);
            }
            let valid = !is_key_or_switch || first;
            self.pending[slot] = (implicit && valid && !self.emitted.contains(&frame_index))
                .then_some((frame_index, ordering));
            first = false;
        }
        newly
    }

    pub(super) fn already_emitted(&self, frame_index: usize) -> bool {
        self.emitted.contains(&frame_index)
    }

    pub(super) fn retains(&self, frame_index: usize) -> bool {
        self.pending
            .iter()
            .flatten()
            .any(|(pending, _)| *pending == frame_index)
    }

    pub(super) fn flush_all(&mut self) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(u32::MAX, &mut newly);
        newly
    }

    pub(super) fn prepare_for_frame(
        &mut self,
        obu_type: ObuType,
        first_picture_in_tu: bool,
    ) -> Vec<usize> {
        if !self.open_loop_active || !first_picture_in_tu || !is_regular_frame_obu(obu_type) {
            return Vec::new();
        }
        self.open_loop_active = false;
        let limit = self.open_loop_order_hint.take();
        let mut newly = Vec::new();
        loop {
            let next = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, ordering)| limit.is_none_or(|limit| *ordering < limit))
                .min_by_key(|(_, ordering)| *ordering)
                .copied();
            let Some((frame_index, _)) = next else {
                return newly;
            };
            self.emit(frame_index, &mut newly);
        }
    }

    pub(super) fn note_frame(
        &mut self,
        obu_type: ObuType,
        first_picture_in_tu: bool,
        order_hint: u32,
        immediate: bool,
        implicit: bool,
    ) {
        if obu_type == ObuType::OpenLoopKey {
            self.open_loop_active = true;
            self.open_loop_order_hint = implicit.then_some(order_hint);
        } else if self.open_loop_active
            && is_regular_frame_obu(obu_type)
            && !first_picture_in_tu
            && (immediate || implicit)
        {
            self.open_loop_order_hint = Some(order_hint);
        }
    }

    pub(super) fn restrict_slots(&mut self, slots: &[usize]) -> Vec<usize> {
        let mut newly = Vec::new();
        loop {
            let next = slots
                .iter()
                .filter_map(|&slot| self.pending.get(slot).copied().flatten())
                .min_by_key(|(_, ordering)| *ordering);
            let Some((frame_index, ordering)) = next else {
                break;
            };
            newly.extend(self.on_immediate(frame_index, ordering));
        }
        for &slot in slots {
            if let Some(pending) = self.pending.get_mut(slot) {
                *pending = None;
            }
        }
        newly
    }

    pub(super) fn start_new_sequence(&mut self, num_slots: usize) -> Vec<usize> {
        let newly = self.flush_all();
        self.pending = vec![None; num_slots];
        self.open_loop_active = false;
        self.open_loop_order_hint = None;
        newly
    }
}

const fn is_regular_frame_obu(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::OpenLoopKey
            | ObuType::RegularTileGroup
            | ObuType::RegularTip
            | ObuType::RegularSef
            | ObuType::Switch
            | ObuType::RasFrame
            | ObuType::BridgeFrame
    )
}

pub(super) fn select_output_frames(
    mut frames: Vec<Option<PipelineFrame>>,
    output_frame_indices: Vec<usize>,
) -> Result<Vec<PipelineFrame>> {
    let mut outputs = Vec::with_capacity(output_frame_indices.len());
    for index in output_frame_indices {
        let output = frames.get_mut(index).and_then(Option::take).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "decode pipeline output ordering references a decoded frame that is unavailable",
            )
        })?;
        outputs.push(output);
    }
    Ok(outputs)
}

pub(super) fn ensure_output_frame_count_limit(
    limits: crate::DecodeLimits,
    output_frame_count: u64,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxOutputFrames, output_frame_count)?;
    Ok(())
}

pub(super) fn ensure_retained_frame_byte_limits(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame: &PipelineFrame,
) -> Result<u64> {
    let frame_bytes = retained_decoded_frame_bytes(frame)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

pub(super) fn ensure_retained_frame_byte_limits_for_core(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_retained_limit",
            offset,
            "decode pipeline requires parsed frame dimensions before charging retained decoded-frame bytes",
        )
    })?;
    let bit_depth = match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    };
    let frame_bytes = decoded_frame_storage_budget(
        frame_size,
        sequence.general.chroma_format_idc,
        bytes_per_sample(bit_depth),
    )
    .map(|budget| budget.decoded_bytes)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)?;
    Ok(())
}

pub(super) fn ensure_retained_frame_byte_limits_for_bytes(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame_bytes: u64,
) -> Result<u64> {
    let next_retained_frame_bytes = checked_add(
        DecodeLimitName::MaxReferenceStoreBytes,
        retained_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(
        DecodeLimitName::MaxReferenceStoreBytes,
        next_retained_frame_bytes,
    )?;
    Ok(next_retained_frame_bytes)
}

pub(super) fn retained_decoded_frame_bytes(frame: &PipelineFrame) -> Result<u64> {
    if frame.frame.handle_count() == 1 {
        Ok(frame.byte_len()? as u64)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod open_loop_tests {
    use super::*;

    #[test]
    fn open_loop_boundary_flushes_only_hints_below_the_tu_limit() {
        let mut scheduler = OutputScheduler::new(2);
        scheduler.pending[0] = Some((10, 3));
        scheduler.pending[1] = Some((11, 6));
        scheduler.note_frame(ObuType::OpenLoopKey, true, 5, false, true);

        let emitted = scheduler.prepare_for_frame(ObuType::RegularTileGroup, true);

        assert_eq!(emitted, vec![10]);
        assert_eq!(scheduler.pending[1], Some((11, 6)));
    }

    #[test]
    fn open_loop_boundary_flushes_pending_frames_in_display_order() {
        let mut scheduler = OutputScheduler::new(2);
        scheduler.pending[0] = Some((10, 4));
        scheduler.pending[1] = Some((11, 2));
        scheduler.note_frame(ObuType::OpenLoopKey, true, 5, false, true);

        let emitted = scheduler.prepare_for_frame(ObuType::RegularTileGroup, true);

        assert_eq!(emitted, vec![11, 10]);
    }

    #[test]
    fn restricted_slots_output_eligible_frames_before_clearing_them() {
        let mut scheduler = OutputScheduler::new(3);
        scheduler.pending[0] = Some((10, 4));
        scheduler.pending[1] = Some((11, 2));
        scheduler.pending[2] = Some((12, 7));

        let emitted = scheduler.restrict_slots(&[0, 2]);

        assert_eq!(emitted, vec![11, 10, 12]);
        assert!(scheduler.pending.iter().all(Option::is_none));
    }
}

pub(super) fn bytes_per_sample(bit_depth: BitDepth) -> u64 {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::pipeline::FrameCdfSubset;
    use crate::pipeline::frame_lifecycle::PipelineDecodedFrame;
    use crate::pipeline::inflight::PipelineFrameSlot;
    use crate::pipeline::output_effects::FrameOutputEffects;
    use crate::prediction::inter::{MotionFieldHandle, MotionFieldLayout};
    use std::sync::Arc;

    use splot_core::headers::metadata::{
        MetadataHdrCll, MetadataPayload, MetadataType, MetadataUnit,
    };
    use splot_recon::SharedFrame;

    /// One settled output frame, `width` wide so `emit` can tell frames apart.
    fn settled_frame(
        width: usize,
        output_effects: FrameOutputEffects,
    ) -> core::result::Result<PipelineFrame, &'static str> {
        let frame = crate::test_support::decoded_frame(width, 4);
        Ok(PipelineFrame {
            frame: PipelineFrameSlot::completed(PipelineDecodedFrame::Eight(SharedFrame::new(
                frame,
            ))),
            display_grain: None,
            output_effects,
            frame_cdfs: crate::prediction::inter::FrameCdfHandle::settled(Arc::new(
                FrameCdfSubset::from_defaults(),
            )),
            motion_field: MotionFieldHandle::pending_with_layout(
                MotionFieldLayout::new(1, 1, 16).ok_or("valid motion-field layout")?,
            ),
            ccso_params: None,
            ccso_grid: crate::prediction::inter::CcsoGridHandle::settled(None),
            segment_ids: crate::prediction::inter::SegmentIdMapHandle::settled(Arc::new(
                crate::bitstream::tile_payload::FrameSegmentIdMap::new(1, width.div_ceil(4))
                    .unwrap(),
            )),
            frame_rate_numerator: 1,
            frame_rate_denominator: 1,
        })
    }

    /// Attached metadata AV2 § 6.16.1 refuses: the payload is not the one the
    /// unit's `metadata_type` selects.
    fn refused_output_effects() -> FrameOutputEffects {
        let mut effects = FrameOutputEffects::empty();
        effects.metadata = vec![MetadataUnit {
            metadata_type: MetadataType::HdrMdcv,
            payload_size: 4,
            payload: MetadataPayload::HdrCll(MetadataHdrCll {
                max_cll: 0,
                max_fall: 0,
            }),
        }];
        effects
    }

    #[test]
    fn a_refused_frame_does_not_suppress_the_frames_scheduled_ahead_of_it()
    -> core::result::Result<(), &'static str> {
        let options = DecodeOptions::default();
        let frames = vec![
            Some(settled_frame(8, FrameOutputEffects::empty())?),
            Some(settled_frame(16, refused_output_effects())?),
        ];
        let mut scheduler = OutputScheduler::new(2);
        scheduler.emitted = vec![0, 1];
        let mut queue = EmissionQueue::default();
        let mut widths = Vec::new();

        let result = charge_emitted_outputs(
            &options,
            &frames,
            &scheduler,
            &mut queue,
            &[0, 1],
            &mut |frame| {
                widths.push(frame.frame.info().coded_luma_size().width());
                Ok(())
            },
        );

        assert!(result.is_err(), "the refused frame still fails the stream");
        assert_eq!(
            widths,
            vec![8],
            "the frame scheduled first reached the caller before the refusal"
        );
        Ok(())
    }

    #[test]
    fn new_sequence_flushes_pending_output_and_recreates_slots() {
        let mut scheduler = OutputScheduler::new(2);
        assert!(scheduler.refresh(0b11, 7, 5, true, false).is_empty());

        let flushed = scheduler.start_new_sequence(4);

        assert_eq!(flushed, vec![7]);
        assert_eq!(scheduler.emitted, vec![7]);
        assert_eq!(scheduler.pending, vec![None; 4]);
    }
}
