// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, ReconSample};

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
    Dip { mode: u8, transpose: bool },
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
    allow_verified_no_neighbour_cardinal: bool,
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
    plan_directional_luma_from_mode(
        modes.y_mode,
        modes.angle_delta_y,
        block_ctx,
        modes.uses_dpcm_y() || allow_verified_no_neighbour_cardinal,
    )
}

impl IntraLumaPlan {
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
    let full_sb_top_left_no_neighbour = is_full_sb && is_top_left && !has_edge;
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
        if full_sb_no_neighbour_cardinal
            && matches!(mode, SupportedDirectionalLumaMode::Vertical)
            && p_angle == 96
        {
            return Ok(IntraLumaPlan::DirectionalMiddle { p_angle });
        }
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
            || full_sb_no_above_left
            || (full_sb_top_left_no_neighbour
                && block_ctx.bit_depth() == BitDepth::Eight
                && matches!(
                    mode,
                    SupportedDirectionalLumaMode::D45 | SupportedDirectionalLumaMode::D67
                )
                && p_angle == directional_mode_p_angle(mode)))
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
#[path = "intra_tests.rs"]
mod tests;
