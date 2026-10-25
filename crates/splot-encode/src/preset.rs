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
    /// RDOQ also quantizes the identity and 1D transform types outside mode
    /// trials; off, they take the deadzone quantizer.
    pub rdoq_every_type: bool,
    /// Parity hiding (`enable_parity_hiding`): a luma block with enough
    /// nonzero levels hides the parity of its first level in their sum. Off
    /// in every row: its gain per CPU does not beat the next slower row.
    pub parity_hiding: bool,
    /// The frames that quantize with TCQ (`allow_tcq`); they code no parity
    /// hiding. `Base` in every row: its BD-rate per CPU beats the next slower
    /// row at every measured preset, and TCQ on the upper mini-GOP layers loses.
    pub tcq: TcqFrames,
    /// The final trial of each leaf in the partition search runs the TCQ
    /// trellis; off, it prices TCQ blocks as plain levels and only the frame
    /// write runs the trellis. Mode trials always price plain levels.
    pub tcq_search_trellis: bool,
    /// The luma transform-type search and the luma trial before the chroma
    /// search run the TCQ trellis; off, they price TCQ blocks as plain levels.
    pub tcq_type_trellis: bool,
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
    /// Loop-filter search depth. Deblocking: `0` derives the level from the
    /// qp; depth 1, 2 or 3 tries `DfDeltaQ` 0, ±1 (3 offsets), then ±2 (5),
    /// then 3, -3 and -4 (8, the whole -4..=3 sequence range of
    /// `df_par_bits` 3) per plane, by SSE after deblocking. CDEF tries 8, 12
    /// or 16 strengths at depth 0, 1 or 2; depth 3 tries 16, as depth 2 does.
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
    /// Interpolation filter search: `0` codes `EIGHTTAP` frames; `1` codes
    /// `SWITCHABLE` frames whose winning inter leaf takes the filter with
    /// the lowest luma prediction SATD plus `interp_filter` rate at its final
    /// vectors; `2` also keeps a filter other than the winner's only when a
    /// trial confirms it.
    pub interp_filter_search: u8,
    /// Sequence MVD tools `[enable_adaptive_mvd, enable_mvd_sign_derive,
    /// enable_flex_mvres]`: a `NEWMV` also tries adaptive-MVD steps and
    /// coarser block precisions, and coarse differences derive one sign.
    pub mvd_tools: [bool; 3],
    /// Extended partitions: `0` off, `1` adds `HORZ_3` and `VERT_3`, `2` also
    /// the uneven 4-way shapes.
    pub ext_partitions: u8,
    /// A three-way shape is tried only when the `NONE` cost is within this
    /// margin, in 1/256, of the best cost so far; `255` always tries it.
    pub ext_three_way_margin: u8,
    /// An uneven four-way shape is tried only when the three-way shape of its
    /// axis costs within this margin, in 1/256, of the best; `255` always.
    pub ext_four_way_margin: u8,
    /// CCSO classifications the filter search tries per plane; `0` disables
    /// CCSO in the sequence.
    pub ccso_candidates: u8,
    /// Luma Wiener-NS bank fits per frame, each on the units the previous fit
    /// took; `0` searches PC-Wiener alone.
    pub wiener_ns: u8,
    /// Wiener-NS bank fits per frame of each chroma plane; `0` codes chroma
    /// restoration off.
    pub wiener_ns_chroma: u8,
    /// The largest `NumFilterClasses` a luma bank fit tries, over the
    /// § 7.20.4 PC-Wiener classes; `1` fits one class.
    pub wiener_ns_classes: u8,
    /// MRL candidates of an intra leaf that get an RD trial besides the luma
    /// shortlist, lowest SATD first; `0` disables MRL in the sequence.
    pub mrl_trials: u8,
    /// `most_probable_stx_set` values whose three kernels a luma block ranks
    /// by kept energy in a leaf's final trial when its plain levels code
    /// `eob > 1`; the best gets one RD trial. `0` disables intra IST. Off in
    /// every row: on top of MRL its BD-rate per CPU loses at every measured
    /// preset.
    pub ist_sets: u8,
    /// A chroma pair in a leaf's final trial tries the CCTX rotation that
    /// leaves V the least energy; off disables `enable_cctx`. Off in every
    /// row: its `cctx_type` flags lose luma and chroma BD-rate.
    pub cctx: bool,
    /// The chroma candidates of a CFL leaf include the three MHCCP
    /// directions; off disables `enable_mhccp`. Off in every row: its
    /// `cfl_mhccp` flags lose luma BD-rate.
    pub mhccp: bool,
    /// The winning luma mode of an intra leaf gets one RD trial with
    /// `fsc_mode`; off disables `enable_fsc`. Off in every row: it pays only
    /// on screen content, so it waits for a screen-content detector.
    pub fsc: bool,
    /// The DIP prediction with the lowest SATD gets one RD trial when it
    /// beats the best angular SATD; off disables `enable_dip`. Off in every
    /// row: the `use_dip` flag cost offsets its wins (2026-10-25).
    pub dip: bool,
    /// Leaves wider or taller than 64 also try inter, skip-mode and compound
    /// predictions; off, they code intra only.
    pub large_inter: bool,
    /// An inter leaf tries the luma transform partitions of a `TX_MODE_SELECT`
    /// frame when one quadrant of its luma prediction residual holds more than
    /// `inter_tx_split / 64` of the energy; `0` codes `NONE` without a trial.
    pub inter_tx_split: u8,
    /// The CDEF search also tries the frame with CDEF on skipped transform
    /// blocks (`cdef_on_skip_txfm_frame_enable`) and keeps the cheaper.
    pub cdef_on_skip: bool,
}

/// The frames of [`PresetParams::tcq`], from none to all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TcqFrames {
    /// No frame.
    Off,
    /// Key frames and the frames of mini-GOP layer 0.
    Base,
    /// Every frame.
    All,
}

impl TcqFrames {
    /// Whether a frame of mini-GOP `layer` quantizes with TCQ.
    #[must_use]
    pub const fn allows(self, layer: u8) -> bool {
        match self {
            Self::Off => false,
            Self::Base => layer == 0,
            Self::All => true,
        }
    }
}

const fn row(
    blocks: (u8, u8),
    intra_modes: u8,
    partition_early_exit: bool,
    (rdoq, full_trials): (bool, bool),
    (tx_type_depth, tx_partition_search): (u8, bool),
    motion: (u16, u8, u16, u8, u8),
    (loop_filter_search, mini_gop_size, lookahead_frames, compound_candidates): (u8, u8, u8, u8),
) -> PresetParams {
    PresetParams {
        min_block_log2: blocks.0,
        max_block_log2: blocks.1,
        intra_modes,
        partition_early_exit,
        rdoq,
        rdoq_every_type: rdoq,
        parity_hiding: false,
        tcq: TcqFrames::Base,
        tcq_search_trellis: true,
        tcq_type_trellis: full_trials,
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
        interp_filter_search: motion.4,
        mvd_tools: [false; 3],
        ext_partitions: 0,
        ext_three_way_margin: 0,
        ext_four_way_margin: 0,
        ccso_candidates: 0,
        wiener_ns: 0,
        wiener_ns_chroma: 0,
        wiener_ns_classes: 1,
        mrl_trials: 0,
        ist_sets: 0,
        cctx: false,
        mhccp: false,
        fsc: false,
        dip: false,
        large_inter: true,
        inter_tx_split: if tx_partition_search { 24 } else { 0 },
        cdef_on_skip: false,
    }
}

/// Search limits indexed by preset: `0` is the slowest and best, `10` the fastest.
pub const PRESETS: [PresetParams; 11] = with_columns(
    ROWS,
    [8, 8, 6, 6, 4, 4, 4, 2, 2, 1, 1],
    [2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1],
    [2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1],
    [16, 16, 16, 16, 16, 16, 16, 4, 4, 4, 4],
    [2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
);

/// `rows` with `ccso_candidates[i]`, `wiener_ns[i]`, `wiener_ns_chroma[i]`,
/// `wiener_ns_classes[i]`, `mrl_trials[i]` and `ist_sets[i]` in row `i`.
const fn with_columns(
    mut rows: [PresetParams; 11],
    ccso_candidates: [u8; 11],
    wiener_ns: [u8; 11],
    wiener_ns_chroma: [u8; 11],
    wiener_ns_classes: [u8; 11],
    mrl_trials: [u8; 11],
    ist_sets: [u8; 11],
) -> [PresetParams; 11] {
    let mut i = 0;
    while i < rows.len() {
        rows[i].ccso_candidates = ccso_candidates[i];
        rows[i].wiener_ns = wiener_ns[i];
        rows[i].wiener_ns_chroma = wiener_ns_chroma[i];
        rows[i].wiener_ns_classes = wiener_ns_classes[i];
        rows[i].mrl_trials = mrl_trials[i];
        rows[i].ist_sets = ist_sets[i];
        i += 1;
    }
    rows
}

const ROWS: [PresetParams; 11] = [
    row(
        (2, 7),
        13,
        false,
        (true, true),
        (3, true),
        (256, 3, 128, 3, 0),
        (3, 8, 16, 8),
    ),
    row(
        (2, 7),
        13,
        false,
        (true, true),
        (3, true),
        (192, 3, 128, 3, 0),
        (3, 8, 16, 8),
    ),
    row(
        (2, 7),
        12,
        true,
        (true, true),
        (3, true),
        (128, 3, 128, 3, 0),
        (2, 8, 16, 6),
    ),
    row(
        (2, 7),
        10,
        true,
        (true, true),
        (2, true),
        (96, 2, 0, 3, 0),
        (2, 8, 16, 6),
    ),
    row(
        (2, 7),
        9,
        true,
        (true, false),
        (2, true),
        (64, 2, 0, 3, 0),
        (2, 4, 8, 4),
    ),
    row(
        (2, 7),
        8,
        true,
        (true, false),
        (2, true),
        (64, 2, 0, 3, 0),
        (1, 4, 8, 4),
    ),
    row(
        (3, 7),
        6,
        true,
        (true, false),
        (1, true),
        (48, 2, 0, 3, 0),
        (1, 4, 8, 3),
    ),
    row(
        (3, 6),
        5,
        true,
        (false, false),
        (1, false),
        (32, 1, 0, 1, 0),
        (1, 4, 8, 2),
    ),
    row(
        (3, 6),
        4,
        true,
        (false, false),
        (1, false),
        (24, 1, 0, 1, 0),
        (0, 2, 4, 1),
    ),
    row(
        (3, 6),
        3,
        true,
        (false, false),
        (0, false),
        (16, 1, 0, 1, 0),
        (0, 2, 4, 0),
    ),
    PresetParams {
        tcq_search_trellis: false,
        ..row(
            (4, 6),
            2,
            true,
            (false, false),
            (0, false),
            (16, 0, 0, 1, 0),
            (0, 1, 2, 0),
        )
    },
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
            assert!(fast.rdoq_every_type <= slow.rdoq_every_type);
            assert!(fast.parity_hiding <= slow.parity_hiding);
            assert!(fast.tcq <= slow.tcq);
            assert!(fast.tcq_search_trellis <= slow.tcq_search_trellis);
            assert!(fast.tcq_type_trellis <= slow.tcq_type_trellis);
            assert!(fast.tcq_search_trellis || !fast.tcq_type_trellis);
            assert!(fast.rdoq || !fast.rdoq_every_type);
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
            assert!(fast.interp_filter_search <= slow.interp_filter_search);
            assert!(
                fast.mvd_tools
                    .iter()
                    .zip(slow.mvd_tools)
                    .all(|(&f, s)| f <= s)
            );
            assert!(fast.ext_partitions <= slow.ext_partitions);
            assert!(fast.ext_three_way_margin <= slow.ext_three_way_margin);
            assert!(fast.ext_four_way_margin <= slow.ext_four_way_margin);
            assert!(fast.ccso_candidates <= slow.ccso_candidates);
            assert!(fast.wiener_ns <= slow.wiener_ns);
            assert!(fast.wiener_ns_chroma <= slow.wiener_ns_chroma);
            assert!(fast.wiener_ns_classes <= slow.wiener_ns_classes);
            assert!(fast.mrl_trials <= slow.mrl_trials);
            assert!(fast.ist_sets <= slow.ist_sets);
            assert!(fast.cctx <= slow.cctx);
            assert!(fast.mhccp <= slow.mhccp);
            assert!(fast.fsc <= slow.fsc);
            assert!(fast.dip <= slow.dip);
            assert!(fast.large_inter <= slow.large_inter);
            assert!(fast.cdef_on_skip <= slow.cdef_on_skip);
            assert!(
                fast.inter_tx_split == 0
                    || (slow.inter_tx_split != 0 && fast.inter_tx_split >= slow.inter_tx_split)
            );
        }
        for params in PRESETS {
            assert!((1..=8).contains(&params.mini_gop_size));
            assert!(params.lookahead_frames >= params.mini_gop_size);
            assert!((1..=5).contains(&params.drl_bits));
        }
    }

    /// The measured placement of the interpolation filter search: off in
    /// every row (see `ENC-INTERP-FILTER`); a merge must not change it
    /// silently.
    #[test]
    fn interp_filter_search_placement() {
        assert_eq!(PRESETS.map(|params| params.interp_filter_search), [0; 11]);
    }

    /// The measured placement of the MVD tools: off in every row (see
    /// `ENC-MV-CODING`).
    #[test]
    fn mvd_tools_placement() {
        assert_eq!(PRESETS.map(|params| params.mvd_tools), [[false; 3]; 11]);
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
