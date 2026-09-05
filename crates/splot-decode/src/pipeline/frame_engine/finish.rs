// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The frame-finish phase and the end-of-walk artifact that feeds it.
//!
//! Decoding one frame is two phases. The walk runs the entropy walk and
//! reconstruction, and leaves a [`WalkedFrame`]: an owned, `Send` capture of
//! everything the shared § 7.2 in-loop filter chain and the frame freeze still
//! need, so the finish phase depends on no borrow of the driver's scratch or
//! reference state. [`finish_walked_frame`] consumes it and returns the filter
//! records for recycling. TIP output and bridge frames have no filter phase, so
//! they leave the walk already final, as [`WalkStage::Complete`].
//!
//! The frame header is shared as an [`Arc`] between the walk output the driver
//! reads and the walked frame the filter phase consumes, so a deferred filter
//! phase owns everything it reads without copying a header per frame.

use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{DecodedFrame, DecodedFrameInfo, ReconSample, SharedFrame};

use crate::Result;
use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::bitstream::tile_payload::FrameSegmentIdMap;
use crate::filters::ccso::CcsoUnitGrid;
use crate::filters::deblock::DeblockQuantDeltas;
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::filters::wienerns_lr::recon::WienerNsLrReconSink;
use crate::pipeline::frame_progress::FrameProgress;
use crate::prediction::inter::{InterFilterInputs, TemporalMotionField};

/// One frame's walk output: the sample-side stage plus the frame-level state the
/// driver consumes as soon as the walk is done.
pub(crate) struct FrameWalk<T: ReconSample> {
    /// How far the frame's samples got during the walk.
    pub(crate) stage: WalkStage<T>,
    /// The frame header the walk consumed, shared with the filter phase.
    pub(crate) core: Arc<FrameHeaderCore>,
    /// The frame's end-of-walk CDF subset.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    /// The walk-parsed CCSO unit grid retained by the canonical `PipelineFrame`.
    pub(crate) ccso_grid: Option<CcsoUnitGrid>,
    pub(crate) segment_ids: Arc<FrameSegmentIdMap>,
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

/// The frame-level facts that turn a filtered-frame sink into a walked frame,
/// captured once so the inter and intra walks share one tail.
pub(crate) struct FilterSinkSetup {
    pub(crate) luma_width: usize,
    pub(crate) luma_height: usize,
    pub(crate) bit_depth: splot_recon::BitDepth,
    pub(crate) gdf_reference: Option<crate::filters::gdf::GdfReferenceContext>,
    pub(crate) cfl_ds_filter_index: u8,
    pub(crate) disable_loopfilters_across_tiles: bool,
    pub(crate) deblock_quant_deltas: DeblockQuantDeltas,
}

impl FilterSinkSetup {
    /// Builds the owned filter setup for a frame whose workspace the
    /// reconstruction already owns.
    pub(crate) fn deferred_filter_setup<T: ReconSample>(
        self,
        info: splot_recon::DecodedFrameInfo,
        plane_sizes: [Option<splot_recon::PlaneSize>; 3],
        filter_inputs: InterFilterInputs,
        core: Arc<FrameHeaderCore>,
        progress: Arc<FrameProgress<T>>,
    ) -> Result<(
        crate::filters::wienerns_lr::recon::OwnedFilterSetup<'static, 'static, T>,
        DeblockQuantDeltas,
    )> {
        let sink = WienerNsLrReconSink::for_deferred_filtering(
            info,
            plane_sizes,
            self.luma_width,
            self.luma_height,
            self.bit_depth,
        );
        let (sink, disable_loopfilters_across_tiles, deblock_quant_deltas) =
            self.fill_sink(sink, filter_inputs);
        let (setup, _) = sink.into_owned_filter_setup_published(
            core,
            disable_loopfilters_across_tiles,
            progress,
        )?;
        Ok((setup, deblock_quant_deltas))
    }

    fn fill_sink<T: ReconSample>(
        self,
        mut sink: WienerNsLrReconSink<T>,
        filter_inputs: InterFilterInputs,
    ) -> (WienerNsLrReconSink<T>, bool, DeblockQuantDeltas) {
        sink.set_gdf_reference_context(self.gdf_reference);
        sink.set_filter_records(filter_inputs.records);
        sink.set_cdef_grid(Some(filter_inputs.cdef_grid));
        sink.set_ccso_grid(filter_inputs.ccso_grid);
        sink.set_gdf_grid(filter_inputs.gdf_grid);
        sink.set_cfl_ds_filter_index(self.cfl_ds_filter_index);
        (
            sink,
            self.disable_loopfilters_across_tiles,
            self.deblock_quant_deltas,
        )
    }

    fn into_sink<T: ReconSample>(
        self,
        workspace: splot_recon::CurrentFrameWorkspace<T>,
        filter_inputs: InterFilterInputs,
    ) -> (WienerNsLrReconSink<T>, bool, DeblockQuantDeltas) {
        let sink = crate::filters::wienerns_lr::recon_final_filter_sink(
            workspace,
            self.luma_width,
            self.luma_height,
            self.bit_depth,
        );
        self.fill_sink(sink, filter_inputs)
    }

    /// Wraps one reconstructed frame and its filter inputs into the walked
    /// frame the § 7.2 filter chain consumes.
    pub(crate) fn walked_frame<T: ReconSample>(
        self,
        workspace: splot_recon::CurrentFrameWorkspace<T>,
        filter_inputs: InterFilterInputs,
        core: Arc<FrameHeaderCore>,
    ) -> WalkedFrame<T> {
        let (sink, disable_loopfilters_across_tiles, deblock_quant_deltas) =
            self.into_sink(workspace, filter_inputs);
        WalkedFrame::new(
            sink,
            core,
            disable_loopfilters_across_tiles,
            deblock_quant_deltas,
        )
    }

    /// Turns one frame's walk output into the driver's [`FrameWalk`].
    ///
    /// `carries_motion_field` is false for intra frames, whose canonical
    /// `PipelineFrame` stores an empty field whatever the walk derived.
    pub(crate) fn frame_walk<T: ReconSample>(
        self,
        workspace: splot_recon::CurrentFrameWorkspace<T>,
        mut filter_inputs: InterFilterInputs,
        core: Arc<FrameHeaderCore>,
        frame_cdfs: Arc<FrameCdfSubset>,
        segment_ids: Arc<FrameSegmentIdMap>,
        carries_motion_field: bool,
    ) -> FrameWalk<T> {
        let ccso_grid = filter_inputs.ccso_grid.clone();
        let derived = filter_inputs.take_motion_field();
        let motion_field = if carries_motion_field {
            derived
        } else {
            TemporalMotionField::empty()
        };
        FrameWalk {
            stage: WalkStage::pending(self.walked_frame(
                workspace,
                filter_inputs,
                Arc::clone(&core),
            )),
            core,
            frame_cdfs,
            ccso_grid,
            segment_ids,
            motion_field,
        }
    }
}

/// Everything one frame's filter stages and freeze need, fully owned and `Send`.
pub(crate) struct WalkedFrame<T: ReconSample> {
    sink: WienerNsLrReconSink<T>,
    core: Arc<FrameHeaderCore>,
    disable_loopfilters_across_tiles: bool,
    deblock_quant_deltas: DeblockQuantDeltas,
}

impl<T: ReconSample> WalkedFrame<T> {
    /// Captures one frame's end-of-walk filter state.
    pub(crate) const fn new(
        sink: WienerNsLrReconSink<T>,
        core: Arc<FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: DeblockQuantDeltas,
    ) -> Self {
        Self {
            sink,
            core,
            disable_loopfilters_across_tiles,
            deblock_quant_deltas,
        }
    }

    /// Returns the geometry the finished frame will report, which the filter
    /// chain carries through unchanged from the reconstruction workspace.
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        self.sink.frame_info()
    }
}

/// Runs the shared § 7.2 in-loop filter chain over a walked frame and freezes it.
///
/// `publish` receives the frozen frame's sole handle while the freeze still
/// holds the frame's publication lock, so a pipelined frame's slot settles
/// before its published row prefix stops being readable without leaving a
/// second owner in the finish task. A `publish` that is never reached — a filter
/// stage failed — is dropped instead, which is how a pending slot learns its
/// filter phase failed.
///
/// # Errors
///
/// Returns the filter chain's own diagnostic when a filter stage or the freeze
/// fails.
pub(crate) fn finish_walked_frame<T: ReconSample>(
    walked: WalkedFrame<T>,
    progress: Option<Arc<FrameProgress<T>>>,
    admit: Option<&dyn splot_parallel::Admit<'_, crate::pipeline::frame_pipeline::FrameTask>>,
    publish: impl FnOnce(SharedFrame<T>),
) -> Result<FrameFilterRecords> {
    let WalkedFrame {
        sink,
        core,
        disable_loopfilters_across_tiles,
        deblock_quant_deltas,
    } = walked;
    let ((), filter_records) = sink.into_filtered_frame(
        core,
        disable_loopfilters_across_tiles,
        deblock_quant_deltas,
        progress,
        admit,
        |frame| publish(SharedFrame::new(frame)),
    )?;
    Ok(filter_records)
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
