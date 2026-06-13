// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn rap_replay_sequence_header_only_before_rap_is_flagged() {
    // TU0: seq(3) sent. TU1: CLK references seq(3) but it is not resent in the CLK's
    // temporal unit -> the random access point at TU1 cannot supply it (§ 7.3.8.1).
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.spec_section.as_deref() == Some("7.3.8.1")),
        "report was: {report}"
    );
    // Disjoint from the linear check: the sequence header IS linearly available.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_resend_in_rap_temporal_unit_passes() {
    // TU0: seq(3). TU1: seq(3) resent before the CLK, then CLK references seq(3).
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent in the random access point's temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_olk_random_access_point_is_flagged() {
    // An OLK is also a § 7.4.1 random access point.
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(OLK_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_ras_random_access_point_is_flagged() {
    // A RAS frame is also a § 7.4.1 random access point.
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(RAS_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_resend_after_reference_in_same_temporal_unit_is_flagged() {
    // § 7.3.8.1 "available ... prior to being referenced": a resend that follows the
    // referencing frame in the same random access point temporal unit does not
    // satisfy availability (matching the linear checks' intra-temporal-unit order).
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // reference precedes the resend
    data.extend(seq_obu(3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_no_random_access_point_does_not_fire() {
    // No CLK/OLK/RAS anywhere -> no random access point governs the references, so a
    // sequence header sent once and referenced by a regular tile group is fine.
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_leading_temporal_unit_resend_does_not_qualify() {
    // TU0: seq(3), seq(7). TU1: CLK ref seq(3) + seq(3) resent (the random access
    // point passes for seq 3). TU2: a LEADING tile group resends seq(7) (a resend in
    // a temporal unit that drops under random access). TU3: a regular tile group
    // references seq(7) -> seq(7) had no qualifying resend in or after the random
    // access point at TU1, so the replay fires.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU1: random access point that resends seq(3).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    // TU2: leading temporal unit that resends seq(7) (does not qualify).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7));
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7));
    // TU3: a non-leading reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
    // seq(3)'s random-access reference (the CLK) was satisfied by its resend.
    assert!(
        report
            .errors()
            .filter(|d| d.rule_id == RAP_RULE)
            .all(|d| d.message.contains("seq_header_id 7")),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_non_leading_resend_after_rap_qualifies() {
    // Same shape as the leading-temporal-unit test, but TU2's resend of seq(7) is in
    // a REGULAR (non-leading) tile group, so it qualifies and TU3's reference passes.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    // TU2: non-leading resend of seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7));
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    // TU3: reference seq(7) -> now qualifies.
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_deduplicates_per_object_per_random_access_point() {
    // Two frames in/after one random access point both reference the same dangling
    // sequence header -> one finding per (object, random access point).
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP frame
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3)); // same TU, same object
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report.errors().filter(|d| d.rule_id == RAP_RULE).count(),
        1,
        "report was: {report}"
    );
}

#[test]
fn rap_replay_distinct_random_access_points_each_report() {
    // The same dangling object referenced at two distinct random access points fires
    // once per random access point.
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP #1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // RAP #2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report.errors().filter(|d| d.rule_id == RAP_RULE).count(),
        2,
        "report was: {report}"
    );
}

#[test]
fn rap_replay_suppressed_under_external_hls_provided() {
    // Under any external-HLS Provided mode the replay is suppressed (the external
    // means escape / partial-declaration policy).
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(3)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_multi_frame_header_reference_before_rap_is_flagged() {
    // TU0: seq(0) + MFH (mfhId 1). TU1: CLK references the MFH (cur_mfh_id 1), which
    // is not resent in the random access point's temporal unit.
    let mut data = td_and_seq(0);
    data.extend(multi_frame_header_obu(0)); // mfhId 1 -> seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_operating_point_set_referenced_only_before_rap_is_flagged() {
    // TU0: a global OPS (31, 0) + seq(3). Not a random access point. TU1: seq(3) is
    // resent (so the sequence-header replay stays silent and isolates the OPS finding),
    // then a CLK random access point referencing seq 3, then a BRT referencing
    // br_ops_id 0. The OPS resolved linearly (defined in TU0) so the BRT buffers a
    // § 7.3.8.5 reference, but the OPS is not resent in the random access point's
    // temporal unit -> § 7.3.8.1 fires naming the operating-point-set family
    // (observe_buffer_removal_timing -> note_rap_reference(OperatingPointSet)). The
    // global OPS precedes the sequence header so it stays a § 7.3.7 global prefix.
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0 only
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // seq resent in the RAP TU -> seq replay silent
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point
    data.extend(brt_dependent_obu(31, 0, 2)); // references OPS (31, 0), matching ops_cnt
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("operating point set")
                && d.message.contains("ops_id 0 for obu_xlayer_id 31")
        }),
        "report was: {report}"
    );
    // Disjoint from the linear OPS check: the OPS IS linearly available in-band.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_operating_point_set_resent_in_rap_temporal_unit_passes() {
    // Control for the previous test: the same OPS is also resent in the CLK random
    // access point's temporal unit (TU1), so the § 7.3.8.5 reference is satisfied and
    // the replay stays silent (seq 3 is likewise resent to silence its own replay).
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(global_ops_obu(false, 0, 2)); // OPS resent in the random access point's TU
    data.extend(seq_obu(3)); // seq resent in the RAP TU
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point
    data.extend(brt_dependent_obu(31, 0, 2)); // references OPS (31, 0)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

// --- finding 1: per-extended-layer random-access anchors (§ 7.4.1 / § 7.4.6) ---

#[test]
fn rap_replay_rap_in_other_xlayer_does_not_govern_this_layers_reference() {
    // Codex's finding-1 counter-example. TU0: seq(3) and seq(7) (xlayer 0; the
    // seq_header_id namespace is global, § 7.3.8.6). TU1: a CLK in xlayer 0 (a random
    // access point for xlayer 0 ONLY, § 7.4.6) referencing seq(3), with seq(3) resent
    // so xlayer 0's own reference is satisfied. A REGULAR frame in xlayer 1 then
    // references seq(7), which is NOT resent. § 7.4.1 / § 7.4.6 scope random access per
    // extended layer: a decoder cannot start decoding xlayer 1 at xlayer 0's random
    // access point ("the decoder shall not decode coded extended layer units for an
    // extended layer until a random access point for that extended layer is
    // encountered"), so the xlayer-0 CLK does NOT govern the xlayer-1 seq(7) reference.
    // With no random access point for xlayer 1, the reference is silent. (Pre-fix the
    // single global anchor let the xlayer-0 CLK govern the xlayer-1 reference, so
    // seq(7) fired — a false positive.)
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> xlayer 0's CLK reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // xlayer 0 random access point
    // xlayer 1 regular frame referencing seq(7): answers to xlayer 1's own (absent)
    // random access point, not xlayer 0's. seq(7) was not resent in TU1.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "an xlayer-0 random access point must not govern an xlayer-1 reference \
         (§ 7.4.6 per-extended-layer random access); report was: {report}"
    );
}

#[test]
fn rap_replay_reference_governed_by_its_own_layers_rap_fires() {
    // The positive counterpart: when xlayer 1 ITSELF random-accesses (a CLK in
    // xlayer 1) and seq(3) is not resent in that random access point's temporal unit,
    // the xlayer-1 reference fires against xlayer 1's own random access point.
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    // xlayer 1 random access point (CLK) referencing seq(3), not resent.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 3")),
        "an xlayer-1 random access point governs the xlayer-1 reference; report was: \
         {report}"
    );
}

// --- finding 4: a random-access-point temporal unit that also carries leading frames ---

#[test]
fn rap_replay_rap_temporal_unit_with_leading_frame_promotes_own_resends() {
    // TU0: seq(3). TU1 is BOTH a random access point and carries a leading frame:
    // seq(3) is resent, then a CLK (random access point) references seq(3), then a
    // LEADING_* frame appears in the same temporal unit. Starting AT this random
    // access point does not drop its OWN temporal unit (§ 7.4.1: "Decoding can be
    // correctly initiated at such a temporal unit") — the leading-frame drop applies to
    // POST-random-access temporal units, not the random access point's own unit. So
    // the in-unit resend of seq(3) must qualify for this random access point, and a
    // later reference must stay silent. (Pre-fix the leading branch discarded ALL of
    // the temporal unit's resends, so seq(3) was not promoted and a post-random-access
    // reference fired — a false positive.)
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent in the random access point's own temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // random access point
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // + leading frame
    // TU2: a regular frame references seq(3) -> governed by the TU1 random access
    // point, which DID see a qualifying resend (the random-access-point unit is always
    // decoded), so this is silent.
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a random access point's own resends must qualify for that random access point \
         even when its temporal unit also carries a leading frame; report was: {report}"
    );
}

// --- cycle-2 finding 1: a same-temporal-unit multi-layer resend must not drop a
// qualifying sender behind a later non-qualifying one (§ 7.3.8.1) ---

#[test]
fn rap_replay_same_tu_qualifying_resend_not_overwritten_by_leading_resend() {
    // Codex cycle-2 finding 1. In one temporal unit the same seq_header_id is resent by
    // a NON-LEADING layer (qualifies) and then again by a LEADING, non-random-access
    // layer (does not qualify). § 7.3.8.1 availability is a per-object question — one
    // qualifying send suffices — so the object must be promoted for this random access
    // point. (Pre-fix the per-temporal-unit resend map kept only the LAST sender, so the
    // leading layer's non-qualifying send overwrote the qualifying one, the object was
    // not promoted, and a later reference fired — a false positive.)
    //
    // TU0: seq(3), seq(7) sent (xlayer 0). TU1 is a random access point for xlayer 0
    // (CLK referencing seq(3), seq(3) resent so its own reference is satisfied) that also
    // resends seq(7) twice: first at xlayer 0 (non-leading -> qualifies), then at
    // xlayer 1, with a LEADING frame in xlayer 1 (so xlayer 1's send does NOT qualify).
    // TU2: a regular frame in xlayer 0 references seq(7) -> governed by xlayer 0's random
    // access point at TU1, which DID see a qualifying (xlayer 0) resend of seq(7), so it
    // must stay silent.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU1: random access point for xlayer 0.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resend seq(3) -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK xlayer 0 -> random access point
    data.extend(seq_obu_layer(7, 0)); // qualifying resend of seq(7) at xlayer 0 (non-leading)
    data.extend(seq_obu_layer(7, 1)); // later resend of seq(7) at xlayer 1 (overwrites pre-fix)
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 1, 7)); // LEADING frame -> xlayer 1 leading
    // TU2: a regular xlayer-0 reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7")),
        "a qualifying same-temporal-unit resend must not be discarded because a later \
         non-qualifying resend of the same object follows it; report was: {report}"
    );
}

// --- finding 2: LCR / local-atlas random-access-point availability replay ---

#[test]
fn rap_replay_global_lcr_referenced_only_before_rap_is_flagged() {
    // TU0: a global LCR id 5 (xlayer_map includes xlayer 3) + a seq(seq_lcr_id 5) at
    // xlayer 3 (buffers the § 7.3.8.3 reference). TU1: the seq is resent at xlayer 3
    // and a CLK random-accesses xlayer 3, but the global LCR is NOT resent -> the seq's
    // seq_lcr_id reference, governed by xlayer 3's random access point at TU1, finds the
    // global LCR last sent at TU0 only -> fires (§ 7.3.8.3 "shall").
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR id 5, xlayer 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0) xlayer 3, seq_lcr_id 5
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq resent (re-buffers the LCR ref)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "report was: {report}"
    );
    // Disjoint from the linear check: the global LCR IS linearly available in-band.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_resent_in_rap_temporal_unit_passes() {
    // Control: the global LCR is also resent in the random access point's temporal
    // unit, satisfying the § 7.3.8.3 reference -> silent.
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in the RAP TU
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

// --- cycle-2 finding 2: a global-HLS resend in a temporal unit containing leading
// frames must not qualify (§ 7.4.4, mirror `07-decoding-process.md` lines 1184-1185:
// "Regular frames that follow leading frames after the OLK temporal unit shall also not
// reference ... HLS OBUs that are indicated in temporal units containing leading
// frames") unless that temporal unit is itself a random access point ---

#[test]
fn rap_replay_global_lcr_resent_only_in_post_rap_leading_tu_is_flagged() {
    // Codex cycle-2 finding 2. A global layer configuration record (extended layer
    // GLOBAL_XLAYER_ID = 31) is never tagged "leading" itself, so pre-fix a global
    // resend in any temporal unit always qualified. But § 7.4.4 forbids a post-leading
    // regular reference from relying on an HLS OBU indicated in a temporal unit that
    // contains leading frames. So a global resend in a post-random-access LEADING,
    // non-random-access temporal unit must NOT qualify.
    //
    // TU0: global LCR 5 (xlayer_map includes xlayer 3) + seq(seq_lcr_id 5)@xlayer 3
    //   (buffers the § 7.3.8.3 reference) + a no-LCR seq(9)@xlayer 3 (for the random
    //   access point's own frame reference). TU1: a CLK@xlayer 3 references seq(9)
    //   (resent so its own reference is satisfied) -> random access point for xlayer 3
    //   at TU1; the global LCR is NOT resent here, and no LCR reference is made here, so
    //   nothing fires at TU1. TU2: a POST-random-access LEADING, non-random-access
    //   temporal unit (a LEADING frame in xlayer 3) that resends the global LCR -> this
    //   resend must not qualify (§ 7.4.4). TU3: seq(seq_lcr_id 5)@xlayer 3 resent (a
    //   non-leading unit, so its reference is not moot), re-buffering the § 7.3.8.3
    //   reference governed by xlayer 3's random access point at TU1 -> the global LCR's
    //   only qualifying send is TU0 (< TU1), so it fires. (Pre-fix the TU2 global resend
    //   qualified, leaving the reference silent -> a missed report.)
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR id 5, xlayer 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0, seq_lcr_id 5)@xlayer 3
    data.extend(seq_obu_layer(9, 3)); // a no-LCR seq(9)@xlayer 3 for the RAP frame ref
    // TU1: random access point for xlayer 3 referencing the no-LCR seq(9); no global LCR
    // resend and no LCR reference here.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3)); // seq(9) resent -> the CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> RAP
    // TU2: a post-random-access LEADING, non-random-access temporal unit that resends
    // the global LCR -> must not qualify (§ 7.4.4).
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in a LEADING TU
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    // TU3: a non-leading unit re-buffers the seq_lcr_id 5 reference.
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5)); // re-buffers the § 7.3.8.3 reference
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "a global-HLS resend in a post-random-access leading temporal unit must not \
         satisfy a post-leading reference (§ 7.4.4); report was: {report}"
    );
}

#[test]
fn rap_replay_global_lcr_resent_in_post_rap_non_leading_tu_passes() {
    // Control for the previous test: the SAME stream, but TU2 carries no leading frame
    // (a regular frame instead), so the global resend qualifies and the TU3 reference
    // stays silent.
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    data.extend(seq_obu_layer(9, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> RAP
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1000, None)); // global LCR resent in a NON-leading TU
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 3, 9)); // REGULAR frame -> not leading
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global resend in a non-leading post-random-access temporal unit qualifies; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_local_atlas_referenced_only_before_rap_is_flagged() {
    // TU0: a local atlas (xlayer 3, id 2) + a local LCR (xlayer 3) referencing it via
    // lcr_local_atlas_id (buffers the § 7.3.8.4 reference). TU1: the local LCR is
    // resent and a CLK random-accesses xlayer 3, but the local atlas is NOT resent ->
    // the local LCR's lcr_local_atlas_id reference, governed by xlayer 3's random
    // access point, finds the atlas last sent at TU0 only -> fires (§ 7.3.8.4 "shall").
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu(3, 2)); // local atlas, xlayer 3, atlas_segment_id 2
    data.extend(local_lcr_obu(3, 0, 1, Some(2))); // local LCR xlayer 3, local_atlas_id 2
    data.extend(temporal_delimiter_obu());
    data.extend(local_lcr_obu(3, 0, 1, Some(2))); // local LCR resent (re-buffers the ref)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.spec_section.as_deref() == Some("7.3.8.1")
                && d.message.contains("local atlas segment")
                && d.message.contains("atlas_segment_id 2 for obu_xlayer_id 3")
        }),
        "report was: {report}"
    );
    // Disjoint from the linear check: the local atlas IS linearly available in-band.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn rap_replay_local_atlas_resent_in_rap_temporal_unit_passes() {
    // Control: the local atlas is also resent in the random access point's temporal
    // unit, satisfying the § 7.3.8.4 reference -> silent.
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu(3, 2));
    data.extend(local_lcr_obu(3, 0, 1, Some(2)));
    data.extend(temporal_delimiter_obu());
    data.extend(atlas_obu(3, 2)); // atlas resent in the RAP TU
    data.extend(local_lcr_obu(3, 0, 1, Some(2)));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0)); // CLK xlayer 3 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

// --- finding 3: per-key external-HLS suppression for declarable kinds ---

#[test]
fn rap_replay_ops_only_external_hls_does_not_suppress_undeclared_seq_header() {
    // Codex's finding-3 example: an OPS-only Provided set declares NO sequence header,
    // so a pre-random-access-point-only seq(3) referenced at a random access point
    // still fires — the caller's declaration is authoritative for the kinds it can
    // express (a declared OPS does not make an undeclared sequence header external).
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // references seq(3), not resent
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 3")),
        "an OPS-only Provided set must not suppress an undeclared sequence-header \
         replay; report was: {report}"
    );
}

#[test]
fn rap_replay_external_hls_declaring_exact_seq_header_suppresses() {
    // Control: the SAME stream with the exact seq_header_id declared external is
    // silent — the declared key is the authoritative external object.
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(3);
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(3)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "report was: {report}"
    );
}

// ----- Film-grain model references in the § 7.3.8.1 random-access-point replay -----

/// A § 5.4 sequence header OBU (seq 0) with `film_grain_params_present == 1`, so a
/// referencing output frame reads `apply_grain` and can reference a film-grain model.
fn film_grain_seq_obu() -> Vec<u8> {
    annex_b_obu(
        0x04,
        &frame_core_seq_payload(FrameCoreSeq {
            film_grain_params_present: true,
            ..FrameCoreSeq::base()
        }),
    )
}

/// A complete intra KEY frame (a § 7.4.1 random access point) that applies grain at
/// `fgm_id` through its § 5.18.2 intra tail, referencing in-band seq 0.
fn clk_intra_frame_applying_grain(fgm_id: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
    fb.bit(0); // frame_size_override_flag == 0 (max dims 16x16)
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // §5.18.2 structure + loop-filter cluster
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    fb.bit(1); // apply_grain = 1
    fb.f(u32::from(fgm_id), 3); // fgm_id f(3)
    fb.f(0, 16); // grain_seed f(16) — complete film_grain_config()
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// `true` if a film-grain-family § 7.3.8.1 replay finding is present.
fn has_film_grain_rap(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == RAP_RULE && d.message.contains("film grain model"))
}

#[test]
fn rap_replay_film_grain_only_before_rap_is_flagged() {
    // TU0: seq(0, film grain) + a film grain OBU defining slot 0. TU1: seq(0) resent (so the
    // sequence-header replay is satisfied and only the film-grain one can fire) + a CLK (a
    // §7.4.1 random access point) applying grain at fgm_id 0. Slot 0 is not resent in TU1, so
    // a decode starting at the CLK cannot supply it (§7.3.8.1) even though the monotonic
    // linear availability test sees it present.
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0)); // defines slot 0 in TU0
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu()); // seq resent -> its own replay is satisfied
    data.extend(clk_intra_frame_applying_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_film_grain_rap(&report),
        "a film-grain model sent only before the random access point must fire the replay; \
         report was: {report}"
    );
    // Disjoint from the linear check: the model IS linearly available (monotonic).
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/film-grain-model-unavailable"),
        "the model is linearly available, so the linear check must stay silent; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_film_grain_resent_in_rap_temporal_unit_passes() {
    // TU1 resends the film grain model (slot 0) before the CLK, so the random access point's
    // own temporal unit supplies it (§7.3.8.1) — no replay finding.
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0)); // resent in the random access point's TU
    data.extend(clk_intra_frame_applying_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_film_grain_rap(&report),
        "a film-grain model resent in the random access point's temporal unit must not fire \
         the replay; report was: {report}"
    );
}

#[test]
fn rap_replay_film_grain_disjoint_from_linear_unavailable() {
    // No film grain OBU ever defines slot 0: the linear frame-header/film-grain-model-unavailable
    // owns the case, and the replay (which only buffers linearly-available references) stays
    // silent — the two predicates are disjoint by construction.
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(clk_intra_frame_applying_grain(0)); // references undefined slot 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/film-grain-model-unavailable"),
        "an undefined model must fire the linear check; report was: {report}"
    );
    assert!(
        !has_film_grain_rap(&report),
        "the replay must not fire for a model the linear check already owns; report was: \
         {report}"
    );
}

#[test]
fn rap_replay_film_grain_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    // ExternalHlsSet cannot express film-grain OBUs, so any Provided mode suppresses the
    // film-grain replay (the model MAY be supplied by external means).
    let mut data = temporal_delimiter_obu();
    data.extend(film_grain_seq_obu());
    data.extend(film_grain_obu_bytes(1 << 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(film_grain_seq_obu());
    data.extend(clk_intra_frame_applying_grain(0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_film_grain_rap(&report),
        "a Provided external-HLS mode must suppress the film-grain replay; report was: {report}"
    );
}
