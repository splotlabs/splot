// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The one-call refine-MV search windows against the § 7.13.3.18 general
//! prediction and a scalar § 7.13.3.6 search.

use super::*;

/// A plane of `width x height` samples at `stride` whose storage ends at its
/// last row's last sample, from an LCG of `seed` masked to `max`.
fn plane(seed: u32, [width, height, stride]: [usize; 3], max: u16) -> Vec<u16> {
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    (0..(height - 1) * stride + width)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 16) as u16 & max
        })
        .collect()
}

/// The window params of a `w x h` window at `[x0, y0]` with phases
/// `[h, v]`, bounded as the decoder bounds a unit four samples in, clamped
/// to the `[width, height]` plane.
fn window_params(
    [w, h]: [usize; 2],
    [x0, y0]: [i32; 2],
    [h_phase, v_phase]: [i32; 2],
    [width, height]: [usize; 2],
    bit_depth: BitDepth,
) -> SubpelPredictParams {
    let clamp = |value: i32, size: usize| value.clamp(0, size as i32 - 1);
    SubpelPredictParams {
        start_x: (x0 << 10) + (h_phase << 6) + 32,
        start_y: (y0 << 10) + (v_phase << 6) + 32,
        first_x: clamp(x0 + 1, width),
        last_x: clamp(x0 + w as i32 - 1, width),
        first_y: clamp(y0 + 1, height),
        last_y: clamp(y0 + h as i32 - 1, height),
        bit_depth,
        ..full_pel_params(InterpolationFilter::Bilinear, w, h, 0, 0, 1, 1)
    }
}

/// Whether the biased centre SAD of two `w x h` windows keeps the centre.
fn centre_exits(windows: [&[u16]; 2], [w, h]: [usize; 2], shift: u32) -> bool {
    let total: u32 = (0..h - 4)
        .step_by(2)
        .flat_map(|row| (0..w - 4).map(move |col| (2 + row) * w + 2 + col))
        .map(|at| u32::from(windows[0][at].abs_diff(windows[1][at])))
        .sum();
    let centre = total >> shift;
    centre - (centre >> 3) < (2 * (w - 4) * (h - 4)) as u32
}

/// The scalar § 7.13.3.6 search of two `w x h` windows at stride `w`.
fn scalar_search(
    windows: [&[u16]; 2],
    [w, h]: [usize; 2],
    allow_center: bool,
    shift: u32,
) -> (i32, i32) {
    let sad = |dx: i32, dy: i32| -> u32 {
        let at = |sign: i32, row: usize, col: usize| {
            ((2 + sign * dy) as usize + row) * w + (2 + sign * dx) as usize + col
        };
        let total: u32 = (0..h - 4)
            .step_by(2)
            .flat_map(|row| (0..w - 4).map(move |col| (row, col)))
            .map(|(row, col)| {
                u32::from(windows[0][at(1, row, col)].abs_diff(windows[1][at(-1, row, col)]))
            })
            .sum();
        total >> shift
    };
    let (mut best, mut best_sad) = ((0, 0), u32::MAX);
    if allow_center {
        let center = sad(0, 0);
        best_sad = center - (center >> 3);
        if best_sad < (2 * (w - 4) * (h - 4)) as u32 {
            return best;
        }
    }
    for &(dy, dx) in &REFINEMV_NEIGHBORS {
        if sad(dx, dy) < best_sad {
            (best, best_sad) = ((dx, dy), sad(dx, dy));
        }
    }
    best
}

/// Runs the one-call search on two planes and checks the offset against the
/// scalar search and the windows against the general prediction: but for
/// the last row, which the scalar search cannot read, or for `u16` windows
/// the centre `(w - 4) x (h - 4)` at `(2, 2)` after a centre exit.
fn check<T: ReconSample, P: WindowSample + Into<u16>>(
    planes: [&[T]; 2],
    [width, height, stride]: [usize; 3],
    params: [SubpelPredictParams; 2],
    bit_depth: BitDepth,
    label: &str,
) {
    let views =
        planes.map(|plane| ReferencePlaneView::from_strided(plane, stride, width, height).unwrap());
    let size = [params[0].w, params[0].h];
    let expected: [Vec<u16>; 2] = [0, 1].map(|i| {
        let mut window = vec![0u16; size[0] * size[1]];
        subpel_predict_block_strided_into(&views[i], &params[i], &mut window, size[0]).unwrap();
        window.truncate(size[0] * (size[1] - 1));
        window
    });
    let shift = u32::from(bit_depth.bits() - 8);
    let pair = [&expected[0][..], &expected[1][..]];
    for allow_center in [false, true] {
        let centre = allow_center && P::words(&[]).is_some() && centre_exits(pair, size, shift);
        let [w, h] = size;
        let inside =
            |at: usize| !centre || (2..h - 2).contains(&(at / w)) && (2..w - 2).contains(&(at % w));
        let mut windows = [[P::default(); REFINEMV_SEARCH_AREA]; 2];
        let offset = refinemv_search_windows(
            [&views[0], &views[1]],
            [&params[0], &params[1]],
            allow_center,
            bit_depth,
            windows.each_mut(),
        );
        for (window, expected) in windows.iter().zip(&expected) {
            let keep = |values: &mut dyn Iterator<Item = u16>| {
                values
                    .enumerate()
                    .filter(|&(at, _)| inside(at))
                    .collect::<Vec<_>>()
            };
            let actual = keep(&mut window[..expected.len()].iter().map(|&s| s.into()));
            let wanted = keep(&mut expected.iter().copied());
            assert_eq!(actual, wanted, "window {label} centre {allow_center}");
        }
        assert_eq!(
            offset,
            Some(scalar_search(pair, size, allow_center, shift)),
            "offset {label} centre {allow_center}"
        );
    }
    let lanes = if T::u8_slice(&[]).is_some() { 16 } else { 8 };
    for (view, (params, expected)) in views.iter().zip(params.iter().zip(&expected)) {
        let plan = refinemv_window::plan(view, params, lanes).unwrap();
        let mut window = [P::default(); REFINEMV_SEARCH_AREA];
        refinemv_window::predict_window(view, &plan, size, &mut window);
        let actual: Vec<u16> = window[..expected.len()].iter().map(|&s| s.into()).collect();
        assert_eq!(&actual, expected, "clamped-path window {label}");
        baseline_simd!(simd => refinemv_window::generic::window(
            simd, view.samples, &plan, size, &mut window,
        ));
        let actual: Vec<u16> = window[..expected.len()].iter().map(|&s| s.into()).collect();
        assert_eq!(&actual, expected, "generic window {label}");
    }
}

/// Every window size, every phase on each axis, and windows inside the
/// plane, clamped at each edge and corner, and wholly outside it, for
/// eight-bit byte and word windows and ten-bit word windows.
#[test]
fn search_windows_match_the_general_prediction_and_search() {
    let geometry = [37, 29, 48];
    let [width, height, _] = geometry;
    let phases: Vec<[i32; 2]> = (0..16)
        .map(|h| [h, (h * 7 + 3) % 16])
        .chain((0..16).map(|v| [(v * 5 + 1) % 16, v]))
        .collect();
    for (bits, max) in [(BitDepth::Eight, 255), (BitDepth::Ten, 1023)] {
        let planes = [plane(3, geometry, max), plane(11, geometry, max)];
        let bytes: [Vec<u8>; 2] = planes
            .each_ref()
            .map(|plane| plane.iter().map(|&s| s as u8).collect());
        for size in [[16, 16], [16, 24], [24, 16], [24, 24]] {
            let ends = |size: usize, extent: usize| {
                let far = extent as i32 - size as i32;
                [
                    -40,
                    -(size as i32),
                    -5,
                    -1,
                    0,
                    3,
                    far - 2,
                    far,
                    far + 2,
                    far + 6,
                    extent as i32 + 9,
                ]
            };
            for (k, x0) in ends(size[0], width).into_iter().enumerate() {
                for (m, y0) in ends(size[1], height).into_iter().enumerate() {
                    let phase = phases[(k * 11 + m * 3) % phases.len()];
                    let moved = [x0 + (k as i32 % 3) - 1, y0 - (m as i32 % 3) + 1];
                    let at = [[x0, y0], moved];
                    let params = [0, 1].map(|i| {
                        let phase = if i == 0 { phase } else { [phase[1], phase[0]] };
                        window_params(size, at[i], phase, [width, height], bits)
                    });
                    let label = format!("{bits:?} {size:?} at {at:?} phase {phase:?}");
                    if bits == BitDepth::Eight {
                        check::<u8, u8>([&bytes[0], &bytes[1]], geometry, params, bits, &label);
                        check::<u8, u16>([&bytes[0], &bytes[1]], geometry, params, bits, &label);
                    }
                    check::<u16, u16>([&planes[0], &planes[1]], geometry, params, bits, &label);
                }
            }
            for phase in &phases {
                let params = [phase, &[phase[1], phase[0]]]
                    .map(|phase| window_params(size, [5, 2], *phase, [width, height], bits));
                let label = format!("{bits:?} {size:?} phase {phase:?}");
                if bits == BitDepth::Eight {
                    check::<u8, u8>([&bytes[0], &bytes[1]], geometry, params, bits, &label);
                }
                check::<u16, u16>([&planes[0], &planes[1]], geometry, params, bits, &label);
            }
        }
    }
}

/// Flat, equal and extreme references: the centre exit, the first
/// neighbour on ties, and SADs of full-range differences.
#[test]
fn search_windows_keep_the_order_of_ties_and_extremes() {
    let geometry = [40, 40, 40];
    for (bits, max) in [(BitDepth::Eight, 255u16), (BitDepth::Ten, 1023)] {
        let flat = vec![max / 3; 40 * 40];
        let full = vec![max; 40 * 40];
        let zero = vec![0; 40 * 40];
        let noise = plane(5, geometry, max);
        for (size, at) in [[16, 16], [16, 24], [24, 16], [24, 24]]
            .into_iter()
            .flat_map(|size| [[6, 7], [-5, 30]].map(|at| (size, at)))
        {
            let params = [0, 1].map(|_| window_params(size, at, [3, 9], [40, 40], bits));
            for (first, second) in [
                (&flat, &flat),
                (&full, &zero),
                (&noise, &noise),
                (&zero, &noise),
            ] {
                check::<u16, u16>([first, second], geometry, params, bits, "extreme");
                if bits == BitDepth::Eight {
                    let bytes = [first, second]
                        .map(|plane| plane.iter().map(|&s| s as u8).collect::<Vec<_>>());
                    check::<u8, u8>([&bytes[0], &bytes[1]], geometry, params, bits, "extreme");
                }
            }
            let views = [&flat, &flat]
                .map(|plane| ReferencePlaneView::from_strided(plane, 40, 40, 40).unwrap());
            for (allow_center, offset) in [(true, (0, 0)), (false, (-2, -2))] {
                let mut windows = [[0u16; REFINEMV_SEARCH_AREA]; 2];
                let found = refinemv_search_windows(
                    [&views[0], &views[1]],
                    [&params[0], &params[1]],
                    allow_center,
                    bits,
                    windows.each_mut(),
                );
                assert_eq!(found, Some(offset));
            }
        }
    }
}

/// Windows outside the contract return `None` without a panic.
#[test]
fn search_windows_reject_windows_outside_the_contract() {
    let geometry = [40, 40, 40];
    let words = plane(7, geometry, 1023);
    let bytes: Vec<u8> = words.iter().map(|&s| s as u8).collect();
    let byte_view = ReferencePlaneView::from_strided(&bytes, 40, 40, 40).unwrap();
    let word_view = ReferencePlaneView::from_strided(&words, 40, 40, 40).unwrap();
    let partial = ReferencePlaneView::from_published_strided(&bytes, 40, 40, 40, 30).unwrap();
    let params = window_params([24, 24], [6, 7], [3, 9], [40, 40], BitDepth::Eight);
    let mut cases = vec![
        SubpelPredictParams {
            step_x: 1100,
            ..params
        },
        SubpelPredictParams { w: 12, ..params },
        SubpelPredictParams { h: 20, ..params },
        SubpelPredictParams {
            interp: InterpolationFilter::EightTap,
            ..params
        },
        SubpelPredictParams {
            last_x: 40,
            ..window_params([24, 24], [20, 7], [3, 9], [40, 40], BitDepth::Eight)
        },
        SubpelPredictParams {
            first_x: -1,
            ..params
        },
        SubpelPredictParams {
            first_y: params.last_y + 1,
            ..params
        },
    ];
    let mut windows = [[0u8; REFINEMV_SEARCH_AREA]; 2];
    let mut words_out = [[0u16; REFINEMV_SEARCH_AREA]; 2];
    for case in cases.drain(..) {
        let search = refinemv_search_windows(
            [&byte_view, &byte_view],
            [&params, &case],
            true,
            BitDepth::Eight,
            windows.each_mut(),
        );
        assert_eq!(search, None, "{case:?}");
        let search = refinemv_search_windows(
            [&word_view, &word_view],
            [&case, &params],
            true,
            BitDepth::Ten,
            words_out.each_mut(),
        );
        assert_eq!(search, None, "{case:?}");
    }
    let low = window_params([24, 24], [6, 10], [3, 9], [40, 40], BitDepth::Eight);
    assert_eq!(
        refinemv_search_windows(
            [&byte_view, &partial],
            [&params, &low],
            true,
            BitDepth::Eight,
            windows.each_mut()
        ),
        None
    );
    assert_eq!(
        refinemv_search_windows(
            [&word_view, &word_view],
            [&params, &params],
            true,
            BitDepth::Ten,
            windows.each_mut()
        ),
        None
    );
    assert_eq!(
        refinemv_search_windows(
            [&byte_view, &byte_view],
            [&params, &params],
            true,
            BitDepth::Ten,
            windows.each_mut()
        ),
        None
    );
    assert!(
        refinemv_search_windows(
            [&byte_view, &byte_view],
            [&params, &params],
            true,
            BitDepth::Eight,
            windows.each_mut()
        )
        .is_some()
    );
}
