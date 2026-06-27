// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **size / configuration** writers (`ENC-BITSTREAM-WRITER`) — the
//! inverses of the § 5.18.4 `frame_size()` and § 5.18.3 `screen_content_params()` /
//! `intrabc_params()` parsers in [`crate::headers::frame`]:
//!
//! - [`write_frame_size`] — `frame_size()` (§ 5.18.4.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-1`): the override `f(n)`
//!   width/height, or no bits on the non-override default path.
//! - [`write_screen_content_params`] — `screen_content_params()` (§ 5.18.3.3,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-3`): the `SELECT`-gated
//!   `allow_screen_content_tools` / `force_integer_mv` flags.
//! - [`write_intrabc_params`] — `intrabc_params()` (§ 5.18.3.4,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-4`): `allow_intrabc` plus the
//!   conditionally-coded `allow_global_intrabc` / `allow_local_intrabc` / `change_bvp_drl`
//!   / `max_bvp_drl_bits_minus_1` fields recorded on [`IntrabcParams`].
//!
//! Each writer threads the same gating inputs the parser receives (the sequence-forced
//! SCC/MV flags, `frame_size_override_flag`, `frame_is_intra`, …) and validates the whole
//! structure before any bit is written (reject-before-write): a value the parser could not
//! have produced — a width that overflows its `f(n)` field, a derived/inferred flag that
//! disagrees with its gate, or an `Option` whose presence disagrees with the syntax — is
//! rejected with a typed [`WriteError`]. With the `intrabc_params()` fields now surfaced on
//! [`IntrabcParams`], the round-trip is **byte-exact**, not merely semantic.

use crate::headers::frame::{FrameSize, IntrabcParams};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `SELECT_SCREEN_CONTENT_TOOLS`: the `seq_force_screen_content_tools` value meaning the
/// frame codes `allow_screen_content_tools` (AV2 v1.0.0 § 3 / § 6.4.7,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-7`). Mirrors the private
/// constant in [`crate::headers::frame`]'s `config` module.
const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;

/// `SELECT_INTEGER_MV`: the `seq_force_integer_mv` value meaning the frame codes
/// `force_integer_mv` (AV2 v1.0.0 § 3 / § 6.4.7,
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-7`).
const SELECT_INTEGER_MV: u8 = 2;

/// Returns `true` if `value` fits in an `n`-bit fixed field (`f(n)`).
fn fits_in_bits(value: u32, n: u32) -> bool {
    n >= u32::BITS || value < (1u32 << n)
}

/// Writes `frame_size()` (AV2 v1.0.0 § 5.18.4.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-1`), the inverse of the
/// `parse_frame_size` parser.
///
/// When `frame_size_override_flag` is set, `frame_width_minus_1` `f(frame_width_bits)` and
/// `frame_height_minus_1` `f(frame_height_bits)` are written; otherwise no bit is written
/// (the dimensions come from the multi-frame-header defaults, so `size` must equal
/// `default_dims`). The model is fully validated before any bit is written.
///
/// # Errors
/// - [`WriteError::ValueTooWide`] if an overridden `frame_width_minus_1` /
///   `frame_height_minus_1` does not fit its `f(n)` field.
/// - [`WriteError::NonCanonicalFrameHeader`] if a dimension is `0` (the parser derives
///   `frame_*_minus_1 + 1 >= 1`), or the non-override `size` does not equal `default_dims`.
pub fn write_frame_size(
    writer: &mut BitWriter,
    size: &FrameSize,
    frame_size_override_flag: bool,
    frame_width_bits: u32,
    frame_height_bits: u32,
    default_dims: Option<(u32, u32)>,
) -> WriteResult<()> {
    check_frame_size_encodable(
        *size,
        frame_size_override_flag,
        frame_width_bits,
        frame_height_bits,
        default_dims,
    )?;

    if frame_size_override_flag {
        writer.write_bits(size.width - 1, frame_width_bits)?;
        writer.write_bits(size.height - 1, frame_height_bits)?;
    }
    // Non-override: dimensions are inferred from default_dims; no bits.
    Ok(())
}

/// Validates a [`FrameSize`] is a model the § 5.18.4.1
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-1`) parser could have produced.
fn check_frame_size_encodable(
    size: FrameSize,
    frame_size_override_flag: bool,
    frame_width_bits: u32,
    frame_height_bits: u32,
    default_dims: Option<(u32, u32)>,
) -> WriteResult<()> {
    // The parser derives FrameWidth/Height as frame_*_minus_1 + 1, so both are >= 1.
    if size.width == 0 || size.height == 0 {
        return Err(WriteError::NonCanonicalFrameHeader { what: "frame_size" });
    }
    if frame_size_override_flag {
        // The f(n) descriptor accepts n <= 32; reject an over-wide field BEFORE either
        // dimension is emitted (a valid width followed by an over-wide height would
        // otherwise leave a partial buffer when write_bits rejects the height).
        if frame_width_bits > u32::BITS {
            return Err(WriteError::BitWidthTooLarge {
                requested: frame_width_bits,
                max: u32::BITS,
            });
        }
        if frame_height_bits > u32::BITS {
            return Err(WriteError::BitWidthTooLarge {
                requested: frame_height_bits,
                max: u32::BITS,
            });
        }
        if !fits_in_bits(size.width - 1, frame_width_bits) {
            return Err(WriteError::ValueTooWide {
                value: u64::from(size.width - 1),
                width_bits: frame_width_bits,
            });
        }
        if !fits_in_bits(size.height - 1, frame_height_bits) {
            return Err(WriteError::ValueTooWide {
                value: u64::from(size.height - 1),
                width_bits: frame_height_bits,
            });
        }
    } else if default_dims != Some((size.width, size.height)) {
        // The non-override path emits no bits, so the size must equal the inferred default.
        return Err(WriteError::NonCanonicalFrameHeader { what: "frame_size" });
    }
    Ok(())
}

/// Writes `screen_content_params()` (AV2 v1.0.0 § 5.18.3.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-3`), the inverse of
/// `parse_screen_content_params_full`.
///
/// `allow_screen_content_tools` is coded only when
/// `seq_force_screen_content_tools == SELECT`; otherwise it is inferred from the sequence
/// force value. `force_integer_mv` is coded only when `allow_screen_content_tools` and
/// `seq_force_integer_mv == SELECT`; otherwise it is inferred. The model is fully validated
/// before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] if `allow_screen_content_tools` or
/// `force_integer_mv` is an inferred (non-coded) value that disagrees with the sequence
/// force values.
pub fn write_screen_content_params(
    writer: &mut BitWriter,
    allow_screen_content_tools: bool,
    force_integer_mv: bool,
    seq_force_screen_content_tools: u8,
    seq_force_integer_mv: u8,
) -> WriteResult<()> {
    check_screen_content_encodable(
        allow_screen_content_tools,
        force_integer_mv,
        seq_force_screen_content_tools,
        seq_force_integer_mv,
    )?;

    if seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS {
        writer.write_flag(allow_screen_content_tools)?;
    }
    if allow_screen_content_tools && seq_force_integer_mv == SELECT_INTEGER_MV {
        writer.write_flag(force_integer_mv)?;
    }
    Ok(())
}

/// Validates the `screen_content_params()` inferred branches.
fn check_screen_content_encodable(
    allow_screen_content_tools: bool,
    force_integer_mv: bool,
    seq_force_screen_content_tools: u8,
    seq_force_integer_mv: u8,
) -> WriteResult<()> {
    // Not coded -> allow_screen_content_tools is inferred as (seq force value != 0).
    if seq_force_screen_content_tools != SELECT_SCREEN_CONTENT_TOOLS
        && allow_screen_content_tools != (seq_force_screen_content_tools != 0)
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "allow_screen_content_tools",
        });
    }
    // force_integer_mv is inferred unless screen-content tools are on and the sequence
    // selects it: 0 when tools off, else the sequence force value.
    let imv_coded = allow_screen_content_tools && seq_force_integer_mv == SELECT_INTEGER_MV;
    if !imv_coded {
        let inferred = allow_screen_content_tools && seq_force_integer_mv != 0;
        if force_integer_mv != inferred {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "force_integer_mv",
            });
        }
    }
    Ok(())
}

/// Writes `intrabc_params()` (AV2 v1.0.0 § 5.18.3.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-4`), the inverse of
/// `parse_intrabc_params_full`.
///
/// Writes `allow_intrabc` `f(1)`; then, when `allow_intrabc`, the `frame_is_intra`-gated
/// `allow_global_intrabc` (and `allow_local_intrabc` when it is set) and the
/// `allow_frame_max_bvp_drl_bits`-gated `change_bvp_drl` (and `max_bvp_drl_bits_minus_1`
/// `ns(2)` when it is set). Each conditionally-coded field on [`IntrabcParams`] must be
/// `Some` exactly when its gate selects it. The model is fully validated before any bit.
///
/// # Errors
/// - [`WriteError::NonCanonicalFrameHeader`] if an `Option` field's presence disagrees with
///   its gate (e.g. `allow_global_intrabc` present while `!allow_intrabc`).
/// - [`WriteError::ValueOutOfRange`] if `max_bvp_drl_bits_minus_1` is outside the `ns(2)`
///   domain.
pub fn write_intrabc_params(
    writer: &mut BitWriter,
    params: &IntrabcParams,
    frame_is_intra: bool,
    allow_frame_max_bvp_drl_bits: bool,
) -> WriteResult<()> {
    check_intrabc_encodable(params, frame_is_intra, allow_frame_max_bvp_drl_bits)?;

    writer.write_flag(params.allow_intrabc)?;
    if params.allow_intrabc {
        if frame_is_intra {
            // Presence guaranteed by check_intrabc_encodable.
            let global =
                params
                    .allow_global_intrabc
                    .ok_or(WriteError::NonCanonicalFrameHeader {
                        what: "allow_global_intrabc",
                    })?;
            writer.write_flag(global)?;
            if global {
                let local =
                    params
                        .allow_local_intrabc
                        .ok_or(WriteError::NonCanonicalFrameHeader {
                            what: "allow_local_intrabc",
                        })?;
                writer.write_flag(local)?;
            }
        }
        if allow_frame_max_bvp_drl_bits {
            let change = params
                .change_bvp_drl
                .ok_or(WriteError::NonCanonicalFrameHeader {
                    what: "change_bvp_drl",
                })?;
            writer.write_flag(change)?;
            if change {
                let raw =
                    params
                        .max_bvp_drl_bits_minus_1
                        .ok_or(WriteError::NonCanonicalFrameHeader {
                            what: "max_bvp_drl_bits_minus_1",
                        })?;
                writer.write_ns(raw, 2)?;
            }
        }
    }
    Ok(())
}

/// Validates an [`IntrabcParams`] is a model the § 5.18.3.4
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3-4`) parser could have produced: each
/// conditionally-read field is `Some` exactly when its gate selects it (and absent
/// otherwise), and `max_bvp_drl_bits_minus_1` lies in the `ns(2)` domain.
fn check_intrabc_encodable(
    params: &IntrabcParams,
    frame_is_intra: bool,
    allow_frame_max_bvp_drl_bits: bool,
) -> WriteResult<()> {
    let global_coded = params.allow_intrabc && frame_is_intra;
    match (global_coded, params.allow_global_intrabc) {
        (true, Some(_)) | (false, None) => {}
        _ => {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "allow_global_intrabc",
            });
        }
    }
    // allow_local_intrabc is coded only when allow_global_intrabc == Some(true).
    let local_coded = params.allow_global_intrabc == Some(true);
    match (local_coded, params.allow_local_intrabc) {
        (true, Some(_)) | (false, None) => {}
        _ => {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "allow_local_intrabc",
            });
        }
    }

    let change_coded = params.allow_intrabc && allow_frame_max_bvp_drl_bits;
    match (change_coded, params.change_bvp_drl) {
        (true, Some(_)) | (false, None) => {}
        _ => {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "change_bvp_drl",
            });
        }
    }
    // max_bvp_drl_bits_minus_1 is coded only when change_bvp_drl == Some(true).
    let max_coded = params.change_bvp_drl == Some(true);
    match (max_coded, params.max_bvp_drl_bits_minus_1) {
        (true, Some(raw)) => {
            if raw >= 2 {
                return Err(WriteError::ValueOutOfRange {
                    descriptor: "ns",
                    value: i64::from(raw),
                });
            }
        }
        (false, None) => {}
        _ => {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "max_bvp_drl_bits_minus_1",
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
    use crate::headers::frame::{
        parse_frame_size, parse_intrabc_params_full, parse_screen_content_params_full,
    };
    use crate::span::ByteOffset;

    const SELECT_SCC: u8 = SELECT_SCREEN_CONTENT_TOOLS;
    const SELECT_IMV: u8 = SELECT_INTEGER_MV;

    // ---- frame_size ----

    fn roundtrip_frame_size(
        size: FrameSize,
        override_flag: bool,
        w_bits: u32,
        h_bits: u32,
        default_dims: Option<(u32, u32)>,
    ) {
        let mut writer = BitWriter::new();
        write_frame_size(
            &mut writer,
            &size,
            override_flag,
            w_bits,
            h_bits,
            default_dims,
        )
        .unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let parsed = parse_frame_size(&mut reader, override_flag, w_bits, h_bits, default_dims)
            .unwrap()
            .expect("size present");
        assert_eq!(parsed, size);
    }

    #[test]
    fn frame_size_override_round_trips() {
        roundtrip_frame_size(FrameSize::new(1920, 1080), true, 12, 12, None);
        roundtrip_frame_size(FrameSize::new(1, 1), true, 1, 1, None);
        roundtrip_frame_size(FrameSize::new(65536, 65536), true, 16, 16, Some((1, 1)));
    }

    #[test]
    fn frame_size_non_override_writes_no_bits() {
        let mut writer = BitWriter::new();
        write_frame_size(
            &mut writer,
            &FrameSize::new(640, 480),
            false,
            12,
            12,
            Some((640, 480)),
        )
        .unwrap();
        assert_eq!(writer.bit_len(), 0);
        roundtrip_frame_size(FrameSize::new(640, 480), false, 12, 12, Some((640, 480)));
    }

    #[test]
    fn frame_size_override_too_wide_rejected() {
        let mut writer = BitWriter::new();
        let err = write_frame_size(&mut writer, &FrameSize::new(4097, 1), true, 12, 12, None)
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 4096,
                width_bits: 12
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn frame_size_overwide_height_bits_rejected_before_any_bit() {
        // width field is fine (12 bits) but height_bits > 32: reject before emitting width.
        let mut writer = BitWriter::new();
        let err =
            write_frame_size(&mut writer, &FrameSize::new(2, 2), true, 12, 33, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::BitWidthTooLarge {
                requested: 33,
                max: 32
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn frame_size_zero_dimension_rejected() {
        let mut writer = BitWriter::new();
        let err =
            write_frame_size(&mut writer, &FrameSize::new(0, 1), true, 12, 12, None).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "frame_size" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn frame_size_non_override_mismatch_rejected() {
        let mut writer = BitWriter::new();
        let err = write_frame_size(
            &mut writer,
            &FrameSize::new(640, 480),
            false,
            12,
            12,
            Some((1, 1)),
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "frame_size" }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ---- screen_content_params ----

    fn roundtrip_scc(allow: bool, imv: bool, seq_scc: u8, seq_imv: u8) {
        let mut writer = BitWriter::new();
        write_screen_content_params(&mut writer, allow, imv, seq_scc, seq_imv).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let parsed = parse_screen_content_params_full(&mut reader, seq_scc, seq_imv).unwrap();
        assert_eq!(parsed.allow_screen_content_tools, allow);
        assert_eq!(parsed.force_integer_mv, imv);
    }

    #[test]
    fn scc_select_both_round_trips() {
        roundtrip_scc(true, true, SELECT_SCC, SELECT_IMV);
        roundtrip_scc(true, false, SELECT_SCC, SELECT_IMV);
        roundtrip_scc(false, false, SELECT_SCC, SELECT_IMV);
    }

    #[test]
    fn scc_forced_round_trips() {
        // Forced off: allow inferred false, imv inferred false.
        roundtrip_scc(false, false, 0, 0);
        // Forced on (seq_force_scc == 1), imv forced on (seq_force_imv == 1).
        roundtrip_scc(true, true, 1, 1);
    }

    #[test]
    fn scc_inferred_allow_mismatch_rejected() {
        // seq_force_scc = 0 -> allow inferred false; storing true is non-canonical.
        let mut writer = BitWriter::new();
        let err = write_screen_content_params(&mut writer, true, false, 0, 0).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "allow_screen_content_tools"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn scc_inferred_imv_mismatch_rejected() {
        // allow on but seq_force_imv = 0 (not SELECT) -> imv inferred false; true is bad.
        let mut writer = BitWriter::new();
        let err = write_screen_content_params(&mut writer, true, true, SELECT_SCC, 0).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "force_integer_mv"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ---- intrabc_params ----

    fn roundtrip_intrabc(p: IntrabcParams, intra: bool, drl: bool) {
        let mut writer = BitWriter::new();
        write_intrabc_params(&mut writer, &p, intra, drl).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let parsed = parse_intrabc_params_full(&mut reader, intra, drl).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn intrabc_disallowed_round_trips() {
        roundtrip_intrabc(
            IntrabcParams {
                allow_intrabc: false,
                allow_global_intrabc: None,
                allow_local_intrabc: None,
                change_bvp_drl: None,
                max_bvp_drl_bits_minus_1: None,
            },
            true,
            true,
        );
    }

    #[test]
    fn intrabc_all_branches_round_trip() {
        // global=1 -> local read; change=1 -> max ns(2).
        roundtrip_intrabc(
            IntrabcParams {
                allow_intrabc: true,
                allow_global_intrabc: Some(true),
                allow_local_intrabc: Some(false),
                change_bvp_drl: Some(true),
                max_bvp_drl_bits_minus_1: Some(1),
            },
            true,
            true,
        );
        // global=0 -> local inferred (None); change=0.
        roundtrip_intrabc(
            IntrabcParams {
                allow_intrabc: true,
                allow_global_intrabc: Some(false),
                allow_local_intrabc: None,
                change_bvp_drl: Some(false),
                max_bvp_drl_bits_minus_1: None,
            },
            true,
            true,
        );
        // not intra -> no global/local; drl off -> no change.
        roundtrip_intrabc(
            IntrabcParams {
                allow_intrabc: true,
                allow_global_intrabc: None,
                allow_local_intrabc: None,
                change_bvp_drl: None,
                max_bvp_drl_bits_minus_1: None,
            },
            false,
            false,
        );
    }

    fn base_intrabc() -> IntrabcParams {
        IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(false),
            allow_local_intrabc: None,
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        }
    }

    fn assert_intrabc_rejected(p: IntrabcParams, intra: bool, drl: bool, what: &'static str) {
        let mut writer = BitWriter::new();
        let err = write_intrabc_params(&mut writer, &p, intra, drl).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what });
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn intrabc_global_present_without_gate_rejected() {
        // not intra, but allow_global_intrabc is Some -> non-canonical.
        let mut p = base_intrabc();
        p.change_bvp_drl = None;
        assert_intrabc_rejected(p, false, false, "allow_global_intrabc");
    }

    #[test]
    fn intrabc_local_present_without_global_rejected() {
        let mut p = base_intrabc();
        p.allow_local_intrabc = Some(true); // global is Some(false) -> local must be None
        assert_intrabc_rejected(p, true, true, "allow_local_intrabc");
    }

    #[test]
    fn intrabc_change_present_without_gate_rejected() {
        // drl off but change_bvp_drl Some -> non-canonical.
        let p = base_intrabc();
        assert_intrabc_rejected(p, true, false, "change_bvp_drl");
    }

    #[test]
    fn intrabc_max_out_of_ns_domain_rejected() {
        let p = IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(false),
            allow_local_intrabc: None,
            change_bvp_drl: Some(true),
            max_bvp_drl_bits_minus_1: Some(2), // ns(2) domain is {0, 1}
        };
        let mut writer = BitWriter::new();
        let err = write_intrabc_params(&mut writer, &p, true, true).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: 2
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
    use crate::headers::frame::{
        parse_frame_size, parse_intrabc_params_full, parse_screen_content_params_full,
    };
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn pack(bits: &[bool]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, b) in chunk.iter().enumerate() {
                byte |= u8::from(*b) << (7 - i);
            }
            out.push(byte);
        }
        out.extend_from_slice(&[0u8; 4]); // pad so the parser never hits EOF mid-field
        out
    }

    proptest! {
        /// Every parser-reachable frame_size round-trips byte-exactly.
        #[test]
        fn frame_size_round_trips(
            override_flag in any::<bool>(),
            w_bits in 1u32..=16,
            h_bits in 1u32..=16,
            bits in proptest::collection::vec(any::<bool>(), 0..40),
            dw in 1u32..=4096,
            dh in 1u32..=4096,
        ) {
            let packed = pack(&bits);
            let default_dims = Some((dw, dh));
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(Some(size)) =
                parse_frame_size(&mut reader, override_flag, w_bits, h_bits, default_dims)
            {
                let mut writer = BitWriter::new();
                write_frame_size(&mut writer, &size, override_flag, w_bits, h_bits, default_dims)
                    .unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed =
                    parse_frame_size(&mut reparse, override_flag, w_bits, h_bits, default_dims)
                        .unwrap()
                        .unwrap();
                prop_assert_eq!(reparsed, size);
            }
        }

        /// Every parser-reachable screen_content_params round-trips byte-exactly.
        #[test]
        fn scc_round_trips(
            seq_scc in 0u8..=2,
            seq_imv in 0u8..=2,
            bits in proptest::collection::vec(any::<bool>(), 0..8),
        ) {
            let packed = pack(&bits);
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(scc) = parse_screen_content_params_full(&mut reader, seq_scc, seq_imv) {
                let mut writer = BitWriter::new();
                write_screen_content_params(
                    &mut writer,
                    scc.allow_screen_content_tools,
                    scc.force_integer_mv,
                    seq_scc,
                    seq_imv,
                )
                .unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed =
                    parse_screen_content_params_full(&mut reparse, seq_scc, seq_imv).unwrap();
                prop_assert_eq!(reparsed, scc);
            }
        }

        /// Every parser-reachable intrabc_params round-trips byte-exactly.
        #[test]
        fn intrabc_round_trips(
            intra in any::<bool>(),
            drl in any::<bool>(),
            bits in proptest::collection::vec(any::<bool>(), 0..8),
        ) {
            let packed = pack(&bits);
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(p) = parse_intrabc_params_full(&mut reader, intra, drl) {
                let mut writer = BitWriter::new();
                write_intrabc_params(&mut writer, &p, intra, drl).unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed = parse_intrabc_params_full(&mut reparse, intra, drl).unwrap();
                prop_assert_eq!(reparsed, p);
            }
        }
    }
}
