// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The frame-finish phase and the end-of-walk artifact that feeds it.
//!
//! Decoding one frame is two phases. The walk runs the entropy walk and
//! reconstruction, and leaves a [`WalkedFrame`]: an owned, `Send` capture of
//! everything the shared § 7.2 in-loop filter chain and the frame freeze still
//! need, so the finish phase depends on no borrow of the driver's scratch or
//! reference state. [`finish_walked_frame`] consumes it and yields a
//! [`FinishedFrame`]. TIP output and bridge frames have no filter phase and so
//! leave the walk already final, as [`WalkStage::Complete`].
//!
//! The frame header is shared as an [`Arc`] between the walk output the driver
//! reads and the walked frame the filter phase consumes, so a deferred filter
//! phase owns everything it reads without copying a header per frame.

use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_recon::{DecodedFrame, DecodedFrameInfo, ReconSample};

use crate::Result;
use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::filters::ccso::CcsoUnitGrid;
use crate::filters::deblock::DeblockQuantDeltas;
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::filters::wienerns_lr::recon::WienerNsLrReconSink;
use crate::prediction::inter::TemporalMotionField;

/// One frame's walk output: the sample-side stage plus the frame-level state the
/// driver consumes as soon as the walk is done.
pub(crate) struct FrameWalk<T: ReconSample> {
    /// How far the frame's samples got during the walk.
    pub(crate) stage: WalkStage<T>,
    /// The frame header the walk consumed, shared with the filter phase.
    pub(crate) core: Arc<FrameHeaderCore>,
    /// The frame's end-of-walk CDF subset.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    /// The walk-parsed CCSO unit grid, retained for the reference update.
    pub(crate) ccso_grid: Option<CcsoUnitGrid>,
    /// The walk-derived temporal motion field.
    pub(crate) motion_field: TemporalMotionField,
}

/// How far one frame's samples got during its walk.
///
/// Both payloads are boxed: a reconstruction workspace is large, so an unboxed
/// variant would dominate the size of every walk output.
pub(crate) enum WalkStage<T: ReconSample> {
    /// The filter stages are still owed; [`finish_walked_frame`] runs them.
    Pending(Box<WalkedFrame<T>>),
    /// The frame has no filter phase and left the walk final.
    Complete(Box<DecodedFrame<T>>),
}

impl<T: ReconSample> WalkStage<T> {
    /// Records that a frame still owes its filter phase.
    pub(crate) fn pending(walked: WalkedFrame<T>) -> Self {
        Self::Pending(Box::new(walked))
    }

    /// Records that a frame left the walk without owing a filter phase.
    pub(crate) fn complete(frame: DecodedFrame<T>) -> Self {
        Self::Complete(Box::new(frame))
    }
}

/// Everything one frame's filter stages and freeze need, fully owned and `Send`.
pub(crate) struct WalkedFrame<T: ReconSample> {
    sink: WienerNsLrReconSink<T>,
    core: Arc<FrameHeaderCore>,
    disable_loopfilters_across_tiles: bool,
    deblock_quant_deltas: DeblockQuantDeltas,
    offset: ByteOffset,
}

impl<T: ReconSample> WalkedFrame<T> {
    /// Captures one frame's end-of-walk filter state.
    pub(crate) const fn new(
        sink: WienerNsLrReconSink<T>,
        core: Arc<FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: DeblockQuantDeltas,
        offset: ByteOffset,
    ) -> Self {
        Self {
            sink,
            core,
            disable_loopfilters_across_tiles,
            deblock_quant_deltas,
            offset,
        }
    }

    /// Returns the geometry the finished frame will report, which the filter
    /// chain carries through unchanged from the reconstruction workspace.
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        self.sink.frame_info()
    }
}

/// One frame after its filter stages, with the filter records to recycle.
pub(crate) struct FinishedFrame<T: ReconSample> {
    /// The filtered, frozen frame.
    pub(crate) frame: DecodedFrame<T>,
    /// The filter-record buffers to hand back to the decode scratch.
    pub(crate) filter_records: FrameFilterRecords,
}

/// Runs the shared § 7.2 in-loop filter chain over a walked frame and freezes it.
///
/// # Errors
///
/// Returns the filter chain's own diagnostic when a filter stage or the freeze
/// fails.
pub(crate) fn finish_walked_frame<T: ReconSample>(
    walked: WalkedFrame<T>,
) -> Result<FinishedFrame<T>> {
    let WalkedFrame {
        sink,
        core,
        disable_loopfilters_across_tiles,
        deblock_quant_deltas,
        offset,
    } = walked;
    let (frame, filter_records) = sink.into_filtered_frame(
        &core,
        disable_loopfilters_across_tiles,
        deblock_quant_deltas,
        offset,
    )?;
    Ok(FinishedFrame {
        frame,
        filter_records,
    })
}

/// Runs any owed filter phase inline and returns the frozen frame.
///
/// # Errors
///
/// Returns the filter chain's own diagnostic when a filter stage fails.
#[cfg(test)]
pub(crate) fn finish_walk_inline<T: ReconSample>(stage: WalkStage<T>) -> Result<DecodedFrame<T>> {
    Ok(match stage {
        WalkStage::Complete(frame) => *frame,
        WalkStage::Pending(walked) => finish_walked_frame(*walked)?.frame,
    })
}

#[cfg(test)]
mod tests {
    use super::WalkedFrame;

    fn assert_send<T: Send>() {}

    #[test]
    fn walked_frame_is_send() {
        assert_send::<WalkedFrame<u8>>();
        assert_send::<WalkedFrame<u16>>();
    }
}
