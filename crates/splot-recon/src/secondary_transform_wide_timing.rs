// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8x8 secondary-transform sums in 32-lane weights, to time against
//! [`accumulate_kernel_pairs`] on AVX-512.

#![allow(clippy::semicolon_if_nothing_returned, dropping_copy_types)]

use super::*;
use fearless_simd::{Level, i16x32, i32x16};
use std::{hint::black_box, time::Instant};

fn wide_sums<S: Simd>(
    simd: S,
    pairs: &[[i16; 2 * IST_8X8_WIDTH]],
    coefs: &[i32],
    wide: bool,
) -> [i32; IST_8X8_WIDTH] {
    simd.vectorize(
        #[inline(always)]
        || {
            let operand = |a: i32, b: i32| {
                i32x16::splat(simd, (a & 0xffff) | (b << 16)).bitcast::<i16x32<S>>()
            };
            let mut sums = [i32x16::splat(simd, 0); 3];
            for (pair, &[a, b]) in pairs.iter().zip(coefs.as_chunks::<2>().0) {
                if a | b == 0 {
                    continue;
                }
                let (high, low) = if wide {
                    (operand(a >> 8, b >> 8), operand(a & 0xff, b & 0xff))
                } else {
                    (operand(a, b), operand(0, 0))
                };
                for (chunk, sum) in pair.as_chunks::<32>().0.iter().zip(sums.iter_mut()) {
                    let weights = i16x32::load_array_ref(simd, chunk);
                    let mut term = high.pairwise_dot_product(weights);
                    if wide {
                        term = (term << 8) + low.pairwise_dot_product(weights);
                    }
                    *sum += term;
                }
            }
            let mut out = [0i32; IST_8X8_WIDTH];
            for (sum, out) in sums.iter().zip(out.as_chunks_mut::<16>().0) {
                *out = (*sum).into();
            }
            out
        },
    )
}

fn narrow_sums<S: Simd>(
    simd: S,
    pairs: &[[i16; 2 * IST_8X8_WIDTH]],
    coefs: &[i32],
    wide: bool,
) -> [i32; IST_8X8_WIDTH] {
    simd.vectorize(
        #[inline(always)]
        || {
            let mut sums = [i32x8::splat(simd, 0); 6];
            accumulate_kernel_pairs(simd, pairs, coefs, wide, &mut sums);
            let mut out = [0i32; IST_8X8_WIDTH];
            for (sum, out) in sums.iter().zip(out.as_chunks_mut::<8>().0) {
                *out = (*sum).into();
            }
            out
        },
    )
}

fn coefs(count: usize, wide: bool) -> [i32; IST_8X8_HEIGHT] {
    let limit = if wide { 1 << 19 } else { 1 << 14 };
    core::array::from_fn(|i| {
        if i < count {
            ((i as i32 * 7919 + 13) % (2 * limit)) - limit
        } else {
            0
        }
    })
}

#[test]
fn wide_secondary_sums_match_the_16_lane_sums() {
    let pairs = &IST_8X8_PACKED[0][0][0].pairs;
    for wide in [false, true] {
        for count in [2, 8, 32] {
            let coefs = coefs(count, wide);
            baseline_simd!(simd => assert_eq!(
                wide_sums(simd, pairs, &coefs, wide),
                narrow_sums(simd, pairs, &coefs, wide),
                "wide {wide} count {count}"
            ));
        }
    }
}

fn ns(run: impl Fn()) -> f64 {
    const ITERATIONS: u32 = 1 << 18;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        run();
    }
    start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
}

#[cfg(target_arch = "x86_64")]
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_secondary_sums() {
    let level = Level::new();
    let avx2 = level.as_avx2().expect("AVX2 required");
    let avx512 = level.as_avx512().expect("AVX-512 required");
    let pairs = &IST_8X8_PACKED[0][0][0].pairs;
    println!("kernel,case,round,avx2_ns,avx512_ns");
    for wide in [false, true] {
        for count in [2, 8, 32] {
            let coefs = coefs(count, wide);
            let narrow =
                || ns(|| drop(black_box(narrow_sums(avx2, pairs, black_box(&coefs), wide))));
            let wider =
                || ns(|| drop(black_box(wide_sums(avx512, pairs, black_box(&coefs), wide))));
            let _ = (narrow(), wider());
            for round in 0..12 {
                let (a, b) = if round % 2 == 0 {
                    (narrow(), wider())
                } else {
                    let b = wider();
                    (narrow(), b)
                };
                println!("stx8,wide={wide} coefs={count},{},{a:.2},{b:.2}", round + 1);
            }
        }
    }
}
