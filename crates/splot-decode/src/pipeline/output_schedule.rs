// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Display-order scheduling and output resource accounting.

use super::{PipelineFrame, unsupported, unsupported_at};
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader};
use splot_core::span::ByteOffset;
use splot_recon::BitDepth;

use crate::error::Result;
use crate::support::pipeline_limits::{checked_add, decoded_frame_storage_budget};
use crate::{DecodeLimitName, DecodeOptions};

pub(super) fn charge_emitted_outputs(
    options: &DecodeOptions,
    frames: &[Option<PipelineFrame>],
    scheduler: &OutputScheduler,
    newly: &[usize],
    mut output_frame_bytes: u64,
    charge_output_bytes: bool,
    emit: &mut impl FnMut(&PipelineFrame) -> Result<()>,
) -> Result<u64> {
    if newly.is_empty() {
        return Ok(output_frame_bytes);
    }
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
        let frame = frames.get(frame_index).and_then(Option::as_ref).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "decode pipeline output ordering references a decoded frame that is unavailable",
            )
        })?;
        emit(frame)?;
        if charge_output_bytes {
            output_frame_bytes =
                ensure_output_frame_byte_limits(options.limits(), output_frame_bytes, frame)?;
        }
    }
    Ok(output_frame_bytes)
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
}

impl OutputScheduler {
    pub(super) fn new(num_slots: usize) -> Self {
        Self {
            pending: vec![None; num_slots],
            emitted: Vec::new(),
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
) -> Result<u64> {
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
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
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
    Ok(frame.byte_len()? as u64)
}

pub(super) fn ensure_output_frame_byte_limits(
    limits: crate::DecodeLimits,
    output_frame_bytes: u64,
    frame: &PipelineFrame,
) -> Result<u64> {
    let frame_bytes = frame.byte_len()? as u64;
    let next_output_frame_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        output_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, next_output_frame_bytes)?;
    Ok(next_output_frame_bytes)
}

pub(super) fn bytes_per_sample(bit_depth: BitDepth) -> u64 {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}
