// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal-unit ordering state and helpers.

use super::*;

#[derive(Debug, Default)]
pub(super) struct TemporalUnitState {
    pub(super) phase: TemporalUnitPhase,
    pub(super) current_coded_xlayer: Option<ExtendedLayerId>,
    pub(super) reported_missing_delimiter: bool,
    /// `true` once any non-reserved, non-delimiter OBU has appeared since the most
    /// recent global temporal delimiter. Used to detect back-to-back delimiters.
    pub(super) saw_obu_since_delimiter: bool,
    /// `true` once a *global suffix* metadata OBU (`metadata_is_suffix == 1`) has
    /// appeared in this temporal unit. A global suffix metadata is part of a coded
    /// frame unit's suffix tail (§ 7.3.3 / § 7.3.4), which lies inside / after the
    /// coded extended layer units, so a later global HLS prefix OBU is out of
    /// order (§ 7.3.7): `obu-order/global-hls-after-metadata-suffix`.
    pub(super) saw_global_suffix_metadata: bool,
    /// The set of extended layers whose coded *frame* OBUs (frame-bearing or
    /// pre-frame content) have begun in the current coded extended layer unit. § 7.3.6
    /// orders the coded extended layer unit as LCR → OPS → atlas → sequence header
    /// → frame units, so a non-global HLS *header* OBU (LCR / OPS / atlas /
    /// sequence header) for an extended layer whose frame region has already begun
    /// is out of order: `obu-order/non-global-hls-before-coded-layer`. Tracking a
    /// *set* (not the last layer alone) catches a reordered header for an earlier
    /// extended layer after a later layer's frame region has begun.
    pub(super) coded_frame_started_xlayer: BTreeSet<ExtendedLayerId>,
}

impl TemporalUnitState {
    pub(super) fn observe_obu(&mut self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
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
        } else if is_metadata_obu(obu) {
            self.observe_metadata(obu, report);
        } else if is_global_hls_prefix_obu(obu) {
            self.observe_global_hls_prefix(obu, report);
        } else if is_coded_extended_layer_obu(obu) {
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    /// Classifies a metadata OBU for temporal-unit ordering from its `metadata_is_suffix`
    /// bit (AV2 § 6.16.3 / § 7.3.7).
    ///
    /// A global *prefix* metadata OBU (`metadata_is_suffix == 0`) is a global temporal-
    /// unit prefix OBU, so it is flagged if it follows a coded extended layer unit. A
    /// global *suffix* metadata OBU (`metadata_is_suffix == 1`) is not a prefix and is
    /// left unclassified (its precise § 7.3.3 / § 7.3.4 placement inside coded frame
    /// units needs frame/tile parsing, which is deferred). Non-global metadata is a coded
    /// extended layer OBU. A metadata OBU whose first payload bit cannot be read is left
    /// unclassified; the structural parse error is reported by the metadata syntax check.
    pub(super) fn observe_metadata(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        if obu.header.extended_layer_id.is_global() {
            // A global *prefix* (metadata_is_suffix == 0) is a § 7.3.7 global prefix OBU.
            // A global *suffix* (metadata_is_suffix == 1) is part of a coded frame unit's
            // suffix tail; record it so a later global HLS prefix OBU is flagged. An
            // unreadable first bit is left unclassified (the metadata syntax check reports
            // the structural error).
            match metadata_is_suffix(obu) {
                Some(false) => self.observe_global_hls_prefix(obu, report),
                Some(true) => self.saw_global_suffix_metadata = true,
                None => {}
            }
        } else {
            // Non-global metadata sits inside a coded frame unit (§ 7.3.3 / § 7.3.4) of
            // its extended layer's coded extended layer unit, i.e. in the frame region.
            self.observe_coded_extended_layer_obu(obu, report);
        }
    }

    pub(super) fn start_temporal_unit(&mut self) {
        self.phase = TemporalUnitPhase::GlobalPrefix;
        self.current_coded_xlayer = None;
        self.reported_missing_delimiter = false;
        self.saw_obu_since_delimiter = false;
        self.saw_global_suffix_metadata = false;
        self.coded_frame_started_xlayer.clear();
    }

    pub(super) fn report_missing_delimiter_once(
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

    pub(super) fn observe_padding(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
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

    pub(super) fn observe_global_hls_prefix(
        &self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        // A global suffix metadata is more specific evidence that the prefix region
        // is over than merely being in the coded-layers phase, so prefer the
        // metadata-suffix rule when it applies (§ 7.3.7).
        if self.saw_global_suffix_metadata {
            report.push(ordering_error(
                "obu-order/global-hls-after-metadata-suffix",
                obu,
                format!(
                    "{} with GLOBAL_XLAYER_ID appears after a global suffix metadata OBU \
                     (metadata_is_suffix == 1); the global HLS prefix region must precede the \
                     coded extended layer units and their suffix metadata",
                    obu.header.obu_type.spec_name()
                ),
            ));
            return;
        }
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

    pub(super) fn observe_coded_extended_layer_obu(
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

        // AV2 § 7.3.6: a coded extended layer unit is ordered LCR → OPS → atlas →
        // sequence header → frame units. A non-global HLS *header* OBU (LCR / OPS /
        // atlas / sequence header) appearing after the frame region of this same
        // extended layer has begun is out of order. The frame region begins at the
        // first non-HLS-header coded-extended-layer OBU (a content-interpretation,
        // multi-frame-header, pre-frame BRT/QM/FGM/metadata, or frame-bearing OBU).
        let is_hls_header = matches!(
            obu.header.obu_type,
            ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
                | ObuType::SequenceHeader
        );
        if is_hls_header {
            if self.coded_frame_started_xlayer.contains(&xlayer) {
                // This rule belongs to § 7.3.6 (coded extended layer unit ordering: LCR
                // → OPS → atlas → sequence header → frame units), not the § 7.3.7
                // temporal-unit ordering that the shared `ordering_error` helper assumes;
                // override the section so the emitted spec_section matches the registry
                // entry (VALIDATOR-DIAGNOSTICS.md documents this diagnostic as § 7.3.6).
                report.push(
                    ordering_error(
                        "obu-order/non-global-hls-before-coded-layer",
                        obu,
                        format!(
                            "{} for obu_xlayer_id {} appears after the coded frame region of its \
                             coded extended layer unit has begun; the HLS header OBUs (LCR / OPS / \
                             atlas / sequence header) must precede the coded frame units",
                            obu.header.obu_type.spec_name(),
                            xlayer.get()
                        ),
                    )
                    .with_spec_section("7.3.6"),
                );
            }
        } else {
            // A frame-region OBU for this extended layer: record that the frame
            // region has begun, so a later HLS header for *that* layer fires — tracked
            // per extended layer so a reordered header for an earlier layer after a
            // later layer's frame region is not masked by the last-layer-only scalar.
            self.coded_frame_started_xlayer.insert(xlayer);
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemporalUnitPhase {
    #[default]
    AwaitingDelimiter,
    GlobalPrefix,
    CodedLayers,
}

pub(super) fn sequence_header_can_activate(obu: &ObuEnvelope<'_>) -> bool {
    !obu.header.extended_layer_id.is_global()
        && obu.header.temporal_layer_id.get() == 0
        && obu.header.embedded_layer_id.get() == 0
}

pub(super) fn requires_active_sequence(obu: &ObuEnvelope<'_>) -> bool {
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

pub(super) fn is_padding_obu(obu: &ObuEnvelope<'_>) -> bool {
    obu.header.obu_type == ObuType::Padding
}

pub(super) fn is_metadata_obu(obu: &ObuEnvelope<'_>) -> bool {
    matches!(
        obu.header.obu_type,
        ObuType::MetadataShort | ObuType::MetadataGroup
    )
}

/// Reads `metadata_is_suffix` (the first payload bit of both metadata OBU types,
/// AV2 § 5.17.2 / § 5.17.3), returning `None` if the payload is empty.
pub(super) fn metadata_is_suffix(obu: &ObuEnvelope<'_>) -> Option<bool> {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    reader.read_bit().ok().map(|bit| bit != 0)
}

pub(super) fn is_global_hls_prefix_obu(obu: &ObuEnvelope<'_>) -> bool {
    // AV2 § 7.3.7 lists the global temporal-unit prefix OBUs exhaustively: MSDO,
    // global LCR, global OPS, global atlas segment, and global *prefix* metadata. The two
    // metadata OBU types are NOT matched here: global metadata is classified by its
    // parsed metadata_is_suffix bit in observe_metadata (a suffix is not a prefix), so it
    // must not be matched as an unconditional global prefix.
    //
    // OBU_BUFFER_REMOVAL_TIMING is deliberately NOT in this set: § 7.3.7 does not list
    // it as a global prefix OBU, and § 7.3.3 / § 7.3.4 place a BRT inside a coded
    // frame unit at the frame's own xlayer (see is_coded_extended_layer_obu). A global
    // BRT therefore has no § 7.3.7 prefix position to enforce here, so it is left
    // unclassified rather than flagged — a sound-over-complete choice that avoids
    // false positives.
    //
    // TODO(spec: AV2-7.3-OBU-ORDERING): a hard `brt/global-ordering-position`
    // diagnostic for a global BRT would need the § 7.3.8 decoder-model / random-access
    // state that is not yet modeled.
    obu.header.extended_layer_id.is_global()
        && matches!(
            obu.header.obu_type,
            ObuType::Msdo
                | ObuType::LayerConfigurationRecord
                | ObuType::OperatingPointSet
                | ObuType::AtlasSegment
        )
}

pub(super) fn is_coded_extended_layer_obu(obu: &ObuEnvelope<'_>) -> bool {
    // A local (non-global) OBU_BUFFER_REMOVAL_TIMING falls here: AV2 § 7.3.3 / § 7.3.4
    // place it inside a coded output / non-output frame unit at the frame's xlayer, so
    // it follows the same coded-extended-layer classification as the frame OBUs.
    !obu.header.extended_layer_id.is_global()
        && !matches!(
            obu.header.obu_type,
            ObuType::TemporalDelimiter
                | ObuType::Padding
                | ObuType::Reserved0
                | ObuType::Reserved(_)
        )
}

pub(super) fn sequence_state_error(
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

pub(super) fn ordering_error(
    rule_id: &'static str,
    obu: &ObuEnvelope<'_>,
    message: String,
) -> Diagnostic {
    Diagnostic::error(rule_id, message)
        .with_spec_section("7.3.7")
        .with_byte_offset(obu.offset)
}
