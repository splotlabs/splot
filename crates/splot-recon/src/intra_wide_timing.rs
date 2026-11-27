// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A 64-lane palette lookup, to time against [`palette_lanes`] on AVX-512.

#![allow(clippy::semicolon_if_nothing_returned, dropping_copy_types)]

use super::*;
use fearless_simd::{Level, SimdCombine as _, u8x64, u16x32};
use std::{hint::black_box, time::Instant};

fn tables<S: Simd>(simd: S, colors: &[u16; 8]) -> [u8x64<S>; 2] {
    [0, 8].map(|shift| {
        let bytes: [u8; 16] =
            core::array::from_fn(|i| colors.get(i).map_or(0, |&c| (c >> shift) as u8));
        let quarter = u8x16::from_slice(simd, &bytes);
        let half = quarter.combine(quarter);
        half.combine(half)
    })
}

fn wide8<S: Simd>(simd: S, colors: &[u16; 8], map: &[u8], out: &mut [u8]) -> u8 {
    simd.vectorize(
        #[inline(always)]
        || {
            let [low, _] = tables(simd, colors);
            let mut max = u8x64::splat(simd, 0);
            for (indices, out) in map
                .as_chunks::<64>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<64>().0)
            {
                let indices = u8x64::load_array_ref(simd, indices);
                max = max.max(indices);
                *out = simd.swizzle_dyn_within_blocks_u8x64(low, indices).into();
            }
            simd.reduce_max_u8x64(max)
        },
    )
}

fn wide16<S: Simd>(simd: S, colors: &[u16; 8], map: &[u8], out: &mut [u16]) -> u8 {
    simd.vectorize(
        #[inline(always)]
        || {
            let [low, high] = tables(simd, colors);
            let mut max = u8x64::splat(simd, 0);
            for (indices, out) in map
                .as_chunks::<64>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<64>().0)
            {
                let indices = u8x64::load_array_ref(simd, indices);
                max = max.max(indices);
                let (low0, low1) =
                    simd.widen_u8x64(simd.swizzle_dyn_within_blocks_u8x64(low, indices));
                let (high0, high1) =
                    simd.widen_u8x64(simd.swizzle_dyn_within_blocks_u8x64(high, indices));
                let (first, second) = out.split_at_mut(32);
                let store = |out: &mut [u16], low: u16x32<S>, high: u16x32<S>| {
                    (low | (high << 8)).store_slice(out);
                };
                store(first, low0, high0);
                store(second, low1, high1);
            }
            simd.reduce_max_u8x64(max)
        },
    )
}

const COLORS: [u16; 8] = [3, 1000, 77, 512, 9, 1023, 0, 600];

fn map() -> Vec<u8> {
    (0..4096).map(|i| ((i * 5 + i / 7) % 8) as u8).collect()
}

#[test]
fn wide_palette_lookup_matches_the_16_lane_lookup() {
    let map = map();
    let (mut narrow, mut wide) = (vec![0u16; 4096], vec![0u16; 4096]);
    let max = predict_intra_palette_into(&COLORS, &map, &mut narrow).unwrap();
    assert_eq!(
        baseline_simd!(simd => wide16(simd, &COLORS, &map, &mut wide)),
        max
    );
    assert_eq!(narrow, wide);
    let colors = COLORS.map(|c| c >> 2);
    let (mut narrow, mut wide) = (vec![0u8; 4096], vec![0u8; 4096]);
    let max = predict_intra_palette_into(&colors, &map, &mut narrow).unwrap();
    assert_eq!(
        baseline_simd!(simd => wide8(simd, &colors, &map, &mut wide)),
        max
    );
    assert_eq!(narrow, wide);
}

fn ns(run: impl Fn()) -> f64 {
    const ITERATIONS: u32 = 1 << 14;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        run();
    }
    start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
}

fn report(label: &str, narrow: impl Fn() -> f64, wide: impl Fn() -> f64) {
    let _ = (narrow(), wide());
    for round in 0..12 {
        let (avx2, avx512) = if round % 2 == 0 {
            (narrow(), wide())
        } else {
            let avx512 = wide();
            (narrow(), avx512)
        };
        println!("palette,{label},{},{avx2:.2},{avx512:.2}", round + 1);
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_palette_blocks() {
    let avx512 = Level::new().as_avx512().expect("AVX-512 required");
    let map = map();
    println!("kernel,case,round,avx2_ns,avx512_ns");
    for len in [256usize, 4096] {
        let map = &map[..len];
        let out = core::cell::RefCell::new(vec![0u16; len]);
        report(
            &format!("u16 {len}"),
            || {
                ns(|| {
                    drop(predict_intra_palette_into(
                        &COLORS,
                        black_box(map),
                        &mut out.borrow_mut()[..],
                    ))
                })
            },
            || {
                ns(|| {
                    drop(wide16(
                        avx512,
                        &COLORS,
                        black_box(map),
                        &mut out.borrow_mut(),
                    ))
                })
            },
        );
        let colors = COLORS.map(|c| c >> 2);
        let out = core::cell::RefCell::new(vec![0u8; len]);
        report(
            &format!("u8 {len}"),
            || {
                ns(|| {
                    drop(predict_intra_palette_into(
                        &colors,
                        black_box(map),
                        &mut out.borrow_mut()[..],
                    ))
                })
            },
            || {
                ns(|| {
                    drop(wide8(
                        avx512,
                        &colors,
                        black_box(map),
                        &mut out.borrow_mut(),
                    ))
                })
            },
        );
    }
}
