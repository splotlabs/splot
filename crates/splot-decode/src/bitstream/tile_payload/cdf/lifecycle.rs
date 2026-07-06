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

        scale_rows!(do_split.flatten());
        scale_rows!(do_ext_partition.flatten());
        scale_rows!(do_square_split.flatten());
        scale_rows!(rect_type.flatten());
        scale_rows!(do_uneven_4way_partition.flatten());
        scale_rows!(tx_do_partition.flatten().flatten());
        scale_rows!(tx_2or3_partition_type.flatten().flatten());
        scale_rows!(tx_partition_type.flatten().flatten());
        scale_rows!(tx_partition_type_reduced.flatten().flatten());
        scale_row!(delta_q);
        scale_rows!(cdef_index0);
        scale_rows!(ccso_blk.flatten());
        scale_row!(cdef_index_minus1_with3);
        scale_row!(cdef_index_minus1_with4);
        scale_row!(cdef_index_minus1_with5);
        scale_row!(cdef_index_minus1_with6);
        scale_row!(cdef_index_minus1_with7);
        scale_row!(cdef_index_minus1_with8);
        scale_rows!(intrabc);
        scale_row!(intrabc_mode);
        scale_row!(intrabc_precision);
        scale_rows!(morph_pred);
        scale_rows!(fsc_mode.flatten());
        scale_rows!(mrl_index);
        scale_rows!(mrl_sec_index);
        scale_rows!(region_type);
        self.block.scale_counts_for_frame_end_update();
    }
}
