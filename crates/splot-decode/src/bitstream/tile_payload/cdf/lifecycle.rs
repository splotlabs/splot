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
    /// Applies the frame-end update. `None` means no tile was saved, in
    /// which case the saved bank would still equal this untouched frame
    /// bank, so only the count scaling applies.
    pub(crate) fn frame_end_update_from_saved(&mut self, saved: Option<SavedCdfSubset>) {
        if let Some(saved) = saved {
            self.rows = saved.rows;
        }
        self.rows.scale_counts_for_frame_end_update();
    }
}

impl SavedCdfSubset {
    /// Applies one completed tile under `policy`, materializing the saved
    /// bank only when the policy actually writes it: a copy policy replaces
    /// it with the tile bank outright, and an averaging policy first seeds
    /// it from the (still untouched) frame bank.
    pub(crate) fn apply_completed_tile(
        slot: &mut Option<SavedCdfSubset>,
        frame: &FrameCdfSubset,
        tile_num: u32,
        tile: &TileCdfSubset,
        policy: TileCdfSavePolicy,
    ) {
        if policy.copy_cdf {
            *slot = Some(SavedCdfSubset {
                rows: tile.rows.clone(),
            });
            return;
        }
        if policy.avg_cdf {
            slot.get_or_insert_with(|| SavedCdfSubset::from_frame(frame))
                .rows
                .avg_from_tile(tile_num, &tile.rows, policy.num_log2);
        }
    }
}

impl TileCdfWorkUnitBoundary {
    pub(crate) fn apply_completed_tile_to_saved(&mut self, tile_num: u32) {
        SavedCdfSubset::apply_completed_tile(
            &mut self.saved_cdfs,
            &self.frame_cdfs,
            tile_num,
            &self.tile_cdfs,
            self.save_policy,
        );
    }

    pub(crate) fn frame_end_update_cdf_subset(&mut self) {
        let saved = self.saved_cdfs.take();
        self.frame_cdfs.frame_end_update_from_saved(saved);
    }
}

impl TileCdfRows {
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
