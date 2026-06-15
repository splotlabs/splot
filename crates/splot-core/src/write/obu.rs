// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 OBU-header and Annex B framing writers — the inverse of the § 5.2.2 header
//! parser ([`crate::obu::read_obu_header_from_slice`];
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2`) and the Annex B OBU
//! envelope ([`crate::annexb`];
//! `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`)
//! (`AV2-5.2.2-OBU-HEADER`, with trailing bits in
//! [`crate::write::BitWriter::write_trailing_bits`] for `AV2-5.2.3-TRAILING-BITS`).
//!
//! This module is additive: it depends on the model/parser read-only and serializes
//! a parsed [`ObuHeader`] back to bytes via [`BitWriter`]. The universal contract is
//! semantic `read(write(x)) == x` for every header the parser can produce;
//! byte-exactness additionally holds for canonical encodings (a no-extension header
//! with parser-inferable layer ids, and a minimal-length Annex B size prefix).

use crate::obu::ObuHeader;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// Writes an OBU header (AV2 v1.0.0 § 5.2.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2`), the inverse of
/// [`crate::obu::read_obu_header_from_slice`]. MSB-first: one byte without the
/// extension, two bytes with it. No size prefix is written (Annex B framing is
/// [`write_annexb_obu`]).
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if the writer is not on a byte boundary
///   (the OBU header is byte-granular).
/// - [`WriteError::InconsistentHeader`] if `has_header_extension` disagrees with
///   `header_size_bytes` (the flag is `true` iff the header is two bytes).
/// - [`WriteError::NonInferableLayerIds`] if a no-extension header carries layer ids
///   the parser could never infer (`obu_mlayer_id != 0`, or `obu_xlayer_id` not equal
///   to the § 5.2.2 inference). Such ids cannot be represented without the extension
///   byte, so the writer rejects them instead of silently dropping them — keeping
///   `read(write(x)) == x` true for every header it accepts.
/// - [`WriteError::ValueTooWide`] if any field exceeds its bit width
///   (`obu_type` f(5), `obu_tlayer_id` f(2), or with the extension `obu_mlayer_id`
///   f(3) / `obu_xlayer_id` f(5)) — unreachable for a parser-produced header.
/// - [`WriteError::NonCanonicalObuType`] if `obu_type` is an `ObuType::Reserved(raw)`
///   the parser would reparse as a different variant.
///
/// All checks run before any byte is written, so a rejected header leaves the writer
/// unchanged.
pub fn write_obu_header(writer: &mut BitWriter, header: &ObuHeader) -> WriteResult<()> {
    // The OBU header is byte-granular and the parser reparses it from a byte boundary,
    // so a mid-byte write would not round-trip. Validate the writer state and the
    // header fully up front, before any byte is written.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    check_header_encodable(header)?;

    // Byte 0: obu_header_extension_flag f(1), obu_type f(5), obu_tlayer_id f(2).
    writer.write_bit(u8::from(header.has_header_extension))?;
    writer.write_bits_u8(header.obu_type.raw(), 5)?;
    writer.write_bits_u8(header.temporal_layer_id.get(), 2)?;

    if header.has_header_extension {
        // Byte 1: obu_mlayer_id f(3), obu_xlayer_id f(5).
        write_obu_header_extension(writer, header.embedded_layer_id, header.extended_layer_id)
    } else {
        Ok(())
    }
}

/// Returns `Ok(())` if `header` is one the § 5.2.2 parser could have produced — its
/// `has_header_extension` flag agrees with `header_size_bytes`, and (without the
/// extension) its layer ids equal the parser's inference. Checked before any bytes are
/// written so an unrepresentable header never leaves a partial encoding in the writer.
///
/// # Errors
/// [`WriteError::InconsistentHeader`] or [`WriteError::NonInferableLayerIds`].
fn check_header_encodable(header: &ObuHeader) -> WriteResult<()> {
    let expected_size = if header.has_header_extension { 2 } else { 1 };
    if header.header_size_bytes != expected_size {
        return Err(WriteError::InconsistentHeader {
            flag: header.has_header_extension,
            size_bytes: header.header_size_bytes,
        });
    }
    // Byte-0 field widths: obu_type f(5), obu_tlayer_id f(2). A model value built
    // outside the parser (e.g. `ObuType::Reserved(32)` or `TemporalLayerId::from_bits(4)`)
    // is rejected here, before any byte is written.
    check_field_width(header.obu_type.raw(), 5)?;
    check_field_width(header.temporal_layer_id.get(), 2)?;
    // Reject a non-canonical `obu_type` whose raw value the parser would map to a
    // different variant on reparse (e.g. `ObuType::Reserved(1)` reparses as a named
    // type), which would break `read(write(x)) == x`. A canonical type satisfies
    // `from_raw(raw) == type`.
    if ObuType::from_raw(header.obu_type.raw()) != header.obu_type {
        return Err(WriteError::NonCanonicalObuType {
            raw: header.obu_type.raw(),
        });
    }
    if header.has_header_extension {
        // Byte-1 field widths: obu_mlayer_id f(3), obu_xlayer_id f(5).
        check_field_width(header.embedded_layer_id.get(), 3)?;
        check_field_width(header.extended_layer_id.get(), 5)?;
    } else {
        // The parser re-infers the layer ids (obu.rs § 5.2.2), so a no-extension header
        // carrying ids it could never infer is unrepresentable in one byte.
        let inferred_xlayer = if header.obu_type.requires_global_xlayer() {
            GLOBAL_XLAYER_ID
        } else {
            ExtendedLayerId::from_bits(0)
        };
        if header.embedded_layer_id.get() != 0 || header.extended_layer_id != inferred_xlayer {
            return Err(WriteError::NonInferableLayerIds {
                embedded: header.embedded_layer_id.get(),
                extended: header.extended_layer_id.get(),
            });
        }
    }
    Ok(())
}

/// Returns `Ok(())` if `value` fits in `width_bits`, else [`WriteError::ValueTooWide`]
/// — the same bound `write_bits_u8` enforces, checked up front so a rejected header
/// never leaves a partial encoding in the writer.
fn check_field_width(value: u8, width_bits: u32) -> WriteResult<()> {
    if u32::from(value) >= (1u32 << width_bits) {
        return Err(WriteError::ValueTooWide {
            value: u64::from(value),
            width_bits,
        });
    }
    Ok(())
}

/// Writes the one-byte OBU extension (`obu_mlayer_id` f(3), `obu_xlayer_id` f(5);
/// AV2 v1.0.0 § 5.2.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2`). Called
/// by [`write_obu_header`] when the extension is present;
/// public for symmetry and direct testing.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if the writer is not on a byte boundary
///   (the extension is a whole byte).
/// - [`WriteError::ValueTooWide`] if `embedded_layer_id` exceeds 3 bits or
///   `extended_layer_id` exceeds 5 bits (unreachable for parser-produced values).
pub fn write_obu_header_extension(
    writer: &mut BitWriter,
    embedded_layer_id: EmbeddedLayerId,
    extended_layer_id: ExtendedLayerId,
) -> WriteResult<()> {
    // Validate the writer state and both fields up front so a rejected call leaves
    // the writer clean.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    check_field_width(embedded_layer_id.get(), 3)?;
    check_field_width(extended_layer_id.get(), 5)?;
    writer.write_bits_u8(embedded_layer_id.get(), 3)?;
    writer.write_bits_u8(extended_layer_id.get(), 5)
}

/// Computes an Annex B OBU's `num_bytes_in_obu` (`header_size_bytes + payload_len`),
/// or errors if it overflows the LEB128 `u32` size domain. Extracted so the overflow
/// path is unit-testable without allocating a `u32::MAX`-byte payload.
fn obu_total_len(header_size_bytes: u8, payload_len: usize) -> WriteResult<u32> {
    let total = u64::from(header_size_bytes).saturating_add(payload_len as u64);
    u32::try_from(total).map_err(|_| WriteError::ObuTooLarge { total })
}

/// Writes one Annex B framed OBU: `leb128(num_bytes_in_obu)` then the OBU header then
/// the payload bytes, where `num_bytes_in_obu == header_size_bytes + payload.len()`
/// (AV2 v1.0.0 Annex B § B.2,
/// `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`).
/// The inverse of one [`crate::annexb`] OBU envelope.
///
/// The size is emitted as canonical minimal-length LEB128, so byte-exact round-trip
/// holds only for inputs whose original size prefix was minimal; semantic round-trip
/// (`read(write(x)) == x`) always holds. The writer must be byte-aligned (Annex B OBUs
/// are byte-granular); the header and payload writes preserve that. All checks run
/// before any byte is written, so a rejected call leaves the writer unchanged.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if the writer is not on a byte boundary.
/// - [`WriteError::ObuTooLarge`] if `header_size_bytes + payload.len()` exceeds
///   `u32::MAX`.
/// - Propagates [`write_obu_header`] errors.
pub fn write_annexb_obu(
    writer: &mut BitWriter,
    header: &ObuHeader,
    payload: &[u8],
) -> WriteResult<()> {
    // Annex B OBUs are byte-granular; a mid-byte writer would mis-position the size
    // and header. Reject before writing rather than relying on a debug-only assert.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    // Validate the header before writing the size prefix so a rejected header leaves
    // no partial bytes (a stray LEB128 size) in the writer.
    check_header_encodable(header)?;
    let total = obu_total_len(header.header_size_bytes, payload.len())?;
    writer.write_leb128(total)?;
    write_obu_header(writer, header)?;
    writer.write_le(payload)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::annexb::parse_annex_b_obus_partial;
    use crate::obu::read_obu_header_from_slice;
    use crate::span::ByteOffset;
    use crate::types::{ObuType, TemporalLayerId};

    fn write_header(header: &ObuHeader) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_obu_header(&mut writer, header).unwrap();
        writer.into_bytes()
    }

    /// A + B: the four canonical parser vectors round-trip semantically and byte-exactly.
    #[test]
    fn canonical_headers_round_trip_semantic_and_byte_exact() {
        for bytes in [&[0x04u8][..], &[0x99, 0x65][..], &[0x08][..], &[0x50][..]] {
            let header = read_obu_header_from_slice(bytes, ByteOffset::new(0)).unwrap();
            let written = write_header(&header);
            // Byte-exact for these canonical encodings.
            assert_eq!(written, bytes, "byte-exact for {bytes:02x?}");
            // Semantic: reparse equals the original.
            let reparsed = read_obu_header_from_slice(&written, ByteOffset::new(0)).unwrap();
            assert_eq!(reparsed, header);
        }
    }

    /// C: exhaustive header-byte sweep, every header the parser can produce.
    #[test]
    fn header_byte_sweep_round_trips() {
        // No-extension: every obu_type x tlayer, with parser-inferred layer ids.
        for type_raw in 0u8..32 {
            for tlayer in 0u8..4 {
                let byte0 = (type_raw << 2) | tlayer; // ext bit 0
                let header = read_obu_header_from_slice(&[byte0], ByteOffset::new(0)).unwrap();
                let reparsed =
                    read_obu_header_from_slice(&write_header(&header), ByteOffset::new(0)).unwrap();
                assert_eq!(reparsed, header, "no-ext type={type_raw} tlayer={tlayer}");
            }
        }
        // Extension: a representative type x every mlayer (3b) x xlayer (5b).
        for mlayer in 0u8..8 {
            for xlayer in 0u8..32 {
                let byte0 = 0x98u8; // 0b1_00110_00: ext=1, type=6, tlayer=0
                let byte1 = (mlayer << 5) | xlayer;
                let header =
                    read_obu_header_from_slice(&[byte0, byte1], ByteOffset::new(0)).unwrap();
                let reparsed =
                    read_obu_header_from_slice(&write_header(&header), ByteOffset::new(0)).unwrap();
                assert_eq!(reparsed, header, "ext mlayer={mlayer} xlayer={xlayer}");
            }
        }
    }

    /// E: Annex B framing round-trips byte-exactly and reparses for the canonical subset.
    #[test]
    fn annexb_framing_round_trips() {
        let td = read_obu_header_from_slice(&[0x08], ByteOffset::new(0)).unwrap();
        let seq = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        let ext = read_obu_header_from_slice(&[0x99, 0x65], ByteOffset::new(0)).unwrap();

        for (header, payload, expected) in [
            (&td, &[][..], vec![0x01, 0x08]),
            (&seq, &[0xAB][..], vec![0x02, 0x04, 0xAB]),
            (&ext, &[][..], vec![0x02, 0x99, 0x65]),
        ] {
            let mut writer = BitWriter::new();
            write_annexb_obu(&mut writer, header, payload).unwrap();
            let bytes = writer.into_bytes();
            assert_eq!(bytes, expected, "byte-exact framing");
            let parsed = parse_annex_b_obus_partial(&bytes);
            assert!(parsed.error.is_none());
            assert_eq!(parsed.obus.len(), 1);
            assert_eq!(parsed.obus[0].header, *header);
            assert_eq!(parsed.obus[0].payload, payload);
        }
    }

    /// F: error paths — inconsistent header, non-inferable ids, oversize OBU.
    #[test]
    fn rejects_invalid_headers_and_oversize() {
        let mut writer = BitWriter::new();
        // has_header_extension=true but header_size_bytes=1.
        let bad = ObuHeader {
            has_header_extension: true,
            obu_type: ObuType::SequenceHeader,
            temporal_layer_id: TemporalLayerId::from_bits(0),
            embedded_layer_id: EmbeddedLayerId::from_bits(0),
            extended_layer_id: ExtendedLayerId::from_bits(0),
            header_size_bytes: 1,
        };
        assert!(matches!(
            write_obu_header(&mut writer, &bad),
            Err(WriteError::InconsistentHeader {
                flag: true,
                size_bytes: 1
            })
        ));

        // No-extension SequenceHeader with a non-inferable xlayer (infers 0).
        let bad_ids = ObuHeader {
            has_header_extension: false,
            obu_type: ObuType::SequenceHeader,
            temporal_layer_id: TemporalLayerId::from_bits(0),
            embedded_layer_id: EmbeddedLayerId::from_bits(0),
            extended_layer_id: ExtendedLayerId::from_bits(5),
            header_size_bytes: 1,
        };
        assert!(matches!(
            write_obu_header(&mut BitWriter::new(), &bad_ids),
            Err(WriteError::NonInferableLayerIds {
                embedded: 0,
                extended: 5
            })
        ));

        // obu_total_len overflow (no giant allocation needed).
        assert_eq!(obu_total_len(1, 0).unwrap(), 1);
        assert!(matches!(
            obu_total_len(1, u32::MAX as usize),
            Err(WriteError::ObuTooLarge { .. })
        ));

        // The error path validates before writing, so the writer is left clean
        // (no partial OBU-header byte or stray LEB128 size prefix).
        let mut clean = BitWriter::new();
        assert!(write_obu_header(&mut clean, &bad_ids).is_err());
        assert_eq!(clean.bit_len(), 0);
        let mut clean_framed = BitWriter::new();
        assert!(write_annexb_obu(&mut clean_framed, &bad, &[0xAB]).is_err());
        assert_eq!(clean_framed.bit_len(), 0);

        // Field values built outside the parser that overflow their bit width
        // (obu_tlayer_id f(2) here, obu_xlayer_id f(5) below) are rejected up front
        // with ValueTooWide, also leaving the writer clean.
        let wide_tlayer = ObuHeader {
            has_header_extension: false,
            obu_type: ObuType::SequenceHeader,
            temporal_layer_id: TemporalLayerId::from_bits(4),
            embedded_layer_id: EmbeddedLayerId::from_bits(0),
            extended_layer_id: ExtendedLayerId::from_bits(0),
            header_size_bytes: 1,
        };
        let mut w_tl = BitWriter::new();
        assert!(matches!(
            write_obu_header(&mut w_tl, &wide_tlayer),
            Err(WriteError::ValueTooWide { .. })
        ));
        assert_eq!(w_tl.bit_len(), 0);

        let wide_xlayer = ObuHeader {
            has_header_extension: true,
            obu_type: ObuType::SequenceHeader,
            temporal_layer_id: TemporalLayerId::from_bits(0),
            embedded_layer_id: EmbeddedLayerId::from_bits(0),
            extended_layer_id: ExtendedLayerId::from_bits(32),
            header_size_bytes: 2,
        };
        let mut w_xl = BitWriter::new();
        assert!(matches!(
            write_annexb_obu(&mut w_xl, &wide_xlayer, &[]),
            Err(WriteError::ValueTooWide { .. })
        ));
        assert_eq!(w_xl.bit_len(), 0);

        // A non-canonical `ObuType::Reserved(raw)` whose raw value reparses as a
        // different variant is rejected (would otherwise break read(write(x)) == x).
        let aliased = ObuHeader {
            has_header_extension: false,
            obu_type: ObuType::Reserved(1), // raw 1 reparses as SequenceHeader
            temporal_layer_id: TemporalLayerId::from_bits(0),
            embedded_layer_id: EmbeddedLayerId::from_bits(0),
            extended_layer_id: ExtendedLayerId::from_bits(0),
            header_size_bytes: 1,
        };
        let mut w_alias = BitWriter::new();
        assert!(matches!(
            write_obu_header(&mut w_alias, &aliased),
            Err(WriteError::NonCanonicalObuType { raw: 1 })
        ));
        assert_eq!(w_alias.bit_len(), 0);

        // A non-byte-aligned writer is rejected before any framing bytes are written.
        let td = read_obu_header_from_slice(&[0x08], ByteOffset::new(0)).unwrap();
        let mut unaligned = BitWriter::new();
        unaligned.write_bit(1).unwrap();
        assert!(matches!(
            write_annexb_obu(&mut unaligned, &td, &[]),
            Err(WriteError::WriterNotByteAligned)
        ));
        assert_eq!(unaligned.bit_len(), 1); // unchanged — the framer wrote nothing

        // A direct `write_obu_header` on an unaligned writer is rejected too.
        let mut unaligned_h = BitWriter::new();
        unaligned_h.write_bit(1).unwrap();
        assert!(matches!(
            write_obu_header(&mut unaligned_h, &td),
            Err(WriteError::WriterNotByteAligned)
        ));
        assert_eq!(unaligned_h.bit_len(), 1);
    }

    /// G: non-canonical caveat — a non-minimal LEB128 size re-emits canonically
    /// (different bytes) but is semantically equal on reparse.
    #[test]
    fn non_canonical_size_reemits_canonically() {
        // [0x81, 0x00] is a non-minimal LEB128 encoding of size 1; header 0x08 (TD).
        let input = [0x81u8, 0x00, 0x08];
        let parsed = parse_annex_b_obus_partial(&input);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        let header = parsed.obus[0].header;
        let payload = parsed.obus[0].payload.to_vec();

        let mut writer = BitWriter::new();
        write_annexb_obu(&mut writer, &header, &payload).unwrap();
        let reemitted = writer.into_bytes();

        // Byte-exact does NOT hold (canonical minimal form differs)...
        assert_ne!(reemitted, input);
        assert_eq!(reemitted, vec![0x01, 0x08]);
        // ...but the semantic round-trip does.
        let reparsed = parse_annex_b_obus_partial(&reemitted);
        assert_eq!(reparsed.obus[0].header, header);
        assert_eq!(reparsed.obus[0].payload, &payload[..]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::annexb::parse_annex_b_obus_partial;
    use crate::obu::read_obu_header_from_slice;
    use crate::span::ByteOffset;
    use crate::types::{ObuType, TemporalLayerId};
    use proptest::prelude::*;

    /// Builds an `ObuHeader` exactly as the § 5.2.2 parser would, so it is always
    /// representable by [`write_obu_header`].
    fn parser_faithful_header(
        ext: bool,
        type_raw: u8,
        tlayer: u8,
        mlayer: u8,
        xlayer: u8,
    ) -> ObuHeader {
        let obu_type = ObuType::from_raw(type_raw);
        let (embedded, extended, size) = if ext {
            (mlayer, xlayer, 2)
        } else {
            let inferred = if obu_type.requires_global_xlayer() {
                GLOBAL_XLAYER_ID.get()
            } else {
                0
            };
            (0, inferred, 1)
        };
        ObuHeader {
            has_header_extension: ext,
            obu_type,
            temporal_layer_id: TemporalLayerId::from_bits(tlayer),
            embedded_layer_id: EmbeddedLayerId::from_bits(embedded),
            extended_layer_id: ExtendedLayerId::from_bits(extended),
            header_size_bytes: size,
        }
    }

    proptest! {
        /// Every parser-producible header round-trips through write -> reparse.
        #[test]
        fn roundtrip_obu_header(
            ext in any::<bool>(),
            type_raw in 0u8..32,
            tlayer in 0u8..4,
            mlayer in 0u8..8,
            xlayer in 0u8..32,
        ) {
            let header = parser_faithful_header(ext, type_raw, tlayer, mlayer, xlayer);
            let mut writer = BitWriter::new();
            write_obu_header(&mut writer, &header).unwrap();
            let bytes = writer.into_bytes();
            let parsed = read_obu_header_from_slice(&bytes, ByteOffset::new(0)).unwrap();
            prop_assert_eq!(parsed, header);
        }

        /// Every framed OBU round-trips through write -> Annex B parse (header + payload).
        #[test]
        fn roundtrip_annexb_obu(
            ext in any::<bool>(),
            type_raw in 0u8..32,
            tlayer in 0u8..4,
            mlayer in 0u8..8,
            xlayer in 0u8..32,
            payload in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let header = parser_faithful_header(ext, type_raw, tlayer, mlayer, xlayer);
            let mut writer = BitWriter::new();
            write_annexb_obu(&mut writer, &header, &payload).unwrap();
            let bytes = writer.into_bytes();
            let parsed = parse_annex_b_obus_partial(&bytes);
            prop_assert!(parsed.error.is_none());
            prop_assert_eq!(parsed.obus.len(), 1);
            prop_assert_eq!(parsed.obus[0].header, header);
            prop_assert_eq!(parsed.obus[0].payload, &payload[..]);
        }

        /// The OBU writers never panic on arbitrary header fields — they return `Result`.
        #[test]
        fn obu_writers_never_panic(
            ext in any::<bool>(),
            type_raw in any::<u8>(),
            tlayer in any::<u8>(),
            mlayer in any::<u8>(),
            xlayer in any::<u8>(),
            size in any::<u8>(),
            payload in proptest::collection::vec(any::<u8>(), 0..16),
        ) {
            let header = ObuHeader {
                has_header_extension: ext,
                obu_type: ObuType::from_raw(type_raw & 0x1f),
                temporal_layer_id: TemporalLayerId::from_bits(tlayer & 0x03),
                embedded_layer_id: EmbeddedLayerId::from_bits(mlayer),
                extended_layer_id: ExtendedLayerId::from_bits(xlayer),
                header_size_bytes: size,
            };
            let mut writer = BitWriter::new();
            let _ = write_obu_header(&mut writer, &header);
            let mut framer = BitWriter::new();
            let _ = write_annexb_obu(&mut framer, &header, &payload);
        }
    }
}
