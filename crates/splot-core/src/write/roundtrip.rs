// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The writer round-trip harness (`ENC-BITSTREAM-WRITER`): `parse → write → reparse` over the
//! complete-OBU dispatch ([`crate::write::write_complete_obu`]).
//!
//! The writer is the inverse of the parser iff `parse(write(parse(x))) == parse(x)` — a *semantic*
//! round-trip on the [`ParsedObu`] model (which derives `PartialEq`). [`roundtrip_obu`] checks
//! exactly this for one OBU: it recovers the opaque `passthrough` bytes ([`recover_roundtrip_passthrough`]),
//! writes the complete OBU, frames it with the Annex B `leb128(num_bytes_in_obu)` size prefix
//! (§ B.2, `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`), reparses,
//! and compares the reparsed model to the input.
//!
//! ## Passthrough recovery
//!
//! [`ParsedObu`] does not hold opaque bytes, so the harness reconstructs the
//! [`crate::write::write_complete_obu`] `passthrough` from the original payload:
//!
//! - **Padding** (§ 5.16, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`): the parser splits at
//!   the *last non-zero byte*, so the `obu_padding_byte` run is exactly `payload[..padding_len]` and
//!   its byte values determine the reparse split — it is recovered as a real slice (byte-exact).
//! - **Metadata blobs** (§ 5.17.9 – § 5.17.13,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17-9`: ITU-T T.35 / ICC / user-data / unknown-raw): the
//!   model stores only the blob *length*, not its bytes, so any bytes of that length reparse to the
//!   same model. The harness returns a **zero-fill of the modeled length** — sufficient for the
//!   semantic round-trip, and no per-unit byte-offset re-derivation is needed. (Byte-exactness does
//!   not hold for a non-zero blob; the model cannot represent the blob bytes, so that is out of
//!   scope.)
//! - **Everything else** (temporal delimiter, sequence header, fully-modeled / cancelled metadata):
//!   empty.
//!
//! Recovery allocates at most `payload.len()` bytes (a parsed model's blob lengths are sub-slices of
//! its payload) and rejects a constructed model whose lengths exceed the payload — both correctness
//! and an out-of-memory guard.
//!
//! ## Why the *complete* OBU
//!
//! A metadata group on the **global** layer-map branch (`obu_xlayer_id == 31`, § 6.16.3,
//! `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-3`) encodes its
//! layer maps differently from the local branch; [`crate::write::write_obu_payload`] has no header so
//! it always writes the local branch, whereas [`crate::write::write_complete_obu`] threads
//! `header.extended_layer_id`. The harness therefore writes the complete OBU and reframes it with the
//! Annex B size prefix (the complete-OBU bytes are exactly the `header ++ payload` the prefix wraps).

use crate::annexb::parse_annex_b_obus;
use crate::obu::{ObuHeader, ParsedObu, PayloadStatus};
use crate::write::bit_writer::BitWriter;
use crate::write::dispatch::write_complete_obu;
use crate::write::error::{WriteError, WriteResult};
use crate::write::metadata::{metadata_group_unit_passthrough_len, metadata_unit_passthrough_len};

/// The result of round-tripping one OBU through [`roundtrip_obu`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundtripOutcome {
    /// The OBU was written via [`crate::write::write_complete_obu`] and reparsed to a model equal to
    /// the input — a clean round-trip.
    RoundTripped,
    /// The OBU type has no body writer yet (the dispatch returned [`WriteError::Unimplemented`]); the
    /// harness skips it, like a parser fuzz target skips an unparsed payload.
    Unwritable {
        /// The matrix Feature ID of the missing body writer.
        feature: &'static str,
    },
    /// A round-trip defect for a *parser-produced* model: a writer reject, an unrecoverable
    /// passthrough, a reparse failure, or a header / model mismatch. `reason` names the class.
    Failed {
        /// A short, stable label for the failure class (for a fuzz panic message / test assertion).
        reason: &'static str,
    },
}

/// Recovers the opaque `passthrough` bytes needed to re-write `parsed` (parsed from `payload`) via
/// [`crate::write::write_complete_obu`], sufficient for a *semantic* round-trip.
///
/// Padding returns its real `obu_padding_byte` run (`payload[..padding_len]`, whose values drive the
/// parser's split); the metadata blobs return a zero-fill of the modeled blob length (the blob values
/// are not modeled); every other type returns an empty `Vec`. See the module docs.
///
/// # Errors
/// [`WriteError::NonCanonicalMetadata`] with `what == "roundtrip_passthrough_len"` if the modeled
/// passthrough length exceeds `payload.len()` — a constructed model whose opaque bytes cannot come
/// from this payload (this also bounds the allocation).
pub fn recover_roundtrip_passthrough(payload: &[u8], parsed: &ParsedObu) -> WriteResult<Vec<u8>> {
    match parsed {
        ParsedObu::Padding(pad) => slice_recover(payload, pad.padding_len),
        ParsedObu::MetadataShort(obu) => {
            let len = obu.unit.as_ref().map_or(0, metadata_unit_passthrough_len);
            zero_fill_recover(payload, len)
        }
        ParsedObu::MetadataGroup(obu) => {
            let len = obu
                .units
                .iter()
                .try_fold(0usize, |acc, unit| {
                    acc.checked_add(metadata_group_unit_passthrough_len(unit))
                })
                .ok_or(WriteError::NonCanonicalMetadata {
                    what: "roundtrip_passthrough_len",
                })?;
            zero_fill_recover(payload, len)
        }
        _ => Ok(Vec::new()),
    }
}

/// Recovers `payload[..len]` as the real opaque run, or rejects if `len` exceeds the payload.
fn slice_recover(payload: &[u8], len: usize) -> WriteResult<Vec<u8>> {
    payload
        .get(..len)
        .map(<[u8]>::to_vec)
        .ok_or(WriteError::NonCanonicalMetadata {
            what: "roundtrip_passthrough_len",
        })
}

/// Returns `len` zero bytes (a length-only blob recovery), or rejects if `len` exceeds the payload —
/// bounding the allocation by the source payload.
fn zero_fill_recover(payload: &[u8], len: usize) -> WriteResult<Vec<u8>> {
    if len > payload.len() {
        return Err(WriteError::NonCanonicalMetadata {
            what: "roundtrip_passthrough_len",
        });
    }
    Ok(vec![0u8; len])
}

/// Round-trips one OBU: recovers the passthrough, writes the complete OBU via
/// [`crate::write::write_complete_obu`], frames it with the Annex B size prefix, reparses, and
/// reports whether the reparsed `ParsedObu` equals `parsed`.
///
/// `header` and `payload` are a parsed OBU's header and raw payload bytes (e.g. from an
/// [`crate::annexb::ObuEnvelope`]); `parsed` is `payload`'s parsed model
/// ([`crate::annexb::ObuEnvelope::payload_status`] returning [`PayloadStatus::Parsed`]). Never
/// panics (splot-core library policy); a caller (a test or fuzz target) decides what is a finding.
/// For a parser-produced `parsed`, a clean writer returns [`RoundtripOutcome::RoundTripped`] for a
/// written type and [`RoundtripOutcome::Unwritable`] for an unwritten one; any
/// [`RoundtripOutcome::Failed`] is a defect.
#[must_use]
pub fn roundtrip_obu(header: &ObuHeader, payload: &[u8], parsed: &ParsedObu) -> RoundtripOutcome {
    let Ok(passthrough) = recover_roundtrip_passthrough(payload, parsed) else {
        return RoundtripOutcome::Failed {
            reason: "passthrough_unrecoverable",
        };
    };

    let mut complete_writer = BitWriter::new();
    match write_complete_obu(&mut complete_writer, header, parsed, &passthrough) {
        Ok(()) => {}
        Err(WriteError::Unimplemented { feature }) => {
            return RoundtripOutcome::Unwritable { feature };
        }
        Err(_) => {
            return RoundtripOutcome::Failed {
                reason: "write_rejected",
            };
        }
    }
    let complete = complete_writer.into_bytes();

    let Ok(total) = u32::try_from(complete.len()) else {
        return RoundtripOutcome::Failed {
            reason: "oversize_obu",
        };
    };
    let mut framed = BitWriter::new();
    if framed.write_leb128(total).is_err() || framed.write_le(&complete).is_err() {
        return RoundtripOutcome::Failed {
            reason: "reframe_failed",
        };
    }
    let bytes = framed.into_bytes();

    let Ok(obus) = parse_annex_b_obus(&bytes) else {
        return RoundtripOutcome::Failed {
            reason: "reparse_failed",
        };
    };
    let [env] = obus.as_slice() else {
        return RoundtripOutcome::Failed { reason: "reframed" };
    };
    if env.header != *header {
        return RoundtripOutcome::Failed {
            reason: "header_mismatch",
        };
    }
    match env.payload_status() {
        Ok(PayloadStatus::Parsed(reparsed)) if reparsed == *parsed => {
            RoundtripOutcome::RoundTripped
        }
        Ok(PayloadStatus::Parsed(_)) => RoundtripOutcome::Failed {
            reason: "model_mismatch",
        },
        _ => RoundtripOutcome::Failed {
            reason: "reparse_not_parsed",
        },
    }
}

#[cfg(test)]
include!("roundtrip_tests.rs");
