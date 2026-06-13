// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn validator_flags_frame_tile_cols_out_of_range() {
    // A 4160x16 frame (BLOCK_64X64, level 0 Main: maxTileWidthSb == 64) has
    // sbCols == 65; a non-uniform tile_params() coding 65 one-superblock columns
    // derives TileCols == 65 > MAX_TILE_COLS (AV2 § 6.17.7.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        frame_width_bits_minus_1: 12, // frame_width_minus_1 f(13)
        max_frame_width_minus_1: 4159,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 4160, 16, (13, 8));
    fb.bit(0); // uniform_tile_spacing_flag = 0 (tile_params, § 5.18.7.3)
    for start in 0..65u32 {
        // width_in_sbs_minus_1 ns(Min(sbCols - startSb, maxTileWidthSb)) == 0.
        ns_zero(&mut fb, (65 - start).min(64));
    }
    ns_zero(&mut fb, 1); // height_in_sbs_minus_1 (sbRows == 1, 0 bits)
    fb.f(0, 7); // context_update_tile_id f(TileRowsLog2 0 + TileColsLog2 7)
    fb.f(0, 2); // tile_size_bytes_minus_1
    quant_seg_tail(&mut fb);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/tile-cols-out-of-range"
                && d.spec_section.as_deref() == Some("6.17.7.2")
        }),
        "report was: {report}"
    );
    // context_update_tile_id 0 < 65 stays valid.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/context-update-tile-id-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_frame_tile_rows_out_of_range() {
    // The transposed layout: a 16x4160 frame has sbRows == 65; a non-uniform
    // tile_params() coding 65 one-superblock rows derives TileRows == 65 >
    // MAX_TILE_ROWS (AV2 § 6.17.7.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        frame_height_bits_minus_1: 12, // frame_height_minus_1 f(13)
        max_frame_height_minus_1: 4159,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 16, 4160, (8, 13));
    fb.bit(0); // uniform_tile_spacing_flag = 0
    ns_zero(&mut fb, 1); // width_in_sbs_minus_1 (sbCols == 1, 0 bits)
    for start in 0..65u32 {
        // height_in_sbs_minus_1 ns(Min(sbRows - startSb, maxTileHeightSb == 65)).
        ns_zero(&mut fb, 65 - start);
    }
    fb.f(0, 7); // context_update_tile_id f(TileRowsLog2 7 + TileColsLog2 0)
    fb.f(0, 2); // tile_size_bytes_minus_1
    quant_seg_tail(&mut fb);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/tile-rows-out-of-range"
                && d.spec_section.as_deref() == Some("6.17.7.2")
        }),
        "report was: {report}"
    );
}

/// Appends the uniform 3x1 tile_info() bits for a 160x16 frame (sbCols == 3:
/// increments 1,1 reach maxLog2TileCols == 2, TileCols == 3, TileColsLog2 == 2),
/// with the given `context_update_tile_id` f(2) (AV2 § 5.18.7.2 / § 5.18.7.3).
pub(in crate::validator::tests) fn uniform_3x1_tile_info(
    fb: &mut Bits,
    context_update_tile_id: u32,
) {
    fb.bit(1); // uniform_tile_spacing_flag
    fb.bit(1); // increment_tile_cols_log2 = 1
    fb.bit(1); // increment_tile_cols_log2 = 1 (reaches maxLog2TileCols)
    fb.f(context_update_tile_id, 2); // f(TileRowsLog2 0 + TileColsLog2 2)
    fb.f(0, 2); // tile_size_bytes_minus_1
}

#[test]
fn validator_flags_frame_context_update_tile_id_out_of_range() {
    // A 160x16 frame splits into TileCols == 3 (not a power of two), so the
    // f(2) context_update_tile_id read can encode 3 >= TileCols * TileRows == 3
    // (AV2 § 6.17.7.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        max_frame_width_minus_1: 159,
        ..FrameCoreSeq::base()
    });
    // The diagnostic is located at the frame OBU header, one Annex B leb128
    // size-prefix byte past the end of the preceding OBUs.
    let frame_obu_offset = data.len() as u64 + 1;
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
    uniform_3x1_tile_info(&mut fb, 3); // context_update_tile_id == 3 (>= 3 * 1)
    quant_seg_tail(&mut fb);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/context-update-tile-id-out-of-range"
                && d.spec_section.as_deref() == Some("6.17.7.2")
                && d.byte_offset.map(|offset| offset.get()) == Some(frame_obu_offset)
        }),
        "expected the § 6.17.7.2 diagnostic at the frame OBU offset \
         {frame_obu_offset}; report was: {report}"
    );
}

#[test]
fn validator_accepts_conforming_frame_tile_layout() {
    // The same 3x1 layout with context_update_tile_id == 2 (< 3) is conformant:
    // none of the § 6.17.7.2 frame tile diagnostics fire.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        max_frame_width_minus_1: 159,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
    uniform_3x1_tile_info(&mut fb, 2); // context_update_tile_id == 2 (< 3 * 1)
    quant_seg_tail(&mut fb);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    for rule in [
        "frame-header/tile-cols-out-of-range",
        "frame-header/tile-rows-out-of-range",
        "frame-header/context-update-tile-id-out-of-range",
    ] {
        assert!(
            !report.errors().any(|d| d.rule_id == rule),
            "{rule} must not fire on a conforming layout; report was: {report}"
        );
    }
}

/// Appends a 16x16 CLK frame whose `setup_qm_params()` (§ 5.18.6.2) references
/// `qm_y[0] == level` with `qm_uv_same_as_y == 1` (so `qm_u[0]` / `qm_v[0]`
/// reference the same slot for the 4:2:0 sequence).
pub(in crate::validator::tests) fn clk_frame_with_qm_reference(level: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 16, 16, (8, 8));
    fb.bit(1); // uniform_tile_spacing_flag (sbCols == 1: no increments)
    fb.f(100, 9); // base_q_idx f(9) (§ 5.18.6.1)
    fb.bit(0); // segmentation_enabled (§ 5.18.7.1)
    fb.bit(1); // using_qmatrix (§ 5.18.6.2)
    fb.f(level, 4); // qm_y[0]
    fb.bit(1); // qm_uv_same_as_y (NumPlanes == 3)
    fb.bit(0); // delta_q_present (§ 5.18.7.8)
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A `quantizer_matrix_obu()` selecting a single default `level` with
/// `qm_chroma_info_present_flag == 1` (`QmNumPlanes == 3`, AV2 § 5.13).
pub(in crate::validator::tests) fn qm_default_level_obu_chroma(level: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(1 << level, 15); // qm_bit_map: set `level`
    bits.bit(1); // qm_chroma_info_present_flag = 1 -> 3 planes
    bits.bit(1); // qm_is_default_flag for `level`
    bits.bit(1); // trailing_one_bit (QM is non-extensible)
    bits.align();
    annex_b_obu(QM_HEADER, &bits.into_bytes())
}

/// As [`qm_default_level_obu_chroma`], but the QM OBU is at `(obu_tlayer_id, obu_mlayer_id) ==
/// (tlayer, mlayer)` on xlayer 0 (QM OBU type 22), so the recorded level's `QmMLayerId` /
/// `QmTLayerId` is `(mlayer, tlayer)`. Lets a test record a QM level at a layer the
/// referencing frame does not depend on (the § 6.17.6.2 layer-dependency check).
pub(in crate::validator::tests) fn qm_default_level_obu_chroma_at_layer(
    level: u32,
    tlayer: u8,
    mlayer: u8,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(1 << level, 15); // qm_bit_map: set `level`
    bits.bit(1); // qm_chroma_info_present_flag = 1 -> 3 planes
    bits.bit(1); // qm_is_default_flag for `level`
    bits.bit(1); // trailing_one_bit (QM is non-extensible)
    bits.align();
    annex_b_obu_with_header(&layer_obu_header(22, tlayer, mlayer, 0), &bits.into_bytes())
}

/// A `quantizer_matrix_obu()` with `qm_bit_map == 0` (the § 5.13 reset/default path)
/// and `qm_chroma_info_present_flag == 1` (`QmNumPlanes == 3`). This makes EVERY custom
/// level available as a default record with `QmMLayerId == -1` (the validator's
/// `mlayer_id == None`), which is the arm the § 5.18.2 SWITCH / RAS `reset_qm()` can
/// PROVABLY reset (mirror :5351, `needsReset = QmMLayerId[level] == -1`). It also sets
/// `QmProtected[level] = 1` for every level (mirror :3010), so a temporal delimiter
/// must intervene before a later reset can act.
pub(in crate::validator::tests) fn qm_reset_obu_chroma() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 15); // qm_bit_map == 0 -> reset/default path
    bits.bit(1); // qm_chroma_info_present_flag = 1 -> 3 planes
    bits.bit(1); // trailing_one_bit (QM is non-extensible)
    bits.align();
    annex_b_obu(QM_HEADER, &bits.into_bytes())
}

#[test]
fn validator_flags_frame_qm_plane_count_mismatch() {
    // A QM OBU defines custom level 0 with QmNumPlanes == 1
    // (qm_chroma_info_present_flag == 0); the 4:2:0 sequence has NumPlanes == 3,
    // so a frame referencing level 0 violates AV2 § 6.17.6.2.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu(0));
    data.extend(clk_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    let matches: Vec<_> = report
        .errors()
        .filter(|d| d.rule_id == "frame-header/qm-plane-count-mismatch")
        .collect();
    assert!(
        matches
            .iter()
            .any(|d| d.spec_section.as_deref() == Some("6.17.6.2")),
        "report was: {report}"
    );
    // qm_y/qm_u/qm_v all reference the same slot: one diagnostic, not three.
    assert_eq!(matches.len(), 1, "report was: {report}");
}

#[test]
fn validator_is_silent_on_frame_qm_reference_without_qm_state() {
    // No quantizer matrix OBU defines level 0: the § 6.17.6.2 plane-count check
    // has no recorded QmNumPlanes to compare and must stay silent (the HLS
    // availability checks own the missing-state case).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
        "missing QM state must not produce a false positive; report was: {report}"
    );
}

#[test]
fn validator_accepts_frame_qm_reference_with_matching_planes() {
    // A chroma QM OBU records QmNumPlanes == 3 for level 0, matching the 4:2:0
    // sequence's NumPlanes == 3: conformant per AV2 § 6.17.6.2.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu_chroma(0));
    data.extend(clk_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn validator_is_silent_on_default_matrix_qm_reference() {
    // qm_y == 15 == NUM_CUSTOM_QMS selects the default matrix, not a custom
    // slot: the § 6.17.6.2 plane-count requirement does not apply, even with
    // mismatched recorded state for other levels.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu(0)); // 1-plane state for (unreferenced) level 0
    data.extend(clk_frame_with_qm_reference(15));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
        "report was: {report}"
    );
}

// --- § 7.3.8.9 quantizer-matrix availability + QmProtected reset ---

#[test]
fn validator_flags_qm_level_unavailable_when_no_qm_obu() {
    // A CLK frame with using_qmatrix == 1 references custom level 0, but no QM OBU has
    // made level 0 available — § 7.3.8.9 violation (the CLK's own reset_qm clears the
    // never-sent unprotected level, and external HLS is disabled by default).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "report was: {report}"
    );
}

#[test]
fn validator_silent_on_qm_level_available_via_qm_obu() {
    // A QM OBU (chroma -> 3 planes, matching the 4:2:0 sequence) makes level 0 available
    // before the referencing CLK in the SAME temporal unit, so the §7.3.8.9 availability
    // check stays silent (the level is QmProtected and present).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu_chroma(0));
    data.extend(clk_frame_with_qm_reference(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "an available QM level must not fire; report was: {report}"
    );
}

#[test]
fn validator_qm_level_unavailable_boundary_default_level_silent() {
    // qm_y == 15 == NUM_CUSTOM_QMS selects the built-in default matrix, never a custom
    // slot, so §7.3.8.9 availability does not apply even with no QM OBU.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_frame_with_qm_reference(15));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "the default matrix is always available; report was: {report}"
    );
}

#[test]
fn validator_qm_level_unavailable_suppressed_under_external_provided() {
    // ExternalHlsSet cannot express QM OBUs, so any Provided mode means the level MAY be
    // supplied externally — the §7.3.8.9 availability check suppresses (inexpressible
    // kind), like the film-grain availability check.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_frame_with_qm_reference(0));
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a Provided external-HLS mode must suppress QM availability; report was: {report}"
    );
}

#[test]
fn validator_qm_protected_reset_clears_unprotected_level_across_temporal_units() {
    // TU1 sends level 0 via a QM OBU (no referencing frame). TU2 starts with a CLK that
    // references level 0 without re-sending it: the temporal delimiter cleared
    // QmProtected, so the CLK's reset_qm() clears the (now unprotected) level 0 — §7.3.8.9
    // unavailable fires. This proves the QmProtected reset is modeled (both-order:
    // QM-then-frame within each TU, frame across the TU boundary).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu_chroma(0)); // TU1: level 0 available + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(clk_frame_with_qm_reference(0)); // CLK reset_qm() clears unprotected lvl 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "a level reset out of a previous temporal unit must be unavailable; report was: {report}"
    );
}

#[test]
fn validator_qm_protected_reset_preserves_resent_level_in_current_temporal_unit() {
    // Same as above, but TU2 RE-SENDS level 0 before the CLK: the QM OBU sets QmProtected
    // for level 0, so the CLK's reset_qm() does NOT clear it — §7.3.8.9 stays silent.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(qm_default_level_obu_chroma(0)); // TU1
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(qm_default_level_obu_chroma(0)); // TU2 re-sends level 0 -> protected
    data.extend(clk_frame_with_qm_reference(0)); // reset_qm() preserves protected level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a level re-sent in the current temporal unit must survive reset_qm; report was: {report}"
    );
}

/// A RAS frame (`OBU_RAS_FRAME`) whose parse REACHES the § 5.18.2 `reset_qm()` call
/// site (mirror :4283) — proving the reset condition fired. It reads
/// `restricted_prediction_switch`, `num_key_ref_frames == 0`, the output flags, the
/// SWITCH-forced `frame_size_override_flag`, and `order_hint`, then converges on the
/// RAS `refresh_frame_flags` derivation arm (mirror :4493, `max_mlayer_id == 0`), which
/// reads no bits and stops with `InterStop::UnmodeledDerivation` — an
/// `UnsupportedUntilFeature` coverage stop that yields a resolvable (`Some`) core. The
/// stop is past :4283, so the RAS reset is CONFIRMED. The sequence must have
/// `long_term_frame_id_bits != 0` (§ 6.4.6), so the caller threads
/// `FrameCoreSeq { long_term_frame_id_bits: 4, .. }`.
pub(in crate::validator::tests) fn ras_frame_confirmed_reset() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
    fb.uvlc(0); // cur_mfh_id == 0 -> direct sequence-header reference
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch f(1)  (read on the RAS/SWITCH arm)
    fb.f(0, 3); // num_key_ref_frames == 0 (no ref_long_term_id reads)
    fb.bit(0); // immediate_output_frame f(1)  (RAS is not OLK; monotonic -> implicit 0)
    // SWITCH_FRAME forces frame_size_override_flag == 1 (no bit).
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // SWITCH primary-ref block reads no bits; the RAS (max_mlayer_id == 0)
    // refresh-flags arm reads no bits and stops -> reset_qm() (:4283) already ran.
    annex_b_obu(RAS_HEADER, &fb.into_bytes())
}

/// A RAS frame (`OBU_RAS_FRAME`) truncated BEFORE the § 5.18.2 `reset_qm()` call site
/// (mirror :4283): it reads `restricted_prediction_switch` and `num_key_ref_frames`, then
/// the payload runs out inside the `ref_long_term_id[i]` loop (mirror :4252) — before the
/// reset is reached. The core parse fails (`Err` propagates -> resolves to `None`), so the
/// RAS reset is UNCONFIRMED. The sequence must have `long_term_frame_id_bits != 0`
/// (§ 6.4.6); the caller threads `FrameCoreSeq { long_term_frame_id_bits: 4, .. }`, for
/// which the unread `ref_long_term_id` reads are f(4) each (7 of them == 28 bits, far past
/// the byte-aligned padding, forcing a true EOF before :4283).
pub(in crate::validator::tests) fn ras_frame_truncated_before_reset() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch f(1)
    fb.f(7, 3); // num_key_ref_frames == 7 -> 7 * f(4) ref_long_term_id reads follow
    // Payload ends here: the ref_long_term_id[0..7] reads (28 bits) run past the payload,
    // so parse_frame_header_core returns Err -> the validator's core resolves to None,
    // BEFORE the reset_qm() call site (mirror :4283).
    annex_b_obu(RAS_HEADER, &fb.into_bytes())
}

/// An INTRA_ONLY regular-tile-group frame whose `setup_qm_params()` references custom
/// `qm_y[0] == level` (the same § 5.18.6.2 reference as
/// [`clk_frame_with_qm_reference`]), but as an INTRA_ONLY frame it triggers NO
/// `reset_qm()` (§ 5.18.2 mirror :4279-4283 resets only on CLK / OLK / SWITCH / RAS).
/// This isolates the QM-availability judgment from the referencing frame's own reset,
/// so a stale / poisoned availability state is the only thing that can flip the
/// §7.3.8.9 diagnostic. `refresh_frame_flags == 1` (a single non-output slot, never
/// all-slots) keeps the §6.17.2 intra-only checks silent.
pub(in crate::validator::tests) fn intra_only_frame_with_qm_reference(level: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY (no reset_qm)
    fb.bit(1); // immediate_output_frame == 1 (output frame; no implicit bit)
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(16 - 1, 8); // frame_width_minus_1 -> FrameWidth 16 (== max)
    fb.f(16 - 1, 8); // frame_height_minus_1 -> FrameHeight 16 (== max)
    fb.f(1, 8); // refresh_frame_flags f(NumRefFrames == 8) == 1 (not all-slots)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    fb.bit(1); // uniform_tile_spacing_flag (sbCols == 1: no increments)
    fb.f(100, 9); // base_q_idx f(9) (§ 5.18.6.1)
    fb.bit(0); // segmentation_enabled (§ 5.18.7.1)
    fb.bit(1); // using_qmatrix (§ 5.18.6.2)
    fb.f(level, 4); // qm_y[0]
    fb.bit(1); // qm_uv_same_as_y (NumPlanes == 3)
    fb.bit(0); // delta_q_present (§ 5.18.7.8)
    annex_b_obu(RTG_HEADER, &fb.into_bytes())
}

#[test]
fn validator_qm_truncated_ras_reset_does_not_falsely_fire_unavailable() {
    // A truncated RAS frame (payload ends after the prefix, before
    // restricted_prediction_switch) never reaches the § 5.18.2 reset_qm() call site
    // (mirror :4283). Its parse fails (Err -> core None), so the RAS reset is NOT
    // confirmed: the QM availability state must be POISONED (unknown), not cleared to
    // "unavailable". A later INTRA_ONLY frame (no reset_qm of its own) referencing a QM
    // level made available before the RAS must therefore stay SILENT — the poison drops
    // the §7.3.8.9 judgment. Pre-fix the RAS branch cleared on the OBU type alone, so
    // this fired falsely.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4, // § 6.4.6: RAS requires long_term_frame_id_bits != 0
        ..FrameCoreSeq::base()
    });
    // The reset QM OBU makes level 0 available with QmMLayerId == -1 (mlayer_id None) —
    // the arm a SWITCH / RAS reset_qm() can PROVABLY clear (mirror :5351).
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(ras_frame_truncated_before_reset()); // truncated RAS: reset unconfirmed
    data.extend(temporal_delimiter_obu()); // TU3 starts: QmProtected cleared
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a truncated RAS whose reset is unconfirmed must POISON (not clear) QM \
         availability, so the later reference stays silent; report was: {report}"
    );
}

#[test]
fn validator_qm_confirmed_ras_reset_fires_unavailable_without_resend() {
    // A RAS frame whose parse reaches the reset_qm() call site (mirror :4283) CONFIRMS
    // the reset: the unprotected level 0 made available in a prior TU is cleared to
    // "unavailable". A later INTRA_ONLY frame (no reset_qm of its own) referencing level
    // 0 without re-sending it must fire §7.3.8.9 unavailable — the confirmed reset
    // grounds "unavailable", not a poison.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    // The reset QM OBU makes level 0 available with QmMLayerId == -1 (mlayer_id None) —
    // the arm the confirmed RAS reset_qm() provably clears (mirror :5351).
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(ras_frame_confirmed_reset()); // confirmed RAS reset_qm() clears unprotected lvl 0
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "a confirmed RAS reset must ground the level unavailable; report was: {report}"
    );
}

#[test]
fn validator_qm_confirmed_ras_reset_clears_same_layer_level_via_presence_map() {
    // The § 5.18.2 reset_qm() SWITCH/RAS presence-map arm (mirror :5352): a level with a
    // recorded QmMLayerId == m (NOT the -1 arm) is reset when
    // MLayerPresenceMap[m][obu_mlayer_id] == 1. Here the QM OBU and the RAS are both at the
    // base embedded layer 0, so QmMLayerId == 0 == obu_mlayer_id and MLayerPresenceMap[0][0]
    // == 1 (reflexive) — the level is reset. A later INTRA_ONLY frame referencing level 0
    // without re-sending it must fire §7.3.8.9 unavailable. (Before the presence arm was
    // modeled this under-reported: the QmMLayerId == 0 level was wrongly left available.)
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    // A NORMAL QM OBU (qm_bit_map sets level 0): the level records QmMLayerId == 0 (the OBU's
    // obu_mlayer_id), the presence-arm case — distinct from qm_reset_obu_chroma's -1 arm.
    data.extend(qm_default_level_obu_chroma(0)); // TU1: level 0 available, QmMLayerId 0 + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(ras_frame_confirmed_reset()); // confirmed RAS at layer 0: reflexive presence reset
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "a confirmed same-layer RAS reset must clear the QmMLayerId == obu_mlayer_id level via \
         the reflexive MLayerPresenceMap arm; report was: {report}"
    );
}

#[test]
fn validator_qm_confirmed_ras_reset_preserves_cross_layer_level_via_presence_map() {
    // The negative of the presence-map arm: a level defined at embedded layer 0 (QmMLayerId 0)
    // is NOT reset by a confirmed RAS at obu_mlayer_id 1, because MLayerPresenceMap[0][1] == 0
    // (decoding layer 0 does not require layer 1, default §5.4.1 fill). The level stays
    // available, so a later INTRA_ONLY frame at layer 0 referencing it must NOT fire §7.3.8.9.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        max_mlayer_id: 1,
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    });
    data.extend(qm_default_level_obu_chroma(0)); // TU1: level 0 at layer 0, QmMLayerId 0
    data.extend(temporal_delimiter_obu()); // TU2 starts
    data.extend(ras_frame_explicit_map_at_layer(1, 0, 1, 8, 1)); // confirmed RAS at layer 1
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references the surviving level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a cross-layer RAS (MLayerPresenceMap[0][1] == 0) must not reset the layer-0 level; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_qm_mlayer_dependency_missing() {
    // §6.17.6.2 (mirror :5413-5415): a QM OBU at embedded layer 1 defines custom level 0
    // (QmMLayerId 1). An INTRA_ONLY frame at obu_mlayer_id 0 references level 0 via
    // using_qmatrix. Under the default lower-triangular fill (max_mlayer_id 1),
    // MLayerDependencyMap[0][1] == 0, so the frame does not depend on the level's defining
    // embedded layer -> frame-header/qm-mlayer-dependency-missing.
    let seq = FrameCoreSeq {
        max_mlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(qm_default_level_obu_chroma_at_layer(0, 0, 1)); // level 0 defined at mlayer 1
    data.extend(intra_only_frame_with_qm_reference(0)); // frame at layer 0 references level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-mlayer-dependency-missing"
                && d.spec_section.as_deref() == Some("6.17.6.2")
        }),
        "a QM level defined at an undepended embedded layer must fire \
         qm-mlayer-dependency-missing; report was: {report}"
    );
}

#[test]
fn validator_flags_qm_tlayer_dependency_missing() {
    // §6.17.6.2 (mirror :5417-5419): a QM OBU at temporal layer 1 defines custom level 0
    // (QmTLayerId 1, QmMLayerId 0). An INTRA_ONLY frame at obu_tlayer_id 0 references it; the
    // embedded-layer dependency is satisfied (MLayerDependencyMap[0][0] == 1) but the default
    // fill (max_tlayer_id 1) leaves TLayerDependencyMap[0][0][1] == 0 -> qm-tlayer-dependency.
    let seq = FrameCoreSeq {
        max_tlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(qm_default_level_obu_chroma_at_layer(0, 1, 0)); // level 0 defined at tlayer 1
    data.extend(intra_only_frame_with_qm_reference(0)); // frame at tlayer 0 references level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-tlayer-dependency-missing"
                && d.spec_section.as_deref() == Some("6.17.6.2")
        }),
        "a QM level defined at an undepended temporal layer must fire \
         qm-tlayer-dependency-missing; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-mlayer-dependency-missing"),
        "the embedded-layer dependency is satisfied (reflexive), so only the tlayer rule \
         fires; report was: {report}"
    );
}

#[test]
fn validator_qm_layer_dependency_satisfied_is_silent() {
    // A QM level defined at the base layer (mlayer 0, tlayer 0) referenced by a layer-0 frame
    // is fully depended-on (MLayerDependencyMap[0][0] == TLayerDependencyMap[0][0][0] == 1):
    // neither §6.17.6.2 layer-dependency diagnostic fires.
    let seq = FrameCoreSeq {
        max_mlayer_id: 1,
        max_tlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(qm_default_level_obu_chroma_at_layer(0, 0, 0)); // level 0 at base layer
    data.extend(intra_only_frame_with_qm_reference(0)); // frame at base layer references it
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| {
            d.rule_id == "frame-header/qm-mlayer-dependency-missing"
                || d.rule_id == "frame-header/qm-tlayer-dependency-missing"
        }),
        "a satisfied QM layer dependency must be silent; report was: {report}"
    );
}

#[test]
fn validator_qm_resend_after_truncated_ras_poison_regrounds_availability() {
    // After a truncated RAS poisons QM availability, a QM OBU re-sending the level
    // re-grounds it (the level is definitively available again), so a referencing frame
    // stays SILENT — the resend lifts the poison.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1)
    data.extend(temporal_delimiter_obu()); // TU2 starts
    data.extend(ras_frame_truncated_before_reset()); // truncated RAS: poisons level 0
    data.extend(temporal_delimiter_obu()); // TU3 starts: QmProtected cleared
    data.extend(qm_reset_obu_chroma()); // re-send level 0 -> re-grounds (poison lifted)
    data.extend(intra_only_frame_with_qm_reference(0)); // references the re-grounded level 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a QM resend after a poison must re-ground availability; report was: {report}"
    );
}

#[test]
fn validator_qm_unconfirmed_switch_reset_does_not_falsely_fire_unavailable() {
    // The symmetric SWITCH case. A SWITCH frame whose core the validator cannot resolve
    // (here it references a sequence header that is not the active one — the same
    // "unresolvable core" class the §5.18.2 SWITCH arm hits when the header is truncated
    // before restricted_prediction_switch, mirror :4281) leaves the reset_qm() effect
    // UNCONFIRMED: the gate bit was never decided. The validator must POISON the
    // availability state (drop later judgments) — NOT clear it to "unavailable" (which
    // would falsely fire §7.3.8.9) and NOT, as pre-fix, silently skip and assert a stale
    // "available" verdict. A later INTRA_ONLY frame (no reset_qm of its own) referencing
    // a level made available before the SWITCH must therefore stay SILENT; this guards
    // the SWITCH arm against the same clear-to-fire regression the RAS arm had.
    let mut data = td_and_seq_header(0, 0, 0); // activate sequence id 0
    // 0x28 = 0b0_01010_00 -> ext=0, type=10 (OBU_SWITCH), tlayer=0.
    const SWITCH_HEADER: u8 = 0x28;
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(frame_obu_direct_seq_ref(SWITCH_HEADER, 5)); // references seq 5 (not active) -> None core -> poison
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "an unconfirmed SWITCH reset must POISON QM availability (not clear-to-fire), so \
         the later reference stays silent; report was: {report}"
    );
}

#[test]
fn validator_qm_unresolvable_clk_reset_clears_unprotected_level() {
    // F1(a): a CLK whose referenced sequence header cannot be resolved in-band still
    // executes its § 5.18.2 reset_qm() — the CLK / OLK reset is decidable from the OBU
    // type and FirstPictureInTU alone (mirror :4106 keyFrame reset, before any
    // sequence-dependent read). A custom level made available + unprotected before the
    // unresolvable CLK must therefore be CLEARED, so a later resolvable frame referencing
    // that level fires §7.3.8.9 qm-level-unavailable. Pre-fix the reset call sat inside the
    // `if let Some(seq_id) = resolved` gate, so the unresolvable CLK skipped reset handling
    // entirely and the stale "available" record suppressed the diagnostic.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base()); // activates seq 0
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    // An unresolvable CLK (references seq id 5, never sent in-band) at FirstPictureInTU:
    // its reset_qm() must still clear the now-unprotected level 0. It does not activate
    // (resolved is None), so seq 0 stays the active header for the later intra frame.
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5));
    data.extend(temporal_delimiter_obu()); // TU3 starts: QmProtected cleared
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "an unresolvable CLK must still execute reset_qm() (decidable from obu_type + \
         FirstPictureInTU), so the later reference fires unavailable; report was: {report}"
    );
}

#[test]
fn validator_qm_unresolvable_ras_reset_poisons_availability() {
    // F1(b): a RAS whose referenced sequence header cannot be resolved in-band cannot be
    // known to have reached its § 5.18.2 reset_qm() call site (mirror :4283 sits past
    // sequence-dependent reads), so the reset is UNCONFIRMED and the validator must POISON
    // the availability state — never silently skip it. A later frame referencing the level
    // therefore stays SILENT (the poison drops the §7.3.8.9 judgment). Pre-fix the reset
    // call sat inside the `if let Some(seq_id) = resolved` gate, so the unresolvable RAS
    // skipped handling entirely and the stale "available" record was asserted as valid.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4, // § 6.4.6: RAS requires long_term_frame_id_bits != 0
        ..FrameCoreSeq::base()
    }); // activates seq 0
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    // An unresolvable RAS (references seq id 5): the reset is unconfirmed -> poison.
    data.extend(frame_obu_direct_seq_ref(RAS_HEADER, 5));
    data.extend(temporal_delimiter_obu()); // TU3 starts: QmProtected cleared
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "an unresolvable RAS reset must POISON availability (not skip), so the later \
         reference stays silent; report was: {report}"
    );
}

/// A RAS frame whose parse passes the § 5.18.2 `reset_qm()` call site (mirror :4283) and
/// then truncates INSIDE the inter reference region's `ref_frame_idx[i]` reads (mirror
/// :4611-4625, well past :4283). `max_mlayer_id != 0` takes the explicit SWITCH
/// `refresh_frame_flags` arm (mirror :4507-4509) so the parse continues into the reference
/// region instead of stopping at the `max_mlayer_id == 0` refresh derivation. A
/// `num_total_refs` of 7 then demands 7 * f(CeilLog2(NumRefFrames)) `ref_frame_idx` reads;
/// the payload supplies none, so the parse hits EOF inside the loop and surfaces
/// `StoppedInsideInterControl` with the pre-reset facts preserved. Because the truncation
/// is past :4283 the RAS reset is provably CONFIRMED.
pub(in crate::validator::tests) fn ras_frame_truncated_inside_ref_frame_idx(
    num_ref_frames: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch f(1)
    fb.f(0, 3); // num_key_ref_frames == 0
    fb.bit(0); // immediate_output_frame f(1) (RAS is not OLK; monotonic -> implicit 0)
    // SWITCH_FRAME forces frame_size_override_flag == 1 (no bit).
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // max_mlayer_id != 0 -> SWITCH refresh arm: refresh_frame_flags f(NumRefFrames).
    fb.f(0, num_ref_frames); // refresh_frame_flags f(NumRefFrames)
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(7, 3); // num_total_refs == 7 -> 7 ref_frame_idx reads demanded, none supplied
    // Payload ends here: the ref_frame_idx[0..7] reads run past the byte-aligned padding,
    // so the inter control region hits EOF AFTER the reset_qm() call site (mirror :4283).
    annex_b_obu(RAS_HEADER, &fb.into_bytes())
}

#[test]
fn validator_qm_ras_truncated_after_reset_point_confirms_reset() {
    // F2: a RAS that reaches reset_qm() (mirror :4283) and then truncates inside the inter
    // reference region (EOF in ref_frame_idx) has PROVABLY executed the reset — the clear
    // is decidable from the preserved partial facts (StoppedInsideInterControl keeps
    // core.inter). The unprotected level made available in a prior TU must be CLEARED, so a
    // later frame referencing it fires §7.3.8.9 unavailable. Pre-fix the confirmation gate
    // required a full core parse (`frame_core_against_referenced_header(...).is_some()`),
    // which dropped this provable reset and poisoned instead (silent).
    let seq = FrameCoreSeq {
        long_term_frame_id_bits: 4,
        explicit_ref_frame_map: true,
        max_mlayer_id: 1, // != 0 so the RAS refresh arm continues into the reference region
        num_ref_frames_minus_1: 7, // NumRefFrames == 8
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq); // activates seq 0
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(ras_frame_truncated_inside_ref_frame_idx(8)); // confirmed reset (past :4283)
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "a RAS truncated AFTER reset_qm() (inside ref_frame_idx) must CONFIRM the reset \
         from the preserved facts, so the later reference fires; report was: {report}"
    );
}

/// A SWITCH frame (`OBU_SWITCH`) with `restricted_prediction_switch == 1` whose parse
/// passes the § 5.18.2 `reset_qm()` call site (mirror :4283, gated on
/// `restricted_prediction_switch`) and then truncates INSIDE `ref_frame_idx[i]` (mirror
/// :4611-4625). Like the RAS case, the truncation preserves the parsed
/// `restricted_prediction_switch` fact on `core.inter`, so the reset is CONFIRMED even
/// though the full core parse did not complete.
pub(in crate::validator::tests) fn switch_frame_rps_truncated_inside_ref_frame_idx(
    num_ref_frames: u32,
) -> Vec<u8> {
    // 0x28 = 0b0_01010_00 -> ext=0, type=10 (OBU_SWITCH), tlayer=0.
    const SWITCH_HEADER: u8 = 0x28;
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // restricted_prediction_switch == 1 -> reset_qm() fires (mirror :4283)
    // SWITCH is not RAS/OLK, so no num_key_ref_frames / ref_long_term_id reads.
    fb.bit(0); // immediate_output_frame f(1) (monotonic -> implicit 0)
    // SWITCH_FRAME forces frame_size_override_flag == 1 (no bit).
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // SWITCH refresh arm: refresh_frame_flags f(NumRefFrames).
    fb.f(0, num_ref_frames); // refresh_frame_flags f(NumRefFrames)
    // SWITCH forces frame_explicit_ref_frame_map == true (no bit, inter.rs :563).
    fb.f(7, 3); // num_total_refs == 7 -> 7 ref_frame_idx reads demanded, none supplied
    // Payload ends here: the ref_frame_idx reads hit EOF AFTER reset_qm() (mirror :4283).
    annex_b_obu(SWITCH_HEADER, &fb.into_bytes())
}

#[test]
fn validator_qm_switch_truncated_after_reset_point_confirms_reset() {
    // F2 (SWITCH symmetric): a restricted SWITCH (restricted_prediction_switch == 1) that
    // reaches reset_qm() and then truncates inside ref_frame_idx has PROVABLY executed the
    // reset — the parsed gate bit is preserved on core.inter. The unprotected level must be
    // CLEARED, so a later reference fires §7.3.8.9 unavailable. Pre-fix the full-parse gate
    // dropped this provable reset and poisoned instead (silent).
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        max_mlayer_id: 1,
        num_ref_frames_minus_1: 7, // NumRefFrames == 8
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq); // activates seq 0
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(switch_frame_rps_truncated_inside_ref_frame_idx(8)); // confirmed reset
    data.extend(temporal_delimiter_obu()); // TU3 starts
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/qm-level-unavailable"
                && d.spec_section.as_deref() == Some("7.3.8.9")
        }),
        "a restricted SWITCH truncated AFTER reset_qm() (inside ref_frame_idx) must CONFIRM \
         the reset from the preserved gate bit, so the later reference fires; report was: {report}"
    );
}

#[test]
fn validator_qm_ras_truncated_before_reset_still_poisons() {
    // F2 guard: a RAS truncated BEFORE the reset_qm() call site (EOF inside the
    // ref_long_term_id loop, mirror :4252) leaves the reset UNCONFIRMED -> poison. This is
    // the existing `ras_frame_truncated_before_reset` shape; re-stated here to prove the F2
    // reached-reset fact does NOT over-confirm an early truncation. A later reference must
    // stay SILENT (poison drops the judgment).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    data.extend(qm_reset_obu_chroma()); // TU1: level 0 available (mlayer_id -1) + protected
    data.extend(temporal_delimiter_obu()); // TU2 starts: QmProtected cleared
    data.extend(ras_frame_truncated_before_reset()); // truncated BEFORE :4283 -> poison
    data.extend(temporal_delimiter_obu()); // TU3 starts: QmProtected cleared
    data.extend(intra_only_frame_with_qm_reference(0)); // references level 0 (no own reset)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/qm-level-unavailable"),
        "a RAS truncated BEFORE reset_qm() must still poison (unconfirmed), so the later \
         reference stays silent; report was: {report}"
    );
}

#[test]
fn validator_preserves_existing_unavailable_sequence_header_check() {
    // The frame-header core wiring must not suppress the activation/HLS checks: a
    // frame referencing an unavailable sequence header still reports it.
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // id 5 is unavailable
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}
