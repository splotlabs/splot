// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Speed presets. Each preset is one column of [`PRESETS`]; encoder stages read the
//! preset's fields instead of comparing preset numbers. A [`Levels`] field
//! holds one value per [`PresetContext`] and is read with [`Levels::at`].

use crate::error::{Error, Result};
use crate::rate_control::FrameType;

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
    /// Number of luma intra modes, and of chroma modes, tried per block,
    /// best-ranked first: in key (intra) frames the intra search, in the
    /// other frame kinds the intra shortlist of a leaf in an inter frame.
    pub intra_modes: Levels<u8>,
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
    /// hiding. Key and base frames in every row: its BD-rate per CPU beats
    /// the next slower row at every measured preset, and TCQ on the upper
    /// mini-GOP layers loses.
    pub tcq: Levels<bool>,
    /// The final trial of each leaf in the partition search runs the TCQ
    /// trellis; off, it prices TCQ blocks as plain levels and only the frame
    /// write runs the trellis. Mode trials always price plain levels.
    pub tcq_search_trellis: bool,
    /// The luma transform-type search and the luma trial before the chroma
    /// search run the TCQ trellis; off, they price TCQ blocks as plain levels.
    pub tcq_type_trellis: bool,
    /// The candidates of a mode, transform-partition or leaf choice that
    /// get a full trial, with RDOQ and the transform-type search: `0` gives
    /// every candidate a `DCT_DCT` deadzone trial and only a leaf's final
    /// trial runs both; `255` gives every candidate a full trial; between,
    /// MDS1 ranks every candidate by the deadzone trial and this many of
    /// the best get the full trial (MDS2) that decides.
    pub full_trials: u8,
    /// Luma transform-type search depth: depth `d` tries the first `2 * d + 1`
    /// types a block offers, most probable first. `0` codes every intra luma
    /// block with `DCT_DCT` and signals no type (`reduced_tx_set = 2`).
    pub tx_type_depth: u8,
    /// Luma transform-partition search (`TX_MODE_SELECT`); off codes every
    /// block with its largest transform (`TX_MODE_LARGEST`).
    pub tx_partition_search: bool,
    /// Full-pel motion search radius in luma samples.
    pub motion_range: u16,
    /// Open-loop motion estimation of the lookahead: the radius of the
    /// quarter-resolution search against the adjacent frame, in quarter
    /// samples; the radius doubles for references two or more frames away.
    pub hme_range: u8,
    /// Open-loop motion estimation of the lookahead: the radius, in luma
    /// samples, of the full-resolution search around the coarse vector; the
    /// search around the zero vector covers half of it.
    pub me_range: u8,
    /// Open-loop seeds of the RD motion search: a leaf with lookahead motion
    /// against its reference also starts from the open-loop vectors of its
    /// square size, the parent and the four children, and when the mean
    /// open-loop SAD per sample of its square is at most this value the
    /// coarse full-pel window shrinks to one sample. `0` adds no seeds.
    pub me_seeds: u8,
    /// Compound skip by open-loop SAD: a leaf skips the compound search when
    /// one reference with lookahead motion predicts its square with a mean
    /// open-loop SAD per sample of at most this value / 16. `0` is off.
    /// 16 at p0-p4 and 24 at p5-p10 (2026-10-31): 48 lost paris p6 +0.92%
    /// over 30 frames at 0.84x CPU; 24 measured +0.29% at 0.87x.
    pub compound_skip_sad: u8,
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
    /// Frames per mini-GOP, `1..=32`: `1` codes every frame shown (IPPP);
    /// larger sizes code the last frame first as a hidden alt-ref and the
    /// frames between hierarchically. `8` in every row (2026-11-01, CRF,
    /// keyint 64, vs main 8de875fa1): p6 paris f30 -4.21% (16: -3.69%, 32:
    /// -3.58%) at 0.94x CPU; p9 niklas f60 -1.31% (16: -1.27%, 32: +0.08%);
    /// p10 paris f30 -31.27% against IPPP at 0.90x CPU.
    pub mini_gop_size: u8,
    /// The mini-GOP of VBR and CBR, at most `mini_gop_size`: their rate
    /// controllers pick one base qindex per mini-GOP, and niklas p9 VBR
    /// missed its targets by up to +3.3% with mini-GOPs of 8 (+29..36% with
    /// 32, within 1% with 2). It can go once they re-plan within a mini-GOP.
    pub bitrate_mini_gop_size: u8,
    /// Frames the lookahead analyses before the next mini-GOP is coded, at
    /// least `mini_gop_size`; scene cuts and the temporal model see this far.
    pub lookahead_frames: u8,
    /// Strength in percent of the block-level temporal model: each
    /// superblock's qindex and lambda move with the importance of its
    /// lookahead blocks over the frame's, coded as `delta_q`. `0` is off.
    /// `100` in every row (2026-10-26, CRF, vs main 9ec81da32): p6 niklas
    /// f30 Y -0.86%, paris f30 -1.92%, park_joy f10 +0.06% at 0.95-1.0x CPU.
    pub tpl_delta_q: u8,
    /// Temporal filtering, `0..=8` per frame kind, `0` off; the lookahead
    /// reads it at the normal coefficient level. Key frames: the most later
    /// neighbours that a key frame is filtered from before it is coded; the
    /// lookahead bounds them. Base frames: the most neighbours per side of a
    /// hidden mini-GOP base frame, which its show-existing frame outputs
    /// filtered. Mid and leaf frames are not filtered. Key frames: `4` at p0-p9 (2026-10-31, p6 vs main
    /// 5530d5de2): paris f30 -1.37%, niklas f30 +0.11% (-0.28% before the
    /// two-pass partition decision), park_joy f10 -0.04% at 0.99-1.01x CPU;
    /// vs 336b8b726: paris p2 -1.60%, park_joy p0 +0.64%, p8 +0.53%. Off at
    /// p10: park_joy -0.82% for +3.4% CPU (+5.3% instructions), under the
    /// fast-row slope. Base frames: off in every row (2026-10-31): with two
    /// per side at strength 25 on top of the key frames, paris p6 f30 went
    /// from -1.74% to -1.42%; the filtered frame is shown without an overlay
    /// frame.
    pub tf_frames: Levels<u8>,
    /// The strength of `tf_frames` in percent per frame kind: it scales the
    /// weight decay, so larger values keep more of the neighbours. `0` is
    /// off.
    pub tf_strength: Levels<u8>,
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
    /// Two-pass partition decision: an inter-frame superblock with
    /// lookahead motion takes first-pass levels from the open-loop motion
    /// alone, and searches a block only within this many binary partition
    /// levels of them; `255` searches every partition.
    pub pd_window: u8,
    /// The first pass charges each square this many bits besides its
    /// open-loop SAD.
    pub pd0_leaf_bits: u8,
    /// Reference statistics: a leaf of an inter frame skips the intra search
    /// when no intra leaf of its primary reference covers its collocated
    /// area grown by this many 8x8 cells; `255` is off.
    pub ref_intra_margin: u8,
    /// The skip reads a primary reference only while fewer than this many
    /// frames separate it from a frame of its chain that searched intra in
    /// every leaf; so new content gets an intra search again.
    pub ref_intra_refresh: u8,
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
    /// The most CDEF strength sets a frame codes, `1..=8`.
    pub cdef_strengths: u8,
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
    /// Global motion (`enable_global_motion`), `0..=1`: inter frames code
    /// `use_global_motion` with the models the lookahead estimates. Off in
    /// every row (2026-10-30): at `1` p6 measured paris +0.01%, niklas
    /// +0.00%, park_joy +0.49%; the lookahead vectors are too coarse.
    pub global_motion: u8,
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
    /// Inter-intra (`seq_enabled_motion_modes[INTERINTRA]`): the cheapest
    /// single-reference `NEWMV` of a leaf adds its best smooth-mask
    /// `interintra_mode` by luma SATD to the RD trials. Off in every row
    /// (2026-10-30): at p6 it measured park_joy +0.39% Y (-0.05% YUV) and
    /// paris +0.13% Y for 3% more CPU, largely the `inter_intra` flag every
    /// eligible block codes.
    pub interintra: bool,
    /// Block adaptive weighted prediction (`enable_bawp`, every inter frame
    /// codes `allow_bawp = 1`): the cheapest single-reference `NEWMV` of a
    /// leaf adds its best BAWP scale by luma SATD to the RD trials. On at
    /// p0-p6 (2026-10-30, p6 vs main): park_joy -0.28%, paris -0.11%, niklas
    /// -0.26% for about 3% more CPU; p7-p10 are not measured.
    pub bawp: bool,
    /// Staged mode decision: the inter candidates of a leaf that each class
    /// (`[MVP, NEWMV, compound]`) keeps for the all-plane trials, lowest
    /// luma prediction SATD plus mode rate first. `255` keeps every one.
    pub mds0_budgets: [u8; 3],
    /// Wavefront search: the superblocks of one tile search in parallel, each
    /// once its left and top-right neighbours are committed, and price from
    /// CDFs averaged three to one from those two neighbours; one writer codes
    /// them in raster order with the exact CDFs. Superblock qindex targets
    /// stay on the `delta_q_res` steps of `base_q_idx`, where each row starts.
    /// Off searches in raster order from the exact running CDFs. On at
    /// p7-p10 only (2026-10-31, vs main d2bedf697, niklas f10): p8 +0.72% BD
    /// for 2x wall time at 8 threads; p6 +0.51% with no 1-thread saving.
    pub wavefront: bool,
}

/// The resolution class of a frame, by luma area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolutionClass {
    /// At most 480p (854x480).
    #[default]
    Sd,
    /// At most 1080p (1920x1088).
    Hd,
    /// Larger than 1080p.
    Uhd,
}

impl ResolutionClass {
    /// The class of a `width` x `height` frame.
    #[must_use]
    pub const fn of(width: u32, height: u32) -> Self {
        match width as u64 * height as u64 {
            ..=409_920 => Self::Sd,
            409_921..=2_088_960 => Self::Hd,
            _ => Self::Uhd,
        }
    }
}

/// The kind of a frame in the mini-GOP hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameKind {
    /// A key frame.
    #[default]
    Key,
    /// An inter frame of mini-GOP layer 0.
    Base,
    /// An inter frame above layer 0 that refreshes a reference slot.
    Mid,
    /// An inter frame above layer 0 that refreshes no reference slot.
    Leaf,
}

impl FrameKind {
    /// The kind of a frame of `frame_type` on mini-GOP `layer` that
    /// refreshes the slots of `refresh_frame_flags`.
    #[must_use]
    pub const fn of(frame_type: FrameType, layer: u8, refresh_frame_flags: u32) -> Self {
        match (frame_type, layer, refresh_frame_flags) {
            (FrameType::Key, ..) => Self::Key,
            (_, 0, _) => Self::Base,
            (_, _, 0) => Self::Leaf,
            _ => Self::Mid,
        }
    }
}

/// The coefficient level of a frame, from its qindex: a fine quantizer
/// leaves high levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoeffLevel {
    /// qindex above 192.
    Low,
    /// qindex 129..=192.
    #[default]
    Normal,
    /// qindex at most 128.
    High,
}

impl CoeffLevel {
    /// The level of a frame coded at `qindex`.
    #[must_use]
    pub const fn of(qindex: u32) -> Self {
        match qindex {
            ..=128 => Self::High,
            129..=192 => Self::Normal,
            _ => Self::Low,
        }
    }
}

/// The context a [`Levels`] field takes its value for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresetContext {
    /// The resolution class.
    pub resolution: ResolutionClass,
    /// The frame kind.
    pub frame: FrameKind,
    /// The coefficient level.
    pub coeff: CoeffLevel,
}

/// One value of a preset field per [`PresetContext`]: 4 frame kinds by 3
/// resolution classes by 3 coefficient levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels<T>([T; 36]);

impl<T: Copy> Levels<T> {
    /// `value` in every context.
    #[must_use]
    pub const fn all(value: T) -> Self {
        Self([value; 36])
    }

    /// One value per frame kind (key, base, mid, leaf).
    #[must_use]
    pub const fn by_frame(values: [T; 4]) -> Self {
        let mut out = [values[0]; 36];
        let mut i = 0;
        while i < out.len() {
            out[i] = values[i / 9];
            i += 1;
        }
        Self(out)
    }

    /// The value in `context`.
    #[must_use]
    pub const fn at(&self, context: PresetContext) -> T {
        self.0
            [context.frame as usize * 9 + context.resolution as usize * 3 + context.coeff as usize]
    }

    /// The values of every context.
    #[must_use]
    pub const fn values(&self) -> &[T; 36] {
        &self.0
    }
}

/// One [`Levels::by_frame`] per preset from one row of 11 values per frame
/// kind (key, base, mid, leaf).
const fn frames<T: Copy>(rows: [[T; 11]; 4]) -> [Levels<T>; 11] {
    let mut out = [Levels::all(rows[0][0]); 11];
    let mut i = 0;
    while i < out.len() {
        out[i] = Levels::by_frame([rows[0][i], rows[1][i], rows[2][i], rows[3][i]]);
        i += 1;
    }
    out
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
        for &modes in self.intra_modes.values() {
            check("intra_modes", modes, 1, u8::MAX, "")?;
        }
        for budget in self.mds0_budgets {
            check("mds0_budgets", budget, 1, u8::MAX, " per class")?;
        }
        check("subpel_depth", self.subpel_depth, 0, 3, "")?;
        check("loop_filter_search", self.loop_filter_search, 0, 3, "")?;
        check("mini_gop_size", self.mini_gop_size, 1, 32, "")?;
        check(
            "bitrate_mini_gop_size",
            self.bitrate_mini_gop_size,
            1,
            self.mini_gop_size,
            " (..mini_gop_size)",
        )?;
        check(
            "lookahead_frames",
            self.lookahead_frames,
            self.mini_gop_size,
            u8::MAX,
            " (mini_gop_size..)",
        )?;
        check("drl_bits", self.drl_bits, 1, 5, "")?;
        check("tpl_delta_q", self.tpl_delta_q, 0, 200, "")?;
        for &frames in self.tf_frames.values() {
            check("tf_frames", frames, 0, 8, "")?;
        }
        check("delta_q_res", self.delta_q_res, 0, 3, "")?;
        check("interp_filter_search", self.interp_filter_search, 0, 2, "")?;
        check("ext_partitions", self.ext_partitions, 0, 2, "")?;
        check("wiener_ns_classes", self.wiener_ns_classes, 1, u8::MAX, "")?;
        check("gdf_candidates", self.gdf_candidates, 0, 16, "")?;
        check("cdef_strengths", self.cdef_strengths, 1, 8, "")?;
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
        )?;
        check("global_motion", self.global_motion, 0, 1, "")
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

impl FieldValue for [u8; 3] {
    const EXPECTED: &'static str = "three comma-separated integers in 0..=255, such as 6,2,4";
    fn parse(value: &str) -> Option<Self> {
        let mut values = value.split(',').map(u8::parse);
        let parsed = [values.next()??, values.next()??, values.next()??];
        values.next().is_none().then_some(parsed)
    }
}

impl<T: FieldValue + Copy> FieldValue for Levels<T> {
    const EXPECTED: &'static str = "one value for every frame, or four comma-separated values for key, base, mid and leaf frames";
    fn parse(value: &str) -> Option<Self> {
        let values: Vec<T> = value.split(',').map(T::parse).collect::<Option<_>>()?;
        match values[..] {
            [all] => Some(Self::all(all)),
            [key, base, mid, leaf] => Some(Self::by_frame([key, base, mid, leaf])),
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
            /// type range, `0`, `1`, `false` or `true` for a flag, one value
            /// or four comma-separated values (key, base, mid, leaf frames)
            /// for a [`Levels`] field, three comma-separated flags for
            /// `mvd_tools`, and three comma-separated integers for
            /// `mds0_budgets`.
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
        pd_window:             [  255,   255,   255,   255,   255,   255,     3,     2,     2,     2,     2],
        pd0_leaf_bits:         [    8,     8,     8,     8,     8,     8,     8,     8,     8,     8,     8],
        ref_intra_margin:      [    1,     1,     1,     1,     1,     1,     1,     1,     1,     1,     1],
        ref_intra_refresh:     [    4,     4,     4,     4,     4,     4,     4,     4,     4,     4,     4],
        sdp:                   [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
    }
    intra {
        intra_modes:           frames([
                               [   13,    13,    12,    10,     9,     8,     6,     5,     4,     3,     2],
                               [   13,    13,    12,    10,     9,     8,     3,     3,     3,     3,     2],
                               [   13,    13,    12,    10,     9,     8,     3,     3,     3,     3,     2],
                               [   13,    13,    12,    10,     9,     8,     3,     3,     3,     3,     2]]),
        mrl_trials:            [    2,     2,     2,     2,     2,     2,     2,     0,     0,     0,     0],
        cfl_filter:            [ true,  true,  true,  true,  true,  true,  true,  true,  true,  true,  true],
        mhccp:                 [false, false, false, false, false, false, false, false, false, false, false],
        dip:                   [false, false, false, false, false, false, false, false, false, false, false],
    }
    transform {
        full_trials:           [    6,     6,     6,     4,     0,     0,     0,     0,     0,     0,     0],
        rdoq:                  [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
        rdoq_every_type:       [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
        parity_hiding:         [false, false, false, false, false, false, false, false, false, false, false],
        tcq:                   frames([[true; 11], [true; 11], [false; 11], [false; 11]]),
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
        hme_range:             [   16,    16,    16,    12,    12,     8,     8,     8,     8,     8,     8],
        me_range:              [   16,    16,    16,    12,    12,     8,     8,     8,     6,     4,     4],
        me_seeds:              [    4,     4,     4,     4,     4,     4,     4,     4,     4,     4,     4],
        compound_skip_sad:     [   16,    16,    16,    16,    16,    24,    24,    24,    24,    24,    24],
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
        global_motion:         [    0,     0,     0,     0,     0,     0,     0,     0,     0,     0,     0],
        compound_types:        [    1,     1,     1,     1,     1,     1,     1,     1,     1,     0,     0],
        implicit_mask:         [false, false, false, false, false, false, false, false, false, false, false],
        interintra:            [false, false, false, false, false, false, false, false, false, false, false],
        bawp:                  [ true,  true,  true,  true,  true,  true,  true, false, false, false, false],
    }
    filters {
        loop_filter_search:    [    3,     3,     2,     2,     2,     1,     1,     1,     0,     0,     0],
        ccso_candidates:       [    8,     8,     6,     6,     4,     4,     4,     2,     2,     1,     1],
        wiener_ns:             [    2,     2,     2,     2,     2,     2,     2,     1,     1,     1,     1],
        wiener_ns_chroma:      [    2,     2,     2,     2,     2,     2,     2,     1,     1,     1,     1],
        wiener_ns_classes:     [   16,    16,    16,    16,    16,    16,    16,     4,     4,     4,     4],
        gdf_candidates:        [   16,    16,    16,    16,    16,    16,    16,    16,    16,    16,    16],
        cdef_strengths:        [    8,     8,     8,     8,     8,     8,     8,     8,     8,     8,     8],
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
        mini_gop_size:         [    8,     8,     8,     8,     8,     8,     8,     8,     8,     8,     8],
        bitrate_mini_gop_size: [    8,     8,     8,     8,     8,     4,     4,     4,     2,     2,     1],
        lookahead_frames:      [   16,    16,    16,    16,    16,     8,     8,     8,     8,     8,     8],
        tpl_delta_q:           [  100,   100,   100,   100,   100,   100,   100,   100,   100,   100,   100],
        tf_frames:             frames([
                               [    4,     4,     4,     4,     4,     4,     4,     4,     4,     4,     0],
                               [0; 11], [0; 11], [0; 11]]),
        tf_strength:           frames([[100; 11], [25; 11], [25; 11], [25; 11]]),
        delta_q_res:           [    1,     1,     1,     1,     1,     1,     1,     1,     1,     1,     1],
    }
    mode_decision {
        mds0_budgets:          [[255; 3], [255; 3], [255; 3], [255; 3], [255; 3], [255; 3], [4, 2, 2], [4, 2, 2], [4, 2, 2], [4, 2, 2], [4, 2, 2]],
    }
    threading {
        wavefront:             [false, false, false, false, false, false, false,  true,  true,  true,  true],
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

    /// `fast` is at most `slow` in every context.
    fn at_most<T: PartialOrd + Copy>(fast: &Levels<T>, slow: &Levels<T>) -> bool {
        fast.values().iter().zip(slow.values()).all(|(f, s)| f <= s)
    }

    #[test]
    fn every_field_is_monotone_from_slow_to_fast() {
        for pair in PRESETS.windows(2) {
            let (slow, fast) = (&pair[0], &pair[1]);
            assert!(fast.min_block_log2 >= slow.min_block_log2);
            assert!(fast.max_block_log2 <= slow.max_block_log2);
            assert!(fast.min_block_log2 <= fast.max_block_log2);
            assert!(at_most(&fast.intra_modes, &slow.intra_modes));
            assert!(fast.partition_early_exit >= slow.partition_early_exit);
            assert!(fast.sdp <= slow.sdp);
            assert!(fast.rdoq <= slow.rdoq);
            assert!(fast.rdoq_every_type <= slow.rdoq_every_type);
            assert!(fast.parity_hiding <= slow.parity_hiding);
            assert!(at_most(&fast.tcq, &slow.tcq));
            assert!(fast.tcq_search_trellis <= slow.tcq_search_trellis);
            assert!(fast.tcq_type_trellis <= slow.tcq_type_trellis);
            assert!(fast.tcq_search_trellis || !fast.tcq_type_trellis);
            assert!(fast.rdoq || !fast.rdoq_every_type);
            assert!(fast.full_trials <= slow.full_trials);
            assert!(fast.tx_type_depth <= slow.tx_type_depth);
            assert!(fast.tx_partition_search <= slow.tx_partition_search);
            assert!(fast.motion_range <= slow.motion_range);
            assert!(fast.hme_range <= slow.hme_range && fast.me_range <= slow.me_range);
            assert!(slow.me_seeds == 0 || fast.me_seeds >= slow.me_seeds);
            assert!(
                slow.compound_skip_sad == 0 || fast.compound_skip_sad >= slow.compound_skip_sad
            );
            assert!(fast.subpel_depth <= slow.subpel_depth);
            assert!(fast.eighth_pel_max_qindex <= slow.eighth_pel_max_qindex);
            assert!(fast.loop_filter_search <= slow.loop_filter_search);
            assert!(fast.mini_gop_size <= slow.mini_gop_size);
            assert!(fast.bitrate_mini_gop_size <= slow.bitrate_mini_gop_size);
            assert!(fast.lookahead_frames <= slow.lookahead_frames);
            assert!(fast.tpl_delta_q <= slow.tpl_delta_q);
            assert!(at_most(&fast.tf_frames, &slow.tf_frames));
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
            assert!(fast.pd_window <= slow.pd_window);
            assert!(fast.pd0_leaf_bits >= slow.pd0_leaf_bits);
            assert!(fast.ref_intra_margin <= slow.ref_intra_margin);
            assert!(fast.ref_intra_refresh >= slow.ref_intra_refresh);
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
            assert!(fast.cdef_strengths <= slow.cdef_strengths);
            assert!(fast.cdef_on_skip <= slow.cdef_on_skip);
            assert!(fast.inter_ddt <= slow.inter_ddt);
            assert!(fast.inter_ist <= slow.inter_ist);
            assert!(fast.tip <= slow.tip);
            assert!(fast.warp <= slow.warp);
            assert!(fast.global_motion <= slow.global_motion);
            assert!(fast.compound_types <= slow.compound_types);
            assert!(fast.implicit_mask <= slow.implicit_mask);
            assert!(fast.interintra <= slow.interintra);
            assert!(fast.bawp <= slow.bawp);
            assert!(fast.wavefront >= slow.wavefront);
            assert!(
                fast.mds0_budgets
                    .iter()
                    .zip(slow.mds0_budgets)
                    .all(|(&f, s)| f <= s)
            );
            assert!(fast.opfl <= slow.opfl);
            assert!(fast.tip_refinemv <= slow.tip_refinemv);
            assert!(
                fast.inter_tx_split == 0
                    || (slow.inter_tx_split != 0 && fast.inter_tx_split >= slow.inter_tx_split)
            );
        }
        for params in PRESETS {
            assert!((1..=32).contains(&params.mini_gop_size));
            assert!(params.bitrate_mini_gop_size <= params.mini_gop_size);
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
        params.set("tcq", "true")?;
        params.set("intra_modes", "5,4,3,2")?;
        params.set("mvd_tools", "1,0,true")?;
        let expected = PresetParams {
            gdf_candidates: 4,
            rdoq: false,
            tcq: Levels::all(true),
            intra_modes: Levels::by_frame([5, 4, 3, 2]),
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
            ("full_trials", "true"),
            ("rdoq", "yes"),
            ("tcq", "base"),
            ("tcq", "1,1"),
            ("intra_modes", "1,2,3,4,5"),
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

    /// Each context axis splits where its type documents.
    #[test]
    fn contexts_classify_frames() {
        use FrameType::{Inter, Key};
        assert_eq!(ResolutionClass::of(854, 480), ResolutionClass::Sd);
        assert_eq!(ResolutionClass::of(1280, 720), ResolutionClass::Hd);
        assert_eq!(ResolutionClass::of(1920, 1088), ResolutionClass::Hd);
        assert_eq!(ResolutionClass::of(2048, 1088), ResolutionClass::Uhd);
        assert_eq!(FrameKind::of(Key, 0, 0xff), FrameKind::Key);
        assert_eq!(FrameKind::of(Inter, 0, 1), FrameKind::Base);
        assert_eq!(FrameKind::of(Inter, 0, 0), FrameKind::Base);
        assert_eq!(FrameKind::of(Inter, 1, 2), FrameKind::Mid);
        assert_eq!(FrameKind::of(Inter, 2, 0), FrameKind::Leaf);
        assert_eq!(CoeffLevel::of(128), CoeffLevel::High);
        assert_eq!(CoeffLevel::of(129), CoeffLevel::Normal);
        assert_eq!(CoeffLevel::of(193), CoeffLevel::Low);
        let mut values = [0u8; 36];
        for (i, value) in values.iter_mut().enumerate() {
            *value = i as u8;
        }
        let levels = Levels(values);
        let context = PresetContext {
            resolution: ResolutionClass::Uhd,
            frame: FrameKind::Mid,
            coeff: CoeffLevel::Low,
        };
        assert_eq!(levels.at(context), 2 * 9 + 2 * 3);
        assert_eq!(Levels::by_frame([1, 2, 3, 4]).at(context), 3);
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
            (&[("mini_gop_size", "33")], "mini_gop_size"),
            (&[("bitrate_mini_gop_size", "0")], "bitrate_mini_gop_size"),
            (&[("bitrate_mini_gop_size", "9")], "bitrate_mini_gop_size"),
            (&[("intra_modes", "0")], "intra_modes"),
            (&[("mds0_budgets", "4,0,2")], "mds0_budgets"),
            (&[("intra_modes", "3,3,0,3")], "intra_modes"),
            (&[("tf_frames", "9")], "tf_frames"),
            (&[("gdf_candidates", "17")], "gdf_candidates"),
            (&[("cdef_strengths", "0")], "cdef_strengths"),
            (&[("tip", "3")], "tip"),
            (&[("tip_output", "201")], "tip_output"),
            (&[("opfl", "false")], "tip_refinemv"),
            (&[("warp", "3")], "warp"),
            (&[("global_motion", "2")], "global_motion"),
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
    const PROBE_VALUES: [&str; 16] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "16", "17", "255", "65535", "1,1,0,0",
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
                ["1", "1,1,1"]
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
