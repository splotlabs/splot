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
fn d45_one_sided_idif_matches_bilinear_copy_for_shift_zero() {
    // pAngle 45 has `shift == 0` for every projection, so the §7.13.2.8 IDIF
    // 4-tap (`Dr_Interp_Filter[0] == {0, 128, 0, 0}`) reduces to the sample copy
    // `AboveRow[base]`, bit-identical to the bilinear one-sided branch. The IDIF
    // above edge spans logical `-2 ..= w + h + 1` (length `w + h + 4 == 12` for
    // 4x4): slice[0] = logical -2 (corner ext), slice[1] = logical -1 (corner),
    // slice[2 + k] = logical k. The logical samples 0..=7 mirror the bilinear
    // `d45_prediction_uses_above_edge_and_edge_end_fallback` test (10..80), so the
    // output must match it exactly.
    let above_idif = [5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
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
fn d45_one_sided_idif_clamps_base_at_max_base_x() {
    // The bottom-right sample of a 4x4 D45 block projects `base = (3 + 1) + 3 = 7
    // == maxBaseX`, still within the IDIF range; `base == maxBaseX + 1` would clamp
    // to `AboveRow[maxBaseX]`. Use a strictly-increasing edge so the clamp is
    // observable: `AboveRow[7] == 80`, and the extension slots `AboveRow[8] ==
    // AboveRow[9] == 80` (so even at the boundary the copy stays 80).
    let above_idif = [0, 0, 1, 2, 3, 4, 5, 6, 7, 80, 80, 80];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D45,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        &mut output,
        4,
    )
    .unwrap();

    // pred[i][j] = AboveRow[i + 1 + j], logical 1..=7 -> slice 3..=9.
    assert_eq!(output, [2, 3, 4, 5, 3, 4, 5, 6, 4, 5, 6, 7, 5, 6, 7, 80]);
}

#[test]
fn d45_one_sided_idif_rejects_wrong_length_edge() {
    // The IDIF above edge must be `w + h + 4` samples (12 for 4x4); a shorter edge
    // is rejected before any output mutation.
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
    // §7.13.2.8 step 3 (zone-3, pAngle 203): the symmetric mirror of D45.
    // `dy = Dr_Intra_Derivative[270 - 203] = Dr_Intra_Derivative[67] = 24`,
    // `idx = (j + 1) * dy`, `base = (idx >> 6) + i`, `shift = (idx >> 1) & 0x1F`.
    // For 4x4 the shifts (j = 0..3) are {12, 24, 4, 16} — genuinely nonzero, so
    // the IDIF 4-tap `Dr_Interp_Filter` interpolates over the left column (NOT a
    // degenerate copy like D45). The left IDIF edge spans logical `-2 ..= w + h + 1`
    // (length 12 for 4x4): slice[0] = logical -2, slice[1] = logical -1 (corner),
    // slice[2 + k] = logical k. The reference output is computed by the §7.13.2.8
    // step-3 formula over the strictly-increasing edge 10..80.
    let left_idif = [5, 5, 10, 20, 30, 40, 50, 60, 70, 80, 80, 80];
    let mut output = [0u8; 16];

    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        rect_size(2, 2),
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::left(&left_idif),
        &mut output,
        4,
    )
    .unwrap();

    assert_eq!(
        output,
        [
            13, 17, 21, 25, 24, 28, 31, 35, 34, 38, 41, 45, 44, 48, 51, 55
        ]
    );
}

#[test]
fn widened_zone1_one_sided_idif_interpolates_with_nonzero_shift() {
    // §7.13.2.8 step 1 over a WIDENED zone-1 pAngle (81, admitted by the §9.2
    // range check, not the legacy D45/D67 whitelist): `dx = Dr_Intra_Derivative[81]
    // = 8`, `idx = (i + 1) * dx`, `base = idx >> 6`, `shift = (idx >> 1) & 0x1F`.
    // For 4x4 the per-row shifts are {4, 9, 13, 18} — all genuinely nonzero, so the
    // IDIF 4-tap `Dr_Interp_Filter` interpolates over the above row (NOT a
    // degenerate `shift == 0` copy like D45). This guards a silent filter-phase
    // regression: a wrong `Dr_Interp_Filter` row or a wrong `shift` derivation
    // changes the output. The above IDIF edge spans logical `-2 ..= w + h + 1`
    // (length 12 for 4x4): slice[0] = logical -2, slice[1] = logical -1 (corner),
    // slice[2 + k] = logical k; the trailing two repeat `maxBaseX`. The ASYMMETRIC
    // strictly-increasing edge makes a symbol/phase swap detectable (per the
    // decode-verify asymmetry lesson). Reference output hand-derived from the
    // §7.13.2.8 step-1 formula.
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
    // D203 is a LEFT-reading zone-3 angle; an above-edge set does not match its
    // required edge and is rejected before any output mutation.
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
    // The IDIF left edge must be `w + h + 4` samples (12 for 4x4); a shorter edge
    // is rejected before any output mutation.
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
    // Over a FLAT left column the §7.13.2.8 step-3 IDIF (luma) and the bilinear
    // one-sided branch (chroma) both reduce to the flat value (the filter taps sum
    // to 128, `Round2(128 * v, 7) == v`). This mirrors the flat-chroma D203-follow
    // path in the committed fixture.
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
fn d135_idif_middle_prediction_is_a_sample_copy_identical_to_bilinear() {
    // §7.13.2.8 D135 (`dx == dy == 64`) projects every sample with `shift == 0`,
    // so `Dr_Interp_Filter[0] == {0, 128, 0, 0}` reduces the IDIF 4-tap to
    // `Clip1(Round2(128 * Edge[base], 7)) == Edge[base]`. The result must be
    // bit-identical to the `enableIdif == 0` bilinear branch over the same edges.
    //
    // IDIF edges span logical `-2..=side+1` (length `side + 4`): index 0 is `-2`,
    // index 1 the `-1` corner. The extension repeats `Edge[-2] = Edge[-1]` and
    // `Edge[side] = Edge[side+1] = Edge[side-1]`.
    // Bilinear edges (logical -1..side, the d135 test): above [100,10,20,30,40],
    // left [110,50,60,70,80].
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

    // Identical to the bilinear D135 copy
    // (`d135_middle_prediction_uses_above_and_left_negative_logical_edges`).
    assert_eq!(
        output,
        [
            100, 10, 20, 30, 50, 100, 10, 20, 60, 50, 100, 10, 70, 60, 50, 100
        ]
    );
}

#[test]
fn d157_idif_middle_prediction_applies_the_4_tap_filter_and_differs_from_bilinear() {
    // §7.13.2.8 D157 (`dx == Dr_Intra_Derivative[23] == 170`,
    // `dy == Dr_Intra_Derivative[67] == 24`) projects with nonzero `shift`, so the
    // IDIF 4-tap genuinely interpolates and the output differs from the bilinear
    // branch over the same edges. Expected values computed directly from
    // `s = Σ Dr_Interp_Filter[shift][t] * Edge[base + t - 1]; Clip1(Round2(s, 7))`.
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

    // The bilinear branch over the inner (logical -1..side) edges differs.
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

#[test]
fn idif_middle_prediction_clamps_negative_4_tap_sum_to_zero() {
    // The §7.13.2.8 IDIF 4-tap has negative taps (e.g. `{-12, 81, 71, -12}`), so a
    // sharp edge — large values at the negative-tap positions (`base - 1`,
    // `base + 2`) and zeros at the positive-tap positions — drives the 4-tap sum
    // NEGATIVE; `Clip1(Round2(s, 7))` must clamp it to 0 (not wrap to a large u8).
    // This left edge (logical -2..=5 = [255, 255, 0, 0, 255, 255, 0, 0]) produces
    // four negative sums that must reconstruct as 0.
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

    // Negative 4-tap sums clamp to exactly 0 (not a wrapped large value); the
    // positive sums interpolate normally. Computed directly from the spec formula.
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

    // D135 (`shift == 0`) is a copy; every value stays within the 10-bit range.
    assert!(output.iter().all(|&v| v <= 1023));
    // Top-left projects to the corner sample `AboveRow[-1] == 900`.
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
    // IDIF edges must be `side + 4` long; a `side + 1` (bilinear-width) edge is
    // rejected before mutation.
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
