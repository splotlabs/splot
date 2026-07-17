// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18 CDEF (Constrained Directional Enhancement Filter) sample math.
//!
//! This module implements the scheduler-free per-block AV2 CDEF primitives
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)):
//! the § 7.18.2 direction search ([`cdef_direction`], `#s-7-18-2`), which measures
//! the dominant direction (`yDir`) and variance (`var`) of an 8x8 luma block from
//! the pre-shifted (`>> (BitDepth - 8)`, minus 128) reconstructed samples using the
//! § 7.18.2 `partial[][]` accumulation and the `Div_Table` cost; the § 7.18.3
//! `constrain` clamp ([`cdef_constrain`], `#s-7-18-3`); and the § 7.18.3 per-sample
//! primary/secondary tap accumulation ([`cdef_filter_sample`], `#s-7-18-3`), which
//! combines the center sample, the two primary directional taps, and the four
//! secondary directional taps (each already fetched and availability-flagged by the
//! caller) into one deringed sample via `Cdef_Pri_Taps` / `Cdef_Sec_Taps`, the
//! `(8 + sum - (sum < 0)) >> 4` rounding, and the `Clip3(min, max, ...)` clamp.
//!
//! The § 7.18.3 `Cdef_Directions` direction-offset table and the § 7.18.1
//! `Cdef_Uv_Dir` chroma-direction remap are exposed as constants for the caller's
//! `cdef_get_at` neighbour addressing; the § 7.18.2 `Div_Table` and the § 7.18.3
//! `Cdef_Pri_Taps` / `Cdef_Sec_Taps` are consumed internally.
//!
//! Scope: these are the per-block direction/variance derivation and the per-sample
//! tap math over caller-resolved spec-derived values. The § 7.18 / § 7.18.1
//! 64x64-unit → 8x8-block traversal, the `cdef_idx` per-64x64 parameter lookup, the
//! § 7.18.1 `skip` / `skipChroma` / strength derivation, the § 5.20.9.3
//! `is_inside_filter_region` availability check, and the `CurrFrame` / `CdefFrame`
//! sample I/O stay with the caller — it passes the resolved 8x8 luma block, the
//! center value, the fetched directional neighbours, and the strength / damping
//! scalars, exactly as the other `splot-recon` primitives take caller-resolved
//! spec-derived values. It does not read frame, segment, or tile state or wire into
//! the runtime decode path.
//!
//! Feature tracking: `RECON-CDEF-FILTER`.

/// AV2 § 7.18.2 `Div_Table[9]`: reciprocal-scaling weights for the direction cost.
const DIV_TABLE: [i32; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];

/// AV2 § 7.18.3 `Cdef_Pri_Taps[2][2]`: primary-tap weights, selected by
/// `(priStr >> coeffShift) & 1`.
const CDEF_PRI_TAPS: [[i32; 2]; 2] = [[4, 2], [3, 3]];

/// AV2 § 7.18.3 `Cdef_Sec_Taps[2][2]`: secondary-tap weights, selected by
/// `(priStr >> coeffShift) & 1`.
const CDEF_SEC_TAPS: [[i32; 2]; 2] = [[2, 1], [2, 1]];

/// AV2 § 7.18.3 `Cdef_Directions[8][2][2]`: the `(dy, dx)` neighbour offsets for
/// direction `dir` and tap index `k` (`[dir][k][0]` is the row offset, `[dir][k][1]`
/// the column offset). The caller's `cdef_get_at` adds `sign * Cdef_Directions[dir]
/// [k]` to the sample position.
pub const CDEF_DIRECTIONS: [[[i32; 2]; 2]; 8] = [
    [[-1, 1], [-2, 2]],
    [[0, 1], [-1, 2]],
    [[0, 1], [0, 2]],
    [[0, 1], [1, 2]],
    [[1, 1], [2, 2]],
    [[1, 0], [2, 1]],
    [[1, 0], [2, 0]],
    [[1, 0], [2, -1]],
];

/// AV2 § 7.18.1 `Cdef_Uv_Dir[2][2][8]`: the chroma direction remap, indexed by
/// `[SubsamplingX][SubsamplingY][yDir]`, producing the chroma `dir` from the luma
/// `yDir`.
pub const CDEF_UV_DIR: [[[usize; 8]; 2]; 2] = [
    [[0, 1, 2, 3, 4, 5, 6, 7], [1, 2, 2, 2, 3, 4, 6, 0]],
    [[7, 0, 2, 4, 5, 6, 6, 6], [0, 1, 2, 3, 4, 5, 6, 7]],
];

/// AV2 § 4.7 `FloorLog2(x)`: the position of the most significant set bit
/// (`FloorLog2(0)` is unreachable in CDEF — `constrain` short-circuits on a zero
/// threshold and `var >> 6` is gated nonzero before its `FloorLog2`).
const fn floor_log2(x: u32) -> u32 {
    if x == 0 { 0 } else { x.ilog2() }
}

/// AV2 § 7.18.2 CDEF direction process.
///
/// `block` is the 8x8 luma neighbourhood with each sample already reduced to the
/// spec's `x = (CurrFrame[0][y0+i][x0+j] >> (BitDepth - 8)) - 128` form (`block[i][j]`
/// = row `i`, column `j`). Returns `(yDir, var)`: the dominant direction index in
/// `0..8` and the variance `var = (bestCost - cost[(yDir + 4) & 7]) >> 10`.
///
/// The `partial[][]` sums of eight `(sample - 128)` terms fit comfortably in `i32`,
/// The squared partials times `Div_Table` use the same `i32` accumulators as AVM;
/// the spec-bounded pre-shifted samples keep every directional cost in range.
#[allow(clippy::needless_range_loop)]
pub fn cdef_direction(block: &[[i32; 8]; 8]) -> (usize, i32) {
    let mut partial_hv = [[0i32; 8]; 2];
    let mut partial_diag = [[0i32; 15]; 2];
    let mut partial_alt = [[0i32; 11]; 4];
    for i in 0..8 {
        for j in 0..8 {
            let x = block[i][j];
            partial_diag[0][i + j] += x;
            partial_alt[0][i + j / 2] += x;
            partial_hv[0][i] += x;
            partial_alt[1][3 + i - j / 2] += x;
            partial_diag[1][7 + i - j] += x;
            partial_alt[2][3 - i / 2 + j] += x;
            partial_hv[1][j] += x;
            partial_alt[3][i / 2 + j] += x;
        }
    }
    finish_cdef_direction(&partial_hv, &partial_diag, &partial_alt)
}

/// AV2 § 7.18.2 CDEF direction process over the interior padded block layout.
///
/// The 8x8 luma block begins at row 2, column 2 of `pad`; `coeff_shift` is
/// `BitDepth - 8`. The result matches [`cdef_direction`] without materializing
/// the intermediate shifted 8x8 array.
pub fn cdef_direction_padded(pad: &[u16; CDEF_PADDED_AREA], coeff_shift: u32) -> (usize, i32) {
    let mut partial_hv = [[0i32; 8]; 2];
    let mut partial_diag = [[0i32; 15]; 2];
    let mut partial_alt = [[0i32; 11]; 4];
    for i in 0..8 {
        let row = (i + 2) * CDEF_PADDED_SIDE + 2;
        for j in 0..8 {
            let x = (i32::from(pad[row + j]) >> coeff_shift) - 128;
            partial_diag[0][i + j] += x;
            partial_alt[0][i + j / 2] += x;
            partial_hv[0][i] += x;
            partial_alt[1][3 + i - j / 2] += x;
            partial_diag[1][7 + i - j] += x;
            partial_alt[2][3 - i / 2 + j] += x;
            partial_hv[1][j] += x;
            partial_alt[3][i / 2 + j] += x;
        }
    }
    finish_cdef_direction(&partial_hv, &partial_diag, &partial_alt)
}

fn finish_cdef_direction(
    partial_hv: &[[i32; 8]; 2],
    partial_diag: &[[i32; 15]; 2],
    partial_alt: &[[i32; 11]; 4],
) -> (usize, i32) {
    let mut cost = [0i32; 8];
    for (&horizontal, &vertical) in partial_hv[0].iter().zip(&partial_hv[1]) {
        cost[2] += horizontal * horizontal;
        cost[6] += vertical * vertical;
    }
    cost[2] *= DIV_TABLE[8];
    cost[6] *= DIV_TABLE[8];
    for i in 0..7 {
        cost[0] += (partial_diag[0][i] * partial_diag[0][i]
            + partial_diag[0][14 - i] * partial_diag[0][14 - i])
            * DIV_TABLE[i + 1];
        cost[4] += (partial_diag[1][i] * partial_diag[1][i]
            + partial_diag[1][14 - i] * partial_diag[1][14 - i])
            * DIV_TABLE[i + 1];
    }
    cost[0] += partial_diag[0][7] * partial_diag[0][7] * DIV_TABLE[8];
    cost[4] += partial_diag[1][7] * partial_diag[1][7] * DIV_TABLE[8];
    for (n, partial) in partial_alt.iter().enumerate() {
        let i = n * 2 + 1;
        for j in 0..5 {
            cost[i] += partial[3 + j] * partial[3 + j];
        }
        cost[i] *= DIV_TABLE[8];
        for j in 0..3 {
            cost[i] += (partial[j] * partial[j] + partial[10 - j] * partial[10 - j])
                * DIV_TABLE[2 * j + 2];
        }
    }

    let mut best_cost = 0i32;
    let mut y_dir = 0usize;
    for (dir, &c) in cost.iter().enumerate() {
        if c > best_cost {
            best_cost = c;
            y_dir = dir;
        }
    }
    let var = (best_cost - cost[(y_dir + 4) & 7]) >> 10;
    (y_dir, var)
}

/// AV2 § 7.18.3 `constrain(diff, threshold, damping)`.
///
/// Returns `0` when `threshold` is `0`; otherwise it signs and clamps `diff` toward
/// zero by `threshold - (Abs(diff) >> dampingAdj)` where `dampingAdj = Max(0, damping
/// - FloorLog2(threshold))`.
pub fn cdef_constrain(diff: i32, threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let damping_adj = (damping - floor_log2(threshold as u32) as i32).max(0);
    let abs = diff.abs();
    let magnitude = (threshold - (abs >> damping_adj)).clamp(0, abs);
    if diff < 0 { -magnitude } else { magnitude }
}

/// One CDEF directional neighbour fetched by the caller's `cdef_get_at`: its value
/// and whether it was inside the filter region (`CdefAvailable`).
#[derive(Clone, Copy, Debug)]
pub struct CdefTap {
    /// The fetched `CurrFrame[plane][y][x]` sample value (ignored when `!available`).
    pub value: i32,
    /// `CdefAvailable`: whether the candidate position was inside the filter region.
    pub available: bool,
}

/// AV2 § 7.18.3 per-sample primary/secondary tap inputs for one output sample.
///
/// `center` is `CurrFrame[plane][y0 + i][x0 + j]` (the `x` of § 7.18.3). For each
/// `k` in `0..2` and each `sign` in `{-1, +1}`, `primary[k][sign_index]` is the
/// `cdef_get_at(..., dir, k, sign, ...)` primary tap and `secondary[k][sign_index]`
/// holds the two `(dir + dirOff) & 7` (`dirOff in {-2, +2}`) secondary taps. The
/// `sign_index` is `0` for `sign == -1` and `1` for `sign == +1`.
#[derive(Clone, Copy, Debug)]
pub struct CdefSampleTaps {
    /// The center sample value (`x`).
    pub center: i32,
    /// Primary taps `[k][sign_index]`.
    pub primary: [[CdefTap; 2]; 2],
    /// Secondary taps `[k][sign_index][dir_off_index]`.
    pub secondary: [[[CdefTap; 2]; 2]; 2],
}

/// AV2 § 7.18.3 per-sample CDEF filter: combines the center sample, the two primary
/// directional taps, and the four secondary directional taps into one deringed
/// output sample.
///
/// `pri_str` / `sec_str` are the bit-depth-scaled primary / secondary strengths
/// (`cdef_*_pri_strength << coeffShift` etc.), `damping` the § 7.18.1 damping shift,
/// and `coeff_shift` is `BitDepth - 8` (selects the `Cdef_Pri_Taps` / `Cdef_Sec_Taps`
/// row via `(pri_str >> coeff_shift) & 1`). Unavailable taps are skipped (they
/// contribute neither to `sum` nor to the `min` / `max` clamp), matching the spec's
/// `if (CdefAvailable)` guard.
pub fn cdef_filter_sample(
    taps: &CdefSampleTaps,
    pri_str: i32,
    sec_str: i32,
    damping: i32,
    coeff_shift: u32,
) -> i32 {
    let tap_row = ((pri_str >> coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(pri_str, damping);
    let sec_adj = constrain_damping_adj(sec_str, damping);

    let mut sum = 0i32;
    let mut max = taps.center;
    let mut min = taps.center;
    for k in 0..2 {
        for sign_index in 0..2 {
            let p = taps.primary[k][sign_index];
            if p.available {
                sum += pri_taps[k] * constrain_with_adj(p.value - taps.center, pri_str, pri_adj);
                max = max.max(p.value);
                min = min.min(p.value);
            }
            for dir_off_index in 0..2 {
                let s = taps.secondary[k][sign_index][dir_off_index];
                if s.available {
                    sum +=
                        sec_taps[k] * constrain_with_adj(s.value - taps.center, sec_str, sec_adj);
                    max = max.max(s.value);
                    min = min.min(s.value);
                }
            }
        }
    }

    let rounded = taps.center + ((8 + sum - i32::from(sum < 0)) >> 4);
    rounded.clamp(min, max)
}

/// Side length of the padded per-block scratch consumed by
/// [`cdef_filter_block_interior`]: an 8x8 block plus the § 7.18.3
/// `Cdef_Directions` tap reach of 2 on every side.
pub const CDEF_PADDED_SIDE: usize = 12;

/// Sample count of the padded per-block scratch: `CDEF_PADDED_SIDE` squared.
pub const CDEF_PADDED_AREA: usize = CDEF_PADDED_SIDE * CDEF_PADDED_SIDE;

/// Per-block § 7.18.3 filter constants for [`cdef_filter_block_interior`].
#[derive(Clone, Copy, Debug)]
pub struct CdefBlockFilter {
    /// Bit-depth-scaled (and, for luma, variance-adjusted) primary strength.
    pub pri_str: i32,
    /// Bit-depth-scaled secondary strength.
    pub sec_str: i32,
    /// § 7.18.1 damping shift, already plane-adjusted.
    pub damping: i32,
    /// Direction index in `0..8`.
    pub dir: usize,
    /// `BitDepth - 8`.
    pub coeff_shift: u32,
}

type CdefPrimaryStarts = [[usize; 2]; 2];
type CdefSecondaryStarts = [[[usize; 2]; 2]; 2];

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_padded_row<const W: usize>(
    pad: &[u16; CDEF_PADDED_AREA],
    start: usize,
) -> Option<&[u16; W]> {
    let end = start.checked_add(W)?;
    pad.get(start..end)?.try_into().ok()
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_output_row<const W: usize>(
    out: &mut [u16],
    stride: usize,
    row: usize,
) -> Option<&mut [u16; W]> {
    out.get_mut(row.checked_mul(stride)?..)?.first_chunk_mut()
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_primary_rows<'a, const W: usize>(
    pad: &'a [u16; CDEF_PADDED_AREA],
    row_offset: usize,
    starts: &CdefPrimaryStarts,
) -> Option<[[&'a [u16; W]; 2]; 2]> {
    Some([
        [
            cdef_padded_row(pad, starts[0][0] + row_offset)?,
            cdef_padded_row(pad, starts[0][1] + row_offset)?,
        ],
        [
            cdef_padded_row(pad, starts[1][0] + row_offset)?,
            cdef_padded_row(pad, starts[1][1] + row_offset)?,
        ],
    ])
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn cdef_secondary_rows<'a, const W: usize>(
    pad: &'a [u16; CDEF_PADDED_AREA],
    row_offset: usize,
    starts: &CdefSecondaryStarts,
) -> Option<[[[&'a [u16; W]; 2]; 2]; 2]> {
    Some([
        [
            [
                cdef_padded_row(pad, starts[0][0][0] + row_offset)?,
                cdef_padded_row(pad, starts[0][0][1] + row_offset)?,
            ],
            [
                cdef_padded_row(pad, starts[0][1][0] + row_offset)?,
                cdef_padded_row(pad, starts[0][1][1] + row_offset)?,
            ],
        ],
        [
            [
                cdef_padded_row(pad, starts[1][0][0] + row_offset)?,
                cdef_padded_row(pad, starts[1][0][1] + row_offset)?,
            ],
            [
                cdef_padded_row(pad, starts[1][1][0] + row_offset)?,
                cdef_padded_row(pad, starts[1][1][1] + row_offset)?,
            ],
        ],
    ])
}

fn cdef_tap_starts(
    center: usize,
    pri_rel: &[[isize; 2]; 2],
    sec_rel: &[[[isize; 2]; 2]; 2],
) -> Option<(CdefPrimaryStarts, CdefSecondaryStarts)> {
    let mut pri = [[0usize; 2]; 2];
    let mut sec = [[[0usize; 2]; 2]; 2];
    for k in 0..2 {
        for sign in 0..2 {
            pri[k][sign] = center.checked_add_signed(pri_rel[k][sign])?;
            for dir in 0..2 {
                sec[k][sign][dir] = center.checked_add_signed(sec_rel[k][sign][dir])?;
            }
        }
    }
    Some((pri, sec))
}

fn cdef_filter_block_interior_rows<const W: usize>(
    pad: &[u16; CDEF_PADDED_AREA],
    h: usize,
    filter: &CdefBlockFilter,
    pri_rel: &[[isize; 2]; 2],
    sec_rel: &[[[isize; 2]; 2]; 2],
    out: &mut [u16],
    out_stride: usize,
) -> Option<()> {
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(filter.pri_str, filter.damping);
    let sec_adj = constrain_damping_adj(filter.sec_str, filter.damping);
    let center_start = 2 * CDEF_PADDED_SIDE + 2;
    let (pri_starts, sec_starts) = cdef_tap_starts(center_start, pri_rel, sec_rel)?;

    if filter.pri_str != 0 && filter.sec_str != 0 {
        for i in 0..h {
            let row_offset = i * CDEF_PADDED_SIDE;
            let center_row = cdef_padded_row::<W>(pad, center_start + row_offset)?;
            let pri_rows = cdef_primary_rows::<W>(pad, row_offset, &pri_starts)?;
            let sec_rows = cdef_secondary_rows::<W>(pad, row_offset, &sec_starts)?;
            let output_row = cdef_output_row::<W>(out, out_stride, i)?;
            for j in 0..W {
                let center = i32::from(center_row[j]);
                let p00 = i32::from(pri_rows[0][0][j]);
                let p01 = i32::from(pri_rows[0][1][j]);
                let p10 = i32::from(pri_rows[1][0][j]);
                let p11 = i32::from(pri_rows[1][1][j]);
                let s000 = i32::from(sec_rows[0][0][0][j]);
                let s001 = i32::from(sec_rows[0][0][1][j]);
                let s010 = i32::from(sec_rows[0][1][0][j]);
                let s011 = i32::from(sec_rows[0][1][1][j]);
                let s100 = i32::from(sec_rows[1][0][0][j]);
                let s101 = i32::from(sec_rows[1][0][1][j]);
                let s110 = i32::from(sec_rows[1][1][0][j]);
                let s111 = i32::from(sec_rows[1][1][1][j]);
                let sum = pri_taps[0]
                    * (constrain_with_adj(p00 - center, filter.pri_str, pri_adj)
                        + constrain_with_adj(p01 - center, filter.pri_str, pri_adj))
                    + pri_taps[1]
                        * (constrain_with_adj(p10 - center, filter.pri_str, pri_adj)
                            + constrain_with_adj(p11 - center, filter.pri_str, pri_adj))
                    + sec_taps[0]
                        * (constrain_with_adj(s000 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s001 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s010 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s011 - center, filter.sec_str, sec_adj))
                    + sec_taps[1]
                        * (constrain_with_adj(s100 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s101 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s110 - center, filter.sec_str, sec_adj)
                            + constrain_with_adj(s111 - center, filter.sec_str, sec_adj));
                let min = center
                    .min(p00)
                    .min(p01)
                    .min(p10)
                    .min(p11)
                    .min(s000)
                    .min(s001)
                    .min(s010)
                    .min(s011)
                    .min(s100)
                    .min(s101)
                    .min(s110)
                    .min(s111);
                let max = center
                    .max(p00)
                    .max(p01)
                    .max(p10)
                    .max(p11)
                    .max(s000)
                    .max(s001)
                    .max(s010)
                    .max(s011)
                    .max(s100)
                    .max(s101)
                    .max(s110)
                    .max(s111);
                let rounded = center + ((8 + sum - i32::from(sum < 0)) >> 4);
                output_row[j] = rounded.clamp(min, max) as u16;
            }
        }
    } else if filter.pri_str != 0 {
        for i in 0..h {
            let row_offset = i * CDEF_PADDED_SIDE;
            let center_row = cdef_padded_row::<W>(pad, center_start + row_offset)?;
            let pri_rows = cdef_primary_rows::<W>(pad, row_offset, &pri_starts)?;
            let output_row = cdef_output_row::<W>(out, out_stride, i)?;
            for j in 0..W {
                let center = i32::from(center_row[j]);
                let mut sum = 0i32;
                for (k, pri_by_sign) in pri_rows.iter().enumerate() {
                    for pri_row in pri_by_sign {
                        let p = i32::from(pri_row[j]);
                        sum +=
                            pri_taps[k] * constrain_with_adj(p - center, filter.pri_str, pri_adj);
                    }
                }
                output_row[j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else if filter.sec_str != 0 {
        for i in 0..h {
            let row_offset = i * CDEF_PADDED_SIDE;
            let center_row = cdef_padded_row::<W>(pad, center_start + row_offset)?;
            let sec_rows = cdef_secondary_rows::<W>(pad, row_offset, &sec_starts)?;
            let output_row = cdef_output_row::<W>(out, out_stride, i)?;
            for j in 0..W {
                let center = i32::from(center_row[j]);
                let mut sum = 0i32;
                for (k, sec_by_sign) in sec_rows.iter().enumerate() {
                    for sec_by_dir in sec_by_sign {
                        for sec_row in sec_by_dir {
                            let s = i32::from(sec_row[j]);
                            sum += sec_taps[k]
                                * constrain_with_adj(s - center, filter.sec_str, sec_adj);
                        }
                    }
                }
                output_row[j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else {
        for i in 0..h {
            let row_offset = i * CDEF_PADDED_SIDE;
            let center_row = cdef_padded_row::<W>(pad, center_start + row_offset)?;
            let output_row = cdef_output_row::<W>(out, out_stride, i)?;
            *output_row = *center_row;
        }
    }
    Some(())
}

fn cdef_relative_offsets(dir: usize) -> ([[isize; 2]; 2], [[[isize; 2]; 2]; 2]) {
    let rel = |dir: usize, k: usize, sign: i32| -> isize {
        (sign * CDEF_DIRECTIONS[dir & 7][k][0]) as isize * CDEF_PADDED_SIDE as isize
            + (sign * CDEF_DIRECTIONS[dir & 7][k][1]) as isize
    };
    let mut pri_rel = [[0isize; 2]; 2];
    let mut sec_rel = [[[0isize; 2]; 2]; 2];
    for k in 0..2 {
        for (sign_index, sign) in [-1i32, 1].into_iter().enumerate() {
            pri_rel[k][sign_index] = rel(dir, k, sign);
            for (dir_off_index, dir_off) in [6usize, 2].into_iter().enumerate() {
                sec_rel[k][sign_index][dir_off_index] = rel(dir + dir_off, k, sign);
            }
        }
    }
    (pri_rel, sec_rel)
}

/// AV2 § 7.18.3 CDEF filter for one fully-interior block written to a strided output.
///
/// Returns `false` when the output geometry cannot hold the 8- or 4-sample-wide
/// CDEF block. The padded input layout matches [`cdef_filter_block_interior`].
pub fn cdef_filter_block_interior_to(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    let w = w.min(8);
    if out_stride < w {
        return false;
    }
    cdef_filter_block_interior_to_valid_stride(pad, w, h, filter, out, out_stride)
}

/// Variant of [`cdef_filter_block_interior_to`] for an already-validated output view.
#[doc(hidden)]
pub fn cdef_filter_block_interior_to_valid_stride(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    let (pri_rel, sec_rel) = cdef_relative_offsets(filter.dir);
    match w.min(8) {
        8 => cdef_filter_block_interior_rows::<8>(
            pad,
            h.min(8),
            filter,
            &pri_rel,
            &sec_rel,
            out,
            out_stride,
        ),
        4 => cdef_filter_block_interior_rows::<4>(
            pad,
            h.min(8),
            filter,
            &pri_rel,
            &sec_rel,
            out,
            out_stride,
        ),
        _ => None,
    }
    .is_some()
}

/// AV2 § 7.18.3 CDEF filter for one fully-interior block over a padded scratch.
///
/// `pad` holds the `CDEF_PADDED_SIDE x CDEF_PADDED_SIDE` row-major
/// neighbourhood in native `u16` sample storage, widened to `i32` per tap. Its
/// `(w x h)` output block starts at row 2, column 2; the
/// caller guarantees every tap position is inside the § 5.20.9.3 filter region
/// (`CdefAvailable` everywhere), which is what makes the per-tap availability
/// guard of [`cdef_filter_sample`] statically true. Bit-exact with calling
/// [`cdef_filter_sample`] per sample on all-available taps.
///
/// Filtered samples are written in native `u16` storage to `out[i * w + j]`
/// for `i in 0..h`, `j in 0..w`; `w` and `h` are clamped to 8. Every tap index
/// provably stays inside the scratch: the center index is at least
/// `2 * CDEF_PADDED_SIDE + 2` and the largest tap displacement is
/// `2 * CDEF_PADDED_SIDE + 2` in either
/// direction.
pub fn cdef_filter_block_interior(
    pad: &[u16; CDEF_PADDED_AREA],
    w: usize,
    h: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16; 64],
) {
    let w = w.min(8);
    let h = h.min(8);

    let (pri_rel, sec_rel) = cdef_relative_offsets(filter.dir);
    let row_result = match w {
        8 => cdef_filter_block_interior_rows::<8>(pad, h, filter, &pri_rel, &sec_rel, out, w),
        4 => cdef_filter_block_interior_rows::<4>(pad, h, filter, &pri_rel, &sec_rel, out, w),
        _ => None,
    };
    if row_result.is_some() {
        return;
    }

    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row];
    let sec_taps = CDEF_SEC_TAPS[tap_row];
    let pri_adj = constrain_damping_adj(filter.pri_str, filter.damping);
    let sec_adj = constrain_damping_adj(filter.sec_str, filter.damping);

    if filter.pri_str != 0 && filter.sec_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                let mut max = center;
                let mut min = center;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        let p = i32::from(
                            pad[center_index.wrapping_add_signed(pri_rel[k][sign_index])],
                        );
                        sum +=
                            pri_taps[k] * constrain_with_adj(p - center, filter.pri_str, pri_adj);
                        max = max.max(p);
                        min = min.min(p);
                        for dir_off_index in 0..2 {
                            let s = i32::from(
                                pad[center_index
                                    .wrapping_add_signed(sec_rel[k][sign_index][dir_off_index])],
                            );
                            sum += sec_taps[k]
                                * constrain_with_adj(s - center, filter.sec_str, sec_adj);
                            max = max.max(s);
                            min = min.min(s);
                        }
                    }
                }
                let rounded = center + ((8 + sum - i32::from(sum < 0)) >> 4);
                out[i * w + j] = rounded.clamp(min, max) as u16;
            }
        }
    // Either tap family alone has total weight 12, so its rounded result cannot
    // leave the neighbour range and the min/max clamp is redundant.
    } else if filter.pri_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        let p = i32::from(
                            pad[center_index.wrapping_add_signed(pri_rel[k][sign_index])],
                        );
                        sum +=
                            pri_taps[k] * constrain_with_adj(p - center, filter.pri_str, pri_adj);
                    }
                }
                out[i * w + j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else if filter.sec_str != 0 {
        for i in 0..h {
            for j in 0..w {
                let center_index = (i + 2) * CDEF_PADDED_SIDE + (j + 2);
                let center = i32::from(pad[center_index]);
                let mut sum = 0i32;
                for k in 0..2 {
                    for sign_index in 0..2 {
                        for dir_off_index in 0..2 {
                            let s = i32::from(
                                pad[center_index
                                    .wrapping_add_signed(sec_rel[k][sign_index][dir_off_index])],
                            );
                            sum += sec_taps[k]
                                * constrain_with_adj(s - center, filter.sec_str, sec_adj);
                        }
                    }
                }
                out[i * w + j] = (center + ((8 + sum - i32::from(sum < 0)) >> 4)) as u16;
            }
        }
    } else {
        for i in 0..h {
            for j in 0..w {
                out[i * w + j] = pad[(i + 2) * CDEF_PADDED_SIDE + (j + 2)];
            }
        }
    }
}

/// The `constrain` dampingAdj, which depends only on the per-call strengths
/// and is therefore derived once per sample instead of once per tap.
const fn constrain_damping_adj(threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let adj = damping - floor_log2(threshold as u32) as i32;
    if adj < 0 { 0 } else { adj }
}

const fn constrain_with_adj(diff: i32, threshold: i32, damping_adj: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let abs = diff.abs();
    let reduced = threshold - (abs >> damping_adj);
    let magnitude = if reduced < 0 {
        0
    } else if reduced > abs {
        abs
    } else {
        reduced
    };
    if diff < 0 { -magnitude } else { magnitude }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn flat_block_has_zero_variance() {
        let block = [[0i32; 8]; 8];
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!((y_dir, var), (0, 0), "flat block: yDir 0, var 0");
    }

    #[test]
    fn horizontal_gradient_picks_a_horizontal_direction() {
        let mut block = [[0i32; 8]; 8];
        for (i, row) in block.iter_mut().enumerate() {
            for cell in row.iter_mut() {
                *cell = i as i32 * 16 - 56; // distinct per-row value, zero mean-ish
            }
        }
        let (y_dir, var) = cdef_direction(&block);
        assert_eq!(y_dir, 2, "row-varying block selects direction 2");
        assert!(var > 0, "a non-flat block has positive variance: var={var}");
    }

    #[test]
    fn padded_direction_matches_materialized_block() {
        for coeff_shift in [0u32, 2, 4] {
            let mut state = 0x9e37_79b9u32 ^ coeff_shift;
            let mut pad = [0u16; CDEF_PADDED_AREA];
            let mut block = [[0i32; 8]; 8];
            for (i, row) in block.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let sample = ((state >> 16) & ((256 << coeff_shift) - 1)) as u16;
                    pad[(i + 2) * CDEF_PADDED_SIDE + j + 2] = sample;
                    *cell = (i32::from(sample) >> coeff_shift) - 128;
                }
            }
            assert_eq!(
                cdef_direction_padded(&pad, coeff_shift),
                cdef_direction(&block),
                "coeff_shift={coeff_shift}"
            );
        }
    }

    #[test]
    fn constrain_with_adj_matches_cdef_constrain() {
        for threshold in [0, 1, 3, 8, 63, 256] {
            for damping in [3, 5, 8] {
                let adj = constrain_damping_adj(threshold, damping);
                for diff in -300..=300 {
                    assert_eq!(
                        cdef_constrain(diff, threshold, damping),
                        constrain_with_adj(diff, threshold, adj),
                        "threshold={threshold} damping={damping} diff={diff}"
                    );
                }
            }
        }
    }

    #[test]
    fn constrain_matches_spec_branches() {
        assert_eq!(cdef_constrain(50, 0, 4), 0, "zero threshold returns 0");
        assert_eq!(cdef_constrain(50, 4, 4), 0, "large diff clamps to 0");
        assert_eq!(
            cdef_constrain(3, 4, 4),
            3,
            "small positive diff passes through"
        );
        assert_eq!(cdef_constrain(-3, 4, 4), -3, "small negative diff negated");
        assert_eq!(
            cdef_constrain(6, 4, 4),
            3,
            "mid diff clamps to threshold-rampdown"
        );
    }

    #[test]
    fn filter_with_all_unavailable_taps_is_identity() {
        let unavail = CdefTap {
            value: 0,
            available: false,
        };
        let taps = CdefSampleTaps {
            center: 130,
            primary: [[unavail; 2]; 2],
            secondary: [[[unavail; 2]; 2]; 2],
        };
        assert_eq!(
            cdef_filter_sample(&taps, 8, 8, 4, 0),
            130,
            "no taps -> identity"
        );
    }

    #[test]
    fn filter_pulls_center_toward_a_brighter_primary_neighbour() {
        let avail = |v| CdefTap {
            value: v,
            available: true,
        };
        let unavail = CdefTap {
            value: 0,
            available: false,
        };
        let taps = CdefSampleTaps {
            center: 100,
            primary: [[unavail, avail(108)], [unavail, unavail]],
            secondary: [[[unavail; 2]; 2]; 2],
        };
        assert_eq!(
            cdef_filter_sample(&taps, 8, 8, 4, 0),
            101,
            "pulled +1 toward neighbour"
        );
    }

    fn per_sample_reference(
        pad: &[u16; CDEF_PADDED_AREA],
        i: usize,
        j: usize,
        filter: &CdefBlockFilter,
    ) -> i32 {
        let at = |dy: isize, dx: isize| -> CdefTap {
            let row = (i + 2).wrapping_add_signed(dy);
            let col = (j + 2).wrapping_add_signed(dx);
            CdefTap {
                value: i32::from(pad[row * CDEF_PADDED_SIDE + col]),
                available: true,
            }
        };
        let fetch = |dir: usize, k: usize, sign: isize| -> CdefTap {
            at(
                sign * CDEF_DIRECTIONS[dir & 7][k][0] as isize,
                sign * CDEF_DIRECTIONS[dir & 7][k][1] as isize,
            )
        };
        let mut taps = CdefSampleTaps {
            center: i32::from(pad[(i + 2) * CDEF_PADDED_SIDE + (j + 2)]),
            primary: [[CdefTap {
                value: 0,
                available: false,
            }; 2]; 2],
            secondary: [[[CdefTap {
                value: 0,
                available: false,
            }; 2]; 2]; 2],
        };
        for k in 0..2 {
            for (sign_index, sign) in [-1isize, 1].into_iter().enumerate() {
                taps.primary[k][sign_index] = fetch(filter.dir, k, sign);
                for (dir_off_index, dir_off) in [6usize, 2].into_iter().enumerate() {
                    taps.secondary[k][sign_index][dir_off_index] =
                        fetch(filter.dir + dir_off, k, sign);
                }
            }
        }
        cdef_filter_sample(
            &taps,
            filter.pri_str,
            filter.sec_str,
            filter.damping,
            filter.coeff_shift,
        )
    }

    #[test]
    fn block_interior_kernel_matches_per_sample_filter() {
        for (coeff_shift, max_sample) in [(0u32, 255u32), (2, 1023), (4, 4095)] {
            let mut state = 0x1234_5678u32 ^ (coeff_shift * 77);
            let mut pad = [0u16; CDEF_PADDED_AREA];
            for cell in &mut pad {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *cell = ((state >> 16) % (max_sample + 1)) as u16;
            }
            for dir in 0..8usize {
                for (pri, sec) in [(0, 0), (1, 0), (0, 2), (4, 2), (7, 3), (63, 63)] {
                    for damping_base in [3i32, 4, 6] {
                        let filter = CdefBlockFilter {
                            pri_str: pri << coeff_shift,
                            sec_str: sec << coeff_shift,
                            damping: damping_base + coeff_shift as i32,
                            dir,
                            coeff_shift,
                        };
                        for (w, h) in [(8usize, 8usize), (4, 8), (4, 4), (5, 3)] {
                            let mut out = [0u16; 64];
                            cdef_filter_block_interior(&pad, w, h, &filter, &mut out);
                            if w == 4 || w == 8 {
                                let mut strided = [u16::MAX; 104];
                                assert!(cdef_filter_block_interior_to(
                                    &pad,
                                    w,
                                    h,
                                    &filter,
                                    &mut strided,
                                    13,
                                ));
                                for i in 0..h {
                                    assert_eq!(
                                        &strided[i * 13..i * 13 + w],
                                        &out[i * w..i * w + w]
                                    );
                                }
                            }
                            for i in 0..h {
                                for j in 0..w {
                                    assert_eq!(
                                        i32::from(out[i * w + j]),
                                        per_sample_reference(&pad, i, j, &filter),
                                        "shift={coeff_shift} dir={dir} pri={pri} sec={sec} \
                                         damping={damping_base} w={w} h={h} i={i} j={j}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        let pad = [128u16; CDEF_PADDED_AREA];
        let filter = CdefBlockFilter {
            pri_str: 4,
            sec_str: 2,
            damping: 3,
            dir: 0,
            coeff_shift: 0,
        };
        let mut out = [u16::MAX; 64];
        assert!(!cdef_filter_block_interior_to(
            &pad, 8, 8, &filter, &mut out, 7,
        ));
        assert!(out.iter().all(|&sample| sample == u16::MAX));
    }

    #[test]
    fn uv_dir_table_is_the_spec_remap() {
        // §7.18.1 Cdef_Uv_Dir: spot-check the 4:2:0 (subX=1, subY=1) row.
        assert_eq!(
            CDEF_UV_DIR[1][1],
            [0, 1, 2, 3, 4, 5, 6, 7],
            "4:2:0 is identity"
        );
        assert_eq!(
            CDEF_UV_DIR[0][0],
            [0, 1, 2, 3, 4, 5, 6, 7],
            "4:4:4 is identity"
        );
        assert_eq!(CDEF_UV_DIR[0][1][1], 2, "4:2:2 row remaps yDir 1 -> 2");
    }

    #[test]
    fn directions_table_matches_spec() {
        assert_eq!(CDEF_DIRECTIONS[0], [[-1, 1], [-2, 2]]);
        assert_eq!(CDEF_DIRECTIONS[7], [[1, 0], [2, -1]]);
    }
}
