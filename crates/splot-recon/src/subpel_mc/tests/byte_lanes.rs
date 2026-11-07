// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The byte-lane eight-bit kernels against the § 7.13.3.18 oracle.

use super::*;

/// Every byte-lane shape, phase class, filter and sink against the § 7.13.3.18
/// oracle, at interior, border-clamped and storage-edge positions, with
/// extreme-contrast rows that reach the `pmaddubsw` pair bounds.
#[test]
fn byte_lane_sinks_match_the_oracle_for_every_shape_and_phase() {
    let (ref_w, ref_h) = (80usize, 56usize);
    let textures: [Vec<u16>; 2] = [
        (0..ref_w * ref_h)
            .map(|i| ((i * 73 + i / ref_w * 29) % 256) as u16)
            .collect(),
        (0..ref_w * ref_h)
            .map(|i| if (i + i / ref_w) % 3 == 0 { 255 } else { 0 })
            .collect(),
    ];
    let interps = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];
    for texture in &textures {
        let bytes = texture.iter().map(|&s| s as u8).collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
        for (w, h) in [
            (4, 4),
            (4, 8),
            (4, 16),
            (8, 4),
            (8, 8),
            (8, 32),
            (16, 4),
            (16, 16),
            (32, 8),
            (48, 2),
        ] {
            for (case, (x, y)) in [(8, 7), (0, 0), (ref_w - w, ref_h - h), (3, 2)]
                .into_iter()
                .enumerate()
            {
                for phase in [0, 1, 7, 8, 15, 16, 17, 100, 136, 255] {
                    let interp = interps[(phase + case) % interps.len()];
                    let params = SubpelPredictParams {
                        interp,
                        start_x: ((x as i32) << 10) + ((phase as i32 % 16) << 6),
                        start_y: ((y as i32) << 10) + ((phase as i32 / 16) << 6),
                        ..full_pel_params(interp, w, h, 0, 0, ref_w as i32, ref_h as i32)
                    };
                    let label = format!("{interp:?} {w}x{h} at {x},{y} phase {phase}");
                    let put = reference_subpel(texture, ref_w, ref_h, &params);
                    let stride = w + 3;
                    let mut actual = vec![0xa5u8; stride * h];
                    subpel_predict_block_strided_into_u8(&view, &params, &mut actual, stride)
                        .unwrap();
                    for row in 0..h {
                        let got = actual[row * stride..][..w].iter().map(|&s| u16::from(s));
                        assert!(got.eq(put[row * w..][..w].iter().copied()), "put {label}");
                    }
                    let prep = reference_subpel_compound(texture, ref_w, ref_h, &params);
                    let mut pred1 = vec![i32::MIN; stride * h];
                    subpel_predict_block_compound_intermediate_into(
                        &view, &params, None, &mut pred1, stride,
                    )
                    .unwrap();
                    for row in 0..h {
                        assert_eq!(
                            pred1[row * stride..][..w],
                            prep[row * w..][..w],
                            "prep {label}"
                        );
                    }
                    let pred0 = prep.iter().rev().copied().collect::<Vec<_>>();
                    for weight in [8i16, 4, 11, 0, 16] {
                        let mut actual = vec![0xa5u8; stride * h];
                        subpel_predict_block_compound_average_strided_into_u8(
                            &view,
                            &params,
                            &pred0,
                            weight,
                            None,
                            &mut actual,
                            stride,
                        )
                        .unwrap();
                        for row in 0..h {
                            for col in 0..w {
                                let index = row * w + col;
                                let want = blend_compound_average_weighted_sample(
                                    pred0[index],
                                    prep[index],
                                    BitDepth::Eight,
                                    weight,
                                );
                                assert_eq!(
                                    u16::from(actual[row * stride + col]),
                                    want,
                                    "average {weight} {label} ({row}, {col})"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The byte search window and its SADs against the general bilinear
/// prediction and a scalar SAD, for every window shape and phase pair.
#[test]
fn byte_search_window_and_sads_match_the_general_path() {
    let (ref_w, ref_h) = (72usize, 64usize);
    let texture: Vec<u8> = (0..ref_w * ref_h)
        .map(|i| ((i * 73 + i / ref_w * 29 + (i * i) % 7) % 256) as u8)
        .collect();
    let view = ReferencePlaneView::new(&texture, ref_w, ref_h).unwrap();
    for (w, h) in [(16, 16), (16, 24), (24, 16), (24, 24)] {
        for (x0, y0) in [(-1i32, -1i32), (5, 9), (ref_w as i32 - w as i32, 30)] {
            for phase in 0..256 {
                let params = SubpelPredictParams {
                    start_x: (x0 << 10) + ((phase % 16) << 6),
                    start_y: (y0 << 10) + ((phase / 16) << 6),
                    first_x: x0 + 1,
                    last_x: x0 + w as i32 - 1,
                    first_y: y0 + 1,
                    last_y: y0 + h as i32 - 1,
                    ..full_pel_params(InterpolationFilter::Bilinear, w, h, 0, 0, 1, 1)
                };
                let mut expected = vec![0u16; w * h];
                subpel_predict_block_strided_into(&view, &params, &mut expected, w).unwrap();
                let mut actual = vec![0u8; w * h];
                assert!(subpel_predict_bilinear_search_window_u8(
                    &view,
                    &params,
                    &mut actual
                ));
                let actual: Vec<u16> = actual.iter().map(|&s| u16::from(s)).collect();
                assert_eq!(actual, expected, "{w}x{h} at {x0},{y0} phase {phase}");
            }
        }
    }
    let windows: [Vec<u8>; 2] = [
        (0..576).map(|i| ((i * 37 + i / 5) % 256) as u8).collect(),
        (0..576).map(|i| ((i * 91 + 13) % 256) as u8).collect(),
    ];
    let offsets: Vec<(i32, i32)> = (-2..=2)
        .flat_map(|dy| (-2..=2).map(move |dx| (dy, dx)))
        .collect();
    for (sad_w, sad_h) in [(12, 12), (12, 20), (20, 12), (20, 20)] {
        let stride = sad_w + 4;
        let mut sads = vec![0; offsets.len()];
        assert!(refinemv_window_sads_u8(
            [&windows[0], &windows[1]],
            stride,
            [sad_w, sad_h],
            &offsets,
            &mut sads
        ));
        for (&(dy, dx), &sad) in offsets.iter().zip(&sads) {
            let at = |sign: i32, row: usize, col: usize| {
                ((2 + sign * dy) as usize + row) * stride + (2 + sign * dx) as usize + col
            };
            let expected: u32 = (0..sad_h)
                .step_by(2)
                .flat_map(|row| (0..sad_w).map(move |col| (row, col)))
                .map(|(row, col)| {
                    u32::from(windows[0][at(1, row, col)].abs_diff(windows[1][at(-1, row, col)]))
                })
                .sum();
            assert_eq!(sad, expected, "{sad_w}x{sad_h} offset {dy},{dx}");
        }
    }
}

/// The byte 8x8 bilinear kernel against the general prediction for every
/// phase pair, and its refusal of a block whose taps would clamp.
#[test]
fn byte_bilinear_8x8_matches_the_general_path() {
    let (ref_w, ref_h) = (40usize, 36usize);
    let texture: Vec<u8> = (0..ref_w * ref_h)
        .map(|i| ((i * 7919 + i / ref_w * 104_729) % 256) as u8)
        .collect();
    let view = ReferencePlaneView::new(&texture, ref_w, ref_h).unwrap();
    let params = |x0: i32, y0: i32, phase: i32| SubpelPredictParams {
        start_x: (x0 << 10) + ((phase % 16) << 6),
        start_y: (y0 << 10) + ((phase / 16) << 6),
        first_x: x0 - 3,
        first_y: y0 - 3,
        last_x: x0 + 11,
        last_y: y0 + 11,
        ..full_pel_params(InterpolationFilter::Bilinear, 8, 8, 0, 0, 1, 1)
    };
    for (x0, y0) in [(0, 0), (9, 4), (31, 26)] {
        for phase in 0..256 {
            let params = params(x0, y0, phase);
            let mut expected = [0u16; 64];
            subpel_predict_block_strided_into(&view, &params, &mut expected, 8).unwrap();
            let mut actual = [0x5a5a; 64];
            assert!(subpel_predict_bilinear_8x8_u8(&view, &params, &mut actual));
            assert_eq!(actual, expected, "{x0},{y0} phase {phase}");
        }
    }
    let mut output = [0; 64];
    assert!(!subpel_predict_bilinear_8x8_u8(
        &view,
        &params(32, 4, 7),
        &mut output
    ));
}
