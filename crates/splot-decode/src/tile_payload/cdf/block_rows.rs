// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal block-symbol CDF rows for the traced runtime frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::tables::cdf::{
    DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF, DEFAULT_V_TXB_SKIP_CDF,
    DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use super::{CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_row, scale_cdf_count};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const PLANE_TYPES: usize = 2;
const TX_SIZE_CONTEXTS: usize = 5;
const TXB_SKIP_CONTEXTS: usize = 10;
const UV_MODE_CONTEXTS: usize = 2;
const V_TXB_SKIP_CONTEXTS: usize = 12;

pub(crate) type YModeSetCdfRow = [i32; Y_MODE_SET_CDF_ROW_LEN];
pub(crate) type YModeIndexCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; Y_MODE_INDEX_CONTEXTS];
pub(crate) type TxbSkipCdfRows = [[[[[i32; CDF_ROW_LEN]; TXB_SKIP_CONTEXTS]; TX_SIZE_CONTEXTS];
    PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type UvModeCflNotAllowedCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; UV_MODE_CONTEXTS];
pub(crate) type VTxbSkipCdfRows = [[[i32; CDF_ROW_LEN]; V_TXB_SKIP_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];

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
}

/// Supported block-symbol CDF arrays for the minimal flat-intra trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockCdfRows {
    pub(super) y_mode_set: YModeSetCdfRow,
    pub(super) y_mode_index: YModeIndexCdfRows,
    pub(super) txb_skip: TxbSkipCdfRows,
    pub(super) uv_mode_cfl_not_allowed: UvModeCflNotAllowedCdfRows,
    pub(super) v_txb_skip: VTxbSkipCdfRows,
}

impl BlockCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index: DEFAULT_Y_MODE_INDEX_CDF,
            txb_skip: DEFAULT_TXB_SKIP_CDF,
            uv_mode_cfl_not_allowed: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF,
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
                let plane_type = checked_plane_type(plane_type)?;
                let tx_size = checked_tx_size(tx_size)?;
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
                let plane_type = checked_plane_type(plane_type)?;
                let tx_size = checked_tx_size(tx_size)?;
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
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.uv_mode_cfl_not_allowed[ctx],
                &tile.uv_mode_cfl_not_allowed[ctx],
                tile_num,
                num_log2,
            );
        }
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
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            scale_cdf_count(&mut self.uv_mode_cfl_not_allowed[ctx]);
        }
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

fn checked_plane_type(plane_type: usize) -> Result<usize, TileCdfError> {
    if plane_type >= PLANE_TYPES {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::TxbSkip,
            index_name: "plane_type",
            actual: plane_type,
            max_exclusive: PLANE_TYPES,
        });
    }
    Ok(plane_type)
}

fn checked_tx_size(tx_size: usize) -> Result<usize, TileCdfError> {
    if tx_size >= TX_SIZE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::TxbSkip,
            index_name: "tx_size",
            actual: tx_size,
            max_exclusive: TX_SIZE_CONTEXTS,
        });
    }
    Ok(tx_size)
}
