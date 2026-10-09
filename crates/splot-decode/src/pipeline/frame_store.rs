// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Logical frame identities over reusable streaming metadata slots, and the
//! decoder state a context keeps between its decode calls.
//!
//! References and pending displays each hold at most `MAX_REF_FRAMES`. A full
//! table stops admission until outstanding work and queued emission drain.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use super::PipelineFrame;
use crate::Result;
use crate::prediction::inter::{
    FrameProductWriters, FrameProducts, MotionFieldHandle, MotionFieldLayout,
};
use core::num::NonZeroUsize;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::MAX_REF_FRAMES;
use std::sync::Arc;

use parking_lot::Mutex;

use super::frame_pipeline::{ReconAdmissionLane, RetainedEntropy};
use super::inflight::InflightRing;
use crate::prediction::inter::InterDecodeScratch;
use crate::support::decode_buffers::DecodeBuffers;

/// The frame-pipelining depth of one context and the decoder state it keeps
/// between decode calls, as dav2d keeps its frame contexts.
pub(crate) struct DecodeSession {
    frame_delay: NonZeroUsize,
    retained: Mutex<Option<RetainedDecode>>,
    record_hashes: Mutex<Vec<u64>>,
}

impl core::fmt::Debug for DecodeSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DecodeSession")
            .field("frame_delay", &self.frame_delay)
            .finish_non_exhaustive()
    }
}

impl DecodeSession {
    pub(crate) fn new(frame_delay: NonZeroUsize) -> Self {
        Self {
            frame_delay,
            retained: Mutex::new(None),
            record_hashes: Mutex::new(Vec::new()),
        }
    }

    /// The IVF record-hash list the last decode on this context planned into.
    pub(crate) fn take_record_hashes(&self) -> Vec<u64> {
        core::mem::take(&mut *self.record_hashes.lock())
    }

    pub(crate) fn keep_record_hashes(&self, hashes: Vec<u64>) {
        *self.record_hashes.lock() = hashes;
    }

    pub(crate) const fn frame_delay(&self) -> NonZeroUsize {
        self.frame_delay
    }

    /// Takes the last decode's state when it was built for `depth` and the
    /// current pool width, which size its cell pools. A concurrent decode on
    /// the same context gets new state instead.
    pub(super) fn take(&self, depth: NonZeroUsize) -> RetainedDecode {
        let width = splot_parallel::current_pool_width();
        match self.retained.lock().take() {
            Some(retained)
                if retained.ring.capacity() == depth.get() && retained.width == width =>
            {
                retained
            }
            _ => RetainedDecode::new(depth, width),
        }
    }

    /// Keeps the state of a decode that finished, for the next call.
    pub(super) fn keep(&self, retained: RetainedDecode) {
        *self.retained.lock() = Some(retained);
    }
}

/// The storage one decode leaves for the next decode on its context.
pub(super) struct RetainedDecode {
    width: usize,
    pub(super) scratch_eight: InterDecodeScratch<u8>,
    pub(super) scratch_ten: InterDecodeScratch<u16>,
    pub(super) ring: InflightRing,
    pub(super) lane: ReconAdmissionLane,
    pub(super) frames: FrameStore,
    pub(super) entropy_eight: RetainedEntropy<u8>,
    pub(super) entropy_ten: RetainedEntropy<u16>,
    pub(super) input_scratch: crate::bitstream::byte_stream::InputScratch,
}

impl RetainedDecode {
    fn new(depth: NonZeroUsize, width: usize) -> Self {
        let buffers = DecodeBuffers::new();
        let mut scratch_eight = InterDecodeScratch::default();
        let mut scratch_ten = InterDecodeScratch::default();
        scratch_eight.set_decode_buffers(&buffers);
        scratch_ten.set_decode_buffers(&buffers);
        Self {
            width,
            scratch_eight,
            scratch_ten,
            ring: InflightRing::new(depth, buffers),
            lane: ReconAdmissionLane::new(depth.get()),
            frames: FrameStore::new(false, depth.get()),
            entropy_eight: RetainedEntropy::default(),
            entropy_ten: RetainedEntropy::default(),
            input_scratch: crate::bitstream::byte_stream::InputScratch::default(),
        }
    }

    /// Opens the state for a new decode: the last decode's frames retire into
    /// their slots, and a lane still gated on unsettled work starts over.
    pub(super) fn begin(&mut self, retain: bool) {
        let depth = self.ring.capacity();
        if retain || self.frames.retain {
            self.frames = FrameStore::new(retain, depth);
        }
        for entry in &mut self.frames.entries {
            if let Some(frame) = entry.frame.take() {
                entry.retired = Some(self.ring.keep_frame_planes(frame.frame));
            }
        }
        self.frames.count = 0;
        self.frames.reserved = None;
        if !self.lane.is_settled() {
            self.lane = ReconAdmissionLane::new(depth);
        }
    }
}

pub(crate) struct FrameEntry {
    pub(super) index: usize,
    pub(super) frame: Option<PipelineFrame>,
    motion: Option<MotionFieldHandle>,
    products: Option<FrameProducts>,
    core: Option<Arc<FrameHeaderCore>>,
    pub(super) retired: Option<super::inflight::PipelineFrameSlot>,
}

pub(crate) struct FrameStore {
    pub(super) entries: Vec<FrameEntry>,
    count: usize,
    retain: bool,
    reserved: Option<usize>,
}

impl FrameStore {
    pub(super) fn new(retain: bool, depth: usize) -> Self {
        let slots = if retain {
            0
        } else {
            2 * MAX_REF_FRAMES + depth + 1
        };
        Self {
            entries: (0..slots)
                .map(|_| FrameEntry {
                    index: 0,
                    frame: None,
                    motion: None,
                    products: None,
                    core: None,
                    retired: None,
                })
                .collect(),
            count: 0,
            retain,
            reserved: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn has_space(&mut self) -> bool {
        self.reserved.is_some() || self.retain || self.entries.iter_mut().any(FrameEntry::available)
    }

    pub(super) fn reserve(&mut self) -> Result<usize> {
        if let Some(index) = self.reserved {
            return Ok(index);
        }
        let index = if self.retain {
            self.entries.push(FrameEntry {
                index: self.count,
                frame: None,
                motion: None,
                products: None,
                core: None,
                retired: None,
            });
            self.entries.len() - 1
        } else {
            self.entries
                .iter_mut()
                .position(FrameEntry::available)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?
        };
        self.reserved = Some(index);
        Ok(index)
    }

    pub(super) fn take_retired(&mut self) -> Result<Option<super::inflight::PipelineFrameSlot>> {
        let index = self.reserve()?;
        Ok(self.entries[index].retired.take())
    }

    pub(super) fn reserve_motion(
        &mut self,
        layout: MotionFieldLayout,
    ) -> Result<MotionFieldHandle> {
        let index = self.reserve()?;
        let motion = self.entries[index]
            .motion
            .get_or_insert_with(|| MotionFieldHandle::pending_with_layout(layout));
        motion.reset_layout(layout)?;
        Ok(motion.clone())
    }

    /// Publishes an already derived field through this frame's reusable
    /// handle, so a fused walk builds no handle, band list or field cell.
    pub(super) fn settle_motion(
        &mut self,
        field: crate::prediction::inter::TemporalMotionField,
    ) -> Result<MotionFieldHandle> {
        let motion = self.reserve_motion(field.layout())?;
        motion.publish(field);
        Ok(motion)
    }

    pub(super) fn reserve_products(&mut self) -> Result<FrameProductWriters> {
        let index = self.reserve()?;
        self.entries[index]
            .products
            .get_or_insert_default()
            .claim()
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    /// Shares `core` through this frame's header cell, rewriting it in place
    /// once the previous frame's readers have dropped it.
    pub(super) fn share_core(&mut self, core: FrameHeaderCore) -> Result<Arc<FrameHeaderCore>> {
        let index = self.reserve()?;
        let cell = &mut self.entries[index].core;
        match cell.as_mut().and_then(Arc::get_mut) {
            Some(held) => *held = core,
            None => *cell = Some(Arc::new(core)),
        }
        cell.clone()
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn get(&self, index: usize) -> Option<&PipelineFrame> {
        if self.retain {
            return self
                .entries
                .get(index)
                .and_then(|entry| entry.frame.as_ref());
        }
        self.entries
            .iter()
            .find(|entry| entry.index == index && entry.frame.is_some())
            .and_then(|entry| entry.frame.as_ref())
    }

    pub(super) fn take(&mut self, index: usize) -> Option<PipelineFrame> {
        if self.retain {
            return self
                .entries
                .get_mut(index)
                .and_then(|entry| entry.frame.take());
        }
        self.entries
            .iter_mut()
            .find(|entry| entry.index == index && entry.frame.is_some())
            .and_then(|entry| entry.frame.take())
    }

    pub(super) fn push(&mut self, frame: PipelineFrame) -> Result<()> {
        let index = self.reserve()?;
        let entry = &mut self.entries[index];
        entry.index = self.count;
        entry.frame = Some(frame);
        self.reserved = None;
        self.count += 1;
        Ok(())
    }
}

#[cfg(test)]
impl From<Vec<Option<PipelineFrame>>> for FrameStore {
    fn from(frames: Vec<Option<PipelineFrame>>) -> Self {
        Self {
            count: frames.len(),
            entries: frames
                .into_iter()
                .enumerate()
                .map(|(index, frame)| FrameEntry {
                    index,
                    frame,
                    motion: None,
                    products: None,
                    core: None,
                    retired: None,
                })
                .collect(),
            retain: true,
            reserved: None,
        }
    }
}

impl FrameEntry {
    fn available(&mut self) -> bool {
        self.frame.is_none()
            && self
                .retired
                .as_ref()
                .is_none_or(super::inflight::PipelineFrameSlot::can_reuse)
            && self
                .motion
                .as_mut()
                .is_none_or(MotionFieldHandle::try_retire)
            && self.products.as_ref().is_none_or(FrameProducts::can_reuse)
    }
}
