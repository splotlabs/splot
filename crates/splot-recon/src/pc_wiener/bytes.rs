// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Eight-bit § 7.20.4 PC-Wiener filtering in byte lanes on x86.
//!
//! Every `Pc_Wiener_Filters` row sums to zero, so with `m` the center the
//! filter sum is `128 * m + S`, `S = Σ c (p - m)` over the 24 outer taps.
//! The taps split into two groups that each hold one tap of every
//! symmetric pair, the center joining each group with weight `-Σ c`. A
//! group's `S` lies in `±255 * max(P, N)` for the positive and negative
//! coefficient sums of the 12 pairs, so with both at most 128 it is exact in
//! `i16` lanes, and so is every `pmaddubsw` pair whose weights' magnitudes
//! sum to at most 128. The groups meet as in the Wiener NS byte kernel: a
//! floor average with biases `32768 + 64` and `32768`, `>> 6`, and the
//! center widened with a `0xfe` high byte give `m + Round2(S, 7)`, which
//! `packuswb` clips to 8 bits. Classes outside these bounds take the
//! `pmaddwd` kernel.
//!
//! Adjacent taps of a row weigh even and odd outputs from two byte loads;
//! the other taps pair across rows through an unpack.

use core::arch::x86_64::*;
use std::sync::LazyLock;

use fearless_simd::{Avx2, SimdBase, SimdInto, u8x32};

use super::{PC_WIENER_CONFIG, PC_WIENER_FILTERS, PC_WIENER_FULL_CLASSES};
use crate::wienerns_filter::eight_bit::ByteLaneOutput;

/// Output columns per vector.
const LANES: usize = 32;
/// Source bytes one vector row reads.
const REACH: usize = LANES + 2 * super::PC_WIENER_FILTER_TAP_RADIUS;
/// Weight pairs of the kernel.
const UNITS: usize = 14;
/// The source bytes of one vector row: a fixed size puts every tap at a
/// constant offset.
type Row = [u8; REACH];

/// The two taps `(dy, dx)` of each weight pair in the order the kernel reads
/// them, `(0, 0)` the center share and `None` a zero weight. Pairs `0..7` are
/// one group and `7..14` the other, each with one interleaved pair last.
const UNIT_TAPS: [[Option<(isize, isize)>; 2]; UNITS] = [
    [Some((0, 0)), Some((0, 1))],
    [Some((0, 2)), Some((0, 3))],
    [Some((1, -2)), Some((1, -1))],
    [Some((1, 0)), Some((1, 1))],
    [Some((2, -1)), Some((2, 0))],
    [Some((3, 0)), None],
    [Some((1, 2)), Some((2, 1))],
    [Some((0, -1)), Some((0, 0))],
    [Some((0, -3)), Some((0, -2))],
    [Some((-1, 1)), Some((-1, 2))],
    [Some((-1, -1)), Some((-1, 0))],
    [Some((-2, 0)), Some((-2, 1))],
    [Some((-3, 0)), None],
    [Some((-1, -2)), Some((-2, -1))],
];

/// The byte weights of every class of one filter set: per class and pair
/// the bytes `w0, w1, w0, w1`, and bit `k` of `fits` set when class `k`
/// meets the bounds of the module docs.
pub(super) struct ByteClasses {
    units: [[u32; 16]; PC_WIENER_FULL_CLASSES],
    fits: u64,
}

/// [`ByteClasses`] of every § 9.8 filter set.
pub(super) static BYTE_CLASSES: LazyLock<Vec<ByteClasses>> = LazyLock::new(|| {
    PC_WIENER_FILTERS
        .iter()
        .map(|set| {
            let mut classes = ByteClasses {
                units: [[0; 16]; PC_WIENER_FULL_CLASSES],
                fits: 0,
            };
            for (index, coeffs) in set.iter().enumerate() {
                if let Some(units) = unit_weights(coeffs) {
                    classes.units[index] = units;
                    classes.fits |= 1 << index;
                }
            }
            classes
        })
        .collect()
});

/// The weight-pair dwords of one class, or `None` outside the bounds.
fn unit_weights(coeffs: &[i32; 13]) -> Option<[u32; 16]> {
    let taps = &coeffs[..PC_WIENER_CONFIG.len()];
    let center = -taps.iter().sum::<i32>();
    let positive: i32 = taps.iter().map(|&c| c.max(0)).sum();
    let negative: i32 = taps.iter().map(|&c| (-c).max(0)).sum();
    if positive > 128 || negative > 128 {
        return None;
    }
    let weight = |tap: Option<(isize, isize)>| match tap {
        None => Some(0),
        Some((0, 0)) => Some(center),
        Some((dy, dx)) => PC_WIENER_CONFIG
            .iter()
            .position(|&pair| pair == (dy, dx) || pair == (-dy, -dx))
            .map(|pair| taps[pair]),
    };
    let mut units = [0; 16];
    for (slot, pair) in units.iter_mut().zip(UNIT_TAPS) {
        let [w0, w1] = [weight(pair[0])?, weight(pair[1])?];
        if w0.abs() + w1.abs() > 128 {
            return None;
        }
        let [w0, w1] = [i8::try_from(w0).ok()? as u8, i8::try_from(w1).ok()? as u8];
        *slot = u32::from_le_bytes([w0, w1, w0, w1]);
    }
    Some(units)
}

/// The weight vectors of one 32-column group: one per adjacent pair, and per
/// interleaved pair one for each unpack half.
#[derive(Clone, Copy)]
struct Weights {
    pairs: [__m256i; UNITS - 2],
    low: [__m256i; 2],
    high: [__m256i; 2],
}

fearless_simd::kernel!(
    #[inline]
    fn uniform_weights(t: Avx2, units: &[u32; 16]) -> Weights {
        let _ = t;
        let splat = |unit: usize| _mm256_set1_epi32(units[unit] as i32);
        let interleaved = [splat(6), splat(13)];
        Weights {
            pairs: core::array::from_fn(|k| splat(k + usize::from(k >= 6))),
            low: interleaved,
            high: interleaved,
        }
    }
);

fearless_simd::kernel!(
    /// Pair `u` of cell `k` sits in dword `k` of vector `u`: an 8x8 dword
    /// transpose of the cells' tables. An interleaved pair's low unpack half
    /// covers cells `0, 1, 4, 5` and its high half cells `2, 3, 6, 7`.
    #[inline]
    fn mixed_weights(t: Avx2, cells: [&[u32; 16]; 8]) -> Weights {
        let transpose = |half: usize| -> [__m256i; 8] {
            let r: [__m256i; 8] = core::array::from_fn(|cell| {
                let units = cells[cell].first_chunk::<8>();
                let units = if half == 0 {
                    units
                } else {
                    cells[cell].last_chunk::<8>()
                };
                fearless_simd::u32x8::load_array_ref(t, units.unwrap_or(&[0; 8])).into()
            });
            let t32: [__m256i; 8] = core::array::from_fn(|k| {
                let (a, b) = (r[k / 2 * 2], r[k / 2 * 2 + 1]);
                if k % 2 == 0 {
                    _mm256_unpacklo_epi32(a, b)
                } else {
                    _mm256_unpackhi_epi32(a, b)
                }
            });
            let t64: [__m256i; 8] = core::array::from_fn(|k| {
                let (block, slot) = (k / 4, k % 4);
                let (a, b) = (t32[4 * block + slot / 2], t32[4 * block + 2 + slot / 2]);
                if slot % 2 == 0 {
                    _mm256_unpacklo_epi64(a, b)
                } else {
                    _mm256_unpackhi_epi64(a, b)
                }
            });
            core::array::from_fn(|unit| {
                let (a, b) = (t64[unit % 4], t64[4 + unit % 4]);
                if unit < 4 {
                    _mm256_permute2x128_si256::<0x20>(a, b)
                } else {
                    _mm256_permute2x128_si256::<0x31>(a, b)
                }
            })
        };
        let (first, second) = (transpose(0), transpose(1));
        let unit = |k: usize| if k < 8 { first[k] } else { second[k - 8] };
        let interleaved = [unit(6), unit(13)];
        Weights {
            pairs: core::array::from_fn(|k| unit(k + usize::from(k >= 6))),
            low: interleaved.map(|v| _mm256_unpacklo_epi32(v, v)),
            high: interleaved.map(|v| _mm256_unpackhi_epi32(v, v)),
        }
    }
);

#[inline]
fn at<const O: usize>(row: &Row) -> &[u8; LANES] {
    row[O..].first_chunk::<LANES>().unwrap_or(&[0; LANES])
}

fearless_simd::kernel!(
    /// One row of 32 outputs from the seven source rows `rows[dy + 3]`,
    /// which start three columns left of the first output.
    #[inline]
    fn filter_row(t: Avx2, rows: &[&Row; 7], w: &Weights) -> __m256i {
        macro_rules! ld {
            ($row:literal, $col:literal) => {
                __m256i::from(u8x32::load_array_ref(t, at::<$col>(rows[$row])))
            };
        }
        let madd = |bytes: __m256i, weights: __m256i| _mm256_maddubs_epi16(bytes, weights);
        let add = |a: __m256i, b: __m256i| _mm256_add_epi16(a, b);
        let group = |bias: i16, even: __m256i, odd: __m256i, a: __m256i, b: __m256i, k: usize| {
            (
                add(
                    add(_mm256_set1_epi16(bias), _mm256_unpacklo_epi16(even, odd)),
                    madd(_mm256_unpacklo_epi8(a, b), w.low[k]),
                ),
                add(
                    add(_mm256_set1_epi16(bias), _mm256_unpackhi_epi16(even, odd)),
                    madd(_mm256_unpackhi_epi8(a, b), w.high[k]),
                ),
            )
        };

        let even = add(
            add(madd(ld!(3, 3), w.pairs[0]), madd(ld!(3, 5), w.pairs[1])),
            add(madd(ld!(4, 1), w.pairs[2]), madd(ld!(4, 3), w.pairs[3])),
        );
        let even = add(
            even,
            add(madd(ld!(5, 2), w.pairs[4]), madd(ld!(6, 3), w.pairs[5])),
        );
        let odd = add(
            add(madd(ld!(3, 4), w.pairs[0]), madd(ld!(3, 6), w.pairs[1])),
            add(madd(ld!(4, 2), w.pairs[2]), madd(ld!(4, 4), w.pairs[3])),
        );
        let odd = add(
            odd,
            add(madd(ld!(5, 3), w.pairs[4]), madd(ld!(6, 4), w.pairs[5])),
        );
        let upper = group((0x8000u16 + 64) as i16, even, odd, ld!(4, 5), ld!(5, 4), 0);

        let even = add(
            add(madd(ld!(3, 2), w.pairs[6]), madd(ld!(3, 0), w.pairs[7])),
            add(madd(ld!(2, 4), w.pairs[8]), madd(ld!(2, 2), w.pairs[9])),
        );
        let even = add(
            even,
            add(madd(ld!(1, 3), w.pairs[10]), madd(ld!(0, 3), w.pairs[11])),
        );
        let odd = add(
            add(madd(ld!(3, 3), w.pairs[6]), madd(ld!(3, 1), w.pairs[7])),
            add(madd(ld!(2, 5), w.pairs[8]), madd(ld!(2, 3), w.pairs[9])),
        );
        let odd = add(
            odd,
            add(madd(ld!(1, 4), w.pairs[10]), madd(ld!(0, 4), w.pairs[11])),
        );
        let lower = group(i16::MIN, even, odd, ld!(2, 1), ld!(1, 2), 1);

        let center = ld!(3, 3);
        let offset = _mm256_set1_epi8(0xfe_u8 as i8);
        let value = |upper: __m256i, lower: __m256i, center: __m256i| {
            let average = _mm256_add_epi16(
                _mm256_and_si256(upper, lower),
                _mm256_srli_epi16::<1>(_mm256_xor_si256(upper, lower)),
            );
            add(_mm256_srli_epi16::<6>(average), center)
        };
        _mm256_packus_epi16(
            value(upper.0, lower.0, _mm256_unpacklo_epi8(center, offset)),
            value(upper.1, lower.1, _mm256_unpackhi_epi8(center, offset)),
        )
    }
);

/// Filters the 32-column groups from column `x` to `end` of the `rows`
/// output rows of a cell row, output row `r` at `r * stride` of `output` and
/// reading the padded rows `window[r..r + 7]`, while every cell's class in
/// `cells` fits; returns the column reached.
pub(super) fn filter_groups<O: ByteLaneOutput>(
    t: Avx2,
    window: &[&[u8]; 10],
    rows: usize,
    (mut x, end): (usize, usize),
    (cells, classes): (&[u8], &ByteClasses),
    (output, stride): (&mut [O], usize),
) -> usize {
    while x + LANES <= end {
        let Some(first) = window[0].get(x..).and_then(<[u8]>::first_chunk::<REACH>) else {
            break;
        };
        let mut source = [first; 10];
        for (slot, row) in source.iter_mut().zip(window).take(rows + 6) {
            match row.get(x..).and_then(<[u8]>::first_chunk::<REACH>) {
                Some(row) => *slot = row,
                None => return x,
            }
        }
        let Some(ids) = cells.get(x / 4..).and_then(<[u8]>::first_chunk::<8>) else {
            break;
        };
        let ids = ids.map(|id| usize::from(id) % PC_WIENER_FULL_CLASSES);
        if ids.iter().any(|&id| classes.fits >> id & 1 == 0) {
            break;
        }
        let weights = if ids == [ids[0]; 8] {
            uniform_weights(t, &classes.units[ids[0]])
        } else {
            mixed_weights(t, ids.map(|id| &classes.units[id]))
        };
        for r in 0..rows {
            let (Some(source), Some(target)) = (
                source.get(r..).and_then(<[&Row]>::first_chunk::<7>),
                output
                    .get_mut(r * stride + x..)
                    .and_then(<[O]>::first_chunk_mut::<LANES>),
            ) else {
                return x;
            };
            let bytes: u8x32<Avx2> = filter_row(t, source, &weights).simd_into(t);
            O::store(bytes.into(), target);
        }
        x += LANES;
    }
    x
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Every class of every set outside the bounds is one whose positive or
    /// negative tap sum passes 128, and the kernel matches the § 7.20.4 sum
    /// for random and extreme samples and cell maps.
    #[test]
    fn byte_rows_match_the_scalar_sum() {
        let Some(t) = fearless_simd::Level::baseline().as_avx2() else {
            return;
        };
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |limit: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % limit
        };
        for (set, classes) in BYTE_CLASSES.iter().enumerate() {
            for (id, coeffs) in PC_WIENER_FILTERS[set].iter().enumerate() {
                let taps = &coeffs[..PC_WIENER_CONFIG.len()];
                let positive: i32 = taps.iter().map(|&c| c.max(0)).sum();
                let negative: i32 = taps.iter().map(|&c| (-c).max(0)).sum();
                let fits = classes.fits >> id & 1 == 1;
                assert_eq!(
                    fits,
                    positive <= 128 && negative <= 128,
                    "set {set} class {id}"
                );
            }
            for case in 0..40 {
                let width = [32, 64, 96, 768][case % 4];
                let rows = 1 + case % 4;
                let source: Vec<Vec<u8>> = (0..rows + 6)
                    .map(|_| {
                        let style = next(3);
                        (0..width + 6)
                            .map(|_| match style {
                                0 => [0, 255][next(2) as usize],
                                _ => next(256) as u8,
                            })
                            .collect()
                    })
                    .collect();
                let fitting: Vec<u8> = (0..64u8)
                    .filter(|&id| classes.fits >> id & 1 == 1)
                    .collect();
                let uniform = fitting[next(fitting.len() as u64) as usize];
                let cells: Vec<u8> = (0..width / 4)
                    .map(|_| match case % 3 {
                        0 => uniform,
                        _ => fitting[next(fitting.len() as u64) as usize],
                    })
                    .collect();
                let mut window = [source[0].as_slice(); 10];
                for (slot, row) in window.iter_mut().zip(&source) {
                    *slot = row;
                }
                let mut output = vec![0u8; width * rows];
                let done = filter_groups(
                    t,
                    &window,
                    rows,
                    (0, width),
                    (&cells, classes),
                    (&mut output, width),
                );
                assert_eq!(done, width);
                for y in 0..rows {
                    for x in 0..width {
                        let coeffs = &PC_WIENER_FILTERS[set][usize::from(cells[x / 4])];
                        let tap = |dy: isize, dx: isize| {
                            i32::from(
                                source[(y as isize + 3 + dy) as usize]
                                    [(x as isize + 3 + dx) as usize],
                            )
                        };
                        let mut sum = tap(0, 0) * (128 + coeffs[12]);
                        for (&(dy, dx), &c) in PC_WIENER_CONFIG.iter().zip(coeffs) {
                            sum += c * (tap(dy, dx) + tap(-dy, -dx));
                        }
                        let expected = ((sum + 64) >> 7).clamp(0, 255) as u8;
                        assert_eq!(
                            output[y * width + x],
                            expected,
                            "set {set} case {case} ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}
