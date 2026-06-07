// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stateful validator context for checks that depend on earlier OBUs.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::sequence::{
    SequenceHeaderGeneral, SequenceHeaderId, TimingInfo, parse_sequence_header_general,
};
use splot_core::span::{BitOffset, ByteOffset};
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType};

use crate::diagnostic::{Diagnostic, ValidationReport};

/// Stateful validator data derived from parseable high-level syntax OBUs.
#[derive(Debug, Default)]
pub(crate) struct ValidatorContext {
    sequence_headers: BTreeMap<SequenceHeaderId, SequenceHeaderGeneral>,
    active_sequence_by_xlayer: BTreeMap<ExtendedLayerId, SequenceHeaderId>,
    /// Payload fingerprints for activated sequence headers, keyed by
    /// `(obu_xlayer_id, seq_header_id)`, used to detect non-bit-identical repeats
    /// of an activated sequence header (AV2 § 7.3.8).
    sequence_fingerprints: BTreeMap<(ExtendedLayerId, SequenceHeaderId), u64>,
    /// Content-interpretation records keyed by `(obu_xlayer_id, obu_mlayer_id)`
    /// within the modeled coded-video-sequence scope, used for cross-embedded-layer
    /// timing consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14).
    content_interpretations: BTreeMap<ContentInterpretationKey, ContentInterpretationRecord>,
    temporal_unit: TemporalUnitState,
}

/// Key identifying a content-interpretation record: `(obu_xlayer_id, obu_mlayer_id)`.
type ContentInterpretationKey = (ExtendedLayerId, EmbeddedLayerId);

/// One observed content-interpretation OBU within the modeled CVS scope.
#[derive(Debug)]
struct ContentInterpretationRecord {
    /// Parsed § 5.15 syntax, used for cross-embedded-layer timing consistency
    /// (AV2 § 6.4.12) and the repeated-CI "same information" check (AV2 § 6.14).
    content: ContentInterpretation,
    /// Source byte offset of the OBU that produced this record.
    offset: ByteOffset,
}

impl ValidatorContext {
    /// Observes one parsed OBU, updating context and emitting stateful diagnostics.
    pub(crate) fn observe_obu(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        self.temporal_unit.observe_obu(obu, report);

        if obu.header.obu_type == ObuType::SequenceHeader {
            self.observe_sequence_header(obu, report);
        } else {
            self.validate_active_sequence_limits(obu, report);
        }

        if obu.header.obu_type == ObuType::ContentInterpretation {
            self.observe_content_interpretation(obu, report);
        }

        self.maybe_reset_coded_video_sequence(obu);
    }

    /// Resets per-extended-layer sequence-header fingerprints at coded-video-sequence
    /// boundaries (AV2 § 7.3.8): a new CVS for an extended layer starts at each
    /// temporal unit containing an `OBU_CLOSED_LOOP_KEY` for that layer, after which
    /// a reconfigured sequence header with the same `seq_header_id` is legal.
    ///
    /// Precise per-CVS scoping ultimately needs CLK frame-header association, which
    /// is not modeled yet. This conservative reset removes the common false positive
    /// where a later CVS reuses a `seq_header_id` with changed parameters, at the cost
    /// of a false negative: because the CVS-opening header precedes its CLK in the
    /// same temporal unit (§7.3.6), a non-identical repeat *after* the CLK within the
    /// same CVS is no longer compared. This sound-over-complete bias (never reject a
    /// valid stream) is intentional until CLK activation is parsed
    /// (see the `sequence-timing-hls-availability` change).
    fn maybe_reset_coded_video_sequence(&mut self, obu: &ObuEnvelope<'_>) {
        if obu.header.obu_type == ObuType::ClosedLoopKey
            && !obu.header.extended_layer_id.is_global()
        {
            let xlayer = obu.header.extended_layer_id;
            // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): scope CVS boundaries precisely
            // once CLK frame-header activation is parsed.
            self.sequence_fingerprints.retain(|(x, _), _| *x != xlayer);
            // Cross-embedded-layer timing and repeated-CI identity are scoped to a
            // coded video sequence (AV2 § 6.4.12 / § 6.14), so clear this xlayer's
            // content-interpretation records at the same conservative CVS boundary.
            self.content_interpretations
                .retain(|(x, _), _| *x != xlayer);
        }
    }

    /// Observes a content-interpretation OBU: checks cross-embedded-layer timing
    /// consistency (AV2 § 6.4.12) and repeated-CI identity (AV2 § 6.14) within the
    /// modeled coded-video-sequence scope.
    ///
    /// Timing values are compared only between two present `timing_info()` values
    /// that are both within the same extended layer's modeled CVS scope (a sound
    /// subset of the spec's "across all embedded layers" requirement; exact
    /// cross-extended-layer scoping needs CLK frame-header activation).
    fn observe_content_interpretation(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures are reported by the stateless ContentInterpretationSyntax
        // check; here we only act on a successful parse.
        let Ok(content_interpretation) = parse_content_interpretation(&mut reader) else {
            return;
        };

        let xlayer = obu.header.extended_layer_id;
        let mlayer = obu.header.embedded_layer_id;

        // Cross-embedded-layer timing consistency: compare this layer's timing
        // against the first other embedded layer (same extended layer) that already
        // carries present timing within this CVS scope.
        if let Some(new_timing) = content_interpretation.timing_info
            && let Some(existing_timing) = self
                .content_interpretations
                .iter()
                .find(|((x, m), record)| {
                    *x == xlayer && *m != mlayer && record.content.timing_info.is_some()
                })
                .and_then(|(_, record)| record.content.timing_info)
        {
            compare_timing_across_embedded_layers(&existing_timing, &new_timing, obu, report);
        }

        match self.content_interpretations.entry((xlayer, mlayer)) {
            Entry::Vacant(slot) => {
                slot.insert(ContentInterpretationRecord {
                    content: content_interpretation,
                    offset: obu.offset,
                });
            }
            Entry::Occupied(slot) => {
                let existing = slot.get();
                // AV2 § 6.14: a repeated CI OBU for the same embedded layer within a
                // CVS must carry the same *information* (a weaker requirement than the
                // sequence header's bit-identity in § 7.3.8). The decoder-ignored
                // ci_reserved_2bit is normalized out before comparing, so a difference
                // confined to the reserved bits is not flagged here (it is surfaced
                // separately as a warning by the stateless syntax check).
                if content_interpretation_information_differs(
                    &existing.content,
                    &content_interpretation,
                ) {
                    report.push(
                        Diagnostic::error(
                            "content-interpretation/repeated-ci-not-identical",
                            format!(
                                "content interpretation OBU for obu_xlayer_id {} / obu_mlayer_id {} \
                                 is repeated within the coded video sequence with different \
                                 information (first seen at byte {})",
                                xlayer.get(),
                                mlayer.get(),
                                existing.offset
                            ),
                        )
                        .with_spec_section("6.14")
                        .with_byte_offset(obu.offset),
                    );
                }
                // Keep the first record for the layer (matching the sequence-header
                // first-wins approximation); a non-identical repeat is reported but
                // does not overwrite the established timing baseline.
            }
        }
    }

    fn observe_sequence_header(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if !sequence_header_can_activate(obu) {
            return;
        }

        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(sequence_header) = parse_sequence_header_general(&mut reader) else {
            return;
        };

        let seq_header_id = sequence_header.seq_header_id;
        let xlayer = obu.header.extended_layer_id;
        let fingerprint = payload_fingerprint(obu.payload);

        // AV2 § 7.3.8: within a coded video sequence, a repeated activated sequence
        // header is allowed only if its payload bytes are bit-identical. Compare a
        // payload fingerprint, not parsed fields, since inferred values can hide
        // syntax differences. Fingerprints are cleared per extended layer at CVS
        // boundaries (see maybe_reset_coded_video_sequence).
        //
        // NOTE: the fingerprint key is (xlayer, seq_header_id); cross-xlayer identity
        // for the same seq_header_id is not yet enforced.
        // TODO(spec: AV2-7.3.8-HLS-AVAILABILITY): validate cross-xlayer seq_header_id
        // identity once the full HLS availability store exists.
        match self.sequence_fingerprints.entry((xlayer, seq_header_id)) {
            Entry::Vacant(slot) => {
                slot.insert(fingerprint);
            }
            Entry::Occupied(slot) => {
                if *slot.get() != fingerprint {
                    report.push(
                        Diagnostic::error(
                            "hls/repeated-sequence-header-not-identical",
                            format!(
                                "activated sequence header seq_header_id {} for obu_xlayer_id {} \
                                 is repeated with different payload bytes",
                                seq_header_id.get(),
                                xlayer.get()
                            ),
                        )
                        .with_spec_section("7.3.8")
                        .with_byte_offset(obu.offset),
                    );
                }
            }
        }

        // `or_insert` keeps the first activated header per id/xlayer for the run.
        // This is a known approximation: when a later CVS (after a CLK) reuses the
        // same seq_header_id with different layer limits, the stale header is still
        // used for max_tlayer_id/max_mlayer_id checks. Resolving this requires CLK
        // frame-header activation (the activating CLK follows its sequence header in
        // OBU order, so the state cannot be reset purely on the CLK without breaking
        // intra-CVS frames). Tracked for the follow-up HLS-availability change.
        // TODO(spec: AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT): activate/reset the per-xlayer
        // sequence header from the CLK frame header instead of the first base-layer copy.
        self.sequence_headers
            .entry(seq_header_id)
            .or_insert(sequence_header);
        self.active_sequence_by_xlayer
            .entry(xlayer)
            .or_insert(seq_header_id);
    }

    fn validate_active_sequence_limits(
        &self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if !requires_active_sequence(obu) {
            return;
        }

        let Some(seq_header_id) = self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)
        else {
            report.push(sequence_state_error(
                "sequence-state/no-active-sequence-header",
                "7.3.8",
                obu,
                None,
                format!(
                    "{} uses obu_xlayer_id {} before an active sequence header is available",
                    obu.header.obu_type.spec_name(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        // Invariant: sequence_headers and active_sequence_by_xlayer are updated
        // together in observe_sequence_header(). This guard only becomes reachable
        // if a future sequence-header eviction policy removes stored headers.
        let Some(sequence_header) = self.sequence_headers.get(seq_header_id) else {
            report.push(sequence_state_error(
                "sequence-state/unknown-sequence-header-id",
                "7.3.8",
                obu,
                None,
                format!(
                    "active seq_header_id {} for obu_xlayer_id {} is unavailable",
                    seq_header_id.get(),
                    obu.header.extended_layer_id.get()
                ),
            ));
            return;
        };

        if obu.header.temporal_layer_id > sequence_header.max_tlayer_id {
            report.push(sequence_state_error(
                "sequence-state/tlayer-exceeds-max",
                "6.2.2",
                obu,
                Some(BitOffset::from_bits(6)),
                format!(
                    "obu_tlayer_id {} exceeds active sequence max_tlayer_id {}",
                    obu.header.temporal_layer_id.get(),
                    sequence_header.max_tlayer_id.get()
                ),
            ));
        }

        if obu.header.embedded_layer_id > sequence_header.max_mlayer_id {
            let byte_offset = obu.offset.saturating_add(1);
            report.push(
                Diagnostic::error(
                    "sequence-state/mlayer-exceeds-max",
                    format!(
                        "obu_mlayer_id {} exceeds active sequence max_mlayer_id {}",
                        obu.header.embedded_layer_id.get(),
                        sequence_header.max_mlayer_id.get()
                    ),
                )
                .with_spec_section("6.2.2")
                .with_byte_offset(byte_offset)
                .with_bit_offset(BitOffset::from_bits(0)),
            );
        }
    }
}

/// Compares two present `timing_info()` values from different embedded layers of
/// the same coded video sequence and emits a diagnostic per differing field
/// (AV2 § 6.4.12: these values, when present, shall be the same across all embedded
/// layers). `new` is located at `obu`.
fn compare_timing_across_embedded_layers(
    existing: &TimingInfo,
    new: &TimingInfo,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    if existing.num_units_in_display_tick != new.num_units_in_display_tick {
        report.push(timing_mismatch_error(
            "sequence-header/timing-display-tick-mismatch",
            obu,
            format!(
                "num_units_in_display_tick {} differs from {} signalled for another embedded layer",
                new.num_units_in_display_tick, existing.num_units_in_display_tick
            ),
        ));
    }
    if existing.time_scale != new.time_scale {
        report.push(timing_mismatch_error(
            "sequence-header/timing-time-scale-mismatch",
            obu,
            format!(
                "time_scale {} differs from {} signalled for another embedded layer",
                new.time_scale, existing.time_scale
            ),
        ));
    }
    if existing.equal_picture_interval != new.equal_picture_interval {
        report.push(timing_mismatch_error(
            "sequence-header/timing-equal-picture-interval-mismatch",
            obu,
            format!(
                "equal_picture_interval {} differs from {} signalled for another embedded layer",
                new.equal_picture_interval, existing.equal_picture_interval
            ),
        ));
    }
    // num_ticks_per_picture_minus_1 is only present when equal_picture_interval is
    // set; compare it only when both layers carry it (AV2 § 6.4.12).
    if let (Some(existing_ticks), Some(new_ticks)) = (
        existing.num_ticks_per_picture_minus_1,
        new.num_ticks_per_picture_minus_1,
    ) && existing_ticks != new_ticks
    {
        report.push(timing_mismatch_error(
            "sequence-header/timing-num-ticks-mismatch",
            obu,
            format!(
                "num_ticks_per_picture_minus_1 {new_ticks} differs from {existing_ticks} signalled \
                 for another embedded layer"
            ),
        ));
    }
}

/// Returns `true` if two content-interpretation OBUs carry different *information*
/// (AV2 § 6.14: a repeated CI OBU must "contain the same information"). The
/// decoder-ignored `ci_reserved_2bit` is normalized out so a difference confined to
/// the reserved bits is not treated as a content change.
fn content_interpretation_information_differs(
    a: &ContentInterpretation,
    b: &ContentInterpretation,
) -> bool {
    let mut a_info = *a;
    let mut b_info = *b;
    a_info.reserved_2bit = 0;
    b_info.reserved_2bit = 0;
    a_info != b_info
}

/// Builds a § 6.4.12 cross-embedded-layer timing-mismatch diagnostic located at `obu`.
fn timing_mismatch_error(
    rule_id: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("6.4.12")
        .with_byte_offset(obu.offset)
}

/// Computes a stable 64-bit FNV-1a fingerprint over an OBU payload's bytes.
///
/// Used to compare repeated activated sequence headers for bit identity without
/// pulling in a hashing dependency (AV2 § 7.3.8).
fn payload_fingerprint(payload: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in payload {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[derive(Debug, Default)]
struct TemporalUnitState {
    phase: TemporalUnitPhase,
    current_coded_xlayer: Option<ExtendedLayerId>,
    reported_missing_delimiter: bool,
    /// `true` once any non-reserved, non-delimiter OBU has appeared since the most
    /// recent global temporal delimiter. Used to detect back-to-back delimiters.
    saw_obu_since_delimiter: bool,
}

impl TemporalUnitState {
    fn observe_obu(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved() {
            return;
        }

        if obu.header.obu_type == ObuType::TemporalDelimiter {
            if obu.header.extended_layer_id.is_global() {
                if !matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter)
                    && !self.saw_obu_since_delimiter
                {
                    report.push(ordering_error(
                        "obu-order/duplicate-temporal-delimiter",
                        obu,
                        "a temporal unit must start with exactly one global \
                         OBU_TEMPORAL_DELIMITER; found a second delimiter with no \
                         intervening OBU"
                            .to_owned(),
                    ));
                }
                self.start_temporal_unit();
            } else if matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter) {
                self.report_missing_delimiter_once(obu, report);
            }
            return;
        }

        if matches!(self.phase, TemporalUnitPhase::AwaitingDelimiter) {
            self.report_missing_delimiter_once(obu, report);
        }
        self.saw_obu_since_delimiter = true;

        if is_padding_obu(obu) {
            self.observe_padding(obu, report);
        } else if is_global_hls_prefix_obu(obu) {
            self.observe_global_hls_prefix(obu, report);
        } else if is_coded_extended_layer_obu(obu) {
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    fn start_temporal_unit(&mut self) {
        self.phase = TemporalUnitPhase::GlobalPrefix;
        self.current_coded_xlayer = None;
        self.reported_missing_delimiter = false;
        self.saw_obu_since_delimiter = false;
    }

    fn report_missing_delimiter_once(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if self.reported_missing_delimiter {
            return;
        }
        self.reported_missing_delimiter = true;
        report.push(ordering_error(
            "obu-order/temporal-unit-missing-delimiter",
            obu,
            format!(
                "{} appears before a global OBU_TEMPORAL_DELIMITER starts the temporal unit",
                obu.header.obu_type.spec_name()
            ),
        ));
    }

    fn observe_padding(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.extended_layer_id.is_global() {
            return;
        }

        let inside_current_coded_layer = matches!(self.phase, TemporalUnitPhase::CodedLayers)
            && self.current_coded_xlayer == Some(obu.header.extended_layer_id);
        if !inside_current_coded_layer {
            report.push(ordering_error(
                "obu-order/padding-non-global-outside-coded-layer",
                obu,
                format!(
                    "OBU_PADDING outside a coded extended layer unit must use \
                     obu_xlayer_id == GLOBAL_XLAYER_ID, found {}",
                    obu.header.extended_layer_id.get()
                ),
            ));
        }
    }

    fn observe_global_hls_prefix(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if matches!(self.phase, TemporalUnitPhase::CodedLayers) {
            report.push(ordering_error(
                "obu-order/global-hls-after-coded-layer",
                obu,
                format!(
                    "{} with GLOBAL_XLAYER_ID appears after a coded extended layer unit",
                    obu.header.obu_type.spec_name()
                ),
            ));
        }
    }

    fn observe_coded_extended_layer_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let xlayer = obu.header.extended_layer_id;
        match self.current_coded_xlayer {
            Some(current) if xlayer < current => {
                report.push(ordering_error(
                    "obu-order/xlayer-order-not-ascending",
                    obu,
                    format!(
                        "coded extended layer units must appear in ascending obu_xlayer_id order \
                         within a temporal unit (found {} after {})",
                        xlayer.get(),
                        current.get()
                    ),
                ));
            }
            Some(current) if xlayer == current => {}
            _ => {
                self.current_coded_xlayer = Some(xlayer);
            }
        }
        self.phase = TemporalUnitPhase::CodedLayers;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TemporalUnitPhase {
    #[default]
    AwaitingDelimiter,
    GlobalPrefix,
    CodedLayers,
}

fn sequence_header_can_activate(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && obu.header.temporal_layer_id.get() == 0
        && obu.header.embedded_layer_id.get() == 0
}

fn requires_active_sequence(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && !matches!(
            obu.header.obu_type,
            ObuType::Reserved0
                | ObuType::Reserved(_)
                | ObuType::SequenceHeader
                | ObuType::TemporalDelimiter
                | ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
        )
}

fn is_padding_obu(obu: &ObuEnvelope<'_>) -> bool {
    obu.header.obu_type == ObuType::Padding
}

fn is_global_hls_prefix_obu(obu: &ObuEnvelope<'_>) -> bool {
    // TODO(spec: AV2-7.3-OBU-ORDERING): BufferRemovalTiming also permits
    // GLOBAL_XLAYER_ID per § 6.2.2; model its ordering position once
    // decoder-model state exists.
    obu.header.extended_layer_id.is_global()
        && matches!(
            obu.header.obu_type,
            ObuType::Msdo
                | ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
                | ObuType::MetadataShort
                | ObuType::MetadataGroup
        )
}

fn is_coded_extended_layer_obu(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && !matches!(
            obu.header.obu_type,
            ObuType::TemporalDelimiter
                | ObuType::Padding
                | ObuType::Reserved0
                | ObuType::Reserved(_)
        )
}

fn sequence_state_error(
    rule_id: &'static str,
    spec_section: &'static str,
    obu: &ObuEnvelope<'_>,
    bit_offset: Option<BitOffset>,
    message: String,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(rule_id, message)
        .with_spec_section(spec_section)
        .with_byte_offset(obu.offset);
    if let Some(bit_offset) = bit_offset {
        diagnostic = diagnostic.with_bit_offset(bit_offset);
    }
    diagnostic
}

fn ordering_error(rule_id: &'static str, obu: &ObuEnvelope<'_>, message: String) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("7.3.7")
        .with_byte_offset(obu.offset)
}
