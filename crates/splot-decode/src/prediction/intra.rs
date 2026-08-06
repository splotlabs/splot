// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    GeneralIntraBlockModes, GeneralIntraResidualError, LumaCoeffBlock, LumaPalette,
    LumaTransformTypeContext, SupportedNonDcLumaMode, TileBlockDecodedState,
};
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::BlockCtx;

const NON_DC_MIN_N4: usize = 8;
const FULL_SB_N4_LUMA: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraLumaPlan {
    Palette { palette: LumaPalette },
    Dip { mode: u8, transpose: bool },
    Dc,
    NonDcFirst { mode: SupportedNonDcLumaMode },
    NonDcNeighbour { mode: SupportedNonDcLumaMode },
    PaethNeighbour,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraLumaUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

impl IntraLumaUnsupported {
    pub(crate) const fn reason_id(self) -> &'static str {
        self.reason_id
    }

    pub(crate) const fn message(self) -> &'static str {
        self.message
    }
}

pub(crate) fn plan_luma_prediction(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    if let Some(palette) = modes.palette_y() {
        return Ok(IntraLumaPlan::Palette { palette });
    }
    if modes.uses_active_dip() {
        return Ok(IntraLumaPlan::Dip {
            mode: modes.dip_mode,
            transpose: modes.dip_transpose != 0,
        });
    }
    if modes.luma_is_dc() {
        return Ok(IntraLumaPlan::Dc);
    }
    if modes.y_mode.is_paeth() {
        return Ok(IntraLumaPlan::PaethNeighbour);
    }
    if let Some(mode) = modes.supported_nondc_luma() {
        return plan_nondc_luma(mode, block_ctx);
    }
    Err(UNSUPPORTED_LUMA_MODE)
}

impl IntraLumaPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct<T: ReconSample>(
        self,
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        luma: &LumaCoeffBlock,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
        qindex: u32,
        use_tcq: bool,
        enable_ibp: bool,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let luma_block = block_ctx.plane_block(PlaneId::Y);
        let tx = luma_block.tx();
        if !tx.is_square() {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
        let x = luma_block.x();
        let y = luma_block.y();
        let log2_side = tx.width_log2();
        let bit_depth = block_ctx.bit_depth();
        let neighbours = block_ctx.neighbours(PlaneId::Y);
        match self {
            Self::Palette { .. } => Err(GeneralIntraResidualError::UnexpectedBranch),
            Self::Dip { mode, transpose } => {
                let decode_aware =
                    block_ctx.neighbours_from_block_decoded(PlaneId::Y, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_dip_rect_block_into(
                    workspace,
                    luma,
                    mode,
                    transpose,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    use_tcq,
                    decode_aware.num_above_right(),
                    decode_aware.num_below_left(),
                    luma_context,
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
            Self::Dc => {
                let ibp_dc = enable_ibp && log2_side != 2;
                crate::pipeline::reconstruct::reconstruct_general_intra_block_rect_with_availability_into(
                    workspace,
                    luma,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    use_tcq,
                    ibp_dc,
                    Some(luma_context),
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
            Self::NonDcFirst { mode } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_nondc_first_block_into(
                    workspace,
                    luma,
                    mode,
                    x,
                    y,
                    log2_side,
                    qindex,
                    use_tcq,
                    luma_context,
                    bit_depth,
                )
            }
            Self::NonDcNeighbour { mode } => {
                let decode_aware =
                    block_ctx.neighbours_from_block_decoded(PlaneId::Y, block_decoded);
                let num4_above_right = decode_aware.num_above_right();
                let num4_below_left = decode_aware.num_below_left();
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_smooth_rect_block_with_availability_into(
                    workspace,
                    luma,
                    mode,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    use_tcq,
                    num4_above_right,
                    num4_below_left,
                    Some(luma_context),
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
            Self::PaethNeighbour => {
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    scratch,
                    workspace, luma, PlaneId::Y, x, y, log2_side, log2_side, qindex, use_tcq,
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
        }
    }
}

fn plan_nondc_luma(
    mode: SupportedNonDcLumaMode,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    let block = block_ctx.block();
    let n4w = block.width4();
    let is_top_left = block_ctx.is_top_left();
    let is_full_sb = n4w == FULL_SB_N4_LUMA;
    let is_non_dc_size = n4w >= NON_DC_MIN_N4;
    let is_smooth_subblock_size = block.width4() >= 1 && block.height4() >= 1;
    let is_smooth_axis_subblock_size = block.width4() >= 1 && block.height4() >= 1;
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    let has_edge = neighbours.has_above() || neighbours.has_left();
    match mode {
        SupportedNonDcLumaMode::Smooth if is_top_left && is_full_sb => {
            Ok(IntraLumaPlan::NonDcFirst { mode })
        }
        SupportedNonDcLumaMode::Smooth if is_full_sb && has_edge => {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        SupportedNonDcLumaMode::SmoothVertical | SupportedNonDcLumaMode::SmoothHorizontal
            if is_top_left && is_non_dc_size =>
        {
            Ok(IntraLumaPlan::NonDcFirst { mode })
        }
        SupportedNonDcLumaMode::SmoothVertical | SupportedNonDcLumaMode::SmoothHorizontal
            if is_full_sb =>
        {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        _ if is_top_left => Err(UNSUPPORTED_NON_DC_NON_DCTONLY_SIZE),
        SupportedNonDcLumaMode::SmoothHorizontal if is_smooth_axis_subblock_size => {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        SupportedNonDcLumaMode::Smooth
            if is_smooth_subblock_size
                && !is_full_sb
                && neighbours.has_above()
                && neighbours.has_left() =>
        {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        SupportedNonDcLumaMode::SmoothVertical if is_smooth_axis_subblock_size && has_edge => {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        _ => Err(UNSUPPORTED_MULTIBLOCK_NON_DC_SUBBLOCK),
    }
}

const fn unsupported(reason_id: &'static str, message: &'static str) -> IntraLumaUnsupported {
    IntraLumaUnsupported { reason_id, message }
}

const UNSUPPORTED_NON_DC_NON_DCTONLY_SIZE: IntraLumaUnsupported = unsupported(
    "general_intra_non_dc_non_dctonly_size",
    missing_capability_message!(
        "intra.luma.non_dc.transform_set",
        block = "smaller_than_32x32"
    ),
);

const UNSUPPORTED_MULTIBLOCK_NON_DC_SUBBLOCK: IntraLumaUnsupported = unsupported(
    "general_intra_multiblock_non_dc_subblock",
    missing_capability_message!(
        "intra.luma.smooth_v.below_left",
        neighbour = "below_left",
        block = "subpartition",
    ),
);

pub(crate) const UNSUPPORTED_LUMA_MODE: IntraLumaUnsupported = unsupported(
    "general_intra_unsupported_luma_mode",
    missing_capability_message!("intra.luma.mode", mode = "unsupported"),
);

#[cfg(test)]
#[path = "intra_tests.rs"]
mod tests;
