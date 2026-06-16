// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Supported-subset AV2 Tile/Saved/Frame CDF lifecycle boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

use super::{
    DO_EXT_PARTITION_CONTEXTS, DO_SPLIT_CONTEXTS, DO_SPLIT_PLANE_CONTEXTS,
    DO_SQUARE_SPLIT_CONTEXTS, DO_UNEVEN_4WAY_PARTITION_CONTEXTS, FrameCdfSubset,
    RECT_TYPE_CONTEXTS, SavedCdfSubset, TileCdfRows, TileCdfSavePolicy, TileCdfSubset,
    TileCdfWorkUnitBoundary, scale_cdf_count,
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
        self.block.scale_counts_for_frame_end_update();
    }
}
