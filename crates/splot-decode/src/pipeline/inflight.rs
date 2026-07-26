// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Completion-backed decoded-frame handles for the decode pipeline.
//!
//! A [`RefFrameSlot`] names one decoded frame by an owned, shareable handle
//! instead of a borrow of the driver's frame vector. Its geometry is known as
//! soon as the frame header is parsed, while its samples are published once
//! through a [`CompletionCell`], so the handle can be stored in a reference
//! store and size-accounted independently of when the pixels land. Every handle
//! built today is created already settled.

use std::sync::Arc;

use splot_parallel::CompletionCell;
use splot_recon::{DecodedFrame, DecodedFrameInfo, ReconSample, SharedFrame};

use super::{PipelineDecodedFrame, unsupported};
use crate::error::{DecodeError, Result};

/// An owned handle to one decoded reference frame and its known geometry.
pub(crate) struct RefFrameSlot<T: ReconSample> {
    cell: Arc<CompletionCell<SharedFrame<T>>>,
    info: DecodedFrameInfo,
}

impl<T: ReconSample> RefFrameSlot<T> {
    /// Wraps an already reconstructed frame in a settled handle.
    pub(crate) fn completed(frame: SharedFrame<T>) -> Self {
        let info = frame.get().info();
        Self {
            cell: Arc::new(CompletionCell::completed(frame)),
            info,
        }
    }

    /// Returns a second handle to the same completion slot without copying
    /// pixels, and without requiring the slot to be settled.
    pub(crate) fn share(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
            info: self.info,
        }
    }

    /// Borrows the decoded frame when its samples have already been published.
    pub(crate) fn try_frozen(&self) -> Option<&DecodedFrame<T>> {
        self.cell.get().map(SharedFrame::get)
    }

    /// Whether the decoded samples have been published.
    pub(crate) fn is_settled(&self) -> bool {
        self.cell.is_set()
    }

    /// Returns the decoded frame geometry, which is known before the samples.
    pub(crate) const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the number of live handles on the published frame storage, or
    /// one while no storage exists yet.
    pub(crate) fn handle_count(&self) -> usize {
        self.cell.get().map_or(1, SharedFrame::handle_count)
    }

    /// Shares the published frame storage, failing closed when the samples have
    /// not landed.
    pub(crate) fn ready(&self) -> Result<SharedFrame<T>> {
        self.cell
            .get()
            .map(SharedFrame::share)
            .ok_or_else(unsettled_slot)
    }

    /// Retires the handle, returning the plane sample buffers to the
    /// reconstruction-plane pool when this was the last handle.
    pub(crate) fn reclaim_planes(self) {
        if let Some(frame) = Arc::into_inner(self.cell).and_then(CompletionCell::into_inner) {
            frame.reclaim_planes();
        }
    }
}

/// A decoded-frame handle in the pipeline's active sample storage bit depth.
pub(crate) enum PipelineFrameSlot {
    /// Eight-bit sample storage.
    Eight(RefFrameSlot<u8>),
    /// Ten-bit sample storage.
    Ten(RefFrameSlot<u16>),
}

impl PipelineFrameSlot {
    /// Wraps an already reconstructed frame in a settled handle.
    pub(crate) fn completed(frame: PipelineDecodedFrame) -> Self {
        match frame {
            PipelineDecodedFrame::Eight(frame) => Self::Eight(RefFrameSlot::completed(frame)),
            PipelineDecodedFrame::Ten(frame) => Self::Ten(RefFrameSlot::completed(frame)),
        }
    }

    /// Returns the decoded frame geometry, which is known before the samples.
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Eight(slot) => slot.info(),
            Self::Ten(slot) => slot.info(),
        }
    }

    /// Whether the decoded samples have been published.
    pub(crate) fn is_settled(&self) -> bool {
        match self {
            Self::Eight(slot) => slot.is_settled(),
            Self::Ten(slot) => slot.is_settled(),
        }
    }

    /// Returns the number of live handles on the published frame storage.
    pub(crate) fn handle_count(&self) -> usize {
        match self {
            Self::Eight(slot) => slot.handle_count(),
            Self::Ten(slot) => slot.handle_count(),
        }
    }

    /// Shares the published frame storage, failing closed when the samples have
    /// not landed.
    pub(crate) fn ready(&self) -> Result<PipelineDecodedFrame> {
        Ok(match self {
            Self::Eight(slot) => PipelineDecodedFrame::Eight(slot.ready()?),
            Self::Ten(slot) => PipelineDecodedFrame::Ten(slot.ready()?),
        })
    }

    /// Retires the handle, returning the plane sample buffers to the
    /// reconstruction-plane pool when this was the last handle.
    pub(crate) fn reclaim_planes(self) {
        match self {
            Self::Eight(slot) => slot.reclaim_planes(),
            Self::Ten(slot) => slot.reclaim_planes(),
        }
    }
}

fn unsettled_slot() -> DecodeError {
    unsupported(
        "decoded_frame_samples_unavailable",
        None,
        "internal invariant violation: a decoded frame handle was read before its samples landed",
    )
}
