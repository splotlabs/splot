// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 7.13.5 4:2:0 8-bit CfL luma `L` of a whole block in one AVX2
//! or NEON pass: `pmaddubsw` pairs the centre and right taps of each luma
//! row, and the left tap where the filter has one. Also the CfL prediction
//! from `L` in 16-bit lanes.

use super::cfl::{CFL_FILTERS_420, gathered_q3};
#[cfg(target_arch = "aarch64")]
pub(super) use neon::{cfl_predict_pair_u8, cfl_predict_pair_u16, cfl_predict_u8, cfl_predict_u16};

/// The token of this target's kernels.
#[cfg(target_arch = "x86_64")]
pub(super) type LumaToken = fearless_simd::Avx2;
/// The token of this target's kernels.
#[cfg(target_arch = "aarch64")]
pub(super) type LumaToken = fearless_simd::Neon;

#[cfg(test)]
#[expect(clippy::inline_always, reason = "a token read in hot CfL paths")]
#[inline(always)]
pub(super) fn luma_token() -> Option<LumaToken> {
    #[cfg(target_arch = "x86_64")]
    return fearless_simd::Level::baseline().as_avx2();
    #[cfg(target_arch = "aarch64")]
    return fearless_simd::Level::baseline().as_neon();
}

/// The [`LumaToken`] carried by `backend`, if it has one.
#[expect(clippy::inline_always, reason = "a token read in hot CfL paths")]
#[inline(always)]
pub(super) fn luma_token_for(backend: splot_core::simd::SimdBackend) -> Option<LumaToken> {
    #[cfg(target_arch = "x86_64")]
    return backend.avx2_token();
    #[cfg(target_arch = "aarch64")]
    return backend.neon_token();
}

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
    #[expect(
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

/// `pmaddubsw` byte-pair weights of each tap row: the (centre, right) pair
/// read from the centre column and the (left, centre) pair from one before;
/// with `clamped`, the left tap folded into the centre.
#[cfg(target_arch = "x86_64")]
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

#[cfg(target_arch = "x86_64")]
type LaneTaps = [[[i16; 16]; 3]; 2];

#[cfg(target_arch = "x86_64")]
const fn lane_tap_index(filter: usize, variant: usize) -> usize {
    if filter == 0 {
        0
    } else {
        1 + (filter - 1) * 3 + variant
    }
}

/// `LANE_TAPS[lane_tap_index(filter, variant)][pair][row]`: 16 `pmaddubsw` lanes of the
/// [`pair_taps`] (centre, right) (`pair` 0) and (left, centre) (`pair` 1)
/// weights of each tap row: unclamped (`variant` 0), or with the left tap of
/// lane 0 (`variant` 1) or lane 8 (`variant` 2) clamped. Filter 0 has no
/// left taps, so all three variants share a single entry.
#[cfg(target_arch = "x86_64")]
static LANE_TAPS: [LaneTaps; 7] = {
    let mut table = [[[[0i16; 16]; 3]; 2]; 7];
    let mut filter = 0;
    while filter < 3 {
        let plain = pair_taps(&CFL_FILTERS_420[filter], false);
        let clamped = pair_taps(&CFL_FILTERS_420[filter], true);
        let mut variant = 0;
        let variants = if filter == 0 { 1 } else { 3 };
        while variant < variants {
            let index = lane_tap_index(filter, variant);
            let mut row = 0;
            while row < 3 {
                let mut lane = 0;
                while lane < 16 {
                    let taps = if variant > 0 && lane == 8 * (variant - 1) {
                        clamped[row]
                    } else {
                        plain[row]
                    };
                    table[index][0][row][lane] = taps.0;
                    table[index][1][row][lane] = taps.1;
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
/// row into `left`; `false`, writing nothing, for a filter outside 4:2:0 or
/// a clamped column off a 16-lane group start. Blocks up to 8 wide whose
/// taps reach 17 samples before the first tap row take the left column from
/// lane 7; [`left_420`] gathers it for the others.
#[inline]
pub(super) fn luma_420<const F: usize>(
    token: LumaToken,
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
    if periodic && first_periodic < width {
        if width <= 8 || !first_periodic.is_multiple_of(16) {
            return false;
        }
        let mut j = first_periodic;
        while j < width {
            clamp_groups |= 1 << (j / 16);
            j += 32;
        }
    }
    let top_taps = weights[0] != [0; 3];
    let lane_left = width <= 8
        && luma_x >= 4
        && block.first_tap_row().is_some_and(|row| row >= 17)
        && left.is_some()
        && !(top_taps && block.periodic && {
            let height = out.len() / width - usize::from(block.above.is_some());
            (block.luma_y + 2).next_multiple_of(64) < block.luma_y + 2 * height
        });
    let stride = block.stride;
    'rows: {
        #[cfg(target_arch = "x86_64")]
        if F == 2
            && width <= 8
            && !lane_left
            && stride >= 17
            && let Some(source) =
                narrow_f2_source(luma, &block, tap_rows(&block, 0), out.len() / width)
        {
            luma_420_narrow_f2(token, clamp_groups & 1 != 0, source, block, out);
            break 'rows;
        }

        if !top_taps
            && lane_left
            && stride >= 64
            && block.above.is_none_or(|[_, mid, bottom]| {
                mid + 2 * stride == block.origin && bottom == mid + stride
            })
            && let Some(start) = tap_rows(&block, 0)[1].checked_sub(17)
            && let Some(rows) = luma.get(start..)
            && rows.len() >= 2 * stride * (out.len() >> width.trailing_zeros())
            && let Some(left) = left.as_deref_mut()
        {
            #[cfg(target_arch = "aarch64")]
            neon::luma_420_pairs::<F>(token, clamp_groups, rows, block, left, out);
            #[cfg(target_arch = "x86_64")]
            if F == 0 {
                luma_420_pairs_f0(token, clamp_groups, rows, block, left, out);
            } else {
                luma_420_pairs_f1(token, clamp_groups, rows, block, left, out);
            }
            break 'rows;
        }

        let lane_out = left.as_deref_mut().filter(|_| lane_left);
        #[cfg(target_arch = "aarch64")]
        neon::luma_420_rows::<F>(token, clamp_groups, luma, block, lane_out, out);
        #[cfg(target_arch = "x86_64")]
        match F {
            0 => luma_420_rows_f0(token, clamp_groups, luma, block, lane_out, out),
            1 => luma_420_rows_f1(token, clamp_groups, luma, block, lane_out, out),
            _ => luma_420_rows_f2(token, clamp_groups, luma, block, lane_out, out),
        }
    }
    if let Some(left) = left.filter(|_| !lane_left) {
        left_420(token, luma, &block, weights, left);
    }
    true
}

#[cfg(target_arch = "x86_64")]
fn narrow_f2_source<'a>(
    luma: &'a [u8],
    block: &Luma420Block,
    [top, mid, bottom]: [usize; 3],
    rows: usize,
) -> Option<(&'a [u8; 16], &'a [u8])> {
    let stride = block.stride;
    if block.above.is_some() && (mid + 2 * stride != block.origin || bottom != mid + stride) {
        return None;
    }
    let start = mid.checked_sub(1)?;
    let top = luma.get(top..)?.first_chunk()?;
    let body = luma.get(start..start + 2 * stride * rows)?;
    Some((top, body))
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// Consecutive narrow output rows share the bottom and top tap row.
    #[inline]
    fn luma_420_narrow_f2(
        token: Avx2,
        clamp_first: bool,
        source: (&[u8; 16], &[u8]),
        block: Luma420Block,
        out: &mut [i16],
    ) {
        use core::arch::x86_64::{__m128i, _mm_add_epi16, _mm_maddubs_epi16, _mm_set1_epi16};
        use fearless_simd::{SimdBase as _, i16x8, u8x16};
        let [mid_centre, mid_left] = [
            &LANE_TAPS[lane_tap_index(2, usize::from(clamp_first))][0][1],
            &LANE_TAPS[lane_tap_index(2, usize::from(clamp_first))][1][1],
        ]
        .map(|weights| -> __m128i { i16x8::from_slice(token, &weights[..8]).into() });
        let vertical = |row: __m128i| _mm_maddubs_epi16(row, _mm_set1_epi16(1));
        let (top, rows) = source;
        let mut top = vertical(u8x16::from_slice(token, top).into());
        let stride = block.stride;
        let skip = usize::from(block.above.is_some());
        let mut y = block.luma_y;
        for (r, (rows, out)) in rows
            .chunks_exact(2 * stride)
            .zip(out.chunks_exact_mut(block.width))
            .enumerate()
        {
            let (mid, bottom) = rows.split_at(stride);
            let (Some(mid), Some(bottom)) = (
                mid.first_chunk::<17>(),
                bottom.get(1..).and_then(<[u8]>::first_chunk::<16>),
            ) else {
                return;
            };
            let left_rd = u8x16::from_slice(token, &mid[..16]).into();
            let right_rd = u8x16::from_slice(token, &mid[1..]).into();
            let side = _mm_add_epi16(
                _mm_maddubs_epi16(left_rd, mid_left),
                _mm_maddubs_epi16(right_rd, mid_centre),
            );
            if r >= skip {
                if r != 0
                    && (r == skip && block.clamp_row0 || block.periodic && y.is_multiple_of(64))
                {
                    top = vertical(right_rd);
                }
                y += 2;
            }
            let bottom = vertical(u8x16::from_slice(token, bottom).into());
            let result = _mm_add_epi16(_mm_add_epi16(top, bottom), side);
            top = bottom;
            store_row8(token, result, out);
        }
    }
);

/// The downsampled luma left of each block row into `left`: each row's taps
/// (clamped only on the block's first row) gathered, then summed in lanes.
fn left_420(
    token: impl fearless_simd::Simd,
    luma: &[u8],
    block: &Luma420Block,
    weights: [[i32; 3]; 3],
    left: &mut [i16],
) {
    let first = usize::from(block.above.is_some());
    let clamp_left = usize::from(block.luma_x != 2);
    for (g, group) in left.chunks_mut(8).enumerate() {
        let mut taps = [[[0u16; 8]; 3]; 3];
        for lane in 0..group.len() {
            let r = first + 8 * g + lane;
            let [_, mid, bottom] = tap_rows(block, r);
            let top = if r == first { mid } else { mid - block.stride };
            for ((taps, row), weights) in taps.iter_mut().zip([top, mid, bottom]).zip(weights) {
                let cols = [row - 2 - clamp_left, row - 2, row - 1];
                for ((tap, col), weight) in taps.iter_mut().zip(cols).zip(weights) {
                    if weight != 0 {
                        tap[lane] = luma[col].into();
                    }
                }
            }
        }
        let lanes = gathered_q3(token, &taps, weights);
        group.copy_from_slice(&lanes[..group.len()]); // splot-copy-ok: publish CfL lumaLeft samples
    }
}

/// Defines the `pmaddubsw` part of [`luma_420`] for one constant filter.
macro_rules! luma_420_rows {
    ($name:ident, $filter:literal) => {
        fearless_simd::kernel!(
            /// The `pmaddubsw` part of [`luma_420`]: the first sample of the
            /// 16-lane groups set in `clamp_groups` (bit `g / 16`; bit 0 alone for
            /// blocks up to 8 wide) with its left tap clamped. With `left`, blocks up
            /// to 8 wide run 16 lanes from 8 samples before the block, and lane 7 is
            /// the sample left of each row, with that row's taps. Wider rows
            /// store whole 16-lane groups into the next row, written after.
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
                use core::arch::x86_64::{
                    __m128i, __m256i, _mm_add_epi16, _mm_maddubs_epi16, _mm256_add_epi16,
                    _mm256_castsi256_si128, _mm256_extract_epi16, _mm256_extracti128_si256,
                    _mm256_maddubs_epi16, _mm256_setzero_si256,
                };
                use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u8x16, u8x32};
                let width = block.width;
                let lanes = |clamped: usize, pair: usize, row: usize| -> __m256i {
                    let taps = &LANE_TAPS[lane_tap_index($filter, clamped)];
                    i16x16::from_slice(token, &taps[pair][row]).into()
                };
                let [plain_centre, plain_left] = &LANE_TAPS[lane_tap_index($filter, 0)];
                let used = |pair: &[[i16; 16]; 3], row: usize| pair[row][1] != 0;
                let first = usize::from(clamp_groups & 1);
                let skip = usize::from(block.above.is_some());
                let span = 2 * width.next_multiple_of(16);
                for r in 0..out.len() / width {
                    let rows = tap_rows(&block, r);
                    let (_, tail) = out.split_at_mut(r * width);
                    if width > 8 {
                        let srcs = rows.map(|row| &luma[row - 1..row + span]);
                        for g in (0..width).step_by(16) {
                            let clamped = usize::from(clamp_groups >> (g / 16) & 1);
                            let mut acc = _mm256_setzero_si256();
                            for (i, src) in srcs.iter().enumerate() {
                                let src = &src[2 * g..2 * g + 33];
                                if used(plain_centre, i) {
                                    let bytes: __m256i = u8x32::from_slice(token, &src[1..]).into();
                                    acc = _mm256_add_epi16(
                                        acc,
                                        _mm256_maddubs_epi16(bytes, lanes(clamped, 0, i)),
                                    );
                                }
                                if used(plain_left, i) {
                                    let bytes: __m256i =
                                        u8x32::from_slice(token, &src[..32]).into();
                                    acc = _mm256_add_epi16(
                                        acc,
                                        _mm256_maddubs_epi16(bytes, lanes(clamped, 1, i)),
                                    );
                                }
                            }
                            let acc = i16x16::simd_from(token, acc);
                            if let Some(out) = tail.get_mut(g..g + 16) {
                                acc.store_slice(out);
                            } else {
                                for (slot, &lane) in tail[g..].iter_mut().zip(acc.as_slice()) {
                                    *slot = lane;
                                }
                            }
                        }
                        continue;
                    }
                    let out = &mut tail[..width];
                    if let Some(left) = left.as_deref_mut() {
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
                let taps = &LANE_TAPS[lane_tap_index($filter, 2 * usize::from(clamp_groups & 1))];
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
#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
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

/// Writes the rows of a CfL block narrower than 16 samples into each
/// `(out, stride, samples)` plane: each group of sixteen `samples(luma)`
/// covers `16 / $width` rows.
macro_rules! cfl_narrow_rows {
    ($width:literal, $luma:ident, $(($out:ident, $stride:ident, $samples:ident)),+) => {
        $(let mut $out = &mut *$out;)+
        for luma in $luma.chunks_exact(16) {
            $(
                for samples in $samples(luma).as_chunks::<$width>().0 {
                    if let Some(out) = $out.first_chunk_mut::<$width>() {
                        *out = *samples;
                    }
                    $out = core::mem::take(&mut $out)
                        .get_mut($stride..)
                        .unwrap_or_default();
                }
            )+
        }
    };
}

/// [`cfl_narrow_rows`] for any CfL width: 4, 8, or a multiple of 16; each
/// group of `L` serves every plane.
macro_rules! cfl_rows {
    ($width:ident, $luma:ident, $(($out:ident, $stride:ident, $samples:ident)),+) => {
        match $width {
            4 => {
                cfl_narrow_rows!(4, $luma, $(($out, $stride, $samples)),+);
            }
            8 => {
                cfl_narrow_rows!(8, $luma, $(($out, $stride, $samples)),+);
            }
            _ => {
                $(let mut $out = &mut *$out;)+
                for luma in $luma.chunks_exact($width) {
                    $(
                        let out = $out.get_mut(..$width).unwrap_or_default();
                        for (out, luma) in out
                            .as_chunks_mut::<16>()
                            .0
                            .iter_mut()
                            .zip(luma.chunks_exact(16))
                        {
                            *out = $samples(luma);
                        }
                        $out = core::mem::take(&mut $out)
                            .get_mut($stride..)
                            .unwrap_or_default();
                    )+
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
        cfl_rows!(width, luma, (out, stride, samples));
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
        cfl_rows!(width, luma, (out, stride, samples));
    }
);

fearless_simd::kernel!(
    /// [`cfl_predict_u8`] of the U and V `[dc, alpha, lumaAvg, max]` models of
    /// one `L` in one pass: each group of `L` serves both planes.
    #[inline]
    pub(super) fn cfl_predict_pair_u8(
        token: Avx2,
        models: [[i32; 4]; 2],
        luma: &[i16],
        width: usize,
        outs: [&mut [u8]; 2],
        strides: [usize; 2],
    ) {
        use core::arch::x86_64::{
            __m256i, _mm_packus_epi16, _mm256_abs_epi16, _mm256_add_epi16, _mm256_castsi256_si128,
            _mm256_extracti128_si256, _mm256_mulhrs_epi16, _mm256_set1_epi16, _mm256_sign_epi16,
            _mm256_sub_epi16,
        };
        use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u8x16};
        let [u_constants, v_constants] = models.map(|model| cfl_predict_constants!(model));
        let samples = |[average, sign, scale, dc]: [__m256i; 4], luma: &[i16]| -> [u8; 16] {
            let v = cfl_predict16!(token, luma, [average, sign, scale, dc]);
            let packed =
                _mm_packus_epi16(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
            u8x16::simd_from(token, packed).into()
        };
        let u_samples = |luma: &[i16]| samples(u_constants, luma);
        let v_samples = |luma: &[i16]| samples(v_constants, luma);
        let ([u, v], [u_stride, v_stride]) = (outs, strides);
        cfl_rows!(
            width,
            luma,
            (u, u_stride, u_samples),
            (v, v_stride, v_samples)
        );
    }
);

fearless_simd::kernel!(
    /// [`cfl_predict_pair_u8`] for samples up to `models[_][3]`, at most 10 bits.
    #[inline]
    pub(super) fn cfl_predict_pair_u16(
        token: Avx2,
        models: [[i32; 4]; 2],
        luma: &[i16],
        width: usize,
        outs: [&mut [u16]; 2],
        strides: [usize; 2],
    ) {
        use core::arch::x86_64::{
            __m256i, _mm256_abs_epi16, _mm256_add_epi16, _mm256_max_epi16, _mm256_min_epi16,
            _mm256_mulhrs_epi16, _mm256_set1_epi16, _mm256_setzero_si256, _mm256_sign_epi16,
            _mm256_sub_epi16,
        };
        use fearless_simd::{SimdBase as _, SimdFrom as _, i16x16, u16x16};
        let [u_constants, v_constants] = models.map(|model| cfl_predict_constants!(model));
        let max = _mm256_set1_epi16(models[0][3] as i16);
        let zero = _mm256_setzero_si256();
        let samples = |[average, sign, scale, dc]: [__m256i; 4], luma: &[i16]| -> [u16; 16] {
            let v = cfl_predict16!(token, luma, [average, sign, scale, dc]);
            u16x16::simd_from(token, _mm256_min_epi16(_mm256_max_epi16(v, zero), max)).into()
        };
        let u_samples = |luma: &[i16]| samples(u_constants, luma);
        let v_samples = |luma: &[i16]| samples(v_constants, luma);
        let ([u, v], [u_stride, v_stride]) = (outs, strides);
        cfl_rows!(
            width,
            luma,
            (u, u_stride, u_samples),
            (v, v_stride, v_samples)
        );
    }
);

/// The NEON form of the AVX2 kernels: `umlal` of the deinterleaved left,
/// centre and right luma columns is `pmaddubsw` (every weight and sum is
/// non-negative and below `2^11`), and `sqrdmulh` is `pmulhrsw` (its
/// operands are non-negative, so it never saturates).
#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::{
        int16x8_t, uint8x16_t, uint16x8_t, vabsq_s16, vaddq_s16, vcombine_u8, vdupq_n_s16,
        vdupq_n_u16, veorq_s16, vget_low_u8, vgetq_lane_u16, vmaxq_s16, vminq_s16, vmlal_high_u8,
        vmlal_u8, vqmovun_s16, vqrdmulhq_s16, vreinterpretq_s16_u16, vreinterpretq_u16_s16,
        vshrq_n_s16, vsubq_s16, vuzp1q_u8, vuzp2q_u8,
    };
    use fearless_simd::{Neon, Simd as _, SimdBase as _, SimdFrom as _, i16x8, u8x16, u16x8};

    use super::{CFL_FILTERS_420, Luma420Block, tap_rows};

    /// `WEIGHTS[filter][variant][row][tap]`: the 16 lane weights of the left,
    /// centre and right `tap` of each tap row, unclamped (`variant` 0) or with
    /// the left tap of lane 0 (`variant` 1) or lane 8 (`variant` 2) folded
    /// into the centre.
    static WEIGHTS: [[[[[u8; 16]; 3]; 3]; 3]; 3] = {
        let mut table = [[[[[0; 16]; 3]; 3]; 3]; 3];
        let mut i = 0;
        while i < 3 * 3 * 3 * 3 * 16 {
            let (filter, variant, row) = (i / 432, i / 144 % 3, i / 48 % 3);
            let (tap, lane) = (i / 16 % 3, i % 16);
            let [left, centre, right] = CFL_FILTERS_420[filter][row];
            let taps = if variant > 0 && lane == 8 * (variant - 1) {
                [0, centre + left, right]
            } else {
                [left, centre, right]
            };
            table[filter][variant][row][tap][lane] = taps[tap] as u8;
            i += 1;
        }
        table
    };

    /// The `L` of filter `F` in 16 lanes (8 unless `WIDE`) from luma
    /// column `at[row]` of each tap row, with lane weights `weights[row]`.
    #[target_feature(enable = "neon")]
    #[inline]
    fn taps<const F: usize, const WIDE: bool>(
        token: Neon,
        luma: &[u8],
        at: [usize; 3],
        weights: &[[[u8; 16]; 3]; 3],
    ) -> [uint16x8_t; 2] {
        let mut acc = [vdupq_n_u16(0); 2];
        let columns = |start: usize| {
            let low: uint8x16_t = u8x16::from_slice(token, &luma[start..start + 16]).into();
            let high = if WIDE {
                u8x16::from_slice(token, &luma[start + 16..start + 32]).into()
            } else {
                low
            };
            (vuzp1q_u8(low, high), vuzp2q_u8(low, high))
        };
        let mut add = |bytes: uint8x16_t, weights: &[u8; 16]| {
            let weights: uint8x16_t = u8x16::from_slice(token, weights).into();
            acc[0] = vmlal_u8(acc[0], vget_low_u8(bytes), vget_low_u8(weights));
            if WIDE {
                acc[1] = vmlal_high_u8(acc[1], bytes, weights);
            }
        };
        for ((&at, [left, centre, right]), weights) in
            at.iter().zip(CFL_FILTERS_420[F]).zip(weights)
        {
            if centre != 0 || right != 0 {
                let (centres, rights) = columns(at);
                add(centres, &weights[1]);
                if right != 0 {
                    add(rights, &weights[2]);
                }
            }
            if left != 0 {
                add(columns(at - 1).0, &weights[0]);
            }
        }
        acc
    }

    /// The lanes of `v` as `i16`; every `L` is below `2^11`.
    #[target_feature(enable = "neon")]
    #[inline]
    fn signed(token: Neon, v: uint16x8_t) -> i16x8<Neon> {
        i16x8::simd_from(token, vreinterpretq_s16_u16(v))
    }

    /// Stores the first `out.len()` (at most 8) lanes of `row`.
    #[target_feature(enable = "neon")]
    #[inline]
    fn store_row8(token: Neon, row: uint16x8_t, out: &mut [i16]) {
        let lanes: [i16; 8] = signed(token, row).into();
        if let Some(out) = out.first_chunk_mut::<8>() {
            *out = lanes;
        } else {
            for (slot, lane) in out.iter_mut().zip(lanes) {
                *slot = lane;
            }
        }
    }

    /// Stores the first `out.len()` (at most 16) lanes of `[low, high]`.
    #[target_feature(enable = "neon")]
    #[inline]
    fn store_row16(token: Neon, [low, high]: [uint16x8_t; 2], out: &mut [i16]) {
        if let Some(out) = out.first_chunk_mut::<16>() {
            let (first, second) = out.split_at_mut(8);
            signed(token, low).store_slice(first);
            signed(token, high).store_slice(second);
        } else {
            store_row8(token, low, out);
            store_row8(token, high, out.get_mut(8..).unwrap_or_default());
        }
    }

    fearless_simd::kernel!(
        /// Applies luma filter `F` to each row of a 4:2:0 block.
        #[inline]
        pub(in super::super) fn luma_420_rows<const F: usize>(
            token: Neon,
            clamp_groups: u8,
            luma: &[u8],
            block: Luma420Block,
            left: Option<&mut [i16]>,
            out: &mut [i16],
        ) {
            let mut left = left;
            let weights = &WEIGHTS[F];
            let width = block.width;
            let first = usize::from(clamp_groups & 1);
            let skip = usize::from(block.above.is_some());
            for (r, out) in out.chunks_exact_mut(width).enumerate() {
                let rows = tap_rows(&block, r);
                if width > 8 {
                    for g in (0..width).step_by(16) {
                        let clamped = usize::from(clamp_groups >> (g / 16) & 1);
                        let at = rows.map(|row| row + 2 * g);
                        let acc = taps::<F, true>(token, luma, at, &weights[clamped]);
                        store_row16(token, acc, &mut out[g..]);
                    }
                } else if let Some(left) = left.as_deref_mut() {
                    let at = rows.map(|row| row - 16);
                    let [low, high] = taps::<F, true>(token, luma, at, &weights[2 * first]);
                    if let Some(slot) = left.get_mut(r.wrapping_sub(skip)) {
                        *slot = vgetq_lane_u16::<7>(low) as i16;
                    }
                    store_row8(token, high, out);
                } else {
                    let [low, _] = taps::<F, false>(token, luma, rows, &weights[first]);
                    store_row8(token, low, out);
                }
            }
        }
    );

    fearless_simd::kernel!(
        /// Applies luma filter `F` without top taps from consecutive row pairs.
        #[inline]
        pub(in super::super) fn luma_420_pairs<const F: usize>(
            token: Neon,
            clamp_groups: u8,
            rows: &[u8],
            block: Luma420Block,
            left: &mut [i16],
            out: &mut [i16],
        ) {
            let stride = block.stride;
            if stride < 64 {
                return;
            }
            let weights = &WEIGHTS[F][2 * usize::from(clamp_groups & 1)];
            let skip = usize::from(block.above.is_some());
            let mut rest = rows;
            for (r, out) in out.chunks_exact_mut(block.width).enumerate() {
                let Some((pair, tail)) = rest.split_at_checked(2 * stride) else {
                    return;
                };
                rest = tail;
                let [low, high] = taps::<F, true>(token, pair, [1, 1, stride + 1], weights);
                if let Some(slot) = left.get_mut(r.wrapping_sub(skip)) {
                    *slot = vgetq_lane_u16::<7>(low) as i16;
                }
                store_row8(token, high, out);
            }
        }
    );

    /// The broadcast `lumaAvg`, `alpha` sign mask, `16 * |alpha|` and `dc`
    /// of a `[dc, alpha, lumaAvg, max]` model.
    #[target_feature(enable = "neon")]
    #[inline]
    fn constants([dc, alpha, average, _]: [i32; 4]) -> [int16x8_t; 4] {
        [
            vdupq_n_s16(average as i16),
            vdupq_n_s16(-i16::from(alpha < 0)),
            vdupq_n_s16((alpha.unsigned_abs() << 4) as i16),
            vdupq_n_s16(dc as i16),
        ]
    }

    /// Eight CfL samples before `Clip1` from `luma[..8]`, as the AVX2
    /// `cfl_predict16`: `Round2(|alpha * d|, 11)` from `sqrdmulh`, then the
    /// sign of `alpha * d` from the sign masks of `d` and `alpha`.
    #[target_feature(enable = "neon")]
    #[inline]
    fn predict8(
        token: Neon,
        [average, sign, scale, dc]: [int16x8_t; 4],
        luma: &[i16],
    ) -> int16x8_t {
        let d = vsubq_s16(i16x8::from_slice(token, &luma[..8]).into(), average);
        let scaled = vqrdmulhq_s16(vabsq_s16(d), scale);
        let sign = veorq_s16(vshrq_n_s16::<15>(d), sign);
        vaddq_s16(vsubq_s16(veorq_s16(scaled, sign), sign), dc)
    }

    fearless_simd::kernel!(
        /// The NEON `cfl_predict_u8`; `sqxtun` is the 8-bit `Clip1`.
        #[inline]
        pub(in super::super) fn cfl_predict_u8(
            token: Neon,
            model: [i32; 4],
            luma: &[i16],
            width: usize,
            out: &mut [u8],
            stride: usize,
        ) {
            let constants = constants(model);
            let samples = |luma: &[i16]| -> [u8; 16] {
                let low = vqmovun_s16(predict8(token, constants, &luma[..8]));
                let high = vqmovun_s16(predict8(token, constants, &luma[8..]));
                u8x16::simd_from(token, vcombine_u8(low, high)).into()
            };
            cfl_rows!(width, luma, (out, stride, samples));
        }
    );

    fearless_simd::kernel!(
        /// The NEON `cfl_predict_pair_u8`.
        #[inline]
        pub(in super::super) fn cfl_predict_pair_u8(
            token: Neon,
            models: [[i32; 4]; 2],
            luma: &[i16],
            width: usize,
            outs: [&mut [u8]; 2],
            strides: [usize; 2],
        ) {
            let [u_constants, v_constants] = models.map(|model| constants(model));
            let samples = |constants: [int16x8_t; 4], luma: &[i16]| -> [u8; 16] {
                let low = vqmovun_s16(predict8(token, constants, &luma[..8]));
                let high = vqmovun_s16(predict8(token, constants, &luma[8..]));
                u8x16::simd_from(token, vcombine_u8(low, high)).into()
            };
            let u_samples = |luma: &[i16]| samples(u_constants, luma);
            let v_samples = |luma: &[i16]| samples(v_constants, luma);
            let ([u, v], [u_stride, v_stride]) = (outs, strides);
            cfl_rows!(
                width,
                luma,
                (u, u_stride, u_samples),
                (v, v_stride, v_samples)
            );
        }
    );

    fearless_simd::kernel!(
        /// The NEON `cfl_predict_pair_u16`.
        #[inline]
        pub(in super::super) fn cfl_predict_pair_u16(
            token: Neon,
            models: [[i32; 4]; 2],
            luma: &[i16],
            width: usize,
            outs: [&mut [u16]; 2],
            strides: [usize; 2],
        ) {
            let [u_constants, v_constants] = models.map(|model| constants(model));
            let (zero, max) = (vdupq_n_s16(0), vdupq_n_s16(models[0][3] as i16));
            let clip = |constants: [int16x8_t; 4], luma: &[i16]| {
                let v = predict8(token, constants, luma);
                u16x8::simd_from(
                    token,
                    vreinterpretq_u16_s16(vminq_s16(vmaxq_s16(v, zero), max)),
                )
            };
            let samples = |constants: [int16x8_t; 4], luma: &[i16]| -> [u16; 16] {
                token
                    .combine_u16x8(clip(constants, &luma[..8]), clip(constants, &luma[8..]))
                    .into()
            };
            let u_samples = |luma: &[i16]| samples(u_constants, luma);
            let v_samples = |luma: &[i16]| samples(v_constants, luma);
            let ([u, v], [u_stride, v_stride]) = (outs, strides);
            cfl_rows!(
                width,
                luma,
                (u, u_stride, u_samples),
                (v, v_stride, v_samples)
            );
        }
    );

    fearless_simd::kernel!(
        /// The NEON `cfl_predict_u16`.
        #[inline]
        pub(in super::super) fn cfl_predict_u16(
            token: Neon,
            model: [i32; 4],
            luma: &[i16],
            width: usize,
            out: &mut [u16],
            stride: usize,
        ) {
            let constants = constants(model);
            let (zero, max) = (vdupq_n_s16(0), vdupq_n_s16(model[3] as i16));
            let clip = |luma: &[i16]| {
                let v = predict8(token, constants, luma);
                u16x8::simd_from(
                    token,
                    vreinterpretq_u16_s16(vminq_s16(vmaxq_s16(v, zero), max)),
                )
            };
            let samples = |luma: &[i16]| -> [u16; 16] {
                token
                    .combine_u16x8(clip(&luma[..8]), clip(&luma[8..]))
                    .into()
            };
            cfl_rows!(width, luma, (out, stride, samples));
        }
    );
}
