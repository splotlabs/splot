// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 7.13.5 4:2:0 8-bit CfL luma `L` of a whole block in one AVX2
//! pass: `pmaddubsw` pairs the centre and right taps of each luma row, and
//! the left tap where the filter has one. Also the CfL prediction from `L`
//! in 16-bit lanes.

use super::cfl::CFL_FILTERS_420;

/// A block whose every luma tap lies inside the stored plane, so no tap
/// clamps at a frame edge.
#[derive(Clone, Copy)]
pub(super) struct Luma420Block {
    /// Index of luma sample `(2 * x, 2 * y)` in the plane samples.
    pub(super) origin: usize,
    pub(super) stride: usize,
    /// Luma coordinates of the block origin, for the § 7.13.5 64-sample clamps.
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    /// Chroma block width.
    pub(super) width: usize,
    /// The CfL `L` clamps: the first row and column, and every 64th luma
    /// row and column; otherwise only the first row or column clamps, when
    /// [`Self::clamp_row0`] or [`Self::clamp_col0`].
    pub(super) periodic: bool,
    pub(super) clamp_row0: bool,
    pub(super) clamp_col0: bool,
    /// The top, centre and bottom tap rows (as indices of luma column
    /// `2 * x`) of an extra first output row, which takes no row clamp: the
    /// § 7.13.5 `lumaAbove` row above the block. Its left sample is not
    /// written.
    pub(super) above: Option<[usize; 3]>,
}

impl Luma420Block {
    /// Whether every load of a block of `height` rows below any
    /// [`Self::above`] row stays inside `len` samples.
    #[allow(
        clippy::inline_always,
        reason = "measured: out of line it costs more than its body"
    )]
    #[inline(always)]
    pub(super) fn fits(&self, height: usize, len: usize) -> bool {
        let row_reach = if self.width <= 8 {
            16
        } else {
            2 * self.width.next_multiple_of(16)
        };
        self.luma_x >= 2
            && self.first_tap_row().is_some_and(|first| first >= 3)
            && self.origin + (2 * height - 1) * self.stride + row_reach <= len
    }

    /// The first luma row any tap reads.
    fn first_tap_row(&self) -> Option<usize> {
        let first = if self.clamp_row0 {
            Some(self.origin)
        } else {
            self.origin.checked_sub(self.stride)
        };
        match self.above {
            Some([top, ..]) => first.map(|first| first.min(top)),
            None => first,
        }
    }
}

/// The luma rows of tap rows top, centre and bottom of output row `r`.
#[inline]
fn tap_rows(block: &Luma420Block, r: usize) -> [usize; 3] {
    let r = match block.above {
        Some(rows) if r == 0 => return rows,
        Some(_) => r - 1,
        None => r,
    };
    let mid = block.origin + 2 * r * block.stride;
    let clamp = (r == 0 && block.clamp_row0)
        || (block.periodic && (block.luma_y + 2 * r).is_multiple_of(64));
    [
        if clamp { mid } else { mid - block.stride },
        mid,
        mid + block.stride,
    ]
}

/// The § 7.13.5 sum of `weights[dy + 1][dx + 1]` around the centre samples
/// `centres` (top, middle, bottom row); the left tap reads the centre when
/// `clamp_left`.
#[allow(clippy::inline_always, reason = "folds the constant filter taps")]
#[inline(always)]
fn tap_sum(luma: &[u8], weights: &[[i32; 3]; 3], centres: [usize; 3], clamp_left: bool) -> i32 {
    let mut total = 0;
    for (&centre, weights) in centres.iter().zip(weights) {
        let cols = [centre - usize::from(!clamp_left), centre, centre + 1];
        for (&col, &weight) in cols.iter().zip(weights) {
            if weight != 0 {
                total += weight * i32::from(luma[col]);
            }
        }
    }
    total
}

/// `pmaddubsw` byte-pair weights of each tap row: the (centre, right) pair
/// read from the centre column and the (left, centre) pair from one before;
/// with `clamped`, the left tap folded into the centre.
const fn pair_taps(weights: &[[i32; 3]; 3], clamped: bool) -> [(i16, i16); 3] {
    let mut taps = [(0i16, 0i16); 3];
    let mut row = 0;
    while row < 3 {
        let [left, centre, right] = weights[row];
        let (left, centre) = if clamped {
            (0, centre + left)
        } else {
            (left, centre)
        };
        taps[row] = (
            ((centre as u8) as i16) | (((right as u8) as i16) << 8),
            (left as u8) as i16,
        );
        row += 1;
    }
    taps
}

/// `LANE_TAPS[filter][variant][pair][row]`: the 16 `pmaddubsw` lanes of the
/// [`pair_taps`] (centre, right) (`pair` 0) and (left, centre) (`pair` 1)
/// weights of each tap row: unclamped (`variant` 0), or with the left tap of
/// lane 0 (`variant` 1) or lane 8 (`variant` 2) clamped.
static LANE_TAPS: [[[[[i16; 16]; 3]; 2]; 3]; 3] = {
    let mut table = [[[[[0i16; 16]; 3]; 2]; 3]; 3];
    let mut filter = 0;
    while filter < 3 {
        let plain = pair_taps(&CFL_FILTERS_420[filter], false);
        let clamped = pair_taps(&CFL_FILTERS_420[filter], true);
        let mut variant = 0;
        while variant < 3 {
            let mut row = 0;
            while row < 3 {
                let mut lane = 0;
                while lane < 16 {
                    let taps = if variant > 0 && lane == 8 * (variant - 1) {
                        clamped[row]
                    } else {
                        plain[row]
                    };
                    table[filter][variant][0][row][lane] = taps.0;
                    table[filter][variant][1][row][lane] = taps.1;
                    lane += 1;
                }
                row += 1;
            }
            variant += 1;
        }
        filter += 1;
    }
    table
};

/// Writes `L` of the block rows with the 4:2:0 filter `F` into `out` (row
/// `r` at `r * width`) and, when given, the downsampled luma left of each
/// row into `left`; `false`, writing nothing, for a filter outside 4:2:0.
/// Clamped columns on a 16-lane group start clamp in the lane pass, the
/// rest per sample; blocks up to 8 wide whose taps reach 17 samples before
/// the first tap row take the left column from lane 7.
#[inline]
pub(super) fn luma_420<const F: usize>(
    token: fearless_simd::Avx2,
    luma: &[u8],
    block: Luma420Block,
    mut left: Option<&mut [i16]>,
    out: &mut [i16],
) -> bool {
    let Some(&weights) = CFL_FILTERS_420.get(F) else {
        return false;
    };
    let (luma_x, width) = (block.luma_x, block.width);
    let clamp_cols = weights.iter().any(|row| row[0] != 0) && block.clamp_col0;
    let periodic = clamp_cols && block.periodic;
    let first_periodic = (32 - (luma_x / 2) % 32) % 32;
    let first_periodic = if first_periodic == 0 {
        32
    } else {
        first_periodic
    };
    let mut clamp_groups = u8::from(clamp_cols);
    let mut lone_columns = periodic && first_periodic < width;
    if periodic && width > 8 && first_periodic.is_multiple_of(16) {
        let mut j = first_periodic;
        while j < width {
            clamp_groups |= 1 << (j / 16);
            j += 32;
        }
        lone_columns = false;
    }
    let lane_left = width <= 8
        && luma_x >= 4
        && block.first_tap_row().is_some_and(|row| row >= 17)
        && left.is_some();
    let top_taps = weights[0] != [0; 3];
    let stride = block.stride;
    let first_mid = tap_rows(&block, 0)[1];
    let paired = !top_taps
        && lane_left
        && stride >= 64
        && block.above.is_none_or(|[_, mid, bottom]| {
            mid + 2 * stride == block.origin && bottom == mid + stride
        });
    let pairs = first_mid
        .checked_sub(17)
        .and_then(|start| luma.get(start..))
        .filter(|rows| paired && rows.len() >= 2 * stride * (out.len() >> width.trailing_zeros()));
    match (F, pairs, left.as_deref_mut()) {
        (0, Some(rows), Some(left)) => {
            luma_420_pairs_f0(token, clamp_groups, rows, block, left, out);
        }
        (1, Some(rows), Some(left)) => {
            luma_420_pairs_f1(token, clamp_groups, rows, block, left, out);
        }
        (_, _, left) => {
            let lane_out = if lane_left { left } else { None };
            match F {
                0 => luma_420_rows_f0(token, clamp_groups, luma, block, lane_out, out),
                1 => luma_420_rows_f1(token, clamp_groups, luma, block, lane_out, out),
                _ => luma_420_rows_f2(token, clamp_groups, luma, block, lane_out, out),
            }
        }
    }
    if !lone_columns && (left.is_none() || (lane_left && !top_taps)) {
        return true;
    }
    let first = usize::from(block.above.is_some());
    for (r, out) in out.chunks_exact_mut(width).enumerate() {
        let rows = tap_rows(&block, r);
        if lone_columns {
            let mut j = first_periodic;
            while j < width {
                out[j] = tap_sum(luma, &weights, rows.map(|row| row + 2 * j), true) as i16;
                j += 32;
            }
        }
        if let Some(slot) = left
            .as_deref_mut()
            .and_then(|left| left.get_mut(r.wrapping_sub(first)))
        {
            let [lane_top, mid, bottom] = rows;
            let top = if r == first { mid } else { mid - block.stride };
            if !lane_left || (top_taps && top != lane_top) {
                *slot = tap_sum(
                    luma,
                    &weights,
                    [top, mid, bottom].map(|row| row - 2),
                    luma_x == 2,
                ) as i16;
            }
        }
    }
    true
}

/// Defines the `pmaddubsw` part of [`luma_420`] for one constant filter.
macro_rules! luma_420_rows {
    ($name:ident, $filter:literal) => {
        fearless_simd::kernel!(
            /// The `pmaddubsw` part of [`luma_420`]: the first sample of the
            /// 16-lane groups set in `clamp_groups` (bit `g / 16`; bit 0 alone for
            /// blocks up to 8 wide) with its left tap clamped. With `left`, blocks up
            /// to 8 wide run 16 lanes from 8 samples before the block, and lane 7 is
            /// the sample left of each row, with that row's taps.
            #[inline]
            fn $name(
                token: Avx2,
                clamp_groups: u8,
                luma: &[u8],
                block: Luma420Block,
                left: Option<&mut [i16]>,
                out: &mut [i16],
            ) {
                let mut left = left;
                let taps = &LANE_TAPS[$filter];
                use core::arch::x86_64::{
                    __m128i, __m256i, _mm_add_epi16, _mm_maddubs_epi16, _mm256_add_epi16,
                    _mm256_castsi256_si128, _mm256_extract_epi16, _mm256_extracti128_si256,
                    _mm256_maddubs_epi16, _mm256_setzero_si256,
                };
                use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u8x16, u8x32};
                let width = block.width;
                let lanes = |clamped: usize, pair: usize, row: usize| -> __m256i {
                    i16x16::from_slice(token, &taps[clamped][pair][row]).into()
                };
                let [[plain_centre, plain_left], ..] = taps;
                let used = |pair: &[[i16; 16]; 3], row: usize| pair[row][1] != 0;
                let first = usize::from(clamp_groups & 1);
                let skip = usize::from(block.above.is_some());
                for (r, out) in out.chunks_exact_mut(width).enumerate() {
                    let rows = tap_rows(&block, r);
                    if width > 8 {
                        for g in (0..width).step_by(16) {
                            let clamped = usize::from(clamp_groups >> (g / 16) & 1);
                            let mut acc = _mm256_setzero_si256();
                            for (i, &row) in rows.iter().enumerate() {
                                let at = row + 2 * g;
                                if used(plain_centre, i) {
                                    let bytes: __m256i =
                                        u8x32::from_slice(token, &luma[at..at + 32]).into();
                                    acc = _mm256_add_epi16(
                                        acc,
                                        _mm256_maddubs_epi16(bytes, lanes(clamped, 0, i)),
                                    );
                                }
                                if used(plain_left, i) {
                                    let bytes: __m256i =
                                        u8x32::from_slice(token, &luma[at - 1..at + 31]).into();
                                    acc = _mm256_add_epi16(
                                        acc,
                                        _mm256_maddubs_epi16(bytes, lanes(clamped, 1, i)),
                                    );
                                }
                            }
                            let acc = i16x16::simd_from(token, acc);
                            if let Some(out) = out.get_mut(g..g + 16) {
                                acc.store_slice(out);
                            } else {
                                for (slot, &lane) in out[g..].iter_mut().zip(acc.as_slice()) {
                                    *slot = lane;
                                }
                            }
                        }
                    } else if let Some(left) = left.as_deref_mut() {
                        let clamped = 2 * first;
                        let mut acc = _mm256_setzero_si256();
                        for (i, &row) in rows.iter().enumerate() {
                            let at = row - 16;
                            if used(plain_centre, i) {
                                let bytes: __m256i =
                                    u8x32::from_slice(token, &luma[at..at + 32]).into();
                                acc = _mm256_add_epi16(
                                    acc,
                                    _mm256_maddubs_epi16(bytes, lanes(clamped, 0, i)),
                                );
                            }
                            if used(plain_left, i) {
                                let bytes: __m256i =
                                    u8x32::from_slice(token, &luma[at - 1..at + 31]).into();
                                acc = _mm256_add_epi16(
                                    acc,
                                    _mm256_maddubs_epi16(bytes, lanes(clamped, 1, i)),
                                );
                            }
                        }
                        if let Some(slot) = left.get_mut(r.wrapping_sub(skip)) {
                            *slot = _mm256_extract_epi16::<7>(acc) as i16;
                        }
                        store_row8(token, _mm256_extracti128_si256::<1>(acc), out);
                    } else {
                        let clamped = first;
                        let mut acc: __m128i = u8x16::splat(token, 0).into();
                        for (i, &row) in rows.iter().enumerate() {
                            if used(plain_centre, i) {
                                let bytes: __m128i =
                                    u8x16::from_slice(token, &luma[row..row + 16]).into();
                                acc = _mm_add_epi16(
                                    acc,
                                    _mm_maddubs_epi16(
                                        bytes,
                                        _mm256_castsi256_si128(lanes(clamped, 0, i)),
                                    ),
                                );
                            }
                            if used(plain_left, i) {
                                let bytes: __m128i =
                                    u8x16::from_slice(token, &luma[row - 1..row + 15]).into();
                                acc = _mm_add_epi16(
                                    acc,
                                    _mm_maddubs_epi16(
                                        bytes,
                                        _mm256_castsi256_si128(lanes(clamped, 1, i)),
                                    ),
                                );
                            }
                        }
                        store_row8(token, acc, out);
                    }
                }
            }
        );
    };
}
luma_420_rows!(luma_420_rows_f0, 0);
luma_420_rows!(luma_420_rows_f1, 1);
luma_420_rows!(luma_420_rows_f2, 2);

/// Defines the paired-row narrow part of [`luma_420`] for one filter without
/// top taps.
macro_rules! luma_420_pairs {
    ($name:ident, $filter:literal) => {
        fearless_simd::kernel!(
            /// The narrow, left-lane part of [`luma_420_rows_f0`] for a filter
            /// without top taps, whose output row `r` reads only the luma rows
            /// `2 * r` and `2 * r + 1` after the first: `rows` starts 17
            /// samples before the first row's centre column and holds two whole
            /// rows of `block.stride` (at least 64) per output row, so each row
            /// pair is split off once and its loads need no bounds checks.
            #[inline]
            fn $name(
                token: Avx2,
                clamp_groups: u8,
                rows: &[u8],
                block: Luma420Block,
                left: &mut [i16],
                out: &mut [i16],
            ) {
                use core::arch::x86_64::{
                    __m256i, _mm256_add_epi16, _mm256_extract_epi16, _mm256_extracti128_si256,
                    _mm256_maddubs_epi16,
                };
                use fearless_simd::{SimdBase as _, i16x16, u8x32};
                let stride = block.stride;
                if stride < 64 {
                    return;
                }
                let taps = &LANE_TAPS[$filter][2 * usize::from(clamp_groups & 1)];
                let lanes = |pair: usize, row: usize| -> __m256i {
                    i16x16::from_slice(token, &taps[pair][row]).into()
                };
                let left_taps = taps[1][1] != [0; 16];
                let (mid_c, mid_l, bottom_c, bottom_l) =
                    (lanes(0, 1), lanes(1, 1), lanes(0, 2), lanes(1, 2));
                let skip = usize::from(block.above.is_some());
                let mut rest = rows;
                for (r, out) in out.chunks_exact_mut(block.width).enumerate() {
                    let Some((pair, tail)) = rest.split_at_checked(2 * stride) else {
                        return;
                    };
                    rest = tail;
                    let (mid, bottom) = pair.split_at(stride);
                    let (Some(mid), Some(bottom)) =
                        (mid.first_chunk::<33>(), bottom.first_chunk::<33>())
                    else {
                        return;
                    };
                    let taps = |row: &[u8; 33], centre: __m256i, left: __m256i| {
                        let bytes: __m256i = u8x32::from_slice(token, &row[1..]).into();
                        let acc = _mm256_maddubs_epi16(bytes, centre);
                        if left_taps {
                            let bytes: __m256i = u8x32::from_slice(token, &row[..32]).into();
                            _mm256_add_epi16(acc, _mm256_maddubs_epi16(bytes, left))
                        } else {
                            acc
                        }
                    };
                    let acc =
                        _mm256_add_epi16(taps(mid, mid_c, mid_l), taps(bottom, bottom_c, bottom_l));
                    if let Some(slot) = left.get_mut(r.wrapping_sub(skip)) {
                        *slot = _mm256_extract_epi16::<7>(acc) as i16;
                    }
                    store_row8(token, _mm256_extracti128_si256::<1>(acc), out);
                }
            }
        );
    };
}
luma_420_pairs!(luma_420_pairs_f0, 0);
luma_420_pairs!(luma_420_pairs_f1, 1);

/// Stores the first `out.len()` (at most 8) lanes of `row`.
#[inline]
fn store_row8(token: fearless_simd::Avx2, row: core::arch::x86_64::__m128i, out: &mut [i16]) {
    use fearless_simd::{SimdFrom as _, i16x8};
    let lanes: [i16; 8] = i16x8::simd_from(token, row).into();
    if let Some(out) = out.first_chunk_mut::<8>() {
        *out = lanes;
    } else if let Ok(out) = <&mut [i16; 4]>::try_from(&mut *out) {
        *out = [lanes[0], lanes[1], lanes[2], lanes[3]];
    } else {
        for (slot, lane) in out.iter_mut().zip(lanes) {
            *slot = lane;
        }
    }
}

/// Whether [`cfl_predict_u8`] and [`cfl_predict_u16`] cover a CfL `alpha`:
/// `16 * |alpha|` must fit a `pmulhrsw` operand.
pub(super) const fn cfl_alpha_fits_i16(alpha: i32) -> bool {
    alpha.unsigned_abs() < 2048
}

/// Sixteen § 7.13.5 CfL samples `dc + Round2Signed(alpha * (L - lumaAvg), 11)`
/// before `Clip1`, from the `L` of `luma[..16]`. With `d = L - lumaAvg` signed
/// by `alpha`, `pmulhrsw` of `|d|` by `16 * |alpha|` is
/// `(16 * |alpha * d| + 2^14) >> 15 = Round2(|alpha * d|, 11)`.
macro_rules! cfl_predict16 {
    ($token:ident, $luma:expr, [$average:ident, $sign:ident, $scale:ident, $dc:ident]) => {{
        let luma: __m256i = i16x16::from_slice($token, &$luma[..16]).into();
        let d = _mm256_sign_epi16(_mm256_sub_epi16(luma, $average), $sign);
        let scaled = _mm256_sign_epi16(_mm256_mulhrs_epi16(_mm256_abs_epi16(d), $scale), d);
        _mm256_add_epi16(scaled, $dc)
    }};
}

/// The broadcast `lumaAvg`, `alpha` sign, `16 * |alpha|` and `dc` of a
/// `[dc, alpha, lumaAvg, max]` model.
macro_rules! cfl_predict_constants {
    ($model:expr) => {{
        let [dc, alpha, average, _] = $model;
        [
            _mm256_set1_epi16(average as i16),
            _mm256_set1_epi16(alpha.signum() as i16),
            _mm256_set1_epi16((alpha.unsigned_abs() << 4) as i16),
            _mm256_set1_epi16(dc as i16),
        ]
    }};
}

/// Writes the rows of a CfL block narrower than 16 samples: each group of
/// sixteen `samples(luma)` covers `16 / $width` rows.
macro_rules! cfl_narrow_rows {
    ($width:literal, $luma:ident, $out:ident, $stride:ident, $samples:ident) => {
        let mut rest = &mut *$out;
        for luma in $luma.chunks_exact(16) {
            let samples = $samples(luma);
            for samples in samples.as_chunks::<$width>().0 {
                if let Some(out) = rest.first_chunk_mut::<$width>() {
                    *out = *samples;
                }
                rest = core::mem::take(&mut rest)
                    .get_mut($stride..)
                    .unwrap_or_default();
            }
        }
    };
}

/// [`cfl_narrow_rows`] for any CfL width: 4, 8, or a multiple of 16.
macro_rules! cfl_rows {
    ($width:ident, $luma:ident, $out:ident, $stride:ident, $samples:ident) => {
        match $width {
            4 => {
                cfl_narrow_rows!(4, $luma, $out, $stride, $samples);
            }
            8 => {
                cfl_narrow_rows!(8, $luma, $out, $stride, $samples);
            }
            _ => {
                let mut rest = &mut *$out;
                for luma in $luma.chunks_exact($width) {
                    let out = rest.get_mut(..$width).unwrap_or_default();
                    for (out, luma) in out
                        .as_chunks_mut::<16>()
                        .0
                        .iter_mut()
                        .zip(luma.chunks_exact(16))
                    {
                        *out = $samples(luma);
                    }
                    rest = core::mem::take(&mut rest)
                        .get_mut($stride..)
                        .unwrap_or_default();
                }
            }
        }
    };
}

fearless_simd::kernel!(
    /// The 8-bit § 7.13.5 CfL prediction `[dc, alpha, lumaAvg, max]` of `L`
    /// (`luma`, rows `width` wide: 4, 8 or a multiple of 16, and a multiple
    /// of 16 samples) into `out`, row `r` at `r * stride`, for an `alpha`
    /// that [`cfl_alpha_fits_i16`]; `packuswb` is the 8-bit `Clip1`.
    #[inline]
    pub(super) fn cfl_predict_u8(
        token: Avx2,
        model: [i32; 4],
        luma: &[i16],
        width: usize,
        out: &mut [u8],
        stride: usize,
    ) {
        use core::arch::x86_64::{
            __m256i, _mm_packus_epi16, _mm256_abs_epi16, _mm256_add_epi16, _mm256_castsi256_si128,
            _mm256_extracti128_si256, _mm256_mulhrs_epi16, _mm256_set1_epi16, _mm256_sign_epi16,
            _mm256_sub_epi16,
        };
        use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u8x16};
        let constants = cfl_predict_constants!(model);
        let [average, sign, scale, dc] = constants;
        let samples = |luma: &[i16]| -> [u8; 16] {
            let v = cfl_predict16!(token, luma, [average, sign, scale, dc]);
            let packed =
                _mm_packus_epi16(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
            u8x16::simd_from(token, packed).into()
        };
        cfl_rows!(width, luma, out, stride, samples);
    }
);

fearless_simd::kernel!(
    /// [`cfl_predict_u8`] for samples up to `model[3]`, at most 10 bits.
    #[inline]
    pub(super) fn cfl_predict_u16(
        token: Avx2,
        model: [i32; 4],
        luma: &[i16],
        width: usize,
        out: &mut [u16],
        stride: usize,
    ) {
        use core::arch::x86_64::{
            __m256i, _mm256_abs_epi16, _mm256_add_epi16, _mm256_max_epi16, _mm256_min_epi16,
            _mm256_mulhrs_epi16, _mm256_set1_epi16, _mm256_setzero_si256, _mm256_sign_epi16,
            _mm256_sub_epi16,
        };
        use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u16x16};
        let [average, sign, scale, dc] = cfl_predict_constants!(model);
        let max = _mm256_set1_epi16(model[3] as i16);
        let zero = _mm256_setzero_si256();
        let samples = |luma: &[i16]| -> [u16; 16] {
            let v = cfl_predict16!(token, luma, [average, sign, scale, dc]);
            u16x16::simd_from(token, _mm256_min_epi16(_mm256_max_epi16(v, zero), max)).into()
        };
        cfl_rows!(width, luma, out, stride, samples);
    }
);
