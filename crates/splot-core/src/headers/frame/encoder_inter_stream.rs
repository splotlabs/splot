// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-backed inter-frame and show-existing-frame builders (§ 5.18.2).
//!
//! [`build_inter_frame_header`] writes the canonical bits of the smallest
//! `OBU_REGULAR_TILE_GROUP` inter frame and parses them with the caller's § 7.23
//! reference state, so the parser derives the reference map (§ 7.7). The core
//! writer does not write inter headers, so the returned header keeps the exact
//! bits, and [`encode_inter_frame_obu`] copies them into the tile group.

use crate::bitio::BitReader;
use crate::headers::film_grain::{
    FilmGrainModel, FilmGrainObu, FilmGrainScalingPoint, FilmGrainSlotUpdate,
};
use crate::headers::frame::encoder_stream::{
    MAX_REDUCED_TX_SET, UniformTiles, obu_header, tile_framing, write_film_grain_config,
    write_quant_and_filters,
};
use crate::headers::frame::global_motion::{REFS_PER_FRAME, default_base_prediction};
use crate::headers::frame::info::build_inter_seq_view;
use crate::headers::frame::inter::tip_weight_coded;
use crate::headers::frame::inter_shared_tail::{
    lr_reference_filter_counts, lr_reference_filter_entries,
};
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CoreSeqView, EncoderHeaderError, FrameFilterParams, FrameHeaderCore, FrameHeaderParseStatus,
    FrameReferenceStateView, KeyFrameTxParams, TipFrameMode, TxMode, init_core_from_prefix,
    parse_core_body, parse_frame_header_prefix,
};
use crate::headers::frame::{DELTAWARP, INTERINTRA};
use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};
use crate::headers::tile_group::{RecordedFrameHeaderBits, TileGroupLayout, TileGroupStructure};
use crate::span::ByteOffset;
use crate::types::ObuType;
use crate::write::{
    BitWriter, LrReferenceFilters, WriteError, global_motion::write_global_param, write_annexb_obu,
    write_tile_group_payload, write_tile_group_structure,
};

/// `PRIMARY_REF_NONE` (§ 3).
const PRIMARY_REF_NONE: u32 = 7;
/// `interpolation_filter` value `EIGHTTAP` (§ 6.17.5.1).
const EIGHTTAP: u32 = 0;
/// `REFINE_AUTO` (§ 3): each frame codes `opfl_refine_type`.
const REFINE_AUTO: u8 = 3;

/// Frame-level choices of [`build_inter_frame_header`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterFrameParams {
    /// `base_q_idx`.
    pub base_q_idx: u32,
    /// The frame's `OrderHint`; the header codes its low `OrderHintBits` bits.
    pub order_hint: u32,
    /// `refresh_frame_flags`, one bit per reference slot.
    pub refresh_frame_flags: u32,
    /// `immediate_output_frame`; a hidden frame is shown later by a
    /// show-existing frame.
    pub show_frame: bool,
    /// Loop-filter choices, as for key frames.
    pub filters: FrameFilterParams,
    /// Transform mode and `reduced_tx_set`, as for key frames.
    pub tx: KeyFrameTxParams,
    /// The uniform tile layout, as for key frames.
    pub tiles: UniformTiles,
    /// `reference_select` (§ 5.18.8.3): blocks may code compound prediction.
    pub reference_select: bool,
    /// `skip_mode_present` (§ 5.18.8.2): blocks may code skip mode; the
    /// parser derives `SkipModeFrame`.
    pub skip_mode_present: bool,
    /// The reference slot whose saved CDFs the frame starts from (§ 5.18.2
    /// `set_primary_ref_frame_and_ctx`); `None` codes `PRIMARY_REF_NONE`.
    pub primary_slot: Option<u32>,
    /// `use_ref_frame_mvs` (§ 5.18.2): blocks take temporal candidates from
    /// the projected motion field; needs `enable_ref_frame_mvs`. A read
    /// `tmvp_sample_step_minus_1` is `0`.
    pub use_ref_frame_mvs: bool,
    /// `allow_high_precision_mv` (§ 5.18.2): eighth-pel motion vectors;
    /// off, `use_qtr_precision_mv` selects quarter-pel.
    pub high_precision_mv: bool,
    /// `is_filter_switchable` (§ 5.18.5.1): blocks code `interp_filter`;
    /// off, the frame codes `EIGHTTAP`.
    pub switchable_interp_filter: bool,
    /// `tip_global_wtd_index` of a frame coding `tip_frame_mode =
    /// TIP_FRAME_AS_REF` (§ 5.18.2); `None` codes `TIP_FRAME_DISABLED`. A
    /// frame without `enable_tip`, `use_ref_frame_mvs` or two references
    /// reads neither and is `TIP_FRAME_DISABLED`.
    pub tip_weight: Option<u8>,
    /// `opfl_refine_type` (§ 5.18.3.2) of a sequence with `enable_opfl_refine
    /// = REFINE_AUTO`: `0` `REFINE_NONE`, `1` `REFINE_SWITCHABLE`, `2`
    /// `REFINE_ALL`. Other sequences code none and ignore it.
    pub opfl_refine_type: u8,
    /// `force_integer_mv` (§ 5.18.3.3) of a sequence that chooses screen
    /// content tools: whole-sample motion vectors; `high_precision_mv` must
    /// then be off.
    pub force_integer_mv: bool,
    /// The `LOCALWARP`, `DELTAWARP` and `EXTENDWARP` motion modes with
    /// `allow_warpmv_mode` (§ 5.18.2) in a sequence that lets each frame
    /// choose them; off, the frame codes them disabled.
    pub warp: bool,
    /// The `gm_params` of each list reference with a `ROTZOOM` or `AFFINE`
    /// global motion model (§ 5.18.9.1), coded with `our_ref =
    /// NumTotalRefs`; `None` codes `IDENTITY`, and all `None` codes
    /// `use_global_motion = 0`. Needs `enable_global_motion`.
    pub global_motion: [Option<[i32; 6]>; REFS_PER_FRAME],
}

/// A parsed frame header and the exact bits it was parsed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedFrameHeader {
    /// The header the decoder derives.
    pub core: FrameHeaderCore,
    /// `NumFrameHeaderBits` bits of `frame_header_info()`.
    pub bits: RecordedFrameHeaderBits,
}

/// Builds an `OBU_REGULAR_TILE_GROUP` inter frame header for `seq` and the § 7.23
/// reference state `refs`.
///
/// The frame has the maximum frame size, the § 7.7 implicit reference map,
/// `primary_ref_frame` at the first position of `params.primary_slot` in that
/// map with `disable_cross_frame_cdf_init = 0` (else `PRIMARY_REF_NONE` with
/// `disable_cross_frame_cdf_init = 1`, also when the map does not hold the
/// slot or holds it only as a restricted reference),
/// `use_ref_frame_mvs` of `params`,
/// quarter-pel or, with `high_precision_mv`, eighth-pel motion vectors, the
/// frame-level `EIGHTTAP` filter or, with `switchable_interp_filter`,
/// `SWITCHABLE`,
/// `reference_select` and `skip_mode_present` of `params`, and the key-frame tail
/// (`TX_MODE_LARGEST`, `reduced_tx_set = 2`) with the filters of `params`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a zero or too-wide `base_q_idx`, a
/// single-picture view, `filters` the sequence cannot carry, bits that do not
/// parse to a complete inter header with `refs`, or `refs` without a valid slot.
pub fn build_inter_frame_header(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
) -> Result<EncodedFrameHeader, EncoderHeaderError> {
    let plain = InterFrameParams {
        use_ref_frame_mvs: false,
        global_motion: [None; REFS_PER_FRAME],
        filters: FrameFilterParams {
            lr_luma_wiener_ns: None,
            lr_chroma_wiener_ns: [None; 2],
            ..params.filters
        },
        ..*params
    };
    let header = parse_inter_frame_header(seq, refs, &plain, (None, 0), &[])?;
    let inter = header
        .core
        .inter
        .as_ref()
        .ok_or(EncoderHeaderError::Incomplete)?;
    let position = params.primary_slot.and_then(|slot| {
        let position = inter
            .ref_frame_idx
            .iter()
            .zip(inter.order_hints.iter())
            .position(|(&index, &hint)| index == slot && hint != u32::MAX)?;
        u32::try_from(position).ok()
    });
    if position.is_none()
        && !params.use_ref_frame_mvs
        && params.filters == plain.filters
        && params.global_motion == plain.global_motion
    {
        return Ok(header);
    }
    let num_total_refs = inter.num_total_refs.unwrap_or(0);
    let ref_frame_idx = inter.ref_frame_idx.clone();
    parse_inter_frame_header(
        seq,
        refs,
        params,
        (position, num_total_refs),
        &ref_frame_idx,
    )
}

/// Parses the bits of `params` with `primary_ref_frame` over a map of
/// `num_total_refs` references.
fn parse_inter_frame_header(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
    map: (Option<u32>, u32),
    ref_frame_idx: &[u32],
) -> Result<EncodedFrameHeader, EncoderHeaderError> {
    let counts = lr_reference_filter_counts(refs, ref_frame_idx, map.1);
    let entries = lr_reference_filter_entries(refs, ref_frame_idx, map.1);
    let lr_refs = LrReferenceFilters {
        num_ref_frames: map.1,
        counts,
        taps: [&entries[0], &entries[1], &entries[2]],
    };
    let weight_coded = tip_weight_coded(
        &build_inter_seq_view(seq),
        refs,
        (ref_frame_idx, params.order_hint),
        false,
        u32::from(params.opfl_refine_type),
    )
    .unwrap_or(true);
    let gm_predictions: Vec<Option<[i32; 6]>> = ref_frame_idx
        .iter()
        .take(map.1 as usize)
        .map(|&slot| {
            let hint = refs.ref_order_hint?.get(slot as usize).copied()?;
            default_base_prediction(params.order_hint, hint)
        })
        .collect();
    let (bytes, bit_len) =
        inter_frame_header_bits(seq, params, map, (lr_refs, &gm_predictions), weight_coded)?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTileGroup, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTileGroup, true);
    parse_core_body(&mut reader, &mut core, seq, None, refs)?;
    core.consumed_bits = reader.consumed_bits();
    let has_reference = core
        .inter
        .as_ref()
        .is_some_and(|inter| !inter.ref_frame_idx.is_empty());
    if core.status != FrameHeaderParseStatus::InterHeaderComplete
        || core.consumed_bits != bit_len
        || !has_reference
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let bits =
        RecordedFrameHeaderBits::record(&mut BitReader::new(&bytes, ByteOffset::new(0)), bit_len)?;
    Ok(EncodedFrameHeader { core, bits })
}

fn inter_frame_header_bits(
    seq: &CoreSeqView,
    params: &InterFrameParams,
    (primary_ref_frame, num_total_refs): (Option<u32>, u32),
    (lr_refs, gm_predictions): (LrReferenceFilters<'_>, &[Option<[i32; 6]>]),
    weight_coded: bool,
) -> Result<(Vec<u8>, u64), EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_flag(true)?; // frame_is_inter
    w.write_flag(params.show_frame)?; // immediate_output_frame
    if !params.show_frame && !seq.monotonic_output_order_flag {
        w.write_flag(false)?; // implicit_output_frame
    }
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    w.write_flag(true)?; // signal_primary_ref_frame
    w.write_flag(primary_ref_frame.is_none())?; // disable_cross_frame_cdf_init
    w.write_bits(primary_ref_frame.unwrap_or(PRIMARY_REF_NONE), 3)?; // primary_ref_frame
    if seq.enable_short_refresh_frame_flags || seq.seq_force_screen_content_tools == 1 {
        return Err(unsupported(
            "short refresh flags or forced screen content tools",
        ));
    }
    w.write_bits(params.refresh_frame_flags, seq.num_ref_frames)?;
    if params.use_ref_frame_mvs && !seq.inter.enable_ref_frame_mvs {
        return Err(unsupported(
            "use_ref_frame_mvs without enable_ref_frame_mvs",
        ));
    }
    if seq.inter.enable_ref_frame_mvs {
        w.write_flag(params.use_ref_frame_mvs)?;
    }
    if params.use_ref_frame_mvs
        && num_total_refs > 1
        && seq.tile.frame_sb_size(false) != SuperblockSize::Block64x64
    {
        w.write_flag(false)?; // tmvp_sample_step_minus_1
    }
    let tip_gate = seq.inter.enable_tip && params.use_ref_frame_mvs && num_total_refs >= 2;
    if tip_gate {
        w.write_flag(params.tip_weight.is_some())?; // tip_frame_mode
    }
    if seq.inter.enable_opfl_refine == REFINE_AUTO {
        match params.opfl_refine_type {
            1 => w.write_bits(1, 1)?, // opfl_refine_type = REFINE_SWITCHABLE
            refine @ (0 | 2) => {
                w.write_bits(0, 1)?; // opfl_refine_type
                w.write_flag(refine == 2)?; // REFINE_ALL or REFINE_NONE
            }
            _ => return Err(unsupported("opfl_refine_type")),
        }
    }
    if let (true, Some(weight)) = (tip_gate, params.tip_weight) {
        if seq.inter.enable_tip_hole_fill {
            w.write_flag(false)?; // allow_tip_hole_fill
        }
        write_tip_weight(&mut w, weight, weight_coded)?;
    }
    let coded_integer_mv = seq.seq_force_screen_content_tools == 2 && seq.seq_force_integer_mv == 2;
    if params.force_integer_mv && (!coded_integer_mv || params.high_precision_mv) {
        return Err(unsupported("force_integer_mv the sequence does not select"));
    }
    if seq.seq_force_screen_content_tools == 2 {
        w.write_flag(true)?; // allow_screen_content_tools
        if seq.seq_force_integer_mv == 2 {
            w.write_flag(params.force_integer_mv)?;
        }
    }
    w.write_flag(false)?; // allow_intrabc
    if !params.force_integer_mv {
        w.write_flag(!params.high_precision_mv)?; // use_qtr_precision_mv
    }
    if params.high_precision_mv {
        w.write_flag(true)?; // allow_high_precision_mv
    }
    w.write_flag(params.switchable_interp_filter)?; // is_filter_switchable
    if !params.switchable_interp_filter {
        w.write_bits(EIGHTTAP, 2)?; // interpolation_filter
    }
    let modes = seq.inter.seq_enabled_motion_modes;
    if !seq.inter.seq_frame_motion_modes_present_flag && modes.contains(&true) {
        return Err(unsupported("motion modes every frame enables"));
    }
    let warp = params.warp && modes[DELTAWARP];
    if params.warp && !warp {
        return Err(unsupported("warped motion the sequence does not enable"));
    }
    for (mode, _) in modes
        .iter()
        .enumerate()
        .skip(INTERINTRA)
        .filter(|&(_, &on)| on)
    {
        w.write_flag(mode == INTERINTRA || warp)?; // frame_enabled_motion_modes[mode]
    }
    w.write_flag(false)?; // disable_cdf_update
    let tx = params.tx;
    if tx.allow_intrabc {
        return Err(unsupported("intra block copy in an inter frame"));
    }
    if tx.tx_mode == TxMode::Only4x4 || tx.reduced_tx_set > MAX_REDUCED_TX_SET {
        return Err(EncoderHeaderError::InvalidTxParams {
            tx_mode: tx.tx_mode,
            reduced_tx_set: tx.reduced_tx_set,
        });
    }
    write_quant_and_filters(
        &mut w,
        seq,
        (params.base_q_idx, params.tiles, tx),
        &params.filters,
        (false, lr_refs),
    )?;
    w.write_flag(tx.tx_mode == TxMode::Select)?; // tx_mode_select
    w.write_flag(params.reference_select)?;
    w.write_flag(params.skip_mode_present)?;
    if seq.inter.enable_bawp {
        return Err(unsupported("block adaptive weighted prediction"));
    }
    if warp {
        w.write_flag(true)?; // allow_warpmv_mode
    }
    w.write_bits(u32::from(tx.reduced_tx_set), 2)?;
    write_global_motion(&mut w, seq, params, (num_total_refs, gm_predictions))?;
    write_film_grain_config(&mut w, seq, params.show_frame, tx.grain_seed)?;
    let bit_len = w.bit_len();
    w.align_to_byte();
    Ok((w.into_bytes(), bit_len))
}

/// Writes `global_motion_params()` (§ 5.18.9.1) of `params` over a map of
/// `num_total_refs` references whose `PrevGmParams` are `predictions`, `None`
/// for a reference that reads no model.
fn write_global_motion(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    params: &InterFrameParams,
    (num_total_refs, predictions): (u32, &[Option<[i32; 6]>]),
) -> Result<(), EncoderHeaderError> {
    let used = params.global_motion.iter().any(Option::is_some);
    if !seq.inter.enable_global_motion {
        return if used {
            Err(unsupported("global motion without enable_global_motion"))
        } else {
            Ok(())
        };
    }
    w.write_flag(used)?; // use_global_motion
    if !used {
        return Ok(());
    }
    w.write_ns(num_total_refs, num_total_refs + 1)?; // our_ref
    for (index, &model) in params.global_motion.iter().enumerate() {
        let Some(&Some(prev)) = predictions.get(index) else {
            if model.is_some() {
                return Err(unsupported("global motion of a reference that reads none"));
            }
            continue;
        };
        w.write_flag(model.is_some())?; // is_global
        let Some(model) = model else {
            continue;
        };
        let rotzoom = model[3].checked_neg() == Some(model[4]) && model[5] == model[2];
        w.write_flag(rotzoom)?; // is_rot_zoom
        let coded: &[usize] = if rotzoom {
            &[2, 3, 0, 1]
        } else {
            &[2, 3, 4, 5, 0, 1]
        };
        for &idx in coded {
            write_global_param(w, idx, prev[idx], model[idx])?;
        }
    }
    Ok(())
}

/// Serializes the `OBU_REGULAR_TILE_GROUP` OBU of `header` whose one tile group
/// carries the `tiles` of the frame: `is_first_tile_group`, the recorded header
/// bits, the § 5.19 structure, and the tiles.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the header has no tile info, a tile is empty
/// or too long for `TileSizeBytes`, the tile count differs from the header, or
/// the OBU does not serialize.
pub fn encode_inter_frame_obu(
    header: &EncodedFrameHeader,
    tiles: &[&[u8]],
) -> Result<Vec<u8>, EncoderHeaderError> {
    let tile_info = header
        .core
        .tile_info
        .as_ref()
        .ok_or(EncoderHeaderError::Incomplete)?;
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    let mut payload = BitWriter::new();
    payload.write_flag(true)?; // is_first_tile_group
    for index in 0..header.bits.num_frame_header_bits() {
        let bit = header
            .bits
            .bit(index)
            .ok_or(EncoderHeaderError::Incomplete)?;
        payload.write_flag(bit)?;
    }
    write_tile_group_structure(
        &mut payload,
        &TileGroupStructure::whole_frame(tiles.len() as u32),
        layout,
    )?;
    write_tile_group_payload(
        &mut payload,
        &tile_framing(&header.core, tiles),
        tiles,
        tile_info.tile_size_bytes.unwrap_or(1),
        false,
    )?;
    let mut w = BitWriter::new();
    write_annexb_obu(
        &mut w,
        &obu_header(ObuType::RegularTileGroup),
        &payload.into_bytes(),
    )?;
    Ok(w.into_bytes())
}

/// Builds the `OBU_REGULAR_TIP` header of a `TIP_FRAME_AS_OUTPUT` frame
/// (§ 5.18.2) over the reference map `ref_frame_idx` for `seq` and the
/// § 7.23 reference state `refs`: the `order_hint`, `refresh_frame_flags`,
/// `show_frame` and `tip_weight` of `params`, `PRIMARY_REF_NONE`,
/// `use_ref_frame_mvs = 1`, a zero `tip_global_mv`, the `EIGHTTAP_SHARP`
/// filter and the `film_grain_config()` of `params.tx.grain_seed`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a sequence without TIP output, the
/// tools the header does not write, a missing `tip_weight`, a `grain_seed`
/// in a sequence without film grain or in a frame that is not shown
/// ([`WriteError::NonCanonicalFrameHeader`]), or bits that do not parse to a
/// complete TIP output header with `refs`.
pub fn build_tip_output_frame_header(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
    ref_frame_idx: &[u32],
) -> Result<EncodedFrameHeader, EncoderHeaderError> {
    let inter = &seq.inter;
    let num_total_refs = ref_frame_idx.len();
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    if !inter.enable_tip_output || !inter.enable_ref_frame_mvs || num_total_refs < 2 {
        return Err(unsupported(
            "TIP output without TIP, reference MVs or two references",
        ));
    }
    if seq.enable_short_refresh_frame_flags
        || inter.enable_tip_hole_fill
        || inter.enable_tip_explicit_qp
        || seq.filter.enable_df_sub_pu
    {
        return Err(unsupported("TIP output header tools"));
    }
    let weight = params
        .tip_weight
        .ok_or(unsupported("tip_global_wtd_index"))?;
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_flag(params.show_frame)?; // immediate_output_frame
    if !params.show_frame && !seq.monotonic_output_order_flag {
        w.write_flag(false)?; // implicit_output_frame
    }
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    w.write_flag(true)?; // signal_primary_ref_frame
    w.write_bits(PRIMARY_REF_NONE, 3)?; // primary_ref_frame
    w.write_bits(params.refresh_frame_flags, seq.num_ref_frames)?;
    w.write_flag(true)?; // use_ref_frame_mvs
    if seq.tile.frame_sb_size(false) != SuperblockSize::Block64x64 {
        w.write_flag(false)?; // tmvp_sample_step_minus_1
    }
    let weight_coded = tip_weight_coded(
        &build_inter_seq_view(seq),
        refs,
        (ref_frame_idx, params.order_hint),
        true,
        0,
    );
    write_tip_weight(&mut w, weight, weight_coded.unwrap_or(true))?;
    w.write_flag(true)?; // tip_global_mv_zero
    w.write_flag(true)?; // tip_sharp
    write_film_grain_config(&mut w, seq, params.show_frame, params.tx.grain_seed)?;
    let bit_len = w.bit_len();
    w.write_trailing_bits(8 - bit_len % 8)?;
    let bytes = w.into_bytes();
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTip, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTip, true);
    parse_core_body(&mut reader, &mut core, seq, None, refs)?;
    core.consumed_bits = reader.consumed_bits();
    let as_output = core
        .inter
        .as_ref()
        .is_some_and(|inter| inter.tip_frame_mode == Some(TipFrameMode::AsOutput));
    if core.status != FrameHeaderParseStatus::InterHeaderComplete
        || core.consumed_bits != bit_len
        || !as_output
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let bits =
        RecordedFrameHeaderBits::record(&mut BitReader::new(&bytes, ByteOffset::new(0)), bit_len)?;
    Ok(EncodedFrameHeader { core, bits })
}

/// Serializes the `OBU_REGULAR_TIP` OBU of `header`: the recorded header
/// bits and `trailing_bits()`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the OBU does not serialize.
pub fn encode_tip_output_frame_obu(
    header: &EncodedFrameHeader,
) -> Result<Vec<u8>, EncoderHeaderError> {
    let mut payload = BitWriter::new();
    for index in 0..header.bits.num_frame_header_bits() {
        let bit = header
            .bits
            .bit(index)
            .ok_or(EncoderHeaderError::Incomplete)?;
        payload.write_flag(bit)?;
    }
    payload.write_trailing_bits(8 - payload.bit_len() % 8)?;
    let mut w = BitWriter::new();
    write_annexb_obu(
        &mut w,
        &obu_header(ObuType::RegularTip),
        &payload.into_bytes(),
    )?;
    Ok(w.into_bytes())
}

/// Serializes an `OBU_REGULAR_SEF` OBU that shows reference slot
/// `frame_to_show_map_idx` with `derive_sef_order_hint = 1` (§ 5.18.2) and
/// the `film_grain_config()` of `grain_seed` (see [`KeyFrameTxParams::grain_seed`]).
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a slot outside the sequence's reference
/// slots, a sequence without a known `film_grain_params_present`, a seed
/// without film grain, or bits that do not parse to a complete show-existing
/// frame.
pub fn encode_show_existing_frame_obu(
    seq: &CoreSeqView,
    frame_to_show_map_idx: u32,
    grain_seed: Option<u16>,
) -> Result<Vec<u8>, EncoderHeaderError> {
    if frame_to_show_map_idx >= seq.num_ref_frames {
        return Err(unsupported("frame_to_show_map_idx"));
    }
    seq.film_grain_params_present
        .ok_or(EncoderHeaderError::Incomplete)?;
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_bits(frame_to_show_map_idx, ceil_log2(seq.num_ref_frames))?;
    w.write_flag(true)?; // derive_sef_order_hint
    write_film_grain_config(&mut w, seq, true, grain_seed)?;
    w.write_trailing_bits(8 - w.bit_len() % 8)?;
    let payload = w.into_bytes();
    let mut reader = BitReader::new(&payload, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularSef, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularSef, true);
    parse_core_body(
        &mut reader,
        &mut core,
        seq,
        None,
        &FrameReferenceStateView::unknown(),
    )?;
    if core.status != FrameHeaderParseStatus::ShowExistingFrameComplete
        || core.sef_trailing_bits != Some(crate::headers::frame::SefTrailingBits::Valid)
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let mut obu = BitWriter::new();
    write_annexb_obu(&mut obu, &obu_header(ObuType::RegularSef), &payload)?;
    Ok(obu.into_bytes())
}

/// Writes `tip_global_wtd_index` = `weight` where the frame codes it; a
/// frame that does not code it infers `0`.
fn write_tip_weight(w: &mut BitWriter, weight: u8, coded: bool) -> Result<(), EncoderHeaderError> {
    match (coded, weight) {
        (true, _) => Ok(w.write_bits(u32::from(weight), 3)?), // tip_global_wtd_index
        (false, 0) => Ok(()),
        (false, _) => Err(unsupported("a TIP weight the frame infers as 0")),
    }
}

/// One piecewise-linear `film_grain_model()` scaling function (§ 5.18.10.2):
/// at most [`Self::MAX_POINTS`] `(point value, scaling)` pairs with strictly
/// increasing values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GrainScaling(Vec<(u8, u8)>);

impl GrainScaling {
    /// The largest `num_*_points` (§ 6.17.10.2).
    pub const MAX_POINTS: usize = 14;

    /// Checks `points` against the § 6.17.10.2 point rules.
    ///
    /// # Errors
    /// Returns [`EncoderHeaderError`] for more than [`Self::MAX_POINTS`]
    /// points or values that do not increase.
    pub fn new(points: Vec<(u8, u8)>) -> Result<Self, EncoderHeaderError> {
        if points.len() > Self::MAX_POINTS || points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(unsupported("film grain scaling points"));
        }
        Ok(Self(points))
    }

    /// The `(point value, scaling)` pairs.
    #[must_use]
    pub fn points(&self) -> &[(u8, u8)] {
        &self.0
    }

    /// Whether the function has no points (the plane gets no grain).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The `film_grain_model()` (§ 5.18.10.2) of the encoder for `chroma`: the
/// `[Y, Cb, Cr]` scaling functions, `grain_scaling_minus_8 = 0`,
/// `ar_coeff_lag = 0` with zero coefficients, and each chroma scaling indexed
/// by its own sample (`*_mult = 192`, `*_luma_mult = 128`, `*_offset = 256`).
///
/// # Errors
/// Returns [`EncoderHeaderError`] for chroma points in a monochrome model, or
/// 4:2:0 chroma with points in one of Cb and Cr only (§ 6.17.10.2).
pub fn encoder_film_grain_model(
    chroma: ChromaFormatIdc,
    [y, cb, cr]: [&GrainScaling; 3],
) -> Result<FilmGrainModel, EncoderHeaderError> {
    let monochrome = chroma.is_monochrome();
    if (monochrome && !(cb.is_empty() && cr.is_empty()))
        || (chroma == ChromaFormatIdc::Yuv420 && cb.is_empty() != cr.is_empty())
    {
        return Err(unsupported("film grain chroma points"));
    }
    let points = |points: &GrainScaling| {
        let points: Vec<FilmGrainScalingPoint> = points
            .points()
            .iter()
            .map(|&(value, scaling)| FilmGrainScalingPoint {
                value: value.into(),
                scaling: scaling.into(),
            })
            .collect();
        u8::try_from(points.len())
            .map(|count| (count, points))
            .map_err(|_| unsupported("film grain scaling points"))
    };
    let (num_y_points, point_y) = points(y)?;
    let (num_cb_points, point_cb) = points(cb)?;
    let (num_cr_points, point_cr) = points(cr)?;
    let chroma_coeffs = |num: u8| vec![0; usize::from(num > 0 && num_y_points > 0)];
    Ok(FilmGrainModel {
        chroma_scaling_from_luma: false,
        num_y_points,
        point_y,
        num_cb_points,
        point_cb,
        num_cr_points,
        point_cr,
        grain_scaling_minus_8: 0,
        ar_coeff_lag: 0,
        ar_coeffs_y: Vec::new(),
        ar_coeffs_cb: chroma_coeffs(num_cb_points),
        ar_coeffs_cr: chroma_coeffs(num_cr_points),
        ar_coeff_shift_minus_6: 0,
        grain_scale_shift: 0,
        cb_mult: (num_cb_points > 0).then_some(192),
        cb_luma_mult: (num_cb_points > 0).then_some(128),
        cb_offset: (num_cb_points > 0).then_some(256),
        cr_mult: (num_cr_points > 0).then_some(192),
        cr_luma_mult: (num_cr_points > 0).then_some(128),
        cr_offset: (num_cr_points > 0).then_some(256),
        overlap_flag: false,
        clip_to_restricted_range: false,
        mc_identity: false,
        film_grain_block_size: false,
    })
}

/// Serializes an `OBU_FILM_GRAIN` (§ 5.14) that saves `model` in slot 0 for
/// `chroma`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a model the OBU cannot code.
pub fn encode_film_grain_obu(
    chroma: ChromaFormatIdc,
    model: &FilmGrainModel,
) -> Result<Vec<u8>, EncoderHeaderError> {
    let (sub_x, sub_y) = match chroma {
        ChromaFormatIdc::Yuv444 => (false, false),
        ChromaFormatIdc::Yuv422 => (true, false),
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (true, true),
    };
    let obu = FilmGrainObu {
        update_flags: 1,
        chroma_idc: chroma.get().into(),
        sub_x,
        sub_y,
        monochrome: chroma.is_monochrome(),
        models: vec![FilmGrainSlotUpdate {
            slot: 0,
            model: model.clone(),
        }],
    };
    let mut payload = BitWriter::new();
    crate::write::write_obu_payload(
        &mut payload,
        &crate::obu::ParsedObu::FilmGrain(Box::new(obu)),
        ObuType::FilmGrain.is_extensible_obu(),
        &[],
    )?;
    let mut w = BitWriter::new();
    write_annexb_obu(
        &mut w,
        &obu_header(ObuType::FilmGrain),
        &payload.into_bytes(),
    )?;
    Ok(w.into_bytes())
}

fn unsupported(what: &'static str) -> EncoderHeaderError {
    WriteError::NonCanonicalFrameHeader { what }.into()
}

#[cfg(test)]
#[path = "encoder_inter_stream_tests.rs"]
mod tests;
