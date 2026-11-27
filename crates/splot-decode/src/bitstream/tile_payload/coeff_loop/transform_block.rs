// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The coefficients of one ordinary (non-FSC) transform block (§ 5.20.7.27)
//! in one call: the reverse-scan base and range pass over a padded level
//! grid, then the sign and remainder pass writing `Quant` directly.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::coefficient::LF_SIG_COEF_CONTEXTS_2D;
use splot_core::symbol::SymbolDecoder;
use splot_recon::{TransformClass, coefficient_scan_position};

use super::super::cdf::block_read::{BlockSymbolTraceReadError, read_row, row_at};
use super::super::cdf::coeff_context::{
    CHROMA_2D_PLANE_CONTEXT_OFFSET, HF_2D_CONTEXT_OFFSET, LF_2D_CONTEXT_CAP_AND_OFFSET,
    LF_SIG_COEF_CONTEXTS_2D_UV, MAX_BASE_BR_RANGE, SIG_OFFSETS, coeff_base_eob_ctx, dc_sign_ctx,
};
use super::super::cdf::{CoeffLevelRows, TileCdfSubset};
use super::super::coeff_state::{
    CoeffBlockLines, CoeffLevelWords, CoeffNonzeroList, LEVEL_STRIDE, LEVEL_WORDS_MASK,
    TileCoeffContextState, TileCoeffStateError,
};
use super::base_level_pass::{CoeffBaseDerivedLevelPassError, tx_class_index};
use super::base_symbol::CoeffBaseSymbolReadError;
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{
    COEFF_BASE_RANGE, CoeffMaxLevelConfig, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS,
    derive_coeff_max_level, visible_max_level,
};
use super::ordinary_pass::{CoeffOrdinaryBranchError, CoeffOrdinaryPassError};
use super::quant_pass::CoeffQuantPassError;
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, NonZeroCoeffQuantState, next_tcq_state,
};
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::CoeffScanEntry;
use super::sign_symbol::{
    CoeffSignReadSymbol, CoeffSignSourceDeriveConfig, derive_nonzero_coeff_sign_input,
    read_preflighted_nonzero_coeff_sign,
};
use super::{CoeffLoopContextError, TxGeometry};

macro_rules! read_level {
    ($symbols:expr, $row:expr, $key:expr, $context:expr, $values:expr) => {{
        #[cfg(feature = "block-evidence")]
        let level = $symbols.read_trusted_with_syntax(
            $row,
            splot_core::symbol::EntropySyntax::new($key, $context, $values),
        );
        #[cfg(not(feature = "block-evidence"))]
        let level = $symbols.read_trusted($row);
        level
    }};
}

const PHTHRESH: u32 = 4;

/// A neighbour `[row, col]` as an offset into the level grid.
const fn grid_offset([row, col]: [usize; 2]) -> usize {
    row * LEVEL_STRIDE + col
}

/// `Sig_Ref_Diff_Offset` per class as level-grid offsets.
const SIG_GRID: [[usize; 5]; 3] = {
    let mut out = [[0; 5]; 3];
    let mut class = 0;
    while class < 3 {
        let mut k = 0;
        while k < 5 {
            let [row, col] = SIG_OFFSETS[class][k];
            out[class][k] = grid_offset([row as usize, col as usize]);
            k += 1;
        }
        class += 1;
    }
    out
};

/// The grid word of a level: `min(level, 3)`, `min(level, 5)` and the level
/// in bytes 0 to 2, so one sum over neighbour words sums both caps. Levels
/// stay below 9, so five-word sums never carry between bytes. Entry
/// `16 + level` adds the [`Q0_BIT`] of a nonzero level.
pub(super) static LEVEL_WORD: [u32; 32] = {
    let mut out = [0; 32];
    let mut level = 0;
    while level < 16 {
        let lo = if level < 3 { level } else { 3 };
        let hi = if level < 5 { level } else { 5 };
        out[level as usize] = lo | (hi << 8) | (level << 16);
        out[16 + level as usize] = out[level as usize] | ((level != 0) as u32) << Q0_BIT;
        level += 1;
    }
    out
};

/// The grid-word bit holding a TCQ level's `(tcqState >> 1) & 1`, above the
/// bytes the neighbour sums read.
const Q0_BIT: u32 = 24;

/// Where a [`GRID_SCANS`] entry keeps its luma 2D `coeff_base` context
/// class (§ 8.3.2), above the grid index: for a low-frequency position
/// (`row + col < 4`) its `LF_2D_CONTEXT_CAP_AND_OFFSET` bucket, otherwise its
/// `HF_2D_CONTEXT_OFFSET` bucket.
const CTX_CLASS_SHIFT: u32 = 11;

const fn ctx_class(row: usize, col: usize) -> usize {
    let diagonal = row + col;
    if diagonal == 0 {
        0
    } else if diagonal < 2 {
        1
    } else if diagonal < 4 {
        2
    } else {
        (diagonal >= 6) as usize + (diagonal >= 8) as usize
    }
}

/// `CTX_2D[lf][class][mag]`: the luma 2D `coeff_base` context of a
/// neighbour sum `mag` in a context class (§ 8.3.2), `lf` for the
/// low-frequency rows. A high-frequency context is stored doubled, the index
/// of its first TCQ row in the flattened rows.
static CTX_2D: [[[u8; 32]; 4]; 2] = {
    let mut out = [[[0; 32]; 4]; 2];
    let mut class = 0;
    while class < 3 {
        let mut mag = 0;
        while mag < 32 {
            let half = (mag + 1) >> 1;
            out[0][class][mag] = (2 * (min(half, 4) + HF_2D_CONTEXT_OFFSET[class])) as u8;
            let (cap, offset) = LF_2D_CONTEXT_CAP_AND_OFFSET[class];
            out[1][class][mag] = (min(half, cap) + offset) as u8;
            mag += 1;
        }
        class += 1;
    }
    out
};

const fn min(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

const SCAN_DIMS: usize = 4;
const GRID_SCAN_LEN: usize = 3600;

/// Where the `4 << wi` by `4 << hi` shape starts in [`GRID_SCANS`]. Earlier widths span 60 rows each.
pub(super) const fn scan_shape_offset(wi: usize, hi: usize) -> usize {
    let w = 4usize << wi;
    let h = 4usize << hi;
    (w - 4) * 60 + w * (h - 4)
}

/// The § 5.20.7.30 scan of every block shape per transform class, each
/// entry holds the grid index in bits 0..11, context class in 11..13,
/// raster position divided by four in 13..21 and scan index in 22..32.
/// Both the raster stride and `LEVEL_STRIDE` are multiples of four, so
/// the two low raster bits already occur in the grid index.
static GRID_SCANS: [[u32; GRID_SCAN_LEN]; 3] = {
    let classes = [
        TransformClass::TwoD,
        TransformClass::Horizontal,
        TransformClass::Vertical,
    ];
    let mut out = [[0; GRID_SCAN_LEN]; 3];
    let mut class = 0;
    while class < 3 {
        let mut wi = 0;
        while wi < SCAN_DIMS {
            let mut hi = 0;
            while hi < SCAN_DIMS {
                let base = scan_shape_offset(wi, hi);
                let width_log2 = wi + 2;
                let mut c = 0;
                while c < (4 << wi) * (4 << hi) {
                    let pos = coefficient_scan_position(wi, hi, classes[class], c) as usize;
                    let (row, col) = (pos >> width_log2, pos & ((1 << width_log2) - 1));
                    let grid = row * LEVEL_STRIDE + col;
                    let context = ctx_class(row, col) << CTX_CLASS_SHIFT;
                    out[class][base + c] = (grid | context | ((pos >> 2) << 13) | (c << 22)) as u32;
                    c += 1;
                }
                hi += 1;
            }
            wi += 1;
        }
        class += 1;
    }
    out
};

/// One past the largest raster position among the first `c + 1` entries of
/// each [`GRID_SCANS`] scan, at the same index.
static GRID_EXTENTS: [[u16; GRID_SCAN_LEN]; 3] = {
    let mut out = [[0; GRID_SCAN_LEN]; 3];
    let mut class = 0;
    while class < 3 {
        let mut wi = 0;
        while wi < SCAN_DIMS {
            let mut hi = 0;
            while hi < SCAN_DIMS {
                let base = scan_shape_offset(wi, hi);
                let (mut c, mut extent) = (0, 0);
                while c < (4 << wi) * (4 << hi) {
                    let pos = entry_pos(GRID_SCANS[class][base + c]) as u16;
                    if pos >= extent {
                        extent = pos + 1;
                    }
                    out[class][base + c] = extent;
                    c += 1;
                }
                hi += 1;
            }
            wi += 1;
        }
        class += 1;
    }
    out
};

/// One past the largest raster position the scan can reach at this EOB.
#[expect(
    clippy::inline_always,
    reason = "measured: out of line it costs 30 instructions per block"
)]
#[inline(always)]
pub(crate) fn scan_extent(class: TransformClass, tx: &TxGeometry, eob: usize) -> usize {
    let len = usize::from(tx.width) * usize::from(tx.height);
    if eob == 0 || eob > len {
        return len;
    }
    GRID_EXTENTS[tx_class_index(class)]
        .get(usize::from(tx.scan_offset) + eob - 1)
        .map_or(len, |&extent| usize::from(extent))
}

/// The transform's coefficient scan through its EOB.
pub(super) fn grid_scan(
    class: TransformClass,
    tx: &TxGeometry,
    eob: usize,
) -> Result<&'static [u32], CoeffOrdinaryPassError> {
    let len = usize::from(tx.width) * usize::from(tx.height);
    if eob == 0 {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob }.into());
    }
    if eob > len {
        return Err(CoeffLoopContextError::ScanWalkEobOutOfRange { eob, scan_len: len }.into());
    }
    let start = usize::from(tx.scan_offset);
    let scans = &GRID_SCANS[tx_class_index(class)];
    Ok(scans.get(start..start + eob).unwrap_or_default())
}

fn half_capped(mag: u32, cap: usize) -> usize {
    (mag as usize).div_ceil(2).min(cap)
}

/// One past the last low-frequency scan index of `get_lf_limits`
/// (§ 5.20.7.27): the 2D diagonal scan visits the ten positions with
/// `row + col < 4` first, the horizontal-class scan whole columns first and
/// the vertical-class scan whole rows first.
pub(crate) const fn lf_scan_end<const CLASS: usize, const LUMA: bool>(
    width: usize,
    height: usize,
) -> usize {
    let lines = if LUMA { 2 } else { 1 };
    match CLASS {
        0 if LUMA => 10,
        0 => 1,
        1 => lines * height,
        _ => lines * width,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CoeffTransformBlockInput<'l> {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) plane: usize,
    pub(crate) tx_class: TransformClass,
    pub(crate) scan_order: bool,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
    pub(crate) lines: &'l CoeffBlockLines,
    pub(crate) lossless: bool,
}

/// Writes decoded coefficients into the zeroed output slice in raster or
/// scan order and returns one past its last written nonzero coefficient.
pub(crate) fn decode_ordinary_transform_block(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mut input: CoeffTransformBlockInput<'_>,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryBranchError> {
    use TransformClass::{Horizontal, TwoD, Vertical};

    input.parity_hiding &= !input.lossless;
    input.use_tcq &= !input.lossless;
    preflight(&input, quant.len())?;
    let result = match (
        input.tx_class,
        input.plane == 0,
        input.use_tcq,
        input.parity_hiding,
    ) {
        (TwoD, true, true, _) => run::<0, true, true, false>(state, cdfs, symbols, &input, quant),
        (TwoD, true, false, true) => {
            run::<0, true, false, true>(state, cdfs, symbols, &input, quant)
        }
        (TwoD, true, false, false) => {
            run::<0, true, false, false>(state, cdfs, symbols, &input, quant)
        }
        (Horizontal, true, _, true) => {
            run::<1, true, false, true>(state, cdfs, symbols, &input, quant)
        }
        (Horizontal, true, _, false) => {
            run::<1, true, false, false>(state, cdfs, symbols, &input, quant)
        }
        (Vertical, true, _, true) => {
            run::<2, true, false, true>(state, cdfs, symbols, &input, quant)
        }
        (Vertical, true, _, false) => {
            run::<2, true, false, false>(state, cdfs, symbols, &input, quant)
        }
        (TwoD, false, ..) => run::<0, false, false, false>(state, cdfs, symbols, &input, quant),
        (Horizontal, false, ..) => {
            run::<1, false, false, false>(state, cdfs, symbols, &input, quant)
        }
        (Vertical, false, ..) => run::<2, false, false, false>(state, cdfs, symbols, &input, quant),
    };
    Ok(result?)
}

fn preflight(
    input: &CoeffTransformBlockInput<'_>,
    quant_len: usize,
) -> Result<(), CoeffOrdinaryPassError> {
    if input.parity_hiding && input.use_tcq {
        return Err(CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq.into());
    }
    if (input.plane > 0 && (input.parity_hiding || input.use_tcq))
        || (input.use_tcq && input.tx_class != TransformClass::TwoD)
    {
        return Err(CoeffBaseDerivedLevelPassError::UnsupportedToolClass.into());
    }
    if input.lines.plane != input.plane {
        return Err(TileCoeffStateError::InvalidPlane {
            plane: input.lines.plane,
        }
        .into());
    }
    let coeff_count = input.start.coeff_count();
    if quant_len > coeff_count {
        return Err(CoeffBaseDerivedLevelPassError::QuantLenExceedsBlock {
            quant_len,
            coeff_count,
        }
        .into());
    }
    Ok(())
}

struct Block<'a> {
    /// The scan up to the EOB, see [`GRID_SCANS`].
    scan: &'static [u32],
    width_log2: u32,
    dc: CoeffSignSourceDeriveConfig<'a>,
    lossless: bool,
    scan_order: bool,
}

struct BlockSummary {
    state: NonZeroCoeffQuantState,
    quant_end: usize,
}

/// One specialised block body per class, plane type and tool set, kept out
/// of line so its loops own the registers.
#[inline(never)]
fn run<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: &CoeffTransformBlockInput<'_>,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryPassError> {
    let tx = input.start.tx();
    let width_log2 = u32::from(tx.width_log2);
    let tx_class = [
        TransformClass::TwoD,
        TransformClass::Horizontal,
        TransformClass::Vertical,
    ][CLASS];
    let eob = input.start.eob_read();
    let scan = grid_scan(tx_class, tx, eob)?;
    let lines = input.lines;
    let (above_dc, left_dc, levels, nonzero) = state.dc_lines_and_levels(input.plane)?;
    let block = Block {
        scan,
        width_log2,
        scan_order: input.scan_order,
        dc: CoeffSignSourceDeriveConfig {
            plane: input.plane,
            plane_type: usize::from(!LUMA),
            tx_class,
            is_hidden: false,
            sum_abs1: 0,
            above_dc,
            left_dc,
            x4: lines.x4,
            y4: lines.y4,
            w4: lines.w4,
            h4: lines.h4,
        },
        lossless: input.lossless,
    };
    let Some(&eob_entry) = scan.last() else {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob: 0 }.into());
    };
    let lf_end = lf_scan_end::<CLASS, LUMA>(usize::from(tx.width), usize::from(tx.height)).max(1);
    let eob_index = scan.len() - 1;
    let mut rows = cdfs
        .coeff_rows_mut()
        .level_rows(usize::from(tx.size_ctx))
        .map_err(|error| CoeffBaseSymbolReadError::from(BlockSymbolTraceReadError::from(error)))
        .map_err(CoeffBaseDerivedLevelPassError::from)?;
    let eob_ctx = coeff_base_eob_ctx(eob_index, width_log2, usize::from(tx.height));
    let eob_level = read_eob_base::<LUMA>(symbols, &mut rows, eob_ctx, eob_index < lf_end) + 1;
    let mut pass = LevelPass {
        symbols: symbols.clone(),
        levels: &mut *levels,
        width_log2,
        plane: input.plane,
        tcq: 0,
        tcq_q0: 0,
        ph: 0,
        nonzero: &mut *nonzero,
        nonzero_len: 0,
    };
    let dc_level =
        pass.levels_pass::<CLASS, LUMA, TCQ, PH>(&mut rows, scan, eob_entry, eob_level, lf_end);
    let dc_q0 = TCQ && pass.tcq_q0 & 1 != 0;
    let (mut owned, ph, nonzero_len) = (pass.symbols, pass.ph, pass.nonzero_len);
    let nonzero = nonzero.get(..nonzero_len).unwrap_or_default();
    let facts = SignFacts {
        dc_level,
        dc_q0,
        lf_end,
        is_hidden: PH && (ph >> 1) >= PHTHRESH,
        sum_abs1: if PH { ph & 1 } else { 0 },
    };
    let result =
        sign_pass::<CLASS, LUMA, TCQ>(cdfs, &mut owned, levels, quant, &block, facts, nonzero);
    *symbols = owned;
    if result.is_err() {
        levels.fill(0);
    }
    let summary = result?;
    state.commit_block(
        lines,
        summary.state.cul_level(),
        summary.state.dc_category(),
    );
    Ok(summary.quant_end)
}

/// Reads the `coeff_base_eob` symbol of context `ctx` (§ 8.3.2).
#[expect(clippy::inline_always, reason = "measured: per-block symbol read")]
#[inline(always)]
fn read_eob_base<const LUMA: bool>(
    symbols: &mut SymbolDecoder<'_>,
    rows: &mut CoeffLevelRows<'_>,
    ctx: usize,
    lf: bool,
) -> u32 {
    #[cfg(feature = "block-evidence")]
    if symbols.observes_entropy() {
        use splot_core::symbol::EntropySyntax;

        symbols.annotate_entropy(match (LUMA, lf) {
            (true, true) => EntropySyntax::new(
                "coeff_base_lf_eob",
                "tx_size_ctx,ctx",
                [rows.tx_size as u16, ctx as u16, 0],
            ),
            (true, false) => EntropySyntax::new(
                "coeff_base_eob",
                "tx_size_ctx,ctx",
                [rows.tx_size as u16, ctx as u16, 0],
            ),
            (false, true) => EntropySyntax::new("coeff_base_lf_eob_uv", "ctx", [ctx as u16, 0, 0]),
            (false, false) => EntropySyntax::new("coeff_base_eob_uv", "ctx", [ctx as u16, 0, 0]),
        });
    }
    match (LUMA, lf) {
        (true, true) => symbols.read_trusted(row_at(rows.base_lf_eob(), ctx)),
        (true, false) => symbols.read_trusted(row_at(rows.base_eob(), ctx)),
        (false, true) => symbols.read_trusted(row_at(rows.cdf.coeff_base_lf_eob_uv(), ctx)),
        (false, false) => symbols.read_trusted(row_at(rows.cdf.coeff_base_eob_uv(), ctx)),
    }
}

#[expect(clippy::inline_always, reason = "measured: per-coefficient index")]
#[inline(always)]
pub(super) const fn entry_grid(entry: u32) -> usize {
    entry as usize & LEVEL_WORDS_MASK
}

#[inline]
pub(super) const fn entry_pos(entry: u32) -> usize {
    (((entry >> 11) & 0x3fc) | (entry & 3)) as usize
}

const fn ctx_class_of(entry: u32) -> usize {
    (entry >> CTX_CLASS_SHIFT) as usize & 3
}

/// The base and range pass state: an owned decoder whose reads skip every
/// per-symbol check (the coefficient rows are decoder-owned), the level
/// grid and the TCQ, parity and nonzero-list state.
struct LevelPass<'a, 'r> {
    symbols: SymbolDecoder<'a>,
    levels: &'r mut CoeffLevelWords,
    width_log2: u32,
    plane: usize,
    tcq: usize,
    /// `(tcqState >> 1) & 1` of the next coefficient, taken from the state
    /// before the previous update (`Tcq_Next_State` moves bit 2 to bit 1) so
    /// it does not wait on the last symbol.
    tcq_q0: usize,
    /// Bit 0 the `sumAbs1` parity, the bits above the nonzero count.
    ph: u32,
    /// The packed scan entries of the nonzero AC levels so far, in decode
    /// order, as its first `nonzero_len` entries.
    nonzero: &'r mut CoeffNonzeroList,
    nonzero_len: usize,
}

/// One entry per scan index of the largest (32 by 32) block.
const NONZERO_LIST: usize = 1024;

impl LevelPass<'_, '_> {
    /// The sum of the grid words at `offsets` from `grid`.
    fn sum(&self, grid: usize, offsets: &[usize]) -> u32 {
        offsets
            .iter()
            .map(|&offset| self.levels[(grid & LEVEL_WORDS_MASK) + (offset & 0xff)])
            .sum()
    }

    fn br_ctx_from_word_sum<const LUMA: bool>(sum: u32, lf_bump: bool) -> usize {
        let mag = (sum >> 8) & 0xff;
        if !LUMA {
            half_capped(mag, 3)
        } else {
            half_capped(mag, MAX_BASE_BR_RANGE as usize) + if lf_bump { 7 } else { 0 }
        }
    }

    /// Adds `coeff_br` at EOB, where the neighbour sum is zero.
    fn add_eob_range<const CLASS: usize, const LUMA: bool, const LF: bool>(
        &mut self,
        rows: &mut CoeffLevelRows<'_>,
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
        let ctx = if LF && (!is_dc || CLASS != 0) { 7 } else { 0 };
        let row = Self::range_row::<LUMA, LF>(rows, ctx);
        level
            + read_level!(
                self.symbols,
                row,
                Self::range_key::<LUMA, LF>(),
                "ctx",
                [ctx as u16, 0, 0]
            )
    }

    #[cfg(feature = "block-evidence")]
    const fn range_key<const LUMA: bool, const LF: bool>() -> &'static str {
        if !LUMA {
            "coeff_br_uv"
        } else if LF {
            "coeff_br_lf"
        } else {
            "coeff_br"
        }
    }

    fn range_row<'a, const LUMA: bool, const LF: bool>(
        rows: &'a mut CoeffLevelRows<'_>,
        ctx: usize,
    ) -> &'a mut [u16; 5] {
        if !LUMA {
            row_at(rows.cdf.coeff_br_uv(), ctx)
        } else if LF {
            row_at(rows.cdf.coeff_br_lf(), ctx)
        } else {
            row_at(rows.cdf.coeff_br(), ctx)
        }
    }

    fn uv_ctx<const CLASS: usize>(&self, mag: u32) -> usize {
        half_capped(mag, 3)
            + if CLASS != 0 {
                LF_SIG_COEF_CONTEXTS_2D_UV
            } else {
                CHROMA_2D_PLANE_CONTEXT_OFFSET[self.plane.min(2)]
            }
    }

    /// Reads a low-frequency level, retaining the near-neighbour sum for
    /// a saturated luma base symbol. `DC` is whether the position is
    /// scan index 0.
    #[expect(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read_lf_level<const CLASS: usize, const LUMA: bool, const DC: bool, const TCQ: bool>(
        &mut self,
        rows: &mut CoeffLevelRows<'_>,
        entry: u32,
    ) -> u32 {
        let (grid, pos) = (entry_grid(entry), entry_pos(entry));
        let offsets = &SIG_GRID[CLASS];
        if !LUMA {
            let count = if CLASS == 0 { 3 } else { 2 };
            let mag = (self.sum(grid, &offsets[..count]) >> 8) & 0xff;
            let ctx = self.uv_ctx::<CLASS>(mag);
            let row = row_at(rows.cdf.coeff_base_lf_uv(), ctx);
            return read_level!(
                self.symbols,
                row,
                "coeff_base_lf_uv",
                "ctx",
                [ctx as u16, 0, 0]
            );
        }
        let (mag, range_sum) = if CLASS == 0 {
            let near = self.sum(grid, &offsets[..3]);
            let far = self.sum(grid, &offsets[3..]);
            (((near + far) >> 8) & 0xff, near)
        } else {
            let near = self.sum(grid, &offsets[..2]);
            let third = self.sum(grid, &offsets[2..3]);
            let far = self.sum(grid, &offsets[3..]);
            (((near >> 8) & 0xff) + ((third + far) & 0xff), near + third)
        };
        let ctx = if CLASS == 0 {
            if DC {
                let (cap, offset) = LF_2D_CONTEXT_CAP_AND_OFFSET[0];
                half_capped(mag, cap) + offset
            } else {
                usize::from(CTX_2D[1][ctx_class_of(entry)][mag as usize & 31]) & 31
            }
        } else {
            let line = if CLASS == 1 {
                pos & ((1usize << self.width_log2) - 1)
            } else {
                pos >> self.width_log2
            };
            if DC || line == 0 {
                LF_SIG_COEF_CONTEXTS_2D + half_capped(mag, 6)
            } else {
                LF_SIG_COEF_CONTEXTS_2D + 7 + half_capped(mag, 4)
            }
        };
        let q0 = if TCQ { self.tcq_q0 & 1 } else { 0 };
        #[cfg(feature = "block-evidence")]
        let tx_size_ctx = rows.tx_size as u16;
        let row = &mut row_at(rows.base_lf(), ctx)[q0];
        let level = read_level!(
            self.symbols,
            row,
            "coeff_base_lf",
            "tx_size_ctx,ctx,tcq_ctx",
            [tx_size_ctx, ctx as u16, q0 as u16]
        );
        if level <= LF_NUM_BASE_LEVELS {
            return level;
        }
        let ctx = Self::br_ctx_from_word_sum::<LUMA>(range_sum, !DC || CLASS != 0);
        let row = Self::range_row::<LUMA, true>(rows, ctx);
        level
            + read_level!(
                self.symbols,
                row,
                Self::range_key::<LUMA, true>(),
                "ctx",
                [ctx as u16, 0, 0]
            )
    }

    /// Reads a high-frequency level, sharing the packed neighbour sum of
    /// the base and range contexts.
    #[expect(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read_hf_level<const CLASS: usize, const LUMA: bool, const TCQ: bool>(
        &mut self,
        rows: &mut CoeffLevelRows<'_>,
        entry: u32,
    ) -> u32 {
        let grid = entry_grid(entry);
        let count = if LUMA || CLASS == 0 { 3 } else { 2 };
        let offsets = &SIG_GRID[CLASS];
        let near = self.sum(grid, &offsets[..count]);
        let far = if LUMA {
            self.sum(grid, &offsets[count..])
        } else {
            0
        };
        let mag = (near + far) & 0xff;
        let level = if !LUMA {
            let ctx = self.uv_ctx::<CLASS>(mag);
            let row = row_at(rows.cdf.coeff_base_uv(), ctx);
            read_level!(
                self.symbols,
                row,
                "coeff_base_uv",
                "ctx",
                [ctx as u16, 0, 0]
            )
        } else {
            let q0 = if TCQ { self.tcq_q0 & 1 } else { 0 };
            #[cfg(feature = "block-evidence")]
            let tx_size_ctx = rows.tx_size as u16;
            if CLASS == 0 {
                let first = usize::from(CTX_2D[0][ctx_class_of(entry)][mag as usize & 31]);
                let row_index = (first | q0) & 31;
                let row = &mut rows.base().as_flattened_mut()[row_index];
                read_level!(
                    self.symbols,
                    row,
                    "coeff_base",
                    "tx_size_ctx,ctx,tcq_ctx",
                    [tx_size_ctx, (row_index >> 1) as u16, q0 as u16]
                )
            } else {
                let ctx = half_capped(mag, 4) + 15;
                let row = &mut row_at(rows.base(), ctx)[q0];
                read_level!(
                    self.symbols,
                    row,
                    "coeff_base",
                    "tx_size_ctx,ctx,tcq_ctx",
                    [tx_size_ctx, ctx as u16, q0 as u16]
                )
            }
        };
        if level <= NUM_BASE_LEVELS {
            return level;
        }
        let ctx = Self::br_ctx_from_word_sum::<LUMA>(near, false);
        let row = Self::range_row::<LUMA, false>(rows, ctx);
        level
            + read_level!(
                self.symbols,
                row,
                Self::range_key::<LUMA, false>(),
                "ctx",
                [ctx as u16, 0, 0]
            )
    }

    /// Records a decoded AC level and advances the TCQ and parity state.
    #[expect(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    fn finish<const TCQ: bool, const PH: bool>(&mut self, entry: u32, level: u32) {
        let nonzero = level != 0;
        let q0 = if TCQ { self.tcq_q0 & 1 } else { 0 };
        self.levels[entry_grid(entry)] = LEVEL_WORD[(q0 << 4) | (level as usize & 15)];
        self.nonzero[self.nonzero_len & (NONZERO_LIST - 1)] = entry;
        self.nonzero_len += usize::from(nonzero);
        if TCQ {
            self.tcq_q0 = (self.tcq >> 2) & 1;
            self.tcq = next_tcq_state(self.tcq, level);
        }
        if PH {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            self.ph = (self.ph ^ (clipped & 1)) + (u32::from(level != 0) << 1);
        }
    }

    /// The high-frequency indices of [`Self::levels_pass`], out of line so
    /// the decoder state stays in registers across the loop.
    #[inline(never)]
    fn hf_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
        &mut self,
        mut rows: CoeffLevelRows<'_>,
        hf: &[u32],
    ) {
        for &entry in hf.iter().rev() {
            let level = self.read_hf_level::<CLASS, LUMA, TCQ>(&mut rows, entry);
            self.finish::<TCQ, PH>(entry, level);
        }
    }

    #[allow(clippy::inline_always, reason = "measured: one loop per tool set")]
    #[cfg_attr(target_arch = "x86_64", inline(never))]
    #[cfg_attr(not(target_arch = "x86_64"), inline(always))]
    fn lf_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
        &mut self,
        mut rows: CoeffLevelRows<'_>,
        lf: &[u32],
    ) {
        for &entry in lf.iter().rev() {
            let level = self.read_lf_level::<CLASS, LUMA, false, TCQ>(&mut rows, entry);
            self.finish::<TCQ, PH>(entry, level);
        }
    }

    /// The base and range symbols of every scan index below the EOB, in
    /// reverse scan order: the high-frequency indices, the low-frequency
    /// indices above 0 (a scan prefix, see [`lf_scan_end`]), and DC, so each
    /// phase reads one row family with its context arithmetic folded. Returns
    /// DC directly because no later context reads its grid cell.
    #[expect(clippy::inline_always, reason = "measured: one loop per tool set")]
    #[inline(always)]
    fn levels_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
        &mut self,
        rows: &mut CoeffLevelRows<'_>,
        scan: &[u32],
        eob_entry: u32,
        eob_level: u32,
        lf_end: usize,
    ) -> u32 {
        let eob_index = scan.len() - 1;
        let eob_level = if eob_index < lf_end {
            self.add_eob_range::<CLASS, LUMA, true>(rows, eob_index == 0, eob_level)
        } else {
            self.add_eob_range::<CLASS, LUMA, false>(rows, false, eob_level)
        };
        if eob_index == 0 {
            return eob_level;
        }
        self.finish::<TCQ, PH>(eob_entry, eob_level);
        if lf_end < eob_index {
            self.hf_pass::<CLASS, LUMA, TCQ, PH>(rows.reborrow(), &scan[lf_end..eob_index]);
        }
        let end = lf_end.min(eob_index);
        if end > 1 {
            self.lf_pass::<CLASS, LUMA, TCQ, PH>(rows.reborrow(), &scan[1..end]);
        }
        if PH && (self.ph >> 1) >= PHTHRESH {
            let count = if LUMA {
                5
            } else if CLASS == 0 {
                3
            } else {
                2
            };
            let mag = self.sum(entry_grid(scan[0]), &SIG_GRID[CLASS][..count]) & 0xff;
            let row = row_at(rows.cdf.coeff_base_ph(), half_capped(mag, 4));
            self.symbols.read_trusted(row)
        } else {
            self.read_lf_level::<CLASS, LUMA, true, TCQ>(rows, scan[0])
        }
    }
}

#[derive(Clone, Copy)]
struct SignFacts {
    dc_level: u32,
    dc_q0: bool,
    lf_end: usize,
    is_hidden: bool,
    sum_abs1: u32,
}

/// The sign and remainder pass: visits the nonzero levels from the EOB
/// again, reads each one's sign and Golomb remainder, writes its `Quant`
/// entry and clears the level grid behind it. A zero level's grid word is
/// already zero.
#[expect(clippy::inline_always, reason = "measured: one loop per tool set")]
#[inline(always)]
fn sign_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    levels: &mut CoeffLevelWords,
    quant: &mut [i32],
    block: &Block<'_>,
    facts: SignFacts,
    nonzero: &[u32],
) -> Result<BlockSummary, CoeffOrdinaryPassError> {
    let tx_class = [
        TransformClass::TwoD,
        TransformClass::Horizontal,
        TransformClass::Vertical,
    ][CLASS];
    let plane = if LUMA { 0 } else { block.dc.plane.max(1) };
    let scan = block.scan;
    let eob = scan.len();
    let wl = block.width_log2;
    let col_mask = (1usize << wl) - 1;
    let mut read_quant = CoeffReadQuantState::new(CoeffReadQuantConfig {
        is_hidden: facts.is_hidden,
        allow_tcq: TCQ,
        hr_level_avg: 0,
    });
    let mut quant_state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: facts.is_hidden,
        sum_abs1: facts.sum_abs1,
        use_tcq: TCQ,
        lossless: block.lossless,
    });
    let sign_config = CoeffSignSourceDeriveConfig {
        plane,
        tx_class,
        is_hidden: facts.is_hidden,
        sum_abs1: facts.sum_abs1,
        ..block.dc
    };
    let (max_hf, max_lf) = (
        visible_max_level(false, plane),
        visible_max_level(true, plane),
    );
    let mut end = 0;
    let mut run = symbols.bypass_run();
    let mut level_sum = 0u32;
    let lf_end = facts.lf_end.min(eob);
    let scan_order = LUMA && block.scan_order;
    for &scan_entry in nonzero {
        let scan_index = (scan_entry >> 22) as usize;
        let max_level = if scan_index < lf_end { max_lf } else { max_hf };
        let grid = entry_grid(scan_entry);
        let word = levels[grid];
        levels[grid] = 0;
        let pos = entry_pos(scan_entry);
        let (level, tcq_q0) = ((word >> 16) & 0xff, word >> Q0_BIT != 0);
        let entry = CoeffScanEntry::new(scan_index, pos, pos >> wl, pos & col_mask);
        let axis_cdf = LUMA
            && match CLASS {
                1 => entry.col() == 0,
                2 => entry.row() == 0,
                _ => false,
            };
        let sign = if axis_cdf {
            let input = derive_nonzero_coeff_sign_input(entry, level, sign_config);
            symbols.end_bypass_run(run);
            let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, input)?.sign();
            run = symbols.bypass_run();
            sign
        } else {
            #[cfg(feature = "block-evidence")]
            if symbols.observes_entropy() {
                symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(
                    "coeff_sign",
                    "",
                    [0; 3],
                ));
            }
            symbols.read_bool_in(&mut run)
        };
        let magnitude = if level + u32::from(TCQ) < max_level {
            level_sum += level;
            if TCQ {
                2 * level - u32::from(tcq_q0)
            } else {
                level
            }
        } else {
            let index = eob - scan_index - 1;
            symbols.end_bypass_run(run);
            let read = read_quant
                .read_one(
                    symbols,
                    index,
                    CoeffReadQuantInput {
                        entry,
                        level,
                        max_level,
                    },
                )
                .map_err(CoeffQuantPassError::from)?;
            run = symbols.bypass_run();
            level_sum = level_sum.saturating_add(read.quant);
            quant_state
                .ac_magnitude(index, read.quant, tcq_q0)
                .map_err(CoeffQuantPassError::from)?
        } as i32;
        let output = if scan_order { scan_index } else { pos };
        if let Some(slot) = quant.get_mut(output) {
            *slot = if sign { -magnitude } else { magnitude };
            end = end.max(output + 1);
        }
    }
    symbols.end_bypass_run(run);
    let mut quant_state = quant_state.with_cul_level(level_sum);

    let pos = entry_pos(scan[0]);
    let dc_output = if scan_order { 0 } else { pos };
    let level = facts.dc_level;
    if !facts.is_hidden && level != 0 {
        let sign = if LUMA {
            let dc = &block.dc;
            let ctx = dc_sign_ctx(dc.above_dc, dc.left_dc, dc.x4, dc.y4, dc.w4, dc.h4);
            #[cfg(feature = "block-evidence")]
            if symbols.observes_entropy() {
                symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(
                    "dc_sign",
                    "ctx",
                    [ctx as u16, 0, 0],
                ));
            }
            read_row(
                symbols,
                row_at(&mut cdfs.block_rows_mut().dc_sign()[0][0], ctx),
            ) != 0
        } else {
            #[cfg(feature = "block-evidence")]
            if symbols.observes_entropy() {
                symbols.annotate_entropy(splot_core::symbol::EntropySyntax::new(
                    "coeff_sign",
                    "",
                    [0; 3],
                ));
            }
            symbols.read_bool_trusted()
        };
        let index = eob - 1;
        let dc_quant = if level + u32::from(TCQ) < max_lf {
            level
        } else {
            let entry = CoeffScanEntry::new(0, pos, pos >> wl, pos & col_mask);
            let input = CoeffReadQuantInput {
                entry,
                level,
                max_level: max_lf,
            };
            let read = read_quant.read_one(symbols, index, input);
            read.map_err(CoeffQuantPassError::from)?.quant
        };
        quant_state.set_dc_sign(sign);
        let magnitude = quant_state
            .ac_magnitude(index, dc_quant, facts.dc_q0)
            .map_err(CoeffQuantPassError::from)? as i32;
        if let Some(slot) = quant.get_mut(dc_output) {
            *slot = if sign { -magnitude } else { magnitude };
            end = end.max(dc_output + 1);
        }
    } else if facts.is_hidden {
        let index = eob - 1;
        let entry = CoeffScanEntry::new(0, pos, pos >> wl, pos & col_mask);
        let sign_input = derive_nonzero_coeff_sign_input(entry, level, sign_config);
        let max_level = derive_coeff_max_level(
            entry,
            CoeffMaxLevelConfig {
                plane,
                tx_class,
                is_hidden: true,
            },
        );
        let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, sign_input)?;
        if facts.sum_abs1 > 0 && sign.symbol() == CoeffSignReadSymbol::None {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry }.into());
        }
        let read = read_quant
            .read_one(
                symbols,
                index,
                CoeffReadQuantInput {
                    entry,
                    level,
                    max_level,
                },
            )
            .map_err(CoeffQuantPassError::from)?;
        quant_state.set_tcq_q0(facts.dc_q0);
        let write = quant_state
            .apply_entry(index, entry, sign.sign(), read)
            .map_err(CoeffQuantPassError::from)?;
        if let Some(slot) = quant.get_mut(dc_output) {
            *slot = write.quant();
            if write.quant() != 0 {
                end = end.max(dc_output + 1);
            }
        }
    }
    Ok(BlockSummary {
        state: NonZeroCoeffQuantState::from_accumulator(quant_state),
        quant_end: end,
    })
}

#[cfg(test)]
#[path = "transform_block_tests.rs"]
mod tests;
