// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-backed sequence-header and key-frame-header builders for any frame size.
//!
//! Each builder writes the canonical AV2 bits for its restricted choice set (§ 5.4,
//! § 5.18.2) and parses them with the decoder parsers, so the returned models are the
//! exact models a decoder derives. [`encode_key_frame_temporal_unit`] wraps one coded
//! tile in an Annex B temporal unit.

use crate::bitio::BitReader;
use crate::headers::frame::IntrabcParams;
use crate::headers::frame::restoration::default_restoration_size;
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CcsoPlaneParams, CdefParams, CdefStrengthSets, CoreSeqView, DeblockingFilterParams,
    FrameHeaderCore, FrameHeaderParseStatus, FrameReferenceStateView, FrameRestorationType,
    GdfGeometry, GdfParams, LrGeometry, LrParams, LrPlaneParams, MfhDeblockingView, MfhFrameView,
    QmSetLevels, QuantizationParams, SegmentationParams, SetupQmParams, TxMode,
    WienerNsFrameFilterBank, encode_temporal_delimiter_obu, gdf_per_block_is_coded,
    init_core_from_prefix, parse_core_body, parse_frame_header_prefix,
};
use crate::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, CroppingWindow, LevelIdx, SequenceHeader, SuperblockSize, Tier,
    parse_sequence_header,
};
use crate::headers::tile_group::TileGroupFraming;
use crate::obu::{ObuHeader, ParsedObu};
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature};
use crate::span::ByteOffset;
use crate::tile::{TileParamsInput, TileSpacing, TileStarts};
use crate::types::{EmbeddedLayerId, ExtendedLayerId, ObuType, TemporalLayerId};
use crate::write::seq_tile::{explicit_tile_layout, uniform_tile_layout, write_tile_params};
use crate::write::{
    BitWriter, LrReferenceFilters, WriteError, write_annexb_obu, write_ccso_plane_params,
    write_cdef_params, write_deblocking_filter_params, write_gdf_params,
    write_inter_segmentation_params, write_intrabc_params, write_lr_params_for_inter,
    write_obu_payload, write_quantization_params, write_setup_qm_params, write_tile_group_obu,
};

/// The [`SegmentationParams`] of [`KeyFrameTxParams::segment_alt_q`].
fn segmentation_params(alt_q: Option<[i16; 4]>) -> SegmentationParams {
    let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    if let Some(alt_q) = alt_q {
        for (segment, data) in features.iter_mut().zip(alt_q) {
            segment[0] = SegmentFeature {
                enabled: true,
                data: i32::from(data),
            };
        }
    }
    SegmentationParams {
        segmentation_enabled: alt_q.is_some(),
        reuse_seg_info: false,
        features,
        segmentation_update_map: alt_q.is_some(),
        segmentation_temporal_update: false,
        seg_id_pre_skip: false,
        last_active_seg_id: if alt_q.is_some() { 3 } else { 0 },
    }
}

/// `seq_level_idx == 31`: the level without tile-size limits (§ 5.18.7.3).
const UNCONSTRAINED_LEVEL_IDX: u32 = 31;
/// `order_hint_bits_minus_1` written by [`build_encoder_sequence_header`].
const ORDER_HINT_BITS_MINUS_1: u32 = 7;
/// `base_*_delta_q` value that gives a zero `Base*DeltaQ` (`-DELTA_DCQUANT_MIN`, § 3).
const ZERO_BASE_DELTA_Q: u32 = 23;
/// `df_par_bits_minus_2` without `EncoderToolFlags::df_par_bits_extra`:
/// `DfDeltaQ` spans `-4..=3` (§ 5.18.5.2).
const DF_PAR_BITS_MINUS_2: u32 = 1;
/// The largest `reduced_tx_set` (`f(2)`, § 5.18.2).
pub(super) const MAX_REDUCED_TX_SET: u8 = 3;

/// One bit per optional AV2 coding tool that `sequence_header_obu()` can enable.
///
/// `Default` disables every tool. TX64 and quantizer matrices have no sequence-level
/// enable flag in § 5.4, so they are not listed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EncoderToolFlags {
    /// `enable_cdef`.
    pub cdef: bool,
    /// `enable_restoration`.
    pub loop_restoration: bool,
    /// `enable_ccso`.
    pub ccso: bool,
    /// `enable_gdf`.
    pub gdf: bool,
    /// `enable_sdp`.
    pub sdp: bool,
    /// `enable_extended_sdp` (needs `sdp`).
    pub extended_sdp: bool,
    /// `enable_ext_partitions`.
    pub ext_partitions: bool,
    /// `enable_uneven_4way_partitions` (needs `ext_partitions`).
    pub uneven_4way_partitions: bool,
    /// `enable_cfl_intra`.
    pub cfl: bool,
    /// `enable_mhccp`.
    pub mhccp: bool,
    /// `cfl_ds_filter_index`, `0..=3`; not coded for monochrome.
    pub cfl_ds_filter: u8,
    /// `seq_choose_screen_content_tools` with `seq_choose_integer_mv`:
    /// every frame codes `allow_screen_content_tools = 1` (palette) and
    /// `force_integer_mv = 0`.
    pub screen_content: bool,
    /// `enable_ibp`.
    pub ibp: bool,
    /// `enable_mrls`.
    pub mrls: bool,
    /// `enable_fsc` (infers `enable_idtx_intra`).
    pub fsc: bool,
    /// `enable_dip`.
    pub dip: bool,
    /// `enable_idtx_intra`.
    pub idtx_intra: bool,
    /// `enable_intra_edge_filter`.
    pub intra_edge_filter: bool,
    /// `enable_intra_ist`.
    pub intra_ist: bool,
    /// `enable_inter_ddt`.
    pub inter_ddt: bool,
    /// `enable_inter_ist`.
    pub inter_ist: bool,
    /// `enable_tip` with `disable_tip_output = 1` and no hole fill: inter
    /// blocks may predict from the TIP frame.
    pub tip: bool,
    /// With `tip`, `disable_tip_output = 0`: `OBU_REGULAR_TIP` frames code
    /// `TIP_FRAME_AS_OUTPUT`.
    pub tip_output: bool,
    /// With `tip_output` and `df_sub_pu`, `enable_tip_explicit_qp` (§ 5.4.6):
    /// `TIP_FRAME_AS_OUTPUT` frames code `quantization_params()`.
    pub tip_explicit_qp: bool,
    /// With `tip`, `enable_tip_hole_fill` (§ 5.4.6): TIP frames may code
    /// `allow_tip_hole_fill` (§ 5.18.2).
    pub tip_hole_fill: bool,
    /// `enable_opfl_refine = REFINE_AUTO`: each inter frame codes its
    /// `opfl_refine_type`.
    pub opfl_refine: bool,
    /// `enable_refinemv`: compound blocks may refine their vectors (DMVR).
    pub refinemv: bool,
    /// With `tip` and `opfl_refine` or `refinemv`, `enable_tip_refinemv`:
    /// TIP blocks and TIP output frames refine their projected vectors.
    pub tip_refinemv: bool,
    /// `enable_cctx`.
    pub cctx: bool,
    /// `enable_tcq` with `choose_tcq_per_frame = 1`: each frame codes `allow_tcq`.
    pub tcq: bool,
    /// `enable_parity_hiding`.
    pub parity_hiding: bool,
    /// `film_grain_params_present` (§ 5.4.1): output frames code
    /// `film_grain_config()` (§ 5.18.10.1).
    pub film_grain: bool,
    /// `enable_avg_cdf` with `avg_cdf_type` (§ 5.4.8): `Some(true)` averages
    /// the saved CDFs across tiles, `Some(false)` blends the CDFs of two
    /// references; `None` disables averaging.
    pub avg_cdf: Option<bool>,
    /// `enable_ref_frame_mvs` with `reduced_ref_frame_mvs_mode = 0` (§ 5.4.6).
    pub ref_frame_mvs: bool,
    /// `enable_refmvbank`.
    pub refmvbank: bool,
    /// `disable_drl_reorder = 0` with `constrain_drl_reorder` (§ 5.4.6):
    /// `Some(true)` is `DRL_REORDER_CONSTRAINT`, `Some(false)`
    /// `DRL_REORDER_ALWAYS`; `None` disables reordering.
    pub drl_reorder: Option<bool>,
    /// `enable_mv_traj` (§ 5.4.6): with `ref_frame_mvs`, the projected motion
    /// field follows motion trajectories (§ 7.9.2).
    pub mv_traj: bool,
    /// `seq_max_drl_bits_minus_1` (§ 5.4.6), `0..=4`: `NEARMV` and `NEWMV`
    /// code `RefMvIdx` below `seq_max_drl_bits_minus_1 + 2`.
    pub max_drl_bits_minus_1: u32,
    /// `seq_max_bvp_drl_bits_minus_1` (§ 5.4.6), `0..=2`: IntraBC leaves code
    /// `RefMvIdx` below `seq_max_bvp_drl_bits_minus_1 + 2` (§ 5.20.5.4).
    pub max_bvp_drl_bits_minus_1: u32,
    /// `enable_adaptive_mvd`, `enable_mvd_sign_derive` and
    /// `enable_flex_mvres` (§ 5.4.6).
    pub mvd_tools: [bool; 3],
    /// `seq_enabled_motion_modes` `LOCALWARP`, `DELTAWARP` and `EXTENDWARP`
    /// with `seq_frame_motion_modes_present_flag` and
    /// `enable_six_param_warp_delta` (§ 5.4.6): each inter frame codes
    /// whether its blocks may warp.
    pub warp: bool,
    /// `enable_global_motion` (§ 5.4.6): each inter frame codes
    /// `use_global_motion`.
    pub global_motion: bool,
    /// `long_term_frame_id_bits`, `0..=7` (§ 5.4.6): key frames code
    /// `long_term_id_plus_1` in this many bits.
    pub long_term_frame_id_bits: u8,
    /// `monotonic_output_order_flag = 0` (§ 5.4.1): hidden frames code
    /// `implicit_output_frame`.
    pub implicit_output: bool,
    /// `explicit_num_ref_frames` with `NumRefFrames`, `1..=16` (§ 5.4.6);
    /// `None` keeps the default 8 slots.
    pub num_ref_frames: Option<u8>,
    /// `explicit_ref_frame_map` (§ 5.4.6): inter frames code
    /// `frame_explicit_ref_frame_map`.
    pub explicit_ref_frame_map: bool,
    /// `seq_tile_info_present_flag` (§ 5.4.2): the sequence codes this
    /// uniform layout, which frames reuse (§ 5.18.7.2).
    pub seq_tiles: Option<SeqTiles>,
    /// `disable_loopfilters_across_tiles` (§ 5.4.10): the in-loop filters do
    /// not read samples of another tile (§ 6.4.10).
    pub disable_loopfilters_across_tiles: bool,
    /// The plane quantizer deltas `[DeltaQYDc, DeltaQU*, DeltaQV*]` frames may
    /// code (§ 5.4.8): a non-zero luma value codes `equal_ac_dc_q = 0` with
    /// `y_dc_delta_q_enabled`, a non-zero chroma value codes the chroma
    /// enables, and `u != v` codes `separate_uv_delta_q`.
    pub plane_delta_q: [i8; 3],
    /// `enable_short_refresh_frame_flags` (§ 5.4.6): inter frames code
    /// `has_refresh_frame_flags` and `frame_to_refresh`.
    pub short_refresh: bool,
    /// `enable_masked_compound` (§ 5.4.6): compound blocks may code a wedge
    /// or difference-weighted mask (§ 5.20.7.16).
    pub masked_compound: bool,
    /// `enable_cwp` (§ 5.4.6): compound blocks may code `cwp_idx`
    /// (§ 5.20.7.6).
    pub cwp: bool,
    /// `enable_imp_msk_bld` (§ 5.4.6): `COMPOUND_AVERAGE` blends with the
    /// implicit mask (§ 7.13.3.14).
    pub implicit_mask: bool,
    /// `enable_df_sub_pu` (§ 5.4.6): inter frames code `allow_df_sub_pu`
    /// (§ 5.18.5.2) and TIP output frames code it `0`.
    pub df_sub_pu: bool,
    /// `df_par_bits_minus_2 = 1 + df_par_bits_extra` (§ 5.4.10), `0..=2`:
    /// frames code `df_delta_q` in `f(3 + df_par_bits_extra)` (§ 5.18.5.2).
    pub df_par_bits_extra: u8,
    /// `seq_enabled_motion_modes[INTERINTRA]` with
    /// `seq_frame_motion_modes_present_flag` (§ 5.4.6): every inter frame
    /// enables inter-intra blocks (§ 5.20.7.15).
    pub interintra: bool,
    /// `enable_bawp` (§ 5.4.6): every inter frame codes `allow_bawp = 1`, so
    /// single-reference blocks may code block adaptive weighted prediction
    /// (§ 5.20.7.6, § 7.13.3.25).
    pub bawp: bool,
    /// `seq_cropping_window_present_flag` with these offsets in luma samples
    /// of the maximum frame (§ 5.4.1); the decoder outputs the
    /// `CropWidth x CropHeight` window (§ 5.18.4.4, § 7.21.2).
    pub crop: Option<CroppingWindow>,
}

/// Stream-level inputs of [`build_encoder_sequence_header`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncoderStreamParams {
    /// Luma width in samples, `1..=65536`.
    pub width: u32,
    /// Luma height in samples, `1..=65536`.
    pub height: u32,
    /// `bit_depth_idc`.
    pub bit_depth: BitDepthIdc,
    /// `chroma_format_idc`.
    pub chroma: ChromaFormatIdc,
    /// Sequence superblock size.
    pub sb_size: SuperblockSize,
    /// Optional coding tools.
    pub tools: EncoderToolFlags,
}

/// Frame-level loop-filter choices of [`build_key_frame_header`].
///
/// `Default` turns every filter off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameFilterParams {
    /// `apply_deblocking_filter[0..4]` (§ 5.18.5.2). Flags 2 and 3 need flag 0 or 1.
    pub deblocking: [bool; 4],
    /// `DfDeltaQ[0..4]` (§ 5.18.5.2), `0` for a plane that is not filtered. Each
    /// offset must fit `f(dfParBits)` after the `1 << (dfParBits - 1)` bias.
    pub deblocking_delta_q: [i32; 4],
    /// `CdefDamping`, `3..=6` (§ 5.18.7.10). Ignored when CDEF is off.
    pub cdef_damping: u8,
    /// The CDEF strength sets (§ 5.18.7.10); empty turns CDEF off.
    pub cdef_strengths: CdefStrengthSets,
    /// `cdef_on_skip_txfm_frame_enable` (§ 5.18.7.10) of a frame with CDEF
    /// on; the encoder sequence codes `CDEF_ON_SKIP_TXFM_ADAPTIVE`.
    pub cdef_on_skip: bool,
    /// `allow_df_sub_pu` (§ 5.18.5.2) of an inter frame of a sequence with
    /// `enable_df_sub_pu`: deblocking also filters prediction sub-unit edges.
    pub deblocking_sub_pu: bool,
    /// `LoopRestorationSize[0]` of luma restoration (§ 5.18.7.11); `None`
    /// turns luma restoration off.
    pub lr_luma_unit_size: Option<u32>,
    /// The luma frame Wiener-NS filters; `None` codes `RESTORE_PC_WIENER`
    /// when `lr_luma_unit_size` is set.
    pub lr_luma_wiener_ns: Option<FrameWienerNs>,
    /// `LoopRestorationSize[1]` of chroma restoration (§ 5.18.7.11); `None`
    /// turns chroma restoration off.
    pub lr_chroma_unit_size: Option<u32>,
    /// The `RESTORE_WIENER_NONSEP` frame filters of U and V; `None` codes
    /// `RESTORE_NONE`. `switchable` must be `false`.
    pub lr_chroma_wiener_ns: [Option<FrameWienerNs>; 2],
    /// `ccso_params()` of each plane (§ 5.18.7.12); the frame codes
    /// `ccso_frame_flag = 1` when a plane sets `ccso_planes`. An inter frame
    /// codes `reuse_ccso`, `sb_reuse_ccso` and `ccso_ref_idx` of each plane and
    /// skips the parameters of a `reuse_ccso` plane.
    pub ccso: [CcsoPlaneParams; 3],
    /// `gdf_params()` (§ 5.18.7.9) of a frame with `gdf_frame_enable = 1`;
    /// `None` codes `gdf_frame_enable = 0` when the sequence enables GDF.
    pub gdf: Option<FrameGdf>,
}

/// A frame coded with `gdf_frame_enable = 1` (§ 5.18.7.9). It codes
/// `gdf_per_block = 1` whenever the bit is coded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameGdf {
    /// `gdf_pic_qc_idx`, `0..=3`.
    pub qc_idx: u8,
    /// `gdf_pic_scale_idx`, `0..=3`.
    pub scale_idx: u8,
}

/// A plane coded with `frame_filters_on = 1` (§ 5.18.7.11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameWienerNs {
    /// `RESTORE_SWITCHABLE` when `true`, else `RESTORE_WIENER_NONSEP`.
    pub switchable: bool,
    /// `rst_ref_pic_idx` of `temporal_pred_flag = 1`; `None` codes `bank`.
    pub rst_ref_pic_idx: Option<u8>,
    /// The coded bank (§ 5.20.10.6); a luma bank's class count is `NumFilterClasses`.
    pub bank: WienerNsFrameFilterBank,
}

/// Transform choices of [`build_key_frame_header`].
///
/// `Default` is `TX_MODE_LARGEST` with `reduced_tx_set = 2`, under which every
/// intra luma block infers `DCT_DCT` (§ 5.20.8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyFrameTxParams {
    /// `TX_MODE_LARGEST` or `TX_MODE_SELECT`; a lossy key frame cannot code `ONLY_4X4`.
    pub tx_mode: TxMode,
    /// `reduced_tx_set`, `0..=3`: the § 5.20.8.3 `get_tx_set` restriction.
    pub reduced_tx_set: u8,
    /// `allow_tcq`; needs a sequence with `choose_tcq_per_frame`.
    pub allow_tcq: bool,
    /// `delta_q_res` of a frame that codes `delta_q_present` (§ 5.18.7.8), `0..=3`.
    pub delta_q_res: Option<u8>,
    /// `allow_intrabc` of a key frame, with `allow_global_intrabc = 1`
    /// (§ 5.18.3.4); an inter frame refuses it.
    pub allow_intrabc: bool,
    /// With `allow_intrabc`, `allow_local_intrabc` (else 0).
    pub allow_local_intrabc: bool,
    /// `grain_seed` of an output frame of a sequence with
    /// `film_grain_params_present`, which codes `apply_grain = 1` with
    /// `fgm_id = 0` (§ 5.18.10.1); `None` codes `apply_grain = 0`.
    pub grain_seed: Option<u16>,
    /// `disable_cdf_update` (§ 5.18.2): the frame codes every symbol with
    /// CDFs that do not adapt.
    pub disable_cdf_update: bool,
    /// `disable_cross_frame_cdf_init = 1` with a primary reference (§ 5.18.2):
    /// an inter frame starts from the default CDFs; a key frame ignores it.
    pub disable_cross_frame_cdf_init: bool,
    /// `signal_primary_ref_frame = 0` (§ 5.18.2): an inter frame codes
    /// `PRIMARY_REF_CHOOSE`, so the decoder derives `primary_ref_frame`; a
    /// key frame ignores it.
    pub choose_primary_ref_frame: bool,
    /// `long_term_id_plus_1` of a key frame (§ 5.18.2), below
    /// `1 << long_term_frame_id_bits`; `0` gives it no long-term id.
    pub long_term_id_plus_1: u32,
    /// `using_qmatrix = 1` with this § 5.18.6.2 level, `0..=15`, in `qm_y`,
    /// `qm_u` and `qm_v`; `None` codes `using_qmatrix = 0`.
    pub qm_level: Option<u8>,
    /// `[DeltaQYDc, DeltaQUDc = DeltaQUAc, DeltaQVDc = DeltaQVAc]` (§ 5.18.6.1),
    /// each in the `su(7)` range; the sequence [`EncoderToolFlags::plane_delta_q`]
    /// must enable each non-zero value.
    pub plane_delta_q: [i8; 3],
    /// `segmentation_enabled` (§ 5.18.7.1) with segments 0..=3 enabling
    /// `SEG_LVL_ALT_Q` at these data, an updated map and no temporal update;
    /// `None` codes `segmentation_enabled = 0`. Every segment shares the
    /// one `qm_level` (`pic_qm_num_minus_1 = 0`).
    pub segment_alt_q: Option<[i16; 4]>,
    /// `cur_mfh_id = 1` (§ 5.18.2): the frame loads its sequence header from
    /// the multi-frame header of
    /// [`encode_multi_frame_header_obu`](crate::headers::frame::encode_multi_frame_header_obu);
    /// `false` codes `cur_mfh_id = 0`.
    pub multi_frame_header: bool,
}

impl Default for KeyFrameTxParams {
    fn default() -> Self {
        Self {
            tx_mode: TxMode::Largest,
            reduced_tx_set: 2,
            allow_tcq: false,
            delta_q_res: None,
            allow_intrabc: false,
            allow_local_intrabc: false,
            grain_seed: None,
            disable_cdf_update: false,
            disable_cross_frame_cdf_init: false,
            choose_primary_ref_frame: false,
            long_term_id_plus_1: 0,
            qm_level: None,
            plane_delta_q: [0; 3],
            segment_alt_q: None,
            multi_frame_header: false,
        }
    }
}

/// Writes `cur_mfh_id` (§ 5.18.2): `1` with `multi_frame_header`, else `0`
/// followed by `seq_header_id_in_frame_header = 0`.
pub(crate) fn write_mfh_reference(
    w: &mut BitWriter,
    multi_frame_header: bool,
) -> Result<(), WriteError> {
    w.write_uvlc(u32::from(multi_frame_header))?; // cur_mfh_id
    if !multi_frame_header {
        w.write_uvlc(0)?; // seq_header_id_in_frame_header
    }
    Ok(())
}

/// The § 5.18.2 state of the multi-frame header of
/// [`encode_multi_frame_header_obu`](crate::headers::frame::encode_multi_frame_header_obu),
/// which codes no frame size, deblocking update or segmentation (§ 5.7);
/// `None` for a frame with `cur_mfh_id = 0`.
pub(crate) const fn encoder_mfh_view(
    seq: &CoreSeqView,
    multi_frame_header: bool,
) -> Option<MfhFrameView> {
    if !multi_frame_header {
        return None;
    }
    Some(MfhFrameView {
        default_dims: (seq.max_frame_width, seq.max_frame_height),
        seg: None,
        deblocking: MfhDeblockingView {
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
        },
    })
}

/// The uniform tile layout of a sequence with `seq_tile_info_present_flag`
/// (§ 5.4.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeqTiles {
    /// `TileColsLog2` before the cap at `maxLog2TileCols`.
    pub cols_log2: u8,
    /// `TileRowsLog2` before the cap at `maxLog2TileRows`.
    pub rows_log2: u8,
    /// `allow_tile_info_change`: frames code `reuse_tile_info`.
    pub allow_tile_info_change: bool,
}

/// The tile layout of the encoder frame headers (§ 5.18.7.3): uniform, or
/// explicit with [`Self::explicit`].
///
/// `Default` is one tile with `TileSizeBytes = 4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTiles {
    /// `TileColsLog2` before the cap at `maxLog2TileCols`.
    pub cols_log2: u8,
    /// `TileRowsLog2` before the cap at `maxLog2TileRows`.
    pub rows_log2: u8,
    /// `TileSizeBytes`, `1..=4`, which a frame of more than one tile codes.
    pub size_bytes: u8,
    /// `context_update_tile_id = TileCols * TileRows / 2` (§ 5.18.7.2), the
    /// middle tile, instead of `0` in a frame that codes it.
    pub middle_cdf_update_tile: bool,
    /// `reuse_tile_info` of a frame that codes it (`allow_tile_info_change`
    /// and an eligible sequence layout, § 5.18.7.2).
    pub reuse_tile_info: bool,
    /// The column and row starts, in sequence superblocks, of a non-uniform
    /// layout (`uniform_tile_spacing_flag = 0`, § 5.18.7.3) in place of
    /// `cols_log2` and `rows_log2`, which are then `0`; each start but the
    /// first is inside the frame. An intra frame of a 256x256 sequence codes
    /// them in its 128x128 superblocks (§ 5.18.2).
    pub explicit: Option<[TileStarts; 2]>,
}

impl Default for FrameTiles {
    fn default() -> Self {
        Self {
            cols_log2: 0,
            rows_log2: 0,
            size_bytes: 4,
            middle_cdf_update_tile: false,
            reuse_tile_info: true,
            explicit: None,
        }
    }
}

impl FrameTiles {
    /// Checks that the intra and the inter frames of `seq` code this layout
    /// (§ 5.18.7.3).
    ///
    /// # Errors
    /// Returns the [`WriteError`] of a layout that `tile_params()` does not code.
    pub fn check(&self, seq: &CoreSeqView) -> Result<(), WriteError> {
        for frame_is_intra in [true, false] {
            let sb_size = seq.tile.frame_sb_size(frame_is_intra);
            write_frame_tile_params(&mut BitWriter::new(), seq, self, sb_size)?;
        }
        Ok(())
    }
}

/// Error from the encoder header builders.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EncoderHeaderError {
    /// A frame dimension is outside `1..=65536` (§ 5.4.1).
    #[error("frame dimension {value} is outside 1..=65536")]
    InvalidDimension {
        /// The rejected dimension.
        value: u32,
    },
    /// The cropping window does not fit the frame ([`CroppingWindow::fits`]).
    #[error("cropping window {window:?} does not fit a {width}x{height} frame")]
    InvalidCroppingWindow {
        /// The rejected window.
        window: CroppingWindow,
        /// The maximum frame width.
        width: u32,
        /// The maximum frame height.
        height: u32,
    },
    /// `base_q_idx` does not fit the § 5.18.6.1 field.
    #[error("base_q_idx {base_q_idx} is outside 0..{limit}")]
    InvalidBaseQIdx {
        /// The rejected value.
        base_q_idx: u32,
        /// One past the largest accepted value.
        limit: u32,
    },
    /// A coded-lossless frame (`base_q_idx = 0`, § 5.18.6.1) with a tool that
    /// makes a segment lossy or that lossless does not use.
    #[error("a lossless frame cannot code {what}")]
    LosslessWith {
        /// The refused tool.
        what: &'static str,
    },
    /// The transform choices are not codable: `ONLY_4X4` in a lossy frame,
    /// another mode in a lossless frame, or `reduced_tx_set > 3`.
    #[error("transform mode {tx_mode:?} with reduced_tx_set {reduced_tx_set} is not codable")]
    InvalidTxParams {
        /// The rejected transform mode.
        tx_mode: TxMode,
        /// The rejected `reduced_tx_set`.
        reduced_tx_set: u8,
    },
    /// `delta_q_res` is set with `base_q_idx = 0`, which codes no
    /// `delta_q_present` (§ 5.18.7.8).
    #[error("delta_q_res needs base_q_idx > 0")]
    DeltaQWithoutBaseQIdx,
    /// `allow_tcq` is set, but the sequence does not code it per frame.
    #[error("allow_tcq needs choose_tcq_per_frame")]
    TcqNotChosen,
    /// The sequence view is a single-picture view; the key-frame builder needs a stream view.
    #[error("single-picture sequence views are not supported")]
    SinglePictureView,
    /// The canonical bits did not serialize.
    #[error("header serialization failed: {0}")]
    Write(#[from] WriteError),
    /// The canonical bits did not parse.
    #[error("header parse failed: {0}")]
    Parse(#[from] crate::error::Error),
    /// The parse stopped before the whole header.
    #[error("canonical header parsed only partially")]
    Incomplete,
}

/// Builds a non-single-picture, non-still sequence header for `params`.
///
/// Every tool outside `params.tools` is off, the level is `31`, the frame size is
/// the maximum frame size, and quantizer offsets are zero.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for an out-of-range dimension, or if the
/// canonical bits do not parse completely.
pub fn build_encoder_sequence_header(
    params: &EncoderStreamParams,
) -> Result<SequenceHeader, EncoderHeaderError> {
    let bytes = sequence_header_bits(params)?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let header = parse_sequence_header(&mut reader)?;
    if !header.is_fully_parsed() {
        return Err(EncoderHeaderError::Incomplete);
    }
    Ok(header)
}

fn dimension_bits(value: u32) -> Result<u32, EncoderHeaderError> {
    if !(1..=(1 << 16)).contains(&value) {
        return Err(EncoderHeaderError::InvalidDimension { value });
    }
    Ok(ceil_log2(value).max(1))
}

fn sequence_header_bits(params: &EncoderStreamParams) -> Result<Vec<u8>, EncoderHeaderError> {
    let tools = params.tools;
    let monochrome = params.chroma.is_monochrome();
    let width_bits = dimension_bits(params.width)?;
    let height_bits = dimension_bits(params.height)?;
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // seq_header_id
    w.write_bits(profile_idc(params.chroma), 5)?;
    w.write_flag(false)?; // single_picture_header_flag
    w.write_bits(UNCONSTRAINED_LEVEL_IDX, 5)?;
    w.write_flag(false)?; // seq_tier
    w.write_uvlc(u32::from(params.chroma.get()))?;
    w.write_uvlc(u32::from(params.bit_depth.get()))?;
    w.write_bits(0, 3)?; // seq_lcr_id
    w.write_flag(false)?; // still_picture
    w.write_bits(0, 2)?; // max_tlayer_id
    w.write_bits(0, 3)?; // max_mlayer_id
    w.write_flag(!tools.implicit_output)?; // monotonic_output_order_flag
    w.write_bits(width_bits - 1, 4)?;
    w.write_bits(height_bits - 1, 4)?;
    w.write_bits(params.width - 1, width_bits)?;
    w.write_bits(params.height - 1, height_bits)?;
    w.write_flag(tools.crop.is_some())?; // seq_cropping_window_present_flag
    if let Some(window) = tools.crop {
        if !window.fits(params.width, params.height, params.chroma) {
            return Err(EncoderHeaderError::InvalidCroppingWindow {
                window,
                width: params.width,
                height: params.height,
            });
        }
        for offset in [window.left, window.right, window.top, window.bottom] {
            w.write_uvlc(offset)?;
        }
    }
    w.write_flag(false)?; // seq_initial_display_delay_present_flag
    w.write_flag(false)?; // decoder_model_info_present_flag

    w.write_flag(params.sb_size == SuperblockSize::Block256x256)?;
    if params.sb_size != SuperblockSize::Block256x256 {
        w.write_flag(params.sb_size == SuperblockSize::Block128x128)?;
    }
    if !monochrome {
        w.write_flag(tools.sdp)?;
    }
    if tools.sdp && !monochrome {
        w.write_flag(tools.extended_sdp)?;
    }
    w.write_flag(tools.ext_partitions)?;
    if tools.ext_partitions {
        w.write_flag(tools.uneven_4way_partitions)?;
    }
    w.write_flag(false)?; // reduce_pb_aspect_ratio

    w.write_flag(false)?; // enable_ext_seg
    w.write_flag(false)?; // seq_seg_info_present_flag

    w.write_flag(tools.dip)?;
    w.write_flag(tools.intra_edge_filter)?;
    w.write_flag(tools.mrls)?;
    w.write_flag(tools.cfl)?;
    if !monochrome {
        w.write_bits_u8(tools.cfl_ds_filter, 2)?;
    }
    w.write_flag(tools.mhccp)?;
    w.write_flag(tools.ibp)?;

    write_inter_config(&mut w, tools)?;

    w.write_flag(tools.screen_content)?; // seq_choose_screen_content_tools
    if tools.screen_content {
        w.write_flag(true)?; // seq_choose_integer_mv
    } else {
        w.write_flag(false)?; // seq_force_screen_content_tools
    }

    write_transform_quant_entropy_config(&mut w, tools, monochrome)?;

    w.write_flag(tools.disable_loopfilters_across_tiles)?;
    w.write_flag(tools.cdef)?;
    w.write_flag(tools.gdf)?;
    if tools.gdf && params.sb_size == SuperblockSize::Block64x64 {
        w.write_flag(false)?; // gdf_unit_matches_sb_size
    }
    w.write_flag(tools.loop_restoration)?;
    if tools.loop_restoration {
        w.write_flag(false)?; // lr_tools_disable[0][RESTORE_PC_WIENER]
        w.write_flag(false)?; // lr_tools_disable[0][RESTORE_WIENER_NONSEP]
        w.write_flag(false)?; // lr_tools_uv_present
    }
    w.write_flag(tools.ccso)?;
    if tools.ccso {
        w.write_flag(false)?; // ccso_unit_matches_sb_size
    }
    w.write_flag(false)?; // cdef_on_skip_txfm_always_on
    w.write_flag(false)?; // cdef_on_skip_txfm_disabled
    w.write_bits(DF_PAR_BITS_MINUS_2 + u32::from(tools.df_par_bits_extra), 2)?;

    w.write_flag(tools.seq_tiles.is_some())?; // seq_tile_info_present_flag
    if let Some(tiles) = tools.seq_tiles {
        w.write_flag(tiles.allow_tile_info_change)?;
        let input = TileParamsInput {
            frame_width: params.width,
            frame_height: params.height,
            uniform_sb_size: params.sb_size,
            sb_size: params.sb_size,
            is_bridge: false,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(UNCONSTRAINED_LEVEL_IDX as u8),
        };
        let layout = uniform_tile_layout(&input, tiles.cols_log2, tiles.rows_log2)?;
        write_tile_params(
            &mut w,
            &layout.params,
            &layout.sb_col_starts,
            &layout.sb_row_starts,
            &input,
        )?;
    }
    w.write_flag(tools.film_grain)?; // film_grain_params_present
    w.align_to_byte();
    Ok(w.into_bytes())
}

const fn profile_idc(chroma: ChromaFormatIdc) -> u32 {
    match chroma {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => 0,
        ChromaFormatIdc::Yuv422 => 3,
        ChromaFormatIdc::Yuv444 => 4,
    }
}

/// `num_same_ref_compound`: the first two ranked references may pair with
/// themselves in a compound block (§ 5.20.7.11).
const NUM_SAME_REF_COMPOUND: u32 = 2;

/// § 5.4.6 for a non-single-picture header with the motion vector prediction,
/// difference and compound tools of `tools` and every other inter tool off.
fn write_inter_config(w: &mut BitWriter, tools: EncoderToolFlags) -> Result<(), WriteError> {
    let modes = (u32::from(tools.interintra) << 3) | if tools.warp { 0b0111 } else { 0 };
    w.write_bits(modes, 4)?; // seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]
    if modes != 0 {
        w.write_flag(true)?; // seq_frame_motion_modes_present_flag
    }
    if tools.warp {
        w.write_flag(true)?; // enable_six_param_warp_delta
    }
    w.write_flag(tools.masked_compound)?; // enable_masked_compound
    w.write_flag(tools.ref_frame_mvs)?;
    if tools.ref_frame_mvs {
        w.write_flag(false)?; // reduced_ref_frame_mvs_mode
    }
    w.write_bits(ORDER_HINT_BITS_MINUS_1, 4)?;
    w.write_flag(tools.refmvbank)?;
    w.write_flag(tools.drl_reorder.is_none())?; // disable_drl_reorder
    if let Some(constrain) = tools.drl_reorder {
        w.write_flag(constrain)?; // constrain_drl_reorder
    }
    w.write_flag(tools.explicit_ref_frame_map)?;
    w.write_flag(tools.num_ref_frames.is_some())?; // explicit_num_ref_frames
    if let Some(slots) = tools.num_ref_frames {
        if !(1..=16).contains(&slots) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "num_ref_frames_minus_1",
            });
        }
        w.write_bits(u32::from(slots) - 1, 4)?; // num_ref_frames_minus_1
    }
    if tools.long_term_frame_id_bits > 7 {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "long_term_frame_id_bits",
        });
    }
    w.write_bits(u32::from(tools.long_term_frame_id_bits), 3)?;
    w.write_ns(tools.max_drl_bits_minus_1, 5)?; // ns(MAX_REF_MV_STACK_SIZE - 1)
    w.write_flag(false)?; // allow_frame_max_drl_bits
    w.write_ns(tools.max_bvp_drl_bits_minus_1, 3)?; // ns(MAX_REF_BV_STACK_SIZE - 1)
    w.write_flag(false)?; // allow_frame_max_bvp_drl_bits
    w.write_bits(NUM_SAME_REF_COMPOUND, 2)?;
    w.write_flag(tools.tip)?; // enable_tip
    if tools.tip {
        w.write_flag(!tools.tip_output)?; // disable_tip_output
        w.write_flag(tools.tip_hole_fill)?; // enable_tip_hole_fill
    }
    w.write_flag(tools.mv_traj)?; // enable_mv_traj
    w.write_flag(tools.bawp)?; // enable_bawp
    w.write_flag(tools.cwp)?; // enable_cwp
    w.write_flag(tools.implicit_mask)?; // enable_imp_msk_bld
    w.write_flag(tools.df_sub_pu)?; // enable_df_sub_pu
    if tools.tip && tools.tip_output && tools.df_sub_pu {
        w.write_flag(tools.tip_explicit_qp)?; // enable_tip_explicit_qp
    }
    w.write_bits(if tools.opfl_refine { 3 } else { 0 }, 2)?; // enable_opfl_refine
    w.write_flag(tools.refinemv)?; // enable_refinemv
    if tools.tip && (tools.opfl_refine || tools.refinemv) {
        w.write_flag(tools.tip_refinemv)?; // enable_tip_refinemv
    }
    w.write_flag(false)?; // enable_bru
    for flag in tools.mvd_tools {
        w.write_flag(flag)?; // adaptive_mvd, mvd_sign_derive, flex_mvres
    }
    w.write_flag(tools.global_motion)?; // enable_global_motion
    w.write_flag(tools.short_refresh) // enable_short_refresh_frame_flags
}

/// § 5.4.8 with zero `Base*DeltaQ` and the frame delta enables of
/// [`EncoderToolFlags::plane_delta_q`].
fn write_transform_quant_entropy_config(
    w: &mut BitWriter,
    tools: EncoderToolFlags,
    monochrome: bool,
) -> Result<(), WriteError> {
    w.write_flag(tools.fsc)?;
    if !tools.fsc {
        w.write_flag(tools.idtx_intra)?;
    }
    w.write_flag(tools.intra_ist)?;
    w.write_flag(tools.inter_ist)?;
    if !monochrome {
        w.write_flag(false)?; // enable_chroma_dctonly
    }
    w.write_flag(tools.inter_ddt)?;
    w.write_flag(false)?; // reduced_tx_part_set
    if !monochrome {
        w.write_flag(tools.cctx)?;
    }
    w.write_flag(tools.tcq)?;
    if tools.tcq {
        w.write_flag(true)?; // choose_tcq_per_frame
    }
    w.write_flag(tools.parity_hiding)?;
    w.write_flag(tools.avg_cdf.is_some())?; // enable_avg_cdf
    if let Some(across_tiles) = tools.avg_cdf {
        w.write_flag(across_tiles)?; // avg_cdf_type
    }
    let [y, u, v] = tools.plane_delta_q.map(|delta| delta != 0);
    let uv = !monochrome && (u || v);
    if !monochrome {
        w.write_flag(tools.plane_delta_q[1] != tools.plane_delta_q[2])?; // separate_uv_delta_q
    }
    w.write_flag(!y)?; // equal_ac_dc_q
    if y {
        w.write_bits(ZERO_BASE_DELTA_Q, 5)?; // base_y_dc_delta_q
        w.write_flag(true)?; // y_dc_delta_q_enabled
    }
    if !monochrome {
        if y {
            w.write_bits(ZERO_BASE_DELTA_Q, 5)?; // base_uv_dc_delta_q
            w.write_flag(uv)?; // uv_dc_delta_q_enabled
        }
        w.write_bits(ZERO_BASE_DELTA_Q, 5)?; // base_uv_ac_delta_q
        w.write_flag(uv)?; // uv_ac_delta_q_enabled
    }
    Ok(())
}

/// Builds the `OBU_CLOSED_LOOP_KEY` frame header for `seq` at `base_q_idx`.
///
/// The frame has the maximum frame size, `show_frame` semantics through
/// `immediate_output_frame = 1`, the uniform `tiles`, no segmentation, the delta-q of `tx`,
/// no quantizer matrices, the deblocking, CDEF, and loop-restoration choices of
/// `filters`, GDF off, the transform choices of `tx`, and no film grain.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a zero or too-wide `base_q_idx`, a
/// single-picture view, `filters` that the sequence or the syntax cannot carry,
/// `tx` that is not codable, or canonical bits that do not parse completely.
pub fn build_key_frame_header(
    seq: &CoreSeqView,
    base_q_idx: u32,
    order_hint: u32,
    filters: &FrameFilterParams,
    tx: KeyFrameTxParams,
    tiles: FrameTiles,
) -> Result<FrameHeaderCore, EncoderHeaderError> {
    let bytes = key_frame_header_bits(seq, (base_q_idx, tiles), order_hint, filters, tx)?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
    parse_core_body(
        &mut reader,
        &mut core,
        seq,
        encoder_mfh_view(seq, tx.multi_frame_header).as_ref(),
        &FrameReferenceStateView::unknown(),
    )?;
    core.consumed_bits = reader.consumed_bits();
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(EncoderHeaderError::Incomplete);
    }
    Ok(core)
}

fn key_frame_header_bits(
    seq: &CoreSeqView,
    (base_q_idx, tiles): (u32, FrameTiles),
    order_hint: u32,
    filters: &FrameFilterParams,
    tx: KeyFrameTxParams,
) -> Result<Vec<u8>, EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    let mut w = BitWriter::new();
    write_mfh_reference(&mut w, tx.multi_frame_header)?;
    if tx.long_term_id_plus_1 >> seq.long_term_frame_id_bits != 0 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "long_term_id_plus_1",
        }
        .into());
    }
    w.write_bits(tx.long_term_id_plus_1, seq.long_term_frame_id_bits)?;
    w.write_flag(true)?; // immediate_output_frame
    w.write_flag(false)?; // frame_size_override_flag
    w.write_bits(order_hint & order_hint_mask, seq.order_hint_bits)?;
    write_intra_frame_tail(&mut w, seq, (base_q_idx, tiles, true), filters, tx)?;
    Ok(w.into_bytes())
}

/// § 5.18.2 of an intra frame from `screen_content_params()` to
/// `film_grain_config()`: the `base_q_idx`, `tiles`, `filters` and `tx` of a
/// frame with `immediate_output_frame` `show`, then the byte alignment.
pub(super) fn write_intra_frame_tail(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    (base_q_idx, tiles, show): (u32, FrameTiles, bool),
    filters: &FrameFilterParams,
    tx: KeyFrameTxParams,
) -> Result<(), EncoderHeaderError> {
    check_tx_params(base_q_idx, tx)?;
    if seq.seq_force_screen_content_tools == 2 {
        w.write_flag(true)?; // allow_screen_content_tools
        if seq.seq_force_integer_mv == 2 {
            w.write_flag(false)?; // force_integer_mv
        }
    }
    let intrabc = IntrabcParams {
        allow_intrabc: tx.allow_intrabc,
        allow_global_intrabc: tx.allow_intrabc.then_some(true),
        allow_local_intrabc: tx.allow_intrabc.then_some(tx.allow_local_intrabc),
        change_bvp_drl: (tx.allow_intrabc && seq.allow_frame_max_bvp_drl_bits).then_some(false),
        max_bvp_drl_bits_minus_1: None,
    };
    write_intrabc_params(w, &intrabc, true, seq.allow_frame_max_bvp_drl_bits)?;
    w.write_flag(tx.disable_cdf_update)?;
    write_quant_and_filters(
        w,
        seq,
        (base_q_idx, tiles, tx),
        filters,
        (true, LrReferenceFilters::default(), None),
    )?;
    if base_q_idx > 0 {
        w.write_flag(tx.tx_mode == TxMode::Select)?; // tx_mode_select
    }
    w.write_bits(u32::from(tx.reduced_tx_set), 2)?;
    write_film_grain_config(w, seq, show, tx.grain_seed)?;
    w.align_to_byte();
    Ok(())
}

/// Refuses `tx` unless its `tx_mode` is `ONLY_4X4` exactly when `base_q_idx = 0`
/// (§ 5.18.8.1 `CodedLossless`) and `reduced_tx_set` fits.
pub(super) fn check_tx_params(
    base_q_idx: u32,
    tx: KeyFrameTxParams,
) -> Result<(), EncoderHeaderError> {
    if (tx.tx_mode == TxMode::Only4x4) != (base_q_idx == 0)
        || tx.reduced_tx_set > MAX_REDUCED_TX_SET
    {
        return Err(EncoderHeaderError::InvalidTxParams {
            tx_mode: tx.tx_mode,
            reduced_tx_set: tx.reduced_tx_set,
        });
    }
    Ok(())
}

/// § 5.18.10.1 `film_grain_config()` of a frame with `immediate_output_frame`
/// `output`: `apply_grain`, then `fgm_id = 0` and `grain_seed` when `seed`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a seed that the frame cannot code.
pub(super) fn write_film_grain_config(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    output: bool,
    seed: Option<u16>,
) -> Result<(), EncoderHeaderError> {
    if seq.film_grain_params_present != Some(true) || !output {
        return match seed {
            Some(_) => Err(WriteError::NonCanonicalFrameHeader { what: "grain_seed" }.into()),
            None => Ok(()),
        };
    }
    w.write_flag(seed.is_some())?; // apply_grain
    if let Some(seed) = seed {
        w.write_bits(0, 3)?; // fgm_id
        w.write_bits(u32::from(seed), 16)?;
    }
    Ok(())
}

/// The § 5.18.6.1 model of `base_q_idx` with the plane deltas of `tx`: chroma
/// codes one value for DC and AC, and V differs from U only with
/// `separate_uv_delta_q`.
pub(super) fn plane_quantization(
    seq: &CoreSeqView,
    base_q_idx: u32,
    tx: KeyFrameTxParams,
) -> QuantizationParams {
    let chroma = seq.quant.num_planes > 1;
    let [y, u, v] = tx.plane_delta_q.map(i32::from);
    let [u, v] = if chroma { [u, v] } else { [0, 0] };
    QuantizationParams {
        base_q_idx,
        delta_q_y_dc: y,
        delta_q_u_dc: u,
        delta_q_u_ac: u,
        delta_q_v_dc: v,
        delta_q_v_ac: v,
        diff_uv_delta: chroma && seq.quant.separate_uv_delta_q && u != v,
    }
}

/// § 5.18.2 from `tile_info()` through `ccso_params()`: the uniform `tiles`, `base_q_idx`, no
/// segmentation, the quantizer-matrix level and the delta-q and `allow_tcq` of `tx`, parity hiding when
/// the sequence enables it and the frame has no TCQ, and the filters of `filters`; an inter
/// frame codes no CCSO reuse.
pub(super) fn write_quant_and_filters(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    (base_q_idx, tiles, tx): (u32, FrameTiles, KeyFrameTxParams),
    filters: &FrameFilterParams,
    (frame_is_intra, lr_refs, coded_map): (bool, LrReferenceFilters<'_>, Option<bool>),
) -> Result<(), EncoderHeaderError> {
    let allow_tcq = tx.allow_tcq;
    if tx.delta_q_res.is_some() && base_q_idx == 0 {
        return Err(EncoderHeaderError::DeltaQWithoutBaseQIdx);
    }
    if allow_tcq && !seq.quant.choose_tcq_per_frame {
        return Err(EncoderHeaderError::TcqNotChosen);
    }
    let q_bits = if seq.quant.bit_depth == 8 { 8 } else { 9 };
    let lossless = base_q_idx == 0;
    let refused = [
        (allow_tcq, "allow_tcq"),
        (tx.segment_alt_q.is_some(), "segmentation"),
        (tx.qm_level.is_some(), "quantizer matrices"),
        (tx.plane_delta_q != [0; 3], "plane quantizer deltas"),
    ];
    if let Some(&(_, what)) = refused.iter().find(|&&(on, _)| on && lossless) {
        return Err(EncoderHeaderError::LosslessWith { what });
    }
    if base_q_idx >= 1 << q_bits {
        return Err(EncoderHeaderError::InvalidBaseQIdx {
            base_q_idx,
            limit: 1 << q_bits,
        });
    }
    let (spacing, sb_shift) = write_encoder_tile_info(w, seq, &tiles, frame_is_intra)?;
    write_quantization_params(
        w,
        &plane_quantization(seq, base_q_idx, tx),
        &seq.quant,
        false,
    )?;
    write_inter_segmentation_params(
        w,
        &segmentation_params(tx.segment_alt_q),
        (&seq.seg, None),
        coded_map,
    )?;
    let chroma = |level| if seq.quant.num_planes > 1 { level } else { 0 };
    let qm = SetupQmParams {
        using_qmatrix: tx.qm_level.is_some(),
        pic_qm_num_minus_1: 0,
        levels: core::array::from_fn(|i| match tx.qm_level {
            Some(level) if i == 0 => QmSetLevels {
                qm_y: level,
                qm_u: chroma(level),
                qm_v: chroma(level),
            },
            _ => QmSetLevels::default(),
        }),
    };
    write_setup_qm_params(w, &qm, &seq.quant, tx.segment_alt_q.is_some())?;
    if base_q_idx > 0 {
        w.write_flag(tx.delta_q_res.is_some())?; // delta_q_present
    }
    if let Some(res) = tx.delta_q_res {
        w.write_bits(u32::from(res), 2)?; // delta_q_res
    }
    if seq.quant.choose_tcq_per_frame && !lossless {
        w.write_flag(allow_tcq)?;
    }
    let allow_tcq = allow_tcq || (seq.quant.enable_tcq && !seq.quant.choose_tcq_per_frame);
    if seq.quant.enable_parity_hiding && !allow_tcq && !lossless {
        w.write_flag(true)?; // allow_parity_hiding
    }
    let [col_starts, row_starts] = spacing.map(|axis| {
        let starts: Vec<u32> = axis
            .starts
            .as_ref()
            .iter()
            .map(|&sb| sb << sb_shift)
            .collect();
        (starts, axis.count)
    });
    let mis = |frame: u32| 2 * frame.div_ceil(8);
    let gdf = GdfGeometry {
        sb_size: seq.tile.frame_sb_size(frame_is_intra),
        mi_cols: mis(seq.max_frame_width),
        mi_rows: mis(seq.max_frame_height),
        tile_cols: col_starts.1,
        tile_rows: row_starts.1,
        mi_col_starts: &col_starts.0,
        mi_row_starts: &row_starts.0,
    };
    let num_refs = lr_refs.num_ref_frames;
    write_filter_params(w, seq, filters, (lr_refs, gdf), (frame_is_intra, lossless))?;
    if seq.ccso.enable_ccso && !lossless {
        let planes = &filters.ccso[..usize::from(seq.quant.num_planes).min(3)];
        let on = planes.iter().any(|plane| plane.ccso_planes);
        w.write_flag(on)?; // ccso_frame_flag
        for plane in planes.iter().filter(|_| on) {
            w.write_flag(plane.ccso_planes)?;
            if plane.ccso_planes {
                write_ccso_reuse(w, plane, frame_is_intra, num_refs)?;
                if !plane.reuse_ccso {
                    let coded = CcsoPlaneParams {
                        reuse_ccso: false,
                        sb_reuse_ccso: false,
                        ccso_ref_idx: None,
                        ..*plane
                    };
                    write_ccso_plane_params(w, &coded)?;
                }
            }
        }
    }
    Ok(())
}

/// `reuse_ccso`, `sb_reuse_ccso` and `ccso_ref_idx` of an enabled plane of an
/// inter frame over `num_refs` references (§ 5.18.7.12); an intra frame codes
/// none and refuses them.
fn write_ccso_reuse(
    w: &mut BitWriter,
    plane: &CcsoPlaneParams,
    frame_is_intra: bool,
    num_refs: u32,
) -> Result<(), EncoderHeaderError> {
    let reuses = plane.reuse_ccso || plane.sb_reuse_ccso;
    if frame_is_intra {
        return if reuses || plane.ccso_ref_idx.is_some() {
            Err(WriteError::NonCanonicalFrameHeader {
                what: "CCSO reuse in an intra frame",
            }
            .into())
        } else {
            Ok(())
        };
    }
    w.write_flag(plane.reuse_ccso)?;
    w.write_flag(plane.sb_reuse_ccso)?;
    if reuses {
        let index = plane.ccso_ref_idx.unwrap_or(0);
        if index >= num_refs.max(1) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_ref_idx outside the references",
            }
            .into());
        }
        if num_refs > 1 {
            w.write_bits(index, crate::headers::frame::size::ceil_log2(num_refs))?;
        }
    }
    Ok(())
}

/// § 5.18.5.2 `deblocking_filter_params()`, § 5.18.7.9 `gdf_params()`,
/// § 5.18.7.10 `cdef_params()`, and § 5.18.7.11 `lr_params()` of `filters`.
/// The encoder codes every frame that is not intra as `INTER_FRAME`.
fn write_filter_params(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    filters: &FrameFilterParams,
    (lr_refs, gdf_geometry): (LrReferenceFilters<'_>, GdfGeometry<'_>),
    (frame_is_intra, lossless): (bool, bool),
) -> Result<(), WriteError> {
    let num_planes = seq.quant.num_planes;
    let delta = filters.deblocking_delta_q;
    let present = core::array::from_fn(|i| {
        filters.deblocking[i] && delta[i] != if i == 1 { delta[0] } else { 0 }
    });
    let mut deblocking = DeblockingFilterParams::new(filters.deblocking, present, delta);
    deblocking.allow_df_sub_pu = filters.deblocking_sub_pu;
    let df_par_bits_minus_2 = seq.filter.df_par_bits_minus_2;
    let sub_pu_coded = seq.filter.enable_df_sub_pu && !frame_is_intra;
    write_deblocking_filter_params(
        w,
        &deblocking,
        lossless,
        num_planes,
        df_par_bits_minus_2,
        sub_pu_coded,
        None,
    )?;
    let off = GdfParams {
        gdf_frame_enable: false,
        gdf_per_block: None,
        gdf_pic_qc_idx: None,
        gdf_pic_scale_idx: None,
    };
    let gdf = filters.gdf.map_or(off, |gdf| GdfParams {
        gdf_frame_enable: true,
        gdf_per_block: Some(gdf_per_block_is_coded(seq.filter, gdf_geometry)),
        gdf_pic_qc_idx: Some(gdf.qc_idx),
        gdf_pic_scale_idx: Some(gdf.scale_idx),
    });
    write_gdf_params(w, &gdf, lossless, &seq.filter, gdf_geometry)?;
    let cdef_on = !filters.cdef_strengths.is_empty();
    let cdef = CdefParams {
        cdef_frame_enable: cdef_on,
        cdef_damping: cdef_on.then_some(filters.cdef_damping),
        cdef_strengths: cdef_on.then_some(filters.cdef_strengths.len() as u8),
        cdef_on_skip_txfm_frame_enable: cdef_on.then_some(filters.cdef_on_skip),
        strengths: filters.cdef_strengths,
    };
    write_cdef_params(w, &cdef, lossless, num_planes, &seq.filter)?;
    let geometry = LrGeometry::new(gdf_geometry.sb_size, seq.chroma_format_idc);
    let chroma = |plane: u8| {
        filters
            .lr_chroma_wiener_ns
            .get(usize::from(plane).wrapping_sub(1))
            .copied()
            .flatten()
            .filter(|_| filters.lr_chroma_unit_size.is_some())
    };
    let uses_chroma = chroma(1).is_some() || chroma(2).is_some();
    let mut lr = LrParams {
        uses_lr: filters.lr_luma_unit_size.is_some() || uses_chroma,
        planes: crate::tile::InlineVec::default(),
        loop_restoration_size: default_restoration_size(geometry),
    };
    if seq.restoration.enable_restoration && !lossless {
        for plane in 0..num_planes {
            let luma = plane == 0 && filters.lr_luma_unit_size.is_some();
            let wiener = if plane == 0 {
                filters.lr_luma_wiener_ns.filter(|_| luma)
            } else {
                chroma(plane)
            };
            let params = match wiener {
                Some(choice) => LrPlaneParams {
                    restoration_type: if choice.switchable {
                        FrameRestorationType::Switchable
                    } else {
                        FrameRestorationType::WienerNonsep
                    },
                    frame_filters_on: true,
                    num_filter_classes: (plane == 0 && choice.rst_ref_pic_idx.is_none())
                        .then_some(choice.bank.classes.len() as u8),
                    frame_filter_bank: choice.rst_ref_pic_idx.is_none().then_some(choice.bank),
                    rst_ref_pic_idx: choice.rst_ref_pic_idx,
                },
                None if luma => LrPlaneParams {
                    restoration_type: FrameRestorationType::PcWiener,
                    ..LrPlaneParams::default()
                },
                None => LrPlaneParams::default(),
            };
            lr.planes
                .push(params)
                .ok_or(WriteError::NonCanonicalFrameHeader {
                    what: "lr_num_planes",
                })?;
        }
    }
    if let Some(size) = filters.lr_luma_unit_size {
        lr.loop_restoration_size[0] = size;
    }
    if let Some(size) = filters.lr_chroma_unit_size.filter(|_| uses_chroma) {
        lr.loop_restoration_size[1] = size;
        lr.loop_restoration_size[2] = size;
    }
    write_lr_params_for_inter(
        w,
        &lr,
        (lossless, num_planes),
        &seq.restoration,
        geometry,
        lr_refs,
    )
}

/// § 5.18.7.2 `tile_info()` of `tiles` (§ 5.18.7.3 uniform or explicit spacing):
/// `reuse_tile_info` when the sequence layout is eligible and may change; a frame
/// that reuses the sequence layout codes it in place of `tiles`, any other frame
/// codes `tiles` (uniform log2 capped at `maxLog2TileCols` and `maxLog2TileRows`);
/// then `context_update_tile_id` and `tiles.size_bytes` when the frame has more
/// than one tile. Returns the column and row spacing and the `sbShift2` of
/// `MiColStarts`.
pub(super) fn write_encoder_tile_info(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    tiles: &FrameTiles,
    frame_is_intra: bool,
) -> Result<([TileSpacing; 2], u32), WriteError> {
    let sb_size = seq.tile.frame_sb_size(frame_is_intra);
    let ([cols, rows], sb_shift) = match reused_tile_layout(seq, sb_size) {
        Some(layout) if !seq.tile.allow_tile_info_change || tiles.reuse_tile_info => {
            if seq.tile.allow_tile_info_change {
                w.write_flag(true)?; // reuse_tile_info
            }
            (layout, crate::tile::mi_width_log2(seq.tile.seq_sb_size))
        }
        reused => {
            if reused.is_some() {
                w.write_flag(false)?; // reuse_tile_info
            }
            write_frame_tile_params(w, seq, tiles, sb_size)?
        }
    };
    if cols.count > 1 || rows.count > 1 {
        if !seq.tile.enable_avg_cdf || seq.tile.avg_cdf_type == 0 {
            let bits =
                crate::tile::tile_log2(1, cols.count) + crate::tile::tile_log2(1, rows.count);
            let middle = cols.count * rows.count / 2;
            let id = if tiles.middle_cdf_update_tile {
                middle
            } else {
                0
            };
            w.write_bits(id, u32::from(bits))?; // context_update_tile_id
        }
        if !(1..=4).contains(&tiles.size_bytes) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            });
        }
        w.write_bits(u32::from(tiles.size_bytes) - 1, 2)?;
    }
    Ok(([cols, rows], sb_shift))
}

/// The § 5.18.7.4 `reuse_tile_params()` layout of the uniform sequence tiles
/// when a frame of superblock size `sb_size` is eligible to reuse them
/// (§ 5.18.7.2), else `None`.
fn reused_tile_layout(seq: &CoreSeqView, sb_size: SuperblockSize) -> Option<[TileSpacing; 2]> {
    let params = seq
        .tile
        .seq_tile_params
        .filter(|_| seq.tile.seq_tile_info_present_flag)?;
    let sb_log2 = crate::tile::mi_width_log2(sb_size);
    let mis = |frame: u32| 2 * frame.div_ceil(8);
    let sbs = |frame: u32| mis(frame).div_ceil(1 << sb_log2);
    let eligible = params.uniform_spacing
        && super::tiling::uniform_eligible(params.tile_rows_log2, sbs(seq.max_frame_height))
        && super::tiling::uniform_eligible(params.tile_cols_log2, sbs(seq.max_frame_width));
    eligible.then(|| {
        [
            (params.tile_cols_log2, seq.max_frame_width),
            (params.tile_rows_log2, seq.max_frame_height),
        ]
        .map(|(log2, frame)| crate::tile::uniform_spacing(log2, mis(frame), seq.tile.seq_sb_size))
    })
}

/// The § 5.18.7.3 `tile_params()` bits of `tiles` over the frame of `seq` in
/// superblocks of `sb_size`; returns the column and row spacing and `sbShift2`.
fn write_frame_tile_params(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    tiles: &FrameTiles,
    sb_size: SuperblockSize,
) -> Result<([TileSpacing; 2], u32), WriteError> {
    let input = TileParamsInput {
        frame_width: seq.max_frame_width,
        frame_height: seq.max_frame_height,
        uniform_sb_size: seq.tile.seq_sb_size,
        sb_size,
        is_bridge: false,
        seq_tier: seq.tile.seq_tier,
        seq_level_idx: seq.tile.seq_level_idx,
    };
    let layout = match tiles.explicit {
        None => uniform_tile_layout(&input, tiles.cols_log2, tiles.rows_log2)?,
        Some(_) if tiles.cols_log2 != 0 || tiles.rows_log2 != 0 => {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "tile log2 with explicit tile starts",
            });
        }
        Some(starts) => {
            let scale = crate::tile::mi_width_log2(seq.tile.seq_sb_size)
                .saturating_sub(crate::tile::mi_width_log2(sb_size));
            let [cols, rows] = starts.map(|axis| {
                axis.iter()
                    .map(|&start| start << scale)
                    .collect::<Vec<u32>>()
            });
            explicit_tile_layout(&input, &cols, &rows)?
        }
    };
    write_tile_params(
        w,
        &layout.params,
        &layout.sb_col_starts,
        &layout.sb_row_starts,
        &input,
    )?;
    let spacing = |starts, count| TileSpacing { starts, count };
    Ok((
        [
            spacing(layout.sb_col_starts, layout.params.tile_cols),
            spacing(layout.sb_row_starts, layout.params.tile_rows),
        ],
        layout.sb_shift2,
    ))
}

/// Serializes one Annex B temporal unit: a temporal delimiter, then `seq` when
/// `with_sequence_header`, then the key frame OBUs of `core` (closed- or
/// open-loop) that carry
/// the `tiles` of the frame in tile order in `tile_groups` tile groups (see
/// [`tile_group_ranges`](crate::headers::frame::tile_group_ranges)).
///
/// # Errors
/// Returns [`EncoderHeaderError`] if `seq` has no complete view, a tile is empty or
/// too long for `TileSizeBytes`, the tile count differs from the header, or an OBU
/// does not serialize. The OBUs `key_obus` (empty for none) follow the sequence
/// header: a content interpretation OBU, then metadata and film grain OBUs
/// (§ 7.3.3).
pub fn encode_key_frame_temporal_unit(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    with_sequence_header: bool,
    (tiles, tile_groups): (&[&[u8]], u32),
    key_obus: &[u8],
) -> Result<Vec<u8>, EncoderHeaderError> {
    let view = CoreSeqView::from_sequence(seq).ok_or(EncoderHeaderError::Incomplete)?;
    if !matches!(core.obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey) {
        return Err(EncoderHeaderError::Incomplete);
    }
    let mut unit = encode_temporal_delimiter_obu()?;
    let mut w = BitWriter::new();
    if with_sequence_header {
        let mut payload = BitWriter::new();
        write_obu_payload(
            &mut payload,
            &ParsedObu::SequenceHeader(Box::new(seq.clone())),
            ObuType::SequenceHeader.is_extensible_obu(),
            &[],
        )?;
        write_annexb_obu(
            &mut w,
            &obu_header(ObuType::SequenceHeader),
            &payload.into_bytes(),
        )?;
    }
    unit.extend_from_slice(&w.into_bytes());
    unit.extend_from_slice(key_obus);
    let groups = crate::headers::frame::encoder_inter_stream::write_tile_groups(
        core,
        core.obu_type,
        (tiles, tile_groups),
        |structure, tiles| {
            let mut payload = BitWriter::new();
            write_tile_group_obu(
                &mut payload,
                core,
                &view,
                encoder_mfh_view(&view, !core.cur_mfh_id.is_zero()).as_ref(),
                true,
                structure,
                &tile_framing(core, tiles),
                tiles,
                true,
            )?;
            Ok(payload.into_bytes())
        },
    )?;
    unit.extend_from_slice(&groups);
    Ok(unit)
}

/// The § 5.20.1 framing of `tiles` with the `TileSizeBytes` of `core`.
pub(super) fn tile_framing(core: &FrameHeaderCore, tiles: &[&[u8]]) -> TileGroupFraming {
    let sizes: Vec<u64> = tiles.iter().map(|tile| tile.len() as u64).collect();
    let size_bytes = core
        .tile_info
        .as_ref()
        .and_then(|info| info.tile_size_bytes)
        .unwrap_or(1);
    TileGroupFraming::tiles(&sizes, size_bytes)
}

pub(super) const fn obu_header(obu_type: ObuType) -> ObuHeader {
    ObuHeader {
        has_header_extension: false,
        obu_type,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
        header_size_bytes: 1,
    }
}

#[cfg(test)]
#[path = "encoder_stream_tests.rs"]
mod tests;
