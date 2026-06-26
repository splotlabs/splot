// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **prefix** writer (`ENC-BITSTREAM-WRITER`) — the inverse of the
//! § 5.18.2 activation prefix parser
//! [`parse_frame_header_prefix`](crate::headers::frame::parse_frame_header_prefix).
//!
//! `frame_header_info()` (AV2 v1.0.0 § 5.18.2,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) opens with the activation /
//! reference fields: `cur_mfh_id` (`uvlc`, inferred `0` for a bridge frame) and, when
//! `cur_mfh_id == 0`, `seq_header_id_in_frame_header` (`uvlc`). Every other field on a
//! [`FrameHeaderPrefix`] is *derived* from `obu_type` (`isKeyFrame`, `IsBridge`,
//! `IsRegular`, `startCVS`) and is **not** written — the writer validates those derived
//! fields are exactly what the § 5.18.2 derivation would produce before emitting any bit,
//! so a model the parser could not have produced is rejected (reject-before-write) rather
//! than silently re-encoded.
//!
//! This is the foundation slice of the frame-header writer; the per-structure config
//! writers and the composing `write_frame_header` (which restricts to the modeled intra
//! path) build on it. The prefix itself is frame-type-agnostic, exactly like the parser:
//! it round-trips every prefix the parser produces (bridge, `cur_mfh_id == 0`, and
//! `cur_mfh_id > 0`).

use crate::headers::frame::{FrameHeaderPrefix, FrameHeaderPrefixStatus};
use crate::headers::sequence::SequenceHeaderId;
use crate::types::ObuType;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// Returns `keyFrame` for `obu_type` per AV2 v1.0.0 § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`). Mirrors the private
/// `crate::headers::frame::derive_key_frame`; the all-`obu_type` round-trip test guards
/// against drift.
fn derive_key_frame(obu_type: ObuType) -> bool {
    matches!(obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey)
}

/// Returns `IsRegular` for `obu_type` per AV2 v1.0.0 § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`). Mirrors the private
/// `crate::headers::frame::derive_is_regular`.
fn derive_is_regular(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::OpenLoopKey
            | ObuType::RegularTileGroup
            | ObuType::RegularTip
            | ObuType::RegularSef
            | ObuType::Switch
            | ObuType::RasFrame
            | ObuType::BridgeFrame
    )
}

/// Validates that `prefix` is a [`FrameHeaderPrefix`] the § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) parser could have produced,
/// before any bit is written.
///
/// The parser derives `isFirst`, `isKeyFrame`, `IsBridge`, `IsRegular`, and (for non-CLK
/// types) `startCVS` from `obu_type`, and reads `cur_mfh_id` / `seq_header_id_in_frame_header`
/// only on the branches described in the module docs. A model whose derived fields or
/// `Option` presence disagrees with that derivation is non-canonical and is rejected with a
/// typed [`WriteError::NonCanonicalFrameHeader`].
pub(crate) fn check_frame_header_prefix_encodable(prefix: &FrameHeaderPrefix) -> WriteResult<()> {
    let reject = |what: &'static str| Err(WriteError::NonCanonicalFrameHeader { what });

    // The prefix parser only produces the first-header path and stops at the activation
    // fields, so these two are invariant.
    if !prefix.is_first {
        return reject("is_first");
    }
    if prefix.status != FrameHeaderPrefixStatus::ActivationFieldsOnly {
        return reject("status");
    }

    // The `is_*` flags are derived from `obu_type`; reject a model that stores anything else.
    if prefix.is_key_frame != derive_key_frame(prefix.obu_type) {
        return reject("is_key_frame");
    }
    if prefix.is_bridge != (prefix.obu_type == ObuType::BridgeFrame) {
        return reject("is_bridge");
    }
    if prefix.is_regular != derive_is_regular(prefix.obu_type) {
        return reject("is_regular");
    }
    // `startCVS = obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU`. The derivation
    // consults `FirstPictureInTU` only for a CLK, so every non-CLK type is `Some(false)`;
    // a CLK may be `Some(true)`, `Some(false)`, or `None` (the input was withheld), none of
    // which affects a written bit.
    if prefix.obu_type != ObuType::ClosedLoopKey && prefix.starts_cvs != Some(false) {
        return reject("starts_cvs");
    }

    // A bridge frame infers `cur_mfh_id = 0`; a stored non-zero value could not have been
    // produced by the parser's bridge branch.
    if prefix.is_bridge && !prefix.cur_mfh_id.is_zero() {
        return reject("cur_mfh_id");
    }

    // `seq_header_id_in_frame_header` is present iff `cur_mfh_id == 0`, and
    // `referenced_sequence_header_id` is its in-range resolution. Accumulate the exact bit
    // count the written fields will occupy so the derived `consumed_bits` can be validated
    // against it. A `uvlc` value of `u32::MAX` is unencodable (`read_uvlc` maxes at
    // `u32::MAX - 1`), so reject it up front rather than let the second `write_uvlc` fail
    // mid-write and leave a partial buffer.
    let mut expected_bits: u64 = 0;
    if !prefix.is_bridge {
        let cur = prefix.cur_mfh_id.get();
        if cur == u32::MAX {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(cur),
            });
        }
        expected_bits += uvlc_bit_len(cur);
    }
    if prefix.cur_mfh_id.is_zero() {
        let Some(raw) = prefix.seq_header_id_in_frame_header else {
            return reject("seq_header_id_in_frame_header");
        };
        if prefix.referenced_sequence_header_id != SequenceHeaderId::try_new(raw) {
            return reject("referenced_sequence_header_id");
        }
        if raw == u32::MAX {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(raw),
            });
        }
        expected_bits += uvlc_bit_len(raw);
    } else {
        if prefix.seq_header_id_in_frame_header.is_some() {
            return reject("seq_header_id_in_frame_header");
        }
        if prefix.referenced_sequence_header_id.is_some() {
            return reject("referenced_sequence_header_id");
        }
    }

    // `consumed_bits` is the derived bit count of the activation fields; a model whose stored
    // value disagrees with the syntax it carries is not parser-reachable and would reparse to
    // a different prefix, so reject it before any bit.
    if prefix.consumed_bits != expected_bits {
        return reject("consumed_bits");
    }

    Ok(())
}

/// Returns the bit length of `value` encoded as `uvlc()` (AV2 v1.0.0 § 4.11.4,
/// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-4`): `2 * leadingZeros + 1` where
/// `leadingZeros = floor(log2(value + 1))`. `value` must be `< u32::MAX` (the encodable
/// domain), which the caller validates before calling.
fn uvlc_bit_len(value: u32) -> u64 {
    let m = u64::from(value) + 1;
    let leading_zeros = 63 - m.leading_zeros() as u64;
    2 * leading_zeros + 1
}

/// Writes the § 5.18.2 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`)
/// frame-header activation prefix, the exact inverse of
/// [`parse_frame_header_prefix`](crate::headers::frame::parse_frame_header_prefix).
///
/// Field writes (in § 5.18.2 read order): `cur_mfh_id` `uvlc` (omitted, inferred `0`, for a
/// bridge frame); then, when `cur_mfh_id == 0`, `seq_header_id_in_frame_header` `uvlc`. The
/// derived `is_*` / `startCVS` fields carry no bits. The model is fully validated before any
/// bit is written.
///
/// Both errors are returned before any bit is written (the writer buffer is left unchanged).
///
/// # Errors
/// - [`WriteError::NonCanonicalFrameHeader`] if the prefix is not a model the § 5.18.2 parser
///   could have produced (a derived flag, `startCVS`, the bridge `cur_mfh_id` inference, an
///   `Option` presence, or `consumed_bits` disagrees with the derivation).
/// - [`WriteError::ValueOutOfRange`] if `cur_mfh_id` or `seq_header_id_in_frame_header` is
///   `u32::MAX` (outside the `uvlc()` domain).
pub fn write_frame_header_prefix(
    writer: &mut BitWriter,
    prefix: &FrameHeaderPrefix,
) -> WriteResult<()> {
    check_frame_header_prefix_encodable(prefix)?;

    // A bridge frame infers cur_mfh_id = 0 (no bits); otherwise it is coded.
    if !prefix.is_bridge {
        writer.write_uvlc(prefix.cur_mfh_id.get())?;
    }
    // cur_mfh_id == 0 references a sequence header directly via seq_header_id_in_frame_header.
    if prefix.cur_mfh_id.is_zero() {
        // Presence guaranteed by check_frame_header_prefix_encodable.
        let raw =
            prefix
                .seq_header_id_in_frame_header
                .ok_or(WriteError::NonCanonicalFrameHeader {
                    what: "seq_header_id_in_frame_header",
                })?;
        writer.write_uvlc(raw)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::parse_frame_header_prefix;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn parse_prefix(bytes: &[u8], obu_type: ObuType, first_pic: Option<bool>) -> FrameHeaderPrefix {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_frame_header_prefix(&mut reader, obu_type, first_pic).unwrap()
    }

    fn write_prefix(prefix: &FrameHeaderPrefix) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_frame_header_prefix(&mut writer, prefix).unwrap();
        writer.into_bytes()
    }

    /// Builds the canonical prefix bytes for `(cur_mfh_id, seq_header_id?)`.
    fn prefix_bytes(is_bridge: bool, cur_mfh_id: u32, seq_header_id: Option<u32>) -> Vec<u8> {
        let mut bits = Bits::default();
        if !is_bridge {
            bits.uvlc(cur_mfh_id);
        }
        if cur_mfh_id == 0 {
            bits.uvlc(seq_header_id.expect("seq_header_id required when cur_mfh_id == 0"));
        }
        bits.into_bytes()
    }

    fn assert_roundtrip(obu_type: ObuType, first_pic: Option<bool>, bytes: &[u8]) {
        let prefix = parse_prefix(bytes, obu_type, first_pic);
        let written = write_prefix(&prefix);
        // Byte-exact over the consumed prefix (uvlc is canonical by construction).
        let consumed_bytes = prefix.consumed_bits.div_ceil(8) as usize;
        assert_eq!(
            &written[..],
            &bytes[..consumed_bytes],
            "{obu_type:?}: prefix not byte-exact"
        );
        // Semantic round-trip.
        let reparsed = parse_prefix(&written, obu_type, first_pic);
        assert_eq!(
            reparsed, prefix,
            "{obu_type:?}: parse(write(prefix)) != prefix"
        );
    }

    #[test]
    fn mfh_zero_round_trips_across_obu_types() {
        // cur_mfh_id == 0 + a seq_header_id, across every frame-bearing obu_type. The
        // derived flags differ by type, so this also guards the local derive_* mirrors.
        for obu_type in [
            ObuType::ClosedLoopKey,
            ObuType::OpenLoopKey,
            ObuType::RegularTileGroup,
            ObuType::RegularTip,
            ObuType::RegularSef,
            ObuType::LeadingSef,
            ObuType::Switch,
            ObuType::RasFrame,
        ] {
            let bytes = prefix_bytes(false, 0, Some(3));
            assert_roundtrip(obu_type, Some(true), &bytes);
            assert_roundtrip(obu_type, Some(false), &bytes);
        }
    }

    #[test]
    fn mfh_zero_clk_withheld_first_picture_round_trips() {
        // A CLK with FirstPictureInTU withheld -> starts_cvs == None (valid, no bit).
        let bytes = prefix_bytes(false, 0, Some(0));
        assert_roundtrip(ObuType::ClosedLoopKey, None, &bytes);
    }

    #[test]
    fn mfh_nonzero_round_trips() {
        // cur_mfh_id > 0 -> no seq_header_id; the prefix resolves the sequence header
        // through the MFH record, so seq_header_id_in_frame_header is None.
        let bytes = prefix_bytes(false, 5, None);
        assert_roundtrip(ObuType::RegularSef, Some(false), &bytes);
    }

    #[test]
    fn bridge_infers_mfh_zero_and_writes_no_cur_mfh_id() {
        // A bridge frame infers cur_mfh_id = 0 and codes only seq_header_id.
        let bytes = prefix_bytes(true, 0, Some(2));
        let prefix = parse_prefix(&bytes, ObuType::BridgeFrame, Some(false));
        assert!(prefix.is_bridge);
        assert!(prefix.cur_mfh_id.is_zero());
        let written = write_prefix(&prefix);
        assert_eq!(written, bytes, "bridge prefix not byte-exact");
        assert_eq!(
            parse_prefix(&written, ObuType::BridgeFrame, Some(false)),
            prefix
        );
    }

    #[test]
    fn seq_header_id_out_of_range_round_trips() {
        // seq_header_id_in_frame_header >= MAX_SEQ_NUM -> referenced id is None but the raw
        // value still round-trips.
        let bytes = prefix_bytes(false, 0, Some(16));
        let prefix = parse_prefix(&bytes, ObuType::ClosedLoopKey, Some(true));
        assert_eq!(prefix.seq_header_id_in_frame_header, Some(16));
        assert_eq!(prefix.referenced_sequence_header_id, None);
        assert_roundtrip(ObuType::ClosedLoopKey, Some(true), &bytes);
    }

    fn base_prefix() -> FrameHeaderPrefix {
        parse_prefix(
            &prefix_bytes(false, 0, Some(1)),
            ObuType::ClosedLoopKey,
            Some(true),
        )
    }

    fn assert_rejected(prefix: &FrameHeaderPrefix, what: &'static str) {
        let mut writer = BitWriter::new();
        let err = write_frame_header_prefix(&mut writer, prefix).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what },
            "expected reject {what}"
        );
        assert_eq!(writer.bit_len(), 0, "{what}: bits written on reject");
    }

    #[test]
    fn reject_inconsistent_is_key_frame() {
        let mut prefix = base_prefix();
        prefix.is_key_frame = false; // CLK derives true
        assert_rejected(&prefix, "is_key_frame");
    }

    #[test]
    fn reject_inconsistent_is_bridge() {
        let mut prefix = base_prefix();
        prefix.is_bridge = true; // CLK is not a bridge
        assert_rejected(&prefix, "is_bridge");
    }

    #[test]
    fn reject_inconsistent_is_regular() {
        let mut prefix = base_prefix();
        prefix.is_regular = true; // CLK is not regular
        assert_rejected(&prefix, "is_regular");
    }

    #[test]
    fn reject_non_clk_starts_cvs_true() {
        let mut prefix = parse_prefix(
            &prefix_bytes(false, 0, Some(1)),
            ObuType::RegularSef,
            Some(false),
        );
        prefix.starts_cvs = Some(true); // non-CLK must be Some(false)
        assert_rejected(&prefix, "starts_cvs");
    }

    #[test]
    fn reject_bridge_nonzero_cur_mfh_id() {
        let mut prefix = parse_prefix(
            &prefix_bytes(true, 0, Some(1)),
            ObuType::BridgeFrame,
            Some(false),
        );
        prefix.cur_mfh_id = crate::hls::MfhId::from_raw(2);
        assert_rejected(&prefix, "cur_mfh_id");
    }

    #[test]
    fn reject_mfh_zero_without_seq_header_id() {
        let mut prefix = base_prefix();
        prefix.seq_header_id_in_frame_header = None;
        assert_rejected(&prefix, "seq_header_id_in_frame_header");
    }

    #[test]
    fn reject_mfh_nonzero_with_seq_header_id() {
        let mut prefix = parse_prefix(
            &prefix_bytes(false, 4, None),
            ObuType::RegularSef,
            Some(false),
        );
        prefix.seq_header_id_in_frame_header = Some(1);
        assert_rejected(&prefix, "seq_header_id_in_frame_header");
    }

    #[test]
    fn reject_referenced_id_mismatch() {
        let mut prefix = base_prefix();
        // base raw seq_header_id is 1 -> referenced should be try_new(1); store a wrong one.
        prefix.referenced_sequence_header_id = SequenceHeaderId::try_new(2);
        assert_rejected(&prefix, "referenced_sequence_header_id");
    }

    #[test]
    fn reject_non_activation_status() {
        let mut prefix = base_prefix();
        prefix.status = FrameHeaderPrefixStatus::CompleteForSpecialCase;
        assert_rejected(&prefix, "status");
    }

    #[test]
    fn reject_consumed_bits_mismatch() {
        // consumed_bits must equal the bit length of the activation fields; a stored value
        // that disagrees with the syntax would reparse to a different prefix.
        let mut prefix = base_prefix();
        prefix.consumed_bits += 1;
        assert_rejected(&prefix, "consumed_bits");
    }

    #[test]
    fn reject_cur_mfh_id_u32_max_before_any_bit() {
        // u32::MAX is unencodable by uvlc; reject before any bit (not mid-write).
        let mut prefix = parse_prefix(
            &prefix_bytes(false, 4, None),
            ObuType::RegularSef,
            Some(false),
        );
        prefix.cur_mfh_id = crate::hls::MfhId::from_raw(u32::MAX);
        let mut writer = BitWriter::new();
        let err = write_frame_header_prefix(&mut writer, &prefix).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(u32::MAX)
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn reject_seq_header_id_u32_max_before_any_bit() {
        // The second uvlc would otherwise fail only after cur_mfh_id was written, leaving a
        // partial buffer; the up-front check rejects it before any bit.
        let mut prefix = base_prefix();
        prefix.seq_header_id_in_frame_header = Some(u32::MAX);
        prefix.referenced_sequence_header_id = SequenceHeaderId::try_new(u32::MAX); // None
        let mut writer = BitWriter::new();
        let err = write_frame_header_prefix(&mut writer, &prefix).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(u32::MAX)
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::parse_frame_header_prefix;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn obu_type(idx: u8) -> ObuType {
        match idx % 8 {
            0 => ObuType::ClosedLoopKey,
            1 => ObuType::OpenLoopKey,
            2 => ObuType::RegularTileGroup,
            3 => ObuType::RegularTip,
            4 => ObuType::RegularSef,
            5 => ObuType::LeadingSef,
            6 => ObuType::Switch,
            _ => ObuType::RasFrame,
        }
    }

    /// Appends `value` as MSB-first `uvlc()` bits to `bits`.
    fn push_uvlc(bits: &mut Vec<u8>, value: u32) {
        let code_num = value + 1;
        let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
        for _ in 0..leading_zeros {
            bits.push(0);
        }
        bits.push(1);
        for shift in (0..leading_zeros).rev() {
            bits.push((((code_num - (1 << leading_zeros)) >> shift) & 1) as u8);
        }
    }

    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, b) in chunk.iter().enumerate() {
                byte |= (*b & 1) << (7 - i);
            }
            out.push(byte);
        }
        out
    }

    proptest! {
        /// Every parser-reachable prefix round-trips (uvlc values up to the descriptor bound,
        /// both reference forms, every frame-bearing type).
        #[test]
        fn prefix_round_trips(
            type_idx in any::<u8>(),
            first_pic in proptest::option::of(any::<bool>()),
            cur_mfh_id in 0u32..4096,
            seq_header_id in 0u32..4096,
        ) {
            let kind = obu_type(type_idx);
            let mut bits = Vec::new();
            push_uvlc(&mut bits, cur_mfh_id);
            if cur_mfh_id == 0 {
                push_uvlc(&mut bits, seq_header_id);
            }
            let bytes = pack(&bits);
            let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
            if let Ok(prefix) = parse_frame_header_prefix(&mut reader, kind, first_pic) {
                let mut writer = BitWriter::new();
                write_frame_header_prefix(&mut writer, &prefix).unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed = parse_frame_header_prefix(&mut reparse, kind, first_pic).unwrap();
                prop_assert_eq!(reparsed, prefix);
            }
        }
    }
}
