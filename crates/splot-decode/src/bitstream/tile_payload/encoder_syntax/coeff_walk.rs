// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 5.20.7.27 level pass of a transform block in one call: the contexts
//! of every scan position come from the decoder's own context functions,
//! inside the decoder crate, so the writer and the rate models share one walk.

use splot_core::coefficient::{LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};

use super::{CoeffLevels, EncoderSyntaxError, IntraTileSyntax, TransformBlock, tcq_next_state};

impl IntraTileSyntax {
    /// Sends the level-pass symbols of `tx` coded as the row-major adjusted
    /// `levels` with `eob >= 1` (`width` adjusted samples per row) to
    /// `symbol`, from scan index `eob - 1` down to 0: each `coeff_base_eob`
    /// or `coeff_base` row and value, then the `coeff_br` row and value when
    /// coded. Each level, and its TCQ state, is stored in `grid` as it is
    /// coded, as the decoder's level pass does.
    ///
    /// # Errors
    /// Returns the errors of `symbol`, [`EncoderSyntaxError::Unsupported`] for
    /// a `width` that is not a power of two, an `eob` past the scan or a zero
    /// level at `eob - 1`, and [`EncoderSyntaxError::Rule`] for a CDF table
    /// defect or a position outside `grid`.
    pub fn code_coeff_levels<E: From<EncoderSyntaxError>>(
        &mut self,
        tx: &TransformBlock,
        grid: &mut CoeffLevels,
        (levels, width, eob): (&[i32], usize, usize),
        mut symbol: impl FnMut(&mut [u16], usize) -> Result<(), E>,
    ) -> Result<(), E> {
        let (plane, tx_size) = (tx.plane, tx.tx_size);
        if !width.is_power_of_two() {
            return Err(EncoderSyntaxError::Unsupported("level row width").into());
        }
        let log2_w = width.trailing_zeros();
        let height = levels.len() / width;
        let scan = grid.scan();
        let scan = scan.get(..eob).ok_or(EncoderSyntaxError::Unsupported(
            "end of block past the scan",
        ))?;
        for (c, &pos) in scan.iter().enumerate().rev() {
            let pos = usize::from(pos);
            let (row, col) = (pos >> log2_w, pos & (width - 1));
            let is_lf = grid.is_low_frequency(row, col, plane);
            let shift = u32::from(grid.hides(c));
            let magnitude = levels.get(pos).map_or(0, |level| level.unsigned_abs());
            let (level, _) = grid.split(c, magnitude >> shift, is_lf, plane);
            let base_levels = if is_lf {
                LF_NUM_BASE_LEVELS
            } else {
                NUM_BASE_LEVELS
            };
            let base = level.min(base_levels + 1);
            if c == eob - 1 {
                let row = self.coeff_base_eob_cdf(plane, tx_size, c, (log2_w, height), is_lf)?;
                let value =
                    (base as usize)
                        .checked_sub(1)
                        .ok_or(EncoderSyntaxError::Unsupported(
                            "zero level at the end of block",
                        ))?;
                symbol(row, value)?;
            } else {
                let row = self.coeff_base_cdf(grid, plane, tx_size, (row, col, c), is_lf)?;
                symbol(row, base as usize)?;
            }
            if base > base_levels && !(is_lf && plane > 0) {
                let row = self.coeff_br_cdf(grid, plane, (row, col), is_lf)?;
                symbol(row, (level - base) as usize)?;
            }
            grid.set(row, col, level)?;
            if let Some(state) = grid.tcq_state() {
                grid.set_tcq_state(tcq_next_state(state, level));
            }
        }
        Ok(())
    }
}
