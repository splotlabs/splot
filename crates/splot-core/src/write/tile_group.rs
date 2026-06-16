// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-group **structure / payload / OBU writers** (`ENC-BITSTREAM-WRITER`) — the exact
//! inverses of the § 5.19 `tile_group_obu()` structure parser and the § 5.20.1
//! `tile_group_payload()` framing parser in [`crate::headers::tile_group`], plus the composing
//! `tile_group_obu()` writer that sequences them:
//!
//! - [`write_tile_group_structure`] — `tile_group_obu()` structure (AV2 v1.0.0 § 5.19,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`): the optional
//!   `tile_start_and_end_present_flag` `f(1)` (only `NumTiles > 1`), the `tg_start` / `tg_end`
//!   `f(tileBits)` range (only `NumTiles > 1 && flag`), and the closing `byte_alignment()`
//!   (§ 6.2.4, `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-4`, the zero pad).
//! - [`write_tile_group_payload`] — `tile_group_payload()` per-tile framing (AV2 v1.0.0 § 5.20.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`) on the intra (non-bridge) path: for
//!   each tile in `framing.tiles` order, a non-last tile writes `tile_size_minus_1 = tile_size - 1`
//!   as `le(TileSizeBytes)` (§ 4.11.5) then its coded-tile bytes; the last tile writes its
//!   coded-tile bytes only (no size field — its `tileSize` is the region remainder). The coded-tile
//!   bytes are not modeled by the parser, so they are supplied as a per-tile passthrough
//!   (`tile_data: &[&[u8]]`) and emitted verbatim.
//! - [`write_tile_group_obu`] — the composing **first-tile-group** `tile_group_obu()` writer (AV2
//!   v1.0.0 § 5.19, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`): the inverse of
//!   `parse_tile_group_prefix` + `frame_header()` + `parse_tile_group_structure` +
//!   `parse_tile_group_framing` for `is_first_tile_group == 1`. It emits, in § 5.19 read order, the
//!   `is_first_tile_group = 1` flag, the embedded `frame_header()` (via
//!   [`crate::write::frame_header_core::write_frame_header_core`]), the § 5.19 structure, and the
//!   § 5.20.1 payload framing — drafting the whole OBU payload into a scratch [`BitWriter`] and
//!   committing only on full success. It owns no OBU header / size / trailing bits.
//!
//! Like the other writers this module is additive: it depends on the model/parser read-only and
//! serializes a parsed structure back to bits via [`BitWriter`]. It threads the same
//! [`TileGroupLayout`] the parser receives (carrying `NumTiles` / `TileColsLog2` / `TileRowsLog2`),
//! so it derives `tileBits` and the range bounds identically. The whole structure is validated
//! before any bit is written (reject-before-write): every reject path leaves `writer.bit_len()`
//! unchanged.
//!
//! The parse-context artifacts the structure parser records — `outcome`, `header_bytes`, and
//! `payload_size` (byte-offset bookkeeping derived from `consumed_bits` and the OBU `sz`) — are
//! *not* emitted here: they belong to the surrounding OBU / composer writer (a following slice).
//! This writer emits only the syntax bits, so `read(write(x)) == x` is **semantic** on the syntax
//! fields (`tile_start_and_end_present_flag`, `tg_start`, `tg_end`) and **byte-exact** for the
//! emitted structure region.

use crate::headers::frame::{CoreSeqView, FrameHeaderCore, MfhFrameView};
use crate::headers::tile_group::{
    TileGroupFraming, TileGroupLayout, TileGroupStructure, TileGroupStructureOutcome,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::frame_header_core::write_frame_header_core;

/// Writes the § 5.19 `tile_group_obu()` structure (AV2 v1.0.0 § 5.19,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`), the exact inverse of
/// [`crate::headers::tile_group::parse_tile_group_structure`] on the intra path.
///
/// In read order it emits:
/// 1. `tile_start_and_end_present_flag` `f(1)` — only when `layout.num_tiles > 1` (a single-tile
///    frame never signals the flag; it stays inferred `0`).
/// 2. `tg_start` then `tg_end`, each `f(tileBits)` where `tileBits = layout.tile_bits()` — only when
///    `layout.num_tiles > 1 && structure.tile_start_and_end_present_flag`. When the range is not
///    signaled the parser infers `0 .. num_tiles - 1`, so no bits are written.
/// 3. `byte_alignment()` (§ 6.2.4, the zero pad), via [`BitWriter::align_to_byte`].
///
/// The `outcome` / `header_bytes` / `payload_size` parse-context fields are not emitted (they are
/// the surrounding OBU writer's job); see the module docs.
///
/// # Errors
/// [`WriteError::NonCanonicalTileGroup`] for any model the § 5.19 parser could not have produced
/// (validated up front, before any bit is written):
/// - `"incomplete_structure"` — `structure.outcome` is not [`TileGroupStructureOutcome::Complete`]
///   (a truncated structure has no faithful byte form).
/// - `"degenerate_layout"` — `layout.num_tiles == 0` (no decodable tile range).
/// - `"flag_without_multi_tile"` — `layout.num_tiles == 1` but the flag is set (the parser never
///   reads the flag for a single tile).
/// - `"inferred_range_mismatch"` — the range is not signaled (`num_tiles == 1`, or
///   `num_tiles > 1 && !flag`) but the model does not hold the inferred default
///   `tg_start == 0 && tg_end == num_tiles - 1`.
/// - `"tg_range"` — the range is signaled (`num_tiles > 1 && flag`) but `tg_start` / `tg_end` does
///   not fit `f(tileBits)` (non-reproducible), or `tg_end < tg_start` (a § 6.18 conformance refusal:
///   the § 5.19 parser reads both values unordered).
/// - `"tg_out_of_range"` — the signaled `tg_end` is `>= num_tiles` (it fits `f(tileBits)` — a
///   non-power-of-two grid has spare codes above `NumTiles - 1` — but § 6.18 requires
///   `tg_end < NumTiles`; another conformance refusal the § 5.19 parser does not enforce).
pub fn write_tile_group_structure(
    writer: &mut BitWriter,
    structure: &TileGroupStructure,
    layout: TileGroupLayout,
) -> WriteResult<()> {
    check_tile_group_structure_encodable(structure, layout)?;

    // § 5.19 (mirror :8469-8473): tile_start_and_end_present_flag is read as f(1) only when
    // NumTiles > 1; for a single tile it stays the inferred 0 and no bit is written.
    if layout.num_tiles > 1 {
        writer.write_bit(u8::from(structure.tile_start_and_end_present_flag))?;
    }

    // § 5.19 (mirror :8475-8493): tg_start / tg_end f(tileBits) are written only when the range is
    // signaled (NumTiles > 1 && flag); otherwise the parser infers 0 .. NumTiles - 1.
    if layout.num_tiles > 1 && structure.tile_start_and_end_present_flag {
        let tile_bits = layout.tile_bits();
        writer.write_bits(structure.tg_start, tile_bits)?;
        writer.write_bits(structure.tg_end, tile_bits)?;
    }

    // § 5.19 (mirror :8519): byte_alignment() — the §6.2.4 zero pad (NOT trailing_bits).
    writer.align_to_byte();
    Ok(())
}

/// Validates a [`TileGroupStructure`] / [`TileGroupLayout`] pair is a model the § 5.19
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`) parser could have produced, before any
/// bit is written. See [`write_tile_group_structure`] for the per-label reject set.
fn check_tile_group_structure_encodable(
    structure: &TileGroupStructure,
    layout: TileGroupLayout,
) -> WriteResult<()> {
    // A truncated structure has no faithful byte form: the parser only reports Truncated on an EOF
    // inside the modeled region, so the unreached fields are not real syntax values.
    if structure.outcome != TileGroupStructureOutcome::Complete {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "incomplete_structure",
        });
    }

    // § 5.19 (mirror :8465): NumTiles = TileCols * TileRows; a decodable frame has NumTiles >= 1.
    if layout.num_tiles == 0 {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "degenerate_layout",
        });
    }

    let range_written = layout.num_tiles > 1 && structure.tile_start_and_end_present_flag;

    if !range_written {
        // § 5.19 (mirror :8467-8473): the parser never reads the flag for a single tile, so a set
        // flag there could not have been produced. (Distinct from the inferred-range check so the
        // single-tile set-flag case has its own stable label.)
        if layout.num_tiles == 1 && structure.tile_start_and_end_present_flag {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "flag_without_multi_tile",
            });
        }
        // § 5.19 (mirror :8475-8479): when the range is not signaled the parser infers
        // tg_start = 0, tg_end = NumTiles - 1; a non-default range there is silently dropped on
        // reparse, so it could not round-trip. (saturating_sub guards the NumTiles == 0 path, but
        // that is already rejected above.)
        let inferred_end = layout.num_tiles.saturating_sub(1);
        if structure.tg_start != 0 || structure.tg_end != inferred_end {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "inferred_range_mismatch",
            });
        }
    } else {
        // tg_start / tg_end are f(tileBits) (§ 5.19, mirror :8483-8491): a value that does not fit
        // the tileBits field is non-reproducible (the parser only ever reads tileBits-wide values).
        let tile_bits = layout.tile_bits();
        // tg_end >= tg_start is a §6.18 conformance requirement (mirror
        // docs/spec/av2/1.0.0/06-syntax-structures-semantics.md:6220) the §5.19 parser does NOT
        // enforce (it reads both values unordered). The writer is deliberately stricter here: it
        // refuses to emit an inverted, non-conformant range rather than produce a stream
        // `splot validate` would reject — the one place it is stricter than the reader.
        if structure.tg_end < structure.tg_start {
            return Err(WriteError::NonCanonicalTileGroup { what: "tg_range" });
        }
        // Writability first: tile_bits is in 0..=32 (TileGroupLayout::tile_bits caps at 32); a u64
        // bound handles the tile_bits == 32 case where every u32 fits without overflowing the
        // 1 << tile_bits shift. A value outside f(tileBits) is non-reproducible.
        let bound = 1u64 << tile_bits;
        if u64::from(structure.tg_start) >= bound || u64::from(structure.tg_end) >= bound {
            return Err(WriteError::NonCanonicalTileGroup { what: "tg_range" });
        }
        // Then the §6.18 in-range conformance: tg_end < NumTiles (mirror
        // docs/spec/av2/1.0.0/06-syntax-structures-semantics.md:6218-6223) the §5.19 parser does NOT
        // enforce — a non-power-of-two grid has spare f(tileBits) codes above the last tile index
        // (e.g. 3 tiles -> tileBits == 2, so tg_end == 3 fits the field but exceeds NumTiles-1 == 2).
        // Checked after the fit so a value that does not even fit the field reports tg_range; only a
        // field-valid index that exceeds NumTiles is the out-of-range conformance refusal. tg_start
        // <= tg_end is already checked, so this bounds both ends. Another deliberate
        // stricter-than-the-reader conformance refusal (tile-group/tg-end-out-of-range).
        if structure.tg_end >= layout.num_tiles {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "tg_out_of_range",
            });
        }
    }

    Ok(())
}

/// Writes the § 5.20.1 `tile_group_payload()` per-tile framing (AV2 v1.0.0 § 5.20.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`), the exact inverse of
/// [`crate::headers::tile_group::parse_tile_group_framing`] on the intra (non-bridge) path.
///
/// For each `(i, tile)` in `framing.tiles` order, with `last = (i == framing.tiles.len() - 1)`:
/// - a non-last tile emits `tile_size_minus_1 = tile.tile_size - 1` as `le(tile_size_bytes)`
///   (§ 4.11.5, via [`BitWriter::write_le_u64`]) then its coded-tile bytes `tile_data[i]` (the
///   `tile.tile_size` payload bytes).
/// - the last tile emits its coded-tile bytes `tile_data[i]` only — no size field, because its
///   `tileSize` is the region remainder the parser recomputes (mirror :8555-8557).
///
/// The coded-tile bytes are not modeled by the parser, so they are supplied as a per-tile
/// passthrough (`tile_data`, one slice per tile) and emitted verbatim — byte-exact, no model change.
/// The per-tile parse-context fields `tile_num` / `size_field_offset` / `tile_data_offset` are *not*
/// emitted: the writer lays tiles sequentially from the region start and the parser recomputes
/// `tile_num = tg_start ..= tg_end` and the offsets from its cursor, so `read(write(x))` is
/// **semantic** on `tile_size` (and byte-exact on the coded-tile passthrough) — not on
/// `tile_num` / offsets, which depend on the reparse's `tg_start` and region. This is the § 5.20.1
/// analogue of [`write_tile_group_structure`] ignoring `header_bytes` / `payload_size`. The § 5.19
/// `tile_group_obu()` structure (a separate writer) and any OBU trailing bits are not written here.
///
/// `is_bridge` mirrors the parser's frame-level `IsBridge`; the intra path has `IsBridge == 0`, so
/// `is_bridge == true` is rejected (a bridge tile reads no size field and records `tile_size == 0`,
/// which is unreconstructable from the model).
///
/// # Errors
/// [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary (the § 5.20 framing is
/// byte-granular, written after the § 5.19 `byte_alignment()`), and [`WriteError::NonCanonicalTileGroup`]
/// for any framing the § 5.20.1 parser could not have produced (both validated up front, before any
/// byte is written — a reject leaves `writer.bit_len()` unchanged):
/// - `"framing_defect"` — `framing.defect.is_some()` (a defective framing has no faithful byte form).
/// - `"bridge_unframeable"` — `is_bridge` (a bridge frame's tiles record `tile_size == 0`,
///   unreconstructable; the intra path has `IsBridge == 0`).
/// - `"empty_framing"` — `framing.tiles` is empty (no tile to lay out).
/// - `"tile_data_count"` — `tile_data.len()` disagrees with the tile count.
/// - `"tile_size_bytes_domain"` — `tile_size_bytes` is outside `1..=4` (§ 6.17.7.3).
/// - `"zero_size_tile"` — a tile records `tile_size == 0` (the § 8.2.4 exit floor; a non-last
///   `tile_size - 1` would also underflow).
/// - `"tile_data_len"` — a `tile_data[i].len()` disagrees with the recorded `tile.tile_size`.
/// - `"tile_size_field_overflow"` — a non-last `tile_size - 1` does not fit `le(tile_size_bytes)`
///   (it would not reparse to the same value).
pub fn write_tile_group_payload(
    writer: &mut BitWriter,
    framing: &TileGroupFraming,
    tile_data: &[&[u8]],
    tile_size_bytes: u32,
    is_bridge: bool,
) -> WriteResult<()> {
    // § 5.20 framing is byte-granular (it follows the § 5.19 byte_alignment()): a mid-byte writer
    // would mis-position every byte. Checked before the encodable check so a reject is total.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    check_tile_group_payload_encodable(framing, tile_data, tile_size_bytes, is_bridge)?;

    let last_index = framing.tiles.len() - 1; // non-empty: guaranteed by the encodable check.
    for (i, tile) in framing.tiles.iter().enumerate() {
        if i != last_index {
            // § 5.20.1 (mirror :8565): tile_size_minus_1 le(TileSizeBytes), then the tileSize
            // coded bytes. tile_size >= 1 (the zero_size_tile reject) so the subtraction is safe,
            // and (tile_size - 1) fits le(tile_size_bytes) (the tile_size_field_overflow reject).
            writer.write_le_u64(tile.tile_size - 1, tile_size_bytes)?;
        }
        // § 5.20.1: the coded-tile bytes (a non-last tile's tileSize bytes, or the last tile's
        // region remainder). Length is validated to equal tile.tile_size by the encodable check.
        writer.write_le(tile_data[i])?;
    }
    Ok(())
}

/// Validates a [`TileGroupFraming`] / `tile_data` / `tile_size_bytes` / `is_bridge` tuple is a
/// framing the § 5.20.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`) parser could have
/// produced, before any byte is written. See [`write_tile_group_payload`] for the per-label reject
/// set. (The writer's own byte-alignment guard is checked separately, in the public function.)
fn check_tile_group_payload_encodable(
    framing: &TileGroupFraming,
    tile_data: &[&[u8]],
    tile_size_bytes: u32,
    is_bridge: bool,
) -> WriteResult<()> {
    // A defective framing records a provable §5.20.1 violation (the parser stops at it); its tile
    // bytes do not exist as a faithful byte form, so it cannot round-trip.
    if framing.defect.is_some() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "framing_defect",
        });
    }

    // §5.20.1 (mirror :8559): a bridge frame's tiles read no size field and record tile_size == 0,
    // so the framing carries no length to re-emit. The intra-complete path has IsBridge == 0.
    if is_bridge {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "bridge_unframeable",
        });
    }

    // A conformant framing has at least one tile (tg_start ..= tg_end is non-empty); an empty
    // tiles list has nothing to lay out.
    if framing.tiles.is_empty() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "empty_framing",
        });
    }

    // One coded-tile slice per framed tile: a mismatch cannot describe this framing.
    if tile_data.len() != framing.tiles.len() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "tile_data_count",
        });
    }

    // §6.17.7.3: TileSizeBytes = tile_size_bytes_minus_1 + 1 over an f(2) read, so the value space
    // is 1..=4. A value outside it could not have been a real TileSizeBytes (and bounds the le()
    // shift below: 8 * 4 == 32 < 64).
    if !(1..=4).contains(&tile_size_bytes) {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "tile_size_bytes_domain",
        });
    }

    let last_index = framing.tiles.len() - 1; // non-empty (checked above).
    for (i, tile) in framing.tiles.iter().enumerate() {
        // §8.2.4 floor: a zero-size non-bridge tile can never satisfy the SymbolMaxBits >= -14 exit
        // requirement (the parser reports it as a defect for the last tile and never produces a
        // non-last zero size). It also makes the non-last `tile_size - 1` underflow, so reject it
        // before the subtraction.
        if tile.tile_size == 0 {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "zero_size_tile",
            });
        }

        // The passthrough must carry exactly the tileSize coded bytes the framing records.
        if tile_data[i].len() as u64 != tile.tile_size {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "tile_data_len",
            });
        }

        // §5.20.1 (mirror :8565): a non-last tile's tile_size_minus_1 is written as
        // le(TileSizeBytes); the value must fit that field or it would not reparse to the same
        // tile_size. (write_le_u64 would also reject it, but check up front for reject-before-write.)
        // tile_size_bytes is in 1..=4 here, so 8 * tile_size_bytes <= 32 and the u64 shift is safe.
        if i != last_index && (tile.tile_size - 1) >= (1u64 << (8 * tile_size_bytes)) {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "tile_size_field_overflow",
            });
        }
    }

    Ok(())
}

/// Writes a whole **first-tile-group** `tile_group_obu()` payload (AV2 v1.0.0 § 5.19,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`), the inverse of the sequence
/// [`parse_tile_group_prefix`](crate::headers::tile_group::parse_tile_group_prefix), `frame_header()`,
/// [`parse_tile_group_structure`](crate::headers::tile_group::parse_tile_group_structure), and
/// [`parse_tile_group_framing`](crate::headers::tile_group::parse_tile_group_framing) for
/// `is_first_tile_group == 1`.
///
/// In § 5.19 read order it emits, into a scratch [`BitWriter`]:
/// 1. `is_first_tile_group` `f(1)` = `1` (the first-group form; `frame_header_present_flag` is the
///    inferred `1`, so no bit — mirror :8431-8435).
/// 2. the embedded `frame_header()` via [`write_frame_header_core`] (the intra path; it takes the
///    already-built `core` + `seq` + optional `mfh` + `first_picture_in_tu`).
/// 3. the § 5.19 structure via [`write_tile_group_structure`] — which emits the tile-range bits then
///    the closing `byte_alignment()`, so the scratch is byte-aligned after it.
/// 4. the § 5.20.1 payload framing via [`write_tile_group_payload`] — which then runs byte-aligned
///    (its own guard holds because the structure ended with `byte_alignment()`).
///
/// The § 5.19 `TileGroupLayout` and the § 5.20.1 `TileSizeBytes` are **derived from** `core.tile_info`
/// (§ 5.18.7.2), not taken as independent inputs, so the payload framing stays consistent with the
/// bits `write_frame_header_core` emitted (an independently-supplied layout / `TileSizeBytes` could
/// make a reparse split the tile bytes differently and fail the round-trip).
///
/// The whole composition is drafted into the scratch writer and committed to the caller's `writer`
/// only on full success (`writer.append(&scratch)`), so any sub-writer reject — the frame-header,
/// structure, or payload check — leaves `writer` untouched (`bit_len()` unchanged): reject-before-write
/// for the whole OBU payload. The composer does **not** insert any alignment itself (the structure
/// writer's `byte_alignment()` handles it) and owns no OBU header / size / trailing bits (the OBU
/// writer's job).
///
/// `is_first_tile_group` is the § 5.19 first-group selector. This composer is the first-group form
/// (`is_first_tile_group == 1`); the non-first `frame_header_copy()` continuation
/// (`is_first_tile_group == 0`) is out of scope and rejected before any bit.
///
/// # Errors
/// All validated before any bit (the caller's `writer` is untouched on failure):
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary (an OBU payload
///   begins byte-aligned).
/// - [`WriteError::NonCanonicalTileGroup`] with `what == "continuation_unsupported"` if
///   `is_first_tile_group == false` (the non-first `frame_header_copy()` continuation is a follow-up
///   slice); `"not_tile_group_obu"` if `core.obu_type` is not a tile-group carrier (a SEF / TIP
///   header); `"first_tg_start_not_zero"` if `structure.tg_start != 0` (the § 6.18 first-group rule);
///   or `"missing_tile_info"` if `core.tile_info` is absent (the layout / `TileSizeBytes` cannot be
///   derived).
/// - Any [`WriteError`] a delegated sub-writer raises: the frame-header check
///   ([`WriteError::NonCanonicalFrameHeader`] for a non-intra / non-reproducible `core`), the
///   structure check ([`WriteError::NonCanonicalTileGroup`] for a non-`Complete` / degenerate /
///   out-of-range structure), or the payload check ([`WriteError::NonCanonicalTileGroup`] for a
///   defective / mismatched framing). Every sub-writer's own reject-before-write composes through the
///   scratch buffer, so the caller's `writer` is untouched on any failure. The payload writer's
///   [`WriteError::WriterNotByteAligned`] guard cannot trip here: step 3's `byte_alignment()` leaves
///   the scratch byte-aligned before the payload step.
#[allow(clippy::too_many_arguments)]
pub fn write_tile_group_obu(
    writer: &mut BitWriter,
    core: &FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    first_picture_in_tu: bool,
    structure: &TileGroupStructure,
    framing: &TileGroupFraming,
    tile_data: &[&[u8]],
    is_first_tile_group: bool,
) -> WriteResult<()> {
    // The tile_group_obu() payload begins at a byte boundary (an OBU payload starts byte-aligned);
    // a mid-byte writer would shift the is_first_tile_group bit and every following byte. Checked
    // before any draft, matching the §5.4 / §5.17 OBU-payload writers.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    // § 5.19 (mirror :8431-8435): this composer is the first-group form. A requested non-first form
    // (the frame_header_copy() continuation) is out of scope; reject before any bit so the caller's
    // writer is untouched.
    if !is_first_tile_group {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "continuation_unsupported",
        });
    }
    // § 5.2.1: only a tile-group-carrying OBU type frames a tile_group_obu(). write_frame_header_core
    // also accepts SEF / TIP single-picture headers, which are NOT tile-group carriers; reject them
    // so the composed bytes are a valid tile_group_obu() payload (not a frame-header OBU re-derived
    // under a different type).
    if !core.obu_type.is_tile_group() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "not_tile_group_obu",
        });
    }
    // § 6.18: the first tile group's tg_start must be 0 (tile-group/first-tg-start-not-zero). The
    // generic structure writer cannot know it is emitting the first group, so the composer enforces
    // it (a non-zero first-group tg_start is a conformance violation `splot validate` rejects).
    if structure.tg_start != 0 {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "first_tg_start_not_zero",
        });
    }
    // The § 5.19 layout and § 5.20.1 TileSizeBytes are determined by the frame header's tile_info()
    // (§ 5.18.7.2) — they are NOT independent inputs. Deriving them from `core` keeps the payload
    // framing consistent with the bits write_frame_header_core emits, so a reparse splits the tiles
    // the same way (an independently-supplied layout / TileSizeBytes could desync the round-trip).
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(WriteError::NonCanonicalTileGroup {
            what: "missing_tile_info",
        })?;
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    // TileSizeBytes is present only for a multi-tile frame (§ 5.18.7.2); a single-tile frame's lone
    // (last) tile reads no size field, so the value is unused — default to the minimum 1 when absent.
    let tile_size_bytes = tile_info.tile_size_bytes.unwrap_or(1);

    // Draft the whole OBU payload into a scratch writer; commit to the caller's `writer` only on
    // full success so any sub-writer reject mid-compose never leaves a partial buffer (the caller's
    // `writer.bit_len()` is unchanged). Each sub-writer also validates before its own first bit, so
    // the whole composition is reject-before-write.
    let mut scratch = BitWriter::new();

    // 1. is_first_tile_group f(1) = 1 (frame_header_present_flag is the inferred 1, no bit).
    scratch.write_bit(1)?;

    // 2. frame_header() — the whole intra frame_header_info() (§ 5.18.2 activation prefix + core).
    write_frame_header_core(&mut scratch, core, seq, mfh, first_picture_in_tu)?;

    // 3. § 5.19 structure: the tg-range bits then the closing byte_alignment() (leaves the scratch
    //    byte-aligned).
    write_tile_group_structure(&mut scratch, structure, layout)?;

    // 4. § 5.20.1 payload framing — runs byte-aligned (the structure's byte_alignment() holds the
    //    payload writer's own guard). is_bridge is false (the intra path has IsBridge == 0).
    write_tile_group_payload(&mut scratch, framing, tile_data, tile_size_bytes, false)?;

    writer.append(&scratch)
}

// The unit/rejection tests and the property tests live in sibling files (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*` resolves to the
// writers and private helpers above.
#[cfg(test)]
include!("tile_group_tests.rs");
#[cfg(test)]
include!("tile_group_proptests.rs");
#[cfg(test)]
include!("tile_group_obu_tests.rs");
