// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::wide_kernels::WideKernels;
use fearless_simd::Level;
use std::{hint::black_box, time::Instant};

/// The first `count` of 32 coefficients set, near the `i16` limit or, for
/// `wide`, past it.
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

fn narrow_sums<S: Simd>(
    simd: S,
    pairs: &[[i16; 2 * IST_8X8_WIDTH]],
    coefs: &[i32],
    wide: bool,
) -> [i32; IST_8X8_WIDTH] {
    let mut sums = [i32x8::splat(simd, 0); IST_8X8_WIDTH / 8];
    accumulate_kernel_pairs(simd, pairs, coefs, wide, &mut sums);
    let mut out = [0i32; IST_8X8_WIDTH];
    for (sum, out) in sums.iter().zip(out.as_chunks_mut::<8>().0) {
        sum.store_array(out);
    }
    out
}

/// The 32-lane sums equal the 8-lane sums for 16-bit and wide coefficients
/// and any coefficient count, in every kernel; on a host with AVX-512 also
/// through the bound kernel pointer.
#[test]
fn wide_secondary_sums_match_the_8_lane_sums() {
    let native = Level::new().as_avx512();
    assert!(native.is_some() || std::env::var_os("SPLOT_REQUIRE_AVX512").is_none());
    for kernel in IST_8X8_PACKED.iter().flatten().flatten() {
        for wide in [false, true] {
            for count in [2, 8, 12, 16, 20, 24, 32] {
                let coefs = coefs(count, wide);
                let (narrow, wider) = baseline_simd!(simd => (
                    narrow_sums(simd, &kernel.pairs, &coefs, wide),
                    ist_8x8_sums_wide(simd, &kernel.pairs, &coefs, wide),
                ));
                assert_eq!(wider, narrow, "wide {wide} count {count}");
                if let Some(simd) = native {
                    let sums = WideKernels::of(simd).ist_8x8_sums;
                    assert_eq!(sums(&kernel.pairs, &coefs, wide), narrow, "native");
                }
            }
        }
    }
}

/// Run on one pinned core of an AVX-512 host. Each sample is one 8x8 sum.
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_secondary_sums() {
    const ITERATIONS: u32 = 1 << 18;
    let level = Level::new();
    let avx2 = level.as_avx2().expect("AVX2 required");
    let avx512 = level.as_avx512().expect("AVX-512 required");
    let pairs = &IST_8X8_PACKED[0][0][0].pairs;
    let sums = WideKernels::of(avx512).ist_8x8_sums;
    let ns = |run: &dyn Fn() -> [i32; IST_8X8_WIDTH]| {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(run());
        }
        start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
    };
    println!("inputs,coefficients,round,avx2_ns,avx512_ns");
    for wide in [false, true] {
        for count in [2, 8, 16, 24, 32] {
            let coefs = coefs(count, wide);
            let narrow =
                || ns(&|| avx2.vectorize(|| narrow_sums(avx2, pairs, black_box(&coefs), wide)));
            let wider = || ns(&|| sums(pairs, black_box(&coefs), wide));
            let _ = (narrow(), wider());
            for round in 0..12 {
                let (a, b) = if round % 2 == 0 {
                    (narrow(), wider())
                } else {
                    let b = wider();
                    (narrow(), b)
                };
                let inputs = if wide { "wide" } else { "16-bit" };
                println!("{inputs},{count},{},{a:.2},{b:.2}", round + 1);
            }
        }
    }
}
