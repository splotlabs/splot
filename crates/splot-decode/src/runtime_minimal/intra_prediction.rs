// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, ReconSample};

use super::block_context::BlockCtx;
use super::capability::missing_capability_message;
use crate::tile_payload::{
    GeneralIntraBlockModes, GeneralIntraResidualError, LumaCoeffBlock,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode, TileBlockDecodedState,
};

const NON_DC_MIN_N4: usize = 8;
const FULL_SB_N4_LUMA: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IntraLumaPlan {
    Dc,
    NonDcFirst { mode: SupportedNonDcLumaMode },
    NonDcNeighbour { mode: SupportedNonDcLumaMode },
    CardinalNeighbour { direction: IntraCardinalDirection },
    DirectionalFirst { mode: SupportedDirectionalLumaMode },
    DirectionalNeighbour { mode: SupportedDirectionalLumaMode },
    DirectionalOneSidedAbove { p_angle: u16 },
    DirectionalOneSidedLeft { p_angle: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntraLumaUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

impl IntraLumaUnsupported {
    pub(super) const fn reason_id(self) -> &'static str {
        self.reason_id
    }

    pub(super) const fn message(self) -> &'static str {
        self.message
    }
}

pub(super) fn plan_luma_prediction(
    modes: &GeneralIntraBlockModes,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    plan_luma_prediction_from_parts(
        modes.luma_is_dc(),
        modes.supported_nondc_luma(),
        modes.supported_directional_luma(),
        block_ctx,
    )
}

impl IntraLumaPlan {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct<T: ReconSample>(
        self,
        workspace: &mut CurrentFrameWorkspace<T>,
        luma: &LumaCoeffBlock,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
        qindex: u32,
        use_tcq: bool,
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
            Self::Dc => crate::runtime_minimal_recon::reconstruct_general_intra_block_into(
                workspace, luma, PlaneId::Y, x, y, log2_side, qindex, use_tcq, bit_depth,
            ),
            Self::NonDcFirst { mode } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_nondc_first_block_into(
                    workspace, luma, mode, x, y, log2_side, qindex, use_tcq, bit_depth,
                )
            }
            Self::NonDcNeighbour { mode } => {
                let num4_above_right = block_ctx
                    .neighbours_from_block_decoded(PlaneId::Y, block_decoded)
                    .num_above_right();
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_nondc_neighbour_block_into(
                    workspace,
                    luma,
                    mode,
                    x,
                    y,
                    log2_side,
                    qindex,
                    use_tcq,
                    num4_above_right,
                    bit_depth,
                )
            }
            Self::CardinalNeighbour { direction } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_cardinal_neighbour_block_into(
                    workspace, luma, direction, PlaneId::Y, x, y, log2_side, log2_side, qindex,
                    use_tcq, bit_depth,
                )
            }
            Self::DirectionalFirst { mode } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_directional_first_block_into(
                    workspace, luma, mode, x, y, log2_side, qindex, use_tcq, bit_depth,
                )
            }
            Self::DirectionalNeighbour { mode } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_directional_neighbour_block_into(
                    workspace, luma, mode, PlaneId::Y, x, y, log2_side, qindex, use_tcq, bit_depth,
                )
            }
            Self::DirectionalOneSidedAbove { p_angle } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_neighbour_block_into(
                    workspace,
                    luma,
                    p_angle,
                    PlaneId::Y,
                    x,
                    y,
                    log2_side,
                    log2_side,
                    qindex,
                    neighbours.num_above_right(),
                    crate::runtime_minimal_recon::OneSidedAboveMrl::default(),
                    use_tcq,
                    bit_depth,
                    crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
                )
            }
            Self::DirectionalOneSidedLeft { p_angle } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_left_neighbour_block_into(
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
                    bit_depth,
                    crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
                )
            }
        }
    }
}

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
    match mode {
        SupportedNonDcLumaMode::Smooth if is_top_left && is_full_sb => {
            Ok(IntraLumaPlan::NonDcFirst { mode })
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
        SupportedNonDcLumaMode::SmoothHorizontal
            if is_non_dc_size && !block.is_row_aligned_to(FULL_SB_N4_LUMA) =>
        {
            Ok(IntraLumaPlan::NonDcNeighbour { mode })
        }
        SupportedNonDcLumaMode::SmoothHorizontal if is_non_dc_size => {
            Err(UNSUPPORTED_SMOOTH_H_ABOVE_RIGHT)
        }
        _ => Err(UNSUPPORTED_MULTIBLOCK_NON_DC_SUBBLOCK),
    }
}

fn plan_directional_luma(
    mode: SupportedDirectionalLumaMode,
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    let neighbours = block_ctx.neighbours(PlaneId::Y);
    let is_full_sb = block_ctx.block().width4() == FULL_SB_N4_LUMA;
    let is_top_left = block_ctx.is_top_left();
    let has_above = neighbours.has_above();
    let has_left = neighbours.has_left();
    let full_sb_first_row = is_full_sb && !has_above;
    let full_sb_with_above = is_full_sb && has_above;
    let full_sb_with_left = is_full_sb && has_left;
    let full_sb_left_only = full_sb_first_row && has_left;
    let full_sb_above_left = full_sb_with_above && has_left;
    let full_sb_above_left_with_above_right =
        full_sb_above_left && neighbours.num_above_right() > 0;
    match mode {
        SupportedDirectionalLumaMode::Vertical => full_sb_with_above
            .then_some(IntraLumaPlan::CardinalNeighbour {
                direction: IntraCardinalDirection::Vertical,
            })
            .ok_or(UNSUPPORTED_CARDINAL_VERTICAL),
        SupportedDirectionalLumaMode::Horizontal => full_sb_with_left
            .then_some(IntraLumaPlan::CardinalNeighbour {
                direction: IntraCardinalDirection::Horizontal,
            })
            .ok_or(UNSUPPORTED_CARDINAL_HORIZONTAL),
        SupportedDirectionalLumaMode::D157 => full_sb_left_only
            .then_some(IntraLumaPlan::DirectionalNeighbour { mode })
            .ok_or(UNSUPPORTED_D157_POSITION),
        SupportedDirectionalLumaMode::D113 => full_sb_above_left
            .then_some(IntraLumaPlan::DirectionalNeighbour { mode })
            .ok_or(UNSUPPORTED_D113_POSITION),
        SupportedDirectionalLumaMode::D45 => full_sb_above_left_with_above_right
            .then_some(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 45 })
            .ok_or(UNSUPPORTED_D45_POSITION),
        SupportedDirectionalLumaMode::D203 => full_sb_left_only
            .then_some(IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 203 })
            .ok_or(UNSUPPORTED_D203_POSITION),
        SupportedDirectionalLumaMode::D135 => {
            if is_top_left && is_full_sb {
                Ok(IntraLumaPlan::DirectionalFirst { mode })
            } else if full_sb_first_row || full_sb_above_left {
                Ok(IntraLumaPlan::DirectionalNeighbour { mode })
            } else if !is_top_left && has_above {
                Err(UNSUPPORTED_MULTIROW_DIRECTIONAL)
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

const UNSUPPORTED_SMOOTH_H_ABOVE_RIGHT: IntraLumaUnsupported = unsupported(
    "general_intra_smooth_h_above_right_unverified",
    missing_capability_message!(
        "intra.luma.smooth_h.above_right",
        neighbour = "cross_superblock",
        block = "subpartition",
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

const UNSUPPORTED_MULTIROW_DIRECTIONAL: IntraLumaUnsupported = unsupported(
    "general_intra_multirow_directional_luma",
    missing_capability_message!(
        "intra.luma.directional.multirow",
        neighbour = "first_col_or_subpartition",
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
#[allow(clippy::panic)]
mod tests {
    use super::super::block_context::{BlockRect, ChromaSampling, TxShape};
    use super::*;
    use splot_recon::BitDepth;

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
    fn rejects_unsupported_luma_prediction_classes() {
        let cases = [
            Case {
                label: "plain smooth interior",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 16,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: Some(SupportedNonDcLumaMode::Smooth),
                directional: None,
                expected: Expected::Error("general_intra_multiblock_non_dc_subblock"),
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
                expected: Expected::Error("general_intra_smooth_h_above_right_unverified"),
            },
            Case {
                label: "vertical first row",
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
            Case {
                label: "d135 first column multirow",
                bit_depth: BitDepth::Eight,
                row4: 16,
                col4: 0,
                width4: 16,
                height4: 16,
                frame_cols4: 32,
                dc: false,
                nondc: None,
                directional: Some(SupportedDirectionalLumaMode::D135),
                expected: Expected::Error("general_intra_multirow_directional_luma"),
            },
        ];

        for case in cases {
            assert_case(case);
        }
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
