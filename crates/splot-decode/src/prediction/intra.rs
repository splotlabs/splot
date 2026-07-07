// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    GeneralIntraBlockModes, GeneralIntraResidualError, IntraYMode, LumaCoeffBlock, LumaPalette,
    LumaTransformTypeContext, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    TileBlockDecodedState,
};
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::BlockCtx;

const ANGLE_STEP: i32 = 3;
const NON_DC_MIN_N4: usize = 8;
const FULL_SB_N4_LUMA: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraLumaPlan {
    Palette { palette: LumaPalette },
    Dc,
    NonDcFirst { mode: SupportedNonDcLumaMode },
    NonDcNeighbour { mode: SupportedNonDcLumaMode },
    CardinalNeighbour { direction: IntraCardinalDirection },
    PaethNeighbour,
    DirectionalFirst { mode: SupportedDirectionalLumaMode },
    DirectionalNeighbour { mode: SupportedDirectionalLumaMode },
    DirectionalMiddle { p_angle: u16 },
    DirectionalOneSidedAbove { p_angle: u16 },
    DirectionalOneSidedLeft { p_angle: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraLumaUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

impl IntraLumaUnsupported {
    pub(crate) const fn new(reason_id: &'static str, message: &'static str) -> Self {
        Self { reason_id, message }
    }

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
    plan_luma_prediction_ext(modes, block_ctx, false)
}

pub(crate) fn plan_luma_prediction_ext(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
    allow_verified_no_neighbour_cardinal: bool,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    if let Some(palette) = modes.palette_y() {
        return Ok(IntraLumaPlan::Palette { palette });
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
    plan_directional_luma_from_mode(
        modes.y_mode,
        modes.angle_delta_y,
        block_ctx,
        modes.uses_dpcm_y() || allow_verified_no_neighbour_cardinal,
    )
}

impl IntraLumaPlan {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct<T: ReconSample>(
        self,
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
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_nondc_neighbour_block_into(
                    workspace,
                    luma,
                    mode,
                    x,
                    y,
                    log2_side,
                    qindex,
                    use_tcq,
                    num4_above_right,
                    num4_below_left,
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    luma_context,
                    bit_depth,
                )
            }
            Self::CardinalNeighbour { direction } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_cardinal_neighbour_block_into(
                    workspace, luma, direction, PlaneId::Y, x, y, log2_side, log2_side, qindex,
                    use_tcq, Some(luma_context),
                    None,
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
            Self::PaethNeighbour => {
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    workspace, luma, PlaneId::Y, x, y, log2_side, log2_side, qindex, use_tcq,
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                )
            }
            Self::DirectionalFirst { mode } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_directional_first_block_into(
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
            Self::DirectionalNeighbour { mode } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_directional_neighbour_block_into(
                    workspace,
                    luma,
                    mode,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    qindex,
                    use_tcq,
                    Some(luma_context),
                    bit_depth,
                    crate::pipeline::reconstruct::MiddleEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                )
            }
            Self::DirectionalMiddle { p_angle } => {
                let neighbours = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_middle_neighbour_rect_block_into(
                    workspace,
                    luma,
                    p_angle,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    use_tcq,
                    Some(luma_context),
                    bit_depth,
                    crate::pipeline::reconstruct::MiddleEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    crate::pipeline::reconstruct::TwoSidedMiddleEdgeFilters {
                        above: crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                        left: crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                    },
                )
            }
            Self::DirectionalOneSidedAbove { p_angle } => {
                let num4_above_right = block_ctx
                    .neighbours_from_block_decoded(PlaneId::Y, block_decoded)
                    .num_above_right();
                crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_neighbour_block_into(
                    workspace,
                    luma,
                    p_angle,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    num4_above_right,
                    crate::pipeline::reconstruct::OneSidedAboveMrl::default(),
                    use_tcq,
                    Some(luma_context),
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                    crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                )
            }
            Self::DirectionalOneSidedLeft { p_angle } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                    workspace,
                    luma,
                    p_angle,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    neighbours.num_below_left(),
                    neighbours.has_above(),
                    0,
                    use_tcq,
                    Some(luma_context),
                    crate::pipeline::reconstruct::IntraEdgeAvailability {
                        above: neighbours.has_above(),
                        left: neighbours.has_left(),
                    },
                    bit_depth,
                    crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                )
            }
        }
    }
}

#[allow(dead_code)]
fn plan_luma_prediction_from_parts(
    luma_is_dc: bool,
    nondc: Option<SupportedNonDcLumaMode>,
    directional: Option<SupportedDirectionalLumaMode>,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    if luma_is_dc {
        return Ok(IntraLumaPlan::Dc);
    }
    if let Some(mode) = nondc {
        return plan_nondc_luma(mode, block_ctx);
    }
    if let Some(mode) = directional {
        return plan_directional_luma(mode, block_ctx);
    }
    Err(UNSUPPORTED_LUMA_MODE)
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

#[allow(dead_code)]
fn plan_directional_luma(
    mode: SupportedDirectionalLumaMode,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    plan_directional_luma_angle(mode, directional_mode_p_angle(mode), block_ctx, false)
}

fn plan_directional_luma_from_mode(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    block_ctx: BlockCtx,
    allow_no_neighbour_cardinal: bool,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    let mode = y_mode
        .supported_directional()
        .ok_or(UNSUPPORTED_LUMA_MODE)?;
    let p_angle = directional_p_angle(y_mode, angle_delta_y).ok_or(UNSUPPORTED_LUMA_MODE)?;
    plan_directional_luma_angle(mode, p_angle, block_ctx, allow_no_neighbour_cardinal)
}

fn directional_p_angle(y_mode: IntraYMode, angle_delta_y: i8) -> Option<u16> {
    let angle =
        i32::from(y_mode.mode_to_angle()?).checked_add(i32::from(angle_delta_y) * ANGLE_STEP)?;
    u16::try_from(angle).ok()
}

pub(crate) const fn directional_mode_p_angle(mode: SupportedDirectionalLumaMode) -> u16 {
    match mode {
        SupportedDirectionalLumaMode::Vertical => 90,
        SupportedDirectionalLumaMode::Horizontal => 180,
        SupportedDirectionalLumaMode::D113 => 113,
        SupportedDirectionalLumaMode::D135 => 135,
        SupportedDirectionalLumaMode::D157 => 157,
        SupportedDirectionalLumaMode::D45 => 45,
        SupportedDirectionalLumaMode::D203 => 203,
        SupportedDirectionalLumaMode::D67 => 67,
    }
}

fn plan_directional_luma_angle(
    mode: SupportedDirectionalLumaMode,
    p_angle: u16,
    block_ctx: BlockCtx,
    allow_no_neighbour_cardinal: bool,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    let is_full_sb = block_ctx.block().width4() == FULL_SB_N4_LUMA;
    let is_top_left = block_ctx.is_top_left();
    let has_above = neighbours.has_above();
    let has_left = neighbours.has_left();
    let has_edge = has_above || has_left;
    let supports_small_cardinal_edge =
        has_edge && block_ctx.block().width4() >= 2 && block_ctx.block().height4() >= 2;
    let supports_small_one_sided_left =
        has_left && block_ctx.block().width4() >= 2 && block_ctx.block().height4() >= 2;
    let supports_small_one_sided_above = has_above
        && has_left
        && block_ctx.block().width4() >= 2
        && block_ctx.block().height4() >= 2
        && neighbours.num_above_right() > 0;
    let full_sb_first_row = is_full_sb && !has_above;
    let full_sb_with_edge = is_full_sb && has_edge;
    let full_sb_no_neighbour_cardinal = is_full_sb && is_top_left && allow_no_neighbour_cardinal;
    let full_sb_with_above = is_full_sb && has_above;
    let full_sb_with_left = is_full_sb && has_left;
    let full_sb_left_only = full_sb_first_row && has_left;
    let full_sb_no_above_left = full_sb_left_only;
    let full_sb_above_left = full_sb_with_above && has_left;
    let full_sb_above_left_with_above_right =
        full_sb_above_left && neighbours.num_above_right() > 0;
    if p_angle > 90 && p_angle < 180 {
        let top_row_left = !has_above && has_left;
        let needs_exact_angle_or_d113_top_row = p_angle != directional_mode_p_angle(mode)
            || (matches!(mode, SupportedDirectionalLumaMode::D113) && top_row_left);
        let exact_d157_subblock =
            matches!(mode, SupportedDirectionalLumaMode::D157) && !is_full_sb && has_above;
        if (needs_exact_angle_or_d113_top_row || exact_d157_subblock)
            && has_left
            && (has_above || top_row_left)
        {
            return Ok(IntraLumaPlan::DirectionalMiddle { p_angle });
        }
    }
    if p_angle > 0 && p_angle < 90 {
        return (full_sb_above_left_with_above_right
            || supports_small_one_sided_above
            || full_sb_no_above_left)
            .then_some(IntraLumaPlan::DirectionalOneSidedAbove { p_angle })
            .ok_or(UNSUPPORTED_D45_POSITION);
    }
    if p_angle > 180 && p_angle < 270 {
        return (full_sb_with_left || supports_small_one_sided_left)
            .then_some(IntraLumaPlan::DirectionalOneSidedLeft { p_angle })
            .ok_or(UNSUPPORTED_D203_POSITION);
    }
    match mode {
        SupportedDirectionalLumaMode::Vertical => {
            (full_sb_with_edge || supports_small_cardinal_edge || full_sb_no_neighbour_cardinal)
                .then_some(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Vertical,
                })
                .ok_or(UNSUPPORTED_CARDINAL_VERTICAL)
        }
        SupportedDirectionalLumaMode::Horizontal => {
            (full_sb_with_edge || supports_small_cardinal_edge || full_sb_no_neighbour_cardinal)
                .then_some(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Horizontal,
                })
                .ok_or(UNSUPPORTED_CARDINAL_HORIZONTAL)
        }
        SupportedDirectionalLumaMode::D157 => full_sb_left_only
            .then_some(IntraLumaPlan::DirectionalNeighbour { mode })
            .ok_or(UNSUPPORTED_D157_POSITION),
        SupportedDirectionalLumaMode::D113 => full_sb_above_left
            .then_some(IntraLumaPlan::DirectionalNeighbour { mode })
            .ok_or(UNSUPPORTED_D113_POSITION),
        SupportedDirectionalLumaMode::D45 => full_sb_above_left_with_above_right
            .then_some(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 45 })
            .ok_or(UNSUPPORTED_D45_POSITION),
        SupportedDirectionalLumaMode::D67 => full_sb_above_left_with_above_right
            .then_some(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 67 })
            .ok_or(UNSUPPORTED_D45_POSITION),
        SupportedDirectionalLumaMode::D203 => full_sb_left_only
            .then_some(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 203 })
            .ok_or(UNSUPPORTED_D203_POSITION),
        SupportedDirectionalLumaMode::D135 => {
            if is_top_left && is_full_sb {
                Ok(IntraLumaPlan::DirectionalFirst { mode })
            } else if full_sb_first_row || full_sb_above_left || (!is_top_left && has_edge) {
                Ok(IntraLumaPlan::DirectionalNeighbour { mode })
            } else if !is_top_left {
                Err(UNSUPPORTED_MULTIBLOCK_DIRECTIONAL)
            } else {
                Err(UNSUPPORTED_DIRECTIONAL_NON_DCTONLY_SIZE)
            }
        }
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

const UNSUPPORTED_CARDINAL_VERTICAL: IntraLumaUnsupported = unsupported(
    "general_intra_cardinal_vertical_unverified",
    missing_capability_message!(
        "intra.luma.cardinal.vertical",
        neighbour = "above",
        block = "non_full_sb_or_first_row",
    ),
);

const UNSUPPORTED_CARDINAL_HORIZONTAL: IntraLumaUnsupported = unsupported(
    "general_intra_cardinal_horizontal_unverified",
    missing_capability_message!(
        "intra.luma.cardinal.horizontal",
        neighbour = "left",
        block = "non_full_sb_or_first_col",
    ),
);

const UNSUPPORTED_D157_POSITION: IntraLumaUnsupported = unsupported(
    "general_intra_d157_unverified_position",
    missing_capability_message!(
        "intra.luma.directional.d157",
        neighbour = "left_only",
        block = "non_full_sb_or_not_first_row",
    ),
);

const UNSUPPORTED_D113_POSITION: IntraLumaUnsupported = unsupported(
    "general_intra_d113_unverified_position",
    missing_capability_message!(
        "intra.luma.directional.d113",
        neighbour = "above_left",
        block = "non_full_sb_or_edge",
    ),
);

const UNSUPPORTED_D45_POSITION: IntraLumaUnsupported = unsupported(
    "general_intra_d45_unverified_position",
    missing_capability_message!(
        "intra.luma.directional.d45",
        neighbour = "above_right",
        block = "non_full_sb_or_edge",
    ),
);

const UNSUPPORTED_D203_POSITION: IntraLumaUnsupported = unsupported(
    "general_intra_d203_unverified_position",
    missing_capability_message!(
        "intra.luma.directional.d203",
        neighbour = "left_below_left",
        block = "non_full_sb_or_not_first_row",
    ),
);

const UNSUPPORTED_MULTIBLOCK_DIRECTIONAL: IntraLumaUnsupported = unsupported(
    "general_intra_multiblock_directional_subblock",
    missing_capability_message!(
        "intra.luma.directional.subpartition",
        neighbour = "block_decoded_state",
    ),
);

const UNSUPPORTED_DIRECTIONAL_NON_DCTONLY_SIZE: IntraLumaUnsupported = unsupported(
    "general_intra_directional_non_dctonly_size",
    missing_capability_message!("intra.luma.directional.transform_set", block = "non_64x64"),
);

const UNSUPPORTED_LUMA_MODE: IntraLumaUnsupported = unsupported(
    "general_intra_unsupported_luma_mode",
    missing_capability_message!("intra.luma.mode", mode = "unsupported"),
);

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::tile::block_context::{BlockRect, ChromaSampling, TxShape};
    use splot_recon::{BitDepth, IntraRectBlockSize, PixelFormat};

    #[derive(Clone, Copy)]
    struct Case {
        label: &'static str,
        bit_depth: BitDepth,
        row4: usize,
        col4: usize,
        width4: usize,
        height4: usize,
        frame_cols4: usize,
        dc: bool,
        nondc: Option<SupportedNonDcLumaMode>,
        directional: Option<SupportedDirectionalLumaMode>,
        expected: Expected,
    }

    #[derive(Clone, Copy)]
    enum Expected {
        Plan(IntraLumaPlan),
        Error(&'static str),
    }

    fn ctx(case: Case) -> BlockCtx {
        let Some(tx) = TxShape::from_luma_4x4(case.width4, case.height4) else {
            panic!("invalid test transform for {}", case.label);
        };
        BlockCtx::new(
            BlockRect::new(case.row4, case.col4, case.width4, case.height4),
            tx,
            case.frame_cols4,
            32,
            case.bit_depth,
            ChromaSampling::Yuv420,
        )
    }

    fn all_zero_luma_block() -> LumaCoeffBlock {
        LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
            intra_ist: None,
            plane_tx_type: 0,
            lossless: false,
        }
    }

    fn workspace_with_tile_boundary_edges(above: u8) -> CurrentFrameWorkspace<u8> {
        let mut ws = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
            64,
            64,
            BitDepth::Eight,
            PixelFormat::Yuv420,
        )
        .unwrap();
        ws.write_rect_block(
            PlaneId::Y,
            8,
            4,
            IntraRectBlockSize::new(3, 2).unwrap(),
            &[above; 32],
        )
        .unwrap();
        let left: [u8; 8] = [40, 45, 50, 55, 60, 65, 70, 75];
        let mut left_block = vec![0u8; 4 * 8];
        for (row, &sample) in left.iter().enumerate() {
            for col in 0..4 {
                left_block[row * 4 + col] = sample;
            }
        }
        ws.write_rect_block(
            PlaneId::Y,
            4,
            8,
            IntraRectBlockSize::new(2, 3).unwrap(),
            &left_block,
        )
        .unwrap();
        ws
    }

    fn tile_top_block_ctx() -> BlockCtx {
        BlockCtx::new(
            BlockRect::new(2, 2, 2, 2),
            TxShape::from_luma_4x4(2, 2).unwrap(),
            16,
            16,
            BitDepth::Eight,
            ChromaSampling::Yuv420,
        )
        .with_tile_bounds(2, 16, 0, 16)
    }

    fn reconstruct_plan_samples(plan: IntraLumaPlan, above: u8) -> Vec<u8> {
        let mut ws = workspace_with_tile_boundary_edges(above);
        let block_decoded = TileBlockDecodedState::new(3, 1, 1, 16, 16, 16).unwrap();
        plan.reconstruct(
            &mut ws,
            &all_zero_luma_block(),
            tile_top_block_ctx(),
            &block_decoded,
            0,
            false,
            false,
            LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0),
        )
        .unwrap();
        (0..8)
            .flat_map(|row| (0..8).map(move |col| (row, col)))
            .map(|(row, col)| {
                ws.reconstructed_sample(PlaneId::Y, 8 + col, 8 + row)
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn square_luma_reconstruct_masks_tile_unavailable_above_edge() {
        let cases = [
            IntraLumaPlan::Dc,
            IntraLumaPlan::NonDcNeighbour {
                mode: SupportedNonDcLumaMode::SmoothVertical,
            },
        ];
        for plan in cases {
            let low_above = reconstruct_plan_samples(plan, 7);
            let high_above = reconstruct_plan_samples(plan, 240);
            assert_eq!(
                low_above, high_above,
                "square luma {plan:?} must ignore the tile-unavailable above edge"
            );
        }
    }

    #[test]
    fn plans_supported_luma_prediction_classes() {
        let cases = [
            Case {
                label: "dc 10-bit top-left",
                bit_depth: BitDepth::Ten,
                row4: 0,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 16,
                dc: true,
                nondc: None,
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::Dc),
            },
            Case {
                label: "smooth top-left",
                bit_depth: BitDepth::Eight,
                row4: 0,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 16,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcFirst {
                    mode: SupportedNonDcLumaMode::Smooth,
                }),
            },
            Case {
                label: "smooth full-sb neighbour",
                bit_depth: BitDepth::Ten,
                row4: 32,
                col4: 64,
                width4: 16,
                height4: 16,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::Smooth,
                }),
            },
            Case {
                label: "smooth horizontal subpartition",
                bit_depth: BitDepth::Eight,
                row4: 8,
                col4: 0,
                width4: 8,
                height4: 8,
                frame_cols4: 16,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::SmoothHorizontal),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::SmoothHorizontal,
                }),
            },
            Case {
                label: "smooth horizontal small interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 42,
                col4: 302,
                width4: 2,
                height4: 2,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::SmoothHorizontal),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::SmoothHorizontal,
                }),
            },
            Case {
                label: "smooth horizontal row-aligned subpartition",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 0,
                width4: 8,
                height4: 8,
                frame_cols4: 16,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::SmoothHorizontal),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::SmoothHorizontal,
                }),
            },
            Case {
                label: "smooth vertical interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 24,
                col4: 192,
                width4: 8,
                height4: 8,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::SmoothVertical),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::SmoothVertical,
                }),
            },
            Case {
                label: "smooth interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 24,
                col4: 192,
                width4: 8,
                height4: 8,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::Smooth,
                }),
            },
            Case {
                label: "small smooth interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 24,
                col4: 202,
                width4: 2,
                height4: 2,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::Smooth,
                }),
            },
            Case {
                label: "4x4 smooth interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 31,
                col4: 296,
                width4: 1,
                height4: 1,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::Smooth,
                }),
            },
            Case {
                label: "4x4 smooth vertical interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 23,
                col4: 306,
                width4: 1,
                height4: 1,
                frame_cols4: 480,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::SmoothVertical),
                directional: None,
                expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                    mode: SupportedNonDcLumaMode::SmoothVertical,
                }),
            },
            Case {
                label: "vertical cardinal",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Vertical),
                expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Vertical,
                }),
            },
            Case {
                label: "vertical cardinal first row left fallback",
                bit_depth: BitDepth::Ten,
                row4: 0,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Vertical),
                expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Vertical,
                }),
            },
            Case {
                label: "horizontal cardinal first column above fallback",
                bit_depth: BitDepth::Ten,
                row4: 80,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Horizontal),
                expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Horizontal,
                }),
            },
            Case {
                label: "small vertical cardinal",
                bit_depth: BitDepth::Ten,
                row4: 20,
                col4: 218,
                width4: 2,
                height4: 2,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Vertical),
                expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Vertical,
                }),
            },
            Case {
                label: "small horizontal cardinal",
                bit_depth: BitDepth::Ten,
                row4: 12,
                col4: 266,
                width4: 2,
                height4: 2,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Horizontal),
                expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                    direction: IntraCardinalDirection::Horizontal,
                }),
            },
            Case {
                label: "d135 first row",
                bit_depth: BitDepth::Eight,
                row4: 0,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D135),
                expected: Expected::Plan(IntraLumaPlan::DirectionalNeighbour {
                    mode: SupportedDirectionalLumaMode::D135,
                }),
            },
            Case {
                label: "d135 interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 16,
                col4: 208,
                width4: 8,
                height4: 8,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D135),
                expected: Expected::Plan(IntraLumaPlan::DirectionalNeighbour {
                    mode: SupportedDirectionalLumaMode::D135,
                }),
            },
            Case {
                label: "d135 top row left-only subpartition",
                bit_depth: BitDepth::Ten,
                row4: 0,
                col4: 9,
                width4: 1,
                height4: 1,
                frame_cols4: 16,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D135),
                expected: Expected::Plan(IntraLumaPlan::DirectionalNeighbour {
                    mode: SupportedDirectionalLumaMode::D135,
                }),
            },
            Case {
                label: "d135 first column above-only",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D135),
                expected: Expected::Plan(IntraLumaPlan::DirectionalNeighbour {
                    mode: SupportedDirectionalLumaMode::D135,
                }),
            },
            Case {
                label: "d157 interior subpartition",
                bit_depth: BitDepth::Ten,
                row4: 40,
                col4: 302,
                width4: 2,
                height4: 2,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D157),
                expected: Expected::Plan(IntraLumaPlan::DirectionalMiddle { p_angle: 157 }),
            },
            Case {
                label: "d45 above-right",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 48,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D45),
                expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 45 }),
            },
            Case {
                label: "d67 above-right",
                bit_depth: BitDepth::Ten,
                row4: 16,
                col4: 240,
                width4: 16,
                height4: 16,
                frame_cols4: 480,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D67),
                expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 67 }),
            },
            Case {
                label: "d203 first row",
                bit_depth: BitDepth::Eight,
                row4: 0,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D203),
                expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 203 }),
            },
        ];

        for case in cases {
            assert_case(case);
        }
    }

    #[test]
    fn plans_angle_delta_d203_as_one_sided_left() {
        let case = Case {
            label: "d203 angle delta above-left",
            bit_depth: BitDepth::Ten,
            row4: 16,
            col4: 208,
            width4: 16,
            height4: 16,
            frame_cols4: 480,
            dc: false,
            nondc: None,
            directional: None,
            expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 209 }),
        };

        assert_eq!(
            plan_directional_luma_angle(SupportedDirectionalLumaMode::D203, 209, ctx(case), false),
            Ok(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 209 })
        );
    }

    #[test]
    fn plans_small_angle_delta_hpred_as_one_sided_left() {
        let case = Case {
            label: "hpred angle delta small block",
            bit_depth: BitDepth::Ten,
            row4: 26,
            col4: 202,
            width4: 2,
            height4: 2,
            frame_cols4: 480,
            dc: false,
            nondc: None,
            directional: None,
            expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 189 }),
        };

        assert_eq!(
            plan_directional_luma_angle(
                SupportedDirectionalLumaMode::Horizontal,
                189,
                ctx(case),
                false,
            ),
            Ok(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 189 })
        );
    }

    #[test]
    fn plans_small_angle_delta_vpred_as_one_sided_above() {
        let case = Case {
            label: "small V angle delta above",
            bit_depth: BitDepth::Ten,
            row4: 20,
            col4: 218,
            width4: 2,
            height4: 2,
            frame_cols4: 480,
            dc: false,
            nondc: None,
            directional: Some(SupportedDirectionalLumaMode::Vertical),
            expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 84 }),
        };

        assert_eq!(
            plan_directional_luma_angle(
                SupportedDirectionalLumaMode::Vertical,
                84,
                ctx(case),
                false
            ),
            Ok(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 84 })
        );
    }

    #[test]
    fn plans_top_row_d45_angle_delta_as_one_sided_above_fallback() {
        let case = Case {
            label: "top row D45 angle delta left fallback",
            bit_depth: BitDepth::Ten,
            row4: 0,
            col4: 224,
            width4: 16,
            height4: 16,
            frame_cols4: 480,
            dc: false,
            nondc: None,
            directional: Some(SupportedDirectionalLumaMode::D45),
            expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 36 }),
        };

        assert_eq!(
            plan_directional_luma_angle(SupportedDirectionalLumaMode::D45, 36, ctx(case), false),
            Ok(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 36 })
        );
    }

    #[test]
    fn plans_top_row_d113_angle_delta_as_middle_fallback() {
        let case = Case {
            label: "top row D113 angle delta left fallback",
            bit_depth: BitDepth::Ten,
            row4: 0,
            col4: 288,
            width4: 32,
            height4: 32,
            frame_cols4: 480,
            dc: false,
            nondc: None,
            directional: Some(SupportedDirectionalLumaMode::D113),
            expected: Expected::Plan(IntraLumaPlan::DirectionalMiddle { p_angle: 119 }),
        };

        assert_eq!(
            plan_directional_luma_angle(SupportedDirectionalLumaMode::D113, 119, ctx(case), false),
            Ok(IntraLumaPlan::DirectionalMiddle { p_angle: 119 })
        );
    }

    #[test]
    fn rejects_unsupported_luma_prediction_classes() {
        let cases = [
            Case {
                label: "vertical cardinal no-neighbour",
                bit_depth: BitDepth::Eight,
                row4: 0,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 16,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Vertical),
                expected: Expected::Error("general_intra_cardinal_vertical_unverified"),
            },
            Case {
                label: "horizontal cardinal no-neighbour",
                bit_depth: BitDepth::Eight,
                row4: 0,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 16,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Horizontal),
                expected: Expected::Error("general_intra_cardinal_horizontal_unverified"),
            },
            Case {
                label: "4x4 vertical cardinal",
                bit_depth: BitDepth::Eight,
                row4: 1,
                col4: 1,
                width4: 1,
                height4: 1,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Vertical),
                expected: Expected::Error("general_intra_cardinal_vertical_unverified"),
            },
            Case {
                label: "4x4 horizontal cardinal",
                bit_depth: BitDepth::Eight,
                row4: 1,
                col4: 1,
                width4: 1,
                height4: 1,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::Horizontal),
                expected: Expected::Error("general_intra_cardinal_horizontal_unverified"),
            },
            Case {
                label: "d45 right edge",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D45),
                expected: Expected::Error("general_intra_d45_unverified_position"),
            },
        ];

        for case in cases {
            assert_case(case);
        }
    }

    #[test]
    fn plans_verified_cardinal_no_neighbour_luma_with_explicit_admission() {
        let case = Case {
            label: "explicit cardinal no-neighbour",
            bit_depth: BitDepth::Eight,
            row4: 0,
            col4: 0,
            width4: 16,
            height4: 16,
            frame_cols4: 16,
            dc: false,
            nondc: None,
            directional: None,
            expected: Expected::Error("unused"),
        };

        for (y_mode, direction) in [
            (
                IntraYMode::V_PRED_FOR_TEST,
                IntraCardinalDirection::Vertical,
            ),
            (
                IntraYMode::H_PRED_FOR_TEST,
                IntraCardinalDirection::Horizontal,
            ),
        ] {
            let modes = GeneralIntraBlockModes::luma_only(
                crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
                    y_mode,
                    angle_delta_y: 0,
                    intra_joint_mode: 0,
                    mrl_index: 0,
                    mrl_sec_index: None,
                    fsc_mode: 0,
                    uses_mrls: 0,
                    use_dpcm_y: 0,
                    dpcm_mode_y: 0,
                },
            );

            assert!(plan_luma_prediction(&modes, ctx(case)).is_err());
            assert_eq!(
                plan_luma_prediction_ext(&modes, ctx(case), true).unwrap(),
                IntraLumaPlan::CardinalNeighbour { direction }
            );
        }
    }

    #[test]
    fn plans_dpcm_cardinal_no_neighbour_luma() {
        let modes = GeneralIntraBlockModes::luma_only(
            crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
                y_mode: IntraYMode::dpcm_horizontal(),
                angle_delta_y: 0,
                intra_joint_mode: 0,
                mrl_index: 0,
                mrl_sec_index: None,
                fsc_mode: 0,
                uses_mrls: 0,
                use_dpcm_y: 1,
                dpcm_mode_y: 1,
            },
        );
        let case = Case {
            label: "horizontal dpcm cardinal no-neighbour",
            bit_depth: BitDepth::Eight,
            row4: 0,
            col4: 0,
            width4: 16,
            height4: 16,
            frame_cols4: 16,
            dc: false,
            nondc: None,
            directional: Some(SupportedDirectionalLumaMode::Horizontal),
            expected: Expected::Plan(IntraLumaPlan::CardinalNeighbour {
                direction: IntraCardinalDirection::Horizontal,
            }),
        };

        assert_eq!(
            plan_luma_prediction(&modes, ctx(case)).unwrap(),
            IntraLumaPlan::CardinalNeighbour {
                direction: IntraCardinalDirection::Horizontal,
            }
        );
    }

    fn assert_case(case: Case) {
        let actual =
            plan_luma_prediction_from_parts(case.dc, case.nondc, case.directional, ctx(case));
        match (actual, case.expected) {
            (Ok(actual), Expected::Plan(expected)) => {
                assert_eq!(actual, expected, "{}", case.label);
            }
            (Err(actual), Expected::Error(expected)) => {
                assert_eq!(actual.reason_id(), expected, "{}", case.label);
                assert!(
                    actual.message().starts_with("unsupported capability: "),
                    "{}",
                    case.label
                );
            }
            (Ok(actual), Expected::Error(expected)) => {
                panic!("{}: expected error {expected}, got {actual:?}", case.label);
            }
            (Err(actual), Expected::Plan(expected)) => {
                panic!(
                    "{}: expected plan {expected:?}, got error {:?}",
                    case.label,
                    actual.reason_id()
                );
            }
        }
    }
}
