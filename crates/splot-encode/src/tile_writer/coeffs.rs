// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.27 `coeffs()` writer for 2D-class transform blocks without TCQ,
//! parity hiding, or forward skip coding.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_decode::encoder_syntax::{
    CoeffLevels, IntraTileSyntax, TransformBlock, is_low_frequency, tx_log2,
};
use splot_recon::{TransformClass, coefficient_scan_slice};

use super::{Result, TileWriterError};

/// `1 << 20`: the § 6.19.7.23 bound on a coded coefficient magnitude.
const MAX_QUANT_MAGNITUDE: u32 = 1 << 20;
/// The `eob_pt_256`/`512`/`1024` symbol that carries an `eob_pt_*_extra` literal.
const LAST_PLAIN_EOB_PT_SYMBOL: usize = 7;
/// The longest `golomb_length_bit` prefix a magnitude below [`MAX_QUANT_MAGNITUDE`] needs.
const MAX_GOLOMB_PREFIX_BITS: u32 = 21;

/// The `Adjusted_Tx_Size` `(log2 width, log2 height)`: each side capped at 32.
pub(crate) fn adjusted_log2(tx_size: usize) -> Result<(u32, u32)> {
    Ok(tx_log2(tx_size).map(|(w, h)| (w.min(5), h.min(5)))?)
}

/// Writes the coefficients of `tx` and records its contexts; returns `eob`.
pub(super) fn write_coeffs(
    syntax: &mut IntraTileSyntax,
    symbols: &mut impl SymbolSink,
    tx: &TransformBlock,
    levels: &[i32],
    eob_u_nonzero: bool,
) -> Result<usize> {
    let coded = code_coeffs(syntax, symbols, tx, levels, eob_u_nonzero)?;
    syntax.record_coefficients(tx, coded.cul_level, coded.dc_category)?;
    Ok(coded.eob)
}

/// What the § 5.20.7.27 walk leaves for the neighbour contexts.
pub(crate) struct CodedCoeffs {
    pub(crate) eob: usize,
    cul_level: u8,
    dc_category: u8,
}

/// Sends the coefficient syntax of `tx` with row-major adjusted `levels` to
/// `sink`, without recording contexts.
pub(crate) fn code_coeffs(
    syntax: &mut IntraTileSyntax,
    symbols: &mut impl SymbolSink,
    tx: &TransformBlock,
    levels: &[i32],
    eob_u_nonzero: bool,
) -> Result<CodedCoeffs> {
    let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
    let (width, height) = (1usize << log2_w, 1usize << log2_h);
    if levels.len() != width * height {
        return Err(TileWriterError::LevelCount {
            expected: width * height,
            actual: levels.len(),
        });
    }
    if let Some(&bad) = levels
        .iter()
        .find(|q| q.unsigned_abs() >= MAX_QUANT_MAGNITUDE)
    {
        return Err(TileWriterError::LevelOutOfRange(bad));
    }
    let scan = coefficient_scan_slice(width, height, TransformClass::TwoD)?;
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);
    write_symbol(
        symbols,
        syntax.txb_skip_cdf(tx, eob_u_nonzero)?,
        usize::from(eob == 0),
    )?;
    if eob == 0 {
        return Ok(CodedCoeffs {
            eob,
            cul_level: 0,
            dc_category: 0,
        });
    }
    write_eob(syntax, symbols, tx, eob)?;

    let plane = tx.plane;
    let position = |c: usize| {
        let pos = usize::from(scan[c]);
        (pos, pos >> log2_w, pos & (width - 1))
    };
    let mut grid = CoeffLevels::new(width, height)?;
    for c in (0..eob).rev() {
        let (pos, row, col) = position(c);
        let is_lf = is_low_frequency(row, col, plane);
        let level = levels[pos].unsigned_abs().min(max_level(is_lf, plane));
        let base_levels = if is_lf {
            LF_NUM_BASE_LEVELS
        } else {
            NUM_BASE_LEVELS
        };
        let base = if c == eob - 1 {
            let base = level.min(base_levels + 1);
            let cdf = syntax.coeff_base_eob_cdf(plane, tx.tx_size, c, (log2_w, height), is_lf)?;
            write_symbol(symbols, cdf, base as usize - 1)?;
            base
        } else {
            let base = level.min(base_levels + 1);
            let cdf = syntax.coeff_base_cdf(&grid, plane, tx.tx_size, (row, col, c), is_lf)?;
            write_symbol(symbols, cdf, base as usize)?;
            base
        };
        if base > base_levels && !(is_lf && plane > 0) {
            let cdf = syntax.coeff_br_cdf(&grid, plane, (row, col), is_lf)?;
            write_symbol(symbols, cdf, (level - base) as usize)?;
        }
        grid.set(row, col, level)?;
    }

    let mut cul_level = 0u32;
    let mut dc_category = 0u8;
    let mut hr_level_avg = 0u32;
    for c in (0..eob).rev() {
        let (pos, row, col) = position(c);
        let quant = levels[pos].unsigned_abs();
        if quant != 0 {
            let negative = u8::from(levels[pos] < 0);
            if pos == 0 && plane == 0 {
                write_symbol(symbols, syntax.dc_sign_cdf(tx)?, usize::from(negative))?;
            } else {
                symbols.literal(u32::from(negative), 1)?;
            }
        }
        let max = max_level(is_low_frequency(row, col, plane), plane);
        if quant >= max {
            hr_level_avg = write_golomb(symbols, quant - max, hr_level_avg)?;
        }
        if pos == 0 && quant > 0 {
            dc_category = if levels[pos] < 0 { 1 } else { 2 };
        }
        cul_level = (cul_level + quant).min(4);
    }
    Ok(CodedCoeffs {
        eob,
        cul_level: cul_level as u8,
        dc_category,
    })
}

/// `maxLevel` of § 5.20.7.27 without parity hiding.
pub(crate) const fn max_level(is_lf: bool, plane: usize) -> u32 {
    match (is_lf, plane == 0) {
        (true, true) => LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
        (true, false) => LF_NUM_BASE_LEVELS + 1,
        (false, _) => NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
    }
}

/// Writes `eob_pt_*`, `eob_pt_*_extra`, `eob_extra`, and `eob_extra_bit` for `eob >= 1`.
pub(crate) fn write_eob(
    syntax: &mut IntraTileSyntax,
    symbols: &mut impl SymbolSink,
    tx: &TransformBlock,
    eob: usize,
) -> Result<()> {
    let eob_pt = if eob <= 2 {
        eob
    } else {
        (eob - 1).ilog2() as usize + 2
    };
    let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
    let eob_multisize = log2_w + log2_h - 4;
    let symbol = (eob_pt - 1).min(LAST_PLAIN_EOB_PT_SYMBOL);
    write_symbol(symbols, syntax.eob_pt_cdf(tx.plane, tx.tx_size)?, symbol)?;
    if eob_multisize >= 4 && symbol == LAST_PLAIN_EOB_PT_SYMBOL {
        let extra_bits = if eob_multisize == 4 { 1 } else { 2 };
        symbols.literal((eob_pt - 8) as u32, extra_bits)?;
    }
    if eob_pt >= 3 {
        let offset = eob - ((1 << (eob_pt - 2)) + 1);
        let high = eob_pt - 3;
        write_symbol(symbols, syntax.eob_extra_cdf()?, (offset >> high) & 1)?;
        let low = offset & ((1 << high) - 1);
        symbols.literal(low as u32, high as u32)?;
    }
    Ok(())
}

/// Writes the § 5.20.7.28 remainder `x` and returns the next `hrLevelAvg`.
fn write_golomb(symbols: &mut impl SymbolSink, x: u32, hr_level_avg: u32) -> Result<u32> {
    let m = hr_level_avg.checked_ilog2().unwrap_or(0).clamp(1, 6);
    let k = m + 1;
    let c_max = (m + 4).min(6);
    let q = x >> m;
    if q < c_max {
        symbols.unary(q, c_max)?;
        symbols.literal(x & ((1 << m) - 1), m)?;
    } else {
        symbols.unary(c_max, c_max)?;
        let shifted = x - (c_max << m) + (1 << k);
        let length = shifted.ilog2();
        symbols.unary(length - k, MAX_GOLOMB_PREFIX_BITS)?;
        symbols.literal(shifted - (1 << length), length)?;
    }
    Ok((x + hr_level_avg) >> 1)
}

pub(crate) fn write_symbol(
    symbols: &mut impl SymbolSink,
    cdf: &mut [u16],
    value: usize,
) -> Result<()> {
    let value = u8::try_from(value).map_err(|_| TileWriterError::Unsupported("symbol value"))?;
    symbols.symbol(cdf, Symbol::new(value))?;
    Ok(())
}
