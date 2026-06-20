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
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};
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
    // Size routing happens after this preflight: the frozen 64x64 hash tier
    // re-imposes its strict 64x64 requirement in `validate_frame_core`, and the
    // general intra path accepts positive multiples of 64 (checked in
    // `is_general_minimal_intra`). Admitting any positive frame size here lets
    // both 64x64 and larger multiple-of-64 frames reach routing while still
    // gating container shape (AV02, single in-memory frame, no warnings/errors).
    if header.fourcc != *b"AV02"
        || header.width == 0
        || header.height == 0
        || header.frame_count != 1
        || ivf.frames.len() != 1
        || !ivf.warnings.is_empty()
        || ivf.error.is_some()
    {
        return Err(unsupported(
            "unsupported_ivf_shape",
            None,
            "minimal tier requires one positive-sized AV02 IVF frame with no container warnings",
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
            // §7.13.2 (lines 5355-5365): with `enable_ibp == 1` a non-4x4
            // `DC_PRED` block runs the §7.13.2.12 IBP DC process, which modifies
            // the prediction using the available left/above neighbours. This path
            // applies only the plain §7.13.2.4 DC predictor, so a neighbour-having
            // DC block (any non-first superblock / split block) would reconstruct
            // wrong pixels under IBP. Reject `enable_ibp` until the IBP DC process
            // is modelled (all committed fixtures are encoded with enable_ibp = 0).
            .is_some_and(|intra| !intra.enable_dip && !intra.enable_ibp)
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

/// Returns whether `core` is a single-tile 8-bit intra key frame whose width and
/// height are positive multiples of 64 forming a (possibly 2-D) grid of 64x64
/// superblocks, with no segmentation, quant matrices, delta-Q, in-loop filters,
/// CCSO, GDF, or film grain — the general intra subset the frontier admits. This
/// mirrors [`validate_frame_core`] but accepts any `base_q_idx`, so blocks can
/// carry a real (nonzero) residual.
fn is_general_minimal_intra(core: &FrameHeaderCore) -> bool {
    core.status == FrameHeaderParseStatus::IntraHeaderComplete
        && core.cur_mfh_id.is_zero()
        && core.show_existing_frame == Some(false)
        && core.frame_is_intra == Some(true)
        && core.is_key_frame
        && core.immediate_output_frame == Some(true)
        && core.implicit_output_frame == Some(false)
        // §5.18.3 / §5.20.2.1 / §7.13.2.1: the general intra path tiles the frame
        // into 64x64 superblocks, so width and height must be positive multiples
        // of the superblock side (64), and the §5.20.2.1 raster loop iterates them
        // (`clear_left_context()` per superblock row) with later superblocks
        // predicting from already-reconstructed left/above neighbours. A full 2-D
        // grid is admitted: a non-rightmost row>0 superblock's full-superblock
        // §7.13.2.13 SMOOTH chroma block has a decoded above-right neighbour
        // (`clear_block_decoded_flags` (§5.20.2.3) marks `BlockDecoded[-1][x] = 1`
        // up to `(MiColEnd - c) >> subX`, which exceeds the superblock width), so
        // the §7.13.2.1 `AboveRow[w]` sentinel reads the real reconstructed
        // `CurrFrame[plane][y-1][Min(aboveLimit, x+w)]` sample (see
        // `reconstruct_general_intra_chroma_smooth_into`), no longer the edge-clamp.
        && core.frame_size.is_some_and(|size| {
            size.width != 0
                && size.height != 0
                && size.width % MINIMAL_WIDTH == 0
                && size.height % MINIMAL_HEIGHT == 0
        })
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
    let (mi_rows, mi_cols) = crate::tile_payload::frame_mi_dimensions(core)
        .map_err(|error| general_intra_partition_frontier_error(error, tile_offset))?;

    // §5.18.3 frame dimensions: `is_general_minimal_intra` already gated these to
    // positive multiples of 64, so the workspace and decode limits are sized to
    // the real frame size (not the 64x64 single-superblock constant) so that
    // multi-superblock frames (e.g. 128x64) reconstruct into the full plane.
    let frame_size = core.frame_size.ok_or_else(|| {
        general_intra_unsupported(
            "general_intra_missing_frame_size",
            Some(tile_offset),
            "general intra decode requires a parsed frame size",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    // Enforce the configured decode limits before allocating reconstruction
    // buffers, matching the frozen minimal path's ordering.
    let tile_size = tile.tile_size();
    let limits = options.limits();
    ensure_runtime_limits(limits, frame_width, frame_height, tile_size)?;

    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace(
        frame_width as usize,
        frame_height as usize,
    )?;
    let mut coeff_ctx =
        crate::tile_payload::TileCoeffContextState::new(mi_rows, mi_cols).map_err(|source| {
            general_intra_residual_error(
                GeneralIntraResidualError::CoeffContextState { source },
                tile_offset,
            )
        })?;

    // Walk the full §5.20.3.1 partition tree, decoding each leaf block's
    // §5.20.5.3 mode info and §5.20.7.27 Y/U/V coefficients and reconstructing it
    // into the workspace in decode order (so later blocks DC-predict from the
    // already-reconstructed neighbours).
    let symbols = crate::tile_payload::decode_general_intra_multiblock_tree(
        tile,
        sequence,
        core,
        limits,
        |work_unit, symbols, frontier| {
            decode_one_general_intra_block(
                work_unit,
                symbols,
                frontier,
                &mut workspace,
                &mut coeff_ctx,
                qindex,
                luma_use_tcq,
                mi_cols,
                tile_offset,
            )
        },
    )
    .map_err(|error| map_general_intra_multiblock_error(error, tile_offset))?;

    // The decoded blocks consume the entire tile payload, so §8.2.4
    // exit_symbol() must hold; a failure means the decode was not bit-exact.
    symbols.exit_symbol().map_err(|_| {
        general_intra_unsupported(
            "general_intra_exit_symbol",
            Some(tile_offset),
            "general intra tile payload did not satisfy §8.2.4 exit_symbol() after the decoded blocks",
            GENERAL_INTRA_RESIDUAL_SPEC_SECTION,
        )
    })?;

    let frame = workspace.freeze()?;
    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

/// Decodes one general intra leaf block (mode info + Y/U/V coefficients) and
/// reconstructs it into `workspace` in decode order. Gated to square DC_PRED
/// blocks: the no-neighbour-aware §7.13.2 DC prediction is read from the
/// partially-built frame, so non-DC modes and non-square partitions are
/// rejected. Chroma is 4:2:0 (half-resolution).
#[allow(clippy::too_many_arguments)]
fn decode_one_general_intra_block(
    work_unit: &mut crate::tile_payload::DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    frontier: &crate::tile_payload::DecodeBlockFrontier,
    workspace: &mut splot_recon::CurrentFrameWorkspace<u8>,
    coeff_ctx: &mut crate::tile_payload::TileCoeffContextState,
    qindex: u32,
    luma_use_tcq: bool,
    mi_cols: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    // Resolve the block geometry and gate the handled subset BEFORE reading the
    // §5.20.5.3 mode info: `uv_mode` is only coded when the block has chroma, and
    // sub-8x8 luma leaves use a different (deferred 4x4) chroma sizing that this
    // path does not model. Reading modes first for those cases would consume a
    // `uv_mode` symbol that is not present and desynchronise the decoder.
    let geometry_error = || {
        general_intra_unsupported(
            "general_intra_block_geometry",
            Some(tile_offset),
            "general intra block geometry lookup failed",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        )
    };
    let n4w = frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| geometry_error())?;
    let n4h = frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| geometry_error())?;
    if n4w != n4h {
        return Err(general_intra_unsupported(
            "general_intra_non_square_block",
            Some(tile_offset),
            "general intra decode only supports square partition blocks; rectangular partitions are not yet implemented",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    // 4:2:0 sub-8x8 luma leaves defer chroma to the bottom-right 4x4 (a 4x4 chroma
    // transform over the 8x8 region, not luma_log2 - 1), and the other three are
    // luma-only; neither chroma sizing/position is modelled yet.
    if n4w < 2 {
        return Err(general_intra_unsupported(
            "general_intra_sub_8x8_block",
            Some(tile_offset),
            "general intra decode does not yet support sub-8x8 luma blocks (deferred 4:2:0 chroma sizing)",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }
    if !frontier.has_chroma {
        return Err(general_intra_unsupported(
            "general_intra_luma_only_block",
            Some(tile_offset),
            "general intra decode does not yet support luma-only (no-chroma) blocks",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ));
    }

    let modes = crate::tile_payload::decode_general_intra_block_modes(work_unit, symbols)
        .map_err(|error| general_intra_block_mode_error(error, tile_offset))?;
    // Chroma is reconstructed with DC prediction or, when the decoded
    // `uv_mode` resolves (via § 5.20.5.3 `get_intra_uv_mode_set`) to
    // `SMOOTH_PRED`, with § 7.13.2.13 smooth prediction over § 7.13.2.1
    // neighbour edges read from the partially-built frame. Other non-DC chroma
    // modes (directional, PAETH, SMOOTH_V/H) need their own § 7.13 predictors
    // and are deferred.
    let Some(supported_chroma) = modes.supported_chroma_mode() else {
        return Err(general_intra_unsupported(
            "general_intra_non_dc_chroma_mode",
            Some(tile_offset),
            "general intra reconstruction only supports DC and SMOOTH chroma prediction; other non-DC chroma (uv_mode) modes are not yet implemented",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    };
    // §7.13.2.1: the SMOOTH chroma path builds the §7.13.2.13 bottom-left
    // (`LeftCol[h]`) sentinel by edge-clamping (repeating the last in-block
    // neighbour sample). In raster decode order a full-superblock block's
    // below-left chroma is never decoded yet (`num4BelowLeft == 0`), so the spec
    // value `CurrFrame[Min(maxY, y+h)][x-1]` equals the clamped last left sample.
    // The top-right (`AboveRow[w]`) sentinel, however, reads the real
    // reconstructed `CurrFrame[plane][y-1][Min(aboveLimit, x+w)]` when the
    // above-right is decoded (`num4AboveRight > 0`): for a non-rightmost row>0
    // superblock `clear_block_decoded_flags` (§5.20.2.3) marks the above row
    // decoded out to `(MiColEnd - c) >> subX`, exceeding the superblock width.
    //
    // SMOOTH chroma is still gated to full-superblock blocks (`n4w == 16`): a
    // sub-partitioned (split) block needs the §5.20.2.3 per-block `BlockDecoded`
    // update (so an intra-superblock above-right / below-left split child is read
    // correctly), which is not yet modelled. A 64x64 superblock is 16 4x4 MI
    // units wide.
    const FULL_SB_N4: usize = 16;
    if supported_chroma == crate::tile_payload::SupportedChromaMode::Smooth && n4w != FULL_SB_N4 {
        return Err(general_intra_unsupported(
            "general_intra_smooth_chroma_subblock",
            Some(tile_offset),
            "general intra SMOOTH chroma is only supported for full 64x64 superblock blocks; sub-partitioned SMOOTH chroma needs the §7.13.2.1 above-right / below-left sentinel neighbours from the per-block §5.20.2.3 BlockDecoded update, which is not yet modelled",
            GENERAL_INTRA_MODE_SPEC_SECTION,
        ));
    }
    // Luma is DC, a supported non-DC mode (§ 7.13.2.13 SMOOTH_V / SMOOTH_H), or
    // the supported directional mode (§ 7.13.2.8 D135_PRED). The non-DC and
    // directional modes are only reconstructed for the top-left (no-neighbour)
    // block, where § 7.13.2.1 supplies pure fallback edges and the
    // `enable_intra_edge_filter` / IDIF / upsample edge synthesis are no-ops.
    //
    // Non-DC smooth: gated to >= 32x32 (`n4w >= 8`), where § 5.20.8.2
    // `get_tx_set` returns TX_SET_DCTONLY (square intra `txSzSqrUp >= TX_32X32`
    // -> forced DCT_DCT, no `intra_tx_type`). Directional D135: gated to the
    // verified 64x64 superblock (`n4w == 16`, TX_64X64 -> TX_SET_DCTONLY); the
    // 32x32 / smaller directional blocks (which may signal a mode-dependent
    // non-DCT TxType) and other angles / non-zero angle deltas are deferred.
    const NON_DC_MIN_N4: usize = 8;
    const FULL_SB_N4_LUMA: usize = 16;
    let supported_nondc_luma = modes.supported_nondc_luma();
    let supported_directional_luma = modes.supported_directional_luma();
    if !modes.luma_is_dc() {
        let is_top_left = frontier.r == 0 && frontier.c == 0;
        if !is_top_left {
            return Err(general_intra_unsupported(
                "general_intra_multiblock_non_dc_luma",
                Some(tile_offset),
                "general intra non-DC / directional luma prediction is only supported for the top-left (no-neighbour) block; multi-block non-DC prediction is not yet implemented",
                GENERAL_INTRA_MODE_SPEC_SECTION,
            ));
        }
        match (supported_nondc_luma, supported_directional_luma) {
            (Some(_), _) if n4w >= NON_DC_MIN_N4 => {}
            (Some(_), _) => {
                return Err(general_intra_unsupported(
                    "general_intra_non_dc_non_dctonly_size",
                    Some(tile_offset),
                    "general intra non-DC luma prediction is only supported for 32x32-or-larger (TX_SET_DCTONLY) blocks; smaller non-DC blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (_, Some(_)) if n4w == FULL_SB_N4_LUMA => {}
            (_, Some(_)) => {
                return Err(general_intra_unsupported(
                    "general_intra_directional_non_dctonly_size",
                    Some(tile_offset),
                    "general intra directional (D135) luma prediction is only supported for the verified 64x64 (TX_SET_DCTONLY) superblock block; smaller directional blocks can signal a mode-dependent transform type that is not yet decoded",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
            (None, None) => {
                return Err(general_intra_unsupported(
                    "general_intra_unsupported_luma_mode",
                    Some(tile_offset),
                    "general intra reconstruction only supports DC, SMOOTH_V / SMOOTH_H, and D135 (pAngle 135) luma prediction; SMOOTH, PAETH, other directional modes, and non-zero angle deltas are not yet implemented",
                    GENERAL_INTRA_MODE_SPEC_SECTION,
                ));
            }
        }
    }

    let uv_mode = usize::from(modes.uv_mode);
    let luma_log2 = n4w.trailing_zeros() + 2;
    let luma_tx = (luma_log2 - 2) as usize;
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma = crate::tile_payload::decode_general_intra_plane_coeffs(
        work_unit, symbols, coeff_ctx, 0, luma_tx, luma_x, luma_y, false, uv_mode,
    )
    .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    match (supported_nondc_luma, supported_directional_luma) {
        (Some(mode), _) => {
            crate::runtime_minimal_recon::reconstruct_general_intra_luma_nondc_first_block_into(
                workspace,
                &luma,
                mode,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (None, Some(mode)) => {
            crate::runtime_minimal_recon::reconstruct_general_intra_luma_directional_first_block_into(
                workspace,
                &luma,
                mode,
                luma_x,
                luma_y,
                luma_log2,
                qindex,
                luma_use_tcq,
            )
            .map_err(|error| general_intra_residual_error(error, tile_offset))?
        }
        (None, None) => crate::runtime_minimal_recon::reconstruct_general_intra_block_into(
            workspace,
            &luma,
            PlaneId::Y,
            luma_x,
            luma_y,
            luma_log2,
            qindex,
            luma_use_tcq,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?,
    }

    if frontier.has_chroma {
        // 4:2:0: chroma is half-resolution, so the chroma transform/log2 is one
        // smaller and the chroma plane position is the luma position >> 1.
        let chroma_log2 = luma_log2 - 1;
        let chroma_tx = (chroma_log2 - 2) as usize;
        let chroma_x = frontier.c * 2;
        let chroma_y = frontier.r * 2;
        // §7.13.2.1 `num4AboveRight` for the full-superblock chroma block, from
        // §5.20.7.25 `count_top_right_avail` over the §5.20.2.3 `BlockDecoded`
        // state. SMOOTH chroma is gated to full-superblock blocks above, so the
        // block is the whole 64x64 superblock; the §7.13.2.13 top-right sentinel
        // needs the real reconstructed above-right sample when an in-frame,
        // already-decoded superblock sits to this superblock's upper-right.
        let num4_above_right =
            full_sb_chroma_num4_above_right(frontier.c, n4w, mi_cols, FRAME_420_SUBSAMPLING_X);
        let u = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit, symbols, coeff_ctx, 1, chroma_tx, chroma_x, chroma_y, false, uv_mode,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_chroma_block_into(
            workspace,
            &u,
            PlaneId::U,
            chroma_x,
            chroma_y,
            chroma_log2,
            qindex,
            supported_chroma,
            num4_above_right,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        let v = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            2,
            chroma_tx,
            chroma_x,
            chroma_y,
            !u.all_zero,
            uv_mode,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
        crate::runtime_minimal_recon::reconstruct_general_intra_chroma_block_into(
            workspace,
            &v,
            PlaneId::V,
            chroma_x,
            chroma_y,
            chroma_log2,
            qindex,
            supported_chroma,
            num4_above_right,
        )
        .map_err(|error| general_intra_residual_error(error, tile_offset))?;
    }
    Ok(())
}

/// 4:2:0 chroma horizontal subsampling (`SubsamplingX == 1`).
const FRAME_420_SUBSAMPLING_X: usize = 1;

/// Derives AV2 § 7.13.2.1 `num4AboveRight` (in chroma 4x4 units) for a
/// full-superblock chroma transform block, faithfully to § 5.20.7.25
/// `count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state.
///
/// For a full 64x64 superblock the block coincides with the superblock, so its
/// chroma sub-block MI position within the superblock is `(0, 0)` and its chroma
/// width in 4x4 units is `w4 = n4w >> SubsamplingX` (the luma `n4w` 4x4 units
/// subsampled). `count_top_right_avail(plane, 0, 0, w4)` scans
/// `BlockDecoded[plane][-1][w4 + i]` for `i in 0..w4`; `clear_block_decoded_flags`
/// (§ 5.20.2.3) marks the above row decoded for chroma columns
/// `x < (MiColEnd - c) >> SubsamplingX` (a single full-frame tile has
/// `MiColEnd == MiCols`), so a column `w4 + i` is decoded while
/// `w4 + i < (MiCols - c) >> SubsamplingX`. The count stops at the first
/// undecoded column (or at `w4`), matching the spec loop's `break`.
fn full_sb_chroma_num4_above_right(c: usize, n4w: usize, mi_cols: usize, sub_x: usize) -> usize {
    let w4 = n4w >> sub_x;
    // Chroma above-row decoded extent (in chroma 4x4 columns) for this
    // superblock, from `clear_block_decoded_flags` `sbWidth4 = (MiColEnd - c) >> subX`.
    let above_decoded_cols = mi_cols.saturating_sub(c) >> sub_x;
    let mut num_top_right = 0;
    for i in 0..w4 {
        if w4 + i < above_decoded_cols {
            num_top_right = i + 1;
        } else {
            break;
        }
    }
    num_top_right
}

/// Maps a general intra multi-block tree-walk error to a decode diagnostic. The
/// leaf-block error is already a structured `DecodeError`; setup, traversal, and
/// MI-size failures collapse to an unsupported-partition diagnostic.
fn map_general_intra_multiblock_error(
    error: crate::tile_payload::GeneralIntraMultiblockError<DecodeError>,
    tile_offset: ByteOffset,
) -> DecodeError {
    use crate::tile_payload::{GeneralIntraMultiblockError, GeneralIntraTreeWalkError};
    match error {
        GeneralIntraMultiblockError::Setup(error) => {
            general_intra_partition_frontier_error(error, tile_offset)
        }
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Leaf(error)) => error,
        // Preserve the resource-limit contract: a partition-step (or other)
        // limit must report as DecodeError::Limit, not unsupported-feature.
        GeneralIntraMultiblockError::Walk(GeneralIntraTreeWalkError::Traversal(
            TilePartitionTraversalError::Limit(source),
        )) => DecodeError::Limit { source },
        GeneralIntraMultiblockError::Walk(_) => general_intra_unsupported(
            "general_intra_partition_walk",
            Some(tile_offset),
            "general intra partition tree walk reached an unsupported path",
            GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
    }
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
        GeneralIntraResidualError::CoeffContextState { .. } => general_intra_unsupported(
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
        | GeneralIntraResidualError::PredictionLength { .. }
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
    // AV2 §5.3.2 4:2:0 chroma plane size uses `(dimension + subsamplingX) >> 1`
    // rounding. Equivalent to `dimension / 2` for the admitted even (multiple-of-64)
    // sizes, but written spec-faithfully so a future size relaxation stays correct.
    let chroma_samples = checked_mul(
        DecodeLimitName::MaxLumaSamplesPerFrame,
        u64::from((width + 1) >> 1),
        u64::from((height + 1) >> 1),
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

    // A single-block DC_PRED intra frame whose luma carries multiple (eob > 1) AC
    // coefficients from a low-frequency half-cosine input; avmdec's raw output is
    // reproduced byte-for-byte (verified locally) and pinned via the frame hash.
    const Q180_COS_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-cos-intra-64x64-q180.ivf");

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

    #[test]
    fn q180_cos_intra_frame_decodes_multi_coefficient_luma() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(Q180_COS_FIXTURE, options).expect("plan");
        let frame = decode_minimal_frame_from_plan(Q180_COS_FIXTURE, options, &plan)
            .expect("decode")
            .frame;

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        // The luma is a reconstructed low-frequency cosine: genuinely non-flat
        // (proving the eob > 1 AC coefficient path ran, not just a DC level).
        let y = frame.y().samples();
        let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert!(
            distinct > 4,
            "luma should be a non-flat AC reconstruction; distinct={distinct}"
        );

        // Frame hash pins splot's output, which reproduces avmdec's raw output
        // byte-for-byte (verified locally against ~/Devel/avm/build/avmdec).
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "8a6751d4517073bad0bbe71f4b5537df8e8b0bfee85fcd6af1ac2d5878dd59e8"
        );
    }

    // A multi-block intra frame: four flat 32x32 luma quadrants that split
    // (Horz -> Vert -> Vert) into four square DC_PRED blocks. Each non-first
    // block DC-predicts from its already-reconstructed neighbour. avmdec and
    // dav2d agree on the decoded output.
    const QUAD_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-quad-intra-64x64-q80.ivf");

    #[test]
    fn quad_multiblock_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(QUAD_FIXTURE, options).expect("plan");
        let frame = decode_minimal_frame_from_plan(QUAD_FIXTURE, options, &plan)
            .expect("decode")
            .frame;

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        let y = frame.y().samples();
        let quad = |r: usize, c: usize| y[r * 64 + c];
        // TL / TR / BL / BR quadrant centres, matching the avmdec/dav2d oracle.
        assert_eq!(
            (quad(16, 16), quad(16, 48), quad(48, 16), quad(48, 48)),
            (80, 200, 160, 40)
        );
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "c54ed4e996841e2178e74033d765dda1e1127d5d89c3012be3266c3e24a7fd28"
        );
    }

    // A 128x64 multi-superblock intra frame: two 64x64 DC_PRED superblocks (left
    // flat luma 80, right flat luma 180). The right superblock DC-predicts its
    // luma from the already-reconstructed left-superblock neighbour, and codes
    // its (residual-free) chroma as SMOOTH_PRED over that flat neighbour. avmdec
    // and dav2d agree on the decoded output (md5 88cf94a2...).
    const TWO_SB_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-2sb-intra-128x64-q80.ivf");

    #[test]
    fn two_superblock_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(TWO_SB_FIXTURE, options).expect("plan");
        let frame = decode_minimal_frame_from_plan(TWO_SB_FIXTURE, options, &plan)
            .expect("decode")
            .frame;

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(64, 32).unwrap()
        );

        // Left superblock (cols 0..64) is flat luma 80, right superblock
        // (cols 64..128) is flat luma 180, matching the avmdec/dav2d oracle.
        let y = frame.y().samples();
        assert!(
            (0..64).all(|r| (0..64).all(|c| y[r * 128 + c] == 80)),
            "left superblock luma must be flat 80"
        );
        assert!(
            (0..64).all(|r| (64..128).all(|c| y[r * 128 + c] == 180)),
            "right superblock luma must be flat 180"
        );
        // Chroma is flat across both superblocks (U=120, V=130).
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
        // raw output byte-for-byte (verified locally).
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f"
        );
    }

    // A 64x128 single-column multi-superblock-ROW intra frame: two vertically
    // stacked 64x64 DC_PRED superblocks (top flat luma 80, bottom flat luma 180,
    // chroma 120/130). Exercises the §5.20.2.1 superblock raster loop across
    // multiple ROWS (`clear_left_context()` per superblock row), with the
    // second-row superblock DC-predicting its luma from the already-reconstructed
    // first-row above neighbour and reconstructing full-superblock SMOOTH chroma
    // at row > 0 (a rightmost-column superblock, so no decoded above-right). avmdec
    // and dav2d agree on the decoded output (md5 bd09ea82...).
    const COL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-2sbcol-intra-64x128-q80.ivf");

    #[test]
    fn multi_row_superblock_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(COL_FIXTURE, options).expect("plan");
        let frame = decode_minimal_frame_from_plan(COL_FIXTURE, options, &plan)
            .expect("decode")
            .frame;

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 128).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(32, 64).unwrap()
        );

        // Top superblock (rows 0..64) flat luma 80; bottom superblock (rows
        // 64..128) flat luma 180, DC-predicted from the reconstructed first-row
        // neighbour. Chroma flat U=120 / V=130. Matches the avmdec/dav2d oracle.
        let y = frame.y().samples();
        assert!(
            (0..64).all(|r| (0..64).all(|c| y[r * 64 + c] == 80)),
            "top superblock luma must be flat 80"
        );
        assert!(
            (64..128).all(|r| (0..64).all(|c| y[r * 64 + c] == 180)),
            "bottom superblock luma must be flat 180"
        );
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "3ee739a805e13597ff7d75659dd1e0150113bf4782c4d69e1d27ae942d6c10a0"
        );
    }

    // A 128x128 2-D grid of four 64x64 DC_PRED-luma superblocks. Luma is uniform
    // (100) so every superblock is DC; chroma is distinct flat per quadrant
    // (U: top-left 110 / top-right 200 / bottom-right 130) except the bottom-left
    // superblock, whose chroma the encoder codes as SMOOTH_PRED over a real 2-D
    // gradient. That bottom-left superblock (raster MI col 0, row > 0) has a
    // decoded above-right neighbour (the top-right superblock), so its §7.13.2.13
    // top-right sentinel `AboveRow[w]` reads the real reconstructed above-right
    // sample (200) per §7.13.2.1 / §5.20.7.25 `count_top_right_avail` — NOT the
    // edge-clamped own-top sample (110). avmdec and dav2d agree on the decoded
    // output (md5 dd2fa84f...); the old repeat-last sentinel mismatched it.
    const GRID_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-grid-intra-128x128-q80.ivf");

    #[test]
    fn grid_2d_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(GRID_FIXTURE, options).expect("plan");
        let frame = decode_minimal_frame_from_plan(GRID_FIXTURE, options, &plan)
            .expect("decode")
            .frame;

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 128).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(64, 64).unwrap()
        );
        assert_eq!(
            frame.v().unwrap().visible_size(),
            PlaneSize::new(64, 64).unwrap()
        );

        // Uniform luma 100 across the whole 2-D grid (all DC_PRED), matching the
        // avmdec/dav2d oracle.
        assert!(
            frame.y().samples().iter().all(|&s| s == 100),
            "luma must be uniform 100 across the 2-D grid"
        );

        // Chroma quadrant helper (64x64 chroma plane, 32x32 quadrants).
        let quad = |plane: &[u8], qr: usize, qc: usize| -> Vec<u8> {
            let mut out = Vec::new();
            for r in (qr * 32)..(qr * 32 + 32) {
                for c in (qc * 32)..(qc * 32 + 32) {
                    out.push(plane[r * 64 + c]);
                }
            }
            out
        };
        let u = frame.u().unwrap().samples();
        let v = frame.v().unwrap().samples();

        // Three flat distinct quadrants (top-left, top-right, bottom-right).
        assert!(
            quad(u, 0, 0).iter().all(|&s| s == 110),
            "U top-left flat 110"
        );
        assert!(
            quad(u, 0, 1).iter().all(|&s| s == 200),
            "U top-right flat 200"
        );
        assert!(
            quad(u, 1, 1).iter().all(|&s| s == 130),
            "U bottom-right flat 130"
        );
        assert!(
            quad(v, 0, 0).iter().all(|&s| s == 120),
            "V top-left flat 120"
        );
        assert!(
            quad(v, 0, 1).iter().all(|&s| s == 160),
            "V top-right flat 160"
        );
        assert!(
            quad(v, 1, 1).iter().all(|&s| s == 140),
            "V bottom-right flat 140"
        );

        // The bottom-left superblock chroma is SMOOTH_PRED over a real gradient
        // (not flat), so the above-right sentinel actually shapes the prediction.
        let u_bl = quad(u, 1, 0);
        let v_bl = quad(v, 1, 0);
        assert!(
            u_bl.iter().collect::<std::collections::BTreeSet<_>>().len() > 1,
            "U bottom-left superblock must be a SMOOTH gradient, not flat"
        );
        assert!(
            v_bl.iter().collect::<std::collections::BTreeSet<_>>().len() > 1,
            "V bottom-left superblock must be a SMOOTH gradient, not flat"
        );
        // The bottom-left superblock's top edge is its own top-left neighbour
        // (110), but its decoded above-right is the top-right superblock (200);
        // the §7.13.2.1 above-right sentinel pulls the top-right corner toward
        // 200, so the bottom-left's top row rises above its own top edge.
        // `u_bl[0]` is the top-left corner (110); `u_bl[31]` is the top-right
        // corner of the bottom-left superblock.
        assert_eq!(
            u_bl[0], 110,
            "U bottom-left top-left corner == own top edge"
        );
        assert!(
            u_bl[31] > u_bl[0],
            "U bottom-left top-right corner must be pulled toward the above-right (200), proving the above-right read"
        );

        // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
        // raw output byte-for-byte (verified locally vs avmdec + dav2d).
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "42bd99faae1ac0acb15c3e24fbededd8fc670612d08987bebb8942de5f4f4874"
        );
    }

    #[test]
    fn full_sb_chroma_num4_above_right_matches_count_top_right_avail() {
        // 128x128 (mi_cols = 32), full 64x64 superblock (n4w = 16), 4:2:0
        // (sub_x = 1) -> chroma w4 = 8. The bottom-left superblock (c = 0) has an
        // in-frame decoded above-right (the top-right superblock): chroma above
        // row decoded out to (32 - 0) >> 1 = 16 columns, so columns 8..15 are all
        // decoded -> num4AboveRight = 8 (capped at w4).
        assert_eq!(full_sb_chroma_num4_above_right(0, 16, 32, 1), 8);
        // The rightmost superblock (c = 16) has no in-frame above-right: chroma
        // above row decoded out to (32 - 16) >> 1 = 8 columns, so columns 8..15
        // are all undecoded -> num4AboveRight = 0 (and the §7.13.2.1 clamp /
        // no-above fallback applies).
        assert_eq!(full_sb_chroma_num4_above_right(16, 16, 32, 1), 0);
        // A single-column frame (mi_cols = 16) has only one superblock per row, so
        // the rightmost (only) superblock at c = 0 has no above-right.
        assert_eq!(full_sb_chroma_num4_above_right(0, 16, 16, 1), 0);
        // A 3-wide grid (mi_cols = 48): the middle superblock (c = 16) still has a
        // decoded above-right (the right superblock): decoded out to
        // (48 - 16) >> 1 = 16 columns, columns 8..15 decoded -> 8.
        assert_eq!(full_sb_chroma_num4_above_right(16, 16, 48, 1), 8);
    }

    // Single-block non-DC intra: a 64x64 vertical-gradient luma block the encoder
    // codes as SMOOTH_V_PRED (DC chroma). The decoder builds the §7.13.2.13
    // vertical smooth prediction over the §7.13.2.1 no-neighbour fallback edges
    // and adds the AC residual. avmdec and dav2d agree on the decoded output.
    const VSMOOTH_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-vsmooth-intra-64x64-q120.ivf");
    // Companion single-block SMOOTH_H_PRED (horizontal-gradient) block.
    const HSMOOTH_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-hsmooth-intra-64x64-q120.ivf");

    fn decode_general_intra_luma(fixture: &[u8]) -> DecodedFrame<u8> {
        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(fixture, options).expect("plan");
        decode_minimal_frame_from_plan(fixture, options, &plan)
            .expect("decode")
            .frame
    }

    #[test]
    fn vsmooth_single_block_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let frame = decode_general_intra_luma(VSMOOTH_FIXTURE);
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        let y = frame.y().samples();
        // SMOOTH_V over a vertical gradient: each row is constant across columns,
        // and the gradient increases top-to-bottom (proving the non-DC prediction
        // plus AC residual ran, not a flat DC level).
        assert!(
            y[0..64].iter().all(|&s| s == y[0]),
            "top row should be constant across columns"
        );
        assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
        let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct > 4, "luma should be a non-flat reconstruction");
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
        // raw output byte-for-byte (verified locally).
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "3aebe2eb215d4878bbc40aa2f97e2178b6140ef51c03afaaae478e69dbbf6bcd"
        );
    }

    #[test]
    fn hsmooth_single_block_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let frame = decode_general_intra_luma(HSMOOTH_FIXTURE);
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        let y = frame.y().samples();
        // SMOOTH_H over a horizontal gradient: each column is constant across rows,
        // and the gradient increases left-to-right.
        assert!(
            (0..64).all(|r| y[r * 64] == y[0]),
            "left column should be constant across rows"
        );
        assert!(y[0] < y[63], "luma should increase left-to-right");
        let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct > 4, "luma should be a non-flat reconstruction");
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "cfc6debd26760cdebf1d1a4497792461f0f68bc7e7773741ddf2cbc34561e702"
        );
    }

    // Single-block directional intra: a 64x64 block the encoder codes as the
    // § 5.20.5.3 `y_mode_offset` escape (`y_mode_set == 0`,
    // `y_mode_index == MODE_INDEX_COUNT - 1`, `y_mode_offset == 3`), which
    // reconstructs `D135_PRED` (pAngle 135, `AngleDeltaY == 0`). The decoder
    // builds the § 7.13.2.8 directional prediction over the § 7.13.2.1
    // no-neighbour fallback edges and adds the residual. avmdec and dav2d agree on
    // the decoded output (md5 1179bcc873c1d1ac49c2c032f11ca44d, DC chroma).
    const HEDGE_DIR_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-hedge-intra-64x64-q80.ivf");

    #[test]
    fn hedge_directional_d135_intra_frame_decodes_to_oracle() {
        use splot_recon::{BitDepth, PixelFormat, PlaneSize};

        let frame = decode_general_intra_luma(HEDGE_DIR_FIXTURE);
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

        // The D135 directional prediction over a top/bottom split residual: the
        // top half reconstructs near 40 and the bottom half near 210 (a genuinely
        // non-flat reconstruction, not a single DC level). Chroma is flat DC.
        let y = frame.y().samples();
        assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
        let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct > 4, "luma should be a non-flat reconstruction");
        assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
        assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

        // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
        // raw output byte-for-byte (verified locally; md5
        // 1179bcc873c1d1ac49c2c032f11ca44d).
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash,
            "b15f267ec6e99ca4d96a70f38bffe5f798ee4c33ad3aaec23761a1ea74b0be33"
        );
    }
}
