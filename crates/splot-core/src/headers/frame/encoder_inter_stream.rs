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
use crate::headers::content_interpretation::ContentInterpretation;
use crate::headers::film_grain::{
    FilmGrainModel, FilmGrainObu, FilmGrainScalingPoint, FilmGrainSlotUpdate,
};
use crate::headers::frame::encoder_stream::{
    FrameTiles, check_tx_params, encoder_mfh_view, obu_header, plane_quantization, tile_framing,
    write_encoder_tile_info, write_film_grain_config, write_intra_frame_tail, write_mfh_reference,
    write_quant_and_filters,
};
use crate::headers::frame::get_ref_frames::get_relative_dist;
use crate::headers::frame::global_motion::{DEFAULT_WARP_PARAMS, REFS_PER_FRAME, base_prediction};
use crate::headers::frame::info::build_inter_seq_view;
use crate::headers::frame::inter::tip_weight_coded;
use crate::headers::frame::inter_shared_tail::lr_reference_filters;
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CcsoPlaneParams, CoreSeqView, EncoderHeaderError, FrameFilterParams, FrameHeaderCore,
    FrameHeaderParseStatus, FrameReferenceStateView, KeyFrameTxParams, TipFrameMode, TxMode,
    init_core_from_prefix, parse_core_body, parse_frame_header_prefix,
};
use crate::headers::frame::{DELTAWARP, INTERINTRA};
use crate::headers::frame::{InterpolationFilter, QuantizationParams, TipGlobalMv};
use crate::headers::metadata::{
    MetadataHdrCll, MetadataHdrMdcv, MetadataPayload, MetadataShortObu, MetadataType, MetadataUnit,
};
use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};
use crate::headers::tile_group::{RecordedFrameHeaderBits, TileGroupLayout, TileGroupStructure};
use crate::hls::MultiFrameHeader;
use crate::span::ByteOffset;
use crate::types::ObuType;
use crate::write::write_quantization_params;
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
    pub tiles: FrameTiles,
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
    /// `TipGlobalMv` of a `TIP_FRAME_AS_OUTPUT` frame (§ 5.18.2), each
    /// component in `-15..=15`; zero codes `tip_mv_zero = 1`.
    pub tip_global_mv: TipGlobalMv,
    /// `TipInterpFilter` of a `TIP_FRAME_AS_OUTPUT` frame: `None` and
    /// `EIGHTTAP_SHARP` code `tip_sharp = 1`; `EIGHTTAP` and `EIGHTTAP_SMOOTH`
    /// code `tip_regular`.
    pub tip_filter: Option<InterpolationFilter>,
    /// `allow_df_sub_pu = apply_deblocking_filter_tip = 1` of a
    /// `TIP_FRAME_AS_OUTPUT` frame of a sequence with `enable_df_sub_pu`: the
    /// TIP frame is deblocked (§ 7.16) and codes a one-tile `tile_info()`.
    pub tip_deblocking: bool,
    /// `allow_tip_hole_fill` (§ 5.18.2) of a TIP frame of a sequence with
    /// `enable_tip_hole_fill`: the TIP motion field fills its holes (§ 7.10.3).
    pub tip_hole_fill: bool,
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
    /// Codes the `our_ref` and `their_ref` (§ 5.18.9.1) of the fewest
    /// `global_motion_params()` bits instead of `our_ref = NumTotalRefs`.
    pub global_motion_predict: bool,
    /// `frame_explicit_ref_frame_map = 1` (§ 5.18.2) with `num_total_refs`
    /// the smaller of this and the default count, over the first references
    /// of the default map. Needs `explicit_ref_frame_map`.
    pub explicit_refs: Option<u8>,
    /// `implicit_output_frame` of a hidden frame (§ 5.18.2): the decoder
    /// outputs it by order hint. Needs `monotonic_output_order_flag = 0`.
    pub implicit_output: bool,
}

/// `our_ref`, `their_ref` with its `ns` range, and the `PrevGmParams` of each
/// reference (§ 5.18.9.1) of one `global_motion_params()` coding.
struct GmBase {
    our_ref: u32,
    their_ref: Option<(u32, u32)>,
    predictions: Vec<Option<[i32; 6]>>,
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
            ccso: params.filters.ccso.map(|plane| CcsoPlaneParams {
                reuse_ccso: false,
                sb_reuse_ccso: false,
                ccso_ref_idx: None,
                ..plane
            }),
            ..params.filters
        },
        ..*params
    };
    let header = parse_inter_frame_header(
        seq,
        refs,
        &InterFrameParams {
            explicit_refs: None,
            ..plain
        },
        (None, 0),
        &[],
    )?;
    let inter = header
        .core
        .inter
        .as_ref()
        .ok_or(EncoderHeaderError::Incomplete)?;
    let mut num_total_refs = inter.num_total_refs.unwrap_or(0);
    if let Some(limit) = params.explicit_refs {
        if !seq.inter.explicit_ref_frame_map || limit == 0 {
            return Err(unsupported(
                "an explicit reference map the sequence does not allow",
            ));
        }
        num_total_refs = num_total_refs.min(u32::from(limit));
    }
    let ref_frame_idx =
        &inter.ref_frame_idx[..inter.ref_frame_idx.len().min(num_total_refs as usize)];
    let position = params.primary_slot.and_then(|slot| {
        let position = ref_frame_idx
            .iter()
            .zip(inter.order_hints.iter())
            .position(|(&index, &hint)| index == slot && hint != u32::MAX)?;
        u32::try_from(position).ok()
    });
    if position.is_none()
        && params.explicit_refs.is_none()
        && !params.use_ref_frame_mvs
        && params.filters == plain.filters
        && params.global_motion == plain.global_motion
    {
        return Ok(header);
    }
    let ref_frame_idx = ref_frame_idx.to_vec();
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
    let (counts, entries) = lr_reference_filters(refs, ref_frame_idx, map.1);
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
    let gm = cheapest_gm_base(seq, refs, params, map.1, ref_frame_idx)?;
    let explicit = params.explicit_refs.map(|_| ref_frame_idx);
    let (bytes, bit_len) =
        inter_frame_header_bits(seq, params, map, (lr_refs, &gm), (weight_coded, explicit))?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTileGroup, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTileGroup, true);
    let mfh = encoder_mfh_view(seq, params.tx.multi_frame_header);
    parse_core_body(&mut reader, &mut core, seq, mfh.as_ref(), refs)?;
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
    (lr_refs, gm): (LrReferenceFilters<'_>, &GmBase),
    (weight_coded, explicit): (bool, Option<&[u32]>),
) -> Result<(Vec<u8>, u64), EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    let mut w = BitWriter::new();
    write_mfh_reference(&mut w, params.tx.multi_frame_header)?;
    w.write_flag(true)?; // frame_is_inter
    w.write_flag(params.show_frame)?; // immediate_output_frame
    write_implicit_output(&mut w, seq, params)?;
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    let choose = params.tx.choose_primary_ref_frame;
    w.write_flag(!choose)?; // signal_primary_ref_frame
    w.write_flag(primary_ref_frame.is_none() || params.tx.disable_cross_frame_cdf_init)?;
    if !choose {
        w.write_bits(primary_ref_frame.unwrap_or(PRIMARY_REF_NONE), 3)?; // primary_ref_frame
    }
    if seq.seq_force_screen_content_tools == 1 {
        return Err(unsupported("forced screen content tools"));
    }
    write_refresh_frame_flags(&mut w, seq, params.refresh_frame_flags)?;
    write_explicit_ref_map(&mut w, seq, explicit)?;
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
            w.write_flag(params.tip_hole_fill)?; // allow_tip_hole_fill
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
    w.write_flag(params.tx.disable_cdf_update)?;
    let tx = params.tx;
    if tx.allow_intrabc {
        return Err(unsupported("intra block copy in an inter frame"));
    }
    check_tx_params(params.base_q_idx, tx)?;
    write_quant_and_filters(
        &mut w,
        seq,
        (params.base_q_idx, params.tiles, tx),
        &params.filters,
        (false, lr_refs, primary_ref_frame.map(|_| false)),
    )?;
    if params.base_q_idx > 0 {
        w.write_flag(tx.tx_mode == TxMode::Select)?; // tx_mode_select
    }
    w.write_flag(params.reference_select)?;
    w.write_flag(params.skip_mode_present)?;
    if seq.inter.enable_bawp {
        w.write_flag(true)?; // allow_bawp
    }
    if warp {
        w.write_flag(true)?; // allow_warpmv_mode
    }
    w.write_bits(u32::from(tx.reduced_tx_set), 2)?;
    write_global_motion(&mut w, seq, params, (num_total_refs, gm))?;
    let output = params.show_frame || params.implicit_output;
    write_film_grain_config(&mut w, seq, output, tx.grain_seed)?;
    let bit_len = w.bit_len();
    w.align_to_byte();
    Ok((w.into_bytes(), bit_len))
}

/// The [`GmBase`] of the fewest `global_motion_params()` bits over the
/// references `ref_frame_idx` of the § 7.23 state `refs`: `our_ref =
/// NumTotalRefs`, and with `global_motion_predict` also each `our_ref` and
/// `their_ref` that § 6.17.9.1 allows. Ties keep the earlier base.
fn cheapest_gm_base(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
    num_total_refs: u32,
    ref_frame_idx: &[u32],
) -> Result<GmBase, EncoderHeaderError> {
    let slots = &ref_frame_idx[..ref_frame_idx.len().min(num_total_refs as usize)];
    let hint = |slot: u32| refs.ref_order_hint?.get(slot as usize).copied();
    let predict = |base: ([i32; 6], i32)| -> Vec<Option<[i32; 6]>> {
        slots
            .iter()
            .map(|&slot| base_prediction(params.order_hint, hint(slot)?, base))
            .collect()
    };
    let default = GmBase {
        our_ref: num_total_refs,
        their_ref: None,
        predictions: predict((DEFAULT_WARP_PARAMS, 1)),
    };
    if !params.global_motion_predict || params.global_motion.iter().all(Option::is_none) {
        return Ok(default);
    }
    let mut bases = vec![default];
    for (our_ref, &slot) in (0u32..).zip(slots) {
        let Some(our_hint) = hint(slot).and_then(|h| i32::try_from(h).ok()) else {
            continue;
        };
        let index = slot as usize;
        let saved = refs.ref_num_total_refs.and_then(|counts| counts.get(index));
        let Some(&count @ 0..=7) = saved else {
            continue;
        };
        if count == 0 {
            bases.push(GmBase {
                our_ref,
                their_ref: None,
                predictions: predict((DEFAULT_WARP_PARAMS, 1)),
            });
            continue;
        }
        let saved_hints = refs
            .saved_global_motion_order_hints
            .and_then(|h| h.get(index));
        let saved_params = refs.saved_global_motion_params.and_then(|p| p.get(index));
        let (Some(saved_hints), Some(saved_params)) = (saved_hints, saved_params) else {
            continue;
        };
        for their_ref in 0..count {
            let their = their_ref as usize;
            let Ok(saved_hint) = i32::try_from(saved_hints[their]) else {
                continue;
            };
            bases.push(GmBase {
                our_ref,
                their_ref: Some((their_ref, count)),
                predictions: predict((
                    saved_params[their],
                    get_relative_dist(our_hint, saved_hint),
                )),
            });
        }
    }
    let mut best: Option<(u64, GmBase)> = None;
    for base in bases {
        let mut w = BitWriter::new();
        write_global_motion(&mut w, seq, params, (num_total_refs, &base))?;
        if best.as_ref().is_none_or(|(bits, _)| w.bit_len() < *bits) {
            best = Some((w.bit_len(), base));
        }
    }
    best.map(|(_, base)| base)
        .ok_or(EncoderHeaderError::Incomplete)
}

/// Writes `global_motion_params()` (§ 5.18.9.1) of `params` over a map of
/// `num_total_refs` references with the base `gm`, whose `predictions` are
/// `None` for a reference that reads no model.
fn write_global_motion(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    params: &InterFrameParams,
    (num_total_refs, gm): (u32, &GmBase),
) -> Result<(), EncoderHeaderError> {
    let predictions = &gm.predictions;
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
    w.write_ns(gm.our_ref, num_total_refs + 1)?; // our_ref
    if let Some((their_ref, count)) = gm.their_ref {
        w.write_ns(their_ref, count)?;
    }
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

/// Builds the `INTRA_ONLY_FRAME` header (§ 5.18.2 `frame_is_inter = 0`) of
/// `params` in an `OBU_REGULAR_TILE_GROUP`, parsed with `refs`: its
/// `order_hint`, `show_frame` and `refresh_frame_flags`, then the key-frame
/// tail of `base_q_idx`, `tiles`, `filters` and `tx`. An intra frame codes
/// `PRIMARY_REF_NONE` and no references; the slots it does not refresh keep
/// their frames for later frames.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a single-picture view, a zero or
/// too-wide `base_q_idx`, `filters` or `tx` the sequence cannot carry, or bits
/// that do not parse to a complete intra header with `refs`.
pub fn build_intra_only_frame_header(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
) -> Result<EncodedFrameHeader, EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    let mut w = BitWriter::new();
    write_mfh_reference(&mut w, params.tx.multi_frame_header)?;
    w.write_flag(false)?; // frame_is_inter
    w.write_flag(params.show_frame)?; // immediate_output_frame
    write_implicit_output(&mut w, seq, params)?;
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    write_refresh_frame_flags(&mut w, seq, params.refresh_frame_flags)?;
    let output = params.show_frame || params.implicit_output;
    let tail = (params.base_q_idx, params.tiles, output);
    write_intra_frame_tail(&mut w, seq, tail, &params.filters, params.tx)?;
    let bytes = w.into_bytes();
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTileGroup, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTileGroup, true);
    let mfh = encoder_mfh_view(seq, params.tx.multi_frame_header);
    parse_core_body(&mut reader, &mut core, seq, mfh.as_ref(), refs)?;
    let bit_len = reader.consumed_bits();
    core.consumed_bits = bit_len;
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_type != Some(crate::headers::frame::FrameType::IntraOnly)
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let bits =
        RecordedFrameHeaderBits::record(&mut BitReader::new(&bytes, ByteOffset::new(0)), bit_len)?;
    Ok(EncodedFrameHeader { core, bits })
}

/// Serializes the `OBU_REGULAR_TILE_GROUP` OBUs of `header` that carry the
/// `tiles` of the frame in `tile_groups` tile groups (see
/// [`tile_group_ranges`]): the first codes `is_first_tile_group` and the
/// recorded header bits, each codes its § 5.19 structure and tiles.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the header has no tile info, a tile is empty
/// or too long for `TileSizeBytes`, the tile count differs from the header, or
/// the OBU does not serialize.
pub fn encode_inter_frame_obu(
    header: &EncodedFrameHeader,
    tiles: &[&[u8]],
    tile_groups: u32,
) -> Result<Vec<u8>, EncoderHeaderError> {
    write_tile_groups(
        &header.core,
        ObuType::RegularTileGroup,
        (tiles, tile_groups),
        |structure, tiles| {
            let mut payload = BitWriter::new();
            payload.write_flag(true)?; // is_first_tile_group
            for index in 0..header.bits.num_frame_header_bits() {
                let bit = header
                    .bits
                    .bit(index)
                    .ok_or(EncoderHeaderError::Incomplete)?;
                payload.write_flag(bit)?;
            }
            write_group_tiles(&mut payload, &header.core, structure, tiles)?;
            Ok(payload.into_bytes())
        },
    )
}

/// The `(tg_start, tg_end)` of each tile group when `num_tiles` tiles in
/// raster order split into `tile_groups` groups of consecutive tiles
/// (§ 5.19): the count clamps to `1..=num_tiles`, and group `g` starts at
/// tile `g * num_tiles / tile_groups`, so the group sizes differ by at most 1.
#[must_use]
pub fn tile_group_ranges(num_tiles: u32, tile_groups: u32) -> Vec<(u32, u32)> {
    let groups = tile_groups.clamp(1, num_tiles.max(1));
    (0..groups)
        .map(|g| {
            let end = (g + 1) * num_tiles / groups;
            (g * num_tiles / groups, end.saturating_sub(1))
        })
        .collect()
}

/// The tile group OBUs of type `obu_type` of the frame `core`, with its
/// `tiles` in the groups of [`tile_group_ranges`]. `first` writes the
/// payload of the first group from its structure and tiles; each later group
/// codes `is_first_tile_group = 0` and `frame_header_present_flag = 0`
/// (§ 5.19), so the frame header stays in the first group alone.
pub(super) fn write_tile_groups(
    core: &FrameHeaderCore,
    obu_type: ObuType,
    (tiles, tile_groups): (&[&[u8]], u32),
    first: impl FnOnce(&TileGroupStructure, &[&[u8]]) -> Result<Vec<u8>, EncoderHeaderError>,
) -> Result<Vec<u8>, EncoderHeaderError> {
    let num_tiles = tiles.len() as u32;
    let ranges = tile_group_ranges(num_tiles, tile_groups);
    let mut first = Some(first);
    let mut w = BitWriter::new();
    for &(tg_start, tg_end) in &ranges {
        let structure = TileGroupStructure {
            tile_start_and_end_present_flag: ranges.len() > 1,
            tg_start,
            tg_end,
            ..TileGroupStructure::whole_frame(num_tiles)
        };
        let group = tiles
            .get(tg_start as usize..=tg_end as usize)
            .ok_or(EncoderHeaderError::Incomplete)?;
        let payload = if let Some(first) = first.take() {
            first(&structure, group)?
        } else {
            let mut payload = BitWriter::new();
            payload.write_flag(false)?; // is_first_tile_group
            payload.write_flag(false)?; // frame_header_present_flag
            write_group_tiles(&mut payload, core, &structure, group)?;
            payload.into_bytes()
        };
        write_annexb_obu(&mut w, &obu_header(obu_type), &payload)?;
    }
    Ok(w.into_bytes())
}

/// Writes the § 5.19 structure of `structure` and the § 5.20.1 framing of
/// its `tiles` with the tile info of `core`.
fn write_group_tiles(
    payload: &mut BitWriter,
    core: &FrameHeaderCore,
    structure: &TileGroupStructure,
    tiles: &[&[u8]],
) -> Result<(), EncoderHeaderError> {
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(EncoderHeaderError::Incomplete)?;
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    write_tile_group_structure(payload, structure, layout)?;
    write_tile_group_payload(
        payload,
        &tile_framing(core, tiles),
        tiles,
        tile_info.tile_size_bytes.unwrap_or(1),
        false,
    )?;
    Ok(())
}

/// Builds the `OBU_REGULAR_TIP` header of a `TIP_FRAME_AS_OUTPUT` frame
/// (§ 5.18.2) over the reference map `ref_frame_idx` for `seq` and the
/// § 7.23 reference state `refs`: the `order_hint`, `refresh_frame_flags`,
/// `show_frame` and `tip_weight` of `params`, `PRIMARY_REF_NONE`,
/// `use_ref_frame_mvs = 1`, the `tip_global_mv` and `tip_filter` of
/// `params`, `quantization_params()` at `base_q_idx` with
/// `enable_tip_explicit_qp`, TIP deblocking with a one-tile `tile_info()`
/// when `tip_deblocking`, and the `film_grain_config()` of
/// `params.tx.grain_seed`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a sequence without TIP output, the
/// tools the header does not write, a missing `tip_weight`, a
/// `tip_global_mv` component outside `-15..=15`, a `tip_filter` other than
/// sharp, regular or smooth, `tip_deblocking` without `enable_df_sub_pu` or
/// with tiles, a `grain_seed` in a sequence without film grain or in a frame
/// that is not shown ([`WriteError::NonCanonicalFrameHeader`]), or bits that
/// do not parse to a complete TIP output header with `refs`.
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
    let mv = params.tip_global_mv;
    if mv.row.unsigned_abs() > 15 || mv.col.unsigned_abs() > 15 {
        return Err(unsupported("TipGlobalMv outside -15..=15"));
    }
    let tip_regular = match params.tip_filter {
        None | Some(InterpolationFilter::EighttapSharp) => None,
        Some(InterpolationFilter::Eighttap) => Some(true),
        Some(InterpolationFilter::EighttapSmooth) => Some(false),
        Some(_) => return Err(unsupported("TipInterpFilter")),
    };
    if params.tip_deblocking
        && (!seq.filter.enable_df_sub_pu || params.tiles != FrameTiles::default())
    {
        return Err(unsupported(
            "TIP deblocking without enable_df_sub_pu or in tiles",
        ));
    }
    let weight = params
        .tip_weight
        .ok_or(unsupported("tip_global_wtd_index"))?;
    let mut w = BitWriter::new();
    write_mfh_reference(&mut w, params.tx.multi_frame_header)?;
    w.write_flag(params.show_frame)?; // immediate_output_frame
    write_implicit_output(&mut w, seq, params)?;
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    w.write_flag(true)?; // signal_primary_ref_frame
    w.write_bits(PRIMARY_REF_NONE, 3)?; // primary_ref_frame
    write_refresh_frame_flags(&mut w, seq, params.refresh_frame_flags)?;
    write_explicit_ref_map(&mut w, seq, None)?;
    w.write_flag(true)?; // use_ref_frame_mvs
    if seq.tile.frame_sb_size(false) != SuperblockSize::Block64x64 {
        w.write_flag(false)?; // tmvp_sample_step_minus_1
    }
    if inter.enable_tip_hole_fill {
        w.write_flag(params.tip_hole_fill)?; // allow_tip_hole_fill
    }
    let weight_coded = tip_weight_coded(
        &build_inter_seq_view(seq),
        refs,
        (ref_frame_idx, params.order_hint),
        true,
        0,
    );
    write_tip_weight(&mut w, weight, weight_coded.unwrap_or(true))?;
    w.write_flag(mv == TipGlobalMv::default())?; // tip_mv_zero
    if mv != TipGlobalMv::default() {
        w.write_bits(mv.row.unsigned_abs(), 4)?; // tip_mv_row
        w.write_bits(mv.col.unsigned_abs(), 4)?; // tip_mv_col
        for value in [mv.row, mv.col].into_iter().filter(|&value| value != 0) {
            w.write_flag(value < 0)?; // tip_mv_row_sign, tip_mv_col_sign
        }
    }
    w.write_flag(tip_regular.is_none())?; // tip_sharp
    if let Some(regular) = tip_regular {
        w.write_flag(regular)?; // tip_regular
    }
    if inter.enable_tip_explicit_qp {
        let plane = plane_quantization(seq, params.base_q_idx, params.tx);
        let dc = |ac| if seq.quant.equal_ac_dc_q { ac } else { 0 };
        let quant = QuantizationParams {
            delta_q_y_dc: 0,
            delta_q_u_dc: dc(plane.delta_q_u_ac),
            delta_q_v_dc: dc(plane.delta_q_v_ac),
            ..plane
        };
        write_quantization_params(&mut w, &quant, &seq.quant, true)?;
    }
    if seq.filter.enable_df_sub_pu {
        w.write_flag(params.tip_deblocking)?; // allow_df_sub_pu
        if params.tip_deblocking {
            w.write_flag(true)?; // apply_deblocking_filter_tip
            write_encoder_tile_info(&mut w, seq, &params.tiles, false)?;
        }
    }
    let output = params.show_frame || params.implicit_output;
    write_film_grain_config(&mut w, seq, output, params.tx.grain_seed)?;
    let bit_len = w.bit_len();
    w.write_trailing_bits(8 - bit_len % 8)?;
    let bytes = w.into_bytes();
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTip, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTip, true);
    let mfh = encoder_mfh_view(seq, params.tx.multi_frame_header);
    parse_core_body(&mut reader, &mut core, seq, mfh.as_ref(), refs)?;
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
/// `frame_to_show_map_idx` (§ 5.18.2) with the `film_grain_config()` of
/// `grain_seed` (see [`KeyFrameTxParams::grain_seed`]): with
/// `derive_sef_order_hint = 1`, or with `sef_order_hint` = the low
/// `OrderHintBits` of `order_hint` when it is given.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a slot outside the sequence's reference
/// slots, a sequence without a known `film_grain_params_present`, a seed
/// without film grain, or bits that do not parse to a complete show-existing
/// frame. With `multi_frame_header` the frame codes `cur_mfh_id = 1`.
pub fn encode_show_existing_frame_obu(
    seq: &CoreSeqView,
    frame_to_show_map_idx: u32,
    order_hint: Option<u32>,
    grain_seed: Option<u16>,
    multi_frame_header: bool,
) -> Result<Vec<u8>, EncoderHeaderError> {
    if frame_to_show_map_idx >= seq.num_ref_frames {
        return Err(unsupported("frame_to_show_map_idx"));
    }
    seq.film_grain_params_present
        .ok_or(EncoderHeaderError::Incomplete)?;
    let mut w = BitWriter::new();
    write_mfh_reference(&mut w, multi_frame_header)?;
    w.write_bits(frame_to_show_map_idx, ceil_log2(seq.num_ref_frames))?;
    w.write_flag(order_hint.is_none())?; // derive_sef_order_hint
    if let Some(order_hint) = order_hint {
        w.write_bits(
            order_hint & ((1 << seq.order_hint_bits) - 1),
            seq.order_hint_bits,
        )?; // sef_order_hint
    }
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
        encoder_mfh_view(seq, multi_frame_header).as_ref(),
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
    annexb_obu(
        ObuType::FilmGrain,
        &crate::obu::ParsedObu::FilmGrain(Box::new(obu)),
    )
}

/// Serializes the `OBU_CONTENT_INTERPRETATION` (§ 5.15) of `content`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for content the OBU cannot code.
pub fn encode_content_interpretation_obu(
    content: &ContentInterpretation,
) -> Result<Vec<u8>, EncoderHeaderError> {
    annexb_obu(
        ObuType::ContentInterpretation,
        &crate::obu::ParsedObu::ContentInterpretation(*content),
    )
}

/// Serializes the `OBU_MULTI_FRAME_HEADER` (§ 5.7) that frames with
/// [`KeyFrameTxParams::multi_frame_header`] refer to: `mfh_id_minus_1 = 0`
/// for sequence header 0, with no frame size, deblocking update or
/// segmentation information.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the OBU does not serialize.
pub fn encode_multi_frame_header_obu() -> Result<Vec<u8>, EncoderHeaderError> {
    annexb_obu(
        ObuType::MultiFrameHeader,
        &crate::obu::ParsedObu::MultiFrameHeader(Box::new(MultiFrameHeader {
            mfh_seq_header_id: 0,
            mfh_id_minus_1: 0,
            mfh_frame_size: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            segment_info: None,
        })),
    )
}

/// Serializes a prefix `OBU_METADATA_SHORT` (§ 5.17.2) with the
/// `metadata_hdr_cll()` (§ 5.17.5) of `light_level`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the OBU does not serialize.
pub fn encode_hdr_cll_obu(light_level: MetadataHdrCll) -> Result<Vec<u8>, EncoderHeaderError> {
    hdr_metadata_obu(
        MetadataType::HdrCll,
        4,
        MetadataPayload::HdrCll(light_level),
    )
}

/// Serializes a prefix `OBU_METADATA_SHORT` (§ 5.17.2) with the
/// `metadata_hdr_mdcv()` (§ 5.17.6) of `display`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the OBU does not serialize.
pub fn encode_hdr_mdcv_obu(display: MetadataHdrMdcv) -> Result<Vec<u8>, EncoderHeaderError> {
    hdr_metadata_obu(MetadataType::HdrMdcv, 24, MetadataPayload::HdrMdcv(display))
}

/// A prefix `OBU_METADATA_SHORT` of `payload_size` bytes for every layer of
/// its extended layer (`LAYER_GLOBAL`: Table 6.17 lists both types as not
/// layer-specific) and the whole sequence (`GLOBAL_PERSISTENCE`, § 6.16.3).
fn hdr_metadata_obu(
    metadata_type: MetadataType,
    payload_size: usize,
    payload: MetadataPayload,
) -> Result<Vec<u8>, EncoderHeaderError> {
    let obu = MetadataShortObu {
        metadata_is_suffix: false,
        muh_layer_idc: 1,
        muh_cancel_flag: false,
        muh_persistence_idc: 0,
        metadata_type,
        metadata_type_leb128_bytes: 1,
        unit: Some(MetadataUnit {
            metadata_type,
            payload_size,
            payload,
        }),
    };
    annexb_obu(
        ObuType::MetadataShort,
        &crate::obu::ParsedObu::MetadataShort(Box::new(obu)),
    )
}

/// `obu` as an Annex B OBU of `obu_type` on layer 0.
fn annexb_obu(
    obu_type: ObuType,
    obu: &crate::obu::ParsedObu,
) -> Result<Vec<u8>, EncoderHeaderError> {
    let mut payload = BitWriter::new();
    crate::write::write_obu_payload(&mut payload, obu, obu_type.is_extensible_obu(), &[])?;
    let mut w = BitWriter::new();
    write_annexb_obu(&mut w, &obu_header(obu_type), &payload.into_bytes())?;
    Ok(w.into_bytes())
}

/// The § 5.18.2 `refresh_frame_flags` of an inter frame: with
/// `enable_short_refresh_frame_flags`, `has_refresh_frame_flags` and the
/// `frame_to_refresh` of at most one slot.
fn write_refresh_frame_flags(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    flags: u32,
) -> Result<(), EncoderHeaderError> {
    if !seq.enable_short_refresh_frame_flags {
        return Ok(w.write_bits(flags, seq.num_ref_frames)?);
    }
    if flags.count_ones() > 1 || flags >> seq.num_ref_frames != 0 {
        return Err(unsupported(
            "short refresh_frame_flags of more than one slot",
        ));
    }
    w.write_flag(flags != 0)?; // has_refresh_frame_flags
    if flags != 0 {
        w.write_bits(flags.trailing_zeros(), ceil_log2(seq.num_ref_frames))?; // frame_to_refresh
    }
    Ok(())
}

/// `frame_explicit_ref_frame_map` of a sequence with `explicit_ref_frame_map`,
/// then for an `explicit` map its `num_total_refs` and `ref_frame_idx`
/// (§ 5.18.2).
fn write_explicit_ref_map(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    explicit: Option<&[u32]>,
) -> Result<(), EncoderHeaderError> {
    if seq.inter.explicit_ref_frame_map {
        w.write_flag(explicit.is_some())?; // frame_explicit_ref_frame_map
    }
    if let Some(map) = explicit {
        w.write_bits(u32::try_from(map.len()).unwrap_or(u32::MAX), 3)?; // num_total_refs
        for &index in map {
            w.write_bits(index, ceil_log2(seq.num_ref_frames))?; // ref_frame_idx
        }
    }
    Ok(())
}

/// `implicit_output_frame` of a hidden frame in a sequence without monotonic
/// output order (§ 5.18.2).
fn write_implicit_output(
    w: &mut BitWriter,
    seq: &CoreSeqView,
    params: &InterFrameParams,
) -> Result<(), EncoderHeaderError> {
    if params.implicit_output && (params.show_frame || seq.monotonic_output_order_flag) {
        return Err(unsupported(
            "implicit output of a shown frame or in monotonic order",
        ));
    }
    if !params.show_frame && !seq.monotonic_output_order_flag {
        w.write_flag(params.implicit_output)?; // implicit_output_frame
    }
    Ok(())
}

fn unsupported(what: &'static str) -> EncoderHeaderError {
    WriteError::NonCanonicalFrameHeader { what }.into()
}

#[cfg(test)]
#[path = "encoder_inter_stream_tests.rs"]
mod tests;
