// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn build_ref(width: usize, height: usize) -> Vec<u16> {
    let mut samples = vec![0u16; width * height];
    for row in 0..height {
        for col in 0..width {
            samples[row * width + col] = ((row * 17 + col * 11 + (row ^ col) * 3) % 251) as u16;
        }
    }
    samples
}

fn default_params(block_x: i32, block_y: i32, ref_w: i32, ref_h: i32) -> WarpPredictBlockParams {
    WarpPredictBlockParams {
        warp_params: IDENTITY_WARP_PARAMS,
        block_x,
        block_y,
        subsampling_x: 0,
        subsampling_y: 0,
        reference_scale_x: 1 << REF_SCALE_SHIFT,
        reference_scale_y: 1 << REF_SCALE_SHIFT,
        first_x: 0,
        first_y: 0,
        last_x: ref_w - 1,
        last_y: ref_h - 1,
        bit_depth: BitDepth::Eight,
    }
}

fn reference_warp_8x8(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &WarpPredictBlockParams,
) -> Vec<u16> {
    let max_sample = i64::from(params.bit_depth.max_sample());
    reference_warp_rounded(samples, ref_w, ref_h, params, INTER_ROUND1_NON_COMPOUND)
        .into_iter()
        .map(|pred| pred.clamp(0, max_sample) as u16)
        .collect()
}

fn reference_warp_rounded(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &WarpPredictBlockParams,
    round1: u32,
) -> Vec<i64> {
    let clip = |lo: i32, hi: i32, value: i32| value.max(lo).min(hi);
    let round = |value: i64, shift: u32| {
        if shift == 0 {
            value
        } else {
            (value + (1 << (shift - 1))) >> shift
        }
    };
    let fetch = |row: i32, col: i32| {
        let row = clip(0, ref_h as i32 - 1, row) as usize;
        let col = clip(0, ref_w as i32 - 1, col) as usize;
        i64::from(samples[row * ref_w + col])
    };

    let shear = setup_shear(params.warp_params).unwrap();
    let projected = project_section_center(params).unwrap();
    let mut intermediate = [0i64; WARP_INTERMEDIATE_ROWS * WARPED_BLOCK_SIZE];
    for i1 in -7i32..8 {
        for i2 in -4i32..4 {
            let sx = projected.sx4 + shear.alpha * i2 + shear.beta * i1;
            let offset = (round(i64::from(sx), WARPEDDIFF_PREC_BITS)
                + i64::from(WARP_FILTER_CENTER)) as usize;
            let taps = &WARPED_FILTERS[offset];
            let mut sum = 0i64;
            for (i3, &tap) in taps.iter().enumerate() {
                let rr = clip(params.first_y, params.last_y, projected.y4_int + i1);
                let cc = clip(
                    params.first_x,
                    params.last_x,
                    projected.x4_int + i2 - 3 + i3 as i32,
                );
                sum += i64::from(tap) * fetch(rr, cc);
            }
            intermediate[(i1 + 7) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                round(sum, INTER_ROUND0);
        }
    }

    let mut out = vec![0i64; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
    for i1 in -4i32..4 {
        for i2 in -4i32..4 {
            let sy = projected.sy4 + shear.gamma * i2 + shear.delta * i1;
            let offset = (round(i64::from(sy), WARPEDDIFF_PREC_BITS)
                + i64::from(WARP_FILTER_CENTER)) as usize;
            let taps = &WARPED_FILTERS[offset];
            let mut sum = 0i64;
            for (i3, &tap) in taps.iter().enumerate() {
                let row = (i1 + i3 as i32 + 4) as usize;
                let col = (i2 + 4) as usize;
                sum += i64::from(tap) * intermediate[row * WARPED_BLOCK_SIZE + col];
            }
            out[(i1 + 4) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] = round2(sum, round1);
        }
    }
    out
}

#[test]
fn clipped_block_matches_clipped_sections_at_the_target_stride() {
    let (ref_w, ref_h) = (64usize, 64usize);
    let samples = (0..ref_w * ref_h)
        .map(|index| ((index * 37) % 256) as u8)
        .collect::<Vec<u8>>();
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(16, 24, ref_w as i32, ref_h as i32);
    params.warp_params = [0, 0, 1 << WARPEDMODEL_PREC_BITS, 384, -256, 70_000];
    let prepared = PreparedWarpPrediction::new(&params).unwrap();
    let stride = 20;
    let mut output = vec![7u8; stride * 16];
    prepared
        .predict_clipped_into(&view, [16, 24, 16, 16], &mut output, stride)
        .unwrap();
    for (x, y) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
        let mut section = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        prepared
            .predict_block_into(&view, 16 + x as i32, 24 + y as i32, false, &mut section)
            .unwrap();
        for (row, values) in section.chunks_exact(WARPED_BLOCK_SIZE).enumerate() {
            let start = (y + row) * stride + x;
            let want = values.iter().map(|&value| value.clamp(0, 255) as u8);
            assert!(
                output[start..start + 8].iter().copied().eq(want),
                "({x}, {y})"
            );
            assert_eq!(output[(y + row) * stride + 16], 7, "row padding untouched");
        }
    }
    assert!(matches!(
        prepared.predict_clipped_into(&view, [16, 24, 16, 16], &mut output, 15),
        Err(ReconError::StrideTooSmall { .. })
    ));
    for at in [[1 << 31, 24, 16, 16], [16, (1 << 31) - 8, 16, 16]] {
        assert!(matches!(
            prepared.predict_clipped_into(&view, at, &mut output, stride),
            Err(ReconError::ArithmeticOverflow { .. })
        ));
    }
    params.bit_depth = BitDepth::Ten;
    let ten_bit = PreparedWarpPrediction::new(&params).unwrap();
    assert!(matches!(
        ten_bit.predict_clipped_into(&view, [16, 24, 16, 16], &mut output, stride),
        Err(ReconError::SampleTypeUnsupportedBitDepth { .. })
    ));
}

#[test]
fn warped_filters_table_shape_and_sums() {
    assert_eq!(WARPED_FILTERS.len(), 449);
    for (index, row) in WARPED_FILTERS.iter().enumerate() {
        assert_eq!(row.len(), WARP_FILTER_TAPS);
        assert_eq!(
            row.iter().map(|&tap| i32::from(tap)).sum::<i32>(),
            128,
            "row {index}"
        );
        assert!(row.iter().map(|&tap| i32::from(tap).abs()).sum::<i32>() <= 222);
    }
}

#[test]
fn projection_matches_the_wide_sum_and_its_overflow() {
    let values = [i32::MIN, i32::MIN + 1, -65_536, -1, 0, 1, 70_000, i32::MAX];
    for coefficient_x in values {
        for coefficient_y in values {
            for (src_x, src_y) in [(i32::MIN, i32::MIN), (i32::MAX, -7), (12, 40)] {
                for translation in [i32::MIN, -1, 0, i32::MAX] {
                    let wide = i128::from(coefficient_x) * i128::from(src_x)
                        + i128::from(coefficient_y) * i128::from(src_y)
                        + i128::from(translation);
                    let got = project_coordinate(
                        coefficient_x,
                        coefficient_y,
                        translation,
                        src_x,
                        src_y,
                        "test",
                    );
                    assert_eq!(got.ok(), i64::try_from(wide).ok());
                }
            }
        }
    }
}

#[test]
fn masked_tap_rows_match_the_filter_rows_over_every_table_phase() {
    let first = -(WARP_FILTER_CENTER << WARPEDDIFF_PREC_BITS) - 512;
    let last = ((WARPED_FILTERS.len() as i32 - WARP_FILTER_CENTER) << WARPEDDIFF_PREC_BITS) - 513;
    for phase in first..=last {
        let want = warped_filter_row(phase).map(i16::from);
        assert_eq!(WARPED_TAPS[filter_index(phase)], want, "phase {phase}");
    }
}

#[test]
fn identity_warp_flat_reference_returns_flat_block() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let samples = vec![77u16; ref_w * ref_h];
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = default_params(4, 4, ref_w as i32, ref_h as i32);

    let out = crate::math::clip1_predicted_samples(
        warp_predict_block(&view, &params, false).unwrap(),
        i32::from(params.bit_depth.max_sample()),
    );
    assert_eq!(out, vec![77u16; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
}

#[test]
fn compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
    let (ref_w, ref_h) = (16usize, 16usize);
    let samples = vec![77u16; ref_w * ref_h];
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = default_params(4, 4, ref_w as i32, ref_h as i32);

    let out = warp_predict_block(&view, &params, true).unwrap();
    assert_eq!(out, vec![16 * 77i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE]);
}

#[test]
fn compound_intermediate_affine_matches_round7_spec_trace() {
    let (ref_w, ref_h) = (32usize, 32usize);
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
    params.warp_params = [
        1 << WARPEDMODEL_PREC_BITS,
        -(2 << WARPEDMODEL_PREC_BITS),
        (1 << WARPEDMODEL_PREC_BITS) + 256,
        -128,
        192,
        (1 << WARPEDMODEL_PREC_BITS) - 320,
    ];

    let shear = setup_shear(params.warp_params).unwrap();
    let projected = project_section_center(&params).unwrap();
    let mut intermediate = [[0i16; WARPED_BLOCK_SIZE]; WARP_INTERMEDIATE_ROWS];
    let bounds = params.bounds();
    baseline_simd!(simd => {
        build_intermediate(simd, &view, bounds, &shear, &projected, &mut intermediate);
    });
    let mut want = vec![0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
    for i1 in -4i32..4 {
        for i2 in -4i32..4 {
            let sy = projected.sy4 + shear.gamma * i2 + shear.delta * i1;
            let taps = &WARPED_FILTERS[(round2(i64::from(sy), WARPEDDIFF_PREC_BITS)
                + i64::from(WARP_FILTER_CENTER)) as usize];
            let mut sum = 0i64;
            for (i3, &tap) in taps.iter().enumerate() {
                let row = (i1 + i3 as i32 + 4) as usize;
                sum += i64::from(tap) * i64::from(intermediate[row][(i2 + 4) as usize]);
            }
            want[(i1 + 4) as usize * WARPED_BLOCK_SIZE + (i2 + 4) as usize] =
                round2(sum, INTER_ROUND1_COMPOUND) as i32;
        }
    }

    let got = warp_predict_block(&view, &params, true).unwrap();
    assert_eq!(got, want);
}

#[test]
fn fixed_block_output_matches_vec_wrapper() {
    let (ref_w, ref_h) = (32usize, 32usize);
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
    params.warp_params = [
        1 << WARPEDMODEL_PREC_BITS,
        -(2 << WARPEDMODEL_PREC_BITS),
        (1 << WARPEDMODEL_PREC_BITS) + 256,
        -128,
        192,
        (1 << WARPEDMODEL_PREC_BITS) - 320,
    ];

    for is_compound in [false, true] {
        let expected = warp_predict_block(&view, &params, is_compound).unwrap();
        let mut output = [i32::MIN; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
        warp_predict_block_into(&view, &params, is_compound, &mut output).unwrap();
        assert_eq!(output.as_slice(), expected);
    }
}

#[test]
fn ext_compound_intermediate_flat_reference_is_scaled_by_filter_gain() {
    let (ref_w, ref_h) = (32usize, 32usize);
    let samples = vec![77u16; ref_w * ref_h];
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = default_params(8, 12, ref_w as i32, ref_h as i32);
    let out = ext_warp_predict_unit(&view, &params, 0, 0, true).unwrap();
    assert_eq!(out, [16 * 77i32; 16]);
}

#[test]
fn scaled_ext_warp_identity_samples_the_reference_at_scaled_steps() {
    let (ref_w, ref_h) = (16usize, 16usize);
    let samples = (0..ref_h)
        .flat_map(|row| (0..ref_w).map(move |col| (row * ref_w + col) as u16))
        .collect::<Vec<_>>();
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
    params.reference_scale_x = 2 << REF_SCALE_SHIFT;
    params.reference_scale_y = 2 << REF_SCALE_SHIFT;

    let out = ext_warp_predict_unit(&view, &params, 0, 0, false).unwrap();
    assert_eq!(
        out,
        [0, 2, 4, 6, 32, 34, 36, 38, 64, 66, 68, 70, 96, 98, 100, 102]
    );
}

#[test]
fn integer_translation_warp_matches_spec_trace() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(6, 5, ref_w as i32, ref_h as i32);
    params.warp_params[0] = 2 << WARPEDMODEL_PREC_BITS;
    params.warp_params[1] = 3 << WARPEDMODEL_PREC_BITS;

    let out = crate::math::clip1_predicted_samples(
        warp_predict_block(&view, &params, false).unwrap(),
        i32::from(params.bit_depth.max_sample()),
    );
    let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);
}

#[test]
fn affine_warp_nontrivial_case_matches_spec_trace() {
    let ref_w = 32usize;
    let ref_h = 32usize;
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(10, 9, ref_w as i32, ref_h as i32);
    params.warp_params = [
        1 << WARPEDMODEL_PREC_BITS,
        -(2 << WARPEDMODEL_PREC_BITS),
        (1 << WARPEDMODEL_PREC_BITS) + 256,
        -128,
        192,
        (1 << WARPEDMODEL_PREC_BITS) - 320,
    ];

    let out = crate::math::clip1_predicted_samples(
        warp_predict_block(&view, &params, false).unwrap(),
        i32::from(params.bit_depth.max_sample()),
    );
    let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);
    assert_eq!(&out[..8], &[25, 33, 41, 49, 58, 115, 121, 129]);
}

/// The interior 16-bit-lane pass must equal the clamped scalar pass
/// everywhere the clamps are inactive, including at the largest legal
/// 10-bit sample, where reinterpreting `u16` as `i16` would change sign if
/// § 6 Table 6.3 admitted a wider sample. Every section here is centred far
/// enough inside the reference that `interior_warp_source_origin` accepts it.
#[test]
fn interior_intermediate_matches_the_clamped_pass_over_the_ten_bit_range() {
    let (ref_w, ref_h) = (64usize, 64usize);
    let samples = (0..ref_w * ref_h)
        .map(|index| match index % 7 {
            0 => 1023,
            1 => 1022,
            2 => 0,
            _ => ((index * 149) % 1024) as u16,
        })
        .collect::<Vec<u16>>();
    assert!(samples.contains(&1023));
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let samples8 = samples
        .iter()
        .map(|&sample| (sample >> 2) as u8)
        .collect::<Vec<u8>>();
    let view8 = ReferencePlaneView::new(&samples8, ref_w, ref_h).unwrap();

    let models = [
        IDENTITY_WARP_PARAMS,
        [
            0,
            0,
            1 << WARPEDMODEL_PREC_BITS,
            384,
            -256,
            1 << WARPEDMODEL_PREC_BITS,
        ],
        [
            1 << WARPEDMODEL_PREC_BITS,
            -(2 << WARPEDMODEL_PREC_BITS),
            (1 << WARPEDMODEL_PREC_BITS) + 256,
            -128,
            192,
            (1 << WARPEDMODEL_PREC_BITS) - 320,
        ],
    ];
    let mut covered = (false, false);
    for warp_params in models {
        for (block_x, block_y) in [(24, 24), (32, 24), (24, 32)] {
            let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
            params.warp_params = warp_params;
            params.bit_depth = BitDepth::Ten;
            let shear = setup_shear(warp_params).unwrap();
            let projected = project_section_center(&params).unwrap();
            let origin = interior_warp_source_origin(&view, &params, &projected).unwrap();
            if shear.alpha == 0 {
                covered.0 = true;
            } else {
                covered.1 = true;
            }

            let mut fast = [[0i16; WARPED_BLOCK_SIZE]; WARP_INTERMEDIATE_ROWS];
            let mut want = fast;
            let bounds = params.bounds();
            baseline_simd!(simd => {
                build_interior_intermediate(simd, &view, &shear, &projected, origin, &mut fast);
                build_intermediate(simd, &view, bounds, &shear, &projected, &mut want);
            });
            assert_eq!(
                fast, want,
                "model {warp_params:?} at ({block_x}, {block_y})"
            );
            baseline_simd!(simd => {
                build_interior_intermediate(simd, &view8, &shear, &projected, origin, &mut fast);
                build_intermediate(simd, &view8, bounds, &shear, &projected, &mut want);
            });
            assert_eq!(fast, want, "8-bit model {warp_params:?}");
        }
    }
    assert_eq!(covered, (true, true), "both shear column cases exercised");
}

/// [`warp_span`] reads a whole vector past `first_col + WARPED_BLOCK_SIZE`,
/// one sample beyond the taps' own reach, so admission must reserve the
/// column after `last_col`. Without the reservation the admitted section at
/// the final column reads off the end of its row.
#[test]
fn interior_admission_reserves_the_column_after_the_last_tap() {
    let (stride, ref_h) = (64usize, 64usize);
    let samples = vec![512u16; stride * ref_h];
    let params = default_params(24, 24, stride as i32, ref_h as i32);
    let projected = project_section_center(&params).unwrap();
    let last_col = usize::try_from(projected.x4_int + 7).unwrap();

    let tight = ReferencePlaneView::from_strided(&samples, stride, last_col + 1, ref_h).unwrap();
    assert_eq!(
        interior_warp_source_origin(&tight, &params, &projected),
        None,
        "a section whose taps end at the final column must not be admitted"
    );

    let roomy = ReferencePlaneView::from_strided(&samples, stride, last_col + 2, ref_h).unwrap();
    let (first_col, first_row) = interior_warp_source_origin(&roomy, &params, &projected).unwrap();
    assert!(
        first_col + 2 * WARPED_BLOCK_SIZE <= roomy.row(first_row).len(),
        "the admitted origin must leave a full two-vector window in the row"
    );
}

#[test]
fn border_extension_clips_reference_reads() {
    let ref_w = 12usize;
    let ref_h = 12usize;
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let samples_u8 = samples
        .iter()
        .map(|&sample| sample as u8)
        .collect::<Vec<_>>();
    let view_u8 = ReferencePlaneView::new(&samples_u8, ref_w, ref_h).unwrap();
    let unit = 1 << WARPEDMODEL_PREC_BITS;
    let translations = [(-2, -1), (-20, -20), (6, 5), (20, 20)];
    for ([p2, p3, p4, p5], (translate_x, translate_y)) in
        [[unit, 0, 0, unit], [unit + 256, -128, 192, unit - 320]]
            .into_iter()
            .flat_map(|linear| translations.map(|translation| (linear, translation)))
    {
        let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
        params.warp_params = [
            translate_x << WARPEDMODEL_PREC_BITS,
            translate_y << WARPEDMODEL_PREC_BITS,
            p2,
            p3,
            p4,
            p5,
        ];
        let projected = project_section_center(&params).unwrap();
        assert_eq!(
            interior_warp_source_origin(&view, &params, &projected),
            None
        );

        let out = crate::math::clip1_predicted_samples(
            warp_predict_block(&view, &params, false).unwrap(),
            i32::from(params.bit_depth.max_sample()),
        );
        let want = reference_warp_8x8(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want, "translation ({translate_x}, {translate_y})");
        let out_u8 = crate::math::clip1_predicted_samples(
            warp_predict_block(&view_u8, &params, false).unwrap(),
            i32::from(params.bit_depth.max_sample()),
        );
        assert_eq!(
            out_u8, want,
            "u8 translation ({translate_x}, {translate_y})"
        );
    }
}

#[test]
fn rejects_invalid_reference_bounds() {
    let samples = vec![0u16; 64];
    let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
    let mut params = default_params(0, 0, 8, 8);
    params.first_x = 4;
    params.last_x = 3;
    assert!(matches!(
        warp_predict_block(&view, &params, false),
        Err(ReconError::WarpReferenceBoundsInvalid { .. })
    ));
}

#[test]
fn rejects_unsupported_subsampling() {
    let samples = vec![0u16; 64];
    let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
    let mut params = default_params(0, 0, 8, 8);
    params.subsampling_x = 2;
    assert!(matches!(
        warp_predict_block(&view, &params, false),
        Err(ReconError::WarpSubsamplingUnsupported { .. })
    ));
}

#[test]
fn rejects_invalid_shear_without_predicting() {
    let samples = vec![0u16; 64];
    let view = ReferencePlaneView::new(&samples, 8, 8).unwrap();
    let mut params = default_params(0, 0, 8, 8);
    params.warp_params[3] = 1 << WARPEDMODEL_PREC_BITS;

    assert!(matches!(
        warp_predict_block(&view, &params, false),
        Err(ReconError::WarpInvalidShear { .. })
    ));
    let prepare = |params: &WarpPredictBlockParams| {
        PreparedWarpPrediction::with_valid_shear(params).map(|prepared| prepared.is_some())
    };
    assert!(matches!(prepare(&params), Ok(false)));
    params.subsampling_x = 2;
    assert!(matches!(prepare(&params), Ok(false)));
    params.warp_params = IDENTITY_WARP_PARAMS;
    assert!(prepare(&params).is_err());
    params.subsampling_x = 0;
    assert!(matches!(prepare(&params), Ok(true)));
}

#[test]
fn ext_warp_identity_reproduces_the_colocated_unit() {
    let (ref_w, ref_h) = (32usize, 32usize);
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = default_params(8, 12, ref_w as i32, ref_h as i32);
    for (i4, j4) in [(0usize, 0usize), (1, 1)] {
        let out = crate::math::clip1_predicted_samples(
            ext_warp_predict_unit(&view, &params, i4, j4, false)
                .unwrap()
                .into(),
            i32::from(params.bit_depth.max_sample()),
        );
        for r in 0..4 {
            for c in 0..4 {
                let src = samples[(12 + i4 * 4 + r) * ref_w + 8 + j4 * 4 + c];
                assert_eq!(out[r * 4 + c], src, "i4={i4} j4={j4} r={r} c={c}");
            }
        }
    }
}

#[test]
fn ext_warp_integer_translation_shifts_the_source_window() {
    let (ref_w, ref_h) = (32usize, 32usize);
    let samples = build_ref(ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(8, 12, ref_w as i32, ref_h as i32);
    params.warp_params[0] = 3 << WARPEDMODEL_PREC_BITS;
    params.warp_params[1] = 2 << WARPEDMODEL_PREC_BITS;
    let out = crate::math::clip1_predicted_samples(
        ext_warp_predict_unit(&view, &params, 0, 1, false)
            .unwrap()
            .into(),
        i32::from(params.bit_depth.max_sample()),
    );
    for r in 0..4 {
        for c in 0..4 {
            let src = samples[(12 + 2 + r) * ref_w + 8 + 4 + 3 + c];
            assert_eq!(out[r * 4 + c], src, "r={r} c={c}");
        }
    }
}

/// Fractional translations at all four plane edges and inside: every
/// sample is the per-tap § 7.13.3.20 sum of the clamped reference reads.
#[test]
fn ext_warp_unit_matches_the_per_tap_sum_at_edges() {
    let (ref_w, ref_h) = (24usize, 20usize);
    let samples = noise_samples(ref_w * ref_h, 1023, 11);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let fetch = |row: i32, col: i32| {
        let (row, col) = (
            row.clamp(0, ref_h as i32 - 1),
            col.clamp(0, ref_w as i32 - 1),
        );
        i32::from(samples[row as usize * ref_w + col as usize])
    };
    let mut cases = vec![
        (0, 0, -150_000, -90_000, 0usize, 0usize),
        (16, 12, 170_000, 140_000, 1, 1),
        (8, 4, 12_345, 54_321, 0, 1),
        (4, 8, -7_777, 33_000, 1, 0),
    ];
    for block in 0..20i32 {
        let (tx, ty) = (block * 9_973 - 90_000, 70_000 - block * 7_919);
        let (i4, j4) = (block as usize % 2, block as usize / 2 % 2);
        cases.push((block % 5 * 4, block / 5 * 4, tx, ty, i4, j4));
    }
    for (block_x, block_y, tx, ty, i4, j4) in cases {
        let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
        params.bit_depth = BitDepth::Ten;
        params.warp_params[0] = tx;
        params.warp_params[1] = ty;
        let x4 = i64::from(block_x + j4 as i32 * 4 + 2) * 65536 + i64::from(tx);
        let y4 = i64::from(block_y + i4 as i32 * 4 + 2) * 65536 + i64::from(ty);
        let (ix4, iy4) = ((x4 >> 16) as i32, (y4 >> 16) as i32);
        let taps = |s: i64| &EXT_WARPED_FILTERS[round2_i32((s & 0xffff) as i32, 10) as usize];
        let (taps_x, taps_y) = (taps(x4), taps(y4));
        let mut intermediate = [0i32; 36];
        for k in 0..9 {
            for l in 0..4 {
                let sum: i32 = (0..6)
                    .map(|m| taps_x[m] * fetch(iy4 + k as i32 - 4, ix4 + (l + m) as i32 - 4))
                    .sum();
                intermediate[k * 4 + l] = round2_i32(sum, INTER_ROUND0);
            }
        }
        for compound in [false, true] {
            let round1 = if compound { 7 } else { 11 };
            let want: [i32; 16] = core::array::from_fn(|index| {
                let (k, l) = (index / 4, index % 4);
                let sum = (0..6)
                    .map(|m| taps_y[m] * intermediate[(k + m) * 4 + l])
                    .sum();
                round2_i32(sum, round1)
            });
            let got = ext_warp_predict_unit(&view, &params, i4, j4, compound).unwrap();
            assert_eq!(got, want, "{block_x} {block_y} {tx} {ty} {compound}");
        }
    }
}

fn noise_samples(len: usize, max: u16, seed: u32) -> Vec<u16> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            match state >> 29 {
                0 => max,
                1 => 0,
                _ => ((state >> 8) % (u32::from(max) + 1)) as u16,
            }
        })
        .collect()
}

/// Translation `t0` whose plane phase `(t0 >> sub) & 0xFFFF` is `frac`, with
/// the bit the plane shift drops set so `t0` is odd whenever `sub` is 1.
fn translation(integer: i32, frac: i32, sub: u8) -> i32 {
    (((integer << WARPEDMODEL_PREC_BITS) + frac) << sub) | i32::from(sub)
}

#[test]
fn two_tap_phase_matches_the_two_tap_filter_rows() {
    let mut found = [0usize; 2];
    for phase in 0..1 << WARPEDMODEL_PREC_BITS {
        let taps = warped_filter_row(phase);
        let nonzero = taps.iter().filter(|&&tap| tap != 0).count();
        match two_tap_phase(phase) {
            Some(high) => {
                assert_eq!(nonzero, 2, "phase {phase}");
                assert_eq!(taps[TWO_TAP_FIRST + high], 127, "phase {phase}");
                assert_eq!(taps[TWO_TAP_FIRST + 1 - high], 1, "phase {phase}");
                found[high] += 1;
            }
            None => assert!(nonzero > 2, "phase {phase}"),
        }
    }
    assert_eq!(found, [512, 512]);
}

/// Zero-shear sections whose phases select two-tap rows take the reduced
/// passes; every one must still equal the § 7.13.3.19 trace, for both
/// `InterRound1` values, both storage types, interior and clamped windows.
#[test]
fn zero_shear_two_tap_sections_match_the_spec_trace() {
    let (ref_w, ref_h) = (64usize, 64usize);
    let samples = noise_samples(ref_w * ref_h, 1023, 3);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let samples8 = samples
        .iter()
        .map(|&sample| (sample >> 2) as u8)
        .collect::<Vec<u8>>();
    let view8 = ReferencePlaneView::new(&samples8, ref_w, ref_h).unwrap();
    let wide8 = samples8.iter().map(|&s| u16::from(s)).collect::<Vec<u16>>();
    let unit = 1 << WARPEDMODEL_PREC_BITS;
    let models = [
        [
            translation(2, 0, 0),
            translation(-1, 65_535, 0),
            unit,
            0,
            0,
            unit,
        ],
        [
            translation(-3, 400, 0),
            translation(1, 65_100, 0),
            unit,
            0,
            0,
            unit,
        ],
        [
            -31 * 28 + 300,
            translation(0, 65_300, 0),
            unit + 31,
            0,
            0,
            unit - 31,
        ],
        [
            translation(1, 100, 0),
            translation(0, 20_000, 0),
            unit,
            0,
            4096,
            unit,
        ],
        [
            translation(0, 20_000, 0),
            translation(2, 65_400, 0),
            unit,
            2048,
            0,
            unit,
        ],
    ];
    let mut covered = [false; 4];
    for warp_params in models {
        for (block_x, block_y) in [(24, 24), (32, 16), (0, 0), (56, 56)] {
            for bit_depth in [BitDepth::Ten, BitDepth::Eight] {
                let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
                params.warp_params = warp_params;
                params.bit_depth = bit_depth;
                let shear = setup_shear(warp_params).unwrap();
                let projected = project_section_center(&params).unwrap();
                let horizontal =
                    shear.alpha == 0 && shear.beta == 0 && two_tap_phase(projected.sx4).is_some();
                let vertical =
                    shear.gamma == 0 && shear.delta == 0 && two_tap_phase(projected.sy4).is_some();
                let interior = interior_warp_source_origin(&view, &params, &projected);
                covered[usize::from(horizontal) * 2 + usize::from(interior.is_some())] |=
                    horizontal || vertical;
                let trace = if bit_depth == BitDepth::Ten {
                    &samples
                } else {
                    &wide8
                };
                for (is_compound, round1) in [
                    (false, INTER_ROUND1_NON_COMPOUND),
                    (true, INTER_ROUND1_COMPOUND),
                ] {
                    let want = reference_warp_rounded(trace, ref_w, ref_h, &params, round1);
                    let got = if bit_depth == BitDepth::Ten {
                        warp_predict_block(&view, &params, is_compound).unwrap()
                    } else {
                        warp_predict_block(&view8, &params, is_compound).unwrap()
                    };
                    let got = got.into_iter().map(i64::from).collect::<Vec<_>>();
                    assert_eq!(got, want, "{warp_params:?} at ({block_x}, {block_y})");
                }
            }
        }
    }
    assert_eq!(covered, [true; 4], "two-tap sections, interior and clamped");
}

#[test]
fn row_invariant_shear_sections_match_the_spec_trace() {
    let (ref_w, ref_h) = (64usize, 64usize);
    let samples = noise_samples(ref_w * ref_h, 1023, 11);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let samples8 = samples.iter().map(|&s| (s >> 2) as u8).collect::<Vec<u8>>();
    let view8 = ReferencePlaneView::new(&samples8, ref_w, ref_h).unwrap();
    let wide8 = samples8.iter().map(|&s| u16::from(s)).collect::<Vec<u16>>();
    let unit = 1 << WARPEDMODEL_PREC_BITS;
    let models = [
        [
            unit,
            -(2 << WARPEDMODEL_PREC_BITS),
            unit + 256,
            0,
            192,
            unit,
        ],
        [
            translation(1, 300, 0),
            900,
            unit - 640,
            0,
            -1024,
            unit + 128,
        ],
        [translation(-1, 7, 0), 4000, unit, 512, 320, unit],
    ];
    let mut covered = [false; 2];
    for warp_params in models {
        let shear = setup_shear(warp_params).unwrap();
        covered[1] |= shear.gamma != 0 && shear.delta == 0;
        for (block_x, block_y) in [(24, 24), (40, 8), (0, 0), (56, 56)] {
            let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
            params.warp_params = warp_params;
            let projected = project_section_center(&params).unwrap();
            covered[0] |= shear.alpha != 0
                && shear.beta == 0
                && interior_warp_source_origin(&view, &params, &projected).is_some();
            for round1 in [INTER_ROUND1_NON_COMPOUND, INTER_ROUND1_COMPOUND] {
                let is_compound = round1 == INTER_ROUND1_COMPOUND;
                params.bit_depth = BitDepth::Ten;
                let want = reference_warp_rounded(&samples, ref_w, ref_h, &params, round1);
                let got = warp_predict_block(&view, &params, is_compound).unwrap();
                assert!(
                    got.iter().map(|&v| i64::from(v)).eq(want),
                    "{warp_params:?}"
                );
                params.bit_depth = BitDepth::Eight;
                let want = reference_warp_rounded(&wide8, ref_w, ref_h, &params, round1);
                let got = warp_predict_block(&view8, &params, is_compound).unwrap();
                assert!(
                    got.iter().map(|&v| i64::from(v)).eq(want),
                    "{warp_params:?}"
                );
            }
        }
    }
    assert_eq!(covered, [true; 2], "both row-invariant passes exercised");
}

/// Runs the block path and the clipped block predictor over one block and
/// checks both against the per-section § 7.13.3.19 trace of `trace` (the
/// view's samples), returning whether the block path accepted the block. A
/// declined block must be left unwritten.
fn block_matches_sections<T: ReconSample + PartialEq>(
    view: &ReferencePlaneView<'_, T>,
    trace: &[u16],
    params: &WarpPredictBlockParams,
    [x, y, width, height]: [usize; 4],
) -> bool {
    let prepared = PreparedWarpPrediction::new(params).unwrap();
    let max_sample = i32::from(params.bit_depth.max_sample());
    let stride = width + 3;
    let sentinel = T::try_from_u16(77).unwrap();
    let mut want = vec![sentinel; stride * height];
    for local_y in (0..height).step_by(WARPED_BLOCK_SIZE) {
        for local_x in (0..width).step_by(WARPED_BLOCK_SIZE) {
            let section_params = WarpPredictBlockParams {
                block_x: (x + local_x) as i32,
                block_y: (y + local_y) as i32,
                ..*params
            };
            let section = reference_warp_8x8(trace, view.width(), view.height(), &section_params);
            for row in 0..WARPED_BLOCK_SIZE.min(height - local_y) {
                for col in 0..WARPED_BLOCK_SIZE.min(width - local_x) {
                    want[(local_y + row) * stride + local_x + col] =
                        T::try_from_u16(section[row * WARPED_BLOCK_SIZE + col]).unwrap();
                }
            }
        }
    }
    let mut got = vec![sentinel; stride * height];
    let block = [x, y, width, height];
    let max_sample = max_sample as i16;
    let taken = baseline_simd!(simd => prepared
        .translation_two_tap_into(simd, view, block, &mut got, stride, max_sample))
    .unwrap();
    if taken {
        assert!(got == want, "block {params:?} {width}x{height}");
    } else {
        assert!(
            got.iter().all(|&s| s.to_u16() == 77),
            "declined block wrote"
        );
    }
    let mut clipped = vec![sentinel; stride * height];
    prepared
        .predict_clipped_into(view, [x, y, width, height], &mut clipped, stride)
        .unwrap();
    assert!(clipped == want, "clipped {params:?} {width}x{height}");
    taken
}

#[test]
fn translation_two_tap_block_matches_the_section_path() {
    let (ref_w, ref_h) = (176usize, 168usize);
    let samples = noise_samples(ref_w * ref_h, 1023, 7);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let wide8 = noise_samples(ref_w * ref_h, 255, 11);
    let samples8 = wide8
        .iter()
        .map(|&sample| sample as u8)
        .collect::<Vec<u8>>();
    let view8 = ReferencePlaneView::new(&samples8, ref_w, ref_h).unwrap();
    let sizes = [
        (8, 4),
        (8, 128),
        (16, 8),
        (24, 12),
        (64, 32),
        (128, 128),
        (120, 20),
    ];
    for sub in [0u8, 1] {
        for (frac_x, frac_y, two_tap) in [
            (0, 0, true),
            (511, 65_535, true),
            (65_024, 300, true),
            (512, 0, false),
            (0, 65_023, false),
            (32_768, 32_768, false),
        ] {
            for (integer_x, integer_y) in [(-3, 2), (5, -4)] {
                for (width, height) in sizes {
                    let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
                    params.subsampling_x = sub;
                    params.subsampling_y = sub;
                    params.warp_params[0] = translation(integer_x, frac_x, sub);
                    params.warp_params[1] = translation(integer_y, frac_y, sub);
                    for bit_depth in [BitDepth::Ten, BitDepth::Eight] {
                        params.bit_depth = bit_depth;
                        let block = [24, 16, width, height];
                        let taken = if bit_depth == BitDepth::Ten {
                            block_matches_sections(&view, &samples, &params, block)
                        } else {
                            block_matches_sections(&view8, &wide8, &params, block)
                        };
                        assert_eq!(taken, two_tap, "{params:?} {width}x{height}");
                    }
                }
            }
        }
    }
}

#[test]
fn translation_two_tap_block_declines_clipped_and_ragged_blocks() {
    let (ref_w, ref_h) = (64usize, 48usize);
    let samples = noise_samples(ref_w * ref_h, 1023, 5);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
    params.bit_depth = BitDepth::Ten;
    for (integer_x, integer_y, block, taken) in [
        (0, 0, [8, 8, 32, 32], true),
        (-5, 0, [0, 8, 16, 16], false),
        (0, -9, [8, 8, 16, 16], false),
        (0, 0, [32, 8, 32, 16], false),
        (0, 0, [8, 24, 16, 24], false),
        (0, 0, [8, 8, 12, 8], false),
    ] {
        params.warp_params[0] = translation(integer_x, 0, 0);
        params.warp_params[1] = translation(integer_y, 0, 0);
        assert_eq!(
            block_matches_sections(&view, &samples, &params, block),
            taken,
            "{block:?}"
        );
    }
    params.warp_params[0] = 0;
    params.warp_params[1] = 0;
    params.last_x = 38;
    assert!(block_matches_sections(
        &view,
        &samples,
        &params,
        [8, 8, 24, 8]
    ));
    assert!(!block_matches_sections(
        &view,
        &samples,
        &params,
        [8, 8, 32, 8]
    ));
}

/// The byte-lane section kernel against the § 7.13.3.19 trace for models
/// with every shear term nonzero, on noise and on extreme-contrast rows that
/// reach the `pmaddubsw` pair and half bounds.
#[test]
fn eight_bit_sections_with_full_shear_match_the_spec_trace() {
    let (ref_w, ref_h) = (64usize, 64usize);
    let unit = 1 << WARPEDMODEL_PREC_BITS;
    let textures = [
        noise_samples(ref_w * ref_h, 255, 5),
        (0..ref_w * ref_h)
            .map(|index| {
                if (index * 7 + index / ref_w) % 5 < 2 {
                    255
                } else {
                    0
                }
            })
            .collect::<Vec<u16>>(),
    ];
    let models = [
        [
            translation(2, 77, 0),
            translation(-1, 901, 0),
            unit + 700,
            -900,
            1100,
            unit - 600,
        ],
        [
            translation(0, 3, 0),
            translation(1, 512, 0),
            unit - 1500,
            1300,
            -1200,
            unit + 900,
        ],
        [0, 0, unit + 64, 4000, -3000, unit - 64],
        [translation(1, 0, 0), 0, unit, 0, 0, unit + 2000],
    ];
    for samples in &textures {
        let bytes = samples.iter().map(|&s| s as u8).collect::<Vec<u8>>();
        let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
        for warp_params in models {
            for (block_x, block_y) in [(24, 24), (16, 32), (32, 16)] {
                let mut params = default_params(block_x, block_y, ref_w as i32, ref_h as i32);
                params.warp_params = warp_params;
                for round1 in [INTER_ROUND1_NON_COMPOUND, INTER_ROUND1_COMPOUND] {
                    let want = reference_warp_rounded(samples, ref_w, ref_h, &params, round1);
                    let got = warp_predict_block(&view, &params, round1 == INTER_ROUND1_COMPOUND)
                        .unwrap();
                    assert!(
                        got.iter().map(|&v| i64::from(v)).eq(want),
                        "{warp_params:?} at ({block_x}, {block_y}) round {round1}"
                    );
                }
            }
            let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
            params.warp_params = warp_params;
            for block in [[8, 8, 48, 16], [16, 24, 32, 24], [0, 40, 24, 8]] {
                assert!(!block_matches_sections(&view, samples, &params, block));
            }
        }
    }
}

/// The byte-lane section and row kernels against the § 7.13.3.19 trace for
/// random valid models (some with zero `alpha` or `gamma`) on noise with
/// extreme runs, at interior and border blocks, of luma and 4:2:0 chroma.
#[test]
fn eight_bit_kernels_match_the_spec_trace_for_random_models() {
    let (ref_w, ref_h) = (72usize, 56usize);
    let unit = 1 << WARPEDMODEL_PREC_BITS;
    let mut state = 0x2545_f491u32;
    let mut next = |span: i32| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as i32 % (2 * span + 1) - span
    };
    let mut checked = 0;
    for seed in 0..48u32 {
        let samples = noise_samples(ref_w * ref_h, 255, seed);
        let bytes = samples.iter().map(|&s| s as u8).collect::<Vec<u8>>();
        let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
        let mut warp_params = [
            translation(next(3), next(0xffff).abs(), 0),
            translation(next(3), next(0xffff).abs(), 0),
            unit + next(7000),
            next(7000),
            next(7000),
            unit + next(7000),
        ];
        match seed % 4 {
            1 => warp_params[2] = unit,
            2 => warp_params[4] = 0,
            _ => {}
        }
        let mut params = default_params(0, 0, ref_w as i32, ref_h as i32);
        params.warp_params = warp_params;
        params.subsampling_x = u8::from(seed % 3 == 0);
        params.subsampling_y = params.subsampling_x;
        let Ok(prepared) = PreparedWarpPrediction::new(&params) else {
            continue;
        };
        checked += 1;
        let (x, y) = (8 * (seed as usize % 6), 8 * (seed as usize % 5));
        let (width, height) = (24, 16);
        let stride = width + 5;
        let mut clipped = vec![0u8; stride * height];
        prepared
            .predict_clipped_into(&view, [x, y, width, height], &mut clipped, stride)
            .unwrap();
        for local_y in (0..height).step_by(8) {
            for local_x in (0..width).step_by(8) {
                let section = WarpPredictBlockParams {
                    block_x: (x + local_x) as i32,
                    block_y: (y + local_y) as i32,
                    ..params
                };
                let at = (section.block_x, section.block_y);
                let want = reference_warp_8x8(&samples, ref_w, ref_h, &section);
                for (row, want) in want.chunks_exact(8).enumerate() {
                    let start = (local_y + row) * stride + local_x;
                    let got = clipped[start..start + 8].iter().map(|&s| u16::from(s));
                    assert!(got.eq(want.iter().copied()), "{warp_params:?} {at:?}");
                }
                for round1 in [INTER_ROUND1_NON_COMPOUND, INTER_ROUND1_COMPOUND] {
                    let want = reference_warp_rounded(&samples, ref_w, ref_h, &section, round1);
                    let compound = round1 == INTER_ROUND1_COMPOUND;
                    let mut got = [0i32; 64];
                    prepared
                        .predict_block_into(&view, at.0, at.1, compound, &mut got)
                        .unwrap();
                    assert!(
                        got.iter().map(|&v| i64::from(v)).eq(want),
                        "{warp_params:?} {at:?} round {round1}"
                    );
                }
            }
        }
    }
    assert!(checked >= 24, "only {checked} valid models");
}
