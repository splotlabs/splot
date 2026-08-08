// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;

#[test]
fn minimal_y_mode_reconstruction_maps_set0_index0_to_dc_pred() {
    assert_eq!(reconstruct_minimal_y_mode(0, 0), Some(IntraYMode::DC_PRED));
}

#[test]
fn minimal_y_mode_reconstruction_covers_the_non_directional_subset() {
    for index in 0..NON_DIRECTIONAL_MODES_COUNT {
        let mode =
            reconstruct_minimal_y_mode(0, index as u8).expect("non-directional index is supported");
        assert!(
            !mode.is_directional(),
            "index {index} must be non-directional"
        );
    }
}

#[test]
fn minimal_y_mode_reconstruction_rejects_unsupported_inputs() {
    assert_eq!(reconstruct_minimal_y_mode(1, 0), None);
    assert_eq!(
        reconstruct_minimal_y_mode(0, NON_DIRECTIONAL_MODES_COUNT as u8),
        None
    );
}

#[test]
fn uv_mode_ctx_is_zero_for_dc_pred_and_one_for_directional() {
    assert_eq!(uv_mode_ctx(IntraYMode::DC_PRED), 0);
    assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::V_PRED)), 1);
    assert_eq!(uv_mode_ctx(IntraYMode::D67_PRED_FOR_TEST), 1);
    assert_eq!(uv_mode_ctx(IntraYMode(12)), 0);
}

#[test]
fn luma_txb_skip_ctx_first_block_filling_transform_is_zero() {
    assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);
}

#[test]
fn luma_txb_skip_ctx_uses_min_clamped_level_sum_when_not_filling() {
    assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
    assert_eq!(txb_skip_ctx_luma(9, 9, false, false), 5);
    assert_eq!(txb_skip_ctx_luma(1, 2, false, false), 3);
}

#[test]
fn luma_txb_skip_ctx_fsc_selects_last_context() {
    assert_eq!(txb_skip_ctx_luma(0, 0, true, true), TXB_SKIP_CONTEXTS - 1);
    assert_eq!(txb_skip_ctx_luma(3, 3, false, true), TXB_SKIP_CONTEXTS - 1);
}

#[test]
fn v_txb_skip_ctx_first_block_larger_chroma_is_three() {
    assert_eq!(v_txb_skip_ctx(false, false, true, false), 3);
}

#[test]
fn v_txb_skip_ctx_adds_neighbour_chroma_and_eob_contributions() {
    assert_eq!(v_txb_skip_ctx(false, false, false, false), 0);
    assert_eq!(v_txb_skip_ctx(true, false, false, false), 1);
    assert_eq!(v_txb_skip_ctx(true, true, false, false), 2);
    assert_eq!(v_txb_skip_ctx(true, true, true, false), 5);
    assert_eq!(v_txb_skip_ctx(true, true, true, true), 11);
}

#[test]
fn y_mode_offset_escape_reconstructs_d135() {
    let escape =
        reconstruct_y_mode_offset_escape_top_left(3).expect("y_mode_offset 3 reconstructs a mode");
    assert_eq!(
        (
            escape.y_mode,
            escape.angle_delta_y,
            escape.y_mode.is_directional()
        ),
        (IntraYMode::D135_PRED_FOR_TEST, 0, true)
    );
}

#[test]
fn y_mode_offset_escape_rejects_out_of_range_offset() {
    assert!(reconstruct_y_mode_offset_escape_top_left(MODE_OFFSET_COUNT).is_none());
    assert!(reconstruct_y_mode_offset_escape_top_left(u8::MAX).is_none());
}

#[test]
fn y_mode_offset_escape_is_total_over_the_legal_offset_range() {
    for offset in 0..MODE_OFFSET_COUNT {
        let escape =
            reconstruct_y_mode_offset_escape_top_left(offset).expect("legal offset reconstructs");
        assert!(escape.angle_delta_y >= -MAX_ANGLE_DELTA);
        assert!(escape.angle_delta_y <= MAX_ANGLE_DELTA);
    }
}

#[test]
fn get_intra_uv_mode_set_directional_luma_returns_y_mode_for_index_zero() {
    let d135 = IntraYMode::D135_PRED_FOR_TEST;
    assert_eq!(
        get_intra_uv_mode_set(d135, 0),
        Some(IntraYMode::D135_PRED_FOR_TEST.0)
    );
}

fn assert_supported_chroma_mode(
    y_mode: IntraYMode,
    uv_mode: u8,
    expected_uv_mode: u8,
    expected_supported: SupportedChromaMode,
) {
    assert_eq!(
        get_intra_uv_mode_set(y_mode, uv_mode),
        Some(expected_uv_mode)
    );
    assert_eq!(
        supported_chroma_mode(y_mode, uv_mode),
        Some(expected_supported)
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d113_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D113_PRED_FOR_TEST,
        0,
        IntraYMode::D113_PRED_FOR_TEST.0,
        SupportedChromaMode::D113Follow,
    );
}

#[test]
fn y_mode_offset_escape_reconstructs_d113() {
    let escape =
        reconstruct_y_mode_offset_escape_top_left(2).expect("y_mode_offset 2 reconstructs a mode");
    assert_eq!(escape.y_mode, IntraYMode::D113_PRED_FOR_TEST);
    assert_eq!(escape.angle_delta_y, 0);
    assert_eq!(escape.intra_joint_mode, 29);
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d157_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D157_PRED_FOR_TEST,
        0,
        IntraYMode::D157_PRED_FOR_TEST.0,
        SupportedChromaMode::D157Follow,
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d203_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D203_PRED_FOR_TEST,
        0,
        IntraYMode::D203_PRED_FOR_TEST.0,
        SupportedChromaMode::D203Follow,
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d67_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D67_PRED_FOR_TEST,
        0,
        IntraYMode::D67_PRED_FOR_TEST.0,
        SupportedChromaMode::D67Follow,
    );
}

#[test]
fn supported_chroma_mode_h_luma_can_select_explicit_d67() {
    assert_supported_chroma_mode(
        IntraYMode(IntraYMode::H_PRED),
        9,
        IntraYMode::D67_PRED_FOR_TEST.0,
        SupportedChromaMode::D67,
    );
}

#[test]
fn supported_chroma_mode_directional_luma_resolves_dc_for_uv_mode_one() {
    let d135 = IntraYMode::D135_PRED_FOR_TEST;
    assert_eq!(
        supported_chroma_mode(d135, 1),
        Some(SupportedChromaMode::Dc)
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d135_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D135_PRED_FOR_TEST,
        0,
        IntraYMode::D135_PRED_FOR_TEST.0,
        SupportedChromaMode::D135Follow,
    );
}

#[test]
fn supported_chroma_mode_explicit_d135_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::DC_PRED,
        8,
        IntraYMode::D135_PRED_FOR_TEST.0,
        SupportedChromaMode::D135,
    );
}

#[test]
fn supported_chroma_mode_explicit_d203_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::DC_PRED,
        12,
        IntraYMode::D203_PRED_FOR_TEST.0,
        SupportedChromaMode::D203,
    );
}

#[test]
fn supported_chroma_mode_d67_luma_can_select_explicit_d203() {
    assert_supported_chroma_mode(
        IntraYMode::D67_PRED_FOR_TEST,
        12,
        IntraYMode::D203_PRED_FOR_TEST.0,
        SupportedChromaMode::D203,
    );
}

#[test]
fn supported_chroma_mode_explicit_d157_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::DC_PRED,
        11,
        IntraYMode::D157_PRED_FOR_TEST.0,
        SupportedChromaMode::D157,
    );
}

#[test]
fn supported_chroma_mode_explicit_paeth_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::DC_PRED,
        4,
        IntraYMode::PAETH_PRED,
        SupportedChromaMode::Paeth,
    );
}

#[test]
fn supported_chroma_mode_directional_luma_can_select_explicit_paeth() {
    assert_supported_chroma_mode(
        IntraYMode::D45_PRED_FOR_TEST,
        5,
        IntraYMode::PAETH_PRED,
        SupportedChromaMode::Paeth,
    );
}

#[test]
fn supported_chroma_mode_non_directional_luma_passes_list_through() {
    let dc = IntraYMode::DC_PRED;
    assert_eq!(supported_chroma_mode(dc, 0), Some(SupportedChromaMode::Dc));
    assert_eq!(
        supported_chroma_mode(dc, 1),
        Some(SupportedChromaMode::Smooth)
    );
    assert_eq!(
        supported_chroma_mode(dc, 5),
        Some(SupportedChromaMode::Vertical)
    );
}

#[test]
fn first_set_directional_reconstructs_v_pred_for_index_five() {
    let result = reconstruct_y_mode_first_set_directional_top_left(5)
        .expect("y_mode_index 5 reconstructs V_PRED");
    assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
    assert_eq!(result.angle_delta_y, 0);
    assert_eq!(result.intra_joint_mode, 22);
}

#[test]
fn first_set_directional_reconstructs_h_pred_for_index_six() {
    let result = reconstruct_y_mode_first_set_directional_top_left(6)
        .expect("y_mode_index 6 reconstructs H_PRED");
    assert_eq!(result.y_mode, IntraYMode(IntraYMode::H_PRED));
    assert_eq!(result.angle_delta_y, 0);
    assert_eq!(result.intra_joint_mode, 50);
}

#[test]
fn first_set_directional_rejects_non_directional_or_escape_indices() {
    for index in 0..(NON_DIRECTIONAL_MODES_COUNT as u8) {
        assert!(reconstruct_y_mode_first_set_directional_top_left(index).is_none());
    }
    assert!(reconstruct_y_mode_first_set_directional_top_left(MODE_INDEX_COUNT - 1).is_none());
    assert!(reconstruct_y_mode_first_set_directional_top_left(u8::MAX).is_none());
}

#[test]
fn second_set_reconstructs_y_mode_from_y_second_mode() {
    let result = reconstruct_y_mode_second_set_top_left(1, 0)
        .expect("legal second-mode branch reconstructs");
    assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
    assert_eq!(result.angle_delta_y, -2);
    assert_eq!(result.intra_joint_mode, 20);
}

#[test]
fn second_set_reconstructs_later_mode_sets() {
    let result = reconstruct_y_mode_second_set_top_left(2, 15)
        .expect("later legal second-mode branch reconstructs");
    assert_eq!(result.y_mode, IntraYMode::D203_PRED_FOR_TEST);
    assert_eq!(result.angle_delta_y, 1);
    assert_eq!(result.intra_joint_mode, 58);
}

#[test]
fn second_set_rejects_first_set_and_out_of_range_literals() {
    assert!(reconstruct_y_mode_second_set_top_left(0, 0).is_none());
    assert!(reconstruct_y_mode_second_set_top_left(1, SECOND_MODE_COUNT).is_none());
    assert!(reconstruct_y_mode_second_set_top_left(1, u8::MAX).is_none());
}

#[test]
fn neighbour_reorder_selects_directional_joint_mode_before_default_list() {
    let result = reconstruct_y_mode_with_neighbours(5, [36, 0], 16, 16)
        .expect("directional neighbour reconstructs");
    assert_eq!(result.intra_joint_mode, 36);
    assert_eq!(result.y_mode, IntraYMode::D135_PRED_FOR_TEST);
    assert_eq!(result.angle_delta_y, 0);
}

#[test]
fn neighbour_reorder_skips_duplicate_directional_neighbours() {
    let result = reconstruct_y_mode_with_neighbours(6, [36, 36], 16, 16)
        .expect("duplicate directional neighbours reconstruct");
    assert_eq!(result.intra_joint_mode, 35);
}

#[test]
fn neighbour_reorder_uses_default_list_after_small_block_neighbour() {
    let result = reconstruct_y_mode_with_neighbours(6, [36, 0], 2, 2)
        .expect("small block directional neighbour reconstructs");
    assert_eq!(result.intra_joint_mode, 22);
    assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
}

#[test]
fn neighbour_reorder_runs_for_wide_tall_sub_8x8_blocks() {
    for (n4w, n4h) in [(1usize, 4usize), (4, 1)] {
        let result = reconstruct_y_mode_with_neighbours(5, [18, 19], n4w, n4h)
            .expect("wide/tall sub-8x8 directional neighbour reconstructs");
        assert_eq!(result.intra_joint_mode, 18, "{n4w}x{n4h} stored joint mode");
        assert_eq!(
            result.y_mode,
            IntraYMode::D67_PRED_FOR_TEST,
            "{n4w}x{n4h} reconstructed YMode"
        );
    }
    for (n4w, n4h) in [(1usize, 1usize), (1, 2), (2, 1)] {
        let result = reconstruct_y_mode_with_neighbours(5, [18, 19], n4w, n4h)
            .expect("sub-8x8 small block reconstructs via the default list");
        assert_eq!(
            result.intra_joint_mode, 22,
            "{n4w}x{n4h} stays on default list"
        );
    }
}

#[test]
fn supported_chroma_mode_cardinal_follow_resolves_v_and_h_for_uv_mode_zero() {
    let v = IntraYMode(IntraYMode::V_PRED);
    let h = IntraYMode(IntraYMode::H_PRED);
    assert_eq!(get_intra_uv_mode_set(v, 0), Some(IntraYMode::V_PRED));
    assert_eq!(get_intra_uv_mode_set(h, 0), Some(IntraYMode::H_PRED));
    assert_eq!(
        supported_chroma_mode(v, 0),
        Some(SupportedChromaMode::VerticalFollow)
    );
    assert_eq!(
        supported_chroma_mode(h, 0),
        Some(SupportedChromaMode::HorizontalFollow)
    );
}

#[test]
fn supported_chroma_mode_cardinal_luma_with_dc_chroma_resolves_dc() {
    let v = IntraYMode(IntraYMode::V_PRED);
    assert_eq!(supported_chroma_mode(v, 1), Some(SupportedChromaMode::Dc));
}

#[test]
fn supported_chroma_mode_non_follow_h_pred_over_dc_luma_resolves_horizontal() {
    let dc = IntraYMode(DC_PRED as u8);
    assert!(!dc.is_directional());
    assert_eq!(get_intra_uv_mode_set(dc, 6), Some(IntraYMode::H_PRED));
    assert_eq!(
        supported_chroma_mode(dc, 6),
        Some(SupportedChromaMode::Horizontal)
    );
}
