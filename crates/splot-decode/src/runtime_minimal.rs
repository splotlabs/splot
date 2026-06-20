// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared minimal-tier runtime implementation.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameHeaderParseStatus,
    FrameReferenceStateView, FrameSize, TxMode, parse_frame_header_core,
};
use splot_core::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, SequenceHeader, parse_sequence_header,
};
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream, parse_bitstream_partial};
use splot_core::symbol::SymbolDecoderSummary;
use splot_core::types::ObuType;
use splot_recon::{DecodedFrame, PlaneId};

use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, GeneralIntraBlockModeError,
    GeneralIntraResidualError, MinimalBlockSymbolTraceError,
    MinimalRuntimeBlockSymbolFrontierError, MinimalRuntimePartitionFrontierError,
    MinimalRuntimeReconstructionTrace, TileGroupPositionFacts, TilePartitionTraversalError,
};
use crate::{
    DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeOptions, DecodePlannedObu,
    DecodeStreamPlan,
};

/// Stable id for the first supported runtime decode tier.
pub const MINIMAL_INTRA_HASH_TIER_ID: &str = "minimal-intra-8bit420-hash-v1";

const FEATURE_ID: &str = "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS";
const MATRIX_ROW: &str = "minimal-decode-tier-contract";
const SPEC_SECTION: &str = "7.1";
const REMEDIATION: &str = "Use a stream inside minimal-intra-8bit420-hash-v1 or wait for the referenced decoder support row.";
const MINIMAL_WIDTH: u32 = 64;
const MINIMAL_HEIGHT: u32 = 64;
const MINIMAL_TRACE_SYMBOLS: u64 = 6;
const MINIMAL_TRACE_TRAILING_BIT_POSITION: u64 = 14;
const MINIMAL_TRACE_PADDING_END_POSITION: u64 = 16;
/// `base_q_idx` of the committed frozen minimal-tier fixture; frames with this
/// quantizer stay on the frozen hash-contract path, all others route to the
/// general intra frontier.
const FROZEN_MINIMAL_BASE_Q_IDX: u32 = 255;

const GENERAL_INTRA_FEATURE_ID: &str = "DECODE-GENERAL-INTRA-FRAME-FRONTIER";
const GENERAL_INTRA_MATRIX_ROW: &str = "general-intra-frame-frontier";
const GENERAL_INTRA_TIER_ID: &str = "general-intra-8bit420-frontier-v1";
const GENERAL_INTRA_TILE_SPEC_SECTION: &str = "5.20.1";
const GENERAL_INTRA_PARTITION_SPEC_SECTION: &str = "5.20.3.1";
const GENERAL_INTRA_MODE_SPEC_SECTION: &str = "5.20.5.3";
const GENERAL_INTRA_RESIDUAL_SPEC_SECTION: &str = "5.20.7.27";
const GENERAL_INTRA_REMEDIATION: &str = "General intra coefficient and reconstruction decode is not yet implemented; track DECODE-GENERAL-INTRA-FRAME-FRONTIER.";
/// AV2 § 5.4.8 `DELTA_DCQUANT_MIN` with `DELTA_DCQUANT_BITS == 5`: the bias added
/// to a raw `base_*_delta_q` field when deriving `Base*DeltaQ` (= -23). A raw
/// field of `-DELTA_DCQUANT_MIN` therefore resolves to a zero base offset.
const GENERAL_INTRA_DELTA_DCQUANT_MIN: i32 = (1 << 3) - (1 << 5) + 1;

pub(crate) struct MinimalRuntimeFrame {
    pub(crate) frame: DecodedFrame<u8>,
    pub(crate) frame_rate_numerator: u32,
    pub(crate) frame_rate_denominator: u32,
}

pub(crate) fn decode_minimal_frame_from_plan(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<MinimalRuntimeFrame> {
    decode_minimal_frame_from_plan_with_ivf_preflight(bytes, options, plan, |_| Ok(()))
}

pub(crate) fn decode_minimal_frame_from_plan_with_ivf_preflight(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(IvfHeader) -> Result<()>,
) -> Result<MinimalRuntimeFrame> {
    ensure_minimal_plan_shape(plan)?;
    let parsed = parse_bitstream_partial(bytes);
    let (ivf, header) = require_minimal_ivf(&parsed)?;
    preflight(header)?;
    let frame = &ivf.frames[0];
    let [td_envelope, sequence_envelope, frame_envelope] =
        require_minimal_obu_order(frame.obus.as_slice())?;
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
        frame_envelope,
        ObuType::ClosedLoopKey,
        "missing_closed_loop_key",
    )?;

    let sequence = parse_sequence(sequence_envelope)?;
    validate_sequence(&sequence, sequence_envelope.offset)?;

    let candidate = single_plan_candidate(plan)?;
    let core = parse_frame_core(frame_envelope, &sequence)?;
    if route_general_minimal_intra(&sequence, &core) {
        return decode_general_minimal_intra_frame(
            plan,
            candidate,
            bytes,
            frame_envelope,
            &sequence,
            &core,
            options,
            header,
        );
    }
    validate_frame_core(&core, frame_envelope.offset)?;

    let mut tile_plan = derive_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        &sequence,
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
                "minimal runtime hash support requires exactly one traced tile work unit",
            ));
        }
    };
    let reconstruction_trace =
        verify_flat_minimal_tile_trace(tile, &sequence, &core, options.limits())?;
    let tile_size = tile.tile_size();

    let limits = options.limits();
    ensure_runtime_limits(limits, MINIMAL_WIDTH, MINIMAL_HEIGHT, tile_size)?;
    let frame =
        crate::runtime_minimal_recon::reconstruct_minimal_traced_frame(reconstruction_trace)?;

    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

fn ensure_minimal_plan_shape(plan: &DecodeStreamPlan) -> Result<()> {
    if plan.source_warnings().is_empty()
        && plan.obu_count() == 3
        && plan.frame_candidate_count() == 1
    {
        Ok(())
    } else {
        Err(unsupported(
            "unexpected_planned_stream_shape",
            None,
            "minimal tier requires exactly three planned OBUs, one frame candidate, and no source warnings",
        ))
    }
}

fn require_minimal_ivf<'a>(
    parsed: &'a ParsedBitstream<'a>,
) -> Result<(&'a ParsedIvfBitstream<'a>, IvfHeader)> {
    let ParsedBitstream::Ivf(ivf) = parsed else {
        return Err(unsupported(
            "non_ivf_input",
            None,
            "minimal runtime hash support currently accepts only the committed IVF fixture shape",
        ));
    };
    let Some(header) = ivf.header else {
        return Err(unsupported(
            "missing_ivf_header",
            None,
            "minimal tier requires a complete IVF header",
        ));
    };
    if header.fourcc != *b"AV02"
        || header.width != MINIMAL_WIDTH as u16
        || header.height != MINIMAL_HEIGHT as u16
        || header.frame_count != 1
        || ivf.frames.len() != 1
        || !ivf.warnings.is_empty()
        || ivf.error.is_some()
    {
        return Err(unsupported(
            "unsupported_ivf_shape",
            None,
            "minimal tier requires one 64x64 AV02 IVF frame with no container warnings",
        ));
    }
    Ok((ivf, header))
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

fn decode_minimal_block_symbol_error(
    error: MinimalBlockSymbolTraceError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        MinimalBlockSymbolTraceError::SymbolRead { .. } => unsupported_at(
            "minimal_tile_symbol_parse",
            offset,
            "minimal runtime hash support requires the traced flat tile symbol stream",
        ),
        MinimalBlockSymbolTraceError::UnexpectedSymbol { reason, .. } => unsupported_at(
            reason,
            offset,
            "minimal runtime hash support only accepts the traced flat tile symbol values",
        ),
        MinimalBlockSymbolTraceError::UnsupportedYMode { .. } => unsupported_at(
            "minimal_tile_y_mode_reconstruction",
            offset,
            "minimal runtime hash support only reconstructs the traced flat tile non-directional YMode subset",
        ),
        MinimalBlockSymbolTraceError::InvalidCoeffContextRange { .. }
        | MinimalBlockSymbolTraceError::CoeffContextDimensionOverflow { .. }
        | MinimalBlockSymbolTraceError::CoeffContextState { .. }
        | MinimalBlockSymbolTraceError::CoeffLoopContext { .. }
        | MinimalBlockSymbolTraceError::CoeffFrameEntry { .. } => unsupported_at(
            "minimal_tile_coeff_context_state",
            offset,
            "minimal runtime hash support requires the traced flat tile coefficient context state",
        ),
        MinimalBlockSymbolTraceError::CoeffTxGeometryDimensionOverflow { .. }
        | MinimalBlockSymbolTraceError::UnsupportedCoeffTxGeometry { .. }
        | MinimalBlockSymbolTraceError::InvalidCoeffTxTableValue { .. } => unsupported_at(
            "minimal_tile_coeff_tx_size_geometry",
            offset,
            "minimal runtime hash support requires traced coefficient transform geometry to map to generated AV2 transform-size tables",
        ),
        MinimalBlockSymbolTraceError::ExitSymbol { .. } => unsupported_at(
            "minimal_tile_exit_symbol",
            offset,
            "minimal runtime hash support requires the traced flat tile payload to satisfy §8.2.4 exit_symbol()",
        ),
    }
}

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
        | MinimalRuntimePartitionFrontierError::Traversal(_)
        | MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => unsupported_at(
            "minimal_tile_partition_frontier",
            offset,
            "minimal runtime hash support requires the traced root AV2 5.20.3.1 partition frontier before block syntax",
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
            "minimal runtime hash support requires the traced flat tile symbol count and padding boundary",
        ))
    }
}

fn require_minimal_obu_order<'a>(obus: &'a [ObuEnvelope<'a>]) -> Result<[ObuEnvelope<'a>; 3]> {
    match obus {
        [td, sequence, frame] => Ok([*td, *sequence, *frame]),
        _ => Err(unsupported(
            "unexpected_obu_order",
            None,
            "minimal tier requires temporal delimiter, sequence header, and one closed-loop-key OBU",
        )),
    }
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
            "minimal tier OBU order does not match the traced fixture shape",
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
            "minimal tier requires 8-bit 4:2:0 output",
        ));
    }
    if general.bit_depth_idc != BitDepthIdc::Eight {
        return Err(unsupported_at(
            "unsupported_bit_depth",
            offset,
            "minimal tier requires 8-bit decoded samples",
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
    if !general.single_picture_header_flag {
        return Err(unsupported_at(
            "non_single_picture_sequence",
            offset,
            "minimal runtime hash support is limited to the traced single-picture sequence shape",
        ));
    }
    let intra = sequence.intra.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_sequence_intra_config",
            offset,
            "minimal tier requires a fully parsed sequence intra config",
        )
    })?;
    if intra.enable_cfl_intra {
        return Err(unsupported_at(
            "unsupported_cfl_intra",
            offset,
            "minimal tier rejects CFL intra syntax before traced UV-mode hash verification",
        ));
    }
    if intra.enable_mhccp {
        return Err(unsupported_at(
            "unsupported_mhccp",
            offset,
            "minimal tier rejects MHCCP syntax before traced UV-mode hash verification",
        ));
    }
    Ok(())
}

fn single_plan_candidate(plan: &DecodeStreamPlan) -> Result<&DecodePlannedObu> {
    let mut candidates = plan.frame_candidates();
    let Some(candidate) = candidates.next() else {
        return Err(unsupported(
            "missing_frame_candidate",
            None,
            "minimal tier requires one selected closed-loop-key frame candidate",
        ));
    };
    if candidates.next().is_some() {
        return Err(unsupported(
            "multiple_frame_candidates",
            None,
            "minimal tier supports exactly one selected output frame",
        ));
    }
    Ok(candidate)
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

fn validate_frame_core(core: &FrameHeaderCore, offset: ByteOffset) -> Result<()> {
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(unsupported_at(
            "incomplete_frame_header",
            offset,
            "minimal tier requires a complete intra frame header",
        ));
    }
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
                "minimal runtime hash support currently accepts only the traced 64x64 frame size",
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
            "minimal runtime hash support requires the traced no-tool, no-filter, no-grain frame header",
        ));
    }
    Ok(())
}

/// Routes a parsed frame to the general intra decode frontier.
///
/// The frozen minimal hash tier owns exactly the committed
/// `base_q_idx == 255` fixture (see [`validate_frame_core`]); any other
/// general minimal-tool intra key frame routes to
/// [`decode_general_minimal_intra_frame`]. Frames that are not minimal-tool
/// intra (segmentation, quant matrices, delta-Q, in-loop filters, CCSO, GDF,
/// film grain, screen-content/palette, DIP, or SDP enabled) fall through to the
/// frozen gate so its precise diagnostics are preserved.
///
/// `enable_dip` (§ 5.20.5.3 `dip_mode_info`) and `enable_sdp` (luma-only key
/// partitions that omit the `uv_mode` read) are checked here at the sequence
/// level — not in [`validate_sequence`] — because the frozen hash fixture is
/// itself an `enable_sdp` stream whose hand-traced symbol path handles it; only
/// the general mode decode cannot yet.
fn route_general_minimal_intra(sequence: &SequenceHeader, core: &FrameHeaderCore) -> bool {
    core.quantization_params
        .is_some_and(|quant| quant.base_q_idx != FROZEN_MINIMAL_BASE_Q_IDX)
        // Reconstruction passes zero quantizer deltas, so admit only frames whose
        // §5.18.6.1 per-plane DeltaQ values are all zero. (Base*DeltaQ is forced
        // to zero by the `equal_ac_dc_q` admission below.)
        && core.quantization_params.is_some_and(|quant| {
            quant.delta_q_y_dc == 0
                && quant.delta_q_u_dc == 0
                && quant.delta_q_u_ac == 0
                && quant.delta_q_v_dc == 0
                && quant.delta_q_v_ac == 0
        })
        && sequence
            .intra
            .as_ref()
            .is_some_and(|intra| !intra.enable_dip)
        && sequence
            .partition
            .is_some_and(|partition| !partition.enable_sdp)
        // §5.20 / §5.20.7.27: FSC, CCTX, IDTX, and IST all add transform-type or
        // cross-component syntax the general path does not yet read; `equal_ac_dc_q`
        // forces every derived Base*DeltaQ to zero (§5.4.8).
        && sequence.transform_quant_entropy.is_some_and(|tq| {
            tq.equal_ac_dc_q
                && !tq.enable_fsc
                && !tq.enable_cctx
                && !tq.enable_idtx_intra
                && !tq.enable_intra_ist
                // §5.4.8: equal_ac_dc_q forces BaseYDcDeltaQ to zero, but the
                // chroma base offsets BaseUVDcDeltaQ / BaseUVAcDeltaQ are derived
                // independently. Reconstruction passes zero deltas, so require
                // both to resolve to zero as well.
                && i32::from(tq.base_uv_dc_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
                && i32::from(tq.base_uv_ac_delta_q) + GENERAL_INTRA_DELTA_DCQUANT_MIN == 0
        })
        // §5.20.6.1: TX_MODE_SELECT inserts read_tx_partition() before coeffs();
        // only the fixed-largest 64x64 transform is handled.
        && core
            .intra_tail
            .is_some_and(|tail| tail.tx_mode == TxMode::Largest)
        && is_general_minimal_intra(core)
}

/// Returns whether `core` is a single-tile 64x64 8-bit intra key frame with no
/// segmentation, quant matrices, delta-Q, in-loop filters, CCSO, GDF, or film
/// grain — the general intra subset the frontier admits. This mirrors
/// [`validate_frame_core`] but accepts any `base_q_idx`, so the single 64x64
/// block can carry a real (nonzero) residual.
fn is_general_minimal_intra(core: &FrameHeaderCore) -> bool {
    core.status == FrameHeaderParseStatus::IntraHeaderComplete
        && core.cur_mfh_id.is_zero()
        && core.show_existing_frame == Some(false)
        && core.frame_is_intra == Some(true)
        && core.is_key_frame
        && core.immediate_output_frame == Some(true)
        && core.implicit_output_frame == Some(false)
        && matches!(
            core.frame_size,
            Some(FrameSize {
                width: MINIMAL_WIDTH,
                height: MINIMAL_HEIGHT,
                ..
            })
        )
        && core
            .tile_info
            .as_ref()
            .is_some_and(|tile_info| tile_info.tile_cols == 1 && tile_info.tile_rows == 1)
        && core.quantization_params.is_some()
        && core
            .segmentation_params
            .as_ref()
            .is_some_and(|seg| !seg.segmentation_enabled)
        && core.setup_qm_params.is_some_and(|qm| !qm.using_qmatrix)
        && core
            .delta_q_params
            .is_some_and(|delta| !delta.delta_q_present)
        && core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| !lossless.coded_lossless)
        && core
            .deblocking_filter_params
            .is_some_and(|filter| filter.apply_deblocking_filter == [false; 4])
        && core.gdf_params.is_some_and(|gdf| !gdf.gdf_frame_enable)
        && core
            .cdef_params
            .as_ref()
            .is_some_and(|cdef| !cdef.cdef_frame_enable)
        && core.lr_params.as_ref().is_some_and(|lr| !lr.uses_lr)
        && core
            .ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.ccso_frame_flag.is_none() && ccso.planes.is_empty())
        && core
            .intra_tail
            .is_some_and(|tail| !tail.film_grain.apply_grain)
        // Screen-content tools enable §5.20.8.1 palette_mode_info() after uv_mode,
        // adding mode symbols the general mode decode does not yet read.
        && core.allow_screen_content_tools != Some(true)
}

/// Decodes a general minimal-tool intra key frame as far as the current
/// frontier reaches.
///
/// This runs the real AV2 § 5.20.3.1 partition traversal over the single tile,
/// confirms the root partition frontier, decodes the § 5.20.5.3 block mode info,
/// decodes the § 5.20.7.27 luma and chroma transform-block coefficients,
/// dequantizes / inverse-transforms / residual-adds each plane over a
/// no-neighbour DC prediction, validates `exit_symbol()`, and returns the
/// reconstructed frame. It never mutates the frozen minimal hash tier.
#[allow(clippy::too_many_arguments)]
fn decode_general_minimal_intra_frame(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: DecodeOptions,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let mut tile_plan = derive_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
    )?;
    let tile = match tile_plan.work_units_mut() {
        [tile] => tile,
        [] => {
            return Err(general_intra_unsupported(
                "general_intra_missing_tile_work_unit",
                None,
                "general intra decode requires one tile work unit",
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
        work_units => {
            return Err(general_intra_unsupported(
                "general_intra_unexpected_tile_work_units",
                work_units.first().map(|tile| tile.tile_byte_span().start),
                "general intra decode currently supports exactly one tile work unit",
                GENERAL_INTRA_TILE_SPEC_SECTION,
            ));
        }
    };
    let tile_offset = tile.tile_byte_span().start;
    let frontier = crate::tile_payload::plan_minimal_runtime_partition_frontier(
        tile,
        sequence,
        core,
        options.limits(),
    )
    .map_err(|error| general_intra_partition_frontier_error(error, tile_offset))?;
    let mut symbols = frontier.into_symbol_decoder();
    let modes = crate::tile_payload::decode_general_intra_block_modes(tile, &mut symbols)
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    // §7.13: the no-neighbour reconstruction only reproduces DC prediction
    // exactly; SMOOTH/PAETH/directional luma modes and non-DC chroma modes need
    // their own §7.13 predictors.
    if !modes.is_dc_only() {
        return Err(general_intra_unsupported(
            "general_intra_non_dc_prediction_mode",
            Some(tile_offset),
            "general intra reconstruction only supports DC prediction; non-DC luma or chroma modes are not yet implemented",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }

    // §7.14.2 quantizer index == base_q_idx for the minimal-tool frame (no
    // segmentation or delta-Q). The §7.14.4 TCQ dqDenom term applies to the luma
    // DCT_DCT (TX_CLASS_2D) non-lossless non-FSC block only.
    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            general_intra_unsupported(
                "general_intra_missing_base_q",
                Some(tile_offset),
                "general intra decode requires a parsed base_q_idx",
                GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
            )
        })?;
    let luma_use_tcq = tile.coeff_frame_facts().allow_tcq();

    let luma = crate::tile_payload::decode_general_intra_luma_coeffs(tile, &mut symbols, modes)
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    let u = crate::tile_payload::decode_general_intra_chroma_coeffs(tile, &mut symbols, 1, false)
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    let v =
        crate::tile_payload::decode_general_intra_chroma_coeffs(tile, &mut symbols, 2, !u.all_zero)
            .map_err(|error| general_intra_residual_error(error, tile_offset))?;

    // §5.20.7.27: a block with eob > 1 codes intra_tx_type (plane 0) or cctx_type
    // (plane 1) before the coefficient levels; only all-zero (eob == 0) and
    // single-DC (eob == 1) blocks are decoded bit-exactly here.
    if luma.eob > 1 || u.eob > 1 || v.eob > 1 {
        return Err(general_intra_unsupported(
            "general_intra_multi_coefficient_block",
            Some(tile_offset),
            "general intra reconstruction only supports all-zero and single-DC transform blocks; multi-coefficient blocks (eob > 1) are not yet implemented",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ));
    }

    // The single 64x64 block consumes the entire tile payload, so §8.2.4
    // exit_symbol() must hold after the luma + chroma coefficients. A failure
    // means the coefficient decode was not bit-exact.
    symbols.exit_symbol().map_err(|_| {
        general_intra_unsupported(
            "general_intra_exit_symbol",
            Some(tile_offset),
            "general intra tile payload did not satisfy §8.2.4 exit_symbol() after the decoded coefficients",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;

    // Enforce the configured decode limits (frame size, luma samples, decoded
    // bytes, output bytes, tile payload) before allocating any reconstruction
    // buffers, matching the frozen minimal path's ordering.
    let tile_size = tile.tile_size();
    let limits = options.limits();
    ensure_runtime_limits(limits, MINIMAL_WIDTH, MINIMAL_HEIGHT, tile_size)?;

    let luma_plane = general_intra_plane_samples(&luma, qindex, PlaneId::Y, 6, luma_use_tcq)
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    let u_plane = general_intra_plane_samples(&u, qindex, PlaneId::U, 5, false)
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    let v_plane = general_intra_plane_samples(&v, qindex, PlaneId::V, 5, false)
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;

    let frame = crate::runtime_minimal_recon::assemble_general_intra_frame(
        &luma_plane,
        &u_plane,
        &v_plane,
    )?;

    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

/// Reconstructs one plane's samples from its decoded coefficient block: a
/// skipped (`all_zero`) block is the flat no-neighbour DC prediction (`128`),
/// otherwise the dequant / inverse-transform / residual-add reconstruction.
fn general_intra_plane_samples(
    block: &crate::tile_payload::LumaCoeffBlock,
    qindex: u32,
    plane_id: splot_recon::PlaneId,
    log2_side: u32,
    use_tcq: bool,
) -> core::result::Result<Vec<u8>, GeneralIntraResidualError> {
    if block.all_zero {
        let side = 1usize << log2_side;
        return Ok(vec![128u8; side * side]);
    }
    crate::tile_payload::reconstruct_general_intra_block(
        &block.quant,
        qindex,
        plane_id,
        log2_side,
        use_tcq,
    )
}

fn general_intra_residual_error(
    error: GeneralIntraResidualError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraResidualError::AllZeroRead { .. }
        | GeneralIntraResidualError::NonZeroPass { .. } => general_intra_unsupported(
            "general_intra_luma_coeff_parse",
            Some(offset),
            "general intra luma transform-block coefficient syntax could not be parsed from the tile payload",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::CoeffContextState { .. }
        | GeneralIntraResidualError::InvalidContextRange { .. } => general_intra_unsupported(
            "general_intra_luma_coeff_state",
            Some(offset),
            "general intra luma coefficient context state could not be derived from the tile work unit",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::UnexpectedBranch => general_intra_unsupported(
            "general_intra_luma_coeff_unexpected_branch",
            Some(offset),
            "general intra luma coefficient decode produced an unexpected branch result",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
        GeneralIntraResidualError::QuantLength { .. }
        | GeneralIntraResidualError::Reconstruct { .. } => general_intra_unsupported(
            "general_intra_luma_reconstruct",
            Some(offset),
            "general intra luma transform-block reconstruction could not be composed from the decoded coefficients",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        ),
    }
}

fn general_intra_block_mode_error(
    error: GeneralIntraBlockModeError,
    offset: ByteOffset,
) -> DecodeError {
    match error {
        GeneralIntraBlockModeError::SymbolRead { .. }
        | GeneralIntraBlockModeError::Literal { .. } => general_intra_unsupported(
            "general_intra_block_mode_parse",
            Some(offset),
            "general intra block mode-info syntax could not be parsed from the tile payload",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::UnsupportedYMode { .. } => general_intra_unsupported(
            "general_intra_unsupported_y_mode",
            Some(offset),
            "general intra decode currently reconstructs only the non-directional luma intra mode subset",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
        GeneralIntraBlockModeError::InvalidUvMode { .. } => general_intra_unsupported(
            "general_intra_invalid_uv_mode",
            Some(offset),
            "general intra decode rejected an out-of-range chroma uv_mode index",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ),
    }
}

fn general_intra_partition_frontier_error(
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
        | MinimalRuntimePartitionFrontierError::Traversal(_)
        | MinimalRuntimePartitionFrontierError::UnexpectedFrontier { .. } => {
            general_intra_unsupported(
                "general_intra_partition_frontier",
                Some(offset),
                "general intra decode could not reach a supported AV2 §5.20.3.1 single-block root partition frontier",
                GENERAL_INTRA_PARTITION_SPEC_SECTION,
            )
        }
    }
}

fn general_intra_unsupported(
    reason: &'static str,
    byte_offset: Option<ByteOffset>,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            GENERAL_INTRA_TIER_ID,
            GENERAL_INTRA_MATRIX_ROW,
            GENERAL_INTRA_FEATURE_ID,
            spec_section,
            message,
            GENERAL_INTRA_REMEDIATION,
            byte_offset,
        )),
    }
}

fn derive_tile_plan<'a>(
    plan: &'a DecodeStreamPlan,
    candidate: &'a DecodePlannedObu,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    options: DecodeOptions,
) -> Result<crate::tile_payload::DecodeTilePayloadPlan<'a>> {
    let tq = sequence.transform_quant_entropy.as_ref().ok_or_else(|| {
        unsupported_at(
            "missing_tq_entropy_config",
            envelope.offset,
            "minimal tier requires sequence transform/quant/entropy config",
        )
    })?;
    let coeff = FrameCandidateCoeffFacts::new(tq.enable_fsc, tq.enable_chroma_dctonly);
    let facts = FrameCandidateTileFacts::from_frame_core(core, coeff)
        .map_err(decode_tile_boundary_error)?;
    let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
    let input = FrameCandidateTileBoundaryInput::new(
        plan,
        candidate,
        bytes,
        envelope,
        TileGroupPositionFacts::new(true, true),
        facts,
        cdf,
        options.limits(),
    );
    crate::tile_payload::plan_derived_tile_payload_boundary(input)
        .map_err(decode_tile_boundary_error)
}

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

fn ensure_runtime_limits(
    limits: crate::DecodeLimits,
    width: u32,
    height: u32,
    tile_payload_bytes: u64,
) -> Result<()> {
    limits.ensure(DecodeLimitName::MaxOutputFrames, 1)?;
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
    limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
    let luma_samples = checked_mul(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        u64::from(width),
        u64::from(height),
    )?;
    limits.ensure(DecodeLimitName::MaxLumaSamplesPerFrame, luma_samples)?;
    let chroma_samples = checked_mul(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        u64::from(width / 2),
        u64::from(height / 2),
    )?;
    let decoded_bytes = checked_add(
        DecodeLimitName::MaxDecodedFrameBytes,
        luma_samples,
        checked_mul(DecodeLimitName::MaxDecodedFrameBytes, chroma_samples, 2)?,
    )?;
    limits.ensure(DecodeLimitName::MaxDecodedFrameBytes, decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, decoded_bytes)?;
    limits.ensure(DecodeLimitName::MaxTileCount, 1)?;
    limits.ensure(DecodeLimitName::MaxTilePayloadBytes, tile_payload_bytes)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, luma_samples)?;
    limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, chroma_samples)?;
    Ok(())
}

fn checked_add(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_add(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Add,
            left,
            right,
        })
}

fn checked_mul(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_mul(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Mul,
            left,
            right,
        })
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod general_intra_tests {
    use splot_parallel::ThreadCount;

    use super::*;
    use crate::{DecodeContext, DecodeRuntimeConfig};

    const Q80_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");

    // avmdec and dav2d both decode this fixture to flat planes.
    const Q80_LUMA: u8 = 100;
    const Q80_CHROMA_U: u8 = 120;
    const Q80_CHROMA_V: u8 = 130;

    // Drives the q80 fixture through the full general intra runtime path: decode
    // modes -> decode luma + chroma coefficients -> dequant -> inverse transform
    // -> residual add over the no-neighbour DC prediction -> frame assembly.
    fn decode_q80_frame() -> DecodedFrame<u8> {
        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(Q80_FIXTURE, options).expect("plan");
        decode_minimal_frame_from_plan(Q80_FIXTURE, options, &plan)
            .expect("decode")
            .frame
    }

    #[test]
    fn q80_intra_frame_reconstructs_flat_planes() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let frame = decode_q80_frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        let y = frame.y().samples();
        assert!(
            y.iter().all(|&s| s == Q80_LUMA),
            "luma must be flat {Q80_LUMA}; first samples: {:?}",
            &y[..8]
        );
        let u = frame.u().unwrap().samples();
        assert!(
            u.iter().all(|&s| s == Q80_CHROMA_U),
            "U must be flat {Q80_CHROMA_U}; first samples: {:?}",
            &u[..8]
        );
        let v = frame.v().unwrap().samples();
        assert!(
            v.iter().all(|&s| s == Q80_CHROMA_V),
            "V must be flat {Q80_CHROMA_V}; first samples: {:?}",
            &v[..8]
        );
    }

    #[test]
    fn q80_intra_frame_hash_is_stable() {
        // Regression pin for the full-frame decode hash. The flat-plane test
        // above is the avmdec/dav2d oracle anchor; this pins the byte layout.
        let frame = decode_q80_frame();
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979"
        );
    }
}
