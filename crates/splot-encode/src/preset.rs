// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Speed presets. Each preset is one column of [`PRESETS`]; encoder stages read the
//! preset's fields instead of comparing preset numbers.

use crate::error::{Error, Result};

/// Search limits of one speed preset. These are encoder policy, not AV2 syntax.
///
/// The intra-only DC coder reads only the block-size limits; the other fields
/// take effect as mode, transform, motion and loop-filter search land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetParams {
    /// log2 of the smallest square block size searched (2 = 4x4).
    pub min_block_log2: u8,
    /// log2 of the largest square block size searched (7 = 128x128); `8`
    /// also codes 256x256 superblocks in frames at least `sb256_min_height`
    /// tall, whose inter frames search 256x256 leaves (intra frames keep
    /// 128x128 superblocks, § 6.4.3).
    pub max_block_log2: u8,
    /// The smallest frame height, at least 64, that codes 256x256
    /// superblocks when `max_block_log2` is `8`.
    pub sb256_min_height: u16,
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
    /// Strength in percent of the block-level temporal model: each
    /// superblock's qindex and lambda move with the importance of its
    /// lookahead blocks over the frame's, coded as `delta_q`. `0` is off.
    /// `100` in every row (2026-10-26, CRF, vs main 9ec81da32): p6 niklas
    /// f30 Y -0.86%, paris f30 -1.92%, park_joy f10 +0.06% at 0.95-1.0x CPU.
    pub tpl_delta_q: u8,
    /// `delta_q_res` of a frame with superblock qindex offsets: the qindex
    /// moves in steps of `1 << delta_q_res`, `0..=3`. `1` in every row:
    /// `2` lost 0.8% on park_joy p6 and gained 0.5% on paris.
    pub delta_q_res: u8,
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
    /// Semi-decoupled partitioning (`enable_sdp`): each 64x64 node of an
    /// intra frame searches its luma tree on luma alone, then a chroma tree
    /// that only merges luma leaves. On in every row: intra frames gain luma
    /// BD-rate, with mixed YUV, at 0.62x to 0.87x of their CPU (2026-10-26).
    pub sdp: bool,
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
    /// on screen content, whose preset rows turn it on.
    pub fsc: bool,
    /// The first frame switches the sequence to the screen-content row of
    /// its preset (FSC, palette, IntraBC and integer motion vectors on) when
    /// it looks like screen content.
    pub screen_detect: bool,
    /// The winning luma mode of an intra leaf gets RD trials against
    /// `DC_PRED` palettes of 2 to 8 of the leaf's most frequent luma values;
    /// off disables `allow_screen_content_tools`. Off in every row: it pays
    /// only on screen content, whose preset rows turn it on.
    pub palette: bool,
    /// Key frames code `allow_intrabc`, and each leaf of at most 64x64
    /// (other than 64x64) gets an RD trial of the closest earlier copy of
    /// its luma that the block hashes find. Off in every row: it pays only
    /// on screen content, whose preset rows turn it on.
    pub intrabc: bool,
    /// Inter frames code `force_integer_mv = 1`: whole-sample motion vectors
    /// and a whole-sample motion search. Off in every row: it pays only on
    /// screen content, whose preset rows turn it on.
    pub integer_mv: bool,
    /// The first frame picks the sequence's `cfl_ds_filter_index` by the
    /// down-sampled luma that best predicts its chroma; off codes `0`.
    pub cfl_filter: bool,
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
    /// GDF `(gdf_pic_qc_idx, gdf_pic_scale_idx)` pairs the filter search
    /// tries per frame; `0` disables GDF in the sequence. All 16 in every
    /// row: fewer pairs lost BD-rate, and each pair costs one filter-chain
    /// run (paris -1.35% Y at p6, -1.28% Y at p10 for +3% CPU).
    pub gdf_candidates: u8,
    /// The CDEF search also tries the frame with CDEF on skipped transform
    /// blocks (`cdef_on_skip_txfm_frame_enable`) and keeps the cheaper. Off
    /// in every row: paris p6 Y -0.04%, park_joy p6 Y +0.42%.
    pub cdef_on_skip: bool,
    /// `enable_inter_ddt`: inter ADST and FDST sides of length 8 or 16
    /// transform with the DDT kernels. Off in every row (2026-10-25): it
    /// measured BD-rate neutral at p6 (park_joy -0.00%, niklas +0.03%).
    pub inter_ddt: bool,
    /// `enable_inter_ist`: in a leaf's final trial, a `DCT_DCT` inter luma
    /// block of at least 16x16 ranks the three secondary kernels by kept
    /// energy and gives the best one an RD trial. Off in every row
    /// (2026-10-25): every such block with `eob > 3` then pays a
    /// `sec_tx_type` symbol, and p6 measured park_joy -0.06%, paris +0.34%.
    pub inter_ist: bool,
    /// TIP (`enable_tip`), `0..=2`: `1` inter frames with reference MVs and
    /// a TIP reference pair code `TIP_FRAME_AS_REF` with equal weights, and
    /// inter leaves also try the TIP `NEARMV` candidates; `2` also picks the
    /// frame's `tip_global_wtd_index` by the luma SSE of the TIP prediction
    /// over every other 64x64 block.
    pub tip: u8,
    /// TIP output (`disable_tip_output = 0`), `0..=200`: with `tip`, a shown
    /// non-reference frame with a TIP reference pair codes an
    /// `OBU_REGULAR_TIP` `TIP_FRAME_AS_OUTPUT` frame when the RD cost of the
    /// TIP output times 100 is at most this value times the RD cost of the
    /// frame coded normally; `0` is off. `100` in every row (2026-10-26):
    /// p6 measured park_joy -0.19%, paris -0.12%, niklas 0.00%.
    pub tip_output: u8,
    /// Optical-flow refinement (`enable_opfl_refine = REFINE_AUTO`): inter
    /// frames with a past and a future reference code `REFINE_SWITCHABLE`,
    /// so compound blocks may code `use_optflow`; others code `REFINE_NONE`.
    /// On at p0-p9 (2026-10-30): p6 measured park_joy -0.05%, paris -0.92%,
    /// niklas -0.22%; off at p10, which codes no compound blocks.
    pub opfl: bool,
    /// Decoder-side vector refinement (`enable_refinemv`): compound blocks
    /// with a past and a future reference may code `use_refinemv`. Off in
    /// every row (2026-10-30): alone it measured niklas +0.23% at p6, largely
    /// from the refinement the decoder infers on `NEAR_NEARMV` blocks.
    pub refinemv: bool,
    /// TIP refinement (`enable_tip_refinemv`): with `tip` and `opfl` or
    /// `refinemv`, TIP blocks and TIP output frames refine their projected
    /// vectors, and frames with a past and a future reference code no TIP
    /// weight; the `tip = 2` weight search is skipped. On at p0-p9
    /// (2026-10-30): p6 measured park_joy -0.92%, paris -0.86%, niklas
    /// -0.12% over OPFL alone; off at p10, which codes no optical flow.
    pub tip_refinemv: bool,
    /// Warped motion (`seq_enabled_motion_modes` `LOCALWARP`, `DELTAWARP`,
    /// `EXTENDWARP`), `0..=2`: inter frames of a layer that picks warp code
    /// `allow_warpmv_mode`, and a leaf gives this many warp models of its best
    /// translational reference an RD trial when their luma SATD plus rate
    /// beats its `NEWMV`. Off in every row (2026-10-30): at `1` p6 measured
    /// paris -0.57%, park_joy +0.63%, niklas +0.02% at about +9% CPU, and
    /// p0 park_joy +0.20%.
    pub warp: u8,
    /// Compound types beyond `COMPOUND_AVERAGE`, `0..=2`: `1` searches wedge
    /// and difference-weighted masks (`enable_masked_compound`), `2` also
    /// `cwp_idx` weights (`enable_cwp`). `1` on p0-p8, the rows with compound
    /// candidates (2026-10-30, p6 vs main): park_joy -5.70%, paris -0.82%,
    /// niklas -1.04% for 11% more CPU. `2` measured paris -1.00% and
    /// park_joy -5.51% at the same CPU, so the weights stay off.
    pub compound_types: u8,
    /// `enable_imp_msk_bld`: `COMPOUND_AVERAGE` and TIP blocks blend with the
    /// implicit mask the decoder derives; no search. Off in every row
    /// (2026-10-30): on alone it measured park_joy +1.26%, paris +0.20% at
    /// p6, as the compound search scores the plain average.
    pub implicit_mask: bool,
    /// Joint `NEW_NEWMV` refinement: each compound `NEW_NEWMV` survivor
    /// moves its two vectors by half-sample steps, one list at a time, while
    /// the SATD of their average plus the vector rate drops. Off in every row
    /// (2026-10-30): alone it measured paris +0.13% at p6 for 2.7% more CPU.
    pub joint_newmv: bool,
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

/// A preset field override that [`PresetParams::set`] rejects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresetError {
    /// No [`PresetParams`] field has this name.
    #[error("unknown preset field `{name}`; valid fields: {}", PresetParams::FIELDS.join(", "))]
    UnknownField {
        /// The requested name.
        name: String,
    },
    /// The value does not parse as the field's type or is outside its range.
    #[error("preset field `{field}` does not accept `{value}`; expected {expected}")]
    InvalidValue {
        /// The field.
        field: &'static str,
        /// The rejected value.
        value: String,
        /// The values the field accepts.
        expected: &'static str,
    },
    /// A field is outside the range the encoder supports, alone or against
    /// another field.
    #[error("preset field `{field}` = {value} is outside {allowed}")]
    OutOfRange {
        /// The field.
        field: &'static str,
        /// Its value.
        value: u16,
        /// The supported range.
        allowed: String,
    },
}

impl PresetParams {
    /// Checks the ranges the encoder supports, per field and across fields;
    /// every [`PRESETS`] row passes, a row with overrides may not.
    ///
    /// # Errors
    /// Returns [`PresetError::OutOfRange`] for the first field outside its range.
    pub fn validate(&self) -> core::result::Result<(), PresetError> {
        let check = |field, value: u8, min: u8, max: u8, bound: &str| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(PresetError::OutOfRange {
                    field,
                    value: value.into(),
                    allowed: format!("{min}..={max}{bound}"),
                })
            }
        };
        check("max_block_log2", self.max_block_log2, 2, 8, "")?;
        if self.sb256_min_height < 64 {
            return Err(PresetError::OutOfRange {
                field: "sb256_min_height",
                value: self.sb256_min_height,
                allowed: "64..=65535".to_owned(),
            });
        }
        check(
            "min_block_log2",
            self.min_block_log2,
            2,
            self.max_block_log2,
            " (max_block_log2)",
        )?;
        check("intra_modes", self.intra_modes, 1, u8::MAX, "")?;
        check("subpel_depth", self.subpel_depth, 0, 3, "")?;
        check("loop_filter_search", self.loop_filter_search, 0, 3, "")?;
        check("mini_gop_size", self.mini_gop_size, 1, 8, "")?;
        check(
            "lookahead_frames",
            self.lookahead_frames,
            self.mini_gop_size,
            u8::MAX,
            " (mini_gop_size..)",
        )?;
        check("drl_bits", self.drl_bits, 1, 5, "")?;
        check("tpl_delta_q", self.tpl_delta_q, 0, 200, "")?;
        check("delta_q_res", self.delta_q_res, 0, 3, "")?;
        check("interp_filter_search", self.interp_filter_search, 0, 2, "")?;
        check("ext_partitions", self.ext_partitions, 0, 2, "")?;
        check("wiener_ns_classes", self.wiener_ns_classes, 1, u8::MAX, "")?;
        check("gdf_candidates", self.gdf_candidates, 0, 16, "")?;
        check("tip", self.tip, 0, 2, "")?;
        check("tip_output", self.tip_output, 0, 200, "")?;
        check(
            "tip_refinemv",
            u8::from(self.tip_refinemv),
            0,
            u8::from(self.opfl || self.refinemv),
            " (1 needs opfl or refinemv)",
        )?;
        check("warp", self.warp, 0, 2, "")?;
        check(
            "compound_types",
            self.compound_types,
            0,
            MAX_COMPOUND_TYPES,
            "",
        )
    }
}

/// The largest [`PresetParams::compound_types`]; tests also take
/// [`crate::mode_decision::FORCED_COMPOUND_TYPES`].
#[cfg(not(test))]
const MAX_COMPOUND_TYPES: u8 = 2;
#[cfg(test)]
const MAX_COMPOUND_TYPES: u8 = crate::mode_decision::FORCED_COMPOUND_TYPES;

/// A [`PresetParams`] field type that [`PresetParams::set`] parses.
trait FieldValue: Sized {
    /// The accepted values, for [`PresetError::InvalidValue`].
    const EXPECTED: &'static str;
    fn parse(value: &str) -> Option<Self>;
}

impl FieldValue for u8 {
    const EXPECTED: &'static str = "an integer in 0..=255";
    fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

impl FieldValue for u16 {
    const EXPECTED: &'static str = "an integer in 0..=65535";
    fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

impl FieldValue for bool {
    const EXPECTED: &'static str = "0, 1, false or true";
    fn parse(value: &str) -> Option<Self> {
        match value {
            "0" | "false" => Some(false),
            "1" | "true" => Some(true),
            _ => None,
        }
    }
}

impl FieldValue for TcqFrames {
    const EXPECTED: &'static str = "Off, Base or All";
    fn parse(value: &str) -> Option<Self> {
        match value {
            "Off" => Some(Self::Off),
            "Base" => Some(Self::Base),
            "All" => Some(Self::All),
            _ => None,
        }
    }
}

impl FieldValue for [bool; 3] {
    const EXPECTED: &'static str = "three comma-separated flags, such as 1,0,1";
    fn parse(value: &str) -> Option<Self> {
        let mut flags = value.split(',').map(bool::parse);
        let parsed = [flags.next()??, flags.next()??, flags.next()??];
        flags.next().is_none().then_some(parsed)
    }
}

fn parse_field<T: FieldValue>(
    field: &'static str,
    value: &str,
) -> core::result::Result<T, PresetError> {
    T::parse(value).ok_or_else(|| PresetError::InvalidValue {
        field,
        value: value.to_owned(),
        expected: T::EXPECTED,
    })
}

/// Builds [`PRESETS`], [`PresetParams::FIELDS`] and [`PresetParams::set`]
/// from a `preset: [p0, ..., p10]` header and groups of `field: [one value
/// per preset]` lines, one line per [`PresetParams`] field; a missing,
/// repeated or short line does not compile.
macro_rules! preset_table {
    (
        preset: [p0, p1, p2, p3, p4, p5, p6, p7, p8, p9, p10],
        $($group:ident { $($field:ident: $values:expr,)* })*
    ) => {
        /// Search limits indexed by preset: `0` is the slowest and best, `10` the fastest.
        pub const PRESETS: [PresetParams; 11] = {
            const fn preset(i: usize) -> PresetParams {
                PresetParams { $($($field: { let values: [_; 11] = $values; values[i] },)*)* }
            }
            [
                preset(0), preset(1), preset(2), preset(3), preset(4), preset(5),
                preset(6), preset(7), preset(8), preset(9), preset(10),
            ]
        };

        impl PresetParams {
            /// The field names [`Self::set`] accepts, in table order.
            pub const FIELDS: &'static [&'static str] = &[$($(stringify!($field),)*)*];

            /// Sets the field `name` to `value`: an integer in the field's
            /// type range, `0`, `1`, `false` or `true` for a flag, a
            /// [`TcqFrames`] variant name for `tcq`, and three comma-separated
            /// flags for `mvd_tools`.
            ///
            /// # Errors
            /// Returns [`PresetError::UnknownField`] when no field has this
            /// name, or [`PresetError::InvalidValue`] when `value` does not fit
            /// the field.
            pub fn set(&mut self, name: &str, value: &str) -> core::result::Result<(), PresetError> {
                match name {
                    $($(stringify!($field) => self.$field = parse_field(stringify!($field), value)?,)*)*
                    _ => return Err(PresetError::UnknownField { name: name.to_owned() }),
                }
                Ok(())
            }
        }
    };
}

use TcqFrames::Base;

#[rustfmt::skip]
preset_table! {
    preset:                    [   p0,    p1,    p2,    p3,    p4,    p5,    p6,    p7,    p8,    p9,   p10],
    partition {
        min_block_log2:        [    2,     2,     2,     2,     2,     2,     3,     3,     3,     3,     4],
        max_block_log2:        [    8,     8,     8,     7,     7,     7,     7,     6,     6,     6,     6],
        sb256_min_height:      [  720,   720,   720,   720,   720,   720,   720,   720,   720,   720,   720],
        partition_early_exit:  [false, false,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        ext_partitions:        [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        ext_three_way_margin:  [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        ext_four_way_margin:   [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        sdp:                   [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
    }
    intra {
        intra_modes:           [   13,    13,    12,    10,     9,     8,     6,     5,     4,     3,     2],
        mrl_trials:            [    2,     2,     2,     2,     2,     2,     2,     0,     0,     0,     0],
        cfl_filter:            [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        mhccp:                 [false, false, false, false, false, false, false, false, false, false, false],
        dip:                   [false, false, false, false, false, false, false, false, false, false, false],
    }
    transform {
        full_trials:           [ true,  true,  true,  true, false, false, false, false, false, false, false],
        rdoq:                  [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
        rdoq_every_type:       [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
        parity_hiding:         [false, false, false, false, false, false, false, false, false, false, false],
        tcq:                   [ Base,  Base,  Base,  Base,  Base,  Base,  Base,  Base,  Base,  Base,  Base],
        tcq_search_trellis:    [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true, false],
        tcq_type_trellis:      [ true,  true,  true,  true, false, false, false, false, false, false, false],
        tx_type_depth:         [    3,     3,     3,     2,     2,     2,     1,     1,     1,     0,     0],
        tx_partition_search:   [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
        inter_tx_split:        [   24,    24,    24,    24,    24,    24,    24,     0,     0,     0,     0],
        ist_sets:              [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        cctx:                  [false, false, false, false, false, false, false, false, false, false, false],
        inter_ist:             [false, false, false, false, false, false, false, false, false, false, false],
        inter_ddt:             [false, false, false, false, false, false, false, false, false, false, false],
    }
    inter {
        motion_range:          [  256,   192,   128,    96,    64,    64,    48,    32,    24,    16,    16],
        subpel_depth:          [    3,     3,     3,     2,     2,     2,     2,     1,     1,     1,     0],
        eighth_pel_max_qindex: [  128,   128,   128,     0,     0,     0,     0,     0,     0,     0,     0],
        compound_candidates:   [    8,     8,     6,     6,     4,     4,     3,     2,     1,     0,     0],
        large_inter:           [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        mv_prediction:         [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        drl_bits:              [    3,     3,     3,     3,     3,     3,     3,     1,     1,     1,     1],
        mvd_tools:             [[false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3], [false; 3]],
        interp_filter_search:  [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        cdf_carry:             [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        tip:                   [    1,     1,     1,     1,     1,     1,     1,     1,     1,     1,     1],
        tip_output:            [  100,   100,   100,   100,   100,   100,   100,   100,   100,   100,   100],
        opfl:                  [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true, false],
        refinemv:              [false, false, false, false, false, false, false, false, false, false, false],
        tip_refinemv:          [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true, false],
        warp:                  [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        compound_types:        [    1,     1,     1,     1,     1,     1,     1,     1,     1,     0,     0],
        implicit_mask:         [false, false, false, false, false, false, false, false, false, false, false],
        joint_newmv:           [false, false, false, false, false, false, false, false, false, false, false],
    }
    filters {
        loop_filter_search:    [    3,     3,     2,     2,     2,     1,     1,     1,     0,     0,     0],
        ccso_candidates:       [    8,     8,     6,     6,     4,     4,     4,     2,     2,     1,     1],
        wiener_ns:             [    2,     2,     2,     2,     2,     2,     2,     1,     1,     1,     1],
        wiener_ns_chroma:      [    2,     2,     2,     2,     2,     2,     2,     1,     1,     1,     1],
        wiener_ns_classes:     [   16,    16,    16,    16,    16,    16,    16,     4,     4,     4,     4],
        gdf_candidates:        [   16,    16,    16,    16,    16,    16,    16,    16,    16,    16,    16],
        cdef_on_skip:          [false, false, false, false, false, false, false, false, false, false, false],
    }
    screen_content {
        fsc:                   [false, false, false, false, false, false, false, false, false, false, false],
        screen_detect:         [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        palette:               [false, false, false, false, false, false, false, false, false, false, false],
        intrabc:               [false, false, false, false, false, false, false, false, false, false, false],
        integer_mv:            [false, false, false, false, false, false, false, false, false, false, false],
    }
    gop {
        mini_gop_size:         [    8,     8,     8,     8,     4,     4,     4,     4,     2,     2,     1],
        lookahead_frames:      [   16,    16,    16,    16,     8,     8,     8,     8,     4,     4,     2],
        tpl_delta_q:           [  100,   100,   100,   100,   100,   100,   100,   100,   100,   100,   100],
        delta_q_res:           [    1,     1,     1,     1,     1,     1,     1,     1,     1,     1,     1],
    }
}

/// [`PRESETS`] with the screen-content tools on and SDP off, for a sequence
/// whose first frame looks like screen content. SDP off: desktop_credits p6
/// measured Y +0.34% intra and +2.30% inter with it (screendata -0.24% and
/// -0.91%, 2026-10-26).
pub(crate) static SCREEN_PRESETS: [PresetParams; 11] = {
    let mut rows = PRESETS;
    let mut i = 0;
    while i < rows.len() {
        rows[i] = PresetParams {
            fsc: true,
            palette: true,
            intrabc: true,
            integer_mv: true,
            sdp: false,
            ..rows[i]
        };
        i += 1;
    }
    rows
};

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

    /// The row of [`SCREEN_PRESETS`] of this preset.
    pub(crate) fn screen_params(self) -> &'static PresetParams {
        &SCREEN_PRESETS[usize::from(self.0)]
    }
}

impl Default for Preset {
    fn default() -> Self {
        Self(6)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{Config, Encoder, SourceFrame};

    #[test]
    fn every_field_is_monotone_from_slow_to_fast() {
        for pair in PRESETS.windows(2) {
            let (slow, fast) = (&pair[0], &pair[1]);
            assert!(fast.min_block_log2 >= slow.min_block_log2);
            assert!(fast.max_block_log2 <= slow.max_block_log2);
            assert!(fast.min_block_log2 <= fast.max_block_log2);
            assert!(fast.intra_modes <= slow.intra_modes);
            assert!(fast.partition_early_exit >= slow.partition_early_exit);
            assert!(fast.sdp <= slow.sdp);
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
            assert!(fast.tpl_delta_q <= slow.tpl_delta_q);
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
            assert!(fast.palette <= slow.palette);
            assert!(fast.intrabc <= slow.intrabc);
            assert!(fast.integer_mv <= slow.integer_mv);
            assert!(fast.screen_detect <= slow.screen_detect);
            assert!(fast.cfl_filter <= slow.cfl_filter);
            assert!(fast.dip <= slow.dip);
            assert!(fast.large_inter <= slow.large_inter);
            assert!(fast.gdf_candidates <= slow.gdf_candidates);
            assert!(fast.cdef_on_skip <= slow.cdef_on_skip);
            assert!(fast.inter_ddt <= slow.inter_ddt);
            assert!(fast.inter_ist <= slow.inter_ist);
            assert!(fast.tip <= slow.tip);
            assert!(fast.warp <= slow.warp);
            assert!(fast.compound_types <= slow.compound_types);
            assert!(fast.implicit_mask <= slow.implicit_mask);
            assert!(fast.joint_newmv <= slow.joint_newmv);
            assert!(fast.opfl <= slow.opfl);
            assert!(fast.tip_refinemv <= slow.tip_refinemv);
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

    /// The measured placement of inter IST and DDT: off in every row (see
    /// `ENC-INTER-IST-DDT`).
    #[test]
    fn inter_ist_and_ddt_placement() {
        assert_eq!(
            PRESETS.map(|params| (params.inter_ist, params.inter_ddt)),
            [(false, false); 11]
        );
    }

    /// The measured placement of the MVD tools: off in every row (see
    /// `ENC-MV-CODING`).
    #[test]
    fn mvd_tools_placement() {
        assert_eq!(PRESETS.map(|params| params.mvd_tools), [[false; 3]; 11]);
    }

    #[test]
    fn set_overrides_one_field() -> core::result::Result<(), PresetError> {
        let mut params = PRESETS[6];
        params.set("gdf_candidates", "4")?;
        params.set("rdoq", "false")?;
        params.set("tcq", "All")?;
        params.set("mvd_tools", "1,0,true")?;
        let expected = PresetParams {
            gdf_candidates: 4,
            rdoq: false,
            tcq: TcqFrames::All,
            mvd_tools: [true, false, true],
            ..PRESETS[6]
        };
        assert_eq!(params, expected);
        Ok(())
    }

    #[test]
    fn set_rejects_unknown_fields_and_bad_values() {
        let mut params = PRESETS[6];
        let unknown = PresetError::UnknownField { name: "gdf".into() };
        assert_eq!(params.set("gdf", "4"), Err(unknown.clone()));
        let message = unknown.to_string();
        assert!(
            PresetParams::FIELDS
                .iter()
                .all(|field| message.contains(field))
        );
        for (field, value) in [
            ("gdf_candidates", "256"),
            ("gdf_candidates", "-1"),
            ("motion_range", "65536"),
            ("rdoq", "2"),
            ("rdoq", "yes"),
            ("tcq", "base"),
            ("mvd_tools", "1,0"),
            ("mvd_tools", "1,0,1,1"),
        ] {
            assert!(
                matches!(
                    params.set(field, value),
                    Err(PresetError::InvalidValue { .. })
                ),
                "{field}={value}"
            );
        }
        assert_eq!(params, PRESETS[6]);
    }

    #[test]
    fn every_row_passes_validate() {
        for params in PRESETS.iter().chain(&SCREEN_PRESETS) {
            assert_eq!(params.validate(), Ok(()));
        }
    }

    #[test]
    fn validate_rejects_unsupported_ranges() {
        for (overrides, field) in [
            (
                &[("min_block_log2", "7"), ("max_block_log2", "6")][..],
                "min_block_log2",
            ),
            (&[("max_block_log2", "9")], "max_block_log2"),
            (&[("sb256_min_height", "63")], "sb256_min_height"),
            (&[("drl_bits", "6")], "drl_bits"),
            (&[("drl_bits", "0")], "drl_bits"),
            (&[("lookahead_frames", "3")], "lookahead_frames"),
            (&[("mini_gop_size", "0")], "mini_gop_size"),
            (&[("mini_gop_size", "9")], "mini_gop_size"),
            (&[("intra_modes", "0")], "intra_modes"),
            (&[("gdf_candidates", "17")], "gdf_candidates"),
            (&[("tip", "3")], "tip"),
            (&[("tip_output", "201")], "tip_output"),
            (&[("opfl", "false")], "tip_refinemv"),
            (&[("warp", "3")], "warp"),
            (&[("compound_types", "4")], "compound_types"),
        ] {
            let mut params = PRESETS[6];
            for (name, value) in overrides {
                assert_eq!(params.set(name, value), Ok(()));
            }
            assert!(
                matches!(params.validate(), Err(PresetError::OutOfRange { field: f, .. }) if f == field),
                "{overrides:?}"
            );
        }
    }

    /// Values at the type extremes and around the supported bounds.
    const PROBE_VALUES: [&str; 17] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "16", "17", "255", "65535", "All", "Off",
        "1,1,1",
    ];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// A random set of overrides either fails validation or encodes two
        /// tiny frames.
        #[test]
        fn random_overrides_validate_or_encode(
            preset in 0..=Preset::MAX,
            picks in prop::collection::vec(
                (0..PresetParams::FIELDS.len(), 0..PROBE_VALUES.len()),
                1..7,
            ),
        ) {
            let mut config = Config::new(24, 16);
            config.preset = Preset::new(preset).unwrap_or_default();
            config.threads = splot_parallel::ThreadCount::from_count_or_auto(1);
            let mut scratch = PRESETS[0];
            for (field, start) in picks {
                let name = PresetParams::FIELDS[field];
                let value = (start..start + PROBE_VALUES.len())
                    .map(|i| PROBE_VALUES[i % PROBE_VALUES.len()])
                    .find(|value| scratch.set(name, value).is_ok());
                config.preset_overrides.extend(value.map(|value| (name.into(), value.into())));
            }
            if config.validate().is_ok() {
                let encode = || -> crate::Result<usize> {
                    let mut encoder = Encoder::new(config.clone())?;
                    for pts in 0..2 {
                        let luma = (0..24 * 16).map(|i| ((i * 37 + i / 24 * 91 + pts * 13) % 97) as u16 + 60);
                        let planes = [luma.collect(), vec![128; 12 * 8], vec![120; 12 * 8]];
                        encoder.send_frame(SourceFrame::new(&config, pts as u64, planes)?)?;
                    }
                    encoder.flush()?;
                    let mut bytes = 0;
                    while let Some(packet) = encoder.receive_packet()? {
                        bytes += packet.data.len();
                    }
                    Ok(bytes)
                };
                let result = encode();
                prop_assert!(result.is_ok(), "{:?}: {:?}", config.preset_overrides, result);
            }
        }
    }

    #[test]
    fn every_field_is_settable_by_name() {
        let mut params = PRESETS[10];
        for &field in PresetParams::FIELDS {
            assert!(
                ["1", "Base", "1,1,1"]
                    .iter()
                    .any(|value| params.set(field, value).is_ok()),
                "{field}"
            );
        }
        assert_ne!(params, PRESETS[10]);
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
