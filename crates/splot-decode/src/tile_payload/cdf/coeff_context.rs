// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 coefficient-symbol CDF context derivation.
//!
//! This module derives the per-symbol `ctx` index that selects a coefficient
//! CDF row in the § 8.3.2 Cdf selection process
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It is the coefficient
//! counterpart of [`super::block_context`] (which derives the block-mode
//! contexts).
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
//!
//! Scope:
//!
//! - the two **position-only** coefficient base contexts — `coeff_base_eob` (the
//!   end-of-block base context, keyed on the scan position relative to the
//!   transform block size) and `coeff_base_bob` (the begin-of-block base context,
//!   keyed on the begin position relative to the segment end-of-block) — pure
//!   functions of caller-supplied scan/segment scalars and caller-resolved
//!   geometry, needing no `Level[]` magnitude buffer; and
//! - the `coeff_br` coefficient base-range context ([`CoeffBrContext`]), the first
//!   context that reads the per-transform-block `Level[]` magnitudes, over a
//!   caller-provided level slice; and
//! - the two identity-transform magnitude contexts `coeff_base_idtx`
//!   ([`coeff_base_idtx_ctx`]) and `coeff_br_idtx` ([`coeff_br_idtx_ctx`]), which
//!   read only the left and above `Level[]` neighbours;
//! - the main 2D significant-coefficient context `coeff_base`
//!   ([`CoeffBaseContext`]), which selects one of five `coeff_base` banks from a
//!   neighbour-magnitude sum; and
//! - the `dc_sign` sign context ([`dc_sign_ctx`]), which sums the above/left
//!   DC-context signs; and
//! - the `idtx_sign` sign context ([`idtx_sign_ctx`]), which sums the left, above,
//!   and above-left `QuantSign[]` neighbours with a `Level[]` threshold.
//!
//! This is the complete set of § 8.3.2 coefficient-symbol CDF contexts. What
//! remains for coefficient decode is the runtime state these read — the
//! per-transform-block `Level[]` / `QuantSign[]` and the `Above`/`Left`
//! DC-context tile buffers — plus the § 5.20.7.27 `coeffs()` loop that fills them
//! and consumes these contexts. Nothing here is wired into a decode path yet, so
//! it is no-output-change (the derivations are exercised by compile-time
//! spec-contract `const` checks and unit tests, not by any decode stage).

use splot_core::tables::conversion::SIG_REF_DIFF_OFFSET;

/// AV2 § 3 `SIG_COEF_CONTEXTS_EOB`: the number of `coeff_base_eob` contexts
/// (`03-symbols.md`); the four contexts are `SIG_COEF_CONTEXTS_EOB - 4 ..=
/// SIG_COEF_CONTEXTS_EOB - 1`, i.e. `0..=3`.
const SIG_COEF_CONTEXTS_EOB: usize = 4;

/// AV2 § 3 `SIG_REF_DIFF_OFFSET_NUM` (`03-symbols.md`): the number of `coeff_base`
/// neighbour samples for luma (chroma uses 3 for 2D, 2 otherwise).
const SIG_REF_DIFF_OFFSET_NUM: usize = 5;

/// AV2 § 3 `LF_SIG_COEF_CONTEXTS_2D` (`03-symbols.md`): the low-frequency luma 2D
/// `coeff_base` context-count offset used by the non-2D low-frequency branch.
const LF_SIG_COEF_CONTEXTS_2D: usize = 21;

/// AV2 § 3 `LF_SIG_COEF_CONTEXTS_2D_UV` (`03-symbols.md`): the chroma 2D
/// `coeff_base` context-count offset used by the non-2D chroma branch.
const LF_SIG_COEF_CONTEXTS_2D_UV: usize = 8;

/// AV2 § 3 `MAX_BASE_BR_RANGE` = `COEFF_BASE_RANGE (3) + NUM_BASE_LEVELS (2) + 1`
/// (`03-symbols.md`); the `coeff_br` magnitude sum clamps each neighbour level to
/// `MAX_BASE_BR_RANGE - 1`.
const MAX_BASE_BR_RANGE: u32 = 6;

/// AV2 § 3 `COEFF_BASE_RANGE` (`03-symbols.md`); the `idtx_sign` context is raised
/// when the current `Level` exceeds it.
const COEFF_BASE_RANGE: u32 = 3;

/// AV2 § 8.3.2 `Mag_Ref_Offset_With_Tx_Class[txClass][idx][rowOrCol]`
/// (`08-parsing-process.md#s-8-3-2`): the up-to-three neighbour `(dRow, dCol)`
/// offsets the `coeff_br` magnitude sum reads, per transform class. Indexed by the
/// spec `txClass` value (`TX_CLASS_2D` = 0, `TX_CLASS_HORIZ` = 1,
/// `TX_CLASS_VERT` = 2).
const MAG_REF_OFFSET_WITH_TX_CLASS: [[[usize; 2]; 3]; 3] = [
    [[0, 1], [1, 0], [1, 1]], // TX_CLASS_2D
    [[0, 1], [1, 0], [0, 2]], // TX_CLASS_HORIZ
    [[0, 1], [1, 0], [2, 0]], // TX_CLASS_VERT
];

/// Returns the AV2 § 8.3.2 `coeff_base_eob` CDF context for the scan position
/// `c` in a transform block of caller-resolved adjusted geometry
/// (`08-parsing-process.md#s-8-3-2`).
///
/// The context partitions the scan position by the adjusted transform block's
/// coefficient count `numCoeffs = height << bwl` (the spec's
/// `Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`):
///
/// - `c == 0` → `SIG_COEF_CONTEXTS_EOB - 4` (`0`)
/// - `c <= numCoeffs / 8` → `SIG_COEF_CONTEXTS_EOB - 3` (`1`)
/// - `c <= numCoeffs / 4` → `SIG_COEF_CONTEXTS_EOB - 2` (`2`)
/// - otherwise → `SIG_COEF_CONTEXTS_EOB - 1` (`3`)
///
/// `bwl` is `Tx_Width_Log2[adjTxSz]` and `height` is `Tx_Height[adjTxSz]`, both
/// caller-resolved from the adjusted transform size (this module does not model
/// the § 9.2 conversion tables). The result is in `0..=3`. The `numCoeffs`
/// shift is computed total: an out-of-range `bwl` saturates to `usize::MAX`
/// rather than overflowing, so the function never panics.
pub(crate) const fn coeff_base_eob_ctx(c: usize, bwl: u32, height: usize) -> usize {
    // numCoeffs = height << bwl, computed without a panic on a bad shift width.
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

/// Returns the AV2 § 8.3.2 `coeff_base_bob` CDF context for the begin-of-block
/// position `bob` relative to the segment end-of-block `seg_eob`
/// (`08-parsing-process.md#s-8-3-2`).
///
/// - `bob <= seg_eob >> 3` → `0`
/// - `bob <= seg_eob >> 2` → `1`
/// - otherwise → `2`
///
/// The result is in `0..=2`. Pure function of the two caller-supplied scalars;
/// total and panic-free.
pub(crate) const fn coeff_base_bob_ctx(bob: usize, seg_eob: usize) -> usize {
    if bob <= seg_eob >> 3 {
        0
    } else if bob <= seg_eob >> 2 {
        1
    } else {
        2
    }
}

/// AV2 § 8.3.2 `coeff_br` coefficient base-range CDF context derivation.
///
/// `coeff_br` selects `TileCoeffBrUvCdf[ctx]` (chroma), `TileCoeffBrLfCdf[ctx]`
/// (low-frequency luma), or `TileCoeffBrCdf[ctx]` (luma) from the `ctx` derived
/// here (`08-parsing-process.md#s-8-3-2`). The context sums up to three
/// neighbouring `Level[]` magnitudes (each clamped to `MAX_BASE_BR_RANGE - 1`) at
/// the transform-class-specific offsets, halves and clamps the sum to `0..=6`,
/// then offsets it by plane / DC-position / low-frequency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBrContext {
    /// The coefficient scan position `pos` within the adjusted transform block.
    pub(crate) pos: usize,
    /// `Tx_Width_Log2[adjTxSz]` — the adjusted block width log2 (`row`/`col`
    /// split of `pos`).
    pub(crate) bwl: u32,
    /// `Tx_Width[adjTxSz]` — the adjusted block width (column bound + row stride).
    pub(crate) txw: usize,
    /// `Tx_Height[adjTxSz]` — the adjusted block height (row bound).
    pub(crate) txh: usize,
    /// The plane index (`0` luma, `> 0` chroma).
    pub(crate) plane: usize,
    /// Whether this transform block is low-frequency (`isLf`).
    pub(crate) is_lf: bool,
    /// The spec `txClass` value: `0` = `TX_CLASS_2D`, `1` = `TX_CLASS_HORIZ`,
    /// `2` = `TX_CLASS_VERT` — the caller-resolved `get_tx_class(PlaneTxType)`
    /// result (kept a scalar here so the entropy CDF-selection layer does not
    /// import a reconstruction transform-class type). An out-of-range value is
    /// treated as `TX_CLASS_2D`.
    pub(crate) tx_class: usize,
}

impl CoeffBrContext {
    /// The bounded `txClass` index into [`MAG_REF_OFFSET_WITH_TX_CLASS`]: the
    /// caller-resolved `tx_class` when in `0..3`, else `0` (`TX_CLASS_2D`), so the
    /// table access is total.
    const fn class_idx(self) -> usize {
        if self.tx_class < 3 { self.tx_class } else { 0 }
    }

    /// Returns the AV2 § 8.3.2 `coeff_br` CDF context, reading the
    /// per-transform-block `Level[]` magnitudes from `level` (row-major,
    /// `txw`-wide; `level[row * txw + col]`).
    ///
    /// Neighbour reads outside the block bounds (`refRow >= txh` or
    /// `refCol >= txw`) or past the `level` slice contribute `0`, matching the
    /// spec's `refRow < txh && refCol < txw` guard. All geometry arithmetic is
    /// checked or saturating — the shift width (`pos >> bwl` / `row << bwl`), and
    /// the flat index (`row * txw + col`) — so the function is total and never
    /// panics for any caller-provided geometry. The result is the spec `ctx`:
    /// `0..=13` (luma / low-frequency) or `0..=3` (chroma).
    pub(crate) const fn ctx(self, level: &[u32]) -> usize {
        // row = pos >> bwl, col = pos - (row << bwl), with a guarded shift width
        // (a malformed bwl >= the word width yields a degenerate but total result
        // rather than a shift-overflow panic).
        let row = match self.pos.checked_shr(self.bwl) {
            Some(v) => v,
            None => 0,
        };
        let shifted = match row.checked_shl(self.bwl) {
            Some(v) => v,
            None => 0,
        };
        let col = self.pos - shifted;
        let class_idx = self.class_idx();
        // num = 3, or 2 for non-2D chroma (§ 8.3.2).
        let num = if class_idx != 0 && self.plane > 0 {
            2
        } else {
            3
        };
        let clamp = MAX_BASE_BR_RANGE - 1;
        let mut mag: u32 = 0;
        let mut idx = 0;
        // `while` (not `for`): iterators are not permitted in a `const fn`.
        while idx < num {
            // Saturating geometry: an out-of-range offset or stride saturates
            // past the block bounds (so it is skipped) instead of overflowing.
            let ref_row = row.saturating_add(MAG_REF_OFFSET_WITH_TX_CLASS[class_idx][idx][0]);
            let ref_col = col.saturating_add(MAG_REF_OFFSET_WITH_TX_CLASS[class_idx][idx][1]);
            if ref_row < self.txh && ref_col < self.txw {
                let flat = ref_row.saturating_mul(self.txw).saturating_add(ref_col);
                if flat < level.len() {
                    let lvl = level[flat];
                    mag += if lvl < clamp { lvl } else { clamp };
                }
            }
            idx += 1;
        }
        // mag = Min((mag + 1) >> 1, 6)
        let halved = (mag + 1) >> 1;
        let mag = (if halved < 6 { halved } else { 6 }) as usize;
        if self.plane > 0 {
            if mag < 3 { mag } else { 3 }
        } else if self.pos == 0 {
            if class_idx != 0 { mag + 7 } else { mag }
        } else if self.is_lf {
            mag + 7
        } else {
            mag
        }
    }
}

/// The shared AV2 § 8.3.2 identity-transform magnitude sum: the left
/// (`Level[row][col-1]`) and above (`Level[row-1][col]`) neighbour magnitudes,
/// each clamped to `clamp`, over a caller-provided row-major `txw`-wide `level`
/// slice. Geometry is saturating and the flat index is slice-bounds-guarded, so
/// out-of-range or short-slice reads contribute `0` and the helper never panics.
const fn idtx_neighbour_mag(level: &[u32], row: usize, col: usize, txw: usize, clamp: u32) -> u32 {
    let mut mag = 0u32;
    if col > 0 {
        let flat = row.saturating_mul(txw).saturating_add(col - 1);
        if flat < level.len() {
            let v = level[flat];
            mag += if v < clamp { v } else { clamp };
        }
    }
    if row > 0 {
        let flat = (row - 1).saturating_mul(txw).saturating_add(col);
        if flat < level.len() {
            let v = level[flat];
            mag += if v < clamp { v } else { clamp };
        }
    }
    mag
}

/// Returns the AV2 § 8.3.2 `coeff_base_idtx` CDF context — the spec `mag`, used
/// directly as the inner index of `TileCoeffBaseIdtxCdf[Min(TX_16X16, txSzCtx)]`
/// (`08-parsing-process.md#s-8-3-2`).
///
/// `mag = Min(3, Level[row][col-1]) + Min(3, Level[row-1][col])` (each neighbour
/// included only when in range). `level` is a caller-provided row-major
/// `txw`-wide `Level[]` slice; out-of-range or short-slice reads contribute `0`,
/// so the function is total and never panics. The result is in `0..=6`.
pub(crate) const fn coeff_base_idtx_ctx(
    level: &[u32],
    row: usize,
    col: usize,
    txw: usize,
) -> usize {
    // The base-level clamp is 3 (= COEFF_BASE_RANGE) per § 8.3.2.
    idtx_neighbour_mag(level, row, col, txw, 3) as usize
}

/// Returns the AV2 § 8.3.2 `coeff_br_idtx` CDF context — the spec `mag`, used
/// directly as the inner index of `TileCoeffBrIdtxCdf[Min(TX_16X16, txSzCtx)]`
/// (`08-parsing-process.md#s-8-3-2`).
///
/// `mag = Min(MAX_BASE_BR_RANGE-1, Level[row][col-1]) + Min(MAX_BASE_BR_RANGE-1,
/// Level[row-1][col])`, then `mag = Min(mag, 6)`. `level` is a caller-provided
/// row-major `txw`-wide `Level[]` slice; out-of-range or short-slice reads
/// contribute `0`, so the function is total and never panics. The result is in
/// `0..=6`.
pub(crate) const fn coeff_br_idtx_ctx(level: &[u32], row: usize, col: usize, txw: usize) -> usize {
    let mag = idtx_neighbour_mag(level, row, col, txw, MAX_BASE_BR_RANGE - 1);
    (if mag < 6 { mag } else { 6 }) as usize
}

/// The AV2 § 8.3.2 `coeff_base` CDF bank selected for a coefficient, plus its
/// context index (`08-parsing-process.md#s-8-3-2`). The caller maps the variant
/// to the bank, supplying the `txSzCtx` / `tcqState` dimensions the [`Lf`] and
/// [`Hf`] banks carry.
///
/// [`Lf`]: CoeffBaseSelection::Lf
/// [`Hf`]: CoeffBaseSelection::Hf
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSelection {
    /// `TileCoeffBasePhCdf[ctx]` — the parity-hidden DC coefficient (`isHidden`
    /// and `c == 0`); `ctx` is `Min((mag+1)>>1, 4)`.
    Ph { ctx: usize },
    /// `TileCoeffBaseLfUvCdf[ctx]` — chroma low-frequency.
    LfUv { ctx: usize },
    /// `TileCoeffBaseUvCdf[ctx]` — chroma.
    Uv { ctx: usize },
    /// `TileCoeffBaseLfCdf[txSzCtx][ctx][(tcqState>>1)&1]` — luma low-frequency.
    Lf { ctx: usize },
    /// `TileCoeffBaseCdf[txSzCtx][ctx][(tcqState>>1)&1]` — luma high-frequency.
    Hf { ctx: usize },
}

/// AV2 § 8.3.2 `coeff_base` CDF context derivation — the main 2D significant-
/// coefficient context (`08-parsing-process.md#s-8-3-2`).
///
/// It sums the significant-neighbour `Level[]` magnitudes (each clamped by a
/// position-dependent `magLimit`) at the `Sig_Ref_Diff_Offset` offsets for the
/// transform class, forms `ctx = (mag+1) >> 1`, and selects one of the five
/// `coeff_base` banks ([`CoeffBaseSelection`]) with its bank-specific context
/// offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseContext {
    /// The coefficient scan position `pos` within the adjusted transform block.
    pub(crate) pos: usize,
    /// `Tx_Width_Log2[adjTxSz]` — the adjusted block width log2 (`row`/`col`
    /// split of `pos`).
    pub(crate) bwl: u32,
    /// `Tx_Width[adjTxSz]` — the adjusted block width (column bound + row stride).
    pub(crate) txw: usize,
    /// `Tx_Height[adjTxSz]` — the adjusted block height (row bound).
    pub(crate) txh: usize,
    /// The plane index (`0` luma, `1` U, `2` V).
    pub(crate) plane: usize,
    /// Whether this transform block is low-frequency (`isLf`).
    pub(crate) is_lf: bool,
    /// Whether the parity is hidden for this block (`isHidden`).
    pub(crate) is_hidden: bool,
    /// The scan index `c` of this coefficient.
    pub(crate) c: usize,
    /// The spec `txClass` value: `0` = `TX_CLASS_2D`, `1` = `TX_CLASS_HORIZ`,
    /// `2` = `TX_CLASS_VERT` (caller-resolved; out-of-range treated as 2D).
    pub(crate) tx_class: usize,
}

impl CoeffBaseContext {
    /// The bounded `txClass` index into `SIG_REF_DIFF_OFFSET`.
    fn class_idx(&self) -> usize {
        if self.tx_class < 3 { self.tx_class } else { 0 }
    }

    /// Returns the AV2 § 8.3.2 `coeff_base` bank selection and context, reading
    /// the per-transform-block `Level[]` magnitudes from `level` (row-major,
    /// `txw`-wide; `level[row * txw + col]`).
    ///
    /// Geometry is checked/saturating and the flat index is slice-bounds-guarded
    /// (the spec's `refRow < height && refCol < width` guard), so out-of-range or
    /// short-slice reads contribute `0` and the function is total and never
    /// panics.
    pub(crate) fn select(&self, level: &[u32]) -> CoeffBaseSelection {
        let row = self.pos.checked_shr(self.bwl).unwrap_or(0);
        let col = self.pos - row.checked_shl(self.bwl).unwrap_or(0);
        let class_idx = self.class_idx();
        // num = 5 (luma); 3 for chroma 2D, 2 for chroma non-2D (§ 8.3.2).
        let num = if self.plane > 0 {
            if class_idx == 0 { 3 } else { 2 }
        } else {
            SIG_REF_DIFF_OFFSET_NUM
        };
        let mut mag: u32 = 0;
        let mut idx = 0;
        while idx < num {
            let off = SIG_REF_DIFF_OFFSET[class_idx][idx];
            let ref_row = row.saturating_add(off[0] as usize);
            let ref_col = col.saturating_add(off[1] as usize);
            // magLimit is 5 for the low-frequency near-DC samples, else 3.
            let mag_limit: u32 =
                if self.is_lf && (class_idx == 0 || idx < 2) && !(self.is_hidden && self.c == 0) {
                    5
                } else {
                    3
                };
            if ref_row < self.txh && ref_col < self.txw {
                let flat = ref_row.saturating_mul(self.txw).saturating_add(ref_col);
                if flat < level.len() {
                    let v = level[flat];
                    mag += if v < mag_limit { v } else { mag_limit };
                }
            }
            idx += 1;
        }
        let ctx = ((mag + 1) >> 1) as usize;

        // The parity-hidden DC coefficient overrides the plane/frequency banks.
        if self.is_hidden && self.c == 0 {
            return CoeffBaseSelection::Ph { ctx: ctx.min(4) };
        }
        if self.plane > 0 {
            let ctx2 = ctx.min(3);
            let uv_ctx = if class_idx != 0 {
                ctx2 + LF_SIG_COEF_CONTEXTS_2D_UV
            } else if self.plane == 1 {
                ctx2
            } else {
                ctx2 + 4
            };
            return if self.is_lf {
                CoeffBaseSelection::LfUv { ctx: uv_ctx }
            } else {
                CoeffBaseSelection::Uv { ctx: uv_ctx }
            };
        }
        if self.is_lf {
            let lf_ctx = if class_idx == 0 {
                if self.c == 0 {
                    ctx.min(8)
                } else if row + col < 2 {
                    ctx.min(6) + 9
                } else {
                    ctx.min(4) + 16
                }
            } else {
                // TX_CLASS_HORIZ (1) keys on col; TX_CLASS_VERT (2) keys on row.
                let lidx = if class_idx == 1 { col } else { row };
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
            if row + col < 6 {
                ctx2
            } else if row + col < 8 {
                ctx2 + 5
            } else {
                ctx2 + 10
            }
        } else {
            ctx2 + 15
        };
        CoeffBaseSelection::Hf { ctx: hf_ctx }
    }
}

/// Returns the AV2 § 8.3.2 `dc_sign` CDF context — the inner index of
/// `TileDcSignCdf[ptype][isHidden][ctx]` (`08-parsing-process.md#s-8-3-2`).
///
/// It nets the DC-sign votes of the block's above and left neighbours:
/// `AboveDcContext[plane][x4+k]` for `k` in `0..w4` and
/// `LeftDcContext[plane][y4+k]` for `k` in `0..h4`, each sign `1` decrementing and
/// sign `2` incrementing a running `dcSign`; the context is `1` if `dcSign < 0`,
/// `2` if `dcSign > 0`, else `0`.
///
/// `above_dc` is `AboveDcContext[plane]` (length `MiCols`) and `left_dc` is
/// `LeftDcContext[plane]` (length `MiRows`); the spec `x4 + k < MiCols` /
/// `y4 + k < MiRows` guards are exactly the slice bounds, so reads past either
/// slice are skipped (matching out-of-frame neighbours). Index arithmetic is
/// saturating, so the function is total and never panics.
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
    // `while` (not `for`): iterators are not permitted in a `const fn`. `idx` is
    // monotonic in `k`, so once it leaves the slice (the spec `x4 + k < MiCols`
    // bound) every later `k` is also out of range — `break` is equivalent to the
    // spec's skip-remaining and bounds the loop to the slice length (so a
    // pathological `w4` cannot spin).
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

/// Returns the AV2 § 8.3.2 `idtx_sign` CDF context — the inner index of
/// `TileIdtxSignCdf[Min(TX_16X16, txSzCtx)][ctx]` (`08-parsing-process.md#s-8-3-2`).
///
/// It nets the signs of the left (`QuantSign[row*txw + col-1]`), above
/// (`QuantSign[(row-1)*txw + col]`), and above-left (`QuantSign[(row-1)*txw +
/// col-1]`) coefficients into `signc`, maps it to a base context (`5` for `signc >
/// 2`, `6` for `signc < -2`, `1` for `signc > 0`, `2` for `signc < 0`, else `0`),
/// then adds `2` when the current `Level[row][col]` exceeds `COEFF_BASE_RANGE` and
/// the base context is non-zero.
///
/// `quant_sign` and `level` are the per-transform-block row-major `txw`-wide
/// `QuantSign[]` (signed, `-1`/`0`/`+1`) and `Level[]` slices; the edge neighbours
/// are gated by `col > 0` / `row > 0`, and the flat index is saturating and
/// slice-bounds-guarded, so the function is total and never panics. The result is
/// in `0..=8`.
pub(crate) const fn idtx_sign_ctx(
    quant_sign: &[i32],
    level: &[u32],
    row: usize,
    col: usize,
    txw: usize,
) -> usize {
    let mut signc: i32 = 0;
    // Left neighbour.
    if col > 0 {
        let idx = row.saturating_mul(txw).saturating_add(col - 1);
        if idx < quant_sign.len() {
            signc += quant_sign[idx];
        }
    }
    // Above neighbour.
    if row > 0 {
        let idx = (row - 1).saturating_mul(txw).saturating_add(col);
        if idx < quant_sign.len() {
            signc += quant_sign[idx];
        }
    }
    // Above-left neighbour.
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
    // Raise the context when the current level exceeds COEFF_BASE_RANGE.
    let lidx = row.saturating_mul(txw).saturating_add(col);
    let level_val = if lidx < level.len() { level[lidx] } else { 0 };
    if level_val > COEFF_BASE_RANGE && ctx != 0 {
        ctx += 2;
    }
    ctx
}

// Compile-time spec-contract checks. These `const` items are the non-test
// consumer of the context derivations until the §5.20.7.27 `coeffs()` decode
// loop wires them: they pin the §8.3.2 contract at the four/three boundaries
// (TX_32X32 geometry: numCoeffs = 32 << 5 = 1024, so thresholds 128 and 256;
// segEob = 64, so thresholds 8 and 16) so any drift fails the build.
const _COEFF_BASE_EOB_CONTRACT: () = {
    assert!(coeff_base_eob_ctx(0, 5, 32) == 0);
    assert!(coeff_base_eob_ctx(128, 5, 32) == 1);
    assert!(coeff_base_eob_ctx(256, 5, 32) == 2);
    assert!(coeff_base_eob_ctx(257, 5, 32) == 3);
};
const _COEFF_BASE_BOB_CONTRACT: () = {
    assert!(coeff_base_bob_ctx(0, 64) == 0);
    assert!(coeff_base_bob_ctx(16, 64) == 1);
    assert!(coeff_base_bob_ctx(17, 64) == 2);
};
const _COEFF_BR_CONTRACT: () = {
    // TX_4X4 (bwl 2, txw/txh 4). All-zero neighbours, DC luma 2D -> ctx 0.
    let zero = [0u32; 16];
    let dc_2d = CoeffBrContext {
        pos: 0,
        bwl: 2,
        txw: 4,
        txh: 4,
        plane: 0,
        is_lf: false,
        tx_class: 0, // TX_CLASS_2D
    };
    assert!(dc_2d.ctx(&zero) == 0);
    // Three saturating neighbours at (0,1),(1,0),(1,1): mag = 4+4+4 = 12 ->
    // (12 + 1) >> 1 = 6 -> DC luma 2D -> ctx 6.
    let mags = [0u32, 4, 0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(dc_2d.ctx(&mags) == 6);
    // DC luma non-2D (VERT): mag 0, pos == 0, txClass != 2D -> mag + 7 = 7.
    let dc_vert = CoeffBrContext {
        pos: 0,
        bwl: 2,
        txw: 4,
        txh: 4,
        plane: 0,
        is_lf: false,
        tx_class: 2, // TX_CLASS_VERT
    };
    assert!(dc_vert.ctx(&zero) == 7);
};
const _COEFF_IDTX_CONTRACT: () = {
    let zero = [0u32; 16];
    assert!(coeff_base_idtx_ctx(&zero, 1, 1, 4) == 0);
    assert!(coeff_br_idtx_ctx(&zero, 1, 1, 4) == 0);
    // TX_4X4: left = Level[1][0] = 2, above = Level[0][1] = 10.
    let lvl = [0u32, 10, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    // base: Min(3,2) + Min(3,10) = 2 + 3 = 5.
    assert!(coeff_base_idtx_ctx(&lvl, 1, 1, 4) == 5);
    // br: Min(5,2) + Min(5,10) = 2 + 5 = 7 -> Min(7,6) = 6.
    assert!(coeff_br_idtx_ctx(&lvl, 1, 1, 4) == 6);
};
const _DC_SIGN_CONTRACT: () = {
    let z2 = [0u8, 0];
    let z1 = [0u8];
    // No signed neighbours -> dcSign 0 -> ctx 0.
    assert!(dc_sign_ctx(&z2, &z1, 0, 0, 2, 1) == 0);
    // Two above sign-2 votes (+1 each) -> dcSign +2 -> ctx 2.
    let pos = [2u8, 2];
    assert!(dc_sign_ctx(&pos, &z1, 0, 0, 2, 1) == 2);
    // One left sign-1 vote (-1) -> dcSign -1 -> ctx 1.
    let neg = [1u8];
    assert!(dc_sign_ctx(&z2, &neg, 0, 0, 2, 1) == 1);
};
const _IDTX_SIGN_CONTRACT: () = {
    let zq = [0i32; 16];
    let zl = [0u32; 16];
    // No neighbour signs -> signc 0 -> ctx 0.
    assert!(idtx_sign_ctx(&zq, &zl, 1, 1, 4) == 0);
    // (1,1) neighbours: above-left q[0], above q[1], left q[4]. Three +1 -> signc
    // 3 -> ctx 5.
    let pos3 = [1i32, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(idtx_sign_ctx(&pos3, &zl, 1, 1, 4) == 5);
    // One +1 neighbour (above q[1]) -> signc 1 -> ctx 1; with Level[1][1]=q-cell 5
    // > COEFF_BASE_RANGE -> ctx + 2 = 3.
    let pos1 = [0i32, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let hi = [0u32, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(idtx_sign_ctx(&pos1, &hi, 1, 1, 4) == 3);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coeff_base_eob_partitions_the_scan_position() {
        // TX_32X32 adjusted: bwl = Tx_Width_Log2 = 5, height = 32, so
        // numCoeffs = 32 << 5 = 1024; thresholds 1024/8 = 128 and 1024/4 = 256.
        let (bwl, height) = (5u32, 32usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0, "c == 0");
        // 1 ..= 128 -> ctx 1.
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(
            coeff_base_eob_ctx(128, bwl, height),
            1,
            "boundary numCoeffs/8"
        );
        // 129 ..= 256 -> ctx 2.
        assert_eq!(coeff_base_eob_ctx(129, bwl, height), 2);
        assert_eq!(
            coeff_base_eob_ctx(256, bwl, height),
            2,
            "boundary numCoeffs/4"
        );
        // > 256 -> ctx 3.
        assert_eq!(coeff_base_eob_ctx(257, bwl, height), 3);
        assert_eq!(coeff_base_eob_ctx(1023, bwl, height), 3, "last position");
    }

    #[test]
    fn coeff_base_eob_smallest_block() {
        // TX_4X4 adjusted: bwl = 2, height = 4, numCoeffs = 4 << 2 = 16;
        // thresholds 16/8 = 2 and 16/4 = 4.
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
        // A pathological bwl must not panic; numCoeffs saturates so any non-zero
        // c lands in ctx 1 (c <= usize::MAX / 8).
        assert_eq!(coeff_base_eob_ctx(0, u32::MAX, 32), 0);
        assert_eq!(coeff_base_eob_ctx(1, u32::MAX, 32), 1);
    }

    #[test]
    fn coeff_base_bob_partitions_the_begin_position() {
        // segEob = 64: thresholds 64 >> 3 = 8 and 64 >> 2 = 16.
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
        // segEob = 0: both thresholds are 0, so only bob == 0 takes ctx 0.
        assert_eq!(coeff_base_bob_ctx(0, 0), 0);
        assert_eq!(coeff_base_bob_ctx(1, 0), 2);
    }

    /// A `coeff_br` context over TX_4X4 adjusted geometry (bwl 2, txw/txh 4).
    /// `tx_class` is the spec `txClass` value (0 = 2D, 1 = HORIZ, 2 = VERT).
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
        // pos 0 -> (row,col) = (0,0); 2D offsets (0,1),(1,0),(1,1) = flat 1,4,5.
        // levels 7,2,10 -> clamped to 5,2,5 (MAX_BASE_BR_RANGE-1) -> mag 12 ->
        // (12 + 1) >> 1 = 6; luma DC 2D -> ctx = mag = 6.
        let mut level = [0u32; 16];
        level[1] = 7;
        level[4] = 2;
        level[5] = 10;
        assert_eq!(br(0, 0, false, 0).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_clamps_halved_magnitude_to_six() {
        // pos 5 -> (row,col) = (1,1); 2D offsets -> flat 6,9,10, all level 5 ->
        // mag 15 -> (15 + 1) >> 1 = 8 -> clamped to 6; luma non-DC -> ctx 6.
        let mut level = [0u32; 16];
        level[6] = 5;
        level[9] = 5;
        level[10] = 5;
        assert_eq!(br(5, 0, false, 0).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_dc_non_2d_and_low_frequency_add_seven() {
        let zero = [0u32; 16];
        // DC (pos 0) luma, non-2D (VERT): mag 0 + 7 -> ctx 7.
        assert_eq!(br(0, 0, false, 2).ctx(&zero), 7);
        // Non-DC (pos 5) luma, low-frequency: mag 0 + 7 -> ctx 7.
        assert_eq!(br(5, 0, true, 0).ctx(&zero), 7);
        // Non-DC (pos 5) luma, non-LF: mag 0 -> ctx 0.
        assert_eq!(br(5, 0, false, 0).ctx(&zero), 0);
    }

    #[test]
    fn coeff_br_chroma_clamps_to_three() {
        // Chroma 2D (num 3): flat 1,4,5 all 5 -> mag 15 -> 8 -> 6; chroma
        // Min(6, 3) -> ctx 3.
        let mut level = [0u32; 16];
        level[1] = 5;
        level[4] = 5;
        level[5] = 5;
        assert_eq!(br(0, 1, false, 0).ctx(&level), 3);
    }

    #[test]
    fn coeff_br_non_2d_chroma_reads_only_two_neighbours() {
        // Chroma VERT at pos 5 -> num 2: reads offsets (0,1),(1,0) = flat 6,9 but
        // NOT the third VERT offset (2,0) = flat 13. levels 1,1 at 6,9 and 4 at 13
        // -> mag 2 -> (3 >> 1) = 1 -> chroma Min(1,3) = 1. (num 3 would read flat
        // 13 too -> mag 6 -> 3 -> Min(3,3) = 3, so ctx 1 proves only two are read.)
        let mut level = [0u32; 16];
        level[6] = 1;
        level[9] = 1;
        level[13] = 4;
        assert_eq!(br(5, 1, false, 2).ctx(&level), 1);
    }

    #[test]
    fn coeff_br_is_total_for_out_of_bounds_and_short_slices() {
        // pos 15 -> (row,col) = (3,3); every 2D offset leaves the 4x4 block, so
        // no neighbour is read -> mag 0 -> luma non-DC ctx 0 (no panic).
        let full = [9u32; 16];
        assert_eq!(br(15, 0, false, 0).ctx(&full), 0);
        // A short slice: only flat 1 is in range (flat 4,5 are past len 4) -> mag
        // = Min(5, 5) = 5 -> (6 >> 1) = 3 -> ctx 3 (no panic).
        let short = [0u32, 9, 0, 0];
        assert_eq!(br(0, 0, false, 0).ctx(&short), 3);
    }

    #[test]
    fn coeff_br_is_total_for_pathological_geometry() {
        // Malformed caller geometry must not panic: an out-of-word-width shift,
        // positions/strides near usize::MAX, and an out-of-range tx_class all take
        // the checked-shift / saturating-geometry / class-guard paths. The test
        // passing (no panic) is the assertion.
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
        // TX_4X4: left = Level[1][0], above = Level[0][1], each Min(3, .).
        let mut lvl = [0u32; 16];
        lvl[4] = 1; // (1,0) = left of (1,1)
        lvl[1] = 9; // (0,1) = above of (1,1)
        // Min(3,1) + Min(3,9) = 1 + 3 = 4.
        assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 1, 4), 4);
    }

    #[test]
    fn coeff_base_idtx_skips_missing_neighbours() {
        let lvl = [7u32; 16];
        // (0,0): no left (col 0) and no above (row 0) -> 0.
        assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 0, 4), 0);
        // (0,1): left = Level[0][0] = 7 -> Min(3,7) = 3; no above -> 3.
        assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 1, 4), 3);
        // (1,0): above = Level[0][0] = 7 -> 3; no left -> 3.
        assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 0, 4), 3);
    }

    #[test]
    fn coeff_br_idtx_clamps_to_five_then_six() {
        let lvl = [9u32; 16];
        // (1,1): left = Min(5,9) = 5, above = Min(5,9) = 5 -> 10 -> Min(10,6) = 6.
        assert_eq!(coeff_br_idtx_ctx(&lvl, 1, 1, 4), 6);
        // (0,1): only left = 5 -> Min(5,6) = 5.
        assert_eq!(coeff_br_idtx_ctx(&lvl, 0, 1, 4), 5);
    }

    #[test]
    fn coeff_idtx_is_total_for_short_slice_and_pathological_geometry() {
        // Short slice: (1,1) txw 4 -> left flat 4 is past len 2 (skipped), above
        // flat 1 is in range -> Level[1] = 3 -> base mag 3.
        let short = [3u32, 3];
        assert_eq!(coeff_base_idtx_ctx(&short, 1, 1, 4), 3);
        // Pathological geometry must not panic (saturating flat index).
        let lvl = [0u32; 4];
        let _ = coeff_base_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
        let _ = coeff_br_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
    }

    /// A `coeff_base` context over TX_8X8 adjusted geometry (bwl 3, txw/txh 8).
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
        // Zero level -> mag 0 -> ctx 0 -> ctx2 0. Luma non-LF 2D selects Hf by
        // the row+col position bucket (< 6, < 8, else).
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
        // Luma non-LF, non-2D (VERT) -> Hf{ctx2 + 15}.
        assert_eq!(
            cb8(0, 0, false, false, 1, 2).select(&z),
            CoeffBaseSelection::Hf { ctx: 15 }
        );
    }

    #[test]
    fn coeff_base_luma_lf_2d_branches() {
        let z = [0u32; 64];
        // c == 0 -> Min(ctx,8).
        assert_eq!(
            cb8(0, 0, true, false, 0, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 0 }
        );
        // c != 0, row+col < 2 -> Min(ctx,6) + 9.
        assert_eq!(
            cb8(1, 0, true, false, 1, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 9 }
        );
        // c != 0, row+col >= 2 -> Min(ctx,4) + 16.
        assert_eq!(
            cb8(9, 0, true, false, 1, 0).select(&z),
            CoeffBaseSelection::Lf { ctx: 16 }
        );
    }

    #[test]
    fn coeff_base_luma_lf_non_2d_keys_on_horiz_col_vert_row() {
        let z = [0u32; 64];
        // HORIZ keys on col: col 0 -> 21 + Min(ctx,6); col != 0 -> 21 + 7 + Min(ctx,4).
        assert_eq!(
            cb8(0, 0, true, false, 1, 1).select(&z),
            CoeffBaseSelection::Lf { ctx: 21 }
        );
        assert_eq!(
            cb8(1, 0, true, false, 1, 1).select(&z),
            CoeffBaseSelection::Lf { ctx: 28 }
        );
        // VERT keys on row: row 0 -> 21; row != 0 -> 28.
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
        // Chroma non-LF 2D: U -> ctx2; V -> ctx2 + 4.
        assert_eq!(
            cb8(0, 1, false, false, 1, 0).select(&z),
            CoeffBaseSelection::Uv { ctx: 0 }
        );
        assert_eq!(
            cb8(0, 2, false, false, 1, 0).select(&z),
            CoeffBaseSelection::Uv { ctx: 4 }
        );
        // Chroma non-LF non-2D: ctx2 + LF_SIG_COEF_CONTEXTS_2D_UV (8).
        assert_eq!(
            cb8(0, 1, false, false, 1, 2).select(&z),
            CoeffBaseSelection::Uv { ctx: 8 }
        );
        // Chroma low-frequency -> LfUv.
        assert_eq!(
            cb8(0, 1, true, false, 1, 0).select(&z),
            CoeffBaseSelection::LfUv { ctx: 0 }
        );
    }

    #[test]
    fn coeff_base_sums_clamped_neighbours_into_hf() {
        // Luma 2D non-LF at (0,0): the 5 offsets {0,1},{1,0},{1,1},{0,2},{2,0}
        // -> flats 1,8,9,2,16. Each level 9 clamps to 3 (non-LF magLimit) -> mag
        // 15 -> ctx (16>>1) = 8 -> ctx2 Min(8,4) = 4 -> Hf (row+col 0 < 6).
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
        // Luma isLf 2D, not hidden, c == 0: one neighbour (offset {0,1} -> flat 1)
        // = 9. magLimit = 5 (isLf && 2D && !(isHidden && c==0)) -> Min(9,5) = 5 ->
        // mag 5 -> ctx (6>>1) = 3 -> Lf c==0 Min(3,8) = 3. (magLimit 3 would give
        // Min(9,3)=3 -> ctx 2, so ctx 3 proves the raise.)
        let mut lvl = [0u32; 64];
        lvl[1] = 9;
        assert_eq!(
            cb8(0, 0, true, false, 0, 0).select(&lvl),
            CoeffBaseSelection::Lf { ctx: 3 }
        );
    }

    #[test]
    fn coeff_base_parity_hidden_overrides_and_caps_maglimit() {
        // isHidden && c == 0 -> Ph, and the magLimit hidden-gate forces 3 (not 5):
        // neighbour flat 1 = 9 -> Min(9,3) = 3 -> mag 3 -> ctx 2 -> Ph Min(2,4) = 2.
        let mut lvl = [0u32; 64];
        lvl[1] = 9;
        assert_eq!(
            cb8(0, 0, true, true, 0, 0).select(&lvl),
            CoeffBaseSelection::Ph { ctx: 2 }
        );
    }

    #[test]
    fn coeff_base_chroma_2d_reads_three_neighbours_not_five() {
        // Chroma 2D -> num 3 (reads offsets 0,1,2 = flats 1,8,9), NOT offset 3
        // (flat 2). Set flat 9 (read) and flat 2 (not read) to 9: only flat 9
        // contributes -> Min(9,3)=3 -> mag 3 -> ctx 2 -> ctx2 Min(2,3)=2 -> Uv{2}.
        // (num 5 would also read flat 2 -> mag 6 -> ctx 3 -> Uv{3}.)
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
        // Short slice: most neighbour flats are past the slice -> contribute 0,
        // no panic. flat 1 ({0,1}) is in range -> Min(9,3)=3 -> mag 3 -> ctx 2 ->
        // ctx2 Min(2,4)=2 -> Hf{2}.
        let short = [0u32, 9];
        assert_eq!(
            cb8(0, 0, false, false, 0, 0).select(&short),
            CoeffBaseSelection::Hf { ctx: 2 }
        );
        // Pathological geometry must not panic.
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
        // above: +1 (sign 2) +1 (sign 2); left: -1 (sign 1) -1 (sign 1) -> 0 -> 0.
        let above = [2u8, 2];
        let left = [1u8, 1];
        assert_eq!(dc_sign_ctx(&above, &left, 0, 0, 2, 2), 0);
        // Net negative: above one -1, left zero -> ctx 1.
        let above_neg = [1u8, 0];
        let z2 = [0u8, 0];
        assert_eq!(dc_sign_ctx(&above_neg, &z2, 0, 0, 2, 2), 1);
        // Net positive: left two +1 -> ctx 2.
        let pos = [2u8, 2];
        assert_eq!(dc_sign_ctx(&z2, &pos, 0, 0, 2, 2), 2);
        // Sign value 0 (no DC sign recorded) contributes nothing.
        let zeros = [0u8, 0];
        assert_eq!(dc_sign_ctx(&zeros, &zeros, 0, 0, 2, 2), 0);
    }

    #[test]
    fn dc_sign_ctx_honours_the_position_offset_and_max_bounds() {
        // x4/y4 offset: only above[1], above[2] read for x4=1,w4=2 (above[0] skipped).
        let above = [1u8, 2, 2]; // index 0 = -1 (skipped), 1,2 = +1 each
        let z = [0u8; 4];
        assert_eq!(dc_sign_ctx(&above, &z, 1, 0, 2, 0), 2); // +1+1 = +2 -> ctx 2
        // Reads beyond the slice (the MiCols/MiRows max bound) are skipped.
        let short = [2u8]; // only index 0 in range
        assert_eq!(dc_sign_ctx(&short, &z, 0, 0, 4, 0), 2); // only above[0]=+1 -> ctx 2
    }

    #[test]
    fn dc_sign_ctx_is_total_for_pathological_geometry() {
        let a = [2u8; 4];
        let l = [1u8; 4];
        // Huge offsets/counts must not panic (saturating index + bounds guard).
        let _ = dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, usize::MAX, usize::MAX);
        assert_eq!(dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, 4, 4), 0); // all out of range -> 0
    }

    #[test]
    fn idtx_sign_ctx_maps_signc_to_base_context() {
        let zl = [0u32; 16];
        // (1,1) neighbours: above-left q[0], above q[1], left q[4].
        // signc 3 (>2) -> ctx 5.
        let p3 = [1i32, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p3, &zl, 1, 1, 4), 5);
        // signc -3 (<-2) -> ctx 6.
        let n3 = [-1i32, -1, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&n3, &zl, 1, 1, 4), 6);
        // signc +1 -> ctx 1; signc -1 -> ctx 2.
        let p1 = [0i32, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p1, &zl, 1, 1, 4), 1);
        let n1 = [0i32, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&n1, &zl, 1, 1, 4), 2);
        // signc 0 -> ctx 0.
        assert_eq!(idtx_sign_ctx(&[0i32; 16], &zl, 1, 1, 4), 0);
    }

    #[test]
    fn idtx_sign_ctx_level_threshold_raises_nonzero_context() {
        // ctx 1 (one +1 neighbour) + Level[1][1] (q[5]) > COEFF_BASE_RANGE -> +2.
        let p1 = [0i32, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let hi = [0u32, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // 4 > 3
        assert_eq!(idtx_sign_ctx(&p1, &hi, 1, 1, 4), 3);
        // Level == COEFF_BASE_RANGE (3) is NOT > 3 -> no raise.
        let eq = [0u32, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&p1, &eq, 1, 1, 4), 1);
        // ctx 0 is never raised, even with a high level.
        assert_eq!(idtx_sign_ctx(&[0i32; 16], &hi, 1, 1, 4), 0);
    }

    #[test]
    fn idtx_sign_ctx_skips_missing_edge_neighbours() {
        let zl = [0u32; 16];
        // (0,0): no left, above, or above-left -> signc 0 -> ctx 0.
        let q = [1i32; 16];
        assert_eq!(idtx_sign_ctx(&q, &zl, 0, 0, 4), 0);
        // (0,1): only left q[0]; no above / above-left (row 0).
        let only_left = [1i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&only_left, &zl, 0, 1, 4), 1);
        // (1,0): only above q[0]; no left / above-left (col 0).
        let only_above = [1i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(idtx_sign_ctx(&only_above, &zl, 1, 0, 4), 1);
    }

    #[test]
    fn idtx_sign_ctx_is_total_for_short_slices_and_pathological_geometry() {
        // Short slices: out-of-range neighbour/level reads contribute 0, no panic.
        let q = [1i32, 1];
        let l = [9u32];
        let _ = idtx_sign_ctx(&q, &l, 1, 1, 4);
        // Pathological geometry must not panic (saturating flat index).
        let _ = idtx_sign_ctx(&q, &l, usize::MAX, usize::MAX, usize::MAX);
    }
}
