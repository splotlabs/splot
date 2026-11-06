// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::symbol::{CdfUpdateMode, SymbolDecoder};

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{
    CHROMA_2D_PLANE_CONTEXT_OFFSET, HF_2D_CONTEXT_OFFSET, LF_2D_CONTEXT_CAP_AND_OFFSET,
    LF_SIG_COEF_CONTEXTS_2D_UV, MAG_REF_OFFSET_WITH_TX_CLASS, MAX_BASE_BR_RANGE, SIG_OFFSETS,
    coeff_base_eob_ctx,
};
use super::super::cdf::{CoeffCdfSelector, CoeffLevelRows, TileCdfSubset};
use super::super::coeff_state::{
    LEVEL_GRID_LEN, LEVEL_INDEX_MASK, LEVEL_STRIDE, LevelGrid, NONZERO_LIST_LEN, NonzeroList,
    TileCoeffStateError, TransformCoeffBlockState,
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
    nonzero: usize,
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

    /// How many leading entries of the block's nonzero list the pass wrote.
    #[must_use]
    pub(crate) const fn nonzero_count(self) -> usize {
        self.nonzero
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

/// One nonzero level of the base pass, packed for the sign and quant pass:
/// bits 0-9 the position, 10-19 the scan index, 20-27 the level, bit 28
/// `(tcqState >> 1) & 1` before it and bit 29 whether it is low frequency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonzeroEntry(u32);

impl NonzeroEntry {
    #[allow(clippy::inline_always, reason = "measured: per-coefficient record")]
    #[inline(always)]
    fn new(pos: usize, scan_index: usize, level: u32, tcq_q0: usize, is_lf: bool) -> Self {
        Self(
            (pos & 0x3ff) as u32
                | (((scan_index & 0x3ff) as u32) << 10)
                | ((level & 0xff) << 20)
                | (((tcq_q0 & 1) as u32) << 28)
                | (u32::from(is_lf) << 29),
        )
    }

    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) const fn pos(self) -> usize {
        (self.0 & 0x3ff) as usize
    }

    pub(crate) const fn scan_index(self) -> usize {
        ((self.0 >> 10) & 0x3ff) as usize
    }

    pub(crate) const fn level(self) -> u32 {
        (self.0 >> 20) & 0xff
    }

    pub(crate) const fn tcq_q0(self) -> bool {
        (self.0 >> 28) & 1 != 0
    }

    pub(crate) const fn is_lf(self) -> bool {
        (self.0 >> 29) & 1 != 0
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
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let (eob_read, owned_block) = start.into_parts()?;
    *block = owned_block;
    preflight_pass(eob_read, block, walk, config)?;
    if symbols.cdf_update_mode() == CdfUpdateMode::Enabled {
        dispatch_tools::<true>(cdfs, symbols, block, walk, config)
    } else {
        dispatch_tools::<false>(cdfs, symbols, block, walk, config)
    }
}

#[allow(clippy::inline_always, reason = "measured: one loop per tool set")]
#[inline(always)]
fn dispatch_tools<const UPD: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    match (config.use_tcq, config.parity_hiding) {
        (false, false) => {
            dispatch_level_pass::<false, false, UPD>(cdfs, symbols, block, walk, config)
        }
        (false, true) => {
            dispatch_level_pass::<false, true, UPD>(cdfs, symbols, block, walk, config)
        }
        (true, _) => dispatch_level_pass::<true, false, UPD>(cdfs, symbols, block, walk, config),
    }
}

/// A by-value decoder copy whose reads skip every per-symbol check; the
/// coefficient rows are decoder-owned, and `UPD` is the update mode.
struct HotReader<'a, const UPD: bool>(SymbolDecoder<'a>);

impl<const UPD: bool> HotReader<'_, UPD> {
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read<const L: usize>(&mut self, row: &mut [u16; L]) -> u32 {
        self.0.read_trusted::<L, UPD>(row)
    }
}

/// Splits the class and plane dispatch so each pass instantiation folds the
/// context derivation and the summary update to one straight-line body.
#[allow(
    clippy::inline_always,
    reason = "measured: one loop per class and plane"
)]
#[inline(always)]
fn dispatch_level_pass<const TCQ: bool, const PH: bool, const UPD: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    use CoeffTransformClass::{Horizontal, TwoD, Vertical};
    match (config.tx_class, config.plane == 0) {
        (TwoD, true) => level_pass::<0, true, TCQ, PH, UPD>(cdfs, symbols, block, walk, config),
        (TwoD, false) => level_pass::<0, false, TCQ, PH, UPD>(cdfs, symbols, block, walk, config),
        (Horizontal, true) => {
            level_pass::<1, true, TCQ, PH, UPD>(cdfs, symbols, block, walk, config)
        }
        (Horizontal, false) => {
            level_pass::<1, false, TCQ, PH, UPD>(cdfs, symbols, block, walk, config)
        }
        (Vertical, true) => level_pass::<2, true, TCQ, PH, UPD>(cdfs, symbols, block, walk, config),
        (Vertical, false) => {
            level_pass::<2, false, TCQ, PH, UPD>(cdfs, symbols, block, walk, config)
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
fn level_pass<
    const CLASS: usize,
    const LUMA: bool,
    const TCQ: bool,
    const PH: bool,
    const UPD: bool,
>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
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
    if walk.len() == 0 {
        return Ok(CoeffBaseFirstPassSummary::default());
    }
    let eob_entry = walk.entry(0);
    let eob_is_lf = coeff_is_low_frequency(eob_entry, config.plane, config.tx_class);
    let selector = base_eob_selector(eob_entry, eob_is_lf, config);
    let eob_level = u32::from(read_coeff_symbol(cdfs, symbols, selector)?) + 1;
    let mut owned = HotReader::<UPD>(symbols.snapshot());
    let result = level_pass_on_snapshot::<CLASS, LUMA, TCQ, PH, UPD>(
        cdfs,
        &mut owned,
        block,
        walk,
        config,
        (eob_is_lf, eob_level),
    );
    symbols.restore(owned.0);
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
fn level_pass_on_snapshot<
    const CLASS: usize,
    const LUMA: bool,
    const TCQ: bool,
    const PH: bool,
    const UPD: bool,
>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut HotReader<'_, UPD>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    (eob_is_lf, eob_level): (bool, u32),
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let Some((&eob_pos, rest)) = walk.scan().split_last() else {
        return Ok(CoeffBaseFirstPassSummary::default());
    };
    let Some((grid, list)) = block.level_grid_and_nonzero_mut() else {
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
        list,
        count: 0,
        width_log2: config.tx_width_log2,
        plane: config.plane,
        rows,
    };
    let eob_index = rest.len();
    let eob_grid = pass.grid_index(usize::from(eob_pos));
    let eob_level = if eob_is_lf {
        pass.add_range::<CLASS, LUMA, true, UPD>(symbols, eob_grid, eob_index == 0, eob_level)
    } else {
        pass.add_range::<CLASS, LUMA, false, UPD>(symbols, eob_grid, false, eob_level)
    };
    pass.finish::<TCQ, PH>(
        eob_index,
        usize::from(eob_pos),
        eob_grid,
        eob_level,
        eob_is_lf,
    );

    let Some((&dc_pos, ac)) = rest.split_first() else {
        return Ok(pass.summary());
    };
    let lf_end = lf_scan_end::<CLASS, LUMA>(config.tx_width, config.tx_height).max(1);
    let (lf, hf) = ac.split_at(ac.len().min(lf_end - 1));
    for (offset, &pos) in hf.iter().enumerate().rev() {
        pass.coefficient::<CLASS, LUMA, TCQ, PH, false, UPD>(symbols, lf_end + offset, pos);
    }
    for (offset, &pos) in lf.iter().enumerate().rev() {
        pass.coefficient::<CLASS, LUMA, TCQ, PH, true, UPD>(symbols, 1 + offset, pos);
    }
    let dc_grid = pass.grid_index(usize::from(dc_pos));
    let level = if PH && pass.first_pass.is_hidden() {
        let ctx = (pass.base_mag::<CLASS, LUMA>(dc_grid, 3, 3) + 1) >> 1;
        symbols.read(row_at(pass.rows.base_ph, ctx as usize))
    } else {
        let level = pass.read_lf_base::<CLASS, LUMA, true, UPD>(symbols, dc_grid, 0);
        pass.add_range::<CLASS, LUMA, true, UPD>(symbols, dc_grid, true, level)
    };
    pass.finish::<TCQ, PH>(0, usize::from(dc_pos), dc_grid, level, true);
    Ok(pass.summary())
}

/// `value.min(limit)` as a conditional move: the clamped neighbour levels
/// are data-dependent, so a branch would mispredict.
#[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
#[inline(always)]
fn clamp<T: Ord + Copy>(value: T, limit: T) -> T {
    core::hint::select_unpredictable(value < limit, value, limit)
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

/// `rows[ctx]` with the context clamped into the array: every context the
/// pass derives is in range, so the clamp only keeps the read panic-free.
#[allow(clippy::inline_always, reason = "measured: per-coefficient row")]
#[inline(always)]
fn row_at<T, const C: usize>(rows: &mut [T; C], ctx: usize) -> &mut T {
    &mut rows[ctx.min(C - 1)]
}

/// What the level loop reads and writes besides the symbol decoder: the
/// level grid, the pass summary, the nonzero list, and the CDF row arrays of the
/// block's transform-size context.
struct LevelPassState<'a> {
    grid: &'a mut LevelGrid,
    first_pass: CoeffBaseFirstPassSummary,
    /// `(tcqState >> 1) & 1` of the next coefficient, taken from the state
    /// before the previous update so it does not wait on the last symbol.
    tcq_q0: usize,
    list: &'a mut NonzeroList,
    count: usize,
    width_log2: u32,
    plane: usize,
    rows: CoeffLevelRows<'a>,
}

impl LevelPassState<'_> {
    fn summary(&self) -> CoeffBaseFirstPassSummary {
        CoeffBaseFirstPassSummary {
            nonzero: self.count,
            ..self.first_pass
        }
    }

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
        clamp(
            u32::from(self.grid[(grid & LEVEL_INDEX_MASK) + (offset & LEVEL_INDEX_MASK)]),
            limit,
        )
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
        let mag = clamp((mag + 1) >> 1, MAX_BASE_BR_RANGE) as usize;
        if !LUMA {
            clamp(mag, 3)
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
    fn add_range<const CLASS: usize, const LUMA: bool, const LF: bool, const UPD: bool>(
        &mut self,
        symbols: &mut HotReader<'_, UPD>,
        grid: usize,
        is_dc: bool,
        level: u32,
    ) -> u32 {
        let base_levels = if LF {
            LF_NUM_BASE_LEVELS
        } else {
            NUM_BASE_LEVELS
        };
        if level <= base_levels || (LF && !LUMA) {
            return level;
        }
        let lf_bump = LF && (!is_dc || CLASS != 0);
        let ctx = self.br_ctx::<CLASS, LUMA>(grid, lf_bump);
        level
            + if !LUMA {
                symbols.read(row_at(self.rows.br_uv, ctx))
            } else if LF {
                symbols.read(row_at(self.rows.br_lf, ctx))
            } else {
                symbols.read(row_at(self.rows.br, ctx))
            }
    }

    /// Reads a low-frequency base symbol; `DC` is whether the position is
    /// scan index 0.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read_lf_base<const CLASS: usize, const LUMA: bool, const DC: bool, const UPD: bool>(
        &mut self,
        symbols: &mut HotReader<'_, UPD>,
        grid: usize,
        pos: usize,
    ) -> u32 {
        let far = if CLASS == 0 { 5 } else { 3 };
        let ctx = ((self.base_mag::<CLASS, LUMA>(grid, 5, far) + 1) >> 1) as usize;
        if LUMA {
            let lf_ctx = if CLASS == 0 {
                let (cap, offset) = if DC {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[0]
                } else if grid < 2 || grid == LEVEL_STRIDE {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[1]
                } else {
                    LF_2D_CONTEXT_CAP_AND_OFFSET[2]
                };
                clamp(ctx, cap) + offset
            } else {
                let col_mask = (1usize << self.width_log2) - 1;
                let line = if CLASS == 1 {
                    pos & col_mask
                } else {
                    pos >> self.width_log2
                };
                if DC || line == 0 {
                    LF_SIG_COEF_CONTEXTS_2D + clamp(ctx, 6)
                } else {
                    LF_SIG_COEF_CONTEXTS_2D + 7 + clamp(ctx, 4)
                }
            };
            symbols.read(&mut row_at(self.rows.base_lf, lf_ctx)[self.tcq_q0 & 1])
        } else {
            let uv_ctx = self.uv_ctx::<CLASS>(ctx);
            symbols.read(row_at(self.rows.base_lf_uv, uv_ctx))
        }
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn uv_ctx<const CLASS: usize>(&self, ctx: usize) -> usize {
        clamp(ctx, 3)
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
        const UPD: bool,
    >(
        &mut self,
        symbols: &mut HotReader<'_, UPD>,
        scan_index: usize,
        pos: u16,
    ) {
        let pos = usize::from(pos);
        let grid = self.grid_index(pos);
        let level = if LF {
            self.read_lf_base::<CLASS, LUMA, false, UPD>(symbols, grid, pos)
        } else {
            let ctx = ((self.base_mag::<CLASS, LUMA>(grid, 3, 3) + 1) >> 1) as usize;
            if LUMA {
                let offset = if CLASS == 0 {
                    let diagonal =
                        (pos >> self.width_log2) + (pos & ((1usize << self.width_log2) - 1));
                    HF_2D_CONTEXT_OFFSET[usize::from(diagonal >= 6) + usize::from(diagonal >= 8)]
                } else {
                    15
                };
                symbols.read(&mut row_at(self.rows.base, clamp(ctx, 4) + offset)[self.tcq_q0 & 1])
            } else {
                let uv_ctx = self.uv_ctx::<CLASS>(ctx);
                symbols.read(row_at(self.rows.base_uv, uv_ctx))
            }
        };
        let level = self.add_range::<CLASS, LUMA, LF, UPD>(symbols, grid, false, level);
        self.finish::<TCQ, PH>(scan_index, pos, grid, level, LF);
    }

    /// Records a decoded level and advances the pass summary.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    fn finish<const TCQ: bool, const PH: bool>(
        &mut self,
        scan_index: usize,
        pos: usize,
        grid: usize,
        level: u32,
        is_lf: bool,
    ) {
        if level != 0 {
            self.list[self.count & (NONZERO_LIST_LEN - 1)] =
                NonzeroEntry::new(pos, scan_index, level, self.tcq_q0, is_lf).0;
            self.count += 1;
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
    if walk.len() > NONZERO_LIST_LEN {
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
