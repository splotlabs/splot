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
