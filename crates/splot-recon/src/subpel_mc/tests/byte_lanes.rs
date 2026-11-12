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
                    let mut narrow = vec![i16::MIN; stride * h];
                    subpel_predict_block_compound_intermediate_into(
                        &view,
                        &params,
                        None,
                        &mut narrow,
                        stride,
                    )
                    .unwrap();
                    for row in 0..h {
                        let got = narrow[row * stride..][..w].iter().map(|&v| i32::from(v));
                        assert!(
                            got.eq(prep[row * w..][..w].iter().copied()),
                            "prep16 {label}"
                        );
                    }
                    let pred0 = prep.iter().rev().copied().collect::<Vec<_>>();
                    let pred0_narrow = pred0.iter().map(|&v| v as i16).collect::<Vec<_>>();
                    for weight in [8i16, 4, 11, 0, 16] {
                        let mut narrow = vec![0x5au8; stride * h];
                        subpel_predict_block_compound_average_strided_into_u8(
                            &view,
                            &params,
                            &pred0_narrow,
                            weight,
                            None,
                            &mut narrow,
                            stride,
                        )
                        .unwrap();
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
                                assert_eq!(
                                    u16::from(narrow[row * stride + col]),
                                    want,
                                    "average16 {weight} {label} ({row}, {col})"
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
                let pair = [&windows[0][..], &windows[1][..]];
                assert_eq!(
                    refinemv_search_u8(pair, stride, sad_h, allow_center),
                    Some(best),
                    "{sad_w}x{sad_h} seed {seed} centre {allow_center}"
                );
                let generic = baseline_simd!(simd => search_window::generic::search(
                    simd,
                    pair,
                    [stride, sad_h],
                    allow_center,
                ));
                assert_eq!(generic, best, "generic {sad_w}x{sad_h} seed {seed}");
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

/// The block entries store into a frame what the plane entries store, for
/// both sample depths, every pass shape, and windows inside and across the
/// reference borders.
#[test]
fn block_entries_match_the_plane_entries() {
    block_entries_match::<u8>(BitDepth::Eight);
    block_entries_match::<u16>(BitDepth::Ten);
}

fn block_entries_match<T: ReconSample>(bit_depth: BitDepth) {
    let (frame_w, frame_h) = (40usize, 32usize);
    let max = u32::from(bit_depth.max_sample());
    let plane = |seed: usize, w: usize, h: usize| -> Vec<T> {
        (0..w * h)
            .map(|i| {
                T::try_from_u16(((i * 73 + i / w * 29 + seed * 17) as u32 % (max + 1)) as u16)
                    .unwrap()
            })
            .collect()
    };
    let sizes = [(frame_w, frame_h), (20, 16), (20, 16)];
    let refs: [[Vec<T>; 3]; 2] =
        [0, 1].map(|r| core::array::from_fn(|p| plane(3 * r + p, sizes[p].0, sizes[p].1)));
    let views = refs.each_ref().map(|planes| {
        core::array::from_fn::<_, 3, _>(|p| {
            ReferencePlaneView::new(&planes[p], sizes[p].0, sizes[p].1).unwrap()
        })
    });
    let info = crate::DecodedFrameInfo::new(
        crate::OutputIndex::new(0),
        bit_depth,
        crate::PixelFormat::Yuv420,
        crate::PlaneSize::new(frame_w, frame_h).unwrap(),
        crate::PlaneRect::new(0, 0, frame_w, frame_h).unwrap(),
    )
    .unwrap();
    let interp = InterpolationFilter::EightTapSharp;
    for (x, y, w, h) in [
        (8, 8, 8, 8),
        (16, 8, 16, 8),
        (0, 0, 8, 16),
        (32, 24, 8, 8),
        (4, 12, 4, 4),
    ] {
        for (phase, offset) in [(0, 0), (37, 0), (133, -5), (90, 7), (255, -12)] {
            let start = |p: usize, x: usize, y: usize, extra: i32| {
                let (sub, ph) = (u32::from(p != 0), phase + extra);
                [
                    (((x >> sub) as i32 + offset) << 10) + ((ph % 16) << 6),
                    (((y >> sub) as i32 - offset) << 10) + ((ph / 16 % 16) << 6),
                ]
            };
            let put = |p: usize, r: usize| {
                let sub = u32::from(p != 0);
                UnscaledPut {
                    rect: crate::PlaneRect::new(x >> sub, y >> sub, w >> sub, h >> sub).unwrap(),
                    start: start(p, x, y, 5 * r as i32),
                    last: [sizes[p].0 as i32 - 1, sizes[p].1 as i32 - 1],
                }
            };
            let params = |p: usize, r: usize| {
                let put = put(p, r);
                SubpelPredictParams {
                    start_x: put.start[0],
                    start_y: put.start[1],
                    bit_depth,
                    ..full_pel_params(
                        interp,
                        put.rect.width(),
                        put.rect.height(),
                        0,
                        0,
                        put.last[0] + 1,
                        put.last[1] + 1,
                    )
                }
            };
            for compound in [false, true] {
                let mut workspace =
                    crate::CurrentFrameWorkspace::<T>::new(info, T::default()).unwrap();
                let mut surface = crate::CurrentFrameSurface::Frame(&mut workspace);
                let written = if compound {
                    subpel_compound_block(
                        &mut surface,
                        interp,
                        6,
                        ([&views[0][0], &views[1][0]], [put(0, 0), put(0, 1)]),
                        Some((
                            [[&views[0][1], &views[1][1]], [&views[0][2], &views[1][2]]],
                            [put(1, 0), put(1, 1)],
                        )),
                    )
                } else {
                    subpel_put_block(
                        &mut surface,
                        interp,
                        (&views[0][0], put(0, 0)),
                        Some(([&views[0][1], &views[0][2]], put(1, 0))),
                    )
                };
                assert_eq!(written, [true; 2], "{x},{y} {w}x{h} phase {phase}");
                for (p, plane_id) in [crate::PlaneId::Y, crate::PlaneId::U, crate::PlaneId::V]
                    .into_iter()
                    .enumerate()
                {
                    let rect = put(p, 0).rect;
                    let mut want = vec![T::default(); rect.width() * rect.height()];
                    if compound {
                        assert!(
                            subpel_predict_block_compound_average_fast_validated_strided_into(
                                &views[0][p],
                                &params(p, 0),
                                &views[1][p],
                                &params(p, 1),
                                6,
                                &mut want,
                                rect.width(),
                            )
                            .unwrap()
                        );
                    } else {
                        let mut wide = vec![0u16; want.len()];
                        subpel_predict_block_strided_into(
                            &views[0][p],
                            &params(p, 0),
                            &mut wide,
                            rect.width(),
                        )
                        .unwrap();
                        want = wide
                            .into_iter()
                            .map(|s| T::try_from_u16(s).unwrap())
                            .collect();
                    }
                    let plane = workspace.plane(plane_id).unwrap();
                    for row in 0..rect.height() {
                        for col in 0..rect.width() {
                            assert_eq!(
                                plane
                                    .reconstructed_sample(rect.x() + col, rect.y() + row)
                                    .unwrap()
                                    .to_u16(),
                                want[row * rect.width() + col].to_u16(),
                                "{plane_id:?} {x},{y} {w}x{h} phase {phase} compound {compound}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A program run stores and reports exactly the cells the placement entry
/// stores, for windows inside, across and outside their clip bounds and
/// storage, from references of differing readable rows, with more cells
/// than one setup batch.
#[test]
fn compound_cell_programs_match_the_placement_entry() {
    let (ref_w, ref_h) = (40usize, 32usize);
    let bytes = (0..ref_w * ref_h)
        .map(|i| ((i * 73 + i / ref_w * 29) % 256) as u8)
        .collect::<Vec<_>>();
    let view = ReferencePlaneView::new(&bytes, ref_w, ref_h).unwrap();
    let short =
        ReferencePlaneView::from_published_strided(&bytes, ref_w, ref_w, ref_h, 20).unwrap();
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
                    [&view, &short],
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
                [Some((&[view, short], &mut got[..], stride)), None],
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

/// The byte-lane kernels against the portable kernels on the same blocks:
/// every filter, both passes at phases of each tap-pair count, 4- to
/// 32-wide blocks with clamped rows, column-remapped windows, every
/// eight-bit sink and the fused compound, over noise and extreme contrast;
/// on AArch64 in the multiply-accumulate kernels and, on a CPU with
/// `FEAT_DotProd`, in the `sdot` kernels, and with `FEAT_I8MM` too in the
/// `usdot` kernels.
#[test]
fn byte_lanes_match_the_portable_kernels() {
    #[cfg(target_arch = "aarch64")]
    {
        let dot = std::arch::is_aarch64_feature_detected!("dotprod");
        let i8mm = dot && std::arch::is_aarch64_feature_detected!("i8mm");
        for families in [0, 1, 2] {
            if families == 0 || (families == 1 && dot) || i8mm {
                eight_bit::neon::dot::FAMILIES.with(|cap| cap.set(families));
                check_byte_lanes();
            }
        }
        eight_bit::neon::dot::FAMILIES.with(|cap| cap.set(2));
    }
    #[cfg(not(target_arch = "aarch64"))]
    check_byte_lanes();
}

fn check_byte_lanes() {
    let (stride, rows) = (72usize, 44usize);
    let textures: [Vec<u8>; 2] = [
        (0..stride * rows)
            .map(|i| ((i * 7919 + i / stride * 104_729) % 256) as u8)
            .collect(),
        (0..stride * rows)
            .map(|i| if (i + i / stride) % 3 == 0 { 255 } else { 0 })
            .collect(),
    ];
    for source in &textures {
        for filter in 0..NUM_FILTER_TYPES {
            let phases = [[0, 0], [8, 8], [1, 15], [13, 4]]
                .into_iter()
                .chain((1..16).flat_map(|phase| [[phase, 0], [0, phase], [phase, phase * 7 % 16]]));
            for (case, [x_phase, y_phase]) in phases.enumerate() {
                let case = case % 6;
                let taps = |phase: usize| block::TAPS[filter][phase].as_ref();
                for (w, h) in [
                    (4, 2),
                    (4, 4),
                    (4, 16),
                    (8, 1),
                    (8, 2),
                    (8, 8),
                    (16, 4),
                    (16, 16),
                    (24, 3),
                    (32, 8),
                ] {
                    let block = block::Block {
                        source,
                        stride,
                        origin: 3 + case + case * stride,
                        w,
                        h,
                        horizontal: taps(x_phase),
                        vertical: taps(y_phase),
                    };
                    let label = format!("{filter} {w}x{h} phases {x_phase},{y_phase}");
                    let span = block.vertical.map_or(1, block::Taps::span);
                    let len = block::stage_len(h + span - 1, w, block.vertical);
                    let mut stage = vec![0i16; len];
                    let fits = eight_bit::fits(&block);
                    let run = |sink: &mut block::Sink<'_>, stage: &mut [i16], neon: bool| {
                        if !neon || !eight_bit::predict(&block, sink) {
                            assert!(!neon || !fits, "declined {label}");
                            portable::predict(&block, sink, stage);
                        }
                    };
                    let out_stride = w + 5;
                    let mut prep = [vec![0i32; out_stride * h], vec![0i32; out_stride * h]];
                    let mut narrow = [vec![0i16; out_stride * h], vec![0i16; out_stride * h]];
                    let mut put = [vec![7u8; out_stride * h], vec![7u8; out_stride * h]];
                    let mut wide = [vec![7u16; out_stride * h], vec![7u16; out_stride * h]];
                    for (neon, (((prep, narrow), put), wide)) in (0..2).zip(
                        prep.iter_mut()
                            .zip(&mut narrow)
                            .zip(&mut put)
                            .zip(&mut wide),
                    ) {
                        let neon = neon == 1;
                        let stride = out_stride;
                        run(
                            &mut block::Sink::Prep {
                                output: prep,
                                stride,
                            },
                            &mut stage,
                            neon,
                        );
                        let sink = &mut block::Sink::PrepNarrow {
                            output: narrow,
                            stride,
                        };
                        run(sink, &mut stage, neon);
                        run(
                            &mut block::Sink::Put {
                                output: put,
                                stride,
                            },
                            &mut stage,
                            neon,
                        );
                        let sink = &mut block::Sink::PutWide {
                            output: wide,
                            stride,
                            max: 255,
                        };
                        run(sink, &mut stage, neon);
                    }
                    assert_eq!(prep[0], prep[1], "prep {label}");
                    assert_eq!(narrow[0], narrow[1], "prep16 {label}");
                    assert_eq!(put[0], put[1], "put {label}");
                    assert_eq!(wide[0], wide[1], "put wide {label}");
                    let pred0: Vec<i32> =
                        (0..w * h).map(|i| (i as i32 * 397) % 8200 - 1100).collect();
                    let pred0_narrow: Vec<i16> = pred0.iter().map(|&v| v as i16).collect();
                    let other = block::Block {
                        horizontal: taps(y_phase),
                        vertical: taps(x_phase),
                        origin: block.origin + stride + 1,
                        ..block
                    };
                    for forward in [8, 0, 5, 16] {
                        let mut average = [vec![9u8; out_stride * h], vec![9u8; out_stride * h]];
                        if !fits || !eight_bit::fits(&other) {
                            continue;
                        }
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
                        let mut narrow = vec![9u8; out_stride * h];
                        let sink = &mut block::Sink::AverageNarrow {
                            pred0: &pred0_narrow,
                            forward,
                            output: &mut narrow,
                            stride: out_stride,
                        };
                        run(sink, &mut stage, true);
                        assert_eq!(average[0], narrow, "average16 {forward} {label}");
                        let mut fused = [vec![3u8; out_stride * h], vec![3u8; out_stride * h]];
                        let mut first = vec![0i16; w * h];
                        let sink = &mut block::Sink::PrepNarrow {
                            output: &mut first,
                            stride: w,
                        };
                        portable::predict(&block, sink, &mut stage);
                        let mut other_stage = vec![0i16; 4096];
                        let sink = &mut block::Sink::AverageNarrow {
                            pred0: &first,
                            forward,
                            output: &mut fused[0],
                            stride: out_stride,
                        };
                        portable::predict(&other, sink, &mut other_stage);
                        eight_bit::compound([&block, &other], forward, &mut fused[1], out_stride);
                        assert_eq!(fused[0], fused[1], "compound {forward} {label}");
                    }
                }
            }
        }
    }
}

/// The 16-bit placement entry stores the § 7.13.3.18 oracle's
/// COMPOUND_AVERAGE for every cell it takes, for 10-bit mixed and
/// extreme-contrast samples at sub-pel and full-sample windows inside,
/// across and outside their clip bounds, and a cell program stores the
/// same cells.
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
        let (depth, max) = (BitDepth::Ten, 1023u16);
        let view = ReferencePlaneView::new(texture, ref_w, ref_h).unwrap();
        let forward = [8, 4, 11, 0, 16][case % 5];
        let weights = (forward as i16, depth);
        let stride = 6 * size;
        let mut program = CompoundCells::default();
        let (mut want, mut expected) = (vec![0u16; 4 * size * stride], 0u64);
        for cell in 0..19 {
            let mut placement = || -> CellPlacement {
                let [x, y] = [next(ref_w as i32 + 8) - 4, next(ref_h as i32 + 8) - 4];
                if cell % 3 == 0 {
                    let [x, y] = [x.clamp(0, 24), y.clamp(0, 16)];
                    return (
                        [x << 10, y << 10],
                        [0, 0, ref_w as i32 - 1, ref_h as i32 - 1],
                    );
                }
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
                    bit_depth: depth,
                    ..full_pel_params(interp, size, size, 0, 0, ref_w as i32, ref_h as i32)
                };
                reference_subpel_compound(texture, ref_w, ref_h, &params)
            });
            for (index, (&a, &b)) in first.iter().zip(&second).enumerate() {
                let got = i32::from(output[index / size * stride + index % size]);
                let blended = super::super::block::blend(a, b, forward, i32::from(max));
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
