// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::bitstream::tile_payload::IntraYMode;
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
        &mut crate::pipeline::general_intra::GeneralIntraReconScratch::default(),
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
            expected: Expected::Plan(IntraLumaPlan::NonDcNeighbour {
                mode: SupportedNonDcLumaMode::SmoothVertical,
            }),
        },
    ];

    for case in cases {
        assert_case(case);
    }
}

#[test]
fn rejects_unsupported_luma_prediction_classes() {
    let block_ctx = ctx(Case {
        label: "top-left full superblock",
        bit_depth: BitDepth::Eight,
        row4: 0,
        col4: 0,
        width4: 16,
        height4: 16,
        frame_cols4: 16,
        dc: false,
        nondc: None,
        expected: Expected::Error("general_intra_unsupported_luma_mode"),
    });

    for y_mode in [
        IntraYMode::V_PRED_FOR_TEST,
        IntraYMode::H_PRED_FOR_TEST,
        IntraYMode::D45_PRED_FOR_TEST,
        IntraYMode::D67_PRED_FOR_TEST,
        IntraYMode::D113_PRED_FOR_TEST,
        IntraYMode::D135_PRED_FOR_TEST,
        IntraYMode::D157_PRED_FOR_TEST,
        IntraYMode::D203_PRED_FOR_TEST,
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
        let Err(error) = plan_luma_prediction(&modes, block_ctx) else {
            panic!("directional luma is planned by the rect planner, not the square planner");
        };

        assert_eq!(error.reason_id(), "general_intra_unsupported_luma_mode");
        assert!(error.message().starts_with("unsupported capability: "));
    }
}

fn assert_case(case: Case) {
    let actual = plan_luma_prediction_from_parts(case.dc, case.nondc, ctx(case));
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
    block_ctx: BlockCtx,
) -> core::result::Result<IntraLumaPlan, IntraLumaUnsupported> {
    if luma_is_dc {
        return Ok(IntraLumaPlan::Dc);
    }
    if let Some(mode) = nondc {
        return plan_nondc_luma(mode, block_ctx);
    }
    Err(UNSUPPORTED_LUMA_MODE)
}
