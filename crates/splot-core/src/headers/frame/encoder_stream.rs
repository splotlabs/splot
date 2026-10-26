// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-backed sequence-header and key-frame-header builders for any frame size.
//!
//! Each builder writes the canonical AV2 bits for its restricted choice set (§ 5.4,
//! § 5.18.2) and parses them with the decoder parsers, so the returned models are the
//! exact models a decoder derives. [`encode_key_frame_temporal_unit`] wraps one coded
//! tile in an Annex B temporal unit.

use crate::bitio::BitReader;
use crate::headers::frame::restoration::default_restoration_size;
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CcsoPlaneParams, CdefParams, CdefStrengthSets, CoreSeqView, DeblockingFilterParams,
    FrameHeaderCore, FrameHeaderParseStatus, FrameReferenceStateView, FrameRestorationType,
    GdfGeometry, GdfParams, LrGeometry, LrParams, LrPlaneParams, TxMode, WienerNsFrameFilterBank,
    encode_temporal_delimiter_obu, gdf_per_block_is_coded, init_core_from_prefix, parse_core_body,
    parse_frame_header_prefix,
};
use crate::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize, parse_sequence_header,
};
use crate::headers::tile_group::{TileGroupFraming, TileGroupStructure};
use crate::obu::{ObuHeader, ParsedObu};
use crate::span::ByteOffset;
use crate::tile::TileSpacing;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, ObuType, TemporalLayerId};
use crate::write::{
    BitWriter, LrReferenceFilters, WriteError, write_annexb_obu, write_ccso_plane_params,
    write_cdef_params, write_deblocking_filter_params, write_gdf_params, write_lr_params_for_inter,
    write_obu_payload, write_tile_group_obu,
};

/// `seq_level_idx == 31`: the level without tile-size limits (§ 5.18.7.3).
const UNCONSTRAINED_LEVEL_IDX: u32 = 31;
/// `order_hint_bits_minus_1` written by [`build_encoder_sequence_header`].
const ORDER_HINT_BITS_MINUS_1: u32 = 7;
/// `base_*_delta_q` value that gives a zero `Base*DeltaQ` (`-DELTA_DCQUANT_MIN`, § 3).
const ZERO_BASE_DELTA_Q: u32 = 23;
/// `df_par_bits_minus_2`: `DfDeltaQ` spans `-4..=3` (§ 5.18.5.2).
const DF_PAR_BITS_MINUS_2: u32 = 1;
/// The largest `reduced_tx_set` (`f(2)`, § 5.18.2).
pub(super) const MAX_REDUCED_TX_SET: u8 = 3;

/// One bit per optional AV2 coding tool that `sequence_header_obu()` can enable.
///
/// `Default` disables every tool. TX64 and quantizer matrices have no sequence-level
/// enable flag in § 5.4, so they are not listed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
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
    /// `enable_cctx`.
    pub cctx: bool,
    /// `enable_tcq` with `choose_tcq_per_frame = 1`: each frame codes `allow_tcq`.
    pub tcq: bool,
    /// `enable_parity_hiding`.
    pub parity_hiding: bool,
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
    /// `seq_max_drl_bits_minus_1` (§ 5.4.6), `0..=4`: `NEARMV` and `NEWMV`
    /// code `RefMvIdx` below `seq_max_drl_bits_minus_1 + 2`.
    pub max_drl_bits_minus_1: u32,
    /// `enable_adaptive_mvd`, `enable_mvd_sign_derive` and
    /// `enable_flex_mvres` (§ 5.4.6).
    pub mvd_tools: [bool; 3],
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
    /// `ccso_params()` of each plane (§ 5.18.7.12) without reuse; the frame
    /// codes `ccso_frame_flag = 1` when a plane sets `ccso_planes`.
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
}

impl Default for KeyFrameTxParams {
    fn default() -> Self {
        Self {
            tx_mode: TxMode::Largest,
            reduced_tx_set: 2,
            allow_tcq: false,
            delta_q_res: None,
        }
    }
}

/// The uniform tile layout of the encoder frame headers (§ 5.18.7.3).
///
/// `Default` is one tile with `TileSizeBytes = 4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniformTiles {
    /// `TileColsLog2` before the cap at `maxLog2TileCols`.
    pub cols_log2: u8,
    /// `TileRowsLog2` before the cap at `maxLog2TileRows`.
    pub rows_log2: u8,
    /// `TileSizeBytes`, `1..=4`, which a frame of more than one tile codes.
    pub size_bytes: u8,
}

impl Default for UniformTiles {
    fn default() -> Self {
        Self {
            cols_log2: 0,
            rows_log2: 0,
            size_bytes: 4,
        }
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
    /// `base_q_idx` is zero (coded lossless) or does not fit the § 5.18.6.1 field.
    #[error("base_q_idx {base_q_idx} is outside 1..{limit}")]
    InvalidBaseQIdx {
        /// The rejected value.
        base_q_idx: u32,
        /// One past the largest accepted value.
        limit: u32,
    },
    /// The transform choices are not codable: `ONLY_4X4` or `reduced_tx_set > 3`.
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
    w.write_flag(true)?; // monotonic_output_order_flag
    w.write_bits(width_bits - 1, 4)?;
    w.write_bits(height_bits - 1, 4)?;
    w.write_bits(params.width - 1, width_bits)?;
    w.write_bits(params.height - 1, height_bits)?;
    w.write_flag(false)?; // seq_cropping_window_present_flag
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

    w.write_flag(false)?; // disable_loopfilters_across_tiles
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
    w.write_bits(DF_PAR_BITS_MINUS_2, 2)?;

    w.write_flag(false)?; // seq_tile_info_present_flag
    w.write_flag(false)?; // film_grain_params_present
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

/// § 5.4.6 for a non-single-picture header with the motion vector prediction
/// and difference tools of `tools` and every other inter tool off.
fn write_inter_config(w: &mut BitWriter, tools: EncoderToolFlags) -> Result<(), WriteError> {
    w.write_bits(0, 4)?; // seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]
    w.write_flag(false)?; // enable_masked_compound
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
    w.write_flag(false)?; // explicit_ref_frame_map
    w.write_flag(false)?; // explicit_num_ref_frames
    w.write_bits(0, 3)?; // long_term_frame_id_bits
    w.write_ns(tools.max_drl_bits_minus_1, 5)?; // ns(MAX_REF_MV_STACK_SIZE - 1)
    w.write_flag(false)?; // allow_frame_max_drl_bits
    w.write_ns(0, 3)?; // seq_max_bvp_drl_bits_minus_1, ns(MAX_REF_BV_STACK_SIZE - 1)
    w.write_flag(false)?; // allow_frame_max_bvp_drl_bits
    w.write_bits(NUM_SAME_REF_COMPOUND, 2)?;
    w.write_flag(tools.tip)?; // enable_tip
    if tools.tip {
        w.write_flag(true)?; // disable_tip_output
        w.write_flag(false)?; // enable_tip_hole_fill
    }
    for _ in 0..5 {
        w.write_flag(false)?; // enable_mv_traj, bawp, cwp, imp_msk_bld, df_sub_pu
    }
    w.write_bits(0, 2)?; // enable_opfl_refine
    w.write_flag(false)?; // enable_refinemv
    w.write_flag(false)?; // enable_bru
    for flag in tools.mvd_tools {
        w.write_flag(flag)?; // adaptive_mvd, mvd_sign_derive, flex_mvres
    }
    w.write_flag(false)?; // enable_global_motion
    w.write_flag(false) // enable_short_refresh_frame_flags
}

/// § 5.4.8 with zero quantizer offsets.
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
    if !monochrome {
        w.write_flag(false)?; // separate_uv_delta_q
    }
    w.write_flag(true)?; // equal_ac_dc_q
    if !monochrome {
        w.write_bits(ZERO_BASE_DELTA_Q, 5)?; // base_uv_ac_delta_q
        w.write_flag(false)?; // uv_ac_delta_q_enabled
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
    tiles: UniformTiles,
) -> Result<FrameHeaderCore, EncoderHeaderError> {
    let bytes = key_frame_header_bits(seq, (base_q_idx, tiles), order_hint, filters, tx)?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
    parse_core_body(
        &mut reader,
        &mut core,
        seq,
        None,
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
    (base_q_idx, tiles): (u32, UniformTiles),
    order_hint: u32,
    filters: &FrameFilterParams,
    tx: KeyFrameTxParams,
) -> Result<Vec<u8>, EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    if tx.tx_mode == TxMode::Only4x4 || tx.reduced_tx_set > MAX_REDUCED_TX_SET {
        return Err(EncoderHeaderError::InvalidTxParams {
            tx_mode: tx.tx_mode,
            reduced_tx_set: tx.reduced_tx_set,
        });
    }
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_bits(0, seq.long_term_frame_id_bits)?; // long_term_id_plus_1
    w.write_flag(true)?; // immediate_output_frame
    w.write_flag(false)?; // frame_size_override_flag
    w.write_bits(order_hint & order_hint_mask, seq.order_hint_bits)?;
    if seq.seq_force_screen_content_tools == 2 {
        w.write_flag(true)?; // allow_screen_content_tools
        if seq.seq_force_integer_mv == 2 {
            w.write_flag(false)?; // force_integer_mv
        }
    }
    w.write_flag(false)?; // allow_intrabc
    w.write_flag(false)?; // disable_cdf_update
    write_quant_and_filters(
        &mut w,
        seq,
        (base_q_idx, tiles, tx),
        filters,
        (true, LrReferenceFilters::default()),
    )?;
    w.write_flag(tx.tx_mode == TxMode::Select)?; // tx_mode_select
    w.write_bits(u32::from(tx.reduced_tx_set), 2)?;
    w.align_to_byte();
    Ok(w.into_bytes())
}

/// § 5.18.2 from `tile_info()` through `ccso_params()`: the uniform `tiles`, `base_q_idx`, no
/// segmentation or quantizer matrices, the delta-q and `allow_tcq` of `tx`, parity hiding when
/// the sequence enables it and the frame has no TCQ, and the filters of `filters`; an inter
/// frame codes no CCSO reuse.
pub(super) fn write_quant_and_filters(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    (base_q_idx, tiles, tx): (u32, UniformTiles, KeyFrameTxParams),
    filters: &FrameFilterParams,
    (frame_is_intra, lr_refs): (bool, LrReferenceFilters<'_>),
) -> Result<(), EncoderHeaderError> {
    let allow_tcq = tx.allow_tcq;
    if tx.delta_q_res.is_some() && base_q_idx == 0 {
        return Err(EncoderHeaderError::DeltaQWithoutBaseQIdx);
    }
    if allow_tcq && !seq.quant.choose_tcq_per_frame {
        return Err(EncoderHeaderError::TcqNotChosen);
    }
    let q_bits = if seq.quant.bit_depth == 8 { 8 } else { 9 };
    if base_q_idx == 0 || base_q_idx >= 1 << q_bits {
        return Err(EncoderHeaderError::InvalidBaseQIdx {
            base_q_idx,
            limit: 1 << q_bits,
        });
    }
    let spacing = write_uniform_tile_info(w, seq, tiles)?;
    w.write_bits(base_q_idx, q_bits)?;
    w.write_flag(false)?; // segmentation_enabled
    w.write_flag(false)?; // using_qmatrix
    if base_q_idx > 0 {
        w.write_flag(tx.delta_q_res.is_some())?; // delta_q_present
    }
    if let Some(res) = tx.delta_q_res {
        w.write_bits(u32::from(res), 2)?; // delta_q_res
    }
    if seq.quant.choose_tcq_per_frame {
        w.write_flag(allow_tcq)?;
    }
    let allow_tcq = allow_tcq || (seq.quant.enable_tcq && !seq.quant.choose_tcq_per_frame);
    if seq.quant.enable_parity_hiding && !allow_tcq {
        w.write_flag(true)?; // allow_parity_hiding
    }
    let sb_shift = crate::tile::mi_width_log2(seq.tile.seq_sb_size);
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
    write_filter_params(w, seq, filters, (lr_refs, gdf))?;
    if seq.ccso.enable_ccso {
        let planes = &filters.ccso[..usize::from(seq.quant.num_planes).min(3)];
        let on = planes.iter().any(|plane| plane.ccso_planes);
        w.write_flag(on)?; // ccso_frame_flag
        for plane in planes.iter().filter(|_| on) {
            w.write_flag(plane.ccso_planes)?;
            if plane.ccso_planes {
                if !frame_is_intra {
                    w.write_flag(false)?; // reuse_ccso
                    w.write_flag(false)?; // sb_reuse_ccso
                }
                write_ccso_plane_params(w, plane)?;
            }
        }
    }
    Ok(())
}

/// § 5.18.5.2 `deblocking_filter_params()`, § 5.18.7.9 `gdf_params()`,
/// § 5.18.7.10 `cdef_params()`, and § 5.18.7.11 `lr_params()` of `filters`.
fn write_filter_params(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    filters: &FrameFilterParams,
    (lr_refs, gdf_geometry): (LrReferenceFilters<'_>, GdfGeometry<'_>),
) -> Result<(), WriteError> {
    let num_planes = seq.quant.num_planes;
    let delta = filters.deblocking_delta_q;
    let present = core::array::from_fn(|i| {
        filters.deblocking[i] && delta[i] != if i == 1 { delta[0] } else { 0 }
    });
    let deblocking = DeblockingFilterParams::new(filters.deblocking, present, delta);
    let df_par_bits_minus_2 = seq.filter.df_par_bits_minus_2;
    write_deblocking_filter_params(w, &deblocking, false, num_planes, df_par_bits_minus_2, None)?;
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
    write_gdf_params(w, &gdf, false, &seq.filter, gdf_geometry)?;
    let cdef_on = !filters.cdef_strengths.is_empty();
    let cdef = CdefParams {
        cdef_frame_enable: cdef_on,
        cdef_damping: cdef_on.then_some(filters.cdef_damping),
        cdef_strengths: cdef_on.then_some(filters.cdef_strengths.len() as u8),
        cdef_on_skip_txfm_frame_enable: cdef_on.then_some(filters.cdef_on_skip),
        strengths: filters.cdef_strengths,
    };
    write_cdef_params(w, &cdef, false, num_planes, &seq.filter)?;
    let geometry = LrGeometry::new(seq.tile.frame_sb_size(true), seq.chroma_format_idc);
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
    if seq.restoration.enable_restoration {
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
        (false, num_planes),
        &seq.restoration,
        geometry,
        lr_refs,
    )
}

/// § 5.18.7.2 `tile_info()` with § 5.18.7.3 uniform spacing at level 31, where
/// `minLog2TileCols` and `minLog2Tiles` are `0`: `tiles` capped at `maxLog2TileCols`
/// and `maxLog2TileRows`, then `context_update_tile_id = 0` and `tiles.size_bytes`
/// when the frame has more than one tile.
fn write_uniform_tile_info(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    tiles: UniformTiles,
) -> Result<[TileSpacing; 2], WriteError> {
    let sb_log2 = crate::tile::mi_width_log2(seq.tile.frame_sb_size(true));
    let mis = |frame: u32| 2 * frame.div_ceil(8);
    let axis = |w: &mut BitWriter,
                log2: u8,
                mis: u32,
                max_tiles: u32|
     -> Result<TileSpacing, WriteError> {
        let max = crate::tile::tile_log2(1, mis.div_ceil(1 << sb_log2).min(max_tiles));
        let log2 = log2.min(max);
        for _ in 0..log2 {
            w.write_flag(true)?; // increment_tile_*_log2
        }
        if log2 < max {
            w.write_flag(false)?;
        }
        Ok(crate::tile::uniform_spacing(
            log2,
            mis,
            seq.tile.seq_sb_size,
        ))
    };
    w.write_flag(true)?; // uniform_tile_spacing_flag
    let cols = axis(
        w,
        tiles.cols_log2,
        mis(seq.max_frame_width),
        crate::tile::MAX_TILE_COLS,
    )?;
    let rows = axis(
        w,
        tiles.rows_log2,
        mis(seq.max_frame_height),
        crate::tile::MAX_TILE_ROWS,
    )?;
    if cols.count > 1 || rows.count > 1 {
        if !seq.tile.enable_avg_cdf || seq.tile.avg_cdf_type == 0 {
            let bits =
                crate::tile::tile_log2(1, cols.count) + crate::tile::tile_log2(1, rows.count);
            w.write_bits(0, u32::from(bits))?; // context_update_tile_id
        }
        if !(1..=4).contains(&tiles.size_bytes) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            });
        }
        w.write_bits(u32::from(tiles.size_bytes) - 1, 2)?;
    }
    Ok([cols, rows])
}

/// Serializes one Annex B temporal unit: a temporal delimiter, then `seq` when
/// `with_sequence_header`, then the `OBU_CLOSED_LOOP_KEY` frame OBU whose one tile
/// group carries the `tiles` of the frame in tile order.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if `seq` has no complete view, a tile is empty or
/// too long for `TileSizeBytes`, the tile count differs from the header, or an OBU
/// does not serialize.
pub fn encode_key_frame_temporal_unit(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    with_sequence_header: bool,
    tiles: &[&[u8]],
) -> Result<Vec<u8>, EncoderHeaderError> {
    let view = CoreSeqView::from_sequence(seq).ok_or(EncoderHeaderError::Incomplete)?;
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
    let mut payload = BitWriter::new();
    write_tile_group_obu(
        &mut payload,
        core,
        &view,
        None,
        true,
        &TileGroupStructure::whole_frame(tiles.len() as u32),
        &tile_framing(core, tiles),
        tiles,
        true,
    )?;
    write_annexb_obu(
        &mut w,
        &obu_header(ObuType::ClosedLoopKey),
        &payload.into_bytes(),
    )?;
    unit.extend_from_slice(&w.into_bytes());
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
