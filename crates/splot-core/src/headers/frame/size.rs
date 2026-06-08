// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-size helpers for the frame-header core parser (AV2 v1.0.0 § 5.18.4).
//!
//! This models only the `frame_size()` paths whose dimensions are exactly
//! derivable from already-parsed state
//! ([`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-1`]):
//!
//! ```text
//! frame_size( ) {
//!     if ( frame_size_override_flag ) {
//!         n = frame_width_bits_minus_1 + 1
//!         frame_width_minus_1                              f(n)
//!         n = frame_height_bits_minus_1 + 1
//!         frame_height_minus_1                             f(n)
//!         FrameWidth = frame_width_minus_1 + 1
//!         FrameHeight = frame_height_minus_1 + 1
//!     } else {
//!         FrameWidth = mfh_frame_width_minus_1[ cur_mfh_id ] + 1
//!         FrameHeight = mfh_frame_height_minus_1[ cur_mfh_id ] + 1
//!     }
//!     compute_image_size( )
//! }
//! ```
//!
//! `frame_size_with_refs()` (§ 5.18.4.3) and `frame_size_with_bridge()`
//! (§ 5.18.4.2) need reference-frame dimensions that this phase does not model,
//! so they are intentionally absent here.

use crate::bitio::BitReader;
use crate::error::Result;

/// A frame's luma dimensions, derived from `frame_size()` (AV2 v1.0.0 § 5.18.4.1).
///
/// `width`/`height` are `FrameWidth`/`FrameHeight` (`frame_width_minus_1 + 1` and
/// `frame_height_minus_1 + 1`), each at least `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameSize {
    /// `FrameWidth` in luma samples.
    pub width: u32,
    /// `FrameHeight` in luma samples.
    pub height: u32,
}

impl FrameSize {
    /// Creates a [`FrameSize`] from already-derived luma dimensions.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// `CeilLog2(x)`: the number of bits needed to code a value in `0..x`
/// (AV2 v1.0.0 § 4.8, `docs/spec/av2/1.0.0/04-conventions.md#s-4-8`).
///
/// Returns `0` for `x < 2`, matching the spec's `CeilLog2(0) == CeilLog2(1) == 0`.
#[must_use]
pub(crate) const fn ceil_log2(x: u32) -> u32 {
    if x < 2 {
        0
    } else {
        u32::BITS - (x - 1).leading_zeros()
    }
}

/// Parses `frame_size()` for the paths whose dimensions are exactly known
/// (AV2 v1.0.0 § 5.18.4.1).
///
/// `frame_width_bits`/`frame_height_bits` are `n` (`frame_*_bits_minus_1 + 1`) from
/// the active sequence header. `default_dims` is `(FrameWidth, FrameHeight)` for the
/// non-override path — `Some` only when the multi-frame-header default dimensions are
/// known (i.e. `cur_mfh_id == 0`, where they equal the sequence maximum per
/// § 5.18.2). When `frame_size_override_flag` is set, the explicit `f(n)` width/height
/// are read regardless of `default_dims`, so the reader always advances by the same
/// number of bits the spec consumes.
///
/// Returns `Ok(None)` when the non-override path's dimensions are not modeled (no
/// bits are read in that case, so the caller's bit position is still correct).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends before an overridden width/height can be read.
pub(crate) fn parse_frame_size(
    reader: &mut BitReader<'_>,
    frame_size_override_flag: bool,
    frame_width_bits: u32,
    frame_height_bits: u32,
    default_dims: Option<(u32, u32)>,
) -> Result<Option<FrameSize>> {
    if frame_size_override_flag {
        // AV2 § 5.18.4.1: f(frame_width_bits_minus_1 + 1) / f(frame_height_bits_minus_1 + 1).
        let frame_width_minus_1 = reader.read_bits(frame_width_bits)?;
        let frame_height_minus_1 = reader.read_bits(frame_height_bits)?;
        Ok(Some(FrameSize::new(
            frame_width_minus_1.saturating_add(1),
            frame_height_minus_1.saturating_add(1),
        )))
    } else {
        // AV2 § 5.18.4.1 else-branch: FrameWidth/Height come from the multi-frame-header
        // defaults, which equal the sequence maximum when cur_mfh_id == 0 (§ 5.18.2).
        Ok(default_dims.map(|(width, height)| FrameSize::new(width, height)))
    }
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
        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bits.push(((value >> shift) & 1) as u8);
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
    fn ceil_log2_matches_spec_definition() {
        // CeilLog2(0) == CeilLog2(1) == 0; otherwise the bits to code 0..x.
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(4), 2);
        assert_eq!(ceil_log2(8), 3);
        assert_eq!(ceil_log2(16), 4);
    }

    #[test]
    fn frame_size_override_reads_explicit_width_and_height() {
        // frame_width_bits = 12, frame_height_bits = 12; 1920x1080.
        let mut bits = Bits::default();
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let size = parse_frame_size(&mut reader, true, 12, 12, Some((4096, 4096)))
            .unwrap()
            .expect("override path always yields a size");
        assert_eq!(size, FrameSize::new(1920, 1080));
        assert_eq!(reader.consumed_bits(), 24);
    }

    #[test]
    fn frame_size_default_uses_known_dimensions_without_reading_bits() {
        let data = [0u8; 0];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let size = parse_frame_size(&mut reader, false, 12, 12, Some((640, 480)))
            .unwrap()
            .expect("default dims known");
        assert_eq!(size, FrameSize::new(640, 480));
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn frame_size_default_unknown_returns_none_without_reading_bits() {
        let data = [0xffu8; 4];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let size = parse_frame_size(&mut reader, false, 12, 12, None).unwrap();
        assert_eq!(size, None);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn frame_size_override_eof_is_structured_error() {
        // Only 12 bits available, but the override path needs 24.
        let mut bits = Bits::default();
        bits.f(0, 12);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_frame_size(&mut reader, true, 12, 12, None),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
