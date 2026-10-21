// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Writer-state snapshots, so a search can code a trial and roll it back.

use splot_core::symbol_encoder::SymbolEncoder;
use splot_decode::encoder_syntax::{IntraTileSyntax, SyntaxSnapshot};

use crate::tile_writer::Result;

/// The facade context, the reconstruction of one luma rectangle, and the
/// symbol encoder, as they were when saved.
pub(crate) struct Checkpoint {
    syntax: SyntaxSnapshot,
    symbols: SymbolEncoder,
}

impl Checkpoint {
    /// Saves the state; `luma` is `(x, y, width, height)` in luma samples and is
    /// clipped to plane storage. A trial must not write recon outside it.
    ///
    /// # Errors
    /// Returns a facade error for a rectangle outside the frame.
    pub(crate) fn save(
        syntax: &IntraTileSyntax,
        symbols: &SymbolEncoder,
        luma: (usize, usize, usize, usize),
    ) -> Result<Self> {
        Ok(Self {
            syntax: syntax.snapshot(luma)?,
            symbols: symbols.clone(),
        })
    }

    /// Puts the saved state back; the checkpoint stays usable.
    ///
    /// # Errors
    /// Returns a facade error if the saved samples no longer fit.
    pub(crate) fn restore(
        &self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut SymbolEncoder,
    ) -> Result<()> {
        syntax.restore(&self.syntax)?;
        symbols.clone_from(&self.symbols);
        Ok(())
    }
}
