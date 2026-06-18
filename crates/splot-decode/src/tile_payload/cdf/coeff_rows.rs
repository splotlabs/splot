// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient base/base-EOB/base-range CDF rows.
//!
//! Feature tracking: `DECODE-COEFF-BASE-CDF-ROWS` and
//! `DECODE-COEFF-BASE-PH-CDF-ROW`.

use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF, DEFAULT_COEFF_BASE_EOB_UV_CDF,
    DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
    DEFAULT_COEFF_BASE_LF_UV_CDF, DEFAULT_COEFF_BASE_PH_CDF, DEFAULT_COEFF_BASE_UV_CDF,
    DEFAULT_COEFF_BR_CDF, DEFAULT_COEFF_BR_LF_CDF, DEFAULT_COEFF_BR_UV_CDF,
};

use super::{TileCdfArray, TileCdfError, avg_cdf_row, scale_cdf_count};

const COEFF_CDF_Q_CONTEXTS: usize = 4;
const TX_SIZE_CONTEXTS: usize = 5;
const COEFF_BASE_CONTEXTS: usize = 20;
const COEFF_BASE_PH_CONTEXTS: usize = 5;
const COEFF_BASE_TCQ_CONTEXTS: usize = 2;
const COEFF_BASE_UV_CONTEXTS: usize = 12;
const COEFF_BASE_EOB_CONTEXTS: usize = 4;
const COEFF_BASE_LF_CONTEXTS: usize = 33;
const COEFF_BASE_LF_UV_CONTEXTS: usize = 12;
const COEFF_BR_CONTEXTS: usize = 7;
const COEFF_BR_LF_CONTEXTS: usize = 14;
const COEFF_BR_UV_CONTEXTS: usize = 4;
const COEFF_BASE_ROW_LEN: usize = 5;
const COEFF_BASE_EOB_ROW_LEN: usize = 4;
const COEFF_BASE_LF_ROW_LEN: usize = 7;
const COEFF_BASE_LF_EOB_ROW_LEN: usize = 6;
const COEFF_BR_ROW_LEN: usize = 5;

pub(crate) type CoeffBaseCdfRows = [[[[[i32; COEFF_BASE_ROW_LEN]; COEFF_BASE_TCQ_CONTEXTS];
    COEFF_BASE_CONTEXTS]; TX_SIZE_CONTEXTS];
    COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBasePhCdfRows =
    [[[i32; COEFF_BASE_ROW_LEN]; COEFF_BASE_PH_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseUvCdfRows =
    [[[i32; COEFF_BASE_ROW_LEN]; COEFF_BASE_UV_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseLfCdfRows = [[[[[i32; COEFF_BASE_LF_ROW_LEN]; COEFF_BASE_TCQ_CONTEXTS];
    COEFF_BASE_LF_CONTEXTS]; TX_SIZE_CONTEXTS];
    COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseLfUvCdfRows =
    [[[i32; COEFF_BASE_LF_ROW_LEN]; COEFF_BASE_LF_UV_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseEobCdfRows = [[[[i32; COEFF_BASE_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS];
    TX_SIZE_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseEobUvCdfRows =
    [[[i32; COEFF_BASE_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseLfEobCdfRows = [[[[i32; COEFF_BASE_LF_EOB_ROW_LEN];
    COEFF_BASE_EOB_CONTEXTS]; TX_SIZE_CONTEXTS];
    COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseLfEobUvCdfRows =
    [[[i32; COEFF_BASE_LF_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBrCdfRows =
    [[[i32; COEFF_BR_ROW_LEN]; COEFF_BR_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBrUvCdfRows =
    [[[i32; COEFF_BR_ROW_LEN]; COEFF_BR_UV_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBrLfCdfRows =
    [[[i32; COEFF_BR_ROW_LEN]; COEFF_BR_LF_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];

/// Ordinary non-IDTX coefficient CDF selector for AV2 §8.3.2 row selection
/// over AV2 §9.3 default CDF banks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffCdfSelector {
    /// `TileCoeffBaseCdf[coeff_cdf_q_ctx][tx_size][ctx][tcq_ctx]`.
    Base {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Significant-coefficient context.
        ctx: usize,
        /// `(tcqState >> 1) & 1` context.
        tcq_ctx: usize,
    },
    /// `TileCoeffBasePhCdf[coeff_cdf_q_ctx][ctx]`.
    BasePh {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Parity-hidden significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseUvCdf[coeff_cdf_q_ctx][ctx]`.
    BaseUv {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Chroma significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseLfCdf[coeff_cdf_q_ctx][tx_size][ctx][tcq_ctx]`.
    BaseLf {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Low-frequency significant-coefficient context.
        ctx: usize,
        /// `(tcqState >> 1) & 1` context.
        tcq_ctx: usize,
    },
    /// `TileCoeffBaseLfUvCdf[coeff_cdf_q_ctx][ctx]`.
    BaseLfUv {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Chroma low-frequency significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`.
    BaseEob {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Transform-size context.
        tx_size: usize,
        /// EOB significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseEobUvCdf[coeff_cdf_q_ctx][ctx]`.
    BaseEobUv {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Chroma EOB significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseLfEobCdf[coeff_cdf_q_ctx][tx_size][ctx]`.
    BaseLfEob {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Low-frequency EOB significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBaseLfEobUvCdf[coeff_cdf_q_ctx][ctx]`.
    BaseLfEobUv {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Chroma low-frequency EOB significant-coefficient context.
        ctx: usize,
    },
    /// `TileCoeffBrCdf[coeff_cdf_q_ctx][ctx]`.
    Br {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Base-range context.
        ctx: usize,
    },
    /// `TileCoeffBrUvCdf[coeff_cdf_q_ctx][ctx]`.
    BrUv {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Chroma base-range context.
        ctx: usize,
    },
    /// `TileCoeffBrLfCdf[coeff_cdf_q_ctx][ctx]`.
    BrLf {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Low-frequency base-range context.
        ctx: usize,
    },
}

/// Supported ordinary non-IDTX coefficient CDF rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoeffCdfRows {
    pub(super) coeff_base: CoeffBaseCdfRows,
    pub(super) coeff_base_ph: CoeffBasePhCdfRows,
    pub(super) coeff_base_uv: CoeffBaseUvCdfRows,
    pub(super) coeff_base_lf: CoeffBaseLfCdfRows,
    pub(super) coeff_base_lf_uv: CoeffBaseLfUvCdfRows,
    pub(super) coeff_base_eob: CoeffBaseEobCdfRows,
    pub(super) coeff_base_eob_uv: CoeffBaseEobUvCdfRows,
    pub(super) coeff_base_lf_eob: CoeffBaseLfEobCdfRows,
    pub(super) coeff_base_lf_eob_uv: CoeffBaseLfEobUvCdfRows,
    pub(super) coeff_br: CoeffBrCdfRows,
    pub(super) coeff_br_uv: CoeffBrUvCdfRows,
    pub(super) coeff_br_lf: CoeffBrLfCdfRows,
}

impl CoeffCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            coeff_base: DEFAULT_COEFF_BASE_CDF,
            coeff_base_ph: DEFAULT_COEFF_BASE_PH_CDF,
            coeff_base_uv: DEFAULT_COEFF_BASE_UV_CDF,
            coeff_base_lf: DEFAULT_COEFF_BASE_LF_CDF,
            coeff_base_lf_uv: DEFAULT_COEFF_BASE_LF_UV_CDF,
            coeff_base_eob: DEFAULT_COEFF_BASE_EOB_CDF,
            coeff_base_eob_uv: DEFAULT_COEFF_BASE_EOB_UV_CDF,
            coeff_base_lf_eob: DEFAULT_COEFF_BASE_LF_EOB_CDF,
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
            coeff_br: DEFAULT_COEFF_BR_CDF,
            coeff_br_uv: DEFAULT_COEFF_BR_UV_CDF,
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF,
        }
    }

    pub(crate) fn row(&self, selector: CoeffCdfSelector) -> Result<&[i32], TileCdfError> {
        match selector {
            CoeffCdfSelector::Base {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
                tcq_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBase, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBase, tx_size)?;
                let ctx = checked_index(TileCdfArray::CoeffBase, "ctx", ctx, COEFF_BASE_CONTEXTS)?;
                let tcq_ctx = checked_index(
                    TileCdfArray::CoeffBase,
                    "tcq_ctx",
                    tcq_ctx,
                    COEFF_BASE_TCQ_CONTEXTS,
                )?;
                Ok(self.coeff_base[q][tx_size][ctx][tcq_ctx].as_slice())
            }
            CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBasePh, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBasePh,
                    "ctx",
                    ctx,
                    COEFF_BASE_PH_CONTEXTS,
                )?;
                Ok(self.coeff_base_ph[q][ctx].as_slice())
            }
            CoeffCdfSelector::BaseUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_UV_CONTEXTS,
                )?;
                Ok(self.coeff_base_uv[q][ctx].as_slice())
            }
            CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
                tcq_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLf, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseLf, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLf,
                    "ctx",
                    ctx,
                    COEFF_BASE_LF_CONTEXTS,
                )?;
                let tcq_ctx = checked_index(
                    TileCdfArray::CoeffBaseLf,
                    "tcq_ctx",
                    tcq_ctx,
                    COEFF_BASE_TCQ_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf[q][tx_size][ctx][tcq_ctx].as_slice())
            }
            CoeffCdfSelector::BaseLfUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_LF_UV_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_uv[q][ctx].as_slice())
            }
            CoeffCdfSelector::BaseEob {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseEob, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseEob, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob[q][tx_size][ctx].as_slice())
            }
            CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseEobUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob_uv[q][ctx].as_slice())
            }
            CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfEob, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseLfEob, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob[q][tx_size][ctx].as_slice())
            }
            CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q =
                    checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfEobUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob_uv[q][ctx].as_slice())
            }
            CoeffCdfSelector::Br {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBr, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBr, "ctx", ctx, COEFF_BR_CONTEXTS)?;
                Ok(self.coeff_br[q][ctx].as_slice())
            }
            CoeffCdfSelector::BrUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrUv, "ctx", ctx, COEFF_BR_UV_CONTEXTS)?;
                Ok(self.coeff_br_uv[q][ctx].as_slice())
            }
            CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrLf, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrLf, "ctx", ctx, COEFF_BR_LF_CONTEXTS)?;
                Ok(self.coeff_br_lf[q][ctx].as_slice())
            }
        }
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: CoeffCdfSelector,
    ) -> Result<&mut [i32], TileCdfError> {
        match selector {
            CoeffCdfSelector::Base {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
                tcq_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBase, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBase, tx_size)?;
                let ctx = checked_index(TileCdfArray::CoeffBase, "ctx", ctx, COEFF_BASE_CONTEXTS)?;
                let tcq_ctx = checked_index(
                    TileCdfArray::CoeffBase,
                    "tcq_ctx",
                    tcq_ctx,
                    COEFF_BASE_TCQ_CONTEXTS,
                )?;
                Ok(self.coeff_base[q][tx_size][ctx][tcq_ctx].as_mut_slice())
            }
            CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBasePh, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBasePh,
                    "ctx",
                    ctx,
                    COEFF_BASE_PH_CONTEXTS,
                )?;
                Ok(self.coeff_base_ph[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_UV_CONTEXTS,
                )?;
                Ok(self.coeff_base_uv[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
                tcq_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLf, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseLf, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLf,
                    "ctx",
                    ctx,
                    COEFF_BASE_LF_CONTEXTS,
                )?;
                let tcq_ctx = checked_index(
                    TileCdfArray::CoeffBaseLf,
                    "tcq_ctx",
                    tcq_ctx,
                    COEFF_BASE_TCQ_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf[q][tx_size][ctx][tcq_ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLfUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_LF_UV_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_uv[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseEob {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseEob, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseEob, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob[q][tx_size][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseEobUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob_uv[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx,
                tx_size,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfEob, coeff_cdf_q_ctx)?;
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseLfEob, tx_size)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob[q][tx_size][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q =
                    checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseLfEobUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseLfEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob_uv[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::Br {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBr, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBr, "ctx", ctx, COEFF_BR_CONTEXTS)?;
                Ok(self.coeff_br[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BrUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrUv, "ctx", ctx, COEFF_BR_UV_CONTEXTS)?;
                Ok(self.coeff_br_uv[q][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrLf, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrLf, "ctx", ctx, COEFF_BR_LF_CONTEXTS)?;
                Ok(self.coeff_br_lf[q][ctx].as_mut_slice())
            }
        }
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        avg_rows(
            self.coeff_base.iter_mut().flatten().flatten().flatten(),
            tile.coeff_base.iter().flatten().flatten().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_ph.iter_mut().flatten(),
            tile.coeff_base_ph.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_uv.iter_mut().flatten(),
            tile.coeff_base_uv.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_lf.iter_mut().flatten().flatten().flatten(),
            tile.coeff_base_lf.iter().flatten().flatten().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_lf_uv.iter_mut().flatten(),
            tile.coeff_base_lf_uv.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_eob.iter_mut().flatten().flatten(),
            tile.coeff_base_eob.iter().flatten().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_eob_uv.iter_mut().flatten(),
            tile.coeff_base_eob_uv.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_lf_eob.iter_mut().flatten().flatten(),
            tile.coeff_base_lf_eob.iter().flatten().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_base_lf_eob_uv.iter_mut().flatten(),
            tile.coeff_base_lf_eob_uv.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_br.iter_mut().flatten(),
            tile.coeff_br.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_br_uv.iter_mut().flatten(),
            tile.coeff_br_uv.iter().flatten(),
            tile_num,
            num_log2,
        );
        avg_rows(
            self.coeff_br_lf.iter_mut().flatten(),
            tile.coeff_br_lf.iter().flatten(),
            tile_num,
            num_log2,
        );
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        scale_rows(self.coeff_base.iter_mut().flatten().flatten().flatten());
        scale_rows(self.coeff_base_ph.iter_mut().flatten());
        scale_rows(self.coeff_base_uv.iter_mut().flatten());
        scale_rows(self.coeff_base_lf.iter_mut().flatten().flatten().flatten());
        scale_rows(self.coeff_base_lf_uv.iter_mut().flatten());
        scale_rows(self.coeff_base_eob.iter_mut().flatten().flatten());
        scale_rows(self.coeff_base_eob_uv.iter_mut().flatten());
        scale_rows(self.coeff_base_lf_eob.iter_mut().flatten().flatten());
        scale_rows(self.coeff_base_lf_eob_uv.iter_mut().flatten());
        scale_rows(self.coeff_br.iter_mut().flatten());
        scale_rows(self.coeff_br_uv.iter_mut().flatten());
        scale_rows(self.coeff_br_lf.iter_mut().flatten());
    }
}

fn avg_rows<'a, 'b, const N: usize>(
    frame: impl Iterator<Item = &'a mut [i32; N]>,
    tile: impl Iterator<Item = &'b [i32; N]>,
    tile_num: u32,
    num_log2: u8,
) {
    for (frame_row, tile_row) in frame.zip(tile) {
        avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
    }
}

fn scale_rows<'a, const N: usize>(rows: impl Iterator<Item = &'a mut [i32; N]>) {
    for row in rows {
        scale_cdf_count(row);
    }
}

fn checked_coeff_cdf_q_context(
    array: TileCdfArray,
    coeff_cdf_q_ctx: usize,
) -> Result<usize, TileCdfError> {
    if coeff_cdf_q_ctx >= COEFF_CDF_Q_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "coeff_cdf_q_ctx",
            actual: coeff_cdf_q_ctx,
            max_exclusive: COEFF_CDF_Q_CONTEXTS,
        });
    }
    Ok(coeff_cdf_q_ctx)
}

fn checked_tx_size(array: TileCdfArray, tx_size: usize) -> Result<usize, TileCdfError> {
    if tx_size >= TX_SIZE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "tx_size",
            actual: tx_size,
            max_exclusive: TX_SIZE_CONTEXTS,
        });
    }
    Ok(tx_size)
}

fn checked_index(
    array: TileCdfArray,
    index_name: &'static str,
    actual: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    if actual >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name,
            actual,
            max_exclusive,
        });
    }
    Ok(actual)
}
