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

use super::coeff_context::CoeffBaseSelection;
use super::util::checked_context;
use super::{
    CoeffCdfQContext, TileCdfArray, TileCdfError, avg_cdf_rows, blend_cdf_rows, scale_cdf_rows,
};

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

pub(crate) type CoeffBaseCdfRows = TcqRows<COEFF_BASE_ROW_LEN, COEFF_BASE_CONTEXTS>;
pub(crate) type CoeffBasePhCdfRows = [[u16; COEFF_BASE_ROW_LEN]; COEFF_BASE_PH_CONTEXTS];
pub(crate) type CoeffBaseUvCdfRows = [[u16; COEFF_BASE_ROW_LEN]; COEFF_BASE_UV_CONTEXTS];
pub(crate) type CoeffBaseLfCdfRows = TcqRows<COEFF_BASE_LF_ROW_LEN, COEFF_BASE_LF_CONTEXTS>;
pub(crate) type CoeffBaseLfUvCdfRows = [[u16; COEFF_BASE_LF_ROW_LEN]; COEFF_BASE_LF_UV_CONTEXTS];
pub(crate) type CoeffBaseEobCdfRows =
    [[[u16; COEFF_BASE_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS]; TX_SIZE_CONTEXTS];
pub(crate) type CoeffBaseEobUvCdfRows = [[u16; COEFF_BASE_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS];
pub(crate) type CoeffBaseBobCdfRows =
    [[[u16; COEFF_BASE_BOB_ROW_LEN]; COEFF_BASE_BOB_CONTEXTS]; FSC_TX_SIZE_CONTEXTS];
pub(crate) type CoeffBaseIdtxCdfRows =
    [[[u16; COEFF_BASE_ROW_LEN]; IDTX_SIG_COEF_CONTEXTS]; FSC_TX_SIZE_CONTEXTS];
pub(crate) type CoeffBaseLfEobCdfRows =
    [[[u16; COEFF_BASE_LF_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS]; TX_SIZE_CONTEXTS];
pub(crate) type CoeffBaseLfEobUvCdfRows =
    [[u16; COEFF_BASE_LF_EOB_ROW_LEN]; COEFF_BASE_EOB_CONTEXTS];
pub(crate) type CoeffBrCdfRows = [[u16; COEFF_BR_ROW_LEN]; COEFF_BR_CONTEXTS];
pub(crate) type CoeffBrUvCdfRows = [[u16; COEFF_BR_ROW_LEN]; COEFF_BR_UV_CONTEXTS];
pub(crate) type CoeffBrLfCdfRows = [[u16; COEFF_BR_ROW_LEN]; COEFF_BR_LF_CONTEXTS];
pub(crate) type CoeffBrIdtxCdfRows =
    [[[u16; COEFF_BR_ROW_LEN]; IDTX_LEVEL_CONTEXTS]; FSC_TX_SIZE_CONTEXTS];
pub(crate) type IdtxSignCdfRows =
    [[[u16; IDTX_SIGN_ROW_LEN]; IDTX_SIGN_CONTEXTS]; FSC_TX_SIZE_CONTEXTS];

/// Decode reads the base and base-range rows through the typed
/// [`CoeffCdfRows::base_row`] and [`CoeffCdfRows::br_row`]; their selector
/// variants only address rows in tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffCdfSelector {
    #[cfg(test)]
    Base {
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    #[cfg(test)]
    BasePh {
        ctx: usize,
    },
    #[cfg(test)]
    BaseUv {
        ctx: usize,
    },
    #[cfg(test)]
    BaseLf {
        tx_size: usize,
        ctx: usize,
        tcq_ctx: usize,
    },
    #[cfg(test)]
    BaseLfUv {
        ctx: usize,
    },
    BaseEob {
        tx_size: usize,
        ctx: usize,
    },
    BaseEobUv {
        ctx: usize,
    },
    BaseBob {
        tx_size_ctx: usize,
        ctx: usize,
    },
    BaseIdtx {
        tx_size_ctx: usize,
        ctx: usize,
    },
    BaseLfEob {
        tx_size: usize,
        ctx: usize,
    },
    BaseLfEobUv {
        ctx: usize,
    },
    #[cfg(test)]
    Br {
        ctx: usize,
    },
    #[cfg(test)]
    BrUv {
        ctx: usize,
    },
    #[cfg(test)]
    BrLf {
        ctx: usize,
    },
    BrIdtx {
        tx_size_ctx: usize,
        ctx: usize,
    },
    IdtxSign {
        tx_size_ctx: usize,
        ctx: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    /// The `Default_*_Cdf` rows of coefficient CDF q context `q`.
    pub(crate) const fn from_defaults(q: CoeffCdfQContext) -> Self {
        let q = q.index();
        Self {
            coeff_base: DEFAULT_COEFF_BASE_CDF[q],
            coeff_base_ph: DEFAULT_COEFF_BASE_PH_CDF[q],
            coeff_base_uv: DEFAULT_COEFF_BASE_UV_CDF[q],
            coeff_base_lf: DEFAULT_COEFF_BASE_LF_CDF[q],
            coeff_base_lf_uv: DEFAULT_COEFF_BASE_LF_UV_CDF[q],
            coeff_base_eob: DEFAULT_COEFF_BASE_EOB_CDF[q],
            coeff_base_eob_uv: DEFAULT_COEFF_BASE_EOB_UV_CDF[q],
            coeff_base_bob: DEFAULT_COEFF_BASE_BOB_CDF[q],
            coeff_base_idtx: DEFAULT_COEFF_BASE_IDTX_CDF[q],
            coeff_base_lf_eob: DEFAULT_COEFF_BASE_LF_EOB_CDF[q],
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[q],
            coeff_br: DEFAULT_COEFF_BR_CDF[q],
            coeff_br_uv: DEFAULT_COEFF_BR_UV_CDF[q],
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF[q],
            coeff_br_idtx: DEFAULT_COEFF_BR_IDTX_CDF[q],
            idtx_sign: DEFAULT_IDTX_SIGN_CDF[q],
        }
    }
}

macro_rules! coeff_cdf_lifecycle_families {
    ($visit:ident) => {
        $visit!(coeff_base.flatten().flatten());
        $visit!(coeff_base_ph);
        $visit!(coeff_base_uv);
        $visit!(coeff_base_lf.flatten().flatten());
        $visit!(coeff_base_lf_uv);
        $visit!(coeff_base_eob.flatten());
        $visit!(coeff_base_eob_uv);
        $visit!(coeff_base_bob.flatten());
        $visit!(coeff_base_idtx.flatten());
        $visit!(coeff_base_lf_eob.flatten());
        $visit!(coeff_base_lf_eob_uv);
        $visit!(coeff_br);
        $visit!(coeff_br_uv);
        $visit!(coeff_br_lf);
        $visit!(coeff_br_idtx.flatten());
        $visit!(idtx_sign.flatten());
    };
}

/// The CDF row of a derived non-EOB base context.
pub(crate) enum CoeffBaseRow<'a> {
    Base(&'a mut [u16; COEFF_BASE_ROW_LEN]),
    Lf(&'a mut [u16; COEFF_BASE_LF_ROW_LEN]),
}

impl CoeffCdfRows {
    /// Typed row of a derived base selection, so hot reads skip the
    /// selector dispatch and the row-length dispatch.
    #[inline]
    pub(crate) fn base_row(
        &mut self,
        selection: CoeffBaseSelection,
        tx_size: usize,
        tcq_ctx: usize,
    ) -> Result<CoeffBaseRow<'_>, TileCdfError> {
        use TileCdfArray::{CoeffBase, CoeffBaseLf, CoeffBaseLfUv, CoeffBasePh, CoeffBaseUv};
        Ok(match selection {
            CoeffBaseSelection::Hf { ctx } => CoeffBaseRow::Base(row4(
                &mut self.coeff_base,
                CoeffBase,
                [tx_size, ctx, tcq_ctx],
            )?),
            CoeffBaseSelection::Ph { ctx } => {
                CoeffBaseRow::Base(row2(&mut self.coeff_base_ph, CoeffBasePh, ctx)?)
            }
            CoeffBaseSelection::Uv { ctx } => {
                CoeffBaseRow::Base(row2(&mut self.coeff_base_uv, CoeffBaseUv, ctx)?)
            }
            CoeffBaseSelection::Lf { ctx } => CoeffBaseRow::Lf(row4(
                &mut self.coeff_base_lf,
                CoeffBaseLf,
                [tx_size, ctx, tcq_ctx],
            )?),
            CoeffBaseSelection::LfUv { ctx } => {
                CoeffBaseRow::Lf(row2(&mut self.coeff_base_lf_uv, CoeffBaseLfUv, ctx)?)
            }
        })
    }

    /// Typed base-range row for `plane` and `is_lf`.
    #[inline]
    pub(crate) fn br_row(
        &mut self,
        plane: usize,
        is_lf: bool,
        ctx: usize,
    ) -> Result<&mut [u16; COEFF_BR_ROW_LEN], TileCdfError> {
        if plane > 0 {
            row2(&mut self.coeff_br_uv, TileCdfArray::CoeffBrUv, ctx)
        } else if is_lf {
            row2(&mut self.coeff_br_lf, TileCdfArray::CoeffBrLf, ctx)
        } else {
            row2(&mut self.coeff_br, TileCdfArray::CoeffBr, ctx)
        }
    }

    #[inline]
    pub(crate) fn row_mut(
        &mut self,
        selector: CoeffCdfSelector,
    ) -> Result<&mut [u16], TileCdfError> {
        match selector {
            #[cfg(test)]
            CoeffCdfSelector::Base {
                tx_size,
                ctx,
                tcq_ctx,
            } => Ok(row4(
                &mut self.coeff_base,
                TileCdfArray::CoeffBase,
                [tx_size, ctx, tcq_ctx],
            )?
            .as_mut_slice()),
            #[cfg(test)]
            CoeffCdfSelector::BasePh { ctx } => {
                Ok(row2(&mut self.coeff_base_ph, TileCdfArray::CoeffBasePh, ctx)?.as_mut_slice())
            }
            #[cfg(test)]
            CoeffCdfSelector::BaseUv { ctx } => {
                Ok(row2(&mut self.coeff_base_uv, TileCdfArray::CoeffBaseUv, ctx)?.as_mut_slice())
            }
            #[cfg(test)]
            CoeffCdfSelector::BaseLf {
                tx_size,
                ctx,
                tcq_ctx,
            } => Ok(row4(
                &mut self.coeff_base_lf,
                TileCdfArray::CoeffBaseLf,
                [tx_size, ctx, tcq_ctx],
            )?
            .as_mut_slice()),
            #[cfg(test)]
            CoeffCdfSelector::BaseLfUv { ctx } => {
                Ok(
                    row2(&mut self.coeff_base_lf_uv, TileCdfArray::CoeffBaseLfUv, ctx)?
                        .as_mut_slice(),
                )
            }
            CoeffCdfSelector::BaseEob { tx_size, ctx } => {
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseEob, tx_size)?;
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob[tx_size][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseEobUv { ctx } => {
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_eob_uv[ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseBob { tx_size_ctx, ctx } => {
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBaseBob, tx_size_ctx)?;
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseBob,
                    "ctx",
                    ctx,
                    COEFF_BASE_BOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_bob[tx_size_ctx][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseIdtx { tx_size_ctx, ctx } => {
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBaseIdtx, tx_size_ctx)?;
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseIdtx,
                    "ctx",
                    ctx,
                    IDTX_SIG_COEF_CONTEXTS,
                )?;
                Ok(self.coeff_base_idtx[tx_size_ctx][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLfEob { tx_size, ctx } => {
                let tx_size = checked_tx_size(TileCdfArray::CoeffBaseLfEob, tx_size)?;
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseLfEob,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob[tx_size][ctx].as_mut_slice())
            }
            CoeffCdfSelector::BaseLfEobUv { ctx } => {
                let ctx = checked_context(
                    TileCdfArray::CoeffBaseLfEobUv,
                    "ctx",
                    ctx,
                    COEFF_BASE_EOB_CONTEXTS,
                )?;
                Ok(self.coeff_base_lf_eob_uv[ctx].as_mut_slice())
            }
            #[cfg(test)]
            CoeffCdfSelector::Br { ctx } => {
                Ok(row2(&mut self.coeff_br, TileCdfArray::CoeffBr, ctx)?.as_mut_slice())
            }
            #[cfg(test)]
            CoeffCdfSelector::BrUv { ctx } => {
                Ok(row2(&mut self.coeff_br_uv, TileCdfArray::CoeffBrUv, ctx)?.as_mut_slice())
            }
            #[cfg(test)]
            CoeffCdfSelector::BrLf { ctx } => {
                Ok(row2(&mut self.coeff_br_lf, TileCdfArray::CoeffBrLf, ctx)?.as_mut_slice())
            }
            CoeffCdfSelector::BrIdtx { tx_size_ctx, ctx } => {
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::CoeffBrIdtx, tx_size_ctx)?;
                let ctx =
                    checked_context(TileCdfArray::CoeffBrIdtx, "ctx", ctx, IDTX_LEVEL_CONTEXTS)?;
                Ok(self.coeff_br_idtx[tx_size_ctx][ctx].as_mut_slice())
            }
            CoeffCdfSelector::IdtxSign { tx_size_ctx, ctx } => {
                let tx_size_ctx = checked_fsc_tx_size(TileCdfArray::IdtxSign, tx_size_ctx)?;
                let ctx = checked_context(TileCdfArray::IdtxSign, "ctx", ctx, IDTX_SIGN_CONTEXTS)?;
                Ok(self.idtx_sign[tx_size_ctx][ctx].as_mut_slice())
            }
        }
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        macro_rules! avg_rows {
            ($field:ident $(. $flatten:ident())*) => {
                avg_cdf_rows(
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(tile.$field $(, $flatten)*),
                    tile_num,
                    num_log2,
                );
            };
        }
        coeff_cdf_lifecycle_families!(avg_rows);
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        macro_rules! blend_rows {
            ($field:ident $(. $flatten:ident())*) => {
                blend_cdf_rows(
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(saved.$field $(, $flatten)*),
                );
            };
        }
        coeff_cdf_lifecycle_families!(blend_rows);
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        macro_rules! scale_rows {
            ($field:ident $(. $flatten:ident())*) => {
                scale_cdf_rows(flat_cdf_rows_mut!(self.$field $(, $flatten)*));
            };
        }
        coeff_cdf_lifecycle_families!(scale_rows);
    }
}

/// Bounds-checked `rows[ctx]`.
#[inline]
fn row2<const L: usize, const C: usize>(
    rows: &mut [[u16; L]; C],
    array: TileCdfArray,
    ctx: usize,
) -> Result<&mut [u16; L], TileCdfError> {
    let ctx = checked_context(array, "ctx", ctx, C)?;
    Ok(&mut rows[ctx])
}

type TcqRows<const L: usize, const C: usize> =
    [[[[u16; L]; COEFF_BASE_TCQ_CONTEXTS]; C]; TX_SIZE_CONTEXTS];

/// Bounds-checked `rows[tx_size][ctx][tcq_ctx]`.
#[inline]
fn row4<const L: usize, const C: usize>(
    rows: &mut TcqRows<L, C>,
    array: TileCdfArray,
    [tx_size, ctx, tcq_ctx]: [usize; 3],
) -> Result<&mut [u16; L], TileCdfError> {
    let tx_size = checked_tx_size(array, tx_size)?;
    let ctx = checked_context(array, "ctx", ctx, C)?;
    let tcq_ctx = checked_context(array, "tcq_ctx", tcq_ctx, COEFF_BASE_TCQ_CONTEXTS)?;
    Ok(&mut rows[tx_size][ctx][tcq_ctx])
}

fn checked_tx_size(array: TileCdfArray, tx_size: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "tx_size", tx_size, TX_SIZE_CONTEXTS)
}

fn checked_fsc_tx_size(array: TileCdfArray, tx_size_ctx: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "tx_size_ctx", tx_size_ctx, FSC_TX_SIZE_CONTEXTS)
}
