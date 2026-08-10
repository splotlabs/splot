// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;

fn mode_index(value: usize) -> ModeIndex {
    ModeIndex::try_new(value).expect("test mode index")
}

fn joint_mode(value: usize) -> IntraJointMode {
    IntraJointMode::try_new(value).expect("test joint mode")
}

#[test]
fn intra_y_mode_encodes_the_complete_thirteen_mode_domain() {
    let modes = [
        IntraYMode::Dc,
        IntraYMode::Vertical,
        IntraYMode::Horizontal,
        IntraYMode::D45,
        IntraYMode::D135,
        IntraYMode::D113,
        IntraYMode::D157,
        IntraYMode::D203,
        IntraYMode::D67,
        IntraYMode::Smooth,
        IntraYMode::SmoothVertical,
        IntraYMode::SmoothHorizontal,
        IntraYMode::Paeth,
    ];
    for (value, mode) in modes.into_iter().enumerate() {
        assert_eq!(mode.value(), value);
    }
    assert_eq!(modes[0].class(), IntraYModeClass::Dc);
    assert_eq!(modes[4].mode_to_angle(), Some(135));
    assert_eq!(
        modes[10].class(),
        IntraYModeClass::Smooth(SupportedNonDcLumaMode::SmoothVertical)
    );
    assert_eq!(modes[12].class(), IntraYModeClass::Paeth);
}

#[test]
fn mode_and_joint_domains_are_bounded_to_zero_through_sixty() {
    for value in 0..INTRA_JOINT_MODE_COUNT {
        assert_eq!(mode_index(value).value(), value);
        assert_eq!(joint_mode(value).value(), value);
    }
    assert!(matches!(
        ModeIndex::try_new(INTRA_JOINT_MODE_COUNT),
        Err(IntraModeStateError::InvalidModeIndex { value: 61 })
    ));
    assert!(matches!(
        IntraJointMode::try_new(INTRA_JOINT_MODE_COUNT),
        Err(IntraModeStateError::InvalidJointMode { value: 61 })
    ));
}

#[test]
fn uv_mode_ctx_is_zero_for_dc_pred_and_one_for_directional() {
    for (mode, expected) in [
        (IntraYMode::Dc, 0),
        (IntraYMode::Vertical, 1),
        (IntraYMode::D67, 1),
        (IntraYMode::Paeth, 0),
    ] {
        assert_eq!(uv_mode_ctx(mode), expected);
    }
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
    let escape = reconstruct_y_mode_top_left(mode_index(10)).expect("mode index 10 reconstructs");
    assert_eq!(
        (
            escape.y_mode,
            escape.angle_delta_y,
            escape.y_mode.is_directional()
        ),
        (IntraYMode::D135, 0, true)
    );
}

#[test]
fn y_mode_offset_escape_is_total_over_the_legal_offset_range() {
    for offset in 0..MODE_OFFSET_COUNT {
        let escape = reconstruct_y_mode_top_left(mode_index(
            usize::from(MODE_INDEX_COUNT - 1) + usize::from(offset),
        ))
        .expect("legal offset reconstructs");
        assert!(escape.angle_delta_y >= -MAX_ANGLE_DELTA);
        assert!(escape.angle_delta_y <= MAX_ANGLE_DELTA);
    }
}

#[test]
fn get_intra_uv_mode_set_directional_luma_returns_y_mode_for_index_zero() {
    let d135 = IntraYMode::D135;
    assert_eq!(
        get_intra_uv_mode_set(d135, 0),
        Some(IntraYMode::D135.value() as u8)
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
        IntraYMode::D113,
        0,
        IntraYMode::D113.value() as u8,
        SupportedChromaMode::D113Follow,
    );
}

#[test]
fn y_mode_offset_escape_reconstructs_d113() {
    let escape = reconstruct_y_mode_top_left(mode_index(9)).expect("mode index 9 reconstructs");
    assert_eq!(escape.y_mode, IntraYMode::D113);
    assert_eq!(escape.angle_delta_y, 0);
    assert_eq!(escape.intra_joint_mode.value(), 29);
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d157_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D157,
        0,
        IntraYMode::D157.value() as u8,
        SupportedChromaMode::D157Follow,
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d203_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D203,
        0,
        IntraYMode::D203.value() as u8,
        SupportedChromaMode::D203Follow,
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d67_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D67,
        0,
        IntraYMode::D67.value() as u8,
        SupportedChromaMode::D67Follow,
    );
}

#[test]
fn supported_chroma_mode_h_luma_can_select_explicit_d67() {
    assert_supported_chroma_mode(
        IntraYMode::Horizontal,
        9,
        IntraYMode::D67.value() as u8,
        SupportedChromaMode::D67,
    );
}

#[test]
fn supported_chroma_mode_directional_luma_resolves_dc_for_uv_mode_one() {
    let d135 = IntraYMode::D135;
    assert_eq!(
        supported_chroma_mode(d135, 1),
        Some(SupportedChromaMode::Dc)
    );
}

#[test]
fn supported_chroma_mode_directional_follow_resolves_d135_for_uv_mode_zero() {
    assert_supported_chroma_mode(
        IntraYMode::D135,
        0,
        IntraYMode::D135.value() as u8,
        SupportedChromaMode::D135Follow,
    );
}

#[test]
fn supported_chroma_mode_explicit_d135_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::Dc,
        8,
        IntraYMode::D135.value() as u8,
        SupportedChromaMode::D135,
    );
}

#[test]
fn supported_chroma_mode_explicit_d203_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::Dc,
        12,
        IntraYMode::D203.value() as u8,
        SupportedChromaMode::D203,
    );
}

#[test]
fn supported_chroma_mode_d67_luma_can_select_explicit_d203() {
    assert_supported_chroma_mode(
        IntraYMode::D67,
        12,
        IntraYMode::D203.value() as u8,
        SupportedChromaMode::D203,
    );
}

#[test]
fn supported_chroma_mode_explicit_d157_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::Dc,
        11,
        IntraYMode::D157.value() as u8,
        SupportedChromaMode::D157,
    );
}

#[test]
fn supported_chroma_mode_explicit_paeth_uses_non_follow_mode() {
    assert_supported_chroma_mode(
        IntraYMode::Dc,
        4,
        IntraYMode::Paeth.value() as u8,
        SupportedChromaMode::Paeth,
    );
}

#[test]
fn supported_chroma_mode_directional_luma_can_select_explicit_paeth() {
    assert_supported_chroma_mode(
        IntraYMode::D45,
        5,
        IntraYMode::Paeth.value() as u8,
        SupportedChromaMode::Paeth,
    );
}

#[test]
fn supported_chroma_mode_non_directional_luma_passes_list_through() {
    let dc = IntraYMode::Dc;
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
    let result = reconstruct_y_mode_top_left(mode_index(5)).expect("mode index 5 reconstructs");
    assert_eq!(result.y_mode, IntraYMode::Vertical);
    assert_eq!(result.angle_delta_y, 0);
    assert_eq!(result.intra_joint_mode.value(), 22);
}

#[test]
fn first_set_directional_reconstructs_h_pred_for_index_six() {
    let result = reconstruct_y_mode_top_left(mode_index(6)).expect("mode index 6 reconstructs");
    assert_eq!(result.y_mode, IntraYMode::Horizontal);
    assert_eq!(result.angle_delta_y, 0);
    assert_eq!(result.intra_joint_mode.value(), 50);
}

#[test]
fn second_set_reconstructs_y_mode_from_y_second_mode() {
    let result = reconstruct_y_mode_top_left(mode_index(13)).expect("mode index 13 reconstructs");
    assert_eq!(result.y_mode, IntraYMode::Vertical);
    assert_eq!(result.angle_delta_y, -2);
    assert_eq!(result.intra_joint_mode.value(), 20);
}

#[test]
fn second_set_reconstructs_later_mode_sets() {
    let result = reconstruct_y_mode_top_left(mode_index(44)).expect("mode index 44 reconstructs");
    assert_eq!(result.y_mode, IntraYMode::D203);
    assert_eq!(result.angle_delta_y, 1);
    assert_eq!(result.intra_joint_mode.value(), 58);
}

#[test]
fn neighbour_reorder_selects_directional_joint_mode_before_default_list() {
    let result = reconstruct_y_mode_with_neighbours(
        mode_index(5),
        [joint_mode(36), IntraJointMode::DC],
        16,
        16,
    )
    .expect("directional neighbour reconstructs");
    assert_eq!(result.intra_joint_mode.value(), 36);
    assert_eq!(result.y_mode, IntraYMode::D135);
    assert_eq!(result.angle_delta_y, 0);
}

#[test]
fn neighbour_reorder_skips_duplicate_directional_neighbours() {
    let result =
        reconstruct_y_mode_with_neighbours(mode_index(6), [joint_mode(36), joint_mode(36)], 16, 16)
            .expect("duplicate directional neighbours reconstruct");
    assert_eq!(result.intra_joint_mode.value(), 35);
}

#[test]
fn neighbour_reorder_uses_default_list_after_small_block_neighbour() {
    let result = reconstruct_y_mode_with_neighbours(
        mode_index(6),
        [joint_mode(36), IntraJointMode::DC],
        2,
        2,
    )
    .expect("small block directional neighbour reconstructs");
    assert_eq!(result.intra_joint_mode.value(), 22);
    assert_eq!(result.y_mode, IntraYMode::Vertical);
}

#[test]
fn neighbour_reorder_runs_for_wide_tall_sub_8x8_blocks() {
    for (n4w, n4h) in [(1usize, 4usize), (4, 1)] {
        let result = reconstruct_y_mode_with_neighbours(
            mode_index(5),
            [joint_mode(18), joint_mode(19)],
            n4w,
            n4h,
        )
        .expect("wide/tall sub-8x8 directional neighbour reconstructs");
        assert_eq!(
            result.intra_joint_mode.value(),
            18,
            "{n4w}x{n4h} stored joint mode"
        );
        assert_eq!(
            result.y_mode,
            IntraYMode::D67,
            "{n4w}x{n4h} reconstructed YMode"
        );
    }
    for (n4w, n4h) in [(1usize, 1usize), (1, 2), (2, 1)] {
        let result = reconstruct_y_mode_with_neighbours(
            mode_index(5),
            [joint_mode(18), joint_mode(19)],
            n4w,
            n4h,
        )
        .expect("sub-8x8 small block reconstructs via the default list");
        assert_eq!(
            result.intra_joint_mode.value(),
            22,
            "{n4w}x{n4h} stays on default list"
        );
    }
}

#[test]
fn supported_chroma_mode_cardinal_follow_resolves_v_and_h_for_uv_mode_zero() {
    let v = IntraYMode::Vertical;
    let h = IntraYMode::Horizontal;
    assert_eq!(
        get_intra_uv_mode_set(v, 0),
        Some(IntraYMode::Vertical.value() as u8)
    );
    assert_eq!(
        get_intra_uv_mode_set(h, 0),
        Some(IntraYMode::Horizontal.value() as u8)
    );
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
    let v = IntraYMode::Vertical;
    assert_eq!(supported_chroma_mode(v, 1), Some(SupportedChromaMode::Dc));
}

#[test]
fn supported_chroma_mode_non_follow_h_pred_over_dc_luma_resolves_horizontal() {
    let dc = IntraYMode::Dc;
    assert!(!dc.is_directional());
    assert_eq!(
        get_intra_uv_mode_set(dc, 6),
        Some(IntraYMode::Horizontal.value() as u8)
    );
    assert_eq!(
        supported_chroma_mode(dc, 6),
        Some(SupportedChromaMode::Horizontal)
    );
}

#[test]
fn every_valid_y_and_uv_mode_pair_resolves_to_a_typed_chroma_mode() {
    let y_modes = [
        IntraYMode::Dc,
        IntraYMode::Vertical,
        IntraYMode::Horizontal,
        IntraYMode::D45,
        IntraYMode::D135,
        IntraYMode::D113,
        IntraYMode::D157,
        IntraYMode::D203,
        IntraYMode::D67,
        IntraYMode::Smooth,
        IntraYMode::SmoothVertical,
        IntraYMode::SmoothHorizontal,
        IntraYMode::Paeth,
    ];
    for y_mode in y_modes {
        for uv_mode in 0..13u8 {
            assert!(
                get_intra_uv_mode_set(y_mode, uv_mode).is_some(),
                "missing coefficient mode for y_mode {} uv_mode {uv_mode}",
                y_mode.value()
            );
            assert!(
                supported_chroma_mode(y_mode, uv_mode).is_some(),
                "missing prediction mode for y_mode {} uv_mode {uv_mode}",
                y_mode.value()
            );
        }
    }
}

#[test]
fn neighbour_reorder_is_total_for_every_typed_mode_and_neighbour_pair() {
    for mode in 0..INTRA_JOINT_MODE_COUNT {
        for left in 0..INTRA_JOINT_MODE_COUNT {
            for above in 0..INTRA_JOINT_MODE_COUNT {
                for (n4w, n4h) in [(1, 1), (2, 2), (16, 16)] {
                    let result = reconstruct_y_mode_with_neighbours(
                        mode_index(mode),
                        [joint_mode(left), joint_mode(above)],
                        n4w,
                        n4h,
                    );
                    assert!(
                        result.is_ok(),
                        "mode={mode} left={left} above={above} geometry={n4w}x{n4h}"
                    );
                }
            }
        }
    }
}
