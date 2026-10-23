// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 5.20.7.27 parity hiding of a transform block, derived by the
//! decoder's own first coefficient pass.

use super::super::coeff_loop::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseFirstPassSummary,
};
use super::super::coeff_loop::max_level::{
    CoeffTransformClass, coeff_is_low_frequency, visible_max_level,
};
use super::super::coeff_loop::scan_walk::CoeffScanEntry;
use super::{IntraTileSyntax, Result, TransformBlock, TxType, rule, tx_log2};

/// A luma block that may hide the parity of its scan index 0 level
/// (`parityHiding` of § 5.20.7.27).
#[derive(Clone, Copy, Debug)]
pub struct ParityHiding {
    tx: TransformBlock,
}

impl IntraTileSyntax {
    /// The parity hiding of `tx`, or `None` when the frame does not allow it
    /// (`allow_parity_hiding`), for chroma, and for `IDTX`.
    #[must_use]
    pub fn parity_hiding(&self, tx: &TransformBlock) -> Option<ParityHiding> {
        (self.tools.parity_hiding && tx.plane == 0 && tx.tx_type != TxType::Idtx)
            .then_some(ParityHiding { tx: *tx })
    }
}

impl ParityHiding {
    /// `Some(sumAbs1)` when the row-major adjusted `levels` make the block
    /// hide the parity of its scan index 0 level (`isHidden`): that level's
    /// magnitude must then have the parity `sumAbs1`, and the block codes
    /// half of it. `None` when the block hides nothing.
    ///
    /// # Errors
    /// Returns [`super::EncoderSyntaxError`] for an invalid transform size or
    /// a short `levels`.
    pub fn sum_abs1(&self, levels: &[i32]) -> Result<Option<u32>> {
        const SHORT: super::EncoderSyntaxError =
            super::EncoderSyntaxError::Unsupported("levels shorter than the block");
        let (log2_w, log2_h) = tx_log2(self.tx.tx_size)?;
        let (log2_w, log2_h) = (log2_w.min(5), log2_h.min(5));
        let scan = super::coefficient_scan(&self.tx)?;
        let tx_class = CoeffTransformClass::from_plane_tx_type(self.tx.tx_type as usize);
        let config = CoeffBaseDerivedLevelPassConfig {
            tx_size_ctx: 0,
            tx_width_log2: log2_w,
            tx_width: 1 << log2_w,
            tx_height: 1 << log2_h,
            plane: 0,
            tx_class,
            parity_hiding: true,
            use_tcq: false,
        };
        let first = scan.first().map_or(0, |&pos| usize::from(pos));
        let mut summary = CoeffBaseFirstPassSummary::default();
        // The summary is a parity and a count over the nonzero levels after scan index 0,
        // so a raster walk gives the decoder's result.
        for (pos, &level) in levels.get(..scan.len()).ok_or(SHORT)?.iter().enumerate() {
            if level == 0 || pos == first {
                continue;
            }
            let entry = CoeffScanEntry::new(1, pos, pos >> log2_w, pos & ((1 << log2_w) - 1));
            let max = visible_max_level(coeff_is_low_frequency(entry, 0, tx_class), 0);
            summary
                .update_after_level(entry, level.unsigned_abs().min(max), config)
                .map_err(rule("parity hiding"))?;
        }
        Ok(summary.is_hidden().then_some(summary.sum_abs1()))
    }

    /// Whether `levels` keep the rule: they hide no parity, or the one their
    /// scan index 0 level has.
    ///
    /// # Errors
    /// As [`Self::sum_abs1`].
    pub fn holds(&self, levels: &[i32]) -> Result<bool> {
        let first = super::coefficient_scan(&self.tx)?
            .first()
            .copied()
            .unwrap_or(0);
        Ok(self.sum_abs1(levels)?.is_none_or(|sum| {
            levels
                .get(usize::from(first))
                .is_some_and(|level| level.unsigned_abs() & 1 == sum)
        }))
    }
}
