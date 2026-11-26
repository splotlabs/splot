// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Layer metadata evidence from AV2 § 6.14 and § 7.3.8.11.

use super::*;

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
/// This API does not provide HDR persistence or frame-level metadata.
pub struct LayerMetadataTracker {
    context: ValidatorContext,
    options: ValidationOptions,
    report: ValidationReport,
    active_ci: BTreeMap<ExtendedLayerId, CiEpoch>,
    pending: bool,
}

impl LayerMetadataTracker {
    /// Creates an empty tracker with the supplied external HLS options.
    #[must_use]
    pub fn new(options: ValidationOptions) -> Self {
        Self {
            context: ValidatorContext::with_options(&options),
            options,
            report: ValidationReport::new(),
            active_ci: BTreeMap::new(),
            pending: false,
        }
    }

    /// Observes an OBU. Returns the completed TU index when a global
    /// temporal delimiter closes a nonempty TU.
    pub fn observe(&mut self, obu: &ObuEnvelope<'_>) -> Option<u64> {
        let delimiter = obu.header.obu_type == ObuType::TemporalDelimiter
            && obu.header.extended_layer_id.is_global();
        let completed = if delimiter { self.finish() } else { None };
        self.context
            .observe_obu(obu, &self.options, &mut self.report);
        self.report.diagnostics.clear();
        self.pending = !delimiter;
        completed
    }

    /// Completes the last pending TU at clean end of input.
    pub fn finish(&mut self) -> Option<u64> {
        if !std::mem::take(&mut self.pending) {
            return None;
        }
        let tu = self.context.cvs.tu_index;
        for (&xlayer, &(rap_tu, reset)) in &self.context.ci_rap_started_in_tu {
            if rap_tu != tu || xlayer.is_global() {
                continue;
            }
            if let Some(epoch) = self.resolve_ci(xlayer, tu, reset) {
                self.active_ci.insert(xlayer, epoch);
            } else {
                self.active_ci.remove(&xlayer);
            }
        }
        Some(tu)
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

    fn resolve_ci(&self, xlayer: ExtendedLayerId, tu: u64, reset: ByteOffset) -> Option<CiEpoch> {
        if self.context.frame_confirmed_activation_tu.get(&xlayer) != Some(&tu) {
            return None;
        }
        let (id, general) = self.context.frame_confirmed_activation_for(xlayer)?;
        let sequence_header = self.context.hls.sequence(xlayer, id)?.offset;
        let presence = general.mlayer_dependency_map.presence_map();
        let mut layers = Vec::new();
        for index in 0..=general.max_mlayer_id.get() {
            let mlayer = EmbeddedLayerId::from_bits(index);
            let value = if let Some(record) =
                self.context.content_interpretations.get(&(xlayer, mlayer))
                && record.tu_index == tu
            {
                CiLayer::Signaled(CiValue {
                    origin: record.offset,
                    inherited_from: Vec::new(),
                    content: record.content,
                })
            } else if matches!(self.options.external_hls, ExternalHlsMode::Disabled) {
                layers
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(source_index, source)| {
                        let source_id = EmbeddedLayerId::from_bits(source_index as u8);
                        if !presence.is_present(mlayer, source_id) {
                            return None;
                        }
                        let CiLayer::Signaled(value) = source else {
                            return None;
                        };
                        let mut value = value.clone();
                        value.inherited_from.insert(0, source_id);
                        Some(CiLayer::Signaled(value))
                    })
                    .unwrap_or(CiLayer::Unspecified)
            } else {
                CiLayer::Unavailable
            };
            layers.push(value);
        }
        Some(CiEpoch {
            reset,
            sequence_header,
            layers,
        })
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
