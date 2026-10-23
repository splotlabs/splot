// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Speed presets. Each preset is one row of [`PRESETS`]; encoder stages read the
//! row's fields instead of comparing preset numbers.

use crate::error::{Error, Result};

/// Search limits of one speed preset. These are encoder policy, not AV2 syntax.
///
/// The intra-only DC coder reads only the block-size limits; the other fields
/// take effect as mode, transform, motion and loop-filter search land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetParams {
    /// log2 of the smallest square block size searched (2 = 4x4).
    pub min_block_log2: u8,
    /// log2 of the largest square block size searched (7 = 128x128).
    pub max_block_log2: u8,
    /// Number of luma intra modes tried per block, best-ranked first.
    pub intra_modes: u8,
    /// Partition search skips a block's remaining partitions once its unsplit
    /// cost does not exceed the best split cost so far.
    pub partition_early_exit: bool,
    /// Rate-distortion optimized quantization.
    pub rdoq: bool,
    /// Candidate trials of the mode and transform-partition search run RDOQ
    /// and the transform-type search too; off, they quantize with a deadzone
    /// and code `DCT_DCT`, and only a leaf's final trial runs both.
    pub full_trials: bool,
    /// Luma transform-type search depth: depth `d` tries the first `2 * d + 1`
    /// types a block offers, most probable first. `0` codes every intra luma
    /// block with `DCT_DCT` and signals no type (`reduced_tx_set = 2`).
    pub tx_type_depth: u8,
    /// Luma transform-partition search (`TX_MODE_SELECT`); off codes every
    /// block with its largest transform (`TX_MODE_LARGEST`).
    pub tx_partition_search: bool,
    /// Full-pel motion search radius in luma samples.
    pub motion_range: u16,
    /// Sub-pel refinement depth: `0` full-pel, `1` half, `2` quarter, `3` eighth.
    pub subpel_depth: u8,
    /// The largest frame `base_q_idx` that codes eighth-pel vectors when
    /// `subpel_depth` is `3`; other inter frames code quarter-pel vectors and
    /// refine to quarter samples. `0` never codes eighth-pel.
    pub eighth_pel_max_qindex: u16,
    /// Loop-filter levels tried per frame; `0` derives the level from the qp.
    pub loop_filter_search: u8,
    /// Frames per mini-GOP, `1..=8`: `1` codes every frame shown (IPPP);
    /// larger sizes code the last frame first as a hidden alt-ref and the
    /// frames between hierarchically.
    pub mini_gop_size: u8,
    /// Frames the lookahead analyses before the next mini-GOP is coded, at
    /// least `mini_gop_size`; scene cuts and the temporal model see this far.
    pub lookahead_frames: u8,
    /// Compound candidates of an inter leaf that get a full RD trial, best
    /// prediction SATD first; skip-mode candidates are tried besides them.
    /// `0` codes neither compound nor skip-mode blocks.
    pub compound_candidates: u8,
    /// Inter frames start from the saved CDFs of a reference frame instead of
    /// the default CDFs, and the sequence enables CDF averaging.
    pub cdf_carry: bool,
    /// Inter frames predict motion vectors from the projected motion fields
    /// of their references and from the reference MV bank, and reorder the
    /// DRL candidates.
    pub mv_prediction: bool,
    /// `MaxDrlBits`, `1..=5`: `NEARMV` and `NEWMV` choose among the first
    /// `drl_bits + 1` entries of the reference MV stack.
    pub drl_bits: u8,
}

const fn row(
    blocks: (u8, u8),
    intra_modes: u8,
    partition_early_exit: bool,
    (rdoq, full_trials): (bool, bool),
    (tx_type_depth, tx_partition_search): (u8, bool),
    motion: (u16, u8, u16, u8),
    (loop_filter_search, mini_gop_size, lookahead_frames, compound_candidates): (u8, u8, u8, u8),
) -> PresetParams {
    PresetParams {
        min_block_log2: blocks.0,
        max_block_log2: blocks.1,
        intra_modes,
        partition_early_exit,
        rdoq,
        full_trials,
        tx_type_depth,
        tx_partition_search,
        motion_range: motion.0,
        subpel_depth: motion.1,
        eighth_pel_max_qindex: motion.2,
        loop_filter_search,
        mini_gop_size,
        lookahead_frames,
        compound_candidates,
        cdf_carry: true,
        mv_prediction: true,
        drl_bits: motion.3,
    }
}

/// Search limits indexed by preset: `0` is the slowest and best, `10` the fastest.
pub const PRESETS: [PresetParams; 11] = [
    row(
        (2, 7),
        13,
        false,
        (true, true),
        (3, true),
        (256, 3, 128, 3),
        (3, 8, 16, 8),
    ),
    row(
        (2, 7),
        13,
        false,
        (true, true),
        (3, true),
        (192, 3, 128, 3),
        (3, 8, 16, 8),
    ),
    row(
        (2, 7),
        12,
        true,
        (true, true),
        (3, true),
        (128, 3, 128, 3),
        (2, 8, 16, 6),
    ),
    row(
        (2, 7),
        10,
        true,
        (true, true),
        (2, true),
        (96, 2, 0, 3),
        (2, 8, 16, 6),
    ),
    row(
        (2, 7),
        9,
        true,
        (true, false),
        (2, true),
        (64, 2, 0, 3),
        (2, 4, 8, 4),
    ),
    row(
        (2, 7),
        8,
        true,
        (true, false),
        (2, true),
        (64, 2, 0, 3),
        (1, 4, 8, 4),
    ),
    row(
        (3, 7),
        6,
        true,
        (true, false),
        (1, true),
        (48, 2, 0, 3),
        (1, 4, 8, 3),
    ),
    row(
        (3, 6),
        5,
        true,
        (false, false),
        (1, false),
        (32, 1, 0, 1),
        (1, 4, 8, 2),
    ),
    row(
        (3, 6),
        4,
        true,
        (false, false),
        (1, false),
        (24, 1, 0, 1),
        (0, 2, 4, 1),
    ),
    row(
        (3, 6),
        3,
        true,
        (false, false),
        (0, false),
        (16, 1, 0, 1),
        (0, 2, 4, 0),
    ),
    row(
        (4, 6),
        2,
        true,
        (false, false),
        (0, false),
        (16, 0, 0, 1),
        (0, 1, 2, 0),
    ),
];

/// A speed preset, `0..=10`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Preset(u8);

impl Preset {
    /// The fastest preset value.
    pub const MAX: u8 = PRESETS.len() as u8 - 1;

    /// Creates a preset.
    ///
    /// # Errors
    /// Returns [`Error::InvalidPreset`] when `value` exceeds [`Self::MAX`].
    pub const fn new(value: u8) -> Result<Self> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(Error::InvalidPreset { preset: value })
        }
    }

    /// Returns the preset number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Returns the search limits of this preset.
    #[must_use]
    pub const fn params(self) -> &'static PresetParams {
        &PRESETS[self.0 as usize]
    }
}

impl Default for Preset {
    fn default() -> Self {
        Self(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_monotone_from_slow_to_fast() {
        for pair in PRESETS.windows(2) {
            let (slow, fast) = (&pair[0], &pair[1]);
            assert!(fast.min_block_log2 >= slow.min_block_log2);
            assert!(fast.max_block_log2 <= slow.max_block_log2);
            assert!(fast.min_block_log2 <= fast.max_block_log2);
            assert!(fast.intra_modes <= slow.intra_modes);
            assert!(fast.partition_early_exit >= slow.partition_early_exit);
            assert!(fast.rdoq <= slow.rdoq);
            assert!(fast.full_trials <= slow.full_trials);
            assert!(fast.tx_type_depth <= slow.tx_type_depth);
            assert!(fast.tx_partition_search <= slow.tx_partition_search);
            assert!(fast.motion_range <= slow.motion_range);
            assert!(fast.subpel_depth <= slow.subpel_depth);
            assert!(fast.eighth_pel_max_qindex <= slow.eighth_pel_max_qindex);
            assert!(fast.loop_filter_search <= slow.loop_filter_search);
            assert!(fast.mini_gop_size <= slow.mini_gop_size);
            assert!(fast.lookahead_frames <= slow.lookahead_frames);
            assert!(fast.compound_candidates <= slow.compound_candidates);
            assert!(fast.cdf_carry <= slow.cdf_carry);
            assert!(fast.mv_prediction <= slow.mv_prediction);
            assert!(fast.drl_bits <= slow.drl_bits);
        }
        for params in PRESETS {
            assert!((1..=8).contains(&params.mini_gop_size));
            assert!(params.lookahead_frames >= params.mini_gop_size);
            assert!((1..=5).contains(&params.drl_bits));
        }
    }

    #[test]
    fn rejects_out_of_range_preset() {
        assert!(Preset::new(Preset::MAX).is_ok());
        assert!(matches!(
            Preset::new(Preset::MAX + 1),
            Err(Error::InvalidPreset { preset: 11 })
        ));
    }
}
