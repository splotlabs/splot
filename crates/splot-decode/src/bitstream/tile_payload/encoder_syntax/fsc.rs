// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Forward skip coding (FSC, § 5.20.5.5, § 5.20.7.27) of the encoder facade:
//! the IDTX luma blocks whose levels are coded from the first nonzero scan
//! position with the `idtx` rows.

use splot_core::coefficient::{COEFF_BASE_RANGE, NUM_BASE_LEVELS};
use splot_core::tables::conversion::{TX_SIZE_SQR, TX_SIZE_SQR_UP};

use super::super::cdf::CoeffCdfSelector;
use super::super::cdf::coeff_context::{
    coeff_base_bob_ctx, coeff_base_idtx_ctx, coeff_br_idtx_ctx, idtx_sign_ctx,
};
use super::rollback::CdfRow;
use super::{
    CoeffLevels, EncoderSyntaxError, IntraTileSyntax, Result, TransformBlock, TxType, rule,
};

/// The largest `Level` an FSC block codes before the remainder (`NUM_BASE_LEVELS
/// + COEFF_BASE_RANGE + 1`, § 5.20.7.27).
pub const FSC_MAX_LEVEL: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1;

impl IntraTileSyntax {
    /// Whether `tx` of the planned leaf codes its levels with FSC (`useFsc`).
    #[must_use]
    pub fn uses_fsc(&self, tx: &TransformBlock) -> bool {
        self.tools.enable_fsc
            && tx.plane == 0
            && tx.tx_type == TxType::Idtx
            && (self.planned_fsc() || self.leaf_is_inter())
    }

    /// The `coeff_base_bob` row of the first coded position `(bob, seg_eob)`,
    /// or the `coeff_base_idtx` row of adjusted `(row, col)` of `tx` coded as
    /// `levels`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn fsc_base_cdf(
        &mut self,
        tx: &TransformBlock,
        levels: &CoeffLevels,
        (row, col): (usize, usize),
        bob: Option<(usize, usize)>,
    ) -> Result<&mut [u16]> {
        let tx_size_ctx = fsc_tx_size_ctx(tx.tx_size)?;
        let selector = match bob {
            Some((bob, seg_eob)) => CoeffCdfSelector::BaseBob {
                tx_size_ctx,
                ctx: coeff_base_bob_ctx(bob, seg_eob),
            },
            None => CoeffCdfSelector::BaseIdtx {
                tx_size_ctx,
                ctx: coeff_base_idtx_ctx(
                    levels.state.level(),
                    row,
                    col,
                    levels.state.level_stride(),
                ),
            },
        };
        self.coeff_row(selector)
    }

    /// The `coeff_br_idtx` row of adjusted `(row, col)` of `tx` coded as `levels`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn fsc_br_cdf(
        &mut self,
        tx: &TransformBlock,
        levels: &CoeffLevels,
        (row, col): (usize, usize),
    ) -> Result<&mut [u16]> {
        let selector = CoeffCdfSelector::BrIdtx {
            tx_size_ctx: fsc_tx_size_ctx(tx.tx_size)?,
            ctx: coeff_br_idtx_ctx(levels.state.level(), row, col, levels.state.level_stride()),
        };
        self.coeff_row(selector)
    }

    /// The `idtx_sign` row of the nonzero level at adjusted `(row, col)` of
    /// `tx` coded as `levels` with the signs set so far.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn idtx_sign_cdf(
        &mut self,
        tx: &TransformBlock,
        levels: &CoeffLevels,
        (row, col): (usize, usize),
    ) -> Result<&mut [u16]> {
        let state = &levels.state;
        let selector = CoeffCdfSelector::IdtxSign {
            tx_size_ctx: fsc_tx_size_ctx(tx.tx_size)?,
            ctx: idtx_sign_ctx(
                state.quant_sign(),
                state.level(),
                row,
                col,
                state.level_stride(),
            ),
        };
        self.coeff_row(selector)
    }

    fn coeff_row(&mut self, selector: CoeffCdfSelector) -> Result<&mut [u16]> {
        let (base, _) = self.cdfs.rows_span();
        let row = self
            .cdfs
            .coeff_rows_mut()
            .row_mut(selector)
            .map_err(rule("FSC coefficient row"))?;
        self.journal.log(base, CdfRow::Coeff(selector), row);
        Ok(row)
    }
}

impl CoeffLevels {
    /// Records the sign of the nonzero level at adjusted `(row, col)` for the
    /// `idtx_sign` contexts of the later levels.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] outside the grid.
    pub fn set_sign(&mut self, row: usize, col: usize, negative: bool) -> Result<()> {
        self.state.ensure_quant_sign().map_err(rule("FSC signs"))?;
        self.state
            .set_quant_sign(row, col, if negative { -1 } else { 1 })
            .map_err(rule("FSC signs"))
    }
}

/// The FSC row group of `tx_size`: `(Tx_Size_Sqr + Tx_Size_Sqr_Up + 1) >> 1`,
/// capped at `TX_16X16`.
fn fsc_tx_size_ctx(tx_size: usize) -> Result<usize> {
    let size = |table: &[i32]| {
        table
            .get(tx_size)
            .and_then(|&v| usize::try_from(v).ok())
            .ok_or(EncoderSyntaxError::Unsupported("transform size"))
    };
    Ok(((size(&TX_SIZE_SQR)? + size(&TX_SIZE_SQR_UP)? + 1) >> 1).min(2))
}
