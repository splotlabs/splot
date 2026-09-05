// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header-copy recording and continuation-tile comparison.

use super::*;

/// First-header bits and tile layout captured from the same parsed core (§ 5.18.1).
/// Continuation tile groups reuse this layout rather than carrying fresh `tile_info()`.
#[derive(Debug, Clone)]
pub(super) struct FrameHeaderCopyRecord {
    /// The first tile group's recorded `frame_header()` bits (`NumFrameHeaderBits`), for the
    /// bit-for-bit `frame_header_copy()` comparison (§6.17.1).
    pub(super) header_bits: RecordedFrameHeaderBits,
    /// `TileCols` from the first header's `tile_info()` (§5.18.7.2), for `NumTiles`.
    pub(super) tile_cols: u32,
    /// `TileRows` from the first header's `tile_info()`, for `NumTiles`.
    pub(super) tile_rows: u32,
    /// `TileColsLog2` from the first header's `tile_info()`, for `tileBits`.
    pub(super) tile_cols_log2: u8,
    /// `TileRowsLog2` from the first header's `tile_info()`, for `tileBits`.
    pub(super) tile_rows_log2: u8,
    /// `TileSizeBytes` from the first header's `tile_info()` (`None` when the single-tile
    /// layout read no size field), for the §5.20.1 per-tile framing.
    pub(super) tile_size_bytes: Option<u32>,
}

impl FrameHeaderCopyRecord {
    /// The first header's tile layout, as the [`TileGroupLayout`] the §5.19 structure parse
    /// consumes for a continuation tile group.
    pub(super) fn layout(&self) -> TileGroupLayout {
        TileGroupLayout::new(
            self.tile_cols,
            self.tile_rows,
            self.tile_cols_log2,
            self.tile_rows_log2,
        )
    }
}

/// Checks a continuation header copy and its tile structure (§ 5.18.1 / § 6.17.1).
/// A mismatched copy still has a known length, so framing checks continue; a truncated
/// copy leaves no framing position. Missing copy regions proceed directly to § 5.19.
pub(super) fn check_frame_header_copy(
    obu: &ObuEnvelope<'_>,
    recorded: &FrameHeaderCopyRecord,
    report: &mut ValidationReport,
) {
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let Ok(is_first) = reader.read_bit() else {
        return;
    };
    if is_first != 0 {
        return;
    }
    let Ok(frame_header_present) = reader.read_bit() else {
        return;
    };
    let sz = obu.payload.len() as u64;
    if frame_header_present == 0 {
        tile_group_structure_checks(
            obu,
            &mut reader,
            recorded.layout(),
            recorded.tile_size_bytes,
            sz,
            false,
            report,
        );
        return;
    }

    let start_byte = reader.byte_offset();
    let start_bit = u64::from(reader.bit_offset().get());

    let copy_outcome = parse_frame_header_copy(&mut reader, &recorded.header_bits);
    #[allow(clippy::match_same_arms)]
    match copy_outcome {
        FrameHeaderCopyOutcome::Matches => {}
        FrameHeaderCopyOutcome::Mismatch { mismatch_bit } => {
            let absolute_bit = start_bit.saturating_add(mismatch_bit);
            let mismatch_byte = start_byte.saturating_add(absolute_bit / 8);
            let mismatch_bit_in_byte =
                BitOffset::try_new((absolute_bit % 8) as u8).unwrap_or(BitOffset::from_bits(0));
            report.push(
                Diagnostic::error(
                    "frame-header/copy-bits-mismatch",
                    format!(
                        "frame_header_copy() differs from the first tile group's frame header: \
                         header_bit[{mismatch_bit}] is not equal to the bit at offset \
                         {mismatch_bit} of the first frame header (NumFrameHeaderBits == {}); \
                         the differing bit is at byte {mismatch_byte}, bit {mismatch_bit_in_byte} \
                         (MSB-first) of the OBU payload",
                        recorded.header_bits.num_frame_header_bits()
                    ),
                )
                .with_spec_section("6.17.1")
                .with_byte_offset(mismatch_byte)
                .with_bit_offset(mismatch_bit_in_byte),
            );
        }
        FrameHeaderCopyOutcome::Truncated { available_bits } => {
            report.push(frame_header_error(
                "frame-header/copy-bits-truncated",
                "6.2.1",
                obu,
                format!(
                    "the OBU payload ends inside frame_header_copy() after {available_bits} of \
                     {} header_bit f(1) reads; frame_header( isFirst == 0 ) must contain exactly \
                     NumFrameHeaderBits copied bits (§ 5.18.1), read from the § 6.2.1 OBU payload",
                    recorded.header_bits.num_frame_header_bits()
                ),
            ));
        }
        _ => {}
    }

    if matches!(
        copy_outcome,
        FrameHeaderCopyOutcome::Matches | FrameHeaderCopyOutcome::Mismatch { .. }
    ) {
        let structure_start_bit = 2u64.saturating_add(recorded.header_bits.num_frame_header_bits());
        let mut structure_reader = BitReader::new(obu.payload, obu.payload_offset());
        if skip_bits(&mut structure_reader, structure_start_bit) {
            tile_group_structure_checks(
                obu,
                &mut structure_reader,
                recorded.layout(),
                recorded.tile_size_bytes,
                sz,
                false,
                report,
            );
        }
    }
}

/// Advances `reader` by exactly `bits` bits, returning `true` when all `bits` were available
/// and `false` (leaving the reader at end of input) when the payload ran out first.
///
/// Used to re-seek a fresh [`BitReader`] to a known bit position without re-reading field
/// contents (e.g. past a `frame_header_copy()` region of `NumFrameHeaderBits` bits). Reads in
/// up-to-32-bit chunks so a long region does not loop bit-by-bit, respecting
/// [`BitReader::read_bits`]'s 32-bit cap.
pub(super) fn skip_bits(reader: &mut BitReader<'_>, bits: u64) -> bool {
    let mut remaining = bits;
    while remaining > 0 {
        let chunk = remaining.min(32) as u32;
        if reader.read_bits(chunk).is_err() {
            return false;
        }
        remaining -= u64::from(chunk);
    }
    true
}

impl ValidatorContext {
    /// Checks a continuation against its recorded first header. Ambiguous frame boundaries
    /// discard the record until a new first header establishes the layout.
    pub(super) fn observe_frame_header_copy(
        &mut self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
        boundary: Option<FrameBoundary>,
        report: &mut ValidationReport,
    ) {
        let Some(boundary) = boundary else {
            return;
        };
        let key = (
            obu.header.extended_layer_id,
            obu.header.embedded_layer_id,
            obu.header.temporal_layer_id,
        );

        let is_tile_group = obu.header.obu_type.is_tile_group();
        match boundary {
            FrameBoundary::OpensNewUnit => {
                self.frame_header_copy_record.remove(&key);
                if is_tile_group
                    && let Some(recorded) = self.record_first_frame_header(obu, first_picture_in_tu)
                {
                    self.frame_header_copy_record.insert(key, recorded);
                }
            }
            FrameBoundary::ContinuesUnit => {
                if is_tile_group && let Some(recorded) = self.frame_header_copy_record.get(&key) {
                    check_frame_header_copy(obu, recorded, report);
                }
            }
            FrameBoundary::Ambiguous => {
                self.frame_header_copy_record.remove(&key);
            }
        }
    }

    /// Records the bits AND tile layout of a first tile group's frame header when it parses
    /// to completion (AV2 § 5.18.1 `NumFrameHeaderBits` + § 5.18.7.2 tile facts). Returns
    /// `None` (record nothing → Unknown routing) when the active sequence header is
    /// unavailable, the referenced header is not the active one, the core parse did not reach
    /// [`FrameHeaderParseStatus::IntraHeaderComplete`], the bits cannot be re-read, or the
    /// completed header carries no `tile_info()` (an `IntraHeaderComplete` core always parses
    /// `tile_info()`, so the last case is defensive). The captured layout (`NumTiles` /
    /// `tileBits` / `TileSizeBytes`) is what every *continuation* tile group of this coded
    /// frame uses to re-derive its §5.19 structure (§5.18.1: a non-first tile group reads
    /// `frame_header_copy()`, not a fresh `tile_info()`).
    pub(super) fn record_first_frame_header(
        &self,
        obu: &ObuEnvelope<'_>,
        first_picture_in_tu: bool,
    ) -> Option<FrameHeaderCopyRecord> {
        let core = self.frame_core_against_referenced_header(obu, first_picture_in_tu)?;
        if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
            return None;
        }
        let tile_info = core.tile_info.as_ref()?;
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        if reader.read_bit().ok()? == 0 {
            return None;
        }
        let header_bits = RecordedFrameHeaderBits::record(&mut reader, core.consumed_bits).ok()?;
        Some(FrameHeaderCopyRecord {
            header_bits,
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            tile_cols_log2: tile_info.tile_cols_log2,
            tile_rows_log2: tile_info.tile_rows_log2,
            tile_size_bytes: tile_info.tile_size_bytes,
        })
    }
}
