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

const fn tx_class_idx(tx_class: usize) -> usize {
    if tx_class < 3 { tx_class } else { 0 }
}

/// Reads one level from the stride-padded grid kept by
/// `TransformCoeffBlockState`; the `LEVEL_GRID_PAD` zero rows and columns
/// below and right of the block absorb every § 8.3.2 neighbor offset, so no
/// per-axis boundary handling is needed.
fn clamped_level_at(level: &[u8], stride: usize, row: usize, col: usize, limit: u32) -> u32 {
    level
        .get(row.wrapping_mul(stride).wrapping_add(col))
        .map_or(0, |&value| u32::from(value).min(limit))
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
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) stride: usize,
    pub(crate) plane: usize,
    pub(crate) is_lf: bool,
    pub(crate) tx_class: usize,
}

impl CoeffBrContext {
    pub(crate) fn ctx(self, level: &[u8]) -> usize {
        let is_dc = self.row == 0 && self.col == 0;
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
                self.stride,
                self.row.wrapping_add(off[0]),
                self.col.wrapping_add(off[1]),
                clamp,
            );
            idx += 1;
        }
        let mag = ((mag + 1) >> 1).min(MAX_BASE_BR_RANGE) as usize;
        if self.plane > 0 {
            mag.min(3)
        } else if (is_dc && class_idx != 0) || (!is_dc && self.is_lf) {
            mag + 7
        } else {
            mag
        }
    }
}

fn idtx_neighbour_mag(level: &[u8], row: usize, col: usize, stride: usize, clamp: u32) -> u32 {
    let mut mag = 0u32;
    if col > 0 {
        mag += clamped_level_at(level, stride, row, col - 1, clamp);
    }
    if row > 0 {
        mag += clamped_level_at(level, stride, row - 1, col, clamp);
    }
    mag
}

pub(crate) fn coeff_base_idtx_ctx(level: &[u8], row: usize, col: usize, stride: usize) -> usize {
    idtx_neighbour_mag(level, row, col, stride, 3) as usize
}

pub(crate) fn coeff_br_idtx_ctx(level: &[u8], row: usize, col: usize, stride: usize) -> usize {
    let mag = idtx_neighbour_mag(level, row, col, stride, MAX_BASE_BR_RANGE - 1);
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
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) stride: usize,
    pub(crate) plane: usize,
    pub(crate) is_lf: bool,
    pub(crate) is_hidden: bool,
    pub(crate) c: usize,
    pub(crate) tx_class: usize,
}

impl CoeffBaseContext {
    pub(crate) fn select(&self, level: &[u8]) -> CoeffBaseSelection {
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
                self.stride,
                self.row.wrapping_add(off[0] as usize),
                self.col.wrapping_add(off[1] as usize),
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
                } else if self.row + self.col < 2 {
                    1
                } else {
                    2
                };
                let (cap, offset) = LF_2D_CONTEXT_CAP_AND_OFFSET[bucket];
                ctx.min(cap) + offset
            } else {
                let lidx = [self.row, self.col, self.row][class_idx];
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
            let diagonal = self.row + self.col;
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
    quant_sign: &[i8],
    level: &[u8],
    row: usize,
    col: usize,
    stride: usize,
) -> usize {
    let mut signc: i32 = 0;
    if col > 0 {
        let idx = row.saturating_mul(stride).saturating_add(col - 1);
        if idx < quant_sign.len() {
            signc += quant_sign[idx] as i32;
        }
    }
    if row > 0 {
        let idx = (row - 1).saturating_mul(stride).saturating_add(col);
        if idx < quant_sign.len() {
            signc += quant_sign[idx] as i32;
        }
    }
    if col > 0 && row > 0 {
        let idx = (row - 1).saturating_mul(stride).saturating_add(col - 1);
        if idx < quant_sign.len() {
            signc += quant_sign[idx] as i32;
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
    let lidx = row.saturating_mul(stride).saturating_add(col);
    let level_val = if lidx < level.len() { level[lidx] } else { 0 };
    if level_val > COEFF_BASE_RANGE as u8 && ctx != 0 {
        ctx += 2;
    }
    ctx
}

#[cfg(test)]
#[path = "coeff_context_tests.rs"]
mod tests;
