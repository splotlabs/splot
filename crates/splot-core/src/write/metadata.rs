// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 metadata-OBU **writers** (`ENC-BITSTREAM-WRITER`) — the exact inverses of the
//! § 5.17 metadata parsers in [`crate::headers::metadata`]:
//!
//! - [`write_metadata_short_obu`] — `metadata_short_obu()` (AV2 v1.0.0 § 5.17.2,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-2`): the 1-byte `muh_*` header, the
//!   `metadata_type` `leb128()` (reproduced byte-exactly), and (unless cancelled) the bounded
//!   `metadata_unit()`.
//! - [`write_metadata_group_obu`] — `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`): the 1-byte group header, the
//!   `metadata_unit_cnt_minus_1` `leb128()`, and each per-unit header + bounded
//!   `metadata_unit()` with layer targeting and priority.
//! - [`write_metadata_unit`] — `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-1`) plus the § 6.16.1
//!   `metadata_unit_remaining_bit` zero padding to the declared size.
//! - [`write_metadata_payload`] — the typed § 5.17.4 – § 5.17.13 child payloads, selected by
//!   `metadata_type`.
//!
//! Like the other writers this module is additive: it depends on the model/parser read-only and
//! serializes a parsed structure back to bits via [`BitWriter`]. The composing writers
//! (`metadata_short_obu()`, `metadata_group_obu()`, and each unit) draft into a local scratch
//! [`BitWriter`] and only [`BitWriter::append`] to the caller on full success, so a mid-composition
//! reject never touches the caller (reject-before-write: every reject path leaves
//! `writer.bit_len()` unchanged). The OBU `trailing_bits()` / `obuPayloadSize` are *not* written
//! here — that is the OBU writer's job; these emit only the metadata-OBU payload content.
//!
//! The length-summarized payloads (ITU-T T.35, ICC, user data, and reserved/unknown raw) are not
//! fully modeled by the parser, so their opaque bytes are supplied separately via a `passthrough`
//! slice and re-emitted verbatim; the fully-modeled payloads and the cancel arms require an empty
//! `passthrough`.

use crate::headers::metadata::{
    BandUnits, BandingComponent, BandingHintsDetail, MetadataBandingHints,
    MetadataDecodedFrameHash, MetadataGroupObu, MetadataGroupUnit, MetadataHdrMdcv,
    MetadataItutT35, MetadataPayload, MetadataScanType, MetadataShortObu, MetadataTimecode,
    MetadataType, MetadataUnit, VaryingBandUnits,
};
use crate::types::ExtendedLayerId;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `muh_layer_idc` value `LAYER_VALUES` (AV2 § 6.16.3): the metadata applies to an explicitly
/// signaled set of layer values, so the layer maps are present.
const LAYER_VALUES: u8 = 3;
/// `muh_layer_idc` / `muh_persistence_idc` are `f(3)` (AV2 § 5.17.2 / § 5.17.3), so they fit `0..8`.
const F3_MAX_PLUS_1: u8 = 8;
/// `muh_reserved_zero_2bits` is `f(2)` and `metadata_necessity_idc` is `f(2)` — both fit `0..4`.
const F2_MAX_PLUS_1: u8 = 4;
/// `metadata_application_id` is `f(5)`, so it fits `0..32`.
const F5_MAX_PLUS_1: u8 = 32;
/// `muh_header_size` is `f(7)`, so it fits `0..128`.
const MUH_HEADER_SIZE_MAX_PLUS_1: u8 = 128;
/// `metadata_unit_cnt_minus_1` must be `< 16383` (AV2 § 5.17.3), the parser's bound.
const METADATA_UNIT_CNT_MAX: u32 = 16383;

/// Writes `value` as a `leb128()` (AV2 v1.0.0 § 4.11.6) occupying EXACTLY `len` bytes (`1..=8`),
/// padding a minimal encoding with continuation groups so a non-minimal parsed encoding
/// round-trips byte-exactly. Byte `i` (0-based) is `((value >> (7*i)) & 0x7f)`, OR'd with `0x80`
/// for every byte but the last; [`crate::bitio::BitReader::read_leb128`] then reparses to `value`
/// consuming exactly `len` bytes.
///
/// # Errors
/// Returns [`WriteError::NonCanonicalMetadata`] with the given `what` label if `len == 0`,
/// `len > 8`, or `len < minimal_leb_len(value)` (the value needs more 7-bit groups than `len`).
fn write_leb128_with_len(
    writer: &mut BitWriter,
    value: u32,
    len: usize,
    what: &'static str,
) -> WriteResult<()> {
    if len == 0 || len > 8 || len < minimal_leb_len(value) {
        return Err(WriteError::NonCanonicalMetadata { what });
    }
    for i in 0..len {
        // i < len <= 8 and i < 5 keeps 7*i < 32, so the shift never exceeds a u32 width; for
        // i >= 5 the high padding groups encode zero (value >> 35 == 0).
        let shift = (7u32).saturating_mul(i as u32);
        let group = if shift >= 32 {
            0u8
        } else {
            ((value >> shift) & 0x7f) as u8
        };
        let byte = if i < len - 1 { group | 0x80 } else { group };
        writer.write_bits_u8(byte, 8)?;
    }
    Ok(())
}

/// Number of 7-bit groups a minimal `leb128()` encoding of `value` occupies (`1..=5`).
fn minimal_leb_len(value: u32) -> usize {
    let mut remaining = value;
    let mut len = 1usize;
    while remaining >= 0x80 {
        remaining >>= 7;
        len += 1;
    }
    len
}

/// Rejects a non-canonical `metadata_type` (AV2 § 6.16, Table 6.17): a
/// [`MetadataType::Reserved`]`(v)` whose `v` the parser's [`MetadataType::from_value`] re-maps to a
/// NAMED variant (`v` in `1..=10`) could never have been produced by the parser, and emitting its
/// `leb128(v)` would reparse as that named type — breaking `read(write(x)) == x`. Mirrors the
/// § 5.2.2 [`WriteError::NonCanonicalObuType`](crate::write::error::WriteError::NonCanonicalObuType)
/// guard. Checked before any bit is written.
fn check_canonical_metadata_type(metadata_type: MetadataType) -> WriteResult<()> {
    if MetadataType::from_value(metadata_type.value()) != metadata_type {
        return Err(WriteError::NonCanonicalMetadata {
            what: "metadata_type_canonical",
        });
    }
    Ok(())
}

/// Writes `metadata_short_obu()` (AV2 v1.0.0 § 5.17.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-2`), the inverse of
/// [`crate::headers::metadata::parse_metadata_short`].
///
/// Writes the 1-byte `muh_*` header, the `metadata_type` `leb128()` reproduced from the stored
/// `metadata_type_leb128_bytes`, and — unless `muh_cancel_flag` — the bounded `metadata_unit()`
/// with the same `passthrough`. The OBU `trailing_bits()` are the OBU writer's job. Drafted into a
/// scratch [`BitWriter`] and appended only on full success, so a reject leaves `writer` untouched.
///
/// `passthrough` is the opaque length-summarized payload bytes for the unit (for the
/// length-summarized payloads); it must be empty for a cancelled OBU and for fully-modeled
/// payloads.
///
/// # Errors
/// [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary;
/// [`WriteError::NonCanonicalMetadata`]: a `muh_layer_idc` / `muh_persistence_idc` outside its
/// `f(3)` field (`muh_field_domain`); a non-canonical `metadata_type` (`metadata_type_canonical`);
/// a `unit` whose presence disagrees with `muh_cancel_flag` (`short_cancel_unit`); a non-empty
/// `passthrough` on a cancelled OBU (`passthrough_len`); a `metadata_type_leb128_bytes` that cannot
/// encode the value (`metadata_type_leb_len`); or any [`write_metadata_unit`] reject. Never writes a
/// bit on error.
pub fn write_metadata_short_obu(
    writer: &mut BitWriter,
    obu: &MetadataShortObu,
    passthrough: &[u8],
) -> WriteResult<()> {
    // The metadata OBU payload begins at a byte boundary (the § 5.17 parser reads it byte-aligned);
    // a mid-byte writer would mis-position every following byte. Matches the §5.4 OBU writers.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    if obu.muh_layer_idc >= F3_MAX_PLUS_1 || obu.muh_persistence_idc >= F3_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_field_domain",
        });
    }
    // § 6.16 Table 6.17: a Reserved value the parser would re-map to a named type is unwritable.
    check_canonical_metadata_type(obu.metadata_type)?;
    if obu.muh_cancel_flag {
        // § 5.17.2: a cancelled OBU carries no unit, so there is nothing to summarize.
        if obu.unit.is_some() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "short_cancel_unit",
            });
        }
        if !passthrough.is_empty() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "passthrough_len",
            });
        }
    } else if obu.unit.is_none() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "short_cancel_unit",
        });
    }

    let mut scratch = BitWriter::new();
    // § 5.17.2: the 1-byte header, MSB-first.
    scratch.write_bit(u8::from(obu.metadata_is_suffix))?;
    scratch.write_bits_u8(obu.muh_layer_idc, 3)?;
    scratch.write_bit(u8::from(obu.muh_cancel_flag))?;
    scratch.write_bits_u8(obu.muh_persistence_idc, 3)?;
    // § 5.17.2: metadata_type leb128(), reproduced byte-exactly from metadata_type_leb128_bytes.
    write_leb128_with_len(
        &mut scratch,
        obu.metadata_type.value(),
        usize::from(obu.metadata_type_leb128_bytes),
        "metadata_type_leb_len",
    )?;
    if let Some(unit) = obu.unit.as_ref() {
        // The metadata_type the unit dispatches on must match the OBU's metadata_type.
        if unit.metadata_type != obu.metadata_type {
            return Err(WriteError::NonCanonicalMetadata {
                what: "type_payload_mismatch",
            });
        }
        write_metadata_unit(&mut scratch, unit, passthrough)?;
    }
    writer.append(&scratch)
}

/// Writes `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`), the inverse of
/// [`crate::headers::metadata::parse_metadata_group`].
///
/// Writes the 1-byte group header, the `metadata_unit_cnt_minus_1` `leb128()` (minimal — its byte
/// count is not modeled), and each per-unit header + bounded `metadata_unit()`. `obu_xlayer_id`
/// selects the global-vs-local layer-map branch exactly as the parser does. `passthrough[i]` is the
/// opaque slice for unit `i`; `passthrough.len()` must equal `obu.units.len()`. Drafted into a
/// scratch [`BitWriter`] and appended only on full success.
///
/// # Errors
/// [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary;
/// [`WriteError::NonCanonicalMetadata`]: a `passthrough` length that disagrees with the unit count
/// (`group_passthrough_count`); an empty or over-large unit count (`group_unit_count`); a
/// `metadata_necessity_idc` / `metadata_application_id` outside its field (`group_header_domain`);
/// or any per-unit reject. Never writes a bit on error.
pub fn write_metadata_group_obu(
    writer: &mut BitWriter,
    obu: &MetadataGroupObu,
    obu_xlayer_id: ExtendedLayerId,
    passthrough: &[&[u8]],
) -> WriteResult<()> {
    // The metadata OBU payload begins at a byte boundary (the § 5.17 parser reads it byte-aligned);
    // a mid-byte writer would mis-position every following byte. Matches the §5.4 OBU writers.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    if passthrough.len() != obu.units.len() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_passthrough_count",
        });
    }
    // § 5.17.3: metadata_unit_cnt_minus_1 = units - 1; the parser bounds it to < 16383.
    if obu.units.is_empty() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_unit_count",
        });
    }
    let cnt_minus_1 = (obu.units.len() - 1) as u64;
    if cnt_minus_1 >= u64::from(METADATA_UNIT_CNT_MAX) {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_unit_count",
        });
    }
    if obu.metadata_necessity_idc >= F2_MAX_PLUS_1 || obu.metadata_application_id >= F5_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_header_domain",
        });
    }

    let mut scratch = BitWriter::new();
    // § 5.17.3: the 1-byte group header, MSB-first.
    scratch.write_bit(u8::from(obu.metadata_is_suffix))?;
    scratch.write_bits_u8(obu.metadata_necessity_idc, 2)?;
    scratch.write_bits_u8(obu.metadata_application_id, 5)?;
    // § 5.17.3: metadata_unit_cnt_minus_1 leb128() (minimal — its byte count is not modeled).
    scratch.write_leb128(cnt_minus_1 as u32)?;
    for (unit, &unit_passthrough) in obu.units.iter().zip(passthrough.iter()) {
        write_metadata_group_unit(&mut scratch, unit, obu_xlayer_id, unit_passthrough)?;
    }
    writer.append(&scratch)
}

/// The opaque `passthrough` byte length the metadata-group unit `unit` consumes when written by
/// [`write_metadata_group_obu`] (AV2 v1.0.0 § 5.17.3 / § 5.17.9 – § 5.17.13,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`): `0` for a cancelled unit (it carries no
/// `metadata_unit`) or a fully-modeled payload, else the length-summarized blob length (ITU-T T.35,
/// ICC, user-data, or unknown-raw). It mirrors the per-payload `require_passthrough_len` checks in
/// [`write_metadata_payload`], letting a caller holding one flat passthrough split it per unit;
/// [`write_metadata_group_obu_flat`] is that caller.
fn metadata_group_unit_passthrough_len(unit: &MetadataGroupUnit) -> usize {
    if unit.muh_cancel_flag {
        return 0;
    }
    match unit.unit.as_ref().map(|inner| &inner.payload) {
        Some(MetadataPayload::ItutT35(p)) => p.payload_len,
        Some(MetadataPayload::IccProfile(p)) => p.payload_len,
        Some(MetadataPayload::UserDataUnregistered(p)) => p.payload_len,
        Some(MetadataPayload::UnknownRaw(p)) => p.raw_len,
        _ => 0,
    }
}

/// Writes a `metadata_group_obu()` (AV2 v1.0.0 § 5.17.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`) from a single flat `passthrough` holding
/// every unit's length-summarized blob bytes concatenated in unit order — the form the unified OBU
/// dispatch ([`crate::write::write_complete_obu`]) holds, where the per-unit split is not available.
/// Splits `passthrough` per unit by each unit's modeled blob length (the private
/// `metadata_group_unit_passthrough_len`) and delegates to [`write_metadata_group_obu`], so a
/// multi-unit group (cancelled, fully-modeled, and length-summarized units in any mix) round-trips
/// without the caller pre-splitting.
///
/// # Errors
/// Every [`write_metadata_group_obu`] error, plus [`WriteError::NonCanonicalMetadata`] with
/// `what == "group_passthrough_len"` when the per-unit blob lengths do not sum to exactly
/// `passthrough.len()` (the flat blob does not match the modeled units). The split uses
/// `checked_add` + slicing bounds, so a constructed over-large blob length rejects rather than
/// panicking. Never writes a bit on error.
pub fn write_metadata_group_obu_flat(
    writer: &mut BitWriter,
    obu: &MetadataGroupObu,
    obu_xlayer_id: ExtendedLayerId,
    passthrough: &[u8],
) -> WriteResult<()> {
    // Split the flat passthrough into one slice per unit by each unit's modeled blob length, in
    // unit order. checked_add + the `end <= len` bound keep a constructed (over-large) length from
    // panicking; an exact-sum mismatch is rejected below.
    let mut slices: Vec<&[u8]> = Vec::with_capacity(obu.units.len());
    let mut offset = 0usize;
    for unit in &obu.units {
        let len = metadata_group_unit_passthrough_len(unit);
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= passthrough.len())
            .ok_or(WriteError::NonCanonicalMetadata {
                what: "group_passthrough_len",
            })?;
        slices.push(&passthrough[offset..end]);
        offset = end;
    }
    if offset != passthrough.len() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_passthrough_len",
        });
    }
    write_metadata_group_obu(writer, obu, obu_xlayer_id, &slices)
}

/// Writes one `metadata_group_obu()` per-unit header plus its `metadata_unit()` (AV2 v1.0.0
/// § 5.17.3), the inverse of `parse_metadata_group_unit`. The whole unit is validated and drafted
/// into a scratch [`BitWriter`] up front, so a reject leaves the outer (group) scratch untouched.
fn write_metadata_group_unit(
    writer: &mut BitWriter,
    unit: &MetadataGroupUnit,
    obu_xlayer_id: ExtendedLayerId,
    passthrough: &[u8],
) -> WriteResult<()> {
    if unit.muh_header_size >= MUH_HEADER_SIZE_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_header_size_domain",
        });
    }
    // § 6.16 Table 6.17: a Reserved value the parser would re-map to a named type is unwritable.
    check_canonical_metadata_type(unit.metadata_type)?;

    let mut scratch = BitWriter::new();
    // § 5.17.3: metadata_type leb128() (minimal — its byte count is not modeled in the group).
    scratch.write_leb128(unit.metadata_type.value())?;
    // § 5.17.3: header byte = (muh_header_size << 1) | muh_cancel_flag, MSB-first f(7) + f(1).
    scratch.write_bits_u8(unit.muh_header_size, 7)?;
    scratch.write_bit(u8::from(unit.muh_cancel_flag))?;

    if unit.muh_cancel_flag {
        // § 5.17.3: a cancelled unit carries no metadata_unit, so there is nothing to summarize;
        // reject supplied opaque bytes rather than silently dropping them (as the short OBU does).
        if !passthrough.is_empty() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "passthrough_len",
            });
        }
        write_group_unit_cancel(&mut scratch, unit)?;
    } else {
        write_group_unit_body(&mut scratch, unit, obu_xlayer_id, passthrough)?;
    }
    writer.append(&scratch)
}

/// Writes the cancelled `metadata_group_obu()` per-unit tail (AV2 v1.0.0 § 5.17.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`): all `muh_*`
/// fields and the unit are absent, and `muh_header_size` zero header-extension bytes follow.
fn write_group_unit_cancel(scratch: &mut BitWriter, unit: &MetadataGroupUnit) -> WriteResult<()> {
    // § 5.17.3: a cancelled unit carries none of the non-cancel fields.
    if unit.muh_payload_size.is_some()
        || unit.muh_layer_idc.is_some()
        || unit.muh_persistence_idc.is_some()
        || unit.muh_priority.is_some()
        || unit.muh_reserved_zero_2bits.is_some()
        || unit.muh_xlayer_map.is_some()
        || !unit.muh_mlayer_maps.is_empty()
        || unit.unit.is_some()
    {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_cancel_fields",
        });
    }
    // On cancel, headerRemainingBytes == muh_header_size (no payload_size / fixed bytes were
    // consumed), so header_extension_len must equal muh_header_size.
    if unit.header_extension_len != usize::from(unit.muh_header_size) {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_header_size",
        });
    }
    for _ in 0..unit.header_extension_len {
        scratch.write_bits_u8(0, 8)?;
    }
    Ok(())
}

/// Writes the non-cancel `metadata_group_obu()` per-unit body (AV2 v1.0.0 § 5.17.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3`): the
/// `muh_payload_size` `leb128()` padded to fill `muh_header_size`, the two fixed `muh_*` bytes,
/// the layer maps, the header-extension bytes, and the bounded `metadata_unit()`.
fn write_group_unit_body(
    scratch: &mut BitWriter,
    unit: &MetadataGroupUnit,
    obu_xlayer_id: ExtendedLayerId,
    passthrough: &[u8],
) -> WriteResult<()> {
    // § 5.17.3: every non-cancel muh_* field is present, as is the unit.
    let (
        Some(muh_payload_size),
        Some(muh_layer_idc),
        Some(muh_persistence_idc),
        Some(muh_priority),
        Some(muh_reserved_zero_2bits),
        Some(metadata_unit),
    ) = (
        unit.muh_payload_size,
        unit.muh_layer_idc,
        unit.muh_persistence_idc,
        unit.muh_priority,
        unit.muh_reserved_zero_2bits,
        unit.unit.as_ref(),
    )
    else {
        return Err(WriteError::NonCanonicalMetadata {
            what: "group_noncancel_fields",
        });
    };
    if muh_layer_idc >= F3_MAX_PLUS_1
        || muh_persistence_idc >= F3_MAX_PLUS_1
        || muh_reserved_zero_2bits >= F2_MAX_PLUS_1
    {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_field_domain",
        });
    }

    // § 6.16.3: layer maps depend on muh_layer_idc and the OBU scope; validate the modeled
    // xlayer/mlayer maps against the branch and derive their byte count.
    let layer_map_bytes = check_layer_maps(unit, muh_layer_idc, obu_xlayer_id)?;

    // headerRemainingBytes = muh_header_size - payload_size_bytes - 2 - layer_map_bytes (then the
    // header-extension bytes). Invert: payload_size_bytes = muh_header_size - 2 - layer_map_bytes -
    // header_extension_len. The result must be a valid leb128() length for muh_payload_size.
    let fixed = 2usize
        .checked_add(layer_map_bytes)
        .and_then(|v| v.checked_add(unit.header_extension_len));
    let payload_size_bytes = fixed
        .and_then(|f| usize::from(unit.muh_header_size).checked_sub(f))
        .ok_or(WriteError::NonCanonicalMetadata {
            what: "muh_header_size",
        })?;
    if !(1..=8).contains(&payload_size_bytes)
        || payload_size_bytes < minimal_leb_len(muh_payload_size)
    {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_payload_size_leb_len",
        });
    }
    // The bounded metadata_unit() occupies exactly muh_payload_size bytes and dispatches on the
    // unit's metadata_type, which must match.
    if metadata_unit.payload_size != muh_payload_size as usize {
        return Err(WriteError::NonCanonicalMetadata {
            what: "muh_payload_size",
        });
    }
    if metadata_unit.metadata_type != unit.metadata_type {
        return Err(WriteError::NonCanonicalMetadata {
            what: "type_payload_mismatch",
        });
    }

    // § 5.17.3 write order: muh_payload_size leb128() padded to payload_size_bytes ...
    write_leb128_with_len(
        scratch,
        muh_payload_size,
        payload_size_bytes,
        "muh_payload_size_leb_len",
    )?;
    // ... then the two fixed bytes (muh_layer_idc f(3), muh_persistence_idc f(3), muh_priority
    // f(8), muh_reserved_zero_2bits f(2)) ...
    scratch.write_bits_u8(muh_layer_idc, 3)?;
    scratch.write_bits_u8(muh_persistence_idc, 3)?;
    scratch.write_bits_u8(muh_priority, 8)?;
    scratch.write_bits_u8(muh_reserved_zero_2bits, 2)?;
    // ... then the layer maps ...
    write_layer_maps(scratch, unit, muh_layer_idc, obu_xlayer_id)?;
    // ... then header_extension_len zero bytes ...
    for _ in 0..unit.header_extension_len {
        scratch.write_bits_u8(0, 8)?;
    }
    // ... then the bounded metadata_unit().
    write_metadata_unit(scratch, metadata_unit, passthrough)
}

/// Validates the modeled `muh_xlayer_map` / `muh_mlayer_maps` against the § 6.16.3 layer-map branch
/// and returns the number of bytes they occupy in `muh_header_size`.
fn check_layer_maps(
    unit: &MetadataGroupUnit,
    muh_layer_idc: u8,
    obu_xlayer_id: ExtendedLayerId,
) -> WriteResult<usize> {
    if muh_layer_idc == LAYER_VALUES {
        if obu_xlayer_id.is_global() {
            // § 6.16.3: muh_xlayer_map f(32) then one muh_mlayer_map per set bit in 0..=30.
            let Some(xlayer_map) = unit.muh_xlayer_map else {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "layer_map_presence",
                });
            };
            let set_bits = (0..31u32).filter(|n| xlayer_map & (1 << n) != 0).count();
            if unit.muh_mlayer_maps.len() != set_bits {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "layer_map_count",
                });
            }
            // 4 (xlayer_map) + one byte per set bit; set_bits <= 31 keeps this small.
            Ok(4 + set_bits)
        } else {
            // § 6.16.3: a single muh_mlayer_map byte; no muh_xlayer_map.
            if unit.muh_xlayer_map.is_some() || unit.muh_mlayer_maps.len() != 1 {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "layer_map_count",
                });
            }
            Ok(1)
        }
    } else {
        // § 6.16.3: no layer maps when muh_layer_idc != LAYER_VALUES.
        if unit.muh_xlayer_map.is_some() || !unit.muh_mlayer_maps.is_empty() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "layer_map_count",
            });
        }
        Ok(0)
    }
}

/// Writes the § 6.16.3 layer maps in parser order (validated by [`check_layer_maps`]).
fn write_layer_maps(
    scratch: &mut BitWriter,
    unit: &MetadataGroupUnit,
    muh_layer_idc: u8,
    obu_xlayer_id: ExtendedLayerId,
) -> WriteResult<()> {
    if muh_layer_idc != LAYER_VALUES {
        return Ok(());
    }
    if obu_xlayer_id.is_global() {
        let xlayer_map = unit.muh_xlayer_map.unwrap_or(0);
        scratch.write_bits(xlayer_map, 32)?;
        // One muh_mlayer_map per set bit in 0..=30, in bit order (the order the parser pushed).
        let mut next = 0usize;
        for n in 0..31u32 {
            if xlayer_map & (1 << n) != 0 {
                let byte = unit.muh_mlayer_maps.get(next).copied().unwrap_or(0);
                scratch.write_bits_u8(byte, 8)?;
                next += 1;
            }
        }
    } else {
        let byte = unit.muh_mlayer_maps.first().copied().unwrap_or(0);
        scratch.write_bits_u8(byte, 8)?;
    }
    Ok(())
}

/// Writes `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-1`) plus the § 6.16.1
/// `metadata_unit_remaining_bit` zero padding to the declared `payload_size`, the inverse of
/// `parse_metadata_unit`.
///
/// The typed payload is drafted into a local scratch via [`write_metadata_payload`]; it must not
/// exceed `payload_size * 8` bits. The drafted payload is appended and the remaining bits up to the
/// declared size are written as zero padding (§ 6.16.1 "can take any value" — the writer emits the
/// canonical zero). Validated before any bit is written to the caller.
///
/// # Errors
/// [`WriteError::NonCanonicalMetadata`]: a `metadata_type` that disagrees with the payload variant
/// (`type_payload_mismatch`); a `payload_size` inconsistent with the payload (`unit_payload_size`);
/// a typed payload that overflows the declared size (`payload_overflows_size`); or any
/// [`write_metadata_payload`] reject. Never writes a bit to the caller on error.
pub fn write_metadata_unit(
    writer: &mut BitWriter,
    unit: &MetadataUnit,
    passthrough: &[u8],
) -> WriteResult<()> {
    check_unit_type_and_size(unit)?;

    let mut scratch = BitWriter::new();
    write_metadata_payload(&mut scratch, &unit.payload, passthrough)?;
    let typed_bits = scratch.bit_len();
    // metadataPayloadSize bytes -> target_bits; payload_size is bounded to the u32 ceiling by
    // check_unit_type_and_size, so the *8 cannot overflow u64. The typed payload must fit.
    let target_bits = (unit.payload_size as u64).saturating_mul(8);
    if typed_bits > target_bits {
        return Err(WriteError::NonCanonicalMetadata {
            what: "payload_overflows_size",
        });
    }
    writer.append(&scratch)?;
    // § 6.16.1: metadata_unit_remaining_bit zero padding to the declared size. The iteration count
    // is bounded by the u32 payload_size cap above, so a constructed model cannot drive it
    // unbounded; for any real (small) padding this is a handful of bits.
    for _ in 0..(target_bits - typed_bits) {
        writer.write_bit(0)?;
    }
    Ok(())
}

/// Validates a [`MetadataUnit`]'s `metadata_type` matches its payload variant and its
/// `payload_size` is consistent with the payload (AV2 v1.0.0 § 5.17.1 / § 6.16.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-1`).
fn check_unit_type_and_size(unit: &MetadataUnit) -> WriteResult<()> {
    // § 6.16 Table 6.17: a Reserved value the parser would re-map to a named type is unwritable.
    // The OBU writers also guard this, but a direct write_metadata_unit caller must be caught here
    // (a Reserved(1..=10) + UnknownRaw unit would otherwise reparse under the named type).
    check_canonical_metadata_type(unit.metadata_type)?;
    if !type_matches_payload(unit.metadata_type, &unit.payload) {
        return Err(WriteError::NonCanonicalMetadata {
            what: "type_payload_mismatch",
        });
    }
    // metadataPayloadSize is derived from the leb128 obuPayloadSize / muh_payload_size, both u32,
    // so a payload_size beyond the u32 ceiling could never have been parsed. Rejecting it also
    // bounds the unit padding (§ 6.16.1) so a constructed model cannot drive an unbounded write.
    if unit.payload_size as u64 > u64::from(u32::MAX) {
        return Err(WriteError::NonCanonicalMetadata {
            what: "unit_payload_size",
        });
    }
    let consistent = match &unit.payload {
        // The length-summarized payloads pin payload_size relative to their modeled length.
        MetadataPayload::IccProfile(p) => unit.payload_size == p.payload_len,
        MetadataPayload::UnknownRaw(p) => unit.payload_size == p.raw_len,
        MetadataPayload::UserDataUnregistered(p) => {
            unit.payload_size == 16usize.saturating_add(p.payload_len)
        }
        MetadataPayload::ItutT35(p) => {
            let ext = usize::from(p.itu_t_t35_country_code_extension_byte.is_some());
            unit.payload_size == 1usize.saturating_add(ext).saturating_add(p.payload_len)
        }
        // The fully-modeled payloads do not constrain payload_size beyond the per-unit padding
        // check in write_metadata_unit, so accept any size here.
        _ => true,
    };
    if !consistent {
        return Err(WriteError::NonCanonicalMetadata {
            what: "unit_payload_size",
        });
    }
    Ok(())
}

/// Returns `true` if `metadata_type` selects `payload`'s variant (AV2 § 6.16, Table 6.17).
fn type_matches_payload(metadata_type: MetadataType, payload: &MetadataPayload) -> bool {
    matches!(
        (metadata_type, payload),
        (MetadataType::HdrCll, MetadataPayload::HdrCll(_))
            | (MetadataType::HdrMdcv, MetadataPayload::HdrMdcv(_))
            | (MetadataType::ItutT35, MetadataPayload::ItutT35(_))
            | (MetadataType::Timecode, MetadataPayload::Timecode(_))
            | (
                MetadataType::DecodedFrameHash,
                MetadataPayload::DecodedFrameHash(_)
            )
            | (MetadataType::BandingHints, MetadataPayload::BandingHints(_))
            | (MetadataType::IccProfile, MetadataPayload::IccProfile(_))
            | (MetadataType::ScanType, MetadataPayload::ScanType(_))
            | (
                MetadataType::TemporalPointInfo,
                MetadataPayload::TemporalPointInfo(_)
            )
            | (
                MetadataType::UserDataUnregistered,
                MetadataPayload::UserDataUnregistered(_)
            )
            | (MetadataType::Reserved(_), MetadataPayload::UnknownRaw(_))
    )
}

/// Writes the typed metadata payload (AV2 v1.0.0 § 5.17.4 – § 5.17.13,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-4`), the inverse of
/// `parse_metadata_payload`.
///
/// `passthrough` carries the opaque length-summarized bytes for the four blob payloads (ITU-T
/// T.35, ICC, user data, and reserved/unknown raw); it must be empty for the fully-modeled
/// payloads. All multi-bit fields are written MSB-first (`f(n)`).
///
/// # Errors
/// [`WriteError::NonCanonicalMetadata`] for any field out of its `f(n)` domain, an `Option`
/// presence that disagrees with its gating flag, a vector length that disagrees with its derived
/// count, or a `passthrough` length that disagrees with the modeled blob length; plus any
/// underlying [`WriteError`] from a primitive write. Never writes a bit to the caller on error
/// (each blob and field is validated before its write).
pub fn write_metadata_payload(
    writer: &mut BitWriter,
    payload: &MetadataPayload,
    passthrough: &[u8],
) -> WriteResult<()> {
    match payload {
        MetadataPayload::HdrCll(p) => {
            require_empty_passthrough(passthrough)?;
            // § 5.17.5: max_cll f(16), max_fall f(16).
            writer.write_bits(u32::from(p.max_cll), 16)?;
            writer.write_bits(u32::from(p.max_fall), 16)
        }
        MetadataPayload::HdrMdcv(p) => {
            require_empty_passthrough(passthrough)?;
            write_hdr_mdcv(writer, p)
        }
        MetadataPayload::ItutT35(p) => write_itut_t35(writer, p, passthrough),
        MetadataPayload::Timecode(p) => {
            require_empty_passthrough(passthrough)?;
            write_timecode(writer, p)
        }
        MetadataPayload::DecodedFrameHash(p) => {
            require_empty_passthrough(passthrough)?;
            write_decoded_frame_hash(writer, p)
        }
        MetadataPayload::BandingHints(p) => {
            require_empty_passthrough(passthrough)?;
            write_banding_hints(writer, p)
        }
        MetadataPayload::IccProfile(p) => {
            // § 5.17.9: the parser reads nothing; the profile bytes are the passthrough.
            require_passthrough_len(passthrough, p.payload_len)?;
            writer.write_le(passthrough)
        }
        MetadataPayload::ScanType(p) => {
            require_empty_passthrough(passthrough)?;
            write_scan_type(writer, p)
        }
        MetadataPayload::TemporalPointInfo(p) => {
            require_empty_passthrough(passthrough)?;
            // § 5.17.11: frame_presentation_time leb128() (minimal — its byte count is not
            // modeled), matching the parser's read_leb128().
            writer.write_leb128(p.frame_presentation_time)
        }
        MetadataPayload::UserDataUnregistered(p) => {
            // § 5.17.13: uuid_iso_iec_11578 (16 bytes) then the user-data passthrough.
            require_passthrough_len(passthrough, p.payload_len)?;
            writer.write_le(&p.uuid_iso_iec_11578)?;
            writer.write_le(passthrough)
        }
        MetadataPayload::UnknownRaw(p) => {
            // § 6.16.1 NOTE: reserved / private types have undefined syntax; the raw bytes are the
            // passthrough.
            require_passthrough_len(passthrough, p.raw_len)?;
            writer.write_le(passthrough)
        }
    }
}

/// Rejects a non-empty `passthrough` for a fully-modeled payload (AV2 v1.0.0 § 5.17,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17`).
fn require_empty_passthrough(passthrough: &[u8]) -> WriteResult<()> {
    if passthrough.is_empty() {
        Ok(())
    } else {
        Err(WriteError::NonCanonicalMetadata {
            what: "passthrough_len",
        })
    }
}

/// Rejects a `passthrough` whose length disagrees with the modeled blob length (AV2 v1.0.0 § 5.17,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17`).
fn require_passthrough_len(passthrough: &[u8], expected: usize) -> WriteResult<()> {
    if passthrough.len() == expected {
        Ok(())
    } else {
        Err(WriteError::NonCanonicalMetadata {
            what: "passthrough_len",
        })
    }
}

/// Writes `metadata_hdr_mdcv()` (AV2 v1.0.0 § 5.17.6,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-6`).
fn write_hdr_mdcv(writer: &mut BitWriter, p: &MetadataHdrMdcv) -> WriteResult<()> {
    // § 5.17.6: per i, primary x then primary y (interleaved), then white x/y f(16), lum f(32).
    for i in 0..3 {
        writer.write_bits(u32::from(p.primary_chromaticity_x[i]), 16)?;
        writer.write_bits(u32::from(p.primary_chromaticity_y[i]), 16)?;
    }
    writer.write_bits(u32::from(p.white_point_chromaticity_x), 16)?;
    writer.write_bits(u32::from(p.white_point_chromaticity_y), 16)?;
    writer.write_bits(p.luminance_max, 32)?;
    writer.write_bits(p.luminance_min, 32)
}

/// Writes `metadata_itut_t35()` (AV2 v1.0.0 § 5.17.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-4`).
fn write_itut_t35(
    writer: &mut BitWriter,
    p: &MetadataItutT35,
    passthrough: &[u8],
) -> WriteResult<()> {
    // § 5.17.4: the extension byte is present iff the country code is 0xFF.
    let ext_present = p.itu_t_t35_country_code == 0xFF;
    if ext_present != p.itu_t_t35_country_code_extension_byte.is_some() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "itut_t35_extension",
        });
    }
    require_passthrough_len(passthrough, p.payload_len)?;
    writer.write_bits_u8(p.itu_t_t35_country_code, 8)?;
    if let Some(ext) = p.itu_t_t35_country_code_extension_byte {
        writer.write_bits_u8(ext, 8)?;
    }
    writer.write_le(passthrough)
}

/// Writes `metadata_timecode()` (AV2 v1.0.0 § 5.17.7,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-7`).
fn write_timecode(writer: &mut BitWriter, p: &MetadataTimecode) -> WriteResult<()> {
    check_timecode_encodable(p)?;

    // § 5.17.7: counting_type f(5), three flags f(1), n_frames f(9).
    writer.write_bits_u8(p.counting_type, 5)?;
    writer.write_bit(u8::from(p.full_timestamp_flag))?;
    writer.write_bit(u8::from(p.discontinuity_flag))?;
    writer.write_bit(u8::from(p.cnt_dropped_flag))?;
    writer.write_bits(u32::from(p.n_frames), 9)?;

    if p.full_timestamp_flag {
        // § 5.17.7: seconds f(6), minutes f(6), hours f(5) (all present, validated up front).
        writer.write_bits_u8(p.seconds_value.unwrap_or(0), 6)?;
        writer.write_bits_u8(p.minutes_value.unwrap_or(0), 6)?;
        writer.write_bits_u8(p.hours_value.unwrap_or(0), 5)?;
    } else {
        // § 5.17.7: a prefix chain of flag-then-value.
        writer.write_bit(u8::from(p.seconds_value.is_some()))?;
        if let Some(seconds) = p.seconds_value {
            writer.write_bits_u8(seconds, 6)?;
            writer.write_bit(u8::from(p.minutes_value.is_some()))?;
            if let Some(minutes) = p.minutes_value {
                writer.write_bits_u8(minutes, 6)?;
                writer.write_bit(u8::from(p.hours_value.is_some()))?;
                if let Some(hours) = p.hours_value {
                    writer.write_bits_u8(hours, 5)?;
                }
            }
        }
    }

    // § 5.17.7: time_offset_length f(5); time_offset_value f(time_offset_length) iff length > 0.
    writer.write_bits_u8(p.time_offset_length, 5)?;
    if let Some(offset) = p.time_offset_value {
        writer.write_bits(offset, u32::from(p.time_offset_length))?;
    }
    Ok(())
}

/// Validates a [`MetadataTimecode`]'s presence chains and field domains (AV2 v1.0.0 § 5.17.7,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-7`), before any
/// bit is written.
fn check_timecode_encodable(p: &MetadataTimecode) -> WriteResult<()> {
    // Field domains: counting_type f(5), n_frames f(9), seconds/minutes f(6), hours f(5),
    // time_offset_length f(5).
    if p.counting_type >= 32
        || p.n_frames >= 512
        || p.seconds_value.is_some_and(|v| v >= 64)
        || p.minutes_value.is_some_and(|v| v >= 64)
        || p.hours_value.is_some_and(|v| v >= 32)
        || p.time_offset_length >= 32
    {
        return Err(WriteError::NonCanonicalMetadata {
            what: "timecode_domain",
        });
    }
    // Presence chains.
    if p.full_timestamp_flag {
        // A full timestamp signals all three values.
        if p.seconds_value.is_none() || p.minutes_value.is_none() || p.hours_value.is_none() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "timecode_presence",
            });
        }
    } else {
        // Partial: minutes present implies seconds present; hours present implies minutes present.
        if (p.minutes_value.is_some() && p.seconds_value.is_none())
            || (p.hours_value.is_some() && p.minutes_value.is_none())
        {
            return Err(WriteError::NonCanonicalMetadata {
                what: "timecode_presence",
            });
        }
    }
    // time_offset_value is present iff time_offset_length > 0, and must fit f(length).
    if (p.time_offset_length > 0) != p.time_offset_value.is_some() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "timecode_presence",
        });
    }
    if let Some(offset) = p.time_offset_value {
        let length = u32::from(p.time_offset_length);
        // length is 1..=31 here (length > 0 implied by Some, and < 32 from the domain check).
        if length < 32 && offset >= (1u32 << length) {
            return Err(WriteError::NonCanonicalMetadata {
                what: "timecode_domain",
            });
        }
    }
    Ok(())
}

/// Writes `metadata_scan_type()` (AV2 v1.0.0 § 5.17.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-10`).
fn write_scan_type(writer: &mut BitWriter, p: &MetadataScanType) -> WriteResult<()> {
    // § 5.17.10 domains: mps_pic_struct_type f(5), mps_source_scan_type_idc f(2).
    if p.mps_pic_struct_type >= 32 || p.mps_source_scan_type_idc >= 4 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "scan_type_domain",
        });
    }
    writer.write_bits_u8(p.mps_pic_struct_type, 5)?;
    writer.write_bits_u8(p.mps_source_scan_type_idc, 2)?;
    writer.write_bit(u8::from(p.mps_duplicate_flag))
}

/// Writes `metadata_decoded_frame_hash()` (AV2 v1.0.0 § 5.17.12,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12`).
fn write_decoded_frame_hash(
    writer: &mut BitWriter,
    p: &MetadataDecodedFrameHash,
) -> WriteResult<()> {
    // § 5.17.12 domains: hash_type f(4), reserved f(1).
    if p.hash_type >= 16 || p.reserved >= 2 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "frame_hash_domain",
        });
    }
    if p.per_plane {
        // § 5.17.12: one plane_hash per plane (1 if monochrome, else 3); no single frame_hash.
        let expected = if p.is_monochrome { 1 } else { 3 };
        if p.plane_hashes.len() != expected || p.frame_hash.is_some() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "frame_hash_presence",
            });
        }
    } else {
        // § 5.17.12: a single frame_hash; no per-plane hashes.
        if !p.plane_hashes.is_empty() || p.frame_hash.is_none() {
            return Err(WriteError::NonCanonicalMetadata {
                what: "frame_hash_presence",
            });
        }
    }

    writer.write_bits_u8(p.hash_type, 4)?;
    writer.write_bit(u8::from(p.per_plane))?;
    writer.write_bit(u8::from(p.has_grain))?;
    writer.write_bit(u8::from(p.is_monochrome))?;
    writer.write_bits_u8(p.reserved, 1)?;
    if p.per_plane {
        for hash in &p.plane_hashes {
            writer.write_le(hash)?;
        }
    } else if let Some(hash) = p.frame_hash.as_ref() {
        writer.write_le(hash)?;
    }
    Ok(())
}

/// Writes `metadata_banding_hints()` (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`).
fn write_banding_hints(writer: &mut BitWriter, p: &MetadataBandingHints) -> WriteResult<()> {
    // § 5.17.8: hints are only signaled (and only present) when coding_banding_present_flag.
    if !p.coding_banding_present_flag && p.hints.is_some() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "banding_hints_presence",
        });
    }
    if let Some(detail) = p.hints.as_ref() {
        check_banding_detail_encodable(detail)?;
    }

    writer.write_bit(u8::from(p.coding_banding_present_flag))?;
    writer.write_bit(u8::from(p.source_banding_present_flag))?;
    if p.coding_banding_present_flag {
        // § 5.17.8: banding_hints_flag f(1) = hints.is_some().
        writer.write_bit(u8::from(p.hints.is_some()))?;
        if let Some(detail) = p.hints.as_ref() {
            write_banding_detail(writer, detail)?;
        }
    }
    Ok(())
}

/// Validates a [`BandingHintsDetail`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`) before any bit is written.
fn check_banding_detail_encodable(detail: &BandingHintsDetail) -> WriteResult<()> {
    let expected_components = if detail.three_color_components_flag {
        3
    } else {
        1
    };
    if detail.components.len() != expected_components {
        return Err(WriteError::NonCanonicalMetadata {
            what: "banding_component_count",
        });
    }
    for component in &detail.components {
        if component.banding_in_component_present_flag {
            // § 5.17.8: max_band_width_minus_4 f(6), max_band_step_minus_1 f(4) when present.
            let (Some(width), Some(step)) = (
                component.max_band_width_minus_4,
                component.max_band_step_minus_1,
            ) else {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "banding_component_fields",
                });
            };
            if width >= 64 || step >= 16 {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "banding_component_domain",
                });
            }
        } else if component.max_band_width_minus_4.is_some()
            || component.max_band_step_minus_1.is_some()
        {
            return Err(WriteError::NonCanonicalMetadata {
                what: "banding_component_fields",
            });
        }
    }
    if let Some(band_units) = detail.band_units.as_ref() {
        check_band_units_encodable(band_units)?;
    }
    Ok(())
}

/// Validates a [`BandUnits`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`) before any bit is written.
fn check_band_units_encodable(band_units: &BandUnits) -> WriteResult<()> {
    // § 5.17.8 domains: num_band_units_rows/cols_minus_1 f(5).
    if band_units.num_band_units_rows_minus_1 >= 32 || band_units.num_band_units_cols_minus_1 >= 32
    {
        return Err(WriteError::NonCanonicalMetadata {
            what: "band_units_domain",
        });
    }
    let rows = usize::from(band_units.num_band_units_rows_minus_1) + 1;
    let cols = usize::from(band_units.num_band_units_cols_minus_1) + 1;
    if let Some(varying) = band_units.varying_size.as_ref() {
        // § 5.17.8: band_block_in_luma_samples f(3); one vert per row, one horz per col, f(5).
        if varying.band_block_in_luma_samples >= 8
            || varying.vert_size_in_band_blocks_minus_1.len() != rows
            || varying.horz_size_in_band_blocks_minus_1.len() != cols
            || varying
                .vert_size_in_band_blocks_minus_1
                .iter()
                .any(|&v| v >= 32)
            || varying
                .horz_size_in_band_blocks_minus_1
                .iter()
                .any(|&v| v >= 32)
        {
            return Err(WriteError::NonCanonicalMetadata {
                what: "band_units_varying",
            });
        }
    }
    // § 5.17.8: banding_in_band_unit_present[r][c] f(1), row-major over rows * cols.
    if band_units.banding_in_band_unit_present.len() != rows.saturating_mul(cols) {
        return Err(WriteError::NonCanonicalMetadata {
            what: "band_units_present_count",
        });
    }
    Ok(())
}

/// Writes a [`BandingHintsDetail`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`), validated by
/// [`check_banding_detail_encodable`].
fn write_banding_detail(writer: &mut BitWriter, detail: &BandingHintsDetail) -> WriteResult<()> {
    writer.write_bit(u8::from(detail.three_color_components_flag))?;
    for component in &detail.components {
        write_banding_component(writer, component)?;
    }
    // § 5.17.8: band_units_information_present_flag f(1) = band_units.is_some().
    writer.write_bit(u8::from(detail.band_units.is_some()))?;
    if let Some(band_units) = detail.band_units.as_ref() {
        write_band_units(writer, band_units)?;
    }
    Ok(())
}

/// Writes one [`BandingComponent`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`).
fn write_banding_component(
    writer: &mut BitWriter,
    component: &BandingComponent,
) -> WriteResult<()> {
    writer.write_bit(u8::from(component.banding_in_component_present_flag))?;
    if let (Some(width), Some(step)) = (
        component.max_band_width_minus_4,
        component.max_band_step_minus_1,
    ) {
        writer.write_bits_u8(width, 6)?;
        writer.write_bits_u8(step, 4)?;
    }
    Ok(())
}

/// Writes a [`BandUnits`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`), validated by
/// [`check_band_units_encodable`].
fn write_band_units(writer: &mut BitWriter, band_units: &BandUnits) -> WriteResult<()> {
    writer.write_bits_u8(band_units.num_band_units_rows_minus_1, 5)?;
    writer.write_bits_u8(band_units.num_band_units_cols_minus_1, 5)?;
    // § 5.17.8: varying_size_band_units_flag f(1) = varying_size.is_some().
    writer.write_bit(u8::from(band_units.varying_size.is_some()))?;
    if let Some(varying) = band_units.varying_size.as_ref() {
        write_varying_band_units(writer, varying)?;
    }
    for &present in &band_units.banding_in_band_unit_present {
        writer.write_bit(u8::from(present))?;
    }
    Ok(())
}

/// Writes a [`VaryingBandUnits`] (AV2 v1.0.0 § 5.17.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8`).
fn write_varying_band_units(writer: &mut BitWriter, varying: &VaryingBandUnits) -> WriteResult<()> {
    writer.write_bits_u8(varying.band_block_in_luma_samples, 3)?;
    for &vert in &varying.vert_size_in_band_blocks_minus_1 {
        writer.write_bits_u8(vert, 5)?;
    }
    for &horz in &varying.horz_size_in_band_blocks_minus_1 {
        writer.write_bits_u8(horz, 5)?;
    }
    Ok(())
}

/// Reparses a `metadata_short_obu()` payload, used by the tests; kept here so the `include!`d test
/// files can drive a true round-trip without re-exporting the parser's private helpers.
///
/// The writer emits only the metadata-OBU payload *content* (header + type + unit); the OBU
/// `trailing_bits()` are the OBU writer's job, so `parse_metadata_short` is driven with
/// `obuPayloadSize = bytes.len() + 1` (the missing one-byte trailing_bits), matching the
/// `metadataPayloadSize = obuPayloadSize - 2 - Leb128Bytes` derivation (§ 5.17.2). The parser
/// bounds the unit to `metadataPayloadSize` and never reads the absent trailing byte.
#[cfg(test)]
fn reparse_short(bytes: &[u8]) -> crate::error::Result<MetadataShortObu> {
    let mut reader = crate::bitio::BitReader::new(bytes, crate::span::ByteOffset::new(0));
    crate::headers::metadata::parse_metadata_short(&mut reader, bytes.len() + 1)
}

/// Reparses a `metadata_group_obu()` payload, used by the tests.
#[cfg(test)]
fn reparse_group(
    bytes: &[u8],
    obu_xlayer_id: ExtendedLayerId,
) -> crate::error::Result<MetadataGroupObu> {
    let mut reader = crate::bitio::BitReader::new(bytes, crate::span::ByteOffset::new(0));
    crate::headers::metadata::parse_metadata_group(&mut reader, obu_xlayer_id)
}

// The unit/rejection tests and the property tests live in sibling files (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*` resolves to the
// writers and private helpers above.
#[cfg(test)]
include!("metadata_tests.rs");
#[cfg(test)]
include!("metadata_proptests.rs");
