// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 coefficient-symbol CDF context derivation.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_SIG_COEF_CONTEXTS_2D, SIG_REF_DIFF_OFFSET_NUM};
use splot_core::tables::conversion::SIG_REF_DIFF_OFFSET;

const SIG_COEF_CONTEXTS_EOB: usize = 4;

const LF_SIG_COEF_CONTEXTS_2D_UV: usize = 8;

const MAX_BASE_BR_RANGE: u32 = 6;

const MAG_REF_OFFSET_WITH_TX_CLASS: [[[usize; 2]; 3]; 3] = [
    [[0, 1], [1, 0], [1, 1]], // TX_CLASS_2D
    [[0, 1], [1, 0], [0, 2]], // TX_CLASS_HORIZ
    [[0, 1], [1, 0], [2, 0]], // TX_CLASS_VERT
];

const CHROMA_2D_PLANE_CONTEXT_OFFSET: [usize; 3] = [4, 0, 4];
const LF_2D_CONTEXT_CAP_AND_OFFSET: [(usize, usize); 3] = [(8, 0), (6, 9), (4, 16)];
const HF_2D_CONTEXT_OFFSET: [usize; 3] = [0, 5, 10];

#[derive(Clone, Copy)]
struct CoeffPosition {
    row: usize,
    col: usize,
}

const fn tx_class_idx(tx_class: usize) -> usize {
    if tx_class < 3 { tx_class } else { 0 }
}

const fn coeff_position(pos: usize, bwl: u32) -> CoeffPosition {
    let row = match pos.checked_shr(bwl) {
        Some(v) => v,
        None => 0,
    };
    let shifted = match row.checked_shl(bwl) {
        Some(v) => v,
        None => 0,
    };
    CoeffPosition {
        row,
        col: pos - shifted,
    }
}

const fn clamp_u32(value: u32, limit: u32) -> u32 {
    if value < limit { value } else { limit }
}

const fn clamped_level_at(
    level: &[u32],
    row: usize,
    col: usize,
    txw: usize,
    txh: usize,
    limit: u32,
) -> u32 {
    if row >= txh || col >= txw {
        return 0;
    }
    let flat = row.saturating_mul(txw).saturating_add(col);
    if flat < level.len() {
        clamp_u32(level[flat], limit)
    } else {
        0
    }
}

/// Returns the AV2 § 8.3.2 `coeff_base_eob` context for scan position `c`.
pub(crate) const fn coeff_base_eob_ctx(c: usize, bwl: u32, height: usize) -> usize {
    let num_coeffs = match height.checked_shl(bwl) {
        Some(v) => v,
        None => usize::MAX,
    };
    if c == 0 {
        SIG_COEF_CONTEXTS_EOB - 4
    } else if c <= num_coeffs / 8 {
        SIG_COEF_CONTEXTS_EOB - 3
    } else if c <= num_coeffs / 4 {
        SIG_COEF_CONTEXTS_EOB - 2
    } else {
        SIG_COEF_CONTEXTS_EOB - 1
    }
}

/// Returns the AV2 § 8.3.2 `coeff_base_bob` context for `bob` and `seg_eob`.
pub(crate) const fn coeff_base_bob_ctx(bob: usize, seg_eob: usize) -> usize {
    if bob <= seg_eob >> 3 {
        0
    } else if bob <= seg_eob >> 2 {
        1
    } else {
        2
    }
}

/// AV2 § 8.3.2 `coeff_br` base-range context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBrContext {
    /// Coefficient scan position.
    pub(crate) pos: usize,
    /// Adjusted block width log2.
    pub(crate) bwl: u32,
    /// Adjusted block width.
    pub(crate) txw: usize,
    /// Adjusted block height.
    pub(crate) txh: usize,
    /// Plane index.
    pub(crate) plane: usize,
    /// Whether this transform block is low-frequency.
    pub(crate) is_lf: bool,
    /// Spec `txClass` value; out-of-range values use `TX_CLASS_2D`.
    pub(crate) tx_class: usize,
}

impl CoeffBrContext {
    /// Returns the AV2 § 8.3.2 `coeff_br` context from row-major `Level[]`.
    pub(crate) const fn ctx(self, level: &[u32]) -> usize {
        let pos = coeff_position(self.pos, self.bwl);
        let class_idx = tx_class_idx(self.tx_class);
        let num = if class_idx != 0 && self.plane > 0 {
            2
        } else {
            3
        };
        let clamp = MAX_BASE_BR_RANGE - 1;
        let mut mag: u32 = 0;
        let mut idx = 0;
        while idx < num {
            let off = MAG_REF_OFFSET_WITH_TX_CLASS[class_idx][idx];
            mag += clamped_level_at(
                level,
                pos.row.saturating_add(off[0]),
                pos.col.saturating_add(off[1]),
                self.txw,
                self.txh,
                clamp,
            );
            idx += 1;
        }
        let mag = clamp_u32((mag + 1) >> 1, MAX_BASE_BR_RANGE) as usize;
        if self.plane > 0 {
            if mag < 3 { mag } else { 3 }
        } else if (self.pos == 0 && class_idx != 0) || (self.pos != 0 && self.is_lf) {
            mag + 7
        } else {
            mag
        }
    }
}

const fn idtx_neighbour_mag(level: &[u32], row: usize, col: usize, txw: usize, clamp: u32) -> u32 {
    let mut mag = 0u32;
    if col > 0 {
        mag += clamped_level_at(level, row, col - 1, txw, usize::MAX, clamp);
    }
    if row > 0 {
        mag += clamped_level_at(level, row - 1, col, txw, usize::MAX, clamp);
    }
    mag
}

/// Returns the AV2 § 8.3.2 `coeff_base_idtx` context.
pub(crate) const fn coeff_base_idtx_ctx(
    level: &[u32],
    row: usize,
    col: usize,
    txw: usize,
) -> usize {
    idtx_neighbour_mag(level, row, col, txw, 3) as usize
}

/// Returns the AV2 § 8.3.2 `coeff_br_idtx` context.
pub(crate) const fn coeff_br_idtx_ctx(level: &[u32], row: usize, col: usize, txw: usize) -> usize {
    let mag = idtx_neighbour_mag(level, row, col, txw, MAX_BASE_BR_RANGE - 1);
    (if mag < 6 { mag } else { 6 }) as usize
}

/// AV2 § 8.3.2 `coeff_base` CDF bank plus context index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSelection {
    /// `TileCoeffBasePhCdf[ctx]`.
    Ph { ctx: usize },
    /// `TileCoeffBaseLfUvCdf[ctx]`.
    LfUv { ctx: usize },
    /// `TileCoeffBaseUvCdf[ctx]`.
    Uv { ctx: usize },
    /// `TileCoeffBaseLfCdf[txSzCtx][ctx][(tcqState>>1)&1]`.
    Lf { ctx: usize },
    /// `TileCoeffBaseCdf[txSzCtx][ctx][(tcqState>>1)&1]`.
    Hf { ctx: usize },
}

/// AV2 § 8.3.2 `coeff_base` CDF context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseContext {
    /// Coefficient scan position.
    pub(crate) pos: usize,
    /// Adjusted block width log2.
    pub(crate) bwl: u32,
    /// Adjusted block width.
    pub(crate) txw: usize,
    /// Adjusted block height.
    pub(crate) txh: usize,
    /// Plane index.
    pub(crate) plane: usize,
    /// Whether this transform block is low-frequency.
    pub(crate) is_lf: bool,
    /// Whether parity is hidden for this block.
    pub(crate) is_hidden: bool,
    /// Scan index of this coefficient.
    pub(crate) c: usize,
    /// Spec `txClass` value; out-of-range values use `TX_CLASS_2D`.
    pub(crate) tx_class: usize,
}

impl CoeffBaseContext {
    /// Returns the AV2 § 8.3.2 `coeff_base` bank and context.
    pub(crate) fn select(&self, level: &[u32]) -> CoeffBaseSelection {
        let pos = coeff_position(self.pos, self.bwl);
        let class_idx = tx_class_idx(self.tx_class);
        let num = if self.plane > 0 {
            if class_idx == 0 { 3 } else { 2 }
        } else {
            SIG_REF_DIFF_OFFSET_NUM
        };
        let mut mag: u32 = 0;
        let mut idx = 0;
        while idx < num {
            let off = SIG_REF_DIFF_OFFSET[class_idx][idx];
            let mag_limit: u32 =
                if self.is_lf && (class_idx == 0 || idx < 2) && !(self.is_hidden && self.c == 0) {
                    5
                } else {
                    3
                };
            mag += clamped_level_at(
                level,
                pos.row.saturating_add(off[0] as usize),
                pos.col.saturating_add(off[1] as usize),
                self.txw,
                self.txh,
                mag_limit,
            );
            idx += 1;
        }
        let ctx = ((mag + 1) >> 1) as usize;

        if self.is_hidden && self.c == 0 {
            return CoeffBaseSelection::Ph { ctx: ctx.min(4) };
        }
        if self.plane > 0 {
            let ctx2 = ctx.min(3);
            let uv_ctx = if class_idx != 0 {
                ctx2 + LF_SIG_COEF_CONTEXTS_2D_UV
            } else {
                let plane = if self.plane < CHROMA_2D_PLANE_CONTEXT_OFFSET.len() {
                    self.plane
                } else {
                    2
                };
                ctx2 + CHROMA_2D_PLANE_CONTEXT_OFFSET[plane]
            };
            return if self.is_lf {
                CoeffBaseSelection::LfUv { ctx: uv_ctx }
            } else {
                CoeffBaseSelection::Uv { ctx: uv_ctx }
            };
        }
        if self.is_lf {
            let lf_ctx = if class_idx == 0 {
                let bucket = if self.c == 0 {
                    0
                } else if pos.row + pos.col < 2 {
                    1
                } else {
                    2
                };
                let (cap, offset) = LF_2D_CONTEXT_CAP_AND_OFFSET[bucket];
                ctx.min(cap) + offset
            } else {
                let lidx = [pos.row, pos.col, pos.row][class_idx];
                if lidx == 0 {
                    LF_SIG_COEF_CONTEXTS_2D + ctx.min(6)
                } else {
                    LF_SIG_COEF_CONTEXTS_2D + 7 + ctx.min(4)
                }
            };
            return CoeffBaseSelection::Lf { ctx: lf_ctx };
        }
        let ctx2 = ctx.min(4);
        let hf_ctx = if class_idx == 0 {
            let diagonal = pos.row + pos.col;
            let bucket = usize::from(diagonal >= 6) + usize::from(diagonal >= 8);
            ctx2 + HF_2D_CONTEXT_OFFSET[bucket]
        } else {
            ctx2 + 15
        };
        CoeffBaseSelection::Hf { ctx: hf_ctx }
    }
}

/// Returns the AV2 § 8.3.2 `dc_sign` context.
pub(crate) const fn dc_sign_ctx(
    above_dc: &[u8],
    left_dc: &[u8],
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
) -> usize {
    let mut dc_sign: isize = 0;
    let mut k = 0;
    while k < w4 {
        let idx = x4.saturating_add(k);
        if idx >= above_dc.len() {
            break;
        }
        match above_dc[idx] {
            1 => dc_sign -= 1,
            2 => dc_sign += 1,
            _ => {}
        }
        k += 1;
    }
    let mut k = 0;
    while k < h4 {
        let idx = y4.saturating_add(k);
        if idx >= left_dc.len() {
            break;
        }
        match left_dc[idx] {
            1 => dc_sign -= 1,
            2 => dc_sign += 1,
            _ => {}
        }
        k += 1;
    }
    if dc_sign < 0 {
        1
    } else if dc_sign > 0 {
        2
    } else {
        0
    }
}

/// Returns the AV2 § 8.3.2 `idtx_sign` context.
pub(crate) const fn idtx_sign_ctx(
    quant_sign: &[i32],
    level: &[u32],
    row: usize,
    col: usize,
    txw: usize,
) -> usize {
    let mut signc: i32 = 0;
    if col > 0 {
        let idx = row.saturating_mul(txw).saturating_add(col - 1);
        if idx < quant_sign.len() {
            signc += quant_sign[idx];
        }
    }
    if row > 0 {
        let idx = (row - 1).saturating_mul(txw).saturating_add(col);
        if idx < quant_sign.len() {
            signc += quant_sign[idx];
        }
    }
    if col > 0 && row > 0 {
        let idx = (row - 1).saturating_mul(txw).saturating_add(col - 1);
        if idx < quant_sign.len() {
            signc += quant_sign[idx];
        }
    }
    let mut ctx: usize = if signc > 2 {
        5
    } else if signc < -2 {
        6
    } else if signc > 0 {
        1
    } else if signc < 0 {
        2
    } else {
        0
    };
    let lidx = row.saturating_mul(txw).saturating_add(col);
    let level_val = if lidx < level.len() { level[lidx] } else { 0 };
    if level_val > COEFF_BASE_RANGE && ctx != 0 {
        ctx += 2;
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coeff_base_eob_partitions_the_scan_position() {
        let (bwl, height) = (5u32, 32usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0, "c == 0");
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(
            coeff_base_eob_ctx(128, bwl, height),
            1,
            "boundary numCoeffs/8"
        );
        assert_eq!(coeff_base_eob_ctx(129, bwl, height), 2);
        assert_eq!(
            coeff_base_eob_ctx(256, bwl, height),
            2,
            "boundary numCoeffs/4"
        );
        assert_eq!(coeff_base_eob_ctx(257, bwl, height), 3);
        assert_eq!(coeff_base_eob_ctx(1023, bwl, height), 3, "last position");
    }

    #[test]
    fn coeff_base_eob_smallest_block() {
        let (bwl, height) = (2u32, 4usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0);
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(coeff_base_eob_ctx(2, bwl, height), 1, "boundary 16/8");
        assert_eq!(coeff_base_eob_ctx(3, bwl, height), 2);
        assert_eq!(coeff_base_eob_ctx(4, bwl, height), 2, "boundary 16/4");
        assert_eq!(coeff_base_eob_ctx(5, bwl, height), 3);
    }

    #[test]
    fn coeff_base_eob_is_total_for_out_of_range_shift() {
        assert_eq!(coeff_base_eob_ctx(0, u32::MAX, 32), 0);
        assert_eq!(coeff_base_eob_ctx(1, u32::MAX, 32), 1);
    }

    #[test]
    fn coeff_base_bob_partitions_the_begin_position() {
        let seg_eob = 64usize;
        assert_eq!(coeff_base_bob_ctx(0, seg_eob), 0);
        assert_eq!(coeff_base_bob_ctx(8, seg_eob), 0, "boundary segEob>>3");
        assert_eq!(coeff_base_bob_ctx(9, seg_eob), 1);
        assert_eq!(coeff_base_bob_ctx(16, seg_eob), 1, "boundary segEob>>2");
        assert_eq!(coeff_base_bob_ctx(17, seg_eob), 2);
        assert_eq!(coeff_base_bob_ctx(64, seg_eob), 2, "bob == segEob");
    }

    #[test]
    fn coeff_base_bob_zero_segment_eob() {
        assert_eq!(coeff_base_bob_ctx(0, 0), 0);
        assert_eq!(coeff_base_bob_ctx(1, 0), 2);
    }

    fn br(pos: usize, plane: usize, is_lf: bool, tx_class: usize) -> CoeffBrContext {
        CoeffBrContext {
            pos,
            bwl: 2,
            txw: 4,
            txh: 4,
            plane,
            is_lf,
            tx_class,
        }
    }

    #[test]
    fn coeff_br_dc_luma_2d_sums_clamped_neighbours() {
        let mut level = [0u32; 16];
        level[1] = 7;
        level[4] = 2;
        level[5] = 10;
        assert_eq!(br(0, 0, false, 0).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_clamps_halved_magnitude_to_six() {
        let mut level = [0u32; 16];
        level[6] = 5;
        level[9] = 5;
        level[10] = 5;
        assert_eq!(br(5, 0, false, 0).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_dc_non_2d_and_low_frequency_add_seven() {
        let zero = [0u32; 16];
        assert_eq!(br(0, 0, false, 2).ctx(&zero), 7);
        assert_eq!(br(5, 0, true, 0).ctx(&zero), 7);
        assert_eq!(br(5, 0, false, 0).ctx(&zero), 0);
    }

    #[test]
    fn coeff_br_chroma_clamps_to_three() {
        let mut level = [0u32; 16];
        level[1] = 5;
        level[4] = 5;
        level[5] = 5;
        assert_eq!(br(0, 1, false, 0).ctx(&level), 3);
    }

    #[test]
    fn coeff_br_non_2d_chroma_reads_only_two_neighbours() {
        let mut level = [0u32; 16];
        level[6] = 1;
        level[9] = 1;
        level[13] = 4;
        assert_eq!(br(5, 1, false, 2).ctx(&level), 1);
    }

    #[test]
    fn coeff_br_is_total_for_out_of_bounds_and_short_slices() {
        let full = [9u32; 16];
        assert_eq!(br(15, 0, false, 0).ctx(&full), 0);
        let short = [0u32, 9, 0, 0];
        assert_eq!(br(0, 0, false, 0).ctx(&short), 3);
    }

    #[test]
    fn coeff_br_is_total_for_pathological_geometry() {
        let level = [0u32; 16];
        let _ = CoeffBrContext {
            pos: usize::MAX,
            bwl: u32::MAX,
            txw: usize::MAX,
            txh: usize::MAX,
            plane: 0,
            is_lf: false,
            tx_class: 9,
        }
        .ctx(&level);
        let _ = CoeffBrContext {
            pos: usize::MAX,
            bwl: 2,
            txw: usize::MAX,
            txh: 4,
            plane: 1,
            is_lf: true,
            tx_class: 2,
        }
        .ctx(&level);
    }

    #[test]
    fn coeff_base_idtx_sums_clamped_left_and_above() {
        let mut lvl = [0u32; 16];
        lvl[4] = 1; // (1,0) = left of (1,1)
        lvl[1] = 9; // (0,1) = above of (1,1)
        assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 1, 4), 4);
    }

    #[test]
    fn coeff_base_idtx_skips_missing_neighbours() {
        let lvl = [7u32; 16];
        assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 0, 4), 0);
        assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 1, 4), 3);
        assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 0, 4), 3);
    }

    #[test]
    fn coeff_br_idtx_clamps_to_five_then_six() {
        let lvl = [9u32; 16];
        assert_eq!(coeff_br_idtx_ctx(&lvl, 1, 1, 4), 6);
        assert_eq!(coeff_br_idtx_ctx(&lvl, 0, 1, 4), 5);
    }

    #[test]
    fn coeff_idtx_is_total_for_short_slice_and_pathological_geometry() {
        let short = [3u32, 3];
        assert_eq!(coeff_base_idtx_ctx(&short, 1, 1, 4), 3);
        let lvl = [0u32; 4];
        let _ = coeff_base_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
        let _ = coeff_br_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
    }

    fn cb8(
        pos: usize,
        plane: usize,
        is_lf: bool,
        is_hidden: bool,
        c: usize,
        tx_class: usize,
    ) -> CoeffBaseContext {
        CoeffBaseContext {
            pos,
            bwl: 3,
            txw: 8,
            txh: 8,
            plane,
            is_lf,
            is_hidden,
            c,
            tx_class,
        }
    }

    #[test]
    fn coeff_base_luma_hf_2d_position_buckets() {
        let z = [0u32; 64];
        assert_eq!(
            cb8(0, 0, false, false, 0, 0).select(&z),
            CoeffBaseSelection::Hf { ctx: 0 }
        ); // (0,0) sum 0
        assert_eq!(
            cb8(27, 0, false, false, 5, 0).select(&z),
            CoeffBaseSelection::Hf { ctx: 5 }
        ); // (3,3) sum 6
        assert_eq!(
            cb8(36, 0, false, false, 5, 0).select(&z),
            CoeffBaseSelection::Hf { ctx: 10 }
        ); // (4,4) sum 8
    }

    #[test]
    fn coeff_base_luma_hf_non_2d_adds_fifteen() {
        let z = [0u32; 64];
        assert_eq!(
            cb8(0, 0, false, false, 1, 2).select(&z),
            CoeffBaseSelection::Hf { ctx: 15 }
        );
    }

    #[test]
    fn coeff_base_luma_lf_2d_branches() {
        let z = [0u32; 64];
        assert_eq!(
            cb8(0, 0, true, false, 0, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 0 }
        );
        assert_eq!(
            cb8(1, 0, true, false, 1, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 9 }
        );
        assert_eq!(
            cb8(9, 0, true, false, 1, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 16 }
        );
    }

    #[test]
    fn coeff_base_luma_lf_non_2d_keys_on_horiz_col_vert_row() {
        let z = [0u32; 64];
        assert_eq!(
            cb8(0, 0, true, false, 1, 1).select(&z),
            CoeffBaseSelection::Lf { ctx: 21 }
        );
        assert_eq!(
            cb8(1, 0, true, false, 1, 1).select(&z),
            CoeffBaseSelection::Lf { ctx: 28 }
        );
        assert_eq!(
            cb8(0, 0, true, false, 1, 2).select(&z),
            CoeffBaseSelection::Lf { ctx: 21 }
        );
        assert_eq!(
            cb8(9, 0, true, false, 1, 2).select(&z),
            CoeffBaseSelection::Lf { ctx: 28 }
        );
    }

    #[test]
    fn coeff_base_chroma_uv_branches() {
        let z = [0u32; 64];
        assert_eq!(
            cb8(0, 1, false, false, 1, 0).select(&z),
            CoeffBaseSelection::Uv { ctx: 0 }
        );
        assert_eq!(
            cb8(0, 2, false, false, 1, 0).select(&z),
            CoeffBaseSelection::Uv { ctx: 4 }
        );
        assert_eq!(
            cb8(0, 1, false, false, 1, 2).select(&z),
            CoeffBaseSelection::Uv { ctx: 8 }
        );
        assert_eq!(
            cb8(0, 1, true, false, 1, 0).select(&z),
            CoeffBaseSelection::LfUv { ctx: 0 }
        );
    }

    #[test]
    fn coeff_base_sums_clamped_neighbours_into_hf() {
        let mut lvl = [0u32; 64];
        for f in [1, 8, 9, 2, 16] {
            lvl[f] = 9;
        }
        assert_eq!(
            cb8(0, 0, false, false, 0, 0).select(&lvl),
            CoeffBaseSelection::Hf { ctx: 4 }
        );
    }

    #[test]
    fn coeff_base_low_frequency_maglimit_raises_to_five() {
        let mut lvl = [0u32; 64];
        lvl[1] = 9;
        assert_eq!(
            cb8(0, 0, true, false, 0, 0).select(&lvl),
            CoeffBaseSelection::Lf { ctx: 3 }
        );
    }

    #[test]
    fn coeff_base_parity_hidden_overrides_and_caps_maglimit() {
        let mut lvl = [0u32; 64];
        lvl[1] = 9;
        assert_eq!(
            cb8(0, 0, true, true, 0, 0).select(&lvl),
            CoeffBaseSelection::Ph { ctx: 2 }
        );
    }

    #[test]
    fn coeff_base_chroma_2d_reads_three_neighbours_not_five() {
        let mut lvl = [0u32; 64];
        lvl[9] = 9;
        lvl[2] = 9;
        assert_eq!(
            cb8(0, 1, false, false, 1, 0).select(&lvl),
            CoeffBaseSelection::Uv { ctx: 2 }
        );
    }

    #[test]
    fn coeff_base_is_total_for_short_slice_and_pathological_geometry() {
        let short = [0u32, 9];
        assert_eq!(
            cb8(0, 0, false, false, 0, 0).select(&short),
            CoeffBaseSelection::Hf { ctx: 2 }
        );
        let z = [0u32; 4];
        let _ = CoeffBaseContext {
            pos: usize::MAX,
            bwl: u32::MAX,
            txw: usize::MAX,
            txh: usize::MAX,
            plane: 0,
            is_lf: true,
            is_hidden: false,
            c: 0,
            tx_class: 9,
        }
        .select(&z);
    }

    #[test]
    fn dc_sign_ctx_nets_above_and_left_votes() {
        let above = [2u8, 2];
        let left = [1u8, 1];
        assert_eq!(dc_sign_ctx(&above, &left, 0, 0, 2, 2), 0);
        let above_neg = [1u8, 0];
        let z2 = [0u8, 0];
        assert_eq!(dc_sign_ctx(&above_neg, &z2, 0, 0, 2, 2), 1);
        let pos = [2u8, 2];
        assert_eq!(dc_sign_ctx(&z2, &pos, 0, 0, 2, 2), 2);
        let zeros = [0u8, 0];
        assert_eq!(dc_sign_ctx(&zeros, &zeros, 0, 0, 2, 2), 0);
    }

    #[test]
    fn dc_sign_ctx_honours_the_position_offset_and_max_bounds() {
        let above = [1u8, 2, 2]; // index 0 = -1 (skipped), 1,2 = +1 each
        let z = [0u8; 4];
        assert_eq!(dc_sign_ctx(&above, &z, 1, 0, 2, 0), 2); // +1+1 = +2 -> ctx 2
        let short = [2u8]; // only index 0 in range
        assert_eq!(dc_sign_ctx(&short, &z, 0, 0, 4, 0), 2); // only above[0]=+1 -> ctx 2
    }

    #[test]
    fn dc_sign_ctx_is_total_for_pathological_geometry() {
        let a = [2u8; 4];
        let l = [1u8; 4];
        let _ = dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, usize::MAX, usize::MAX);
        assert_eq!(dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, 4, 4), 0); // all out of range -> 0
    }

    #[test]
    fn idtx_sign_ctx_maps_signc_to_base_context() {
        let zl = [0u32; 16];
        let p3 = [1i32, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p3, &zl, 1, 1, 4), 5);
        let n3 = [-1i32, -1, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&n3, &zl, 1, 1, 4), 6);
        let p1 = [0i32, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p1, &zl, 1, 1, 4), 1);
        let n1 = [0i32, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&n1, &zl, 1, 1, 4), 2);
        assert_eq!(idtx_sign_ctx(&[0i32; 16], &zl, 1, 1, 4), 0);
    }

    #[test]
    fn idtx_sign_ctx_level_threshold_raises_nonzero_context() {
        let p1 = [0i32, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let hi = [0u32, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // 4 > 3
        assert_eq!(idtx_sign_ctx(&p1, &hi, 1, 1, 4), 3);
        let eq = [0u32, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p1, &eq, 1, 1, 4), 1);
        assert_eq!(idtx_sign_ctx(&[0i32; 16], &hi, 1, 1, 4), 0);
    }

    #[test]
    fn idtx_sign_ctx_skips_missing_edge_neighbours() {
        let zl = [0u32; 16];
        let q = [1i32; 16];
        assert_eq!(idtx_sign_ctx(&q, &zl, 0, 0, 4), 0);
        let only_left = [1i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&only_left, &zl, 0, 1, 4), 1);
        let only_above = [1i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&only_above, &zl, 1, 0, 4), 1);
    }

    #[test]
    fn idtx_sign_ctx_is_total_for_short_slices_and_pathological_geometry() {
        let q = [1i32, 1];
        let l = [9u32];
        let _ = idtx_sign_ctx(&q, &l, 1, 1, 4);
        let _ = idtx_sign_ctx(&q, &l, usize::MAX, usize::MAX, usize::MAX);
    }
}
