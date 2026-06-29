// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `buffer_removal_timing_obu()` writer (AV2 v1.0.0 § 5.12,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-12`) — the inverse of
//! [`crate::headers::buffer_removal_timing::parse_buffer_removal_timing`].
//!
//! The OBU has two forms selected by `br_ops_dependent_flag` (`f(1)`): the extended-layer form (a
//! single `br_time` `rg(4)`), and the operating-point-set form (`br_ops_id` `f(4)`, `br_ops_cnt`
//! `f(3)`, then `br_ops_cnt` per-operating-point entries of `br_decoder_model_present_op_flag`
//! `f(1)` plus an optional `br_time_op` `rg(4)`). `OBU_BUFFER_REMOVAL_TIMING` is **not** an
//! extensible OBU type, so the OBU tail is `trailing_bits()` only (the dispatch's generic tail with
//! `is_extensible == false`); this writer emits the body, not the tail.

use crate::headers::buffer_removal_timing::BufferRemovalTiming;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `br_ops_id` is a 4-bit field (`f(4)`).
const BR_OPS_ID_BITS: u32 = 4;
/// `br_ops_cnt` is a 3-bit field (`f(3)`).
const BR_OPS_COUNT_BITS: u32 = 3;
/// `br_time` / `br_time_op` use a `rg(4)` Rice-Golomb code.
const BR_TIME_RG_ORDER: u32 = 4;

/// Writes a `buffer_removal_timing_obu()` body (AV2 v1.0.0 § 5.12), the inverse of
/// [`crate::headers::buffer_removal_timing::parse_buffer_removal_timing`]. The OBU header and the
/// `trailing_bits()` tail are the dispatch's job ([`crate::write::write_complete_obu`]); this writes
/// the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU payload begins on a
///   byte boundary).
/// - [`WriteError::NonCanonicalBufferRemovalTiming`] for a constructed model the § 5.12 parser could
///   never produce: an `op_times` length that disagrees with `br_ops_cnt` (`op_count`), a
///   per-operating-point `index` that disagrees with its position (`op_index`), or a `br_time_op`
///   presence that disagrees with `br_decoder_model_present_op_flag` (`op_decoder_model_flag`).
/// - [`WriteError::ValueTooWide`] (`br_ops_id` / `br_ops_cnt` out of their field) or
///   [`WriteError::ValueOutOfRange`] (a `br_time` / `br_time_op` whose `rg(4)` quotient is `≥ 32`,
///   which the parser could not have produced) from the primitive writers.
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch and appended
/// only on full success), so a rejected model leaves `writer` unchanged and the writer never panics.
pub fn write_buffer_removal_timing(
    writer: &mut BitWriter,
    brt: &BufferRemovalTiming,
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let mut scratch = BitWriter::new();
    match brt {
        BufferRemovalTiming::ExtendedLayer { br_time } => {
            scratch.write_bit(0)?;
            scratch.write_rg(*br_time, BR_TIME_RG_ORDER)?;
        }
        BufferRemovalTiming::OperatingPointSet {
            br_ops_id,
            br_ops_cnt,
            op_times,
        } => {
            if op_times.len() != usize::from(*br_ops_cnt) {
                return Err(WriteError::NonCanonicalBufferRemovalTiming { what: "op_count" });
            }
            scratch.write_bit(1)?;
            scratch.write_bits_u8(*br_ops_id, BR_OPS_ID_BITS)?;
            scratch.write_bits_u8(*br_ops_cnt, BR_OPS_COUNT_BITS)?;
            for (i, op) in op_times.iter().enumerate() {
                if usize::from(op.index) != i {
                    return Err(WriteError::NonCanonicalBufferRemovalTiming { what: "op_index" });
                }
                if op.decoder_model_present != op.br_time_op.is_some() {
                    return Err(WriteError::NonCanonicalBufferRemovalTiming {
                        what: "op_decoder_model_flag",
                    });
                }
                scratch.write_flag(op.decoder_model_present)?;
                if let Some(br_time_op) = op.br_time_op {
                    scratch.write_rg(br_time_op, BR_TIME_RG_ORDER)?;
                }
            }
        }
    }
    writer.append(&scratch)
}

#[cfg(test)]
include!("buffer_removal_timing_tests.rs");
