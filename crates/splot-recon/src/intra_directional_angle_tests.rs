// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
    IntraRectBlockSize::new(log2_width, log2_height).unwrap()
}

fn assert_angle_prediction(
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleEdges<'_, u8>,
    expected: [u8; 16],
) {
    let mut output = [0u8; 16];
    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        angle,
        edges,
        &mut output,
        4,
    )
    .unwrap();
    assert_eq!(output, expected);
}

fn assert_idif_prediction(
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleIdifEdges<'_, u8>,
    expected: [u8; 16],
) {
    let mut output = [0u8; 16];
    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        angle,
        edges,
        &mut output,
        4,
    )
    .unwrap();
    assert_eq!(output, expected);
}

fn middle_prediction(
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleEdges<'_, u8>,
) -> [u8; 16] {
    let mut output = [0u8; 16];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        angle,
        edges,
        &mut output,
        4,
    )
    .unwrap();
    output
}

fn middle_idif_prediction(
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleIdifEdges<'_, u8>,
) -> [u8; 16] {
    let mut output = [0u8; 16];
    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        angle,
        edges,
        &mut output,
        4,
    )
    .unwrap();
    output
}

#[test]
fn d45_prediction_uses_above_edge_and_edge_end_fallback() {
    let above = [10, 20, 30, 40, 50, 60, 70, 80];
    assert_angle_prediction(
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        [
            20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80,
        ],
    );
}

#[test]
fn d45_one_sided_idif_matches_bilinear_copy_for_shift_zero() {
    let above_idif = [5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80];
    assert_idif_prediction(
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        [
            20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80,
        ],
    );
}

#[test]
fn d45_one_sided_idif_clamps_base_at_max_base_x() {
    let above_idif = [0, 0, 1, 2, 3, 4, 5, 6, 7, 80, 80, 80];
    assert_idif_prediction(
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        [2, 3, 4, 5, 3, 4, 5, 6, 4, 5, 6, 7, 5, 6, 7, 80],
    );
}

/// D45 (dx = 64) has shift == 0 everywhere, so the IDIF reduces to the copy
/// `Edge[base]` with `base = (i + 1 + mrlIndex) + j`. The ascending edge makes the
/// mrlIndex shift visible (slot k is logical k - 2; edge length `w + h + 4 + (mrl <<
/// 1)`): mrlIndex == 0 gives `value = (i + j + 1) + 2`, and mrlIndex == 2 reads two
/// lines further out (`value = (i + j + 3) + 2`, every sample larger by 2).
#[test]
fn d45_mrl_idif_reads_the_offset_reference_line_not_the_adjacent_one() {
    let above_idif: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut mrl0 = [0u8; 16];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif[..12]),
        0,
        &mut mrl0,
        4,
    )
    .unwrap();
    assert_eq!(mrl0, [3, 4, 5, 6, 4, 5, 6, 7, 5, 6, 7, 8, 6, 7, 8, 9]);

    let mut mrl2 = [0u8; 16];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        2,
        &mut mrl2,
        4,
    )
    .unwrap();
    assert_eq!(mrl2, [5, 6, 7, 8, 6, 7, 8, 9, 7, 8, 9, 10, 8, 9, 10, 11]);
}

/// `maxBase = w + h - 1 + (mrlIndex << 1) == 9` for 4x4 mrlIndex == 1; the trailing
/// slots repeat the clamp value, so `base = (i + 2 + j)`, `value = Edge[min(base,
/// 9)]`. The 250 sentinels past maxBase prove the walk never over-reads.
#[test]
fn d45_mrl_idif_clamps_at_the_widened_max_base() {
    let above_idif: [u8; 14] = [0, 0, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80, 250, 250];
    let mut output = [0u8; 16];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        1,
        &mut output,
        4,
    )
    .unwrap();
    assert_eq!(
        output,
        [
            30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80, 60, 70, 80, 80
        ]
    );
}

#[test]
fn d45_one_sided_idif_rejects_wrong_length_edge() {
    let above_idif = [0u8; 8];
    let mut output = [0u8; 16];

    let result = predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        &mut output,
        4,
    );
    assert!(result.is_err());
    assert_eq!(output, [0u8; 16]);
}

#[test]
fn d203_one_sided_idif_interpolates_real_left_column() {
    let left_idif = [5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80];
    assert_idif_prediction(
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::left(&left_idif),
        [
            13, 17, 21, 25, 24, 28, 31, 35, 34, 38, 41, 45, 44, 48, 51, 55,
        ],
    );
}

#[test]
fn widened_zone1_one_sided_idif_interpolates_with_nonzero_shift() {
    let above_idif = [10, 10, 25, 33, 48, 60, 77, 90, 110, 130, 130, 130];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::try_from_p_angle(81).unwrap(),
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            26, 35, 49, 62, 27, 37, 51, 65, 28, 39, 53, 67, 29, 41, 55, 70
        ]
    );
}

#[test]
fn d203_one_sided_idif_rejects_above_edge() {
    let above_idif = [5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80];
    let mut output = [0u8; 16];

    let result = predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        &mut output,
        4,
    );
    assert!(result.is_err());
    assert_eq!(output, [0u8; 16]);
}

#[test]
fn d203_one_sided_idif_rejects_wrong_length_edge() {
    let left_idif = [0u8; 8];
    let mut output = [0u8; 16];

    let result = predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::left(&left_idif),
        &mut output,
        4,
    );
    assert!(result.is_err());
    assert_eq!(output, [0u8; 16]);
}

#[test]
fn d203_one_sided_idif_matches_bilinear_for_flat_left_column() {
    let flat_idif = [120u8; 12];
    let flat_bilinear = [120u8; 8];
    let mut idif_output = [0u8; 16];
    let mut bilinear_output = [0u8; 16];

    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::left(&flat_idif),
        &mut idif_output,
        4,
    )
    .unwrap();
    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleEdges::left(&flat_bilinear),
        &mut bilinear_output,
        4,
    )
    .unwrap();

    assert_eq!(idif_output, [120u8; 16]);
    assert_eq!(bilinear_output, [120u8; 16]);
}

#[test]
fn d67_prediction_matches_non_idif_bilinear_formula() {
    let above = [0, 32, 64, 96, 128, 160, 192, 224];
    assert_angle_prediction(
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleEdges::above(&above),
        [
            12, 44, 76, 108, 24, 56, 88, 120, 36, 68, 100, 132, 48, 80, 112, 144,
        ],
    );
}

#[test]
fn d203_prediction_matches_non_idif_bilinear_formula() {
    let left = [0, 32, 64, 96, 128, 160, 192, 224];
    assert_angle_prediction(
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleEdges::left(&left),
        [
            12, 24, 36, 48, 44, 56, 68, 80, 76, 88, 100, 112, 108, 120, 132, 144,
        ],
    );
}

#[test]
fn directional_angle_prediction_accepts_10_bit_u16_samples() {
    let above = [0u16, 64, 128, 192, 256, 320, 384, 1023];
    let mut output = [0u16; 16];

    predict_intra_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(output[0], 24);
    assert_eq!(output[15], 288);
}

#[test]
fn directional_angle_types_reject_out_of_zone_pangles() {
    for p_angle in [0, 90, 113, 135, 157, 180, 270] {
        assert_eq!(
            IntraDirectionalAngle::try_from_p_angle(p_angle),
            Err(ReconError::UnsupportedIntraDirectionalAngle { p_angle })
        );
    }
    for p_angle in [0, 45, 67, 90, 180, 203, 270] {
        assert_eq!(
            IntraMiddleDirectionalAngle::try_from_p_angle(p_angle),
            Err(ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle })
        );
    }
}

#[test]
fn directional_angle_prediction_validates_required_edge_presence() {
    let mut output = [9u8; 16];

    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleEdges::new(None, None),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraDirectionalAngleEdgeUnavailable {
            p_angle: 203,
            edge: IntraDirectionalAngleEdge::Left
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn directional_angle_prediction_validates_edge_lengths() {
    let above = [10, 20, 30, 40, 50, 60, 70];
    let mut output = [9u8; 16];

    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraDirectionalAngleEdgeLengthMismatch {
            edge: IntraDirectionalAngleEdge::Above,
            expected: 8,
            actual: 7
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn directional_angle_prediction_clamps_zone1_projection_to_edge_end() {
    let above = (0..48).collect::<Vec<u16>>();
    let mut output = vec![999u16; 32 * 16];

    predict_intra_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(5, 4),
        IntraDirectionalAngle::try_from_p_angle(39).unwrap(),
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        32,
    )
    .unwrap();

    assert_eq!(output[15 * 32 + 31], 47);
}

#[test]
fn directional_angle_prediction_validates_edge_sample_ranges() {
    let above = [0u16, 1, 2, 3, 4, 5, 6, 1024];
    let mut output = [9u16; 16];

    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraDirectionalAngleSampleOutOfRange {
            edge: IntraDirectionalAngleEdge::Above,
            sample_index: 7,
            value: 1024,
            max: 1023
        }
    );
    assert_eq!(output, [9u16; 16]);
}

#[test]
fn directional_angle_prediction_validates_sample_type_against_bit_depth() {
    let above = [10, 20, 30, 40, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn directional_angle_prediction_validates_output_shape() {
    let above = [10, 20, 30, 40, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        3,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraPredictionStrideTooSmall {
            stride_samples: 3,
            width: 4
        }
    );
    assert_eq!(output, [9u8; 16]);

    let mut short_output = [9u8; 15];
    let err = predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        &mut short_output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraPredictionOutputTooSmall {
            expected: 16,
            actual: 15
        }
    );
    assert_eq!(short_output, [9u8; 15]);
}

#[test]
fn d135_middle_prediction_uses_above_and_left_negative_logical_edges() {
    let above = [100, 10, 20, 30, 40];
    let left = [110, 50, 60, 70, 80];

    assert_eq!(
        middle_prediction(
            IntraMiddleDirectionalAngle::D135,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
        ),
        [
            100, 10, 20, 30, 50, 100, 10, 20, 60, 50, 100, 10, 70, 60, 50, 100
        ]
    );
}

#[test]
fn d113_middle_prediction_matches_non_idif_bilinear_formula() {
    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];

    assert_eq!(
        middle_prediction(
            IntraMiddleDirectionalAngle::D113,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
        ),
        [
            44, 23, 43, 63, 78, 15, 35, 55, 79, 21, 28, 48, 27, 55, 20, 40
        ]
    );
}

#[test]
fn d157_middle_prediction_matches_non_idif_bilinear_formula() {
    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];

    assert_eq!(
        middle_prediction(
            IntraMiddleDirectionalAngle::D157,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
        ),
        [
            54, 88, 69, 17, 33, 25, 31, 65, 53, 45, 38, 30, 73, 65, 58, 50
        ]
    );
}

#[test]
fn middle_directional_angle_prediction_accepts_10_bit_u16_samples() {
    let above = [900u16, 100, 200, 300, 400];
    let left = [950u16, 500, 600, 700, 1023];
    let mut output = [0u16; 16];

    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(output[0], 900);
    assert_eq!(output[15], 900);
}

#[test]
fn d135_idif_middle_prediction_is_a_sample_copy_identical_to_bilinear() {
    let above_idif = [100, 100, 10, 20, 30, 40, 40, 40];
    let left_idif = [110, 110, 50, 60, 70, 80, 80, 80];

    assert_eq!(
        middle_idif_prediction(
            IntraMiddleDirectionalAngle::D135,
            IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        ),
        [
            100, 10, 20, 30, 50, 100, 10, 20, 60, 50, 100, 10, 70, 60, 50, 100
        ]
    );
}

#[test]
fn d157_idif_middle_prediction_applies_the_4_tap_filter_and_differs_from_bilinear() {
    let above_idif = [100, 100, 10, 30, 50, 70, 70, 70];
    let left_idif = [110, 110, 20, 40, 60, 80, 80, 80];
    let idif_output = middle_idif_prediction(
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
    );

    assert_eq!(
        idif_output,
        [
            50, 88, 69, 7, 24, 16, 28, 63, 53, 45, 34, 20, 74, 66, 58, 50
        ]
    );

    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];
    let bilinear_output = middle_prediction(
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
    );
    assert_ne!(idif_output, bilinear_output);
}

#[test]
fn idif_middle_prediction_noncanonical_p132_matches_avm_z2_idif() {
    let above_idif = [200, 200, 120, 130, 140, 150, 150, 150];
    let left_idif = [200, 200, 60, 70, 80, 90, 90, 90];

    assert_eq!(
        middle_idif_prediction(
            IntraMiddleDirectionalAngle::try_from_p_angle(132).unwrap(),
            IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        ),
        [
            192, 117, 131, 141, 78, 181, 115, 132, 63, 95, 170, 115, 79, 58, 119, 159
        ]
    );
}

#[test]
fn idif_middle_prediction_clamps_negative_4_tap_sum_to_zero() {
    let above_idif = [0, 0, 0, 0, 0, 0, 0, 0];
    let left_idif = [255, 255, 0, 0, 255, 255, 0, 0];

    assert_eq!(
        middle_idif_prediction(
            IntraMiddleDirectionalAngle::D157,
            IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        ),
        [
            92, 197, 0, 0, 0, 0, 26, 128, 163, 58, 0, 0, 255, 255, 229, 128
        ]
    );
}

#[test]
fn idif_middle_prediction_accepts_10_bit_u16_samples_and_clips_to_bit_depth() {
    let above_idif = [900u16, 900, 100, 200, 300, 400, 400, 400];
    let left_idif = [950u16, 950, 500, 600, 700, 1023, 1023, 1023];
    let mut output = [0u16; 16];

    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert!(output.iter().all(|&v| v <= 1023));
    assert_eq!(output[0], 900);
}

#[test]
fn idif_middle_prediction_validates_idif_edge_lengths() {
    let above_short = [100u8, 10, 20, 30, 40];
    let left_idif = [110u8; 8];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_short),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraMiddleDirectionalAngleEdgeLengthMismatch {
            edge: IntraDirectionalAngleEdge::Above,
            expected: 8,
            actual: 5,
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn middle_directional_angle_prediction_accepts_asymmetric_block_bounds() {
    let above_wide = [0u16; 65];
    let left_wide = [0u16; 5];
    let mut wide_output = [0u16; 64 * 4];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(6, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleEdges::both(&left_wide, &above_wide),
        &mut wide_output,
        64,
    )
    .unwrap();

    let above_tall = [0u16; 5];
    let left_tall = [0u16; 65];
    let mut tall_output = [0u16; 64 * 4];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 6),
        IntraMiddleDirectionalAngle::D113,
        IntraMiddleDirectionalAngleEdges::both(&left_tall, &above_tall),
        &mut tall_output,
        4,
    )
    .unwrap();
}

#[test]
fn middle_directional_angle_prediction_validates_required_edge_presence() {
    let above = [100, 10, 20, 30, 40];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::new(None, Some(&above)),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle: IntraMiddleDirectionalAngle::D135,
            edge: IntraDirectionalAngleEdge::Left
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn middle_directional_angle_prediction_validates_edge_lengths() {
    let above = [100, 10, 20, 30];
    let left = [110, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraMiddleDirectionalAngleEdgeLengthMismatch {
            edge: IntraDirectionalAngleEdge::Above,
            expected: 5,
            actual: 4
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn middle_directional_angle_prediction_validates_edge_sample_ranges() {
    let above = [0u16, 10, 20, 30, 40];
    let left = [1024u16, 50, 60, 70, 80];
    let mut output = [9u16; 16];

    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraMiddleDirectionalAngleSampleOutOfRange {
            edge: IntraDirectionalAngleEdge::Left,
            sample_index: 0,
            value: 1024,
            max: 1023
        }
    );
    assert_eq!(output, [9u16; 16]);
}

#[test]
fn middle_directional_angle_prediction_validates_sample_type_against_bit_depth() {
    let above = [100, 10, 20, 30, 40];
    let left = [110, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Ten,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten
        }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn middle_directional_angle_prediction_validates_output_shape() {
    let above = [100, 10, 20, 30, 40];
    let left = [110, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        3,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraPredictionStrideTooSmall {
            stride_samples: 3,
            width: 4
        }
    );
    assert_eq!(output, [9u8; 16]);

    let mut short_output = [9u8; 15];
    let err = predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut short_output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::IntraPredictionOutputTooSmall {
            expected: 16,
            actual: 15
        }
    );
    assert_eq!(short_output, [9u8; 15]);
}

fn owned_intra_edge_filter_reference(edge: &[u16], sz: usize, strength: u8) -> Vec<u16> {
    let original = edge[..sz].to_vec(); // splot-copy-ok: test oracle source snapshot
    let mut filtered = edge.to_vec(); // splot-copy-ok: test oracle expected output
    let last = sz - 1;
    let kernel = INTRA_EDGE_KERNEL[usize::from(strength - 1)];
    for (i, output) in filtered.iter_mut().take(sz).enumerate().skip(1) {
        let weighted = kernel
            .iter()
            .enumerate()
            .map(|(j, &tap)| {
                let source = i.saturating_add(j).saturating_sub(2).min(last);
                tap * i32::from(original[source])
            })
            .sum::<i32>();
        *output = ((weighted + 8) >> 4) as u16;
    }
    filtered
}

fn assert_intra_edge_filter_matches_owned_reference<T: ReconSample>(
    edge: Vec<T>,
    sz: usize,
    strength: u8,
) {
    let source = edge
        .iter()
        .map(|sample| sample.to_u16())
        .collect::<Vec<_>>();
    let expected = owned_intra_edge_filter_reference(&source, sz, strength);
    let mut actual = edge;
    apply_intra_edge_filter(&mut actual, sz, strength).unwrap();
    assert_eq!(
        actual
            .iter()
            .map(|sample| sample.to_u16())
            .collect::<Vec<_>>(),
        expected,
        "sz={sz}, strength={strength}, sample_type={}",
        T::TYPE_NAME
    );
}

#[test]
fn intra_edge_filter_kernels_match_spec_verbatim() {
    let base = [10u16, 200, 30, 180, 50, 160, 70, 140];

    let mut s1 = base;
    apply_intra_edge_filter(&mut s1, 8, 1).unwrap();
    assert_eq!(s1, [10, 110, 110, 110, 110, 110, 110, 123]);

    let mut s2 = base;
    apply_intra_edge_filter(&mut s2, 8, 2).unwrap();
    assert_eq!(s2, [10, 88, 130, 93, 125, 98, 120, 118]);

    let mut s3 = base;
    apply_intra_edge_filter(&mut s3, 8, 3).unwrap();
    assert_eq!(s3, [10, 84, 110, 110, 110, 110, 116, 125]);
}

#[test]
fn intra_edge_filter_matches_owned_reference_for_all_sizes_and_strengths() {
    for sz in 1..=129 {
        for strength in 1..=3 {
            let wide = (0..sz + 3)
                .map(|i| ((i * 40_503 + sz * 7_919) ^ (i << 7)) as u16)
                .collect::<Vec<_>>();
            let narrow = wide.iter().map(|&sample| sample as u8).collect();
            assert_intra_edge_filter_matches_owned_reference(wide, sz, strength);
            assert_intra_edge_filter_matches_owned_reference(narrow, sz, strength);

            let last = sz - 1;
            for position in [
                0,
                1.min(last),
                2.min(last),
                last / 2,
                last.saturating_sub(2),
                last.saturating_sub(1),
                last,
            ] {
                let mut impulse = vec![0u16; sz + 3];
                impulse[position] = u16::MAX;
                impulse[sz..].fill(0x5a5a);
                assert_intra_edge_filter_matches_owned_reference(impulse, sz, strength);
            }
        }
    }
}

#[test]
fn intra_edge_filter_preserves_noop_and_error_ordering() {
    let mut edge = [10u16, 20];
    assert_eq!(apply_intra_edge_filter(&mut edge, 3, 0), Ok(()));
    assert_eq!(edge, [10, 20]);

    assert_eq!(apply_intra_edge_filter(&mut edge, 0, u8::MAX), Ok(()));
    assert_eq!(edge, [10, 20]);

    let err = apply_intra_edge_filter(&mut edge, 3, u8::MAX).unwrap_err();
    assert_eq!(
        err,
        ReconError::ArithmeticOverflow {
            context: "intra edge filter size exceeds edge length"
        }
    );
    assert_eq!(edge, [10, 20]);

    let mut single = [9u8];
    let err = apply_intra_edge_filter(&mut single, 1, 4).unwrap_err();
    assert_eq!(
        err,
        ReconError::ArithmeticOverflow {
            context: "intra edge filter strength out of range"
        }
    );
    assert_eq!(single, [9]);

    for strength in 1..=3 {
        apply_intra_edge_filter(&mut single, 1, strength).unwrap();
        assert_eq!(single, [9]);
    }
}

#[test]
fn filter_intra_edge_corner_matches_spec_verbatim() {
    assert_eq!(filter_intra_edge_corner(40, 200, 80), 113);
    assert_eq!(filter_intra_edge_corner(7, 255, 3), 99);
}

/// The row walk must reproduce the per-sample § 7.13.2.8 zone-2 reference
/// exactly for every block shape, middle angle, and MRL index.
#[test]
fn middle_row_walk_matches_the_per_sample_reference() {
    for p_angle in (ZONE_1_MAX + 1)..ZONE_3_MIN {
        let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle).unwrap();
        let branch = angle.branch();
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = rect_size(log2_width, log2_height);
                for mrl_index in 0..4usize {
                    for row in 0..size.height() {
                        let mut walk =
                            MiddleRowWalk::new(row, size.width(), branch, mrl_index).unwrap();
                        for column in 0..size.width() {
                            let expected =
                                middle_sample_reference_mrl(row, column, branch, mrl_index)
                                    .unwrap();
                            assert_eq!(
                                walk.next(),
                                expected,
                                "p_angle {p_angle} mrl {mrl_index} at ({row}, {column})"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The vectorized `u16` one-sided IDIF line writer must reproduce the
/// per-sample § 7.13.2.8 reference exactly. Zone-1 (step 1) writes along the
/// row and zone-3 (step 3) writes down the column, so this covers both the
/// contiguous and the strided store form, every chunk width, the `shift == 0`
/// copy, and the `maxBase` clamp tail.
#[test]
fn one_sided_idif_u16_lines_match_the_per_sample_reference() {
    for p_angle in [3u16, 45, 67, 81, 87, 183, 189, 203, 225, 267] {
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle).unwrap();
        let branch = angle.branch();
        let derivative = one_sided_idif_derivative(angle);
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = rect_size(log2_width, log2_height);
                for mrl_index in 0..3usize {
                    let edge_len = required_one_sided_idif_edge_len(size, mrl_index).unwrap();
                    let edge: Vec<u16> = (0..edge_len)
                        .map(|index| ((index * 37 + 11) % 1024) as u16)
                        .collect();
                    let edges = match angle.required_edge() {
                        IntraDirectionalAngleEdge::Above => {
                            IntraDirectionalAngleIdifEdges::above(&edge)
                        }
                        IntraDirectionalAngleEdge::Left => {
                            IntraDirectionalAngleIdifEdges::left(&edge)
                        }
                    };
                    let stride = size.width() + 3;
                    let mut output = vec![0u16; stride * size.height()];
                    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
                        BitDepth::Ten,
                        size,
                        angle,
                        edges,
                        mrl_index,
                        &mut output,
                        stride,
                    )
                    .unwrap();

                    let max_base = one_sided_max_base(size, mrl_index).unwrap();
                    for row in 0..size.height() {
                        for column in 0..size.width() {
                            let reference = one_sided_idif_reference(
                                branch, row, column, derivative, mrl_index,
                            )
                            .unwrap();
                            let expected = if reference.base <= max_base {
                                idif_tap(&edge, reference.base, reference.shift, BitDepth::Ten)
                                    .unwrap()
                            } else {
                                logical_idif_edge_sample(&edge, max_base).unwrap()
                            };
                            assert_eq!(
                                output[row * stride + column],
                                expected,
                                "p_angle {p_angle} size {}x{} mrl {mrl_index} at ({row}, {column})",
                                size.width(),
                                size.height()
                            );
                        }
                    }
                }
            }
        }
    }
}
