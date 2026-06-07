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
    }
}
