// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 `seg_info(numSegments)` writer — the inverse of the § 5.4.9
//! `seg_info()` parser ([`crate::segment::parse_seg_info`];
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-9`) (`ENC-BITSTREAM-WRITER`).
//!
//! `seg_info()` is the reusable segmentation-feature syntax shared by the sequence
//! segment config (§ 5.4.4) and the multi-frame header (§ 5.7). This writer is
//! additive: it reads a parsed [`SegmentInfo`] and serializes it back to bits via
//! [`BitWriter`], in the spec's `for i { for j { ... } }` order.
//!
//! The parser stores *clipped* feature data — each enabled signed feature is
//! `Clip3(-limit, limit, su(n))` and each unsigned feature is `Clip3(0, limit, f(b))`
//! — and disabled features are zeroed, so several model values have no bitstream home
//! and must be rejected up front with a typed [`WriteError`] before any bit is written
//! (reject-before-write): a feature whose data lies outside its `[-limit, limit]` clip
//! window, an enabled zero-width unsigned feature carrying non-zero data, a disabled
//! feature carrying non-zero data, or a segment beyond `numSegments` that is not
//! disabled. Because `Clip3` is idempotent on in-window values, an in-window value
//! round-trips exactly. See [`WriteError::NonCanonicalSequenceValue`].

use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `Segmentation_Feature_Bits[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-9`): value bits coded per enabled
/// feature. Duplicated locally (the parser's copy is private) so the writer derives the
/// exact `su(n)`/`f(b)` widths the parser read.
const SEGMENTATION_FEATURE_BITS: [u32; SEG_LVL_MAX] = [9, 0, 0];

/// `Segmentation_Feature_Signed[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9): whether each
/// feature value is signed (`su`) or unsigned (`f`).
const SEGMENTATION_FEATURE_SIGNED: [bool; SEG_LVL_MAX] = [true, false, false];

/// `MAXQ_BITS = MAXQ_8_BITS + 4 * MAXQ_OFFSET = 255 + 4 * 24` (AV2 v1.0.0 § 3), the
/// `SEG_LVL_ALT_Q` clip limit.
const MAXQ_BITS: i32 = 255 + 4 * 24;

/// `Segmentation_Feature_Max[SEG_LVL_MAX]` (AV2 v1.0.0 § 5.4.9): the per-feature clip
/// limit. An enabled feature's stored data must lie in `[-max, max]` (signed) or
/// `[0, max]` (unsigned) to be reproducible.
const SEGMENTATION_FEATURE_MAX: [i32; SEG_LVL_MAX] = [MAXQ_BITS, 0, 0];

/// Writes `seg_info(numSegments)` (AV2 v1.0.0 § 5.4.9,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-9`), the exact inverse of
/// [`crate::segment::parse_seg_info`].
///
/// `num_segments` is the caller's `MaxSegments` (8 or 16 at the sequence/MFH sites);
/// the writer emits, for each of the first `min(num_segments, MAX_SEGMENTS)` segments
/// and each of the `SEG_LVL_MAX` features, the `feature_enabled` bit followed (when
/// enabled) by the feature value via `su(1 + bitsToRead)` for signed features or
/// `f(bitsToRead)` for unsigned ones. For features `1` and `2` `bitsToRead` is `0`, so
/// no value bits follow even when the feature is enabled (matching the parser).
///
/// The model is fully validated before any bit is written, so a rejected model leaves
/// `writer` unchanged.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] if [`SegmentInfo::num_segments`] does not
///   equal `min(num_segments, MAX_SEGMENTS)`; if a segment at index `>= num_segments` is
///   not disabled; if a disabled feature carries non-zero data; if an enabled signed
///   feature's data is outside `[-limit, limit]`; or if an enabled zero-width unsigned
///   feature carries non-zero data (it can only ever decode to `0`).
/// - [`WriteError::ValueOutOfRange`] propagated from the `su` writer for a value outside
///   its width (unreachable once the clip-window check passes).
pub fn write_seg_info(
    writer: &mut BitWriter,
    info: &SegmentInfo,
    num_segments: u8,
) -> WriteResult<()> {
    check_seg_info_encodable(info, num_segments)?;

    let count = (num_segments as usize).min(MAX_SEGMENTS);
    for segment in info.features.iter().take(count) {
        for (j, feature) in segment.iter().enumerate() {
            // feature_enabled: f(1).
            writer.write_flag(feature.enabled)?;
            if feature.enabled {
                let bits_to_read = SEGMENTATION_FEATURE_BITS[j];
                if SEGMENTATION_FEATURE_SIGNED[j] {
                    // AV2 § 5.4.9: n = 1 + bitsToRead; feature_value = su(n).
                    writer.write_su(feature.data, 1 + bits_to_read)?;
                } else {
                    // f(bitsToRead); for j = 1, 2 bitsToRead is 0 and `write_bits` emits
                    // nothing, matching the parser (the value can only decode to 0).
                    // `data` is validated `>= 0` and within the `f(bitsToRead)` width by
                    // `check_seg_info_encodable`, so the cast is lossless.
                    writer.write_bits(feature.data as u32, bits_to_read)?;
                }
            }
        }
    }
    Ok(())
}

/// Validates that `info` is a [`SegmentInfo`] the § 5.4.9 parser could have produced for
/// `num_segments`, returning a typed [`WriteError`] before any bit is written. Mirrors
/// the parser's clip/zero derivation so the writer emits bits only from reproducible
/// values.
///
/// Exposed `pub(crate)` so a containing structure (e.g. `sequence_segment_config()`,
/// § 5.4.4) can pre-validate the nested `seg_info()` body in its own up-front check pass
/// — otherwise the outer writer would emit its leading flags before [`write_seg_info`]
/// rejected a bad body, breaking reject-before-write for the composite structure.
pub(crate) fn check_seg_info_encodable(info: &SegmentInfo, num_segments: u8) -> WriteResult<()> {
    let count = (num_segments as usize).min(MAX_SEGMENTS);
    // The parser stores `num_segments = count`; a model claiming a different count could
    // not have come from this call site.
    if info.num_segments as usize != count {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "seg_info_num_segments",
        });
    }

    for (i, segment) in info.features.iter().enumerate() {
        for (j, feature) in segment.iter().enumerate() {
            if i >= count {
                // Segments beyond numSegments are never signaled; the parser leaves them
                // disabled with zero data.
                if *feature != SegmentFeature::DISABLED {
                    return Err(WriteError::NonCanonicalSequenceValue {
                        what: "seg_info_unsignaled_segment",
                    });
                }
                continue;
            }
            check_feature_encodable(*feature, j)?;
        }
    }
    Ok(())
}

/// Validates one `(enabled, data)` feature pair against the § 5.4.9 clip/zero rules for
/// feature index `j`.
fn check_feature_encodable(feature: SegmentFeature, j: usize) -> WriteResult<()> {
    if !feature.enabled {
        // A disabled feature is `data = 0` (the parser never reads its value).
        if feature.data != 0 {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_disabled_data",
            });
        }
        return Ok(());
    }
    let limit = SEGMENTATION_FEATURE_MAX[j];
    if SEGMENTATION_FEATURE_SIGNED[j] {
        // The parser stores `Clip3(-limit, limit, su(n))`; only values already in the
        // clip window round-trip (Clip3 is idempotent in-window).
        if feature.data < -limit || feature.data > limit {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_signed_data",
            });
        }
    } else {
        // The parser stores `Clip3(0, limit, f(bitsToRead))`. For features 1 and 2
        // `bitsToRead == 0` and `limit == 0`, so the only reproducible value is 0.
        let bits = SEGMENTATION_FEATURE_BITS[j];
        if feature.data < 0 || feature.data > limit {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_unsigned_data",
            });
        }
        // Also reject a value that would not fit the `f(bitsToRead)` field (defensive;
        // `limit <= (1 << bits) - 1` holds for the spec table, so the clip check above is
        // sufficient, but this guards against future table changes).
        if bits < 32 && (feature.data as u32) >= (1u32 << bits) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_unsigned_data",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::segment::parse_seg_info;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn parse(bytes: &[u8], num_segments: u8) -> SegmentInfo {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_seg_info(&mut reader, num_segments).unwrap()
    }

    fn write(info: &SegmentInfo, num_segments: u8) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_seg_info(&mut writer, info, num_segments).unwrap();
        writer.into_bytes()
    }

    /// Asserts the semantic round-trip `parse(write(info)) == info` and byte-stability.
    fn assert_roundtrip(info: &SegmentInfo, num_segments: u8) {
        let bytes = write(info, num_segments);
        let reparsed = parse(&bytes, num_segments);
        assert_eq!(&reparsed, info, "parse(write(info)) != info");
        assert_eq!(
            write(&reparsed, num_segments),
            bytes,
            "write not idempotent"
        );
    }

    #[test]
    fn all_disabled_eight_segments_byte_exact() {
        // 8 segments * SEG_LVL_MAX (3) enabled bits, all 0 (24 bits = 3 bytes).
        let mut bits = Bits::default();
        for _ in 0..(8 * SEG_LVL_MAX) {
            bits.bit(0);
        }
        let data = bits.into_bytes();
        let info = parse(&data, 8);
        let written = write(&info, 8);
        assert_eq!(written, data, "all-disabled seg_info not byte-exact");
        assert_roundtrip(&info, 8);
    }

    #[test]
    fn all_disabled_sixteen_segments_round_trips() {
        let mut bits = Bits::default();
        for _ in 0..(16 * SEG_LVL_MAX) {
            bits.bit(0);
        }
        let info = parse(&bits.into_bytes(), 16);
        assert_eq!(info.num_segments, 16);
        assert_roundtrip(&info, 16);
    }

    #[test]
    fn signed_quantizer_feature_round_trips() {
        // Segment 0 feature 0 enabled with su(10) = 100 (within +/-351, unclipped).
        let mut bits = Bits::default();
        bits.bit(1); // feature_enabled[0][0]
        bits.f(100, 10); // su(10) value 100
        bits.bit(0); // [0][1]
        bits.bit(0); // [0][2]
        let info = parse(&bits.into_bytes(), 1);
        assert!(info.features[0][0].enabled);
        assert_eq!(info.features[0][0].data, 100);
        assert_roundtrip(&info, 1);
    }

    #[test]
    fn clipped_negative_quantizer_round_trips_at_the_clip_limit() {
        // su(10) = -512 parses to clipped -351; re-writing -351 (in-window) round-trips.
        let mut bits = Bits::default();
        bits.bit(1); // feature_enabled[0][0]
        bits.f(0b10_0000_0000, 10); // su(10) = -512 -> clipped -351
        bits.bit(0);
        bits.bit(0);
        // segment 1 all-disabled
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let info = parse(&bits.into_bytes(), 2);
        assert_eq!(info.features[0][0].data, -351);
        assert_roundtrip(&info, 2);
    }

    #[test]
    fn enabled_zero_width_feature_round_trips() {
        // Feature 1 (bitsToRead == 0) enabled emits only the enable bit; data is 0.
        let mut bits = Bits::default();
        bits.bit(0); // [0][0]
        bits.bit(1); // [0][1] enabled, no value bits
        bits.bit(0); // [0][2]
        let info = parse(&bits.into_bytes(), 1);
        assert!(info.features[0][1].enabled);
        assert_eq!(info.features[0][1].data, 0);
        assert_roundtrip(&info, 1);
    }

    // ----- Rejection tests (one per WriteError reject path) -----

    #[test]
    fn rejects_wrong_num_segments() {
        let info = parse(&[0u8; 6], 8);
        let mut writer = BitWriter::new();
        // Claim 16 while the model holds num_segments 8.
        assert!(matches!(
            write_seg_info(&mut writer, &info, 16),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_num_segments"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_non_disabled_segment_beyond_count() {
        let mut info = parse(&[0u8; 3], 8);
        // Segment 8 is beyond num_segments (8); the parser leaves it disabled.
        info.features[8][0] = SegmentFeature {
            enabled: true,
            data: 0,
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_seg_info(&mut writer, &info, 8),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_unsignaled_segment"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_disabled_feature_with_non_zero_data() {
        let mut info = parse(&[0u8; 3], 8);
        info.features[0][0] = SegmentFeature {
            enabled: false,
            data: 5, // disabled must be 0
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_seg_info(&mut writer, &info, 8),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_disabled_data"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_signed_data_out_of_clip_window() {
        let mut info = parse(&[0u8; 3], 8);
        info.features[0][0] = SegmentFeature {
            enabled: true,
            data: MAXQ_BITS + 1, // beyond the +/-351 clip window
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_seg_info(&mut writer, &info, 8),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_signed_data"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_enabled_zero_width_feature_with_non_zero_data() {
        let mut info = parse(&[0u8; 3], 8);
        info.features[0][1] = SegmentFeature {
            enabled: true,
            data: 1, // feature 1 has bitsToRead 0 / limit 0; only 0 decodes
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_seg_info(&mut writer, &info, 8),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "seg_info_unsigned_data"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::segment::parse_seg_info;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn parse_ok(bytes: &[u8], num_segments: u8) -> Option<SegmentInfo> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_seg_info(&mut reader, num_segments).ok()
    }

    proptest! {
        /// Every parser-reachable `seg_info` round-trips: parse(write(info)) == info,
        /// and re-emission is byte-stable.
        #[test]
        fn roundtrip_seg_info(
            num_segments in prop_oneof![Just(8u8), Just(16u8)],
            // One bit per (segment, feature) enable decision, plus a 10-bit su value per
            // enabled signed feature; build raw bytes long enough to cover the worst case.
            data in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let Some(info) = parse_ok(&data, num_segments) else { return Ok(()); };
            let mut writer = BitWriter::new();
            write_seg_info(&mut writer, &info, num_segments).unwrap();
            let bytes = writer.into_bytes();
            let reparsed = parse_ok(&bytes, num_segments).expect("written bytes must reparse");
            prop_assert_eq!(&reparsed, &info);
            let mut w2 = BitWriter::new();
            write_seg_info(&mut w2, &reparsed, num_segments).unwrap();
            prop_assert_eq!(w2.into_bytes(), bytes);
        }

        /// The writer never panics on a parsed model regardless of `num_segments`.
        #[test]
        fn writer_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            num_segments in any::<u8>(),
        ) {
            if let Some(info) = parse_ok(&data, num_segments) {
                let mut writer = BitWriter::new();
                let _ = write_seg_info(&mut writer, &info, num_segments);
            }
        }
    }
}
