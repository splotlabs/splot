// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-group **structure writer** (`ENC-BITSTREAM-WRITER`) — the exact inverse of the
//! § 5.19 `tile_group_obu()` structure parser in [`crate::headers::tile_group`]:
//!
//! - [`write_tile_group_structure`] — `tile_group_obu()` structure (AV2 v1.0.0 § 5.19,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`): the optional
//!   `tile_start_and_end_present_flag` `f(1)` (only `NumTiles > 1`), the `tg_start` / `tg_end`
//!   `f(tileBits)` range (only `NumTiles > 1 && flag`), and the closing `byte_alignment()`
//!   (§ 6.2.4, `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-4`, the zero pad).
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

use crate::headers::tile_group::{TileGroupLayout, TileGroupStructure, TileGroupStructureOutcome};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

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
///   the § 5.19 parser reads both values unordered, so this is the one reject deliberately stricter
///   than the reader rather than a non-reproducibility).
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
        // tile_bits is in 0..=32 (TileGroupLayout::tile_bits caps at 32); a u64 bound handles the
        // tile_bits == 32 case where every u32 fits without overflowing the 1 << tile_bits shift.
        let bound = 1u64 << tile_bits;
        if u64::from(structure.tg_start) >= bound || u64::from(structure.tg_end) >= bound {
            return Err(WriteError::NonCanonicalTileGroup { what: "tg_range" });
        }
    }

    Ok(())
}

// The unit/rejection tests and the property tests live in sibling files (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*` resolves to the
// writer and private helper above.
#[cfg(test)]
include!("tile_group_tests.rs");
#[cfg(test)]
include!("tile_group_proptests.rs");
