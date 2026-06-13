// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn rap_replay_mfh_reference_blanket_suppressed_under_any_provided() {
    // A multi-frame header is an inexpressible kind (ExternalHlsSet cannot enumerate
    // it), so ANY Provided mode keeps the blanket suppression — an MFH MAY exist
    // externally unenumerated. Even an OPS-only set (which does not list the MFH or
    // anything related) suppresses the MFH replay.
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq(0);
    data.extend(multi_frame_header_obu(0)); // mfhId 1 -> seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // references the MFH, not resent
    // Sanity: under Disabled the MFH replay fires.
    let baseline = Validator::new(false).validate_bytes(&data);
    assert!(
        baseline
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "the MFH replay must fire under Disabled; report was: {baseline}"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("multi-frame header")),
        "an inexpressible MFH kind must be blanket-suppressed under any Provided mode; \
         report was: {report}"
    );
}

// --- Codex cycle-3: the replay qualification is ANCHOR-RELATIVE, not a global
// last-good state. The five findings below each exercise a case the single-scalar model
// got wrong; the model now replays stored (re)send EVENTS per governing random access
// point under the visible-under-start-at-R predicate (see RapReplayTracker). ---

// --- cycle-3 finding 1 (3395689081): a global referencing OBU sitting in a post-random-
// access LEADING temporal unit is itself dropped under replay, so its reference is moot
// (no diagnostic). ---

#[test]
fn rap_replay_global_reference_in_post_rap_leading_tu_is_moot() {
    // TU0: a global OPS (31, 0) + seq(3); not a random access point. TU1: a CLK random
    // access point for xlayer 0 referencing seq(3) (resent so the sequence-header replay
    // stays silent); the OPS is NOT resent. TU2: a LEADING frame (xlayer 0) plus a global
    // buffer-removal-timing OBU referencing the OPS. Under a decode that starts at the
    // TU1 random access point, § 7.3.8.1 "drops any temporal units containing leading
    // frames" — so the whole of TU2 (a strictly-later leading temporal unit) is dropped,
    // the global BRT is never decoded, and its stale OPS reference is MOOT. No diagnostic.
    //
    // (Pre-fix the moot test exempted global references explicitly, so the global BRT's
    // OPS reference — governed by the global anchor at TU1, with the OPS last sent at TU0
    // only — fired `hls/unavailable-at-random-access-point`: a false positive for a
    // temporal unit a random-access decode would drop entirely.)
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2)); // OPS (31, 0) defined in TU0 only
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // seq resent in the RAP TU -> seq replay silent
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point (xlayer 0)
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // leading frame -> TU2 leading
    data.extend(brt_dependent_obu(31, 0, 2)); // global BRT references OPS (31, 0)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global reference in a post-random-access leading temporal unit is moot \
         (§ 7.3.8.1 drops the whole temporal unit); report was: {report}"
    );
}

#[test]
fn rap_replay_global_reference_in_non_leading_tu_still_fires() {
    // Control for the previous test: the SAME stream but TU2 carries no leading frame
    // (a regular frame instead), so TU2 is decoded under start-at-TU1 and the global BRT's
    // OPS reference is NOT moot — the OPS was never resent at/after the random access
    // point, so § 7.3.8.1 fires (anchor-relative visibility, with the reference's own
    // temporal unit decodable).
    let mut data = temporal_delimiter_obu();
    data.extend(global_ops_obu(false, 0, 2));
    data.extend(seq_obu(3));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 3)); // regular -> TU2 not leading
    data.extend(brt_dependent_obu(31, 0, 2));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("operating point set") }),
        "a global OPS reference in a decoded (non-leading) post-random-access temporal \
         unit still fires; report was: {report}"
    );
}

// --- cycle-3 finding 2 (3395689092): a global resend in a MIXED temporal unit (a random
// access point for layer A, a leading frame in layer B) qualifies for anchors AT that
// temporal unit but NOT for earlier anchors. Anchor-relative visibility handles both
// directions. ---

#[test]
fn rap_replay_global_resend_in_mixed_leading_tu_not_visible_to_earlier_anchor() {
    // The earlier-anchor direction. A global LCR (extended layer GLOBAL_XLAYER_ID) is
    // resent in a temporal unit that is a random access point for xlayer 1 but carries a
    // LEADING frame in xlayer 3. A later xlayer-3 reference is governed by xlayer 3's
    // EARLIER random access point. Under that earlier start, the mixed temporal unit is a
    // strictly-later temporal unit containing a leading frame -> it drops, so the global
    // resend there is NOT visible and the reference fires.
    //
    // TU0: a global LCR 5 (xlayer_map includes xlayers 1 and 3) + seq(seq_lcr_id 5)@xlayer 3
    //   (buffers the § 7.3.8.3 reference) + a no-LCR seq(9)@xlayer 3.
    // TU1: xlayer 3 random access point (CLK@xlayer 3 ref seq(9), resent) -> R3 = TU1.
    //   The global LCR is NOT resent here.
    // TU2: a random access point for xlayer 1 (CLK@xlayer 1) AND a LEADING frame in
    //   xlayer 3; the global LCR is resent here. For xlayer 3's anchor (TU1) this temporal
    //   unit is a strictly-later leading temporal unit -> the global resend is invisible.
    // TU3: seq(seq_lcr_id 5)@xlayer 3 re-buffers the § 7.3.8.3 reference governed by R3=TU1
    //   -> the global LCR's only visible send is TU0 (< TU1) -> fires.
    //
    // (Pre-fix the global resend in TU2 qualified for ALL later anchors because TU2 is a
    // random access point for *some* layer; the earlier xlayer-3 reference was therefore
    // silenced — a missed report.)
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1010, None)); // global LCR id 5; xlayers 1 and 3 in map
    data.extend(sequence_header_obu_with_lcr(3, 5)); // seq(0, seq_lcr_id 5)@xlayer 3
    data.extend(seq_obu_layer(9, 3)); // a no-LCR seq(9)@xlayer 3 for the RAP frame ref
    // TU1: xlayer 3 random access point.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(9, 3)); // seq(9) resent -> CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 9)); // CLK@xlayer 3 -> R3 = TU1
    // TU2: random access point for xlayer 1, leading frame in xlayer 3, global LCR resent.
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b1010, None)); // global LCR resent in the mixed TU
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 9)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    // TU3: a non-leading unit re-buffers the seq_lcr_id 5 reference @xlayer 3.
    data.extend(temporal_delimiter_obu());
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("global layer configuration record")
                && d.message.contains("lcr_global_config_record_id 5")
        }),
        "a global resend in a mixed leading temporal unit must not be visible to an \
         EARLIER anchor whose start drops that temporal unit; report was: {report}"
    );
}

#[test]
fn rap_replay_global_resend_in_mixed_leading_tu_visible_to_anchor_at_that_tu() {
    // The at-that-temporal-unit direction (the other half of finding 2). The SAME mixed
    // temporal unit is a random access point for xlayer 1; an xlayer-1 reference governed
    // by THAT random access point sees the global resend as visible (clause (a):
    // S.tu == R, the random access point's own temporal unit is always decoded), so it
    // stays silent.
    //
    // TU0: global LCR 5 (xlayer_map includes xlayer 1) only (no resend later). TU1: a
    // random access point for xlayer 1 (CLK@xlayer 1) that resents the global LCR and then
    // a seq(seq_lcr_id 5)@xlayer 1 makes the § 7.3.8.3 reference governed by xlayer 1's
    // random access point at TU1, with a LEADING frame in xlayer 3 also present -> the
    // mixed temporal unit. The global LCR was resent in this same (random access point's
    // own) temporal unit, so the reference is satisfied. Silent.
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b0010, None)); // global LCR id 5; xlayer 1 in map
    data.extend(seq_obu_layer(9, 1)); // a no-LCR seq(9)@xlayer 1 for the RAP frame ref
    // TU1: the mixed random access point (xlayer 1) with a leading frame in xlayer 3.
    data.extend(temporal_delimiter_obu());
    data.extend(global_lcr_obu(5, 0b0010, None)); // global LCR resent in the RAP's own TU
    data.extend(seq_obu_layer(9, 1)); // seq(9) resent -> CLK's own seq reference satisfied
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 9)); // CLK@xlayer 1 -> RAP at TU1
    data.extend(sequence_header_obu_with_lcr(1, 5)); // seq_lcr_id 5 @xlayer 1 -> § 7.3.8.3 ref
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 3, 9)); // LEADING frame -> xlayer 3 leading
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a global resend in the random access point's own (mixed leading) temporal unit \
         is visible to that anchor (clause (a)); report was: {report}"
    );
}

// --- cycle-3 finding 3 (3395689094, § 7.4.6): a resend by a non-global layer L is
// visible to layer M's anchor only when L is decoded under start-at-M's-anchor — clause
// (c) sender-decodability. ---

#[test]
fn rap_replay_resend_by_undecodable_other_layer_does_not_satisfy_reference() {
    // Object resent ONLY by another non-global layer L that has had no random access
    // point since M's anchor -> under start-at-M's-anchor, L's coded extended layer units
    // are not decoded (§ 7.4.6), so the resend never populates state for that decode path
    // and M's reference fires.
    //
    // seq(3) is the xlayer-0 random access point's own (satisfied) reference; seq(7) is
    // the cross-layer object whose only post-anchor resend is by xlayer 1.
    //
    // TU-a: seq(3), seq(7) (xlayer 0; the seq_header_id namespace is global, § 7.3.8.6).
    // TU-b: a CLK random access point for xlayer 0 referencing seq(3), with seq(3) resent
    //   so the random access point's own reference is satisfied -> R0 = TU-b; seq(7) is
    //   NOT resent here. TU-c: seq(7) resent ONLY by xlayer 1 (a non-leading, non-random-
    //   access layer that never random-accessed). TU-d: an xlayer-0 regular reference to
    //   seq(7) governed by R0 = TU-b. The TU-c resend is by xlayer 1, which has no random
    //   access point in [TU-b, TU-c], so it is NOT decoded under a decode that starts at
    //   xlayer 0's random access point -> the reference is unsatisfied and fires.
    //
    // (Pre-fix any layer's resend promoted the object globally, so the xlayer-1 resend at
    // TU-c satisfied the xlayer-0 reference -> a missed report.)
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU-b: xlayer 0 random access point referencing seq(3) (resent); seq(7) NOT resent.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0
    // TU-c: seq(7) resent only by xlayer 1 (no random access point for xlayer 1).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    // TU-d: an xlayer-0 regular reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a resend by an other layer that is not decoded under start-at-M's-anchor must \
         not satisfy M's reference (§ 7.4.6 sender-decodability); report was: {report}"
    );
}

#[test]
fn rap_replay_resend_by_decodable_other_layer_satisfies_reference() {
    // Control for finding 3: the same shape, but layer L (xlayer 1) HAS its own random
    // access point at the resend's temporal unit, so under start-at-M's-anchor L's coded
    // extended layer units ARE decoded by then (§ 7.4.6 clause (c): L had a random access
    // point in [R, S.tu]) and the resend satisfies M's reference -> silent.
    //
    // TU-a: seq(3), seq(7). TU-b: CLK@xlayer 0 ref seq(3) (resent) -> R0; seq(7) not
    // resent. TU-c: a CLK@xlayer 1 (a random access point for xlayer 1) that resends
    // seq(7) -> visible to xlayer 0's start (xlayer 1 random-accesses at TU-c, within
    // [R0, TU-c], so it is decoded). TU-d: an xlayer-0 regular reference to seq(7) ->
    // satisfied, silent.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 7)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a resend by a layer that random-accesses within [R, S.tu] is decoded under \
         start-at-R and satisfies the reference (§ 7.4.6); report was: {report}"
    );
}

// --- cycle-3 finding 4 (3395689108): a leading-temporal-unit redefinition. § 7.4.4's
// sentence ("Regular frames that follow leading frames after the OLK temporal unit shall
// also not reference ... HLS OBUs that are indicated in temporal units containing leading
// frames") makes the LINEAR reference non-conformant in that scenario — a CONTENT-identity
// divergence, distinct from § 7.3.8.1 availability. The sound choice (documented on the
// matrix row + a § 7.4.4 residual TODO): the replay-availability treatment is correct and
// SUFFICIENT for the availability dimension — when a valid random-access-point version is
// available, the availability check must NOT fire (the object IS available at the random
// access point); naively "invalidating" availability on a leading redefinition would be a
// false positive. The § 7.4.4 content-divergence is not implemented as a new diagnostic
// (it would need per-resend content-identity modelling and risks false positives on an
// identical leading re-send), and is recorded as a residual. ---

#[test]
fn rap_replay_leading_tu_redefinition_does_not_invalidate_available_rap_version() {
    // Soundness guard. seq(7) is resent in the random access point's own temporal unit
    // (so a valid random-access-point version exists), then "redefined" (resent) only in
    // a later LEADING temporal unit. A later regular reference governed by the random
    // access point must stay SILENT: the random-access-point version is available
    // (clause (a)), and the leading-temporal-unit version — which a random-access decode
    // drops — is correctly NOT what the availability check answers about. The § 7.4.4
    // content-divergence (sequential decoding would use the leading-temporal-unit version)
    // is a separate, documented residual, not an availability false positive.
    //
    // TU0: seq(7). TU1: a CLK random access point (xlayer 0) that resents seq(7) and
    // references it (the random access point's own version). TU2: a LEADING temporal unit
    // that resends ("redefines") seq(7). TU3: a regular reference to seq(7) -> the
    // random-access-point version (TU1) is visible, so silent.
    let mut data = td_and_seq(7);
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // resent in the random access point's own temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 7)); // CLK random access point at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // "redefined" in a leading temporal unit
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7)); // -> TU2 leading
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a leading-temporal-unit redefinition must NOT invalidate an available random-\
         access-point version (the availability check answers availability, not the \
         § 7.4.4 content divergence); report was: {report}"
    );
}

#[test]
fn rap_replay_leading_tu_redefinition_with_no_rap_version_still_fires() {
    // Companion to the guard: when the random-access-point version is ABSENT and the only
    // post-anchor (re)send is in a leading temporal unit, the leading version is invisible
    // under random access (clause (b)) and the reference fires — the model does not let a
    // leading-temporal-unit (re)send satisfy a random-access reference.
    //
    // TU0: seq(7). TU1: a CLK random access point (xlayer 0) referencing seq(3) (resent so
    // its own reference is satisfied); seq(7) NOT resent here -> R0 = TU1, seq(7) last sent
    // at TU0. TU2: a LEADING temporal unit that resends seq(7) (invisible to R0). TU3: a
    // regular reference to seq(7) -> no visible send at/after R0 -> fires.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK random access point at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(7)); // resent only in a leading temporal unit
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 7)); // -> TU2 leading
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a leading-temporal-unit (re)send must not satisfy a random-access reference when \
         no random-access-point version exists; report was: {report}"
    );
}

// --- cycle-4 finding 1 (3395919323, § 7.4.6): clause (a) sender-decodability. A (re)send
// in the random access point's OWN temporal unit carried by a non-global layer that has no
// random access point in that temporal unit is NOT decoded under start-at-that-random-
// access-point (§ 7.4.6: "the decoder shall not decode coded extended layer units for an
// extended layer until a random access point for that extended layer is encountered"), so
// it must not satisfy the reference. ---

#[test]
fn rap_replay_resend_in_rap_tu_by_undecodable_other_layer_does_not_satisfy() {
    // The random access point's own temporal unit resends the object, but only by a layer
    // that does NOT random-access there. seq(3) is xlayer 0's own (satisfied) reference;
    // seq(7) is the cross-layer object resent only by xlayer 1 in the random access point's
    // temporal unit.
    //
    // TU0: seq(3), seq(7) (xlayer 0; the seq_header_id namespace is global, § 7.3.8.6).
    // TU1: a CLK random access point for xlayer 0 referencing seq(3) (resent so the random
    //   access point's own reference is satisfied) -> R0 = TU1. seq(7) is resent in TU1 but
    //   ONLY by xlayer 1, which has NO random access point in TU1 (a bare sequence header,
    //   not a CLK/OLK/RAS). Under start-at-R0 (xlayer 0's random access point), xlayer 1's
    //   coded extended layer units are not decoded, so the TU1 resend of seq(7) is invisible
    //   to this start. TU2: an xlayer-0 regular reference to seq(7) governed by R0 = TU1 ->
    //   no visible send at/after R0 -> fires.
    //
    // (Pre-fix clause (a) returned visible for any `S.tu == R` regardless of the sender, so
    // the xlayer-1 TU1 resend satisfied the xlayer-0 reference -> a missed report.)
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU1: random access point for xlayer 0.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU1
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent in TU1 ONLY by xlayer 1 (no RAP there)
    // TU2: an xlayer-0 regular reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| { d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7") }),
        "a resend in the random access point's own temporal unit by a layer that does NOT \
         random-access there is not decoded under start-at-that-random-access-point \
         (§ 7.4.6); report was: {report}"
    );
}

#[test]
fn rap_replay_resend_in_rap_tu_by_layer_that_random_accesses_there_satisfies() {
    // Control for cycle-4 finding 1: the SAME shape, but xlayer 1 ITSELF random-accesses in
    // the random access point's temporal unit (a CLK@xlayer 1), so under a start that
    // decodes xlayer 1 from there its resend of seq(7) IS visible (clause (a) with the
    // sending layer random-accessing at R) and the xlayer-0 reference stays silent.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU1
    data.extend(seq_obu_layer(7, 1)); // seq(7) resent in TU1 by xlayer 1, before its CLK
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 7)); // CLK@xlayer 1 -> RAP for xlayer 1 at TU1
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a resend in the random access point's own temporal unit by a layer that DOES \
         random-access there is decoded and satisfies the reference; report was: {report}"
    );
}

// --- cycle-4 finding 2 (3395919332, § 7.3.8.1 "any random access point"): a reference must
// be satisfied under EVERY governing random access point at or before it, not just the most
// recent. A clause-(a) resend in a random-access temporal unit that also carries leading
// frames satisfies the newest anchor (its own start) yet is invisible to an older anchor
// (under which that temporal unit drops). ---

#[test]
fn rap_replay_reference_must_satisfy_every_governing_anchor_not_just_newest() {
    // seq(3) keeps each random access point's own frame reference satisfied; seq(7) is the
    // object under test. xlayer 0 random-accesses TWICE; the second random access point's
    // temporal unit also carries a leading frame (temporal-unit indices are 1-based — the
    // first temporal unit is TU1).
    //
    // TU1: seq(3), seq(7) (xlayer 0). TU2: a CLK@xlayer 0 referencing seq(3) (resent) ->
    //   R_a = TU2; seq(7) NOT resent here. TU3: a SECOND CLK@xlayer 0 referencing seq(3)
    //   (resent) AND a leading frame in xlayer 0, with seq(7) resent here -> R_b = TU3, a
    //   random-access temporal unit that also carries leading frames. TU4: a non-leading
    //   xlayer-0 regular reference to seq(7), governed by BOTH R_a = TU2 and R_b = TU3.
    //     - Under R_b = TU3: the TU3 resend of seq(7) is clause (a) (its own start, xlayer 0
    //       random-accesses there) -> visible -> satisfied.
    //     - Under R_a = TU2: TU3 is a strictly-later leading temporal unit -> it drops, so
    //       the TU3 resend is invisible, and seq(7)'s only other send (TU1) is < TU2 ->
    //       unsatisfied -> fires, naming the EARLIER start point (temporal unit 2).
    //
    // (Pre-fix only the most-recent anchor R_b = TU3 governed the reference, which the TU3
    // clause-(a) resend satisfied -> the violation under R_a = TU2 was silenced — a missed
    // report.)
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU2: first random access point for xlayer 0 (R_a); seq(7) not resent.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the first CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_a = TU2
    // TU3: second random access point for xlayer 0 (R_b) that also carries a leading frame
    // and resends seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the second CLK's own reference is satisfied at R_b
    data.extend(seq_obu(7)); // seq(7) resent (clause (a) for R_b, invisible to R_a)
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_b = TU3
    data.extend(frame_obu_direct_seq_ref(LEADING_TILE_GROUP_HEADER, 3)); // + leading frame
    // TU4: a non-leading xlayer-0 regular reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("seq_header_id 7")
                && d.message.contains("random access point at temporal unit 2")
        }),
        "a reference satisfied by the newest anchor must still fire against an OLDER \
         governing anchor it cannot satisfy, naming that older start point (§ 7.3.8.1 \
         'any random access point'); report was: {report}"
    );
    // It must NOT also fire against the newest anchor (TU3), which the clause-(a) resend
    // satisfies.
    assert!(
        !report.errors().any(|d| {
            d.rule_id == RAP_RULE
                && d.message.contains("seq_header_id 7")
                && d.message.contains("random access point at temporal unit 3")
        }),
        "the newest anchor is satisfied and must not fire; report was: {report}"
    );
}

#[test]
fn rap_replay_non_leading_decodable_resend_covers_every_governing_anchor() {
    // Control for cycle-4 finding 2: the SAME two-anchor shape, but seq(7)'s post-anchor
    // resend is in a NON-leading temporal unit decodable from the earliest anchor, so it
    // covers EVERY governing anchor in [R_a, S.tu] and the later reference stays silent.
    //
    // TU0: seq(3), seq(7). TU1: CLK@xlayer 0 ref seq(3) (resent) -> R_a = TU1; seq(7) not
    // resent. TU2: a SECOND CLK@xlayer 0 ref seq(3) (resent) -> R_b = TU2, a NON-leading
    // temporal unit that resends seq(7). The TU2 resend is visible to R_a (TU2 > TU1,
    // non-leading, xlayer 0 decodable from TU1) AND to R_b (clause (a)). TU3: a non-leading
    // reference to seq(7) -> satisfied under both anchors -> silent.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> first CLK's own reference satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_a = TU1
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> second CLK's own reference satisfied
    data.extend(seq_obu(7)); // seq(7) resent in a NON-leading random-access temporal unit
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R_b = TU2 (non-leading)
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a non-leading decodable resend covers every governing anchor in [R_a, S.tu]; \
         report was: {report}"
    );
}

#[test]
fn rap_replay_sender_rap_in_post_anchor_leading_tu_does_not_enable_layer() {
    // Round-5 finding: a sending layer L (xlayer 1) random-accesses ONCE, in a post-anchor
    // MIXED leading temporal unit (it carries L's CLK *and* a leading frame). That random
    // access point cannot enable L under start-at-the-earlier-xlayer-0-anchor R0, because
    // the temporal unit that holds it drops wholesale under R0 (§ 7.3.8.1 leading drop) —
    // L never random-accesses on that decode path. So a LATER xlayer-1 resend of seq(7) in
    // a non-leading temporal unit is NOT decoded under start-at-R0, and the xlayer-0
    // reference must fire.
    //
    // TU1: seq(3), seq(7) (xlayer 0). TU2: CLK@xlayer 0 ref seq(3) (resent) -> R0 = TU2;
    //   seq(7) NOT resent. TU3: a MIXED leading temporal unit for xlayer 1 — CLK@xlayer 1
    //   (xlayer 1's ONLY random access point) ref seq(3) (resent) AND a leading frame@xlayer
    //   1; seq(7) NOT resent here. TU4: seq(7) resent by xlayer 1 in a NON-leading temporal
    //   unit (a regular xlayer-1 frame). TU5: an xlayer-0 regular reference to seq(7),
    //   governed by R0 = TU2.
    //     sender_decodable_at(xlayer 1, S.tu = TU4, R = R0 = TU2): xlayer 1's only random
    //     access point is TU3, which is != R0 and leading -> its temporal unit is NOT
    //     decoded under start-at-R0, so it does not enable xlayer 1 -> the TU4 resend is not
    //     decoded under R0 -> the reference is unsatisfied and fires.
    //
    // (Pre-fix sender_decodable_at only checked that xlayer 1 had *some* random access point
    // in [R0, TU4] — TU3 qualified regardless of its own leading-ness — so the TU4 resend
    // wrongly satisfied the xlayer-0 reference -> a missed report.)
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    // TU2: xlayer-0 random access point (R0); seq(7) not resent.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU2
    // TU3: xlayer 1's ONLY random access point, in a MIXED leading temporal unit.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(3, 1)); // seq(3) resent by xlayer 1 (satisfies the CLK below)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3)); // CLK@xlayer 1 -> RAP for xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(6, 0, 0, 1, 3)); // + LEADING frame@xlayer 1
    // TU4: seq(7) resent by xlayer 1 in a NON-leading temporal unit.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    // TU5: an xlayer-0 regular reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == RAP_RULE && d.message.contains("seq_header_id 7")),
        "a sender layer whose only random access point sits in a post-anchor leading \
         temporal unit is not enabled under the earlier anchor, so its later resend cannot \
         satisfy the reference (§ 7.4.6 + § 7.3.8.1 leading drop); report was: {report}"
    );
}

#[test]
fn rap_replay_sender_rap_in_post_anchor_non_leading_tu_enables_layer() {
    // Control for the round-5 finding: the SAME shape, but xlayer 1's random access point
    // temporal unit (TU3) is NON-leading. Its temporal unit IS decoded under start-at-R0
    // (non-leading, strictly after R0), so it enables xlayer 1 from R0 -> the later TU4
    // resend of seq(7) by xlayer 1 IS decoded under R0 and satisfies the xlayer-0 reference
    // -> silent.
    let mut data = td_and_seq(3);
    data.extend(seq_obu(7));
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu(3)); // resent -> the CLK's own reference is satisfied
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // CLK@xlayer 0 -> R0 = TU2
    // TU3: xlayer 1's only random access point, in a NON-leading temporal unit (no leading
    // frame this time).
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(3, 1)); // seq(3) resent by xlayer 1 (satisfies the CLK below)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 3)); // CLK@xlayer 1 -> RAP for xlayer 1
    // TU4: seq(7) resent by xlayer 1 in a non-leading temporal unit.
    data.extend(temporal_delimiter_obu());
    data.extend(seq_obu_layer(7, 1)); // resend by xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 7)); // regular xlayer-1 frame
    // TU5: an xlayer-0 regular reference to seq(7).
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == RAP_RULE),
        "a sender layer whose random access point temporal unit is non-leading is enabled \
         under the earlier anchor, so its later resend satisfies the reference (§ 7.4.6); \
         report was: {report}"
    );
}
