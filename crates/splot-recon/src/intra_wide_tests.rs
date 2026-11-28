// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use fearless_simd::Level;
use std::{hint::black_box, time::Instant};

const COLORS: [u16; 8] = [3, 1000, 77, 512, 9, 1023, 0, 600];

fn tables(colors: &[u16; 8]) -> ([u8; 16], [u8; 16]) {
    let byte = |shift: u32| -> [u8; 16] {
        core::array::from_fn(|i| colors.get(i).map_or(0, |&color| (color >> shift) as u8))
    };
    (byte(0), byte(8))
}

fn map(len: usize, colors: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 5 + i / 7) % colors) as u8).collect()
}

fn check<P: PaletteSample + Default + Clone + PartialEq + core::fmt::Debug>(wide_samples: bool) {
    let native = Level::new().as_avx512();
    assert!(native.is_some() || std::env::var_os("SPLOT_REQUIRE_AVX512").is_none());
    let (low, high) = tables(&COLORS);
    let high = wide_samples.then_some(high);
    for (len, colors) in [(64, 2), (128, 8), (256, 5), (4096, 8)] {
        let map = map(len, colors);
        let (mut narrow, mut wide) = (vec![P::default(); len], vec![P::default(); len]);
        let (max, wide_max) = baseline_simd!(simd => (
            palette_lanes(simd, low, high, &map, &mut narrow),
            palette_lanes_wide(simd, low, high, &map, &mut wide),
        ));
        assert_eq!((wide_max, &wide), (max, &narrow), "{len} samples");
        assert_eq!(usize::from(max), colors - 1);
        if let Some(simd) = native {
            let mut wide = vec![P::default(); len];
            let palette = P::wide_palette(WideKernels::of(simd));
            assert_eq!(palette(low, high, &map, &mut wide), max);
            assert_eq!(wide, narrow, "native {len} samples");
        }
    }
}

/// The 64-lane lookup writes what the 16-lane lookup writes and returns the
/// same largest index, into bytes and words; on a host with AVX-512 also
/// through the bound kernel pointers.
#[test]
fn wide_palette_lookup_matches_the_16_lane_lookup() {
    check::<u8>(false);
    check::<u16>(true);
}

/// Run on one pinned core of an AVX-512 host. Each sample is one block.
#[cfg(target_arch = "x86_64")]
#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_palette_blocks() {
    const ITERATIONS: u32 = 1 << 16;
    fn bench<P: PaletteSample + Default + Clone>(depth: u8, len: usize) {
        let level = Level::new();
        let avx2 = level.as_avx2().expect("AVX2 required");
        let avx512 = level.as_avx512().expect("AVX-512 required");
        let (low, high) = tables(&COLORS);
        let high = (depth > 8).then_some(high);
        let map = map(len, 8);
        let ns = |run: &dyn Fn(&mut [P]) -> u8| {
            let mut out = vec![P::default(); len];
            let start = Instant::now();
            let mut max = 0;
            for _ in 0..ITERATIONS {
                max |= run(black_box(&mut out));
            }
            black_box((out, max));
            start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS)
        };
        let narrow = || ns(&|out| palette_lanes(avx2, low, high, black_box(&map), out));
        let palette = P::wide_palette(WideKernels::of(avx512));
        let wide = || ns(&|out| palette(low, high, black_box(&map), out));
        let _ = (narrow(), wide());
        for round in 0..12 {
            let (avx2, avx512) = if round % 2 == 0 {
                (narrow(), wide())
            } else {
                let avx512 = wide();
                (narrow(), avx512)
            };
            println!("{depth},{len},{},{avx2:.2},{avx512:.2}", round + 1);
        }
    }
    println!("depth,samples,round,avx2_ns_per_block,avx512_ns_per_block");
    for len in [64, 256, 1024, 4096] {
        bench::<u8>(8, len);
        bench::<u16>(10, len);
    }
}
