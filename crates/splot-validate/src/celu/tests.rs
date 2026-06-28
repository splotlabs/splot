// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the [`super`] CELU (§ 7.3.6) and DOH (§ 7.3.7) state machine.

use super::*;
use splot_core::obu::ObuHeader;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};

/// Synthetic OBU envelope for feeding the tracker directly. The payload is unused
/// (the tracker holds no parser state — facts arrive via [`CeluRole`]); the offset is
/// the running OBU index, so anchors are distinguishable.
fn obu(obu_type: ObuType, xlayer: u8, mlayer: u8, offset: u64) -> ObuEnvelope<'static> {
    ObuEnvelope {
        offset: ByteOffset::new(offset),
        size: 1,
        header: ObuHeader {
            has_header_extension: true,
            obu_type,
            temporal_layer_id: TemporalLayerId::from_bits(0),
            embedded_layer_id: EmbeddedLayerId::from_bits(mlayer),
            extended_layer_id: ExtendedLayerId::from_bits(xlayer),
            header_size_bytes: 2,
        },
        payload: &[],
    }
}

/// A frame `CeluRole` with the given facts and an explicit segmenter
/// [`FrameBoundary`]. The `opens` flag is mapped to
/// [`FrameBoundary::OpensNewUnit`] / [`FrameBoundary::ContinuesUnit`]; the
/// [`FrameBoundary::Ambiguous`] case is exercised by [`ambiguous_role`].
fn frame_role(
    obu_type: ObuType,
    opens: bool,
    output: Option<bool>,
    order_hint: Option<u32>,
    leadingness: Leadingness,
) -> CeluRole {
    frame_role_with_boundary(
        obu_type,
        if opens {
            FrameBoundary::OpensNewUnit
        } else {
            FrameBoundary::ContinuesUnit
        },
        output,
        order_hint,
        None,
        leadingness,
    )
}

/// A frame `CeluRole` carrying an explicit segmenter [`FrameBoundary`] and per-frame
/// `order_hint_bits` (round-6 F2: the cross-CELU §7.3.7 comparison gate reads the output
/// units' own bits) — the tracker consumes the segmenter's boundary verbatim (§ 7.3.6),
/// so the tests drive both directly.
fn frame_role_with_boundary(
    obu_type: ObuType,
    boundary: FrameBoundary,
    output: Option<bool>,
    order_hint: Option<u32>,
    order_hint_bits: Option<u32>,
    leadingness: Leadingness,
) -> CeluRole {
    CeluRole::Frame(FrameFacts {
        obu_type,
        boundary,
        output,
        order_hint,
        order_hint_bits,
        leadingness,
    })
}

/// A frame `CeluRole` whose segmenter boundary is [`FrameBoundary::Ambiguous`] (a
/// same-type no-delimiter TIP, or an unreadable tile-group delimiter).
fn ambiguous_role(
    obu_type: ObuType,
    output: Option<bool>,
    order_hint: Option<u32>,
    leadingness: Leadingness,
) -> CeluRole {
    frame_role_with_boundary(
        obu_type,
        FrameBoundary::Ambiguous,
        output,
        order_hint,
        None,
        leadingness,
    )
}

/// A simple output tile-group frame at (xlayer, mlayer) with a given OrderHint and no
/// declared OrderHintBits — for tests that exercise the OrderHint judgments without the
/// cross-CELU §7.3.7 bits gate (the gate drops a pair with unknown bits, so a cross-CELU
/// mismatch test must use [`output_frame_bits`] to declare equal known bits).
fn output_frame(order_hint: u32) -> CeluRole {
    frame_role(
        ObuType::RegularTileGroup,
        true,
        Some(true),
        Some(order_hint),
        Leadingness::Regular,
    )
}

/// An output tile-group frame with a given OrderHint and explicit `OrderHintBits`
/// (round-6 F2): the cross-CELU §7.3.7 OrderHint comparison is gated on the two compared
/// output units' own bits being known and equal, so a cross-CELU mismatch/agreement test
/// declares those bits here rather than only via the TU-wide `note_order_hint_bits`.
fn output_frame_bits(order_hint: u32, order_hint_bits: u32) -> CeluRole {
    frame_role_with_boundary(
        ObuType::RegularTileGroup,
        FrameBoundary::OpensNewUnit,
        Some(true),
        Some(order_hint),
        Some(order_hint_bits),
        Leadingness::Regular,
    )
}

fn has(report: &ValidationReport, rule: &str) -> bool {
    report.errors().any(|d| d.rule_id == rule)
}

fn fresh() -> CodedExtendedLayerTracker {
    CodedExtendedLayerTracker::default()
}

// --- in-unit ordering (HLS-header phases) ---

#[test]
fn hls_headers_out_of_order_is_flagged() {
    // § 7.3.6: LCR -> OPS -> atlas -> sequence header. An OPS then an LCR (LCR after a
    // later HLS phase) violates the in-unit order.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::OperatingPointSet, 0, 0, 0),
        CeluRole::OperatingPointSet,
        &mut r,
    );
    t.observe(
        &obu(ObuType::LayerConfigurationRecord, 0, 0, 1),
        CeluRole::LayerConfigurationRecord,
        &mut r,
    );
    assert!(has(&r, "celu/in-unit-order"), "report: {r}");
}

#[test]
fn hls_headers_in_order_is_silent() {
    // The conforming order LCR -> OPS -> atlas -> sequence header is silent, including
    // repeats of an already-passed phase (zero-or-more of each).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    for (i, role) in [
        CeluRole::LayerConfigurationRecord,
        CeluRole::LayerConfigurationRecord,
        CeluRole::OperatingPointSet,
        CeluRole::AtlasSegment,
        CeluRole::SequenceHeader,
    ]
    .into_iter()
    .enumerate()
    {
        let ty = match role {
            CeluRole::LayerConfigurationRecord => ObuType::LayerConfigurationRecord,
            CeluRole::OperatingPointSet => ObuType::OperatingPointSet,
            CeluRole::AtlasSegment => ObuType::AtlasSegment,
            CeluRole::SequenceHeader => ObuType::SequenceHeader,
            _ => unreachable!(),
        };
        t.observe(&obu(ty, 0, 0, i as u64), role, &mut r);
    }
    assert!(!has(&r, "celu/in-unit-order"), "report: {r}");
}

#[test]
fn hls_header_after_frame_region_is_not_celu_in_unit_order() {
    // Disjointness: an HLS header after the frame region began is the existing
    // obu-order/non-global-hls-before-coded-layer rule's territory, NOT celu/in-unit-order.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::LayerConfigurationRecord, 0, 0, 1),
        CeluRole::LayerConfigurationRecord,
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "the celu tracker must defer the after-frame-region case to obu-order/*; report: {r}"
    );
}

#[test]
fn descending_mlayer_frame_units_is_flagged() {
    // § 7.3.6: frame units in ascending obu_mlayer_id. A frame unit at mlayer 0 after one
    // at mlayer 1 violates the order.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    assert!(has(&r, "celu/in-unit-order"), "report: {r}");
}

#[test]
fn ascending_mlayer_frame_units_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        output_frame(0),
        &mut r,
    );
    assert!(!has(&r, "celu/in-unit-order"), "report: {r}");
}

#[test]
fn padding_is_transparent_to_in_unit_order() {
    // PADDING is position-free: an OPS, a PADDING, then an LCR still flags the LCR (the
    // padding does not advance or reset the phase).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::OperatingPointSet, 0, 0, 0),
        CeluRole::OperatingPointSet,
        &mut r,
    );
    t.observe(&obu(ObuType::Padding, 0, 0, 1), CeluRole::Padding, &mut r);
    t.observe(
        &obu(ObuType::LayerConfigurationRecord, 0, 0, 2),
        CeluRole::LayerConfigurationRecord,
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "padding must be transparent; report: {r}"
    );
}

// --- output-unit presence / non-output-implies-output ---

#[test]
fn celu_with_no_output_unit_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // A single coded non-output frame unit, no output unit.
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/missing-output-frame-unit"), "report: {r}");
}

#[test]
fn celu_with_output_unit_is_silent_on_presence() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/missing-output-frame-unit"), "report: {r}");
}

#[test]
fn missing_output_unit_drops_when_a_unit_output_class_is_unknown() {
    // Unknown invariant: a unit with an undecidable output class means the CELU might
    // contain an unclassified output unit; the presence rule is dropped.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/missing-output-frame-unit"),
        "an Unknown output class must drop the presence rule; report: {r}"
    );
}

#[test]
fn non_output_without_output_in_layer_is_flagged() {
    // One embedded layer with a non-output unit but no output unit; another layer with an
    // output unit (so the whole-CELU presence rule is satisfied, isolating the per-layer one).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/non-output-without-output"), "report: {r}");
}

#[test]
fn non_output_with_output_in_same_layer_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // mlayer 0: a non-output then an output unit (ascending order is fine; both open).
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/non-output-without-output"), "report: {r}");
}

#[test]
fn non_output_without_output_drops_when_layer_output_unknown() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // Whole-CELU presence satisfied by a decided output unit in layer 1.
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 0),
        output_frame(0),
        &mut r,
    );
    // Layer 0: a non-output unit and an Unknown-output unit -> drop the per-layer rule.
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 2),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/non-output-without-output"),
        "an Unknown output class in the layer must drop the per-layer rule; report: {r}"
    );
}

// --- within-layer output-slot presence grammar (Finding 1, mirror lines 528-529) ---

#[test]
fn second_output_unit_in_layer_fires_output_slot_grammar() {
    // A layer's single coded output frame unit must be LAST (zero or more non-output then
    // zero or one output). A SECOND decided-output unit in the same embedded layer opens
    // after the first consumed the output slot, so the output-slot grammar fires.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "a second output unit after the output slot must fire the grammar; report: {r}"
    );
}

#[test]
fn non_output_after_output_in_layer_fires_output_slot_grammar() {
    // A decided NON-OUTPUT unit opening after the layer's coded output frame unit also
    // violates the grammar (the output unit was not last), regardless of the later unit's
    // own output class.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "a non-output unit after the output slot must fire the grammar; report: {r}"
    );
}

#[test]
fn unknown_class_after_output_in_layer_fires_output_slot_grammar() {
    // Even an Unknown-output-class later unit fires: its mere EXISTENCE (a decided
    // OpensNewUnit boundary) after the output slot is the violation, independent of its
    // own (undecidable) output class.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "an Unknown-class unit after the output slot must still fire the grammar; report: {r}"
    );
}

#[test]
fn unknown_class_earlier_unit_does_not_consume_output_slot() {
    // An Unknown-output-class earlier unit does NOT consume the output slot (the validator
    // cannot confirm it is the layer's coded output frame unit), so a later unit in the
    // same layer does not fire the output-slot grammar (drop, never guess).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "an Unknown earlier unit must not consume the output slot; report: {r}"
    );
}

#[test]
fn non_outputs_then_output_in_layer_is_silent_on_output_slot_grammar() {
    // The conformant shape: zero or more non-output units THEN the output unit. The output
    // unit is last, so the grammar stays silent.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 2),
        output_frame(0),
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "non-output units then a final output unit is conformant; report: {r}"
    );
}

#[test]
fn decided_unit_after_output_with_intervening_ambiguous_fires_output_slot_grammar() {
    // Round-8 F1: the output slot was consumed by a DECIDED output unit; a later DECIDED
    // (OpensNewUnit) unit then opens in the same layer — a decidedly-separate unit after the
    // decided output — so the output-slot grammar fires REGARDLESS of an intervening Ambiguous
    // OBU. The ambiguity changes the unit COUNT/INDEX, not the relative order of these two
    // DECIDED units: OpensNewUnit is the segmenter's decided split, so no resolution of the
    // ambiguity can merge them into one unit. The judgment ("a decided output precedes a later
    // decided OpensNewUnit unit") needs no exact index, so the per-layer poison must NOT drop
    // it. Pre-fix: silent (the `!ambiguous_poisoned` gate suppressed it), a false NEGATIVE.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // A decided OUTPUT unit consumes the layer's output slot.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // An intervening Ambiguous OBU poisons the layer's unit-count-dependent judgments.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 1),
        ambiguous_role(ObuType::RegularTip, None, None, Leadingness::Regular),
        &mut r,
    );
    // A later DECIDED unit opens after the output slot -> the output-slot grammar fires.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 2),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "a decided unit after the output slot must fire the output-slot grammar despite an \
             intervening ambiguous OBU; report: {r}"
    );
}

#[test]
fn output_then_ambiguous_no_later_decided_unit_is_silent_on_output_slot_grammar() {
    // Round-8 F1 control: a decided OUTPUT unit consumes the slot, then an Ambiguous OBU with
    // NO later decided unit. The ambiguous OBU's existence as a separate unit is undecided
    // (the ambiguity could resolve to a continuation of the output unit), so there is no
    // provably-later decided unit and the grammar stays silent. Zero false positives.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 1),
        ambiguous_role(ObuType::RegularTip, None, None, Leadingness::Regular),
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "an ambiguous OBU after the output slot (no later decided unit) must stay silent; \
             report: {r}"
    );
}

// --- header-only CELU presence (Finding 3, mirror line 536) ---

#[test]
fn header_only_celu_fires_missing_output() {
    // A CELU consisting only of HLS-header / CI / interior OBUs (≥ 1 non-padding OBU at a
    // non-global xlayer, zero frame-bearing OBUs) has no coded output frame unit, so § 7.3.6
    // line 536 fires. Here an LCR-only CELU at a non-global xlayer.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LayerConfigurationRecord, 0, 0, 0),
        CeluRole::LayerConfigurationRecord,
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/missing-output-frame-unit"),
        "a header-only CELU must fire missing-output; report: {r}"
    );
}

#[test]
fn padding_only_xlayer_group_is_silent() {
    // A padding-only (or reserved-type-only, which celu_role_for maps to Padding) xlayer
    // group is NOT a CELU and must stay silent — padding never opens a CELU.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(&obu(ObuType::Padding, 0, 0, 0), CeluRole::Padding, &mut r);
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/missing-output-frame-unit"),
        "a padding-only group must not constitute a CELU; report: {r}"
    );
}

#[test]
fn frame_bearing_with_unknown_classes_is_silent_on_missing_output() {
    // A CELU with a frame-bearing OBU whose output class is Unknown must still drop the
    // missing-output rule (the existing Unknown-invariant behavior must keep passing): the
    // header-only branch does not apply (a frame-bearing OBU exists).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/missing-output-frame-unit"),
        "a frame-bearing CELU with an Unknown output class must drop missing-output; \
             report: {r}"
    );
}

// --- same-OrderHint across output units ---

#[test]
fn output_units_with_different_order_hint_is_flagged() {
    // The same-OrderHint judgment resolves at the CELU (temporal-unit) boundary, so a
    // later undecidable output unit can still drop it (the Unknown invariant).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(3),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        output_frame(7),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/output-order-hint-mismatch"), "report: {r}");
}

#[test]
fn output_units_with_same_order_hint_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(5),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        output_frame(5),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/output-order-hint-mismatch"), "report: {r}");
}

#[test]
fn output_order_hint_mismatch_fires_despite_a_third_unit_with_unknown_hint() {
    // Round-7 F3: a mismatch PROVEN between two KNOWN output units (3 and 7) must fire even
    // when a THIRD output unit's order_hint is undecidable. An undecidable participant can
    // only prevent proving AGREEMENT (which the validator never reports); it cannot make a
    // proven pair conforming — the §7.3.6 same-OrderHint requirement is already violated by
    // the two known, differing units. The undecidable flag no longer gates emission.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(3),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 2, 2),
        output_frame(7),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/output-order-hint-mismatch"),
        "a mismatch proven between two known output units must fire despite a third \
             undecidable output unit; report: {r}"
    );
}

#[test]
fn output_order_hint_no_proven_mismatch_with_unknown_hint_is_silent() {
    // Round-7 F3 control: when there is NO proven mismatch among the KNOWN output units (one
    // known hint plus one undecidable hint), the rule stays silent. The undecidable unit
    // could equally agree or disagree, so nothing is proven and the validator reports
    // nothing (zero false positives).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(3),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/output-order-hint-mismatch"),
        "a single known hint plus an undecidable hint proves no mismatch; report: {r}"
    );
}

// --- CLK / OLK rules ---

#[test]
fn clk_and_olk_mixed_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::OpenLoopKey, 0, 1, 1),
        frame_role(
            ObuType::OpenLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(has(&r, "celu/clk-olk-mixed"), "report: {r}");
}

#[test]
fn clk_not_in_first_unit_of_layer_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // mlayer 0: first unit a regular output frame, then a CLK opening a SECOND unit.
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 1),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(has(&r, "celu/key-not-in-first-unit"), "report: {r}");
}

#[test]
fn clk_in_first_unit_of_layer_is_silent_on_first_unit_rule() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(!has(&r, "celu/key-not-in-first-unit"), "report: {r}");
}

#[test]
fn clk_lowest_layer_not_key_is_flagged() {
    // A CELU contains a CLK, but the lowest embedded layer's first unit is a regular
    // frame (CLK is at a higher layer).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 1, 1),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/lowest-layer-not-key"), "report: {r}");
}

#[test]
fn clk_lowest_layer_is_key_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // Lowest layer (mlayer 0) first unit is a CLK; higher layer is a CLK too (no OLK mix).
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 1, 1),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/lowest-layer-not-key"), "report: {r}");
}

// --- all-leading-or-none ---

#[test]
fn mixed_leading_and_non_leading_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 0, 0),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(has(&r, "celu/leading-frame-mix"), "report: {r}");
}

#[test]
fn clk_base_layer_with_regular_higher_layer_is_silent_on_leading_mix() {
    // Regression (celu/leading-frame-mix false positive): a CLK in the base embedded
    // layer (embedded layer 0) plus a Regular frame unit in a higher embedded layer is
    // the structure the § 7.3.6 CLK rule explicitly contemplates (mirror lines 541-549:
    // higher embedded layers' first units may be non-CLK regular frames). § 5.18.2
    // IsRegular classes a CLK as non-regular, but the AVM oracle tri-states a CLK as
    // INDETERMINATE (obu.c:2544-2549), neither leading nor regular. The mix rule must
    // therefore stay SILENT — the CLK is excluded from the all-leading-or-none judgment.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        !has(&r, "celu/leading-frame-mix"),
        "a CLK (indeterminate leading-ness) with a regular frame must not fire the mix \
             rule; report: {r}"
    );
}

#[test]
fn clk_with_leading_is_silent_on_leading_mix() {
    // A CLK (indeterminate) coexisting with a LEADING_* frame must also stay silent: the
    // indeterminate unit is excluded entirely, leaving an all-leading CELU.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 1, 1),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    assert!(
        !has(&r, "celu/leading-frame-mix"),
        "a CLK (indeterminate) with a leading frame must not fire the mix rule; report: {r}"
    );
}

#[test]
fn all_leading_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 0, 0),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::LeadingSef, 0, 1, 1),
        frame_role_with_boundary(
            ObuType::LeadingSef,
            FrameBoundary::OpensNewUnit,
            Some(true),
            Some(0),
            None,
            Leadingness::Leading,
        ),
        &mut r,
    );
    assert!(!has(&r, "celu/leading-frame-mix"), "report: {r}");
}

// --- CELU-scoped CI rule ---

#[test]
fn ci_outside_first_frame_unit_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // First frame unit for mlayer 0, then a CI -> the CI heads a later unit.
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ContentInterpretation, 0, 0, 1),
        CeluRole::ContentInterpretation,
        &mut r,
    );
    assert!(
        has(&r, "celu/content-interpretation-not-in-first-unit"),
        "report: {r}"
    );
}

#[test]
fn ci_in_first_frame_unit_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // CI before the layer's first coded frame -> in the first unit.
    t.observe(
        &obu(ObuType::ContentInterpretation, 0, 0, 0),
        CeluRole::ContentInterpretation,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    assert!(
        !has(&r, "celu/content-interpretation-not-in-first-unit"),
        "report: {r}"
    );
}

// --- DOH OrderHint (cross-CELU) ---

#[test]
fn doh_cross_celu_order_hint_mismatch_under_flag_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // xlayer 0 output OrderHint 1; xlayer 1 output OrderHint 2; DOH flag active. Both
    // OUTPUT units carry the SAME, KNOWN OrderHintBits, so the cross-CELU OrderHint
    // comparison (an LSB proxy) is sound: same-width LSBs that differ imply different
    // OrderHints. The per-unit bits gate the comparison (round-6 F2); the matching TU-wide
    // `note_order_hint_bits` keeps constraint (1)'s same-bits judgment fed.
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(4), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(2, 4),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/doh-order-hint-mismatch"), "report: {r}");
}

#[test]
fn doh_cross_celu_order_hint_mismatch_drops_when_bits_differ() {
    // Finding B gate: the cross-CELU §7.3.7 DOH OrderHint comparison is an LSB proxy; it
    // is UNSOUND when the two layers' OrderHintBits differ (equal decoded OrderHints can
    // carry different-width LSB encodings -> a false positive). When the bits differ, the
    // celu/doh-order-hint-bits-mismatch rule already fires; the OrderHint comparison must
    // then DROP. Two CELUs, DOH flag on, KNOWN but different OrderHintBits, LSBs differ:
    // only doh-order-hint-bits-mismatch fires, doh-order-hint-mismatch is SILENT.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(1)); // different OrderHintBits
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(2, 5), // different OrderHintBits on the output unit too
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-bits-mismatch"),
        "the differing OrderHintBits must fire the bits-mismatch rule; report: {r}"
    );
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "the cross-CELU OrderHint comparison must DROP when OrderHintBits differ (the LSB \
             proxy is unsound across different bit widths); report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_drops_when_bits_unknown() {
    // Finding B gate: when one COMPARED output unit's OrderHintBits is unknown, the
    // cross-CELU OrderHint comparison cannot be confirmed sound (the LSB widths may
    // differ), so it DROPS — even though the LSBs differ and the flag is set. The unknown
    // bits are on the OUTPUT unit being compared (round-6 F2: the gate is per compared
    // output unit, not the TU-wide same-bits judgment).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    t.note_order_hint_bits(None, ByteOffset::new(1)); // unknown OrderHintBits
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame(2), // output unit with UNKNOWN OrderHintBits (no declared bits)
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "an unknown OrderHintBits on a compared output unit must drop the cross-CELU \
             OrderHint comparison; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_without_flag_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(1),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame(2),
        &mut r,
    );
    // DOH flag NOT set -> the cross-CELU agreement is not required.
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "the DOH check must stay silent with the flag off; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_agreement_under_flag_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(4),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame(4),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/doh-order-hint-mismatch"), "report: {r}");
}

#[test]
fn doh_cross_celu_order_hint_drops_when_a_celu_hint_is_unknown() {
    // Unknown invariant (round-7 F5 control): with only ONE known CELU output OrderHint and
    // one undecidable CELU, NO mismatch is proven among the known samples, so the cross-CELU
    // rule stays silent even under the flag. (Contrast with the round-7 F5 firing case: two
    // known, differing CELUs prove a mismatch that a third undecidable CELU cannot excuse.)
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(1),
        &mut r,
    );
    // xlayer 1's only output unit has an unreadable order_hint -> undecidable CELU hint.
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "an undecidable CELU OrderHint must drop the cross-CELU rule; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_fires_despite_a_third_undecidable_celu() {
    // Round-7 F5: a mismatch PROVEN between two output CELUs with KNOWN EQUAL OrderHintBits
    // and differing OrderHints (1 and 2, bits 4) must fire even when a THIRD output CELU's
    // OrderHint is undecidable. The third CELU sets the cross-CELU `undecidable` flag, but
    // an undecidable participant can only prevent proving agreement — it cannot make the
    // already-proven pair conforming. The §7.3.7 / §7.4.6 same-OrderHint requirement is
    // violated by the two known, equal-bits, differing-hint CELUs regardless.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(4), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(2, 4), // proven mismatch vs CELU 0 (known equal bits)
        &mut r,
    );
    // xlayer 2 output unit with an unreadable order_hint -> its CELU's output OrderHint is
    // undecidable, setting the cross-CELU `undecidable` flag.
    t.note_order_hint_bits(Some(4), ByteOffset::new(2));
    t.observe(
        &obu(ObuType::RegularTileGroup, 2, 0, 2),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(true),
            None, // unreadable order_hint -> undecidable CELU output OrderHint
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-mismatch"),
        "a mismatch proven between two known equal-bits output CELUs must fire despite a \
             third undecidable output CELU; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_fires_despite_unknown_bits_nonoutput_unit() {
    // Round-6 F2: §7.3.7 has TWO distinct constraints. (1) ALL frame units in the temporal
    // unit share one OrderHintBits — rightly DROPS when any frame unit's bits are unknown.
    // (2) coded OUTPUT frame units in multiple CELUs share one OrderHint — the LSB-proxy
    // soundness for (2) needs only the COMPARED output units' bits to be known and equal.
    //
    // Two output CELUs (xlayer 0 OrderHint 1, xlayer 1 OrderHint 2) carry KNOWN EQUAL
    // OrderHintBits (4) and differing OrderHints — a genuine constraint (2) violation. A
    // THIRD, non-output frame unit (xlayer 2) carries UNKNOWN OrderHintBits. Pre-round-6 the
    // unknown-bits non-output unit set a TU-wide undecidable-bits flag that gated the
    // cross-CELU comparison off and SILENTLY suppressed the decidable output-unit mismatch.
    // The comparison is now gated per pair on the two compared output units' own
    // (known, equal) bits, so `celu/doh-order-hint-mismatch` FIRES, while
    // `celu/doh-order-hint-bits-mismatch` correctly DROPS (an unknown participant in
    // constraint (1)).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // xlayer 0 output unit: OrderHint 1, OrderHintBits 4.
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    // xlayer 1 output unit: OrderHint 2, OrderHintBits 4 (equal known bits -> sound proxy).
    t.note_order_hint_bits(Some(4), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 2),
        output_frame_bits(2, 4),
        &mut r,
    );
    // xlayer 2 NON-output frame unit with UNKNOWN OrderHintBits — drops constraint (1) but
    // must not suppress the constraint (2) mismatch above (it carries an output OrderHint
    // for neither CELU compared above; its own bits are None).
    t.note_order_hint_bits(None, ByteOffset::new(2)); // TU-wide bits become undecidable
    t.observe(
        &obu(ObuType::RegularTileGroup, 2, 0, 3),
        frame_role(
            ObuType::RegularTileGroup,
            true,
            Some(false), // non-output
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-mismatch"),
        "the two output CELUs share known equal OrderHintBits and carry differing \
             OrderHints, so the cross-CELU §7.3.7 mismatch must fire despite an unrelated \
             non-output frame unit with unknown bits; report: {r}"
    );
    assert!(
        !has(&r, "celu/doh-order-hint-bits-mismatch"),
        "constraint (1) (all frame units share one OrderHintBits) must DROP because a \
             non-output frame unit's bits are unknown; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_drops_for_unequal_bits_pair_despite_third_match() {
    // Round-6 F2 (careful case from the finding): when two OUTPUT units have KNOWN but
    // UNEQUAL bits, constraint (1) fires `celu/doh-order-hint-bits-mismatch`, and the
    // cross-CELU OrderHint comparison for THAT pair must DROP (unequal widths -> unsound
    // proxy). Two output CELUs with OrderHints 1 and 2 but OrderHintBits 4 and 5: the
    // bits-mismatch fires and the OrderHint comparison stays silent.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(1, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(2, 5),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-bits-mismatch"),
        "two output units with known unequal bits fire the bits-mismatch; report: {r}"
    );
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "the cross-CELU OrderHint comparison must DROP for a known-but-unequal-bits pair \
             (the LSB proxy is unsound across widths); report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_mismatch_fires_within_a_later_bits_group() {
    // Round-8 F3: three output CELUs with (OrderHintBits, OrderHint) = (4,0), (5,1), (5,2).
    // The cross-CELU OrderHint LSB proxy is sound only within ONE known OrderHintBits width,
    // so samples are grouped by their known bits and compared to the GROUP's representative,
    // not only to the very first sample. The two 5-bit CELUs (OrderHints 1 and 2) prove a
    // mismatch within the bits-5 group -> celu/doh-order-hint-mismatch fires. The 4-bit CELU
    // is in a different group and never gates the bits-5 comparison. Pre-fix: only the very
    // first sample (4,0) was the representative; the (5,1) and (5,2) samples both failed the
    // equal-bits gate against it (4 != 5), so the proven bits-5 mismatch was MISSED. The
    // 4 != 5 width difference also fires constraint (1) doh-order-hint-bits-mismatch.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(0, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(1, 5),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(2));
    t.observe(
        &obu(ObuType::RegularTileGroup, 2, 0, 2),
        output_frame_bits(2, 5),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-mismatch"),
        "a proven OrderHint mismatch within the bits-5 group must fire even though the first \
             sample is in the bits-4 group; report: {r}"
    );
    assert!(
        has(&r, "celu/doh-order-hint-bits-mismatch"),
        "the differing OrderHintBits (4 vs 5) must also fire the bits-mismatch; report: {r}"
    );
}

#[test]
fn doh_cross_celu_order_hint_groups_agree_within_each_bits_group_is_silent() {
    // Round-8 F3 control: three output CELUs (4,0), (5,7), (5,7). The two bits-5 CELUs AGREE
    // (OrderHint 7 in both), and the bits-4 CELU is in a separate group, so the cross-CELU
    // OrderHint comparison proves no within-group disagreement and stays SILENT. Only the
    // width difference (4 vs 5) fires constraint (1); constraint (2) must not false-positive
    // by comparing across groups.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame_bits(0, 4),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame_bits(7, 5),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(2));
    t.observe(
        &obu(ObuType::RegularTileGroup, 2, 0, 2),
        output_frame_bits(7, 5),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/doh-order-hint-mismatch"),
        "equal OrderHints within each bits group must stay silent on the cross-CELU \
             comparison; report: {r}"
    );
}

// --- DOH OrderHintBits ---

#[test]
fn doh_order_hint_bits_mismatch_under_flag_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.note_order_hint_bits(Some(5), ByteOffset::new(1));
    t.observe(
        &obu(ObuType::RegularTileGroup, 1, 0, 1),
        output_frame(0),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/doh-order-hint-bits-mismatch"), "report: {r}");
}

#[test]
fn doh_order_hint_bits_mismatch_without_flag_is_silent() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.note_order_hint_bits(Some(5), ByteOffset::new(1));
    t.reset_temporal_unit(&mut r);
    assert!(!has(&r, "celu/doh-order-hint-bits-mismatch"), "report: {r}");
}

#[test]
fn doh_order_hint_bits_mismatch_fires_despite_an_undecidable_frame() {
    // Round-7 F4: a mismatch PROVEN between two KNOWN OrderHintBits (4 and 5) must fire even
    // when a later frame unit's OrderHintBits is undecidable. §7.3.7 requires all frame
    // units in a temporal unit to share one OrderHintBits; two known differing values
    // already violate that. An undecidable participant can only prevent proving agreement,
    // never make a proven pair conforming, so it no longer gates emission. (Ordering here:
    // two known, differing bits first, then a later unresolved frame unit.)
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.note_order_hint_bits(Some(5), ByteOffset::new(1)); // proven mismatch among known bits
    t.note_order_hint_bits(None, ByteOffset::new(2)); // later unresolved frame unit
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-bits-mismatch"),
        "a mismatch proven between two known OrderHintBits must fire despite a later \
             undecidable frame unit; report: {r}"
    );
}

#[test]
fn doh_order_hint_bits_mismatch_fires_with_undecidable_between_known() {
    // Round-7 F4 (interleaved ordering): an undecidable OrderHintBits sandwiched between
    // two known, differing values must NOT suppress the proven mismatch. The mismatch is
    // recorded only between the two known samples (4 and 5); the undecidable one in the
    // middle changes nothing.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.note_order_hint_bits(None, ByteOffset::new(1)); // undecidable in the middle
    t.note_order_hint_bits(Some(5), ByteOffset::new(2));
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/doh-order-hint-bits-mismatch"),
        "an undecidable bits value between two known differing values must not suppress \
             the proven mismatch; report: {r}"
    );
}

#[test]
fn doh_order_hint_bits_no_proven_mismatch_with_undecidable_is_silent() {
    // Round-7 F4 control: a single known OrderHintBits plus an undecidable one proves no
    // mismatch, so the rule stays silent (zero false positives).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.note_order_hint_bits(Some(4), ByteOffset::new(0));
    t.note_order_hint_bits(None, ByteOffset::new(1)); // undecidable -> nothing proven
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/doh-order-hint-bits-mismatch"),
        "one known bits value plus an undecidable one proves no mismatch; report: {r}"
    );
}

// --- no-delimiter boundary (TIP/bridge): segmenter-authoritative (Finding 2) ---

#[test]
fn decided_unit_then_ambiguous_then_decided_clk_fires_key_not_in_first_unit() {
    // Round-8 F2: a DECIDED earlier unit (the first TIP, OpensNewUnit at index 0) exists in
    // the layer; a later DECIDED CLK then opens a new unit at the same mlayer. The CLK is
    // provably NOT the layer's first coded frame unit WHATEVER the intervening same-type
    // Ambiguous TIP was — the ambiguity changes the unit COUNT/INDEX, not the fact that a
    // decided unit precedes the decided CLK. So key-not-in-first-unit must FIRE: no resolution
    // of the ambiguity makes the CLK the first decided unit (the first TIP already decided to
    // open unit 0). Pre-fix: the blanket ambiguous-poison gate dropped it, a false NEGATIVE.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // First TIP opens a decided unit 0 (OpensNewUnit from the segmenter).
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // Second same-type TIP: Ambiguous -> poisons the layer's unit COUNT.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 1),
        ambiguous_role(
            ObuType::RegularTip,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // A CLK now opens a decided new unit at the same mlayer; a decided earlier unit exists,
    // so the CLK is provably not the layer's first frame unit -> key-not-in-first-unit fires.
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 2),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(false),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/key-not-in-first-unit"),
        "a decided CLK after a decided earlier unit must fire key-not-in-first-unit despite \
             an intervening ambiguous OBU; report: {r}"
    );
}

#[test]
fn ambiguous_then_decided_clk_no_decided_earlier_unit_is_silent_on_key_rule() {
    // Round-8 F2 control: an Ambiguous OBU FIRST (no decided earlier unit), then a decided
    // CLK opens the layer's FIRST decided unit. The ambiguous OBU might or might not have been
    // an earlier unit, so the validator cannot prove the CLK is not the layer's first coded
    // frame unit — the key-itself-is-the-first-decided-unit case stays dropped (Unknown
    // invariant). key-not-in-first-unit must stay SILENT.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // An ambiguous OBU before any decided unit: undecided whether it is the layer's first
    // unit. (units_opened stays 0 — only decided OpensNewUnit units are counted.)
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        ambiguous_role(
            ObuType::RegularTip,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // The CLK opens the layer's first DECIDED unit (units_opened == 0 -> is_first_unit_of_layer
    // is true), so nothing is proven about a prior unit -> silent.
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 1),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(false),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(
        !has(&r, "celu/key-not-in-first-unit"),
        "a decided CLK opening the layer's first decided unit (only an ambiguous OBU before \
             it) must stay silent; report: {r}"
    );
}

#[test]
fn tip_brt_tip_yields_two_units() {
    // Finding 2 regression: TIP, BRT, TIP. The FrameUnitSegmenter splits at the BRT (a new
    // unit head after the first TIP's coded frame), so the second TIP reports
    // FrameBoundary::OpensNewUnit (a brand-new unit), not a same-type continuation that an
    // obu_type-only comparison would have merged. The second unit's facts are therefore
    // enforced: it is a decided NON-OUTPUT unit opening after the first (decided OUTPUT)
    // unit consumed the layer's output slot, so the output-slot grammar (celu/in-unit-order)
    // fires — proving the two TIPs counted as two units.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // First TIP: a decided OUTPUT unit (consumes the output slot). The intervening BRT is a
    // FrameInterior OBU between them in the real stream; the segmenter resolves the split,
    // and here we feed the segmenter-decided OpensNewUnit boundary for the second TIP.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // Second TIP after the BRT split: OpensNewUnit, a decided NON-OUTPUT unit after the
    // output slot.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 2),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "the second TIP is a new unit after the output slot; report: {r}"
    );
}

#[test]
fn ambiguous_run_with_no_output_fires_missing_output_on_decided_unit() {
    // A same-type no-delimiter run with no output unit: the first TIP opens a decided
    // non-output unit; the second is Ambiguous (poison). The decided non-output unit gives
    // the CELU a decided unit with no output, so missing-output-frame-unit fires on it
    // (the decided unit, not the ambiguous one), and no spurious in-unit-order fires (a
    // single mlayer, no output slot consumed).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTip, 0, 1, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTip, 0, 1, 1),
        ambiguous_role(
            ObuType::RegularTip,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/missing-output-frame-unit") && !has(&r, "celu/in-unit-order"),
        "the decided non-output unit fires missing-output; the ambiguous unit is silent; \
             report: {r}"
    );
}

#[test]
fn fully_unknown_celu_is_silent() {
    // A CELU whose only frame has every derived fact undecidable fires nothing.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        frame_role_with_boundary(
            ObuType::RegularTileGroup,
            FrameBoundary::OpensNewUnit,
            None,
            None,
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(
        r.diagnostics.is_empty(),
        "a fully-Unknown CELU must be silent; report: {r}"
    );
}

// --- F1: ambiguous boundaries poison output-presence judgments (precision) ---

#[test]
fn ambiguous_undecided_class_obu_poisons_missing_output() {
    // F1: a CELU whose only DECIDED unit is a non-output unit, followed by an Ambiguous
    // same-type undecided-output-class OBU. The ambiguous OBU might open a new unit whose
    // class could be output (its class is undecided, not type-decided non-output), so the
    // CELU's missing-output presence judgment cannot be confirmed — it must be POISONED
    // (dropped), not fired. Pre-fix: missing-output fires (the decided non-output unit with
    // no output unit), a false positive.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 0),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    // Ambiguous same-type TIP, output class UNDECIDED (None) -> might open an output unit.
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 1),
        ambiguous_role(ObuType::RegularTip, None, None, Leadingness::Regular),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/missing-output-frame-unit"),
        "an ambiguous undecided-class OBU must poison missing-output; report: {r}"
    );
}

#[test]
fn ambiguous_undecided_class_obu_poisons_non_output_without_output() {
    // F1: an embedded layer with a DECIDED non-output unit plus an Ambiguous same-type
    // undecided-output-class OBU in that layer. The ambiguous OBU might be the layer's
    // output unit, so the per-layer non-output-implies-output judgment must be POISONED at
    // layer scope. A second layer carries a decided output unit so the whole-CELU presence
    // rule is satisfied (isolating the per-layer rule). Pre-fix: non-output-without-output
    // fires for layer 1, a false positive.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // Layer 0: a decided output unit (whole-CELU presence satisfied).
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    // Layer 1: a decided non-output unit, then an ambiguous undecided-class same-type OBU.
    t.observe(
        &obu(ObuType::RegularTip, 0, 1, 1),
        frame_role(
            ObuType::RegularTip,
            true,
            Some(false),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTip, 0, 1, 2),
        ambiguous_role(ObuType::RegularTip, None, None, Leadingness::Regular),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        !has(&r, "celu/non-output-without-output"),
        "an ambiguous undecided-class OBU in the layer must poison \
             non-output-without-output; report: {r}"
    );
}

#[test]
fn ambiguous_bridge_does_not_poison_missing_output() {
    // F1 precision (combined with F3): a bridge-only CELU. A BRIDGE is type-decided
    // NON-OUTPUT whichever way an ambiguous boundary resolves, so it can NEVER satisfy
    // output presence and must NOT poison the missing-output judgment. The first BRIDGE
    // opens a decided non-output unit; a second ambiguous BRIDGE is still non-output by
    // type. missing-output must still FIRE (no output unit anywhere, and the ambiguous
    // bridge cannot rescue it). Over-poisoning here would be a false NEGATIVE.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::BridgeFrame, 0, 0, 0),
        frame_role(
            ObuType::BridgeFrame,
            true,
            Some(false),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::BridgeFrame, 0, 0, 1),
        ambiguous_role(
            ObuType::BridgeFrame,
            Some(false),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/missing-output-frame-unit"),
        "an ambiguous BRIDGE (type-decided non-output) must not poison missing-output; \
             report: {r}"
    );
}

#[test]
fn ambiguous_bridge_does_not_poison_non_output_without_output() {
    // F1 precision (with F3): a layer whose only DECIDED unit is a non-output BRIDGE plus
    // a second ambiguous BRIDGE; another layer supplies a decided output unit. The
    // ambiguous BRIDGE is type-decided non-output, so it cannot be the layer's output unit
    // and must NOT poison the per-layer rule: non-output-without-output must still FIRE for
    // the bridge layer.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    // Layer 0: a decided output unit (whole-CELU presence satisfied).
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    // Layer 1: a decided non-output BRIDGE, then an ambiguous BRIDGE (still non-output).
    t.observe(
        &obu(ObuType::BridgeFrame, 0, 1, 1),
        frame_role(
            ObuType::BridgeFrame,
            true,
            Some(false),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::BridgeFrame, 0, 1, 2),
        ambiguous_role(
            ObuType::BridgeFrame,
            Some(false),
            None,
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.reset_temporal_unit(&mut r);
    assert!(
        has(&r, "celu/non-output-without-output"),
        "an ambiguous BRIDGE (type-decided non-output) must not poison \
             non-output-without-output; report: {r}"
    );
}

// --- F2: ascending-mlayer ordering counts frame-unit heads/interiors ---

#[test]
fn ci_at_higher_mlayer_then_frame_at_lower_fires_in_unit_order() {
    // F2: a CI is the head of its embedded layer's frame unit (§ 7.3.3). A CI@mlayer1
    // establishes that mlayer1's frame unit has begun; a later coded frame@mlayer0 then
    // opens a frame unit at a LOWER mlayer, violating the ascending-obu_mlayer_id ordering
    // (mirror line 525). Pre-fix: silent, because only coded-frame OBUs updated
    // max_embedded_seen, so the CI@1 head escaped the accounting.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ContentInterpretation, 0, 1, 0),
        CeluRole::ContentInterpretation,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "a frame unit at a lower mlayer after a CI head at a higher mlayer must fire \
             in-unit-order; report: {r}"
    );
}

#[test]
fn frame_interior_at_higher_mlayer_then_frame_at_lower_fires_in_unit_order() {
    // F2: a frame-interior OBU (BRT/QM/FGM/prefix-metadata/MFH) is a constituent of its
    // embedded layer's frame unit (§ 7.3.3). A QM@mlayer1 establishes that mlayer1's unit
    // has begun; a later coded frame@mlayer0 opens at a lower mlayer -> in-unit-order.
    // Pre-fix: silent.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::QuantizationMatrix, 0, 1, 0),
        CeluRole::FrameInterior,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    assert!(
        has(&r, "celu/in-unit-order"),
        "a frame unit at a lower mlayer after a frame-interior head at a higher mlayer must \
             fire in-unit-order; report: {r}"
    );
}

#[test]
fn conformant_ascending_with_ci_heads_is_silent() {
    // F2 conformant: CI@0, frame@0, CI@1, frame@1 — ascending heads and frames, all in
    // order. The CI heads participate in the accounting but never violate monotonicity.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ContentInterpretation, 0, 0, 0),
        CeluRole::ContentInterpretation,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ContentInterpretation, 0, 1, 2),
        CeluRole::ContentInterpretation,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 3),
        output_frame(0),
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "ascending CI heads and frames must be silent; report: {r}"
    );
}

#[test]
fn suffix_interior_same_mlayer_does_not_break_monotonicity() {
    // F2: a suffix metadata (FrameInterior) belongs to the just-closed unit of its OWN
    // mlayer — same mlayer as the frame it follows, so it cannot lower max_embedded_seen.
    // frame@0, suffix-interior@0, frame@1 stays silent (the suffix@0 follows the frame@0
    // at the same mlayer; the frame@1 ascends).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
    t.observe(
        &obu(ObuType::MetadataShort, 0, 0, 1),
        CeluRole::FrameInterior,
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 2),
        output_frame(0),
        &mut r,
    );
    assert!(
        !has(&r, "celu/in-unit-order"),
        "a same-mlayer suffix interior must not break ascending-mlayer monotonicity; \
             report: {r}"
    );
}

// --- F5: type-decided per-OBU facts recorded before the Ambiguous return ---

#[test]
fn olk_then_ambiguous_clk_fires_clk_olk_mixed() {
    // F5: an OLK then an Ambiguous CLK (e.g. a CLK with an unreadable tile-group
    // delimiter). CLK/OLK identity is a boundary-INDEPENDENT type-decided fact: the CELU
    // contains both an OLK and a CLK whichever way the ambiguous boundary resolves, so
    // clk-olk-mixed must FIRE. Pre-fix: the Ambiguous early-return skips recording the
    // CLK identity, so the mix is missed (false negative).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::OpenLoopKey, 0, 0, 0),
        frame_role(
            ObuType::OpenLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 1),
        ambiguous_role(
            ObuType::ClosedLoopKey,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/clk-olk-mixed"),
        "an OLK then an ambiguous CLK must fire clk-olk-mixed (type-decided identity); \
             report: {r}"
    );
}

#[test]
fn leading_then_ambiguous_regular_fires_leading_frame_mix() {
    // F5: a LEADING_* frame then an Ambiguous REGULAR-typed OBU. Leading-ness is a
    // boundary-INDEPENDENT type-decided fact (a LEADING/Regular-typed OBU evidences a
    // leading/regular frame unit whichever unit it belongs to), so the all-leading-or-none
    // mix rule must FIRE. Pre-fix: the Ambiguous early-return skips the leadingness
    // accounting, so the mix is missed.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 0, 0),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTip, 0, 0, 1),
        ambiguous_role(ObuType::RegularTip, None, None, Leadingness::Regular),
        &mut r,
    );
    assert!(
        has(&r, "celu/leading-frame-mix"),
        "a LEADING frame then an ambiguous REGULAR-typed OBU must fire leading-frame-mix \
             (type-decided leadingness); report: {r}"
    );
}

#[test]
fn ambiguous_clk_indeterminate_leadingness_stays_excluded() {
    // F5 precision: an ambiguous CLK is Leadingness::Indeterminate; it must NOT introduce
    // a leading/regular signal. A lone LEADING frame then an ambiguous CLK keeps the CELU
    // all-leading (the CLK is excluded), so leading-frame-mix stays SILENT.
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 0, 0),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 1),
        ambiguous_role(
            ObuType::ClosedLoopKey,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(
        !has(&r, "celu/leading-frame-mix"),
        "an ambiguous CLK (indeterminate) must stay excluded from leading-frame-mix; \
             report: {r}"
    );
}

// --- F2 (round-5): type-decided per-OBU facts recorded before the ContinuesUnit return ---

#[test]
fn clk_opener_then_olk_continuation_fires_clk_olk_mixed() {
    // Round-5 F2: a CLK opener then an OLK reported as ContinuesUnit (a non-first
    // tile group of an already-opened coded frame — the segmenter flags
    // frame-unit/mixed-coded-frame-types and reports ContinuesUnit). CLK/OLK identity
    // is a boundary-INDEPENDENT type-decided fact: the CELU contains both a CLK and an
    // OLK whichever way the boundary resolves, so § 7.3.6 forbids the mix regardless of
    // unit structure and clk-olk-mixed must FIRE. Pre-fix: the ContinuesUnit early-return
    // skips recording the OLK identity, so the mix is missed (false negative).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::ClosedLoopKey, 0, 0, 0),
        frame_role(
            ObuType::ClosedLoopKey,
            true,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::OpenLoopKey, 0, 0, 1),
        frame_role(
            ObuType::OpenLoopKey,
            false,
            Some(true),
            Some(0),
            Leadingness::Indeterminate,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/clk-olk-mixed"),
        "a CLK opener then an OLK ContinuesUnit continuation must fire clk-olk-mixed \
             (type-decided identity, boundary-independent); report: {r}"
    );
}

#[test]
fn leading_opener_then_regular_continuation_fires_leading_frame_mix() {
    // Round-5 F2: a LEADING_* opener then a REGULAR-typed OBU reported as ContinuesUnit.
    // Leading-ness is a boundary-INDEPENDENT type-decided fact (a LEADING/Regular-typed
    // OBU evidences a leading/regular frame unit whichever unit it belongs to), so the
    // all-leading-or-none mix rule must FIRE. Pre-fix: the ContinuesUnit early-return
    // skips the leadingness accounting, so the mix is missed (false negative).
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::LeadingTileGroup, 0, 0, 0),
        frame_role(
            ObuType::LeadingTileGroup,
            true,
            Some(true),
            Some(0),
            Leadingness::Leading,
        ),
        &mut r,
    );
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 1),
        frame_role(
            ObuType::RegularTileGroup,
            false,
            Some(true),
            Some(0),
            Leadingness::Regular,
        ),
        &mut r,
    );
    assert!(
        has(&r, "celu/leading-frame-mix"),
        "a LEADING opener then a REGULAR-typed ContinuesUnit continuation must fire \
             leading-frame-mix (type-decided leadingness, boundary-independent); report: {r}"
    );
}
