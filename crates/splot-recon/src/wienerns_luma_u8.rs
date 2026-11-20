// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Eight-bit § 7.20.3 luma Wiener NS filtering in byte lanes.
//!
//! A vector covers 32 outputs of one row. Each `pmaddubsw` weighs a pair of
//! source bytes per output: two horizontally adjacent taps of one row, read
//! as unaligned byte pairs for the even and the odd outputs, or two taps of
//! different rows interleaved by an unpack. The center joins a row-0 pair as
//! `k * m`, which turns the sum into the difference form
//! `Σ c (p - m)` per group of taps.
//!
//! The 33 taps form two groups, the upper half of the diamond plus both
//! `c3` taps and the lower half. A group's difference sum lies in
//! `±255 * max(P, N)` for its positive and negative coefficient sums, so with
//! both at most 128 each group and every tap pair in it fit `i16`. The groups
//! meet in a rounding unsigned average with biases `32768 + 64` and `32767`,
//! which gives `((S + 64) >> 1) + 32768`. Flipping its sign bit subtracts
//! 32768 before the signed shift by 6. Adding the widened center gives
//! the filtered value, which `packuswb` clips to 8 bits.
//! Classes outside these bounds, which the § 5.20.2.1 tap ranges allow, take
//! the general kernel.
//!
//! The aarch64 kernel has no byte-pair multiply. It adds each symmetric tap
//! pair in `u16` lanes and weighs the sums in `i16` lanes, in four groups of
//! four coefficients. A group's sum lies in `±510 * max(P, N)`, so with both
//! at most 64 it is exact in `i16` lanes, and every § 5.20.2.1 class fits.
//! The groups and the center meet in `i32` lanes.

#[cfg(target_arch = "aarch64")]
use super::WIENER_NS_CONFIG_Y_PAIRS;
use super::{FlatChunks, LumaSubclassLayout, WIENER_NS_LUMA_COEFFS, WIENER_NS_LUMA_TAP_RADIUS};

/// Distinct weight pairs: ten adjacent-tap pairs, then four interleaved ones.
#[cfg(target_arch = "x86_64")]
const UNITS: usize = 14;
/// Output columns per vector.
#[cfg(target_arch = "x86_64")]
const LANES: usize = 32;
/// A subclass map indexes classes through 16-entry byte tables.
const MAX_CLASSES: usize = 16;
/// Bytes per staged row: the widest § 7.20 block, a last restoration unit
/// of 1.5 times `RESTORATION_TILESIZE_MAX`, plus the tap reach. A fixed row
/// size puts every tap of a vector group at a constant offset from one base.
pub(crate) const STAGE_STRIDE: usize = 3 * 512 / 2 + 2 * WIENER_NS_LUMA_TAP_RADIUS;
/// Source rows of one 4x4 cell row group.
const GROUP_ROWS: usize = 12;

/// A weight of [`unit_weights`]: coefficient `j`, or a group's center share.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
enum Weight {
    Coeff(usize),
    UpperCenter,
    LowerCenter,
}

#[cfg(target_arch = "x86_64")]
use Weight::{Coeff, LowerCenter, UpperCenter};

/// The weight pairs in the order the kernel reads them.
#[cfg(target_arch = "x86_64")]
const UNIT_WEIGHTS: [[Weight; 2]; UNITS] = [
    [Coeff(6), Coeff(2)],
    [Coeff(8), Coeff(4)],
    [Coeff(0), Coeff(5)],
    [Coeff(13), Coeff(11)],
    [Coeff(1), UpperCenter],
    [Coeff(2), Coeff(6)],
    [Coeff(5), Coeff(0)],
    [Coeff(4), Coeff(8)],
    [Coeff(11), Coeff(13)],
    [LowerCenter, Coeff(1)],
    [Coeff(12), Coeff(10)],
    [Coeff(14), Coeff(15)],
    [Coeff(7), Coeff(9)],
    [Coeff(3), Coeff(3)],
];

#[cfg(target_arch = "x86_64")]
type UnitWeights = [[i8; 2]; UNITS];

/// The byte-lane weights of one coefficient class, or `None` outside the
/// exactness bounds of the module docs.
#[cfg(target_arch = "x86_64")]
pub(super) fn unit_weights(coeffs: &[i16; WIENER_NS_LUMA_COEFFS]) -> Option<UnitWeights> {
    let coeffs = coeffs.map(i32::from);
    let (positive, negative) = coeffs
        .iter()
        .chain([&coeffs[3]])
        .fold((0, 0), |(p, n), &c| (p + c.max(0), n - c.min(0)));
    if positive.max(negative) > 128 {
        return None;
    }
    let upper = negative - positive;
    let lower = upper + 2 * coeffs[3];
    let mut weights = [[0i8; 2]; UNITS];
    for (slot, pair) in weights.iter_mut().zip(UNIT_WEIGHTS) {
        let values = pair.map(|weight| match weight {
            Coeff(j) => coeffs[j],
            UpperCenter => upper,
            LowerCenter => lower,
        });
        *slot = [i8::try_from(values[0]).ok()?, i8::try_from(values[1]).ok()?];
    }
    Some(weights)
}

#[cfg(target_arch = "x86_64")]
type ClassWeights = UnitWeights;
#[cfg(target_arch = "x86_64")]
use unit_weights as class_weights;
#[cfg(not(target_arch = "x86_64"))]
type ClassWeights = PairWeights;
#[cfg(not(target_arch = "x86_64"))]
use pair_weights as class_weights;

/// The coefficients of the aarch64 kernel's four groups, in the order it
/// reads them.
#[cfg(not(target_arch = "x86_64"))]
const PAIR_ORDER: [usize; WIENER_NS_LUMA_COEFFS] =
    [0, 6, 7, 8, 1, 9, 10, 11, 2, 3, 12, 13, 4, 5, 14, 15];

/// A class's coefficients in [`PAIR_ORDER`], then its center scale.
#[cfg(not(target_arch = "x86_64"))]
type PairWeights = [i16; WIENER_NS_LUMA_COEFFS + 1];

/// The tap-pair weights of one coefficient class, or `None` outside the
/// group bounds of the module docs.
#[cfg(not(target_arch = "x86_64"))]
pub(super) fn pair_weights(coeffs: &[i16; WIENER_NS_LUMA_COEFFS]) -> Option<PairWeights> {
    let mut weights = [0; WIENER_NS_LUMA_COEFFS + 1];
    for (slots, group) in weights.chunks_exact_mut(4).zip(PAIR_ORDER.chunks_exact(4)) {
        let (mut positive, mut negative) = (0, 0);
        for (slot, &j) in slots.iter_mut().zip(group) {
            *slot = coeffs[j];
            positive += i32::from(coeffs[j]).max(0);
            negative -= i32::from(coeffs[j]).min(0);
        }
        if positive.max(negative) > 64 {
            return None;
        }
    }
    let sum: i32 = coeffs.iter().map(|&c| i32::from(c)).sum();
    weights[WIENER_NS_LUMA_COEFFS] = i16::try_from(128 - 2 * sum).ok()?;
    Some(weights)
}

/// Byte rows and class weights of one block.
#[derive(Debug, Default)]
pub(super) struct EightBitLuma {
    staged: Vec<[u8; STAGE_STRIDE]>,
    classes: Vec<ClassWeights>,
    /// The last bank examined. An empty `classes` marks an ineligible bank.
    prepared: Vec<[i16; WIENER_NS_LUMA_COEFFS]>,
    /// Each dword holds the byte weight pairs of two consecutive classes.
    #[cfg(target_arch = "x86_64")]
    tables: [[u8; 2 * MAX_CLASSES]; UNITS],
}

impl EightBitLuma {
    /// Narrows the padded rows `rows(i)` of a `width` x `height` block;
    /// returns whether every row is present and every sample at most 255.
    pub(super) fn stage<'r, T: crate::ReconSample>(
        &mut self,
        rows: impl Fn(usize) -> Option<&'r [T]>,
        width: usize,
        height: usize,
    ) -> bool {
        let padded_width = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let padded_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let len = padded_rows + GROUP_ROWS;
        if padded_width > STAGE_STRIDE
            || self
                .staged
                .try_reserve(len.saturating_sub(self.staged.len()))
                .is_err()
        {
            return false;
        }
        if self.staged.len() < len {
            self.staged.resize(len, [0; STAGE_STRIDE]);
        }
        self.staged
            .iter_mut()
            .take(padded_rows)
            .enumerate()
            .all(|(index, staged)| {
                rows(index)
                    .and_then(|row| row.get(..padded_width))
                    .is_some_and(|row| narrow_row(&mut staged[..padded_width], row))
            })
    }

    /// Prepares the class weights of the staged block; returns whether the
    /// byte-lane kernel takes it.
    pub(super) fn prepare(
        &mut self,
        width: usize,
        coeffs_by_class: &[[i16; WIENER_NS_LUMA_COEFFS]],
        layout: LumaSubclassLayout<'_>,
    ) -> bool {
        if !kernel_available()
            || width < MIN_WIDTH
            || coeffs_by_class.len() > MAX_CLASSES
            || matches!(layout, LumaSubclassLayout::Samples(_))
        {
            return false;
        }
        if self.prepared == coeffs_by_class {
            return !self.classes.is_empty();
        }
        self.prepared.clear();
        self.prepared.extend_from_slice(coeffs_by_class); // splot-copy-ok: weight-cache key, not samples
        self.classes.clear();
        for coeffs in coeffs_by_class {
            let Some(weights) = class_weights(coeffs) else {
                self.classes.clear();
                return false;
            };
            self.classes.push(weights);
        }
        #[cfg(target_arch = "x86_64")]
        {
            for (class, weights) in self.classes.iter().enumerate() {
                for (unit, pair) in weights.iter().enumerate() {
                    let [first, second] = pair.map(|w| w as u8);
                    self.tables[unit][2 * class] = first;
                    self.tables[unit][2 * class + 1] = second;
                }
            }
        }
        true
    }

    /// Filters the rows `first..end` of one 4x4 cell row, block row `r`
    /// landing at output row `r - origin`, over the leading whole vectors of
    /// `width` columns, filling the vectors that `flat` marks; returns the
    /// columns done. A missing class or output stops early and leaves the
    /// rest to the general kernel.
    pub(super) fn filter_cell_rows<O: ByteLaneOutput>(
        &self,
        output: &mut [O],
        output_stride: usize,
        (first, end, origin): (usize, usize, usize),
        width: usize,
        layout: LumaSubclassLayout<'_>,
        flat: &FlatChunks<WIENER_NS_LUMA_TAP_RADIUS>,
    ) -> usize {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
            return avx2::filter_cell_rows(
                token,
                self,
                output,
                output_stride,
                (first, end, origin),
                width,
                layout,
                flat,
            );
        }
        #[cfg(target_arch = "aarch64")]
        if let Some(token) = fearless_simd::Level::baseline().as_neon() {
            return paired::filter_cell_rows(
                token,
                self,
                output,
                output_stride,
                (first, end, origin),
                width,
                layout,
                flat,
            );
        }
        let _ = (
            output,
            output_stride,
            first,
            end,
            origin,
            width,
            layout,
            flat,
        );
        0
    }
}

/// The narrowest block the kernel takes: one vector of outputs.
#[cfg(target_arch = "x86_64")]
const MIN_WIDTH: usize = LANES;
#[cfg(not(target_arch = "x86_64"))]
const MIN_WIDTH: usize = 16;

fn kernel_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        fearless_simd::Level::baseline().as_avx2().is_some()
    }
    #[cfg(target_arch = "aarch64")]
    {
        fearless_simd::Level::baseline().as_neon().is_some()
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

/// Copies `row` into `staged` as bytes; returns `false` for a sample above 255.
pub(crate) fn narrow_row<T: crate::ReconSample>(staged: &mut [u8], row: &[T]) -> bool {
    if let Some(row) = T::u8_slice(row) {
        let len = staged.len().min(row.len());
        staged[..len].copy_from_slice(&row[..len]); // splot-copy-ok: fixed-stride byte-lane kernel rows
        return true;
    }
    T::u16_slice(row)
        .is_some_and(|row| crate::lr_madd::lr_simd!(simd => narrow_row_lanes(simd, staged, row)))
}

/// Narrows `row` into `staged` 32 lanes at a time: whole chunks, an
/// overlapping last chunk, and a row under 32 samples through a zero-padded
/// chunk. Returns whether every sample is at most 255.
#[inline]
fn narrow_row_lanes<S: fearless_simd::Simd>(simd: S, staged: &mut [u8], row: &[u16]) -> bool {
    use fearless_simd::{prelude::*, u16x16};
    let len = staged.len().min(row.len());
    let (staged, row) = (&mut staged[..len], &row[..len]);
    let narrow = |lanes: &[u16; 32], out: &mut [u8; 32]| {
        let (low, high) = lanes.split_at(16);
        let (low, high) = (
            u16x16::from_slice(simd, low),
            u16x16::from_slice(simd, high),
        );
        low.narrow(high).store_array(out);
        low.max(high)
    };
    let Some(last) = len.checked_sub(32) else {
        let (mut lanes, mut bytes) = ([0; 32], [0; 32]);
        lanes[..len].copy_from_slice(row); // splot-copy-ok: zero-padded chunk of a row under 32 samples
        let peak = narrow(&lanes, &mut bytes);
        staged.copy_from_slice(&bytes[..len]); // splot-copy-ok: narrowed chunk into the staged row
        return peak.reduce_max() <= 255;
    };
    let mut peak = u16x16::splat(simd, 0);
    for chunk in 0..len.div_ceil(32) {
        let at = (32 * chunk).min(last);
        let (Some(lanes), Some(out)) = (
            row.get(at..).and_then(<[u16]>::first_chunk::<32>),
            staged.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<32>),
        ) else {
            return false;
        };
        peak = peak.max(narrow(lanes, out));
    }
    peak.reduce_max() <= 255
}

/// Output samples the byte-lane kernel writes.
pub(crate) trait ByteLaneOutput: Copy {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn store<const N: usize>(bytes: [u8; N], output: &mut [Self; N]);
}

impl ByteLaneOutput for u8 {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[inline]
    fn store<const N: usize>(bytes: [u8; N], output: &mut [Self; N]) {
        *output = bytes;
    }
}

impl ByteLaneOutput for u16 {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[inline]
    fn store<const N: usize>(bytes: [u8; N], output: &mut [Self; N]) {
        for (slot, byte) in output.iter_mut().zip(bytes) {
            *slot = u16::from(byte);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::WIENER_NS_CONFIG_Y_PAIRS;
    use super::*;

    /// Rows of every width 1..=64, in range and with one sample above 255
    /// at each position, narrow like the per-sample `as u8` oracle.
    #[test]
    fn narrow_row_matches_the_sample_oracle() {
        for len in 1..=64 {
            let row: Vec<u16> = (0..len)
                .map(|i| [255, 0, (i * 37 % 256) as u16][i % 3])
                .collect();
            for spike in [None].into_iter().chain((0..len).map(Some)) {
                let mut row = row.clone();
                if let Some(at) = spike {
                    row[at] = 256 + at as u16;
                }
                let mut staged = vec![7u8; len + 1];
                let fits = narrow_row(&mut staged[..len], &row);
                assert_eq!(fits, spike.is_none(), "{len} {spike:?}");
                if spike.is_none() {
                    let want: Vec<u8> = row.iter().map(|&sample| sample as u8).collect();
                    assert_eq!(&staged[..len], &want[..], "{len}");
                }
                assert_eq!(staged[len], 7, "{len}");
            }
        }
    }

    /// Every vector the kernel fills holds the § 7.20.3 sum of its class,
    /// for random and extreme classes, cell maps, samples, and widths. On
    /// aarch64 every class prepares and every whole vector is filled.
    #[test]
    fn kernel_vectors_match_the_scalar_sum() {
        const MIN: [i16; WIENER_NS_LUMA_COEFFS] = [
            -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
        ];
        const K: [u32; WIENER_NS_LUMA_COEFFS] = [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |limit: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % limit
        };
        let r = WIENER_NS_LUMA_TAP_RADIUS;
        let mut luma = EightBitLuma::default();
        let mut filled = 0;
        for case in 0..60 {
            let width = [16, 32, 48, 64, 80, 40, 128][case % 7];
            let height = 1 + next(12) as usize;
            let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = if case == 1 {
                (0..16)
                    .map(|class| [39, 39, 17, 0, class, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                    .collect()
            } else {
                (0..=next(16))
                    .map(|_| {
                        let style = next(4);
                        core::array::from_fn(|j| {
                            let span = 1u64 << K[j];
                            MIN[j] + [0, span - 1, next(span), next(span)][style as usize] as i16
                        })
                    })
                    .collect()
            };
            let rows: Vec<Vec<u16>> = (0..height + 2 * r)
                .map(|_| {
                    let style = next(3);
                    (0..width + 2 * r)
                        .map(|_| match style {
                            0 => [0, 255][next(2) as usize],
                            _ => next(256) as u16,
                        })
                        .collect()
                })
                .collect();
            let cols = width / 4;
            let values: Vec<u8> = (0..cols * height.div_ceil(4))
                .map(|_| next(coeffs.len() as u64) as u8)
                .collect();
            let layout = if case % 2 == 0 {
                LumaSubclassLayout::Uniform
            } else {
                LumaSubclassLayout::Cells {
                    values: &values,
                    cols,
                }
            };
            assert!(luma.stage(|index| rows.get(index).map(Vec::as_slice), width, height));
            let prepared = luma.prepare(width, &coeffs, layout);
            #[cfg(target_arch = "aarch64")]
            assert!(prepared, "case {case}");
            if case == 1 && kernel_available() {
                assert!(prepared);
            }
            if !prepared {
                continue;
            }
            let mut output = vec![0u8; width * height];
            for first in (0..height).step_by(4) {
                let end = (first + 4).min(height);
                let done = luma.filter_cell_rows(
                    &mut output[first * width..],
                    width,
                    (first, end, first),
                    width,
                    layout,
                    &FlatChunks::new(),
                );
                #[cfg(target_arch = "aarch64")]
                assert_eq!(done, width / 16 * 16, "case {case}");
                if case == 1 {
                    assert_eq!(done, width);
                }
                filled += done;
                for y in first..end {
                    for x in 0..done {
                        let class = match layout {
                            LumaSubclassLayout::Cells { values, cols } => {
                                usize::from(values[(y / 4) * cols + x / 4])
                            }
                            _ => 0,
                        };
                        let c = &coeffs[class];
                        let at = |dy: isize, dx: isize| {
                            i32::from(
                                rows[r.wrapping_add_signed(dy) + y][r.wrapping_add_signed(dx) + x],
                            )
                        };
                        let sum: i32 = c.iter().map(|&c| i32::from(c)).sum();
                        let mut total = (128 - 2 * sum) * at(0, 0) + 64;
                        for &(dy, dx, j) in &WIENER_NS_CONFIG_Y_PAIRS {
                            total += i32::from(c[j]) * (at(dy, dx) + at(-dy, -dx));
                        }
                        let expected = (total >> 7).clamp(0, 255) as u8;
                        assert_eq!(output[y * width + x], expected, "case {case} ({x}, {y})");
                    }
                }
            }
        }
        #[cfg(target_arch = "aarch64")]
        assert!(filled > 0);
        let _ = filled;
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    use fearless_simd::{Avx2, SimdBase, SimdInto, u8x32};

    use super::{
        ByteLaneOutput, EightBitLuma, FlatChunks, GROUP_ROWS, LANES, LumaSubclassLayout,
        STAGE_STRIDE, UNITS, UnitWeights, WIENER_NS_LUMA_TAP_RADIUS,
    };

    const PAIR_UNITS: usize = 10;
    /// Source bytes one vector row reads.
    const REACH: usize = LANES + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    const INTERLEAVED: usize = UNITS - PAIR_UNITS;
    type Row = [u8; STAGE_STRIDE];

    /// The weight vectors of one 32-column group: one per adjacent-tap unit,
    /// and per interleaved unit one for each unpack half.
    #[derive(Clone, Copy)]
    struct Weights {
        pairs: [__m256i; PAIR_UNITS],
        low: [__m256i; INTERLEAVED],
        high: [__m256i; INTERLEAVED],
    }

    #[inline]
    fn at<const O: usize>(row: &Row, x: usize) -> &[u8; LANES] {
        row.get(x + O..)
            .and_then(<[u8]>::first_chunk::<LANES>)
            .unwrap_or(&[0; LANES])
    }

    fearless_simd::kernel!(
        #[inline]
        fn uniform_weights(t: Avx2, weights: &UnitWeights) -> Weights {
            let _ = t;
            let splat =
                |pair: [i8; 2]| _mm256_set1_epi16(i16::from_le_bytes(pair.map(|w| w as u8)));
            let interleaved: [__m256i; INTERLEAVED] =
                core::array::from_fn(|unit| splat(weights[PAIR_UNITS + unit]));
            Weights {
                pairs: core::array::from_fn(|unit| splat(weights[unit])),
                low: interleaved,
                high: interleaved,
            }
        }
    );

    fearless_simd::kernel!(
        #[inline]
        fn mixed_weights(t: Avx2, tables: &[[u8; 32]; UNITS], cells: &[u8; 8]) -> Weights {
            let classes = _mm256_cvtepu8_epi32(_mm_cvtsi64_si128(i64::from_le_bytes(*cells)));
            let patterns = [
                classes,
                _mm256_shuffle_epi32::<0x50>(classes),
                _mm256_shuffle_epi32::<0xfa>(classes),
            ];
            let selectors = patterns.map(|pattern| {
                let odd = _mm256_and_si256(pattern, _mm256_set1_epi32(1));
                let high = _mm256_sub_epi32(_mm256_setzero_si256(), odd);
                let offset = _mm256_and_si256(high, _mm256_set1_epi8(2));
                let bytes = _mm256_setr_epi32(
                    0x0100_0100,
                    0x0504_0504,
                    0x0908_0908,
                    0x0d0c_0d0c,
                    0x0100_0100,
                    0x0504_0504,
                    0x0908_0908,
                    0x0d0c_0d0c,
                );
                (
                    _mm256_srli_epi32::<1>(pattern),
                    _mm256_or_si256(bytes, offset),
                )
            });
            let weights = |unit: usize, set: usize| {
                let table: __m256i = u8x32::load_array_ref(t, &tables[unit]).into();
                let (dwords, bytes) = selectors[set];
                _mm256_shuffle_epi8(_mm256_permutevar8x32_epi32(table, dwords), bytes)
            };
            Weights {
                pairs: core::array::from_fn(|unit| weights(unit, 0)),
                low: core::array::from_fn(|unit| weights(PAIR_UNITS + unit, 1)),
                high: core::array::from_fn(|unit| weights(PAIR_UNITS + unit, 2)),
            }
        }
    );

    fearless_simd::kernel!(
        /// One row of 32 outputs from columns `x..x + REACH` of the nine
        /// source rows `rows[dy + 4]`, which start four columns left of the
        /// first output; `x + REACH` is at most the row size.
        #[inline]
        fn filter_row(t: Avx2, rows: &[Row; 9], x: usize, w: &Weights) -> __m256i {
            let x = x.min(STAGE_STRIDE - REACH);
            macro_rules! ld {
                ($row:literal, $col:literal) => {
                    __m256i::from(u8x32::load_array_ref(t, at::<$col>(&rows[$row], x)))
                };
            }
            let madd = |bytes: __m256i, weights: __m256i| _mm256_maddubs_epi16(bytes, weights);
            let add = |a: __m256i, b: __m256i| _mm256_add_epi16(a, b);
            let interleaved = |a: __m256i, b: __m256i, unit: usize| {
                (
                    madd(_mm256_unpacklo_epi8(a, b), w.low[unit]),
                    madd(_mm256_unpackhi_epi8(a, b), w.high[unit]),
                )
            };
            let finish = |bias: i16, even: __m256i, odd: __m256i, parts: &[(__m256i, __m256i)]| {
                let (mut low, mut high) = (
                    add(_mm256_set1_epi16(bias), _mm256_unpacklo_epi16(even, odd)),
                    add(_mm256_set1_epi16(bias), _mm256_unpackhi_epi16(even, odd)),
                );
                for &(l, h) in parts {
                    low = add(low, l);
                    high = add(high, h);
                }
                (low, high)
            };

            let even = add(
                add(madd(ld!(2, 3), w.pairs[0]), madd(ld!(3, 2), w.pairs[1])),
                add(madd(ld!(3, 4), w.pairs[2]), madd(ld!(4, 0), w.pairs[3])),
            );
            let even = add(even, madd(ld!(4, 3), w.pairs[4]));
            let odd = add(
                add(madd(ld!(2, 4), w.pairs[0]), madd(ld!(3, 3), w.pairs[1])),
                add(madd(ld!(3, 5), w.pairs[2]), madd(ld!(4, 1), w.pairs[3])),
            );
            let odd = add(odd, madd(ld!(4, 4), w.pairs[4]));
            let upper = finish(
                (0x8000u16 + 64) as i16,
                even,
                odd,
                &[
                    interleaved(ld!(0, 4), ld!(1, 4), 0),
                    interleaved(ld!(1, 1), ld!(1, 7), 1),
                    interleaved(ld!(2, 5), ld!(3, 6), 2),
                    interleaved(ld!(4, 2), ld!(4, 6), 3),
                ],
            );

            let even = add(
                add(madd(ld!(6, 4), w.pairs[5]), madd(ld!(5, 3), w.pairs[6])),
                add(madd(ld!(5, 5), w.pairs[7]), madd(ld!(4, 7), w.pairs[8])),
            );
            let even = add(even, madd(ld!(4, 4), w.pairs[9]));
            let odd = add(
                add(madd(ld!(6, 5), w.pairs[5]), madd(ld!(5, 4), w.pairs[6])),
                add(madd(ld!(5, 6), w.pairs[7]), madd(ld!(4, 8), w.pairs[8])),
            );
            let odd = add(odd, madd(ld!(4, 5), w.pairs[9]));
            let lower = finish(
                i16::MAX,
                even,
                odd,
                &[
                    interleaved(ld!(8, 4), ld!(7, 4), 0),
                    interleaved(ld!(7, 7), ld!(7, 1), 1),
                    interleaved(ld!(6, 3), ld!(5, 2), 2),
                ],
            );

            let center = ld!(4, 4);
            let zero = _mm256_setzero_si256();
            let bias = _mm256_set1_epi16(i16::MIN);
            let value = |upper: __m256i, lower: __m256i, center: __m256i| {
                let average = _mm256_avg_epu16(upper, lower);
                add(
                    _mm256_srai_epi16::<6>(_mm256_xor_si256(average, bias)),
                    center,
                )
            };
            _mm256_packus_epi16(
                value(upper.0, lower.0, _mm256_unpacklo_epi8(center, zero)),
                value(upper.1, lower.1, _mm256_unpackhi_epi8(center, zero)),
            )
        }
    );

    #[allow(clippy::too_many_arguments)]
    pub(super) fn filter_cell_rows<O: ByteLaneOutput>(
        t: Avx2,
        luma: &EightBitLuma,
        output: &mut [O],
        output_stride: usize,
        (first, end, origin): (usize, usize, usize),
        width: usize,
        layout: LumaSubclassLayout<'_>,
        flat: &FlatChunks<WIENER_NS_LUMA_TAP_RADIUS>,
    ) -> usize {
        let Some(group) = luma
            .staged
            .get(first..)
            .and_then(<[Row]>::first_chunk::<GROUP_ROWS>)
        else {
            return 0;
        };
        let mut x = 0;
        while x + LANES <= width && x + REACH <= STAGE_STRIDE {
            if let Some(value) = flat.chunk_value(x / LANES) {
                let value = u8::try_from(value).unwrap_or(u8::MAX);
                for r in first..end {
                    let Some(target) = output
                        .get_mut((r - origin) * output_stride + x..)
                        .and_then(<[O]>::first_chunk_mut::<LANES>)
                    else {
                        return x;
                    };
                    O::store([value; LANES], target);
                }
                x += LANES;
                continue;
            }
            let weights = match layout {
                LumaSubclassLayout::Uniform => match luma.classes.first() {
                    Some(class) => uniform_weights(t, class),
                    None => break,
                },
                LumaSubclassLayout::Cells { values, cols } => {
                    let Some(cells) = values
                        .get((first / 4) * cols + x / 4..)
                        .and_then(<[u8]>::first_chunk::<8>)
                    else {
                        break;
                    };
                    if *cells == [cells[0]; 8] {
                        let Some(class) = luma.classes.get(usize::from(cells[0])) else {
                            break;
                        };
                        uniform_weights(t, class)
                    } else {
                        mixed_weights(t, &luma.tables, cells)
                    }
                }
                LumaSubclassLayout::Samples(_) => break,
            };
            for r in first..end {
                let Some(rows) = group.get(r - first..).and_then(<[Row]>::first_chunk::<9>) else {
                    return x;
                };
                let Some(target) = output
                    .get_mut((r - origin) * output_stride + x..)
                    .and_then(<[O]>::first_chunk_mut::<LANES>)
                else {
                    return x;
                };
                let bytes: u8x32<Avx2> = filter_row(t, rows, x, &weights).simd_into(t);
                O::store(bytes.into(), target);
            }
            x += LANES;
        }
        x
    }
}

/// The tap-pair kernel of the module docs, 16 outputs of one row per vector.
/// It uses only `fearless_simd` operations; x86 keeps the byte-pair kernel.
#[cfg(target_arch = "aarch64")]
mod paired {
    use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16};

    use super::{
        ByteLaneOutput, EightBitLuma, FlatChunks, GROUP_ROWS, LumaSubclassLayout, PAIR_ORDER,
        PairWeights, STAGE_STRIDE, WIENER_NS_CONFIG_Y_PAIRS, WIENER_NS_LUMA_COEFFS,
        WIENER_NS_LUMA_TAP_RADIUS,
    };

    const LANES: usize = 16;
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    /// Source bytes one vector row reads.
    const REACH: usize = LANES + 2 * R;
    const SLOTS: usize = WIENER_NS_LUMA_COEFFS + 1;
    type Row = [u8; STAGE_STRIDE];

    #[inline]
    fn at(row: &Row, x: usize) -> &[u8; LANES] {
        row.get(x..)
            .and_then(<[u8]>::first_chunk::<LANES>)
            .unwrap_or(&[0; LANES])
    }

    /// One row of 16 outputs from columns `x..x + REACH` of the nine source
    /// rows `rows[dy + 4]`; `weight(slot, half)` gives the [`PairWeights`]
    /// slot of outputs `8 * half..8 * half + 8`.
    #[allow(
        clippy::inline_always,
        reason = "the row kernel must inline into its row loop"
    )]
    #[inline(always)]
    fn filter_row<S: Simd>(
        simd: S,
        rows: &[Row; 9],
        x: usize,
        weight: &impl Fn(usize, usize) -> i16x8<S>,
    ) -> u8x16<S> {
        let x = x.min(STAGE_STRIDE - REACH);
        let load = |dy: isize, dx: isize| {
            u8x16::load_array_ref(
                simd,
                at(
                    &rows[R.wrapping_add_signed(dy)],
                    x + R.wrapping_add_signed(dx),
                ),
            )
        };
        let mut groups = [i16x8::splat(simd, 0); 8];
        // A literal slot makes every tap offset a constant.
        macro_rules! pairs {
            ($($slot:literal)*) => {$({
                const TAP: (isize, isize, usize) = WIENER_NS_CONFIG_Y_PAIRS[PAIR_ORDER[$slot]];
                let (low, high) = load(TAP.0, TAP.1).widening_add(load(-TAP.0, -TAP.1));
                groups[$slot / 4 * 2] += low.bitcast::<i16x8<S>>() * weight($slot, 0);
                groups[$slot / 4 * 2 + 1] += high.bitcast::<i16x8<S>>() * weight($slot, 1);
            })*};
        }
        pairs!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
        let (center_low, center_high) = load(0, 0).widen();
        let half = |half: usize, center: i16x8<S>| {
            let (g0, g1) = groups[half].widening_add(groups[2 + half]);
            let (g2, g3) = groups[4 + half].widening_add(groups[6 + half]);
            let (m0, m1) = center.widening_mul(weight(WIENER_NS_LUMA_COEFFS, half));
            let round = i32x4::splat(simd, 64);
            let value = simd
                .saturating_narrow_i32x4((g0 + g2 + m0 + round) >> 7, (g1 + g3 + m1 + round) >> 7)
                .max(i16x8::splat(simd, 0));
            value.bitcast()
        };
        simd.saturating_narrow_u16x8(
            half(0, center_low.bitcast()),
            half(1, center_high.bitcast()),
        )
    }

    /// The x86 `filter_cell_rows` contract over 16-column vectors.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn filter_cell_rows<S: Simd, O: ByteLaneOutput>(
        simd: S,
        luma: &EightBitLuma,
        output: &mut [O],
        output_stride: usize,
        (first, end, origin): (usize, usize, usize),
        width: usize,
        layout: LumaSubclassLayout<'_>,
        flat: &FlatChunks<WIENER_NS_LUMA_TAP_RADIUS>,
    ) -> usize {
        let Some(group) = luma
            .staged
            .get(first..)
            .and_then(<[Row]>::first_chunk::<GROUP_ROWS>)
        else {
            return 0;
        };
        let rows = (first, end, origin);
        let mut x = 0;
        while x + LANES <= width && x + REACH <= STAGE_STRIDE {
            if let Some(value) = flat.chunk_value(x / 32) {
                let value = u8::try_from(value).unwrap_or(u8::MAX);
                for r in first..end {
                    let Some(target) = output
                        .get_mut((r - origin) * output_stride + x..)
                        .and_then(<[O]>::first_chunk_mut::<LANES>)
                    else {
                        return x;
                    };
                    O::store([value; LANES], target);
                }
                x += LANES;
                continue;
            }
            let cells: [Option<&PairWeights>; 4] = match layout {
                LumaSubclassLayout::Uniform => [luma.classes.first(); 4],
                LumaSubclassLayout::Cells { values, cols } => {
                    let Some(cells) = values
                        .get((first / 4) * cols + x / 4..)
                        .and_then(<[u8]>::first_chunk::<4>)
                    else {
                        break;
                    };
                    cells.map(|class| luma.classes.get(usize::from(class)))
                }
                LumaSubclassLayout::Samples(_) => break,
            };
            let [Some(c0), Some(c1), Some(c2), Some(c3)] = cells else {
                break;
            };
            let uniform = core::ptr::eq(c0, c1) && core::ptr::eq(c1, c2) && core::ptr::eq(c2, c3);
            let done = if uniform {
                let weight = |slot: usize, _| i16x8::splat(simd, c0[slot]);
                filter_rows(simd, group, (output, output_stride), rows, x, &weight)
            } else {
                let cells = [c0, c1, c2, c3];
                let mixed: [[[i16; 8]; 2]; SLOTS] = core::array::from_fn(|slot| {
                    core::array::from_fn(|half| {
                        core::array::from_fn(|lane| cells[2 * half + lane / 4][slot])
                    })
                });
                let weight =
                    |slot: usize, half: usize| i16x8::load_array_ref(simd, &mixed[slot][half]);
                filter_rows(simd, group, (output, output_stride), rows, x, &weight)
            };
            if !done {
                return x;
            }
            x += LANES;
        }
        x
    }

    /// Filters the vector at column `x` of the rows `first..end`; returns
    /// `false`, the vector unfinished, when an output row is short.
    #[allow(
        clippy::inline_always,
        reason = "the weights must stay in registers across the rows"
    )]
    #[inline(always)]
    fn filter_rows<S: Simd, O: ByteLaneOutput>(
        simd: S,
        group: &[Row; GROUP_ROWS],
        (output, output_stride): (&mut [O], usize),
        (first, end, origin): (usize, usize, usize),
        x: usize,
        weight: &impl Fn(usize, usize) -> i16x8<S>,
    ) -> bool {
        for r in first..end {
            let (Some(rows), Some(target)) = (
                group.get(r - first..).and_then(<[Row]>::first_chunk::<9>),
                output
                    .get_mut((r - origin) * output_stride + x..)
                    .and_then(<[O]>::first_chunk_mut::<LANES>),
            ) else {
                return false;
            };
            O::store(filter_row(simd, rows, x, weight).into(), target);
        }
        true
    }
}
