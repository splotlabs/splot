// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AVX2 pair weights and row arithmetic shared by luma and PC-Wiener.

use core::arch::x86_64::*;

use fearless_simd::{Avx2, SimdBase, SimdInto, i16x16};

use crate::{LaneClass, MaddWeights};

/// Forms each 128-bit half of a pair's weights by broadcasting one 32-bit
/// weight pair of its cell.
#[inline]
pub fn in_lane_weights(
    t: fearless_simd::Avx2,
    classes: [&LaneClass; 4],
    pairs: usize,
) -> [MaddWeights; 9] {
    fearless_simd::kernel!(
        #[inline(always)]
        fn kernel(t: Avx2, classes: [&LaneClass; 4], pairs: usize) -> [MaddWeights; 9] {
            let load =
                |class: &LaneClass| -> __m256i { i16x16::load_array_ref(t, &class.0).into() };
            let center = |class: &LaneClass| i32::from(class.1 as u16) | (1 << 22);
            let store = |weights: &mut [i16; 16], lanes: __m256i| {
                let lanes: i16x16<Avx2> = lanes.simd_into(t);
                lanes.store_array(weights);
            };
            let mut weights = [[[0; 16]; 2]; 9];
            for (half, (low, high)) in [(classes[0], classes[2]), (classes[1], classes[3])]
                .into_iter()
                .enumerate()
            {
                let (x, y) = (load(low), load(high));
                let first = _mm256_permute2x128_si256::<0x20>(x, y);
                let last = _mm256_permute2x128_si256::<0x31>(x, y);
                macro_rules! pair {
                    ($($pair:literal $lanes:ident $imm:literal),*) => {$(
                        if $pair < pairs {
                            store(&mut weights[$pair][half], _mm256_shuffle_epi32::<$imm>($lanes));
                        }
                    )*};
                }
                pair!(0 first 0x00, 1 first 0x55, 2 first 0xaa, 3 first 0xff);
                pair!(4 last 0x00, 5 last 0x55, 6 last 0xaa, 7 last 0xff);
                if let Some(weights) = weights.get_mut(pairs) {
                    let (a, b) = (center(low), center(high));
                    store(
                        &mut weights[half],
                        _mm256_setr_epi32(a, a, a, a, b, b, b, b),
                    );
                }
            }
            weights
        }
    );
    kernel(t, classes, pairs)
}

/// Defines the fixed-geometry AVX2 row arithmetic for one sample type.
#[doc(hidden)]
#[macro_export]
macro_rules! madd_row_kernel {
    ($name:ident, $sample:ty, $taps:ty, $rows:literal, $reach:literal, |$t:ident, $lanes:ident| $words:block) => {
        fearless_simd::kernel!(
            #[inline(always)]
            fn $name(
                $t: Avx2,
                window: &[&[$sample; $reach]; $rows],
                weights: &[MaddWeights],
                max_sample: u16,
            ) -> [u16; 16] {
                let r = <$taps as $crate::TapGeometry>::RADIUS;
                let taps = <$taps as $crate::TapGeometry>::TAPS;
                let load = |row: usize, col: usize| -> __m256i {
                    match window
                        .get(row)
                        .and_then(|row| row.get(col..))
                        .and_then(<[$sample]>::first_chunk::<16>)
                    {
                        Some($lanes) => $words,
                        None => _mm256_setzero_si256(),
                    }
                };
                let term = |j: usize| {
                    let (dy, dx) = taps[j];
                    _mm256_add_epi16(
                        load(r.wrapping_add_signed(dy), r.wrapping_add_signed(dx)),
                        load(r.wrapping_add_signed(-dy), r.wrapping_add_signed(-dx)),
                    )
                };
                let half = |weights: &MaddWeights, half: usize| -> __m256i {
                    i16x16::load_array_ref($t, &weights[half]).into()
                };
                let mut sums = [_mm256_setzero_si256(); 2];
                let mut add = |a: __m256i, b: __m256i, weights: &MaddWeights| {
                    sums[0] = _mm256_add_epi32(
                        sums[0],
                        _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), half(weights, 0)),
                    );
                    sums[1] = _mm256_add_epi32(
                        sums[1],
                        _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), half(weights, 1)),
                    );
                };
                let pairs = taps.len() / 2;
                for (pair, weights) in weights.iter().enumerate().take(pairs) {
                    add(term(2 * pair), term(2 * pair + 1), weights);
                }
                if let Some(weights) = weights.get(pairs) {
                    add(load(r, r), _mm256_set1_epi16(1), weights);
                }
                let values = _mm256_min_epu16(
                    _mm256_packus_epi32(
                        _mm256_srai_epi32::<7>(sums[0]),
                        _mm256_srai_epi32::<7>(sums[1]),
                    ),
                    _mm256_set1_epi16(max_sample as i16),
                );
                let values: u16x16<Avx2> = values.simd_into($t);
                values.into()
            }
        );
    };
}
