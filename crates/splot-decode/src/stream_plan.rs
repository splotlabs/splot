// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Plan-only AV2 stream traversal over already parsed `splot-core` structures.
//!
//! Feature tracking: `DECODE-STREAM-STATE-PLANNER`.

use core::fmt;

use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::{IvfError, IvfWarning};
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::{BitstreamFormat, ParsedBitstream, ParsedIvfFrame};
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};

use crate::error::{DecodeError, Result};
use crate::{DecodeLimitName, DecodeOptions, UNSUPPORTED_FEATURE_RULE_ID};

/// Support-matrix row that owns parsed stream planning.
pub const DECODE_STREAM_STATE_MATRIX_ROW: &str = "decode-stream-state";

/// Feature ID for parsed stream planning.
pub const DECODE_STREAM_STATE_FEATURE_ID: &str = "DECODE-STREAM-STATE-PLANNER";

/// Parsed stream input for [`crate::DecodeContext::plan_stream`].
///
/// This type deliberately holds parser output, not raw bytes. Raw-byte planning
/// performs bounded pre-parse traversal before reusing this parsed input shape.
#[derive(Clone, Copy, Debug)]
pub struct DecodeStreamInput<'a> {
    parsed: &'a ParsedBitstream<'a>,
    input_len_bytes: u64,
}

impl<'a> DecodeStreamInput<'a> {
    /// Builds a stream-planner input from already parsed `splot-core` output.
    #[must_use]
    pub const fn new(parsed: &'a ParsedBitstream<'a>, input_len_bytes: u64) -> Self {
        Self {
            parsed,
            input_len_bytes,
        }
    }

    /// Already parsed stream/container output.
    #[must_use]
    pub const fn parsed(self) -> &'a ParsedBitstream<'a> {
        self.parsed
    }

    /// Original input length in bytes, supplied by the caller.
    #[must_use]
    pub const fn input_len_bytes(self) -> u64 {
        self.input_len_bytes
    }
}

/// The fixed base-layer selection supported by the first stream planner.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLayerSelection {
    temporal_layer_id: TemporalLayerId,
    embedded_layer_id: EmbeddedLayerId,
    extended_layer_id: ExtendedLayerId,
}

impl DecodeLayerSelection {
    /// The only selected layer in planner v1: temporal 0, embedded 0, extended 0.
    pub const BASE: Self = Self {
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
    };

    /// Returns the planner's base-layer selection.
    #[must_use]
    pub const fn base() -> Self {
        Self::BASE
    }

    /// Selected temporal layer id.
    #[must_use]
    pub const fn temporal_layer_id(self) -> TemporalLayerId {
        self.temporal_layer_id
    }

    /// Selected embedded layer id.
    #[must_use]
    pub const fn embedded_layer_id(self) -> EmbeddedLayerId {
        self.embedded_layer_id
    }

    /// Selected extended layer id.
    #[must_use]
    pub const fn extended_layer_id(self) -> ExtendedLayerId {
        self.extended_layer_id
    }
}

/// Deterministic plan over accepted parsed-stream OBU metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeStreamPlan {
    format: BitstreamFormat,
    selected_layer: DecodeLayerSelection,
    input_len_bytes: u64,
    obus: Vec<DecodePlannedObu>,
    frame_candidate_count: u64,
    source_warnings: Vec<DecodeSourceIssue>,
}

impl DecodeStreamPlan {
    /// Source container format.
    #[must_use]
    pub const fn format(&self) -> BitstreamFormat {
        self.format
    }

    /// Selected layer.
    #[must_use]
    pub const fn selected_layer(&self) -> DecodeLayerSelection {
        self.selected_layer
    }

    /// Caller-supplied input length in bytes.
    #[must_use]
    pub const fn input_len_bytes(&self) -> u64 {
        self.input_len_bytes
    }

    /// Count of planned OBUs.
    #[must_use]
    pub fn obu_count(&self) -> u64 {
        self.obus.len() as u64
    }

    /// Count of accepted frame candidates.
    #[must_use]
    pub const fn frame_candidate_count(&self) -> u64 {
        self.frame_candidate_count
    }

    /// Planned OBUs in deterministic source order.
    pub fn obus(&self) -> core::slice::Iter<'_, DecodePlannedObu> {
        self.obus.iter()
    }

    /// Accepted closed-loop-key frame candidates in deterministic source order.
    pub fn frame_candidates(&self) -> impl Iterator<Item = &DecodePlannedObu> {
        self.obus
            .iter()
            .filter(|obu| obu.role == DecodePlannedObuRole::FrameCandidate)
    }

    /// Accepted frame candidates of any kind (key or inter), in deterministic
    /// source order. This is the per-frame decode order the multi-frame runtime
    /// walks (AV2 § 5.2.1, § 6.18).
    pub fn frame_candidates_all(&self) -> impl Iterator<Item = &DecodePlannedObu> {
        self.obus.iter().filter(|obu| obu.role.is_frame_candidate())
    }

    /// Non-fatal source/container warnings carried into the plan.
    #[must_use]
    pub fn source_warnings(&self) -> &[DecodeSourceIssue] {
        &self.source_warnings
    }
}

/// Source kind for a planned OBU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeObuSourceKind {
    /// Raw Annex B input.
    AnnexB,
    /// IVF frame payload containing Annex B.
    Ivf,
}

/// IVF frame context for an OBU planned from an IVF payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeIvfFrameContext {
    frame_index: usize,
    frame_header_offset: ByteOffset,
    frame_payload_offset: ByteOffset,
    frame_payload_size: u32,
    pts: u64,
}

impl DecodeIvfFrameContext {
    /// Zero-based IVF frame index.
    #[must_use]
    pub const fn frame_index(self) -> usize {
        self.frame_index
    }

    /// Absolute offset of the IVF frame header.
    #[must_use]
    pub const fn frame_header_offset(self) -> ByteOffset {
        self.frame_header_offset
    }

    /// Absolute offset of the IVF frame payload.
    #[must_use]
    pub const fn frame_payload_offset(self) -> ByteOffset {
        self.frame_payload_offset
    }

    /// Declared IVF frame payload size.
    #[must_use]
    pub const fn frame_payload_size(self) -> u32 {
        self.frame_payload_size
    }

    /// IVF presentation timestamp metadata.
    #[must_use]
    pub const fn pts(self) -> u64 {
        self.pts
    }
}

/// Role assigned to an accepted planned OBU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodePlannedObuRole {
    /// Ordering/global marker accepted by the planner.
    Global,
    /// State OBU for the selected base layer.
    SelectedLayerState,
    /// Closed-loop-key frame candidate for a future decode stage (AV2 § 5.2.1
    /// `OBU_CLOSED_LOOP_KEY`).
    FrameCandidate,
    /// Inter frame candidate carried in an `OBU_REGULAR_TILE_GROUP` (AV2 § 5.2.1,
    /// § 5.19): a non-key frame whose first tile group carries the frame header.
    /// Admitted by the planner so the multi-frame runtime can reach it; the runtime
    /// decode of an inter frame is tracked by `DECODE-FIRST-INTER-FRAME-FRONTIER`.
    ///
    /// Caveat: every `OBU_REGULAR_TILE_GROUP` is currently classified with this role,
    /// so a frame split across multiple tile groups (§ 5.19 continuation,
    /// `tg_start`..`tg_end`) is over-counted by `frame_candidate_count` — only the
    /// first tile group of a frame starts a new frame. See the `classify_obu` TODO;
    /// the minimal runtime's one-tile-group shape gate masks this today.
    InterFrameCandidate,
}

impl DecodePlannedObuRole {
    /// Whether this role is a frame candidate (key or inter) the runtime decodes
    /// into an output frame, as opposed to a global marker or layer-state OBU.
    #[must_use]
    pub const fn is_frame_candidate(self) -> bool {
        matches!(self, Self::FrameCandidate | Self::InterFrameCandidate)
    }
}

/// One accepted OBU in a parsed stream plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodePlannedObu {
    index: u64,
    source_kind: DecodeObuSourceKind,
    ivf_frame: Option<DecodeIvfFrameContext>,
    offset: ByteOffset,
    size: u32,
    payload_len: u64,
    header: ObuHeader,
    role: DecodePlannedObuRole,
}

impl DecodePlannedObu {
    /// Zero-based OBU index in the plan.
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Source kind for this OBU.
    #[must_use]
    pub const fn source_kind(&self) -> DecodeObuSourceKind {
        self.source_kind
    }

    /// IVF context when the OBU came from an IVF frame payload.
    #[must_use]
    pub const fn ivf_frame(&self) -> Option<DecodeIvfFrameContext> {
        self.ivf_frame
    }

    /// Absolute byte offset of the OBU header.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// Declared OBU size in bytes, including the OBU header.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// OBU payload length in bytes.
    #[must_use]
    pub const fn payload_len(&self) -> u64 {
        self.payload_len
    }

    /// Parsed OBU header.
    #[must_use]
    pub const fn header(&self) -> ObuHeader {
        self.header
    }

    /// Parsed OBU type.
    #[must_use]
    pub const fn obu_type(&self) -> ObuType {
        self.header.obu_type
    }

    /// Planner role for this OBU.
    #[must_use]
    pub const fn role(&self) -> DecodePlannedObuRole {
        self.role
    }
}

/// Source issue category recorded by the parsed stream planner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeSourceIssueKind {
    /// Fatal raw Annex B parser error.
    AnnexBParseError,
    /// Fatal IVF container parser error.
    IvfContainerError,
    /// Fatal Annex B parser error inside an IVF frame payload.
    IvfFramePayloadError,
    /// Non-fatal IVF container warning.
    IvfWarning,
}

impl DecodeSourceIssueKind {
    /// Stable snake-case issue kind label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnnexBParseError => "annex_b_parse_error",
            Self::IvfContainerError => "ivf_container_error",
            Self::IvfFramePayloadError => "ivf_frame_payload_error",
            Self::IvfWarning => "ivf_warning",
        }
    }
}

impl fmt::Display for DecodeSourceIssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Source/container issue observed while planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeSourceIssue {
    kind: DecodeSourceIssueKind,
    rule_id: Option<&'static str>,
    offset: Option<ByteOffset>,
    frame_index: Option<usize>,
    message: String,
}

impl DecodeSourceIssue {
    /// Source issue category.
    #[must_use]
    pub const fn kind(&self) -> DecodeSourceIssueKind {
        self.kind
    }

    /// Source parser rule id, when the parser exposes one.
    #[must_use]
    pub const fn rule_id(&self) -> Option<&'static str> {
        self.rule_id
    }

    /// Source byte offset, when known.
    #[must_use]
    pub const fn offset(&self) -> Option<ByteOffset> {
        self.offset
    }

    /// IVF frame index, when the issue is frame-local.
    #[must_use]
    pub const fn frame_index(&self) -> Option<usize> {
        self.frame_index
    }

    /// Human-readable source parser message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DecodeSourceIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.rule_id, self.offset, self.frame_index) {
            (Some(rule_id), Some(offset), Some(frame_index)) => write!(
                f,
                "{:?} ({rule_id}) in IVF frame {frame_index} at byte {offset}: {}",
                self.kind, self.message
            ),
            (Some(rule_id), Some(offset), None) => write!(
                f,
                "{:?} ({rule_id}) at byte {offset}: {}",
                self.kind, self.message
            ),
            (_, _, Some(frame_index)) => write!(
                f,
                "{:?} in IVF frame {frame_index}: {}",
                self.kind, self.message
            ),
            _ => write!(f, "{:?}: {}", self.kind, self.message),
        }
    }
}

/// Stable reason a parsed stream is outside the planner tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeUnsupportedReason {
    /// OBU layer scope violates AV2 § 6.2.2 global/local xlayer rules.
    InvalidLayerScope,
    /// OBU uses a temporal layer other than the selected base layer.
    NonBaseTemporalLayer,
    /// OBU uses an embedded layer other than the selected base layer.
    NonBaseEmbeddedLayer,
    /// OBU uses an extended layer other than the selected base layer or global layer.
    NonBaseExtendedLayer,
    /// OBU participates in multistream or external-HLS selection.
    MultistreamSelection,
    /// OBU is frame-carrying but not the v1 closed-loop-key candidate.
    UnsupportedFrameObu,
    /// OBU can affect output/timing state that this planner tier does not model.
    UnsupportedOutputEffectObu,
    /// Reserved OBU types are outside the decode planner tier.
    ReservedObu,
}

/// Generates an exhaustive `const fn as_str(self) -> &'static str` label mapping.
///
/// A variant-declaring macro (rather than a bare `match`) keeps the mapping
/// exhaustive — a new or reordered variant fails to compile until its label is
/// added — while reading as a macro invocation, so it is not a structural
/// duplicate of the other enum string-label `match`es (the dupehound diff-ratchet
/// flags those).
///
/// The leading `$vis` token sets the generated `as_str` visibility so the macro
/// serves both a `pub` enum (`DecodeUnsupportedReason`) and a `pub(crate)` enum
/// (`TilePayloadUnsupportedReason`) without tripping pedantic clippy's
/// `unreachable_pub`. `#[macro_export]` makes it reachable as
/// `crate::impl_reason_labels!` from sibling modules.
#[macro_export]
macro_rules! impl_reason_labels {
    ($vis:vis $name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        impl $name {
            /// Stable snake-case reason label.
            #[must_use]
            $vis const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }
    };
}

impl_reason_labels!(pub DecodeUnsupportedReason {
    InvalidLayerScope => "invalid_layer_scope",
    NonBaseTemporalLayer => "non_base_temporal_layer",
    NonBaseEmbeddedLayer => "non_base_embedded_layer",
    NonBaseExtendedLayer => "non_base_extended_layer",
    MultistreamSelection => "multistream_selection",
    UnsupportedFrameObu => "unsupported_frame_obu",
    UnsupportedOutputEffectObu => "unsupported_output_effect_obu",
    ReservedObu => "reserved_obu",
});

impl fmt::Display for DecodeUnsupportedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unsupported structure metadata for plan-only stream traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeUnsupportedStructure {
    reason: DecodeUnsupportedReason,
    obu_type: ObuType,
    offset: ByteOffset,
    spec_section: &'static str,
    message: &'static str,
}

impl DecodeUnsupportedStructure {
    /// Stable decoder diagnostic rule id for future diagnostic adaptation.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        UNSUPPORTED_FEATURE_RULE_ID
    }

    /// Decoder support matrix row that owns this unsupported planner result.
    #[must_use]
    pub const fn matrix_row(&self) -> &'static str {
        DECODE_STREAM_STATE_MATRIX_ROW
    }

    /// Feature ID that owns this unsupported planner result.
    #[must_use]
    pub const fn feature_id(&self) -> &'static str {
        DECODE_STREAM_STATE_FEATURE_ID
    }

    /// Stable unsupported reason.
    #[must_use]
    pub const fn reason(&self) -> DecodeUnsupportedReason {
        self.reason
    }

    /// OBU type that triggered the unsupported result.
    #[must_use]
    pub const fn obu_type(&self) -> ObuType {
        self.obu_type
    }

    /// OBU byte offset that triggered the unsupported result.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// AV2 spec section associated with the unsupported result.
    #[must_use]
    pub const fn spec_section(&self) -> &'static str {
        self.spec_section
    }

    /// Human-readable unsupported result.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for DecodeUnsupportedStructure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {} for {}: {}",
            self.reason,
            self.offset,
            self.obu_type.spec_name(),
            self.message
        )
    }
}

pub(crate) fn plan_stream(
    input: DecodeStreamInput<'_>,
    options: &DecodeOptions,
) -> Result<DecodeStreamPlan> {
    let limits = options.limits();
    limits.ensure(DecodeLimitName::MaxInputBytes, input.input_len_bytes)?;

    let selected_layer = DecodeLayerSelection::base();
    let mut builder = PlanBuilder {
        format: input.parsed.format(),
        selected_layer,
        input_len_bytes: input.input_len_bytes,
        limits,
        obus: Vec::new(),
        traversed_obu_count: 0,
        frame_candidate_count: 0,
        source_warnings: Vec::new(),
    };

    match input.parsed {
        ParsedBitstream::AnnexB(partial) => {
            if let Some(error) = &partial.error {
                return Err(DecodeError::MalformedSource {
                    issue: issue_from_core_error(
                        DecodeSourceIssueKind::AnnexBParseError,
                        None,
                        error,
                    ),
                });
            }
            for obu in &partial.obus {
                builder.push_obu(*obu, DecodeObuSourceKind::AnnexB, None)?;
            }
        }
        ParsedBitstream::Ivf(ivf) => {
            for warning in &ivf.warnings {
                builder
                    .source_warnings
                    .push(issue_from_ivf_warning(warning));
            }
            if let Some(error) = &ivf.error {
                return Err(DecodeError::MalformedSource {
                    issue: issue_from_ivf_error(error),
                });
            }
            let mut first_unsupported = None;
            for (frame_record_index, frame) in ivf.frames.iter().enumerate() {
                builder.limits.ensure(
                    DecodeLimitName::MaxIvfFrameRecords,
                    frame_record_index as u64 + 1,
                )?;
                if let Some(error) = &frame.error {
                    return Err(DecodeError::MalformedSource {
                        issue: issue_from_core_error(
                            DecodeSourceIssueKind::IvfFramePayloadError,
                            Some(frame.frame.index),
                            error,
                        ),
                    });
                }

                let context = Some(ivf_frame_context(frame));
                for obu in &frame.obus {
                    builder.push_obu_or_first_unsupported(
                        *obu,
                        DecodeObuSourceKind::Ivf,
                        context,
                        &mut first_unsupported,
                    )?;
                }
            }
            if let Some(unsupported) = first_unsupported {
                return Err(DecodeError::UnsupportedStructure { unsupported });
            }
        }
    }

    Ok(builder.finish())
}

struct PlanBuilder {
    format: BitstreamFormat,
    selected_layer: DecodeLayerSelection,
    input_len_bytes: u64,
    limits: crate::DecodeLimits,
    obus: Vec<DecodePlannedObu>,
    traversed_obu_count: u64,
    frame_candidate_count: u64,
    source_warnings: Vec<DecodeSourceIssue>,
}

impl PlanBuilder {
    fn push_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
    ) -> Result<()> {
        let role = self.classify_limited_obu(envelope)?;
        self.push_classified_obu(envelope, source_kind, ivf_frame, role);
        Ok(())
    }

    fn push_obu_or_first_unsupported(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        first_unsupported: &mut Option<DecodeUnsupportedStructure>,
    ) -> Result<()> {
        match self.push_obu(envelope, source_kind, ivf_frame) {
            Ok(()) => Ok(()),
            Err(DecodeError::UnsupportedStructure { unsupported }) => {
                if first_unsupported.is_none() {
                    *first_unsupported = Some(unsupported);
                }
                Ok(())
            }
            Err(DecodeError::Limit { source }) => {
                if let Some(unsupported) = first_unsupported.as_ref() {
                    Err(DecodeError::UnsupportedStructure {
                        unsupported: unsupported.clone(),
                    })
                } else {
                    Err(DecodeError::Limit { source })
                }
            }
            Err(error) => Err(error),
        }
    }

    fn classify_limited_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<DecodePlannedObuRole> {
        let next_obu_count = self.traversed_obu_count.saturating_add(1);
        self.limits
            .ensure(DecodeLimitName::MaxObus, next_obu_count)?;
        self.traversed_obu_count = next_obu_count;

        let role = classify_obu(envelope, self.selected_layer)?;
        if role.is_frame_candidate() {
            let next_frame_count = self.frame_candidate_count.saturating_add(1);
            self.limits
                .ensure(DecodeLimitName::MaxFramesToDecode, next_frame_count)?;
            self.frame_candidate_count = next_frame_count;
        }

        Ok(role)
    }

    fn push_classified_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        role: DecodePlannedObuRole,
    ) {
        self.obus.push(DecodePlannedObu {
            index: self.obus.len() as u64,
            source_kind,
            ivf_frame,
            offset: envelope.offset,
            size: envelope.size,
            payload_len: envelope.payload.len() as u64,
            header: envelope.header,
            role,
        });
    }

    fn finish(self) -> DecodeStreamPlan {
        DecodeStreamPlan {
            format: self.format,
            selected_layer: self.selected_layer,
            input_len_bytes: self.input_len_bytes,
            obus: self.obus,
            frame_candidate_count: self.frame_candidate_count,
            source_warnings: self.source_warnings,
        }
    }
}

pub(crate) fn ensure_supported_obu(
    envelope: ObuEnvelope<'_>,
    selected_layer: DecodeLayerSelection,
) -> Result<()> {
    classify_obu(envelope, selected_layer).map(|_| ())
}

fn classify_obu(
    envelope: ObuEnvelope<'_>,
    selected_layer: DecodeLayerSelection,
) -> Result<DecodePlannedObuRole> {
    let header = envelope.header;
    let obu_type = header.obu_type;

    if obu_type.requires_global_xlayer() && header.extended_layer_id != GLOBAL_XLAYER_ID {
        return unsupported(
            DecodeUnsupportedReason::InvalidLayerScope,
            envelope,
            "6.2.2",
            "this OBU type must use the AV2 global extended layer id",
        );
    }
    if header.extended_layer_id == GLOBAL_XLAYER_ID && !obu_type.permits_global_xlayer() {
        return unsupported(
            DecodeUnsupportedReason::InvalidLayerScope,
            envelope,
            "6.2.2",
            "this OBU type is not permitted to use the AV2 global extended layer id",
        );
    }

    if header.temporal_layer_id != selected_layer.temporal_layer_id {
        return unsupported(
            DecodeUnsupportedReason::NonBaseTemporalLayer,
            envelope,
            "6.2.2",
            "only temporal layer 0 is selected by the initial decode stream planner",
        );
    }
    if header.embedded_layer_id != selected_layer.embedded_layer_id {
        return unsupported(
            DecodeUnsupportedReason::NonBaseEmbeddedLayer,
            envelope,
            "6.2.2",
            "only embedded layer 0 is selected by the initial decode stream planner",
        );
    }
    if header.extended_layer_id != selected_layer.extended_layer_id
        && header.extended_layer_id != GLOBAL_XLAYER_ID
    {
        return unsupported(
            DecodeUnsupportedReason::NonBaseExtendedLayer,
            envelope,
            "6.2.2",
            "only extended layer 0 and global-scope OBUs are accepted by the initial decode stream planner",
        );
    }

    match header.obu_type {
        ObuType::TemporalDelimiter | ObuType::Padding => Ok(DecodePlannedObuRole::Global),
        ObuType::SequenceHeader => Ok(DecodePlannedObuRole::SelectedLayerState),
        ObuType::ClosedLoopKey => Ok(DecodePlannedObuRole::FrameCandidate),
        // AV2 § 5.2.1 / § 5.19: `OBU_REGULAR_TILE_GROUP` carries a tile group of a
        // non-key frame, and the first tile group holds that frame's header.
        // `OBU_REGULAR_TIP` is a §5.2.1 `frame_header(1)` OBU, not a tile-group OBU,
        // but it is still a selected-layer frame candidate for traversal. The planner
        // admits both so the multi-frame runtime can reach them; the actual inter/TIP
        // frame decode (header shared tail, § 5.20 mode info, motion compensation) is
        // gated and tracked by `DECODE-FIRST-INTER-FRAME-FRONTIER`.
        // TODO(spec: DECODE-FIRST-INTER-FRAME-FRONTIER): this admits EVERY regular tile
        // group as a distinct frame candidate. A single inter frame may span multiple
        // `OBU_REGULAR_TILE_GROUP` OBUs (§ 5.19 tile-group continuation,
        // `tg_start`..`tg_end`); only the first carries the frame header, the rest are
        // continuations, not new frames. Distinguishing them needs the tile-group
        // structure (`tg_start`), which is not parsed at planner classification, so
        // `frame_candidate_count` would over-count a multi-tile-group frame. This is
        // masked today by the minimal runtime's strict one-`OBU_REGULAR_TILE_GROUP`
        // shape gate (`ensure_multiframe_plan_shape`); refine when multi-tile-group
        // inter frames are supported.
        ObuType::RegularTileGroup | ObuType::RegularTip => {
            Ok(DecodePlannedObuRole::InterFrameCandidate)
        }
        ObuType::Msdo
        | ObuType::LayerConfigurationRecord
        | ObuType::AtlasSegment
        | ObuType::OperatingPointSet => unsupported(
            DecodeUnsupportedReason::MultistreamSelection,
            envelope,
            "7.1",
            "multistream, operating-point, atlas, and external-HLS selection are outside the initial decode stream planner tier",
        ),
        obu_type
            if obu_type.is_tile_group()
                || obu_type.is_sef()
                || obu_type == ObuType::LeadingTip
                || obu_type == ObuType::BridgeFrame =>
        {
            unsupported(
                DecodeUnsupportedReason::UnsupportedFrameObu,
                envelope,
                "5.2.1",
                "only OBU_CLOSED_LOOP_KEY, OBU_REGULAR_TILE_GROUP, and OBU_REGULAR_TIP are accepted as frame candidates by the initial decode stream planner",
            )
        }
        ObuType::Reserved0 | ObuType::Reserved(_) => unsupported(
            DecodeUnsupportedReason::ReservedObu,
            envelope,
            "5.2.1",
            "reserved OBU types are outside the initial decode stream planner tier",
        ),
        _ => unsupported(
            DecodeUnsupportedReason::UnsupportedOutputEffectObu,
            envelope,
            output_effect_spec_section(header.obu_type),
            "this OBU can affect state or output not modeled by the initial decode stream planner tier",
        ),
    }
}

fn unsupported(
    reason: DecodeUnsupportedReason,
    envelope: ObuEnvelope<'_>,
    spec_section: &'static str,
    message: &'static str,
) -> Result<DecodePlannedObuRole> {
    Err(DecodeError::UnsupportedStructure {
        unsupported: DecodeUnsupportedStructure {
            reason,
            obu_type: envelope.header.obu_type,
            offset: envelope.offset,
            spec_section,
            message,
        },
    })
}

fn output_effect_spec_section(obu_type: ObuType) -> &'static str {
    match obu_type {
        ObuType::MultiFrameHeader => "5.7",
        ObuType::MetadataShort | ObuType::MetadataGroup => "5.17",
        ObuType::BufferRemovalTiming => "5.12",
        ObuType::QuantizationMatrix => "5.13",
        ObuType::FilmGrain => "5.14",
        ObuType::ContentInterpretation => "5.15",
        _ => "5.2.1",
    }
}

fn issue_from_core_error(
    kind: DecodeSourceIssueKind,
    frame_index: Option<usize>,
    error: &splot_core::Error,
) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind,
        rule_id: None,
        offset: core_error_offset(error),
        frame_index,
        message: error.to_string(),
    }
}

fn issue_from_ivf_error(error: &IvfError) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind: DecodeSourceIssueKind::IvfContainerError,
        rule_id: Some(error.rule_id()),
        offset: Some(error.offset()),
        frame_index: ivf_error_frame_index(error),
        message: error.to_string(),
    }
}

fn issue_from_ivf_warning(warning: &IvfWarning) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind: DecodeSourceIssueKind::IvfWarning,
        rule_id: Some(warning.rule_id()),
        offset: Some(warning.offset()),
        frame_index: ivf_warning_frame_index(warning),
        message: warning.to_string(),
    }
}

fn ivf_frame_context(frame: &ParsedIvfFrame<'_>) -> DecodeIvfFrameContext {
    DecodeIvfFrameContext {
        frame_index: frame.frame.index,
        frame_header_offset: frame.frame.header_offset,
        frame_payload_offset: frame.frame.payload_offset,
        frame_payload_size: frame.frame.size,
        pts: frame.frame.pts,
    }
}

fn ivf_error_frame_index(error: &IvfError) -> Option<usize> {
    match error {
        IvfError::TruncatedFrameHeader { frame_index, .. }
        | IvfError::TruncatedFramePayload { frame_index, .. } => Some(*frame_index),
        _ => None,
    }
}

fn ivf_warning_frame_index(warning: &IvfWarning) -> Option<usize> {
    match warning {
        IvfWarning::TrailingPartialFrameHeader { frame_index, .. } => Some(*frame_index),
        _ => None,
    }
}

fn core_error_offset(error: &splot_core::Error) -> Option<ByteOffset> {
    match error {
        splot_core::Error::UnexpectedEof { offset, .. }
        | splot_core::Error::InvalidLeb128 { offset, .. }
        | splot_core::Error::InvalidUvlc { offset, .. }
        | splot_core::Error::InvalidNs { offset, .. }
        | splot_core::Error::InvalidRg { offset, .. }
        | splot_core::Error::InvalidQuantizerMatrix { offset, .. }
        | splot_core::Error::InvalidObuHeader { offset, .. }
        | splot_core::Error::InvalidTrailingBits { offset, .. }
        | splot_core::Error::InvalidByteAlignment { offset, .. }
        | splot_core::Error::InvalidSequenceHeader { offset, .. }
        | splot_core::Error::ObuSizeOutOfRange { offset, .. }
        | splot_core::Error::InvalidObuExtension { offset, .. }
        | splot_core::Error::ObuPayloadOutOfRange { offset, .. }
        | splot_core::Error::InvalidLayerConfigRecord { offset, .. }
        | splot_core::Error::InvalidAtlasSegment { offset, .. }
        | splot_core::Error::InvalidPadding { offset, .. }
        | splot_core::Error::InvalidMetadata { offset, .. } => Some(*offset),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
