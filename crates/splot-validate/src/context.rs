// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stateful validator context for checks that depend on earlier OBUs.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::sequence::{
    SequenceHeaderGeneral, SequenceHeaderId, TimingInfo, parse_sequence_header,
};
use splot_core::hls::parse_multi_frame_header;
use splot_core::obu::finish_obu_payload;
use splot_core::span::{BitOffset, ByteOffset};
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType};

use crate::diagnostic::{Diagnostic, ValidationReport};
use crate::options::{ExternalHlsMode, ValidationOptions};

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
    /// Availability of in-band HLS objects, for reference checks (AV2 § 7.3.8).
    hls: HlsAvailabilityStore,
    temporal_unit: TemporalUnitState,
}

/// Availability of in-band HLS objects, for the § 7.3.8 reference checks.
///
/// Only sequence-header availability is modeled today (the one in-band reference
/// path implemented — the multi-frame header's `mfh_seq_header_id`). MSDO / MFH /
/// LCR / atlas / OPS availability records are deferred: their consumers
/// (frame-header `cur_mfh_id`, the random-access-point "identical MSDO" rule,
/// `seq_lcr_id` resolution, …) require frame-header parsing or RAP detection that is
/// out of scope, so storing them now would be unconsumed state.
///
/// The set is kept **monotonic** (never cleared): an object included earlier in the
/// bitstream stays available, so the validator never falsely reports it unavailable.
/// AV2 § 7.3.8.1's "HLS OBUs must be resent at each random access point" requirement
/// needs CLK frame-header activation to model and is intentionally not enforced (a
/// sound-over-complete false negative).
#[derive(Debug, Default)]
struct HlsAvailabilityStore {
    /// `seq_header_id` values of sequence headers seen in-band so far (§ 7.3.8.6).
    sequence_header_ids: BTreeSet<u32>,
}

/// How a referenced HLS object resolves against available objects (AV2 § 7.3.8).
enum HlsResolution {
    /// Available in the bitstream.
    InBand,
    /// Available only through caller-provided external HLS.
    External,
    /// Not available by any modeled means.
    Unavailable,
}

impl HlsAvailabilityStore {
    /// Records a sequence header (by `seq_header_id`) as available in-band
    /// (AV2 § 7.3.8.6).
    fn record_sequence_header(&mut self, seq_header_id: u32) {
        self.sequence_header_ids.insert(seq_header_id);
    }

    /// Resolves a `seq_header_id` reference against in-band then caller-provided
    /// external availability (AV2 § 7.3.8.6).
    fn resolve_sequence_header(&self, id: u32, options: &ValidationOptions) -> HlsResolution {
        if self.sequence_header_ids.contains(&id) {
            return HlsResolution::InBand;
        }
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.has_sequence_header(id)
        {
            return HlsResolution::External;
        }
        HlsResolution::Unavailable
    }
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
    pub(crate) fn observe_obu(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        self.temporal_unit.observe_obu(obu, report);

        if obu.header.obu_type == ObuType::SequenceHeader {
            self.observe_sequence_header(obu, report);
        } else {
            self.validate_active_sequence_limits(obu, options, report);
        }

        match obu.header.obu_type {
            ObuType::ContentInterpretation => self.observe_content_interpretation(obu, report),
            ObuType::MultiFrameHeader => self.observe_multi_frame_header(obu, options, report),
            _ => {}
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
            // This shares the sequence-fingerprint reset's documented sound-over-
            // complete bias: a CI OBU at CVS start precedes its CLK in the same
            // temporal unit (§ 7.3.6), so its record is cleared here and a later
            // non-identical CI *within the same CVS* is not caught (a false negative,
            // never a false positive). Exact scoping needs CLK frame-header
            // activation (AV2-5.18-FRAME-HEADER).
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
            && let Some((existing_mlayer, existing_timing)) = self
                .content_interpretations
                .iter()
                .find(|((x, m), record)| {
                    *x == xlayer && *m != mlayer && record.content.timing_info.is_some()
                })
                .and_then(|((_, m), record)| record.content.timing_info.map(|t| (*m, t)))
        {
            compare_timing_across_embedded_layers(
                existing_mlayer,
                &existing_timing,
                &new_timing,
                obu,
                report,
            );
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

    /// Observes a multi-frame header OBU and checks that the sequence header it
    /// references via `mfh_seq_header_id` is available in-band or through
    /// caller-provided external HLS (AV2 § 7.3.8.6 / § 7.3.8.7).
    fn observe_multi_frame_header(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and the mfh_seq_header_id range check are handled by the
        // stateless MultiFrameHeaderSyntax check; here we only resolve the reference.
        let Ok(mfh) = parse_multi_frame_header(&mut reader) else {
            return;
        };
        // An out-of-range id (>= MAX_SEQ_NUM) cannot name a valid sequence header and
        // is already flagged as mfh/seq-header-id-out-of-range; do not double-report.
        if !mfh.seq_header_id_in_range() {
            return;
        }

        let id = mfh.mfh_seq_header_id;
        match self.hls.resolve_sequence_header(id, options) {
            HlsResolution::InBand | HlsResolution::External => {}
            HlsResolution::Unavailable => {
                let external_note = if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    " (external HLS is disabled)"
                } else {
                    " in-band or through the supplied external HLS"
                };
                report.push(
                    Diagnostic::error(
                        "mfh/sequence-header-unavailable",
                        format!(
                            "multi-frame header references mfh_seq_header_id {id}, but no sequence \
                             header with that id is available{external_note}"
                        ),
                    )
                    .with_spec_section("7.3.8.6")
                    .with_byte_offset(obu.offset),
                );
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    // Advisory: the finding assumes no external HLS. If the referenced
                    // sequence header is supplied out-of-band, the caller can declare
                    // it via ValidationOptions to refine the check (AV2 § 7.3.8.1).
                    report.push(
                        Diagnostic::warning(
                            "hls/external-hls-disabled",
                            format!(
                                "sequence header {id} is not available in-band and external HLS is \
                                 disabled; supply it via ValidationOptions if it is provided through \
                                 external means"
                            ),
                        )
                        .with_spec_section("7.3.8.1")
                        .with_byte_offset(obu.offset),
                    );
                }
            }
        }
    }

    fn observe_sequence_header(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Gate availability and activation on the same validation the
        // SequenceHeaderSyntax check applies: the full sequence_header_obu() parse,
        // accepting a bounded-but-Ok parse (one that stops at an unimplemented child
        // config), and — for a fully parsed header — a valid §5.2.1 payload tail
        // (obu_extension_flag + trailing_bits). A header that fails its child configs
        // or its tail is malformed and is NOT recorded as available, so a later MFH
        // cannot resolve against it (AV2 § 7.3.8.6).
        let Ok(sequence_header) = parse_sequence_header(&mut reader) else {
            return;
        };
        if sequence_header.is_fully_parsed()
            && finish_obu_payload(
                &mut reader,
                obu.payload,
                obu.header.obu_type.is_extensible_obu(),
            )
            .is_err()
        {
            return;
        }
        let general = sequence_header.general;

        // A conformant sequence header must be base-layer and non-global (AV2 §6.2.2);
        // sequence_header_can_activate() captures exactly that layer-id validity. A
        // header that violates it is malformed (flagged by the stateless §6.2.2
        // checks) and is neither available (§7.3.8.6) nor activatable, so a later MFH
        // cannot resolve against it.
        if !sequence_header_can_activate(obu) {
            return;
        }

        // Record in-band availability (AV2 § 7.3.8.6): a well-formed sequence header
        // included in the bitstream makes its seq_header_id available to later
        // references.
        self.hls
            .record_sequence_header(u32::from(general.seq_header_id.get()));

        let seq_header_id = general.seq_header_id;
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
            .or_insert(general);
        self.active_sequence_by_xlayer
            .entry(xlayer)
            .or_insert(seq_header_id);
    }

    fn validate_active_sequence_limits(
        &self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !requires_active_sequence(obu) {
            return;
        }

        // When external HLS declares any sequence header, an externally-provided
        // sequence header may be the active one for this extended layer (AV2
        // § 7.3.8.1: external HLS objects "remain available ... until superseded"),
        // with layer limits this validator does not model. The in-band
        // active-sequence-limit checks (missing active header and tlayer/mlayer
        // limits) are therefore unreliable and suppressed, so the validator never
        // rejects a conformant external-HLS stream. An empty external set declares no
        // sequence header that could be active, so it does NOT suppress (the missing
        // active header is still an error). Exact enforcement needs external
        // sequence-header activation and layer limits (AV2-5.18-FRAME-HEADER).
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.declares_any_sequence_header()
        {
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
/// layers). `new` is located at `obu` (embedded layer `obu.header.embedded_layer_id`);
/// `existing` is the value previously seen for `existing_mlayer`. Both embedded-layer
/// ids are named in each message so the finding is self-contained.
fn compare_timing_across_embedded_layers(
    existing_mlayer: EmbeddedLayerId,
    existing: &TimingInfo,
    new: &TimingInfo,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    let new_mlayer = obu.header.embedded_layer_id.get();
    let existing_mlayer = existing_mlayer.get();
    if existing.num_units_in_display_tick != new.num_units_in_display_tick {
        report.push(timing_mismatch_error(
            "sequence-header/timing-display-tick-mismatch",
            obu,
            format!(
                "num_units_in_display_tick {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) \
                 in the same coded video sequence",
                new.num_units_in_display_tick,
                new_mlayer,
                existing.num_units_in_display_tick,
                existing_mlayer
            ),
        ));
    }
    if existing.time_scale != new.time_scale {
        report.push(timing_mismatch_error(
            "sequence-header/timing-time-scale-mismatch",
            obu,
            format!(
                "time_scale {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in the same \
                 coded video sequence",
                new.time_scale, new_mlayer, existing.time_scale, existing_mlayer
            ),
        ));
    }
    if existing.equal_picture_interval != new.equal_picture_interval {
        report.push(timing_mismatch_error(
            "sequence-header/timing-equal-picture-interval-mismatch",
            obu,
            format!(
                "equal_picture_interval {} (obu_mlayer_id {}) differs from {} (obu_mlayer_id {}) in \
                 the same coded video sequence",
                new.equal_picture_interval, new_mlayer, existing.equal_picture_interval, existing_mlayer
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
                "num_ticks_per_picture_minus_1 {new_ticks} (obu_mlayer_id {new_mlayer}) differs \
                 from {existing_ticks} (obu_mlayer_id {existing_mlayer}) in the same coded video \
                 sequence"
            ),
        ));
    }
}

/// Returns `true` if two content-interpretation OBUs carry different *information*
/// (AV2 § 6.14: a repeated CI OBU must "contain the same information").
///
/// Only fields whose parsed value uniquely determines the information regardless of
/// encoding are compared: `ci_scan_type_idc`, the chroma sample position, and
/// `timing_info()`. Deliberately excluded:
/// - `ci_reserved_2bit` — decoder-ignored (§ 6.14); surfaced separately as a warning.
/// - `ci_color_description` and the aspect ratio — these can encode the *same*
///   information in multiple ways (a Table 6.13 / aspect preset vs. an explicit
///   triple or SAR), so a raw difference is not necessarily a content change.
///   Comparing them raw would risk a false-positive hard error against a conformant
///   stream, which this validator must never do; soundly comparing them needs the
///   § 6.14 preset normalization, which is not modeled yet (a documented
///   false-negative, never a false-positive).
///
// TODO(spec: AV2-5.15-CONTENT-INTERPRETATION): normalize § 6.14 color-description
// (Table 6.13) and aspect-ratio presets to derived values so repeated-CI
// color/aspect differences can be compared soundly and promoted to this check.
fn content_interpretation_information_differs(
    a: &ContentInterpretation,
    b: &ContentInterpretation,
) -> bool {
    a.scan_type_idc != b.scan_type_idc
        || a.chroma_sample_position != b.chroma_sample_position
        || a.timing_info != b.timing_info
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
