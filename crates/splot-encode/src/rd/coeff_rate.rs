// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rates of the § 5.20.7.27 `coeffs()` syntax, priced by running the writer's
//! own walk ([`code_coeffs`]) into a [`RateSink`].

use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::{SymbolSink, adapt_cdf_u16};
use splot_core::write::WriteResult;
use splot_decode::encoder_syntax::{IntraTileSyntax, TransformBlock};

use super::{Bits, symbol_cost, unary_cost};
use crate::tile_writer::coeffs::code_coeffs;
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
    match code_coeffs(syntax, sink, tx, levels, eob_u_nonzero) {
        Ok(_) => Ok(Bits(sink.bits.0 - start.0)),
        Err(TileWriterError::LevelOutOfRange(_)) => Ok(Bits::IMPOSSIBLE),
        Err(error) => Err(error),
    }
}
