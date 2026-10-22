// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rates of the § 5.20.7.27 `coeffs()` syntax, priced by running the writer's
//! own walk into a [`RateSink`], and the CDF-backed RDOQ rate model.

use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::{SymbolSink, adapt_cdf_u16};
use splot_core::write::WriteResult;
use splot_decode::encoder_syntax::{IntraTileSyntax, TransformBlock};

use super::{Bits, row_costs, symbol_cost, unary_cost};
use crate::kernels::rdoq::{BASE_SYMBOLS, BIT, BR_SYMBOLS, CoeffRates, LevelContext, RdoqBlock};
use crate::tile_writer::coeffs::{adjusted_log2, write_eob, write_symbol};
use crate::tile_writer::{Result, TileWriterError};

/// A [`SymbolSink`] that sums the rate of the syntax it receives.
#[derive(Debug, Default)]
pub(crate) struct RateSink {
    /// The rate so far.
    pub(crate) bits: Bits,
    /// Adapts each priced row as coding would, so later symbols in the same
    /// block see the adapted rows.
    adapt: bool,
}

impl RateSink {
    /// Prices symbols with the rows as they stand; the rows do not change.
    pub(crate) const fn frozen() -> Self {
        Self {
            bits: Bits(0),
            adapt: false,
        }
    }

    /// Prices each symbol, then adapts its row like the writer. Restore the
    /// facade after a trial.
    pub(crate) const fn adapting() -> Self {
        Self {
            bits: Bits(0),
            adapt: true,
        }
    }
}

impl SymbolSink for RateSink {
    fn symbol(&mut self, cdf: &mut [u16], value: Symbol) -> WriteResult<()> {
        self.bits += symbol_cost(cdf, usize::from(value.get()));
        if self.adapt {
            adapt_cdf_u16(cdf, value)?;
        }
        Ok(())
    }

    fn literal(&mut self, _value: u32, bits: u32) -> WriteResult<()> {
        self.bits += Bits::whole(bits);
        Ok(())
    }

    fn unary(&mut self, value: u32, max_bits: u32) -> WriteResult<()> {
        self.bits += unary_cost(value, max_bits);
        Ok(())
    }
}

/// The rate of the coefficient syntax of `tx` with row-major adjusted `levels`;
/// [`Bits::IMPOSSIBLE`] for a level the writer cannot code.
///
/// # Errors
/// Returns [`TileWriterError::LevelCount`] for a wrong level count and facade errors.
pub(crate) fn block_rate(
    syntax: &mut IntraTileSyntax,
    sink: &mut RateSink,
    tx: &TransformBlock,
    levels: &[i32],
    eob_u_nonzero: bool,
) -> Result<Bits> {
    let start = sink.bits;
    match crate::tile_writer::coeffs::code_coeffs(syntax, sink, tx, levels, eob_u_nonzero) {
        Ok(_) => Ok(Bits(sink.bits.0 - start.0)),
        Err(TileWriterError::LevelOutOfRange(_)) => Ok(Bits::IMPOSSIBLE),
        Err(error) => Err(error),
    }
}

/// The largest rate a [`CdfCoeffRates`] query returns, so RDOQ sums of
/// impossible symbols stay inside `u32`.
const MAX_QUERY_RATE: u64 = 1 << 24;

/// [`CoeffRates`] read from the rows the writer will code `tx` with. The rows
/// stay frozen while one block is quantized. Shortcut: V's `all_zero` is priced
/// as if U coded nothing; it becomes exact once RDOQ sees U's `eob`.
pub(crate) struct CdfCoeffRates<'a> {
    pub(crate) syntax: &'a mut IntraTileSyntax,
    pub(crate) tx: &'a TransformBlock,
}

impl CdfCoeffRates<'_> {
    fn priced(row: Result<&mut [u16]>, symbols: usize, first: usize) -> [u32; BASE_SYMBOLS] {
        let mut out = [MAX_QUERY_RATE as u32; BASE_SYMBOLS];
        if let Ok(row) = row {
            for (cost, bits) in out[first..symbols].iter_mut().zip(row_costs(row)) {
                *cost = clamp(bits);
            }
        }
        out
    }
}

fn clamp(bits: Bits) -> u32 {
    bits.0.min(MAX_QUERY_RATE) as u32
}

impl CoeffRates for CdfCoeffRates<'_> {
    fn eob(&mut self, _block: &RdoqBlock, eob: usize) -> u32 {
        let mut sink = RateSink::frozen();
        let mut price = || -> Result<()> {
            let row = self.syntax.txb_skip_cdf(self.tx, false)?;
            write_symbol(&mut sink, row, usize::from(eob == 0))?;
            if eob > 0 {
                write_eob(self.syntax, &mut sink, self.tx, eob)?;
            }
            Ok(())
        };
        let priced = price();
        priced.map_or(MAX_QUERY_RATE as u32, |()| clamp(sink.bits))
    }

    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS] {
        let (plane, tx_size) = (self.tx.plane, self.tx.tx_size);
        if ctx.is_last {
            let shape = adjusted_log2(tx_size).map(|(w, h)| (w, 1usize << h));
            let row = shape.and_then(|shape| {
                Ok(self
                    .syntax
                    .coeff_base_eob_cdf(plane, tx_size, ctx.c, shape, ctx.is_lf)?)
            });
            Self::priced(row, BASE_SYMBOLS, 1)
        } else {
            let row = self
                .syntax
                .coeff_base_cdf(
                    ctx.levels,
                    plane,
                    tx_size,
                    (ctx.row, ctx.col, ctx.c),
                    ctx.is_lf,
                )
                .map_err(TileWriterError::from);
            Self::priced(row, BASE_SYMBOLS, 0)
        }
    }

    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS] {
        let row =
            self.syntax
                .coeff_br_cdf(ctx.levels, self.tx.plane, (ctx.row, ctx.col), ctx.is_lf);
        let mut out = [MAX_QUERY_RATE as u32; BR_SYMBOLS];
        if let Ok(row) = row {
            for (cost, bits) in out.iter_mut().zip(row_costs(row)) {
                *cost = clamp(bits);
            }
        }
        out
    }

    fn dc_sign(&mut self) -> u32 {
        self.syntax.dc_sign_cdf(self.tx).map_or(BIT, |row| {
            clamp(Bits((symbol_cost(row, 0).0 + symbol_cost(row, 1).0) / 2))
        })
    }
}
