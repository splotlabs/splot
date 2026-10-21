// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Writer-state snapshots, so a search can code a trial and roll it back.

use splot_core::symbol_encoder::SymbolEncoder;
use splot_decode::encoder_syntax::IntraTileSyntax;
use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect};

use crate::tile_writer::Result;

/// The facade, the symbol encoder, and the 4:2:0 reconstruction of one luma
/// rectangle, as they were when saved.
pub(crate) struct Checkpoint {
    syntax: IntraTileSyntax,
    symbols: SymbolEncoder,
    /// Saved samples of each plane's part of the rectangle, row-major.
    recon: Vec<(PlaneId, PlaneRect, Vec<u16>)>,
}

impl Checkpoint {
    /// Saves the state; `luma` is `(x, y, width, height)` in luma samples and is
    /// clipped to plane storage. A trial must not write recon outside it.
    ///
    /// # Errors
    /// Returns an error when `recon` lacks a plane.
    pub(crate) fn save(
        syntax: &IntraTileSyntax,
        symbols: &SymbolEncoder,
        recon: &CurrentFrameWorkspace<u16>,
        luma: (usize, usize, usize, usize),
    ) -> Result<Self> {
        let (x, y, w, h) = luma;
        let mut saved = Vec::with_capacity(3);
        for (plane, shift) in [(PlaneId::Y, 0), (PlaneId::U, 1), (PlaneId::V, 1)] {
            let size = recon.plane(plane)?.storage_size();
            let (px, py) = (x >> shift, y >> shift);
            let pw = ((x + w).div_ceil(1 << shift))
                .min(size.width())
                .saturating_sub(px);
            let ph = ((y + h).div_ceil(1 << shift))
                .min(size.height())
                .saturating_sub(py);
            if pw == 0 || ph == 0 {
                continue;
            }
            let rect = PlaneRect::new(px, py, pw, ph)?;
            let mut samples = Vec::with_capacity(pw * ph);
            for row in recon.rect_rows(plane, rect)? {
                samples.extend_from_slice(row);
            }
            saved.push((plane, rect, samples));
        }
        Ok(Self {
            syntax: syntax.clone(),
            symbols: symbols.clone(),
            recon: saved,
        })
    }

    /// Puts the saved state back; the checkpoint stays usable.
    ///
    /// # Errors
    /// Returns an error when `recon` lacks a saved plane.
    pub(crate) fn restore(
        &self,
        syntax: &mut IntraTileSyntax,
        symbols: &mut SymbolEncoder,
        recon: &mut CurrentFrameWorkspace<u16>,
    ) -> Result<()> {
        syntax.clone_from(&self.syntax);
        symbols.clone_from(&self.symbols);
        for (plane, rect, samples) in &self.recon {
            recon.write_rect(*plane, *rect, samples, rect.width())?;
        }
        Ok(())
    }
}
