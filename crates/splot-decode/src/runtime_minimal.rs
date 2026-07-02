// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared minimal-tier runtime implementation.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, FrameSize, TxMode, parse_frame_header_core,
};
use splot_core::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, SequenceHeader, parse_sequence_header,
};
use splot_core::ivf::{IvfHeader, IvfWarning};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream, parse_bitstream_partial};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};
use splot_core::types::ObuType;
use splot_recon::{BitDepth, DecodedFrame, DecodedFrameHashInput};

use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, FrameCdfSubset,
    GeneralIntraBlockModeError, GeneralIntraResidualError, MinimalBlockSymbolTraceError,
    MinimalRuntimeBlockSymbolFrontierError, MinimalRuntimePartitionFrontierError,
    MinimalRuntimeReconstructionTrace, TileGroupPositionFacts, TilePartitionTraversalError,
};
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use self::capability::missing_capability_message;
use self::limits::{checked_add, decoded_frame_byte_budget};
#[cfg(test)]
use self::wienerns_lr::{
    LR_LIVE_FRAME_SAMPLE_STORAGE_BYTES, LR_LIVE_TX_SKIP_STORAGE_BYTES_PER_VALUE,
    WIENER_NS_CHROMA_SOURCE_TAP_COUNT, WienerNsLrClassifiedWienerStorageInputs,
    WienerNsLrClassifiedWienerValueSourceSample, WienerNsLrFilterClassValue,
    WienerNsLrLiveStorageAllocation, WienerNsLrRuntimeStorageRetentionFrontier,
    WienerNsLrSourceReadConfig, WienerNsLrSourceReadFrontier, WienerNsLrSourceReadSample,
    WienerNsLrTxSkipGrid, WienerNsLrTxSkipLookup, WienerNsLrTxSkipTransformRecord,
    derive_wienerns_lr_classified_wiener_frontier,
    derive_wienerns_lr_classified_wiener_storage_frontier,
    derive_wienerns_lr_classified_wiener_values_frontier,
    derive_wienerns_lr_live_storage_allocation, derive_wienerns_lr_runtime_source_frontiers,
    derive_wienerns_lr_runtime_storage_retention_frontier, derive_wienerns_lr_source_read_frontier,
    derive_wienerns_lr_tx_skip_grid_retention, map_wienerns_lr_unit_frontier_error,
    populate_wienerns_lr_live_tx_skip_from_transform_records,
    record_wienerns_lr_chroma_luma_source_reads,
    wienerns_lr_classified_wiener_storage_runtime_error,
    wienerns_lr_live_frame_samples_unpopulated_error, wienerns_lr_live_storage_allocation_error,
    wienerns_lr_runtime_storage_retention_error, wienerns_lr_source_read_config,
    wienerns_lr_source_read_runtime_error, wienerns_lr_tx_mode_select_transform_record_error,
};
use self::wienerns_lr::{
    ensure_wienerns_lr_unit_runtime_frontier, reconstruct_ac0ej3_selectable_intra_region,
};
pub const MINIMAL_INTRA_HASH_TIER_ID: &str = "minimal-intra-8bit420-hash-v1";

const FEATURE_ID: &str = "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS";
const MATRIX_ROW: &str = "minimal-decode-tier-contract";
const SPEC_SECTION: &str = "7.1";
const REMEDIATION: &str = "Use a stream inside minimal-intra-8bit420-hash-v1 or wait for the referenced decoder support row.";
const AC0EJ3_CHROMA_FEATURE_ID: &str = "DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER";
const AC0EJ3_CHROMA_MATRIX_ROW: &str = "ac0ej3-sequence-chroma-frontier";
const AC0EJ3_WIENERNS_FEATURE_ID: &str = "DECODE-AC0EJ3-WIENERNS-FRONTIER";
const AC0EJ3_WIENERNS_MATRIX_ROW: &str = "ac0ej3-wienerns-frontier";
const AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID: &str = "DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER";
const AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW: &str = "ac0ej3-lr-unit-selections-frontier";
const AC0EJ3_LR_SOURCE_READ_FEATURE_ID: &str = "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER";
const AC0EJ3_LR_SOURCE_READ_MATRIX_ROW: &str = "ac0ej3-lr-source-read-frontier";
#[allow(dead_code)]
const AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-STORAGE";
#[allow(dead_code)]
const AC0EJ3_LR_CLASSIFIED_WIENER_STORAGE_MATRIX_ROW: &str = "ac0ej3-lr-classified-wiener-storage";
const AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION";
const AC0EJ3_LR_RUNTIME_STORAGE_RETENTION_MATRIX_ROW: &str = "ac0ej3-lr-runtime-storage-retention";
#[allow(dead_code)]
const AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION";
#[allow(dead_code)]
const AC0EJ3_LR_LIVE_STORAGE_ALLOCATION_MATRIX_ROW: &str = "ac0ej3-lr-live-storage-allocation";
#[allow(dead_code)]
const AC0EJ3_LR_LIVE_TX_SKIP_GRID_FEATURE_ID: &str = "DECODE-AC0EJ3-LR-LIVE-TX-SKIP-GRID";
#[allow(dead_code)]
const AC0EJ3_LR_LIVE_TX_SKIP_GRID_MATRIX_ROW: &str = "ac0ej3-lr-live-tx-skip-grid";
const AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF";
const AC0EJ3_LR_LIVE_TRANSFORM_RECORD_HANDOFF_MATRIX_ROW: &str =
    "ac0ej3-lr-live-transform-record-handoff";
const AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_FEATURE_ID: &str =
    "DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS";
const AC0EJ3_SELECTABLE_TRANSFORM_RECORDS_MATRIX_ROW: &str = "ac0ej3-selectable-transform-records";
const AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LUMA-TXTYPE-RESIDUAL-HANDOFF";
const AC0EJ3_LUMA_TXTYPE_RESIDUAL_HANDOFF_MATRIX_ROW: &str = "ac0ej3-luma-txtype-residual-handoff";
const AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_FEATURE_ID: &str = "DECODE-AC0EJ3-DCTONLY-RESIDUAL-FRONTIER";
const AC0EJ3_DCTONLY_RESIDUAL_FRONTIER_MATRIX_ROW: &str = "ac0ej3-dctonly-residual-frontier";

pub(crate) fn effective_allow_screen_content_tools(core: &FrameHeaderCore) -> bool {
    core.allow_screen_content_tools
        .or_else(|| {
            core.inter
                .as_ref()
                .and_then(|inter| inter.allow_screen_content_tools)
        })
        .unwrap_or(false)
}
const AC0EJ3_INTRA_IST_ZERO_FRONTIER_FEATURE_ID: &str = "DECODE-AC0EJ3-INTRA-IST-ZERO-FRONTIER";
const AC0EJ3_INTRA_IST_ZERO_FRONTIER_MATRIX_ROW: &str = "ac0ej3-intra-ist-zero-frontier";
const MINIMAL_WIDTH: u32 = 64;
const MINIMAL_HEIGHT: u32 = 64;
const MINIMAL_TRACE_SYMBOLS: u64 = 6;
const MINIMAL_TRACE_TRAILING_BIT_POSITION: u64 = 14;
const MINIMAL_TRACE_PADDING_END_POSITION: u64 = 16;
const FROZEN_MINIMAL_BASE_Q_IDX: u32 = 255;

const GENERAL_INTRA_FEATURE_ID: &str = "DECODE-GENERAL-INTRA-FRAME-FRONTIER";
const GENERAL_INTRA_MATRIX_ROW: &str = "general-intra-frame-frontier";
const GENERAL_INTRA_TIER_ID: &str = "general-intra-8bit420-frontier-v1";
const GENERAL_INTRA_TILE_SPEC_SECTION: &str = "5.20.1";
const GENERAL_INTRA_PARTITION_SPEC_SECTION: &str = "5.20.3.1";
const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";
const GENERAL_INTRA_REMEDIATION: &str =
    "Use an admitted general-intra subset or track DECODE-GENERAL-INTRA-FRAME-FRONTIER.";
const GENERAL_INTRA_DELTA_DCQUANT_MIN: i32 = (1 << 3) - (1 << 5) + 1;
pub(crate) enum MinimalRuntimeDecodedFrame {
    Eight(DecodedFrame<u8>),
    Ten(DecodedFrame<u16>),
}

fn deblock_quant_deltas(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> deblock::DeblockQuantDeltas {
    let (Some(tq), Some(quant)) = (
        sequence.transform_quant_entropy.as_ref(),
        core.quantization_params,
    ) else {
        return deblock::DeblockQuantDeltas::ZERO;
    };
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
    deblock::DeblockQuantDeltas::from_frame_quant(quant, seq_quant.base_uv_ac_delta_q)
}

pub(crate) struct MinimalRuntimeFrame {
    pub(crate) frame: MinimalRuntimeDecodedFrame,
    pub(crate) frame_cdfs: FrameCdfSubset,
    pub(crate) frame_rate_numerator: u32,
    pub(crate) frame_rate_denominator: u32,
}

impl MinimalRuntimeFrame {
    pub(crate) fn frame_eight(&self) -> Result<&DecodedFrame<u8>> {
        match &self.frame {
            MinimalRuntimeDecodedFrame::Eight(frame) => Ok(frame),
            MinimalRuntimeDecodedFrame::Ten(_) => Err(unsupported(
                "unsupported_10bit_reference_retention",
                None,
                missing_capability_message!("reference.retention bit_depth=10"),
            )),
        }
    }
    pub(crate) fn frame_ten(&self) -> Result<&DecodedFrame<u16>> {
        match &self.frame {
            MinimalRuntimeDecodedFrame::Ten(frame) => Ok(frame),
            MinimalRuntimeDecodedFrame::Eight(_) => Err(unsupported(
                "unsupported_8bit_reference_for_10bit_decode",
                None,
                "minimal inter runtime requires reference frames to match the active 10-bit storage",
            )),
        }
    }
    pub(crate) fn byte_len(&self) -> Result<usize> {
        match &self.frame {
            MinimalRuntimeDecodedFrame::Eight(frame) => {
                Ok(DecodedFrameHashInput::new(frame).byte_len()?)
            }
            MinimalRuntimeDecodedFrame::Ten(frame) => {
                Ok(DecodedFrameHashInput::new(frame).byte_len()?)
            }
        }
    }
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(crate) fn frame(&self) -> &DecodedFrame<u8> {
        match &self.frame {
            MinimalRuntimeDecodedFrame::Eight(frame) => frame,
            MinimalRuntimeDecodedFrame::Ten(_) => {
                panic!("frame() called on a 10-bit MinimalRuntimeFrame; use into_frame_ten()")
            }
        }
    }
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(crate) fn into_frame_eight(self) -> DecodedFrame<u8> {
        match self.frame {
            MinimalRuntimeDecodedFrame::Eight(frame) => frame,
            MinimalRuntimeDecodedFrame::Ten(_) => {
                panic!("into_frame_eight() called on a 10-bit MinimalRuntimeFrame")
            }
        }
    }
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(crate) fn into_frame_ten(self) -> DecodedFrame<u16> {
        match self.frame {
            MinimalRuntimeDecodedFrame::Ten(frame) => frame,
            MinimalRuntimeDecodedFrame::Eight(_) => {
                panic!("into_frame_ten() called on an 8-bit MinimalRuntimeFrame")
            }
        }
    }
}
#[cfg(test)]
pub(crate) fn decode_minimal_frame_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<MinimalRuntimeFrame> {
    let mut frames = decode_minimal_frames_from_plan(bytes, options, plan)?;
    if frames.is_empty() {
        return Err(unsupported(
            "missing_decoded_frame",
            None,
            "minimal tier requires at least one decoded frame",
        ));
    }
    Ok(frames.swap_remove(0))
}
#[cfg(test)]
fn reconstruct_ac0ej3_intra_region_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    full_recon: bool,
) -> Result<wienerns_lr::WienerNsLrReconSink<u16>> {
    let parsed = parse_bitstream_partial(bytes);
    let (ivf, _header) = require_multiframe_ivf(&parsed)?;
    let first_ivf_frame = ivf.frames.first().ok_or_else(|| {
        unsupported(
            "missing_first_ivf_frame",
            None,
            "ac0ej3 reconstruction requires at least one IVF frame",
        )
    })?;
    let leading_obus = first_ivf_frame.obus.as_slice();
    let [_td, sequence_envelope, key_envelope] = require_minimal_obu_order(leading_obus)?;
    let sequence = parse_sequence(sequence_envelope)?;
    let key_core = parse_frame_core(key_envelope, &sequence)?;
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "ac0ej3 reconstruction requires one key frame candidate",
        )
    })?;
    Ok(wienerns_lr::reconstruct_ac0ej3_selectable_intra_region(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        &sequence,
        &key_core,
        full_recon,
    )?
    .sink)
}
pub(crate) fn decode_minimal_frames_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<MinimalRuntimeFrame>> {
    decode_minimal_frames_from_plan_with_ivf_preflight(bytes, options, plan, |_| Ok(()))
}
fn decode_minimal_key_frame(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let core = parse_frame_core(frame_envelope, sequence)?;
    if route_wienerns_lr_selectable_full_recon(sequence, &core) {
        return decode_wienerns_lr_selectable_full_recon_key_frame(
            bytes,
            options,
            plan,
            candidate,
            frame_envelope,
            sequence,
            &core,
            header,
        );
    }
    if general_intra::route_general_minimal_intra(sequence, &core) {
        return general_intra::decode_general_minimal_intra_frame(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            &core,
            options,
            header,
        );
    }
    if sequence.general.bit_depth_idc != BitDepthIdc::Eight {
        return Err(unsupported_at(
            "unsupported_10bit_frozen_minimal_tier",
            frame_envelope.offset,
            missing_capability_message!("frozen_minimal_tier bit_depth=10"),
        ));
    }
    validate_frame_core(&core, frame_envelope.offset)?;

    let mut tile_plan = derive_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        &core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(unsupported(
                "missing_tile_work_unit",
                None,
                "minimal tier requires one tile work unit",
            ));
        }
        work_units => {
            return Err(unsupported(
                "unexpected_tile_work_units",
                work_units.first().map(|tile| tile.tile_byte_span().start),
                missing_capability_message!("tile.work_unit_count !=1"),
            ));
        }
    };
    let reconstruction_trace =
        verify_flat_minimal_tile_trace(tile, sequence, &core, options.limits())?;
    tile.apply_frame_end_cdf_update();
    let frame_cdfs = tile.frame_cdfs();
    let tile_size = tile.tile_size();

    let limits = options.limits();
    ensure_runtime_limits(
        limits,
        MINIMAL_WIDTH,
        MINIMAL_HEIGHT,
        tile_size,
        BitDepth::Eight,
    )?;
    let frame =
        crate::runtime_minimal_recon::reconstruct_minimal_traced_frame(reconstruction_trace)?;

    Ok(MinimalRuntimeFrame {
        frame: MinimalRuntimeDecodedFrame::Eight(frame),
        frame_cdfs,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

fn route_wienerns_lr_selectable_full_recon(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> bool {
    sequence.general.bit_depth_idc == BitDepthIdc::Ten
        && core.is_key_frame
        && core.frame_is_intra == Some(true)
        && core
            .intra_tail
            .as_ref()
            .is_some_and(|tail| tail.tx_mode == TxMode::Select)
}

#[allow(clippy::too_many_arguments)]
fn decode_wienerns_lr_selectable_full_recon_key_frame(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let region = reconstruct_ac0ej3_selectable_intra_region(
        bytes,
        options,
        plan,
        candidate,
        frame_envelope,
        sequence,
        core,
        true,
    )?;
    let mut sink = region.sink;
    sink.finish_intra_reconstruction(frame_envelope.offset)?;
    Ok(MinimalRuntimeFrame {
        frame: MinimalRuntimeDecodedFrame::Ten(sink.into_filtered_frame(
            core,
            deblock_quant_deltas(sequence, core),
            frame_envelope.offset,
        )?),
        frame_cdfs: region.frame_cdfs,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

pub(crate) fn decode_minimal_frames_from_plan_with_ivf_preflight(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(IvfHeader) -> Result<()>,
) -> Result<Vec<MinimalRuntimeFrame>> {
    ensure_multiframe_plan_shape(plan)?;
    let parsed = parse_bitstream_partial(bytes);
    let (ivf, header) = require_multiframe_ivf(&parsed)?;
    preflight(header)?;

    let first_ivf_frame = ivf.frames.first().ok_or_else(|| {
        unsupported(
            "missing_first_ivf_frame",
            None,
            "minimal tier requires at least one IVF frame",
        )
    })?;
    let leading_obus = first_ivf_frame.obus.as_slice();
    let [td_envelope, sequence_envelope, key_envelope] = require_minimal_obu_order(leading_obus)?;
    require_obu_type(
        td_envelope,
        ObuType::TemporalDelimiter,
        "missing_temporal_delimiter",
    )?;
    require_obu_type(
        sequence_envelope,
        ObuType::SequenceHeader,
        "missing_sequence_header",
    )?;
    require_obu_type(
        key_envelope,
        ObuType::ClosedLoopKey,
        "missing_closed_loop_key",
    )?;

    let sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;

    let key_core = parse_frame_core(key_envelope, &sequence)?;
    ensure_intra_header_complete(&key_core, key_envelope.offset)?;
    let full_recon_key_frame = route_wienerns_lr_selectable_full_recon(&sequence, &key_core);
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "minimal tier requires one selected key frame candidate",
        )
    })?;
    if !full_recon_key_frame {
        reject_extra_leading_key_payload_obus(leading_obus)?;
    }
    if !full_recon_key_frame {
        ensure_wienerns_lr_unit_runtime_frontier(
            bytes,
            options,
            plan,
            key_candidate,
            key_envelope,
            sequence_envelope.offset,
            &sequence,
            &key_core,
        )?;
        ensure_sequence_chroma_tools_before_tile_decode(&sequence, sequence_envelope.offset)?;
    }
    ensure_runtime_storage_bit_depth(&sequence, sequence_envelope.offset)?;

    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .ok_or_else(|| {
                unsupported(
                    "missing_sequence_inter_config",
                    None,
                    "minimal multi-frame decode requires the sequence inter config (NumRefFrames)",
                )
            })?
            .num_ref_frames,
    );
    let mut reference = reference_buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;

    let mut frames = Vec::new();
    let mut scheduler = OutputScheduler::new(num_ref_frames);
    let mut retained_frame_bytes = 0;
    let mut output_frame_bytes = 0;
    let mut next_unvalidated_following_ivf_record = 1;
    ensure_retained_frame_byte_limits_for_core(
        options.limits(),
        retained_frame_bytes,
        &key_core,
        key_envelope.offset,
    )?;
    let key_frame = decode_minimal_key_frame(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        &sequence,
        header,
    )?;
    retained_frame_bytes =
        ensure_retained_frame_byte_limits(options.limits(), retained_frame_bytes, &key_frame)?;
    frames.push(key_frame);
    let key_update =
        frame_ref_update_from_core(&key_core, key_envelope.offset, frames[0].frame_cdfs.clone())?;
    let key_hint = key_core.order_hint_lsb.unwrap_or(0);
    let evicted = scheduler.on_refresh(key_update.refresh_frame_flags);
    output_frame_bytes =
        charge_emitted_outputs(options, &frames, &scheduler, &evicted, output_frame_bytes)?;
    reference.update(0, &key_update);
    if key_core.implicit_output_frame == Some(true) {
        scheduler.hold(key_update.refresh_frame_flags, 0, key_hint);
    }
    if key_core.immediate_output_frame == Some(true) {
        let emitted = scheduler.on_immediate(0, key_hint);
        output_frame_bytes =
            charge_emitted_outputs(options, &frames, &scheduler, &emitted, output_frame_bytes)?;
    }
    if output_frame_limit_reached(options, scheduler.emitted.len()) {
        return select_output_frames(frames, scheduler.emitted);
    }

    for next_candidate in candidates {
        match next_candidate.obu_type() {
            ObuType::RegularTileGroup => {
                let inter_envelope = following_inter_envelope(
                    ivf,
                    next_candidate,
                    &mut next_unvalidated_following_ivf_record,
                )?;
                if reference.valid_count() > 2 {
                    return Err(unsupported_at(
                        "inter_too_many_valid_references",
                        next_candidate.offset(),
                        missing_capability_message!("inter.reference_count valid_refs>2"),
                    ));
                }
                let (inter_frame, inter_core, frame_cdfs) = match sequence.general.bit_depth_idc {
                    BitDepthIdc::Eight => {
                        let (store, meta) = reference.build_store_eight(&frames)?;
                        let inter_state = inter::InterReferenceState {
                            store: &store,
                            ref_valid: meta.ref_valid,
                            ref_order_hint: meta.ref_order_hint,
                            ref_frame_width: meta.ref_frame_width,
                            ref_frame_height: meta.ref_frame_height,
                            ref_base_q_idx: meta.ref_base_q_idx,
                            ref_is_inter: meta.ref_is_inter,
                            ref_adapted: meta.ref_adapted,
                            lr_frame_filter_class_counts: meta.lr_frame_filter_class_counts,
                            ref_frame_cdfs: meta.ref_frame_cdfs,
                        };
                        let inter_core = inter::parse_validated_inter_frame_core(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                        )?;
                        if frame_is_output(&inter_core) {
                            let next_output_frame_count = checked_add(
                                DecodeLimitName::MaxOutputFrames,
                                scheduler.emitted.len() as u64,
                                1,
                            )?;
                            ensure_output_frame_count_limit(
                                options.limits(),
                                next_output_frame_count,
                            )?;
                        }
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained_frame_bytes,
                            &inter_core,
                            inter_envelope.offset,
                        )?;
                        let (frame, inter_core, frame_cdfs) = inter::decode_minimal_inter_frame(
                            plan,
                            next_candidate,
                            bytes,
                            inter_envelope,
                            inter_core,
                            &sequence,
                            options,
                            header,
                            &inter_state,
                            BitDepth::Eight,
                        )?;
                        (
                            MinimalRuntimeDecodedFrame::Eight(frame),
                            inter_core,
                            frame_cdfs,
                        )
                    }
                    BitDepthIdc::Ten => {
                        let (store, meta) = reference.build_store_ten(&frames)?;
                        let inter_state = inter::InterReferenceState {
                            store: &store,
                            ref_valid: meta.ref_valid,
                            ref_order_hint: meta.ref_order_hint,
                            ref_frame_width: meta.ref_frame_width,
                            ref_frame_height: meta.ref_frame_height,
                            ref_base_q_idx: meta.ref_base_q_idx,
                            ref_is_inter: meta.ref_is_inter,
                            ref_adapted: meta.ref_adapted,
                            lr_frame_filter_class_counts: meta.lr_frame_filter_class_counts,
                            ref_frame_cdfs: meta.ref_frame_cdfs,
                        };
                        let inter_core = inter::parse_validated_inter_frame_core(
                            inter_envelope,
                            &sequence,
                            &inter_state,
                        )?;
                        if frame_is_output(&inter_core) {
                            let next_output_frame_count = checked_add(
                                DecodeLimitName::MaxOutputFrames,
                                scheduler.emitted.len() as u64,
                                1,
                            )?;
                            ensure_output_frame_count_limit(
                                options.limits(),
                                next_output_frame_count,
                            )?;
                        }
                        ensure_retained_frame_byte_limits_for_core(
                            options.limits(),
                            retained_frame_bytes,
                            &inter_core,
                            inter_envelope.offset,
                        )?;
                        let (frame, inter_core, frame_cdfs) = inter::decode_minimal_inter_frame(
                            plan,
                            next_candidate,
                            bytes,
                            inter_envelope,
                            inter_core,
                            &sequence,
                            options,
                            header,
                            &inter_state,
                            BitDepth::Ten,
                        )?;
                        (
                            MinimalRuntimeDecodedFrame::Ten(frame),
                            inter_core,
                            frame_cdfs,
                        )
                    }
                };
                let inter_frame = MinimalRuntimeFrame {
                    frame: inter_frame,
                    frame_cdfs,
                    frame_rate_numerator: header.timebase_denominator,
                    frame_rate_denominator: header.timebase_numerator,
                };
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                let frame_index = frames.len();
                frames.push(inter_frame);
                retained_frame_bytes = next_retained_frame_bytes;
                let inter_update = frame_ref_update_from_core(
                    &inter_core,
                    inter_envelope.offset,
                    frames[frame_index].frame_cdfs.clone(),
                )?;
                let inter_hint = inter_core.order_hint_lsb.unwrap_or(0);
                let evicted = scheduler.on_refresh(inter_update.refresh_frame_flags);
                output_frame_bytes = charge_emitted_outputs(
                    options,
                    &frames,
                    &scheduler,
                    &evicted,
                    output_frame_bytes,
                )?;
                reference.update(frame_index, &inter_update);
                if inter_core.implicit_output_frame == Some(true) {
                    scheduler.hold(inter_update.refresh_frame_flags, frame_index, inter_hint);
                }
                if inter_core.immediate_output_frame == Some(true) {
                    let emitted = scheduler.on_immediate(frame_index, inter_hint);
                    output_frame_bytes = charge_emitted_outputs(
                        options,
                        &frames,
                        &scheduler,
                        &emitted,
                        output_frame_bytes,
                    )?;
                }
                if output_frame_limit_reached(options, scheduler.emitted.len()) {
                    break;
                }
            }
            _ => {
                return Err(unsupported_at(
                    "multiple_frames_unimplemented",
                    next_candidate.offset(),
                    missing_capability_message!("frame.sequence key_plus_inter"),
                ));
            }
        }
    }

    if !output_frame_limit_reached(options, scheduler.emitted.len()) {
        let flushed = scheduler.flush_all();
        output_frame_bytes =
            charge_emitted_outputs(options, &frames, &scheduler, &flushed, output_frame_bytes)?;
        let _ = output_frame_bytes;
    }
    let emitted = std::mem::take(&mut scheduler.emitted);
    let limited = match options.output_frame_limit() {
        Some(limit) => {
            let limit = usize::try_from(limit.get()).unwrap_or(usize::MAX);
            emitted.into_iter().take(limit).collect()
        }
        None => emitted,
    };
    select_output_frames(frames, limited)
}

/// Charges the output count/byte limits for newly emitted display frames.
fn charge_emitted_outputs(
    options: &DecodeOptions,
    frames: &[MinimalRuntimeFrame],
    scheduler: &OutputScheduler,
    newly: &[usize],
    mut output_frame_bytes: u64,
) -> Result<u64> {
    if newly.is_empty() {
        return Ok(output_frame_bytes);
    }
    ensure_output_frame_count_limit(options.limits(), scheduler.emitted.len() as u64)?;
    for &frame_index in newly {
        let frame = frames.get(frame_index).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "minimal runtime output ordering references a decoded frame that is unavailable",
            )
        })?;
        output_frame_bytes =
            ensure_output_frame_byte_limits(options.limits(), output_frame_bytes, frame)?;
    }
    Ok(output_frame_bytes)
}

fn output_frame_limit_reached(options: &DecodeOptions, output_frame_count: usize) -> bool {
    options
        .output_frame_limit()
        .is_some_and(|limit| output_frame_count as u64 >= limit.get())
}

fn frame_is_output(core: &FrameHeaderCore) -> bool {
    core.immediate_output_frame == Some(true) || core.implicit_output_frame == Some(true)
}

/// § 7.21 output scheduling: implicit-output frames are held in their
/// reference slots and released in `output_ordering` (order-hint) order —
/// flushed by an immediate-output frame (§ 7.21.6 with -1), by their slot
/// being refreshed (§ 7.23 → § 7.21.6 with the slot), by a successive-hint
/// chain (§ 7.21.3/§ 7.21.4), or by the end-of-stream flush (§ 7.21.5).
/// Single-layer streams only: `output_ordering(i)` reduces to the order hint.
struct OutputScheduler {
    pending: Vec<Option<(usize, u32)>>,
    emitted: Vec<usize>,
}

impl OutputScheduler {
    fn new(num_slots: usize) -> Self {
        Self {
            pending: vec![None; num_slots],
            emitted: Vec::new(),
        }
    }

    /// § 7.21.1 output process, deduplicated per decoded frame.
    fn emit(&mut self, frame_index: usize, newly: &mut Vec<usize>) {
        if !self.emitted.contains(&frame_index) {
            self.emitted.push(frame_index);
            newly.push(frame_index);
        }
        for slot in &mut self.pending {
            if slot.is_some_and(|(held, _)| held == frame_index) {
                *slot = None;
            }
        }
    }

    /// The § 7.21.6 leading loop: outputs held frames with a lower ordering,
    /// lowest first.
    fn flush_lower_than(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        loop {
            let next = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held < ordering)
                .min_by_key(|(_, held)| *held)
                .copied();
            let Some((frame_index, _)) = next else {
                return;
            };
            self.emit(frame_index, newly);
        }
    }

    /// § 7.21.3 / § 7.21.4 successive-hint outputs.
    fn output_successive(&mut self, ordering: u32, newly: &mut Vec<usize>) {
        let mut target = ordering.saturating_add(1);
        loop {
            let matches: Vec<usize> = self
                .pending
                .iter()
                .flatten()
                .filter(|(_, held)| *held == target)
                .map(|(frame_index, _)| *frame_index)
                .collect();
            if matches.is_empty() {
                return;
            }
            for frame_index in matches {
                self.emit(frame_index, newly);
            }
            target = target.saturating_add(1);
        }
    }

    /// § 7.21.6 with `refIdx == -1` (an immediate-output frame).
    fn on_immediate(&mut self, frame_index: usize, ordering: u32) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(ordering, &mut newly);
        self.emit(frame_index, &mut newly);
        self.output_successive(ordering, &mut newly);
        newly
    }

    /// § 7.23 slot refresh of a held eligible frame → § 7.21.6 with the slot.
    fn on_refresh(&mut self, refresh_frame_flags: u32) -> Vec<usize> {
        let mut newly = Vec::new();
        for slot in 0..self.pending.len() {
            if (refresh_frame_flags >> slot) & 1 == 0 {
                continue;
            }
            let Some((frame_index, ordering)) = self.pending[slot] else {
                continue;
            };
            self.flush_lower_than(ordering, &mut newly);
            self.emit(frame_index, &mut newly);
            self.output_successive(ordering, &mut newly);
        }
        newly
    }

    /// Marks an implicit-output frame as held in its refreshed slots.
    fn hold(&mut self, refresh_frame_flags: u32, frame_index: usize, ordering: u32) {
        for slot in 0..self.pending.len() {
            if (refresh_frame_flags >> slot) & 1 == 1 {
                self.pending[slot] = Some((frame_index, ordering));
            }
        }
    }

    /// § 7.21.5 end-of-stream flush, lowest ordering first.
    fn flush_all(&mut self) -> Vec<usize> {
        let mut newly = Vec::new();
        self.flush_lower_than(u32::MAX, &mut newly);
        newly
    }
}

fn select_output_frames(
    frames: Vec<MinimalRuntimeFrame>,
    output_frame_indices: Vec<usize>,
) -> Result<Vec<MinimalRuntimeFrame>> {
    let mut frames = frames.into_iter().map(Some).collect::<Vec<_>>();
    let mut outputs = Vec::with_capacity(output_frame_indices.len());
    for index in output_frame_indices {
        let output = frames.get_mut(index).and_then(Option::take).ok_or_else(|| {
            unsupported(
                "displayed_frame_index_unavailable",
                None,
                "minimal runtime output ordering references a decoded frame that is unavailable",
            )
        })?;
        outputs.push(output);
    }
    Ok(outputs)
}
fn following_inter_envelope<'a>(
    ivf: &'a ParsedIvfBitstream<'a>,
    candidate: &DecodePlannedObu,
    next_unvalidated_following_ivf_record: &mut usize,
) -> Result<ObuEnvelope<'a>> {
    for (ivf_frame_index, ivf_frame) in ivf.frames.iter().enumerate() {
        let Some(position) = ivf_frame
            .obus
            .iter()
            .position(|envelope| envelope.offset == candidate.offset())
        else {
            continue;
        };
        require_following_ivf_obu_order_through(
            ivf,
            next_unvalidated_following_ivf_record,
            ivf_frame_index,
        )?;
        let inter_envelope = ivf_frame.obus[position];
        require_obu_type(
            inter_envelope,
            ObuType::RegularTileGroup,
            "missing_inter_regular_tile_group",
        )?;
        if is_leading_record_regular_after_key(ivf_frame_index, position, ivf_frame.obus.as_slice())
        {
            return Ok(inter_envelope);
        }
        let Some(td_envelope) = position
            .checked_sub(1)
            .and_then(|previous| ivf_frame.obus.get(previous))
            .copied()
        else {
            return Err(unsupported_at(
                "missing_inter_temporal_delimiter",
                candidate.offset(),
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        };
        require_obu_type(
            td_envelope,
            ObuType::TemporalDelimiter,
            "missing_inter_temporal_delimiter",
        )?;
        return Ok(inter_envelope);
    }
    Err(unsupported_at(
        "missing_inter_ivf_obu",
        candidate.offset(),
        "the planned inter candidate offset was not found in the parsed IVF payloads",
    ))
}

fn is_leading_record_regular_after_key(
    ivf_frame_index: usize,
    position: usize,
    obus: &[ObuEnvelope<'_>],
) -> bool {
    ivf_frame_index == 0
        && position >= 3
        && require_minimal_obu_order(obus).is_ok()
        && obus
            .iter()
            .skip(3)
            .all(|envelope| envelope.header.obu_type == ObuType::RegularTileGroup)
}

fn require_following_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for (ivf_frame_index, frame) in ivf
        .frames
        .iter()
        .enumerate()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_following_ivf_record_obu_order(frame.obus.as_slice(), ivf_frame_index)?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
    Ok(())
}

fn require_following_ivf_record_obu_order(
    obus: &[ObuEnvelope<'_>],
    ivf_frame_index: usize,
) -> Result<()> {
    if ivf_frame_index == 0 {
        require_leading_ivf_obu_order(obus)
    } else {
        require_inter_obu_order(obus)
    }
}

fn require_leading_ivf_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    require_minimal_obu_order(obus)?;
    for envelope in obus.iter().skip(3) {
        require_obu_type(
            *envelope,
            ObuType::RegularTileGroup,
            "unexpected_leading_obu_after_key",
        )?;
    }
    Ok(())
}

fn require_inter_obu_order(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    for (index, envelope) in obus.iter().enumerate() {
        let expected = if index % 2 == 0 {
            ObuType::TemporalDelimiter
        } else {
            ObuType::RegularTileGroup
        };
        if envelope.header.obu_type != expected {
            return Err(unsupported_at(
                "unexpected_inter_obu_order",
                envelope.offset,
                missing_capability_message!("inter.ivf_frame_unit_order"),
            ));
        }
    }
    if !obus.len().is_multiple_of(2) {
        let offset = obus
            .last()
            .map_or(ByteOffset::new(0), |envelope| envelope.offset);
        return Err(unsupported_at(
            "unexpected_inter_obu_order",
            offset,
            missing_capability_message!("inter.ivf_frame_unit_order"),
        ));
    }
    Ok(())
}
fn ensure_multiframe_plan_shape(plan: &DecodeStreamPlan) -> Result<()> {
    let frame_count = plan.frame_candidate_count();
    if frame_count == 0 {
        return Err(unsupported(
            "unsupported_frame_candidate_count",
            None,
            "minimal tier requires at least one selected key frame candidate",
        ));
    }
    if plan.obu_count() >= 3 {
        Ok(())
    } else {
        Err(unsupported(
            "unexpected_planned_stream_shape",
            None,
            "minimal tier requires a leading [TD, SEQ, CLK] frame unit",
        ))
    }
}
fn require_multiframe_ivf<'a>(
    parsed: &'a ParsedBitstream<'a>,
) -> Result<(&'a ParsedIvfBitstream<'a>, IvfHeader)> {
    let ParsedBitstream::Ivf(ivf) = parsed else {
        return Err(unsupported(
            "non_ivf_input",
            None,
            missing_capability_message!("container.ivf"),
        ));
    };
    let Some(header) = ivf.header else {
        return Err(unsupported(
            "missing_ivf_header",
            None,
            "minimal tier requires a complete IVF header",
        ));
    };
    let parsed_frame_count = ivf.frames.len() as u64;
    let header_frame_count = u64::from(header.frame_count);
    let header_count_matches = header_frame_count == 0 || header_frame_count == parsed_frame_count;
    let all_frame_records_positive = ivf.frames.iter().all(|frame| frame.frame.size > 0);
    if header.fourcc != *b"AV02"
        || header.width == 0
        || header.height == 0
        || ivf.frames.is_empty()
        || !header_count_matches
        || !all_frame_records_positive
        || !supported_ivf_warnings(&ivf.warnings)
        || ivf.error.is_some()
    {
        return Err(unsupported(
            "unsupported_ivf_shape",
            None,
            missing_capability_message!("container.ivf_av02_frame_records"),
        ));
    }
    Ok((ivf, header))
}

fn supported_ivf_warnings(warnings: &[IvfWarning]) -> bool {
    warnings
        .iter()
        .all(|warning| matches!(warning, IvfWarning::TrailingPartialFrameHeader { .. }))
}

fn ensure_output_frame_count_limit(
    limits: crate::DecodeLimits,
    output_frame_count: u64,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxOutputFrames, output_frame_count)?;
    Ok(())
}

fn ensure_retained_frame_byte_limits(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame: &MinimalRuntimeFrame,
) -> Result<u64> {
    let frame_bytes = retained_decoded_frame_bytes(frame)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

fn ensure_retained_frame_byte_limits_for_core(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<u64> {
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_retained_limit",
            offset,
            "minimal runtime requires parsed frame dimensions before charging retained decoded-frame bytes",
        )
    })?;
    let frame_bytes = decoded_frame_byte_budget(frame_size, bytes_per_sample(BitDepth::Eight))
        .map(|budget| budget.decoded_bytes)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

fn ensure_retained_frame_byte_limits_for_bytes(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame_bytes: u64,
) -> Result<u64> {
    let next_retained_frame_bytes = checked_add(
        DecodeLimitName::MaxReferenceStoreBytes,
        retained_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(
        DecodeLimitName::MaxReferenceStoreBytes,
        next_retained_frame_bytes,
    )?;
    Ok(next_retained_frame_bytes)
}

fn retained_decoded_frame_bytes(frame: &MinimalRuntimeFrame) -> Result<u64> {
    Ok(frame.byte_len()? as u64)
}

fn ensure_output_frame_byte_limits(
    limits: crate::DecodeLimits,
    output_frame_bytes: u64,
    frame: &MinimalRuntimeFrame,
) -> Result<u64> {
    let frame_bytes = frame.byte_len()? as u64;
    let next_output_frame_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        output_frame_bytes,
        frame_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, next_output_frame_bytes)?;
    Ok(next_output_frame_bytes)
}

fn verify_flat_minimal_tile_trace(
    tile: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
) -> Result<MinimalRuntimeReconstructionTrace> {
    let tile_offset = tile.tile_byte_span().start;
    let frontier = match crate::tile_payload::plan_minimal_runtime_block_symbol_frontier(
        tile, sequence, core, limits,
    ) {
        Ok(frontier) => frontier,
        Err(error) => {
            return Err(decode_minimal_block_symbol_frontier_error(
                error,
                tile_offset,
            ));
        }
    };
    validate_minimal_trace_summary(frontier.summary(), tile)?;
    Ok(frontier.reconstruction_trace())
}

fn decode_minimal_block_symbol_frontier_error(
    error: MinimalRuntimeBlockSymbolFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        MinimalRuntimeBlockSymbolFrontierError::Partition(error) => {
            decode_minimal_partition_frontier_error(error, offset)
        }
        MinimalRuntimeBlockSymbolFrontierError::Block(error) => {
            decode_minimal_block_symbol_error(error, offset)
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn decode_minimal_block_symbol_error(
    error: MinimalBlockSymbolTraceError,
    offset: ByteOffset,
) -> DecodeError {
    let (reason, message) = match error {
        MinimalBlockSymbolTraceError::SymbolRead { .. } => (
            "minimal_tile_symbol_parse",
            missing_capability_message!("tile.symbol_stream flat_minimal"),
        ),
        MinimalBlockSymbolTraceError::UnexpectedSymbol { reason, .. } => (
            reason,
            missing_capability_message!("tile.symbol_values flat_minimal"),
        ),
        MinimalBlockSymbolTraceError::UnsupportedYMode { .. } => (
            "minimal_tile_y_mode_reconstruction",
            missing_capability_message!("intra.y_mode non_directional_flat"),
        ),
        MinimalBlockSymbolTraceError::InvalidCoeffContextRange { .. }
        | MinimalBlockSymbolTraceError::CoeffContextDimensionOverflow { .. }
        | MinimalBlockSymbolTraceError::CoeffContextState { .. }
        | MinimalBlockSymbolTraceError::CoeffLoopContext { .. }
        | MinimalBlockSymbolTraceError::CoeffFrameEntry { .. } => (
            "minimal_tile_coeff_context_state",
            missing_capability_message!("residual.coeff_context flat_minimal"),
        ),
        MinimalBlockSymbolTraceError::CoeffTxGeometryDimensionOverflow { .. }
        | MinimalBlockSymbolTraceError::UnsupportedCoeffTxGeometry { .. }
        | MinimalBlockSymbolTraceError::InvalidCoeffTxTableValue { .. } => (
            "minimal_tile_coeff_tx_size_geometry",
            missing_capability_message!("residual.tx_geometry"),
        ),
        MinimalBlockSymbolTraceError::ExitSymbol { .. } => (
            "minimal_tile_exit_symbol",
            missing_capability_message!("tile.exit_symbol §8.2.4"),
        ),
    };
    unsupported_at(reason, offset, message)
}

#[allow(clippy::needless_pass_by_value)]
fn decode_minimal_partition_frontier_error(
    error: MinimalRuntimePartitionFrontierError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        MinimalRuntimePartitionFrontierError::Limit(source)
        | MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(
            source,
        )) => DecodeError::Limit { source },
        MinimalRuntimePartitionFrontierError::MissingFact { .. }
        | MinimalRuntimePartitionFrontierError::MiSizeState(_)
        | MinimalRuntimePartitionFrontierError::IntraJointModeState(_)
        | MinimalRuntimePartitionFrontierError::UsesMrlsState(_)
        | MinimalRuntimePartitionFrontierError::FscModeState(_)
        | MinimalRuntimePartitionFrontierError::UvCflState(_)
        | MinimalRuntimePartitionFrontierError::LumaPaletteState(_)
        | MinimalRuntimePartitionFrontierError::Traversal(_)
        | MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => unsupported_at(
            "minimal_tile_partition_frontier",
            offset,
            missing_capability_message!("tile.partition §5.20.3.1"),
        ),
    }
}

fn validate_minimal_trace_summary(
    summary: SymbolDecoderSummary,
    tile: &crate::tile_payload::DecodeTileWorkUnit<'_>,
) -> Result<()> {
    if summary.symbol_count == MINIMAL_TRACE_SYMBOLS
        && summary.trailing_bit_position.get() == MINIMAL_TRACE_TRAILING_BIT_POSITION
        && summary.padding_end_position.get() == MINIMAL_TRACE_PADDING_END_POSITION
        && summary.consumed_bits.get() == MINIMAL_TRACE_PADDING_END_POSITION
    {
        Ok(())
    } else {
        Err(unsupported_at(
            "minimal_tile_trace_summary",
            tile.tile_byte_span().start,
            missing_capability_message!("tile.trace_summary flat_minimal"),
        ))
    }
}

fn require_minimal_obu_order<'a>(obus: &'a [ObuEnvelope<'a>]) -> Result<[ObuEnvelope<'a>; 3]> {
    match obus {
        [td, sequence, frame, ..] => Ok([*td, *sequence, *frame]),
        _ => Err(unsupported(
            "unexpected_obu_order",
            None,
            "minimal tier requires a leading temporal delimiter, sequence header, and closed-loop-key OBU",
        )),
    }
}

fn reject_extra_leading_key_payload_obus(obus: &[ObuEnvelope<'_>]) -> Result<()> {
    let Some(extra) = obus.get(3) else {
        return Ok(());
    };
    Err(unsupported_at(
        "unexpected_leading_obu_after_key",
        extra.offset,
        "minimal tier does not decode extra OBUs after the leading closed-loop-key OBU",
    ))
}

fn require_obu_type(
    envelope: ObuEnvelope<'_>,
    expected: ObuType,
    reason: &'static str,
) -> Result<()> {
    if envelope.header.obu_type == expected {
        Ok(())
    } else {
        Err(unsupported_at(
            reason,
            envelope.offset,
            missing_capability_message!("obu.order minimal_frame_unit"),
        ))
    }
}

fn parse_sequence(envelope: ObuEnvelope<'_>) -> Result<SequenceHeader> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    parse_sequence_header(&mut reader).map_err(|_| {
        unsupported_at(
            "sequence_header_parse",
            envelope.offset,
            "minimal tier requires a fully parseable sequence header",
        )
    })
}

fn validate_sequence(sequence: &SequenceHeader, offset: ByteOffset) -> Result<()> {
    let general = &sequence.general;
    if !sequence.is_fully_parsed() {
        return Err(unsupported_at(
            "sequence_header_not_fully_parsed",
            offset,
            "minimal tier requires a fully parsed sequence header",
        ));
    }
    if general.seq_profile_idc.get() != 0 {
        return Err(unsupported_at(
            "unsupported_profile",
            offset,
            "minimal tier requires seq_profile_idc == 0",
        ));
    }
    if general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
        return Err(unsupported_at(
            "unsupported_chroma_format",
            offset,
            "minimal tier requires YUV 4:2:0 output",
        ));
    }
    if general.max_tlayer_id.get() != 0 || general.max_mlayer_id.get() != 0 {
        return Err(unsupported_at(
            "non_base_layer_sequence",
            offset,
            "minimal tier requires a single base temporal and embedded layer",
        ));
    }
    if general.seq_cropping_window_present_flag {
        return Err(unsupported_at(
            "crop_window_present",
            offset,
            "minimal tier does not support sequence crop windows",
        ));
    }
    if sequence.intra.is_none() {
        return Err(unsupported_at(
            "missing_sequence_intra_config",
            offset,
            "minimal tier requires a fully parsed sequence intra config",
        ));
    }
    Ok(())
}

fn ensure_sequence_chroma_tools_before_tile_decode(
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    let intra = sequence.intra.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_sequence_intra_config",
            offset,
            "minimal tier requires a fully parsed sequence intra config",
        )
    })?;
    for (enabled, reason, message) in [
        (
            intra.enable_cfl_intra,
            "unsupported_cfl_intra",
            missing_capability_message!("intra.chroma.cfl §5.20.5.6"),
        ),
        (
            intra.enable_mhccp,
            "unsupported_mhccp",
            missing_capability_message!("intra.chroma.mhccp §5.20.5.6"),
        ),
    ] {
        if enabled {
            return Err(unsupported_feature_at(
                reason,
                offset,
                message,
                AC0EJ3_CHROMA_MATRIX_ROW,
                AC0EJ3_CHROMA_FEATURE_ID,
                "5.20.5.6",
            ));
        }
    }
    Ok(())
}
#[allow(clippy::unnecessary_wraps)]
fn ensure_runtime_storage_bit_depth(sequence: &SequenceHeader, _offset: ByteOffset) -> Result<()> {
    match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight | BitDepthIdc::Ten => Ok(()),
    }
}

fn parse_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        unsupported_at(
            "tile_group_prefix_parse",
            envelope.offset,
            "minimal tier requires a parseable first tile-group prefix",
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(unsupported_at(
            "non_first_tile_group",
            envelope.offset,
            "minimal tier requires the frame header in the first tile group",
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu: true,
        active_sequence: Some(sequence),
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "frame_header_parse",
            envelope.offset,
            "minimal tier requires a fully parseable closed-loop-key frame header",
        )
    })
}
fn frame_ref_update_from_core(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_cdfs: FrameCdfSubset,
) -> Result<reference_buffer::FrameRefUpdate> {
    let refresh_frame_flags = core.refresh_frame_flags.ok_or_else(|| {
        unsupported_at(
            "missing_refresh_frame_flags",
            offset,
            "minimal multi-frame decode requires a parsed refresh_frame_flags for the §7.23 update",
        )
    })?;
    let order_hint = core.order_hint_lsb.unwrap_or(0);
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_ref_update",
            offset,
            "minimal multi-frame decode requires a parsed frame size for the §7.23 update",
        )
    })?;
    let base_q_idx = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            unsupported_at(
                "missing_base_q_for_ref_update",
                offset,
                "minimal multi-frame decode requires a parsed base_q_idx for the §7.23 update",
            )
        })?;
    let is_inter = !core.is_key_frame;
    let adapted = core.disable_cdf_update != Some(true);
    Ok(reference_buffer::FrameRefUpdate {
        refresh_frame_flags,
        order_hint,
        width: frame_size.width,
        height: frame_size.height,
        base_q_idx,
        is_key_or_switch: core.is_key_frame,
        is_inter,
        adapted,
        frame_cdfs,
        lr_frame_filter_class_counts: lr_frame_filter_class_counts(core),
    })
}

fn lr_frame_filter_class_counts(core: &FrameHeaderCore) -> [u8; 3] {
    let mut counts = [0u8; 3];
    let Some(lr) = core.lr_params.as_ref() else {
        return counts;
    };
    for (plane, params) in lr.planes.iter().enumerate().take(3) {
        if !params.frame_filters_on {
            continue;
        }
        let classes = params
            .frame_filter_bank
            .as_ref()
            .map(|bank| bank.classes.len())
            .or_else(|| params.num_filter_classes.map(usize::from))
            .unwrap_or(1);
        counts[plane] = u8::try_from(classes).unwrap_or(u8::MAX);
    }
    counts
}

fn validate_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    ensure_intra_header_complete(core, offset)?;
    if !core.cur_mfh_id.is_zero()
        || core.show_existing_frame != Some(false)
        || core.frame_is_intra != Some(true)
        || !core.is_key_frame
        || core.immediate_output_frame != Some(true)
        || core.implicit_output_frame != Some(false)
    {
        return Err(unsupported_at(
            "unsupported_frame_control",
            offset,
            "minimal tier requires one immediate-output intra key frame without MFH indirection",
        ));
    }
    match core.frame_size {
        Some(FrameSize {
            width: MINIMAL_WIDTH,
            height: MINIMAL_HEIGHT,
            ..
        }) => {}
        _ => {
            return Err(unsupported_at(
                "unsupported_frame_size",
                offset,
                missing_capability_message!("frame.size width!=64 || height!=64"),
            ));
        }
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(unsupported_at(
            "missing_tile_info",
            offset,
            "minimal tier requires parsed one-tile frame layout",
        ));
    };
    if tile_info.tile_cols != 1 || tile_info.tile_rows != 1 {
        return Err(unsupported_at(
            "multi_tile_frame",
            offset,
            "minimal tier supports one tile",
        ));
    }
    if core
        .quantization_params
        .is_none_or(|quant| quant.base_q_idx != 255)
        || core
            .segmentation_params
            .as_ref()
            .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .delta_q_params
            .is_none_or(|delta| delta.delta_q_present)
        || core
            .lossless_info
            .as_ref()
            .is_none_or(|lossless| lossless.coded_lossless)
        || core
            .deblocking_filter_params
            .is_none_or(|filter| filter.apply_deblocking_filter != [false; 4])
        || core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable)
        || core
            .cdef_params
            .as_ref()
            .is_none_or(|cdef| cdef.cdef_frame_enable)
        || core.lr_params.as_ref().is_none_or(|lr| lr.uses_lr)
        || core
            .ccso_params
            .as_ref()
            .is_none_or(|ccso| ccso.ccso_frame_flag.is_some() || !ccso.planes.is_empty())
        || core
            .intra_tail
            .is_none_or(|tail| tail.film_grain.apply_grain)
    {
        return Err(unsupported_at(
            "unsupported_frame_tools",
            offset,
            missing_capability_message!("frame.tools no_filters_no_grain"),
        ));
    }
    Ok(())
}

fn ensure_intra_header_complete(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(incomplete_intra_header_error(core.status, offset));
    }
    Ok(())
}

fn incomplete_intra_header_error(
    status: FrameHeaderParseStatus,
    offset: ByteOffset,
) -> DecodeError {
    match status {
        FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { .. } => unsupported_feature_at(
            "unsupported_wienerns_filter",
            offset,
            missing_capability_message!("filters.wiener_ns read_wienerns_filter §5.18.7.11"),
            AC0EJ3_WIENERNS_MATRIX_ROW,
            AC0EJ3_WIENERNS_FEATURE_ID,
            "5.18.7.11",
        ),
        _ => unsupported_at(
            "incomplete_frame_header",
            offset,
            "minimal tier requires a complete intra frame header",
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test_support {
    use splot_recon::{
        CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize,
    };

    use super::BitDepth;

    pub(super) fn yuv420_workspace(
        width: usize,
        height: usize,
        fill: u8,
    ) -> CurrentFrameWorkspace<u8> {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(width, height).unwrap(),
            PlaneRect::new(0, 0, width, height).unwrap(),
        )
        .unwrap();
        CurrentFrameWorkspace::<u8>::new(info, fill).unwrap()
    }
}

mod block_context;
mod capability;
mod ccso;
mod cdef;
mod deblock;
mod general_intra;
mod inter;
mod intra_prediction;
mod limits;
mod reference_buffer;
mod residual_pipeline;
mod wienerns_lr;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_tests;

#[cfg(test)]
mod wienerns_lr_recon_tests;
#[derive(Clone, Copy)]
enum TileFactsKind {
    Intra,
    Inter,
}
#[allow(clippy::too_many_arguments)]
fn derive_tile_plan_with<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
    kind: TileFactsKind,
    initial_cdfs: Option<FrameCdfSubset>,
) -> Result<crate::tile_payload::DecodeTilePayloadPlan<'a>> {
    let tq = sequence.transform_quant_entropy.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_tq_entropy_config",
            envelope.offset,
            "minimal tier requires sequence transform/quant/entropy config",
        )
    })?;
    let coeff = FrameCandidateCoeffFacts::from_tq(tq);
    let facts = match kind {
        TileFactsKind::Intra => FrameCandidateTileFacts::from_frame_core(core, coeff),
        TileFactsKind::Inter => FrameCandidateTileFacts::from_inter_frame_core(core, coeff),
    }
    .map_err(decode_tile_boundary_error)?;
    let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
    let mut input = FrameCandidateTileBoundaryInput::new(
        plan,
        candidate,
        bytes,
        envelope,
        TileGroupPositionFacts::new(true, true),
        facts,
        cdf,
        options.limits(),
    );
    if let Some(cdfs) = initial_cdfs {
        input = input.with_initial_cdfs(cdfs);
    }
    crate::tile_payload::plan_derived_tile_payload_boundary(&input)
        .map_err(decode_tile_boundary_error)
}

fn derive_tile_plan<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
) -> Result<crate::tile_payload::DecodeTilePayloadPlan<'a>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        envelope,
        sequence,
        core,
        options,
        TileFactsKind::Intra,
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn derive_inter_tile_plan<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: &DecodeOptions,
    initial_cdfs: FrameCdfSubset,
) -> Result<crate::tile_payload::DecodeTilePayloadPlan<'a>> {
    derive_tile_plan_with(
        plan,
        candidate,
        bytes,
        envelope,
        sequence,
        core,
        options,
        TileFactsKind::Inter,
        Some(initial_cdfs),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn decode_tile_boundary_error(error: FrameCandidateTileBoundaryError) -> DecodeError {
    match error {
        FrameCandidateTileBoundaryError::Limit(source) => DecodeError::Limit { source },
        FrameCandidateTileBoundaryError::Malformed(malformed) => unsupported(
            malformed_tile_boundary_reason(malformed),
            None,
            "minimal tier could not derive a source-backed tile payload boundary",
        ),
        FrameCandidateTileBoundaryError::MissingFact { .. } => unsupported(
            "missing_tile_fact",
            None,
            "minimal tier requires complete parser-derived tile facts",
        ),
        FrameCandidateTileBoundaryError::Unsupported { .. }
        | FrameCandidateTileBoundaryError::Boundary(_) => unsupported(
            "unsupported_tile_boundary",
            None,
            "minimal tier requires a single source-backed tile work unit",
        ),
    }
}

fn malformed_tile_boundary_reason(
    malformed: crate::tile_payload::FrameCandidateTileMalformed,
) -> &'static str {
    match malformed {
        crate::tile_payload::FrameCandidateTileMalformed::CandidateNotInPlan => {
            "candidate_not_in_plan"
        }
        crate::tile_payload::FrameCandidateTileMalformed::PlanSourceKindMismatch { .. } => {
            "plan_source_kind_mismatch"
        }
        crate::tile_payload::FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field } => {
            match field {
                "payload_source" => "payload_source_mismatch",
                "offset" => "candidate_offset_mismatch",
                "size" => "candidate_size_mismatch",
                "header" => "candidate_header_mismatch",
                "payload_len" => "candidate_payload_len_mismatch",
                "payload" => "candidate_payload_mismatch",
                "input_len_bytes" => "input_len_mismatch",
                "ivf_frame" => "ivf_frame_mismatch",
                _ => "candidate_envelope_mismatch",
            }
        }
        crate::tile_payload::FrameCandidateTileMalformed::ObuSizeSmallerThanHeader { .. } => {
            "obu_size_smaller_than_header"
        }
        crate::tile_payload::FrameCandidateTileMalformed::SourceRangeOutOfBounds { .. } => {
            "source_range_out_of_bounds"
        }
        crate::tile_payload::FrameCandidateTileMalformed::TileGroupStructureIncomplete => {
            "tile_group_structure_incomplete"
        }
        crate::tile_payload::FrameCandidateTileMalformed::TileGroupStructureInvalid => {
            "tile_group_structure_invalid"
        }
        crate::tile_payload::FrameCandidateTileMalformed::TileGroupPayloadRangeInvalid => {
            "tile_group_payload_range_invalid"
        }
        crate::tile_payload::FrameCandidateTileMalformed::TileGroupRangeInvalid { .. } => {
            "tile_group_range_invalid"
        }
    }
}
fn bytes_per_sample(bit_depth: BitDepth) -> u64 {
    match bit_depth {
        BitDepth::Eight => 1,
        BitDepth::Ten => 2,
    }
}

fn ensure_runtime_limits(
    limits: crate::DecodeLimits,
    width: u32,
    height: u32,
    tile_payload_bytes: u64,
    bit_depth: BitDepth,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
    limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
    let budget =
        decoded_frame_byte_budget(FrameSize::new(width, height), bytes_per_sample(bit_depth))?;
    limits.ensure(DecodeLimitName::MaxLumaSamplesPerFrame, budget.luma_samples)?;
    limits.ensure(DecodeLimitName::MaxDecodedFrameBytes, budget.decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, budget.decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxTileCount, 1)?;
    limits.ensure(DecodeLimitName::MaxTilePayloadBytes, tile_payload_bytes)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, budget.luma_samples)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, budget.chroma_samples)?;
    Ok(())
}

fn unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            MINIMAL_INTRA_HASH_TIER_ID,
            MATRIX_ROW,
            FEATURE_ID,
            SPEC_SECTION,
            message,
            REMEDIATION,
            byte_offset,
        )),
    }
}

fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
) -> DecodeError {
    unsupported(reason, Some(byte_offset), message)
}

fn unsupported_feature_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    matrix_row: &'static str,
    feature_id: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            MINIMAL_INTRA_HASH_TIER_ID,
            matrix_row,
            feature_id,
            spec_section,
            message,
            REMEDIATION,
            Some(byte_offset),
        )),
    }
}
