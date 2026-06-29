// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `multi_frame_header_obu()` writer (AV2 v1.0.0 § 5.7,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-7`) — the inverse of
//! [`crate::hls::parse_multi_frame_header`].
//!
//! The OBU is, in parse order: `mfh_seq_header_id` `uvlc()`; `mfh_id_minus_1`
//! `uvlc()`; `mfh_frame_size_present_flag` `f(1)` and — only when set — the frame-size
//! payload (`mfh_frame_width_bits_minus_1` `f(4)`, `mfh_frame_height_bits_minus_1`
//! `f(4)`, `mfh_frame_width_minus_1` `f(width_bits)`, `mfh_frame_height_minus_1`
//! `f(height_bits)`, where `width_bits = mfh_frame_width_bits_minus_1 + 1`);
//! `mfh_deblocking_filter_update` `f(1)` and — only when set — four
//! `mfh_apply_deblocking_filter[i]` `f(1)`; `mfh_seg_info_present_flag` `f(1)` and —
//! only when set — `mfh_ext_seg_flag` `f(1)`, `mfh_allow_seg_info_change` `f(1)`, and
//! `seg_info(mfh_ext_seg_flag ? 16 : 8)` (§ 5.4.9). The nested `seg_info()` body is the
//! shared § 5.4.9 structure, inverted by [`crate::write::write_seg_info`] called with
//! `num_segments = if mfh_ext_seg_flag { 16 } else { 8 }`.
//!
//! `OBU_MULTI_FRAME_HEADER` is an **extensible** OBU type (§ 5.2.1), so the OBU tail is
//! the dispatch's generic extensible tail (`obu_extension_flag = 0` then
//! `trailing_bits()`); this writer emits the body, not the tail.
//!
//! `mfh_seq_header_id` and `mfh_id_minus_1` out of their § 6.x conformance range (the
//! validator flags them via [`crate::hls::MultiFrameHeader::seq_header_id_in_range`] /
//! [`crate::hls::MultiFrameHeader::mfh_id_in_range`], but the parser returns `Ok`) are
//! reproduced verbatim, never rejected — the `uvlc()` descriptor is their only domain
//! constraint.

use crate::hls::{MfhFrameSize, MultiFrameHeader};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::segment::write_seg_info;

/// `mfh_frame_width_bits_minus_1` / `mfh_frame_height_bits_minus_1` are `f(4)`.
const FRAME_SIZE_BITS_F4: u32 = 4;
/// `seg_info(16)` selector when `mfh_ext_seg_flag` is set; otherwise `seg_info(8)`.
const EXT_SEG_NUM_SEGMENTS: u8 = 16;
/// `seg_info(8)` selector when `mfh_ext_seg_flag` is clear.
const BASE_SEG_NUM_SEGMENTS: u8 = 8;

/// Writes a `multi_frame_header_obu()` body (AV2 v1.0.0 § 5.7), the inverse of
/// [`crate::hls::parse_multi_frame_header`]. The OBU header and the extensible OBU tail
/// are the dispatch's job ([`crate::write::write_complete_obu`]); this writes the typed
/// body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU
///   payload begins on a byte boundary).
/// - [`WriteError::NonCanonicalMultiFrameHeader`] for a constructed model the § 5.7
///   parser could never produce, so it would not round-trip. The `what` label names the
///   offending field:
///   - `"deblocking_apply_forced_false"`: `mfh_deblocking_filter_update` is `false` but
///     a `mfh_apply_deblocking_filter[i]` is `true` (the parser reads the four flags
///     only when an update is signalled, leaving the array all-`false` otherwise, so a
///     non-`false` array without an update is parser-unproducible).
///   - `"frame_width_bits"` / `"frame_height_bits"`: a stored `width_bits` /
///     `height_bits` outside `1..=16` (it is `mfh_frame_width_bits_minus_1 + 1` with
///     `mfh_frame_width_bits_minus_1` an `f(4)` value `0..=15`, so any value outside
///     `1..=16` could not have been read as `f(4) + 1`).
///   - `"seg_info_present_flag"`: `mfh_seg_info_present_flag` disagrees with the
///     presence of the three segment-info `Option`s (`mfh_ext_seg_flag`,
///     `mfh_allow_seg_info_change`, `segment_info`). The parser reads all three iff the
///     flag is set, so a flag-vs-`Option` disagreement is parser-unproducible.
/// - [`WriteError::NonCanonicalSequenceValue`] propagated from
///   [`crate::write::write_seg_info`] for a nested `seg_info()` body the § 5.4.9 parser
///   could not have produced.
/// - [`WriteError::ValueTooWide`] / [`WriteError::ValueOutOfRange`] from the primitive
///   writers for a field value outside its descriptor's domain (e.g. a
///   `mfh_frame_width_minus_1` that does not fit in `f(width_bits)`, or a
///   `mfh_seq_header_id` of `u32::MAX`, which the parser could not have produced).
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch
/// and appended only on full success), so a rejected model leaves `writer` unchanged and
/// the writer never panics.
pub fn write_multi_frame_header(writer: &mut BitWriter, mfh: &MultiFrameHeader) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let mut scratch = BitWriter::new();

    scratch.write_uvlc(mfh.mfh_seq_header_id)?;
    scratch.write_uvlc(mfh.mfh_id_minus_1)?;

    scratch.write_flag(mfh.mfh_frame_size.is_some())?;
    if let Some(frame_size) = &mfh.mfh_frame_size {
        write_frame_size(&mut scratch, frame_size)?;
    }

    scratch.write_flag(mfh.mfh_deblocking_filter_update)?;
    if mfh.mfh_deblocking_filter_update {
        for &apply in &mfh.mfh_apply_deblocking_filter {
            scratch.write_flag(apply)?;
        }
    } else if mfh.mfh_apply_deblocking_filter.iter().any(|&apply| apply) {
        return Err(non_canonical("deblocking_apply_forced_false"));
    }

    scratch.write_flag(mfh.mfh_seg_info_present_flag)?;
    write_seg_info_section(&mut scratch, mfh)?;

    writer.append(&scratch)
}

/// Writes the `mfh_frame_size_present_flag` payload (AV2 v1.0.0 § 5.7):
/// `mfh_frame_width_bits_minus_1` `f(4)`, `mfh_frame_height_bits_minus_1` `f(4)`,
/// `mfh_frame_width_minus_1` `f(width_bits)`, `mfh_frame_height_minus_1`
/// `f(height_bits)`. The stored `width_bits` / `height_bits` are the `+ 1` values
/// (`mfh_frame_*_bits_minus_1 + 1`), so they must lie in `1..=16` to be written back as
/// `f(4) + 1`.
fn write_frame_size(scratch: &mut BitWriter, frame_size: &MfhFrameSize) -> WriteResult<()> {
    let width_minus_1_bits = checked_bits_minus_1(frame_size.width_bits, "frame_width_bits")?;
    let height_minus_1_bits = checked_bits_minus_1(frame_size.height_bits, "frame_height_bits")?;
    scratch.write_bits_u8(width_minus_1_bits, FRAME_SIZE_BITS_F4)?;
    scratch.write_bits_u8(height_minus_1_bits, FRAME_SIZE_BITS_F4)?;
    scratch.write_bits(frame_size.width_minus_1, u32::from(frame_size.width_bits))?;
    scratch.write_bits(frame_size.height_minus_1, u32::from(frame_size.height_bits))
}

/// Validates a stored `bits` value (a `mfh_frame_*_bits_minus_1 + 1`) lies in `1..=16`
/// and returns the `f(4)` `mfh_frame_*_bits_minus_1` it must be written as. Rejecting a
/// `0` or `> 16` value avoids both a non-reproducible model and a subtraction underflow.
fn checked_bits_minus_1(bits: u8, what: &'static str) -> WriteResult<u8> {
    if !(1..=16).contains(&bits) {
        return Err(non_canonical(what));
    }
    Ok(bits - 1)
}

/// Writes the segment-info section gated on `mfh_seg_info_present_flag` (AV2 v1.0.0
/// § 5.7): `mfh_ext_seg_flag` `f(1)`, `mfh_allow_seg_info_change` `f(1)`, then
/// `seg_info(mfh_ext_seg_flag ? 16 : 8)` (§ 5.4.9). The parser reads all three iff the
/// flag is set, so a flag-vs-`Option` disagreement is rejected.
fn write_seg_info_section(scratch: &mut BitWriter, mfh: &MultiFrameHeader) -> WriteResult<()> {
    if mfh.mfh_seg_info_present_flag {
        let ext_seg = mfh
            .mfh_ext_seg_flag
            .ok_or_else(|| non_canonical("seg_info_present_flag"))?;
        let allow_change = mfh
            .mfh_allow_seg_info_change
            .ok_or_else(|| non_canonical("seg_info_present_flag"))?;
        let segment_info = mfh
            .segment_info
            .as_ref()
            .ok_or_else(|| non_canonical("seg_info_present_flag"))?;
        scratch.write_flag(ext_seg)?;
        scratch.write_flag(allow_change)?;
        let num_segments = if ext_seg {
            EXT_SEG_NUM_SEGMENTS
        } else {
            BASE_SEG_NUM_SEGMENTS
        };
        write_seg_info(scratch, segment_info, num_segments)?;
    } else if mfh.mfh_ext_seg_flag.is_some()
        || mfh.mfh_allow_seg_info_change.is_some()
        || mfh.segment_info.is_some()
    {
        return Err(non_canonical("seg_info_present_flag"));
    }
    Ok(())
}

/// Helper constructing the multi-frame-header-specific non-canonical reject with a stable
/// `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalMultiFrameHeader { what }
}

#[cfg(test)]
include!("multi_frame_header_tests.rs");
