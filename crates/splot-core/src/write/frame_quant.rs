// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **quantization** writers (`ENC-BITSTREAM-WRITER`) — the inverses
//! of the § 5.18.6 / § 5.18.7.8 / § 5.18.2 quantization parsers in
//! [`crate::headers::frame`]:
//!
//! - [`write_read_delta_q`] — `read_delta_q()` (§ 5.18.6.3,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-3`): `delta_coded` `f(1)`,
//!   then `delta_q` `su(7)` when coded.
//! - [`write_quantization_params`] — `quantization_params()` (§ 5.18.6.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`): `base_q_idx` `f(n)` then
//!   the gated luma/chroma `read_delta_q()` cascade.
//! - [`write_setup_qm_params`] — `setup_qm_params()` (§ 5.18.6.2,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`): `using_qmatrix`,
//!   `pic_qm_num_minus_1`, and the per-level `qm_y` / `qm_uv_same_as_y` / `qm_u` / `qm_v`
//!   cascade.
//! - [`write_delta_q_params`] — `delta_q_params()` (§ 5.18.7.8,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8`): the `base_q_idx`-gated
//!   `delta_q_present` and `delta_q_res`.
//! - [`write_lossless_info`] — the § 5.18.2 per-segment lossless/QM derivation tail
//!   (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`): the recovered per-segment
//!   `qm_index`, plus `allow_tcq` / `allow_parity_hiding`.
//!
//! Like the other frame-header config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed structure back to bits via [`BitWriter`].
//! Each writer threads the same gating inputs the parser receives (the
//! [`CoreSeqQuantView`] sequence flags, `tip_frame_as_output`, `segmentation_enabled`,
//! `base_q_idx`, …) and validates the whole structure before any bit is written
//! (reject-before-write): every reject path leaves `writer.bit_len() == 0`.
//!
//! **Canonical encodings (semantic round-trip universal; byte-exact on the canonical
//! subset).** Several syntax elements admit redundant encodings of the same modeled value,
//! exactly like the sequence writer's minimal-length LEB128 size prefix
//! ([`crate::write::obu::write_annexb_obu`], § 4.11.6) and the uniform tile-loop's
//! shortest-`tileColsLog2` target ([`crate::write::seq_tile`], § 5.18.7.3). For each, the
//! writer emits the canonical (shortest / equality-preserving) form, so
//! `parse(write(x)) == x` holds for every parser-reachable model while byte-exactness is
//! guaranteed only on the canonical subset. Three are redundant *coded* encodings (1–3); the
//! fourth (4) is a parser read-and-discard:
//!
//! 1. `read_delta_q()` (§ 5.18.6.3): `delta_q == 0` is written as `delta_coded == 0` (no
//!    `su(7)`), never as `delta_coded == 1` with a coded `su(7)` zero. The parser maps both
//!    to `0`, so the model loses the distinction; the writer always picks the no-`su` form.
//! 2. `setup_qm_params()` (§ 5.18.6.2): when `qm_u == qm_y && qm_v == qm_y` the level is
//!    written with `qm_uv_same_as_y == 1` (no `qm_u` / `qm_v` bits), never the explicit
//!    `qm_uv_same_as_y == 0` form that happens to repeat `qm_y`. Both decode to the same
//!    triple.
//! 3. The § 5.18.2 `qm_index`: the parser stores only the *resolved* level triple
//!    `SegQMLevel[plane][segmentId]`, discarding the coded `qm_index`. The writer recovers
//!    the smallest `qm_index` over the full `f(CeilLog2(qmNum))` coded domain whose level
//!    triple equals the stored one (including indices `>= qmNum`, which reference the zeroed
//!    default levels the parser also indexes). A model whose stored triple matches no level
//!    is non-canonical and rejected.
//! 4. `quantization_params()` (§ 5.18.6.1) under `equal_ac_dc_q`: when
//!    `equal_ac_dc_q == 1`, the parser still *reads* the chroma DC `read_delta_q()` field
//!    (gated only on `uv_dc_delta_q_enabled`), then overwrites the result with the chroma AC
//!    value, discarding the coded DC. The model therefore retains `DeltaQ*Dc == DeltaQ*Ac`
//!    regardless of what DC was coded. The writer re-emits the retained DC value (equal to
//!    the AC value on a canonical model), so the bit count and reparsed model match; the
//!    original discarded DC bits are not recovered byte-exactly.

use crate::headers::frame::{
    CoreSeqQuantView, DeltaQParams, LosslessInfo, MAX_PIC_QM_NUM, QmSetLevels, QuantizationParams,
    SegmentationParams, SetupQmParams, ceil_log2, get_qindex_ignore_delta_q,
};
use crate::segment::MAX_SEGMENTS;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `su(7)` signed domain for `read_delta_q()`'s `delta_q` field (AV2 v1.0.0 § 5.18.6.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-3`): the two's-complement range
/// `-(2^6) ..= 2^6 - 1`, matching [`BitWriter::write_su`] / [`crate::bitio::BitReader::read_su`]
/// with `n == 7`.
const DELTA_Q_MIN: i32 = -(1 << 6);
/// Upper bound of the `su(7)` `delta_q` domain (see [`DELTA_Q_MIN`]).
const DELTA_Q_MAX: i32 = (1 << 6) - 1;

/// `qm_y` / `qm_u` / `qm_v` are `f(4)` (AV2 v1.0.0 § 5.18.6.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`), so each level is `0..16`.
const QM_LEVEL_MAX_PLUS_1: u8 = 16;

/// Writes `read_delta_q()` (AV2 v1.0.0 § 5.18.6.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-3`), the inverse of
/// [`crate::headers::frame::read_delta_q`]: `delta_coded` `f(1)`, then `delta_q` `su(7)`
/// when coded.
///
/// **Canonicalization 1** (see the module docs): `delta_q == 0` is written as
/// `delta_coded == 0` with no `su(7)` field — never the redundant `delta_coded == 1` form
/// with a coded zero. The parser maps both to `0`, so the model cannot distinguish them and
/// the writer always emits the shorter, canonical form. Semantic round-trip
/// (`parse(write(d)) == d`) is universal; byte-exactness holds on this canonical subset.
///
/// The whole value is validated before any bit is written: a non-zero `delta_q` outside the
/// `su(7)` domain `[-64, 63]` (which [`BitWriter::write_su`] would otherwise reject) is
/// rejected first, so the reject path leaves the writer empty.
///
/// # Errors
/// [`WriteError::ValueOutOfRange`] if `delta_q` is outside the `su(7)` domain `[-64, 63]`.
pub fn write_read_delta_q(writer: &mut BitWriter, delta_q: i32) -> WriteResult<()> {
    check_delta_q_encodable(delta_q)?;
    if delta_q == 0 {
        writer.write_bit(0)
    } else {
        writer.write_bit(1)?;
        writer.write_su(delta_q, 7)
    }
}

/// Validates that `delta_q` is within the `su(7)` domain (AV2 v1.0.0 § 5.18.6.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-3`) before any bit is written.
fn check_delta_q_encodable(delta_q: i32) -> WriteResult<()> {
    if !(DELTA_Q_MIN..=DELTA_Q_MAX).contains(&delta_q) {
        return Err(WriteError::ValueOutOfRange {
            descriptor: "su",
            value: i64::from(delta_q),
        });
    }
    Ok(())
}

/// Writes `quantization_params()` (AV2 v1.0.0 § 5.18.6.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`), the inverse of
/// [`crate::headers::frame::parse_quantization_params`].
///
/// Writes `base_q_idx` `f(n)` with `n = BitDepth == 8 ? 8 : 9`, then the same gated luma /
/// chroma `read_delta_q()` cascade the parser reads, threading `tip_frame_as_output`
/// (`TipFrameMode == TIP_FRAME_AS_OUTPUT`, always `false` on the intra path). Each gated-off
/// or inferred field must equal what the parser would derive (a delta whose gate is off must
/// be `0`; an `equal_ac_dc_q`-inferred `DeltaQUDc == DeltaQUAc`; a `!diff_uv_delta`-inferred
/// `DeltaQV* == DeltaQU*`; `diff_uv_delta` only set when its gate reads it). The model is
/// fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// - [`WriteError::ValueTooWide`] if `base_q_idx` does not fit its `f(n)` field.
/// - [`WriteError::ValueOutOfRange`] if a coded `read_delta_q()` value is outside the `su(7)`
///   domain.
/// - [`WriteError::NonCanonicalFrameHeader`] if an inferred / gated-off field disagrees with
///   the § 5.18.6.1 derivation.
pub fn write_quantization_params(
    writer: &mut BitWriter,
    params: &QuantizationParams,
    quant: &CoreSeqQuantView,
    tip_frame_as_output: bool,
) -> WriteResult<()> {
    check_quantization_encodable(params, quant, tip_frame_as_output)?;

    let n = if quant.bit_depth == 8 { 8 } else { 9 };
    writer.write_bits(params.base_q_idx, n)?;

    if !tip_frame_as_output && quant.y_dc_delta_q_enabled {
        write_read_delta_q(writer, params.delta_q_y_dc)?;
    }
    if quant.num_planes > 1
        && (quant.uv_ac_delta_q_enabled || (!tip_frame_as_output && quant.uv_dc_delta_q_enabled))
    {
        if quant.separate_uv_delta_q {
            writer.write_flag(params.diff_uv_delta)?;
        }
        if !tip_frame_as_output && quant.uv_dc_delta_q_enabled {
            write_read_delta_q(writer, params.delta_q_u_dc)?;
        }
        if quant.uv_ac_delta_q_enabled {
            write_read_delta_q(writer, params.delta_q_u_ac)?;
        }
        if params.diff_uv_delta {
            if !tip_frame_as_output && quant.uv_dc_delta_q_enabled {
                write_read_delta_q(writer, params.delta_q_v_dc)?;
            }
            if quant.uv_ac_delta_q_enabled {
                write_read_delta_q(writer, params.delta_q_v_ac)?;
            }
        }
    }
    Ok(())
}

/// Validates a [`QuantizationParams`] is a model the § 5.18.6.1
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`) parser could have produced,
/// before any bit is written. Every gated-off or inferred field must equal the parser's
/// derivation; every coded `read_delta_q()` value must be in the `su(7)` domain.
fn check_quantization_encodable(
    params: &QuantizationParams,
    quant: &CoreSeqQuantView,
    tip_frame_as_output: bool,
) -> WriteResult<()> {
    let n = if quant.bit_depth == 8 { 8 } else { 9 };
    if params.base_q_idx >= (1u32 << n) {
        return Err(WriteError::ValueTooWide {
            value: u64::from(params.base_q_idx),
            width_bits: n,
        });
    }

    let y_dc_coded = !tip_frame_as_output && quant.y_dc_delta_q_enabled;
    if y_dc_coded {
        check_delta_q_encodable(params.delta_q_y_dc)?;
    } else if params.delta_q_y_dc != 0 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "delta_q_y_dc",
        });
    }

    let chroma_block = quant.num_planes > 1
        && (quant.uv_ac_delta_q_enabled || (!tip_frame_as_output && quant.uv_dc_delta_q_enabled));
    if !chroma_block {
        if params.diff_uv_delta
            || params.delta_q_u_dc != 0
            || params.delta_q_u_ac != 0
            || params.delta_q_v_dc != 0
            || params.delta_q_v_ac != 0
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "quant_chroma_delta",
            });
        }
        return Ok(());
    }

    if !quant.separate_uv_delta_q && params.diff_uv_delta {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "diff_uv_delta",
        });
    }

    let uv_dc_coded = !tip_frame_as_output && quant.uv_dc_delta_q_enabled;
    let uv_ac_coded = quant.uv_ac_delta_q_enabled;

    check_chroma_ac(params.delta_q_u_ac, uv_ac_coded, "delta_q_u_ac")?;
    check_chroma_dc(
        params.delta_q_u_dc,
        params.delta_q_u_ac,
        uv_dc_coded,
        quant.equal_ac_dc_q,
        "delta_q_u_dc",
    )?;

    if params.diff_uv_delta {
        check_chroma_ac(params.delta_q_v_ac, uv_ac_coded, "delta_q_v_ac")?;
        check_chroma_dc(
            params.delta_q_v_dc,
            params.delta_q_v_ac,
            uv_dc_coded,
            quant.equal_ac_dc_q,
            "delta_q_v_dc",
        )?;
    } else if params.delta_q_v_dc != params.delta_q_u_dc
        || params.delta_q_v_ac != params.delta_q_u_ac
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "quant_v_inferred",
        });
    }
    Ok(())
}

/// Validates a chroma AC delta (AV2 v1.0.0 § 5.18.6.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`): when coded it must lie in the
/// `su(7)` domain, else it is inferred `0`.
fn check_chroma_ac(value: i32, coded: bool, what: &'static str) -> WriteResult<()> {
    if coded {
        check_delta_q_encodable(value)
    } else if value != 0 {
        Err(WriteError::NonCanonicalFrameHeader { what })
    } else {
        Ok(())
    }
}

/// Validates a chroma DC delta (AV2 v1.0.0 § 5.18.6.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`). The DC `read_delta_q()` is
/// emitted whenever `dc_coded` (`!tip && uv_dc_delta_q_enabled`), independent of
/// `equal_ac_dc_q`. When `equal_ac_dc_q` the parser overwrites the read DC with the AC value,
/// so the retained model `dc` must equal `ac` (canonicalization 4) — but the field is still
/// coded when `dc_coded`, so the emitted `dc` value (`== ac`) must lie in the `su(7)` domain.
/// When `!dc_coded`, the DC is inferred `0` (or `ac` under `equal_ac_dc_q`).
fn check_chroma_dc(
    dc: i32,
    ac: i32,
    dc_coded: bool,
    equal_ac_dc_q: bool,
    what: &'static str,
) -> WriteResult<()> {
    if equal_ac_dc_q {
        if dc != ac {
            return Err(WriteError::NonCanonicalFrameHeader { what });
        }
        if dc_coded {
            return check_delta_q_encodable(dc);
        }
        Ok(())
    } else if dc_coded {
        check_delta_q_encodable(dc)
    } else if dc != 0 {
        Err(WriteError::NonCanonicalFrameHeader { what })
    } else {
        Ok(())
    }
}

/// Writes `setup_qm_params()` (AV2 v1.0.0 § 5.18.6.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`), the inverse of
/// [`crate::headers::frame::parse_setup_qm_params`].
///
/// Writes `using_qmatrix` `f(1)`; when set, `pic_qm_num_minus_1` `f(2)` only when
/// `segmentation_enabled` (else inferred `0`), then the per-level cascade `qm_y` `f(4)`,
/// and for `NumPlanes > 1` the `qm_uv_same_as_y` `f(1)` / `qm_u` `f(4)` / `qm_v` `f(4)`
/// fields.
///
/// **Canonicalization 2** (see the module docs): a level with `qm_u == qm_y && qm_v == qm_y`
/// is written as `qm_uv_same_as_y == 1` (no `qm_u` / `qm_v` bits), never the explicit
/// `qm_uv_same_as_y == 0` form that repeats `qm_y`. Both decode to the same triple, so
/// semantic round-trip is universal; byte-exactness holds on this canonical subset.
///
/// The model is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// - [`WriteError::ValueTooWide`] if `pic_qm_num_minus_1` (`f(2)`) or any `qm_*` (`f(4)`)
///   overflows its field.
/// - [`WriteError::NonCanonicalFrameHeader`] if `using_qmatrix == false` carries non-default
///   `pic_qm_num_minus_1` / `levels`; if `pic_qm_num_minus_1` is non-zero while
///   `!segmentation_enabled` (inferred `0`); or if a `!separate_uv_delta_q` level has
///   `qm_v != qm_u` (the parser copies `qm_v = qm_u` with no read).
pub fn write_setup_qm_params(
    writer: &mut BitWriter,
    qm: &SetupQmParams,
    quant: &CoreSeqQuantView,
    segmentation_enabled: bool,
) -> WriteResult<()> {
    check_setup_qm_encodable(qm, quant, segmentation_enabled)?;

    writer.write_flag(qm.using_qmatrix)?;
    if !qm.using_qmatrix {
        return Ok(());
    }
    if segmentation_enabled {
        writer.write_bits_u8(qm.pic_qm_num_minus_1, 2)?;
    }
    let qm_num = usize::from(qm.pic_qm_num_minus_1) + 1;
    for level in qm.levels.iter().take(qm_num) {
        writer.write_bits_u8(level.qm_y, 4)?;
        if quant.num_planes > 1 {
            let same_as_y = level.qm_u == level.qm_y && level.qm_v == level.qm_y;
            writer.write_flag(same_as_y)?;
            if !same_as_y {
                writer.write_bits_u8(level.qm_u, 4)?;
                if quant.separate_uv_delta_q {
                    writer.write_bits_u8(level.qm_v, 4)?;
                }
            }
        }
    }
    Ok(())
}

/// Validates a [`SetupQmParams`] is a model the § 5.18.6.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`) parser could have produced,
/// before any bit is written.
fn check_setup_qm_encodable(
    qm: &SetupQmParams,
    quant: &CoreSeqQuantView,
    segmentation_enabled: bool,
) -> WriteResult<()> {
    if !qm.using_qmatrix {
        if qm.pic_qm_num_minus_1 != 0 || qm.levels.iter().any(|l| *l != QmSetLevels::default()) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "setup_qm_disabled",
            });
        }
        return Ok(());
    }

    if qm.pic_qm_num_minus_1 >= 4 {
        return Err(WriteError::ValueTooWide {
            value: u64::from(qm.pic_qm_num_minus_1),
            width_bits: 2,
        });
    }
    if !segmentation_enabled && qm.pic_qm_num_minus_1 != 0 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "pic_qm_num_minus_1",
        });
    }

    let qm_num = usize::from(qm.pic_qm_num_minus_1) + 1;
    for (i, level) in qm.levels.iter().enumerate() {
        if i < qm_num {
            check_qm_level_value(level.qm_y)?;
            if quant.num_planes > 1 {
                check_qm_level_value(level.qm_u)?;
                check_qm_level_value(level.qm_v)?;
                if !quant.separate_uv_delta_q && level.qm_v != level.qm_u {
                    return Err(WriteError::NonCanonicalFrameHeader { what: "qm_v" });
                }
            } else if level.qm_u != 0 || level.qm_v != 0 {
                return Err(WriteError::NonCanonicalFrameHeader {
                    what: "qm_monochrome",
                });
            }
        } else if *level != QmSetLevels::default() {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "qm_level_beyond_num",
            });
        }
    }
    Ok(())
}

/// Validates a single `qm_*` level value (`f(4)`, AV2 v1.0.0 § 5.18.6.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`).
fn check_qm_level_value(value: u8) -> WriteResult<()> {
    if value >= QM_LEVEL_MAX_PLUS_1 {
        return Err(WriteError::ValueTooWide {
            value: u64::from(value),
            width_bits: 4,
        });
    }
    Ok(())
}

/// Writes `delta_q_params()` (AV2 v1.0.0 § 5.18.7.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8`), the inverse of
/// [`crate::headers::frame::parse_delta_q_params`].
///
/// Writes `delta_q_present` `f(1)` only when `base_q_idx > 0` (else inferred `0`), then
/// `delta_q_res` `f(2)` only when `delta_q_present` (else inferred `0`). The model is fully
/// validated before any bit is written (reject-before-write).
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] if `delta_q_present` is set while
/// `base_q_idx == 0` (inferred `0`), if `delta_q_res` is non-zero while `!delta_q_present`
/// (inferred `0`), or if `delta_q_res >= 4` (outside its `f(2)` field).
pub fn write_delta_q_params(
    writer: &mut BitWriter,
    dq: &DeltaQParams,
    base_q_idx: u32,
) -> WriteResult<()> {
    check_delta_q_params_encodable(*dq, base_q_idx)?;

    if base_q_idx > 0 {
        writer.write_flag(dq.delta_q_present)?;
    }
    if dq.delta_q_present {
        writer.write_bits_u8(dq.delta_q_res, 2)?;
    }
    Ok(())
}

/// Validates a [`DeltaQParams`] is a model the § 5.18.7.8
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8`) parser could have produced.
fn check_delta_q_params_encodable(dq: DeltaQParams, base_q_idx: u32) -> WriteResult<()> {
    if base_q_idx == 0 && dq.delta_q_present {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "delta_q_present",
        });
    }
    if !dq.delta_q_present && dq.delta_q_res != 0 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "delta_q_res",
        });
    }
    if dq.delta_q_res >= 4 {
        return Err(WriteError::ValueTooWide {
            value: u64::from(dq.delta_q_res),
            width_bits: 2,
        });
    }
    Ok(())
}

/// Writes the § 5.18.2 per-segment lossless/QM derivation tail (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`), the inverse of
/// [`crate::headers::frame::parse_lossless_info`].
///
/// Re-derives `LosslessArray` / `CodedLossless` / `HasLosslessSegment` exactly as the parser
/// does (via the crate-internal `get_qindex_ignore_delta_q` and the § 5.18.2 delta-sum
/// formula) and rejects a stored value that disagrees, before any bit is written. Then, when
/// `using_qmatrix`, writes
/// the recovered `qm_index` `f(CeilLog2(qmNum))` for each non-lossless segment, and reproduces
/// `allow_tcq` (`f(1)` only when `!CodedLossless && choose_tcq_per_frame`, else inferred) and
/// `allow_parity_hiding` (`f(1)` only when `!CodedLossless && enable_parity_hiding &&
/// !allow_tcq`, else inferred `false`).
///
/// **Canonicalization 3** (see the module docs): the parser stores only the resolved level
/// triple `SegQMLevel[plane][segmentId]`, discarding the coded `qm_index`. The writer recovers
/// the smallest `qm_index` in `0..qmNum` whose `levels[qm_index]` triple equals the stored one
/// (a lossless segment stores `[15, 15, 15]` and codes no `qm_index`). A stored triple matching
/// no level set is non-canonical. `CeilLog2(qmNum)` can be `0` (`qmNum == 1`), in which case no
/// bits are written but the triple is still validated against the single level set.
///
/// The model is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// - [`WriteError::NonCanonicalFrameHeader`] if a stored `lossless_array` / `coded_lossless` /
///   `has_lossless_segment` entry disagrees with the re-derivation; if a non-lossless
///   segment's stored level triple matches no `levels[..qmNum]` set; if a lossless segment's
///   triple is not `[15, 15, 15]`; or if `allow_tcq` / `allow_parity_hiding` disagree with
///   their inferred (gated-off) values.
#[allow(clippy::too_many_arguments)]
pub fn write_lossless_info(
    writer: &mut BitWriter,
    info: &LosslessInfo,
    quant: &CoreSeqQuantView,
    quantization: &QuantizationParams,
    qm: &SetupQmParams,
    delta_q: &DeltaQParams,
    segmentation: &SegmentationParams,
    max_segments: u8,
) -> WriteResult<()> {
    let derived = check_lossless_encodable(
        info,
        quant,
        quantization,
        qm,
        *delta_q,
        segmentation,
        max_segments,
    )?;

    if qm.using_qmatrix {
        let qm_num = u32::from(qm.pic_qm_num_minus_1) + 1;
        let qm_index_bits = ceil_log2(qm_num);
        for (&lossless, &qm_index) in info
            .lossless_array
            .iter()
            .zip(derived.qm_indices.iter())
            .take(derived.count)
        {
            if !lossless {
                writer.write_bits(u32::from(qm_index), qm_index_bits)?;
            }
        }
    }

    if !derived.coded_lossless && quant.choose_tcq_per_frame {
        writer.write_flag(info.allow_tcq)?;
    }
    if !derived.coded_lossless && quant.enable_parity_hiding && !info.allow_tcq {
        writer.write_flag(info.allow_parity_hiding)?;
    }
    Ok(())
}

/// The re-derived § 5.18.2 state the writer needs before emitting any bit: the loop count,
/// `CodedLossless`, and the recovered per-segment `qm_index` (canonicalization 3).
struct DerivedLossless {
    /// `min(max_segments, MAX_SEGMENTS)`: the spec loop bound.
    count: usize,
    /// Re-derived `CodedLossless`.
    coded_lossless: bool,
    /// Recovered `qm_index` per segment (meaningful only for non-lossless segments when
    /// `using_qmatrix`; `0` otherwise).
    qm_indices: [u8; MAX_SEGMENTS],
}

/// Validates a [`LosslessInfo`] is a model the § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) parser could have produced, and
/// recovers the per-segment `qm_index` (canonicalization 3), before any bit is written.
///
/// Re-derives `LosslessArray` / `CodedLossless` / `HasLosslessSegment` exactly as
/// [`crate::headers::frame::parse_lossless_info`] does — every subtraction is in `i64` and
/// every index is `min`-bounded against [`MAX_SEGMENTS`], so a hostile constructed model
/// never panics.
fn check_lossless_encodable(
    info: &LosslessInfo,
    quant: &CoreSeqQuantView,
    quantization: &QuantizationParams,
    qm: &SetupQmParams,
    delta_q: DeltaQParams,
    segmentation: &SegmentationParams,
    max_segments: u8,
) -> WriteResult<DerivedLossless> {
    check_setup_qm_encodable(qm, quant, segmentation.segmentation_enabled)?;

    let count = usize::from(max_segments).min(MAX_SEGMENTS);

    let mut coded_lossless = true;
    let mut has_lossless_segment = false;
    let mut qm_indices = [0u8; MAX_SEGMENTS];

    for (segment_id, ((qm_index_slot, &stored_lossless), &stored_levels)) in qm_indices
        .iter_mut()
        .zip(info.lossless_array.iter())
        .zip(info.seg_qm_levels.iter())
        .enumerate()
        .take(count)
    {
        let qindex =
            get_qindex_ignore_delta_q(quant, quantization.base_q_idx, segmentation, segment_id);
        let lossless = qindex == 0
            && !delta_q.delta_q_present
            && i64::from(quantization.delta_q_y_dc) + i64::from(quant.base_y_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_u_dc) + i64::from(quant.base_uv_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_v_dc) + i64::from(quant.base_uv_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_u_ac) + i64::from(quant.base_uv_ac_delta_q) <= 0
            && i64::from(quantization.delta_q_v_ac) + i64::from(quant.base_uv_ac_delta_q) <= 0;

        if stored_lossless != lossless {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "lossless_array",
            });
        }
        if lossless {
            has_lossless_segment = true;
        } else {
            coded_lossless = false;
        }

        if qm.using_qmatrix {
            if lossless {
                if stored_levels != [15, 15, 15] {
                    return Err(WriteError::NonCanonicalFrameHeader {
                        what: "seg_qm_level_lossless",
                    });
                }
            } else {
                *qm_index_slot = recover_qm_index(qm, stored_levels)?;
            }
        } else if stored_levels != [0, 0, 0] {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "seg_qm_level_disabled",
            });
        }
    }

    if info.lossless_array.iter().skip(count).any(|&l| l)
        || info
            .seg_qm_levels
            .iter()
            .skip(count)
            .any(|&t| t != [0, 0, 0])
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "lossless_tail",
        });
    }

    if info.coded_lossless != coded_lossless {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "coded_lossless",
        });
    }
    if info.has_lossless_segment != has_lossless_segment {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "has_lossless_segment",
        });
    }

    let tcq_coded = !coded_lossless && quant.choose_tcq_per_frame;
    if !tcq_coded {
        let inferred = if coded_lossless {
            false
        } else {
            quant.enable_tcq
        };
        if info.allow_tcq != inferred {
            return Err(WriteError::NonCanonicalFrameHeader { what: "allow_tcq" });
        }
    }
    let ph_coded = !coded_lossless && quant.enable_parity_hiding && !info.allow_tcq;
    if !ph_coded && info.allow_parity_hiding {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "allow_parity_hiding",
        });
    }

    Ok(DerivedLossless {
        count,
        coded_lossless,
        qm_indices,
    })
}

/// Recovers the smallest `qm_index` whose `levels[qm_index]` triple equals `stored`
/// (canonicalization 3, AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
///
/// The parser reads `qm_index` as `f(CeilLog2(qmNum))` and indexes `levels[qm_index]` for
/// **any** value in that field's `0 ..= 2^CeilLog2(qmNum) - 1` range (entries at or beyond
/// `qmNum` are the zeroed defaults). The writer therefore searches that full coded domain —
/// not just `0..qmNum` — so it faithfully inverts a stream that coded an index `>= qmNum`
/// referencing a default level. The result fits the `CeilLog2(qmNum)`-bit field by
/// construction. Returns a typed reject (never panics) when no level reproduces the triple —
/// a constructed model the parser could not have produced.
fn recover_qm_index(qm: &SetupQmParams, stored: [u8; 3]) -> WriteResult<u8> {
    let qm_index_bits = ceil_log2(u32::from(qm.pic_qm_num_minus_1) + 1);
    let coded_domain = (1usize << qm_index_bits).min(MAX_PIC_QM_NUM);
    for (i, level) in qm.levels.iter().enumerate().take(coded_domain) {
        if [level.qm_y, level.qm_u, level.qm_v] == stored {
            return Ok(i as u8);
        }
    }
    Err(WriteError::NonCanonicalFrameHeader {
        what: "seg_qm_level",
    })
}

#[cfg(test)]
include!("frame_quant_tests.rs");
#[cfg(test)]
include!("frame_quant_proptests.rs");
