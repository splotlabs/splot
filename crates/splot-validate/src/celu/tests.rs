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
/// `order_hint_bits`; the cross-CELU §7.3.7 comparison gate reads the output units' own bits.
/// The tracker consumes the segmenter's boundary verbatim (§ 7.3.6), so the tests drive both
/// directly.
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

/// An output tile-group frame with a given OrderHint and explicit `OrderHintBits`.
/// The cross-CELU §7.3.7 OrderHint comparison is gated on the two compared output units'
/// own bits being known and equal, so a cross-CELU mismatch/agreement test declares those
/// bits here rather than only via the TU-wide `note_order_hint_bits`.
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

#[test]
fn hls_headers_out_of_order_is_flagged() {
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

#[test]
fn celu_with_no_output_unit_is_flagged() {
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
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 1, 0),
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

#[test]
fn second_output_unit_in_layer_fires_output_slot_grammar() {
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

#[test]
fn header_only_celu_fires_missing_output() {
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
fn output_units_with_different_order_hint_is_flagged() {
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

#[test]
fn ci_outside_first_frame_unit_is_flagged() {
    let mut t = fresh();
    let mut r = ValidationReport::new();
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

#[test]
fn doh_cross_celu_order_hint_mismatch_under_flag_is_flagged() {
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
        output_frame_bits(2, 4),
        &mut r,
    );
    t.set_doh_flag_active(true);
    t.reset_temporal_unit(&mut r);
    assert!(has(&r, "celu/doh-order-hint-mismatch"), "report: {r}");
}

#[test]
fn doh_cross_celu_order_hint_mismatch_drops_when_bits_differ() {
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
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(1),
        &mut r,
    );
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
        &obu(ObuType::RegularTileGroup, 1, 0, 2),
        output_frame_bits(2, 4),
        &mut r,
    );
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
fn doh_cross_celu_order_hint_mismatch_fires_within_a_later_bits_group() {
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

#[test]
fn decided_unit_then_ambiguous_then_decided_clk_fires_key_not_in_first_unit() {
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
    let mut t = fresh();
    let mut r = ValidationReport::new();
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

#[test]
fn ambiguous_undecided_class_obu_poisons_missing_output() {
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
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
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
    let mut t = fresh();
    let mut r = ValidationReport::new();
    t.observe(
        &obu(ObuType::RegularTileGroup, 0, 0, 0),
        output_frame(0),
        &mut r,
    );
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

#[test]
fn ci_at_higher_mlayer_then_frame_at_lower_fires_in_unit_order() {
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

#[test]
fn olk_then_ambiguous_clk_fires_clk_olk_mixed() {
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

#[test]
fn clk_opener_then_olk_continuation_fires_clk_olk_mixed() {
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
