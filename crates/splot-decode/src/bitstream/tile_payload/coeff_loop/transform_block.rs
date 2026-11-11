// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The coefficients of one ordinary (non-FSC) transform block (§ 5.20.7.27)
//! in one call: the reverse-scan base and range pass over a padded level
//! grid, then the sign and remainder pass writing `Quant` directly.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::coefficient::LF_SIG_COEF_CONTEXTS_2D;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder};
use splot_recon::{TransformClass, coefficient_scan_position};

use super::super::cdf::block_read::{BlockSymbolTraceReadError, read_row, row_at};
use super::super::cdf::coeff_context::{
    CHROMA_2D_PLANE_CONTEXT_OFFSET, HF_2D_CONTEXT_OFFSET, LF_2D_CONTEXT_CAP_AND_OFFSET,
    LF_SIG_COEF_CONTEXTS_2D_UV, MAG_REF_OFFSET_WITH_TX_CLASS, MAX_BASE_BR_RANGE, SIG_OFFSETS,
    coeff_base_eob_ctx, dc_sign_ctx,
};
use super::super::cdf::{CoeffLevelRows, TileCdfSubset};
use super::super::coeff_state::{
    CoeffBlockLines, CoeffLevelWords, CoeffNonzeroList, LEVEL_STRIDE, LEVEL_WORDS_MASK,
    TileCoeffContextState, TileCoeffStateError,
};
use super::CoeffLoopContextError;
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError, tx_class_index,
};
use super::base_symbol::CoeffBaseSymbolReadError;
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{
    COEFF_BASE_RANGE, CoeffMaxLevelConfig, CoeffTransformClass, LF_NUM_BASE_LEVELS,
    NUM_BASE_LEVELS, derive_coeff_max_level, visible_max_level,
};
use super::ordinary_pass::CoeffOrdinaryPassError;
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

/// `Mag_Ref_Offset_With_Tx_Class` per class as level-grid offsets.
const MAG_GRID: [[usize; 3]; 3] = {
    let mut out = [[0; 3]; 3];
    let mut class = 0;
    while class < 3 {
        let mut k = 0;
        while k < 3 {
            out[class][k] = grid_offset(MAG_REF_OFFSET_WITH_TX_CLASS[class][k]);
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

/// `HALF_CAPPED[cap][mag]` is `min((mag + 1) >> 1, cap)`, the capped
/// context of a neighbour sum, as a load instead of a data-dependent branch.
static HALF_CAPPED: [[u8; 32]; 16] = {
    let mut table = [[0; 32]; 16];
    let mut cap = 0;
    while cap < 16 {
        let mut mag = 0;
        while mag < 32 {
            let half = (mag + 1) >> 1;
            table[cap][mag] = if half < cap { half } else { cap } as u8;
            mag += 1;
        }
        cap += 1;
    }
    table
};

/// `Tcq_Next_State[state][parity]` at `state << 1 | parity`.
static TCQ_NEXT: [u8; 16] = {
    let mut out = [0; 16];
    let mut index = 0;
    while index < 16 {
        out[index] = next_tcq_state(index >> 1, (index & 1) as u32) as u8;
        index += 1;
    }
    out
};

#[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
#[inline(always)]
fn tcq_next(state: usize, level: u32) -> usize {
    usize::from(TCQ_NEXT[((state << 1) | (level as usize & 1)) & 15])
}

const SCAN_DIMS: usize = 4;
const GRID_SCAN_LEN: usize = 3600;

/// Where the `4 << wi` by `4 << hi` shape starts in [`GRID_SCANS`].
const fn scan_shape_offset(wi: usize, hi: usize) -> usize {
    let (mut offset, mut w, mut h) = (0, 0, 0);
    while w < wi || (w == wi && h < hi) {
        offset += (4 << w) * (4 << h);
        h += 1;
        if h == SCAN_DIMS {
            (w, h) = (w + 1, 0);
        }
    }
    offset
}

/// [`scan_shape_offset`] by `[wi][hi]`.
static SCAN_SHAPE_OFFSETS: [[usize; SCAN_DIMS]; SCAN_DIMS] = {
    let mut out = [[0; SCAN_DIMS]; SCAN_DIMS];
    let mut i = 0;
    while i < SCAN_DIMS * SCAN_DIMS {
        out[i / SCAN_DIMS][i % SCAN_DIMS] = scan_shape_offset(i / SCAN_DIMS, i % SCAN_DIMS);
        i += 1;
    }
    out
};

/// The § 5.20.7.30 scan of every block shape per transform class, each
/// position as its level-grid index, its context class at
/// [`CTX_CLASS_SHIFT`] and the position itself above bit 16.
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
                    out[class][base + c] = (grid | context | (pos << 16)) as u32;
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
                    let pos = (GRID_SCANS[class][base + c] >> 16) as u16;
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

/// One past the largest raster position of the first `eob` scan positions
/// of a block of `class` whose sides are `2^width_log2` and `2^height_log2`
/// (each at most 32 after the § 7.13.3 adjustment): the coefficients of the
/// block that can be nonzero lie below it. The whole block for an invalid
/// shape or `eob`.
#[allow(
    clippy::inline_always,
    reason = "measured: out of line it costs 30 instructions per block"
)]
#[inline(always)]
pub(crate) fn scan_extent(
    class: CoeffTransformClass,
    (width_log2, height_log2): (usize, usize),
    eob: usize,
) -> usize {
    let (wi, hi) = (width_log2.clamp(2, 5) - 2, height_log2.clamp(2, 5) - 2);
    let len = 16 << (wi + hi);
    if eob == 0 || eob > len {
        return len;
    }
    let offset = SCAN_SHAPE_OFFSETS[wi.min(3)][hi.min(3)];
    GRID_EXTENTS[tx_class_index(class)]
        .get(offset + eob - 1)
        .map_or(len, |&extent| usize::from(extent))
}

/// The scan of a `width` by `height` block of `class` up to `eob`.
pub(super) fn grid_scan(
    class: CoeffTransformClass,
    width: usize,
    height: usize,
    eob: usize,
) -> Result<&'static [u32], CoeffOrdinaryPassError> {
    let dim = |extent: usize| {
        (4..=32)
            .contains(&extent)
            .then(|| extent.trailing_zeros() as usize - 2)
    };
    let (Some(wi), Some(hi)) = (dim(width), dim(height)) else {
        return Err(CoeffLoopContextError::ScanWalkWidthNotPowerOfTwo { width }.into());
    };
    if !width.is_power_of_two() || !height.is_power_of_two() {
        return Err(CoeffLoopContextError::ScanWalkWidthNotPowerOfTwo { width }.into());
    }
    let len = width * height;
    if eob == 0 {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob }.into());
    }
    if eob > len {
        return Err(CoeffLoopContextError::ScanWalkEobOutOfRange { eob, scan_len: len }.into());
    }
    let start = SCAN_SHAPE_OFFSETS[wi][hi];
    let scans = &GRID_SCANS[tx_class_index(class)];
    Ok(scans.get(start..start + eob).unwrap_or_default())
}

#[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
#[inline(always)]
fn half_capped(mag: u32, cap: usize) -> usize {
    usize::from(HALF_CAPPED[cap & 15][mag as usize & 31])
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
    pub(crate) config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) lines: &'l CoeffBlockLines,
    pub(crate) lossless: bool,
}

/// Decodes the block's coefficients into `quant` (zero on entry, one entry
/// per adjusted position up to at least the [`scan_extent`] of the EOB),
/// commits its above and left contexts, and returns one past its last
/// nonzero entry.
pub(crate) fn decode_ordinary_transform_block(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffTransformBlockInput<'_>,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryPassError> {
    let config = input.config;
    preflight(&input, quant.len())?;
    let scan = grid_scan(
        config.tx_class,
        config.tx_width,
        config.tx_height,
        input.start.eob_read().eob(),
    )?;
    let lines = input.lines;
    let plane = config.plane;
    let (above_dc, left_dc, levels, nonzero) = state.dc_lines_and_levels(plane)?;
    let block = Block {
        scan,
        config,
        dc: CoeffSignSourceDeriveConfig {
            plane,
            plane_type: usize::from(plane > 0),
            tx_class: config.tx_class,
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
    let summary = if symbols.cdf_update_mode() == CdfUpdateMode::Enabled {
        dispatch::<true>(cdfs, symbols, (levels, nonzero), quant, &block)
    } else {
        dispatch::<false>(cdfs, symbols, (levels, nonzero), quant, &block)
    }?;
    state.commit_block(
        lines,
        summary.state.cul_level(),
        summary.state.dc_category(),
    );
    Ok(summary.quant_end)
}

fn preflight(
    input: &CoeffTransformBlockInput<'_>,
    quant_len: usize,
) -> Result<(), CoeffOrdinaryPassError> {
    let config = input.config;
    if config.parity_hiding && config.use_tcq {
        return Err(CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq.into());
    }
    if (config.plane > 0 && (config.parity_hiding || config.use_tcq))
        || (config.use_tcq && config.tx_class != CoeffTransformClass::TwoD)
    {
        return Err(CoeffBaseDerivedLevelPassError::UnsupportedToolClass.into());
    }
    if input.lines.plane != config.plane {
        return Err(TileCoeffStateError::InvalidPlane {
            plane: input.lines.plane,
        }
        .into());
    }
    if input.lossless && config.use_tcq {
        return Err(CoeffQuantPassError::InconsistentTcqConfig.into());
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
    config: CoeffBaseDerivedLevelPassConfig,
    dc: CoeffSignSourceDeriveConfig<'a>,
    lossless: bool,
}

struct BlockSummary {
    state: NonZeroCoeffQuantState,
    quant_end: usize,
}

#[allow(clippy::inline_always, reason = "measured: one body per tool set")]
#[inline(always)]
fn dispatch<const UPD: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    levels: (&mut CoeffLevelWords, &mut CoeffNonzeroList),
    quant: &mut [i32],
    block: &Block<'_>,
) -> Result<BlockSummary, CoeffOrdinaryPassError> {
    use CoeffTransformClass::{Horizontal, TwoD, Vertical};
    let config = block.config;
    let args = (cdfs, symbols, levels, quant, block);
    match (
        config.tx_class,
        config.plane == 0,
        config.use_tcq,
        config.parity_hiding,
    ) {
        (TwoD, true, true, _) => run::<0, true, true, false, UPD>(args),
        (TwoD, true, false, true) => run::<0, true, false, true, UPD>(args),
        (TwoD, true, false, false) => run::<0, true, false, false, UPD>(args),
        (Horizontal, true, _, true) => run::<1, true, false, true, UPD>(args),
        (Horizontal, true, _, false) => run::<1, true, false, false, UPD>(args),
        (Vertical, true, _, true) => run::<2, true, false, true, UPD>(args),
        (Vertical, true, _, false) => run::<2, true, false, false, UPD>(args),
        (TwoD, false, ..) => run::<0, false, false, false, UPD>(args),
        (Horizontal, false, ..) => run::<1, false, false, false, UPD>(args),
        (Vertical, false, ..) => run::<2, false, false, false, UPD>(args),
    }
}

type RunArgs<'r, 'a, 'b> = (
    &'r mut TileCdfSubset,
    &'r mut SymbolDecoder<'a>,
    (&'r mut CoeffLevelWords, &'r mut CoeffNonzeroList),
    &'r mut [i32],
    &'r Block<'b>,
);

/// One specialised block body per class, plane type and tool set, kept out
/// of line so its loops own the registers.
#[inline(never)]
fn run<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool, const UPD: bool>(
    (cdfs, symbols, (levels, nonzero), quant, block): RunArgs<'_, '_, '_>,
) -> Result<BlockSummary, CoeffOrdinaryPassError> {
    let config = block.config;
    let scan = block.scan;
    let Some(&eob_entry) = scan.last() else {
        return Err(CoeffLoopContextError::InvalidScanWalkEob { eob: 0 }.into());
    };
    let lf_end = lf_scan_end::<CLASS, LUMA>(config.tx_width, config.tx_height).max(1);
    let eob_index = scan.len() - 1;
    let mut rows = cdfs
        .coeff_rows_mut()
        .level_rows(config.tx_size_ctx)
        .map_err(|error| CoeffBaseSymbolReadError::from(BlockSymbolTraceReadError::from(error)))
        .map_err(CoeffBaseDerivedLevelPassError::from)?;
    let eob_ctx = coeff_base_eob_ctx(eob_index, config.tx_width_log2, config.tx_height);
    let eob_level = read_eob_base::<LUMA, UPD>(symbols, &mut rows, eob_ctx, eob_index < lf_end) + 1;
    let mut pass = LevelPass::<UPD> {
        symbols: symbols.snapshot(),
        levels: &mut *levels,
        rows,
        width_log2: config.tx_width_log2,
        plane: config.plane,
        tcq: 0,
        tcq_q0: 0,
        ph: 0,
        nonzero: &mut *nonzero,
        nonzero_len: 0,
    };
    pass.levels_pass::<CLASS, LUMA, TCQ, PH>(scan, eob_entry, eob_level, lf_end);
    let (mut owned, ph, nonzero_len) = (pass.symbols, pass.ph, pass.nonzero_len);
    let nonzero = nonzero.get(..nonzero_len).unwrap_or_default();
    let facts = SignFacts {
        lf_end,
        is_hidden: PH && (ph >> 1) >= PHTHRESH,
        sum_abs1: if PH { ph & 1 } else { 0 },
    };
    let result =
        sign_pass::<CLASS, LUMA, TCQ>(cdfs, &mut owned, levels, quant, block, facts, nonzero);
    symbols.restore(owned);
    if result.is_err() {
        levels.fill(0);
    }
    result
}

/// Reads the `coeff_base_eob` symbol of context `ctx` (§ 8.3.2).
#[allow(clippy::inline_always, reason = "measured: per-block symbol read")]
#[inline(always)]
fn read_eob_base<const LUMA: bool, const UPD: bool>(
    symbols: &mut SymbolDecoder<'_>,
    rows: &mut CoeffLevelRows<'_>,
    ctx: usize,
    lf: bool,
) -> u32 {
    match (LUMA, lf) {
        (true, true) => symbols.read_trusted::<6, UPD>(row_at(rows.base_lf_eob, ctx)),
        (true, false) => symbols.read_trusted::<4, UPD>(row_at(rows.base_eob, ctx)),
        (false, true) => symbols.read_trusted::<6, UPD>(row_at(rows.base_lf_eob_uv, ctx)),
        (false, false) => symbols.read_trusted::<4, UPD>(row_at(rows.base_eob_uv, ctx)),
    }
}

#[allow(clippy::inline_always, reason = "measured: per-coefficient index")]
#[inline(always)]
pub(super) const fn entry_grid(entry: u32) -> usize {
    entry as usize & LEVEL_WORDS_MASK
}

#[allow(clippy::inline_always, reason = "measured: per-coefficient index")]
#[inline(always)]
const fn ctx_class_of(entry: u32) -> usize {
    (entry >> CTX_CLASS_SHIFT) as usize & 3
}

/// The base and range pass state: an owned decoder whose reads skip every
/// per-symbol check (the coefficient rows are decoder-owned), the level
/// grid, and the CDF rows of the block's transform-size context.
struct LevelPass<'a, 'r, const UPD: bool> {
    symbols: SymbolDecoder<'a>,
    levels: &'r mut CoeffLevelWords,
    rows: CoeffLevelRows<'r>,
    width_log2: u32,
    plane: usize,
    tcq: usize,
    /// `(tcqState >> 1) & 1` of the next coefficient, taken from the state
    /// before the previous update (`Tcq_Next_State` moves bit 2 to bit 1) so
    /// it does not wait on the last symbol.
    tcq_q0: usize,
    /// Bit 0 the `sumAbs1` parity, the bits above the nonzero count.
    ph: u32,
    /// The scan indices above 0 of the nonzero levels so far, in decode
    /// order, as its first `nonzero_len` entries.
    nonzero: &'r mut CoeffNonzeroList,
    nonzero_len: usize,
}

/// One entry per scan index of the largest (32 by 32) block.
const NONZERO_LIST: usize = 1024;

impl<const UPD: bool> LevelPass<'_, '_, UPD> {
    /// The sum of the grid words at `offsets` from `grid`.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn sum(&self, grid: usize, offsets: &[usize]) -> u32 {
        offsets
            .iter()
            .map(|&offset| self.levels[(grid & LEVEL_WORDS_MASK) + (offset & 0xff)])
            .sum()
    }

    /// The clamped `coeff_base` neighbour sum (§ 8.3.2): `NEAR5` and `FAR5`
    /// cap the first two and the remaining `Sig_Ref_Diff_Offset` entries at
    /// 5 instead of 3.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn base_mag<const CLASS: usize, const LUMA: bool, const NEAR5: bool, const FAR5: bool>(
        &self,
        grid: usize,
    ) -> u32 {
        let count = if LUMA {
            5
        } else if CLASS == 0 {
            3
        } else {
            2
        };
        let offsets = &SIG_GRID[CLASS][..count];
        if NEAR5 == FAR5 {
            let shift = if NEAR5 { 8 } else { 0 };
            (self.sum(grid, offsets) >> shift) & 0xff
        } else {
            ((self.sum(grid, &offsets[..2]) >> 8) & 0xff) + (self.sum(grid, &offsets[2..]) & 0xff)
        }
    }

    /// The `coeff_br` context (§ 8.3.2); `lf_bump` is whether the context
    /// takes the low-frequency offset of 7.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn br_ctx<const CLASS: usize, const LUMA: bool>(&self, grid: usize, lf_bump: bool) -> usize {
        let count = if LUMA || CLASS == 0 { 3 } else { 2 };
        let mag = (self.sum(grid, &MAG_GRID[CLASS][..count]) >> 8) & 0xff;
        if !LUMA {
            half_capped(mag, 3)
        } else {
            half_capped(mag, MAX_BASE_BR_RANGE as usize) + if lf_bump { 7 } else { 0 }
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
        let ctx = self.br_ctx::<CLASS, LUMA>(grid, LF && (!is_dc || CLASS != 0));
        level
            + if !LUMA {
                let row = row_at(self.rows.br_uv, ctx);
                self.symbols.read_trusted::<5, UPD>(row)
            } else if LF {
                let row = row_at(self.rows.br_lf, ctx);
                self.symbols.read_trusted::<5, UPD>(row)
            } else {
                let row = row_at(self.rows.br, ctx);
                self.symbols.read_trusted::<5, UPD>(row)
            }
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient context")]
    #[inline(always)]
    fn uv_ctx<const CLASS: usize>(&self, mag: u32) -> usize {
        half_capped(mag, 3)
            + if CLASS != 0 {
                LF_SIG_COEF_CONTEXTS_2D_UV
            } else {
                CHROMA_2D_PLANE_CONTEXT_OFFSET[self.plane.min(2)]
            }
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
        entry: u32,
    ) -> u32 {
        let (grid, pos) = (entry_grid(entry), (entry >> 16) as usize);
        let mag = if CLASS == 0 {
            self.base_mag::<CLASS, LUMA, true, true>(grid)
        } else {
            self.base_mag::<CLASS, LUMA, true, false>(grid)
        };
        if !LUMA {
            let ctx = self.uv_ctx::<CLASS>(mag);
            let row = row_at(self.rows.base_lf_uv, ctx);
            return self.symbols.read_trusted::<7, UPD>(row);
        }
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
        let q0 = self.tcq_q0 & 1;
        let row = &mut row_at(self.rows.base_lf, ctx)[q0];
        self.symbols.read_trusted::<7, UPD>(row)
    }

    /// Reads one high-frequency base symbol.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient symbol read"
    )]
    #[inline(always)]
    fn read_hf_base<const CLASS: usize, const LUMA: bool>(&mut self, entry: u32) -> u32 {
        let mag = self.base_mag::<CLASS, LUMA, false, false>(entry_grid(entry));
        if !LUMA {
            let ctx = self.uv_ctx::<CLASS>(mag);
            let row = row_at(self.rows.base_uv, ctx);
            return self.symbols.read_trusted::<5, UPD>(row);
        }
        let q0 = self.tcq_q0 & 1;
        let row = if CLASS == 0 {
            let first = usize::from(CTX_2D[0][ctx_class_of(entry)][mag as usize & 31]);
            &mut self.rows.base.as_flattened_mut()[(first | q0) & 31]
        } else {
            &mut row_at(self.rows.base, half_capped(mag, 4) + 15)[q0]
        };
        self.symbols.read_trusted::<5, UPD>(row)
    }

    /// Records a decoded level and advances the TCQ and parity state.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    fn finish<const TCQ: bool, const PH: bool>(
        &mut self,
        scan_index: usize,
        grid: usize,
        level: u32,
    ) {
        let nonzero = level != 0;
        let q0 = if TCQ { self.tcq_q0 & 1 } else { 0 };
        self.levels[grid & LEVEL_WORDS_MASK] = LEVEL_WORD[(q0 << 4) | (level as usize & 15)];
        if scan_index > 0 {
            self.nonzero[self.nonzero_len & (NONZERO_LIST - 1)] = scan_index as u32;
            self.nonzero_len += usize::from(nonzero);
        }
        if TCQ {
            self.tcq_q0 = (self.tcq >> 2) & 1;
            self.tcq = tcq_next(self.tcq, level);
        }
        if PH && scan_index > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            self.ph = (self.ph ^ (clipped & 1)) + (u32::from(level != 0) << 1);
        }
    }

    /// The high-frequency indices of [`Self::levels_pass`], out of line so
    /// the decoder state stays in registers across the loop.
    #[inline(never)]
    fn hf_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
        &mut self,
        hf: &[u32],
        lf_end: usize,
    ) {
        for (offset, &entry) in hf.iter().enumerate().rev() {
            let grid = entry_grid(entry);
            let level = self.read_hf_base::<CLASS, LUMA>(entry);
            let level = self.add_range::<CLASS, LUMA, false>(grid, false, level);
            self.finish::<TCQ, PH>(lf_end + offset, grid, level);
        }
    }

    /// The base and range symbols of every scan index below the EOB, in
    /// reverse scan order: the high-frequency indices, the low-frequency
    /// indices above 0 (a scan prefix, see [`lf_scan_end`]), and DC, so each
    /// phase reads one row family with its context arithmetic folded.
    #[allow(clippy::inline_always, reason = "measured: one loop per tool set")]
    #[inline(always)]
    fn levels_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
        &mut self,
        scan: &[u32],
        eob_entry: u32,
        eob_level: u32,
        lf_end: usize,
    ) {
        let eob_index = scan.len() - 1;
        let eob_grid = entry_grid(eob_entry);
        let eob_level = if eob_index < lf_end {
            self.add_range::<CLASS, LUMA, true>(eob_grid, eob_index == 0, eob_level)
        } else {
            self.add_range::<CLASS, LUMA, false>(eob_grid, false, eob_level)
        };
        self.finish::<TCQ, PH>(eob_index, eob_grid, eob_level);
        if eob_index == 0 {
            return;
        }
        self.hf_pass::<CLASS, LUMA, TCQ, PH>(
            scan.get(lf_end..eob_index).unwrap_or_default(),
            lf_end,
        );
        let lf = scan.get(1..lf_end.min(eob_index)).unwrap_or_default();
        for (offset, &entry) in lf.iter().enumerate().rev() {
            let grid = entry_grid(entry);
            let level = self.read_lf_base::<CLASS, LUMA, false>(entry);
            let level = self.add_range::<CLASS, LUMA, true>(grid, false, level);
            self.finish::<TCQ, PH>(1 + offset, grid, level);
        }
        let dc_grid = entry_grid(scan[0]);
        let level = if PH && (self.ph >> 1) >= PHTHRESH {
            let mag = self.base_mag::<CLASS, LUMA, false, false>(dc_grid);
            let row = row_at(self.rows.base_ph, half_capped(mag, 4));
            self.symbols.read_trusted::<5, UPD>(row)
        } else {
            let level = self.read_lf_base::<CLASS, LUMA, true>(scan[0]);
            self.add_range::<CLASS, LUMA, true>(dc_grid, true, level)
        };
        self.finish::<TCQ, PH>(0, dc_grid, level);
    }
}

#[derive(Clone, Copy)]
struct SignFacts {
    lf_end: usize,
    is_hidden: bool,
    sum_abs1: u32,
}

/// The sign and remainder pass: visits the nonzero levels from the EOB
/// again, reads each one's sign and Golomb remainder, writes its `Quant`
/// entry and clears the level grid behind it. A zero level's grid word is
/// already zero.
#[allow(clippy::inline_always, reason = "measured: one loop per tool set")]
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
        CoeffTransformClass::TwoD,
        CoeffTransformClass::Horizontal,
        CoeffTransformClass::Vertical,
    ][CLASS];
    let plane = if LUMA { 0 } else { block.config.plane.max(1) };
    let scan = block.scan;
    let eob = scan.len();
    let wl = block.config.tx_width_log2;
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
    macro_rules! sign_step {
        ($scan_index:expr, $max_level:expr) => {{
            let (scan_index, max_level) = ($scan_index, $max_level);
            let Some(&scan_entry) = scan.get(scan_index) else {
                continue;
            };
            let grid = entry_grid(scan_entry);
            let word = levels[grid];
            levels[grid] = 0;
            let pos = (scan_entry >> 16) as usize;
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
            if let Some(slot) = quant.get_mut(pos) {
                *slot = if sign { -magnitude } else { magnitude };
                end = end.max(pos + 1);
            }
        }};
    }
    let lf_end = facts.lf_end.min(eob);
    for &scan_index in nonzero {
        let scan_index = scan_index as usize;
        sign_step!(
            scan_index,
            if scan_index < lf_end { max_lf } else { max_hf }
        );
    }
    symbols.end_bypass_run(run);
    let mut quant_state = quant_state.with_cul_level(level_sum);

    let (pos, grid) = ((scan[0] >> 16) as usize, entry_grid(scan[0]));
    let word = levels[grid];
    levels[grid] = 0;
    let level = (word >> 16) & 0xff;
    if !facts.is_hidden && level != 0 {
        let sign = if LUMA {
            let dc = &block.dc;
            let ctx = dc_sign_ctx(dc.above_dc, dc.left_dc, dc.x4, dc.y4, dc.w4, dc.h4);
            read_row(
                symbols,
                row_at(&mut cdfs.block_rows_mut().dc_sign[0][0], ctx),
            ) != 0
        } else {
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
            .ac_magnitude(index, dc_quant, word >> Q0_BIT != 0)
            .map_err(CoeffQuantPassError::from)? as i32;
        if let Some(slot) = quant.get_mut(pos) {
            *slot = if sign { -magnitude } else { magnitude };
            end = end.max(pos + 1);
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
                is_hidden: facts.is_hidden,
            },
        );
        let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, sign_input)?;
        if facts.is_hidden && facts.sum_abs1 > 0 && sign.symbol() == CoeffSignReadSymbol::None {
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
        quant_state.set_tcq_q0(word >> Q0_BIT != 0);
        let write = quant_state
            .apply_entry(index, entry, sign.sign(), read)
            .map_err(CoeffQuantPassError::from)?;
        if let Some(slot) = quant.get_mut(pos) {
            *slot = write.quant();
            if write.quant() != 0 {
                end = end.max(pos + 1);
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
