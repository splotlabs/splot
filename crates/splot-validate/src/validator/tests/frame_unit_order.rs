// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A full intra CLK frame whose `immediate_output_frame` (and therefore the
/// coded-frame-unit output classification, AV2 § 7.3.3) is decidable: the body
/// reaches `intra_structure_tail`, so the core parser settles the output flags
/// rather than stopping early. With `immediate_output == true` the frame is an
/// output coded frame; with `false` (and the sequence's monotonic_output_order)
/// it is a non-output coded frame. `first_tile_group` sets `is_first_tile_group`.
pub(in crate::validator::tests) fn clk_frame_decidable(
    first_tile_group: bool,
    immediate_output: bool,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(u8::from(first_tile_group)); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(u8::from(immediate_output)); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // structure + loop-filter cluster (no bits past)
    // The §5.18.2 intra tail through IntraHeaderComplete: a complete CLK header is what
    // makes its §7.23 reference-state effect (the ClkReset + allFrames refresh) grounded.
    // A truncated tail would land on StoppedInside*, which `derive_ref_update` poisons
    // (a non-conformant truncated frame's slot effect is unestablished).
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A `frame_core_seq` temporal unit (TD + activating sequence header for xlayer
/// 0) for the decidable-frame segmentation tests.
pub(in crate::validator::tests) fn seg_td_and_seq() -> Vec<u8> {
    td_and_frame_core_seq(FrameCoreSeq::base())
}

/// A bare buffer-removal-timing OBU at xlayer 0 (the payload is not parsed by
/// the segmenter; its role is fixed by the OBU type).
pub(in crate::validator::tests) fn brt_obu() -> Vec<u8> {
    annex_b_obu(BRT_HEADER, &[0x80])
}

pub(in crate::validator::tests) fn has_frame_unit_error(
    report: &ValidationReport,
    rule: &str,
) -> bool {
    report.errors().any(|d| d.rule_id == rule)
}

#[test]
fn frame_unit_first_tile_group_flag_zero_on_first_is_flagged() {
    // AV2 § 7.3.3: the first tile OBU of a coded frame must have
    // is_first_tile_group == 1. A decidable (output) CLK with the flag 0 fires.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(false, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/first-tile-group-flag"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_first_tile_group_flag_conformant_is_silent() {
    // The same CLK with is_first_tile_group == 1 (the single tile OBU of an
    // output coded frame) emits no segmentation diagnostic.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_adjacent_units_split_on_first_tile_group_flag_is_silent() {
    // AV2 § 7.3.6: a coded extended layer unit may carry back-to-back coded frame
    // units with no intervening head OBU. A tile OBU with is_first_tile_group == 1
    // arriving while a coded frame is open STARTS THE NEXT unit (closing the
    // previous), it is not an out-of-place non-first tile. Two single-OBU output
    // CLK frames (each flag 1) are two valid units and must be silent. Pre-fix the
    // state machine forced the second CLK into the first unit and reported it as a
    // non-first tile (frame-unit/first-tile-group-flag).
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1: flag 1 (ok)
    data.extend(clk_frame_decidable(true, true)); // unit 2: flag 1 (new unit, ok)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back flag-1 units must split silently; report was: {report}"
    );
}

#[test]
fn frame_unit_back_to_back_multi_tile_units_are_silent() {
    // AV2 § 7.3.6 / § 7.3.3: two back-to-back coded frame units, each a multi-OBU
    // coded frame with the is_first_tile_group 1-then-0 pattern. The flag-1 OBU
    // opening the second frame splits the first unit; the flag-0 OBUs continue
    // their own coded frame. The whole run must be silent. Pre-fix the second
    // flag-1 CLK was a non-first tile (flagged) and the trailing flag-0 CLK was a
    // first tile with flag 0 of the (never split) unit (also flagged).
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1, first tile: flag 1
    data.extend(clk_frame_decidable(false, true)); // unit 1, non-first tile: flag 0
    data.extend(clk_frame_decidable(true, true)); // unit 2, first tile: flag 1 (split)
    data.extend(clk_frame_decidable(false, true)); // unit 2, non-first tile: flag 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back 1-then-0 units must be silent; report was: {report}"
    );
}

#[test]
fn frame_unit_undecidable_first_tile_group_stays_silent() {
    // A tile-group OBU whose output class is undecidable (the activation-prefix-
    // only frame stops before the output flags) routes the unit to Unknown. This
    // RTG carries a conformant is_first_tile_group == 1 (the helper's first bit),
    // so there is no structural first-tile-group violation; the test verifies an
    // undecidable-output-class tile OBU with a conformant flag emits no frame-unit
    // diagnostic (the output-class-derived judgments stay silent, and the
    // structural first-tile-group check finds nothing to report).
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an undecidable unit must be silent; report was: {report}"
    );
}

#[test]
fn frame_unit_brt_multiplicity_in_non_output_unit_is_flagged() {
    // AV2 § 7.3.4: a coded NON-output frame unit allows zero or one BRT. A bridge
    // frame is always a non-output coded frame (mirror line 470). Two BRT OBUs
    // before it violate the bound, resolved at the unit boundary.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(brt_obu());
    data.extend(brt_obu());
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes()));
    data.extend(temporal_delimiter_obu()); // resolve the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_brt_multiplicity_in_output_unit_is_conformant() {
    // AV2 § 7.3.3: a coded OUTPUT frame unit allows zero or MORE BRT. A SEF is
    // always an output coded frame, so two BRT OBUs before it are conforming.
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(brt_obu());
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // output coded frame
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "an output unit permits multiple BRT; report was: {report}"
    );
}

/// An `OBU_BRIDGE_FRAME` whose `bridge_frame_ref_idx == 0` (in range for
/// `NumRefFrames >= 1`). A bridge frame is always a non-output coded frame (mirror
/// line 470). Carries no `is_first_tile_group` delimiter.
pub(in crate::validator::tests) fn bridge_obu() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    annex_b_obu(BRIDGE_HEADER, &fb.into_bytes())
}

#[test]
fn frame_unit_brt_multiplicity_with_back_to_back_bridge_frames_is_flagged() {
    // AV2 § 7.3.4: a coded NON-output frame unit allows zero or one BRT. A bridge
    // frame is always non-output BY TYPE (mirror line 470), independent of any
    // frame-header parse. Two BRT OBUs followed by two back-to-back same-type bridge
    // OBUs are non-conforming whether the bridges are one coded frame ("one or more
    // OBUs") or two back-to-back coded frame units: either way the (single) coded
    // frame unit holding the two BRTs is non-output. The segmenter's BridgeFrame arm is
    // type-decided non-output (it was already type-decided before the ecdf4e9 refactor),
    // so this test guards that `type_decided_output` keeps returning Some(false) for a
    // bridge — the same-type-no-delimiter continuation preserves the type-decided
    // non-output class so the § 7.3.4 BRT bound stays evaluable rather than dropping to
    // Unknown.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(brt_obu());
    data.extend(brt_obu());
    data.extend(bridge_obu()); // bridge coded frame (open)
    data.extend(bridge_obu()); // same-type continuation -> still non-output by type
    data.extend(temporal_delimiter_obu()); // resolve the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "back-to-back bridge frames stay non-output by type, so the § 7.3.4 BRT bound \
         must still fire; report was: {report}"
    );
}

#[test]
fn celu_bridge_only_celu_fires_missing_output() {
    // F3: a bridge-only CELU. § 7.3.4 type-defines OBU_BRIDGE_FRAME as a coded
    // NON-OUTPUT frame unit (it appears only in the § 7.3.4 list, mirror line 470). The
    // CELU therefore has a decided non-output unit and NO coded output frame unit, so
    // § 7.3.6 line 536 ("at least one coded output frame unit shall be present") fires
    // celu/missing-output-frame-unit. Pre-fix: the CELU facts derived output only from the
    // parsed immediate/implicit flags, and a bridge parser stops early -> output routes to
    // Unknown -> the Unknown invariant suppressed the presence check (false negative).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(bridge_obu());
    data.extend(temporal_delimiter_obu()); // resolve the CELU
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "a bridge-only CELU is type-decided non-output with no output unit, so \
         missing-output-frame-unit must fire; report was: {report}"
    );
}

#[test]
fn celu_bridge_layer_with_output_layer_fires_non_output_without_output() {
    // F3 (with § 7.3.6 line 537-538): one embedded layer carries only a type-decided
    // non-output BRIDGE frame; a higher embedded layer carries an output SEF (so the
    // whole-CELU presence rule is satisfied, isolating the per-layer rule). The bridge
    // layer has a coded non-output frame unit but no coded output frame unit in that same
    // layer, so celu/non-output-without-output fires. The BRIDGE (non-output) and SEF
    // (output) output classes are both TYPE-DECIDED, so neither needs a parseable inter
    // config. Pre-fix: the bridge routed to Unknown output, dropping the per-layer rule
    // (false negative).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1))); // max_mlayer_id == 1
    // Embedded layer 0: a type-decided non-output BRIDGE (no output unit in layer 0).
    let mut bridge = Bits::default();
    bridge.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bridge.f(0, 3); // bridge_frame_ref_idx == 0
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(19, 0, 0, 0), // OBU_BRIDGE_FRAME, mlayer 0
        &bridge.into_bytes(),
    ));
    // Embedded layer 1: an output SEF (type-decided output) -> whole-CELU presence ok.
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(12, 0, 1, 0), // OBU_REGULAR_SEF, mlayer 1
        &[0x80],
    ));
    data.extend(temporal_delimiter_obu()); // resolve the CELU
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/non-output-without-output"),
        "the bridge embedded layer has a non-output unit but no output unit, so \
         non-output-without-output must fire; report was: {report}"
    );
}

#[test]
fn frame_unit_suffix_metadata_before_coded_frame_is_flagged() {
    // AV2 § 7.3.3: suffix metadata (metadata_is_suffix == 1) belongs to the
    // tail, after the coded frame. A suffix metadata appearing before any coded
    // frame in the unit is out of order.
    let mut data = seg_td_and_seq();
    // metadata_short payload: first byte high bit = metadata_is_suffix == 1.
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &suffix_meta,
    ));
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/suffix-metadata-before-coded-frame"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_duplicate_content_interpretation_is_flagged() {
    // AV2 § 7.3.3: zero or one CI per coded frame unit.
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None));
    data.extend(content_interpretation_obu(0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/duplicate-content-interpretation"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_ci_after_mfh_is_region_order_error() {
    // AV2 § 7.3.3: CI is the first region; a CI after an MFH is out of order.
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(MFH_HEADER, &[0x80]));
    data.extend(content_interpretation_obu(0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/region-order"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_sef_single_obu_violation_is_flagged() {
    // AV2 § 7.3.3: a SEF coded frame is exactly one OBU and is the complete
    // coded-frame alternative of its unit. The genuine violation is a *non-SEF*
    // frame OBU claiming to continue the SEF coded frame: a SEF followed by a
    // *non-first* regular tile group (is_first_tile_group == 0, an explicit in-band
    // continuation claim) in the same unit. The SEF is already the unit's complete
    // coded frame, so the trailing tile OBU cannot belong to it.
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: the complete coded frame
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0: continuation claim against the SEF
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/sef-single-obu"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_tile_delimiter_after_sef_is_undecidable_and_silent() {
    // AV2 § 7.3.3 / § 7.3.5: a tile-group OBU whose is_first_tile_group bit cannot be
    // read (empty payload) arriving after a completed SEF coded frame is undecidable:
    // that bit is exactly the delimiter that decides whether this OBU continues the
    // open coded frame or begins the next coded frame unit (mirror lines 413-414 /
    // 486-487). The validator must NOT guess — the structural sef-single-obu judgment
    // (which assumes the OBU continues the SEF) is suppressed. Pre-fix the missing bit
    // was treated as a continuation claim and fired frame-unit/sef-single-obu.
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: the complete coded frame
    // A regular tile group with an EMPTY payload: the is_first_tile_group bit is
    // unreadable, so seg_role_for derives `is_first_tile_group: None`.
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable tile delimiter after a SEF is undecidable and must not fire \
         sef-single-obu; report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_tile_delimiter_after_different_type_frame_is_undecidable_and_silent() {
    // AV2 § 7.3.3 / § 7.3.5: same undecidability after a DIFFERENT-type open coded
    // frame. A decidable-output CLK coded frame followed by a regular-tile-group OBU
    // with an unreadable is_first_tile_group bit (empty payload): the missing bit is
    // the delimiter that would decide continuation (which would be a mixed-types
    // violation) versus the next coded frame unit (which would be silent). The
    // validator cannot decide, so frame-unit/mixed-coded-frame-types is suppressed.
    // Pre-fix the missing bit was treated as a continuation and fired
    // frame-unit/mixed-coded-frame-types.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CLK coded frame (open)
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &[])); // unreadable flag, different type
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable tile delimiter after a different-type coded frame is undecidable \
         and must not fire mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_sef_splits_into_new_unit_silently() {
    // AV2 § 7.3.3 / § 7.3.6: a SEF is the complete coded-frame alternative of its
    // unit ("Or: one OBU of either type OBU_LEADING_SEF or OBU_REGULAR_SEF", mirror
    // line 417), exactly one OBU, so it can never continue an already-open coded
    // frame. A SEF following a completed SEF unit STARTS A NEW coded frame unit
    // (back-to-back units, § 7.3.6), exactly like a flag-1 tile OBU — it is not a
    // sef-single-obu violation. Two back-to-back single-OBU SEF units must be
    // silent. Pre-fix the second SEF was forced into the first unit and fired
    // frame-unit/sef-single-obu (a false positive).
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(LEADING_SEF_HEADER, &[0x80])); // unit 1: SEF
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back SEF units must split silently; report was: {report}"
    );
}

#[test]
fn frame_unit_mixed_coded_frame_types_is_flagged() {
    // AV2 § 7.3.3: the OBUs of one coded frame must share an obu_type. A
    // decidable-output CLK followed by a *non-first* regular tile group (different
    // type, is_first_tile_group == 0) continues the same coded frame and fires
    // mixed-types. (A different-type tile OBU with is_first_tile_group == 1 would
    // instead start the next coded frame unit, § 7.3.6.)
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CLK, output, decidable
    // A regular tile group with is_first_tile_group == 0 (a non-first tile of a
    // header-copy tile group): continues the CLK coded frame as a mismatched type.
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/mixed-coded-frame-types"),
        "report was: {report}"
    );
}

// OBU header bytes for the no-in-band-delimiter frame types (obu_type << 2):
pub(in crate::validator::tests) const LEADING_TIP_HEADER: u8 = 13 << 2; // OBU_LEADING_TIP (0x34)
pub(in crate::validator::tests) const REGULAR_TIP_HEADER: u8 = 14 << 2; // OBU_REGULAR_TIP (0x38)

#[test]
fn frame_unit_no_delimiter_different_type_back_to_back_splits_silently() {
    // AV2 § 7.3.3 / § 7.3.4 / § 7.3.6: OBU_LEADING_TIP / OBU_REGULAR_TIP /
    // OBU_BRIDGE_FRAME carry no is_first_tile_group delimiter (mirror lines 404-411
    // / 473-484 omit them). The OBUs of one coded frame share one obu_type (mirror
    // lines 392-393 / 459-461), so a DIFFERENT no-delimiter type after a completed
    // coded frame cannot continue it — the type change is a decidable coded-frame
    // boundary that starts a new unit (§ 7.3.6 back-to-back units). A leading TIP
    // followed by a regular TIP (two different types) must split silently. Pre-fix
    // the second TIP stayed in the first coded frame and fired
    // frame-unit/mixed-coded-frame-types (a false positive).
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(LEADING_TIP_HEADER, &[0x80])); // coded frame 1 (TIP 13)
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // different type -> new unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a different no-delimiter frame type after a completed coded frame must split \
         silently, not fire mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_no_delimiter_same_type_back_to_back_is_undecidable_and_silent() {
    // AV2 § 7.3.3 / § 7.3.4: with no is_first_tile_group delimiter on the TIP / bridge
    // types, a SAME-obu_type OBU after a completed coded frame of that type is
    // undecidable — it could be a later OBU of the one coded frame ("one or more
    // OBUs", mirror lines 391-393 / 459-461) or the first OBU of a new same-type
    // coded frame. The validator must not guess: no false split, no
    // mixed-coded-frame-types, no sef-single-obu (it is not a SEF). Two regular TIP
    // OBUs back-to-back must be silent (the unit routes to Unknown).
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // coded frame, TIP 14
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // same type -> undecidable
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a same-type no-delimiter continuation is undecidable and must be silent (no \
         mixed-types false positive); report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_tip_frame_splits_silently() {
    // AV2 § 7.3.3 / § 7.3.6: a SEF is the complete coded-frame alternative of its
    // unit ("Or: one OBU of either type OBU_LEADING_SEF or OBU_REGULAR_SEF", mirror
    // line 417), exactly one OBU, so it can never continue an already-open coded
    // frame — regardless of the open frame's type. A SEF following a *completed*
    // no-delimiter (REGULAR_TIP) coded frame STARTS A NEW coded frame unit
    // (back-to-back units, § 7.3.6): the unconditional `starts_next_sef` branch in
    // `starts_new_unit` fires before the no-delimiter Unknown routing in
    // `observe_frame` can reach the SEF, so the TIP unit is sealed first and the SEF
    // opens a fresh unit. It must be silent — no sef-single-obu, no
    // mixed-coded-frame-types.
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // unit 1: TIP coded frame
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a SEF after a completed TIP coded frame must split silently (no \
         sef-single-obu / mixed-coded-frame-types); report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_bridge_frame_splits_silently() {
    // Bridge variant of `frame_unit_sef_after_tip_frame_splits_silently`. A bridge
    // frame is the other no-delimiter (Unknown-routing) coded-frame type (mirror
    // line 470). A SEF following a *completed* OBU_BRIDGE_FRAME coded frame likewise
    // starts a new coded frame unit (§ 7.3.6 back-to-back units): the unconditional
    // `starts_next_sef` branch seals the bridge unit before the bridge's same-type
    // Unknown routing can absorb the SEF. It must be silent — no sef-single-obu, no
    // mixed-coded-frame-types.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes())); // unit 1: bridge coded frame
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a SEF after a completed bridge coded frame must split silently (no \
         sef-single-obu / mixed-coded-frame-types); report was: {report}"
    );
}

#[test]
fn frame_unit_mixed_coded_frame_types_still_flags_tile_group_continuation() {
    // The no-delimiter split must NOT suppress the genuine mixed-types violation for
    // tile-group OBUs, which DO carry the is_first_tile_group delimiter. A flag-0
    // regular tile group after a completed TIP coded frame makes an explicit in-band
    // continuation claim against a mismatched open type, so mixed-coded-frame-types
    // still fires (control for the no-delimiter different-type split).
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // TIP coded frame
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0: explicit non-first-tile continuation claim
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/mixed-coded-frame-types"),
        "a flag-0 tile group continuing a mismatched TIP coded frame must still fire \
         mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_suffix_metadata_after_coded_frame_keeps_unit_intact() {
    // AV2 § 7.3.3 / § 7.3.4: a metadata OBU whose metadata_is_suffix bit cannot be
    // read (empty payload) has an undecidable region (prefix head vs suffix tail),
    // so it must NOT be treated as a unit head that closes a valid open coded frame
    // unit — doing so would orphan the closed unit's tail into a head-only unit and
    // cascade a false frame-unit/missing-coded-frame finding. Instead the OBU stays
    // in the current unit and sets region-blind (suppressing region-order checks for
    // the unit). A valid coded frame (SEF) + unreadable-suffix metadata + readable
    // suffix metadata must produce no frame-unit diagnostics. Pre-fix the unreadable
    // metadata started a new head-only unit and the flush reported missing-coded-frame.
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // valid coded frame
    // metadata_short with an EMPTY payload: the metadata_is_suffix bit is unreadable.
    data.extend(annex_b_obu_with_header(&layer_obu_header(8, 0, 0, 0), &[]));
    // A readable suffix metadata (first payload bit = 1) after it.
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &suffix_meta,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable-suffix metadata after a coded frame must keep the unit intact \
         (region-blind), not cascade a missing-coded-frame finding; report was: {report}"
    );
}

#[test]
fn frame_unit_ci_in_second_frame_unit_is_flagged() {
    // AV2 § 7.3.8.10: a CI may appear only in the first coded frame unit of its
    // embedded layer in the temporal unit. CI -> SEF (first unit) -> CI (second
    // unit's head) fires.
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None)); // first unit's CI (ok)
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // closes the first unit
    data.extend(content_interpretation_obu(0, 0, None)); // second unit's CI (violation)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/ci-not-in-first-frame-unit"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_ci_in_later_temporal_layer_unit_is_flagged() {
    // AV2 § 7.3.8.10: the "first coded frame unit of each embedded layer within a
    // temporal unit" scope is NOT keyed by obu_tlayer_id (mirror line 880). The
    // embedded layer (xlayer 0, mlayer 0) has its first coded frame unit in
    // temporal layer 0 (CI -> SEF); a later unit in temporal layer 1 that starts
    // with another CI is outside that embedded layer's first coded frame unit and
    // must fire. Pre-fix the count was stored per (xlayer, mlayer, tlayer) triple
    // and the first_coded_unit_started clause required the current unit to already
    // hold a coded frame, so the temporal-layer-1 CI was accepted.
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None)); // tlayer 0: first unit's CI
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // tlayer 0: closes first unit
    // A content-interpretation OBU at temporal layer 1, same embedded layer.
    let mut ci_bits = Bits::default();
    ci_bits.f(0, 2); // ci_scan_type_idc
    ci_bits.bit(0); // ci_color_description_present_flag
    ci_bits.bit(0); // ci_chroma_sample_position_present_flag
    ci_bits.bit(0); // ci_aspect_ratio_info_present_flag
    ci_bits.bit(0); // ci_timing_info_present_flag
    ci_bits.f(0, 2); // ci_reserved_2bit
    ci_bits.bit(0); // obu_extension_flag
    ci_bits.bit(1); // trailing_one_bit
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(24, 1, 0, 0), // CI at tlayer 1
        &ci_bits.into_bytes(),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/ci-not-in-first-frame-unit"),
        "a CI in a later temporal-layer unit of the same embedded layer must fire; \
         report was: {report}"
    );
}

#[test]
fn frame_unit_padding_is_position_free() {
    // AV2 § 7.3.3: OBU_PADDING may appear at any position within a coded frame
    // unit. A padding OBU between the pre-frame region and the coded frame must
    // not be flagged. Use a non-global (xlayer 0) padding inside the coded layer.
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(25, 0, 0, 0),
        &[0x80],
    )); // padding
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "padding is position-free; report was: {report}"
    );
}

#[test]
fn frame_unit_conformant_output_unit_is_silent() {
    // A conforming coded output frame unit: CI -> MFH -> BRT -> QM -> output CLK.
    // No segmentation diagnostic.
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None));
    data.extend(annex_b_obu(MFH_HEADER, &[0x80]));
    data.extend(brt_obu());
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_trailing_head_run_at_temporal_unit_end_is_flagged() {
    // AV2 § 7.3.3 / § 7.3.4: every coded frame unit must contain a coded frame. A
    // trailing head OBU run (here a BRT) after the last coded frame, ended by a
    // temporal delimiter with no further coded frame, is a head-only unit and must
    // fire as an error at the temporal-unit boundary. Pre-fix the open head-only
    // unit was silently dropped.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1 (complete)
    data.extend(brt_obu()); // starts a head-only unit 2
    data.extend(temporal_delimiter_obu()); // seals unit 2 with no coded frame
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a head-only unit sealed by a temporal delimiter must error; report was: {report}"
    );
}

#[test]
fn frame_unit_trailing_head_run_at_stream_end_is_a_warning() {
    // AV2 § 7.3.3: at the end of the bitstream a trailing head run may be a
    // truncated stream rather than a malformed unit, so the head-only unit is a
    // warning (not an error). Pre-fix the open unit was silently dropped.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1 (complete)
    data.extend(brt_obu()); // head-only unit 2, no coded frame, stream ends
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a head-only unit at the end of the bitstream must warn; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "the end-of-stream head-only unit must not be an error; report was: {report}"
    );
}

#[test]
fn frame_unit_complete_unit_at_stream_end_has_no_missing_coded_frame() {
    // A unit whose head OBUs are followed by a coded frame is complete; the
    // end-of-stream flush must not report a missing coded frame for it.
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(clk_frame_decidable(true, true)); // completes the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a complete unit must not report a missing coded frame; report was: {report}"
    );
}

// --- § 7.3.7 / § 7.3.6 backlog obu-order rows ---

#[test]
fn obu_order_global_hls_after_metadata_suffix_is_flagged() {
    // AV2 § 7.3.7: a global suffix metadata is part of a coded frame unit's
    // suffix tail; a global HLS prefix OBU after it is out of order. A global
    // OBU_METADATA_SHORT with metadata_is_suffix == 1, then a global LCR.
    let mut data = temporal_delimiter_obu();
    // metadata_short payload: first byte high bit = metadata_is_suffix == 1.
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &suffix_meta,
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(16, 0, 0, 31),
        &[],
    )); // global LCR
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/global-hls-after-metadata-suffix"),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_before_coded_layer_is_flagged() {
    // AV2 § 7.3.6: the coded extended layer unit is ordered LCR/OPS/atlas/seq
    // header -> frame units. A non-global LCR after the frame region of the same
    // extended layer has begun is out of order. CLK frame (xlayer 0) then a
    // non-global LCR (xlayer 0).
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // frame region for xlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    // The emitted spec_section must match the registry entry (§ 7.3.6, the coded
    // extended layer unit ordering rule), not the § 7.3.7 the shared ordering_error
    // helper defaults to. Pre-fix this diagnostic carried § 7.3.7.
    assert!(
        report.errors().any(|d| {
            d.rule_id == "obu-order/non-global-hls-before-coded-layer"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_header_before_frame_is_conformant() {
    // The conforming order — non-global LCR before the frame region — must not
    // be flagged.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // frame region after
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "obu-order/non-global-hls-before-coded-layer"),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_before_earlier_xlayer_frame_is_flagged() {
    // AV2 § 7.3.6 multi-extended-layer: after xlayer 0's frame region starts AND
    // xlayer 1's frame region starts, a non-global LCR for xlayer 0 is still out of
    // order — its coded extended layer unit's frame region has already begun. The
    // started-xlayer state must be a SET, not the last xlayer alone; pre-fix the
    // scalar held only xlayer 1, so the xlayer-0 LCR was checked against the wrong
    // layer and missed. A non-global BRT begins a layer's frame region (§ 7.3.3 /
    // § 7.3.4) without any deep frame parse, isolating the ordering check.
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, 0),
        &[0x80],
    )); // BRT xlayer 0
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, 1),
        &[0x80],
    )); // BRT xlayer 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/non-global-hls-before-coded-layer"),
        "an LCR for an earlier xlayer whose frame region began must fire even after a \
         later xlayer's frame region; report was: {report}"
    );
}
