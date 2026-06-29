// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-group range, structure, framing, and tile-info checks.

use super::*;

/// Emits the locally decidable § 6.18 tile-group-range diagnostics for the FIRST tile
/// group of an intra-complete coded frame (AV2 v1.0.0 § 5.19 / § 6.18).
///
/// The § 5.19 structure after `frame_header()` is decidable only when the first tile
/// group's frame header parsed to completion on the intra path
/// ([`FrameHeaderParseStatus::IntraHeaderComplete`] with `frame_is_intra == Some(true)`
/// and a parsed `tile_info()`): then `use_bru == 0` and `bru_inactive == 0` are the
/// § 5.18.2 intra-derived constants (mirror :4127-4129 / :4653), so the `bru_inactive`
/// early-return and the `use_bru` `bru_tile_active` loop are both dead, and
/// [`parse_tile_group_structure`] consumes the structure exactly. `NumTiles` /
/// `TileColsLog2` / `TileRowsLog2` come from the parsed `tile_info()`.
///
/// The locally-decidable § 6.18 clauses for the FIRST tile group are:
///
/// - **tg_start of the first tile group is 0** (mirror :6215-6216: `tg_start` equals
///   `TileNum` at `tile_group_payload`, and `TileNum = 0` for the first tile group of a
///   regular intra frame, mirror :3956);
/// - **tg_end >= tg_start** (mirror :6220);
/// - **tg_end <= NumTiles - 1** (mirror :6218-6223 — `tg_end` is a zero-based tile index,
///   and the last tile group's `tg_end` is `NumTiles - 1`, so no `tg_end` may exceed it).
///
/// Under-reported (needs prior-tile-group state the segmenter would thread): the
/// cross-tile-group continuity (`tg_start == previous tg_end + 1`) and the requirement
/// that the LAST tile group's `tg_end == NumTiles - 1` when the range is split across
/// multiple groups (residual: tile-group-continuity-across-groups). Only the first tile
/// group is checked here, so only the `TileNum == 0` instance of the continuity clause is
/// decided.
pub(super) fn tile_group_range_checks(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    report: &mut ValidationReport,
) {
    if !obu.header.obu_type.is_tile_group() {
        return;
    }

    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let Ok(is_first) = reader.read_bit() else {
        return;
    };
    if is_first == 0 {
        return;
    }
    let input = FrameHeaderParseInput {
        obu_type: obu.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(active_sequence),
        mfh_record,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let Ok(core) = parse_frame_header_core(&mut reader, &input) else {
        return;
    };

    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_is_intra != Some(true)
    {
        return;
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return;
    };

    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    let sz = obu.payload.len() as u64;
    tile_group_structure_checks(
        obu,
        &mut reader,
        layout,
        tile_info.tile_size_bytes,
        sz,
        true,
        report,
    );
}

/// Parses the §5.19 `tile_group_obu()` structure that follows `frame_header()` /
/// `frame_header_copy()` and emits the locally decidable §6.18 tile-group-range and
/// §5.20.1 tile-payload framing diagnostics (AV2 v1.0.0 §5.19 / §6.18 / §5.20.1).
///
/// `reader` must be positioned at the first bit **after** the optional `frame_header()`
/// (first tile group) or `frame_header_copy()` / `frame_header_present_flag == 0` flag
/// (non-first tile group), and constructed at the OBU payload start so
/// [`parse_tile_group_structure`] derives `headerBytes` from `reader.consumed_bits()`.
/// `layout` is the coded frame's tile layout (always the FIRST tile group's `tile_info()`,
/// §5.18.1), `tile_size_bytes_field` is that header's `TileSizeBytes`, and `sz` is the OBU
/// payload size in bytes.
///
/// `is_first` gates the FIRST-tile-group `tg_start == 0` clause (§6.18 mirror :6215-6216):
/// it is `true` for the first tile group (`TileNum == 0`) and `false` for a continuation,
/// whose `tg_start == previous tg_end + 1` needs prior-tile-group state the segmenter does
/// not thread (residual: tile-group-continuity-across-groups). The `tg_end >= tg_start`,
/// `tg_end <= NumTiles - 1`, byte-alignment, truncation, and framing clauses apply to every
/// tile group and run for both.
pub(super) fn tile_group_structure_checks(
    obu: &ObuEnvelope<'_>,
    reader: &mut BitReader<'_>,
    layout: TileGroupLayout,
    tile_size_bytes_field: Option<u32>,
    sz: u64,
    is_first: bool,
    report: &mut ValidationReport,
) {
    let num_tiles = layout.num_tiles;
    let Ok(structure) = parse_tile_group_structure(reader, layout, sz) else {
        report.push(frame_header_error(
            "tile-group/byte-alignment-zero-bit",
            "6.2.4",
            obu,
            "the §5.19 tile_group_obu() byte_alignment() padding contains a non-zero \
             zero_bit (§6.2.4 requires every alignment bit to be 0)"
                .to_owned(),
        ));
        return;
    };

    if structure.outcome == TileGroupStructureOutcome::Truncated {
        report.push(frame_header_error(
            "tile-group/truncated-structure",
            "6.2.1",
            obu,
            "the OBU payload ends inside the §5.19 tile_group_obu() structure \
             (tile_start_and_end_present_flag / tg_start / tg_end / byte_alignment) before \
             it could be read; the §6.2.1 OBU payload must contain every mandatory \
             tile-group syntax element"
                .to_owned(),
        ));
        return;
    }

    if is_first && structure.tile_start_and_end_present_flag && structure.tg_start != 0 {
        report.push(frame_header_error(
            "tile-group/first-tg-start-not-zero",
            "6.18",
            obu,
            format!(
                "the first tile group codes tg_start={}, but §6.18 requires tg_start to \
                 equal TileNum at tile_group_payload, which is 0 for the first tile group \
                 of the coded frame (§5.19 mirror :3956)",
                structure.tg_start
            ),
        ));
    }

    if structure.tg_end < structure.tg_start {
        report.push(frame_header_error(
            "tile-group/tg-end-before-tg-start",
            "6.18",
            obu,
            format!(
                "the tile group codes tg_end={} < tg_start={}, but §6.18 requires tg_end to \
                 be greater than or equal to tg_start",
                structure.tg_end, structure.tg_start
            ),
        ));
    }

    if structure.tile_start_and_end_present_flag
        && num_tiles > 0
        && structure.tg_end > num_tiles - 1
    {
        report.push(frame_header_error(
            "tile-group/tg-end-out-of-range",
            "6.18",
            obu,
            format!(
                "the tile group codes tg_end={}, which exceeds NumTiles-1={} (§6.18: tg_end \
                 is a zero-based tile index and the last tile group's tg_end is NumTiles-1)",
                structure.tg_end,
                num_tiles - 1
            ),
        ));
    }

    if structure.tg_end < structure.tg_start || (num_tiles > 0 && structure.tg_end > num_tiles - 1)
    {
        return;
    }
    tile_group_framing_checks(obu, &structure, tile_size_bytes_field, report);
}

/// Emits the locally decidable § 5.20.1 tile-payload framing diagnostics for a COMPLETE
/// § 5.19 tile-group structure (AV2 v1.0.0 § 5.20.1, mirror :8553-8640;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`).
///
/// The framing slice is decidable from the bytes alone: each non-last, non-bridge tile reads
/// `tile_size_minus_1 le(TileSizeBytes)` (§ 4.11.5: exactly `TileSizeBytes` bytes) and
/// bookkeeps `sz -= tileSize + TileSizeBytes` (mirror :8571); the last tile takes the
/// remaining `sz` (mirror :8557). Two arms are provable framing defects:
///
/// - **`tile-payload/size-field-truncated`** — the `le(TileSizeBytes)` size field of a
///   non-last tile runs past the payload region (§ 4.11.5 / § 6.2.1);
/// - **`tile-payload/tile-size-overflows-payload`** — a non-last tile's
///   `tileSize + TileSizeBytes` exceeds the remaining `sz`, so the mirror :8571 subtraction
///   would go negative.
///
/// - **`tile-payload/zero-size-tile`** — a zero-size non-bridge tile: `init_symbol(0)`
///   starts `SymbolMaxBits` at `8*0-15 = -15` (§ 8.2.2, mirror 08:87), the counter only
///   decreases (08:327), and § 8.2.4 requires `>= -14` at `exit_symbol()` (08:342) —
///   unsatisfiable regardless of content, so it is framing-decidable.
///
/// Named residuals (NOT framing checks; owned by `AV2-5.20-TILE-GROUP-PAYLOAD` / its child
/// rows): the `exit_symbol()` conformance for NONZERO tiles (the exact exit value, the
/// trailing one-bit at `trailingBitPosition`, § 8.2.4) depends on the symbol decoder's
/// consumption during `decode_tile()`. The `IsBridge` / `BruTileActive` arms (mirror
/// :8559 / :8585) are dead on this intra-complete tile-group path (`IsBridge == 0`,
/// `use_bru == 0`).
pub(super) fn tile_group_framing_checks(
    obu: &ObuEnvelope<'_>,
    structure: &splot_core::headers::tile_group::TileGroupStructure,
    tile_size_bytes_field: Option<u32>,
    report: &mut ValidationReport,
) {
    let (Some(header_bytes), Some(payload_size)) = (structure.header_bytes, structure.payload_size)
    else {
        return;
    };

    let num_tiles_in_group = u64::from(structure.tg_end - structure.tg_start) + 1;
    let tile_size_bytes = match tile_size_bytes_field {
        Some(tsb) if (1..=4).contains(&tsb) => tsb,
        None if num_tiles_in_group == 1 => 1,
        _ => return,
    };

    let start = usize::try_from(header_bytes).unwrap_or(usize::MAX);
    let end = usize::try_from(header_bytes.saturating_add(payload_size)).unwrap_or(usize::MAX);
    let Some(region) = obu.payload.get(start..end.min(obu.payload.len())) else {
        return;
    };

    let framing = parse_tile_group_framing(
        region,
        structure.tg_start,
        structure.tg_end,
        tile_size_bytes,
        false,
    );

    let Some(defect) = framing.defect else {
        return;
    };

    let region_base = obu.payload_offset().get().saturating_add(header_bytes);
    let anchor = ByteOffset::new(region_base.saturating_add(defect.size_field_offset()));

    match defect {
        TileFramingDefect::SizeFieldTruncated {
            tile_num,
            available,
            ..
        } => {
            report.push(
                Diagnostic::error(
                    "tile-payload/size-field-truncated",
                    format!(
                        "the §5.20.1 tile_group_payload() size field for TileNum={tile_num} is \
                         truncated: tile_size_minus_1 le(TileSizeBytes={tile_size_bytes}) needs \
                         {tile_size_bytes} bytes but only {available} remain in the payload \
                         region (§4.11.5 reads exactly TileSizeBytes bytes; §6.2.1 the OBU \
                         payload must contain every mandatory tile syntax element)"
                    ),
                )
                .with_spec_section("5.20.1")
                .with_byte_offset(anchor),
            );
        }
        TileFramingDefect::TileSizeOverflowsPayload {
            tile_num,
            tile_size,
            tile_size_bytes: tsb,
            remaining,
            ..
        } => {
            report.push(
                Diagnostic::error(
                    "tile-payload/tile-size-overflows-payload",
                    format!(
                        "the §5.20.1 tile_group_payload() framing for TileNum={tile_num} \
                         overflows the payload region: tileSize={tile_size} + \
                         TileSizeBytes={tsb} exceeds the {remaining} bytes still available, so \
                         the bookkeeping sz -= tileSize + TileSizeBytes (mirror :8571) would go \
                         negative"
                    ),
                )
                .with_spec_section("5.20.1")
                .with_byte_offset(anchor),
            );
        }
        TileFramingDefect::ZeroSizeTile { tile_num, .. } => {
            report.push(
                Diagnostic::error(
                    "tile-payload/zero-size-tile",
                    format!(
                        "the §5.20.1 tile_group_payload() framing gives TileNum={tile_num} a \
                         zero-size coded tile: init_symbol(0) starts SymbolMaxBits at -15 \
                         (§8.2.2, mirror 08:87) and the counter only decreases, so the \
                         §8.2.4 exit_symbol() requirement SymbolMaxBits >= -14 (mirror 08:342) \
                         can never be satisfied"
                    ),
                )
                .with_spec_section("8.2.4")
                .with_byte_offset(anchor),
            );
        }
        _ => {}
    }
}

/// Emits the locally decidable § 6.17.7.2 tile-info diagnostics for a parsed frame
/// `tile_info()` (AV2 v1.0.0 § 6.17.7.2,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2`):
/// `TileCols <= MAX_TILE_COLS`, `TileRows <= MAX_TILE_ROWS`, and
/// `context_update_tile_id < TileCols * TileRows`. `MAX_TILE_COLS` /
/// `MAX_TILE_ROWS` are 64 (AV2 § 3, `docs/spec/av2/1.0.0/03-symbols.md`).
pub(super) fn frame_tile_info_checks(
    tile_info: &TileInfo,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    if tile_info.tile_cols > MAX_TILE_COLS {
        report.push(frame_header_error(
            "frame-header/tile-cols-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "tile_info() derives TileCols {}, which must be less than or equal to \
                 MAX_TILE_COLS ({MAX_TILE_COLS})",
                tile_info.tile_cols
            ),
        ));
    }
    if tile_info.tile_rows > MAX_TILE_ROWS {
        report.push(frame_header_error(
            "frame-header/tile-rows-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "tile_info() derives TileRows {}, which must be less than or equal to \
                 MAX_TILE_ROWS ({MAX_TILE_ROWS})",
                tile_info.tile_rows
            ),
        ));
    }
    let tile_count = u64::from(tile_info.tile_cols) * u64::from(tile_info.tile_rows);
    if u64::from(tile_info.context_update_tile_id) >= tile_count {
        report.push(frame_header_error(
            "frame-header/context-update-tile-id-out-of-range",
            "6.17.7.2",
            obu,
            format!(
                "context_update_tile_id {} must be less than TileCols * TileRows ({} * {})",
                tile_info.context_update_tile_id, tile_info.tile_cols, tile_info.tile_rows
            ),
        ));
    }
}

/// Emits the locally decidable § 6.17.7.8 CCSO-params diagnostics for a parsed frame
/// `ccso_params()` (AV2 v1.0.0 § 6.17.7.8,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-8`, mirror
/// :5819 / :5824):
///
/// - `frame-header/ccso-ext-filter-reserved` (error): `ccso_ext_filter != 7` (mirror
///   :5819). `ccso_ext_filter` is `f(3)` (0..=7), so the reserved value 7 is reachable.
/// - `frame-header/ccso-max-band-out-of-range` (error): `1 << ccso_max_band_log2 <=
///   CCSO_BAND_NUM` (mirror :5824). `ccso_max_band_log2` is `f(2 + ccso_bo_only)`
///   (0..=7), so a value > 6 (`1 << 7 == 128 > CCSO_BAND_NUM == 64`) violates the bound;
///   it is only reachable in the `ccso_bo_only` arm (`f(3)`).
///
/// Both bounds are fully determined by the parsed per-plane fields, so they hold on the
/// intra path independent of reference-frame state. The reference-state CCSO requirements
/// (`ccso_ref_idx < NumTotalRefs`, the `SavedCcso*` / `RefMi*` reuse equalities) are dead
/// on the intra path (`NumTotalRefs == 0`), so they are not modeled here.
pub(super) fn frame_ccso_params_checks(
    ccso: &CcsoParams,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    for (plane, params) in ccso.planes.iter().enumerate() {
        if params.ccso_ext_filter == Some(7) {
            report.push(frame_header_error(
                "frame-header/ccso-ext-filter-reserved",
                "6.17.7.8",
                obu,
                format!(
                    "ccso_ext_filter for plane {plane} is 7, which is the reserved value \
                     §6.17.7.8 forbids"
                ),
            ));
        }
        if let Some(max_band_log2) = params.ccso_max_band_log2 {
            let max_band = 1u32 << u32::from(max_band_log2);
            if max_band > CCSO_BAND_NUM {
                report.push(frame_header_error(
                    "frame-header/ccso-max-band-out-of-range",
                    "6.17.7.8",
                    obu,
                    format!(
                        "ccso_max_band_log2 for plane {plane} is {max_band_log2}, so \
                         1 << ccso_max_band_log2 == {max_band} exceeds CCSO_BAND_NUM \
                         ({CCSO_BAND_NUM})"
                    ),
                ));
            }
        }
    }
}
