// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-bearing OBU parsing helpers and frame-unit fact derivation.

use super::*;

pub(super) fn is_frame_bearing(obu_type: ObuType) -> bool {
    obu_type.is_tile_group()
        || obu_type.is_sef()
        || obu_type.is_tip_frame()
        || obu_type == ObuType::BridgeFrame
}

/// The § 7.3.6 all-leading-or-none [`Leadingness`] derived from `obu_type`, mirroring AVM's
/// tri-state `is_leading_picture` (`av2/decoder/obu.c:2544-2549`) rather than the § 6.4.1-area
/// gloss (`06-syntax-structures-semantics.md:4546`) that reads `IsRegular == 0` as exactly
/// "leading":
///
/// - the `av2_is_leading_vcl_obu` set (`av2/decoder/obu.c:1666` — `OBU_LEADING_TILE_GROUP`,
///   `OBU_LEADING_SEF`, `OBU_LEADING_TIP`) is [`Leadingness::Leading`];
/// - the `av2_is_regular_vcl_obu` set (`av2/decoder/decodeframe.c:7015` — `OLK` plus
///   `REGULAR_TILE_GROUP` / `REGULAR_SEF` / `REGULAR_TIP` / `SWITCH` / `RAS` / `BRIDGE`,
///   i.e. the § 5.18.2 `IsRegular == 1` set) is [`Leadingness::Regular`];
/// - a CLK lands in neither AVM set, so the oracle leaves `is_leading_picture == -1`; the
///   validator follows it and classes a CLK [`Leadingness::Indeterminate`], excluding it
///   from the all-leading-or-none judgment (the documented ambiguous-spec under-report).
///
/// Type-decided, so the § 7.3.6 all-leading-or-none rule never routes to Unknown.
pub(super) fn frame_leadingness(obu_type: ObuType) -> Leadingness {
    match obu_type {
        ObuType::LeadingTileGroup | ObuType::LeadingSef | ObuType::LeadingTip => {
            Leadingness::Leading
        }
        ObuType::OpenLoopKey
        | ObuType::RegularTileGroup
        | ObuType::RegularTip
        | ObuType::RegularSef
        | ObuType::Switch
        | ObuType::RasFrame
        | ObuType::BridgeFrame => Leadingness::Regular,
        _ => Leadingness::Indeterminate,
    }
}

/// Parses a frame-bearing OBU's frame-header prefix (best-effort), returning `None`
/// on any parse failure or when no parseable frame header is present (an absent
/// header or a non-first tile group's `frame_header_copy()`).
pub(super) fn parse_frame_prefix(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
) -> Option<FrameHeaderPrefix> {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu.header.obu_type.is_tile_group() {
        parse_tile_group_prefix(&mut reader, obu.header.obu_type, Some(first_picture_in_tu))
            .ok()
            .and_then(|tile_group| tile_group.frame_header)
    } else {
        parse_frame_header_prefix(&mut reader, obu.header.obu_type, Some(first_picture_in_tu)).ok()
    }
}

/// Parses the frame-header core of a frame-bearing OBU against its active sequence
/// header (AV2 § 5.18.2), positioning the reader past the `tile_group_obu()` prefix
/// for tile-group OBUs. Returns `Ok(None)` when there is no parseable first-tile-group
/// frame header and propagates core parser errors to callers that own diagnostics.
///
/// `mfh_record` is the in-band multi-frame header resolving this frame's `cur_mfh_id`
/// (`> 0`) reference, or `None` for a `cur_mfh_id == 0` direct reference (or when the
/// MFH is unavailable). It is threaded into the core parser so the `cur_mfh_id > 0`
/// paths can resolve their multi-frame-header-derived state as that coverage lands; the
/// currently-reachable fields (the §5.18.2 control region through the output flags) are
/// determined by the active sequence header alone.
pub(super) fn parse_frame_core(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    reference_state: &FrameReferenceStateView<'_>,
) -> splot_core::Result<Option<FrameHeaderCore>> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu_type.is_tile_group() {
        if reader.read_bit()? == 0 {
            return Ok(None);
        }
    } else if !is_frame_bearing(obu_type) {
        return Ok(None);
    }
    let input = FrameHeaderParseInput {
        obu_type,
        first_picture_in_tu,
        active_sequence: Some(active_sequence),
        mfh_record,
        reference_state: *reference_state,
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map(Some)
}

impl ValidatorContext {
    pub(super) fn frame_core_against_resolved_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        seq_id: SequenceHeaderId,
    ) -> Option<FrameHeaderCore> {
        let active_sequence = self.sequence_headers.get(&seq_id)?;

        let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);

        let mut reference_scratch = ReferenceStateScratch::default();
        let reference_state = self
            .reference_state
            .view_into(obu.header.extended_layer_id, &mut reference_scratch)
            .unwrap_or_else(FrameReferenceStateView::unknown);

        let core = parse_frame_core(
            obu,
            first_picture_in_tu,
            active_sequence,
            mfh_record,
            &reference_state,
        )
        .ok()
        .flatten()?;

        let referenced = if core.cur_mfh_id.is_zero() {
            core.referenced_sequence_header_id
        } else {
            mfh_record.map(|record| record.mfh_seq_header_id)
        };
        (referenced == Some(seq_id)).then_some(core)
    }

    /// Classifies `obu` into its coded-frame-unit [`SegRole`] (AV2 § 7.3.3 /
    /// § 7.3.4). The frame-header-derived facts come from the same best-effort
    /// parse the activation path uses; any field the parse cannot reach is left
    /// `None`, which the segmenter treats as undecidable (routing the unit to
    /// Unknown for the output class, or skipping a first-tile-group check).
    pub(super) fn seg_role_for(&self, obu: &ObuEnvelope<'_>, first_picture_in_tu: bool) -> SegRole {
        let obu_type = obu.header.obu_type;
        if obu_type == ObuType::Padding {
            return SegRole::Padding;
        }
        match obu_type {
            ObuType::ContentInterpretation => SegRole::ContentInterpretation,
            ObuType::MultiFrameHeader => SegRole::MultiFrameHeader,
            ObuType::BufferRemovalTiming => SegRole::BufferRemovalTiming,
            ObuType::QuantizationMatrix => SegRole::QuantizationMatrix,
            ObuType::FilmGrain => SegRole::FilmGrain,
            ObuType::MetadataShort | ObuType::MetadataGroup => SegRole::Metadata {
                is_suffix: metadata_is_suffix(obu),
            },
            ObuType::LeadingSef | ObuType::RegularSef => SegRole::SefFrame,
            ObuType::BridgeFrame => SegRole::BridgeFrame {
                output: self.bridge_output_class(obu),
            },
            ObuType::LeadingTip | ObuType::RegularTip => SegRole::TipFrame {
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            _ if obu_type.is_tile_group() => SegRole::TileFrame {
                is_first_tile_group: Self::frame_is_first_tile_group(obu),
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            _ => SegRole::Padding,
        }
    }

    /// Returns the output class implied by the active sequence header for a bridge frame.
    /// AV2 § 5.18.2 makes every single-picture frame an immediate output frame; a bridge in
    /// a video sequence remains the § 7.3.4 non-output bridge form.
    fn bridge_output_class(&self, obu: &ObuEnvelope<'_>) -> Option<bool> {
        let seq_id = self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)?;
        let seq = self.sequence_headers.get(seq_id)?;
        Some(seq.general.single_picture_header_flag)
    }

    /// Reads `is_first_tile_group` from a tile-group OBU's prefix (AV2 § 5.19),
    /// `None` if the first bit cannot be read.
    pub(super) fn frame_is_first_tile_group(obu: &ObuEnvelope<'_>) -> Option<bool> {
        if !obu.header.obu_type.is_tile_group() {
            return None;
        }
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        reader.read_bit().ok().map(|bit| bit != 0)
    }

    /// Resolves the frame prefix’s multi-frame-header reference against available in-band
    /// HLS. Missing or incomplete prefixes and unavailable records return `None`.
    pub(super) fn resolve_frame_mfh_record(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        seq_id: SequenceHeaderId,
    ) -> Option<&MultiFrameHeaderRecord> {
        let prefix = parse_frame_prefix(obu, first_picture_in_tu)?;
        if prefix.cur_mfh_id.is_zero() || !prefix.cur_mfh_id.in_range() {
            return None;
        }
        let record = self.hls.multi_frame_header(prefix.cur_mfh_id)?;
        (record.mfh_seq_header_id == seq_id).then_some(record)
    }

    pub(super) fn frame_core_against_referenced_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<FrameHeaderCore> {
        let seq_id = *self
            .active_sequence_by_xlayer
            .get(&obu.header.extended_layer_id)?;
        self.frame_core_against_resolved_header(obu, first_picture_in_tu, seq_id)
    }

    /// Derives a frame-bearing OBU's output class (`immediate_output_frame == 1 ||
    /// implicit_output_frame == 1`, AV2 § 7.3.3 / § 6.17.2) from a best-effort core
    /// parse against its active sequence header. `None` (undecidable) when the
    /// active sequence is unavailable, the frame's referenced sequence header is not the
    /// active header parsed against ([`Self::frame_core_against_referenced_header`]), or the
    /// core parse stops before the output flags — which routes the unit to Unknown rather
    /// than guessing.
    pub(super) fn frame_output_class(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<bool> {
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu)?;
        match (core.immediate_output_frame, core.implicit_output_frame) {
            (Some(immediate), Some(implicit)) => Some(immediate || implicit),
            (Some(true), _) | (_, Some(true)) => Some(true),
            _ => None,
        }
    }

    /// Classifies a **non-frame-bearing** `obu` into its coded-extended-layer-unit
    /// [`CeluRole`] (AV2 § 7.3.6), parallel to [`Self::seg_role_for`]. The HLS headers (LCR /
    /// OPS / atlas / sequence header) and content-interpretation map directly; all other
    /// coded-extended-layer-interior OBUs (BRT / QM / FGM / metadata / MFH) are `FrameInterior`;
    /// padding is position-free. Frame-bearing OBUs are dispatched by the caller (see
    /// [`Self::observe_frame_bearing_obu`]) so their facts and OrderHintBits come from a single
    /// shared parse + resolution; if one reaches here it is treated as transparent padding.
    #[allow(clippy::match_same_arms)]
    pub(super) fn celu_role_for(obu: &ObuEnvelope<'_>) -> CeluRole {
        match obu.header.obu_type {
            ObuType::Padding => CeluRole::Padding,
            ObuType::LayerConfigurationRecord => CeluRole::LayerConfigurationRecord,
            ObuType::OperatingPointSet => CeluRole::OperatingPointSet,
            ObuType::AtlasSegment => CeluRole::AtlasSegment,
            ObuType::SequenceHeader => CeluRole::SequenceHeader,
            ObuType::ContentInterpretation => CeluRole::ContentInterpretation,
            ObuType::BufferRemovalTiming
            | ObuType::QuantizationMatrix
            | ObuType::FilmGrain
            | ObuType::MetadataShort
            | ObuType::MetadataGroup
            | ObuType::MultiFrameHeader => CeluRole::FrameInterior,
            _ => CeluRole::Padding,
        }
    }

    /// Derives the [`FrameFacts`] for a frame-bearing OBU from a best-effort core parse
    /// against its active sequence header (AV2 § 5.18.2). Leading-ness is type-decided from
    /// `obu_type` (see [`frame_leadingness`]), so it never routes to Unknown; the output
    /// class and `order_hint` are `None` when the parse stops before them or the active
    /// sequence header is unavailable (the Unknown invariant).
    ///
    /// The coded-frame-unit `boundary` is the [`FrameUnitSegmenter`]'s authoritative signal
    /// for this OBU (the segmenter is the single source of truth for coded-frame-unit
    /// boundaries, § 7.3.6); the CELU tracker consumes it rather than re-deriving boundaries.
    /// `boundary` is `None` only for a *global* frame-bearing OBU (the segmenter ignores
    /// globals), which the CELU tracker also filters before it reads this field — so the
    /// `OpensNewUnit` fallback is never observed.
    pub(super) fn frame_celu_facts(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
    ) -> (FrameFacts, Option<u32>) {
        let obu_type = obu.header.obu_type;
        let leadingness = frame_leadingness(obu_type);

        let type_decided = if obu_type == ObuType::BridgeFrame {
            self.bridge_output_class(obu)
        } else {
            type_decided_output(obu_type)
        };

        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu);
        let (flag_output, order_hint, bits) = match &core {
            Some(core) => {
                let flag_output = match (core.immediate_output_frame, core.implicit_output_frame) {
                    (Some(immediate), Some(implicit)) => Some(immediate || implicit),
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    _ => None,
                };
                let bits = self
                    .active_sequence_by_xlayer
                    .get(&obu.header.extended_layer_id)
                    .and_then(|seq_id| self.sequence_headers.get(seq_id))
                    .and_then(|seq| seq.inter.as_ref())
                    .map(|inter| u32::from(inter.order_hint_bits));
                (flag_output, core.order_hint_lsb, bits)
            }
            None => (None, None, None),
        };

        let output = type_decided.or(flag_output);

        (
            FrameFacts {
                obu_type,
                boundary: boundary.unwrap_or(FrameBoundary::OpensNewUnit),
                output,
                order_hint,
                order_hint_bits: bits,
                leadingness,
            },
            bits,
        )
    }
}
