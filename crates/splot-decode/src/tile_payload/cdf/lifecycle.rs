// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Supported-subset AV2 Tile/Saved/Frame CDF lifecycle boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

use super::{
    CDEF_STRENGTH_INDEX0_CONTEXTS, DO_EXT_PARTITION_CONTEXTS, DO_SPLIT_CONTEXTS,
    DO_SPLIT_PLANE_CONTEXTS, DO_SQUARE_SPLIT_CONTEXTS, DO_UNEVEN_4WAY_PARTITION_CONTEXTS,
    FSC_BSIZE_CONTEXTS, FSC_MODE_CONTEXTS, FrameCdfSubset, INTRABC_CONTEXTS, MRL_INDEX_CONTEXTS,
    RECT_TYPE_CONTEXTS, SavedCdfSubset, TX_2OR3_PARTITION_TYPE_CONTEXTS, TX_FSC_CONTEXTS,
    TX_IS_INTER_CONTEXTS, TX_PARTITION_TYPE_CONTEXTS, TXFM_SPLIT_GROUPS, TileCdfRows,
    TileCdfSavePolicy, TileCdfSubset, TileCdfWorkUnitBoundary, scale_cdf_count,
};

impl FrameCdfSubset {
    /// Applies the supported § 7.5 frame-end CDF update from saved rows.
    pub(crate) fn frame_end_update_from_saved(&mut self, saved: &SavedCdfSubset) {
        self.rows = saved.rows.clone();
        self.rows.scale_counts_for_frame_end_update();
    }
}

impl SavedCdfSubset {
    /// Applies the recorded copy/average decision for a completed tile.
    pub(crate) fn apply_completed_tile(
        &mut self,
        tile_num: u32,
        tile: &TileCdfSubset,
        policy: TileCdfSavePolicy,
    ) {
        if policy.copy_cdf {
            self.rows.copy_from_tile(&tile.rows);
            return;
        }
        if policy.avg_cdf {
            self.rows
                .avg_from_tile(tile_num, &tile.rows, policy.num_log2);
        }
    }
}

impl TileCdfWorkUnitBoundary {
    /// Applies the completed tile-local CDF subset to Saved CDF rows.
    pub(crate) fn apply_completed_tile_to_saved(&mut self, tile_num: u32) {
        self.saved_cdfs
            .apply_completed_tile(tile_num, &self.tile_cdfs, self.save_policy);
    }

    /// Applies the supported subset `frame_end_update_cdf()` to Frame CDF rows.
    pub(crate) fn frame_end_update_cdf_subset(&mut self) {
        self.frame_cdfs
            .frame_end_update_from_saved(&self.saved_cdfs);
    }
}

impl TileCdfRows {
    fn copy_from_tile(&mut self, tile: &Self) {
        *self = tile.clone();
    }

    fn scale_counts_for_frame_end_update(&mut self) {
        for plane in 0..DO_SPLIT_PLANE_CONTEXTS {
            for ctx in 0..DO_SPLIT_CONTEXTS {
                scale_cdf_count(&mut self.do_split[plane][ctx]);
            }
            for ctx in 0..DO_EXT_PARTITION_CONTEXTS {
                scale_cdf_count(&mut self.do_ext_partition[plane][ctx]);
            }
            for ctx in 0..DO_SQUARE_SPLIT_CONTEXTS {
                scale_cdf_count(&mut self.do_square_split[plane][ctx]);
            }
            for ctx in 0..RECT_TYPE_CONTEXTS {
                scale_cdf_count(&mut self.rect_type[plane][ctx]);
            }
            for ctx in 0..DO_UNEVEN_4WAY_PARTITION_CONTEXTS {
                scale_cdf_count(&mut self.do_uneven_4way_partition[plane][ctx]);
            }
        }
        for fsc_mode in 0..TX_FSC_CONTEXTS {
            for is_inter in 0..TX_IS_INTER_CONTEXTS {
                for ctx in 0..TXFM_SPLIT_GROUPS {
                    scale_cdf_count(&mut self.tx_do_partition[fsc_mode][is_inter][ctx]);
                }
                for ctx in 0..TX_2OR3_PARTITION_TYPE_CONTEXTS {
                    scale_cdf_count(&mut self.tx_2or3_partition_type[fsc_mode][is_inter][ctx]);
                }
                for ctx in 0..TX_PARTITION_TYPE_CONTEXTS {
                    scale_cdf_count(&mut self.tx_partition_type[fsc_mode][is_inter][ctx]);
                    scale_cdf_count(&mut self.tx_partition_type_reduced[fsc_mode][is_inter][ctx]);
                }
            }
        }
        scale_cdf_count(&mut self.delta_q);
        for ctx in 0..CDEF_STRENGTH_INDEX0_CONTEXTS {
            scale_cdf_count(&mut self.cdef_index0[ctx]);
        }
        scale_cdf_count(&mut self.cdef_index_minus1_with3);
        scale_cdf_count(&mut self.cdef_index_minus1_with4);
        scale_cdf_count(&mut self.cdef_index_minus1_with5);
        scale_cdf_count(&mut self.cdef_index_minus1_with6);
        scale_cdf_count(&mut self.cdef_index_minus1_with7);
        scale_cdf_count(&mut self.cdef_index_minus1_with8);
        for ctx in 0..INTRABC_CONTEXTS {
            scale_cdf_count(&mut self.intrabc[ctx]);
        }
        scale_cdf_count(&mut self.intrabc_mode);
        scale_cdf_count(&mut self.intrabc_precision);
        for ctx in 0..FSC_MODE_CONTEXTS {
            for bsize_group in 0..FSC_BSIZE_CONTEXTS {
                scale_cdf_count(&mut self.fsc_mode[ctx][bsize_group]);
            }
        }
        for ctx in 0..MRL_INDEX_CONTEXTS {
            scale_cdf_count(&mut self.mrl_index[ctx]);
            scale_cdf_count(&mut self.mrl_sec_index[ctx]);
        }
        self.block.scale_counts_for_frame_end_update();
    }
}
