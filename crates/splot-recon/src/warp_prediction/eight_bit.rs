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
    WARPEDDIFF_PREC_BITS, WarpPredictBlockParams, two_tap_phase,
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

/// The projection of the first of a row of sections, its step to the next
/// section eight samples right (§ 7.13.3.19 `dstX` and `dstY` are affine in
/// the section column), and the clip bounds and size of the reference.
struct RowProjection {
    first: [i64; 2],
    step: [i64; 2],
    sub: [u32; 2],
    bounds: [i64; 4],
    size: [i64; 2],
}

/// Stores the `Clip1` single-reference prediction of each full 8x8 section
/// of one row of `sections` whose source window lies inside its bounds and
/// the storage, into `output` rows `stride` apart from the row's top-left,
/// and returns the mask of the sections written; the caller predicts the
/// rest. `params` places the row's first section.
///
/// Sections start below `2^20`, where every projection is exact in `i64`
/// and an interior source position fits `i32`.
pub(super) fn put_row(
    reference: &ReferencePlaneView<'_, u8>,
    params: &WarpPredictBlockParams,
    shear: &Shear,
    sections: usize,
    output: &mut [u8],
    stride: usize,
) -> u64 {
    const LIMIT: i32 = 1 << 20;
    if sections > 64
        || !(0..LIMIT).contains(&params.block_x)
        || !(0..LIMIT).contains(&params.block_y)
    {
        return 0;
    }
    let sub = [
        u32::from(params.subsampling_x),
        u32::from(params.subsampling_y),
    ];
    let [w0, w1, w2, w3, w4, w5] = params.warp_params.map(i64::from);
    let src_x = i64::from(params.block_x + 4) << sub[0];
    let src_y = i64::from(params.block_y + 4) << sub[1];
    let projection = RowProjection {
        first: [w2 * src_x + w3 * src_y + w0, w4 * src_x + w5 * src_y + w1],
        step: [w2 << (3 + sub[0]), w4 << (3 + sub[0])],
        sub,
        bounds: params.bounds().map(i64::from),
        size: [reference.width() as i64, reference.height() as i64],
    };
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::put_row(
            token,
            reference,
            &projection,
            [shear.alpha, shear.beta, shear.gamma, shear.delta],
            sections,
            output,
            stride,
        );
    }
    let _ = (projection, shear, output, stride);
    0
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{
        EVEN_ODD, NATURAL, RowProjection, WARP_FILTER_CENTER, WARPEDDIFF_PREC_BITS, Window,
        two_tap_phase,
    };
    use crate::subpel_mc::ReferencePlaneView;
    use crate::warp_prediction::INTER_ROUND1_NON_COMPOUND;
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
        pub(super) fn put_row(
            t: Avx2,
            reference: &ReferencePlaneView<'_, u8>,
            projection: &RowProjection,
            shear: [i32; 4],
            sections: usize,
            output: &mut [u8],
            stride: usize,
        ) -> u64 {
            const ROUND1: i32 = INTER_ROUND1_NON_COMPOUND as i32;
            let RowProjection {
                first,
                step,
                sub,
                bounds: [first_x, first_y, last_x, last_y],
                size: [width, height],
            } = *projection;
            let two_tap = shear[0] == 0 && shear[1] == 0;
            let mut written = 0;
            for section in 0..sections {
                let x4 = (first[0] + section as i64 * step[0]) >> sub[0];
                let y4 = (first[1] + section as i64 * step[1]) >> sub[1];
                let (col, row) = ((x4 >> 16) - 7, (y4 >> 16) - 7);
                let phases = [(x4 & 0xffff) as i32, (y4 & 0xffff) as i32];
                if col < first_x
                    || col + 14 > last_x
                    || col + 15 >= width
                    || row < first_y
                    || row + 14 > last_y
                    || row + 14 >= height
                    || (two_tap && two_tap_phase(phases[0]).is_some())
                {
                    continue;
                }
                let (Some(window), Some(target)) = (
                    Window::new(reference, (col as usize, row as usize)),
                    output.get_mut(8 * section..8 * section + 7 * stride + 8),
                ) else {
                    continue;
                };
                let rows = sums(t, &window, shear, phases, INTER_ROUND1_NON_COMPOUND);
                for (pair, rows) in rows.chunks_exact(2).enumerate() {
                    let words = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packs_epi32(
                        _mm256_srai_epi32::<{ ROUND1 + 8 }>(rows[0]),
                        _mm256_srai_epi32::<{ ROUND1 + 8 }>(rows[1]),
                    ));
                    let bytes = _mm256_packus_epi16(words, words);
                    let lines = [
                        _mm_cvtsi128_si64(_mm256_castsi256_si128(bytes)),
                        _mm_cvtsi128_si64(_mm256_extracti128_si256::<1>(bytes)),
                    ];
                    for (row, line) in (2 * pair..).zip(lines) {
                        if let Some(slot) = target
                            .get_mut(row * stride..)
                            .and_then(<[u8]>::first_chunk_mut::<8>)
                        {
                            *slot = line.to_le_bytes();
                        }
                    }
                }
                written |= 1 << section;
            }
            written
        }
    );

    /// A pass phase offset by the `Warped_Filters` bias and shifted up so
    /// that its top nine bits are the row `filter_index` selects, masked.
    #[inline]
    fn key(phase: i32) -> u32 {
        const BIAS: i32 =
            (1 << (WARPEDDIFF_PREC_BITS - 1)) + (WARP_FILTER_CENTER << WARPEDDIFF_PREC_BITS);
        (phase.wrapping_add(BIAS) as u32) << (23 - WARPEDDIFF_PREC_BITS)
    }

    /// The table row of a [`key`].
    #[inline]
    fn tap(table: &[[u8; 8]; 512], key: u32) -> i64 {
        i64::from_le_bytes(table[(key >> 23) as usize])
    }

    /// The filters of output columns `0, 1 | 4, 5` and `2, 3 | 6, 7` whose
    /// [`key`]s are the lanes of `keys` in that order.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn filters(table: &[[u8; 8]; 512], keys: __m256i) -> (__m256i, __m256i) {
        let rows = _mm256_srli_epi32::<23>(keys);
        let tap = |row: i32| i64::from_le_bytes(table[row as usize & 511]);
        let taps = |lanes: __m128i| {
            _mm256_set_epi64x(
                tap(_mm_extract_epi32::<3>(lanes)),
                tap(_mm_extract_epi32::<2>(lanes)),
                tap(_mm_extract_epi32::<1>(lanes)),
                tap(_mm_cvtsi128_si32(lanes)),
            )
        };
        (
            taps(_mm256_castsi256_si128(rows)),
            taps(_mm256_extracti128_si256::<1>(rows)),
        )
    }

    /// The [`key`]s of the eight columns, in [`filters`] order, of a pass
    /// whose column-0 key is `first` and whose columns step by `shear`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn column_keys(first: u32, shear: i32) -> __m256i {
        let columns = _mm256_setr_epi32(0, 1, 4, 5, 2, 3, 6, 7);
        _mm256_add_epi32(
            _mm256_set1_epi32(first as i32),
            _mm256_mullo_epi32(
                _mm256_set1_epi32(shear << (23 - WARPEDDIFF_PREC_BITS)),
                columns,
            ),
        )
    }

    /// The vertical-pass sums of the eight output rows of a section, each
    /// scaled by 256 with the `Round2(.., round1)` bias added.
    ///
    /// Each horizontal row keeps `Round2(sum, InterRound0)` in the high word
    /// of its lanes (`(sum << 13) + (1 << 15)`), so one shift and blend
    /// interleave two rows into the word pairs of a vertical tap pair.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn sums(
        t: Avx2,
        window: &Window<'_>,
        [alpha, beta, gamma, delta]: [i32; 4],
        [sx4, sy4]: [i32; 2],
        round1: u32,
    ) -> [__m256i; 8] {
        let shuffle_a: __m256i = u8x32::load_array(t, SHUFFLE_A).into();
        let shuffle_b: __m256i = u8x32::load_array(t, SHUFFLE_B).into();
        let (scale, bias) = (_mm256_set1_epi16(1 << 13), _mm256_set1_epi32(1 << 15));
        let step = |shear: i32| _mm256_set1_epi32(shear << (23 - WARPEDDIFF_PREC_BITS));
        let first = key(sx4.wrapping_sub(7 * beta).wrapping_sub(4 * alpha));
        let mut keys = column_keys(first, alpha);
        let row_step = step(beta);
        let horizontal: [__m256i; 15] = core::array::from_fn(|row| {
            let (taps_a, taps_b) = if alpha == 0 {
                let taps = _mm256_set1_epi64x(tap(&EVEN_ODD, _mm256_cvtsi256_si32(keys) as u32));
                (taps, taps)
            } else {
                filters(&EVEN_ODD, keys)
            };
            keys = _mm256_add_epi32(keys, row_step);
            let source = window
                .rows
                .get(window.column + row * window.stride..)
                .and_then(<[u8]>::first_chunk::<16>)
                .map_or(_mm_setzero_si128(), |bytes| {
                    u8x16::load_array_ref(t, bytes).into()
                });
            let source = _mm256_broadcastsi128_si256(source);
            let a = _mm256_maddubs_epi16(_mm256_shuffle_epi8(source, shuffle_a), taps_a);
            let b = _mm256_maddubs_epi16(_mm256_shuffle_epi8(source, shuffle_b), taps_b);
            _mm256_add_epi32(_mm256_madd_epi16(_mm256_hadd_epi16(a, b), scale), bias)
        });
        let pairs: [__m256i; 14] = core::array::from_fn(|row| {
            _mm256_blend_epi16::<0xaa>(
                _mm256_srli_epi32::<16>(horizontal[row]),
                horizontal[row + 1],
            )
        });
        let round = _mm256_set1_epi32(1 << (round1 + 7));
        let zero = _mm256_setzero_si256();
        let first = key(sy4.wrapping_sub(4 * delta).wrapping_sub(4 * gamma));
        let mut keys = column_keys(first, gamma);
        let row_step = step(delta);
        core::array::from_fn(|row| {
            let weights: [__m256i; 4] = if gamma == 0 {
                let taps = tap(&NATURAL, _mm256_cvtsi256_si32(keys) as u32);
                let low = _mm256_unpacklo_epi8(zero, _mm256_set1_epi64x(taps));
                [
                    _mm256_shuffle_epi32::<0b00_00_00_00>(low),
                    _mm256_shuffle_epi32::<0b01_01_01_01>(low),
                    _mm256_shuffle_epi32::<0b10_10_10_10>(low),
                    _mm256_shuffle_epi32::<0b11_11_11_11>(low),
                ]
            } else {
                let (first, second) = filters(&NATURAL, keys);
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
            keys = _mm256_add_epi32(keys, row_step);
            let mut sum = round;
            for (pair, &weights) in weights.iter().enumerate() {
                sum = _mm256_add_epi32(sum, _mm256_madd_epi16(pairs[row + 2 * pair], weights));
            }
            sum
        })
    }
}
