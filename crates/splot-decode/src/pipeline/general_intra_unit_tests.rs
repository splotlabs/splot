// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;
use crate::bitstream::tile_payload::{CflIndex, CflParams, GeneralIntraChromaBlockMode};
use crate::prediction::intra::IntraLumaPlan;

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

fn assert_chroma_admitted(mode: SupportedChromaMode, block: BlockCtx) {
    assert!(ensure_supported_chroma_capability(mode, block).is_ok());
}

fn assert_rect_chroma_admitted(mode: SupportedChromaMode, block: BlockCtx) {
    let plan = rect_chroma_plan_for_mode(mode, 0, None, block);
    let admitted = match plan {
        RectChromaPlan::Mode(planned, None) => planned == mode,
        RectChromaPlan::OneSided(1..=89 | 181..=270, None)
        | RectChromaPlan::Middle(91..=179, None) => true,
        _ => false,
    };
    assert!(admitted, "invalid rect chroma plan for {mode:?}: {plan:?}");
}

fn assert_rect_chroma_plan(
    mode: SupportedChromaMode,
    angle_delta: i8,
    block: BlockCtx,
    expected: RectChromaPlan,
    label: &str,
) {
    assert_eq!(
        rect_chroma_plan_for_mode(mode, angle_delta, None, block),
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

fn luma_modes_with_dpcm(
    y_mode: IntraYMode,
    use_dpcm_y: u8,
    dpcm_mode_y: u8,
) -> GeneralIntraBlockModes {
    luma_modes_with_parts(y_mode, 0, use_dpcm_y, dpcm_mode_y)
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

fn luma_mrl_modes(
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
) -> GeneralIntraBlockModes {
    GeneralIntraBlockModes::luma_only(crate::bitstream::tile_payload::GeneralIntraLumaBlockMode {
        y_mode,
        angle_delta_y,
        intra_joint_mode: 0,
        mrl_index,
        mrl_sec_index,
        fsc_mode: 0,
        uses_mrls: 1,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    })
}

fn with_active_fsc(mut modes: GeneralIntraBlockModes) -> GeneralIntraBlockModes {
    modes.fsc_mode = 1;
    modes
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

#[test]
fn lossless_prediction_guard_admits_dc_luma_and_dpcm() {
    let modes = luma_modes(IntraYMode::DC_PRED);
    let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

    assert!(
        ensure_lossless_verified_prediction_subset(
            true,
            false,
            &modes,
            top_left,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(0),
        )
        .is_ok()
    );

    let modes = luma_modes_with_dpcm(IntraYMode::V_PRED_FOR_TEST, 1, 0);
    assert!(
        ensure_lossless_verified_prediction_subset(
            true,
            false,
            &modes,
            top_left,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(9),
        )
        .is_ok()
    );
}

fn assert_lossless_directional_luma_admitted(mode: IntraYMode) {
    let top_left = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
    let modes = luma_modes(mode);

    assert!(
        ensure_lossless_verified_prediction_subset(
            true,
            false,
            &modes,
            top_left,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(9),
        )
        .is_ok()
    );
}

#[test]
fn lossless_prediction_guard_admits_top_left_cardinal_luma() {
    for mode in [IntraYMode::V_PRED_FOR_TEST, IntraYMode::H_PRED_FOR_TEST] {
        assert_lossless_directional_luma_admitted(mode);
    }
}

#[test]
fn lossless_prediction_guard_admits_top_left_d135_luma() {
    assert_lossless_directional_luma_admitted(IntraYMode::D135_PRED_FOR_TEST);
}

#[test]
fn lossless_prediction_guard_admits_top_left_d157_luma() {
    assert_lossless_directional_luma_admitted(IntraYMode::D157_PRED_FOR_TEST);
}

#[test]
fn lossless_prediction_guard_admits_top_left_d67_luma() {
    assert_lossless_directional_luma_admitted(IntraYMode::D67_PRED_FOR_TEST);
}

#[test]
fn lossless_prediction_guard_admits_top_left_d113_luma() {
    assert_lossless_directional_luma_admitted(IntraYMode::D113_PRED_FOR_TEST);
}

#[test]
fn lossless_prediction_guard_admits_top_left_d203_luma() {
    assert_lossless_directional_luma_admitted(IntraYMode::D203_PRED_FOR_TEST);
}

#[test]
fn lossless_prediction_guard_rejects_unverified_active_fsc_luma() {
    let modes = with_active_fsc(luma_modes(IntraYMode::D45_PRED_FOR_TEST));
    let error = ensure_lossless_verified_prediction_subset(
        true,
        false,
        &modes,
        ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        FULL_SB_N4_LUMA,
        splot_core::span::ByteOffset::new(11),
    )
    .expect_err("unverified active-FSC lossless luma prediction must fail");

    let reason = match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
        _ => "",
    };
    assert_eq!(reason, "general_intra_lossless_other_nondc_luma_unverified");
}

#[test]
fn lossless_prediction_guard_admits_top_left_active_fsc_dc_luma() {
    let modes = with_active_fsc(luma_modes(IntraYMode::DC_PRED));

    ensure_lossless_verified_prediction_subset(
        true,
        false,
        &modes,
        ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight),
        FULL_SB_N4_LUMA,
        splot_core::span::ByteOffset::new(12),
    )
    .expect("verified DC prediction should not be blocked by active FSC");
}

#[test]
fn lossless_prediction_guard_admits_edge_backed_active_fsc_rect_luma() {
    let modes = with_active_fsc(luma_modes_with_angle(IntraYMode::D67_PRED_FOR_TEST, -2));
    let above_only = ctx_with_bit_depth(3, 0, 1, 1, BitDepth::Eight);

    ensure_lossless_verified_prediction_subset(
        true,
        false,
        &modes,
        above_only,
        FULL_SB_N4_LUMA,
        splot_core::span::ByteOffset::new(12),
    )
    .expect("edge-backed active-FSC prediction shape should reach residual decode");
}

#[test]
fn lossless_prediction_guard_admits_top_row_active_mrl_rect_luma() {
    let modes = luma_mrl_modes(IntraYMode::D67_PRED_FOR_TEST, -1, 3, Some(0));
    let top_row_left_edge = ctx_with_bit_depth(0, 8, 8, 2, BitDepth::Eight);

    assert_eq!(
        rect_luma_plan(&modes, top_row_left_edge, false, 32),
        Ok(RectLumaPlan::OneSidedAboveMrl {
            p_angle: 64,
            mrl_index: 3,
            above_mrl_index: 0,
            secondary_mrl: false,
            use_tcq: false,
        })
    );
    ensure_lossless_verified_prediction_subset(
        true,
        false,
        &modes,
        top_row_left_edge,
        32,
        splot_core::span::ByteOffset::new(34),
    )
    .expect("top-row edge-backed active MRL luma should reach residual decode");
}

#[test]
fn lossless_prediction_guard_admits_edge_backed_rect_luma() {
    let smooth_modes = luma_modes(IntraYMode::SMOOTH_PRED_FOR_TEST);
    let top_row_left_edge = ctx_with_bit_depth(0, 6, 2, 2, BitDepth::Eight);

    assert_eq!(
        rect_luma_plan(&smooth_modes, top_row_left_edge, false, FULL_SB_N4_LUMA),
        Ok(RectLumaPlan::Smooth {
            mode: SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        })
    );
    assert!(
        ensure_lossless_verified_prediction_subset(
            true,
            false,
            &smooth_modes,
            top_row_left_edge,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(9),
        )
        .is_ok()
    );

    let directional_modes = luma_modes_with_angle(IntraYMode::D67_PRED_FOR_TEST, -2);
    let above_only = ctx_with_bit_depth(3, 0, 1, 1, BitDepth::Eight);

    assert_eq!(
        rect_luma_plan(&directional_modes, above_only, false, FULL_SB_N4_LUMA),
        Ok(RectLumaPlan::OneSidedAbove {
            p_angle: 61,
            use_tcq: false,
        })
    );
    assert!(
        ensure_lossless_verified_prediction_subset(
            true,
            false,
            &directional_modes,
            above_only,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(9),
        )
        .is_ok()
    );

    for (modes, block_ctx) in [
        (
            &smooth_modes,
            ctx_with_bit_depth(0, 0, 2, 2, BitDepth::Eight),
        ),
        (&smooth_modes, ctx(0, 6, 2, 2)),
        (
            &directional_modes,
            ctx_with_bit_depth(0, 0, 1, 1, BitDepth::Eight),
        ),
        (&directional_modes, ctx(3, 0, 1, 1)),
    ] {
        let error = ensure_lossless_verified_prediction_subset(
            true,
            false,
            modes,
            block_ctx,
            FULL_SB_N4_LUMA,
            splot_core::span::ByteOffset::new(9),
        )
        .expect_err("edge-backed 8-bit subset must not admit no-edge or 10-bit blocks");

        let reason = match error {
            DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
            _ => "",
        };
        assert_eq!(reason, "general_intra_lossless_other_nondc_luma_unverified");
    }
}

#[test]
fn lossless_chroma_part_guard_admits_rect_prediction_subset() {
    let top_left_smooth_part = ctx_with_bit_depth(0, 0, 4, 16, BitDepth::Eight);
    assert!(lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::Smooth),
        false,
        IntraYMode::DC_PRED,
        top_left_smooth_part,
        32,
    ));

    let top_left_full = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
    assert!(lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::HorizontalFollow),
        false,
        IntraYMode::H_PRED_FOR_TEST,
        top_left_full,
        FULL_SB_N4_LUMA,
    ));

    let edge_backed_horizontal_part = ctx_with_bit_depth(8, 4, 8, 8, BitDepth::Eight);
    assert!(lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::Horizontal),
        false,
        IntraYMode::DC_PRED,
        edge_backed_horizontal_part,
        32,
    ));

    let first_row_left_edge = ctx_with_bit_depth(
        0,
        FULL_SB_N4_LUMA,
        FULL_SB_N4_LUMA,
        FULL_SB_N4_LUMA,
        BitDepth::Eight,
    );
    assert!(lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::D113Follow),
        false,
        IntraYMode::D113_PRED_FOR_TEST,
        first_row_left_edge,
        FULL_SB_N4_LUMA,
    ));
    assert!(lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::D203Follow),
        false,
        IntraYMode::D203_PRED_FOR_TEST,
        first_row_left_edge,
        FULL_SB_N4_LUMA,
    ));

    assert!(!lossless_chroma_part_prediction_verified(
        Some(SupportedChromaMode::Horizontal),
        false,
        IntraYMode::DC_PRED,
        top_left_smooth_part,
        32,
    ));
}

#[test]
fn lossless_chroma_block_guard_admits_interior_rect_prediction_subset() {
    let block = ctx_with_bit_depth(6, 10, 4, 2, BitDepth::Eight);
    assert!(lossless_chroma_block_prediction_verified(
        Some(SupportedChromaMode::Smooth),
        false,
        IntraYMode::DC_PRED,
        block,
        32,
    ));
    assert!(!lossless_chroma_block_prediction_verified(
        Some(SupportedChromaMode::Smooth),
        true,
        IntraYMode::DC_PRED,
        block,
        32,
    ));
}

#[test]
fn lossless_chroma_block_guard_admits_top_left_d113_follow_subset() {
    let top_left = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);

    assert!(lossless_chroma_block_prediction_verified(
        Some(SupportedChromaMode::D113Follow),
        false,
        IntraYMode::D113_PRED_FOR_TEST,
        top_left,
        FULL_SB_N4_LUMA,
    ));
    assert!(!lossless_chroma_block_prediction_verified(
        Some(SupportedChromaMode::D113Follow),
        false,
        IntraYMode::D135_PRED_FOR_TEST,
        top_left,
        FULL_SB_N4_LUMA,
    ));
    assert!(!lossless_chroma_block_prediction_verified(
        Some(SupportedChromaMode::D113Follow),
        false,
        IntraYMode::D113_PRED_FOR_TEST,
        ctx_with_bit_depth(0, 0, 8, 8, BitDepth::Eight),
        FULL_SB_N4_LUMA,
    ));
}

#[test]
fn lossless_chroma_part_guard_delegates_cfl_to_cfl_plan() {
    let block = ctx_with_bit_depth(2, 0, 2, 2, BitDepth::Eight);
    let params = CflParams {
        index: CflIndex::Explicit,
        alpha_u: 1,
        alpha_v: -1,
        mh_dir: None,
    };
    let chroma = GeneralIntraChromaBlockMode::cfl_for_test(params);

    assert!(lossless_chroma_part_prediction_guard_passes(
        chroma,
        IntraYMode::H_PRED_FOR_TEST,
        block,
        32,
    ));
    let result = chroma_plan_for_parts(chroma, IntraYMode::H_PRED_FOR_TEST, 0, block, 1, 32);
    assert!(result.is_ok());
    let plan = result
        .ok()
        .expect("CFL chroma part should reach the CFL planner");

    assert_eq!(
        plan,
        RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index: 1,
            sb_mib: 32,
        }
    );
}

#[test]
fn lossless_chroma_block_guard_delegates_cfl_to_cfl_plan() {
    let block = ctx_with_bit_depth(25, 23, 1, 1, BitDepth::Eight);
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

    assert!(lossless_chroma_block_prediction_guard_passes(
        &modes, block, 16,
    ));
    let result = chroma_plan_for_modes(&modes, block, 1, 16, true);
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
fn cardinal_top_left_guard_admits_only_verified_shapes() {
    let top_left_8 = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
    let vertical = luma_modes_with_angle(IntraYMode::V_PRED_FOR_TEST, 2);
    let horizontal = luma_modes(IntraYMode::H_PRED_FOR_TEST);

    assert_eq!(
        plan_luma_prediction_for_segment(&vertical, top_left_8, false, FULL_SB_N4_LUMA)
            .expect("verified top-left V_PRED angle delta should plan"),
        crate::prediction::intra::IntraLumaPlan::DirectionalMiddle { p_angle: 96 }
    );
    assert_eq!(
        plan_luma_prediction_for_segment(&horizontal, top_left_8, false, FULL_SB_N4_LUMA)
            .expect("verified top-left H_PRED cardinal should plan"),
        crate::prediction::intra::IntraLumaPlan::CardinalNeighbour {
            direction: IntraCardinalDirection::Horizontal,
        }
    );

    for (modes, block_ctx, sb_mib) in [
        (
            luma_modes(IntraYMode::V_PRED_FOR_TEST),
            top_left_8,
            FULL_SB_N4_LUMA,
        ),
        (
            luma_modes_with_angle(IntraYMode::V_PRED_FOR_TEST, 1),
            top_left_8,
            FULL_SB_N4_LUMA,
        ),
        (
            vertical,
            ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
            FULL_SB_N4_LUMA,
        ),
        (vertical, top_left_8, 32),
        (
            horizontal,
            ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
            FULL_SB_N4_LUMA,
        ),
        (
            vertical,
            ctx_with_bit_depth(0, 0, 4, 4, BitDepth::Eight),
            FULL_SB_N4_LUMA,
        ),
    ] {
        assert!(
            plan_luma_prediction_for_segment(&modes, block_ctx, false, sb_mib).is_err(),
            "unverified non-lossless cardinal luma shape must fail closed"
        );
    }
}

#[test]
fn lossless_prediction_guard_rejects_unverified_directional_variants() {
    let top_left_8 = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
    let top_left_10 = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

    for mode in [
        IntraYMode::V_PRED_FOR_TEST,
        IntraYMode::H_PRED_FOR_TEST,
        IntraYMode::D45_PRED_FOR_TEST,
        IntraYMode::D135_PRED_FOR_TEST,
        IntraYMode::D157_PRED_FOR_TEST,
    ] {
        for (modes, block_ctx, sb_mib) in [
            (luma_modes_with_angle(mode, 1), top_left_8, FULL_SB_N4_LUMA),
            (luma_modes(mode), top_left_10, FULL_SB_N4_LUMA),
            (luma_modes(mode), top_left_8, 32),
        ] {
            let error = ensure_lossless_verified_prediction_subset(
                true,
                false,
                &modes,
                block_ctx,
                sb_mib,
                splot_core::span::ByteOffset::new(9),
            )
            .expect_err("unverified lossless directional luma variant must fail closed");

            let reason = match error {
                DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
                _ => "",
            };
            assert_eq!(reason, "general_intra_lossless_other_nondc_luma_unverified");
        }
    }
}

#[test]
fn admits_10bit_chroma_edge_cases() {
    for (mode, block) in [
        (
            SupportedChromaMode::Vertical,
            ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        ),
        (
            SupportedChromaMode::HorizontalFollow,
            ctx(FULL_SB_N4_LUMA, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        ),
        (
            SupportedChromaMode::Paeth,
            ctx(0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA),
        ),
    ] {
        assert_chroma_admitted(mode, block);
    }
}

#[test]
fn admits_top_left_horizontal_chroma_subblock() {
    let top_left_subblock = ctx(0, 0, 8, 8);

    assert!(
        ensure_supported_chroma_capability(SupportedChromaMode::Horizontal, top_left_subblock,)
            .is_ok()
    );
}

#[test]
fn admits_horizontal_chroma_without_neighbours() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let frame_origin = ctx_with_bit_depth(0, 0, 2, 2, bit_depth);
        let tile_origin = ctx_with_bit_depth(0, 16, 2, 2, bit_depth).with_tile_bounds(
            0,
            FULL_SB_N4_LUMA,
            FULL_SB_N4_LUMA,
            2 * FULL_SB_N4_LUMA,
        );

        for block in [frame_origin, tile_origin] {
            let neighbours = block.neighbours(PlaneId::U);
            assert!(!neighbours.has_above() && !neighbours.has_left());
            assert_chroma_admitted(SupportedChromaMode::Horizontal, block);
        }
    }
}

#[test]
fn admits_top_left_cardinal_follow_chroma_without_neighbours() {
    let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

    for mode in [
        SupportedChromaMode::Vertical,
        SupportedChromaMode::VerticalFollow,
    ] {
        assert_chroma_admitted(mode, top_left);
    }

    assert!(
        ensure_supported_chroma_capability(SupportedChromaMode::HorizontalFollow, top_left).is_ok()
    );
}

#[test]
fn admits_top_left_rect_horizontal_chroma_subblock() {
    let top_left_rect = ctx(0, 0, 8, 4);

    assert_rect_chroma_admitted(SupportedChromaMode::Horizontal, top_left_rect);
}

#[test]
fn plans_every_rect_chroma_mode_without_real_edges() {
    let frame_start = ctx(0, 0, 8, 4);
    let tile_start = ctx(16, 32, 8, 4).with_tile_bounds(16, 24, 32, 40);
    let modes = [
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
    ];

    for block in [frame_start, tile_start] {
        let neighbours = block.neighbours(PlaneId::U);
        assert!(!neighbours.has_above() && !neighbours.has_left());
        for mode in modes {
            assert_rect_chroma_admitted(mode, block);
        }
    }
}

#[test]
fn admits_top_left_smooth_chroma_subblock() {
    let top_left = ctx(0, 0, 2, 2);

    assert!(
        ensure_supported_chroma_capability(SupportedChromaMode::SmoothHorizontal, top_left).is_ok()
    );
    assert_rect_chroma_admitted(SupportedChromaMode::SmoothHorizontal, top_left);
}

#[test]
fn admits_10bit_rect_chroma_edge_cases() {
    for (mode, block) in [
        (
            SupportedChromaMode::Horizontal,
            ctx(0, 224, 32, FULL_SB_N4_LUMA),
        ),
        (SupportedChromaMode::Smooth, ctx(0, 288, FULL_SB_N4_LUMA, 8)),
        (
            SupportedChromaMode::Paeth,
            ctx(FULL_SB_N4_LUMA, 192, FULL_SB_N4_LUMA, 8),
        ),
    ] {
        assert_rect_chroma_admitted(mode, block);
    }
}

#[test]
fn admits_10bit_small_rect_smooth_chroma_with_above_left_edges() {
    let rect_block = ctx(24, 200, 2, 4);

    assert_rect_chroma_admitted(SupportedChromaMode::Smooth, rect_block);
}

#[test]
fn admits_square_smooth_chroma_subblock_with_above_left_edges() {
    let square_block = ctx(24, 200, 8, 8);

    assert_chroma_admitted(SupportedChromaMode::Smooth, square_block);
}

#[test]
fn admits_small_vertical_follow_chroma_with_above_edge() {
    let small_block = ctx(20, 218, 2, 2);

    assert_chroma_admitted(SupportedChromaMode::VerticalFollow, small_block);
}

#[test]
fn admits_rect_vertical_follow_chroma_with_left_only_edge() {
    let first_row_rect_block = ctx(0, 416, 8, FULL_SB_N4_LUMA);

    assert_chroma_admitted(SupportedChromaMode::VerticalFollow, first_row_rect_block);
    assert_rect_chroma_admitted(SupportedChromaMode::VerticalFollow, first_row_rect_block);
}

#[test]
fn admits_small_d135_follow_chroma_with_above_left_edges() {
    let small_block = ctx(24, 200, 8, 8);

    assert_chroma_admitted(SupportedChromaMode::D135Follow, small_block);
}

#[test]
fn admits_square_middle_chroma_with_one_sided_edge() {
    let above_only = ctx(FULL_SB_N4_LUMA, 0, 8, 8);
    let left_only = ctx(0, FULL_SB_N4_LUMA, 8, 8);
    let modes = [
        SupportedChromaMode::D113Follow,
        SupportedChromaMode::D113,
        SupportedChromaMode::D135Follow,
        SupportedChromaMode::D135,
        SupportedChromaMode::D157Follow,
        SupportedChromaMode::D157,
    ];

    for mode in modes {
        assert_chroma_admitted(mode, above_only);
        assert_chroma_admitted(mode, left_only);
    }
}

#[test]
fn admits_small_d157_follow_chroma_with_above_left_edges() {
    let small_block = ctx(16, 416, 8, 8);

    assert_chroma_admitted(SupportedChromaMode::D157Follow, small_block);
}

#[test]
fn admits_small_d67_follow_chroma_with_above_right_edge() {
    let small_block = ctx(28, 216, 2, 4);

    assert_chroma_admitted(SupportedChromaMode::D67Follow, small_block);
    assert_rect_chroma_admitted(SupportedChromaMode::D67Follow, small_block);
}

#[test]
fn admits_d67_follow_chroma_with_above_only_edge() {
    let first_col_block = ctx(128, 0, 32, 32);
    let neighbours = first_col_block.neighbours(PlaneId::U);

    assert!(neighbours.has_above());
    assert_eq!(neighbours.num_above_right(), 0);

    assert_chroma_admitted(SupportedChromaMode::D67Follow, first_col_block);
    assert_rect_chroma_admitted(SupportedChromaMode::D67Follow, first_col_block);

    let right_edge_block = ctx(128, 448, 32, 32);
    let right_edge_neighbours = right_edge_block.neighbours(PlaneId::U);
    assert!(right_edge_neighbours.has_above());
    assert_eq!(right_edge_neighbours.num_above_right(), 0);
    assert_chroma_admitted(SupportedChromaMode::D67Follow, right_edge_block);
    assert_rect_chroma_admitted(SupportedChromaMode::D67Follow, right_edge_block);
}

#[test]
fn admits_top_left_zone1_chroma_without_neighbours() {
    let blocks = [ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA), ctx(0, 0, 8, 8)];

    for block in blocks {
        for mode in [
            SupportedChromaMode::D45,
            SupportedChromaMode::D45Follow,
            SupportedChromaMode::D67,
            SupportedChromaMode::D67Follow,
        ] {
            assert!(ensure_supported_chroma_capability(mode, block).is_ok());
        }
    }
}

#[test]
fn admits_top_left_full_sb_d113_chroma_without_neighbours() {
    let top_left = ctx(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA);

    for mode in [SupportedChromaMode::D113, SupportedChromaMode::D113Follow] {
        assert_chroma_admitted(mode, top_left);
    }
}

#[test]
fn admits_non_full_d135_chroma_without_neighbours() {
    let blocks = [
        ctx_with_bit_depth(0, 0, 8, 8, BitDepth::Eight),
        ctx_with_bit_depth(0, 0, 8, 8, BitDepth::Ten),
        ctx_with_bit_depth(16, 32, 8, 8, BitDepth::Eight).with_tile_bounds(16, 24, 32, 40),
        ctx_with_bit_depth(16, 32, 8, 8, BitDepth::Ten).with_tile_bounds(16, 24, 32, 40),
    ];

    for block in blocks {
        let neighbours = block.neighbours(PlaneId::U);
        assert!(!neighbours.has_above() && !neighbours.has_left());
        assert!(block.block().width4() < FULL_SB_N4_LUMA);
        for mode in [SupportedChromaMode::D135, SupportedChromaMode::D135Follow] {
            assert_chroma_admitted(mode, block);
        }
    }
}

#[test]
fn admits_non_full_d157_chroma_without_neighbours() {
    let blocks = [
        ctx_with_bit_depth(0, 0, 8, 8, BitDepth::Eight),
        ctx_with_bit_depth(0, 0, 8, 8, BitDepth::Ten),
        ctx_with_bit_depth(16, 32, 8, 8, BitDepth::Eight).with_tile_bounds(16, 24, 32, 40),
        ctx_with_bit_depth(16, 32, 8, 8, BitDepth::Ten).with_tile_bounds(16, 24, 32, 40),
    ];

    for block in blocks {
        let neighbours = block.neighbours(PlaneId::U);
        assert!(!neighbours.has_above() && !neighbours.has_left());
        assert!(block.block().width4() < FULL_SB_N4_LUMA);
        for mode in [SupportedChromaMode::D157, SupportedChromaMode::D157Follow] {
            assert_chroma_admitted(mode, block);
        }
    }
}

#[test]
fn admits_rect_d113_chroma_at_tile_start_with_above_edge() {
    let tile_start_block = ctx(8, 16, 16, 8).with_tile_bounds(0, 16, 16, 32);
    let neighbours = tile_start_block.neighbours(PlaneId::U);

    assert_eq!(
        (neighbours.has_above(), neighbours.has_left()),
        (true, false)
    );
    assert_rect_chroma_admitted(SupportedChromaMode::D113, tile_start_block);
}

#[test]
fn admits_chroma_ref_rect_d157_follow_chroma_with_above_left_edges() {
    let chroma_ref = BlockRect::new(24, 220, 2, 4);
    let chroma_tx = TxShape::from_luma_4x4(2, 4).expect("valid chroma reference transform");
    let rect_block = ctx(24, 221, 1, 4).with_chroma_ref(chroma_ref, chroma_tx);

    assert_rect_chroma_admitted(SupportedChromaMode::D157Follow, rect_block);
}

#[test]
fn admits_10bit_square_paeth_chroma_subblock_with_above_left_edges() {
    let square_subblock = ctx(40, 160, 8, 8);

    assert_chroma_admitted(SupportedChromaMode::Paeth, square_subblock);
}

#[test]
fn admits_10bit_small_d203_follow_chroma_subblock() {
    let d203_subblock = ctx(32, 300, 2, 8);

    assert_chroma_admitted(SupportedChromaMode::D203Follow, d203_subblock);
}

#[test]
fn admits_rect_d203_follow_chroma_subblock_with_left_edge() {
    let d203_subblock = ctx(32, 300, 2, 8);

    assert_rect_chroma_admitted(SupportedChromaMode::D203Follow, d203_subblock);
}

#[test]
fn admits_rect_d203_follow_chroma_subblock_with_above_only_edge() {
    let d203_subblock = ctx(46, 0, 4, 2);
    let neighbours = d203_subblock.neighbours(PlaneId::U);

    assert!(neighbours.has_above());
    assert!(!neighbours.has_left());
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::D203Follow, 0, None, d203_subblock),
        RectChromaPlan::OneSided(203, None)
    );
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
        plan_luma_prediction(&modes, block, false),
        Ok(crate::prediction::intra::IntraLumaPlan::Dip {
            mode: 2,
            transpose: true,
        })
    );
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
fn admits_top_left_full_sb_lossless_directional_luma_cases() {
    let top_left = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);

    for (label, y_mode, expected) in [
        (
            "d67",
            IntraYMode::D67_PRED_FOR_TEST,
            IntraLumaPlan::DirectionalOneSidedAbove { p_angle: 67 },
        ),
        (
            "d113",
            IntraYMode::D113_PRED_FOR_TEST,
            IntraLumaPlan::DirectionalFirst {
                mode: SupportedDirectionalLumaMode::D113,
            },
        ),
        (
            "d157",
            IntraYMode::D157_PRED_FOR_TEST,
            IntraLumaPlan::DirectionalFirst {
                mode: SupportedDirectionalLumaMode::D157,
            },
        ),
        (
            "d203",
            IntraYMode::D203_PRED_FOR_TEST,
            IntraLumaPlan::DirectionalOneSidedLeft { p_angle: 203 },
        ),
    ] {
        let modes = luma_modes(y_mode);

        assert_eq!(
            plan_luma_prediction_for_segment(&modes, top_left, true, FULL_SB_N4_LUMA),
            Ok(expected),
            "{label}"
        );
    }
}

#[test]
fn top_left_d113_luma_requires_lossless_admission() {
    let top_left = ctx_with_bit_depth(0, 0, FULL_SB_N4_LUMA, FULL_SB_N4_LUMA, BitDepth::Eight);
    let modes = luma_modes(IntraYMode::D113_PRED_FOR_TEST);

    assert!(plan_luma_prediction(&modes, top_left, false).is_err());
    assert!(plan_luma_prediction_for_segment(&modes, top_left, false, FULL_SB_N4_LUMA).is_err());
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

    assert!(plan_luma_prediction(&modes, first_col_block, false).is_err());
    assert_eq!(
        rect_luma_plan(&modes, first_col_block, false, 32),
        Ok(RectLumaPlan::OneSidedAbove {
            p_angle: 64,
            use_tcq: false,
        })
    );
    assert!(square_luma_needs_rect_residual_path(
        &modes,
        first_col_block,
        false,
        32,
        false
    ));
}

#[test]
fn admits_rect_middle_chroma_cases() {
    for (label, mode, angle_delta, block, expected) in [
        (
            "above-left d135 follow",
            SupportedChromaMode::D135Follow,
            -3,
            ctx(FULL_SB_N4_LUMA, 320, 8, FULL_SB_N4_LUMA),
            RectChromaPlan::Middle(126, None),
        ),
        (
            "top-row d113 follow with left-only edge",
            SupportedChromaMode::D113Follow,
            -1,
            ctx(0, 316, 4, 8),
            RectChromaPlan::Middle(110, None),
        ),
        (
            "top-row d157 follow with left-only edge",
            SupportedChromaMode::D157Follow,
            -1,
            ctx(0, 352, 16, 8),
            RectChromaPlan::Middle(154, None),
        ),
        (
            "top-row d135 with left-only edge",
            SupportedChromaMode::D135,
            0,
            ctx(0, 320, 32, 16),
            RectChromaPlan::Middle(135, None),
        ),
    ] {
        assert_rect_chroma_plan(mode, angle_delta, block, expected, label);
    }
}

#[test]
fn follows_luma_angle_delta_for_directional_chroma() {
    let tall_chroma_block = ctx(48, 220, 4, 16);

    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::VerticalFollow,
            -1,
            None,
            tall_chroma_block,
        ),
        RectChromaPlan::OneSided(87, None)
    );
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::VerticalFollow,
            1,
            None,
            tall_chroma_block,
        ),
        RectChromaPlan::Middle(93, None)
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::Vertical, 0, None, tall_chroma_block),
        RectChromaPlan::Mode(SupportedChromaMode::Vertical, None)
    );
    assert_eq!(
        rect_chroma_plan_for_mode(
            SupportedChromaMode::Vertical,
            0,
            Some(DpcmDirection::Vertical),
            tall_chroma_block,
        ),
        RectChromaPlan::Mode(SupportedChromaMode::Vertical, Some(DpcmDirection::Vertical))
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
            ctx(8, 4, 4, 2),
        ),
        RectChromaPlan::OneSided(186, horizontal_dpcm)
    );
}

#[test]
fn admits_top_row_rect_d45_follow_chroma_with_left_only_edge() {
    let first_row_rect_block = ctx(0, 352, 32, 16);
    let right_edge_rect_block = ctx(28, 478, 2, 2);

    assert_rect_chroma_admitted(SupportedChromaMode::D45Follow, first_row_rect_block);
    assert_eq!(
        right_edge_rect_block
            .neighbours(PlaneId::U)
            .num_above_right(),
        0
    );
    assert_eq!(
        rect_chroma_plan_for_mode(SupportedChromaMode::D45, 0, None, right_edge_rect_block),
        RectChromaPlan::OneSided(45, None)
    );
}

#[test]
fn admits_rect_middle_chroma_with_one_sided_edge() {
    let first_row_rect_block = ctx(0, 320, 8, FULL_SB_N4_LUMA);
    let first_col_rect_block = ctx(320, 0, FULL_SB_N4_LUMA, 8);
    let modes = [
        SupportedChromaMode::D113Follow,
        SupportedChromaMode::D113,
        SupportedChromaMode::D135Follow,
        SupportedChromaMode::D135,
        SupportedChromaMode::D157Follow,
        SupportedChromaMode::D157,
    ];

    for mode in modes {
        assert_rect_chroma_admitted(mode, first_row_rect_block);
        assert_rect_chroma_admitted(mode, first_col_rect_block);
    }
}
