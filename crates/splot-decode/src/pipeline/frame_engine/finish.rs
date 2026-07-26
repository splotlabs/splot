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

use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_recon::{DecodedFrame, ReconSample};

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
    /// The frame's end-of-walk CDF subset.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    /// The walk-parsed CCSO unit grid, retained for the reference update.
    pub(crate) ccso_grid: Option<CcsoUnitGrid>,
    /// The walk-derived temporal motion field.
    pub(crate) motion_field: TemporalMotionField,
}

/// How far one frame's samples got during its walk.
///
/// Both payloads are boxed: a frame header is large, so an unboxed variant would
/// dominate the size of every walk output.
pub(crate) enum WalkStage<T: ReconSample> {
    /// The filter stages are still owed; [`finish_walked_frame`] runs them.
    Pending(Box<WalkedFrame<T>>),
    /// The frame has no filter phase and left the walk final.
    Complete(Box<CompletedFrame<T>>),
}

impl<T: ReconSample> WalkStage<T> {
    /// Records that a frame still owes its filter phase.
    pub(crate) fn pending(walked: WalkedFrame<T>) -> Self {
        Self::Pending(Box::new(walked))
    }

    /// Records that a frame left the walk without owing a filter phase.
    pub(crate) fn complete(frame: DecodedFrame<T>, core: FrameHeaderCore) -> Self {
        Self::Complete(Box::new(CompletedFrame { frame, core }))
    }
}

/// A frame that reached its final samples without a filter phase.
pub(crate) struct CompletedFrame<T: ReconSample> {
    /// The decoded frame.
    pub(crate) frame: DecodedFrame<T>,
    /// The frame header the walk consumed.
    pub(crate) core: FrameHeaderCore,
}

/// Everything one frame's filter stages and freeze need, fully owned and `Send`.
pub(crate) struct WalkedFrame<T: ReconSample> {
    sink: WienerNsLrReconSink<T>,
    core: FrameHeaderCore,
    disable_loopfilters_across_tiles: bool,
    deblock_quant_deltas: DeblockQuantDeltas,
    offset: ByteOffset,
}

impl<T: ReconSample> WalkedFrame<T> {
    /// Captures one frame's end-of-walk filter state.
    pub(crate) const fn new(
        sink: WienerNsLrReconSink<T>,
        core: FrameHeaderCore,
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
}

/// One frame after its filter stages, with the filter records to recycle.
pub(crate) struct FinishedFrame<T: ReconSample> {
    /// The filtered, frozen frame.
    pub(crate) frame: DecodedFrame<T>,
    /// The frame header the filter stages read.
    pub(crate) core: FrameHeaderCore,
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
        core,
        filter_records,
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
