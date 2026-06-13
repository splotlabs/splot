// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn distinct_mlayer_count_exceeds_seqmax_is_flagged() {
    // § 6.4.1: SeqMaxMlayerCnt == 1, but the coded video sequence carries the
    // sequence header (embedded layer 0, forced by § 6.2.2) and a frame at embedded
    // layer 1 -> 2 distinct obu_mlayer_id values > SeqMaxMlayerCnt.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    // OBU_REGULAR_TILE_GROUP at mlayer 1, xlayer 0, references seq 0.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_within_seqmax_is_conforming() {
    // § 6.4.1: with SeqMaxMlayerCnt == 2 (sequence_header_payload(0, 1)), the same
    // two distinct embedded layers 0 and 1 are within budget — no diagnostic.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_resets_at_cvs_boundary() {
    // § 6.4.1 / § 7.3.6: the count is scoped to each coded video sequence. CVS 0
    // uses embedded layer 0 only; a CLK at embedded layer 1 starts CVS 1 for the
    // extended layer, where embedded layer 1 is the only distinct value. Each coded
    // video sequence carries one distinct obu_mlayer_id (<= SeqMaxMlayerCnt 1), so
    // the cumulative {0, 1} must NOT fire once the count resets at the boundary.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    // CVS 0: a frame at embedded layer 0 (references seq 0).
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    // Next temporal unit: a CLK at embedded layer 1 starts a new coded video sequence.
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the count must reset at the § 7.3.6 CVS boundary; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_first_clk_uses_active_header() {
    // Pre-first-CLK edge: with no CLK boundary yet, the implicit coded video
    // sequence still counts against the active (OBU-order fallback) header. A frame
    // at embedded layer 1 plus the embedded-layer-0 sequence header exceeds
    // SeqMaxMlayerCnt 1 even though no CLK has occurred.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // mlayer 1, no frame ref
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_emits_once_per_cvs() {
    // The check emits once per coded video sequence: two further frames at embedded
    // layer 1 after the first exceedance do not repeat the diagnostic.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
            .count(),
        1,
        "the § 6.4.1 distinct-mlayer check must emit once per CVS; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_pre_clk_obu_in_boundary_tu_is_not_flagged() {
    // § 7.3.6 / § 6.4.1: a coded video sequence starts AT the temporal unit that
    // contains the CLK, so an OBU of the same extended layer observed earlier in that
    // temporal unit already belongs to the NEW coded video sequence. Here the old CVS
    // (temporal unit 0) carries only embedded layer 0 (the sequence header) — within
    // SeqMaxMlayerCnt 1. Temporal unit 1 has a pre-CLK OBU at embedded layer 1 then a
    // CLK at embedded layer 1. Under FIX 4 (exact re-attribution), the new CVS is
    // re-seeded from the boundary temporal unit's seen set {1} (count 1 <= 1), so the
    // new CVS never exceeds; and the pre-CLK OBU's single-pass count into the *old*
    // CVS ({0, 1} = 2 > 1, first counted in temporal unit 0 so deferred) is dropped at
    // the boundary because the extended layer started a new CVS in temporal unit 1.
    // Both mechanisms leave nothing to emit. (This is also the FIX 4 "still-needed
    // pending-drop" coverage: the deferred exceedance whose set spanned a
    // pre-boundary temporal unit is dropped.)
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    // A frame at embedded layer 0 keeps the old CVS at {0} (count 1, conforming).
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    // Temporal unit 1: a pre-CLK OBU at embedded layer 1, then a CLK at embedded
    // layer 1 that begins a new coded video sequence for the extended layer.
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-CLK OBU in the CVS-starting temporal unit belongs to the new coded video \
         sequence; the new CVS {{1}} does not exceed and the deferred old-CVS exceedance \
         is dropped; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_pre_clk_header_reattributed_to_new_cvs_is_flagged() {
    // FIX 4 (Codex finding 1) positive case. § 7.3.6 (mirror
    // `07-decoding-process.md` lines 604-606): the new coded video sequence starts AT
    // the temporal unit containing the CLK, so the § 7.3.8.1 resent-at-RAP sequence
    // header observed BEFORE the CLK in that temporal unit (forced to obu_mlayer_id 0,
    // § 6.4.1 NOTE / § 6.2.2) belongs to the NEW coded video sequence and must count
    // toward SeqMaxMlayerCnt. A single temporal unit = [seq header @ mlayer 0,
    // CLK @ mlayer 1] with SeqMaxMlayerCnt 1 truly carries {0, 1} = 2 > 1 in the new
    // coded video sequence. The former whole-state drop at reset_cvs missed this; the
    // re-attribution must emit the exceedance exactly once.
    let mut data = temporal_delimiter_obu();
    // Resent-at-RAP sequence header (embedded layer 0, SeqMaxMlayerCnt 1).
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    // CLK at embedded layer 1 referencing seq 0 begins the new CVS at this temporal
    // unit; the pre-CLK header (mlayer 0) is re-attributed to it -> {0, 1} = 2 > 1.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the pre-CLK header is re-attributed to the new CVS; {{0, 1}} = 2 > 1 must fire \
         exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_excludes_pre_boundary_tu_ids() {
    // FIX 4 no-false-positive case. The re-seeded new-CVS set must include ONLY ids
    // from the boundary temporal unit, never ids from earlier temporal units. With
    // SeqMaxMlayerCnt 2: temporal unit 0's old CVS carries {0, 2} (count 2 <= 2,
    // conforming); temporal unit 1 = [header @ mlayer 0, CLK @ mlayer 1] re-attributes
    // only the boundary temporal unit's ids -> new CVS {0, 1} = 2 <= 2. Neither CVS
    // exceeds, so no diagnostic. (If reset_cvs wrongly carried temporal unit 0's ids,
    // the new CVS would be {0, 1, 2} = 3 > 2 and falsely fire.)
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    // SeqMaxMlayerCnt 2 (max_mlayer_id 2). sequence_header_payload(0, 2) sets
    // seq_max_mlayer_cnt_minus_1 = max_mlayer_id = 2 -> SeqMaxMlayerCnt 3; use an
    // explicit SeqMaxMlayerCnt-2 header instead.
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two()));
    // Old CVS ids {0 (header), 2}: a frame at embedded layer 2 (allowed, max_mlayer 2).
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 2, 0, 0));
    // Temporal unit 1: resent header (mlayer 0) then CLK @ mlayer 1 -> new CVS {0, 1}.
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two())); // resent header, mlayer 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK @ mlayer 1, ref seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the new CVS must re-seed only boundary-temporal-unit ids ({{0, 1}} <= 2); \
         earlier-temporal-unit ids must not count; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_reports_once_across_clk_in_boundary_tu() {
    // FIX 4 once-per-CVS guard across the boundary. In a single temporal unit (so the
    // set's first temporal unit is the boundary temporal unit and the exceedance emits
    // eagerly), pre-CLK ids {0, 1} already exceed SeqMaxMlayerCnt 1 (emitted once),
    // then a CLK @ mlayer 1 begins a new CVS re-seeded from {0, 1}. Because the old
    // state's first temporal unit equals the boundary temporal unit, the `reported`
    // flag carries into the re-seeded new-CVS state, so a further post-CLK OBU in the
    // same (now single) CVS does not re-report.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header, mlayer 0
    // Pre-CLK frame @ mlayer 1 -> {0, 1} = 2 > 1, first counted this temporal unit ->
    // eager emit (once).
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    // CLK @ mlayer 1 begins the new CVS, re-seeded from {0, 1} with the reported flag.
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
    // A further post-CLK OBU @ mlayer 1 in the same new CVS must not re-report.
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
            .count(),
        1,
        "the exceedance visible both pre- and post-CLK in the boundary temporal unit \
         must report exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_compares_against_clk_activated_header() {
    // PR #41 Codex false-positive regression. § 6.4.1 (mirror
    // `06-syntax-structures-semantics.md` lines 445-447): the distinct-obu_mlayer_id
    // count is scoped to "the coded video sequence associated with this sequence
    // header" — for the NEW coded video sequence a CLK begins that is the header the
    // CLK *activates*, not the outgoing one still active when the § 7.3.6 boundary
    // event fires. Outgoing header (id 0, SeqMaxMlayerCnt 1) is active and
    // frame-confirmed; the boundary temporal unit carries a re-sent header (mlayer 0)
    // and a pre-CLK OBU (mlayer 1) for the per-temporal-unit set {0, 1}, then a CLK
    // (mlayer 0) referencing a DIFFERENT header (id 1, SeqMaxMlayerCnt 2). The
    // re-seeded set {0, 1} = 2 conforms to the CLK-activated header's max 2, so
    // nothing must fire. Comparing 2 against the outgoing max 1 at reset time would
    // be a false positive.
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
    // Header id 1 with max_mlayer_id 1 -> SeqMaxMlayerCnt 2 (allows mlayer 0 and 1).
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 1)));
    // A frame at embedded layer 0 referencing seq 0 activates and frame-confirms it.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    // Temporal unit 1 (boundary): re-sent header (mlayer 0), a pre-CLK OBU (mlayer 1),
    // then a CLK (mlayer 0) referencing the DIFFERENT header id 1.
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the re-seeded {{0, 1}} = 2 set must be compared against the CLK-activated header \
         (id 1, SeqMaxMlayerCnt 2), not the outgoing header (id 0, max 1); report was: \
         {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_clk_activated_header_lower_max_is_flagged() {
    // Reverse-direction true positive that the reset-time check direction-masked.
    // Outgoing header (id 0, SeqMaxMlayerCnt 2) is active and frame-confirmed; the
    // boundary temporal unit carries a re-sent header (mlayer 0) and a pre-CLK OBU
    // (mlayer 1) for the per-temporal-unit set {0, 1}, then a CLK (mlayer 0)
    // referencing a DIFFERENT header (id 1, SeqMaxMlayerCnt 1). The re-seeded set
    // {0, 1} = 2 exceeds the CLK-activated header's max 1, so the § 6.4.1 exceedance
    // must fire exactly once, anchored at the CLK's extension byte. The old reset-time
    // check passed (2 <= outgoing max 2) and the activation-path retroactive check
    // catches it because the referenced id changes.
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    // Header id 0 with max_mlayer_id 1 -> SeqMaxMlayerCnt 2.
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one_id(1))); // header id 1, max 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
    // Temporal unit 1 (boundary): re-sent header (mlayer 0), pre-CLK OBU (mlayer 1),
    // CLK (mlayer 0) referencing the DIFFERENT header id 1 (max 1).
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1))); // re-sent header id 0, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1 (max 1)
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the re-seeded {{0, 1}} = 2 set exceeds the CLK-activated header's max 1 and must \
         fire exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_same_header_exceedance_is_flagged() {
    // Same-header coverage the immediate reset-time check used to own: when the CLK
    // re-references the SAME already-frame-confirmed header, the re-seeded pre-CLK set
    // may already exceed that header's max in a way the eager count_distinct_mlayer
    // cannot re-surface (it never re-yields an already-seen id). Outgoing header id 0
    // (SeqMaxMlayerCnt 1) active and frame-confirmed; boundary temporal unit carries a
    // re-sent header (mlayer 0) and a pre-CLK OBU (mlayer 1) for the per-temporal-unit
    // set {0, 1} = 2 > 1, then a CLK (mlayer 1) referencing the SAME header id 0. The
    // CLK's own mlayer 1 is already in the re-seeded set, so the eager path yields
    // nothing; the post-activation retroactive check must still fire once.
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
    // Temporal unit 1 (boundary): re-sent header (mlayer 0), pre-CLK OBU (mlayer 1),
    // CLK (mlayer 1) referencing the SAME header id 0.
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK mlayer 1, ref seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the re-seeded {{0, 1}} = 2 set exceeds the re-referenced header's max 1 and must \
         fire exactly once even though the CLK's own mlayer is already in the set; report \
         was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // § 6.4.1: under caller-provided external HLS the active sequence header (and its
    // SeqMaxMlayerCnt) may be supplied out of band, so the in-band distinct-mlayer
    // count is unreliable and the check is suppressed even on the otherwise-firing
    // two-embedded-layer stream.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the § 6.4.1 distinct-mlayer check; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_accumulated_before_header_activation_is_flagged() {
    // § 6.4.1: OBUs arriving before any active sequence header for their extended
    // layer accumulate a distinct-obu_mlayer_id count that the eager per-OBU check
    // cannot compare (no active header yet, and the activating header's own
    // already-seen obu_mlayer_id == 0 yields nothing new). Here two pre-header OBUs at
    // embedded layers 0 and 1 accumulate {0, 1} = 2 before the sequence header
    // activates with SeqMaxMlayerCnt 1; the activation-path retroactive check must
    // emit the exceedance, exactly once.
    let mut data = temporal_delimiter_obu();
    // Pre-header OBUs at embedded layers 0 and 1 (counted, no header active yet).
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    // The sequence header (embedded layer 0, forced by § 6.2.2) now activates with
    // SeqMaxMlayerCnt 1; its own obu_mlayer_id 0 was already counted above.
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "a pre-header distinct-mlayer count must fire once on activation; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_accumulated_before_header_activation_within_seqmax_is_conforming() {
    // § 6.4.1: the same pre-header accumulation of embedded layers 0 and 1 ({0, 1} =
    // 2) is within budget when the activating header has SeqMaxMlayerCnt 2
    // (sequence_header_payload(0, 1)); the retroactive check must NOT fire.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_header_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // § 6.4.1: caller-provided external HLS suppresses the retroactive activation-path
    // check exactly as it suppresses the eager per-OBU check — an out-of-band header
    // may carry a SeqMaxMlayerCnt this validator does not model.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the retroactive distinct-mlayer check; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_is_flagged() {
    // § 6.4.1 / § 5.18.2: the retroactive distinct-mlayer comparison fires on the
    // frame-header load_sequence_header activation path, not only on an OBU-order
    // sequence-header activation. Extended layer 1 (which the xlayer-0 sequence
    // header does NOT activate by OBU order) accumulates {0, 1} = 2 distinct
    // obu_mlayer_id values before any header is active for it; a non-CLK frame
    // (OBU_REGULAR_TILE_GROUP) at xlayer 1 then references seq 0 (SeqMaxMlayerCnt 1)
    // and frame-confirms its activation. The activating frame's own obu_mlayer_id 0
    // is already in the set, so the eager count_distinct_mlayer yields nothing — only
    // the activation-path retroactive check can flag the exceedance.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    // Pre-header OBUs at xlayer 1, embedded layers 0 and 1 (no header active for
    // xlayer 1 yet, so they only accumulate the distinct count).
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    // A non-CLK frame at xlayer 1, embedded layer 0, references seq 0: the § 5.18.2
    // activation that makes SeqMaxMlayerCnt available for xlayer 1.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "a pre-header count must fire once on frame-header activation; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_within_seqmax_is_conforming() {
    // § 6.4.1: the same pre-header accumulation at xlayer 1 ({0, 1} = 2) is within
    // budget when the frame-confirmed activating header has SeqMaxMlayerCnt 2
    // (sequence_header_payload(0, 1)); the frame-header-path retroactive check must
    // NOT fire.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // § 6.4.1: caller-provided external HLS suppresses the frame-header-path
    // retroactive check exactly as it suppresses the OBU-order-path and eager checks —
    // an out-of-band header may carry a SeqMaxMlayerCnt this validator does not model.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the frame-header-path retroactive check; report was: {report}"
    );
}

// --- § 7.3.6 single active sequence header per extended layer per CVS (3.3) --

#[test]
fn second_activation_without_clk_is_flagged() {
    // § 7.3.6: a frame-confirmed activation of seq 0, then a non-CLK frame in the
    // same coded video sequence activating a different seq 1, violates the
    // single-active-sequence-header rule.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    // Two OBU_REGULAR_TILE_GROUP (type 7) frames, xlayer 0: confirm seq 0, then seq 1.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn reactivation_across_clk_is_conforming() {
    // § 7.3.6: a CLK starts a new coded video sequence, so re-activating a different
    // seq 1 across the CLK is permitted — the rule resets at each CLK.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
    // New temporal unit, then a CLK that starts a new CVS and activates seq 1.
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "a CLK re-activation must not fire the § 7.3.6 check; report was: {report}"
    );
}

#[test]
fn fallback_guess_then_frame_reference_is_not_flagged() {
    // § 7.3.6: when the prior activation was only the OBU-order fallback guess (not
    // frame-confirmed), the first frame referencing a different seq must not fire —
    // a guess a frame can contradict was never a real activation.
    let mut data = temporal_delimiter_obu();
    // OBU-order fallback activates seq 0 (first seen); seq 1 is also available.
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    // The first frame frame-confirms seq 1 (different from the fallback seq 0).
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "a fallback guess overridden by the first frame must not fire; report was: {report}"
    );
}

#[test]
fn unreferenced_extra_sequence_header_is_conforming() {
    // § 7.3.6: additional sequence header OBUs with a different seq_header_id may be
    // present but unactivated; one never referenced by a frame must not fire.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    // The frame confirms seq 0.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    // An extra, unreferenced sequence header with a different id.
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "an unreferenced extra sequence header must not fire; report was: {report}"
    );
}

#[test]
fn second_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // § 7.3.6: under caller-provided external HLS the active sequence header may be
    // supplied out of band, so the in-band activation history is unreliable and the
    // check is suppressed even on the otherwise-firing two-activation stream.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "external HLS must suppress the § 7.3.6 check; report was: {report}"
    );
}

/// Builds the otherwise-firing two-activation stream of
/// [`second_activation_without_clk_is_flagged`] (frame-confirm seq 0, then a non-CLK
/// frame activating seq 1 in the same CVS for xlayer 0).
pub(in crate::validator::tests) fn two_activation_stream() -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // confirm seq 1
    data
}

#[test]
fn second_activation_under_empty_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // FIX 2 (Codex finding 3). `ExternalHlsSet::new()` declares an external channel
    // that declares NO sequence header, so it cannot supply an out-of-band active
    // sequence header (the validator emits hls/unavailable-sequence-header on that
    // premise elsewhere). The § 7.3.6 gate must therefore narrow to
    // declares_any_sequence_header() — an empty set must NOT suppress.
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "an empty external set declares no sequence header and must not suppress; \
         report was: {report}"
    );
}

#[test]
fn second_activation_under_sequence_free_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // FIX 2: a non-empty external set that declares only an operating point set (no
    // sequence header) likewise cannot supply an out-of-band active sequence header,
    // so the § 7.3.6 check must still fire.
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(0, 0),
        ),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "a sequence-header-free external set must not suppress; report was: {report}"
    );
}

#[test]
fn second_activation_under_out_of_range_external_hls_id_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // FIX 2: an out-of-range declared id is ignored (options.rs), so the set declares
    // no usable sequence header and must not suppress the § 7.3.6 check (mirrors
    // external_hls_out_of_range_id_does_not_suppress_no_active_sequence_header).
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(16)),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "an out-of-range external id is ignored and must not suppress; report was: {report}"
    );
}

// --- § 6.4.1 monotonic_output_order_flag agreement across a CMVS (3.4) -------

/// A sequence-header payload (xlayer-neutral) with the given `seq_header_id`,
/// `max_mlayer_id == 0`, and an explicit `monotonic_output_order_flag`.
pub(in crate::validator::tests) fn seq_header_payload_monotonic(
    seq_header_id: u32,
    monotonic: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id);
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id
    bits.f(0, 3); // max_mlayer_id = 0 (no seq_max_mlayer_cnt_minus_1 field)
    bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    // max_mlayer_id == 0 -> no mlayer_dependency_present_flag; max_tlayer_id == 0 ->
    // no tlayer_dependency_present_flag (§ 5.4.1).
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A sequence-header OBU for `xlayer` carrying [`seq_header_payload_monotonic`].
pub(in crate::validator::tests) fn seq_header_obu_monotonic(
    xlayer: u8,
    seq_header_id: u32,
    monotonic: bool,
) -> Vec<u8> {
    let payload = seq_header_payload_monotonic(seq_header_id, monotonic);
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

/// Builds a stream whose temporal unit 1 begins a § 7.3.2 CMVS (begin condition 1:
/// a CLK temporal unit with an MSDO present and no CMVS yet active), with sequence
/// headers for extended layers 0 and 1 whose `monotonic_output_order_flag` values
/// are `monotonic_x0` and `monotonic_x1`. Temporal unit 2 (the CMVS is definitively
/// `Inside` by then) *frame-confirms* both extended layers in turn — first xlayer 0,
/// then xlayer 1 — so each layer is associated with its referenced sequence header
/// per § 5.18.2 rather than the OBU-order fallback guess (§ 7.3.6 forbids treating
/// an unreferenced extra header as activated). The cross-layer agreement check runs
/// at each frame; the disagreement, if any, is emitted "when the second of the two
/// headers is activated" (the xlayer-1 frame).
pub(in crate::validator::tests) fn cmvs_two_layer_stream(
    monotonic_x0: bool,
    monotonic_x1: bool,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // starts temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
    data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
    data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
    // Frame-confirm xlayer 0 (ref seq 0), then xlayer 1 (ref seq 1); the agreement
    // check runs at each, and the disagreement fires at the second activation.
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
    data
}
