// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{FrameHeaderCore, IntrabcParams};
use splot_core::span::ByteOffset;
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};

use super::super::{SPEC_MODE_INFO, unsupported_at};
use crate::bitstream::tile_payload::{DecodeBlockFrontier, FrameQmSegmentScope};
use crate::filters::wienerns_lr::intrabc_records::{
    IntrabcBlockGeometry, IntrabcInfo, IntrabcPredictionGeometry,
    derive_intrabc_luma_prediction_geometry,
};
use crate::prediction::inter::mv_scaling::{PlaneScaling, derive_plane_scaling};
use crate::prediction::inter::{
    InterResidual, InterResidualBlock, InterResidualReconScratch, Mv, mc,
};
use crate::{Result, prediction::inter::add_inter_residual_to_workspace};

pub(super) struct IntrabcReconPrediction {
    luma: IntrabcPredictionGeometry,
    chroma: Option<IntrabcChromaPrediction>,
    morph_mv: Option<Mv>,
    /// § 7.13.3.25 `AvailU` / `AvailL`, which `is_inside` scopes to the current tile.
    morph_avail: (bool, bool),
    global_fence: bool,
}

#[derive(Clone, Copy)]
struct IntrabcChromaPrediction {
    target: PlaneRect,
    scaling: PlaneScaling,
}

impl IntrabcReconPrediction {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn derive(
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        morph_avail: (bool, bool),
        n4w: usize,
        n4h: usize,
        info: IntrabcInfo,
        (sub_x, sub_y): (u32, u32),
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let luma = derive_intrabc_luma_prediction_geometry(
            core,
            IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h),
            info,
            tile_offset,
        )?;
        let chroma = if frontier.has_chroma {
            Self::derive_chroma(core, frontier, info, luma, sub_x, sub_y, tile_offset)?
        } else {
            None
        };
        let morph_mv = info.morph_pred.then_some(Mv {
            row: info.block_mv.row,
            col: info.block_mv.col,
        });
        Ok(Self {
            luma,
            chroma,
            morph_mv,
            morph_avail,
            global_fence: global_intrabc_enabled(core.intrabc),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn derive_chroma(
        core: &FrameHeaderCore,
        frontier: &DecodeBlockFrontier,
        info: IntrabcInfo,
        luma_prediction: IntrabcPredictionGeometry,
        sub_x: u32,
        sub_y: u32,
        tile_offset: ByteOffset,
    ) -> Result<Option<IntrabcChromaPrediction>> {
        let prediction = if frontier.chroma_offset {
            let chroma_ref = frontier.chroma_ref_geometry();
            derive_intrabc_luma_prediction_geometry(
                core,
                IntrabcBlockGeometry::from_chroma_ref(
                    chroma_ref.row(),
                    chroma_ref.col(),
                    chroma_ref.size(),
                    tile_offset,
                )?,
                info,
                tile_offset,
            )?
        } else {
            luma_prediction
        };
        let luma = prediction.target;
        let frame_size = core.frame_size.ok_or_else(|| {
            inter_missing!(
                "inter_intrabc_missing_frame_size",
                tile_offset,
                "inter.intrabc.frame_size",
                SPEC_MODE_INFO
            )
        })?;
        let (cx, cy) = (luma.x() >> sub_x, luma.y() >> sub_y);
        let (cw, ch) = (luma.width() >> sub_x, luma.height() >> sub_y);
        if cw == 0 || ch == 0 {
            return Ok(None);
        }
        let scaling = derive_plane_scaling(
            cx as i32,
            cy as i32,
            info.block_mv.row,
            info.block_mv.col,
            sub_x,
            sub_y,
            frame_size.width as i32,
            frame_size.height as i32,
            frame_size.width as i32,
            frame_size.height as i32,
        );
        let target = PlaneRect::new(cx, cy, cw, ch).map_err(|_| {
            inter_cap!(
                "inter_intrabc_chroma_geometry",
                tile_offset,
                "inter.intrabc.chroma.geometry",
                SPEC_MODE_INFO
            )
        })?;
        Ok(Some(IntrabcChromaPrediction { target, scaling }))
    }
}

pub(super) struct IntrabcReconCommand {
    prediction: IntrabcReconPrediction,
    residual: Option<InterResidual>,
    segment_id: u8,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
}

impl IntrabcReconCommand {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        prediction: IntrabcReconPrediction,
        residual: Option<InterResidual>,
        segment_id: u8,
        qindex: u32,
        luma_use_tcq: bool,
        residual_use_ddt: bool,
        bit_depth: BitDepth,
        tile_offset: ByteOffset,
    ) -> Self {
        Self {
            prediction,
            residual,
            segment_id,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        }
    }

    pub(super) const fn requires_global_fence(&self) -> bool {
        self.prediction.global_fence
    }

    pub(super) fn reconstruct<T: ReconSample>(
        self,
        residual_scratch: &mut InterResidualReconScratch<T>,
        residual_blocks: &[InterResidualBlock],
        workspace: &mut CurrentFrameWorkspace<T>,
    ) -> Result<()> {
        let _segment_scope = FrameQmSegmentScope::install(usize::from(self.segment_id));
        self.reconstruct_with_installed_quantizer(residual_scratch, residual_blocks, workspace)
    }

    fn reconstruct_with_installed_quantizer<T: ReconSample>(
        self,
        residual_scratch: &mut InterResidualReconScratch<T>,
        residual_blocks: &[InterResidualBlock],
        workspace: &mut CurrentFrameWorkspace<T>,
    ) -> Result<()> {
        let prediction = self.prediction;
        if prediction.luma.fractional {
            mc::intrabc_predict_fractional_luma_into(
                workspace,
                prediction.luma.target,
                prediction.luma.scaling,
            )?;
        } else {
            mc::intrabc_copy_plane_into(
                workspace,
                PlaneId::Y,
                prediction.luma.source,
                prediction.luma.target,
            )
            .map_err(|_| {
                inter_cap!(
                    "inter_intrabc_copy",
                    self.tile_offset,
                    "inter.intrabc.copy",
                    SPEC_MODE_INFO
                )
            })?;
        }
        if let Some(mv) = prediction.morph_mv {
            crate::prediction::inter::bawp::apply_intrabc_morph_pred(
                workspace,
                prediction.luma.target,
                prediction.morph_avail,
                mv,
            )?;
        }
        if let Some(chroma) = prediction.chroma {
            for plane in [PlaneId::U, PlaneId::V] {
                mc::intrabc_predict_subpel_plane_into(
                    workspace,
                    plane,
                    chroma.target,
                    chroma.scaling,
                )?;
            }
        }
        if let Some(residual) = self.residual.as_ref() {
            add_inter_residual_to_workspace(
                residual_scratch,
                &mut mc::WorkspaceSink::Frame(workspace),
                residual,
                residual_blocks,
                self.qindex,
                self.luma_use_tcq,
                self.residual_use_ddt,
                true,
                self.bit_depth,
                self.tile_offset,
            )?;
        }
        Ok(())
    }
}

pub(crate) fn global_intrabc_enabled(params: Option<IntrabcParams>) -> bool {
    params.is_some_and(|params| params.allow_global_intrabc == Some(true))
}

#[cfg(test)]
mod tests {
    use splot_core::headers::frame::IntrabcParams;

    use super::{IntrabcReconCommand, global_intrabc_enabled};

    #[test]
    fn recon_command_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IntrabcReconCommand>();
    }

    #[test]
    fn global_intrabc_capability_requires_a_fence_even_when_local_is_enabled() {
        let params = |global, local| IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(global),
            allow_local_intrabc: local,
            change_bvp_drl: None,
            max_bvp_drl_bits_minus_1: None,
        };

        assert!(!global_intrabc_enabled(None));
        assert!(!global_intrabc_enabled(Some(params(false, None))));
        assert!(global_intrabc_enabled(Some(params(true, Some(false)))));
        assert!(global_intrabc_enabled(Some(params(true, Some(true)))));
    }
}
