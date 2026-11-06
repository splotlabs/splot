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

use super::{ProjectedCenter, Shear, WARP_FILTER_TAPS, WARPED_FILTERS, WARPED_TAPS, filter_index};
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

/// The `Round2(sum, round1)` rows of the section whose source window starts
/// at `(first_col, first_row)`, or `None` without AVX2.
pub(super) fn section(
    reference: &ReferencePlaneView<'_, u8>,
    shear: &Shear,
    projected: &ProjectedCenter,
    (first_col, first_row): (usize, usize),
    round1: u32,
) -> Option<[[i32; 8]; 8]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        let mut windows = [[0u8; 16]; 15];
        for (row, window) in windows.iter_mut().enumerate() {
            *window = *reference
                .row(first_row + row)
                .get(first_col..)?
                .first_chunk::<16>()?;
        }
        return Some(avx2::section(
            token,
            &windows,
            [shear.alpha, shear.beta, shear.gamma, shear.delta],
            [projected.sx4, projected.sy4],
            round1,
        ));
    }
    let _ = (reference, shear, projected, first_col, first_row, round1);
    None
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{EVEN_ODD, NATURAL, filter_index};
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
            windows: &[[u8; 16]; 15],
            shear: [i32; 4],
            phases: [i32; 2],
            round1: u32,
        ) -> [[i32; 8]; 8] {
            let ([alpha, beta, gamma, delta], [sx4, sy4]) = (shear, phases);
            let filter = |table: &[[u8; 8]; 512], phase: i32| -> i64 {
                i64::from_le_bytes(table[filter_index(phase)])
            };
            let shuffle_a: __m256i = u8x32::load_array(t, SHUFFLE_A).into();
            let shuffle_b: __m256i = u8x32::load_array(t, SHUFFLE_B).into();
            let ones = _mm256_set1_epi16(1);
            let bias = _mm256_set1_epi32(1 << 2);
            let mut rows = [_mm256_setzero_si256(); 15];
            for (row, (window, out)) in windows.iter().zip(&mut rows).enumerate() {
                let phase = sx4 + beta * (row as i32 - 7) - 4 * alpha;
                let (taps_a, taps_b) = if alpha == 0 {
                    let taps = _mm256_set1_epi64x(filter(&EVEN_ODD, phase));
                    (taps, taps)
                } else {
                    let tap = |column: i32| filter(&EVEN_ODD, phase + alpha * column);
                    (
                        _mm256_set_epi64x(tap(5), tap(4), tap(1), tap(0)),
                        _mm256_set_epi64x(tap(7), tap(6), tap(3), tap(2)),
                    )
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
            let shift = _mm_cvtsi32_si128(round1 as i32 + 8);
            let round = _mm256_set1_epi32(1 << (round1 + 7));
            let zero = _mm256_setzero_si256();
            let mut out = [[0i32; 8]; 8];
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
                    let tap = |column: i32| filter(&NATURAL, phase + gamma * column);
                    let first = _mm256_set_epi64x(tap(5), tap(4), tap(1), tap(0));
                    let second = _mm256_set_epi64x(tap(7), tap(6), tap(3), tap(2));
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
                i32x8::<Avx2>::simd_from(t, _mm256_sra_epi32(sum, shift)).store_array(out);
            }
            out
        }
    );
}
