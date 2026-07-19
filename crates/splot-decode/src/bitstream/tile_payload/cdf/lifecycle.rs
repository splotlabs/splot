// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Supported-subset AV2 Tile/Saved/Frame CDF lifecycle boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

use super::{
    FrameCdfSubset, SavedCdfSubset, TileCdfRows, TileCdfSavePolicy, TileCdfSubset, scale_cdf_count,
    scale_cdf_rows,
};

impl FrameCdfSubset {
    /// Builds the frame-end updated bank. `None` means no tile was saved,
    /// in which case the saved bank would still equal the untouched frame
    /// bank, so only the count scaling applies.
    #[must_use]
    pub(crate) fn frame_end_updated(frame: &Self, saved: Option<SavedCdfSubset>) -> Self {
        let mut rows = match saved {
            Some(saved) => saved.rows,
            None => frame.rows.clone(),
        };
        rows.scale_counts_for_frame_end_update();
        Self { rows }
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

impl TileCdfRows {
    fn scale_counts_for_frame_end_update(&mut self) {
        macro_rules! scale_row {
            ($field:ident) => {
                scale_cdf_count(&mut self.$field);
            };
        }
        macro_rules! scale_rows {
            ($field:ident $(. $flatten:ident())*) => {
                scale_cdf_rows(flat_cdf_rows_mut!(self.$field $(, $flatten)*));
            };
        }

        tile_cdf_common_count_rows!(scale_row, scale_rows);
        self.block.scale_counts_for_frame_end_update();
    }
}
