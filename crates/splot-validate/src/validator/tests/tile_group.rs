// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// The `frame_header()` bits of a complete intra CLK first tile group (the syntax
/// AFTER the `tile_group_obu()` `is_first_tile_group` flag) for a
/// [`frame_core_seq_payload`] base sequence. Reaches `IntraHeaderComplete`. The same
/// bit sequence is the `frame_header_copy()` a conformant non-first tile group must
/// carry (AV2 § 5.18.1 / § 6.17.1).
pub(in crate::validator::tests) fn complete_intra_clk_frame_header_body() -> Bits {
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag == 0 (max dims 16x16)
    fb.f(0, 1); // order_hint f(1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // structure + loop-filter cluster (no bits past)
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    fb
}

/// A complete intra CLK FIRST tile group OBU: `is_first_tile_group == 1` then the
/// frame header body (AV2 § 5.19 / § 5.18.1).
pub(in crate::validator::tests) fn clk_first_tile_group() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
    for b in complete_intra_clk_frame_header_body().drain_bits() {
        fb.bit(b);
    }
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A CLK NON-FIRST tile group OBU: `is_first_tile_group == 0`,
/// `frame_header_present_flag == 1`, then `body_bits` as the `frame_header_copy()`
/// region (AV2 § 5.18.1). `body_bits` is appended verbatim so a caller can supply a
/// matching, mismatched, or truncated copy.
pub(in crate::validator::tests) fn clk_non_first_tile_group(body_bits: &[u8]) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0
    fb.bit(1); // frame_header_present_flag == 1 (copy follows)
    for &b in body_bits {
        fb.bit(b);
    }
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// Builds a complete intra CLK FIRST tile group for a 160x16 frame (TileCols == 3,
/// TileRows == 1, NumTiles == 3, TileColsLog2 == 2, TileRowsLog2 == 0 -> tileBits ==
/// 2), reaching `IntraHeaderComplete`, then appends the §5.19 tile_group_obu()
/// structure: `tile_start_and_end_present_flag` (NumTiles > 1) and, when present, the
/// `tg_start` / `tg_end` f(2) reads, followed by `payload` as the byte-aligned
/// tile_group_payload() region (§5.20.1). `tg_range` is `Some((tg_start, tg_end))` for an
/// explicit range (flag == 1) or `None` to infer 0 .. NumTiles - 1 (flag == 0).
/// The complete intra CLK FIRST tile group `frame_header()` bits for the 160x16
/// (TileCols == 3, TileRows == 1, NumTiles == 3, TileColsLog2 == 2, TileRowsLog2 == 0 ->
/// tileBits == 2) multi-tile layout, INCLUDING the leading `tile_group_obu()`
/// `is_first_tile_group == 1` flag ([`clk_frame_until_tile_info`] writes it first),
/// reaching `IntraHeaderComplete`.
pub(in crate::validator::tests) fn multitile_clk_first_body() -> Bits {
    let mut fb = Bits::default();
    clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
    uniform_3x1_tile_info(&mut fb, 2); // TileCols == 3, context_update_tile_id == 2 (< 3)
    quant_seg_tail(&mut fb);
    fb.bit(0); // apply_deblocking_filter[0]
    fb.bit(0); // apply_deblocking_filter[1]
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    fb
}

/// The `frame_header_copy()` bits for the 160x16 multi-tile layout: the
/// [`multitile_clk_first_body`] with the leading `is_first_tile_group` flag stripped, so
/// it is exactly the `frame_header()` a conformant non-first tile group of the same coded
/// frame must carry (AV2 § 5.18.1 / § 6.17.1 — the copy excludes the bits before
/// `frame_header`).
pub(in crate::validator::tests) fn multitile_clk_copy_body() -> Vec<u8> {
    let mut body = multitile_clk_first_body().drain_bits();
    body.remove(0); // drop the leading is_first_tile_group(1) flag
    body
}

/// Appends the §5.19 `tile_group_obu()` structure (the `tile_start_and_end_present_flag`
/// and, for an explicit range, the `tg_start` / `tg_end` f(tileBits == 2) reads) for the
/// 160x16 (NumTiles == 3) layout, then the byte-aligned `tile_group_payload()` region.
/// `tg_range` is `Some((tg_start, tg_end))` for an explicit range (flag == 1) or `None`
/// to infer 0 .. NumTiles - 1 (flag == 0). Returns the finished OBU bytes.
pub(in crate::validator::tests) fn finish_multitile_tile_group(
    mut fb: Bits,
    tg_range: Option<(u32, u32)>,
    payload: &[u8],
) -> Vec<u8> {
    match tg_range {
        Some((start, end)) => {
            fb.bit(1); // tile_start_and_end_present_flag
            fb.f(start, 2); // tg_start f(tileBits == 2)
            fb.f(end, 2); // tg_end f(tileBits == 2)
        }
        None => {
            fb.bit(0); // tile_start_and_end_present_flag == 0 -> range inferred
        }
    }
    let mut bytes = fb.into_bytes();
    bytes.extend_from_slice(payload);
    annex_b_obu(CLK_HEADER, &bytes)
}

pub(in crate::validator::tests) fn clk_first_tile_group_multitile_payload(
    tg_range: Option<(u32, u32)>,
    payload: &[u8],
) -> Vec<u8> {
    finish_multitile_tile_group(multitile_clk_first_body(), tg_range, payload)
}

/// A 160x16 multi-tile CLK NON-FIRST tile group OBU: `is_first_tile_group == 0`,
/// `frame_header_present_flag` per `header_present`, then (when present) the
/// `frame_header_copy()` bits from `copy_body`, then the §5.19 structure for `tg_range`
/// and the `tile_group_payload()` region (AV2 § 5.18.1 / § 5.19). `copy_body` is appended
/// verbatim so a caller can supply a matching or mismatched copy; it is ignored when
/// `header_present` is false.
pub(in crate::validator::tests) fn clk_non_first_tile_group_multitile_payload(
    header_present: bool,
    copy_body: &[u8],
    tg_range: Option<(u32, u32)>,
    payload: &[u8],
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0
    if header_present {
        fb.bit(1); // frame_header_present_flag == 1 (copy region follows)
        for &b in copy_body {
            fb.bit(b);
        }
    } else {
        fb.bit(0); // frame_header_present_flag == 0 (no copy region; structure follows)
    }
    finish_multitile_tile_group(fb, tg_range, payload)
}

/// A conformant §5.20.1 tile_group_payload() region for the full 3-tile (0..=2) range of
/// the 160x16 layout (TileSizeBytes == 1): tile0 `le(1)=0`->tileSize 1 + 1 data byte,
/// tile1 likewise, tile2 (last) takes the remaining byte.
pub(in crate::validator::tests) fn conformant_3tile_payload() -> Vec<u8> {
    vec![0x00, 0xAA, 0x00, 0xBB, 0xCC]
}

/// As [`clk_first_tile_group_multitile_payload`] but with a conformant full-range tile
/// payload, for tests that only exercise the §5.19 tg-range diagnostics.
pub(in crate::validator::tests) fn clk_first_tile_group_multitile(
    tg_range: Option<(u32, u32)>,
) -> Vec<u8> {
    clk_first_tile_group_multitile_payload(tg_range, &conformant_3tile_payload())
}

/// A 160x16 frame-core sequence header (TileCols == 3 layout) plus a temporal delimiter.
pub(in crate::validator::tests) fn td_and_frame_core_seq_160() -> Vec<u8> {
    td_and_frame_core_seq(FrameCoreSeq {
        max_frame_width_minus_1: 159,
        ..FrameCoreSeq::base()
    })
}

#[test]
fn validator_tile_group_range_silent_on_conforming_multitile() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile(Some((0, 2))));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-group/")
                || d.rule_id.starts_with("tile-payload/")),
        "a conforming multi-tile tg range + framing must be silent; report was: {report}"
    );
}

#[test]
fn validator_tile_group_range_silent_on_inferred_range() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile(None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-group/")),
        "an inferred tg range must be silent; report was: {report}"
    );
}

#[test]
fn validator_flags_first_tile_group_tg_start_not_zero() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile(Some((1, 2))));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-group/first-tg-start-not-zero"
                && d.spec_section.as_deref() == Some("6.18")),
        "a first tile group with tg_start != 0 must fire first-tg-start-not-zero; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_tg_end_before_tg_start() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile(Some((2, 1))));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-group/tg-end-before-tg-start"
                && d.spec_section.as_deref() == Some("6.18")),
        "tg_end < tg_start must fire tg-end-before-tg-start; report was: {report}"
    );
}

#[test]
fn validator_flags_tg_end_out_of_range() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile(Some((0, 3))));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-group/tg-end-out-of-range"
                && d.spec_section.as_deref() == Some("6.18")),
        "tg_end > NumTiles - 1 must fire tg-end-out-of-range; report was: {report}"
    );
}

#[test]
fn validator_tile_group_range_silent_on_single_tile() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-group/")),
        "a single-tile frame must not fire any tile-group range diagnostic; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_tile_group_structure_truncation() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        max_frame_width_minus_1: 159,
        max_frame_height_minus_1: 159,
        frame_height_bits_minus_1: 7,
        ..FrameCoreSeq::base()
    });
    let mut fh = Bits::default();
    clk_frame_until_tile_info(&mut fh, 160, 160, (8, 8));
    fh.bit(1); // uniform_tile_spacing_flag
    fh.bit(1); // increment_tile_cols_log2 = 1
    fh.bit(1); // increment_tile_cols_log2 = 1 (reaches maxLog2TileCols)
    fh.bit(1); // increment_tile_rows_log2 = 1
    fh.bit(1); // increment_tile_rows_log2 = 1 (reaches maxLog2TileRows)
    fh.f(0, 4); // context_update_tile_id (< 9)
    fh.f(0, 2); // tile_size_bytes_minus_1
    quant_seg_tail(&mut fh);
    fh.bit(0); // apply_deblocking_filter[0]
    fh.bit(0); // apply_deblocking_filter[1]
    fh.bit(0); // tx_mode_select
    fh.f(0, 2); // reduced_tx_set
    let fh_bits = fh.bit_len();
    let fh_bytes = fh_bits.div_ceil(8);
    let mut fb = fh;
    fb.bit(1); // tile_start_and_end_present_flag == 1
    fb.f(0, 4); // tg_start
    fb.f(8, 4); // tg_end (== NumTiles - 1)
    let mut payload = fb.into_bytes();
    payload.truncate(fh_bytes);
    data.extend(annex_b_obu(CLK_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-group/truncated-structure"
                && d.spec_section.as_deref() == Some("6.2.1")),
        "a payload ending inside the §5.19 structure must fire truncated-structure; \
         report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "the frame header is complete; only the §5.19 structure truncates; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_tile_group_byte_alignment_nonzero_pad() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    for b in complete_intra_clk_frame_header_body().drain_bits() {
        fb.bit(b);
    }
    assert!(
        !fb.bit_len().is_multiple_of(8),
        "test precondition: the intra header body must end unaligned so \
         byte_alignment() reads pad bits"
    );
    fb.bit(1); // a non-zero byte_alignment() zero_bit (§6.2.4 violation)
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-group/byte-alignment-zero-bit"
                && d.spec_section.as_deref() == Some("6.2.4")),
        "a non-zero §5.19 byte_alignment() pad bit must fire byte-alignment-zero-bit; \
         report was: {report}"
    );
}

#[test]
fn validator_silent_on_conformant_tile_payload_framing() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 2)),
        &conformant_3tile_payload(),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-payload/")),
        "a conformant tile-payload framing must be silent; report was: {report}"
    );
}

#[test]
fn validator_flags_tile_payload_size_field_truncated() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(Some((0, 2)), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/size-field-truncated"
                && d.spec_section.as_deref() == Some("5.20.1")),
        "an empty payload region must fire size-field-truncated; report was: {report}"
    );
}

#[test]
fn validator_flags_tile_payload_tile_size_overflows() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 2)),
        &[250, 0, 0, 0],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/tile-size-overflows-payload"
                && d.spec_section.as_deref() == Some("5.20.1")),
        "an overflowing tile size must fire tile-size-overflows-payload; report was: {report}"
    );
}

#[test]
fn validator_tile_payload_anchors_at_offending_tile_offset() {
    let mut data = td_and_frame_core_seq_160();
    let prefix_len = data.len() as u64;
    data.extend(clk_first_tile_group_multitile_payload(Some((0, 2)), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/size-field-truncated"
                && d.byte_offset.is_some_and(|o| o.get() >= prefix_len)),
        "the framing anchor must be inside the tile-group OBU payload region (>= byte \
         {prefix_len}); report was: {report}"
    );
}

#[test]
fn validator_silent_on_single_tile_no_size_field() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    for b in complete_intra_clk_frame_header_body().drain_bits() {
        fb.bit(b);
    }
    let mut payload = fb.into_bytes();
    payload.push(0x00); // one coded-tile byte: SymbolMaxBits starts at 8-15 = -7 >= -14
    data.extend(annex_b_obu(CLK_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-payload/")),
        "a single-tile group with a one-byte coded tile must be framing-silent; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_zero_size_single_tile() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // headerBytes == payload len -> sz == 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/zero-size-tile"),
        "a zero-size non-bridge tile must fire tile-payload/zero-size-tile; \
         report was: {report}"
    );
}

#[test]
fn validator_silent_on_matching_frame_header_copy() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let body = complete_intra_clk_frame_header_body().drain_bits();
    data.extend(clk_non_first_tile_group(&body));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "a bit-identical frame_header_copy() must be silent; report was: {report}"
    );
}

#[test]
fn validator_flags_frame_header_copy_mismatch() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut body = complete_intra_clk_frame_header_body().drain_bits();
    body[2] ^= 1;
    data.extend(clk_non_first_tile_group(&body));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-mismatch"),
        "a non-bit-identical frame_header_copy() must fire copy-bits-mismatch; report was: {report}"
    );
}

#[test]
fn validator_copy_bits_mismatch_anchors_at_offending_bit() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut body = complete_intra_clk_frame_header_body().drain_bits();
    body[2] ^= 1; // flip header_bit[2]
    let non_first_header_offset = (data.len() + 1) as u64;
    data.extend(clk_non_first_tile_group(&body));
    let report = Validator::new(false).validate_bytes(&data);
    let payload_first_byte = non_first_header_offset + 1;
    let precise = report.errors().find(|d| {
        d.rule_id == "frame-header/copy-bits-mismatch"
            && d.byte_offset.map(splot_core::span::ByteOffset::get) == Some(payload_first_byte)
            && d.bit_offset.map(splot_core::span::BitOffset::get) == Some(4)
            && d.message.contains("header_bit[2]")
    });
    assert!(
        precise.is_some(),
        "copy-bits-mismatch must anchor at byte {payload_first_byte} bit 4 (the differing \
         bit), not the OBU header at {non_first_header_offset}, and carry header_bit[2] in \
         the message; report was: {report}"
    );
    assert!(
        !report.errors().any(|d| {
            d.rule_id == "frame-header/copy-bits-mismatch"
                && d.byte_offset.map(splot_core::span::ByteOffset::get)
                    == Some(non_first_header_offset)
        }),
        "the mismatch must not anchor at the OBU header offset; report was: {report}"
    );
}

#[test]
fn validator_flags_frame_header_copy_truncated() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let body = complete_intra_clk_frame_header_body().drain_bits();
    let mut copy = Bits::default();
    copy.bit(0); // is_first_tile_group == 0
    copy.bit(1); // frame_header_present_flag == 1
    for b in body {
        copy.bit(b);
    }
    let mut payload = copy.into_bytes();
    payload.truncate(3); // keep 24 bits (22 matching copy bits) < 26 NumFrameHeaderBits
    data.extend(annex_b_obu(CLK_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-truncated"),
        "a frame_header_copy() shorter than NumFrameHeaderBits must fire \
         copy-bits-truncated; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-mismatch"),
        "a clean truncation must not also fire a mismatch; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_silent_when_header_not_present() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0
    fb.bit(0); // frame_header_present_flag == 0 (no copy region)
    for _ in 0..26 {
        fb.bit(1);
    }
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "frame_header_present_flag == 0 carries no copy region; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_silent_when_first_header_incomplete() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut first = Bits::default();
    first.bit(1); // is_first_tile_group
    first.uvlc(0); // cur_mfh_id == 0
    first.uvlc(0); // seq_header_id_in_frame_header
    first.bit(1); // frame_is_inter == 1 -> INTER_FRAME (parser stops; no NumFrameHeaderBits)
    data.extend(annex_b_obu(RTG_HEADER, &first.into_bytes()));
    let mut second = Bits::default();
    second.bit(0); // is_first_tile_group == 0
    second.bit(1); // frame_header_present_flag == 1
    second.f(0xFFFF, 16); // arbitrary bits
    data.extend(annex_b_obu(RTG_HEADER, &second.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "an incomplete first header must leave the copy region unparsed (no copy \
         diagnostic); report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_dropped_on_ambiguous_boundary() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    data.extend(annex_b_obu(CLK_HEADER, &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "an Ambiguous-boundary tile group must make no copy judgment; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_poisoned_after_ambiguous_boundary() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // records NumFrameHeaderBits for this triple
    data.extend(annex_b_obu(CLK_HEADER, &[]));
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "a record poisoned by an Ambiguous boundary must not pair with a later flag-0 \
         tile group; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_decided_continuation_still_fires_after_no_ambiguity() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-mismatch"),
        "a decided continuation must still fire on a real mismatch; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_re_records_after_ambiguous_then_new_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // TU1 frame: records, then poisoned below
    data.extend(annex_b_obu(CLK_HEADER, &[])); // Ambiguous -> poison TU1 record
    data.extend(temporal_delimiter_obu());
    data.extend(clk_first_tile_group()); // TU2 frame: re-records NumFrameHeaderBits
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched)); // pairs with the TU2 record
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-mismatch"),
        "a re-recorded first header after an Ambiguous boundary must pair with its own \
         coded frame's non-first tile group; report was: {report}"
    );
}

#[test]
fn validator_frame_header_copy_record_resets_across_temporal_units() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // TU1: records NumFrameHeaderBits
    data.extend(temporal_delimiter_obu());
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "a record from a prior temporal unit must not pair across the boundary; \
         report was: {report}"
    );
}

#[test]
fn validator_silent_on_conformant_continuation_tile_group_framing() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 1)),
        &[0x00, 0xAA, 0xBB], // tile0 le(1)=0 -> 1 + 1 data byte; tile1 (last of this group)
    ));
    data.extend(clk_non_first_tile_group_multitile_payload(
        true,
        &multitile_clk_copy_body(),
        Some((2, 2)),
        &[0xCC], // tile2 (last) takes the one remaining byte
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-payload/")
                || d.rule_id.starts_with("frame-header/copy-bits-")),
        "a conformant continuation tile group must be framing- and copy-silent; \
         report was: {report}"
    );
}

#[test]
fn validator_flags_continuation_tile_group_size_field_truncated() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 0)),
        &[0xAA], // first tile group covers tile 0 (last of its group): one data byte
    ));
    data.extend(clk_non_first_tile_group_multitile_payload(
        true,
        &multitile_clk_copy_body(),
        Some((0, 2)), // continuity (tg_start) is not checked for a continuation
        &[],          // EMPTY region: tile0's le(1) size field truncated
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/size-field-truncated"
                && d.spec_section.as_deref() == Some("5.20.1")),
        "a continuation tile group with a truncated size field must fire \
         size-field-truncated; report was: {report}"
    );
}

#[test]
fn validator_flags_continuation_tile_group_tile_size_overflows() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 0)),
        &[0xAA],
    ));
    data.extend(clk_non_first_tile_group_multitile_payload(
        true,
        &multitile_clk_copy_body(),
        Some((0, 2)),
        &[250, 0, 0, 0],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/tile-size-overflows-payload"
                && d.spec_section.as_deref() == Some("5.20.1")),
        "a continuation tile group with an overflowing tile size must fire \
         tile-size-overflows-payload; report was: {report}"
    );
}

#[test]
fn validator_continuation_tile_group_framing_runs_after_copy_mismatch() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 0)),
        &[0xAA],
    ));
    let mut copy = multitile_clk_copy_body();
    copy[2] ^= 1; // flip a header_bit so the copy is no longer bit-identical (§6.17.1)
    data.extend(clk_non_first_tile_group_multitile_payload(
        true,
        &copy,
        Some((0, 2)),
        &[], // EMPTY region: tile0's le(1) size field truncated
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/copy-bits-mismatch"),
        "the mismatching copy must fire copy-bits-mismatch; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/size-field-truncated"),
        "framing must still run after a copy mismatch (the bit position past the copy is \
         exact); report was: {report}"
    );
}

#[test]
fn validator_flags_continuation_tile_group_framing_without_header_copy() {
    let mut data = td_and_frame_core_seq_160();
    data.extend(clk_first_tile_group_multitile_payload(
        Some((0, 0)),
        &[0xAA],
    ));
    data.extend(clk_non_first_tile_group_multitile_payload(
        false, // frame_header_present_flag == 0: no copy region
        &[],
        Some((0, 2)),
        &[], // EMPTY region: tile0's le(1) size field truncated
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "tile-payload/size-field-truncated"
                && d.spec_section.as_deref() == Some("5.20.1")),
        "a frame_header_present == 0 continuation with a defective framing must fire \
         size-field-truncated; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "the frame_header_present == 0 arm carries no copy region; report was: {report}"
    );
}

#[test]
fn validator_continuation_tile_group_framing_silent_without_record() {
    let mut data = td_and_frame_core_seq_160();
    let mut first = Bits::default();
    first.bit(1); // is_first_tile_group
    first.uvlc(0); // cur_mfh_id == 0
    first.uvlc(0); // seq_header_id_in_frame_header
    first.bit(1); // frame_is_inter == 1 -> INTER_FRAME (parser stops)
    data.extend(annex_b_obu(RTG_HEADER, &first.into_bytes()));
    let mut second = Bits::default();
    second.bit(0); // is_first_tile_group == 0
    second.bit(0); // frame_header_present_flag == 0
    data.extend(annex_b_obu(RTG_HEADER, &second.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-payload/")
                || d.rule_id.starts_with("tile-group/")),
        "a continuation with no recorded first-header layout must leave the §5.19 \
         structure unparsed; report was: {report}"
    );
}

/// A conformant grain-free REGULAR_SEF for a [`FrameCoreSeq::base()`] sequence at the
/// default `(xlayer, mlayer, tlayer) == (0, 0, 0)` triple: cur_mfh_id / seq ref,
/// frame_to_show_map_idx f(3), derive_sef_order_hint == 1, then a §5.2.3
/// trailing_one_bit (apply_grain inferred 0, no grain bits). The SEF is its own
/// single-OBU coded frame (§ 7.3.3) and shares the CLK tile groups' triple.
pub(in crate::validator::tests) fn conformant_sef_same_triple() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(0, 3); // frame_to_show_map_idx f(3)
    fb.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    fb.bit(1); // §5.2.3 trailing_one_bit
    annex_b_obu(REGULAR_SEF_HEADER, &fb.into_bytes())
}
