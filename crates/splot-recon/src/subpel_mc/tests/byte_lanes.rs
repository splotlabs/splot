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
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
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
    let offsets: Vec<(i32, i32)> = (-2..=2)
        .flat_map(|dy| (-2..=2).map(move |dx| (dy, dx)))
        .filter(|&offset| offset != (0, 0))
        .collect();
    for seed in 0..12usize {
        let noise = |i: usize, salt: usize| ((i * 37 + i / 5 + seed * salt) % 256) as u8;
        let windows: [Vec<u8>; 2] = [
            (0..576).map(|i| noise(i, 11)).collect(),
            (0..576)
                .map(|i| {
                    if seed % 3 == 0 {
                        noise(i, 11) / 2 + 3
                    } else {
                        noise(i * 91 + 13, 7)
                    }
                })
                .collect(),
        ];
        for (sad_w, sad_h) in [(12, 12), (12, 20), (20, 12), (20, 20)] {
            let stride = sad_w + 4;
            let sad = |dy: i32, dx: i32| -> u32 {
                let at = |sign: i32, row: usize, col: usize| {
                    ((2 + sign * dy) as usize + row) * stride + (2 + sign * dx) as usize + col
                };
                (0..sad_h)
                    .step_by(2)
                    .flat_map(|row| (0..sad_w).map(move |col| (row, col)))
                    .map(|(row, col)| {
                        u32::from(
                            windows[0][at(1, row, col)].abs_diff(windows[1][at(-1, row, col)]),
                        )
                    })
                    .sum()
            };
            for allow_center in [false, true] {
                let center = sad(0, 0);
                let mut best = (0, 0);
                let mut best_sad = if allow_center {
                    center - (center >> 3)
                } else {
                    u32::MAX
                };
                if !allow_center || best_sad >= (2 * sad_w * sad_h) as u32 {
                    for &(dy, dx) in &offsets {
                        if sad(dy, dx) < best_sad {
                            best_sad = sad(dy, dx);
                            best = (dx, dy);
                        }
                    }
                }
                assert_eq!(
                    refinemv_search_u8([&windows[0], &windows[1]], stride, sad_h, allow_center),
                    Some(best),
                    "{sad_w}x{sad_h} seed {seed} centre {allow_center}"
                );
            }
        }
    }
    assert_eq!(
        refinemv_search_u8([&[0; 576], &[0; 575]], 24, 20, true),
        None
    );
}

/// The byte bilinear kernel against the general prediction for every phase
/// pair and several shapes and strides, and its refusal of a block whose
/// taps would clamp.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn byte_bilinear_blocks_match_the_general_path() {
    let (ref_w, ref_h) = (72usize, 60usize);
    let texture: Vec<u8> = (0..ref_w * ref_h)
        .map(|i| ((i * 7919 + i / ref_w * 104_729) % 256) as u8)
        .collect();
    let view = ReferencePlaneView::new(&texture, ref_w, ref_h).unwrap();
    let params = |x0: i32, y0: i32, phase: i32, [w, h]: [usize; 2]| SubpelPredictParams {
        start_x: (x0 << 10) + ((phase % 16) << 6),
        start_y: (y0 << 10) + ((phase / 16) << 6),
        first_x: x0 - 3,
        first_y: y0 - 3,
        last_x: x0 + w as i32 + 3,
        last_y: y0 + h as i32 + 3,
        ..full_pel_params(InterpolationFilter::Bilinear, w, h, 0, 0, 1, 1)
    };
    for size in [[8, 8], [16, 8], [8, 16], [32, 16], [24, 6]] {
        let stride = size[0] + 8;
        for (x0, y0) in [(0, 0), (9, 4), (ref_w as i32 - 1 - size[0] as i32, 26)] {
            for phase in (0..256).step_by(7) {
                let params = params(x0, y0, phase, size);
                let mut expected = vec![0x5a5au16; stride * size[1]];
                subpel_predict_block_strided_into(&view, &params, &mut expected, stride).unwrap();
                let mut actual = vec![0x5a5au16; stride * size[1]];
                assert!(subpel_predict_bilinear_u8(
                    &view,
                    &params,
                    &mut actual,
                    stride
                ));
                assert_eq!(actual, expected, "{size:?} at {x0},{y0} phase {phase}");
            }
        }
    }
    let mut output = [0; 64];
    let clamped = SubpelPredictParams {
        last_x: 39,
        ..params(32, 4, 7, [8, 8])
    };
    assert!(!subpel_predict_bilinear_u8(&view, &clamped, &mut output, 8));
}

/// 4x4 and 8x8 put and compound cells whose windows cross tighter clip
/// bounds on each side against the § 7.13.3.18 oracle.
#[test]
fn clamped_cells_match_the_oracle() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let texture = (0..ref_w * ref_h)
        .map(|i| ((i * 73 + i / ref_w * 29) % 256) as u16)
        .collect::<Vec<_>>();
    let bytes = texture.iter().map(|&s| s as u8).collect::<Vec<_>>();
    let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
    for size in [4usize, 8] {
        for (dx, dy) in [
            (-3, 0),
            (4, 0),
            (0, -2),
            (0, 5),
            (-1, 3),
            (2, -4),
            (-5, 1),
            (6, -6),
            (1, 7),
        ] {
            for phase in [0, 5, 8, 80, 133, 255] {
                let interp = [
                    InterpolationFilter::EightTap,
                    InterpolationFilter::EightTapSharp,
                ][phase % 2];
                let (x, y) = (12 + dx, 10 + dy);
                let params = SubpelPredictParams {
                    interp,
                    start_x: (x << 10) + ((phase as i32 % 16) << 6),
                    start_y: (y << 10) + ((phase as i32 / 16) << 6),
                    first_x: 12 - 3,
                    first_y: 10 - 3,
                    last_x: 12 + size as i32 + 3,
                    last_y: 10 + size as i32 + 3,
                    ..full_pel_params(interp, size, size, 0, 0, ref_w as i32, ref_h as i32)
                };
                let label = format!("{size} at {dx},{dy} phase {phase}");
                let put = reference_subpel(&texture, ref_w, ref_h, &params);
                let mut actual = vec![0u8; size * size];
                subpel_predict_block_strided_into_u8(&view, &params, &mut actual, size).unwrap();
                assert!(actual.iter().map(|&s| u16::from(s)).eq(put), "put {label}");
                let other = SubpelPredictParams {
                    start_x: params.start_y,
                    start_y: params.start_x,
                    ..params
                };
                let [pred0, pred1] = [&params, &other]
                    .map(|params| reference_subpel_compound(&texture, ref_w, ref_h, params));
                let mut actual = vec![0u8; size * size];
                assert_eq!(
                    subpel_predict_block_compound_average_unscaled_strided_into(
                        &view,
                        &params,
                        &view,
                        &other,
                        8,
                        &mut actual,
                        size,
                    ),
                    Ok(true)
                );
                let want = pred0.iter().zip(&pred1).map(|(&a, &b)| {
                    blend_compound_average_weighted_sample(a, b, BitDepth::Eight, 8)
                });
                assert!(
                    actual.iter().map(|&s| u16::from(s)).eq(want),
                    "average {label}"
                );
            }
        }
    }
}

/// The paired chroma put writes what two single puts write, for cell,
/// staged and copied shapes, and declines planes of differing geometry.
#[test]
fn paired_put_matches_two_single_puts() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let planes = [3usize, 11].map(|seed| {
        (0..ref_w * ref_h)
            .map(|i| ((i * 73 + i / ref_w * 29 + seed * 17) % 256) as u8)
            .collect::<Vec<_>>()
    });
    let views = planes
        .each_ref()
        .map(|plane| ReferencePlaneView::new(plane, ref_w, ref_h).unwrap());
    for (w, h, phase) in [(4, 4, 37), (8, 8, 0), (8, 8, 133), (16, 8, 90), (2, 4, 5)] {
        let interp = InterpolationFilter::EightTap;
        let params = SubpelPredictParams {
            start_x: (9 << 10) + ((phase % 16) << 6),
            start_y: (7 << 10) + ((phase / 16) << 6),
            ..full_pel_params(interp, w, h, 0, 0, ref_w as i32, ref_h as i32)
        };
        let mut want = [vec![0u8; w * h], vec![0u8; w * h]];
        let single = views
            .iter()
            .zip(&mut want)
            .all(|(view, out)| block::put_u8(view, &params, out, w));
        let mut got = [vec![1u8; w * h], vec![1u8; w * h]];
        let [first, second] = &mut got;
        let paired =
            block::put_u8_planes([&views[0], &views[1]], &params, [(first, w), (second, w)]);
        assert_eq!(paired, single, "{w}x{h} phase {phase}");
        if paired {
            assert_eq!(got, want, "{w}x{h} phase {phase}");
        } else {
            assert!(got.iter().flatten().all(|&s| s == 1));
        }
    }
    let narrow = ReferencePlaneView::new(&planes[1][..ref_w * 16], ref_w, 16).unwrap();
    let params = full_pel_params(InterpolationFilter::EightTap, 8, 8, 0, 0, ref_w as i32, 16);
    let mut got = [vec![1u8; 64], vec![1u8; 64]];
    let [first, second] = &mut got;
    assert!(!block::put_u8_planes(
        [&views[0], &narrow],
        &params,
        [(first, 8), (second, 8)]
    ));
    assert!(got.iter().flatten().all(|&s| s == 1));
}

/// A program run stores and reports exactly the cells the placement entry
/// stores, for windows inside, across and outside their clip bounds and
/// storage, with more cells than one setup batch.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn compound_cell_programs_match_the_placement_entry() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let bytes = (0..ref_w * ref_h)
        .map(|i| ((i * 73 + i / ref_w * 29) % 256) as u8)
        .collect::<Vec<_>>();
    let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
    let mut seed = 7u32;
    let mut next = |range: i32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 8) as i32 % range
    };
    for interp in [
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ] {
        for size in [4usize, 8] {
            let stride = 6 * size;
            let mut program = CompoundCells::default();
            let mut placed = Vec::new();
            for cell in 0..19 {
                let mut placement = || -> CellPlacement {
                    let [x, y] = [next(ref_w as i32 + 8) - 4, next(ref_h as i32 + 8) - 4];
                    let [first_x, first_y] = [next(ref_w as i32) - 2, next(ref_h as i32) - 2];
                    (
                        [(x << 10) + (next(16) << 6), (y << 10) + (next(16) << 6)],
                        [
                            first_x,
                            first_y,
                            first_x + next(30) - 3,
                            first_y + next(30) - 3,
                        ],
                    )
                };
                let placements = [placement(), placement()];
                let position = [(cell / 6) * size, (cell % 6) * size];
                assert!(program.push(position, placements));
                placed.push((position, placements));
            }
            let mut want = vec![0u8; 4 * size * stride];
            let mut expected = 0u64;
            for (index, ([row, col], placements)) in placed.iter().enumerate() {
                let output = &mut want[row * stride + col..];
                if subpel_compound_cell(
                    [&view, &view],
                    interp,
                    size,
                    *placements,
                    (11, BitDepth::Eight),
                    output,
                    stride,
                ) {
                    expected |= 1 << index;
                }
            }
            let mut got = vec![0u8; 4 * size * stride];
            let written = subpel_compound_cells(
                [Some((&[view, view], &mut got[..], stride)), None],
                interp,
                size,
                &mut program,
                (11, BitDepth::Eight),
            );
            assert_eq!(written[0], [expected, 0, 0, 0], "{interp:?} {size}");
            assert!(
                expected.count_ones() > 4 && expected.count_ones() < 19,
                "{expected:b}"
            );
            assert_eq!(got, want, "{interp:?} {size}");
        }
    }
    let mut full = CompoundCells::default();
    while full.push([0, 0], [([0; 2], [0; 4]); 2]) {}
    assert_eq!(full.len(), 256);
}

/// The placement entry of a compound cell stores what the validated
/// compound entry stores for the same parameters, inside and across its
/// clip bounds, and declines shapes outside the register cells.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn placed_compound_cells_match_the_compound_entry() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let bytes = (0..ref_w * ref_h)
        .map(|i| ((i * 73 + i / ref_w * 29) % 256) as u8)
        .collect::<Vec<_>>();
    let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
    let interp = InterpolationFilter::EightTapSharp;
    for size in [4usize, 8] {
        for (case, (x, y)) in [(12, 10), (9, 13), (16, 7), (1, 1)].into_iter().enumerate() {
            let placement = |phase: i32| -> CellPlacement {
                (
                    [
                        (x << 10) + ((phase % 16) << 6),
                        (y << 10) + ((phase / 16) << 6),
                    ],
                    [
                        10,
                        9,
                        10 + size as i32 + 4,
                        9 + size as i32 + 3 * case as i32,
                    ],
                )
            };
            let placements = [placement(37), placement(200)];
            let params =
                placements.map(
                    |([start_x, start_y], [fx, fy, lx, ly])| SubpelPredictParams {
                        start_x,
                        start_y,
                        first_x: fx,
                        first_y: fy,
                        last_x: lx,
                        last_y: ly,
                        ..full_pel_params(interp, size, size, 0, 0, ref_w as i32, ref_h as i32)
                    },
                );
            let mut want = vec![0u8; size * size];
            subpel_predict_block_compound_average_unscaled_strided_into(
                &view, &params[0], &view, &params[1], 11, &mut want, size,
            )
            .unwrap();
            let mut got = vec![0u8; size * size];
            assert!(subpel_compound_cell(
                [&view, &view],
                interp,
                size,
                placements,
                (11, BitDepth::Eight),
                &mut got,
                size
            ));
            assert_eq!(got, want, "{size} at {x},{y}");
        }
    }
    let placement = ([8 << 10, 8 << 10], [0, 0, 39, 31]);
    let mut out = vec![0u8; 256];
    assert!(!subpel_compound_cell(
        [&view, &view],
        interp,
        16,
        [placement; 2],
        (8, BitDepth::Eight),
        &mut out,
        16
    ));
    assert!(!subpel_compound_cell(
        [&view, &view],
        interp,
        8,
        [placement; 2],
        (8, BitDepth::Eight),
        &mut out[..60],
        8
    ));
}

/// A full-sample cell, and a wider full-sample block, store the
/// materialized COMPOUND_AVERAGE of their two windows for every weight, at
/// interior and storage-edge positions, and write nothing beside their rows.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn full_sample_cells_match_the_materialized_average() {
    let (ref_w, ref_h) = (24usize, 20usize);
    let textures: [Vec<u8>; 2] = [
        (0..ref_w * ref_h)
            .map(|i| ((i * 17 + 3) % 256) as u8)
            .collect(),
        (0..ref_w * ref_h)
            .map(|i| {
                if (i + i / ref_w) % 3 == 0 {
                    255
                } else {
                    (i % 7) as u8
                }
            })
            .collect(),
    ];
    let views = textures
        .each_ref()
        .map(|bytes| ReferencePlaneView::new(bytes, ref_w, ref_h).unwrap());
    let interp = InterpolationFilter::EightTapSharp;
    for size in [4usize, 8] {
        let last = [(ref_w - size) as i32, (ref_h - size) as i32];
        for (x, y) in [(3, 5), (0, 0), (last[0], last[1]), (last[0] - 1, 2)] {
            let placements: [CellPlacement; 2] =
                [(x, y), (last[0] - x, last[1] - y)].map(|(x, y)| {
                    (
                        [x << 10, y << 10],
                        [0, 0, ref_w as i32 - 1, ref_h as i32 - 1],
                    )
                });
            let params = placements.map(|([start_x, start_y], _)| SubpelPredictParams {
                start_x,
                start_y,
                ..full_pel_params(interp, size, size, 0, 0, ref_w as i32, ref_h as i32)
            });
            let preds = [0, 1].map(|reference| {
                subpel_predict_block_compound_intermediate(&views[reference], &params[reference])
                    .unwrap()
            });
            for weight in [0i16, 8, 11, 16] {
                let want =
                    blend_compound_average_weighted(&preds[0], &preds[1], BitDepth::Eight, weight)
                        .unwrap();
                let stride = size + 3;
                let mut got = vec![u8::MAX; stride * size];
                assert!(subpel_compound_cell(
                    [&views[0], &views[1]],
                    interp,
                    size,
                    placements,
                    (weight, BitDepth::Eight),
                    &mut got,
                    stride,
                ));
                for row in 0..size {
                    let got = &got[row * stride..(row + 1) * stride];
                    let want = want[row * size..(row + 1) * size].iter().map(|&v| v as u8);
                    assert!(
                        got[..size].iter().copied().eq(want),
                        "{size} {x},{y} {weight}"
                    );
                    assert!(got[size..].iter().all(|&v| v == u8::MAX));
                }
            }
        }
    }
    for (w, h) in [(16, 8), (24, 2), (8, 16), (4, 8)] {
        let params = [(3, 5), (ref_w - w, ref_h - h)].map(|(x, y)| SubpelPredictParams {
            start_x: (x as i32) << 10,
            start_y: (y as i32) << 10,
            ..full_pel_params(interp, w, h, 0, 0, ref_w as i32, ref_h as i32)
        });
        let preds = [0, 1].map(|reference| {
            subpel_predict_block_compound_intermediate(&views[reference], &params[reference])
                .unwrap()
        });
        for weight in [-4i16, 0, 8, 11, 16, 20] {
            let want =
                blend_compound_average_weighted(&preds[0], &preds[1], BitDepth::Eight, weight)
                    .unwrap();
            let stride = w + 3;
            let mut got = vec![u8::MAX; stride * h];
            subpel_predict_block_compound_average_unscaled_strided_into(
                &views[0], &params[0], &views[1], &params[1], weight, &mut got, stride,
            )
            .unwrap();
            for row in 0..h {
                let got = &got[row * stride..(row + 1) * stride];
                let want = want[row * w..(row + 1) * w].iter().map(|&v| v as u8);
                assert!(got[..w].iter().copied().eq(want), "{w}x{h} {weight}");
                assert!(got[w..].iter().all(|&v| v == u8::MAX));
            }
        }
    }
}

/// The AArch64 byte-lane kernels against the portable kernels on the same
/// blocks: every filter, both passes at phases of each tap-pair count, 4- to
/// 32-wide blocks with clamped rows, column-remapped windows, every
/// eight-bit sink and the fused compound, over noise and extreme contrast.
#[cfg(target_arch = "aarch64")]
#[test]
fn neon_byte_lanes_match_the_portable_kernels() {
    let (stride, rows) = (72usize, 44usize);
    let textures: [Vec<u8>; 2] = [
        (0..stride * rows)
            .map(|i| ((i * 7919 + i / stride * 104_729) % 256) as u8)
            .collect(),
        (0..stride * rows)
            .map(|i| if (i + i / stride) % 3 == 0 { 255 } else { 0 })
            .collect(),
    ];
    let remap: [u8; 16] = core::array::from_fn(|lane| (lane as u8).saturating_sub(2).min(9));
    for source in &textures {
        for filter in 0..NUM_FILTER_TYPES {
            for (case, [x_phase, y_phase]) in [[0, 0], [7, 0], [0, 9], [1, 15], [8, 8], [13, 4]]
                .into_iter()
                .enumerate()
            {
                let taps = |phase: usize| block::TAPS[filter][phase].as_ref();
                for (w, h) in [(4, 4), (4, 16), (8, 2), (8, 8), (16, 4), (16, 16), (32, 8)] {
                    for remapped in [false, true] {
                        if remapped && (w > 8 || x_phase == 0) {
                            continue;
                        }
                        let block = block::Block {
                            source,
                            stride,
                            origin: 3 + case,
                            top: case as isize - 3,
                            rows: [0, 20 + case as isize],
                            w,
                            h,
                            horizontal: taps(x_phase),
                            vertical: taps(y_phase),
                            remap: remapped.then_some(remap),
                        };
                        let label =
                            format!("{filter} {w}x{h} phases {x_phase},{y_phase} {remapped}");
                        let span = block.vertical.map_or(1, block::Taps::span);
                        let len = block::stage_len(h + span - 1, w, block.vertical);
                        let mut stage = vec![0i16; len];
                        let run = |sink: &mut block::Sink<'_>, stage: &mut [i16], neon: bool| {
                            if neon {
                                assert!(eight_bit::predict(&block, sink, &mut []), "{label}");
                            } else {
                                portable::predict(&block, sink, stage);
                            }
                        };
                        let out_stride = w + 5;
                        let mut prep = [vec![0i32; out_stride * h], vec![0i32; out_stride * h]];
                        let mut put = [vec![7u8; out_stride * h], vec![7u8; out_stride * h]];
                        let mut wide = [vec![7u16; out_stride * h], vec![7u16; out_stride * h]];
                        for (neon, ((prep, put), wide)) in
                            (0..2).zip(prep.iter_mut().zip(&mut put).zip(&mut wide))
                        {
                            let neon = neon == 1;
                            let sink = &mut block::Sink::Prep {
                                output: prep,
                                stride: out_stride,
                            };
                            run(sink, &mut stage, neon);
                            run(
                                &mut block::Sink::Put {
                                    output: put,
                                    stride: out_stride,
                                },
                                &mut stage,
                                neon,
                            );
                            let sink = &mut block::Sink::PutWide {
                                output: wide,
                                stride: out_stride,
                                max: 255,
                            };
                            run(sink, &mut stage, neon);
                        }
                        assert_eq!(prep[0], prep[1], "prep {label}");
                        assert_eq!(put[0], put[1], "put {label}");
                        assert_eq!(wide[0], wide[1], "put wide {label}");
                        let pred0: Vec<i32> =
                            (0..w * h).map(|i| (i as i32 * 397) % 8200 - 1100).collect();
                        for forward in [8, 0, 5, 16] {
                            let mut average =
                                [vec![9u8; out_stride * h], vec![9u8; out_stride * h]];
                            for (neon, output) in (0..2).zip(&mut average) {
                                let sink = &mut block::Sink::Average {
                                    pred0: &pred0,
                                    forward,
                                    output,
                                    stride: out_stride,
                                };
                                run(sink, &mut stage, neon == 1);
                            }
                            assert_eq!(average[0], average[1], "average {forward} {label}");
                            if w * h > 64 {
                                continue;
                            }
                            let other = block::Block {
                                horizontal: taps(y_phase),
                                vertical: taps(x_phase),
                                remap: None,
                                ..block
                            };
                            let mut fused = [vec![3u8; out_stride * h], vec![3u8; out_stride * h]];
                            portable::compound_u8(
                                [&block, &other],
                                forward,
                                &mut vec![0; 4096],
                                &mut fused[0],
                                out_stride,
                            );
                            let mut declined = vec![3u8; out_stride * h];
                            assert!(
                                eight_bit::compound(
                                    [&block, &other],
                                    forward,
                                    &mut [],
                                    &mut declined,
                                    out_stride
                                ),
                                "{label}"
                            );
                            // The cells run every compound in the NEON kernel.
                            let token = fearless_simd::Level::baseline().as_neon().unwrap();
                            eight_bit::neon::compound(
                                token,
                                [&block, &other],
                                forward,
                                &mut fused[1],
                                out_stride,
                            );
                            assert_eq!(fused[0], fused[1], "compound {forward} {label}");
                        }
                    }
                }
            }
        }
    }
    let view = ReferencePlaneView::new(&textures[0], stride, rows).unwrap();
    let params = SubpelPredictParams {
        start_x: (9 << 10) + (5 << 6),
        ..full_pel_params(InterpolationFilter::EightTap, 8, 8, 0, 0, 72, 44)
    };
    assert!(cell::put_cell(&view, &params).is_some());
}

/// The 16-bit placement entry stores the § 7.13.3.18 oracle's
/// COMPOUND_AVERAGE for every cell it takes, for 10-bit mixed and
/// extreme-contrast samples at windows inside, across and outside their
/// clip bounds, and a cell program stores the same cells.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn sixteen_bit_compound_cells_match_the_oracle() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let textures: [Vec<u16>; 2] = [
        (0..ref_w * ref_h)
            .map(|i| ((i * 73 + i / ref_w * 29) % 1024) as u16)
            .collect(),
        (0..ref_w * ref_h)
            .map(|i| if (i + i / ref_w) % 3 == 0 { 1023 } else { 0 })
            .collect(),
    ];
    let mut seed = 11u32;
    let mut next = |range: i32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 8) as i32 % range
    };
    let filters = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];
    let mut taken = 0;
    for (case, (texture, interp, size)) in textures
        .iter()
        .flat_map(|texture| filters.map(|interp| (texture, interp)))
        .flat_map(|(texture, interp)| [4, 8].map(|size| (texture, interp, size)))
        .enumerate()
    {
        let view = ReferencePlaneView::new(texture, ref_w, ref_h).unwrap();
        let forward = [8, 4, 11, 0, 16][case % 5];
        let weights = (forward as i16, BitDepth::Ten);
        let stride = 6 * size;
        let mut program = CompoundCells::default();
        let (mut want, mut expected) = (vec![0u16; 4 * size * stride], 0u64);
        for cell in 0..19 {
            let mut placement = || -> CellPlacement {
                let [x, y] = [next(ref_w as i32 + 8) - 4, next(ref_h as i32 + 8) - 4];
                let [first_x, first_y] = [next(ref_w as i32) - 2, next(ref_h as i32) - 2];
                let [last_x, last_y] = [first_x + next(30) - 3, first_y + next(30) - 3];
                (
                    [(x << 10) + (next(16) << 6), (y << 10) + (next(16) << 6)],
                    [first_x, first_y, last_x, last_y],
                )
            };
            let placements = [placement(), placement()];
            let [row, col] = [(cell / 6) * size, (cell % 6) * size];
            assert!(program.push([row, col], placements));
            let output = &mut want[row * stride + col..];
            if !subpel_compound_cell(
                [&view; 2], interp, size, placements, weights, output, stride,
            ) {
                continue;
            }
            expected |= 1 << cell;
            let [first, second] = placements.map(|([start_x, start_y], [fx, fy, lx, ly])| {
                let params = SubpelPredictParams {
                    start_x,
                    start_y,
                    first_x: fx,
                    first_y: fy,
                    last_x: lx,
                    last_y: ly,
                    bit_depth: BitDepth::Ten,
                    ..full_pel_params(interp, size, size, 0, 0, ref_w as i32, ref_h as i32)
                };
                reference_subpel_compound(texture, ref_w, ref_h, &params)
            });
            for (index, (&a, &b)) in first.iter().zip(&second).enumerate() {
                let got = i32::from(output[index / size * stride + index % size]);
                let blended = super::super::block::blend(a, b, forward, 1023);
                assert_eq!(got, blended, "{interp:?} {size} cell {cell}");
            }
        }
        taken += expected.count_ones();
        let mut got = vec![0u16; 4 * size * stride];
        let written = subpel_compound_cells(
            [Some((&[view, view], &mut got[..], stride)), None],
            interp,
            size,
            &mut program,
            weights,
        );
        assert_eq!(written[0], [expected, 0, 0, 0], "{interp:?} {size}");
        assert_eq!(got, want, "{interp:?} {size}");
    }
    assert!(taken > 60, "{taken}");
}
