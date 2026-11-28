// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::wide_kernels::WideKernels;
use crate::wide_kernels::test_support::{alternate, native_avx512};
use fearless_simd::Level;
use std::{hint::black_box, time::Instant};

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

fn check<O: IntraLaneSample + PartialEq + core::fmt::Debug>(max: u32, extreme: bool) {
    let native = native_avx512();
    for (log2_width, log2_height) in [(5u8, 3u8), (5, 6), (6, 4), (6, 6)] {
        let (width, height) = (1usize << log2_width, 1usize << log2_height);
        let size = IntraRectBlockSize::new(log2_width, log2_height).unwrap();
        let (mut left, mut above, mut top_left) = inputs::<O>(width, max);
        if extreme {
            let edge = |index: usize| {
                O::from_lane(if index.is_multiple_of(3) {
                    max as u16
                } else {
                    0
                })
            };
            left = (0..64).map(edge).collect();
            above = (1..=width).map(edge).collect();
            top_left = max as u16;
        }
        let stride = width + 3;
        let len = stride * (height - 1) + width;
        let (mut narrow, mut wide) = (vec![O::default(); len], vec![O::default(); len]);
        baseline_simd!(simd => {
            paeth_rows(simd, size, &left, &above, top_left, &mut narrow, stride);
            paeth_rows_wide(simd, size, (&left, &above, top_left), &mut wide, stride);
        });
        assert_eq!(wide, narrow, "{width}x{height}");
        if let Some(simd) = native {
            let mut wide = vec![O::default(); len];
            let paeth = O::wide_paeth(WideKernels::of(simd));
            paeth(size, (&left, &above, top_left), &mut wide, stride);
            assert_eq!(wide, narrow, "native {width}x{height}");
        }
    }
}

/// The 32-lane rows write what the 16-lane rows write, into bytes and
/// words, on random and extreme edges; on a host with AVX-512 also through
/// the bound kernel pointers.
#[test]
fn wide_paeth_rows_match_the_16_lane_rows() {
    for extreme in [false, true] {
        check::<u8>(255, extreme);
        check::<u16>(1023, extreme);
    }
}

/// Run on one pinned core of an AVX-512 host. Each sample is one block.
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_paeth_blocks() {
    const ITERATIONS: u32 = 1 << 14;
    fn bench<O: IntraLaneSample>(depth: u8, log2_width: u8, log2_height: u8) {
        let level = Level::new();
        let avx2 = level.as_avx2().expect("AVX2 required");
        let avx512 = level.as_avx512().expect("AVX-512 required");
        let (width, height) = (1usize << log2_width, 1usize << log2_height);
        let size = IntraRectBlockSize::new(log2_width, log2_height).unwrap();
        let (left, above, top_left) = inputs::<O>(width, (1 << depth) - 1);
        let ns = |run: &dyn Fn(&mut [O])| {
            let mut out = vec![O::default(); width * height];
            let start = Instant::now();
            for _ in 0..ITERATIONS {
                run(black_box(&mut out));
            }
            black_box(out);
            start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
        };
        let narrow =
            || ns(&|out| paeth_rows(avx2, size, black_box(&left), &above, top_left, out, width));
        let paeth = O::wide_paeth(WideKernels::of(avx512));
        let wide = || {
            ns(&|out| {
                paeth(
                    size,
                    (black_box(&left[..]), &above[..], top_left),
                    out,
                    width,
                );
            })
        };
        alternate(&format!("{depth},{width}x{height}"), narrow, wide);
    }
    println!("depth,block,round,avx2_ns_per_block,avx512_ns_per_block");
    for (log2_width, log2_height) in [(5, 3), (5, 5), (6, 4), (6, 6)] {
        bench::<u8>(8, log2_width, log2_height);
        bench::<u16>(10, log2_width, log2_height);
    }
}
