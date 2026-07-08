// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

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
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
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
            label: "d45 top-left no-neighbour fallback",
            bit_depth: BitDepth::Eight,
            row4: 0,
            col4: 0,
            width4: 16,
            height4: 16,
            frame_cols4: 16,
            dc: false,
            nondc: None,
            directional: Some(SupportedDirectionalLumaMode::D45),
            expected: Expected::Plan(IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 45 }),
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
fn plans_angle_delta_directional_luma() {
    let cases = [
        (
            SupportedDirectionalLumaMode::D203,
            209,
            Case {
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
            },
        ),
        (
            SupportedDirectionalLumaMode::Horizontal,
            189,
            Case {
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
            },
        ),
        (
            SupportedDirectionalLumaMode::Vertical,
            84,
            Case {
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
            },
        ),
        (
            SupportedDirectionalLumaMode::D45,
            36,
            Case {
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
            },
        ),
        (
            SupportedDirectionalLumaMode::D113,
            119,
            Case {
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
            },
        ),
    ];

    for (mode, p_angle, case) in cases {
        let Expected::Plan(expected) = case.expected else {
            panic!("{}: expected directional angle plan", case.label);
        };
        assert_eq!(
            plan_directional_luma_angle(mode, p_angle, ctx(case), false),
            Ok(expected),
            "{}",
            case.label
        );
    }
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
        Case {
            label: "10-bit d45 top-left no-neighbour",
            bit_depth: BitDepth::Ten,
            row4: 0,
            col4: 0,
            width4: 16,
            height4: 16,
            frame_cols4: 16,
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
                use_dip: 0,
                dip_transpose: 0,
                dip_mode: 0,
                use_dpcm_y: 0,
                dpcm_mode_y: 0,
            },
        );

        assert!(plan_luma_prediction(&modes, ctx(case), false).is_err());
        assert_eq!(
            plan_luma_prediction(&modes, ctx(case), true).unwrap(),
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
            use_dip: 0,
            dip_transpose: 0,
            dip_mode: 0,
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
        plan_luma_prediction(&modes, ctx(case), false).unwrap(),
        IntraLumaPlan::CardinalNeighbour {
            direction: IntraCardinalDirection::Horizontal,
        }
    );
}

fn assert_case(case: Case) {
    let actual = plan_luma_prediction_from_parts(case.dc, case.nondc, case.directional, ctx(case));
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
        return plan_directional_luma_angle(mode, directional_mode_p_angle(mode), block_ctx, false);
    }
    Err(UNSUPPORTED_LUMA_MODE)
}
