// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Supported-subset AV2 Tile/Saved/Frame CDF lifecycle boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

use super::{
    FrameCdfSubset, SavedCdfSubset, TileCdfRows, TileCdfSavePolicy, TileCdfSubset,
    TileCdfWorkUnitBoundary, scale_cdf_count, scale_cdf_rows,
};

impl FrameCdfSubset {
    pub(crate) fn frame_end_update_from_saved(&mut self, saved: &SavedCdfSubset) {
        self.rows = saved.rows.clone();
        self.rows.scale_counts_for_frame_end_update();
    }
}

impl SavedCdfSubset {
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
    pub(crate) fn apply_completed_tile_to_saved(&mut self, tile_num: u32) {
        self.saved_cdfs
            .apply_completed_tile(tile_num, &self.tile_cdfs, self.save_policy);
    }

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
        macro_rules! scale_row {
            ($field:ident) => {
                scale_cdf_count(&mut self.$field);
            };
        }
        macro_rules! scale_rows {
            ($field:ident $(. $flatten:ident())*) => {
                scale_cdf_rows(self.$field.iter_mut()$(.$flatten())*);
            };
        }

        tile_cdf_common_count_rows!(scale_row, scale_rows);
        self.block.scale_counts_for_frame_end_update();
    }
}
