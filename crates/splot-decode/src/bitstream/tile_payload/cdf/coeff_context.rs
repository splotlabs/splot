// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 coefficient-symbol CDF context derivation.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_SIG_COEF_CONTEXTS_2D, SIG_REF_DIFF_OFFSET_NUM};
use splot_core::tables::conversion::SIG_REF_DIFF_OFFSET;

use super::super::coeff_state::{DC_CATEGORY_SHIFT, LEVEL_INDEX_MASK, LevelGrid};

const SIG_COEF_CONTEXTS_EOB: usize = 4;

pub(crate) const LF_SIG_COEF_CONTEXTS_2D_UV: usize = 8;

pub(crate) const SIG_OFFSETS: [[[i32; 2]; SIG_REF_DIFF_OFFSET_NUM]; 3] = SIG_REF_DIFF_OFFSET;

pub(crate) const MAX_BASE_BR_RANGE: u32 = 6;

pub(crate) const MAG_REF_OFFSET_WITH_TX_CLASS: [[[usize; 2]; 3]; 3] = [
    [[0, 1], [1, 0], [1, 1]], // TX_CLASS_2D
    [[0, 1], [1, 0], [0, 2]], // TX_CLASS_HORIZ
    [[0, 1], [1, 0], [2, 0]], // TX_CLASS_VERT
];

pub(crate) const CHROMA_2D_PLANE_CONTEXT_OFFSET: [usize; 3] = [4, 0, 4];
pub(crate) const LF_2D_CONTEXT_CAP_AND_OFFSET: [(usize, usize); 3] = [(8, 0), (6, 9), (4, 16)];
pub(crate) const HF_2D_CONTEXT_OFFSET: [usize; 3] = [0, 5, 10];

/// Every neighbour offset `[row, col]` the `coeff_base` and `coeff_br`
/// contexts of class index `tx_class` read.
pub(crate) fn context_offsets(tx_class: usize) -> [[usize; 2]; 8] {
    let class = tx_class_idx(tx_class);
    let sig = SIG_OFFSETS[class].map(|[r, c]| [r as usize, c as usize]);
    let mag = MAG_REF_OFFSET_WITH_TX_CLASS[class];
    [
        sig[0], sig[1], sig[2], sig[3], sig[4], mag[0], mag[1], mag[2],
    ]
}

const fn tx_class_idx(tx_class: usize) -> usize {
    if tx_class < 3 { tx_class } else { 0 }
}

/// Reads one level from the stride-padded grid kept by
/// `TransformCoeffBlockState`; the `LEVEL_GRID_PAD` zero rows and columns
/// below and right of the block absorb every § 8.3.2 neighbor offset, so no
/// per-axis boundary handling is needed.
#[inline]
fn clamped_level_at(level: &[u8], stride: usize, row: usize, col: usize, limit: u32) -> u32 {
    level
        .get(row.wrapping_mul(stride).wrapping_add(col))
        .map_or(0, |&value| u32::from(value).min(limit))
}

/// The clamped level of the neighbour `(dr, dc)` of the flat position
/// `base`. Block positions and neighbour offsets are below the index
/// mask, so masking never changes a valid index.
#[inline]
fn level_near(
    level: &LevelGrid,
    base: usize,
    stride: usize,
    dr: usize,
    dc: usize,
    limit: u32,
) -> u32 {
    let offset = dr.wrapping_mul(stride).wrapping_add(dc);
    u32::from(level[(base & LEVEL_INDEX_MASK) + (offset & LEVEL_INDEX_MASK)]).min(limit)
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
    #[allow(
        clippy::inline_always,
        reason = "measured: plain inline leaves two out-of-line calls in the level loop"
    )]
    #[inline(always)]
    pub(crate) fn ctx(self, level: &LevelGrid) -> usize {
        let is_dc = self.row == 0 && self.col == 0;
        let class_idx = tx_class_idx(self.tx_class);
        let mag = match class_idx {
            1 => self.neighbour_mag::<1>(level),
            2 => self.neighbour_mag::<2>(level),
            _ => self.neighbour_mag::<0>(level),
        };
        let mag = ((mag + 1) >> 1).min(MAX_BASE_BR_RANGE) as usize;
        if self.plane > 0 {
            mag.min(3)
        } else if (is_dc && class_idx != 0) || (!is_dc && self.is_lf) {
            mag + 7
        } else {
            mag
        }
    }

    /// Clamped neighbour sum for one class; a constant class makes every
    /// offset a constant.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn neighbour_mag<const CLASS: usize>(self, level: &LevelGrid) -> u32 {
        let base = self.row.wrapping_mul(self.stride).wrapping_add(self.col);
        let at = |[dr, dc]: [usize; 2]| {
            level_near(level, base, self.stride, dr, dc, MAX_BASE_BR_RANGE - 1)
        };
        let offsets = MAG_REF_OFFSET_WITH_TX_CLASS[CLASS];
        let mag = at(offsets[0]) + at(offsets[1]);
        if CLASS != 0 && self.plane > 0 {
            mag
        } else {
            mag + at(offsets[2])
        }
    }
}

#[inline]
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

#[inline]
pub(crate) fn coeff_base_idtx_ctx(level: &[u8], row: usize, col: usize, stride: usize) -> usize {
    idtx_neighbour_mag(level, row, col, stride, 3) as usize
}

#[inline]
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
    #[allow(
        clippy::inline_always,
        reason = "the coefficient loop calls this per coefficient; keep it inlined beside the encoder facade"
    )]
    #[inline(always)]
    pub(crate) fn select(&self, level: &LevelGrid) -> CoeffBaseSelection {
        let class_idx = tx_class_idx(self.tx_class);
        let mag = match class_idx {
            1 => self.neighbour_mag::<1>(level),
            2 => self.neighbour_mag::<2>(level),
            _ => self.neighbour_mag::<0>(level),
        };
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

    /// Clamped neighbour sum for one class; a constant class makes every
    /// offset a constant.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn neighbour_mag<const CLASS: usize>(&self, level: &LevelGrid) -> u32 {
        let lf_limit = self.is_lf && !(self.is_hidden && self.c == 0);
        let near_limit = if lf_limit { 5 } else { 3 };
        let far_limit = if lf_limit && CLASS == 0 { 5 } else { 3 };
        let base = self.row.wrapping_mul(self.stride).wrapping_add(self.col);
        let at = |[dr, dc]: [i32; 2], limit| {
            level_near(level, base, self.stride, dr as usize, dc as usize, limit)
        };
        let offsets = SIG_OFFSETS[CLASS];
        let mag = at(offsets[0], near_limit) + at(offsets[1], near_limit);
        if self.plane == 0 {
            mag + at(offsets[2], far_limit) + at(offsets[3], far_limit) + at(offsets[4], far_limit)
        } else if CLASS == 0 {
            mag + at(offsets[2], far_limit)
        } else {
            mag
        }
    }
}

/// The `dc_sign` context (§ 8.3.2) from the above and left context lines
/// (see `coeff_state::DC_CATEGORY_SHIFT`) of a block at `x4, y4` of `w4` by `h4`.
pub(crate) fn dc_sign_ctx(
    above: &[u8],
    left: &[u8],
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
) -> usize {
    let dc_sign = dc_sign_votes(above, x4, w4) + dc_sign_votes(left, y4, h4);
    match dc_sign.cmp(&0) {
        core::cmp::Ordering::Less => 1,
        core::cmp::Ordering::Greater => 2,
        core::cmp::Ordering::Equal => 0,
    }
}

/// The number of zero bytes in `word`; padding bytes past a short slice
/// are zero before the category xor, so they never count.
const fn zero_bytes(word: u64) -> i32 {
    const LOW7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    (!((word & LOW7).wrapping_add(LOW7) | word | LOW7)).count_ones() as i32
}

/// The `dcCategory` of a context-line byte.
const fn dc_category(byte: u8) -> u8 {
    byte >> DC_CATEGORY_SHIFT
}

/// Net sign of the `len` DC categories of the context-line bytes from
/// `start`; categories are random per block, so the sum is branchless.
#[allow(clippy::inline_always, reason = "measured: out of line twice per DC")]
#[inline(always)]
fn dc_sign_votes(line: &[u8], start: usize, len: usize) -> i32 {
    let vote = |&byte: &u8| i32::from(dc_category(byte) == 2) - i32::from(dc_category(byte) == 1);
    let Some(slice) = start.checked_add(len).and_then(|end| line.get(start..end)) else {
        return line
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take(len)
            .map(vote)
            .sum();
    };
    match len {
        0 => 0,
        1 => vote(&slice[0]),
        2 => vote(&slice[0]) + vote(&slice[1]),
        4 | 8 | 16 | 32 => slice
            .chunks_exact(len.min(8))
            .map(|word| {
                let mut bytes = [0u8; 8];
                bytes[..word.len()].copy_from_slice(word);
                let word = (u64::from_ne_bytes(bytes) >> DC_CATEGORY_SHIFT) & 0x0303_0303_0303_0303;
                zero_bytes(word ^ 0x0202_0202_0202_0202) - zero_bytes(word ^ 0x0101_0101_0101_0101)
            })
            .sum(),
        _ => slice.iter().map(vote).sum(),
    }
}

#[inline]
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
