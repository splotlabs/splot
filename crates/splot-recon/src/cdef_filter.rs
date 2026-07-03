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
const DIV_TABLE: [i64; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];

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
/// but the squared partials times `Div_Table` (up to `840`) accumulate well past
/// `i32`, so the cost uses `i64` accumulators per the spec's exact index mapping.
#[allow(clippy::needless_range_loop)]
pub fn cdef_direction(block: &[[i32; 8]; 8]) -> (usize, i64) {
    let mut partial = [[0i64; 15]; 8];
    for i in 0..8 {
        for j in 0..8 {
            let x = i64::from(block[i][j]);
            partial[0][i + j] += x;
            partial[1][i + j / 2] += x;
            partial[2][i] += x;
            partial[3][3 + i - j / 2] += x;
            partial[4][7 + i - j] += x;
            partial[5][3 - i / 2 + j] += x;
            partial[6][j] += x;
            partial[7][i / 2 + j] += x;
        }
    }

    let mut cost = [0i64; 8];
    for i in 0..8 {
        cost[2] += partial[2][i] * partial[2][i];
        cost[6] += partial[6][i] * partial[6][i];
    }
    cost[2] *= DIV_TABLE[8];
    cost[6] *= DIV_TABLE[8];
    for i in 0..7 {
        cost[0] += (partial[0][i] * partial[0][i] + partial[0][14 - i] * partial[0][14 - i])
            * DIV_TABLE[i + 1];
        cost[4] += (partial[4][i] * partial[4][i] + partial[4][14 - i] * partial[4][14 - i])
            * DIV_TABLE[i + 1];
    }
    cost[0] += partial[0][7] * partial[0][7] * DIV_TABLE[8];
    cost[4] += partial[4][7] * partial[4][7] * DIV_TABLE[8];
    let mut i = 1;
    while i < 8 {
        for j in 0..5 {
            cost[i] += partial[i][3 + j] * partial[i][3 + j];
        }
        cost[i] *= DIV_TABLE[8];
        for j in 0..3 {
            cost[i] += (partial[i][j] * partial[i][j] + partial[i][10 - j] * partial[i][10 - j])
                * DIV_TABLE[2 * j + 2];
        }
        i += 2;
    }

    let mut best_cost = 0i64;
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
    // `constrain`'s dampingAdj depends only on the per-call strengths, so it is
    // derived once per sample instead of once per tap.
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
