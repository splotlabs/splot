// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `quantizer_matrix_obu()` writer (AV2 v1.0.0 § 5.13 / § 5.4.11,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-13`,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-11`) — the inverse of
//! [`crate::headers::quantizer_matrix::parse_quantizer_matrix`].
//!
//! **Canonicalizing writer (like § 5.14 film grain).** The parsed model is *lossy* versus the
//! wire: [`UserDefinedQmPlane::values`] holds the fully *decoded* coefficients (each `1..=255`),
//! not the wire `quant_delta`s, and the four optional compressions —
//! `qm_8x8_is_symmetric`, `qm_4x8_is_transpose_of_8x4`, `qm_copy_from_previous_plane`, and the
//! `quant2 == 0` coefficient-repeat — are collapsed away. Every one of those is *optional*, and
//! every decoded coefficient is non-zero, so the writer emits the **long form**: each per-plane
//! skip flag is written as `0`, and every cell is written as one `svlc()` `quant_delta` in the
//! AV2 2D diagonal scan order (`get_scan(txSz, TX_CLASS_2D)`, § 5.20.7.30,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30`), recomputing the minimal in-range
//! delta (`-128..=127`, § 6.4.11,
//! `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-11`) that drives the running
//! `quant` (initially `32`) to the target coefficient. The re-emission decodes to the exact
//! `values`, so the semantic round-trip holds; byte-exactness is **not** guaranteed (the original
//! may have used a shorter compressed form), exactly like film grain.
//!
//! Why the long form always works: a coefficient `v` in `1..=255` is reached from any running
//! `quant` by the unique delta in `-128..=127` congruent to `v - quant (mod 256)`, and the
//! resulting `quant2 = (quant + delta) & 255 == v` is non-zero, so it never trips the repeat
//! sentinel. A `0` coefficient is unrepresentable (the parser only ever decodes `1..=255`), so it
//! is rejected. The symmetric / transpose / copy paths are pure compressions of a matrix the long
//! form can always encode directly, so emitting their flags as `0` and re-encoding every cell is
//! always valid and never references a sibling transform or plane.
//!
//! `OBU_QUANTIZATION_MATRIX` is **not** an extensible OBU type (§ 5.2.1), so the OBU tail is the
//! dispatch's generic non-extensible tail (`trailing_bits()` only); this writer emits the body.

use crate::headers::quantizer_matrix::{
    FundamentalQmTransform, QuantizerMatrixObu, UserDefinedQmPlane, UserDefinedQmTransform,
    diagonal_scan_2d,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `qm_bit_map` is `f(NUM_CUSTOM_QMS)` = `f(15)` (AV2 § 5.13).
const QM_BIT_MAP_BITS: u32 = 15;
/// `NUM_CUSTOM_QMS` (AV2 § 3): the number of custom quantizer-matrix levels.
const NUM_CUSTOM_QMS: u8 = 15;
/// Initial running quantizer for the user-defined fill (`quant = 32`, AV2 § 5.4.11).
const INITIAL_QM_QUANT: i32 = 32;
/// The `& 255` modulus of the `quant2 = (quant + quant_delta) & 255` recurrence (AV2 § 5.4.11).
const QUANT_MODULUS: i32 = 256;
/// Inclusive upper bound of the conformant `quant_delta` range (AV2 § 6.4.11); the lower bound is
/// `-128`. A delta `> 127` is folded to its negative representative.
const QUANT_DELTA_MAX: i32 = 127;

/// Writes a `quantizer_matrix_obu()` body (AV2 v1.0.0 § 5.13), the inverse of
/// [`crate::headers::quantizer_matrix::parse_quantizer_matrix`], canonicalizing to the long form
/// (see the module docs). The OBU header and the non-extensible OBU tail are the dispatch's job
/// ([`crate::write::write_complete_obu`]); this writes the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU payload begins on
///   a byte boundary).
/// - [`WriteError::NonCanonicalQuantizationMatrix`] for a constructed model the § 5.13 parser could
///   never produce (a `num_planes` vs `qm_chroma_info_present_flag` disagreement, a `levels` list
///   that disagrees with the `qm_bit_map` set bits, an `is_default` vs `matrices` disagreement, a
///   transform / plane count / dimension / value-count mismatch, or a `0` coefficient); the `what`
///   label names the offending field.
/// - [`WriteError::ValueTooWide`] from [`BitWriter::write_bits`] for a `qm_bit_map` that does not fit
///   `f(15)` (a value the 15-bit reader could never have produced).
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch and appended
/// only on full success), so a rejected model leaves `writer` unchanged and the writer never panics.
pub fn write_quantizer_matrix(writer: &mut BitWriter, qm: &QuantizerMatrixObu) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let derived_num_planes: u8 = if qm.chroma_info_present { 3 } else { 1 };
    if qm.num_planes != derived_num_planes {
        return Err(non_canonical("num_planes"));
    }

    let set_bits: Vec<u8> = (0..NUM_CUSTOM_QMS)
        .filter(|&level| qm.qm_bit_map & (1u16 << level) != 0)
        .collect();
    if qm.levels.len() != set_bits.len() {
        return Err(non_canonical("level_count"));
    }

    let mut scratch = BitWriter::new();
    scratch.write_bits(u32::from(qm.qm_bit_map), QM_BIT_MAP_BITS)?;
    scratch.write_flag(qm.chroma_info_present)?;

    for (level, &bit) in qm.levels.iter().zip(&set_bits) {
        if level.level != bit {
            return Err(non_canonical("level_index"));
        }
        match (level.is_default, &level.matrices) {
            (true, None) => scratch.write_bit(1)?,
            (false, Some(matrices)) => {
                scratch.write_bit(0)?;
                write_user_defined_qm_level(&mut scratch, matrices, qm.num_planes)?;
            }
            _ => return Err(non_canonical("is_default_gate")),
        }
    }

    writer.append(&scratch)
}

/// Writes the `for t { for plane { user_defined_qm(level, t, plane) } }` loop of a single
/// non-default level (AV2 v1.0.0 § 5.13 / § 5.4.11). The `matrices` must be the three fundamental
/// transforms in `Fundamental_Tx_Size` order, each with `num_planes` planes.
fn write_user_defined_qm_level(
    scratch: &mut BitWriter,
    matrices: &[UserDefinedQmTransform],
    num_planes: u8,
) -> WriteResult<()> {
    if matrices.len() != FundamentalQmTransform::ALL.len() {
        return Err(non_canonical("transform_count"));
    }
    for (t, matrix) in matrices.iter().enumerate() {
        if matrix.transform != FundamentalQmTransform::ALL[t] {
            return Err(non_canonical("transform_order"));
        }
        if matrix.planes.len() != usize::from(num_planes) {
            return Err(non_canonical("plane_count"));
        }
        for (plane_idx, plane) in matrix.planes.iter().enumerate() {
            write_user_defined_qm_plane(scratch, plane, t, plane_idx)?;
        }
    }
    Ok(())
}

/// Writes one `user_defined_qm(level, t, plane)` matrix in canonical long form (AV2 v1.0.0
/// § 5.4.11): the skip flags (`qm_copy_from_previous_plane` for `plane > 0`, `qm_8x8_is_symmetric`
/// for `t == 0`, `qm_4x8_is_transpose_of_8x4` for `t == 2`) emitted as `0`, then one `svlc()`
/// `quant_delta` per cell in 2D diagonal scan order.
fn write_user_defined_qm_plane(
    scratch: &mut BitWriter,
    plane: &UserDefinedQmPlane,
    t: usize,
    plane_idx: usize,
) -> WriteResult<()> {
    let (width, height) = FundamentalQmTransform::ALL[t].dimensions();
    if plane.width != width || plane.height != height {
        return Err(non_canonical("plane_dimensions"));
    }
    let (w, h) = (usize::from(width), usize::from(height));
    if plane.values.len() != w * h {
        return Err(non_canonical("plane_value_count"));
    }

    if plane_idx > 0 {
        scratch.write_bit(0)?;
    }
    if t == 0 || t == 2 {
        scratch.write_bit(0)?;
    }

    let scan = diagonal_scan_2d(w, h);
    let mut quant = INITIAL_QM_QUANT;
    for pos in scan {
        let value = plane.values[pos];
        if value == 0 {
            return Err(non_canonical("coefficient_zero"));
        }
        let target = i32::from(value);
        scratch.write_svlc(quant_delta_for(quant, target))?;
        quant = target;
    }
    Ok(())
}

/// Returns the unique `quant_delta` in `-128..=127` (AV2 § 6.4.11) with
/// `(quant + quant_delta) & 255 == target`, i.e. the signed representative of `target - quant`
/// modulo 256 (AV2 § 5.4.11 `quant2` recurrence). `target` is `1..=255` and `quant` is `1..=255`
/// (or the initial `32`), so `rem_euclid` never underflows and the result is in range.
fn quant_delta_for(quant: i32, target: i32) -> i32 {
    let raw = (target - quant).rem_euclid(QUANT_MODULUS);
    if raw > QUANT_DELTA_MAX {
        raw - QUANT_MODULUS
    } else {
        raw
    }
}

/// Helper constructing the quantizer-matrix-specific non-canonical reject with a stable `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalQuantizationMatrix { what }
}

#[cfg(test)]
include!("quantizer_matrix_tests.rs");
