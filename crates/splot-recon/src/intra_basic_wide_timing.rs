// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A 32-lane PAETH row walk, to time against [`paeth_rows`] on AVX-512.

#![allow(clippy::semicolon_if_nothing_returned)]

use super::*;
use fearless_simd::{Level, i16x32, u8x64, u16x32};
use std::{hint::black_box, time::Instant};

fn lanes32<S: Simd>(above: i16x32<S>, left: i16x32<S>, top_left: i16x32<S>) -> i16x32<S> {
    let p_left = (above - top_left).abs();
    let p_top = (left - top_left).abs();
    let p_top_left = (above + left - top_left - top_left).abs();
    let take_left = p_left.simd_le(p_top) & p_left.simd_le(p_top_left);
    let take_top = p_top.simd_le(p_top_left);
    take_left.select(left, take_top.select(above, top_left))
}

fn wide16<S: Simd>(simd: S, left: &[u16], above: &[u16], top_left: u16, out: &mut [u16]) {
    simd.vectorize(
        #[inline(always)]
        || {
            let width = above.len();
            let top_left = i16x32::splat(simd, top_left as i16);
            for (row, &left) in out.chunks_exact_mut(width).zip(left) {
                let left = i16x32::splat(simd, left as i16);
                for (out, above) in row
                    .as_chunks_mut::<32>()
                    .0
                    .iter_mut()
                    .zip(above.as_chunks::<32>().0)
                {
                    let above = u16x32::load_array_ref(simd, above).bitcast::<i16x32<S>>();
                    *out = lanes32(above, left, top_left).bitcast::<u16x32<S>>().into();
                }
            }
        },
    );
}

fn wide8<S: Simd>(simd: S, left: &[u8], above: &[u8], top_left: u16, out: &mut [u8]) {
    simd.vectorize(
        #[inline(always)]
        || {
            let width = above.len();
            let top_left = i16x32::splat(simd, top_left as i16);
            for (row, &left) in out.chunks_exact_mut(width).zip(left) {
                let left = i16x32::splat(simd, i16::from(left));
                for (out, above) in row
                    .as_chunks_mut::<64>()
                    .0
                    .iter_mut()
                    .zip(above.as_chunks::<64>().0)
                {
                    let (low, high) = simd.widen_u8x64(u8x64::load_array_ref(simd, above));
                    let half = |above: u16x32<S>| {
                        lanes32(above.bitcast(), left, top_left).bitcast::<u16x32<S>>()
                    };
                    *out = simd.narrow_u16x32(half(low), half(high)).into();
                }
            }
        },
    );
}

fn inputs<T: ReconSample>(width: usize, max: u32) -> (Vec<T>, Vec<T>, u16) {
    let mut state = 0x9e37_79b9_u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        T::try_from_u16(((state >> 8) % (max + 1)) as u16).unwrap()
    };
    let left = (0..64).map(|_| next()).collect();
    let above = (0..width).map(|_| next()).collect();
    (left, above, (max / 3) as u16)
}

#[test]
fn wide_paeth_rows_match_the_16_lane_rows() {
    for log2_width in [5u8, 6] {
        let width = 1usize << log2_width;
        let size = IntraRectBlockSize::new(log2_width, 6).unwrap();
        let (left, above, top_left) = inputs::<u16>(width, 1023);
        let (mut narrow, mut wide) = (vec![0u16; width * 64], vec![0u16; width * 64]);
        baseline_simd!(simd => {
            paeth_rows(simd, size, &left, &above, top_left, &mut narrow, width);
            wide16(simd, &left, &above, top_left, &mut wide);
        });
        assert_eq!(narrow, wide);
        if log2_width == 6 {
            let (left, above, top_left) = inputs::<u8>(width, 255);
            let (mut narrow, mut wide) = (vec![0u8; width * 64], vec![0u8; width * 64]);
            baseline_simd!(simd => {
                paeth_rows(simd, size, &left, &above, top_left, &mut narrow, width);
                wide8(simd, &left, &above, top_left, &mut wide);
            });
            assert_eq!(narrow, wide);
        }
    }
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
        println!("paeth,{label},{},{avx2:.2},{avx512:.2}", round + 1);
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_paeth_blocks() {
    let level = Level::new();
    let avx2 = level.as_avx2().expect("AVX2 required");
    let avx512 = level.as_avx512().expect("AVX-512 required");
    println!("kernel,case,round,avx2_ns,avx512_ns");
    for log2_width in [5u8, 6] {
        let width = 1usize << log2_width;
        let size = IntraRectBlockSize::new(log2_width, 6).unwrap();
        let (left, above, top_left) = inputs::<u16>(width, 1023);
        let out = core::cell::RefCell::new(vec![0u16; width * 64]);
        report(
            &format!("u16 {width}x64"),
            || {
                ns(|| {
                    avx2.vectorize(|| {
                        paeth_rows(
                            avx2,
                            size,
                            black_box(&left),
                            &above,
                            top_left,
                            &mut out.borrow_mut(),
                            width,
                        )
                    })
                })
            },
            || {
                ns(|| {
                    wide16(
                        avx512,
                        black_box(&left),
                        &above,
                        top_left,
                        &mut out.borrow_mut(),
                    )
                })
            },
        );
    }
    let size = IntraRectBlockSize::new(6, 6).unwrap();
    let (left, above, top_left) = inputs::<u8>(64, 255);
    let out = core::cell::RefCell::new(vec![0u8; 64 * 64]);
    report(
        "u8 64x64",
        || {
            ns(|| {
                avx2.vectorize(|| {
                    paeth_rows(
                        avx2,
                        size,
                        black_box(&left),
                        &above,
                        top_left,
                        &mut out.borrow_mut(),
                        64,
                    )
                })
            })
        },
        || {
            ns(|| {
                wide8(
                    avx512,
                    black_box(&left),
                    &above,
                    top_left,
                    &mut out.borrow_mut(),
                )
            })
        },
    );
}
