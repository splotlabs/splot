// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A global `OBU_METADATA_GROUP` (xlayer 31) carrying one non-cancel timecode unit
/// (type 4) with `muh_layer_idc = LAYER_VALUES`, `muh_xlayer_map` selecting the
/// single extended layer `xlayer_bit`, and a single `muh_mlayer_map` byte targeting
/// the embedded layers in `mlayer_map`. No temporal-delimiter prefix (chainable).
pub(in crate::validator::tests) fn global_timecode_group_layer_values_obu(
    xlayer_bit: u8,
    mlayer_map: u8,
    unit: &[u8],
) -> Vec<u8> {
    // muh_header_size = payload_size leb (1) + fixed 2 + muh_xlayer_map 4 + one
    // muh_mlayer_map byte (one extended layer selected) = 8. The timecode unit is a
    // handful of bytes, so its length is a single-byte leb128 muh_payload_size.
    assert!(unit.len() < 128, "timecode unit fits a 1-byte leb128");
    assert!(
        xlayer_bit < 31,
        "muh_xlayer_map bit 31 must be 0 (§ 6.16.3)"
    );
    let payload_size = unit.len() as u8;
    // muh_xlayer_map is f(32), MSB-first: the selected bit lands in the big-endian
    // 4-byte field, so bit x sets value (1 << x).
    let xlayer_map = 1u32 << xlayer_bit;
    let xlayer_map_bytes = xlayer_map.to_be_bytes();
    let mut payload = vec![
        0x00, // is_suffix=0, necessity=0, application_id=0
        0x00, // metadata_unit_cnt_minus_1 = 0
        0x04, // metadata_type = 4 (METADATA_TYPE_TIMECODE)
        0x10, // muh_header_size = 8, cancel = 0
        payload_size,
        0x60,
        0x00, // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
    ];
    payload.extend_from_slice(&xlayer_map_bytes); // muh_xlayer_map (4 bytes)
    payload.push(mlayer_map); // muh_mlayer_map for the selected extended layer
    payload.extend_from_slice(unit);
    payload.push(0x80); // OBU trailing byte
    annex_b_obu_with_header(&layer_obu_header(9, 0, 0, 31), &payload)
}

/// [`global_timecode_group_layer_values_obu`] for extended layer 0, with a
/// temporal-delimiter prefix.
pub(in crate::validator::tests) fn global_timecode_group_layer_values(
    mlayer_map: u8,
    unit: &[u8],
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(global_timecode_group_layer_values_obu(0, mlayer_map, unit));
    data
}

#[test]
fn metadata_timecode_n_frames_targeting_excludes_untargeted_layer_ci() {
    // Finding 4 (§ 6.16.3 layer targeting): a global LAYER_VALUES timecode targeting
    // embedded layer 1 only. Embedded layer 0 carries a low-rate CI timing whose
    // maxPicPerSecond the timecode's n_frames would exceed; embedded layer 1 carries a
    // CI whose timing makes the n_frames legal. The timecode must pair only with its
    // targeted layer (1), so no diagnostic — pairing with the untargeted layer 0 CI
    // (the pre-fix behavior) would wrongly fire.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // Embedded layer 0: low-rate CI (maxPicPerSecond 1); embedded layer 1: BASE_TIMING
    // (maxPicPerSecond 15). The timecode below targets layer 1 only with n_frames 2,
    // which is < 15 (legal for layer 1) but >= 1 (would violate layer 0).
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(content_interpretation_obu(1, 0, Some(BASE_TIMING)));
    let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
    // muh_mlayer_map bit 1 set -> targets embedded layer 1 only.
    data.extend(global_timecode_group_layer_values(0x02, &unit));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_VALUES timecode targeting layer 1 only must not pair with layer 0's \
         CI timing; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_targeting_pairs_with_targeted_layer_ci() {
    // Finding 4 control: the same targeting still pairs with the TARGETED layer's CI.
    // Embedded layer 1 carries a low-rate CI (maxPicPerSecond 1); the layer-1-targeted
    // timecode's n_frames 2 exceeds it, so the diagnostic fires.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(1, 0, Some(low_rate)));
    let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
    data.extend(global_timecode_group_layer_values(0x02, &unit));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_VALUES timecode targeting layer 1 must pair with layer 1's CI; \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_olk_reinit_drops_deferred_pairing() {
    // Finding 5 (§ 7.3.8.11 CI reinit): a prior-TU timecode carries n_frames 5. A
    // later temporal unit holds, in decoding order, a content interpretation OBU whose
    // low-rate timing (maxPicPerSecond 1) makes the prior-TU timecode violate the
    // n_frames bound — that pairing is *deferred* (the timecode sits in an earlier
    // temporal unit) — and then an OLK, a § 7.3.8.11 random access point that
    // reinitializes ci_timing_info_present_flag to 0. The OLK must drop the deferred
    // n_frames pairing (its pre-epoch timing no longer constrains the post-epoch
    // pictures), so no diagnostic survives. (Pre-fix the n_frames rule was not in the
    // OLK's drop set, so the deferred diagnostic would flush and fire.)
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: timecode with n_frames 5 (no CI yet, so the bound is not decided here).
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(temporal_delimiter_obu()); // -> TU1
    // TU1: the CI establishes the violating timing -> the recheck DEFERS the n_frames
    // diagnostic (TU0 observation vs TU1 CI), then the OLK reinit drops it.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> § 7.3.8.11 RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "the OLK reinitializes ci_timing_info_present_flag to 0, so the deferred \
         pairing against the prior-TU CI must drop; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_identical_ci_repairs() {
    // Round-2 finding 1 (epoch-aware identical-CI guard): a pre-RAP CI at TU0
    // establishes a low-rate timing T (maxPicPerSecond 1). A later RAP temporal unit
    // holds, in decoding order, a timecode that violates T (n_frames 5), then the SAME
    // CI re-sent with timing IDENTICAL to the pre-RAP copy, then the CLK (a § 7.3.8.11
    // random access point). The eager timecode pairing against the stale pre-RAP CI is
    // deferred. When the identical CI is re-sent the pre-RAP record is still present
    // (the CLK has not yet pruned it), so the timing-equality dedup guard skips the
    // recheck; the CLK then drops the deferred pre-RAP pairing — and with the recheck
    // skipped, nothing re-pairs the timecode against the post-epoch CI, so the
    // violation vanishes. The re-sent CI is the § 7.3.8.11 authority for this RAP
    // temporal unit's pictures and MUST re-pair the timecode regardless of the timing
    // matching the pre-epoch copy's: the epoch-aware guard re-pairs and the diagnostic
    // fires, anchored at the timecode metadata OBU.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes the low-rate timing.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: timecode (n_frames 5 >= maxPicPerSecond 1) -> the SAME CI re-sent with
    // identical timing (still before the RAP, so the pre-RAP record is the dedup
    // baseline) -> CLK (§ 7.3.8.11 RAP, drops the deferred pre-RAP pairing).
    let timecode_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.byte_offset.map(|o| o.get()) == Some(timecode_offset)
        }),
        "the post-RAP re-sent CI must re-pair the RAP-temporal-unit timecode even \
         though its timing equals the pre-RAP copy's; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_identical_ci_before_timecode_reports_once() {
    // Round-7 finding 2 (RAP re-pair must not duplicate an eagerly-emitted pairing).
    // A pre-RAP CI at TU0 establishes a low-rate timing T (maxPicPerSecond 1). The RAP
    // temporal unit holds, in decoding order, the SAME CI re-sent identical FIRST, then
    // a timecode that violates T (n_frames 5), then the CLK. Because the re-sent CI is
    // recorded BEFORE the timecode, the eager timecode-time n_frames check pairs against
    // that same-temporal-unit CI and emits the diagnostic right away (defer_or_emit
    // emits eagerly within one temporal unit). The CLK's repair hook re-pairs the
    // suppressed re-send — but this same-RAP-TU observation was already paired-and-
    // emitted, so it must be skipped: exactly ONE diagnostic. Pre-fix the repair re-
    // paired every post-epoch observation, emitting the violation TWICE. (Contrast
    // `metadata_timecode_n_frames_rap_resent_identical_ci_repairs`, where the timecode
    // precedes the re-sent CI: there the eager pairing DEFERS against the stale pre-RAP
    // CI and is dropped at the RAP, so the repair is the sole source of the one
    // diagnostic and must still fire.)
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes the low-rate timing.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: the SAME CI re-sent identical FIRST -> timecode (n_frames 5 >=
    // maxPicPerSecond 1) -> CLK (§ 7.3.8.11 RAP).
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "an identical CI re-sent BEFORE the violating timecode in the RAP temporal unit \
         is paired-and-emitted eagerly; the CLK repair hook must not re-pair it and \
         duplicate the diagnostic; report was: {report}"
    );
}

/// Counts `metadata/timecode-n-frames-exceeds-rate` errors whose message names the
/// given `obu_xlayer_id` content interpretation, so a per-layer pairing can be
/// asserted independently of the other layer's.
pub(in crate::validator::tests) fn n_frames_errors_for_xlayer(
    report: &ValidationReport,
    xlayer: u8,
) -> usize {
    report
        .errors()
        .filter(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.message
                    .contains(&format!("obu_xlayer_id {xlayer} / obu_mlayer_id 0"))
        })
        .count()
}

#[test]
fn metadata_timecode_n_frames_rap_eager_pairing_for_one_layer_does_not_suppress_other() {
    // Round-9 finding (per-CI eager-emission identity, not a per-observation bool). A
    // single global LAYER_GLOBAL timecode (one observation, Universal targeting, so it
    // pairs with every layer's CI) is constrained by two extended layers' low-rate CIs,
    // and the layers' CIs are re-sent in OPPOSITE orderings relative to the timecode in
    // one RAP temporal unit that also carries a CLK for each extended layer:
    //
    //   - extended layer 0's identical CI is re-sent BEFORE the timecode, so the eager
    //     timecode-time n_frames check pairs against it and EMITS layer 0's violation
    //     right away (defer_or_emit emits eagerly within one temporal unit);
    //   - extended layer 1's identical CI is re-sent AFTER the timecode, so its
    //     CI-time recheck is suppressed by the epoch-aware dedup guard (the pre-RAP
    //     record is still present) and DEFERRED; the layer-1 CLK then drops the stale
    //     pre-RAP pairing, leaving the CLK repair hook as the sole source of layer 1's
    //     violation.
    //
    // Pre-fix `eagerly_emitted` was a per-observation bool: layer 0's eager pairing set
    // it, and the repair hook then skipped the WHOLE observation, suppressing layer 1's
    // repair too — layer 1's violation was MISSED. With the bool replaced by a set of
    // the eagerly-emitted CI identities, the repair skips only the (observation, layer-0
    // CI) pair already emitted and still re-pairs the (observation, layer-1 CI) pair, so
    // BOTH layers report exactly once.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CIs for extended layers 0 and 1 establish the low-rate bound.
    data.extend(content_interpretation_scan_obu_at(0, 0, 0, Some(low_rate)));
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1, in decoding order: extended-layer-0 CI re-sent identical BEFORE the timecode
    // -> the global LAYER_GLOBAL timecode (n_frames 5 >= maxPicPerSecond 1, Universal)
    // -> extended-layer-1 CI re-sent identical AFTER the timecode -> a CLK for extended
    // layer 0 -> a CLK for extended layer 1 (two CLKs, same temporal unit).
    data.extend(content_interpretation_scan_obu_at(0, 0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 1), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        n_frames_errors_for_xlayer(&report, 0),
        1,
        "extended layer 0's violation (paired eagerly before the timecode) must report \
         exactly once; report was: {report}"
    );
    assert_eq!(
        n_frames_errors_for_xlayer(&report, 1),
        1,
        "extended layer 1's violation (re-paired at the CLK after a deferred-and-dropped \
         pairing) must report exactly once — a per-observation eager flag would wrongly \
         suppress it; report was: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        2,
        "exactly one violation per layer, no duplicates; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_double_clk_same_layer_same_tu_repairs_once() {
    // Round-9 nit (idempotent repair_post_rap_ci_pairings). Malformed input: a RAP
    // temporal unit carries TWO CLKs for the SAME extended layer. A pre-RAP CI at TU0
    // establishes a low-rate bound; in the RAP temporal unit the timecode (n_frames 5)
    // PRECEDES the re-sent identical CI, so the eager timecode pairing defers against
    // the stale pre-RAP CI and is dropped at the first CLK, and the repair hook is the
    // sole source of the one diagnostic (the same shape as
    // metadata_timecode_n_frames_rap_resent_identical_ci_repairs). The SECOND same-layer
    // CLK's observe_ci_rap leaves the epoch at this temporal unit and drops nothing new,
    // so without the idempotent guard repair_post_rap_ci_pairings runs a second time
    // against the same post-epoch CI snapshot and emits the violation TWICE. The
    // (extended layer, temporal unit) guard short-circuits the redundant second re-pair,
    // so the diagnostic fires exactly once.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes the low-rate bound for extended layer 0.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: timecode (n_frames 5 >= maxPicPerSecond 1) -> the SAME CI re-sent identical
    // -> TWO CLKs for extended layer 0 (malformed: two random access points for one
    // extended layer in one temporal unit).
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "two CLKs for the same extended layer in one temporal unit must not run the RAP \
         re-pair twice and duplicate the diagnostic; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_olk_rap_resent_identical_ci_repairs() {
    // The OLK analogue of `metadata_timecode_n_frames_rap_resent_identical_ci_repairs`.
    // An OLK is also a § 7.3.8.11 random access point whose observe_ci_rap advances
    // the epoch and drops the deferred pre-RAP pairings, so the epoch-aware
    // identical-CI guard must re-pair against the CI re-sent in the OLK's temporal
    // unit exactly as it does at a CLK. A pre-RAP CI at TU0 establishes a low-rate
    // timing T (maxPicPerSecond 1). The OLK temporal unit holds, in decoding order, a
    // timecode that violates T (n_frames 5), then the SAME CI re-sent with timing
    // IDENTICAL to the pre-RAP copy, then the OLK (§ 7.3.8.11 RAP). The eager timecode
    // pairing against the stale pre-RAP CI is deferred; the identical re-send is
    // skipped by the timing-equality dedup guard (pre-RAP record still present); the
    // OLK drops the deferred pre-RAP pairing. With repair wired at the OLK the re-sent
    // CI re-pairs the timecode against the post-epoch authority and the violation
    // fires, anchored at the timecode metadata OBU. (Pre-fix the OLK branch did not
    // call repair_post_rap_ci_pairings, so the violation vanished.)
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes the low-rate timing.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: timecode (n_frames 5 >= maxPicPerSecond 1) -> the SAME CI re-sent with
    // identical timing (still before the RAP, so the pre-RAP record is the dedup
    // baseline) -> OLK (§ 7.3.8.11 RAP, drops the deferred pre-RAP pairing).
    let timecode_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.byte_offset.map(|o| o.get()) == Some(timecode_offset)
        }),
        "the post-OLK re-sent CI must re-pair the RAP-temporal-unit timecode even \
         though its timing equals the pre-RAP copy's; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_different_ci_reports_once() {
    // Round-5 finding 1 (CLK re-pair filter): a pre-RAP CI at TU0 establishes a
    // low-rate timing (maxPicPerSecond 1). The RAP temporal unit holds, in decoding
    // order, a timecode that violates the bound (n_frames 5), then a re-sent CI with
    // CHANGED (different) timing (maxPicPerSecond 2 — n_frames 5 still violates),
    // then the CLK. The changed timing defeats the epoch-aware dedup guard, so the
    // eager CI-time recheck already pairs the timecode against the post-epoch CI and
    // reports the violation. The CLK's repair hook must NOT re-pair it again: only an
    // IDENTICAL re-send (whose recheck the dedup guard suppressed) is re-paired.
    // Pre-fix the repair re-paired every re-sent post-epoch CI unconditionally, so
    // the changed CI reported the violation TWICE.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let changed_rate = CiTiming {
        display_tick: 1000,
        time_scale: 3000, // maxPicPerSecond = ceil(3000 / 2000) = 2 (differs from low_rate)
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes the low-rate timing.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: timecode (n_frames 5 >= maxPicPerSecond) -> a re-sent CI with DIFFERENT
    // timing (still before the CLK) -> CLK (§ 7.3.8.11 RAP).
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(changed_rate)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "a post-RAP CI re-sent with CHANGED timing rechecks eagerly at CI-time; the \
         CLK repair hook must not re-pair it and duplicate the diagnostic; report \
         was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_identical_ci_repeat_no_rap_reports_once() {
    // Cycle-3 finding 1 (epoch-aware dedup, not temporal-unit identity). TU0
    // establishes a low-rate CI (maxPicPerSecond 1) and then a timecode whose
    // n_frames 5 violates it -> exactly one diagnostic. TU1 re-sends the IDENTICAL
    // CI with NO random access point between. The re-sent CI is content-identical
    // and the existing TU0 record is still the post-epoch authority (no RAP advanced
    // the epoch past TU0), so it must NOT replay the recheck and re-report the
    // already-reported TU0 observation. Pre-fix the temporal-unit-identity guard
    // (existing.tu_index == tu_index) treated the later-TU repeat as "changed" and
    // replayed the recheck -> a DUPLICATE second diagnostic.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: CI establishes the low-rate timing, then the violating timecode pairs
    // against it eagerly -> diagnostic #1.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(temporal_delimiter_obu()); // -> TU1, no CLK / OLK (no RAP)
    // TU1: the identical CI re-sent. No RAP, so the epoch did not advance past TU0.
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "an identical CI repeat in a later temporal unit with no random access point \
         must not re-report the already-paired observation; report was: {report}"
    );
}

#[test]
fn scan_type_identical_ci_repeat_no_rap_reports_once() {
    // Cycle-3 finding 1, the § 6.16.10 Table 6.18 analogue of
    // `metadata_timecode_n_frames_identical_ci_repeat_no_rap_reports_once`. TU0
    // establishes ci_scan_type_idc 2 and then scan-type metadata
    // mps_pic_struct_type 0 (Frame group, requires idc 1) that violates it -> one
    // diagnostic. TU1 re-sends the IDENTICAL CI with NO random access point. The
    // re-sent CI must not replay the recheck and re-report the already-reported TU0
    // observation. Pre-fix the temporal-unit-identity guard replayed it -> duplicate.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: CI establishes idc 2, then the violating scan-type metadata pairs
    // against it eagerly -> diagnostic #1.
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(global_scan_type_obu(0x00, 0)); // Frame group, requires idc 1
    data.extend(temporal_delimiter_obu()); // -> TU1, no CLK / OLK (no RAP)
    data.extend(content_interpretation_scan_obu(2, None)); // identical re-send
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "an identical CI repeat in a later temporal unit with no random access point \
         must not re-report the already-paired observation; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_global_layer_values_survives_other_layer_clk() {
    // Cycle-3 finding 2 (§ 7.3.6 per-extended-layer CVS boundaries). A global
    // LAYER_VALUES timecode targeting extended layer 1 only is observed in TU0
    // (n_frames 5). TU1 holds a CLK for extended layer 0 ONLY — which restarts
    // extended layer 0's coded video sequence, NOT extended layer 1's. The
    // layer-1-targeted observation must survive that CLK, so when a low-rate CI for
    // extended layer 1 (maxPicPerSecond 1) arrives in TU1 the preserved observation
    // re-pairs and the n_frames bound fires. Pre-fix the global-bucket timecode
    // scope was pruned at EVERY CLK (including the unrelated extended-layer-0 CLK),
    // dropping the observation, so nothing re-paired and the violation vanished.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: global LAYER_VALUES timecode targeting extended layer 1 (xlayer_bit 1),
    // embedded layer 0 (mlayer_map bit 0), n_frames 5. No CI yet, so the bound is
    // not decided here.
    let unit = timecode_unit_bits(5, Some(0), Some(0), Some(0));
    data.extend(global_timecode_group_layer_values_obu(1, 0x01, &unit));
    data.extend(temporal_delimiter_obu()); // -> TU1
    // TU1: a CLK for extended layer 0 ONLY (must not prune the layer-1 observation).
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    // TU1: a low-rate CI for extended layer 1 / embedded layer 0 establishes
    // maxPicPerSecond 1, which the preserved n_frames 5 observation exceeds.
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a CLK for extended layer 0 must not prune a global timecode observation \
         targeting extended layer 1; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_global_layer_values_pending_survives_other_layer_clk() {
    // Round-5 finding 2 (target-aware pending-inference resolution). A global
    // LAYER_VALUES timecode targeting extended layer 1 carries a PRESENT seconds_value
    // in TU0. In TU1 the same-targeted global timecode OMITS seconds_value, seeded by
    // TU0's present value in an EARLIER temporal unit — so the inference-presence
    // diagnostic is deferred pending TU1's § 7.3.6 CVS scope. A CLK for extended layer
    // 0 ONLY then closes TU1: it restarts extended layer 0's coded video sequence, NOT
    // extended layer 1's, so the deferred timecode's earlier-temporal-unit seed stays
    // intra-CVS for the layer it targets and the field infers cleanly — NO diagnostic.
    // Pre-fix emit_pending_timecode_inference fired on ANY CLK whose pending entry was
    // carried on a global OBU, so this unrelated extended-layer-0 CLK wrongly fired the
    // inference error for the layer-1-targeted timecode.
    let build = |clk_xlayer: u8| {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        // TU0: global LAYER_VALUES timecode targeting extended layer 1 (xlayer_bit 1),
        // embedded layer 0 (mlayer_map bit 0), with a PRESENT seconds_value (the seed).
        let seed = timecode_unit_bits(0, Some(0), Some(0), Some(0));
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &seed));
        data.extend(temporal_delimiter_obu()); // -> TU1
        // TU1: the same-targeted global timecode OMITS seconds_value (seeded by TU0).
        let omitting = timecode_unit_bits(0, None, None, None);
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &omitting));
        // TU1: a CLK for `clk_xlayer` closes the deferred inference's fate.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(4, 0, 0, clk_xlayer),
            &[],
        ));
        data
    };
    // Negative: a CLK for extended layer 0 does not restart extended layer 1's coded
    // video sequence, so the layer-1-targeted seed survives -> no diagnostic.
    let report = Validator::new(false).validate_bytes(&build(0));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "a CLK for extended layer 0 must not fire the pending inference of a global \
         LAYER_VALUES timecode targeting extended layer 1; report was: {report}"
    );
    // Control: a CLK for extended layer 1 (the targeted layer, in the omitting
    // timecode's own temporal unit) detaches the earlier-temporal-unit seed and fires
    // the inference-without-previous diagnostic per the same-TU-CLK rule (§ 7.3.6).
    let report = Validator::new(false).validate_bytes(&build(1));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a CLK for the TARGETED extended layer 1 must fire the deferred inference \
         diagnostic; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_chain_global_layer_values_survives_other_layer_clk() {
    // Round-7 finding 1 (target-aware inference-CHAIN pruning). A global LAYER_VALUES
    // timecode targeting extended layer 1 carries a PRESENT seconds_value in TU0 — the
    // inference chain SEED. A CLK in TU1 then prunes the inference chain
    // (prune_timecode_scope) BEFORE the omitting timecode in TU2 reads it. In TU2 a
    // same-targeted global timecode OMITS seconds_value. Whether the seed survives the
    // TU1 CLK depends on the CLK's extended layer:
    //
    //   - CLK for extended layer 0 (negative): does NOT restart extended layer 1's
    //     coded video sequence, so the layer-1-targeted seed survives the chain prune.
    //     The TU2 omission is seeded by the earlier-temporal-unit present value — the
    //     diagnostic is deferred pending TU2's CVS scope and, with no CLK closing TU2,
    //     dropped silently. NO inference diagnostic. Pre-fix the chain entry, carried on
    //     a global (obu_xlayer_id 31) OBU, was pruned at ANY CLK (the carrying-scope-
    //     only `is_global()` predicate), so the seed vanished and the TU2 omission fired
    //     `inferred-without-previous` for a missing seed.
    //   - CLK for extended layer 1 (control): restarts extended layer 1's coded video
    //     sequence, detaching the earlier-temporal-unit seed from the new sequence. The
    //     TU2 omission has no in-CVS previous present value -> the diagnostic fires
    //     eagerly (the new-CVS/no-previous behavior), confirming the chain reset is
    //     still target-aware for the layer it DOES target.
    let build = |clk_xlayer: u8| {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        // TU0: global LAYER_VALUES timecode targeting extended layer 1 (xlayer_bit 1),
        // embedded layer 0 (mlayer_map bit 0), with a PRESENT seconds_value (the seed).
        let seed = timecode_unit_bits(0, Some(0), Some(0), Some(0));
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &seed));
        data.extend(temporal_delimiter_obu()); // -> TU1
        // TU1: a CLK for `clk_xlayer` prunes the inference chain BEFORE the omitting
        // timecode reads it (the chain reset must be target-aware).
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(4, 0, 0, clk_xlayer),
            &[],
        ));
        data.extend(temporal_delimiter_obu()); // -> TU2
        // TU2: the same-targeted global timecode OMITS seconds_value (would be seeded
        // by TU0's preserved present value when the chain survived).
        let omitting = timecode_unit_bits(0, None, None, None);
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &omitting));
        data
    };
    // Negative: a CLK for extended layer 0 does not restart extended layer 1's coded
    // video sequence, so the layer-1-targeted chain seed survives -> no diagnostic.
    let report = Validator::new(false).validate_bytes(&build(0));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "a CLK for extended layer 0 must not prune a global LAYER_VALUES inference \
         chain targeting extended layer 1; report was: {report}"
    );
    // Control: a CLK for extended layer 1 restarts that layer's coded video sequence,
    // detaching the earlier-temporal-unit seed -> the TU2 omission fires.
    let report = Validator::new(false).validate_bytes(&build(1));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a CLK for the TARGETED extended layer 1 must reset the inference chain so the \
         TU2 omission has no previous present value; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_keyed_per_targeted_embedded_layer() {
    // Cycle-3 finding 3 (inference keyed per targeted (xlayer, mlayer), not just
    // obu_xlayer_id). A full-timestamp LAYER_CURRENT timecode on (xlayer 0,
    // mlayer 0) carries every field. A following LAYER_CURRENT timecode on (xlayer
    // 0, mlayer 1) that omits seconds_value must NOT be seeded by the (0, 0)
    // timecode — METADATA_TYPE_TIMECODE is layer-specific (§ 6.16.3 Table 6.17),
    // so the (0, 0) set is not the "previous set in decoding order" for (0, 1). The
    // inference-without-previous diagnostic must fire. Pre-fix the chain was keyed
    // only by obu_xlayer_id, so the (0, 0) set wrongly seeded the (0, 1) inference
    // and the diagnostic was suppressed.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // LAYER_CURRENT (muh_layer_idc 2) short-form first byte: is_suffix 0,
    // muh_layer_idc 2 (f(3)), cancel 0, persistence 0 -> 0b0_010_0_000 = 0x20.
    const LAYER_CURRENT_FIRST: u8 = 0x20;
    // (xlayer 0, mlayer 0): full-timestamp, all fields present.
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &metadata_short_payload(LAYER_CURRENT_FIRST, 4, &{
            let mut bits = Bits::default();
            bits.f(0, 5); // counting_type
            bits.bit(1); // full_timestamp_flag
            bits.bit(0); // discontinuity_flag
            bits.bit(0); // cnt_dropped_flag
            bits.f(0, 9); // n_frames
            bits.f(0, 6); // seconds_value
            bits.f(0, 6); // minutes_value
            bits.f(0, 5); // hours_value
            bits.f(0, 5); // time_offset_length = 0
            bits.align();
            bits.into_bytes()
        }),
    ));
    // (xlayer 0, mlayer 1): omits seconds_value (seconds_flag 0).
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 1, 0),
        &metadata_short_payload(
            LAYER_CURRENT_FIRST,
            4,
            &timecode_unit_bits(0, None, None, None),
        ),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a (xlayer 0, mlayer 0) timecode must not seed the inference of a (xlayer 0, \
         mlayer 1) timecode (per-targeted-layer keying); report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_unspecified_targeting_compares_nothing() {
    // Cycle-3 finding 4 (zero-false-positive for underivable targeting). A
    // LAYER_UNSPECIFIED short-form timecode (first byte 0x00) "does not specify to
    // what layers the metadata applies to" (§ 6.16.3), so no CI's rate can be
    // soundly bound to it — the n_frames bound must compare NOTHING. Here an
    // extended-layer-0 CI establishes a low-rate timing (maxPicPerSecond 1) that the
    // timecode's n_frames 5 would exceed; with the underivable targeting the bound
    // must not fire. Pre-fix the coarse fallback paired a global LAYER_UNSPECIFIED
    // timecode with EVERY CI, firing a hard error against an unrelated layer.
    let low_rate = CiTiming {
        display_tick: 1000,
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    // LAYER_UNSPECIFIED short-form (first byte 0x00) carrying a timecode with
    // n_frames 5 (>= maxPicPerSecond 1 for layer 0's CI).
    let mut bits = Bits::default();
    bits.f(0, 5); // counting_type
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(5, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x00, 4, &bits.into_bytes()),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_UNSPECIFIED timecode does not specify its layers, so the n_frames \
         bound must compare nothing (no false positive against layer 0's CI); \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_counting_type_reserved_is_warned() {
    // Cycle-3 finding 5 (§ 6.16.7 counting_type table marks 7..31 reserved, with no
    // "shall"). A reserved counting_type is a decoder-ignored producer anomaly
    // (warning), matching the established reserved-value pattern for
    // table-"reserved"-without-"shall" fields (metadata/persistence-idc-reserved).
    let mut bits = Bits::default();
    bits.f(7, 5); // counting_type = 7 (reserved)
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_warning(&report, "metadata/timecode-counting-type-reserved"),
        "report was: {report}"
    );
    // It is a warning, not a conformance error.
    assert!(
        report.is_conformant(),
        "a reserved counting_type is decoder-ignored, not a violation; \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_counting_type_defined_is_silent() {
    // counting_type 6 is the highest DEFINED value (§ 6.16.7 table), so no warning.
    let mut bits = Bits::default();
    bits.f(6, 5); // counting_type = 6 (defined)
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !has_warning(&report, "metadata/timecode-counting-type-reserved"),
        "counting_type 6 is defined; report was: {report}"
    );
}
