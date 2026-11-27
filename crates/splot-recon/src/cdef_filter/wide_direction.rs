// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! [`cdef_direction_avx2`] with its two four-term partial sums merged into
//! one 512-bit register, to time against it on AVX-512.

#![allow(clippy::type_complexity)]

use super::*;
use fearless_simd::Avx512;
use std::{hint::black_box, time::Instant};

fearless_simd::kernel!(
    #[inline(always)]
    fn direction_avx512(_t: Avx512, rows: [i16x8<Avx512>; 8]) -> (usize, i32) {
        use core::arch::x86_64::{
            __m256i, _mm_add_epi16, _mm_cvtsi128_si32, _mm_hadd_epi16, _mm_max_epi32,
            _mm_shuffle_epi32, _mm256_add_epi16, _mm256_add_epi32, _mm256_bslli_epi128,
            _mm256_bsrli_epi128, _mm256_castsi256_ps, _mm256_castsi256_si128, _mm256_cmpeq_epi32,
            _mm256_extracti128_si256, _mm256_hadd_epi16, _mm256_hadd_epi32, _mm256_madd_epi16,
            _mm256_movemask_ps, _mm256_mullo_epi32, _mm256_permutevar8x32_epi32, _mm256_set_m128i,
            _mm256_set1_epi32, _mm256_setr_epi8, _mm256_setr_epi32, _mm256_shuffle_epi8,
            _mm256_unpackhi_epi16, _mm256_unpacklo_epi16,
        };
        use core::arch::x86_64::{
            __m512i, _mm512_add_epi16, _mm512_add_epi32, _mm512_broadcast_i64x4,
            _mm512_bslli_epi128, _mm512_bsrli_epi128, _mm512_castsi256_si512,
            _mm512_castsi512_si256, _mm512_extracti64x4_epi64, _mm512_inserti64x4,
            _mm512_madd_epi16, _mm512_mullo_epi32, _mm512_setzero_si512, _mm512_shuffle_epi8,
            _mm512_unpackhi_epi16, _mm512_unpacklo_epi16,
        };
        let r: [__m256i; 8] =
            core::array::from_fn(|k| _mm256_set_m128i(rows[7 - k].into(), rows[k].into()));
        let add = _mm256_add_epi16;
        let column = add(add(r[0], r[1]), add(r[2], r[3]));
        let column = _mm_add_epi16(
            _mm256_castsi256_si128(column),
            _mm256_extracti128_si256::<1>(column),
        );
        let quads = _mm256_hadd_epi16(_mm256_hadd_epi16(r[0], r[1]), _mm256_hadd_epi16(r[2], r[3]));
        let row = _mm_hadd_epi16(
            _mm256_castsi256_si128(quads),
            _mm256_extracti128_si256::<1>(quads),
        );
        let lines = _mm256_set_m128i(row, column);
        let lines = _mm256_mullo_epi32(_mm256_madd_epi16(lines, lines), _mm256_set1_epi32(105));
        let shifted = |v: &[__m256i]| -> (__m256i, __m256i) {
            let mut low = v[0];
            let mut high = _mm256_set1_epi32(0);
            macro_rules! step {
                ($k:literal) => {
                    if $k < v.len() {
                        low = add(low, _mm256_bslli_epi128::<{ 2 * $k }>(v[$k]));
                        high = add(high, _mm256_bsrli_epi128::<{ 16 - 2 * $k }>(v[$k]));
                    }
                };
            }
            step!(1);
            step!(2);
            step!(3);
            step!(4);
            step!(5);
            step!(6);
            step!(7);
            (low, high)
        };
        let cost = |(low, high): (__m256i, __m256i), reverse: __m256i, weights: [__m256i; 2]| {
            let high = _mm256_shuffle_epi8(high, reverse);
            let first = _mm256_unpacklo_epi16(low, high);
            let second = _mm256_unpackhi_epi16(low, high);
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_madd_epi16(first, first), weights[0]),
                _mm256_mullo_epi32(_mm256_madd_epi16(second, second), weights[1]),
            )
        };
        let z = -1;
        let diagonal = _mm256_setr_epi8(
            12, 13, 10, 11, 8, 9, 6, 7, 4, 5, 2, 3, 0, 1, z, z, 12, 13, 10, 11, 8, 9, 6, 7, 4, 5,
            2, 3, 0, 1, z, z,
        );
        let alternate = _mm256_setr_epi8(
            4, 5, 2, 3, 0, 1, z, z, z, z, z, z, z, z, z, z, 4, 5, 2, 3, 0, 1, z, z, z, z, z, z, z,
            z, z, z,
        );
        let div = |a, b, c, d| _mm256_setr_epi32(a, b, c, d, a, b, c, d);
        let diagonal_weights = [div(840, 420, 280, 210), div(168, 140, 120, 105)];
        let alternate_weights = [div(420, 210, 140, 105), div(105, 105, 105, 105)];
        let diagonals = cost(shifted(&r), diagonal, diagonal_weights);
        let both: [__m512i; 4] = core::array::from_fn(|k| {
            let pairs = _mm256_hadd_epi16(r[k], r[k + 4]);
            let columns = add(r[2 * k], r[2 * k + 1]);
            _mm512_inserti64x4::<1>(_mm512_castsi256_si512(pairs), columns)
        });
        let mut low = both[0];
        let mut high = _mm512_setzero_si512();
        macro_rules! wide_step {
            ($k:literal) => {
                low = _mm512_add_epi16(low, _mm512_bslli_epi128::<{ 2 * $k }>(both[$k]));
                high = _mm512_add_epi16(high, _mm512_bsrli_epi128::<{ 16 - 2 * $k }>(both[$k]));
            };
        }
        wide_step!(1);
        wide_step!(2);
        wide_step!(3);
        let high = _mm512_shuffle_epi8(high, _mm512_broadcast_i64x4(alternate));
        let first = _mm512_unpacklo_epi16(low, high);
        let second = _mm512_unpackhi_epi16(low, high);
        let both_costs = _mm512_add_epi32(
            _mm512_mullo_epi32(
                _mm512_madd_epi16(first, first),
                _mm512_broadcast_i64x4(alternate_weights[0]),
            ),
            _mm512_mullo_epi32(
                _mm512_madd_epi16(second, second),
                _mm512_broadcast_i64x4(alternate_weights[1]),
            ),
        );
        let pair_costs = _mm512_castsi512_si256(both_costs);
        let column_costs = _mm512_extracti64x4_epi64::<1>(both_costs);
        let sums = _mm256_hadd_epi32(
            _mm256_hadd_epi32(diagonals, pair_costs),
            _mm256_hadd_epi32(column_costs, lines),
        );
        let costs = _mm256_permutevar8x32_epi32(sums, _mm256_setr_epi32(0, 1, 7, 5, 4, 6, 3, 2));
        let max = _mm_max_epi32(
            _mm256_castsi256_si128(costs),
            _mm256_extracti128_si256::<1>(costs),
        );
        let max = _mm_max_epi32(max, _mm_shuffle_epi32::<0x4e>(max));
        let max = _mm_max_epi32(max, _mm_shuffle_epi32::<0xb1>(max));
        let max = _mm_cvtsi128_si32(max);
        let ties = _mm256_cmpeq_epi32(costs, _mm256_set1_epi32(max));
        let best = (_mm256_movemask_ps(_mm256_castsi256_ps(ties)) as u32).trailing_zeros() & 7;
        let opposite = _mm256_permutevar8x32_epi32(costs, _mm256_set1_epi32((best ^ 4) as i32));
        let variance = (max - _mm_cvtsi128_si32(_mm256_castsi256_si128(opposite))) >> 10;
        (best as usize, variance)
    }
);

fn blocks() -> Vec<[[i16; 8]; 8]> {
    let mut state = 0x7f4a_7c15_u32;
    (0..64)
        .map(|block| {
            core::array::from_fn(|y| {
                core::array::from_fn(|x| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 24) as i16 >> (block % 5);
                    ((x * (block % 7) + y * (block % 3)) as i16 * 9 + noise).clamp(0, 255) - 128
                })
            })
        })
        .collect()
}

/// On a host with AVX-512, the merged kernel returns what the AVX2 kernel
/// returns.
#[test]
fn wide_direction_matches_the_avx2_kernel() {
    let level = Level::new();
    let (Some(avx2), Some(avx512)) = (level.as_avx2(), level.as_avx512()) else {
        assert!(std::env::var_os("SPLOT_REQUIRE_AVX512").is_none());
        return;
    };
    for block in blocks() {
        let narrow = avx2
            .vectorize(|| cdef_direction_avx2(avx2, block.map(|row| i16x8::load_array(avx2, row))));
        let wide = avx512.vectorize(|| {
            direction_avx512(avx512, block.map(|row| i16x8::load_array(avx512, row)))
        });
        assert_eq!(wide, narrow);
    }
}

#[test]
#[ignore = "native AVX-512 timing"]
fn measure_wide_direction() {
    const ITERATIONS: u32 = 1 << 14;
    let level = Level::new();
    let avx2 = level.as_avx2().expect("AVX2 required");
    let avx512 = level.as_avx512().expect("AVX-512 required");
    let blocks = blocks();
    let ns = |run: &dyn Fn(&[[i16; 8]; 8]) -> (usize, i32)| {
        let start = Instant::now();
        let mut sum = 0usize;
        for _ in 0..ITERATIONS {
            for block in &blocks {
                sum += run(black_box(block)).0;
            }
        }
        black_box(sum);
        start.elapsed().as_secs_f64() * 1.0e9 / f64::from(ITERATIONS) / blocks.len() as f64
    };
    let narrow = || {
        ns(&|block| {
            avx2.vectorize(
                #[inline(always)]
                || cdef_direction_avx2(avx2, block.map(|row| i16x8::load_array(avx2, row))),
            )
        })
    };
    let wide = || {
        ns(&|block| {
            avx512.vectorize(
                #[inline(always)]
                || direction_avx512(avx512, block.map(|row| i16x8::load_array(avx512, row))),
            )
        })
    };
    let _ = (narrow(), wide());
    println!("kernel,case,round,avx2_ns,avx512_ns");
    for round in 0..12 {
        let (a, b) = if round % 2 == 0 {
            (narrow(), wide())
        } else {
            let b = wide();
            (narrow(), b)
        };
        println!("cdef_dir,8x8,{},{a:.2},{b:.2}", round + 1);
    }
}
