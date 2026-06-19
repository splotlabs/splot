// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal block-symbol CDF rows for the traced runtime frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::tables::cdf::{
    DEFAULT_DC_SIGN_CDF, DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF,
    DEFAULT_EOB_PT_64_CDF, DEFAULT_EOB_PT_128_CDF, DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use super::coeff_rows::{CoeffCdfRows, CoeffCdfSelector};
use super::{CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_row, scale_cdf_count};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const EOB_PLANE_CTXS: usize = 3;
const PLANE_TYPES: usize = 2;
const TX_SIZE_CONTEXTS: usize = 5;
const TXB_SKIP_CONTEXTS: usize = 10;
const UV_MODE_CONTEXTS: usize = 2;
const V_TXB_SKIP_CONTEXTS: usize = 12;
const DC_SIGN_GROUPS: usize = 2;
const DC_SIGN_CONTEXTS: usize = 3;

pub(crate) type YModeSetCdfRow = [i32; Y_MODE_SET_CDF_ROW_LEN];
pub(crate) type YModeIndexCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; Y_MODE_INDEX_CONTEXTS];
pub(crate) type TxbSkipCdfRows = [[[[[i32; CDF_ROW_LEN]; TXB_SKIP_CONTEXTS]; TX_SIZE_CONTEXTS];
    PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type UvModeCflNotAllowedCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; UV_MODE_CONTEXTS];
pub(crate) type VTxbSkipCdfRows = [[[i32; CDF_ROW_LEN]; V_TXB_SKIP_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
// `eob_extra` is a binary symbol, so its rows are width 3 — the same as the
// generic `CDF_ROW_LEN`. `DEFAULT_EOB_EXTRA_CDF` is `[[i32; 3]; 4]`, so this alias
// must keep an inner width of 3; if `CDF_ROW_LEN` ever changes, give `eob_extra`
// its own width constant rather than over-allocating from the generic one.
pub(crate) type EobExtraCdfRows = [[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS];
// §9.3 `dc_sign`: `[coeff_cdf_q_ctx][plane_type][isHidden group][ctx][3]`. §8.3.2
// reads `TileDcSignCdf[ptype][isHidden][ctx]`; `ctx` (0/1/2) is derived from the
// Above/Left DC-context buffers (deferred with the coeffs() loop).
pub(crate) type DcSignCdfRows =
    [[[[[i32; CDF_ROW_LEN]; DC_SIGN_CONTEXTS]; DC_SIGN_GROUPS]; PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];

// The §9.3 `eob_pt` CDF family: one bank per transform-size class, each
// `[coeff_cdf_q_ctx][eobCtx][N]` with a class-specific symbol width N. §8.3.2
// selects `TileEobPt<size>Cdf[eobCtx]` for the active q-context.
pub(crate) type EobPt16CdfRows = [[[i32; 6]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt32CdfRows = [[[i32; 7]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt64CdfRows = [[[i32; 8]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt128CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt256CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt512CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt1024CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];

/// The AV2 `eob_pt` transform-size class, selecting which `TileEobPt<size>Cdf`
/// family bank the §8.3.2 `eob_pt` symbol reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EobPtSize {
    /// `TileEobPt16Cdf`.
    Pt16,
    /// `TileEobPt32Cdf`.
    Pt32,
    /// `TileEobPt64Cdf`.
    Pt64,
    /// `TileEobPt128Cdf`.
    Pt128,
    /// `TileEobPt256Cdf`.
    Pt256,
    /// `TileEobPt512Cdf`.
    Pt512,
    /// `TileEobPt1024Cdf`.
    Pt1024,
}

/// Block-symbol CDF selectors handled by this focused row bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockCdfSelector {
    /// `TileYModeSetCdf`.
    YModeSet,
    /// `TileYModeIndexCdf[ctx]`.
    YModeIndex {
        /// Intra mode context index.
        ctx: usize,
    },
    /// `TileTxbSkipCdf[coeff_cdf_q_ctx][plane_type][tx_size][ctx]`.
    TxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context.
        plane_type: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Transform-skip context index.
        ctx: usize,
    },
    /// `TileUvModeCflNotAllowedCdf[ctx]`.
    UvModeCflNotAllowed {
        /// Chroma mode context index.
        ctx: usize,
    },
    /// `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]`.
    VTxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// V-plane transform-skip context index.
        ctx: usize,
    },
    /// `TileEobExtraCdf[coeff_cdf_q_ctx]` (AV2 § 8.3.2: the cdf is given by
    /// `TileEobExtraCdf` directly, with no per-symbol context).
    EobExtra {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
    },
    /// `TileEobPt<size>Cdf[coeff_cdf_q_ctx][eobCtx]` (AV2 § 8.3.2): the
    /// transform-size class selects the bank and `eobCtx` selects the row.
    EobPt {
        /// Transform-size class selecting the `eob_pt` family bank.
        size: EobPtSize,
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// `eobCtx = (plane > 0) ? 2 : is_inter` (`0..EOB_PLANE_CTXS`).
        eob_ctx: usize,
    },
    /// `TileDcSignCdf[coeff_cdf_q_ctx][plane_type][group][ctx]` (AV2 § 8.3.2):
    /// `group` is the spec `isHidden` flag; `ctx` (`0..DC_SIGN_CONTEXTS`) is the
    /// caller-resolved DC-sign context.
    DcSign {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context (luma vs chroma).
        plane_type: usize,
        /// `isHidden` group (`0..DC_SIGN_GROUPS`).
        group: usize,
        /// DC-sign context (`0..DC_SIGN_CONTEXTS`).
        ctx: usize,
    },
    /// Coefficient base/base-EOB/base-range and IDTX CDF rows.
    Coeff(CoeffCdfSelector),
}

/// Supported block-symbol CDF arrays for the minimal flat-intra trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockCdfRows {
    pub(super) y_mode_set: YModeSetCdfRow,
    pub(super) y_mode_index: YModeIndexCdfRows,
    pub(super) txb_skip: TxbSkipCdfRows,
    pub(super) uv_mode_cfl_not_allowed: UvModeCflNotAllowedCdfRows,
    pub(super) v_txb_skip: VTxbSkipCdfRows,
    pub(super) eob_extra: EobExtraCdfRows,
    pub(super) eob_pt_16: EobPt16CdfRows,
    pub(super) eob_pt_32: EobPt32CdfRows,
    pub(super) eob_pt_64: EobPt64CdfRows,
    pub(super) eob_pt_128: EobPt128CdfRows,
    pub(super) eob_pt_256: EobPt256CdfRows,
    pub(super) eob_pt_512: EobPt512CdfRows,
    pub(super) eob_pt_1024: EobPt1024CdfRows,
    pub(super) dc_sign: DcSignCdfRows,
    pub(super) coeff: CoeffCdfRows,
}

impl BlockCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index: DEFAULT_Y_MODE_INDEX_CDF,
            txb_skip: DEFAULT_TXB_SKIP_CDF,
            uv_mode_cfl_not_allowed: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF,
            eob_extra: DEFAULT_EOB_EXTRA_CDF,
            eob_pt_16: DEFAULT_EOB_PT_16_CDF,
            eob_pt_32: DEFAULT_EOB_PT_32_CDF,
            eob_pt_64: DEFAULT_EOB_PT_64_CDF,
            eob_pt_128: DEFAULT_EOB_PT_128_CDF,
            eob_pt_256: DEFAULT_EOB_PT_256_CDF,
            eob_pt_512: DEFAULT_EOB_PT_512_CDF,
            eob_pt_1024: DEFAULT_EOB_PT_1024_CDF,
            dc_sign: DEFAULT_DC_SIGN_CDF,
            coeff: CoeffCdfRows::from_defaults(),
        }
    }

    pub(crate) fn row(&self, selector: BlockCdfSelector) -> Result<&[i32], TileCdfError> {
        match selector {
            BlockCdfSelector::YModeSet => Ok(self.y_mode_set.as_slice()),
            BlockCdfSelector::YModeIndex { ctx } => {
                let row = self
                    .y_mode_index
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::YModeIndex,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: Y_MODE_INDEX_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::TxbSkip, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::TxbSkip, plane_type)?;
                let tx_size = checked_tx_size(TileCdfArray::TxbSkip, tx_size)?;
                let row = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size]
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: TXB_SKIP_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::UvModeCflNotAllowed { ctx } => {
                let row = self.uv_mode_cfl_not_allowed.get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::UvModeCflNotAllowed,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: UV_MODE_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::VTxbSkip, coeff_cdf_q_ctx)?;
                let row = self.v_txb_skip[coeff_cdf_q_ctx].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::VTxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: V_TXB_SKIP_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok(self.eob_extra[coeff_cdf_q_ctx].as_slice())
            }
            BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::EobPt, coeff_cdf_q_ctx)?;
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => self.eob_pt_16[q][c].as_slice(),
                    EobPtSize::Pt32 => self.eob_pt_32[q][c].as_slice(),
                    EobPtSize::Pt64 => self.eob_pt_64[q][c].as_slice(),
                    EobPtSize::Pt128 => self.eob_pt_128[q][c].as_slice(),
                    EobPtSize::Pt256 => self.eob_pt_256[q][c].as_slice(),
                    EobPtSize::Pt512 => self.eob_pt_512[q][c].as_slice(),
                    EobPtSize::Pt1024 => self.eob_pt_1024[q][c].as_slice(),
                })
            }
            BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::DcSign, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::DcSign, plane_type)?;
                let group = checked_dc_sign_group(group)?;
                let row = self.dc_sign[q][plane_type][group].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DcSign,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DC_SIGN_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::Coeff(selector) => self.coeff.row(selector),
        }
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: BlockCdfSelector,
    ) -> Result<&mut [i32], TileCdfError> {
        match selector {
            BlockCdfSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
            BlockCdfSelector::YModeIndex { ctx } => {
                let max_exclusive = self.y_mode_index.len();
                let row =
                    self.y_mode_index
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::YModeIndex,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::TxbSkip, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::TxbSkip, plane_type)?;
                let tx_size = checked_tx_size(TileCdfArray::TxbSkip, tx_size)?;
                let max_exclusive = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size].len();
                let row = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size]
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::UvModeCflNotAllowed { ctx } => {
                let max_exclusive = self.uv_mode_cfl_not_allowed.len();
                let row = self.uv_mode_cfl_not_allowed.get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::UvModeCflNotAllowed,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::VTxbSkip, coeff_cdf_q_ctx)?;
                let max_exclusive = self.v_txb_skip[coeff_cdf_q_ctx].len();
                let row = self.v_txb_skip[coeff_cdf_q_ctx].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::VTxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok(self.eob_extra[coeff_cdf_q_ctx].as_mut_slice())
            }
            BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::EobPt, coeff_cdf_q_ctx)?;
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => self.eob_pt_16[q][c].as_mut_slice(),
                    EobPtSize::Pt32 => self.eob_pt_32[q][c].as_mut_slice(),
                    EobPtSize::Pt64 => self.eob_pt_64[q][c].as_mut_slice(),
                    EobPtSize::Pt128 => self.eob_pt_128[q][c].as_mut_slice(),
                    EobPtSize::Pt256 => self.eob_pt_256[q][c].as_mut_slice(),
                    EobPtSize::Pt512 => self.eob_pt_512[q][c].as_mut_slice(),
                    EobPtSize::Pt1024 => self.eob_pt_1024[q][c].as_mut_slice(),
                })
            }
            BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::DcSign, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::DcSign, plane_type)?;
                let group = checked_dc_sign_group(group)?;
                let row = self.dc_sign[q][plane_type][group].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DcSign,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DC_SIGN_CONTEXTS,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::Coeff(selector) => self.coeff.row_mut(selector),
        }
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        avg_cdf_row(&mut self.y_mode_set, &tile.y_mode_set, tile_num, num_log2);
        for ctx in 0..Y_MODE_INDEX_CONTEXTS {
            avg_cdf_row(
                &mut self.y_mode_index[ctx],
                &tile.y_mode_index[ctx],
                tile_num,
                num_log2,
            );
        }
        for coeff_cdf_q_ctx in 0..COEFF_CDF_Q_CONTEXTS {
            for plane_type in 0..PLANE_TYPES {
                for tx_size in 0..TX_SIZE_CONTEXTS {
                    for ctx in 0..TXB_SKIP_CONTEXTS {
                        avg_cdf_row(
                            &mut self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                            &tile.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                            tile_num,
                            num_log2,
                        );
                    }
                }
            }
            for ctx in 0..V_TXB_SKIP_CONTEXTS {
                avg_cdf_row(
                    &mut self.v_txb_skip[coeff_cdf_q_ctx][ctx],
                    &tile.v_txb_skip[coeff_cdf_q_ctx][ctx],
                    tile_num,
                    num_log2,
                );
            }
            avg_cdf_row(
                &mut self.eob_extra[coeff_cdf_q_ctx],
                &tile.eob_extra[coeff_cdf_q_ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.uv_mode_cfl_not_allowed[ctx],
                &tile.uv_mode_cfl_not_allowed[ctx],
                tile_num,
                num_log2,
            );
        }
        avg_eob_pt_bank(&mut self.eob_pt_16, &tile.eob_pt_16, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_32, &tile.eob_pt_32, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_64, &tile.eob_pt_64, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_128, &tile.eob_pt_128, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_256, &tile.eob_pt_256, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_512, &tile.eob_pt_512, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_1024, &tile.eob_pt_1024, tile_num, num_log2);
        for (frame_row, tile_row) in self
            .dc_sign
            .iter_mut()
            .flatten()
            .flatten()
            .flatten()
            .zip(tile.dc_sign.iter().flatten().flatten().flatten())
        {
            avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
        }
        self.coeff.avg_from_tile(tile_num, &tile.coeff, num_log2);
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        scale_cdf_count(&mut self.y_mode_set);
        for ctx in 0..Y_MODE_INDEX_CONTEXTS {
            scale_cdf_count(&mut self.y_mode_index[ctx]);
        }
        for coeff_cdf_q_ctx in 0..COEFF_CDF_Q_CONTEXTS {
            for plane_type in 0..PLANE_TYPES {
                for tx_size in 0..TX_SIZE_CONTEXTS {
                    for ctx in 0..TXB_SKIP_CONTEXTS {
                        scale_cdf_count(
                            &mut self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                        );
                    }
                }
            }
            for ctx in 0..V_TXB_SKIP_CONTEXTS {
                scale_cdf_count(&mut self.v_txb_skip[coeff_cdf_q_ctx][ctx]);
            }
            scale_cdf_count(&mut self.eob_extra[coeff_cdf_q_ctx]);
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            scale_cdf_count(&mut self.uv_mode_cfl_not_allowed[ctx]);
        }
        scale_eob_pt_bank(&mut self.eob_pt_16);
        scale_eob_pt_bank(&mut self.eob_pt_32);
        scale_eob_pt_bank(&mut self.eob_pt_64);
        scale_eob_pt_bank(&mut self.eob_pt_128);
        scale_eob_pt_bank(&mut self.eob_pt_256);
        scale_eob_pt_bank(&mut self.eob_pt_512);
        scale_eob_pt_bank(&mut self.eob_pt_1024);
        for row in self.dc_sign.iter_mut().flatten().flatten().flatten() {
            scale_cdf_count(row);
        }
        self.coeff.scale_counts_for_frame_end_update();
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_set(&self) -> &YModeSetCdfRow {
        &self.y_mode_set
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_index(&self) -> &YModeIndexCdfRows {
        &self.y_mode_index
    }

    #[cfg(test)]
    pub(crate) const fn txb_skip(&self) -> &TxbSkipCdfRows {
        &self.txb_skip
    }

    #[cfg(test)]
    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &UvModeCflNotAllowedCdfRows {
        &self.uv_mode_cfl_not_allowed
    }

    #[cfg(test)]
    pub(crate) const fn v_txb_skip(&self) -> &VTxbSkipCdfRows {
        &self.v_txb_skip
    }

    #[cfg(test)]
    pub(crate) const fn eob_extra(&self) -> &EobExtraCdfRows {
        &self.eob_extra
    }
}

/// Averages one `eob_pt` family bank (`[coeff_cdf_q_ctx][eobCtx][N]`) against the
/// completed tile's matching bank, for any class width `N`.
fn avg_eob_pt_bank<const N: usize>(
    frame: &mut [[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
    tile: &[[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
    tile_num: u32,
    num_log2: u8,
) {
    for (frame_q, tile_q) in frame.iter_mut().zip(tile.iter()) {
        for (frame_row, tile_row) in frame_q.iter_mut().zip(tile_q.iter()) {
            avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
        }
    }
}

/// Scales the frame-end adaptation count of every row in one `eob_pt` family
/// bank, for any class width `N`.
fn scale_eob_pt_bank<const N: usize>(
    bank: &mut [[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
) {
    for q_rows in bank.iter_mut() {
        for row in q_rows.iter_mut() {
            scale_cdf_count(row);
        }
    }
}

fn checked_eob_plane_ctx(eob_ctx: usize) -> Result<usize, TileCdfError> {
    if eob_ctx >= EOB_PLANE_CTXS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::EobPt,
            index_name: "eob_ctx",
            actual: eob_ctx,
            max_exclusive: EOB_PLANE_CTXS,
        });
    }
    Ok(eob_ctx)
}

fn checked_dc_sign_group(group: usize) -> Result<usize, TileCdfError> {
    if group >= DC_SIGN_GROUPS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "group",
            actual: group,
            max_exclusive: DC_SIGN_GROUPS,
        });
    }
    Ok(group)
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

fn checked_plane_type(array: TileCdfArray, plane_type: usize) -> Result<usize, TileCdfError> {
    if plane_type >= PLANE_TYPES {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "plane_type",
            actual: plane_type,
            max_exclusive: PLANE_TYPES,
        });
    }
    Ok(plane_type)
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
