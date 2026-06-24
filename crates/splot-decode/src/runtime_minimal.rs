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
use splot_core::ivf::{IvfHeader, IvfWarning};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, ParsedIvfBitstream, parse_bitstream_partial};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};
use splot_core::types::ObuType;
use splot_recon::{DecodedFrame, DecodedFrameHashInput, IntraCardinalDirection};

use crate::error::{DecodeError, DecodeUnsupportedFeature, Result};
use crate::tile_payload::{
    FrameCandidateCdfFacts, FrameCandidateCoeffFacts, FrameCandidateTileBoundaryError,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, GeneralIntraBlockModeError,
    GeneralIntraResidualError, MinimalBlockSymbolTraceError,
    MinimalRuntimeBlockSymbolFrontierError, MinimalRuntimePartitionFrontierError,
    MinimalRuntimeReconstructionTrace, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    TileGroupPositionFacts, TilePartitionTraversalError,
};
use crate::{DecodeLimitName, DecodeOptions, DecodePlannedObu, DecodeStreamPlan};

use self::limits::{checked_add, decoded_frame_byte_budget};
use self::wienerns_lr::ensure_wienerns_lr_unit_runtime_frontier;
#[cfg(test)]
use self::wienerns_lr::{
    WIENER_NS_CHROMA_SOURCE_TAP_COUNT, WienerNsLrClassifiedWienerValueSourceSample,
    WienerNsLrFilterClassValue, WienerNsLrSourceReadConfig, WienerNsLrSourceReadFrontier,
    WienerNsLrSourceReadSample, WienerNsLrTxSkipLookup,
    derive_wienerns_lr_classified_wiener_frontier,
    derive_wienerns_lr_classified_wiener_values_frontier,
    derive_wienerns_lr_runtime_source_frontiers, derive_wienerns_lr_source_read_frontier,
    map_wienerns_lr_unit_frontier_error, record_wienerns_lr_chroma_luma_source_reads,
    wienerns_lr_classified_wiener_values_runtime_error, wienerns_lr_source_read_config,
    wienerns_lr_source_read_runtime_error,
};

/// Stable id for the first supported runtime decode tier.
pub const MINIMAL_INTRA_HASH_TIER_ID: &str = "minimal-intra-8bit420-hash-v1";

const FEATURE_ID: &str = "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS";
const MATRIX_ROW: &str = "minimal-decode-tier-contract";
const SPEC_SECTION: &str = "7.1";
const REMEDIATION: &str = "Use a stream inside minimal-intra-8bit420-hash-v1 or wait for the referenced decoder support row.";
const AC0EJ3_10BIT_FEATURE_ID: &str = "DECODE-AC0EJ3-10BIT-SEQUENCE-FRONTIER";
const AC0EJ3_10BIT_MATRIX_ROW: &str = "ac0ej3-10bit-sequence-frontier";
const AC0EJ3_CHROMA_FEATURE_ID: &str = "DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER";
const AC0EJ3_CHROMA_MATRIX_ROW: &str = "ac0ej3-sequence-chroma-frontier";
const AC0EJ3_WIENERNS_FEATURE_ID: &str = "DECODE-AC0EJ3-WIENERNS-FRONTIER";
const AC0EJ3_WIENERNS_MATRIX_ROW: &str = "ac0ej3-wienerns-frontier";
const AC0EJ3_LR_UNIT_SELECTIONS_FEATURE_ID: &str = "DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER";
const AC0EJ3_LR_UNIT_SELECTIONS_MATRIX_ROW: &str = "ac0ej3-lr-unit-selections-frontier";
const AC0EJ3_LR_SOURCE_READ_FEATURE_ID: &str = "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER";
const AC0EJ3_LR_SOURCE_READ_MATRIX_ROW: &str = "ac0ej3-lr-source-read-frontier";
const AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_FEATURE_ID: &str =
    "DECODE-AC0EJ3-LR-CLASSIFIED-WIENER-VALUES";
const AC0EJ3_LR_CLASSIFIED_WIENER_VALUES_MATRIX_ROW: &str = "ac0ej3-lr-classified-wiener-values";
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

impl MinimalRuntimeFrame {
    /// Borrows the decoded frame for output / hashing / §7.23 reference retention.
    pub(crate) fn frame(&self) -> &DecodedFrame<u8> {
        &self.frame
    }
}

/// Decodes the leading closed-loop-key frame into a single [`MinimalRuntimeFrame`].
///
/// This is the frozen intra-tier convenience entry for the intra runtime tests. It
/// delegates to the multi-frame driver and returns the first (key) frame; for a
/// single-frame intra stream the result is byte-identical to the prior single-frame
/// behavior. Production output adapters call [`decode_minimal_frames_from_plan`].
#[cfg(test)]
pub(crate) fn decode_minimal_frame_from_plan(
    bytes: &[u8],
    options: DecodeOptions,
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

/// Decodes every frame candidate the planner accepted (one key frame, optionally
/// followed by inter frames), emitting one [`MinimalRuntimeFrame`] per displayed
/// frame in output order (AV2 § 5.2.1, § 6.18).
///
/// Feature tracking: `DECODE-FIRST-INTER-FRAME-FRONTIER` (the inter frame loop and
/// reference retention), layered on `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.
pub(crate) fn decode_minimal_frames_from_plan(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<MinimalRuntimeFrame>> {
    decode_minimal_frames_from_plan_with_ivf_preflight(bytes, options, plan, |_| Ok(()))
}

/// Decodes one closed-loop-key frame candidate into a [`MinimalRuntimeFrame`].
///
/// This is the per-key-frame body shared by the single-frame frozen-tier entry and
/// the multi-frame runtime loop. It routes to the general intra path or the frozen
/// hash tier exactly as the single-frame entry historically did.
fn decode_minimal_key_frame(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    header: IvfHeader,
) -> Result<MinimalRuntimeFrame> {
    let core = parse_frame_core(frame_envelope, sequence)?;
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
                "minimal runtime hash support requires exactly one traced tile work unit",
            ));
        }
    };
    let reconstruction_trace =
        verify_flat_minimal_tile_trace(tile, sequence, &core, options.limits())?;
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

/// Multi-frame minimal-tier runtime driver (AV2 § 5.2.1, § 5.19, § 6.18).
///
/// Decodes the leading closed-loop-key frame, then walks any further
/// inter frame candidates in planned OBU stream order. IVF records are treated as
/// container payload groups and are not required to map one-to-one to decoded
/// frames. Each displayed frame becomes one
/// [`MinimalRuntimeFrame`]; the key frame's decoded planes are retained as the
/// reference state the inter frame consumes (§ 7.23). A single-frame intra stream
/// still yields a one-element vector, byte-identical to the single-frame entry.
///
/// Feature tracking: `DECODE-FIRST-INTER-FRAME-FRONTIER`.
pub(crate) fn decode_minimal_frames_from_plan_with_ivf_preflight(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    preflight: impl FnOnce(IvfHeader) -> Result<()>,
) -> Result<Vec<MinimalRuntimeFrame>> {
    ensure_multiframe_plan_shape(plan)?;
    let parsed = parse_bitstream_partial(bytes);
    let (ivf, header) = require_multiframe_ivf(&parsed)?;
    preflight(header)?;

    // The sequence header lives in the first IVF frame's OBU stream; it activates for
    // every subsequent frame in the stream (AV2 § 7.2.1).
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

    // Parse-only key-frame checks are safe before the 8-bit sample-storage boundary:
    // they do not allocate `DecodedFrame<u8>` or emit output, and they let real mission
    // streams fail at the next true header/tool frontier instead of at sequence flags.
    let key_core = parse_frame_core(key_envelope, &sequence)?;
    ensure_intra_header_complete(&key_core, key_envelope.offset)?;
    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().ok_or_else(|| {
        unsupported(
            "missing_frame_candidate",
            None,
            "minimal tier requires one selected key frame candidate",
        )
    })?;
    ensure_wienerns_lr_unit_runtime_frontier(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        &sequence,
        &key_core,
    )?;
    ensure_sequence_chroma_tools_before_tile_decode(&sequence, sequence_envelope.offset)?;
    ensure_8bit_runtime_storage(&sequence, sequence_envelope.offset)?;
    // This structural gate intentionally runs after the parse-only header/tool
    // checks above: later leading OBUs are outside `key_envelope`, so malformed
    // key headers report their true frontier while otherwise-supported streams
    // still reject extra leading OBUs before allocation or output.
    reject_extra_leading_key_payload_obus(leading_obus)?;

    // §7.23 reference-frame buffer over the sequence's NumRefFrames active slots. Each
    // decoded frame's refresh_frame_flags refreshes the named slots (the key frame's
    // §7.20 allFrames mask, then each inter frame's selected slot), so a later inter
    // frame can rank up to two valid references via the §7.7 implicit map.
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
    let mut retained_frame_bytes = 0;
    let mut next_unvalidated_following_ivf_record = 1;
    ensure_output_frame_count_limit(options.limits(), 1)?;
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
    // §7.20 / §7.23: retain the decoded key frame in the slots its refresh_frame_flags
    // names (a CLK key uses allFrames, so §7.23 :14100 marks only slot 0 valid).
    reference.update(
        0,
        frame_ref_update_from_core(&key_core, key_envelope.offset)?,
    );

    // Decode each following inter `OBU_REGULAR_TILE_GROUP` frame in stream order,
    // retaining each into the §7.23 buffer so a later frame can reference it. The
    // verified subset admits the single-reference (non-compound) NEARMV / GLOBALMV /
    // NEWMV path with NumTotalRefs ∈ {1, 2}; anything else (a second key frame, a
    // compound or NumTotalRefs > 2 frame) is rejected with a structured diagnostic
    // before any output. Tracked by `DECODE-INTER-MULTIREF-RUNTIME` (layered on
    // `DECODE-FIRST-INTER-FRAME-FRONTIER`).
    // VERIFIED-SUBSET DISCIPLINE (CDF inheritance): the decoder decodes every frame
    // from the default (init_*_cdfs) entropy state and does NOT model the §7.23
    // save_cdfs / §5 load_cdfs cross-frame CDF flow. Each refreshed slot records (per
    // slot) whether its stored frame ADAPTED its CDFs (disable_cdf_update == 0) and its
    // RefFrameType; decode_minimal_inter_frame resolves §5 set_primary_ref_frame_and_ctx
    // (including the PRIMARY_REF_CHOOSE resolution) and rejects ONLY when the RESOLVED
    // loaded slot adapted. The committed fixtures resolve to PRIMARY_REF_NONE or a
    // non-adapted slot, so nothing is rejected.
    // The key frame's §7.23 adaptation state is already recorded in the reference buffer
    // by `reference.update(0, ...)` above (via `frame_ref_update_from_core`).
    for next_candidate in candidates {
        match next_candidate.obu_type() {
            ObuType::RegularTileGroup => {
                let next_output_frame_count =
                    checked_add(DecodeLimitName::MaxOutputFrames, frames.len() as u64, 1)?;
                ensure_output_frame_count_limit(options.limits(), next_output_frame_count)?;
                let inter_envelope = following_inter_envelope(
                    ivf,
                    next_candidate,
                    &mut next_unvalidated_following_ivf_record,
                )?;
                // VERIFIED-SUBSET DISCIPLINE: the §7.7 ranking + single_ref wiring is
                // proven bit-exact only for up to TWO valid reference slots (NumTotalRefs
                // ∈ {1, 2}). Reject before output if the buffer already holds more — a
                // three-valid-slot §7.7 ranking / multi-decision single_ref is unfixtured.
                if reference.valid_count() > 2 {
                    return Err(unsupported_at(
                        "inter_too_many_valid_references",
                        next_candidate.offset(),
                        "minimal multi-reference decode is verified only for up to two valid reference slots; a third valid slot needs a richer §7.7 ranking / multi-decision single_ref that is not yet fixtured",
                    ));
                }
                // The P2 order-hint wrap guard (the stored RefOrderHint is the unwrapped
                // OrderHint only while the GOP fits in one OrderHintBits window) is applied
                // inside decode_minimal_inter_frame, which has the parsed order_hint_lsb and
                // the reference history, and rejects BEFORE any output.
                // The §7.23 cross-frame CDF-inheritance reject (a frame whose §5-resolved
                // primary_ref_frame loads a prior adapted slot's CDFs, which the decoder
                // does not model) is applied inside decode_minimal_inter_frame, which has the
                // parsed primary_ref_frame / disable_cross_frame_cdf_init / ref_frame_idx and
                // rejects BEFORE the tile entropy decode.
                // Build the §7.23 reference store from the frames decoded so far, then
                // decode this inter frame over it. `build_store` borrows `frames`; the
                // borrow ends when the inter decode returns its owned frame.
                let (store, meta) = reference.build_store(&frames)?;
                let inter_state = inter::InterReferenceState {
                    store: &store,
                    ref_valid: meta.ref_valid,
                    ref_order_hint: meta.ref_order_hint,
                    ref_frame_width: meta.ref_frame_width,
                    ref_frame_height: meta.ref_frame_height,
                    ref_base_q_idx: meta.ref_base_q_idx,
                    ref_is_inter: meta.ref_is_inter,
                    ref_adapted: meta.ref_adapted,
                };
                let inter_core = inter::parse_validated_inter_frame_core(
                    inter_envelope,
                    &sequence,
                    &inter_state,
                )?;
                ensure_retained_frame_byte_limits_for_core(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_core,
                    inter_envelope.offset,
                )?;
                let (inter_frame, inter_core) = inter::decode_minimal_inter_frame(
                    plan,
                    next_candidate,
                    bytes,
                    inter_envelope,
                    inter_core,
                    &sequence,
                    options,
                    header,
                    &inter_state,
                )?;
                let next_retained_frame_bytes = ensure_retained_frame_byte_limits(
                    options.limits(),
                    retained_frame_bytes,
                    &inter_frame,
                )?;
                drop(store);
                let frame_index = frames.len();
                frames.push(inter_frame);
                retained_frame_bytes = next_retained_frame_bytes;
                reference.update(
                    frame_index,
                    frame_ref_update_from_core(&inter_core, inter_envelope.offset)?,
                );
            }
            _ => {
                return Err(unsupported_at(
                    "multiple_frames_unimplemented",
                    next_candidate.offset(),
                    "minimal tier decodes a key frame followed by single-reference inter frames; a following frame candidate outside that runtime subset (for example, a second key frame or TIP frame) is admitted at the planner but decoding it is not yet implemented",
                ));
            }
        }
    }

    Ok(frames)
}

/// Resolves the `OBU_REGULAR_TILE_GROUP` envelope for a following inter frame by
/// planned OBU offset. An IVF frame record is only a non-normative byte envelope:
/// before each candidate is decoded, the runtime lazily validates every
/// following IVF payload up to and including the candidate's payload as complete
/// `[TD, OBU_REGULAR_TILE_GROUP]` pairs. That catches candidate-less state OBUs
/// before they can make a later frame decode against stale sequence state, while
/// malformed future records do not pre-empt an earlier candidate's runtime gate.
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
        let Some(td_envelope) = position
            .checked_sub(1)
            .and_then(|previous| ivf_frame.obus.get(previous))
            .copied()
        else {
            return Err(unsupported_at(
                "missing_inter_temporal_delimiter",
                candidate.offset(),
                "minimal tier requires each following inter frame candidate to be immediately preceded by OBU_TEMPORAL_DELIMITER in its IVF payload",
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

fn require_following_ivf_obu_order_through(
    ivf: &ParsedIvfBitstream<'_>,
    next_unvalidated_following_ivf_record: &mut usize,
    target_ivf_frame_index: usize,
) -> Result<()> {
    let validation_end = target_ivf_frame_index.saturating_add(1);
    for frame in ivf
        .frames
        .iter()
        .take(validation_end)
        .skip(*next_unvalidated_following_ivf_record)
    {
        require_inter_obu_order(frame.obus.as_slice())?;
    }
    *next_unvalidated_following_ivf_record =
        (*next_unvalidated_following_ivf_record).max(validation_end);
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
                "minimal tier requires following inter IVF payloads to contain only [OBU_TEMPORAL_DELIMITER, OBU_REGULAR_TILE_GROUP] frame units before each inter candidate",
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
            "minimal tier requires following inter IVF payloads to contain complete [OBU_TEMPORAL_DELIMITER, OBU_REGULAR_TILE_GROUP] frame units",
        ));
    }
    Ok(())
}

/// Validates the planned stream shape for the multi-frame runtime: one accepted
/// frame candidate for a single intra key, followed by zero or more frame candidates
/// the runtime will attempt in stream order. The shape stays the minimal tier's:
/// one base-layer sequence header, and enough planned OBUs for the leading
/// `[TD, SEQ, CLK]` key frame. Container warning policy is enforced by
/// `require_multiframe_ivf`, which permits only terminal trailing partial IVF
/// headers. Each following
/// `OBU_REGULAR_TILE_GROUP` is validated by `following_inter_envelope`; non-inter
/// frame candidates fail closed before output because every following IVF payload
/// must contain only complete `[OBU_TEMPORAL_DELIMITER, OBU_REGULAR_TILE_GROUP]`
/// frame-unit pairs. Unverified reference/tool states fail at their precise
/// runtime gates before caller-visible output.
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

/// Container-shape gate for the multi-frame runtime: an `AV02` IVF with positive-sized
/// records, no fatal container errors, and only terminal trailing partial IVF
/// header warnings. The IVF header's `frame_count` is a container-record count
/// when present (and is often zero in real streams), not the AV2 decoded
/// frame-candidate count.
fn require_multiframe_ivf<'a>(
    parsed: &'a ParsedBitstream<'a>,
) -> Result<(&'a ParsedIvfBitstream<'a>, IvfHeader)> {
    let ParsedBitstream::Ivf(ivf) = parsed else {
        return Err(unsupported(
            "non_ivf_input",
            None,
            "minimal runtime support currently accepts only the committed IVF fixture shape",
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
            "minimal tier requires positive-sized AV02 IVF frame records with no fatal container errors and only terminal trailing partial-frame-header warnings; declared IVF frame_count must be zero or match the parsed record count",
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
    let frame_bytes = decoded_frame_byte_budget(frame_size).map(|budget| budget.decoded_bytes)?;
    ensure_retained_frame_byte_limits_for_bytes(limits, retained_frame_bytes, frame_bytes)
}

fn ensure_retained_frame_byte_limits_for_bytes(
    limits: crate::DecodeLimits,
    retained_frame_bytes: u64,
    frame_bytes: u64,
) -> Result<u64> {
    let next_retained_frame_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        retained_frame_bytes,
        frame_bytes,
    )?;
    // The runtime report retains every decoded frame, so this is a
    // conservative upper bound for the live reference-store bytes.
    limits.ensure(
        DecodeLimitName::MaxReferenceStoreBytes,
        next_retained_frame_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, next_retained_frame_bytes)?;
    Ok(next_retained_frame_bytes)
}

fn retained_decoded_frame_bytes(frame: &MinimalRuntimeFrame) -> Result<u64> {
    let byte_len = DecodedFrameHashInput::new(frame.frame()).byte_len()?;
    Ok(byte_len as u64)
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
        | MinimalRuntimePartitionFrontierError::IntraJointModeState(_)
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
    // A multi-frame stream (key + inter) carries a non-single-picture sequence
    // header (`single_picture_header_flag == 0`), so this is admitted here: the
    // per-frame header parse and the frame-core validation downstream
    // (`validate_frame_core` / `is_general_minimal_intra`) gate the actual decode
    // shape, and a single-picture sequence is just the one-frame case. Frame-header
    // bit layout differences (e.g. the `frame_size_override_flag` read) are handled
    // by `parse_frame_header_core` and proven bit-exact by the per-frame decode.
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
    if intra.enable_cfl_intra {
        return Err(unsupported_feature_at(
            "unsupported_cfl_intra",
            offset,
            "minimal tier parses the sequence CFL flag but still rejects before §5.20.5.6 is_cfl / UV_CFL_PRED mode-info syntax can be skipped",
            AC0EJ3_CHROMA_MATRIX_ROW,
            AC0EJ3_CHROMA_FEATURE_ID,
            "5.20.5.6",
        ));
    }
    if intra.enable_mhccp {
        return Err(unsupported_feature_at(
            "unsupported_mhccp",
            offset,
            "minimal tier parses the sequence MHCCP flag but still rejects before §5.20.5.6 is_mhccp_allowed mode-info syntax can be skipped",
            AC0EJ3_CHROMA_MATRIX_ROW,
            AC0EJ3_CHROMA_FEATURE_ID,
            "5.20.5.6",
        ));
    }
    Ok(())
}

fn ensure_8bit_runtime_storage(sequence: &SequenceHeader, offset: ByteOffset) -> Result<()> {
    if sequence.general.bit_depth_idc != BitDepthIdc::Eight {
        return Err(unsupported_feature_at(
            "unsupported_bit_depth",
            offset,
            "minimal runtime parses 10-bit sequences but still stores decoded samples as 8-bit",
            AC0EJ3_10BIT_MATRIX_ROW,
            AC0EJ3_10BIT_FEATURE_ID,
            "6.4.1",
        ));
    }
    Ok(())
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

/// Extracts the AV2 § 7.23 reference frame update inputs from a parsed frame header.
///
/// The stored `RefOrderHint[i]` is the UNWRAPPED `OrderHint` (`get_disp_order_hint()`,
/// mirror :4375 / § 7.23 :14123), not the raw `OrderHintLsbs`. The minimal multi-frame
/// subset is admitted only when the GOP never wraps `OrderHintBits` (enforced by
/// [`order_hint_history_unwrapped`] before any inter decode), so within the admitted
/// subset `OrderHint == OrderHintLsbs` exactly; a wrapping history is rejected rather
/// than stored with a stale small LSB value that a § 7.7 /
/// `choose_primary_secondary_ref_frame` ranking would mis-order. There is no
/// segmentation / delta-Q so `qindex == base_q_idx`. The caller applies the refresh into
/// the [`reference_buffer::RuntimeReferenceBuffer`].
fn frame_ref_update_from_core(
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<reference_buffer::FrameRefUpdate> {
    let refresh_frame_flags = core.refresh_frame_flags.ok_or_else(|| {
        unsupported_at(
            "missing_refresh_frame_flags",
            offset,
            "minimal multi-frame decode requires a parsed refresh_frame_flags for the §7.23 update",
        )
    })?;
    // The admitted (non-wrapping) subset makes OrderHint == OrderHintLsbs exactly; a
    // wrapping history is rejected up front, so storing the LSBs here is the unwrapped
    // OrderHint for every admitted stream.
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
    // §7.23 :14110 RefFrameType[i] = FrameType: the minimal subset stores either the CLK
    // KEY_FRAME or an INTER_FRAME, so `!is_key_frame` is exactly RefFrameType == INTER_FRAME
    // (the §5 choose_primary_secondary_ref_frame candidate filter). A SWITCH frame is not
    // admitted here (the validators require a non-key inter frame or the CLK key).
    let is_inter = !core.is_key_frame;
    // Whether this frame ADAPTED its CDFs (disable_cdf_update == 0): recorded per slot so a
    // later frame's cross-frame CDF-load reject keys on the RESOLVED loaded slot.
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
    })
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
            "minimal runtime reached an unsupported read_wienerns_filter() (§5.20.10.6) branch from AV2 §5.18.7.11 lr_params() before loop-restoration params could be modeled completely",
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

mod general_intra;
mod inter;
mod limits;
mod reference_buffer;
mod wienerns_lr;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod general_intra_tests;

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

/// Derives the inter frame's tile-payload plan (DECODE-FIRST-INTER-FRAME-FRONTIER).
///
/// Mirrors [`derive_tile_plan`] but uses the inter tile-facts derivation
/// ([`FrameCandidateTileFacts::from_inter_frame_core`]) so the geometry / base_q_idx
/// / disable_cdf_update / coefficient facts are read from the parsed §5.18.2 inter
/// header (`InterHeaderComplete`).
fn derive_inter_tile_plan<'a>(
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
    let facts = FrameCandidateTileFacts::from_inter_frame_core(core, coeff)
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
    limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
    limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
    let budget = decoded_frame_byte_budget(FrameSize::new(width, height))?;
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
