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

fn clamped_level_at(
    level: &[u8],
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
        u32::from(level[flat]).min(limit)
    } else {
        0
    }
}

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

pub(crate) const fn coeff_base_bob_ctx(bob: usize, seg_eob: usize) -> usize {
    if bob <= seg_eob >> 3 {
        0
    } else if bob <= seg_eob >> 2 {
        1
    } else {
        2
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBrContext {
    pub(crate) pos: usize,
    pub(crate) bwl: u32,
    pub(crate) txw: usize,
    pub(crate) txh: usize,
    pub(crate) plane: usize,
    pub(crate) is_lf: bool,
    pub(crate) tx_class: usize,
}

impl CoeffBrContext {
    pub(crate) fn ctx(self, level: &[u8]) -> usize {
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
        let mag = ((mag + 1) >> 1).min(MAX_BASE_BR_RANGE) as usize;
        if self.plane > 0 {
            mag.min(3)
        } else if (self.pos == 0 && class_idx != 0) || (self.pos != 0 && self.is_lf) {
            mag + 7
        } else {
            mag
        }
    }
}

fn idtx_neighbour_mag(level: &[u8], row: usize, col: usize, txw: usize, clamp: u32) -> u32 {
    let mut mag = 0u32;
    if col > 0 {
        mag += clamped_level_at(level, row, col - 1, txw, usize::MAX, clamp);
    }
    if row > 0 {
        mag += clamped_level_at(level, row - 1, col, txw, usize::MAX, clamp);
    }
    mag
}

pub(crate) fn coeff_base_idtx_ctx(level: &[u8], row: usize, col: usize, txw: usize) -> usize {
    idtx_neighbour_mag(level, row, col, txw, 3) as usize
}

pub(crate) fn coeff_br_idtx_ctx(level: &[u8], row: usize, col: usize, txw: usize) -> usize {
    let mag = idtx_neighbour_mag(level, row, col, txw, MAX_BASE_BR_RANGE - 1);
    mag.min(6) as usize
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSelection {
    Ph { ctx: usize },
    LfUv { ctx: usize },
    Uv { ctx: usize },
    Lf { ctx: usize },
    Hf { ctx: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseContext {
    pub(crate) pos: usize,
    pub(crate) bwl: u32,
    pub(crate) txw: usize,
    pub(crate) txh: usize,
    pub(crate) plane: usize,
    pub(crate) is_lf: bool,
    pub(crate) is_hidden: bool,
    pub(crate) c: usize,
    pub(crate) tx_class: usize,
}

impl CoeffBaseContext {
    pub(crate) fn select(&self, level: &[u8]) -> CoeffBaseSelection {
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
                let plane = self.plane.min(2);
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

pub(crate) const fn idtx_sign_ctx(
    quant_sign: &[i32],
    level: &[u8],
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
    if level_val > COEFF_BASE_RANGE as u8 && ctx != 0 {
        ctx += 2;
    }
    ctx
}

#[cfg(test)]
#[path = "coeff_context_tests.rs"]
mod tests;
