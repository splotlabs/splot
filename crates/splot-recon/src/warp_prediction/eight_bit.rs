// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 7.13.3.19 warp of one interior 8x8 section of an eight-bit plane in
//! byte lanes.
//!
//! Every `Warped_Filters` tap is an `i8`, so one `pmaddubsw` weighs two source
//! bytes. Pairing taps `(0, 2)`, `(4, 6)`, `(1, 3)` and `(5, 7)` keeps each
//! pair, and each of the even and odd tap halves, within `±128` (checked
//! below), so a pair sum and a half sum of `u8` samples are exact in `i16`;
//! the two halves then add in `i32`. The vertical pass multiplies `i16`
//! intermediates by taps widened in the high byte of each word, which scales
//! every sum by 256 exactly.

use super::{
    ProjectedCenter, Shear, WARP_FILTER_CENTER, WARP_FILTER_TAPS, WARPED_FILTERS, WARPED_TAPS,
    WARPEDDIFF_PREC_BITS, filter_index,
};
use crate::subpel_mc::ReferencePlaneView;

/// One `Warped_Filters` row as bytes: taps in order (`natural`) and the even
/// taps then the odd taps (`even_odd`).
const fn rows(even_odd: bool) -> [[u8; WARP_FILTER_TAPS]; 512] {
    let mut table = [[0; WARP_FILTER_TAPS]; 512];
    let mut row = 0;
    while row < WARPED_FILTERS.len() {
        let mut tap = 0;
        while tap < WARP_FILTER_TAPS {
            let source = if even_odd {
                (tap % 4) * 2 + tap / 4
            } else {
                tap
            };
            table[row][tap] = WARPED_FILTERS[row][source] as u8;
            tap += 1;
        }
        row += 1;
    }
    table
}

static EVEN_ODD: [[u8; WARP_FILTER_TAPS]; 512] = rows(true);
static NATURAL: [[u8; WARP_FILTER_TAPS]; 512] = rows(false);

/// Every two-tap pair and every four-tap half of the byte-lane pass stays
/// within `±128`, so `255 * sum` fits `i16`.
const _: () = {
    let mut row = 0;
    while row < WARPED_TAPS.len() {
        let taps = &WARPED_TAPS[row];
        let mut group = 0;
        while group < 6 {
            let members: [usize; 4] = match group {
                0 => [0, 2, 8, 8],
                1 => [4, 6, 8, 8],
                2 => [1, 3, 8, 8],
                3 => [5, 7, 8, 8],
                4 => [0, 2, 4, 6],
                _ => [1, 3, 5, 7],
            };
            let (mut positive, mut negative) = (0, 0);
            let mut member = 0;
            while member < 4 {
                if members[member] < 8 {
                    let tap = taps[members[member]];
                    if tap > 0 {
                        positive += tap;
                    } else {
                        negative += tap;
                    }
                }
                member += 1;
            }
            assert!(positive <= 128 && negative >= -128);
            group += 1;
        }
        row += 1;
    }
};

/// The 15 source rows of 16 bytes of an interior section from `column`.
struct Window<'a> {
    rows: &'a [u8],
    column: usize,
    stride: usize,
}

impl<'a> Window<'a> {
    fn new(
        reference: &ReferencePlaneView<'a, u8>,
        (first_col, first_row): (usize, usize),
    ) -> Option<Self> {
        let (rows, stride) = reference.rows_from(first_row)?;
        (first_col + 14 * stride + 16 <= rows.len()).then_some(Self {
            rows,
            column: first_col,
            stride,
        })
    }
}

/// The `Round2(sum, round1)` rows of the section whose source window starts
/// at `(first_col, first_row)`, or `None` without AVX2.
pub(super) fn section(
    reference: &ReferencePlaneView<'_, u8>,
    shear: &Shear,
    projected: &ProjectedCenter,
    origin: (usize, usize),
    round1: u32,
) -> Option<[[i32; 8]; 8]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return Some(avx2::section(
            token,
            &Window::new(reference, origin)?,
            [shear.alpha, shear.beta, shear.gamma, shear.delta],
            [projected.sx4, projected.sy4],
            round1,
        ));
    }
    let _ = (reference, shear, projected, origin, round1);
    None
}

/// Stores the first `width` columns of the first `height` rows of the
/// single-reference prediction of a section into `output` rows `stride`
/// apart, after the final `Clip1`; `false` without writing when the source
/// window leaves the storage or without AVX2.
pub(super) fn put(
    reference: &ReferencePlaneView<'_, u8>,
    shear: &Shear,
    projected: &ProjectedCenter,
    origin: (usize, usize),
    output: &mut [u8],
    (stride, width, height): (usize, usize, usize),
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2()
        && let Some(window) = Window::new(reference, origin)
    {
        avx2::put(
            token,
            &window,
            [shear.alpha, shear.beta, shear.gamma, shear.delta],
            [projected.sx4, projected.sy4],
            output,
            [stride, width, height],
        );
        return true;
    }
    let _ = (
        reference, shear, projected, origin, output, stride, width, height,
    );
    false
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{
        EVEN_ODD, NATURAL, WARP_FILTER_CENTER, WARPED_TAPS, WARPEDDIFF_PREC_BITS, Window,
        filter_index,
    };
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i32x8, u8x16, u8x32};

    /// Source bytes of columns `0, 1 | 4, 5` (`A`) and `2, 3 | 6, 7` (`B`)
    /// in even-odd tap order.
    const fn shuffle(first: usize) -> [u8; 32] {
        let mut out = [0; 32];
        let mut lane = 0;
        while lane < 32 {
            let column = first + (lane / 16) * 4 + (lane % 16) / 8;
            let tap = lane % 8;
            out[lane] = (column + (tap % 4) * 2 + tap / 4) as u8;
            lane += 1;
        }
        out
    }

    const SHUFFLE_A: [u8; 32] = shuffle(0);
    const SHUFFLE_B: [u8; 32] = shuffle(2);

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn section(
            t: Avx2,
            window: &Window<'_>,
            shear: [i32; 4],
            phases: [i32; 2],
            round1: u32,
        ) -> [[i32; 8]; 8] {
            let shift = _mm_cvtsi32_si128(round1 as i32 + 8);
            let mut out = [[0i32; 8]; 8];
            for (out, sum) in out.iter_mut().zip(sums(t, window, shear, phases, round1)) {
                i32x8::<Avx2>::simd_from(t, _mm256_sra_epi32(sum, shift)).store_array(out);
            }
            out
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(
            t: Avx2,
            window: &Window<'_>,
            shear: [i32; 4],
            phases: [i32; 2],
            output: &mut [u8],
            size: [usize; 3],
        ) {
            let [stride, width, height] = size;
            let rows = sums(
                t,
                window,
                shear,
                phases,
                super::super::INTER_ROUND1_NON_COMPOUND,
            );
            let shift = _mm_cvtsi32_si128(super::super::INTER_ROUND1_NON_COMPOUND as i32 + 8);
            for (pair, rows) in rows.chunks_exact(2).enumerate() {
                let words = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packs_epi32(
                    _mm256_sra_epi32(rows[0], shift),
                    _mm256_sra_epi32(rows[1], shift),
                ));
                let bytes = _mm256_packus_epi16(words, words);
                let lines = [
                    _mm_cvtsi128_si64(_mm256_castsi256_si128(bytes)),
                    _mm_cvtsi128_si64(_mm256_extracti128_si256::<1>(bytes)),
                ];
                for (row, line) in (2 * pair..height).zip(lines) {
                    let line = line.to_le_bytes();
                    let Some(slot) = output.get_mut(row * stride..) else {
                        continue;
                    };
                    if let Some(slot) = slot.first_chunk_mut::<8>().filter(|_| width == 8) {
                        *slot = line;
                    } else if let Some(slot) = slot.get_mut(..width) {
                        for (slot, &byte) in slot.iter_mut().zip(&line) {
                            *slot = byte;
                        }
                    }
                }
            }
        }
    );

    /// The filters of output columns `0, 1 | 4, 5` and `2, 3 | 6, 7` at
    /// phases `phase + steps`, `steps` in that column order: the indices of
    /// `filter_index` computed in lanes.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn filters(table: &[[u8; 8]; 512], phase: i32, steps: __m256i) -> (__m256i, __m256i) {
        const BIAS: i32 =
            (1 << (WARPEDDIFF_PREC_BITS - 1)) + (WARP_FILTER_CENTER << WARPEDDIFF_PREC_BITS);
        let index = _mm256_srli_epi32::<{ WARPEDDIFF_PREC_BITS as i32 }>(_mm256_add_epi32(
            _mm256_set1_epi32(phase.wrapping_add(BIAS)),
            steps,
        ));
        let tap = |index: i32| i64::from_le_bytes(table[index as usize & (WARPED_TAPS.len() - 1)]);
        let taps = |lanes: __m128i| {
            _mm256_set_epi64x(
                tap(_mm_extract_epi32::<3>(lanes)),
                tap(_mm_extract_epi32::<2>(lanes)),
                tap(_mm_extract_epi32::<1>(lanes)),
                tap(_mm_cvtsi128_si32(lanes)),
            )
        };
        (
            taps(_mm256_castsi256_si128(index)),
            taps(_mm256_extracti128_si256::<1>(index)),
        )
    }

    /// The vertical-pass sums of the eight output rows of a section, each
    /// scaled by 256 with the `Round2(.., round1)` bias added.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn sums(
        t: Avx2,
        window: &Window<'_>,
        shear: [i32; 4],
        phases: [i32; 2],
        round1: u32,
    ) -> [__m256i; 8] {
        {
            let ([alpha, beta, gamma, delta], [sx4, sy4]) = (shear, phases);
            let filter = |table: &[[u8; 8]; 512], phase: i32| -> i64 {
                i64::from_le_bytes(table[filter_index(phase)])
            };
            let shuffle_a: __m256i = u8x32::load_array(t, SHUFFLE_A).into();
            let shuffle_b: __m256i = u8x32::load_array(t, SHUFFLE_B).into();
            let ones = _mm256_set1_epi16(1);
            let columns = _mm256_setr_epi32(0, 1, 4, 5, 2, 3, 6, 7);
            let (alpha_steps, gamma_steps) = (
                _mm256_mullo_epi32(_mm256_set1_epi32(alpha), columns),
                _mm256_mullo_epi32(_mm256_set1_epi32(gamma), columns),
            );
            let bias = _mm256_set1_epi32(1 << 2);
            let mut rows = [_mm256_setzero_si256(); 15];
            for (row, out) in rows.iter_mut().enumerate() {
                let Some(window) = window
                    .rows
                    .get(window.column + row * window.stride..)
                    .and_then(<[u8]>::first_chunk::<16>)
                else {
                    continue;
                };
                let phase = sx4 + beta * (row as i32 - 7) - 4 * alpha;
                let (taps_a, taps_b) = if alpha == 0 {
                    let taps = _mm256_set1_epi64x(filter(&EVEN_ODD, phase));
                    (taps, taps)
                } else {
                    filters(&EVEN_ODD, phase, alpha_steps)
                };
                let source: __m128i = u8x16::load_array_ref(t, window).into();
                let source = _mm256_broadcastsi128_si256(source);
                let a = _mm256_maddubs_epi16(_mm256_shuffle_epi8(source, shuffle_a), taps_a);
                let b = _mm256_maddubs_epi16(_mm256_shuffle_epi8(source, shuffle_b), taps_b);
                let sums = _mm256_madd_epi16(_mm256_hadd_epi16(a, b), ones);
                *out = _mm256_srai_epi32::<3>(_mm256_add_epi32(sums, bias));
            }
            let packed: [__m256i; 8] = core::array::from_fn(|m| {
                _mm256_packs_epi32(rows[2 * m], rows[(2 * m + 1).min(14)])
            });
            let even: [__m256i; 8] =
                packed.map(|p| _mm256_unpacklo_epi16(p, _mm256_bsrli_epi128::<8>(p)));
            let odd: [__m256i; 7] = core::array::from_fn(|m| {
                _mm256_unpacklo_epi16(_mm256_bsrli_epi128::<8>(packed[m]), packed[m + 1])
            });
            let round = _mm256_set1_epi32(1 << (round1 + 7));
            let zero = _mm256_setzero_si256();
            let mut out = [zero; 8];
            for (row, out) in out.iter_mut().enumerate() {
                let phase = sy4 + delta * (row as i32 - 4) - 4 * gamma;
                let weights: [__m256i; 4] = if gamma == 0 {
                    let taps = _mm256_set1_epi64x(filter(&NATURAL, phase));
                    let low = _mm256_unpacklo_epi8(zero, taps);
                    core::array::from_fn(|pair| match pair {
                        0 => _mm256_shuffle_epi32::<0b00_00_00_00>(low),
                        1 => _mm256_shuffle_epi32::<0b01_01_01_01>(low),
                        2 => _mm256_shuffle_epi32::<0b10_10_10_10>(low),
                        _ => _mm256_shuffle_epi32::<0b11_11_11_11>(low),
                    })
                } else {
                    let (first, second) = filters(&NATURAL, phase, gamma_steps);
                    let low = _mm256_unpacklo_epi16(first, second);
                    let high = _mm256_unpackhi_epi16(first, second);
                    let near = _mm256_unpacklo_epi16(low, high);
                    let far = _mm256_unpackhi_epi16(low, high);
                    [
                        _mm256_unpacklo_epi8(zero, near),
                        _mm256_unpackhi_epi8(zero, near),
                        _mm256_unpacklo_epi8(zero, far),
                        _mm256_unpackhi_epi8(zero, far),
                    ]
                };
                let pairs = |pair: usize| {
                    if row % 2 == 0 {
                        even[row / 2 + pair]
                    } else {
                        odd[row / 2 + pair]
                    }
                };
                let mut sum = round;
                for (pair, &weights) in weights.iter().enumerate() {
                    sum = _mm256_add_epi32(sum, _mm256_madd_epi16(pairs(pair), weights));
                }
                *out = sum;
            }
            out
        }
    }
}
