// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Supported-subset AV2 Tile/Saved/Frame CDF lifecycle boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

#[cfg(test)]
use super::SavedCdfSubset;
use super::{
    FrameCdfSubset, TileCdfRows, TileCdfSavePolicy, TileCdfSubset, scale_cdf_count, scale_cdf_rows,
};

impl FrameCdfSubset {
    /// Builds the frame-end updated bank. `None` means no tile was saved,
    /// in which case the saved bank would still equal the untouched frame
    /// bank, so only the count scaling applies.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn frame_end_updated(frame: &Self, saved: Option<SavedCdfSubset>) -> Self {
        let mut rows = match saved {
            Some(saved) => saved.rows,
            None => frame.rows.clone(),
        };
        rows.scale_counts_for_frame_end_update();
        Self { rows }
    }

    pub(crate) fn reset_output_from(&mut self, frame: &Self, base_q_idx: u32) {
        self.rows.clone_from(&frame.rows);
        self.replicate_coeff_q_context_for_base_q(base_q_idx);
    }

    pub(crate) fn reset_saved_from_tile(
        &mut self,
        frame: &Self,
        tile_num: u32,
        tile: &TileCdfSubset,
        policy: TileCdfSavePolicy,
        saved: &mut bool,
    ) {
        if policy.copy_cdf {
            self.rows.clone_from(&tile.rows);
            *saved = true;
        } else if policy.avg_cdf {
            if !*saved {
                self.rows.clone_from(&frame.rows);
                *saved = true;
            }
            self.rows
                .avg_from_tile(tile_num, &tile.rows, policy.num_log2);
        }
    }

    pub(crate) fn finish_saved(&mut self, frame: &Self, saved: bool, base_q_idx: u32) {
        if !saved {
            self.rows.clone_from(&frame.rows);
        }
        self.rows.scale_counts_for_frame_end_update();
        self.replicate_coeff_q_context_for_base_q(base_q_idx);
    }
}

#[cfg(test)]
impl SavedCdfSubset {
    /// Applies one completed tile under `policy`, materializing the saved
    /// bank only when the policy actually writes it: a copy policy replaces
    /// it with the tile bank outright, and an averaging policy first seeds
    /// it from the (still untouched) frame bank.
    #[cfg(test)]
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
