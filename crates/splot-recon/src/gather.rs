// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AVX2 table gathers behind a safe interface: every index is masked into the
//! table before the gather reads it.

#![allow(
    unsafe_code,
    reason = "the AVX2 gather intrinsics take a raw base pointer"
)]

use core::arch::x86_64::{__m128i, __m256i, _mm_and_si128, _mm_set1_epi32, _mm256_i32gather_epi64};

/// The eight-byte rows `table[rows[k] % N]` of the four `i32` lanes of
/// `rows`, in lane order.
#[inline]
#[target_feature(enable = "avx2")]
pub(crate) fn rows_u64<const N: usize>(table: &[[u8; 8]; N], rows: __m128i) -> __m256i {
    const { assert!(N.is_power_of_two() && N <= 1 << 30) };
    let rows = _mm_and_si128(rows, _mm_set1_epi32(N as i32 - 1));
    unsafe { _mm256_i32gather_epi64::<8>(table.as_ptr().cast(), rows) } // SAFETY: each masked index is below `N`, so every eight-byte read lies inside `table`.
}

#[cfg(test)]
mod tests {
    use super::rows_u64;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i32x4, u64x4};

    fearless_simd::kernel!(
        fn gathered(t: Avx2, table: &[[u8; 8]; 16], rows: [i32; 4]) -> [u64; 4] {
            let rows = i32x4::load_array(t, rows).into();
            u64x4::<Avx2>::simd_from(t, rows_u64(table, rows)).to_array()
        }
    );

    #[test]
    fn gathers_the_masked_rows_in_lane_order() {
        let Some(avx2) = fearless_simd::Level::baseline().as_avx2() else {
            return;
        };
        let table: [[u8; 8]; 16] = core::array::from_fn(|row| [row as u8; 8]);
        let want = [3u8, 0, 15, 15].map(|row| u64::from_le_bytes([row; 8]));
        assert_eq!(gathered(avx2, &table, [3, 0, 15, -1]), want);
    }
}
