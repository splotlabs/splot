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
        // A CLK is neither leading nor regular under the AVM tri-state (the § 6.4.1 gloss
        // would call it leading; the oracle and this validator do not).
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
        // Tile-group OBUs carry tile_group_obu(); a frame header is parseable only for
        // the first tile group (a non-first tile group carries frame_header_copy()).
        parse_tile_group_prefix(&mut reader, obu.header.obu_type, Some(first_picture_in_tu))
            .ok()
            .and_then(|tile_group| tile_group.frame_header)
    } else {
        // SEF / TIP / bridge frames call frame_header( 1 ) directly (AV2 § 5.2.1).
        parse_frame_header_prefix(&mut reader, obu.header.obu_type, Some(first_picture_in_tu)).ok()
    }
}

/// Parses the frame-header core of a frame-bearing OBU against its active sequence
/// header (AV2 § 5.18.2), positioning the reader past the `tile_group_obu()` prefix
/// for tile-group OBUs. Returns `None` when there is no parseable first-tile-group
/// frame header or the core parse fails (best-effort, never an error).
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
    reference_state: FrameReferenceStateView<'_>,
) -> Option<FrameHeaderCore> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu_type.is_tile_group() {
        // tile_group_obu(): only the first tile group carries a parseable frame_header(1);
        // its frame_header_present_flag is inferred 1 (AV2 § 5.19).
        if reader.read_bit().ok()? == 0 {
            return None;
        }
    } else if !is_frame_bearing(obu_type) {
        return None;
    }
    let input = FrameHeaderParseInput {
        obu_type,
        first_picture_in_tu,
        active_sequence: Some(active_sequence),
        mfh_record,
        // AV2 § 7.23: the modeled per-extended-layer reference-frame buffer view. No
        // §5.18 INTRA parse branch consumes it today (the intra paths derive their state
        // without RefValid/RefOrderHint/dims); it is forward plumbing so the §5.18 INTER
        // reference paths (explicit reference map, frame_size_with_refs, primary-ref) can
        // read the modeled state once they land (AV2-5.18.2-FRAME-HEADER-INFO inter path)
        // without changing the parser's call signature. The validator already consumes
        // the modeled state directly for the §6.17.2 show-existing-frame slot check.
        reference_state,
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).ok()
}

impl ValidatorContext {
    pub(super) fn frame_core_against_resolved_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        seq_id: SequenceHeaderId,
    ) -> Option<FrameHeaderCore> {
        let active_sequence = self.sequence_headers.get(&seq_id)?;

        // Resolve the frame's `cur_mfh_id` (> 0) reference to its in-band multi-frame header,
        // so the parser can be invoked with the resolving record (shared §7.3.8.7 discipline).
        let mfh_record = self.resolve_frame_mfh_record(obu, first_picture_in_tu, seq_id);

        // AV2 § 7.23: thread the modeled per-extended-layer reference-frame buffer view into
        // the core parse (no §5.18 reference read precedes the §5.18.2 reset_qm() call site, so
        // the `reached_qm_reset` fact this caller reads is independent of the buffer view). The
        // scratch arrays must outlive the parse, so they are stack-local here.
        let mut ref_valid = [false; NUM_REF_FRAMES];
        let mut ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        let reference_state = if self
            .reference_state
            .view_into(
                obu.header.extended_layer_id,
                &mut ref_valid,
                &mut ref_oh,
                &mut ref_w,
                &mut ref_h,
            )
            .is_some()
        {
            FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h)
        } else {
            FrameReferenceStateView::unknown()
        };

        let core = parse_frame_core(
            obu,
            first_picture_in_tu,
            active_sequence,
            mfh_record,
            reference_state,
        )?;

        // The frame must reference `seq_id` (the header parsed against): for a `cur_mfh_id == 0`
        // direct reference, the prefix's resolved id; for a `cur_mfh_id > 0` reference, the
        // resolved in-band MFH record's `mfh_seq_header_id` (§ 7.3.8.7).
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
            ObuType::BridgeFrame => SegRole::BridgeFrame,
            ObuType::LeadingTip | ObuType::RegularTip => SegRole::TipFrame {
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            _ if obu_type.is_tile_group() => SegRole::TileFrame {
                is_first_tile_group: self.frame_is_first_tile_group(obu),
                output: self.frame_output_class(obu, first_picture_in_tu),
            },
            // Sequence headers, LCR/OPS/atlas/MSDO, temporal delimiters, reserved:
            // not part of a coded frame unit's grammar (§ 7.3.3 / § 7.3.4 list none
            // of them). They live at the temporal-unit / coded-extended-layer level
            // and are ordered by the § 7.3.7 / § 7.3.6 machinery. Map to Padding so
            // the segmenter treats them as position-free separators (they neither
            // start nor advance a coded frame unit).
            _ => SegRole::Padding,
        }
    }

    /// Reads `is_first_tile_group` from a tile-group OBU's prefix (AV2 § 5.19),
    /// `None` if the first bit cannot be read.
    // Member of the `&self` `ValidatorContext` frame-fact method family (dispatched as
    // `self.frame_is_first_tile_group(obu)` beside `self`-reading siblings); the uniform
    // receiver keeps the family consistent.
    #[allow(clippy::unused_self)]
    pub(super) fn frame_is_first_tile_group(&self, obu: &ObuEnvelope<'_>) -> Option<bool> {
        if !obu.header.obu_type.is_tile_group() {
            return None;
        }
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        reader.read_bit().ok().map(|bit| bit != 0)
    }

    /// Parses a frame-bearing OBU's [`FrameHeaderCore`] against the layer's active sequence
    /// header, but **only** returns it when the frame's referenced sequence header resolved
    /// to the very header parsed against (AV2 § 5.18.2). `None` (undecidable → Unknown
    /// routing) when:
    ///
    /// - no sequence header is active for the layer, or its stored header is missing;
    /// - the core parse failed (a payload the skeleton cannot reach); or
    /// - the frame's referenced sequence header is **not** the active header parsed against.
    ///
    /// A frame's referenced sequence header is the active header when **either**:
    ///
    /// - **`cur_mfh_id == 0`** (direct reference) and the §5.18.2 prefix's
    ///   `referenced_sequence_header_id` (set only when `seq_header_id_in_frame_header` is in
    ///   range) equals the parsed-against id; or
    /// - **`cur_mfh_id > 0`** (multi-frame-header reference) and an *in-band* multi-frame
    ///   header record resolves that `cur_mfh_id` (in range, present in
    ///   [`HlsAvailabilityStore::multi_frame_header`]) whose `mfh_seq_header_id` equals the
    ///   parsed-against id (§ 7.3.8.7). The §5.18.2 control region through the output flags is
    ///   determined by the active (== resolved) sequence header alone, so the output class /
    ///   `order_hint` are decidable on this path even though `referenced_sequence_header_id` is
    ///   `None` (the prefix leaves it unset for `cur_mfh_id > 0`).
    ///
    /// External-HLS caveat: an MFH only *externally* declared (`ExternalHlsMode::Provided`, not
    /// in-band) is **not** a verifiable association — `multi_frame_header` returns `None` for
    /// it — so the frame stays Unknown (the PR #49 partial-declaration policy). An out-of-range
    /// `cur_mfh_id`, an absent record, or an MFH whose `mfh_seq_header_id` names a different
    /// header all keep Unknown.
    ///
    /// This is the stale-activation safety: the sequence-header-dependent field widths
    /// (`order_hint` is `f(OrderHintBits)`, etc.) make any post-prefix field a misparse when
    /// read against the wrong header, so the output class and `order_hint` would be garbage. The
    /// activation/reference prefix (`cur_mfh_id`, `seq_header_id_in_frame_header`,
    /// `referenced_sequence_header_id`) is parsed *before* any sequence-dependent field, so it
    /// stays reliable even when the parse ran against a stale header — making the resolution
    /// check sound. The same guard is applied by the frame-unit segmenter's output-class
    /// derivation ([`Self::frame_output_class`]) so the two layers route to Unknown together.
    /// Resolves a frame's `cur_mfh_id` (`> 0`) reference to the in-band multi-frame
    /// header record the `cur_mfh_id > 0` core parse must consume, with the §7.3.8.7
    /// resolution discipline (AV2 § 5.18.2): the lightweight prefix parse reads only the
    /// activation fields (`cur_mfh_id` is before any sequence-dependent field, so it is
    /// reliable even against a stale active header), the `cur_mfh_id` must be nonzero
    /// and in range, an in-band record must resolve it, and that record's
    /// `mfh_seq_header_id` must equal `seq_id` — the sequence header the frame is parsed
    /// against. `None` for a `cur_mfh_id == 0` direct reference, an out-of-range
    /// `cur_mfh_id`, an absent record, or a record naming a different sequence header;
    /// the core parser then keeps its `cur_mfh_id > 0`-unresolvable early-stop rather
    /// than guessing a multi-frame-header-derived size.
    ///
    /// Shared by [`Self::frame_core_against_referenced_header`] (output-class /
    /// reference-header derivation) and [`frame_header_core_checks`] (frame-header
    /// diagnostics) so the resolution predicate has a single definition.
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
        // §7.3.8.7: the resolved record must name the sequence header parsed against,
        // otherwise the multi-frame-header state would be applied against the wrong
        // maxima; a mismatch keeps the unresolvable early-stop.
        (record.mfh_seq_header_id == seq_id).then_some(record)
    }

    pub(super) fn frame_core_against_referenced_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<FrameHeaderCore> {
        // The extended layer's currently active (§5.18.2-confirmed) sequence header is the one
        // this frame parses against. The actual parse + referenced-header guard is shared with
        // the pre-activation reset path via `frame_core_against_resolved_header`.
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
            // One flag known and already true settles the output class; the other
            // being unreached cannot flip an output frame to non-output.
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
    // Parallel to the `&self` `Self::seg_role_for`; dispatched as `self.celu_role_for(obu)`,
    // so the uniform receiver keeps the role-classifier method family consistent.
    #[allow(clippy::unused_self)]
    // The explicit `Padding` arm and the reserved-type catch-all both yield the transparent
    // `Padding` role but are kept distinct on purpose: the named arm documents the actual
    // OBU_PADDING mapping, the `_` arm the § 7.3.6 "ignore undefined types" rule.
    #[allow(clippy::match_same_arms)]
    pub(super) fn celu_role_for(&self, obu: &ObuEnvelope<'_>) -> CeluRole {
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
            // Reserved types (and the global-only temporal delimiter / MSDO, which the
            // tracker filters as global) are ignored by the § 7.3.6 grammar ("OBU types that
            // are not defined in this specification can be ignored", mirror line 618). Map to
            // Padding so they are transparent — neither opening a frame nor advancing an HLS
            // phase.
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

        // F3: the output class is TYPE-DECIDED for a SEF (§ 7.3.3 "Or" branch -> output) and a
        // BRIDGE (§ 7.3.4 list only -> non-output) by `obu_type` alone, BEFORE consulting any
        // parsed flag — `type_decided_output` is the single source of truth shared with the
        // frame-unit segmenter. A bridge parser stops early and would otherwise route to Unknown,
        // suppressing the § 7.3.6 presence checks; the type decision keeps it decided.
        let type_decided = type_decided_output(obu_type);

        // F4: one core parse + resolution drives BOTH the flag-derived facts AND the OrderHintBits
        // contribution. `frame_core_against_referenced_header` returns `Some` only when the
        // frame's referenced sequence header resolved to the active header it parsed against
        // (the stale-activation guard). When it resolves, the active header IS the referenced
        // one, so its `OrderHintBits` is this frame's bits; when it does not resolve, the bits
        // contribution is `None` (not the stale active header's bits) so the § 7.3.7
        // same-OrderHintBits-in-TU check is never fed a wrong-bits value.
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu);
        let (flag_output, order_hint, bits) = match &core {
            Some(core) => {
                let flag_output = match (core.immediate_output_frame, core.implicit_output_frame) {
                    (Some(immediate), Some(implicit)) => Some(immediate || implicit),
                    // One flag known and already true settles output; the other being unreached
                    // cannot flip an output frame to non-output (mirror § 6.17.2).
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    _ => None,
                };
                // The resolved frame's OrderHintBits is the active (== referenced) header's,
                // when its inter config was parsed (the Unknown invariant otherwise).
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

        // The type decision wins when present; otherwise the flag-derived class (Unknown when
        // the parse did not resolve / reach the flags). `order_hint` is the parsed `order_hint_lsb`
        // (the LSB proxy, see [`crate::celu`]); a SEF/bridge with an absent reference keeps its
        // type-decided output but contributes no order_hint / bits.
        let output = type_decided.or(flag_output);

        (
            FrameFacts {
                obu_type,
                boundary: boundary.unwrap_or(FrameBoundary::OpensNewUnit),
                output,
                order_hint,
                // Round-6 F2: carry the per-frame OrderHintBits into the facts so the CELU
                // tracker can gate the cross-CELU §7.3.7 OrderHint comparison on only the two
                // COMPARED output units' bits being known and equal — the SAME resolved bits
                // value also threaded TU-wide to `note_order_hint_bits` for the same-bits
                // judgment (constraint 1). A frame whose referenced header did not resolve
                // contributes `None` here too (the stale-activation guard).
                order_hint_bits: bits,
                leadingness,
            },
            bits,
        )
    }
}
