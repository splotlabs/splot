// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming observations of the decoder's reference buffer.

use parking_lot::Mutex;
use splot_core::headers::frame::{FrameHeaderCore, FrameSize, RefIdxBuf};
use splot_core::types::ObuType;

use crate::pipeline::frame_store::FrameStore;
use crate::prediction::inter::{mv_scaling::reference_is_scaled, named_pixel_reference_slots};
use crate::reference::buffer::{FrameRefUpdate, RuntimeReferenceBuffer};
use crate::{EmbeddedLayerId, ExtendedLayerId, PictureProvenance, TemporalLayerId};

/// A reference picture available in a decoder buffer slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ReferencePicture {
    /// The in-band OBU that coded the picture and its active sequence header.
    pub provenance: PictureProvenance,
    /// The picture's embedded layer.
    pub embedded_layer_id: EmbeddedLayerId,
    /// The picture's temporal layer.
    pub temporal_layer_id: TemporalLayerId,
    /// The luma size used by the reference buffer.
    pub frame_size: FrameSize,
    /// The stored long-term reference identifier, when present.
    pub long_term_id: Option<u32>,
    /// The decoder has restricted the slot's order hint during a layer switch.
    pub restricted_order_hint: bool,
}

/// A slot's state in the active extended layer's decoder reference buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReferenceSlotState {
    /// The decoder has never stored a frame in this slot since the buffer was created.
    Empty,
    /// The slot has been written, but its valid flag is clear.
    Invalid,
    /// The slot is valid, but the decoder has no retained picture identity.
    Unavailable,
    /// The slot holds a retained reference picture.
    Retained(ReferencePicture),
}

/// Reference buffer state at one point in decoder order.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ReferenceSnapshot {
    /// Slots in buffer order. Their indices are local to the extended layer.
    pub slots: Vec<ReferenceSlotState>,
}

/// Where the frame's reference list gets its slot indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReferenceMappingOrigin {
    /// The frame signals its `ref_frame_idx` entries.
    Signaled,
    /// The decoder derives the list from the frame and reference context.
    Derived,
    /// The first list entry follows the signaled `bridge_frame_ref_idx`.
    Bridge,
}

/// The frame-level list of candidate prediction references.
/// Block prediction and TIP can select a subset of these entries.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ReferenceMapping {
    /// How the frame obtains the slot indices.
    pub origin: ReferenceMappingOrigin,
    /// Each list index maps to a slot in the frame's input reference snapshot.
    pub slots: RefIdxBuf,
}

impl ReferenceMapping {
    pub(crate) fn from_core(core: &FrameHeaderCore) -> Option<Self> {
        let inter = core.inter.as_ref()?;
        if inter.ref_frame_idx.is_empty() {
            return None;
        }
        let origin = if core.is_bridge {
            ReferenceMappingOrigin::Bridge
        } else if inter.explicit_ref_frame_map? {
            ReferenceMappingOrigin::Signaled
        } else {
            ReferenceMappingOrigin::Derived
        };
        Some(Self {
            origin,
            slots: inter.ref_frame_idx.clone(),
        })
    }
}

/// Reference state observed when the decoder admits a coded frame.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ReferenceFrameEvent {
    /// The admitted frame's coded OBU and active sequence header.
    pub frame: PictureProvenance,
    /// The kind of coded OBU that starts the frame.
    pub obu_type: ObuType,
    /// The extended layer that owns the buffer.
    pub extended_layer_id: ExtendedLayerId,
    /// The admitted frame's embedded layer.
    pub embedded_layer_id: EmbeddedLayerId,
    /// The admitted frame's temporal layer.
    pub temporal_layer_id: TemporalLayerId,
    /// The admitted frame's luma size used by reference scaling.
    pub frame_size: FrameSize,
    /// The input slots after per-frame preparation.
    /// For a closed-loop key reset, these are the slots before the buffer is replaced.
    pub before: ReferenceSnapshot,
    /// The slots after the decoder refresh and open-loop key update.
    pub after: ReferenceSnapshot,
    /// The effective slot-refresh mask that the decoder applies.
    pub refresh_frame_flags: u32,
    /// A closed-loop key frame starts a new coded video sequence in this extended layer.
    pub resets_sequence: bool,
    /// The open-loop key slot mask for this frame, when that state limits references.
    pub allowed_input_slots: Option<u32>,
    /// The frame-level prediction map, when the frame header defines a non-empty list.
    pub mapping: Option<ReferenceMapping>,
    /// The signaled bridge source slot, including cases with no prediction list.
    pub bridge_slot: Option<u32>,
    /// Named input slots whose frame-dimension scale is non-unit.
    pub scaled_slots: u32,
    /// Long-term reference identifiers required by the parsed frame header.
    pub required_long_term_ids: Vec<u32>,
}

pub(crate) trait ReferenceObserver: Send + Sync {
    fn capture_input(&self, reference: &RuntimeReferenceBuffer, frames: &FrameStore);

    fn publish_update(
        &self,
        frames: &FrameStore,
        frame_index: usize,
        reference: &RuntimeReferenceBuffer,
        core: &FrameHeaderCore,
        update: &FrameRefUpdate<'_>,
    );
}

pub(crate) struct CaptureReferences {
    publish: Box<dyn Fn(ReferenceFrameEvent) + Send + Sync>,
    before: Mutex<Option<ReferenceSnapshot>>,
}

impl CaptureReferences {
    pub(crate) fn new(publish: impl Fn(ReferenceFrameEvent) + Send + Sync + 'static) -> Self {
        Self {
            publish: Box::new(publish),
            before: Mutex::new(None),
        }
    }
}

impl ReferenceObserver for CaptureReferences {
    fn capture_input(&self, reference: &RuntimeReferenceBuffer, frames: &FrameStore) {
        *self.before.lock() = Some(reference.snapshot(frames));
    }

    fn publish_update(
        &self,
        frames: &FrameStore,
        frame_index: usize,
        reference: &RuntimeReferenceBuffer,
        core: &FrameHeaderCore,
        update: &FrameRefUpdate<'_>,
    ) {
        let before = self.before.lock().take();
        let (Some(frame), Some(before)) = (frames.get(frame_index), before) else {
            return;
        };
        let mut scaled_slots = 0;
        for slot in named_pixel_reference_slots(core) {
            if let Some(ReferenceSlotState::Retained(picture)) = before.slots.get(slot as usize)
                && reference_is_scaled(
                    picture.frame_size.width as i32,
                    picture.frame_size.height as i32,
                    update.width as i32,
                    update.height as i32,
                )
            {
                scaled_slots |= 1 << slot;
            }
        }
        (self.publish)(ReferenceFrameEvent {
            frame: frame.provenance,
            obu_type: core.obu_type,
            extended_layer_id: frame.extended_layer_id,
            embedded_layer_id: frame.embedded_layer_id,
            temporal_layer_id: frame.temporal_layer_id,
            frame_size: FrameSize::new(update.width, update.height),
            before,
            after: reference.snapshot(frames),
            refresh_frame_flags: update.refresh_frame_flags,
            resets_sequence: core.obu_type == ObuType::ClosedLoopKey,
            allowed_input_slots: reference.allowed_input_slots(),
            mapping: ReferenceMapping::from_core(core),
            bridge_slot: core.bridge_frame_ref_idx,
            scaled_slots,
            required_long_term_ids: core.ref_long_term_ids.clone(),
        });
    }
}
