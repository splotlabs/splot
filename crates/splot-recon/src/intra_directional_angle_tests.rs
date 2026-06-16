// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
    IntraRectBlockSize::new(log2_width, log2_height).unwrap()
}

#[test]
fn d45_prediction_uses_above_edge_and_edge_end_fallback() {
    let above = [10, 20, 30, 40, 50, 60, 70, 80];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80
        ]
    );
}

#[test]
fn d67_prediction_matches_non_idif_bilinear_formula() {
    let above = [0, 32, 64, 96, 128, 160, 192, 224];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleEdges::above(&above),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            12, 44, 76, 108, 24, 56, 88, 120, 36, 68, 100, 132, 48, 80, 112, 144
        ]
    );
}

#[test]
fn d203_prediction_matches_non_idif_bilinear_formula() {
    let left = [0, 32, 64, 96, 128, 160, 192, 224];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleEdges::left(&left),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            12, 24, 36, 48, 44, 56, 68, 80, 76, 88, 100, 112, 108, 120, 132, 144
        ]
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
