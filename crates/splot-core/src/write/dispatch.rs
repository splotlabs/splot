// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unified complete-OBU payload writer (`ENC-BITSTREAM-WRITER`) — the inverse of
//! [`crate::obu::dispatch_obu_payload`] + [`crate::obu::finish_obu_payload`].
//!
//! [`write_obu_payload`] turns a parsed [`ParsedObu`] back into the OBU **payload** bytes
//! (the typed body plus the § 5.2.1 / § 6.2.1 OBU tail), and [`write_complete_obu`]
//! prepends the § 5.2.2 OBU header so the pair is the inverse of one parsed OBU
//! (`ObuHeader` + `ParsedObu`). The Annex B size prefix (`leb128(num_bytes_in_obu)`) stays
//! with [`crate::write::write_annexb_obu`], which already frames a complete OBU; this
//! module owns the header + payload it would wrap, not the length prefix.
//!
//! ## The OBU tail (the inverse of `finish_obu_payload`)
//!
//! After a **non-empty** body, [`crate::obu::finish_obu_payload`] reads (AV2 v1.0.0
//! § 5.2.1, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1`):
//!
//! - for an **extensible** OBU type, an `obu_extension_flag` (`f(1)`, a § 6.2.1
//!   conformance `0`) then `trailing_bits()`;
//! - for a **non-extensible** type, `trailing_bits()` directly;
//! - for an **empty** body (the temporal delimiter), **nothing**.
//!
//! The writer reproduces this exactly: an extensible non-empty body emits one `0` bit
//! before its `trailing_bits()`; a non-extensible non-empty body emits only
//! `trailing_bits()`; the temporal delimiter emits no tail. The `trailing_bits()` width
//! is the number of bits from the end of the body (after the optional extension flag) to
//! the next byte boundary — a full byte (`8` bits) when the body is already byte-aligned,
//! matching `finish_obu_payload`'s `reader.remaining_bits()` over the byte-granular OBU
//! payload (§ 5.2.3 `trailing_bits()`,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3`).
//!
//! ## Padding and partial coverage
//!
//! [`ParsedObu::Padding`] (§ 5.16) owns its own tail (the `obu_padding_byte` run plus the
//! `trailing_bits()` that begin at the last non-zero byte), so it is **not** followed by
//! the generic extensible tail above — exactly as `dispatch_obu_payload` special-cases it.
//! The seven OBU types with a body writer ([`ParsedObu::TemporalDelimiter`],
//! [`ParsedObu::SequenceHeader`], [`ParsedObu::Padding`], [`ParsedObu::MetadataShort`],
//! [`ParsedObu::MetadataGroup`], [`ParsedObu::BufferRemovalTiming`], [`ParsedObu::Msdo`]) are
//! emitted; the remaining seven variants have no body writer yet and return
//! [`WriteError::Unimplemented`] with the matrix Feature ID of their OBU type
//! ([`ParsedObu::feature_id`]).

use crate::headers::padding::PaddingObu;
use crate::obu::{ObuHeader, ParsedObu};
use crate::types::{ExtendedLayerId, ObuType};
use crate::write::bit_writer::BitWriter;
use crate::write::buffer_removal_timing::write_buffer_removal_timing;
use crate::write::error::{WriteError, WriteResult};
use crate::write::metadata::{write_metadata_group_obu_flat, write_metadata_short_obu};
use crate::write::msdo::write_msdo;
use crate::write::obu::write_obu_header;
use crate::write::seq_tile::write_sequence_header;

/// Writes one complete OBU — the § 5.2.2 header then the payload + tail — the inverse of a
/// parsed (`ObuHeader`, `ParsedObu`) pair. Equivalent to [`write_obu_header`] followed by
/// [`write_obu_payload`] with `is_extensible` derived from `header.obu_type`. The Annex B
/// size prefix is [`crate::write::write_annexb_obu`]'s job, not this writer's.
///
/// `passthrough` carries the opaque bytes the typed model does not hold (the
/// `obu_padding_byte` run for [`ParsedObu::Padding`], or every metadata unit's length-summarized
/// blob bytes concatenated in unit order); it must be empty for every other type. For
/// [`ParsedObu::MetadataGroup`] the group's `obu_xlayer_id` (which selects the § 6.16.3
/// global-vs-local layer-map branch) is taken from `header.extended_layer_id`, and the flat
/// `passthrough` is split per unit by each unit's modeled blob length, so a multi-unit group on
/// either layer-map branch round-trips.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary (an OBU
///   begins byte-aligned).
/// - [`WriteError::ObuTypePayloadMismatch`] if `header.obu_type` does not select `payload`'s
///   variant (a pair the § 5.2.1 OBU dispatch could never have produced).
/// - Any [`write_obu_header`] reject (inconsistent header, non-inferable layer ids,
///   non-canonical `obu_type`, oversize field).
/// - Any [`write_obu_payload`] reject (a delegated body-writer `NonCanonical*`, a
///   `passthrough` mismatch, or [`WriteError::Unimplemented`] for an unwritten type).
///
/// All checks run before any bit is written, so a rejected OBU leaves `writer` unchanged.
pub fn write_complete_obu(
    writer: &mut BitWriter,
    header: &ObuHeader,
    payload: &ParsedObu,
    passthrough: &[u8],
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    // § 5.2.1: the OBU dispatch routes one obu_type to exactly one payload syntax, so a header whose
    // obu_type does not select `payload`'s variant could never have come from parsing one OBU — and
    // would reparse as the header's type, not the supplied payload. Reject it (the writer rejects
    // exactly what the reader could not have produced), mirroring the §5.2.2 NonCanonicalObuType and
    // §6.16 metadata-type guards.
    if !obu_type_matches_payload(header.obu_type, payload) {
        return Err(WriteError::ObuTypePayloadMismatch {
            payload: payload.syntax_name(),
        });
    }
    // Draft the whole OBU (header + payload + tail) into a scratch and commit only on full
    // success, so a payload reject leaves no stray header byte in the caller's writer.
    let mut scratch = BitWriter::new();
    write_obu_header(&mut scratch, header)?;
    write_obu_payload_inner(
        &mut scratch,
        payload,
        header.obu_type.is_extensible_obu(),
        header.extended_layer_id,
        passthrough,
    )?;
    writer.append(&scratch)
}

/// Writes one OBU payload — the typed body plus the § 5.2.1 / § 6.2.1 OBU tail — the inverse
/// of one [`crate::obu::dispatch_obu_payload`] arm followed by
/// [`crate::obu::finish_obu_payload`]. The OBU header is [`write_obu_header`] /
/// [`write_complete_obu`]; the Annex B size prefix is [`crate::write::write_annexb_obu`].
///
/// `is_extensible` is `header.obu_type.is_extensible_obu()` (§ 5.2.1): a non-empty body of an
/// extensible type emits the `obu_extension_flag = 0` (`f(1)`) before `trailing_bits()`.
/// `passthrough` carries the opaque bytes the typed model does not hold — the
/// `obu_padding_byte` run for [`ParsedObu::Padding`] (its length must equal `padding_len`), or
/// every metadata unit's blob bytes concatenated in unit order; it must be empty for every other
/// type and for the temporal delimiter.
///
/// **Metadata-group layer-map scope.** This function has no OBU header, so a
/// [`ParsedObu::MetadataGroup`] is written with the § 6.16.3 **local** layer-map branch
/// (`obu_xlayer_id` = `0`). The flat `passthrough` is split per unit by each unit's modeled blob
/// length, so a multi-unit group round-trips here; only a group that used the **global**
/// (`obu_xlayer_id == 31`) layer-map branch must go through [`write_complete_obu`], which supplies
/// the real `header.extended_layer_id`.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary.
/// - [`WriteError::Unimplemented`] for the seven `ParsedObu` variants without a body writer
///   (the Feature ID is [`ParsedObu::feature_id`]).
/// - A non-empty `passthrough` for the temporal delimiter (`temporal_delimiter_passthrough`),
///   or a `padding_len` that disagrees with `passthrough.len()`
///   (`padding_passthrough_len`).
/// - Any delegated body-writer reject ([`write_sequence_header`],
///   [`write_metadata_short_obu`], [`write_metadata_group_obu_flat`], including its
///   `group_passthrough_len` split mismatch) or [`WriteError::EmptyTrailingBits`] (unreachable: a
///   non-empty body always leaves room).
///
/// All checks run before any bit reaches `writer` (the body + tail are drafted into a
/// scratch and appended only on full success), so a rejected payload leaves `writer`
/// unchanged.
pub fn write_obu_payload(
    writer: &mut BitWriter,
    payload: &ParsedObu,
    is_extensible: bool,
    passthrough: &[u8],
) -> WriteResult<()> {
    // No header is available here, so the metadata-group branch uses the non-global
    // (local) layer-map scope; see the function-level note.
    write_obu_payload_inner(
        writer,
        payload,
        is_extensible,
        ExtendedLayerId::from_bits(0),
        passthrough,
    )
}

/// Shared implementation of [`write_obu_payload`] / [`write_complete_obu`].
///
/// `obu_xlayer_id` is the OBU header's `obu_xlayer_id` (the real value from
/// [`write_complete_obu`], or the non-global default from [`write_obu_payload`]); it selects
/// the [`ParsedObu::MetadataGroup`] § 6.16.3 layer-map branch. Drafts the body + tail into a
/// scratch and appends only on full success (reject-before-write).
fn write_obu_payload_inner(
    writer: &mut BitWriter,
    payload: &ParsedObu,
    is_extensible: bool,
    obu_xlayer_id: ExtendedLayerId,
    passthrough: &[u8],
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let mut scratch = BitWriter::new();
    match payload {
        // § 5.5: an empty body. `finish_obu_payload` returns early for an empty payload, so
        // there is no tail. The OBU carries no payload bytes, so the passthrough is empty.
        ParsedObu::TemporalDelimiter => {
            if !passthrough.is_empty() {
                return Err(WriteError::NonCanonicalMetadata {
                    what: "temporal_delimiter_passthrough",
                });
            }
            // No body, no tail: an empty payload round-trips through write -> reparse.
        }
        // § 5.4: the sequence-header body, then the generic extensible tail.
        ParsedObu::SequenceHeader(header) => {
            if !passthrough.is_empty() {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "sequence_header_passthrough",
                });
            }
            write_sequence_header(&mut scratch, header)?;
            write_generic_tail(&mut scratch, is_extensible)?;
        }
        // § 5.16: padding owns its whole tail (the obu_padding_byte run plus its own
        // trailing_bits), so it is NOT followed by the generic extensible tail.
        ParsedObu::Padding(padding) => {
            write_padding_payload(&mut scratch, padding, passthrough)?;
        }
        // § 5.17.2: the short-metadata body, then the generic tail (metadata is
        // non-extensible, so the tail is trailing_bits() only).
        ParsedObu::MetadataShort(obu) => {
            write_metadata_short_obu(&mut scratch, obu, passthrough)?;
            write_generic_tail(&mut scratch, is_extensible)?;
        }
        // § 5.17.3: the group-metadata body, then the generic tail. The flat-passthrough writer
        // splits `passthrough` per unit by each unit's modeled blob length, so a multi-unit group
        // (cancelled / fully-modeled / length-summarized units in any mix) round-trips.
        ParsedObu::MetadataGroup(obu) => {
            write_metadata_group_obu_flat(&mut scratch, obu, obu_xlayer_id, passthrough)?;
            write_generic_tail(&mut scratch, is_extensible)?;
        }
        // § 5.12: the buffer-removal-timing body, then the generic tail (the OBU type is not
        // extensible, so the tail is trailing_bits() only). It carries no passthrough.
        ParsedObu::BufferRemovalTiming(brt) => {
            if !passthrough.is_empty() {
                return Err(WriteError::NonCanonicalBufferRemovalTiming {
                    what: "passthrough",
                });
            }
            write_buffer_removal_timing(&mut scratch, brt)?;
            write_generic_tail(&mut scratch, is_extensible)?;
        }
        // § 5.6: the multistream-decoder-operation body, then the generic tail (not extensible). It
        // carries no passthrough.
        ParsedObu::Msdo(msdo) => {
            if !passthrough.is_empty() {
                return Err(WriteError::NonCanonicalMsdo {
                    what: "passthrough",
                });
            }
            write_msdo(&mut scratch, msdo)?;
            write_generic_tail(&mut scratch, is_extensible)?;
        }
        // The seven OBU types without a body writer yet: an honest typed stub naming the
        // matrix Feature ID of the OBU type (ParsedObu::feature_id stays in sync with the
        // model). bit_len() is unchanged because nothing was appended.
        ParsedObu::MultiFrameHeader(_)
        | ParsedObu::LayerConfigurationRecord(_)
        | ParsedObu::AtlasSegment(_)
        | ParsedObu::OperatingPointSet(_)
        | ParsedObu::QuantizationMatrix(_)
        | ParsedObu::FilmGrain(_)
        | ParsedObu::ContentInterpretation(_) => {
            return Err(WriteError::Unimplemented {
                feature: payload.feature_id(),
            });
        }
    }
    writer.append(&scratch)
}

/// Writes the generic OBU tail after a non-empty body (the inverse of
/// [`crate::obu::finish_obu_payload`] on its non-empty path; AV2 v1.0.0 § 5.2.1 / § 6.2.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1`).
///
/// For an extensible OBU type, emits `obu_extension_flag = 0` (`f(1)`) then
/// `trailing_bits()`; for a non-extensible type, only `trailing_bits()`. The
/// `trailing_bits()` width fills `scratch` to the next byte boundary — a full byte when the
/// body (plus the optional extension flag) is already byte-aligned — matching the parser's
/// `reader.remaining_bits()` over the byte-granular OBU payload.
fn write_generic_tail(scratch: &mut BitWriter, is_extensible: bool) -> WriteResult<()> {
    if is_extensible {
        // § 6.2.1: obu_extension_flag is 0 in this specification version.
        scratch.write_bit(0)?;
    }
    // trailing_bits() runs from here to the OBU payload's byte boundary: a full byte (8 bits)
    // when byte-aligned, else the bits that complete the current byte.
    let nb_bits = bits_to_byte_boundary(scratch.bit_len());
    scratch.write_trailing_bits(nb_bits)
}

/// Number of `trailing_bits()` bits from `bit_len` to the next byte boundary: `8` when
/// `bit_len` is byte-aligned (a whole trailing byte), else `8 - (bit_len % 8)`.
fn bits_to_byte_boundary(bit_len: u64) -> u64 {
    let rem = bit_len % 8;
    if rem == 0 { 8 } else { 8 - rem }
}

/// Writes the `padding_obu()` payload (AV2 v1.0.0 § 5.16 / § 6.15,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`), the inverse of
/// [`crate::headers::padding::parse_padding_obu`].
///
/// The § 5.16 split is reproduced exactly: `padding.padding_len` opaque `obu_padding_byte`
/// values (taken from `passthrough`, whose length must equal `padding_len`) followed by
/// `padding.trailing_len` bytes of `trailing_bits()`. The edge cases match the parser: an
/// empty payload (`padding_len == 0 && trailing_len == 0`, `obuPayloadSize == 0`) writes
/// nothing; a single trailing byte (`padding_len == 0 && trailing_len == 1`,
/// `obuPayloadSize == 1`) writes one `trailing_bits()` byte. Padding owns its tail, so the
/// caller does NOT add the generic extensible tail.
///
/// # Errors
/// - [`WriteError::NonCanonicalMetadata`] with `what == "padding_trailing_len"` if
///   `padding.trailing_len == 0` while `padding.padding_len != 0` — a split the § 5.16
///   parser never produces (see below), so the writer rejects it instead of emitting a
///   stream that would not reparse.
/// - [`WriteError::NonCanonicalMetadata`] with `what == "padding_passthrough_len"` if
///   `passthrough.len() != padding.padding_len`. (Padding shares the metadata error variant
///   for a passthrough-length mismatch; the `what` label disambiguates.)
fn write_padding_payload(
    scratch: &mut BitWriter,
    padding: &PaddingObu,
    passthrough: &[u8],
) -> WriteResult<()> {
    // § 5.16 / § 6.15: the parser splits the payload at the last non-zero byte, so a
    // non-empty payload always has at least one trailing_bits() byte — `trailing_len == 0`
    // occurs only for the empty payload (`padding_len == 0`). Any other split
    // (`trailing_len == 0` with `padding_len > 0`) is a hand-built model the parser could
    // not have produced; emitting it would write `padding_len` bytes with no trailing byte,
    // whose last non-zero byte is not a valid trailing_bits() pattern (the reparse fails).
    // Reject it before any bit, like the sibling metadata/sequence writers.
    if padding.trailing_len == 0 && padding.padding_len != 0 {
        return Err(WriteError::NonCanonicalMetadata {
            what: "padding_trailing_len",
        });
    }
    if passthrough.len() != padding.padding_len {
        return Err(WriteError::NonCanonicalMetadata {
            what: "padding_passthrough_len",
        });
    }
    // § 5.16: the leading obu_padding_byte run is opaque; re-emit it verbatim.
    scratch.write_le(passthrough)?;
    // § 5.16: trailing_len bytes of trailing_bits() (0 only for an empty payload). The
    // parser derives trailing_len from the last non-zero byte; padding_len + trailing_len
    // == obuPayloadSize, so this restores the exact byte count.
    if padding.trailing_len > 0 {
        let nb_bits = (padding.trailing_len as u64).saturating_mul(8);
        scratch.write_trailing_bits(nb_bits)?;
    }
    Ok(())
}

/// Returns `true` when `obu_type` is the OBU type that [`crate::obu::dispatch_obu_payload`] routes to
/// `payload`'s variant — the 1:1 § 5.2.1 `obu_type` → payload-syntax mapping. Used by
/// [`write_complete_obu`] to reject a mispaired `(ObuHeader, ParsedObu)` the parser could not have
/// produced. The opaque (`Reserved`) and frame-carrying (tile-group / SEF / TIP) OBU types do not
/// produce a [`ParsedObu`], so they never match.
fn obu_type_matches_payload(obu_type: ObuType, payload: &ParsedObu) -> bool {
    matches!(
        (obu_type, payload),
        (ObuType::TemporalDelimiter, ParsedObu::TemporalDelimiter)
            | (ObuType::SequenceHeader, ParsedObu::SequenceHeader(_))
            | (ObuType::Msdo, ParsedObu::Msdo(_))
            | (ObuType::MultiFrameHeader, ParsedObu::MultiFrameHeader(_))
            | (
                ObuType::LayerConfigurationRecord,
                ParsedObu::LayerConfigurationRecord(_)
            )
            | (ObuType::AtlasSegment, ParsedObu::AtlasSegment(_))
            | (ObuType::OperatingPointSet, ParsedObu::OperatingPointSet(_))
            | (
                ObuType::BufferRemovalTiming,
                ParsedObu::BufferRemovalTiming(_)
            )
            | (
                ObuType::QuantizationMatrix,
                ParsedObu::QuantizationMatrix(_)
            )
            | (ObuType::FilmGrain, ParsedObu::FilmGrain(_))
            | (
                ObuType::ContentInterpretation,
                ParsedObu::ContentInterpretation(_)
            )
            | (ObuType::Padding, ParsedObu::Padding(_))
            | (ObuType::MetadataShort, ParsedObu::MetadataShort(_))
            | (ObuType::MetadataGroup, ParsedObu::MetadataGroup(_))
    )
}

// The round-trip / Unimplemented / reject-propagation tests live in a sibling file (kept under
// the advisory source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writers and private helpers above.
#[cfg(test)]
include!("dispatch_tests.rs");
