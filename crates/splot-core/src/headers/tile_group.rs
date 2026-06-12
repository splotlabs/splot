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
use crate::error::Result;
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
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or a
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
    /// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if fewer than
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
    }
}
