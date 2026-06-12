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
/// `obu_type` is the tile-group OBU type, and `first_picture_in_tu` is forwarded to
/// the frame-header prefix parser for `startCVS` derivation. The parser reads
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
    first_picture_in_tu: bool,
) -> Result<TileGroupHeaderPrefix> {
    let start_bits = reader.consumed_bits();

    let is_first_tile_group = reader.read_bit()? != 0;
    let frame_header_present_flag = if is_first_tile_group {
        true
    } else {
        reader.read_bit()? != 0
    };

    // Only the first tile group carries a parseable frame_header(1). A non-first tile
    // group with frame_header_present_flag == 1 carries frame_header_copy(), which is
    // a bit copy of the first header and is not modeled by this prefix parser.
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

    // § 5.19 (:8467-8473): tile_start_and_end_present_flag defaults to 0 and is read as
    // f(1) only when NumTiles > 1. An EOF here is a truncation of the modeled region.
    let mut structure = TileGroupStructure {
        tile_start_and_end_present_flag: false,
        tg_start: 0,
        // NumTiles >= 1 for any decodable frame; saturating_sub keeps a degenerate
        // NumTiles == 0 layout from underflowing (tg_end stays 0).
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

    // § 5.19 (:8475-8493): when NumTiles == 1 or the flag is 0, tg_start/tg_end are
    // inferred (0 .. NumTiles - 1, already set above). Otherwise read both f(tileBits).
    if num_tiles > 1 && structure.tile_start_and_end_present_flag {
        let tile_bits = layout.tile_bits();
        let Ok(tg_start) = reader.read_bits(tile_bits) else {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        };
        structure.tg_start = tg_start;
        let Ok(tg_end) = reader.read_bits(tile_bits) else {
            // tg_start was read; preserve it and surface the truncation.
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        };
        structure.tg_end = tg_end;
    }

    // § 5.19 (:8519): byte_alignment(). The use_bru BruTileActives loop (:8495-8517) is
    // dead on the intra path (use_bru == 0), so byte_alignment() runs next. A non-zero
    // pad bit is the decidable §6.2.4 defect; an EOF before the boundary is a truncation.
    match reader.byte_align_zero() {
        Ok(()) => {}
        Err(Error::UnexpectedEof { .. }) => {
            structure.outcome = TileGroupStructureOutcome::Truncated;
            return Ok(structure);
        }
        Err(other) => return Err(other),
    }

    // § 5.19 (:8521-8527): headerBytes = (endBitPos - startBitPos) / 8 over the whole
    // tile_group_obu() header. The reader was constructed at startBitPos (the OBU payload
    // start), so endBitPos - startBitPos == reader.consumed_bits(); now byte-aligned, the
    // division is exact. Then sz -= headerBytes and tile_group_payload(sz) runs over the
    // remainder.
    let header_bytes = reader.consumed_bits() / 8;
    structure.header_bytes = Some(header_bytes);
    structure.payload_size = Some(sz.saturating_sub(header_bytes));
    Ok(structure)
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
        // Reject an out-of-range count up front, before allocating: `num_frame_header_bits`
        // is public API, so a hostile/garbage value (e.g. `u64::MAX`) must not drive a
        // `ceil(n/8)`-byte allocation that OOM-aborts. The bit-by-bit loop below would EOF
        // anyway, but only after the buffer is reserved, so the guard must precede it.
        if reader.remaining_bits() < num_frame_header_bits {
            // The deficit, reported in whole bytes, matches the per-bit `read_bit()` EOF the
            // loop would have raised at the first missing bit.
            let needed_bits = num_frame_header_bits.saturating_sub(reader.remaining_bits());
            return Err(crate::error::Error::UnexpectedEof {
                offset: reader.byte_offset(),
                needed: usize::try_from(needed_bits.div_ceil(8)).unwrap_or(usize::MAX),
            });
        }
        let byte_len = num_frame_header_bits.div_ceil(8);
        // The bit count is bounded by the remaining payload (checked above), so the cast is
        // sound; a payload large enough to overflow `usize` cannot be held in memory anyway.
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
    /// beyond [`Self::num_frame_header_bits`].
    #[must_use]
    fn bit(&self, index: u64) -> Option<bool> {
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
            // Payload ended inside the copy region: every bit read so far matched (a
            // mismatch would have returned above), so this is a clean truncation.
            return FrameHeaderCopyOutcome::Truncated {
                available_bits: index,
            };
        };
        let actual = actual != 0;
        // `index < total` guarantees `bit(index)` is `Some`.
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

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    #[test]
    fn tile_group_prefix_reads_first_tile_group_and_frame_header() {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(2); // seq_header_id_in_frame_header
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix = parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, true).unwrap();
        assert!(prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        let frame_header = prefix.frame_header.expect("first tile group has a header");
        assert!(frame_header.cur_mfh_id.is_zero());
        assert_eq!(frame_header.seq_header_id_in_frame_header, Some(2));
        assert!(frame_header.starts_cvs); // CLK + FirstPictureInTU
    }

    #[test]
    fn tile_group_prefix_non_first_without_header_stops_at_present_flag() {
        let mut bits = Bits::default();
        bits.bit(0); // is_first_tile_group == 0
        bits.bit(0); // frame_header_present_flag == 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::RegularTileGroup, false).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(!prefix.frame_header_present_flag);
        assert_eq!(prefix.frame_header, None);
        assert_eq!(prefix.consumed_bits, 2);
    }

    #[test]
    fn tile_group_prefix_non_first_header_copy_is_not_parsed() {
        // is_first_tile_group == 0 but frame_header_present_flag == 1 -> a
        // frame_header_copy() the prefix parser records but does not parse.
        let mut bits = Bits::default();
        bits.bit(0); // is_first_tile_group == 0
        bits.bit(1); // frame_header_present_flag == 1 (header copy follows)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::RegularTileGroup, false).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        assert_eq!(prefix.frame_header, None);
    }

    #[test]
    fn tile_group_prefix_eof_is_structured_error() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, true),
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
        // A non-byte-aligned bit count exercises the trailing partial byte.
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0]; // 11 bits
        let (recorded, copy_bytes) = record_bits(&pattern);
        assert_eq!(recorded.num_frame_header_bits(), 11);
        let mut reader = BitReader::new(&copy_bytes, ByteOffset::new(0));
        assert_eq!(
            parse_frame_header_copy(&mut reader, &recorded),
            FrameHeaderCopyOutcome::Matches
        );
        // The copy reader consumed exactly NumFrameHeaderBits.
        assert_eq!(reader.consumed_bits(), 11);
    }

    #[test]
    fn frame_header_copy_reports_first_mismatch_bit() {
        let pattern = [1u8, 0, 1, 1, 0, 0, 1, 0, 1]; // 9 bits
        let (recorded, _) = record_bits(&pattern);
        // Flip bit 5 of the copy (0 -> 1).
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
        // The copy payload carries only the first 6 (matching) bits.
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
        // Regression (codex round-8 F2): a huge num_frame_header_bits must NOT allocate
        // ceil(n/8) bytes before any EOF check — that can OOM-abort instead of returning the
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
        // Max AV2-legal log2 is 6 each -> 12 bits.
        assert_eq!(TileGroupLayout::new(64, 64, 6, 6).tile_bits(), 12);
        // Out-of-domain log2 values are capped at 32 so the read width stays legal.
        assert_eq!(TileGroupLayout::new(0, 0, 200, 200).tile_bits(), 32);
        // num_tiles is a saturating product.
        assert_eq!(TileGroupLayout::new(64, 64, 6, 6).num_tiles, 4096);
    }

    #[test]
    fn structure_single_tile_infers_range_and_payload_boundary() {
        // NumTiles == 1: no flag, tg_start = 0, tg_end = 0, byte_alignment pads to the
        // byte boundary. Prefix = 1 bit (is_first_tile_group). After the prefix bit the
        // reader is at bit 1; byte_alignment pads 7 zero bits to byte 1. headerBytes = 1.
        let structure = Bits::default(); // no structure bits before byte_alignment
        let (data, _) = structure_reader(1, &structure);
        // Pad the payload to allow byte_alignment + a payload region (sz = 4 bytes).
        let mut data = data;
        data.resize(4, 0);
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        // Consume the 1 prefix bit so the reader's consumed_bits matches the caller state.
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
        // Prefix(1) + flag(1) + tg_start(2) + tg_end(2) = 6 bits, padded to byte 1.
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
            first_picture in any::<bool>(),
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
            // Recording may EOF (num_bits may exceed the payload); only a successful record
            // produces a comparison input. Either branch must be panic-free.
            if let Ok(recorded) = RecordedFrameHeaderBits::record(&mut rec_reader, num_bits) {
                prop_assert_eq!(recorded.num_frame_header_bits(), num_bits);
                let mut copy_reader = BitReader::new(&copy_data, ByteOffset::new(0));
                let outcome = parse_frame_header_copy(&mut copy_reader, &recorded);
                // The copy reader consumed at most NumFrameHeaderBits and at most the payload.
                prop_assert!(copy_reader.consumed_bits() <= num_bits);
                prop_assert!(copy_reader.consumed_bits() <= (copy_data.len() as u64) * 8);
                if let FrameHeaderCopyOutcome::Truncated { available_bits } = outcome {
                    prop_assert!(available_bits < num_bits);
                }
            }
        }

        /// Recording a huge bit count from a small payload must EOF cleanly (the documented
        /// UnexpectedEof) instead of pre-allocating ceil(n/8) bytes and OOM-aborting — the
        /// remaining-bits guard must run before the allocation (round-8 F2).
        #[test]
        fn record_huge_count_short_reader_never_oom(
            data in proptest::collection::vec(any::<u8>(), 0..16),
            num_bits in (1u64 << 32)..=u64::MAX,
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            // The payload holds at most 16*8 == 128 bits, far fewer than num_bits, so the
            // result must be the structured EOF error — and crucially without allocating.
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
                    // tg_end >= tg_start is a structural property of the parse output on
                    // the inferred-range path (NumTiles == 1 or flag == 0); the explicit
                    // path may carry an out-of-range pair that the validator flags.
                    if structure.outcome == TileGroupStructureOutcome::Complete {
                        prop_assert!(structure.header_bytes.is_some());
                        prop_assert!(structure.payload_size.is_some());
                    }
                }
                Err(crate::error::Error::InvalidByteAlignment { .. }) => {}
                Err(other) => prop_assert!(false, "unexpected error: {other:?}"),
            }
        }
    }
}
