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
    RecordedFrameHeaderBits, TileGroupFraming, TileGroupLayout, TileGroupStructure,
    TileGroupStructureOutcome,
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

    if layout.num_tiles > 1 {
        writer.write_flag(structure.tile_start_and_end_present_flag)?;
    }

    if layout.num_tiles > 1 && structure.tile_start_and_end_present_flag {
        let tile_bits = layout.tile_bits();
        writer.write_bits(structure.tg_start, tile_bits)?;
        writer.write_bits(structure.tg_end, tile_bits)?;
    }

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
    if structure.outcome != TileGroupStructureOutcome::Complete {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "incomplete_structure",
        });
    }

    if layout.num_tiles == 0 {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "degenerate_layout",
        });
    }

    let range_written = layout.num_tiles > 1 && structure.tile_start_and_end_present_flag;

    if !range_written {
        if layout.num_tiles == 1 && structure.tile_start_and_end_present_flag {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "flag_without_multi_tile",
            });
        }
        let inferred_end = layout.num_tiles.saturating_sub(1);
        if structure.tg_start != 0 || structure.tg_end != inferred_end {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "inferred_range_mismatch",
            });
        }
    } else {
        let tile_bits = layout.tile_bits();
        if structure.tg_end < structure.tg_start {
            return Err(WriteError::NonCanonicalTileGroup { what: "tg_range" });
        }
        let bound = 1u64 << tile_bits;
        if u64::from(structure.tg_start) >= bound || u64::from(structure.tg_end) >= bound {
            return Err(WriteError::NonCanonicalTileGroup { what: "tg_range" });
        }
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
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    check_tile_group_payload_encodable(framing, tile_data, tile_size_bytes, is_bridge)?;

    let last_index = framing.tiles.len() - 1; // non-empty: guaranteed by the encodable check.
    for (i, tile) in framing.tiles.iter().enumerate() {
        if i != last_index {
            writer.write_le_u64(tile.tile_size - 1, tile_size_bytes)?;
        }
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
    if framing.defect.is_some() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "framing_defect",
        });
    }

    if is_bridge {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "bridge_unframeable",
        });
    }

    if framing.tiles.is_empty() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "empty_framing",
        });
    }

    if tile_data.len() != framing.tiles.len() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "tile_data_count",
        });
    }

    if !(1..=4).contains(&tile_size_bytes) {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "tile_size_bytes_domain",
        });
    }

    let last_index = framing.tiles.len() - 1; // non-empty (checked above).
    for (i, tile) in framing.tiles.iter().enumerate() {
        if tile.tile_size == 0 {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "zero_size_tile",
            });
        }

        if tile_data[i].len() as u64 != tile.tile_size {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "tile_data_len",
            });
        }

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
///   `"framing_range_mismatch"` if `framing.tiles.len()` disagrees with the structure's
///   `tg_end - tg_start + 1` tile count; or `"missing_tile_info"` if `core.tile_info` is absent (the
///   layout / `TileSizeBytes` cannot be derived).
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
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    if !is_first_tile_group {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "continuation_unsupported",
        });
    }
    if !core.obu_type.is_tile_group() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "not_tile_group_obu",
        });
    }
    if structure.tg_start != 0 {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "first_tg_start_not_zero",
        });
    }
    let expected_tiles = u64::from(structure.tg_end).saturating_add(1);
    if framing.tiles.len() as u64 != expected_tiles {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "framing_range_mismatch",
        });
    }
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
    let tile_size_bytes = tile_info.tile_size_bytes.unwrap_or(1);

    let mut scratch = BitWriter::new();

    scratch.write_bit(1)?;

    write_frame_header_core(&mut scratch, core, seq, mfh, first_picture_in_tu)?;

    write_tile_group_structure(&mut scratch, structure, layout)?;

    write_tile_group_payload(&mut scratch, framing, tile_data, tile_size_bytes, false)?;

    writer.append(&scratch)
}

/// Writes a whole **non-first-tile-group** (continuation) `tile_group_obu()` payload (AV2 v1.0.0
/// § 5.19, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`), the inverse of
/// [`parse_tile_group_prefix`](crate::headers::tile_group::parse_tile_group_prefix) on the
/// `is_first_tile_group == 0` path, the `frame_header_copy()` region (§ 5.18.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-1`),
/// [`parse_tile_group_structure`](crate::headers::tile_group::parse_tile_group_structure), and
/// [`parse_tile_group_framing`](crate::headers::tile_group::parse_tile_group_framing).
///
/// In § 5.19 read order it emits, into a scratch [`BitWriter`]:
/// 1. `is_first_tile_group` `f(1)` = `0` (the continuation form).
/// 2. `frame_header_present_flag` `f(1)` (read explicitly for a non-first group — mirror :8431-8435).
/// 3. when `frame_header_present_flag` is set, `frame_header_copy()` — the recorded first header's
///    `NumFrameHeaderBits` `header_bit` `f(1)` values verbatim (§ 5.18.1, a bit-copy, *not* a
///    re-serialized [`FrameHeaderCore`]; the bits come from `recorded`).
/// 4. the § 5.19 structure via [`write_tile_group_structure`] — a continuation's `tg_start` is the
///    running tile offset (`>= 1`, not pinned to `0` like the first group, but never `0` either); it
///    emits the tile-range bits then the closing `byte_alignment()`.
/// 5. the § 5.20.1 payload framing via [`write_tile_group_payload`] (runs byte-aligned).
///
/// `layout` and `tile_size_bytes` are the coded frame's shared values, taken from the **first** tile
/// group's `tile_info()` (§ 5.18.7.2) — every tile group of one coded frame shares them because
/// `frame_header(isFirst==0)` is a bit-copy of the first header. They are inputs here (the continuation
/// has no parseable header of its own to derive them from). `is_bridge` forwards the § 5.20.1
/// `IsBridge` selector to the payload writer.
///
/// The whole composition is drafted into the scratch and committed to the caller's `writer` only on
/// full success (`writer.append(&scratch)`); any sub-writer reject leaves `writer` untouched
/// (reject-before-write). The composer owns no OBU header / size / trailing bits.
///
/// # Errors
/// All validated before any bit (the caller's `writer` is untouched on failure):
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary (an OBU payload
///   begins byte-aligned).
/// - [`WriteError::NonCanonicalTileGroup`] for a constructed model the parser could never produce —
///   `what` names the field: `"frame_header_copy_gate"` if `frame_header_present_flag` disagrees with
///   whether `recorded` copy bits are supplied (the flag is set iff a `frame_header_copy()` follows);
///   `"empty_frame_header_copy"` if the flag is set but `recorded` has `NumFrameHeaderBits == 0` (a
///   real first header is never empty); `"continuation_tg_start_zero"` if `structure.tg_start == 0` (a
///   continuation's running tile offset is `>= 1` per § 6.18, so it never restarts at tile 0);
///   `"framing_range_mismatch"` if `framing.tiles.len()` disagrees with the structure's
///   `tg_end - tg_start + 1` tile count; or `"framing_tile_number"` if a `framing.tiles[k].tile_num`
///   disagrees with the derived `tg_start + k`.
/// - Any [`WriteError`] a delegated sub-writer raises: the structure check
///   ([`WriteError::NonCanonicalTileGroup`] for a non-`Complete` / degenerate / out-of-range
///   structure) or the payload check (defective / mismatched framing). The payload writer's
///   [`WriteError::WriterNotByteAligned`] guard cannot trip here: step 4's `byte_alignment()` leaves
///   the scratch byte-aligned before the payload step.
#[allow(clippy::too_many_arguments)]
pub fn write_tile_group_continuation_obu(
    writer: &mut BitWriter,
    recorded: Option<&RecordedFrameHeaderBits>,
    frame_header_present_flag: bool,
    layout: TileGroupLayout,
    tile_size_bytes: u32,
    structure: &TileGroupStructure,
    framing: &TileGroupFraming,
    tile_data: &[&[u8]],
    is_bridge: bool,
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    if frame_header_present_flag != recorded.is_some() {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "frame_header_copy_gate",
        });
    }
    if let Some(recorded) = recorded
        && recorded.num_frame_header_bits() == 0
    {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "empty_frame_header_copy",
        });
    }
    if structure.tg_start == 0 {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "continuation_tg_start_zero",
        });
    }
    let expected_tiles = u64::from(structure.tg_end)
        .saturating_sub(u64::from(structure.tg_start))
        .saturating_add(1);
    if framing.tiles.len() as u64 != expected_tiles {
        return Err(WriteError::NonCanonicalTileGroup {
            what: "framing_range_mismatch",
        });
    }
    for (k, tile) in framing.tiles.iter().enumerate() {
        let expected_tile_num = structure
            .tg_start
            .saturating_add(u32::try_from(k).unwrap_or(u32::MAX));
        if tile.tile_num != expected_tile_num {
            return Err(WriteError::NonCanonicalTileGroup {
                what: "framing_tile_number",
            });
        }
    }

    let mut scratch = BitWriter::new();

    // 1. is_first_tile_group f(1) = 0 (the continuation form).
    scratch.write_bit(0)?;
    // 2. frame_header_present_flag f(1) (explicit for a non-first group).
    scratch.write_flag(frame_header_present_flag)?;

    // 3. frame_header_copy(): the recorded first header's NumFrameHeaderBits bits verbatim (§ 5.18.1).
    //    The bit count is bounded by the recorded header (already bounded by its source payload), and
    //    `recorded.bit(i)` returns Some for every i < num_frame_header_bits, so the loop never panics.
    if let Some(recorded) = recorded {
        for i in 0..recorded.num_frame_header_bits() {
            let bit = recorded.bit(i).ok_or(WriteError::NonCanonicalTileGroup {
                what: "frame_header_copy_gate",
            })?;
            scratch.write_flag(bit)?;
        }
    }

    write_tile_group_structure(&mut scratch, structure, layout)?;

    write_tile_group_payload(&mut scratch, framing, tile_data, tile_size_bytes, is_bridge)?;

    writer.append(&scratch)
}

#[cfg(test)]
include!("tile_group_tests.rs");
#[cfg(test)]
include!("tile_group_proptests.rs");
#[cfg(test)]
include!("tile_group_obu_tests.rs");
#[cfg(test)]
include!("tile_group_continuation_tests.rs");
