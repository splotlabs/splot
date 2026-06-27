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
    // is_first_tile_group(1) + frame_header() through tile_info() for a 160x16 frame.
    clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
    uniform_3x1_tile_info(&mut fb, 2); // TileCols == 3, context_update_tile_id == 2 (< 3)
    quant_seg_tail(&mut fb);
    // loop-filter cluster (deblocking 2 bits, gdf/cdef disabled) then the §5.18.2 tail.
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
    // §5.19 tile_group_obu() structure (use_bru/bru_inactive == 0 on the intra path):
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
    // byte_alignment() pads to the byte boundary; then append the tile_group_payload().
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
    // An explicit tg range covering the whole 3-tile frame (tg_start == 0, tg_end ==
    // 2 == NumTiles - 1) with a conformant §5.20.1 tile_group_payload() satisfies every
    // locally-decidable §6.18 range AND §5.20.1 framing clause.
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
    // tile_start_and_end_present_flag == 0 infers tg_start == 0, tg_end == NumTiles -
    // 1: always conformant, no §6.18 diagnostic.
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
    // The first tile group of a coded frame has TileNum == 0, so tg_start must be 0
    // (§6.18 mirror :6215-6216). An explicit tg_start == 1 is a conformance defect.
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
    // tg_end < tg_start violates §6.18 (mirror :6220). Use tg_start == 0 (so the
    // first-tg-start rule stays silent) is impossible with tg_end < 0; instead build
    // tg_start == 2, tg_end == 1 — this also trips first-tg-start-not-zero, so assert
    // the tg-end rule specifically fires.
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
    // tg_end == 3 exceeds NumTiles - 1 == 2 (§6.18 mirror :6218-6223): no tile group's
    // tg_end may exceed the last tile index. tg_start == 0 keeps the first-tg rule
    // silent so the out-of-range rule is isolated.
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
    // A single-tile frame (NumTiles == 1) reads no tile_start_and_end_present_flag and
    // infers tg_start == 0, tg_end == 0: no §6.18 diagnostic, and the existing
    // single-tile fixtures stay valid.
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
    // A multi-tile frame whose payload ends after the tile_start_and_end_present_flag
    // but before tg_start/tg_end (§6.2.1 mandatory-syntax truncation). The frame header
    // must stay COMPLETE (so the §5.19 structure — not the frame header — is what
    // truncates), so keep every frame-header bit plus the flag and drop the rest at the
    // bit level.
    // Use a 160x160 frame: TileCols == TileRows == 3, NumTiles == 9, TileColsLog2 ==
    // TileRowsLog2 == 2 -> tileBits == 4. The explicit §5.19 range then needs
    // flag(1) + tg_start(4) + tg_end(4) == 9 bits, which reliably SPILLS past the frame
    // header's final byte, so truncating to the whole-byte frame-header length keeps the
    // frame header complete while the structure read runs off the buffer end.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        max_frame_width_minus_1: 159,
        max_frame_height_minus_1: 159,
        frame_height_bits_minus_1: 7,
        ..FrameCoreSeq::base()
    });
    let mut fh = Bits::default();
    clk_frame_until_tile_info(&mut fh, 160, 160, (8, 8));
    // 3x3 uniform tile_info(): col increments 1,1 then row increments 1,1, then
    // context_update_tile_id f(TileRowsLog2 2 + TileColsLog2 2 == 4) and
    // tile_size_bytes_minus_1 f(2).
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
    // Append the explicit-range structure (flag + tg_start(4) + tg_end(4)), then truncate
    // to the whole-byte frame-header length so the 9-bit structure cannot be read in full.
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
    // The frame header itself completed, so it must NOT fire the frame-header truncation.
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
    // §6.2.4: every byte_alignment() pad bit must be 0. A single-tile intra frame whose
    // §5.19 byte_alignment() contains a non-zero zero_bit is a conformance defect. The
    // complete intra frame header reaches IntraHeaderComplete; append a stray 1 bit so
    // byte_alignment() reads a non-zero pad.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    for b in complete_intra_clk_frame_header_body().drain_bits() {
        fb.bit(b);
    }
    // NumTiles == 1 -> no tile_start_and_end_present_flag; byte_alignment() runs next.
    // A pad bit can only be corrupted when the header ends unaligned — assert the
    // precondition so a future fixture change to an aligned length fails loudly
    // instead of silently skipping the violation (claude review, PR #61).
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
    // A conformant 3-tile §5.20.1 framing (tile0/1 le(1) size fields + last tile) fires
    // no tile-payload defect.
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
    // §5.20.1 / §4.11.5: a non-last tile reads le(TileSizeBytes). With an EMPTY
    // tile_group_payload() region, tile0's le(1) size field cannot be read — the size
    // field is truncated (§6.2.1: the OBU payload must contain every mandatory element).
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
    // §5.20.1 (mirror :8571): a non-last tile whose tileSize + TileSizeBytes exceeds the
    // remaining sz overflows the payload. tile0 codes le(1) == 250 -> tileSize 251, but
    // the region is only 4 bytes, so 251 + 1 > 4.
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
    // The diagnostic anchors at the offending tile's size-field byte offset within the
    // bitstream (not the OBU header). For an empty region the defect sits at the start of
    // the tile_group_payload() region (headerBytes into the OBU payload), so the byte
    // offset is strictly past the OBU header and equals the region base.
    let mut data = td_and_frame_core_seq_160();
    let prefix_len = data.len() as u64;
    data.extend(clk_first_tile_group_multitile_payload(Some((0, 2)), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    // The diagnostic carries a byte offset that lands inside the tile-group OBU's payload
    // (past the container/OBU header that precedes it), so it is at least where the
    // tile-group OBU started — proving the anchor is at the tile, not the OBU header.
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
    // A single-tile intra frame has one (last) tile and reads NO size field (§5.20.1):
    // with at least one coded-tile byte the framing is conformant. (A ZERO-byte tile
    // would fire tile-payload/zero-size-tile — see the companion test.)
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
    // §8.2.2/§8.2.4: a zero-byte non-bridge tile starts SymbolMaxBits at -15, below
    // the exit_symbol() floor of -14 — framing-decidable (codex review, PR #68).
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
    // A completed intra first tile group followed by a non-first tile group whose
    // frame_header_copy() is bit-identical: § 6.17.1 is satisfied, no copy diagnostic.
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
    // The non-first tile group's copy differs from the first header in one bit.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut body = complete_intra_clk_frame_header_body().drain_bits();
    // Flip the immediate_output_frame bit (offset 2 of the body, after the two uvlc(0)
    // single-bit codes) so the copy is no longer bit-identical (§ 6.17.1).
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
    // Regression (codex round-9 F3): the copy-bits-mismatch diagnostic must anchor at the
    // precise byte+bit of the differing header_bit, not at the OBU header. The flipped
    // header_bit is offset 2 of the frame_header body (== mismatch_bit 2). The copy region
    // starts after the two tile_group_obu() prefix bits, so the offending bit is at
    // payload bit (2 + 2) == 4: within the FIRST payload byte, MSB-first bit 4. Pre-fix the
    // diagnostic anchored at the OBU header offset and carried no bit offset.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut body = complete_intra_clk_frame_header_body().drain_bits();
    body[2] ^= 1; // flip header_bit[2]
    // The non-first tile group's OBU header offset (its `obu.offset`): the bytes so far are
    // the TD + sequence-bearing frame-core preamble + the first CLK tile group; the
    // non-first tile group's leb128 length byte precedes its header byte, whose offset is
    // `data.len() + 1` once we know the preamble length. Capture it before appending.
    let non_first_header_offset = (data.len() + 1) as u64;
    data.extend(clk_non_first_tile_group(&body));
    let report = Validator::new(false).validate_bytes(&data);
    // The differing bit is the OBU payload's first byte (header offset + 1), MSB-first
    // bit 4. Assert the precise anchor on the fired mismatch (no panicking unwrap/expect).
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
    // Control: NO copy-bits-mismatch anchors at the OBU header offset (the pre-fix anchor).
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
    // The non-first tile group's copy region is shorter than NumFrameHeaderBits: every
    // available bit matches, but the payload ends before all copied bits (§ 5.18.1 /
    // § 6.2.1). NumFrameHeaderBits is 26 here; build the full matching non-first OBU,
    // then truncate its PAYLOAD to 3 whole bytes (24 bits total = 22 copy bits after the
    // is_first_tile_group + frame_header_present_flag prefix), all of which match — so
    // the payload ends cleanly inside the copy region with no trailing pad to misread as
    // a differing bit.
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
    // A non-first tile group with frame_header_present_flag == 0 carries NO
    // frame_header_copy() (AV2 § 5.19): the bytes after the flag are tile data, so
    // even with a live first-header record nothing may be compared. Guards the
    // check_frame_header_copy early-exit against regressions that would read tile
    // bytes as copy bits (claude-review PR #60 integration gap).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0
    fb.bit(0); // frame_header_present_flag == 0 (no copy region)
    // Arbitrary tile-data bytes that deliberately do NOT match the first header.
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
    // The first tile group's frame header does NOT complete (an INTER first frame stops
    // at UnsupportedUntilFeature), so NumFrameHeaderBits is unknown and the non-first
    // tile group's copy region is left unparsed — no copy diagnostic (Unknown routing).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    // First tile group: an inter RTG that stops after frame-type (coverage stop).
    let mut first = Bits::default();
    first.bit(1); // is_first_tile_group
    first.uvlc(0); // cur_mfh_id == 0
    first.uvlc(0); // seq_header_id_in_frame_header
    first.bit(1); // frame_is_inter == 1 -> INTER_FRAME (parser stops; no NumFrameHeaderBits)
    data.extend(annex_b_obu(RTG_HEADER, &first.into_bytes()));
    // Non-first RTG carrying arbitrary "copy" bits that would mismatch ANY first header.
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
    // The non-first tile group's is_first_tile_group bit is unreadable (empty payload),
    // so the segmenter reports Ambiguous and the copy judgment is dropped. A second
    // non-first tile group whose copy region would MISMATCH stays silent because the
    // ambiguous OBU poisons nothing here — the record is intact but the unreadable OBU
    // makes no copy judgment.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group());
    // A CLK tile group with an EMPTY payload: the is_first_tile_group bit is unreadable,
    // so seg_role_for derives is_first_tile_group: None -> Ambiguous boundary.
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
    // Regression (codex round-8 F1): an Ambiguous tile-group OBU (unreadable
    // is_first_tile_group while a coded frame is open) MAY have started a new coded
    // frame. If the first header's record is left intact, a LATER readable flag-0 tile
    // group pairs against the PREVIOUS frame's record and false-positives a copy
    // mismatch/truncation — yet in the equally-valid interpretation it belongs to the
    // ambiguous new frame, whose first header is unknown. Per the poison-scope rule the
    // Ambiguous boundary must drop the triple's record so subsequent pairings stay silent
    // until the next decided OpensNewUnit re-records.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // records NumFrameHeaderBits for this triple
    // A CLK tile group with an EMPTY payload: is_first_tile_group is unreadable, so the
    // segmenter reports Ambiguous (it may have opened a new coded frame).
    data.extend(annex_b_obu(CLK_HEADER, &[]));
    // A readable flag-0 (is_first_tile_group == 0) non-first tile group whose copy region
    // does NOT match the recorded first header. Pre-fix the still-intact record pairs and
    // fires copy-bits-mismatch; post-fix the poisoned record drops the pairing -> silent.
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
    // Control for the F1 poison: with NO ambiguous OBU in between, a decided continuation
    // (readable flag-0 non-first tile group) still pairs against the intact record and
    // fires copy-bits-mismatch on a real mismatch — the poison is scoped to Ambiguous only.
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
    // Control for the F1 poison: after an Ambiguous boundary poisons the record, a new
    // decided coded frame (its own temporal unit, OpensNewUnit) RE-RECORDS its first
    // header, and a following non-first tile group of that frame pairs correctly — a real
    // mismatch fires, a bit-identical copy is silent.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // TU1 frame: records, then poisoned below
    data.extend(annex_b_obu(CLK_HEADER, &[])); // Ambiguous -> poison TU1 record
    // TU2: a fresh temporal delimiter starts a new coded frame; OpensNewUnit re-records.
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
    // A completed first header in temporal unit 1 must not pair with a non-first tile
    // group in temporal unit 2 (a coded frame does not span temporal units, § 7.3.7):
    // the record is cleared at the temporal-delimiter boundary. The TU2 non-first tile
    // group finds no record and stays silent even with a mismatching copy.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // TU1: records NumFrameHeaderBits
    // TU2: a new temporal delimiter clears the record; the lone non-first tile group has
    // no first header of its own (already a segmenter concern) and no record to pair.
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
    // A split frame: a first tile group covering tiles 0..=1 (records the layout) then a
    // CONTINUATION (is_first == 0, frame_header_present == 1, bit-identical copy) carrying
    // tile 2 with a conformant §5.20.1 framing — its single (last) tile reads no size
    // field, so the framing is silent and the copy matches.
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
    // Codex PR #68 P2: tile_group_framing_checks ran only for FIRST tile groups, so a
    // malformed §5.20.1 payload in a CONTINUATION tile group produced no tile-payload/*
    // diagnostic. Here a continuation with a bit-identical copy frames tiles 0..=2 over an
    // EMPTY payload region: tile0's le(1) size field cannot be read -> size-field-truncated
    // must fire on the continuation OBU (pre-fix this was silent).
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
    // The continuation's tile0 codes le(1) == 250 -> tileSize 251 over a 4-byte region:
    // 251 + 1 > 4, so the §5.20.1 bookkeeping (mirror :8571) overflows. tile-payload/
    // tile-size-overflows-payload must fire on the CONTINUATION OBU (pre-fix silent).
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
    // The after-mismatch decision: a mismatching copy fires copy-bits-mismatch, but the
    // bit position past the copy region is still exact (the copy is exactly
    // NumFrameHeaderBits whether or not its content matches), so the §5.19 structure stays
    // decidable and the framing checks still run. A continuation whose copy differs in one
    // bit AND frames an empty payload must fire BOTH copy-bits-mismatch and
    // size-field-truncated.
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
    // The frame_header_present_flag == 0 arm: the continuation carries NO copy region, so
    // the §5.19 structure starts right after the flag. The recorded first header still
    // supplies the layout, so a defective framing (empty region -> size-field-truncated)
    // must fire even with no copy to compare (pre-fix silent on the whole arm).
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
    // No copy region exists, so no copy diagnostic may be emitted.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "the frame_header_present == 0 arm carries no copy region; report was: {report}"
    );
}

#[test]
fn validator_continuation_tile_group_framing_silent_without_record() {
    // No completed first header was recorded for this triple (the first tile group is an
    // INTER RTG that stops before completion), so a later continuation finds no layout
    // record and its §5.19 structure stays unparsed — no tile-payload/* even over a
    // malformed payload (Unknown routing, as today).
    let mut data = td_and_frame_core_seq_160();
    // First tile group: an inter RTG that stops after frame-type (no NumFrameHeaderBits,
    // no layout recorded).
    let mut first = Bits::default();
    first.bit(1); // is_first_tile_group
    first.uvlc(0); // cur_mfh_id == 0
    first.uvlc(0); // seq_header_id_in_frame_header
    first.bit(1); // frame_is_inter == 1 -> INTER_FRAME (parser stops)
    data.extend(annex_b_obu(RTG_HEADER, &first.into_bytes()));
    // A continuation RTG with frame_header_present == 0 and an empty would-be payload.
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
