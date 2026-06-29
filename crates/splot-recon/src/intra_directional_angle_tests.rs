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

    predict_intra_directional_angle_rect_one_sided_idif_from_p_angle_into(
        BitDepth::Eight,
        rect_size(2, 2),
        81,
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
fn directional_angle_prediction_rejects_unsupported_pangles_without_mutation() {
    let above = [10, 20, 30, 40, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    let err = predict_intra_directional_angle_rect_from_p_angle_into(
        BitDepth::Eight,
        rect_size(2, 2),
        90,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::UnsupportedIntraDirectionalAngle { p_angle: 90 }
    );
    assert_eq!(output, [9u8; 16]);
}

#[test]
fn directional_angle_prediction_rejects_all_excluded_pangles() {
    for p_angle in [0, 90, 113, 135, 157, 180, 270] {
        assert_eq!(
            IntraDirectionalAngle::try_from_p_angle(p_angle),
            Err(ReconError::UnsupportedIntraDirectionalAngle { p_angle })
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
            angle: IntraDirectionalAngle::D203,
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
    let mut output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            100, 10, 20, 30, 50, 100, 10, 20, 60, 50, 100, 10, 70, 60, 50, 100
        ]
    );
}

#[test]
fn d113_middle_prediction_matches_non_idif_bilinear_formula() {
    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];
    let mut output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D113,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            44, 23, 43, 63, 78, 15, 35, 55, 79, 21, 28, 48, 27, 55, 20, 40
        ]
    );
}

#[test]
fn d157_middle_prediction_matches_non_idif_bilinear_formula() {
    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];
    let mut output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
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
    let mut output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            100, 10, 20, 30, 50, 100, 10, 20, 60, 50, 100, 10, 70, 60, 50, 100
        ]
    );
}

#[test]
fn d157_idif_middle_prediction_applies_the_4_tap_filter_and_differs_from_bilinear() {
    let above_idif = [100, 100, 10, 30, 50, 70, 70, 70];
    let left_idif = [110, 110, 20, 40, 60, 80, 80, 80];
    let mut idif_output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut idif_output,
        4,
    )
    .unwrap();

    assert_eq!(
        idif_output,
        [
            50, 88, 69, 7, 24, 16, 28, 63, 53, 45, 34, 20, 74, 66, 58, 50
        ]
    );

    let above = [100, 10, 30, 50, 70];
    let left = [110, 20, 40, 60, 80];
    let mut bilinear_output = [0u8; 16];
    predict_intra_middle_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut bilinear_output,
        4,
    )
    .unwrap();
    assert_ne!(idif_output, bilinear_output);
}

/// NON-CANONICAL zone-2 pAngle (`pAngle == 132`, an `AngleDeltaY != 0` middle
/// angle that is NOT one of the named `D113` / `D135` / `D157` modes) over a 4x4
/// luma IDIF block with ASYMMETRIC above / left edges and a DISTINCT corner. The
/// expected samples are computed VERBATIM from AVM `av2_highbd_dr_prediction_z2_idif_c`
/// (`dx = dr_intra_derivative[180 - 132] == dr_intra_derivative[48] == 56`,
/// `dy = dr_intra_derivative[132 - 90] == dr_intra_derivative[42] == 73`, the
/// `base_x >= -1` above-vs-left branch, the 4-tap `Dr_Interp_Filter[shift]` over the
/// IDIF edge, `Clip1(Round2(s, 7))`). This proves the generalized
/// [`IntraMiddleDirectionalAngle::try_from_p_angle`] / `branch` computes the correct
/// per-angle `(dx, dy)` from the §9.2 table for an arbitrary in-band pAngle, not just
/// the three canonical angles. The asymmetric edges make a wrong `(dx, dy)`, a
/// swapped above/left branch, or a wrong shift observable.
#[test]
fn idif_middle_prediction_noncanonical_p132_matches_avm_z2_idif() {
    let above_idif = [200, 200, 120, 130, 140, 150, 150, 150];
    let left_idif = [200, 200, 60, 70, 80, 90, 90, 90];
    let mut output = [0u8; 16];

    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::try_from_p_angle(132).unwrap(),
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            192, 117, 131, 141, 78, 181, 115, 132, 63, 95, 170, 115, 79, 58, 119, 159
        ]
    );
}

#[test]
fn idif_middle_prediction_clamps_negative_4_tap_sum_to_zero() {
    let above_idif = [0, 0, 0, 0, 0, 0, 0, 0];
    let left_idif = [255, 255, 0, 0, 255, 255, 0, 0];
    let mut output = [9u8; 16];

    predict_intra_middle_directional_angle_rect_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraMiddleDirectionalAngle::D157,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
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
fn idif_middle_prediction_rejects_unsupported_pangles_without_mutation() {
    let above_idif = [100u8; 8];
    let left_idif = [110u8; 8];
    let mut output = [9u8; 16];

    let err = predict_intra_middle_directional_angle_rect_idif_from_p_angle_into(
        BitDepth::Eight,
        rect_size(2, 2),
        45,
        IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle: 45 }
    );
    assert_eq!(output, [9u8; 16]);
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
fn middle_directional_angle_prediction_rejects_unsupported_pangles_without_mutation() {
    let above = [100, 10, 20, 30, 40];
    let left = [110, 50, 60, 70, 80];
    let mut output = [9u8; 16];

    for p_angle in [0, 45, 67, 90, 180, 203, 270] {
        assert_eq!(
            IntraMiddleDirectionalAngle::try_from_p_angle(p_angle),
            Err(ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle })
        );
    }

    let err = predict_intra_middle_directional_angle_rect_from_p_angle_into(
        BitDepth::Eight,
        rect_size(2, 2),
        45,
        IntraMiddleDirectionalAngleEdges::both(&left, &above),
        &mut output,
        4,
    )
    .unwrap_err();

    assert_eq!(
        err,
        ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle: 45 }
    );
    assert_eq!(output, [9u8; 16]);
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

#[test]
fn intra_edge_filter_strength_zero_is_a_no_op() {
    let mut edge = [10u16, 200, 30, 180, 50, 160, 70, 140];
    let original = edge;
    apply_intra_edge_filter(&mut edge, 8, 0).unwrap();
    assert_eq!(edge, original, "strength 0 must leave the edge unchanged");
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
fn intra_edge_filter_rejects_size_beyond_edge() {
    let mut edge = [1u16, 2, 3, 4];
    assert!(apply_intra_edge_filter(&mut edge, 5, 1).is_err());
}

#[test]
fn filter_intra_edge_corner_matches_spec_verbatim() {
    assert_eq!(filter_intra_edge_corner(40, 200, 80), 113);
    assert_eq!(filter_intra_edge_corner(7, 255, 3), 99);
}
