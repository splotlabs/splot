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
use crate::bitstream::byte_stream::SourceBytes;
use crate::bitstream::stream_plan::tile_group_continuations;
use crate::decoder::CodedFrameObserver;
use crate::prediction::inter::{
    FrameProductWriters, FrameProducts, MotionFieldHandle, MotionFieldLayout,
};
use crate::{
    BaseQuantizer, BaseQuantizerOrigin, CodedFrameEvent, DecodePlannedObu, ObuLocation,
    PictureProvenance, Result,
};
use core::num::NonZeroUsize;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::MAX_REF_FRAMES;
use splot_core::span::ByteOffset;
use std::sync::Arc;

use parking_lot::Mutex;

use super::frame_tasks::{DecoderRowScheduler, RowPools};
use super::inflight::InflightRing;
use crate::support::decode_buffers::DecodeBuffers;

/// The frame-pipelining depth of one context and the decoder state it keeps
/// between decode calls, as dav2d keeps its frame contexts.
pub(crate) struct DecodeSession {
    pub(crate) coded_frames: Option<CodedFrameObserver>,
    pub(crate) external_atlases: crate::ExternalAtlases,
    frame_delay: NonZeroUsize,
    /// Immutable concrete SIMD backend validated when the session opened.
    backend: splot_core::simd::ResolvedBackend,
    /// The row scheduler of a multi-threaded context, which the split inter
    /// frames run on.
    rows: Option<Arc<DecoderRowScheduler>>,
    retained: Mutex<Option<RetainedDecode>>,
    /// Whether frames that an outside owner still holds leave the store.
    detach_held: bool,
    /// The plane pool of the decode in progress, which held frames return to.
    planes: Mutex<Option<Arc<splot_recon::PlanePool>>>,
}

impl core::fmt::Debug for DecodeSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DecodeSession")
            .field("frame_delay", &self.frame_delay)
            .field("backend", &self.backend.kind())
            .finish_non_exhaustive()
    }
}

impl DecodeSession {
    pub(crate) fn new(
        frame_delay: NonZeroUsize,
        rows: Option<Arc<DecoderRowScheduler>>,
        backend: splot_core::simd::ResolvedBackend,
    ) -> Self {
        Self {
            frame_delay,
            external_atlases: crate::ExternalAtlases::default(),
            coded_frames: None,
            backend,
            rows,
            retained: Mutex::new(None),
            detach_held: false,
            planes: Mutex::new(None),
        }
    }

    /// A session whose frames leave the store while an outside owner, such
    /// as a caller's picture, still holds their samples.
    pub(crate) fn detaching_held(
        frame_delay: NonZeroUsize,
        rows: Option<Arc<DecoderRowScheduler>>,
        backend: splot_core::simd::ResolvedBackend,
    ) -> Self {
        Self {
            detach_held: true,
            ..Self::new(frame_delay, rows, backend)
        }
    }

    pub(super) fn observe_coded_frame(
        &self,
        provenance: PictureProvenance,
        candidate: &DecodePlannedObu,
        bytes: SourceBytes<'_>,
        core: &FrameHeaderCore,
        tip_quantizer_derived: bool,
    ) {
        let Some(observe) = &self.coded_frames else {
            return;
        };
        let header = candidate.header();
        observe(CodedFrameEvent::Frame {
            coded: provenance.coded,
            sequence_header: provenance.sequence_header,
            extended_layer_id: header.extended_layer_id,
            embedded_layer_id: header.embedded_layer_id,
            temporal_layer_id: header.temporal_layer_id,
            base_quantizer: core.quantization_params.map(|quantizer| BaseQuantizer {
                index: quantizer.base_q_idx,
                origin: if tip_quantizer_derived {
                    BaseQuantizerOrigin::Derived
                } else if core.is_bridge
                    || core
                        .inter
                        .as_ref()
                        .is_some_and(|inter| inter.bru_inactive == Some(true))
                {
                    BaseQuantizerOrigin::Inherited
                } else {
                    BaseQuantizerOrigin::Signaled
                },
            }),
        });
        if header.obu_type.is_tile_group() {
            for group in tile_group_continuations(bytes, candidate) {
                observe(CodedFrameEvent::Continuation {
                    coded: provenance.coded,
                    obu: ObuLocation {
                        user_data: provenance.coded.user_data,
                        offset: ByteOffset::new(group.offset().get() - bytes.base()),
                    },
                });
            }
        }
    }

    /// The concrete SIMD backend locked for this session.
    pub(crate) const fn backend(&self) -> splot_core::simd::ResolvedBackend {
        self.backend
    }

    /// The row scheduler, or `None` when the decode runs on one thread.
    pub(crate) fn rows(&self) -> Option<&DecoderRowScheduler> {
        self.rows.as_deref()
    }

    /// The threads this session decodes on: the row workers, or one.
    pub(crate) fn threads(&self) -> usize {
        self.rows.as_ref().map_or(1, |rows| rows.threads())
    }

    /// The plane pool of the last decode that started on this session.
    pub(crate) fn planes(&self) -> Option<Arc<splot_recon::PlanePool>> {
        self.planes.lock().clone()
    }

    pub(crate) const fn frame_delay(&self) -> NonZeroUsize {
        self.frame_delay
    }

    /// Takes the last decode's state when it was built for `depth` and the
    /// session's thread count, which size its cell pools. A concurrent decode on
    /// the same context gets new state instead.
    pub(super) fn take(&self, depth: NonZeroUsize) -> RetainedDecode {
        let width = self.threads();
        let mut retained = match self.retained.lock().take() {
            Some(retained)
                if retained.ring.capacity() == depth.get() && retained.width == width =>
            {
                retained
            }
            _ => RetainedDecode::new(depth, width),
        };
        retained.frames.detach_held = self.detach_held;
        *self.planes.lock() = Some(Arc::clone(retained.ring.planes()));
        retained
    }

    /// Keeps the state of a decode that finished, for the next call.
    pub(super) fn keep(&self, retained: RetainedDecode) {
        *self.retained.lock() = Some(retained);
    }
}

/// The storage one decode leaves for the next decode on its context.
pub(super) struct RetainedDecode {
    width: usize,
    pub(super) ring: InflightRing,
    pub(super) pools: RowPools,
    pub(super) frames: FrameStore,
    pub(super) input_scratch: crate::bitstream::byte_stream::InputScratch,
}

impl RetainedDecode {
    fn new(depth: NonZeroUsize, width: usize) -> Self {
        let buffers = DecodeBuffers::new();
        Self {
            width,
            pools: RowPools::new(depth.get(), width, &buffers),
            ring: InflightRing::new(depth, buffers, width),
            frames: FrameStore::new(depth.get()),
            input_scratch: crate::bitstream::byte_stream::InputScratch::default(),
        }
    }

    /// Opens the state for a new decode: the last decode's frames retire into
    /// their slots.
    pub(super) fn begin(&mut self) {
        for entry in &mut self.frames.entries {
            if let Some(frame) = entry.frame.take() {
                entry.retired = Some(self.ring.keep_frame_planes(frame.frame));
            }
        }
        self.frames.count = 0;
        self.frames.reserved = None;
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
    reserved: Option<usize>,
    pub(super) detach_held: bool,
}

impl FrameStore {
    pub(super) fn new(depth: usize) -> Self {
        Self {
            entries: (0..=2 * MAX_REF_FRAMES + depth)
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
            reserved: None,
            detach_held: false,
        }
    }

    pub(super) fn ensure_capacity_for_layers(&mut self, layer_count: usize, depth: usize) {
        if layer_count <= 1 {
            return;
        }
        let target = 2 * MAX_REF_FRAMES * layer_count + depth + 1;
        while self.entries.len() < target {
            self.entries.push(FrameEntry {
                index: 0,
                frame: None,
                motion: None,
                products: None,
                core: None,
                retired: None,
            });
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn has_space(&mut self) -> bool {
        self.reserved.is_some() || self.entries.iter_mut().any(FrameEntry::available)
    }

    pub(super) fn reserve(&mut self) -> Result<usize> {
        if let Some(index) = self.reserved {
            return Ok(index);
        }
        let index = self
            .entries
            .iter_mut()
            .position(FrameEntry::available)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
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
        self.entries
            .iter()
            .find(|entry| entry.index == index && entry.frame.is_some())
            .and_then(|entry| entry.frame.as_ref())
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
            reserved: None,
            detach_held: false,
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
