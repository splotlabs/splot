// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Layer metadata evidence from AV2 § 6.14 and § 7.3.8.11.

use super::*;
use splot_core::obu::{ParsedObu, PayloadStatus};

/// Effective content interpretation for one embedded layer at a completed TU.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectiveContentInterpretation<'a> {
    /// The observed stream does not establish the layer's state.
    Unavailable,
    /// The random access point established the AV2 unspecified defaults.
    Unspecified {
        /// Header offset of the CLK or OLK that initialized this state.
        reset: ByteOffset,
    },
    /// A CI OBU established the layer's parameters directly or by inheritance.
    Signaled {
        /// Header offset of the CLK or OLK that initialized this state.
        reset: ByteOffset,
        /// Header offset of the CI OBU that supplied the parameters.
        origin: ByteOffset,
        /// The source layers in order from the nearest source to the origin.
        /// Empty for a direct declaration.
        inherited_from: &'a [EmbeddedLayerId],
        /// Header offset of the activated sequence header. External sequence
        /// headers have no in-stream offset.
        sequence_header: Option<ByteOffset>,
        /// The parsed syntax of the source CI OBU.
        content: &'a ContentInterpretation,
    },
}

#[derive(Debug, Clone)]
struct CiValue {
    origin: ByteOffset,
    inherited_from: Vec<EmbeddedLayerId>,
    content: ContentInterpretation,
}

#[derive(Debug)]
struct CiEpoch {
    reset: ByteOffset,
    sequence_header: Option<ByteOffset>,
    layers: Vec<CiLayer>,
}

#[derive(Debug)]
enum CiLayer {
    Unavailable,
    Unspecified,
    Signaled(CiValue),
}

/// Tracks effective CI from ordered AV2 OBU envelopes for one stream.
///
/// The tracker uses in-band CI and frame-confirmed sequence headers. It
/// outputs evidence at completed temporal units because a later CLK or OLK
/// in the same unit can initialize CI that appears earlier in that unit.
/// HDR persistence and frame-level metadata are outside this API's scope.
/// The tracker retains one CI record per embedded layer in the current TU
/// and one HLS definition per ID. It does not accumulate validator diagnostics.
/// Decode output can expose a different set of metadata as picture fields.
pub struct LayerMetadataTracker {
    context: ValidatorContext,
    options: ValidationOptions,
    active_ci: BTreeMap<ExtendedLayerId, CiEpoch>,
    pending_ci: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), CiLayer>,
    pending_rap: BTreeMap<ExtendedLayerId, Option<Rap>>,
    seen_frames: BTreeSet<ExtendedLayerId>,
    tu_index: u64,
    pending: bool,
}

#[derive(Clone, Copy)]
struct Rap {
    reset: ByteOffset,
    sequence_header: Option<ByteOffset>,
    max_mlayer_id: EmbeddedLayerId,
    presence: MLayerPresenceMap,
}

impl LayerMetadataTracker {
    /// Creates an empty tracker with the supplied external HLS options.
    #[must_use]
    pub fn new(options: ValidationOptions) -> Self {
        Self {
            context: ValidatorContext::with_options(&options),
            options,
            active_ci: BTreeMap::new(),
            pending_ci: BTreeMap::new(),
            pending_rap: BTreeMap::new(),
            seen_frames: BTreeSet::new(),
            tu_index: 0,
            pending: false,
        }
    }

    /// Observes an OBU. Returns the completed TU index when a global
    /// temporal delimiter closes a nonempty TU.
    pub fn observe(&mut self, obu: &ObuEnvelope<'_>) -> Option<u64> {
        let xlayer = obu.header.extended_layer_id;
        let obu_type = obu.header.obu_type;
        if obu_type == ObuType::TemporalDelimiter && xlayer.is_global() {
            let completed = self.finish();
            self.tu_index = self.tu_index.saturating_add(1);
            return completed;
        }
        self.pending = true;
        if xlayer.get() >= 31 || obu.header.embedded_layer_id.get() >= 8 {
            return None;
        }
        match obu_type {
            ObuType::SequenceHeader if sequence_header_can_activate(obu) => {
                if let Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(header))) =
                    obu.payload_status()
                {
                    let id = header.general.seq_header_id;
                    self.context.hls.layer_mut(xlayer).sequences[usize::from(id.get())] =
                        Some(Box::new(StoredSequenceHeader {
                            header: *header,
                            offset: Some(obu.offset),
                            fingerprint: payload_fingerprint(obu.payload),
                        }));
                }
            }
            ObuType::MultiFrameHeader => {
                if let Ok(PayloadStatus::Parsed(ParsedObu::MultiFrameHeader(mfh))) =
                    obu.payload_status()
                {
                    self.context.hls.save_multi_frame_header(obu, &mfh);
                }
            }
            ObuType::ContentInterpretation => self.observe_ci(obu),
            _ if is_frame_bearing(obu_type) => {
                let first_picture = self.seen_frames.insert(xlayer);
                if matches!(obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey) {
                    let rap = self.resolve_rap(obu, first_picture);
                    self.pending_rap.insert(xlayer, rap);
                }
            }
            _ => {}
        }
        None
    }

    fn observe_ci(&mut self, obu: &ObuEnvelope<'_>) {
        let key = (obu.header.extended_layer_id, obu.header.embedded_layer_id);
        if let Ok(PayloadStatus::Parsed(ParsedObu::ContentInterpretation(content))) =
            obu.payload_status()
        {
            match self.pending_ci.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(CiLayer::Signaled(CiValue {
                        origin: obu.offset,
                        inherited_from: Vec::new(),
                        content,
                    }));
                }
                Entry::Occupied(mut entry) => {
                    if matches!(entry.get(), CiLayer::Signaled(value) if value.content != content) {
                        entry.insert(CiLayer::Unavailable);
                    }
                }
            }
        } else {
            self.pending_ci.insert(key, CiLayer::Unavailable);
        }
    }

    fn resolve_rap(&self, obu: &ObuEnvelope<'_>, first_picture: bool) -> Option<Rap> {
        let prefix = parse_frame_prefix(obu, first_picture)?;
        let mut report = ValidationReport::new();
        let id = self.context.resolve_frame_header_reference(
            &prefix,
            obu,
            &self.options,
            &mut report,
        )?;
        if !report.is_conformant() {
            return None;
        }
        let sequence = self
            .context
            .hls
            .sequence(obu.header.extended_layer_id, id)?;
        Some(Rap {
            reset: obu.offset,
            sequence_header: sequence.offset,
            max_mlayer_id: sequence.header.general.max_mlayer_id,
            presence: sequence.header.general.mlayer_dependency_map.presence_map(),
        })
    }

    /// Completes the last pending TU at clean end of input.
    pub fn finish(&mut self) -> Option<u64> {
        if !std::mem::take(&mut self.pending) {
            return None;
        }
        for (xlayer, rap) in std::mem::take(&mut self.pending_rap) {
            if let Some(rap) = rap {
                let epoch = self.resolve_ci(xlayer, &rap);
                self.active_ci.insert(xlayer, epoch);
            } else {
                self.active_ci.remove(&xlayer);
            }
        }
        self.pending_ci.clear();
        self.seen_frames.clear();
        Some(self.tu_index)
    }

    /// Returns CI evidence at the last completed temporal unit.
    #[must_use]
    pub fn content_interpretation(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
    ) -> EffectiveContentInterpretation<'_> {
        let Some(epoch) = self.active_ci.get(&xlayer) else {
            return EffectiveContentInterpretation::Unavailable;
        };
        match epoch.layers.get(usize::from(mlayer.get())) {
            Some(CiLayer::Signaled(value)) => EffectiveContentInterpretation::Signaled {
                reset: epoch.reset,
                origin: value.origin,
                inherited_from: &value.inherited_from,
                sequence_header: epoch.sequence_header,
                content: &value.content,
            },
            Some(CiLayer::Unspecified) => {
                EffectiveContentInterpretation::Unspecified { reset: epoch.reset }
            }
            Some(CiLayer::Unavailable) | None => EffectiveContentInterpretation::Unavailable,
        }
    }

    fn resolve_ci(&mut self, xlayer: ExtendedLayerId, rap: &Rap) -> CiEpoch {
        let mut layers = Vec::new();
        for index in 0..=rap.max_mlayer_id.get() {
            let mlayer = EmbeddedLayerId::from_bits(index);
            let value = if let Some(declaration) = self.pending_ci.remove(&(xlayer, mlayer)) {
                declaration
            } else if matches!(self.options.external_hls, ExternalHlsMode::Disabled) {
                layers
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(source_index, source)| {
                        let source_id = EmbeddedLayerId::from_bits(source_index as u8);
                        if !rap.presence.is_present(mlayer, source_id) {
                            return None;
                        }
                        match source {
                            CiLayer::Signaled(value) => {
                                let mut value = value.clone();
                                value.inherited_from.insert(0, source_id);
                                Some(CiLayer::Signaled(value))
                            }
                            CiLayer::Unavailable => Some(CiLayer::Unavailable),
                            CiLayer::Unspecified => None,
                        }
                    })
                    .unwrap_or(CiLayer::Unspecified)
            } else {
                CiLayer::Unavailable
            };
            layers.push(value);
        }
        CiEpoch {
            reset: rap.reset,
            sequence_header: rap.sequence_header,
            layers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

    #[test]
    fn content_interpretation_at_completed_tu_uses_rap_and_ci_offsets() {
        let bytes = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-output-ci-2frame-64x64.ivf"
        );
        let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
            panic!("Expected an IVF stream.");
        };
        let mut tracker = LayerMetadataTracker::new(ValidationOptions::default());
        let x = ExtendedLayerId::from_bits(0);
        let m = EmbeddedLayerId::BASE;
        for obu in &parsed.frames[0].obus {
            assert_eq!(tracker.observe(obu), None);
        }
        assert_eq!(
            tracker.content_interpretation(x, m),
            EffectiveContentInterpretation::Unavailable
        );
        assert_eq!(tracker.observe(&parsed.frames[1].obus[0]), Some(1));
        assert!(matches!(
            tracker.content_interpretation(x, m),
            EffectiveContentInterpretation::Signaled { reset, origin, inherited_from, .. }
                if reset.get() == 83 && origin.get() == 71 && inherited_from.is_empty()
        ));
        assert_eq!(
            tracker.content_interpretation(GLOBAL_XLAYER_ID, m),
            EffectiveContentInterpretation::Unavailable
        );
    }

    #[test]
    fn content_interpretation_resets_at_olk_during_sequential_inspection() {
        let bytes = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-frame-olk-leading-64x64.ivf"
        );
        let ci_fixture = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-output-ci-2frame-64x64.ivf"
        );
        let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
            panic!("Expected an IVF stream.");
        };
        let ci = parse_annex_b_obus(&ci_fixture[70..82]).unwrap()[0];
        let mut tracker = LayerMetadataTracker::new(ValidationOptions::default());
        let x = ExtendedLayerId::from_bits(0);
        let m = EmbeddedLayerId::BASE;
        for obu in &parsed.frames[0].obus[..2] {
            tracker.observe(obu);
        }
        tracker.observe(&ObuEnvelope {
            offset: ByteOffset::new(69),
            ..ci
        });
        tracker.observe(&parsed.frames[0].obus[2]);
        for obu in &parsed.frames[1].obus[..3] {
            if tracker.observe(obu).is_some() {
                assert!(matches!(
                    tracker.content_interpretation(x, m),
                    EffectiveContentInterpretation::Signaled { origin, .. } if origin.get() == 69
                ));
            }
        }
        assert_eq!(tracker.observe(&parsed.frames[1].obus[3]), Some(2));
        assert!(matches!(
            tracker.content_interpretation(x, m),
            EffectiveContentInterpretation::Unspecified { reset } if reset.get() == 141
        ));
    }

    #[test]
    fn content_interpretation_inherits_and_resets_in_a_new_cvs() {
        let multilayer = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-mlayer2-reproducible-8bit.obu"
        );
        let ci_fixture = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-output-ci-2frame-64x64.ivf"
        );
        // Insert the independent CI OBU before the first multilayer CLK.
        let mut bytes = multilayer[..33].to_vec();
        bytes.extend_from_slice(&ci_fixture[70..82]);
        bytes.extend_from_slice(&multilayer[33..525]);
        // The next CVS resends the same frame and sequence data with no CI.
        bytes.extend_from_slice(&multilayer[..525]);
        let obus = parse_annex_b_obus(&bytes).unwrap();
        let mut tracker = LayerMetadataTracker::new(ValidationOptions::default());
        let x = ExtendedLayerId::from_bits(0);
        let enhancement = EmbeddedLayerId::from_bits(1);
        for obu in obus {
            if tracker.observe(&obu).is_some() {
                assert!(matches!(
                    tracker.content_interpretation(x, enhancement),
                    EffectiveContentInterpretation::Signaled { origin, inherited_from, .. }
                        if origin.get() == 34 && inherited_from == [EmbeddedLayerId::BASE]
                ));
            }
        }
        assert!(tracker.finish().is_some());
        assert!(matches!(
            tracker.content_interpretation(x, enhancement),
            EffectiveContentInterpretation::Unspecified { reset } if reset.get() == 572
        ));
    }
}
