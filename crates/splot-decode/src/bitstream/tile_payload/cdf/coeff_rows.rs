// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient base/base-EOB/base-range and IDTX CDF rows.
//!
//! Feature tracking: `DECODE-COEFF-BASE-CDF-ROWS` and
//! `DECODE-COEFF-BASE-PH-CDF-ROW`, plus
//! `DECODE-COEFF-IDTX-CDF-ROWS`.

use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_BOB_CDF, DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF,
    DEFAULT_COEFF_BASE_EOB_UV_CDF, DEFAULT_COEFF_BASE_IDTX_CDF, DEFAULT_COEFF_BASE_LF_CDF,
    DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF, DEFAULT_COEFF_BASE_LF_UV_CDF,
    DEFAULT_COEFF_BASE_PH_CDF, DEFAULT_COEFF_BASE_UV_CDF, DEFAULT_COEFF_BR_CDF,
    DEFAULT_COEFF_BR_IDTX_CDF, DEFAULT_COEFF_BR_LF_CDF, DEFAULT_COEFF_BR_UV_CDF,
    DEFAULT_IDTX_SIGN_CDF,
};

use super::{TileCdfArray, TileCdfError, avg_cdf_rows, blend_cdf_rows, scale_cdf_rows};

fn replicate_q_context_rows<T: Copy, const N: usize>(rows: &mut [T; N], q: usize) {
    let row = rows[q];
    rows.fill(row);
}

const COEFF_CDF_Q_CONTEXTS: usize = 4;
const TX_SIZE_CONTEXTS: usize = 5;
const FSC_TX_SIZE_CONTEXTS: usize = 3;
const COEFF_BASE_CONTEXTS: usize = 20;
const COEFF_BASE_PH_CONTEXTS: usize = 5;
const COEFF_BASE_TCQ_CONTEXTS: usize = 2;
const COEFF_BASE_UV_CONTEXTS: usize = 12;
const COEFF_BASE_EOB_CONTEXTS: usize = 4;
const COEFF_BASE_BOB_CONTEXTS: usize = 3;
const COEFF_BASE_LF_CONTEXTS: usize = 33;
const COEFF_BASE_LF_UV_CONTEXTS: usize = 12;
const IDTX_SIG_COEF_CONTEXTS: usize = 7;
const COEFF_BR_CONTEXTS: usize = 7;
const COEFF_BR_LF_CONTEXTS: usize = 14;
const COEFF_BR_UV_CONTEXTS: usize = 4;
const IDTX_LEVEL_CONTEXTS: usize = 7;
const IDTX_SIGN_CONTEXTS: usize = 9;
const COEFF_BASE_ROW_LEN: usize = 5;
const COEFF_BASE_EOB_ROW_LEN: usize = 4;
const COEFF_BASE_BOB_ROW_LEN: usize = 4;
const COEFF_BASE_LF_ROW_LEN: usize = 7;
const COEFF_BASE_LF_EOB_ROW_LEN: usize = 6;
const COEFF_BR_ROW_LEN: usize = 5;
const IDTX_SIGN_ROW_LEN: usize = 3;

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
pub(crate) type CoeffBaseBobCdfRows = [[[[i32; COEFF_BASE_BOB_ROW_LEN]; COEFF_BASE_BOB_CONTEXTS];
    FSC_TX_SIZE_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type CoeffBaseIdtxCdfRows = [[[[i32; COEFF_BASE_ROW_LEN]; IDTX_SIG_COEF_CONTEXTS];
    FSC_TX_SIZE_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
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
pub(crate) type CoeffBrIdtxCdfRows =
    [[[[i32; COEFF_BR_ROW_LEN]; IDTX_LEVEL_CONTEXTS]; FSC_TX_SIZE_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type IdtxSignCdfRows =
    [[[[i32; IDTX_SIGN_ROW_LEN]; IDTX_SIGN_CONTEXTS]; FSC_TX_SIZE_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffCdfSelector {
    Base {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    BasePh {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BaseUv {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BaseLf {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    BaseLfUv {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BaseEob {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
    },
    BaseEobUv {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BaseBob {
        coeff_cdf_q_ctx: usize,
        tx_size_ctx: usize,
        ctx: usize,
    },
    BaseIdtx {
        coeff_cdf_q_ctx: usize,
        tx_size_ctx: usize,
        ctx: usize,
    },
    BaseLfEob {
        coeff_cdf_q_ctx: usize,
        tx_size: usize,
        ctx: usize,
    },
    BaseLfEobUv {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    Br {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BrUv {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BrLf {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    BrIdtx {
        coeff_cdf_q_ctx: usize,
        tx_size_ctx: usize,
        ctx: usize,
    },
    IdtxSign {
        coeff_cdf_q_ctx: usize,
        tx_size_ctx: usize,
        ctx: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoeffCdfRows {
    pub(crate) coeff_base: CoeffBaseCdfRows,
    pub(crate) coeff_base_ph: CoeffBasePhCdfRows,
    pub(crate) coeff_base_uv: CoeffBaseUvCdfRows,
    pub(crate) coeff_base_lf: CoeffBaseLfCdfRows,
    pub(crate) coeff_base_lf_uv: CoeffBaseLfUvCdfRows,
    pub(crate) coeff_base_eob: CoeffBaseEobCdfRows,
    pub(crate) coeff_base_eob_uv: CoeffBaseEobUvCdfRows,
    pub(crate) coeff_base_bob: CoeffBaseBobCdfRows,
    pub(crate) coeff_base_idtx: CoeffBaseIdtxCdfRows,
    pub(crate) coeff_base_lf_eob: CoeffBaseLfEobCdfRows,
    pub(crate) coeff_base_lf_eob_uv: CoeffBaseLfEobUvCdfRows,
    pub(crate) coeff_br: CoeffBrCdfRows,
    pub(crate) coeff_br_uv: CoeffBrUvCdfRows,
    pub(crate) coeff_br_lf: CoeffBrLfCdfRows,
    pub(crate) coeff_br_idtx: CoeffBrIdtxCdfRows,
    pub(crate) idtx_sign: IdtxSignCdfRows,
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
            coeff_base_bob: DEFAULT_COEFF_BASE_BOB_CDF,
            coeff_base_idtx: DEFAULT_COEFF_BASE_IDTX_CDF,
            coeff_base_lf_eob: DEFAULT_COEFF_BASE_LF_EOB_CDF,
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
            coeff_br: DEFAULT_COEFF_BR_CDF,
            coeff_br_uv: DEFAULT_COEFF_BR_UV_CDF,
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF,
            coeff_br_idtx: DEFAULT_COEFF_BR_IDTX_CDF,
            idtx_sign: DEFAULT_IDTX_SIGN_CDF,
        }
    }

    pub(crate) fn replicate_q_context(
        &mut self,
        coeff_cdf_q_ctx: usize,
    ) -> Result<(), TileCdfError> {
        let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBase, coeff_cdf_q_ctx)?;
        replicate_q_context_rows(&mut self.coeff_base, q);
        replicate_q_context_rows(&mut self.coeff_base_ph, q);
        replicate_q_context_rows(&mut self.coeff_base_uv, q);
        replicate_q_context_rows(&mut self.coeff_base_lf, q);
        replicate_q_context_rows(&mut self.coeff_base_lf_uv, q);
        replicate_q_context_rows(&mut self.coeff_base_eob, q);
        replicate_q_context_rows(&mut self.coeff_base_eob_uv, q);
        replicate_q_context_rows(&mut self.coeff_base_bob, q);
        replicate_q_context_rows(&mut self.coeff_base_idtx, q);
        replicate_q_context_rows(&mut self.coeff_base_lf_eob, q);
        replicate_q_context_rows(&mut self.coeff_base_lf_eob_uv, q);
        replicate_q_context_rows(&mut self.coeff_br, q);
        replicate_q_context_rows(&mut self.coeff_br_uv, q);
        replicate_q_context_rows(&mut self.coeff_br_lf, q);
        replicate_q_context_rows(&mut self.coeff_br_idtx, q);
        replicate_q_context_rows(&mut self.idtx_sign, q);
        Ok(())
    }
}

macro_rules! coeff_cdf_row {
    ($self:ident, $selector:ident, $as_slice:ident) => {
        match $selector {
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
                Ok($self.coeff_base[q][tx_size][ctx][tcq_ctx].$as_slice())
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
                Ok($self.coeff_base_ph[q][ctx].$as_slice())
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
                Ok($self.coeff_base_uv[q][ctx].$as_slice())
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
                Ok($self.coeff_base_lf[q][tx_size][ctx][tcq_ctx].$as_slice())
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
                Ok($self.coeff_base_lf_uv[q][ctx].$as_slice())
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
                Ok($self.coeff_base_eob[q][tx_size][ctx].$as_slice())
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
                Ok($self.coeff_base_eob_uv[q][ctx].$as_slice())
            }
            CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseBob, coeff_cdf_q_ctx)?;
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBaseBob, tx_size_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseBob,
                    "ctx",
                    ctx,
                    COEFF_BASE_BOB_CONTEXTS,
                )?;
                Ok($self.coeff_base_bob[q][tx_size_ctx][ctx].$as_slice())
            }
            CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBaseIdtx, coeff_cdf_q_ctx)?;
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBaseIdtx, tx_size_ctx)?;
                let ctx = checked_index(
                    TileCdfArray::CoeffBaseIdtx,
                    "ctx",
                    ctx,
                    IDTX_SIG_COEF_CONTEXTS,
                )?;
                Ok($self.coeff_base_idtx[q][tx_size_ctx][ctx].$as_slice())
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
                Ok($self.coeff_base_lf_eob[q][tx_size][ctx].$as_slice())
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
                Ok($self.coeff_base_lf_eob_uv[q][ctx].$as_slice())
            }
            CoeffCdfSelector::Br {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBr, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBr, "ctx", ctx, COEFF_BR_CONTEXTS)?;
                Ok($self.coeff_br[q][ctx].$as_slice())
            }
            CoeffCdfSelector::BrUv {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrUv, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrUv, "ctx", ctx, COEFF_BR_UV_CONTEXTS)?;
                Ok($self.coeff_br_uv[q][ctx].$as_slice())
            }
            CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrLf, coeff_cdf_q_ctx)?;
                let ctx = checked_index(TileCdfArray::CoeffBrLf, "ctx", ctx, COEFF_BR_LF_CONTEXTS)?;
                Ok($self.coeff_br_lf[q][ctx].$as_slice())
            }
            CoeffCdfSelector::BrIdtx {
                coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::CoeffBrIdtx, coeff_cdf_q_ctx)?;
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBrIdtx, tx_size_ctx)?;
                let ctx =
                    checked_index(TileCdfArray::CoeffBrIdtx, "ctx", ctx, IDTX_LEVEL_CONTEXTS)?;
                Ok($self.coeff_br_idtx[q][tx_size_ctx][ctx].$as_slice())
            }
            CoeffCdfSelector::IdtxSign {
                coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::IdtxSign, coeff_cdf_q_ctx)?;
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::IdtxSign, tx_size_ctx)?;
                let ctx = checked_index(TileCdfArray::IdtxSign, "ctx", ctx, IDTX_SIGN_CONTEXTS)?;
                Ok($self.idtx_sign[q][tx_size_ctx][ctx].$as_slice())
            }
        }
    };
}

macro_rules! avg_row_family {
    (2, $self:ident, $tile:ident, $tile_num:ident, $num_log2:ident, $field:ident) => {
        avg_cdf_rows(
            $self.$field.iter_mut().flatten(),
            $tile.$field.iter().flatten(),
            $tile_num,
            $num_log2,
        );
    };
    (3, $self:ident, $tile:ident, $tile_num:ident, $num_log2:ident, $field:ident) => {
        avg_cdf_rows(
            $self.$field.iter_mut().flatten().flatten(),
            $tile.$field.iter().flatten().flatten(),
            $tile_num,
            $num_log2,
        );
    };
    (4, $self:ident, $tile:ident, $tile_num:ident, $num_log2:ident, $field:ident) => {
        avg_cdf_rows(
            $self.$field.iter_mut().flatten().flatten().flatten(),
            $tile.$field.iter().flatten().flatten().flatten(),
            $tile_num,
            $num_log2,
        );
    };
}

macro_rules! scale_row_family {
    (2, $self:ident, $field:ident) => {
        scale_cdf_rows($self.$field.iter_mut().flatten());
    };
    (3, $self:ident, $field:ident) => {
        scale_cdf_rows($self.$field.iter_mut().flatten().flatten());
    };
    (4, $self:ident, $field:ident) => {
        scale_cdf_rows($self.$field.iter_mut().flatten().flatten().flatten());
    };
}

macro_rules! blend_row_family {
    (2, $self:ident, $saved:ident, $field:ident) => {
        blend_cdf_rows(
            $self.$field.iter_mut().flatten(),
            $saved.$field.iter().flatten(),
        );
    };
    (3, $self:ident, $saved:ident, $field:ident) => {
        blend_cdf_rows(
            $self.$field.iter_mut().flatten().flatten(),
            $saved.$field.iter().flatten().flatten(),
        );
    };
    (4, $self:ident, $saved:ident, $field:ident) => {
        blend_cdf_rows(
            $self.$field.iter_mut().flatten().flatten().flatten(),
            $saved.$field.iter().flatten().flatten().flatten(),
        );
    };
}

macro_rules! coeff_cdf_lifecycle_families {
    ($macro:ident $(, $arg:tt)*) => {
        $macro!(4 $(, $arg)*, coeff_base);
        $macro!(2 $(, $arg)*, coeff_base_ph);
        $macro!(2 $(, $arg)*, coeff_base_uv);
        $macro!(4 $(, $arg)*, coeff_base_lf);
        $macro!(2 $(, $arg)*, coeff_base_lf_uv);
        $macro!(3 $(, $arg)*, coeff_base_eob);
        $macro!(2 $(, $arg)*, coeff_base_eob_uv);
        $macro!(3 $(, $arg)*, coeff_base_bob);
        $macro!(3 $(, $arg)*, coeff_base_idtx);
        $macro!(3 $(, $arg)*, coeff_base_lf_eob);
        $macro!(2 $(, $arg)*, coeff_base_lf_eob_uv);
        $macro!(2 $(, $arg)*, coeff_br);
        $macro!(2 $(, $arg)*, coeff_br_uv);
        $macro!(2 $(, $arg)*, coeff_br_lf);
        $macro!(3 $(, $arg)*, coeff_br_idtx);
        $macro!(3 $(, $arg)*, idtx_sign);
    };
}

impl CoeffCdfRows {
    pub(crate) fn row(&self, selector: CoeffCdfSelector) -> Result<&[i32], TileCdfError> {
        coeff_cdf_row!(self, selector, as_slice)
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: CoeffCdfSelector,
    ) -> Result<&mut [i32], TileCdfError> {
        coeff_cdf_row!(self, selector, as_mut_slice)
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        coeff_cdf_lifecycle_families!(avg_row_family, self, tile, tile_num, num_log2);
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        coeff_cdf_lifecycle_families!(blend_row_family, self, saved);
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        coeff_cdf_lifecycle_families!(scale_row_family, self);
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

fn checked_fsc_tx_size(array: TileCdfArray, tx_size_ctx: usize) -> Result<usize, TileCdfError> {
    if tx_size_ctx >= FSC_TX_SIZE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "tx_size_ctx",
            actual: tx_size_ctx,
            max_exclusive: FSC_TX_SIZE_CONTEXTS,
        });
    }
    Ok(tx_size_ctx)
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
