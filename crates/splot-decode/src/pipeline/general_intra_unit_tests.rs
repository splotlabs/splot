// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;
use crate::bitstream::tile_payload::{CflIndex, CflParams, GeneralIntraChromaBlockMode};

#[test]
fn general_intra_recon_command_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<GeneralIntraReconCommand>();
}

fn ctx(row4: usize, col4: usize, width4: usize, height4: usize) -> BlockCtx {
    ctx_with_bit_depth(row4, col4, width4, height4, BitDepth::Ten)
}

fn ctx_with_bit_depth(
    row4: usize,
    col4: usize,
    width4: usize,
    height4: usize,
    bit_depth: BitDepth,
) -> BlockCtx {
    BlockCtx::new(
        BlockRect::new(row4, col4, width4, height4),
        TxShape::from_luma_4x4(width4, height4).expect("valid transform shape"),
        480,
        270,
        bit_depth,
        ChromaSampling::Yuv420,
    )
}

fn assert_rect_chroma_plan(
    mode: SupportedChromaMode,
    angle_delta: i8,
    expected: RectChromaPlan,
    label: &str,
) {
    assert_eq!(
        rect_chroma_plan_for_mode(mode, angle_delta, None),
        expected,
        "{label}"
    );
}

fn assert_rect_luma_plan(
    mode: Option<crate::bitstream::tile_payload::SupportedNonDcLumaMode>,
    directional_p_angle: Option<u16>,
    block: BlockCtx,
    expected: RectLumaPlan,
    label: &str,
) {
    assert_eq!(
        rect_luma_plan_for_parts(mode, directional_p_angle, false, block, false),
        Ok(expected),
        "{label}"
    );
}

fn rect_luma_plan_for_parts(
    nondc: Option<SupportedNonDcLumaMode>,
    directional_p_angle: Option<u16>,
    luma_is_dc: bool,
    block_ctx: BlockCtx,
    use_tcq: bool,
) -> core::result::Result<RectLumaPlan, IntraLumaUnsupported> {
    rect_luma_plan_for_parts_ext(
        false,
        nondc,
        directional_p_angle,
        luma_is_dc,
        block_ctx,
        use_tcq,
    )
}

fn assert_rect_luma_mrl_plan(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
    block: BlockCtx,
    expected: RectLumaPlan,
    label: &str,
) {
    assert_eq!(
        rect_luma_mrl_plan_for_parts(
            y_mode,
            angle_delta_y,
            mrl_index,
            mrl_sec_index,
            block,
            false,
            32,
        ),
        Ok(expected),
        "{label}"
    );
}

fn luma_modes(y_mode: IntraYMode) -> GeneralIntraBlockModes {
    luma_modes_with_angle(y_mode, 0)
}

fn luma_modes_with_angle(y_mode: IntraYMode, angle_delta_y: i8) -> GeneralIntraBlockModes {
    luma_modes_with_parts(y_mode, angle_delta_y, 0, 0)
}

fn luma_modes_with_parts(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    use_dpcm_y: u8,
    dpcm_mode_y: u8,
) -> GeneralIntraBlockModes {
    GeneralIntraBlockModes::luma_only(crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode,
        angle_delta_y,
        intra_joint_mode: 0,
        mrl_index: 0,
        mrl_sec_index: None,
        fsc_mode: 0,
        uses_mrls: 0,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y,
        dpcm_mode_y,
    })
}

#[test]
fn luma_tx_partition_context_uses_current_block_lossless_flag() {
    let block_size_index = 6;

    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Select), block_size_index, false),
        Some(LumaTransformPartitionContext::new(block_size_index))
    );
    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Select), block_size_index, true),
        None
    );
    assert_eq!(
        luma_tx_partition_context(Some(TxMode::Largest), block_size_index, false),
        None
    );
}

/// Luma 4x4 units spanning a full AV2 superblock axis.
const FULL_SB_N4_LUMA: usize = 16;

#[test]
fn lossless_luma_uses_generic_prediction_planner() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let block_ctx = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, bit_depth);
        for mode in [
            IntraYMode::V_PRED_FOR_TEST,
            IntraYMode::H_PRED_FOR_TEST,
            IntraYMode::D45_PRED_FOR_TEST,
            IntraYMode::D67_PRED_FOR_TEST,
            IntraYMode::D113_PRED_FOR_TEST,
            IntraYMode::D135_PRED_FOR_TEST,
            IntraYMode::D157_PRED_FOR_TEST,
            IntraYMode::D203_PRED_FOR_TEST,
            IntraYMode::SMOOTH_PRED_FOR_TEST,
        ] {
            let modes = luma_modes(mode);
            assert!(
                rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA).is_ok(),
                "lossless {bit_depth:?} {mode:?}"
            );
        }
    }
}

#[test]
fn lossless_adjusted_directional_luma_uses_rect_planner() {
    let block_ctx = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);

    for (mode, angle_delta_y, p_angle) in [
        (IntraYMode::V_PRED_FOR_TEST, 1, 93),
        (IntraYMode::H_PRED_FOR_TEST, -1, 177),
        (IntraYMode::D135_PRED_FOR_TEST, -1, 132),
        (IntraYMode::D135_PRED_FOR_TEST, 1, 138),
    ] {
        let modes = luma_modes_with_angle(mode, angle_delta_y);

        assert_eq!(
            rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA),
            Ok(RectLumaPlan::Middle {
                p_angle,
                use_tcq: false,
            })
        );
    }
}

#[test]
fn chroma_part_cfl_reaches_cfl_plan() {
    let params = CflParams {
        index: CflIndex::Explicit,
        alpha_u: 1,
        alpha_v: -1,
        mh_dir: None,
    };
    let chroma = GeneralIntraChromaBlockMode::cfl_for_test(params);

    assert_eq!(
        chroma_plan_for_parts(chroma, IntraYMode::H_PRED_FOR_TEST, 0, 1, 32).ok(),
        Some(RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 32,
        })
    );
}

#[test]
fn lossless_chroma_block_cfl_reaches_cfl_plan() {
    let params = CflParams {
        index: CflIndex::Multi,
        alpha_u: 0,
        alpha_v: 0,
        mh_dir: Some(2),
    };
    let luma = crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode: IntraYMode::H_PRED_FOR_TEST,
        angle_delta_y: 0,
        intra_joint_mode: 0,
        mrl_index: 0,
        mrl_sec_index: None,
        fsc_mode: 1,
        uses_mrls: 0,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };
    let chroma = GeneralIntraChromaBlockMode::cfl_for_test(params);
    let modes = GeneralIntraBlockModes::from_luma_chroma_palette(luma, chroma, None);

    let result = rect_chroma_plan(&modes, 1, 16);
    assert_eq!(
        result.ok(),
        Some(RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 16,
        })
    );
}

#[test]
fn rect_planner_serves_every_directional_luma_shape() {
    let shapes = [
        ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight),
        ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Ten),
        ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
        ctx_with_bit_depth(FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, 4, 4, BitDepth::Ten),
    ];
    let modes = [
        IntraYMode::V_PRED_FOR_TEST,
        IntraYMode::H_PRED_FOR_TEST,
        IntraYMode::D45_PRED_FOR_TEST,
        IntraYMode::D67_PRED_FOR_TEST,
        IntraYMode::D113_PRED_FOR_TEST,
        IntraYMode::D135_PRED_FOR_TEST,
        IntraYMode::D157_PRED_FOR_TEST,
        IntraYMode::D203_PRED_FOR_TEST,
    ];

    for block_ctx in shapes {
        for mode in modes {
            for angle_delta_y in [-3, 0, 2] {
                let modes = luma_modes_with_angle(mode, angle_delta_y);
                assert!(
                    rect_luma_plan(&modes, block_ctx, false, FULL_SB_N4_LUMA).is_ok(),
                    "rect planner must serve directional luma {mode:?} {angle_delta_y}"
                );
            }
        }
    }
}

#[test]
fn plans_every_chroma_mode() {
    for mode in [
        SupportedChromaMode::Dc,
        SupportedChromaMode::Smooth,
        SupportedChromaMode::D135Follow,
        SupportedChromaMode::D113Follow,
        SupportedChromaMode::D157Follow,
        SupportedChromaMode::VerticalFollow,
        SupportedChromaMode::Vertical,
        SupportedChromaMode::HorizontalFollow,
        SupportedChromaMode::Horizontal,
        SupportedChromaMode::D45Follow,
        SupportedChromaMode::D67Follow,
        SupportedChromaMode::D45,
        SupportedChromaMode::D67,
        SupportedChromaMode::D135,
        SupportedChromaMode::D113,
        SupportedChromaMode::D203Follow,
        SupportedChromaMode::D203,
        SupportedChromaMode::D157,
        SupportedChromaMode::Paeth,
        SupportedChromaMode::SmoothVertical,
        SupportedChromaMode::SmoothHorizontal,
    ] {
        let planned = match rect_chroma_plan_for_mode(mode, 0, None) {
            RectChromaPlan::Mode(planned, None)
            | RectChromaPlan::Directional {
                mode: planned,
                angle_delta_uv: 0,
                dpcm: None,
            } => Some(planned),
            _ => None,
        };
        assert_eq!(planned, Some(mode));
    }
}
#[test]
fn admits_rect_smooth_luma_cases() {
    use crate::bitstream::tile_payload::SupportedNonDcLumaMode::{
        Smooth, SmoothHorizontal, SmoothVertical,
    };

    for (label, mode, block) in [
        (
            "large vertical rect with left-only edge",
            SmoothVertical,
            ctx(0, 256, 32, FULL_SB_N4_LUMA),
        ),
        (
            "small rect with above-left edges",
            Smooth,
            ctx(24, 200, 2, 4),
        ),
        (
            "thin rect with above-left edges",
            Smooth,
            ctx(48, 150, 1, 4),
        ),
        (
            "small horizontal rect with left edge",
            SmoothHorizontal,
            ctx(17, 220, 4, 1),
        ),
    ] {
        assert_rect_luma_plan(
            Some(mode),
            None,
            block,
            RectLumaPlan::Smooth {
                mode,
                use_tcq: false,
            },
            label,
        );
    }
}

#[test]
fn admits_small_rect_paeth_luma_regardless_of_neighbour_edges() {
    let want = Ok(RectLumaPlan::Paeth { use_tcq: false });
    for block in [ctx(18, 220, 4, 2), ctx(0, 0, 4, 2)] {
        assert_eq!(
            rect_luma_plan_for_parts_ext(true, None, None, false, block, false),
            want
        );
    }
}

#[test]
fn active_dip_luma_routes_before_dc() {
    let modes = GeneralIntraBlockModes::luma_only(
        crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
            y_mode: IntraYMode::DC_PRED,
            angle_delta_y: 0,
            intra_joint_mode: 0,
            mrl_index: 0,
            mrl_sec_index: None,
            fsc_mode: 0,
            uses_mrls: 0,
            use_dip: 1,
            dip_transpose: 1,
            dip_mode: 2,
            use_dpcm_y: 0,
            dpcm_mode_y: 0,
        },
    );
    let block = ctx(0, 10, 2, 4);

    assert_eq!(
        rect_luma_plan(&modes, block, true, 16),
        Ok(RectLumaPlan::Dip {
            mode: 2,
            transpose: true,
            use_tcq: true,
        })
    );
}

#[test]
fn admits_rect_luma_mrl_cases() {
    for (label, y_mode, angle_delta_y, mrl_index, mrl_sec_index, block, expected) in [
        (
            "small rect d135 middle",
            IntraYMode::D135_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(20, 216, 1, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 135,
                mrl_index: 3,
                above_mrl_index: 3,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "small square vertical cardinal secondary",
            IntraYMode::V_PRED_FOR_TEST,
            0,
            3,
            Some(1),
            ctx(16, 264, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Vertical,
                mrl_index: 3,
                above_mrl_index: 3,
                secondary_mrl: true,
                use_tcq: false,
            },
        ),
        (
            "d45 one-sided above sb boundary",
            IntraYMode::D45_PRED_FOR_TEST,
            -2,
            1,
            Some(0),
            ctx(32, 216, 4, 4),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 40,
                mrl_index: 1,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "small square d157 middle",
            IntraYMode::D157_PRED_FOR_TEST,
            3,
            1,
            Some(0),
            ctx(26, 222, 2, 2),
            RectLumaPlan::MiddleMrl {
                p_angle: 167,
                mrl_index: 1,
                above_mrl_index: 1,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-row rect d113 middle",
            IntraYMode::D113_PRED_FOR_TEST,
            -1,
            2,
            Some(1),
            ctx(0, 316, 4, 8),
            RectLumaPlan::MiddleMrl {
                p_angle: 109,
                mrl_index: 2,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: true,
                use_tcq: false,
            },
        ),
        (
            "left-edge active-mrl vpred middle",
            IntraYMode::V_PRED_FOR_TEST,
            0,
            1,
            Some(0),
            ctx(4, 0, 1, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 91,
                mrl_index: 1,
                above_mrl_index: 1,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-row rect d67 one-sided above from left edge",
            IntraYMode::D67_PRED_FOR_TEST,
            -1,
            3,
            Some(0),
            ctx(0, 8, 8, 2),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 64,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "rect d67 one-sided left after wide-angle mapping",
            IntraYMode::D67_PRED_FOR_TEST,
            0,
            2,
            Some(0),
            ctx(22, 313, 2, 8),
            RectLumaPlan::OneSidedLeftMrl {
                p_angle: 246,
                mrl_index: 2,
                above_mrl_index: 2,
                is_sb_boundary: false,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left vertical cardinal without neighbours",
            IntraYMode::V_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Vertical,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left horizontal cardinal without neighbours",
            IntraYMode::H_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::CardinalMrl {
                direction: IntraCardinalDirection::Horizontal,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d45 without neighbours",
            IntraYMode::D45_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::OneSidedAboveMrl {
                p_angle: 45,
                mrl_index: 3,
                above_mrl_index: 0,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d135 without neighbours",
            IntraYMode::D135_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::MiddleMrl {
                p_angle: 135,
                mrl_index: 3,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
        (
            "top-left d203 without neighbours",
            IntraYMode::D203_PRED_FOR_TEST,
            0,
            3,
            Some(0),
            ctx(0, 0, 4, 4),
            RectLumaPlan::OneSidedLeftMrl {
                p_angle: 203,
                mrl_index: 3,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl: false,
                use_tcq: false,
            },
        ),
    ] {
        assert_rect_luma_mrl_plan(
            y_mode,
            angle_delta_y,
            mrl_index,
            mrl_sec_index,
            block,
            expected,
            label,
        );
    }
}

#[test]
fn admits_right_edge_rect_d45_as_one_sided_above_luma_without_above_right() {
    let right_edge_block = ctx(8, 479, 1, 2);

    let neighbours = right_edge_block.neighbours(PlaneId::Y);
    assert!(neighbours.has_above());
    assert_eq!(neighbours.num_above_right(), 0);

    let plan = rect_luma_plan_for_parts(None, Some(45), false, right_edge_block, false);
    assert!(matches!(
        plan,
        Ok(RectLumaPlan::OneSidedAbove {
            p_angle: 45,
            use_tcq: false,
        })
    ));
}

#[test]
fn admits_rect_cardinal_luma_cases() {
    for (label, p_angle, block, direction) in [
        (
            "large vertical with above edge",
            90,
            ctx(FULL_SB_N4_LUMA, 256, 32, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "vertical with left-only edge",
            90,
            ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "horizontal with above-only edge",
            180,
            ctx(80, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Horizontal,
        ),
        (
            "small vertical with above edge",
            90,
            ctx(24, 204, 1, 2),
            IntraCardinalDirection::Vertical,
        ),
        (
            "top-left vertical without neighbours",
            90,
            ctx(0, 0, 4, FULL_SB_N4_LUMA),
            IntraCardinalDirection::Vertical,
        ),
        (
            "top-left horizontal without neighbours",
            180,
            ctx(0, 0, FULL_SB_N4_LUMA, 4),
            IntraCardinalDirection::Horizontal,
        ),
    ] {
        assert_rect_luma_plan(
            None,
            Some(p_angle),
            block,
            RectLumaPlan::Cardinal {
                direction,
                use_tcq: false,
            },
            label,
        );
    }
}

#[test]
fn admits_rect_angle_luma_cases() {
    for (label, p_angle, block, expected) in [
        (
            "small rect d67 one-sided above",
            76,
            ctx(28, 216, 2, 4),
            RectLumaPlan::OneSidedAbove {
                p_angle: 76,
                use_tcq: false,
            },
        ),
        (
            "small first-row hpred one-sided left",
            186,
            ctx(0, 264, 8, 4),
            RectLumaPlan::OneSidedLeft {
                p_angle: 186,
                use_tcq: false,
            },
        ),
        (
            "horizontal angle delta with above-only edge",
            183,
            ctx(80, 0, FULL_SB_N4_LUMA, 4),
            RectLumaPlan::OneSidedLeft {
                p_angle: 183,
                use_tcq: false,
            },
        ),
        (
            "first-column d203 uses its above edge",
            203,
            ctx(46, 0, 4, 2),
            RectLumaPlan::OneSidedLeft {
                p_angle: 203,
                use_tcq: false,
            },
        ),
        (
            "small rect middle",
            151,
            ctx(26, 204, 1, 2),
            RectLumaPlan::Middle {
                p_angle: 151,
                use_tcq: false,
            },
        ),
        (
            "rect d67 one-sided above",
            61,
            ctx(8, 336, FULL_SB_N4_LUMA, 8),
            RectLumaPlan::OneSidedAbove {
                p_angle: 61,
                use_tcq: false,
            },
        ),
        (
            "rect d135 middle",
            126,
            ctx(FULL_SB_N4_LUMA, 320, 8, FULL_SB_N4_LUMA),
            RectLumaPlan::Middle {
                p_angle: 126,
                use_tcq: false,
            },
        ),
        (
            "first-column vpred angle-delta middle uses above edge",
            93,
            ctx(4, 0, 8, 4),
            RectLumaPlan::Middle {
                p_angle: 93,
                use_tcq: false,
            },
        ),
        (
            "top-row d157 middle uses left edge",
            157,
            ctx(0, 4, 2, 1),
            RectLumaPlan::Middle {
                p_angle: 157,
                use_tcq: false,
            },
        ),
    ] {
        assert_rect_luma_plan(None, Some(p_angle), block, expected, label);
    }
}

#[test]
fn admits_rect_directional_luma_without_real_edges() {
    let block = ctx(0, 0, 2, 1);
    for (p_angle, expected) in [
        (
            45,
            RectLumaPlan::OneSidedAbove {
                p_angle: 45,
                use_tcq: false,
            },
        ),
        (
            157,
            RectLumaPlan::Middle {
                p_angle: 157,
                use_tcq: false,
            },
        ),
        (
            203,
            RectLumaPlan::OneSidedLeft {
                p_angle: 203,
                use_tcq: false,
            },
        ),
    ] {
        assert_eq!(
            rect_luma_plan_for_parts(None, Some(p_angle), false, block, false),
            Ok(expected),
        );
    }
}

#[test]
fn square_d67_angle_delta_uses_rect_residual_path_when_square_plan_rejects() {
    let first_col_block = ctx(128, 0, 32, 32);
    let modes = GeneralIntraBlockModes::luma_only(
        crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
            y_mode: IntraYMode::D67_PRED_FOR_TEST,
            angle_delta_y: -1,
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

    assert_eq!(
        rect_luma_plan(&modes, first_col_block, false, 32),
        Ok(RectLumaPlan::OneSidedAbove {
            p_angle: 64,
            use_tcq: false,
        })
    );
}

#[test]
fn retains_rect_directional_chroma_context() {
    for (label, mode, angle_delta, expected) in [
        (
            "above-left d135 follow",
            SupportedChromaMode::D135Follow,
            -3,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D135Follow,
                angle_delta_uv: -3,
                dpcm: None,
            },
        ),
        (
            "top-row d113 follow with left-only edge",
            SupportedChromaMode::D113Follow,
            -1,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D113Follow,
                angle_delta_uv: -1,
                dpcm: None,
            },
        ),
        (
            "top-row d157 follow with left-only edge",
            SupportedChromaMode::D157Follow,
            -1,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D157Follow,
                angle_delta_uv: -1,
                dpcm: None,
            },
        ),
        (
            "top-row d135 with left-only edge",
            SupportedChromaMode::D135,
            0,
            RectChromaPlan::Directional {
                mode: SupportedChromaMode::D135,
                angle_delta_uv: 0,
                dpcm: None,
            },
        ),
    ] {
        assert_rect_chroma_plan(mode, angle_delta, expected, label);
    }
}

#[test]
fn follows_luma_angle_delta_for_directional_chroma() {
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::VerticalFollow, -1, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::VerticalFollow,
            angle_delta_uv: -1,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::VerticalFollow, 1, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::VerticalFollow,
            angle_delta_uv: 1,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::Vertical, 0, None),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Vertical,
            angle_delta_uv: 0,
            dpcm: None,
        }
    );
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::Vertical,
            0,
            Some(DpcmDirection::Vertical),
        ),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Vertical,
            angle_delta_uv: 0,
            dpcm: Some(DpcmDirection::Vertical),
        }
    );

    let horizontal_dpcm = Some(DpcmDirection::Horizontal);
    let angle_delta = inherited_chroma_angle_delta(
        IntraYMode::H_PRED_FOR_TEST.value(),
        IntraYMode::H_PRED_FOR_TEST,
        2,
    );
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::Horizontal,
            angle_delta,
            horizontal_dpcm,
        ),
        RectChromaPlan::Directional {
            mode: SupportedChromaMode::Horizontal,
            angle_delta_uv: 2,
            dpcm: horizontal_dpcm,
        }
    );
}
