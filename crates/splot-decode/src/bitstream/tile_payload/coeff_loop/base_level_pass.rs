// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfArray::{
    CoeffBase, CoeffBaseLf, CoeffBaseLfUv, CoeffBasePh, CoeffBaseUv, CoeffBr, CoeffBrLf, CoeffBrUv,
};
use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{
    CHROMA_2D_PLANE_CONTEXT_OFFSET, HF_2D_CONTEXT_OFFSET, LF_2D_CONTEXT_CAP_AND_OFFSET,
    LF_SIG_COEF_CONTEXTS_2D_UV, MAG_REF_OFFSET_WITH_TX_CLASS, MAX_BASE_BR_RANGE, SIG_OFFSETS,
    coeff_base_eob_ctx,
};
use super::super::cdf::{CoeffCdfSelector, CoeffLevelRows, TileCdfSubset, row2};
use super::super::coeff_state::{
    LEVEL_GRID_LEN, LEVEL_INDEX_MASK, LEVEL_STRIDE, LevelGrid, TileCoeffStateError,
    TransformCoeffBlockState,
};
use super::NonZeroCoeffEob;
use super::base_symbol::{CoeffBaseSymbolReadError, read_coeff_symbol};
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{
    COEFF_BASE_RANGE, CoeffTransformClass, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS,
    coeff_is_low_frequency,
};
use super::quant_state::next_tcq_state;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};
use splot_core::coefficient::LF_SIG_COEF_CONTEXTS_2D;

const PHTHRESH: usize = 4;

/// `Sig_Ref_Diff_Offset` as unsigned `[row, col]` offsets per class.
const CONTEXT_OFFSETS: [[[usize; 2]; 5]; 3] = {
    let mut out = [[[0; 2]; 5]; 3];
    let mut class = 0;
    while class < 3 {
        let mut k = 0;
        while k < 5 {
            let [row, col] = SIG_OFFSETS[class][k];
            out[class][k] = [row as usize, col as usize];
            k += 1;
        }
        class += 1;
    }
    out
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseDerivedLevelPassConfig {
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width_log2: u32,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

/// The base pass's cross-coefficient state packed into one register-sized
/// value: bits 0-2 `tcqState`, bit 3 the `sumAbs1` parity, bits 4-7 the
/// nonzero count capped below `is_hidden`'s threshold decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffBaseFirstPassSummary {
    packed: u32,
}

const SUMMARY_TCQ_MASK: u32 = 0b111;
const SUMMARY_PARITY_SHIFT: u32 = 3;
const SUMMARY_COUNT_SHIFT: u32 = 4;
const SUMMARY_COUNT_MASK: u32 = 0b1111;

impl CoeffBaseFirstPassSummary {
    #[must_use]
    pub(crate) const fn sum_abs1(self) -> u32 {
        (self.packed >> SUMMARY_PARITY_SHIFT) & 1
    }

    #[must_use]
    pub(crate) const fn is_hidden(self) -> bool {
        ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK) >= PHTHRESH as u32
    }

    const fn tcq_state(self) -> usize {
        (self.packed & SUMMARY_TCQ_MASK) as usize
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    pub(crate) fn update_after_level<const TCQ: bool, const PH: bool>(
        &mut self,
        scan_index: usize,
        level: u32,
    ) {
        if TCQ {
            let tcq_state = next_tcq_state(self.tcq_state(), level);
            self.packed = (self.packed & !SUMMARY_TCQ_MASK) | tcq_state as u32;
        }
        if PH && level != 0 && scan_index > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            let parity = ((self.packed >> SUMMARY_PARITY_SHIFT) & 1) ^ (clipped & 1);
            let count = ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK)
                .min(SUMMARY_COUNT_MASK - 1);
            self.packed = (self.packed
                & !(SUMMARY_COUNT_MASK << SUMMARY_COUNT_SHIFT)
                & !(1 << SUMMARY_PARITY_SHIFT))
                | (parity << SUMMARY_PARITY_SHIFT)
                | ((count + 1) << SUMMARY_COUNT_SHIFT);
        }
    }
}

/// One bit per scan index, enough for the 32x32 coefficient maximum.
const SCAN_MASK_WORDS: usize = 16;

/// Per-scan-index facts the base pass hands to the sign and quant pass, so
/// that pass visits only the nonzero levels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffLevelMasks {
    nonzero: [u64; SCAN_MASK_WORDS],
    /// `(tcqState >> 1) & 1` before each scan index.
    tcq_q0: [u64; SCAN_MASK_WORDS],
}

impl CoeffLevelMasks {
    #[inline]
    fn record(&mut self, scan_index: usize, level: u32, tcq_q0: usize) {
        let word = (scan_index >> 6) & (SCAN_MASK_WORDS - 1);
        let shift = scan_index & 63;
        self.nonzero[word] |= u64::from(level != 0) << shift;
        self.tcq_q0[word] |= (tcq_q0 as u64) << shift;
    }

    #[inline]
    pub(crate) fn is_nonzero(&self, scan_index: usize) -> bool {
        (self.nonzero[(scan_index >> 6) & (SCAN_MASK_WORDS - 1)] >> (scan_index & 63)) & 1 != 0
    }

    #[inline]
    pub(crate) fn tcq_q0(&self, scan_index: usize) -> bool {
        (self.tcq_q0[(scan_index >> 6) & (SCAN_MASK_WORDS - 1)] >> (scan_index & 63)) & 1 != 0
    }

    /// The nonzero scan indices below `eob` from high to low (the walk
    /// order), plus scan index 0 when `with_dc`.
    pub(crate) fn nonzero_scan_indices(
        &self,
        eob: usize,
        with_dc: bool,
    ) -> impl Iterator<Item = usize> + '_ {
        let words = eob.div_ceil(64).min(SCAN_MASK_WORDS);
        (0..words).rev().flat_map(move |word| {
            let mut bits = self.nonzero[word] | u64::from(with_dc && word == 0);
            core::iter::from_fn(move || {
                let top = 63_u32.checked_sub(bits.leading_zeros())?;
                bits &= !(1 << top);
                Some(word * 64 + top as usize)
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseDerivedLevelPassError {
    #[error("coefficient base/level scan entries {entries} do not match eob {eob}")]
    ScanEntryCountMismatch { eob: usize, entries: usize },
    #[error("coefficient base/level scan has {entries} entries, above the 32x32 maximum")]
    ScanTooLong { entries: usize },
    #[error(
        "coefficient base/level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        block_width: usize,
        block_height: usize,
        config_width: usize,
        config_height: usize,
    },
    #[error("coefficient base/level tx_width_log2 {tx_width_log2} does not match width {tx_width}")]
    TxWidthLog2Mismatch { tx_width_log2: u32, tx_width: usize },
    #[error("coefficient base/level block has no level grid")]
    LevelGridUnavailable,
    #[error("coefficient base/level config cannot enable parity hiding and TCQ together")]
    InconsistentParityAndTcq,
    #[error("coefficient base/level base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    #[error("coefficient base/level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Kept out of line: inlined into the ordinary-branch function, the level
/// loop shares that function's registers and spills its context state.
///
#[inline(never)]
pub(crate) fn apply_nonzero_coeff_base_derived_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    block: &mut TransformCoeffBlockState,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let (eob_read, owned_block) = start.into_parts();
    *block = owned_block;
    preflight_pass(eob_read, block, walk, config)?;
    match (config.use_tcq, config.parity_hiding) {
        (false, false) => {
            dispatch_level_pass::<false, false>(cdfs, symbols, block, walk, config, masks)
        }
        (false, true) => {
            dispatch_level_pass::<false, true>(cdfs, symbols, block, walk, config, masks)
        }
        (true, _) => dispatch_level_pass::<true, false>(cdfs, symbols, block, walk, config, masks),
    }
}

/// Splits the class and plane dispatch so each pass instantiation folds the
/// context derivation and the summary update to one straight-line body.
#[inline(always)]
fn dispatch_level_pass<const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    use CoeffTransformClass::{Horizontal, TwoD, Vertical};
    match (config.tx_class, config.plane == 0) {
        (TwoD, true) => level_pass::<0, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks),
        (TwoD, false) => level_pass::<0, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks),
        (Horizontal, true) => {
            level_pass::<1, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Horizontal, false) => {
            level_pass::<1, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Vertical, true) => {
            level_pass::<2, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Vertical, false) => {
            level_pass::<2, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
    }
}

/// The level loop for one transform class and plane type, so every class
/// and plane branch of the context derivation folds away.
#[allow(
    clippy::inline_always,
    reason = "measured: one specialised loop per class and plane type"
)]
#[inline(always)]
fn level_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let mut owned = symbols.snapshot();
    let result = level_pass_on_snapshot::<CLASS, LUMA, TCQ, PH>(
        cdfs, &mut owned, block, walk, config, masks,
    );
    symbols.restore(owned);
    result
}

/// The loop body over an owned decoder, so the arithmetic state stays in
/// registers instead of behind a pointer. The scan runs in three phases:
/// the high-frequency indices, the low-frequency indices above 0 (a scan
/// prefix, see [`lf_scan_end`]), and DC, so each phase reads one row family
/// with its context arithmetic folded to constants.
#[allow(
    clippy::inline_always,
    reason = "measured: merges the owned decoder into the specialised loop"
)]
#[inline(always)]
fn level_pass_on_snapshot<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let config = CoeffBaseDerivedLevelPassConfig {
        tx_class: [
            CoeffTransformClass::TwoD,
            CoeffTransformClass::Horizontal,
            CoeffTransformClass::Vertical,
        ][CLASS],
        plane: if LUMA { 0 } else { config.plane.max(1) },
        ..config
    };
    *masks = CoeffLevelMasks::default();
    let scan = walk.scan();
    let Some((&eob_pos, rest)) = scan.split_last() else {
        return Ok(CoeffBaseFirstPassSummary::default());
    };
    let eob_entry = walk.entry(0);
    let eob_is_lf = coeff_is_low_frequency(eob_entry, config.plane, config.tx_class);
    let selector = base_eob_selector(eob_entry, eob_is_lf, config);
    let eob_level = u32::from(read_coeff_symbol(cdfs, symbols, selector)?) + 1;

    let Some(grid) = block.level_grid_mut() else {
        return Err(CoeffBaseDerivedLevelPassError::LevelGridUnavailable);
    };
    let rows = cdfs
        .coeff_rows_mut()
        .level_rows(config.tx_size_ctx)
        .map_err(read_error)?;
    let mut pass = LevelPassState {
        grid,
        first_pass: CoeffBaseFirstPassSummary::default(),
        tcq_q0: 0,
        masks,
        width_log2: config.tx_width_log2,
        plane: config.plane,
        rows,
    };
    let eob_index = rest.len();
    let eob_grid = pass.grid_index(usize::from(eob_pos));
    let eob_level = if eob_is_lf {
        pass.add_range::<CLASS, LUMA, true>(symbols, eob_grid, eob_index == 0, eob_level)?
    } else {
        pass.add_range::<CLASS, LUMA, false>(symbols, eob_grid, false, eob_level)?
    };
    pass.finish::<TCQ, PH>(eob_index, eob_grid, eob_level);

    let Some((&dc_pos, ac)) = rest.split_first() else {
        return Ok(pass.first_pass);
    };
    let lf_end = lf_scan_end::<CLASS, LUMA>(config.tx_width, config.tx_height).max(1);
    let (lf, hf) = ac.split_at(ac.len().min(lf_end - 1));
    for (offset, &pos) in hf.iter().enumerate().rev() {
        pass.coefficient::<CLASS, LUMA, TCQ, PH, false>(symbols, lf_end + offset, pos)?;
    }
    for (offset, &pos) in lf.iter().enumerate().rev() {
        pass.coefficient::<CLASS, LUMA, TCQ, PH, true>(symbols, 1 + offset, pos)?;
    }
    let dc_grid = pass.grid_index(usize::from(dc_pos));
    let level = if PH && pass.first_pass.is_hidden() {
        let ctx = (pass.base_mag::<CLASS, LUMA>(dc_grid, 3, 3) + 1) >> 1;
        let row = row2(pass.rows.base_ph, CoeffBasePh, ctx.min(4) as usize).map_err(read_error)?;
        read_row(symbols, row)?
    } else {
        let level = pass.read_lf_base::<CLASS, LUMA, true>(symbols, dc_grid, 0)?;
        pass.add_range::<CLASS, LUMA, true>(symbols, dc_grid, true, level)?
    };
    pass.finish::<TCQ, PH>(0, dc_grid, level);
    Ok(pass.first_pass)
}

/// One past the last low-frequency scan index of `get_lf_limits`
/// (§ 5.20.7.27): the 2D diagonal scan visits the ten positions with
/// `row + col < 4` first, the horizontal-class scan whole columns first and
/// the vertical-class scan whole rows first.
const fn lf_scan_end<const CLASS: usize, const LUMA: bool>(width: usize, height: usize) -> usize {
    let lines = if LUMA { 2 } else { 1 };
    match CLASS {
        0 if LUMA => 10,
        0 => 1,
        1 => lines * height,
        _ => lines * width,
    }
}

#[allow(
    clippy::inline_always,
    reason = "measured: per-coefficient symbol read"
)]
#[inline(always)]
fn read_row<const L: usize>(
    symbols: &mut SymbolDecoder<'_>,
    row: &mut [u16; L],
) -> Result<u32, CoeffBaseDerivedLevelPassError> {
    Ok(u32::from(
        symbols
            .read_symbol_u16_array(row)
            .map_err(read_error)?
            .get(),
    ))
}

/// What the level loop reads and writes besides the symbol decoder: the
/// level grid, the pass summary, the masks, and the CDF row arrays of the
/// block's transform-size context.
struct LevelPassState<'a> {
    grid: &'a mut LevelGrid,
    first_pass: CoeffBaseFirstPassSummary,
    /// `(tcqState >> 1) & 1` of the next coefficient, taken from the state
    /// before the previous update so it does not wait on the last symbol.
    tcq_q0: usize,
    masks: &'a mut CoeffLevelMasks,
    width_log2: u32,
    plane: usize,
    rows: CoeffLevelRows<'a>,
}

impl LevelPassState<'_> {
    #[allow(clippy::inline_always, reason = "measured: per-coefficient index")]
    #[inline(always)]
    fn grid_index(&self, pos: usize) -> usize {
        let col_mask = (1usize << self.width_log2) - 1;
        (pos >> self.width_log2) * LEVEL_STRIDE + (pos & col_mask)
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn at(&self, grid: usize, [dr, dc]: [usize; 2], limit: u32) -> u32 {
        let offset = dr * LEVEL_STRIDE + dc;
        u32::from(self.grid[(grid & LEVEL_INDEX_MASK) + (offset & LEVEL_INDEX_MASK)]).min(limit)
    }

    /// The clamped `coeff_base` neighbour sum (§ 8.3.2) with the limits of
    /// the first two and of the remaining `Sig_Ref_Diff_Offset` entries.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn base_mag<const CLASS: usize, const LUMA: bool>(
        &self,
        grid: usize,
        near: u32,
        far: u32,
    ) -> u32 {
        let offsets = CONTEXT_OFFSETS[CLASS];
        let mag = self.at(grid, offsets[0], near) + self.at(grid, offsets[1], near);
        if LUMA {
            mag + self.at(grid, offsets[2], far)
                + self.at(grid, offsets[3], far)
                + self.at(grid, offsets[4], far)
        } else if CLASS == 0 {
            mag + self.at(grid, offsets[2], far)
        } else {
            mag
        }
    }

    /// The `coeff_br` context (§ 8.3.2); `LF_BUMP` is whether the context
    /// takes the low-frequency offset of 7.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn br_ctx<const CLASS: usize, const LUMA: bool>(&self, grid: usize, lf_bump: bool) -> usize {
        let offsets = MAG_REF_OFFSET_WITH_TX_CLASS[CLASS];
        let limit = MAX_BASE_BR_RANGE - 1;
        let mut mag = self.at(grid, offsets[0], limit) + self.at(grid, offsets[1], limit);
        if LUMA || CLASS == 0 {
            mag += self.at(grid, offsets[2], limit);
        }
        let mag = ((mag + 1) >> 1).min(MAX_BASE_BR_RANGE) as usize;
        if !LUMA {
            mag.min(3)
        } else if lf_bump {
            mag + 7
        } else {
            mag
        }
    }

    /// Adds `coeff_br` when the base symbol saturates.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn add_range<const CLASS: usize, const LUMA: bool, const LF: bool>(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        grid: usize,
        is_dc: bool,
        level: u32,
    ) -> Result<u32, CoeffBaseDerivedLevelPassError> {
        let base_levels = if LF {
            LF_NUM_BASE_LEVELS
        } else {
            NUM_BASE_LEVELS
        };
        if level <= base_levels || (LF && !LUMA) {
            return Ok(level);
        }
        let lf_bump = LF && (!is_dc || CLASS != 0);
        let ctx = self.br_ctx::<CLASS, LUMA>(grid, lf_bump);
        let row = if !LUMA {
            row2(self.rows.br_uv, CoeffBrUv, ctx)
        } else if LF {
            row2(self.rows.br_lf, CoeffBrLf, ctx)
        } else {
            row2(self.rows.br, CoeffBr, ctx)
        }
        .map_err(read_error)?;
        Ok(level + read_row(symbols, row)?)
    }

    /// Reads a low-frequency base symbol; `DC` is whether the position is
    /// scan index 0.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read_lf_base<const CLASS: usize, const LUMA: bool, const DC: bool>(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        grid: usize,
        pos: usize,
    ) -> Result<u32, CoeffBaseDerivedLevelPassError> {
        let far = if CLASS == 0 { 5 } else { 3 };
        let ctx = ((self.base_mag::<CLASS, LUMA>(grid, 5, far) + 1) >> 1) as usize;
        let row = if LUMA {
            let lf_ctx = if CLASS == 0 {
                let (cap, offset) = if DC {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[0]
                } else if grid < 2 || grid == LEVEL_STRIDE {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[1]
                } else {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[2]
                };
                ctx.min(cap) + offset
            } else {
                let col_mask = (1usize << self.width_log2) - 1;
                let line = if CLASS == 1 {
                    pos & col_mask
                } else {
                    pos >> self.width_log2
                };
                if DC || line == 0 {
                    LF_SIG_COEF_CONTEXTS_2D + ctx.min(6)
                } else {
                    LF_SIG_COEF_CONTEXTS_2D + 7 + ctx.min(4)
                }
            };
            &mut row2(self.rows.base_lf, CoeffBaseLf, lf_ctx).map_err(read_error)?[self.tcq_q0 & 1]
        } else {
            let uv_ctx = self.uv_ctx::<CLASS>(ctx);
            row2(self.rows.base_lf_uv, CoeffBaseLfUv, uv_ctx).map_err(read_error)?
        };
        read_row(symbols, row)
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn uv_ctx<const CLASS: usize>(&self, ctx: usize) -> usize {
        ctx.min(3)
            + if CLASS != 0 {
                LF_SIG_COEF_CONTEXTS_2D_UV
            } else {
                CHROMA_2D_PLANE_CONTEXT_OFFSET[self.plane.min(2)]
            }
    }

    /// Reads one coefficient above scan index 0 and below the EOB one.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient body")]
    #[inline(always)]
    fn coefficient<
        const CLASS: usize,
        const LUMA: bool,
        const TCQ: bool,
        const PH: bool,
        const LF: bool,
    >(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        scan_index: usize,
        pos: u16,
    ) -> Result<(), CoeffBaseDerivedLevelPassError> {
        let pos = usize::from(pos);
        let grid = self.grid_index(pos);
        let level = if LF {
            self.read_lf_base::<CLASS, LUMA, false>(symbols, grid, pos)?
        } else {
            let ctx = ((self.base_mag::<CLASS, LUMA>(grid, 3, 3) + 1) >> 1) as usize;
            let row = if LUMA {
                let offset = if CLASS == 0 {
                    let diagonal =
                        (pos >> self.width_log2) + (pos & ((1usize << self.width_log2) - 1));
                    HF_2D_CONTEXT_OFFSET[usize::from(diagonal >= 6) + usize::from(diagonal >= 8)]
                } else {
                    15
                };
                &mut row2(self.rows.base, CoeffBase, ctx.min(4) + offset).map_err(read_error)?
                    [self.tcq_q0 & 1]
            } else {
                let uv_ctx = self.uv_ctx::<CLASS>(ctx);
                row2(self.rows.base_uv, CoeffBaseUv, uv_ctx).map_err(read_error)?
            };
            read_row(symbols, row)?
        };
        let level = self.add_range::<CLASS, LUMA, LF>(symbols, grid, false, level)?;
        self.finish::<TCQ, PH>(scan_index, grid, level);
        Ok(())
    }

    /// Records a decoded level and advances the pass summary.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    fn finish<const TCQ: bool, const PH: bool>(
        &mut self,
        scan_index: usize,
        grid: usize,
        level: u32,
    ) {
        if level != 0 {
            self.masks.record(scan_index, level, self.tcq_q0);
            self.grid[grid & (LEVEL_GRID_LEN - 1)] = level as u8;
        }
        if TCQ {
            self.tcq_q0 = (self.first_pass.tcq_state() >> 2) & 1;
        }
        self.first_pass
            .update_after_level::<TCQ, PH>(scan_index, level);
    }
}

fn preflight_pass(
    eob_read: NonZeroCoeffEob,
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<(), CoeffBaseDerivedLevelPassError> {
    if config.parity_hiding && config.use_tcq {
        return Err(CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq);
    }
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffBaseDerivedLevelPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    if 1usize.checked_shl(config.tx_width_log2) != Some(config.tx_width) {
        return Err(CoeffBaseDerivedLevelPassError::TxWidthLog2Mismatch {
            tx_width_log2: config.tx_width_log2,
            tx_width: config.tx_width,
        });
    }

    let eob = eob_read.eob();
    if walk.len() > SCAN_MASK_WORDS * 64 {
        return Err(CoeffBaseDerivedLevelPassError::ScanTooLong {
            entries: walk.len(),
        });
    }
    if eob != walk.len() {
        return Err(CoeffBaseDerivedLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: walk.len(),
        });
    }
    Ok(())
}

fn base_eob_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let ctx = coeff_base_eob_ctx(entry.scan_index(), config.tx_width_log2, config.tx_height);
    if config.plane > 0 {
        if is_lf {
            CoeffCdfSelector::BaseLfEobUv { ctx }
        } else {
            CoeffCdfSelector::BaseEobUv { ctx }
        }
    } else if is_lf {
        CoeffCdfSelector::BaseLfEob {
            tx_size: config.tx_size_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::BaseEob {
            tx_size: config.tx_size_ctx,
            ctx,
        }
    }
}

fn read_error(error: impl Into<BlockSymbolTraceReadError>) -> CoeffBaseDerivedLevelPassError {
    CoeffBaseSymbolReadError::from(error.into()).into()
}

pub(crate) const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}

#[cfg(test)]
#[path = "base_level_pass_tests.rs"]
mod tests;
