// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rates of the § 5.20.7.27 `coeffs()` syntax that `tile_writer::coeffs` writes:
//! 2D-class blocks without TCQ, parity hiding, or forward skip coding.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolEncoder;
use splot_decode::encoder_syntax::{
    CoeffLevels, IntraTileSyntax, TransformBlock, is_low_frequency,
};
use splot_recon::{TransformClass, coefficient_scan_slice};

use super::{Bits, symbol_cost, unary_cost};
use crate::tile_writer::coeffs::adjusted_log2;
use crate::tile_writer::{Result, TileWriterError};

/// `1 << 20`: the § 6.19.7.23 bound on a coded coefficient magnitude.
const MAX_QUANT_MAGNITUDE: u32 = 1 << 20;
/// The `eob_pt_*` symbol that carries an `eob_pt_*_extra` literal.
const LAST_PLAIN_EOB_PT_SYMBOL: usize = 7;
/// The longest `golomb_length_bit` prefix.
const MAX_GOLOMB_PREFIX_BITS: u32 = 21;
/// Scratch-encoder operations before [`FacadeRates`] drops its discarded output.
const SCRATCH_RESET_OPERATIONS: usize = 1 << 19;

/// The rate of each part of one transform block's coefficient syntax.
///
/// A model only prices symbols; the writer still records the coded contexts.
pub(crate) trait CoeffRateModel {
    /// The `all_zero` symbol; `eob_u_nonzero` is `EobU != 0` for a V block.
    ///
    /// # Errors
    /// Returns an error for a transform block outside the tile.
    fn all_zero_rate(
        &mut self,
        tx: &TransformBlock,
        eob_u_nonzero: bool,
        all_zero: bool,
    ) -> Result<Bits>;

    /// `eob_pt_*`, `eob_pt_*_extra`, `eob_extra`, and `eob_extra_bit`; zero for `eob == 0`.
    ///
    /// # Errors
    /// Returns an error for an unknown transform size.
    fn eob_rate(&mut self, tx: &TransformBlock, eob: usize) -> Result<Bits>;

    /// The base, range, sign, and Golomb syntax of the signed `level` at adjusted
    /// `(row, col)` and scan index `c`, in a block with `eob`. `coded` holds the
    /// clamped magnitudes of the scan indices above `c`; `hr_level_avg` is the
    /// § 5.20.7.28 running average, updated in reverse scan order.
    ///
    /// # Errors
    /// Returns an error for a CDF table defect.
    fn level_rate(
        &mut self,
        tx: &TransformBlock,
        coded: &CoeffLevels,
        position: (usize, usize, usize),
        level: i32,
        eob: usize,
        hr_level_avg: &mut u32,
    ) -> Result<Bits>;
}

/// `maxLevel` of § 5.20.7.27 without parity hiding.
pub(crate) const fn max_level(is_lf: bool, plane: usize) -> u32 {
    match (is_lf, plane == 0) {
        (true, true) => LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
        (true, false) => LF_NUM_BASE_LEVELS + 1,
        (false, _) => NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
    }
}

/// The rate of the coefficient syntax of `tx` with row-major adjusted `levels`.
///
/// # Errors
/// Returns [`TileWriterError::LevelCount`] for a wrong level count and model errors.
pub(crate) fn block_rate(
    model: &mut impl CoeffRateModel,
    tx: &TransformBlock,
    levels: &[i32],
    eob_u_nonzero: bool,
) -> Result<Bits> {
    let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
    let (width, height) = (1usize << log2_w, 1usize << log2_h);
    if levels.len() != width * height {
        return Err(TileWriterError::LevelCount {
            expected: width * height,
            actual: levels.len(),
        });
    }
    let scan = coefficient_scan_slice(width, height, TransformClass::TwoD)?;
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);
    let mut bits = model.all_zero_rate(tx, eob_u_nonzero, eob == 0)?;
    if eob == 0 {
        return Ok(bits);
    }
    bits += model.eob_rate(tx, eob)?;
    let mut coded = CoeffLevels::new(width, height)?;
    let mut hr_level_avg = 0;
    for c in (0..eob).rev() {
        let pos = usize::from(scan[c]);
        let (row, col) = (pos >> log2_w, pos & (width - 1));
        let level = levels[pos];
        bits += model.level_rate(tx, &coded, (row, col, c), level, eob, &mut hr_level_avg)?;
        let max = max_level(is_low_frequency(row, col, tx.plane), tx.plane);
        coded.set(row, col, level.unsigned_abs().min(max))?;
    }
    Ok(bits)
}

/// The rate of the § 5.20.7.28 Golomb remainder `x` and the next `hrLevelAvg`.
pub(crate) fn golomb_rate(x: u32, hr_level_avg: u32) -> (Bits, u32) {
    let m = hr_level_avg.checked_ilog2().unwrap_or(0).clamp(1, 6);
    let k = m + 1;
    let c_max = (m + 4).min(6);
    let q = x >> m;
    let bits = if q < c_max {
        unary_cost(q, c_max) + Bits::whole(m)
    } else {
        let length = (x - (c_max << m) + (1 << k)).ilog2();
        unary_cost(c_max, c_max)
            + unary_cost(length - k, MAX_GOLOMB_PREFIX_BITS)
            + Bits::whole(length)
    };
    (bits, (x + hr_level_avg) >> 1)
}

/// A [`CoeffRateModel`] that prices symbols with the facade's CDF rows.
pub(crate) struct FacadeRates<'a> {
    syntax: &'a mut IntraTileSyntax,
    /// Adapts each priced row like the writer when set.
    scratch: Option<SymbolEncoder>,
}

impl<'a> FacadeRates<'a> {
    /// Prices every symbol with the rows as they stand; the rows do not change.
    pub(crate) const fn frozen(syntax: &'a mut IntraTileSyntax) -> Self {
        Self {
            syntax,
            scratch: None,
        }
    }

    /// Prices each symbol, then adapts its row exactly as the writer would, so a
    /// trial sees in-block adaptation. Restore the facade afterwards.
    pub(crate) fn adapting(syntax: &'a mut IntraTileSyntax) -> Self {
        Self {
            syntax,
            scratch: Some(SymbolEncoder::new()),
        }
    }
}

/// The cost of `symbol` in `row`; adapts `row` through `scratch` when set.
fn charge(scratch: &mut Option<SymbolEncoder>, row: &mut [u16], symbol: usize) -> Result<Bits> {
    let cost = symbol_cost(row, symbol);
    if let Some(scratch) = scratch {
        // shortcut: splot-core keeps `update_cdf` private, so the row adapts through a
        // discarded encoder; expose `update_cdf` to drop the scratch.
        if scratch.operation_count() > SCRATCH_RESET_OPERATIONS {
            *scratch = SymbolEncoder::new();
        }
        let symbol =
            u8::try_from(symbol).map_err(|_| TileWriterError::Unsupported("symbol value"))?;
        scratch.write_symbol_u16(row, Symbol::new(symbol))?;
    }
    Ok(cost)
}

impl CoeffRateModel for FacadeRates<'_> {
    fn all_zero_rate(
        &mut self,
        tx: &TransformBlock,
        eob_u_nonzero: bool,
        all_zero: bool,
    ) -> Result<Bits> {
        let cdf = self.syntax.txb_skip_cdf(tx, eob_u_nonzero)?;
        charge(&mut self.scratch, cdf, usize::from(all_zero))
    }

    fn eob_rate(&mut self, tx: &TransformBlock, eob: usize) -> Result<Bits> {
        if eob == 0 {
            return Ok(Bits::default());
        }
        let eob_pt = if eob <= 2 {
            eob
        } else {
            (eob - 1).ilog2() as usize + 2
        };
        let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
        let eob_multisize = log2_w + log2_h - 4;
        let symbol = (eob_pt - 1).min(LAST_PLAIN_EOB_PT_SYMBOL);
        let cdf = self.syntax.eob_pt_cdf(tx.plane, tx.tx_size)?;
        let mut bits = charge(&mut self.scratch, cdf, symbol)?;
        if eob_multisize >= 4 && symbol == LAST_PLAIN_EOB_PT_SYMBOL {
            bits += Bits::whole(if eob_multisize == 4 { 1 } else { 2 });
        }
        if eob_pt >= 3 {
            let high = eob_pt - 3;
            let offset = eob - ((1 << (eob_pt - 2)) + 1);
            let cdf = self.syntax.eob_extra_cdf()?;
            bits += charge(&mut self.scratch, cdf, (offset >> high) & 1)?;
            bits += Bits::whole(high as u32);
        }
        Ok(bits)
    }

    fn level_rate(
        &mut self,
        tx: &TransformBlock,
        coded: &CoeffLevels,
        (row, col, c): (usize, usize, usize),
        level: i32,
        eob: usize,
        hr_level_avg: &mut u32,
    ) -> Result<Bits> {
        let plane = tx.plane;
        let quant = level.unsigned_abs();
        if quant >= MAX_QUANT_MAGNITUDE {
            return Ok(Bits::IMPOSSIBLE);
        }
        let is_lf = is_low_frequency(row, col, plane);
        let max = max_level(is_lf, plane);
        let clamped = quant.min(max);
        let base_levels = if is_lf {
            LF_NUM_BASE_LEVELS
        } else {
            NUM_BASE_LEVELS
        };
        let base = clamped.min(base_levels + 1);
        let mut bits = if c + 1 == eob {
            let Some(symbol) = (base as usize).checked_sub(1) else {
                return Ok(Bits::IMPOSSIBLE);
            };
            let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
            let cdf = self.syntax.coeff_base_eob_cdf(
                plane,
                tx.tx_size,
                c,
                (log2_w, 1 << log2_h),
                is_lf,
            )?;
            charge(&mut self.scratch, cdf, symbol)?
        } else {
            let cdf = self
                .syntax
                .coeff_base_cdf(coded, plane, tx.tx_size, (row, col, c), is_lf)?;
            charge(&mut self.scratch, cdf, base as usize)?
        };
        if base > base_levels && !(is_lf && plane > 0) {
            let cdf = self.syntax.coeff_br_cdf(coded, plane, (row, col), is_lf)?;
            bits += charge(&mut self.scratch, cdf, (clamped - base) as usize)?;
        }
        if quant != 0 {
            bits += if row == 0 && col == 0 && plane == 0 {
                let cdf = self.syntax.dc_sign_cdf(tx)?;
                charge(&mut self.scratch, cdf, usize::from(level < 0))?
            } else {
                Bits::whole(1)
            };
        }
        if quant >= max {
            let (golomb, next) = golomb_rate(quant - max, *hr_level_avg);
            bits += golomb;
            *hr_level_avg = next;
        }
        Ok(bits)
    }
}
