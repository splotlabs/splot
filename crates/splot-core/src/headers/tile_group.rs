// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prefix-only AV2 tile-group parsing (AV2 v1.0.0 § 5.19).
//!
//! This reads only the head of `tile_group_obu()` — enough to locate an optional
//! frame header — and stops before any tile payload syntax:
//!
//! ```text
//! tile_group_obu( sz ) {
//!     is_first_tile_group                              f(1)
//!     if ( is_first_tile_group )
//!         frame_header_present_flag = 1
//!     else
//!         frame_header_present_flag                    f(1)
//!     if ( frame_header_present_flag )
//!         frame_header( is_first_tile_group )
//!     ...                                              // tile payload, not parsed
//! }
//! ```
//!
//! When `is_first_tile_group` is `1`, `frame_header(1)` parses the
//! [`FrameHeaderPrefix`]. When it is `0`, `frame_header(0)` is a `frame_header_copy()`
//! (a bit copy of the first header), which this prefix parser does not model — it
//! records that a header is present but does not parse it.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::headers::frame::{FrameHeaderPrefix, parse_frame_header_prefix};
use crate::types::ObuType;

/// A prefix-only parse of `tile_group_obu()` (AV2 v1.0.0 § 5.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileGroupHeaderPrefix {
    /// `is_first_tile_group`.
    pub is_first_tile_group: bool,
    /// `frame_header_present_flag` (inferred `1` when `is_first_tile_group`).
    pub frame_header_present_flag: bool,
    /// The parsed frame-header prefix, present only for the first tile group (a
    /// non-first tile group carries `frame_header_copy()`, which is not parsed here).
    pub frame_header: Option<FrameHeaderPrefix>,
    /// Bits consumed by this prefix parse (not the whole tile group).
    pub consumed_bits: u64,
}

/// Parses the `tile_group_obu()` prefix (AV2 v1.0.0 § 5.19).
///
/// `obu_type` is the tile-group OBU type, and `first_picture_in_tu` is forwarded
/// unchanged to the frame-header prefix parser for `startCVS` derivation (AV2
/// § 5.18.2). Pass `Some(known)` on a stateful path that tracks `FirstPictureInTU`;
/// pass `None` on the stateless dispatch front door. The parser reads
/// `is_first_tile_group`, infers or reads `frame_header_present_flag`, and parses the
/// [`FrameHeaderPrefix`] only for the first tile group. It stops before tile payload
/// syntax.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`] or a
/// descriptor error if the payload ends or is malformed before the prefix fields can
/// be read.
pub fn parse_tile_group_prefix(
    reader: &mut BitReader<'_>,
    obu_type: ObuType,
    first_picture_in_tu: Option<bool>,
) -> Result<TileGroupHeaderPrefix> {
    let start_bits = reader.consumed_bits();

    let is_first_tile_group = reader.read_flag()?;
    let frame_header_present_flag = if is_first_tile_group {
        true
    } else {
        reader.read_flag()?
    };

    let frame_header = if frame_header_present_flag && is_first_tile_group {
        Some(parse_frame_header_prefix(
            reader,
            obu_type,
            first_picture_in_tu,
        )?)
    } else {
        None
    };

    Ok(TileGroupHeaderPrefix {
        is_first_tile_group,
        frame_header_present_flag,
        frame_header,
        consumed_bits: reader.consumed_bits().saturating_sub(start_bits),
    })
}

/// The frame's tile layout supplied to [`parse_tile_group_structure`], derived from the
/// coded frame's first tile group's parsed `tile_info()` (AV2 v1.0.0 § 5.18.7.2): every
/// tile group of one coded frame shares the same `NumTiles` / `TileColsLog2` /
/// `TileRowsLog2`, because `frame_header(isFirst==0)` is a bit-copy of the first header
/// (§ 5.18.1). A non-first tile group therefore takes this layout from the coded frame's
/// first tile group (the validator/inspector hold that paired first header's core).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileGroupLayout {
    /// `NumTiles = TileCols * TileRows` (AV2 § 5.19, mirror :8465).
    pub num_tiles: u32,
    /// `TileColsLog2` (AV2 § 5.18.7.2), used to size `tg_start`/`tg_end`.
    pub tile_cols_log2: u8,
    /// `TileRowsLog2` (AV2 § 5.18.7.2), used to size `tg_start`/`tg_end`.
    pub tile_rows_log2: u8,
}

impl TileGroupLayout {
    /// Builds a layout from the parsed tile counts and log2 sizes. `num_tiles` is
    /// computed as `tile_cols * tile_rows` with a saturating multiply so a degenerate
    /// (out-of-domain) layout cannot overflow; AV2-legal layouts have
    /// `num_tiles <= MAX_TILE_COLS * MAX_TILE_ROWS`.
    #[must_use]
    pub fn new(tile_cols: u32, tile_rows: u32, tile_cols_log2: u8, tile_rows_log2: u8) -> Self {
        Self {
            num_tiles: tile_cols.saturating_mul(tile_rows),
            tile_cols_log2,
            tile_rows_log2,
        }
    }

    /// `tileBits = TileColsLog2 + TileRowsLog2` (AV2 § 5.19, mirror :8483): the width of
    /// the `tg_start` / `tg_end` `f(tileBits)` reads.
    ///
    /// Each log2 is bounded by `tile_log2(1, 64) == 6` for an AV2-legal layout
    /// (`MAX_TILE_COLS == MAX_TILE_ROWS == 64`), so `tileBits <= 12`. The sum is computed
    /// in `u32` and capped at 32 so a hostile/out-of-domain layout cannot request a read
    /// wider than [`BitReader::read_bits`] accepts (it would otherwise return
    /// `BitWidthTooLarge`); the cap only affects inputs already outside the AV2 domain.
    #[must_use]
    pub fn tile_bits(self) -> u32 {
        (u32::from(self.tile_cols_log2) + u32::from(self.tile_rows_log2)).min(32)
    }
}

/// Why [`parse_tile_group_structure`] could not be invoked / consumed because the
/// `use_bru` / `bru_inactive` state is not derivable from the modeled (intra) path.
///
/// On the intra path both `use_bru` and `bru_inactive` are the § 5.18.2 defaults `0`
/// (mirror :4127-4129): they are only reassigned inside the
/// `if ( enable_bru && FrameType == INTER_FRAME && !is_tip_frame() && !IsBridge )` gate
/// (mirror :4653-4669), which an intra frame never enters. So the `bru_inactive`
/// early-return `trailing_bits()` arm (mirror :8453-8463) and the `use_bru`
/// `bru_tile_active` loop (mirror :8495-8517) are both **dead** on the intra path, and
/// the structure is fully decidable. This honest-stop reason exists for callers that
/// cannot establish the intra precondition (e.g. an inter / TIP / bridge frame, or a
/// frame whose header did not parse to completion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BruUndecidable {
    /// The frame is not known to be intra-complete, so `use_bru` / `bru_inactive`
    /// cannot be derived; the BRU arms (mirror :8453-8463 / :8495-8517) own the
    /// remaining inter-path coverage (frame-header-inter-reference-paths).
    NotIntraComplete,
}

/// A parse of the `tile_group_obu()` § 5.19 structure *after* the optional
/// `frame_header()` (AV2 v1.0.0 § 5.19, mirror :8465-8527), on the intra path where
/// `use_bru` / `bru_inactive` are the derived constants `0`.
///
/// This covers `NumTiles`, the optional `tile_start_and_end_present_flag` (read only when
/// `NumTiles > 1`, mirror :8469-8473), the `tg_start` / `tg_end` `f(tileBits)` reads (or
/// their inference to `0 .. NumTiles - 1` when the flag is absent/zero, mirror
/// :8475-8493), the closing `byte_alignment()` (mirror :8519), and the `headerBytes` /
/// remaining-`sz` payload boundary (mirror :8521-8527). The § 5.20 `tile_group_payload()`
/// body is intentionally left unparsed (owned by `AV2-5.20-TILE-GROUP-PAYLOAD`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileGroupStructure {
    /// `tile_start_and_end_present_flag` (inferred `0` when `NumTiles == 1`, mirror
    /// :8467-8473).
    pub tile_start_and_end_present_flag: bool,
    /// `tg_start`: the zero-based index of the first tile in this tile group (mirror
    /// :8477 / :8485). Inferred `0` when the flag is absent or zero.
    pub tg_start: u32,
    /// `tg_end`: the zero-based index of the last tile in this tile group (mirror
    /// :8479 / :8491). Inferred `NumTiles - 1` when the flag is absent or zero.
    pub tg_end: u32,
    /// Where the parse stopped relative to the modeled structure.
    pub outcome: TileGroupStructureOutcome,
    /// `headerBytes = (endBitPos - startBitPos) / 8` over the WHOLE `tile_group_obu()`
    /// header (from the first bit of `tile_group_obu()` through `byte_alignment()`,
    /// mirror :8523), or `None` when the parse stopped before `byte_alignment()`
    /// completed (a truncation). `tile_group_payload(sz)` then runs over the remaining
    /// `sz - headerBytes` bytes.
    pub header_bytes: Option<u64>,
    /// `sz - headerBytes`: the byte length of the unparsed § 5.20 `tile_group_payload()`
    /// region (mirror :8525-8527), or `None` when `header_bytes` is unknown. The payload
    /// bytes themselves are NOT parsed here (named residual: `AV2-5.20-TILE-GROUP-PAYLOAD`).
    pub payload_size: Option<u64>,
}

impl TileGroupStructure {
    /// Builds the AV2 § 5.19 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`,
    /// mirror :8467-8493) structure for a single-tile first tile group (`NumTiles == 1`):
    /// `tile_start_and_end_present_flag` is inferred `0` (the parser never reads it for a
    /// single tile, mirror :8467-8473), `tg_start = 0`, and `tg_end = 0` (`= NumTiles - 1`).
    /// `outcome` is [`TileGroupStructureOutcome::Complete`].
    ///
    /// `header_bytes` / `payload_size` are the parser's byte-accounting of a real
    /// `tile_group_obu()` header (mirror :8523-8527) and are left `None`: the § 5.19
    /// writers ([`write_tile_group_structure`] / [`write_tile_group_obu`]) ignore them —
    /// they recompute the header length and take the payload length from the tile data,
    /// so a reparse recomputes both. The round-trip is therefore semantic on the
    /// `flag` / `tg_start` / `tg_end` syntax fields (the bits a single tile actually
    /// writes), matching the writer's parse-context contract.
    ///
    /// [`write_tile_group_structure`]: crate::write::tile_group::write_tile_group_structure
    /// [`write_tile_group_obu`]: crate::write::tile_group::write_tile_group_obu
    #[must_use]
    pub fn single_tile_first_group() -> Self {
        Self {
            tile_start_and_end_present_flag: false,
            tg_start: 0,
            tg_end: 0,
            outcome: TileGroupStructureOutcome::Complete,
            header_bytes: None,
            payload_size: None,
        }
    }
}

/// How far [`parse_tile_group_structure`] reached through the § 5.19 modeled region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileGroupStructureOutcome {
    /// The whole modeled structure parsed: the tile-group range (`tg_start`/`tg_end`),
    /// `byte_alignment()`, and the `headerBytes` payload boundary are all known. The
    /// § 5.20 payload region (`payload_size` bytes) stays unparsed by design.
    Complete,
    /// The OBU payload ran out **inside** the modeled § 5.19 region (the
    /// `tile_start_and_end_present_flag` / `tg_start` / `tg_end` reads, or the
    /// `byte_alignment()` pad). The fields read before the EOF are preserved on the
    /// returned [`TileGroupStructure`]; the unreached fields keep their inferred /
    /// default values and `header_bytes` / `payload_size` stay `None`. Like the
    /// frame-header truncation precedent, this is a payload-bounds condition surfaced as
    /// a status, not a hard parse error.
    Truncated,
}

impl TileGroupStructureOutcome {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Truncated => "truncated",
        }
    }
}

/// Parses the `tile_group_obu()` § 5.19 structure that follows the optional
/// `frame_header()`, on the intra path (AV2 v1.0.0 § 5.19, mirror :8465-8527).
///
/// `reader` must be the **same** reader the `tile_group_obu()` prefix was read from,
/// positioned at the first bit **after** `frame_header()` (i.e. right after
/// [`parse_tile_group_prefix`] / [`parse_frame_header_copy`] returns) and constructed at
/// the OBU payload start (`startBitPos`). The function reads `headerBytes` from
/// `reader.consumed_bits()` (which already spans `is_first_tile_group`,
/// `frame_header_present_flag`, and `frame_header()`), so it never re-reads the header.
///
/// `layout` is the coded frame's tile layout (from the first tile group's `tile_info()`),
/// and `sz` is the OBU payload size in bytes (§ 5.2.1); `header_bytes` and `payload_size`
/// are derived from `headerBytes` and `sz`.
///
/// # BRU precondition
///
/// This entry is only valid when `bru_inactive == 0` and `use_bru == 0`, which the
/// § 5.18.2 intra path derives as constants (mirror :4127-4129 / :4653): the
/// `bru_inactive` early-return `trailing_bits()` arm (mirror :8453-8463) and the
/// `use_bru` `bru_tile_active` loop (mirror :8495-8517) are dead on the intra path, so
/// neither is read here. A caller that cannot establish the intra precondition must not
/// call this and should record [`BruUndecidable::NotIntraComplete`] instead (the inter
/// BRU arms are owned by `frame-header-inter-reference-paths`).
///
/// # Errors
/// Never returns an error for a payload-bounds (EOF) condition — a truncation inside the
/// modeled region is reported via [`TileGroupStructureOutcome::Truncated`] with the facts
/// read so far preserved (the established EOF-preserves-facts pattern). Returns
/// [`Error::InvalidByteAlignment`] when a `byte_alignment()` pad bit is non-zero
/// (§ 6.2.4: `zero_bit` must be `0`) — a decidable structural defect, not a truncation.
pub fn parse_tile_group_structure(
    reader: &mut BitReader<'_>,
    layout: TileGroupLayout,
    sz: u64,
) -> Result<TileGroupStructure> {
    let num_tiles = layout.num_tiles;

    let mut structure = TileGroupStructure {
        tile_start_and_end_present_flag: false,
        tg_start: 0,
        tg_end: num_tiles.saturating_sub(1),
        outcome: TileGroupStructureOutcome::Complete,
        header_bytes: None,
        payload_size: None,
    };

    if num_tiles > 1 {
        let Ok(flag) = reader.read_bit() else {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        };
        structure.tile_start_and_end_present_flag = flag != 0;
    }

    if num_tiles > 1 && structure.tile_start_and_end_present_flag {
        let tile_bits = layout.tile_bits();
        let Ok(tg_start) = reader.read_bits(tile_bits) else {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        };
        structure.tg_start = tg_start;
        let Ok(tg_end) = reader.read_bits(tile_bits) else {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        };
        structure.tg_end = tg_end;
    }

    match reader.byte_align_zero() {
        Ok(()) => {}
        Err(Error::UnexpectedEof { .. }) => {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        }
        Err(other) => return Err(other),
    }

    let header_bytes = reader.consumed_bits() / 8;
    structure.header_bytes = Some(header_bytes);
    structure.payload_size = Some(sz.saturating_sub(header_bytes));
    Ok(structure)
}

/// The byte framing of one tile inside `tile_group_payload()` (AV2 v1.0.0 § 5.20.1,
/// mirror :8553-8640): where its `tile_size_minus_1 le(TileSizeBytes)` length field (if
/// any) and its `tileSize`-byte coded-tile region sit relative to the start of the
/// `tile_group_payload()` region.
///
/// All offsets are relative to the **first byte of `tile_group_payload()`** (the
/// byte-aligned position right after the § 5.19 `byte_alignment()`, i.e. `headerBytes`
/// into the OBU payload). A caller anchors a diagnostic at the absolute bitstream offset
/// by adding `obu.payload_offset() + headerBytes + size_field_offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileFraming {
    /// `TileNum` (AV2 § 5.20.1, mirror :8553): the zero-based tile index within the frame.
    pub tile_num: u32,
    /// Byte offset of this tile's `tile_size_minus_1 le(TileSizeBytes)` length field within
    /// the `tile_group_payload()` region, or `None` for the last tile and for every tile of
    /// a bridge frame (neither reads a size field, mirror :8559-8571).
    pub size_field_offset: Option<u64>,
    /// Byte offset of this tile's `tileSize`-byte coded-tile region within the
    /// `tile_group_payload()` region (right after the size field, or at the region cursor
    /// for the last/bridge tiles).
    pub tile_data_offset: u64,
    /// `tileSize` (AV2 § 5.20.1, mirror :8557 / :8569): the coded-tile byte length. For the
    /// last tile this is the remaining `sz` (mirror :8557); otherwise it is
    /// `tile_size_minus_1 + 1` (mirror :8569).
    pub tile_size: u64,
}

/// A provable § 5.20.1 tile-framing defect: a point where the per-tile bookkeeping
/// (mirror :8559-8571) cannot be satisfied by the bytes the `tile_group_payload()` region
/// actually contains. Both arms are decidable from the framing alone (no symbol decode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileFramingDefect {
    /// The `tile_size_minus_1 le(TileSizeBytes)` length field of a non-last, non-bridge tile
    /// runs past the end of the `tile_group_payload()` region — the size field itself is
    /// truncated (AV2 § 4.11.5 `le(n)` reads exactly `n` bytes; § 6.2.1 requires the OBU
    /// payload to contain every mandatory syntax element).
    SizeFieldTruncated {
        /// `TileNum` of the tile whose size field is truncated.
        tile_num: u32,
        /// Byte offset of the truncated size field within the `tile_group_payload()` region.
        size_field_offset: u64,
        /// Bytes that were available for the `TileSizeBytes`-byte size field before the
        /// region ended (`< TileSizeBytes`).
        available: u64,
    },
    /// A non-bridge tile's `tileSize` is zero, which can never satisfy the arithmetic
    /// exit requirement: `init_symbol(tileSize)` sets `SymbolMaxBits = 8 * sz - 15`
    /// (§ 8.2.2, mirror `08-parsing-process.md`:87), the counter only ever decreases
    /// during decoding (:327), and § 8.2.4 (:342) requires `SymbolMaxBits >= -14` at
    /// `exit_symbol()` — a zero-size tile starts at `-15`, below the floor, so the
    /// defect is decidable from framing alone. Bridge tiles run no `init_symbol`
    /// (§ 5.20.1 gates it on `!IsBridge`), so they are exempt.
    ZeroSizeTile {
        /// `TileNum` of the zero-size tile.
        tile_num: u32,
        /// Byte offset of the tile's (empty) data region within the payload region.
        tile_data_offset: u64,
    },
    /// A non-last tile's `tileSize + TileSizeBytes` exceeds the remaining `sz`, so the
    /// § 5.20.1 bookkeeping `sz -= tileSize + TileSizeBytes` (mirror :8571) would go
    /// negative: the coded-tile region the size field claims runs past the bytes the
    /// payload region still holds.
    TileSizeOverflowsPayload {
        /// `TileNum` of the overflowing tile.
        tile_num: u32,
        /// Byte offset of this tile's size field within the `tile_group_payload()` region.
        size_field_offset: u64,
        /// The coded `tileSize` (`tile_size_minus_1 + 1`).
        tile_size: u64,
        /// `TileSizeBytes` (the size-field width, included in the bookkeeping subtraction).
        tile_size_bytes: u64,
        /// The `sz` (remaining payload bytes) available when this tile was framed.
        remaining: u64,
    },
}

impl TileFramingDefect {
    /// `TileNum` of the offending tile, for anchoring a diagnostic.
    #[must_use]
    pub const fn tile_num(self) -> u32 {
        match self {
            Self::SizeFieldTruncated { tile_num, .. }
            | Self::TileSizeOverflowsPayload { tile_num, .. }
            | Self::ZeroSizeTile { tile_num, .. } => tile_num,
        }
    }

    /// Byte offset (within the `tile_group_payload()` region) of the offending tile's size
    /// field, for anchoring a diagnostic at the defect site.
    #[must_use]
    pub const fn size_field_offset(self) -> u64 {
        match self {
            Self::SizeFieldTruncated {
                size_field_offset, ..
            }
            | Self::TileSizeOverflowsPayload {
                size_field_offset, ..
            } => size_field_offset,
            Self::ZeroSizeTile {
                tile_data_offset, ..
            } => tile_data_offset,
        }
    }

    /// A stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SizeFieldTruncated { .. } => "size-field-truncated",
            Self::TileSizeOverflowsPayload { .. } => "tile-size-overflows-payload",
            Self::ZeroSizeTile { .. } => "zero-size-tile",
        }
    }
}

/// The result of parsing the § 5.20.1 per-tile framing over a `tile_group_payload()` region
/// (AV2 v1.0.0 § 5.20.1, mirror :8553-8640).
///
/// `tiles` records the framing of every tile that could be framed before a defect (if any);
/// a conformant tile group has one record per tile in `tg_start ..= tg_end` and `defect`
/// is `None`. When a defect is found, `tiles` holds the records up to (but not including)
/// the offending tile and `defect` carries the provable violation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileGroupFraming {
    /// Per-tile framing records, in `TileNum` order from `tg_start`.
    pub tiles: Vec<TileFraming>,
    /// The provable § 5.20.1 framing defect, or `None` for a conformant framing.
    pub defect: Option<TileFramingDefect>,
}

impl TileGroupFraming {
    /// Builds the AV2 § 5.20.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`,
    /// mirror :8557) framing for a single-tile tile group (the first tile group,
    /// `TileNum 0`): one tile whose `tileSize` coded region spans the whole
    /// `tile_group_payload()` from offset 0, with **no** `tile_size_minus_1` field (the
    /// only tile is the last tile). This is the exact encoder-side inverse of the
    /// parser: the result equals `parse_tile_group_framing(payload, 0, 0, _, false)`
    /// for a `payload` of `tile_size` coded bytes, so a write (via
    /// [`write_tile_group_payload`]) then reparse round-trips value-equal.
    ///
    /// `tile_size` is the coded-tile byte length; callers pass real § 8.2 coded bytes
    /// (`>= 1`). To preserve the parser-inverse contract for the degenerate input, a
    /// `tile_size == 0` returns the same defective framing the parser would — the
    /// § 8.2.2 [`TileFramingDefect::ZeroSizeTile`] (which [`write_tile_group_payload`]
    /// also rejects) — rather than a `defect: None` framing that falsely reads as
    /// conformant.
    ///
    /// [`write_tile_group_payload`]: crate::write::tile_group::write_tile_group_payload
    #[must_use]
    pub fn single_tile(tile_size: u64) -> Self {
        let defect = (tile_size == 0).then_some(TileFramingDefect::ZeroSizeTile {
            tile_num: 0,
            tile_data_offset: 0,
        });
        Self {
            tiles: vec![TileFraming {
                tile_num: 0,
                size_field_offset: None,
                tile_data_offset: 0,
                tile_size,
            }],
            defect,
        }
    }
}

/// Parses the § 5.20.1 per-tile framing of a `tile_group_payload()` region (AV2 v1.0.0
/// § 5.20.1, mirror :8553-8640).
///
/// `payload` is the `tile_group_payload()` region — the `payload_size` bytes after the
/// § 5.19 `byte_alignment()` (the structure's `payload_size`). `tg_start` / `tg_end` are the
/// inclusive tile range (§ 5.19), `tile_size_bytes` is `TileSizeBytes` from the parsed
/// `tile_info()` (§ 6.17.7.3; `1 ..= 4`), and `is_bridge` is the frame-level `IsBridge`.
///
/// The function walks the loop verbatim: each non-last, non-bridge tile reads
/// `tile_size_minus_1 le(TileSizeBytes)` (mirror :8565), sets `tileSize = +1` (mirror :8569),
/// and bookkeeps `sz -= tileSize + TileSizeBytes` (mirror :8571); the last tile takes the
/// remaining `sz` (mirror :8557); a bridge frame's tiles read no size field and consume no
/// bookkeeping (the `else if ( !IsBridge )` arm is skipped, mirror :8559). It records each
/// tile's framing and stops at the first provable defect.
///
/// # § 8.2 residual (checkable-without-decoding)
///
/// `init_symbol(tileSize)` (mirror :8607) reads `f(Min(tileSize * 8, 15))` (§ 8.2.2) and
/// sets `SymbolMaxBits = 8 * sz - 15` (08:87). The counter only ever decreases during
/// decoding (08:327), and § 8.2.4 requires `SymbolMaxBits >= -14` at `exit_symbol()`
/// (08:342) — so a zero-size non-bridge tile starts at `-15`, below the floor, and can
/// never satisfy the exit requirement regardless of content: that violation IS decidable
/// from framing alone and is reported here as [`TileFramingDefect::ZeroSizeTile`]
/// (bridge tiles run no `init_symbol` and are exempt). The remaining `exit_symbol()`
/// conformance for nonzero tiles (the exact `SymbolMaxBits` at exit; the trailing
/// one-bit at `trailingBitPosition`) depends on the symbol decoder's consumption during
/// `decode_tile()` and stays a named residual of `AV2-5.20-TILE-GROUP-PAYLOAD`.
///
/// # `IsBridge` / BRU residual
///
/// A bridge frame's tiles read no size field; the validator path that reaches this only does
/// so for tile-group OBU types on the intra-complete path (where `IsBridge == 0` and
/// `use_bru == 0`), so the bridge and `BruTileActive` arms (mirror :8585) are honest-stop
/// residuals there. `is_bridge` is still honored here so the parser models the loop exactly.
///
/// This never errors and never panics; an undecidable / defective region is reported via
/// [`TileGroupFraming::defect`].
#[must_use]
pub fn parse_tile_group_framing(
    payload: &[u8],
    tg_start: u32,
    tg_end: u32,
    tile_size_bytes: u32,
    is_bridge: bool,
) -> TileGroupFraming {
    let region_len = payload.len() as u64;
    let tsb = u64::from(tile_size_bytes.clamp(1, 4));
    let mut tiles = Vec::new();
    tiles
        .try_reserve_exact(usize::try_from(tg_end.saturating_sub(tg_start)).unwrap_or(0) + 1)
        .ok();

    let max_tiles = crate::tile::MAX_TILE_COLS * crate::tile::MAX_TILE_ROWS;
    let tg_end = tg_end.min(tg_start.saturating_add(max_tiles - 1));

    let mut pos = 0u64;
    let mut sz = region_len;

    if tg_end < tg_start {
        return TileGroupFraming {
            tiles,
            defect: None,
        };
    }

    for tile_num in tg_start..=tg_end {
        let last_tile = tile_num == tg_end;

        if last_tile {
            tiles.push(TileFraming {
                tile_num,
                size_field_offset: None,
                tile_data_offset: pos,
                tile_size: sz,
            });
            if sz == 0 && !is_bridge {
                return TileGroupFraming {
                    tiles,
                    defect: Some(TileFramingDefect::ZeroSizeTile {
                        tile_num,
                        tile_data_offset: pos,
                    }),
                };
            }
            break;
        }

        if is_bridge {
            tiles.push(TileFraming {
                tile_num,
                size_field_offset: None,
                tile_data_offset: pos,
                tile_size: 0,
            });
            continue;
        }

        let size_field_offset = pos;
        if pos.saturating_add(tsb) > region_len {
            return TileGroupFraming {
                tiles,
                defect: Some(TileFramingDefect::SizeFieldTruncated {
                    tile_num,
                    size_field_offset,
                    available: region_len.saturating_sub(pos),
                }),
            };
        }

        let mut tile_size_minus_1 = 0u64;
        for i in 0..tsb {
            let byte = payload[(pos + i) as usize];
            tile_size_minus_1 |= u64::from(byte) << (i * 8);
        }
        let tile_size = tile_size_minus_1 + 1; // §5.20.1 (:8569): tileSize = +1.

        let claimed = tile_size.saturating_add(tsb);
        if claimed > sz {
            return TileGroupFraming {
                tiles,
                defect: Some(TileFramingDefect::TileSizeOverflowsPayload {
                    tile_num,
                    size_field_offset,
                    tile_size,
                    tile_size_bytes: tsb,
                    remaining: sz,
                }),
            };
        }

        tiles.push(TileFraming {
            tile_num,
            size_field_offset: Some(size_field_offset),
            tile_data_offset: pos + tsb,
            tile_size,
        });
        pos += claimed;
        sz -= claimed;
    }

    TileGroupFraming {
        tiles,
        defect: None,
    }
}

/// `NumFrameHeaderBits` plus the exact bits of a completed first frame header, recorded
/// so a non-first tile group's `frame_header_copy()` can be checked bit-for-bit against
/// it (AV2 v1.0.0 § 5.18.1, mirror :3924 / :3973-3981; § 6.17.1).
///
/// `frame_header(isFirst=1)` records `NumFrameHeaderBits = get_position() - startBitPos`
/// over `frame_header_info()` (mirror :3920-3924). `frame_header(isFirst=0)` is
/// `frame_header_copy()` — exactly `NumFrameHeaderBits` raw `header_bit` `f(1)` reads
/// (mirror :3973-3981). The bits start at the **first bit of `frame_header()`** — *not*
/// the `tile_group_obu()` `is_first_tile_group` flag before it (§ 6.17.1 mirror :4303-4305:
/// "the duplicate copies have a different bit alignment within bytes"). So the recorded
/// region begins right after that flag, where `frame_header_info()` does, and spans
/// `NumFrameHeaderBits`.
///
/// The bit count is bounded by the OBU payload (already bounded), and the bits are stored
/// MSB-first packed into bytes so the comparison reads no further than the recorded length.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordedFrameHeaderBits {
    /// `NumFrameHeaderBits`: the bit length of the recorded first `frame_header()`.
    num_frame_header_bits: u64,
    /// The recorded bits, MSB-first within each byte; only the first
    /// `num_frame_header_bits` bits are meaningful (a trailing partial byte is zero-padded).
    bits: Vec<u8>,
}

impl RecordedFrameHeaderBits {
    /// Records `num_frame_header_bits` bits starting at `reader`'s current position.
    ///
    /// `reader` is positioned at the **first bit of `frame_header()`** (after the
    /// `tile_group_obu()` `is_first_tile_group` flag for a tile-group OBU). The reader is
    /// left advanced by `num_frame_header_bits` bits on success. On EOF the partial result
    /// is discarded and the error is returned (the caller only records a *completed* first
    /// header, so this path is not expected, but it is handled without panicking).
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if fewer than
    /// `num_frame_header_bits` bits remain. The remaining-bits check runs **before** the
    /// backing buffer is allocated, so a caller-supplied count larger than the reader's
    /// payload returns the structured error rather than attempting a `ceil(n/8)`-byte
    /// allocation (which would abort the process for a huge count — a no-panic violation).
    pub fn record(reader: &mut BitReader<'_>, num_frame_header_bits: u64) -> Result<Self> {
        if reader.remaining_bits() < num_frame_header_bits {
            let needed_bits = num_frame_header_bits.saturating_sub(reader.remaining_bits());
            return Err(crate::error::Error::UnexpectedEof {
                offset: reader.byte_offset(),
                needed: usize::try_from(needed_bits.div_ceil(8)).unwrap_or(usize::MAX),
            });
        }
        let byte_len = num_frame_header_bits.div_ceil(8);
        let byte_len = usize::try_from(byte_len).unwrap_or(usize::MAX);
        let mut bits = vec![0u8; byte_len];
        for i in 0..num_frame_header_bits {
            let bit = reader.read_bit()?;
            if bit != 0 {
                let byte = (i / 8) as usize;
                let shift = 7 - (i % 8) as u32;
                bits[byte] |= 1u8 << shift;
            }
        }
        Ok(Self {
            num_frame_header_bits,
            bits,
        })
    }

    /// `NumFrameHeaderBits`: the recorded first header's exact bit length.
    #[must_use]
    pub const fn num_frame_header_bits(&self) -> u64 {
        self.num_frame_header_bits
    }

    /// The recorded bit at offset `index` (MSB-first), or `None` when `index` is at or
    /// beyond [`Self::num_frame_header_bits`]. Used by the § 6.17.1 copy check and by the
    /// non-first tile-group writer to re-emit `frame_header_copy()` verbatim.
    #[must_use]
    pub fn bit(&self, index: u64) -> Option<bool> {
        if index >= self.num_frame_header_bits {
            return None;
        }
        let byte = (index / 8) as usize;
        let shift = 7 - (index % 8) as u32;
        self.bits.get(byte).map(|b| (b >> shift) & 1 != 0)
    }
}

/// The outcome of parsing a non-first tile group's `frame_header_copy()` region against a
/// recorded first header (AV2 v1.0.0 § 5.18.1 mirror :3973-3981; § 6.17.1 mirror :4296-4300).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderCopyOutcome {
    /// All `NumFrameHeaderBits` copy bits were present and bit-identical to the first
    /// header (§ 6.17.1: `header_bit[ i ]` is equal to the value of the bit at offset `i`).
    Matches,
    /// All `NumFrameHeaderBits` copy bits were present, but `header_bit[ mismatch_bit ]`
    /// differs from the first header's bit at that offset — a § 6.17.1 conformance defect.
    /// `mismatch_bit` is the **first** differing bit offset (zero-based from the start of
    /// the copy region), so the diagnostic can anchor precisely.
    Mismatch {
        /// The first bit offset (zero-based) at which the copy differs from the first header.
        mismatch_bit: u64,
    },
    /// The payload ended before `NumFrameHeaderBits` copy bits could be read
    /// (`available_bits < NumFrameHeaderBits`) — a § 5.18.1 / § 6.2.1 truncation. The copy
    /// bits read so far all matched (a mismatch within the available prefix is reported as
    /// [`Self::Mismatch`] instead, since a differing bit is decidable even when truncated).
    Truncated {
        /// The number of copy bits that were available before the payload ended.
        available_bits: u64,
    },
}

impl FrameHeaderCopyOutcome {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::Mismatch { .. } => "mismatch",
            Self::Truncated { .. } => "truncated",
        }
    }
}

/// Parses the non-first tile group's `frame_header_copy()` region and compares it
/// bit-for-bit against a recorded first header (AV2 v1.0.0 § 5.18.1 / § 6.17.1).
///
/// `reader` must be positioned at the **first bit of the copy region** — i.e. right after
/// the `tile_group_obu()` `is_first_tile_group` (`0`) and `frame_header_present_flag`
/// (`1`) flags, where `frame_header_copy()` begins (mirror :8435-8451). The function reads
/// up to `recorded.num_frame_header_bits()` `header_bit` `f(1)` values, advancing the
/// reader past every bit it could read. It returns:
///
/// - [`FrameHeaderCopyOutcome::Mismatch`] at the first differing bit (decidable even if the
///   payload later truncates — a differing bit within the available prefix is a definite
///   § 6.17.1 violation);
/// - [`FrameHeaderCopyOutcome::Truncated`] when the payload ends before
///   `NumFrameHeaderBits` bits and every available bit matched; or
/// - [`FrameHeaderCopyOutcome::Matches`] when all `NumFrameHeaderBits` bits were read and
///   matched.
///
/// The reader is left positioned after the last copy bit read; the § 5.19 tail beyond the
/// copy region (tile data) is intentionally left unparsed.
#[must_use]
pub fn parse_frame_header_copy(
    reader: &mut BitReader<'_>,
    recorded: &RecordedFrameHeaderBits,
) -> FrameHeaderCopyOutcome {
    let total = recorded.num_frame_header_bits();
    let mut index = 0u64;
    while index < total {
        let Ok(actual) = reader.read_bit() else {
            return FrameHeaderCopyOutcome::Truncated {
                available_bits: index,
            };
        };
        let actual = actual != 0;
        let expected = recorded.bit(index).unwrap_or(actual);
        if actual != expected {
            return FrameHeaderCopyOutcome::Mismatch {
                mismatch_bit: index,
            };
        }
        index += 1;
    }
    FrameHeaderCopyOutcome::Matches
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    #[test]
    fn tile_group_prefix_reads_first_tile_group_and_frame_header() {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(2); // seq_header_id_in_frame_header
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        assert!(prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        let frame_header = prefix.frame_header.expect("first tile group has a header");
        assert!(frame_header.cur_mfh_id.is_zero());
        assert_eq!(frame_header.seq_header_id_in_frame_header, Some(2));
        assert_eq!(frame_header.starts_cvs, Some(true)); // CLK + FirstPictureInTU
    }

    #[test]
    fn tile_group_prefix_non_first_without_header_stops_at_present_flag() {
        let mut bits = Bits::default();
        bits.bit(0); // is_first_tile_group == 0
        bits.bit(0); // frame_header_present_flag == 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::RegularTileGroup, Some(false)).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(!prefix.frame_header_present_flag);
        assert_eq!(prefix.frame_header, None);
        assert_eq!(prefix.consumed_bits, 2);
    }

    #[test]
    fn tile_group_prefix_non_first_header_copy_is_not_parsed() {
        let mut bits = Bits::default();
        bits.bit(0); // is_first_tile_group == 0
        bits.bit(1); // frame_header_present_flag == 1 (header copy follows)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::RegularTileGroup, Some(false)).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        assert_eq!(prefix.frame_header, None);
    }

    #[test]
    fn tile_group_prefix_eof_is_structured_error() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    /// Records `bit_pattern` (a bit-per-element slice) as the first header's bits and
    /// returns the recording plus the packed payload bytes a copy reader would re-read.
    fn record_bits(bit_pattern: &[u8]) -> (RecordedFrameHeaderBits, Vec<u8>) {
        let mut bits = Bits::default();
        for &b in bit_pattern {
            bits.bit(b);
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let recorded =
            RecordedFrameHeaderBits::record(&mut reader, bit_pattern.len() as u64).unwrap();
        (recorded, data)
    }

    #[test]
    fn recorded_frame_header_bits_round_trips_through_copy() {
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0]; // 11 bits
        let (recorded, copy_bytes) = record_bits(&pattern);
        assert_eq!(recorded.num_frame_header_bits(), 11);
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Matches
        );
        assert_eq!(reader.consumed_bits(), 11);
    }

    #[test]
    fn frame_header_copy_reports_first_mismatch_bit() {
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1]; // 9 bits
        let (recorded, _) = record_bits(&pattern);
        let mut copy = pattern;
        copy[5] = 1;
        let mut bits = Bits::default();
        for &b in &copy {
            bits.bit(b);
        }
        let copy_bytes = bits.into_bytes();
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Mismatch { mismatch_bit: 5 }
        );
    }

    #[test]
    fn frame_header_copy_reports_truncation_when_payload_short() {
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1, 1]; // 10 bits recorded
        let (recorded, _) = record_bits(&pattern);
        let mut bits = Bits::default();
        for &b in &pattern[..6] {
            bits.bit(b);
        }
        let copy_bytes = bits.into_bytes();
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        // The packed payload is 1 byte (8 bits) — 6 meaningful + 2 zero pad. Bits 6 and 7 of
        // the recorded pattern are (1, 0); the zero pad makes bit 6 (recorded 1) differ, so the
        // first decidable defect inside the available 8 bits is a mismatch at bit 6, not a
        // truncation. Use an exact-length payload to exercise the pure truncation path.
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Mismatch { mismatch_bit: 6 }
        );

        // Now a payload that is genuinely shorter than NumFrameHeaderBits with every
        // available bit matching: record 20 bits, supply a copy of exactly the first 12.
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 1, 1, 0, 1, 0];
        let (recorded, _) = record_bits(&pattern);
        let mut bits = Bits::default();
        for &b in &pattern[..12] {
            bits.bit(b);
        }
        let copy_bytes = bits.into_bytes(); // exactly 12 bits -> not byte aligned? 12 -> pads to 16
        // The packed payload holds 16 bits; bits 12..16 are zero pad. Recorded bits 12,13 are
        // (0, 0) and match the pad, bit 14 is 1 -> differs from pad 0, so a mismatch at 14.
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Mismatch { mismatch_bit: 14 }
        );

        // A truly truncated payload (exactly N bytes, fewer bits than recorded, all matching):
        // record 20 bits, supply a payload of exactly 1 byte (8 bits) matching bits 0..8.
        let mut bits = Bits::default();
        for &b in &pattern[..8] {
            bits.bit(b);
        }
        let copy_bytes = bits.into_bytes(); // exactly 1 byte = 8 bits, no pad
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Truncated { available_bits: 8 }
        );
    }

    #[test]
    fn record_frame_header_bits_eof_is_structured_error() {
        let data = [0b1010_0000u8]; // 8 bits available
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            RecordedFrameHeaderBits::record(&mut reader, 16),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn record_frame_header_bits_huge_count_short_reader_is_eof_not_oom() {
        // A huge num_frame_header_bits must NOT allocate ceil(n/8) bytes before any EOF check;
        // that can OOM-abort instead of returning the
        // documented UnexpectedEof (no-panic rule). The remaining-bits check must precede the
        // allocation, so an empty / short reader yields a structured error and no blowup.
        let mut empty = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            RecordedFrameHeaderBits::record(&mut empty, u64::MAX),
            Err(Error::UnexpectedEof { .. })
        ));

        let data = [0xFFu8; 4]; // 32 bits available
        let mut short = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            RecordedFrameHeaderBits::record(&mut short, 1u64 << 40),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    // --- § 5.19 post-frame-header structure (parse_tile_group_structure) ---

    /// Builds a reader pre-positioned `prefix_bits` into a payload (simulating the
    /// is_first_tile_group [+ frame_header_present_flag] + frame_header() span the
    /// caller already consumed), with `structure_bits` appended afterward.
    fn structure_reader(prefix_bits: u32, structure: &Bits) -> (Vec<u8>, u64) {
        let mut bits = Bits::default();
        for _ in 0..prefix_bits {
            bits.bit(1); // arbitrary prefix content; only its length matters
        }
        for &b in &structure.bits {
            bits.bit(b);
        }
        (bits.into_bytes(), u64::from(prefix_bits))
    }

    #[test]
    fn tile_group_layout_tile_bits_sums_and_caps() {
        assert_eq!(TileGroupLayout::new(2, 1, 1, 0).tile_bits(), 1);
        assert_eq!(TileGroupLayout::new(4, 4, 2, 2).tile_bits(), 4);
        assert_eq!(TileGroupLayout::new(64, 64, 6, 6).tile_bits(), 12);
        assert_eq!(TileGroupLayout::new(0, 0, 200, 200).tile_bits(), 32);
        assert_eq!(TileGroupLayout::new(64, 64, 6, 6).num_tiles, 4096);
    }

    #[test]
    fn structure_single_tile_infers_range_and_payload_boundary() {
        // NumTiles == 1: no flag, tg_start = 0, tg_end = 0, byte_alignment pads to the
        // byte boundary. Prefix = 1 bit (is_first_tile_group). After the prefix bit the
        // reader is at bit 1; byte_alignment pads 7 zero bits to byte 1. headerBytes = 1.
        let structure = Bits::default(); // no structure bits before byte_alignment
        let (data, _) = structure_reader(1, &structure);
        let mut data = data;
        data.resize(4, 0);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bit().unwrap();
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        let s = parse_tile_group_structure(&mut reader, layout, 4).unwrap();
        assert!(!s.tile_start_and_end_present_flag);
        assert_eq!(s.tg_start, 0);
        assert_eq!(s.tg_end, 0);
        assert_eq!(s.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(s.header_bytes, Some(1));
        assert_eq!(s.payload_size, Some(3));
    }

    #[test]
    fn structure_multi_tile_reads_flag_and_explicit_range() {
        // NumTiles == 4, TileColsLog2 = 1, TileRowsLog2 = 1 -> tileBits = 2. Prefix = 1
        // bit. Structure: flag = 1, tg_start = f(2) = 1, tg_end = f(2) = 3, then
        // byte_alignment.
        let mut structure = Bits::default();
        structure.bit(1); // tile_start_and_end_present_flag = 1
        structure.f(1, 2); // tg_start = 1
        structure.f(3, 2); // tg_end = 3
        let (mut data, _) = structure_reader(1, &structure);
        data.resize(8, 0);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bit().unwrap(); // consume the prefix bit
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let s = parse_tile_group_structure(&mut reader, layout, 8).unwrap();
        assert!(s.tile_start_and_end_present_flag);
        assert_eq!(s.tg_start, 1);
        assert_eq!(s.tg_end, 3);
        assert_eq!(s.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(s.header_bytes, Some(1));
        assert_eq!(s.payload_size, Some(7));
    }

    #[test]
    fn structure_multi_tile_flag_zero_infers_full_range() {
        // NumTiles == 4 but tile_start_and_end_present_flag == 0 -> tg covers the whole
        // frame (0 .. NumTiles - 1), no tg_start/tg_end bits.
        let mut structure = Bits::default();
        structure.bit(0); // tile_start_and_end_present_flag = 0
        let (mut data, _) = structure_reader(1, &structure);
        data.resize(4, 0);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bit().unwrap();
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let s = parse_tile_group_structure(&mut reader, layout, 4).unwrap();
        assert!(!s.tile_start_and_end_present_flag);
        assert_eq!(s.tg_start, 0);
        assert_eq!(s.tg_end, 3);
        assert_eq!(s.outcome, TileGroupStructureOutcome::Complete);
    }

    #[test]
    fn structure_eof_inside_range_is_truncation_preserving_facts() {
        // NumTiles == 4, tileBits = 2. Lay out a 1-byte payload as
        // prefix(6) + flag(1) + 1 bit, so after reading the flag at bit 6, the reader
        // is at bit 7 with a single bit left — tg_start's f(2) read then EOFs, surfacing
        // a truncation before any range field completes.
        let mut s = Bits::default();
        s.bit(1); // flag = 1 (range present)
        s.bit(0); // a single leftover bit; tg_start f(2) needs two and will EOF
        let (data, _) = structure_reader(6, &s); // 6 prefix + 1 flag + 1 = 8 bits = 1 byte
        let one_byte = vec![data[0]];
        let mut reader = BitReader::new(&one_byte, ByteOffset::new(0));
        for _ in 0..6 {
            reader.read_bit().unwrap(); // consume the 6 prefix bits
        }
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let parsed = parse_tile_group_structure(&mut reader, layout, 1).unwrap();
        assert!(parsed.tile_start_and_end_present_flag); // flag survived
        assert_eq!(parsed.outcome, TileGroupStructureOutcome::Truncated);
        assert!(parsed.header_bytes.is_none());
        assert!(parsed.payload_size.is_none());
    }

    #[test]
    fn structure_eof_before_flag_is_truncation() {
        // NumTiles > 1 but the payload ends exactly at the prefix boundary, so the
        // tile_start_and_end_present_flag f(1) read EOFs -> Truncated, defaults kept.
        let data = vec![0b1000_0000u8]; // 1 byte; we consume all 8 bits as prefix
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        for _ in 0..8 {
            reader.read_bit().unwrap();
        }
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let parsed = parse_tile_group_structure(&mut reader, layout, 1).unwrap();
        assert!(!parsed.tile_start_and_end_present_flag);
        assert_eq!(parsed.tg_start, 0);
        assert_eq!(parsed.tg_end, 3); // NumTiles - 1 default preserved
        assert_eq!(parsed.outcome, TileGroupStructureOutcome::Truncated);
    }

    #[test]
    fn structure_nonzero_byte_alignment_pad_is_invalid() {
        // NumTiles == 1: byte_alignment runs immediately. A non-zero pad bit is the
        // §6.2.4 defect, surfaced as InvalidByteAlignment (not a truncation).
        // Prefix = 1 bit, then 7 pad bits with a 1 in them.
        let mut bits = Bits::default();
        bits.bit(1); // prefix (is_first_tile_group)
        bits.bit(1); // a non-zero pad bit at position 1 -> byte_alignment must reject
        let mut data = bits.into_bytes();
        data.resize(4, 0);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bit().unwrap(); // consume the prefix bit; reader now at bit 1
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        assert!(matches!(
            parse_tile_group_structure(&mut reader, layout, 4),
            Err(Error::InvalidByteAlignment { .. })
        ));
    }

    #[test]
    fn structure_alignment_boundary_at_buffer_end_completes() {
        // A multi-tile flag=0 frame whose byte_alignment boundary lands exactly at the
        // end of a 1-byte payload: prefix(1) + flag(0) + 6 zero pad -> byte 1 == buffer
        // end. The structure completes with headerBytes == 1 and a zero-byte payload.
        let mut bits = Bits::default();
        bits.bit(1); // prefix
        bits.bit(0); // flag = 0 (NumTiles > 1, range inferred 0 .. NumTiles - 1)
        let data = bits.into_bytes(); // exactly 1 byte (6 zero pad)
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bit().unwrap();
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let s = parse_tile_group_structure(&mut reader, layout, 1).unwrap();
        assert_eq!(s.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(s.tg_start, 0);
        assert_eq!(s.tg_end, 3);
        assert_eq!(s.header_bytes, Some(1));
        assert_eq!(s.payload_size, Some(0));
    }

    // --- § 5.20.1 per-tile framing (parse_tile_group_framing) ---

    /// Encodes `tile_size_minus_1` as a `TileSizeBytes`-byte little-endian `le(n)` field
    /// (§ 4.11.5), the way a conformant tile group writes its tile size.
    fn le_size_field(tile_size_minus_1: u64, tile_size_bytes: u32) -> Vec<u8> {
        (0..tile_size_bytes)
            .map(|i| ((tile_size_minus_1 >> (i * 8)) & 0xFF) as u8)
            .collect()
    }

    #[test]
    fn framing_single_tile_takes_whole_region_no_size_field() {
        // A single-tile group: tg_start == tg_end == 0, the lone tile is the last tile and
        // reads no size field — it takes the whole 5-byte region.
        let region = vec![0xAA; 5];
        let framing = parse_tile_group_framing(&region, 0, 0, 1, false);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 1);
        let t = framing.tiles[0];
        assert_eq!(t.tile_num, 0);
        assert_eq!(t.size_field_offset, None);
        assert_eq!(t.tile_data_offset, 0);
        assert_eq!(t.tile_size, 5);
    }

    #[test]
    fn single_tile_constructor_matches_parser() {
        // The encoder-side constructor reproduces exactly the defect-free framing the
        // parser yields for a single-tile region (tg_start == tg_end == 0).
        let region = vec![0xAA; 5];
        let parsed = parse_tile_group_framing(&region, 0, 0, 1, false);
        assert_eq!(TileGroupFraming::single_tile(5), parsed);
    }

    #[test]
    fn single_tile_constructor_matches_parser_for_zero_size() {
        // The parser-inverse contract holds even for the degenerate zero-size input: the
        // constructor returns the same ZeroSizeTile-defective framing the parser yields
        // for an empty single-tile region, not a falsely-conformant defect: None.
        let parsed = parse_tile_group_framing(&[], 0, 0, 1, false);
        assert!(matches!(
            parsed.defect,
            Some(TileFramingDefect::ZeroSizeTile { tile_num: 0, .. })
        ));
        assert_eq!(TileGroupFraming::single_tile(0), parsed);
    }

    #[test]
    fn single_tile_framing_write_then_reparse_round_trips() {
        use crate::write::bit_writer::BitWriter;
        use crate::write::tile_group::write_tile_group_payload;

        // A single (last) tile writes no size field, so the payload region is byte-exact
        // with the coded data, and a reparse is value-equal to the constructed framing.
        let coded = [0x12u8, 0x34, 0x56, 0x78];
        let framing = TileGroupFraming::single_tile(coded.len() as u64);
        let mut writer = BitWriter::new();
        write_tile_group_payload(&mut writer, &framing, &[&coded], 1, false).unwrap();
        let region = writer.into_bytes();

        assert_eq!(region, coded, "single-tile payload is the coded bytes");
        assert_eq!(parse_tile_group_framing(&region, 0, 0, 1, false), framing);
    }

    #[test]
    fn single_tile_first_group_structure_has_canonical_fields() {
        let s = TileGroupStructure::single_tile_first_group();
        assert!(!s.tile_start_and_end_present_flag);
        assert_eq!(s.tg_start, 0);
        assert_eq!(s.tg_end, 0);
        assert_eq!(s.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(s.header_bytes, None);
        assert_eq!(s.payload_size, None);
    }

    #[test]
    fn single_tile_first_group_structure_round_trips() {
        use crate::span::ByteOffset;
        use crate::write::bit_writer::BitWriter;
        use crate::write::tile_group::write_tile_group_structure;

        let layout = TileGroupLayout::new(1, 1, 0, 0);
        let s = TileGroupStructure::single_tile_first_group();
        let mut writer = BitWriter::new();
        write_tile_group_structure(&mut writer, &s, layout).unwrap();
        let bytes = writer.into_bytes();

        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let parsed = parse_tile_group_structure(&mut reader, layout, bytes.len() as u64).unwrap();
        assert!(!parsed.tile_start_and_end_present_flag);
        assert_eq!(parsed.tg_start, 0);
        assert_eq!(parsed.tg_end, 0);
        assert_eq!(parsed.outcome, TileGroupStructureOutcome::Complete);
    }

    #[test]
    fn framing_multi_tile_records_sizes_and_offsets() {
        let mut region = Vec::new();
        region.extend(le_size_field(3 - 1, 2)); // tile0 tile_size_minus_1 = 2 -> tileSize 3
        region.extend([0x10, 0x11, 0x12]); // tile0 data (3 bytes)
        region.extend(le_size_field(2 - 1, 2)); // tile1 tile_size_minus_1 = 1 -> tileSize 2
        region.extend([0x20, 0x21]); // tile1 data (2 bytes)
        region.extend([0x30, 0x31, 0x32, 0x33, 0x34]); // tile2 (last) data (5 bytes)
        assert_eq!(region.len(), 2 + 3 + 2 + 2 + 5);

        let framing = parse_tile_group_framing(&region, 0, 2, 2, false);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 3);

        assert_eq!(framing.tiles[0].size_field_offset, Some(0));
        assert_eq!(framing.tiles[0].tile_data_offset, 2);
        assert_eq!(framing.tiles[0].tile_size, 3);

        assert_eq!(framing.tiles[1].size_field_offset, Some(5));
        assert_eq!(framing.tiles[1].tile_data_offset, 7);
        assert_eq!(framing.tiles[1].tile_size, 2);

        assert_eq!(framing.tiles[2].size_field_offset, None);
        assert_eq!(framing.tiles[2].tile_data_offset, 9);
        assert_eq!(framing.tiles[2].tile_size, 5);
    }

    #[test]
    fn framing_bridge_tiles_read_no_size_field() {
        let region = vec![0xCC; 8];
        let framing = parse_tile_group_framing(&region, 0, 2, 2, true);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 3);
        assert_eq!(framing.tiles[0].size_field_offset, None);
        assert_eq!(framing.tiles[0].tile_size, 0);
        assert_eq!(framing.tiles[1].size_field_offset, None);
        assert_eq!(framing.tiles[2].size_field_offset, None);
        assert_eq!(framing.tiles[2].tile_size, 8);
    }

    #[test]
    fn framing_flags_size_field_truncated() {
        let region = vec![0x01, 0x02];
        let framing = parse_tile_group_framing(&region, 0, 1, 3, false);
        assert!(framing.tiles.is_empty());
        assert_eq!(
            framing.defect,
            Some(TileFramingDefect::SizeFieldTruncated {
                tile_num: 0,
                size_field_offset: 0,
                available: 2,
            })
        );
    }

    #[test]
    fn framing_flags_tile_size_overflows_payload() {
        let mut region = Vec::new();
        region.extend(le_size_field(250, 1)); // tile0 size field: tileSize = 251
        region.extend([0u8; 5]); // 5 more bytes (far short of 251)
        let framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        assert!(framing.tiles.is_empty());
        assert_eq!(
            framing.defect,
            Some(TileFramingDefect::TileSizeOverflowsPayload {
                tile_num: 0,
                size_field_offset: 0,
                tile_size: 251,
                tile_size_bytes: 1,
                remaining: 6,
            })
        );
    }

    #[test]
    fn framing_exact_fit_two_tiles_is_conformant() {
        let mut region = Vec::new();
        region.extend(le_size_field(2 - 1, 1)); // tile0 tileSize = 2
        region.extend([0xA0, 0xA1]); // tile0 data
        region.extend([0xB0]); // tile1 (last) data
        assert_eq!(region.len(), 4);
        let framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 2);
        assert_eq!(framing.tiles[1].tile_size, 1); // last tile = remaining 1 byte
    }

    #[test]
    fn framing_zero_size_last_tile_is_a_defect() {
        let mut region = Vec::new();
        region.extend(le_size_field(2 - 1, 1)); // tile0 tileSize = 2 -> consumes 1 + 2 = 3
        region.extend([0xA0, 0xA1]); // tile0 data; region length == 3, last tile gets 0
        assert_eq!(region.len(), 3);
        let framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        assert!(matches!(
            framing.defect,
            Some(TileFramingDefect::ZeroSizeTile { tile_num: 1, .. })
        ));
        assert_eq!(framing.tiles[1].tile_size, 0);
    }

    #[test]
    fn framing_zero_size_last_bridge_tile_is_exempt() {
        let framing = parse_tile_group_framing(&[], 0, 0, 1, true);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles[0].tile_size, 0);
    }

    #[test]
    fn framing_huge_range_is_bounded_by_the_spec_tile_ceiling() {
        let framing = parse_tile_group_framing(&[], 0, u32::MAX, 1, true);
        assert!(framing.tiles.len() <= 4096);
    }

    #[test]
    fn framing_empty_range_records_nothing() {
        let framing = parse_tile_group_framing(&[0u8; 4], 2, 1, 1, false);
        assert!(framing.tiles.is_empty());
        assert_eq!(framing.defect, None);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The tile-group prefix parser must never panic on arbitrary input.
        #[test]
        fn parse_tile_group_prefix_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            raw_type in 0u8..=31,
            first_picture in any::<Option<bool>>(),
        ) {
            let obu_type = ObuType::from_raw(raw_type);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_tile_group_prefix(&mut reader, obu_type, first_picture);
        }

        /// Recording N bits from an arbitrary payload and replaying the copy must never
        /// panic, and the reader must never consume more than the available bits.
        #[test]
        fn frame_header_copy_never_panics(
            recorded_data in proptest::collection::vec(any::<u8>(), 0..32),
            copy_data in proptest::collection::vec(any::<u8>(), 0..32),
            num_bits in 0u64..=200,
        ) {
            let mut rec_reader = BitReader::new(&recorded_data, ByteOffset::new(0));
            if let Ok(recorded) = RecordedFrameHeaderBits::record(&mut rec_reader, num_bits) {
                prop_assert_eq!(recorded.num_frame_header_bits(), num_bits);
                let mut copy_reader = BitReader::new(&copy_data, ByteOffset::new(0));
                let outcome = parse_frame_header_copy(&mut copy_reader, &recorded);
                prop_assert!(copy_reader.consumed_bits() <= num_bits);
                prop_assert!(copy_reader.consumed_bits() <= (copy_data.len() as u64) * 8);
                if let FrameHeaderCopyOutcome::Truncated { available_bits } = outcome {
                    prop_assert!(available_bits < num_bits);
                }
            }
        }

        /// Recording a huge bit count from a small payload must EOF cleanly (the documented
        /// UnexpectedEof) instead of pre-allocating ceil(n/8) bytes and OOM-aborting — the
        /// remaining-bits guard must run before the allocation.
        #[test]
        fn record_huge_count_short_reader_never_oom(
            data in proptest::collection::vec(any::<u8>(), 0..16),
            num_bits in (1u64 << 32)..=u64::MAX,
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let result = RecordedFrameHeaderBits::record(&mut reader, num_bits);
            let is_eof = matches!(result, Err(crate::error::Error::UnexpectedEof { .. }));
            prop_assert!(is_eof);
        }

        /// The § 5.19 structure parser must never panic on arbitrary input or tile
        /// layout, and a successful (non-byte-alignment-defect) parse must never consume
        /// more than the available payload. The only non-EOF error it may return is
        /// InvalidByteAlignment (a §6.2.4 zero-bit defect); a payload-bounds EOF is always
        /// the Truncated outcome, never an error.
        #[test]
        fn parse_tile_group_structure_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            tile_cols in 0u32..=64,
            tile_rows in 0u32..=64,
            cols_log2 in 0u8..=8,
            rows_log2 in 0u8..=8,
            sz in 0u64..=256,
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let layout = TileGroupLayout::new(tile_cols, tile_rows, cols_log2, rows_log2);
            match parse_tile_group_structure(&mut reader, layout, sz) {
                Ok(structure) => {
                    prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
                    if structure.outcome == TileGroupStructureOutcome::Complete {
                        prop_assert!(structure.header_bytes.is_some());
                        prop_assert!(structure.payload_size.is_some());
                    }
                }
                Err(crate::error::Error::InvalidByteAlignment { .. }) => {}
                Err(other) => prop_assert!(false, "unexpected error: {other:?}"),
            }
        }

        /// The §5.20.1 framing parser must never panic over arbitrary payload bytes, tile
        /// ranges, TileSizeBytes, or IsBridge — and its recorded framing must stay within the
        /// region (every recorded offset/size is bounded by the region length, and the
        /// per-tile bookkeeping never overruns).
        #[test]
        fn parse_tile_group_framing_never_panics(
            payload in proptest::collection::vec(any::<u8>(), 0..128),
            tg_start in 0u32..=8,
            tg_end in 0u32..=8,
            tile_size_bytes in any::<u32>(),
            is_bridge in any::<bool>(),
        ) {
            let framing =
                parse_tile_group_framing(&payload, tg_start, tg_end, tile_size_bytes, is_bridge);
            let region_len = payload.len() as u64;
            for t in &framing.tiles {
                if let Some(sf) = t.size_field_offset {
                    prop_assert!(sf <= region_len);
                }
                prop_assert!(t.tile_data_offset <= region_len);
                prop_assert!(t.tile_size <= region_len);
                prop_assert!(t.tile_data_offset.saturating_add(t.tile_size) <= region_len);
            }
            if let Some(defect) = framing.defect {
                prop_assert!(defect.size_field_offset() <= region_len);
            }
        }
    }
}
