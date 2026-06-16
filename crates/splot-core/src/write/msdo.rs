// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `multistream_decoder_operation_obu()` writer (AV2 v1.0.0 § 5.6,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-6`) — the inverse of
//! [`crate::hls::parse_msdo`].
//!
//! The OBU is flat fixed-width fields plus one bounded per-substream loop:
//! `num_streams_minus_2` `f(3)`, `multistream_profile_idc` `f(5)`, `multistream_level_idx` `f(5)`,
//! `multistream_tier` `f(1)`, `multistream_even_allocation_flag` `f(1)`, an optional
//! `multistream_large_picture_idc` `f(3)` (present iff allocation is **not** even), then
//! `num_streams_minus_2 + 2` per-substream entries (`sub_xlayer_id` `f(5)`,
//! `sub_stream_max_profile` `f(5)`, `sub_stream_max_level` `f(5)`, `sub_stream_max_tier` `f(1)`),
//! and `multistream_doh_constraint_flag` `f(1)`. `OBU_MSDO` is **not** extensible, so the OBU tail
//! is `trailing_bits()` only (the dispatch's generic tail with `is_extensible == false`).

use crate::hls::{MultistreamDecoderOperation, SubStreamConfig};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `num_streams_minus_2` is `f(3)`.
const NUM_STREAMS_MINUS_2_BITS: u32 = 3;
/// `multistream_profile_idc` / `multistream_level_idx` / the per-substream profile, level, and
/// xlayer ids are `f(5)`.
const F5: u32 = 5;
/// `multistream_tier` / `sub_stream_max_tier` / `multistream_large_picture_idc`-less flags are 1 or
/// 3 bits; `multistream_large_picture_idc` is `f(3)`.
const LARGE_PICTURE_IDC_BITS: u32 = 3;
/// `multistream_tier` and `sub_stream_max_tier` are `f(1)`.
const TIER_BITS: u32 = 1;

/// Writes a `multistream_decoder_operation_obu()` body (AV2 v1.0.0 § 5.6), the inverse of
/// [`crate::hls::parse_msdo`]. The OBU header and the `trailing_bits()` tail are the dispatch's job
/// ([`crate::write::write_complete_obu`]); this writes the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU payload begins on a
///   byte boundary).
/// - [`WriteError::NonCanonicalMsdo`] for a constructed model the § 5.6 parser could never produce: a
///   `sub_stream_count` that disagrees with `num_streams_minus_2 + 2` or overflows the sub-stream
///   array (`sub_stream_count`); a non-zero unused `sub_streams` slot (`unused_sub_stream`); or a
///   `multistream_large_picture_idc` presence that disagrees with `multistream_even_allocation_flag`
///   (`large_picture_idc_flag`).
/// - [`WriteError::ValueTooWide`] for a field value outside its descriptor's domain
///   (`multistream_profile_idc` / `multistream_large_picture_idc` / a per-substream field), from the
///   primitive writers.
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch and appended
/// only on full success), so a rejected model leaves `writer` unchanged and the writer never panics.
pub fn write_msdo(writer: &mut BitWriter, msdo: &MultistreamDecoderOperation) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    // § 5.6: the per-substream loop runs num_streams_minus_2 + 2 times, and the parser stores that
    // derived count in sub_stream_count and fills only the first `count` array slots. A model whose
    // stored count disagrees, or whose count overflows the fixed array (e.g. a constructed
    // num_streams_minus_2 >= 8), is one the parser could never have produced. Checked before the
    // slice below so the indexing cannot panic.
    let count = usize::from(msdo.num_streams_minus_2) + 2;
    if usize::from(msdo.sub_stream_count) != count || count > msdo.sub_streams.len() {
        return Err(WriteError::NonCanonicalMsdo {
            what: "sub_stream_count",
        });
    }
    // The parser leaves the unused slots zero, so a non-zero slot there is parser-unproducible and
    // would not round-trip (the writer emits only `count` entries).
    let zero = SubStreamConfig {
        sub_xlayer_id: 0,
        sub_stream_max_profile: 0,
        sub_stream_max_level: 0,
        sub_stream_max_tier: 0,
    };
    if msdo.sub_streams[count..].iter().any(|slot| *slot != zero) {
        return Err(WriteError::NonCanonicalMsdo {
            what: "unused_sub_stream",
        });
    }
    // § 5.6: multistream_large_picture_idc is read iff !multistream_even_allocation_flag, so the
    // parser ties Some(..) to the flag being clear. Reject a model that stores one without the other.
    if msdo.multistream_large_picture_idc.is_some() == msdo.multistream_even_allocation_flag {
        return Err(WriteError::NonCanonicalMsdo {
            what: "large_picture_idc_flag",
        });
    }

    let mut scratch = BitWriter::new();
    scratch.write_bits_u8(msdo.num_streams_minus_2, NUM_STREAMS_MINUS_2_BITS)?;
    scratch.write_bits_u8(msdo.multistream_profile_idc.get(), F5)?;
    scratch.write_bits_u8(msdo.multistream_level_idx, F5)?;
    scratch.write_bits_u8(msdo.multistream_tier, TIER_BITS)?;
    scratch.write_bit(u8::from(msdo.multistream_even_allocation_flag))?;
    if let Some(large_picture_idc) = msdo.multistream_large_picture_idc {
        scratch.write_bits_u8(large_picture_idc, LARGE_PICTURE_IDC_BITS)?;
    }
    for sub in &msdo.sub_streams[..count] {
        scratch.write_bits_u8(sub.sub_xlayer_id, F5)?;
        scratch.write_bits_u8(sub.sub_stream_max_profile, F5)?;
        scratch.write_bits_u8(sub.sub_stream_max_level, F5)?;
        scratch.write_bits_u8(sub.sub_stream_max_tier, TIER_BITS)?;
    }
    scratch.write_bit(u8::from(msdo.multistream_doh_constraint_flag))?;
    writer.append(&scratch)
}

// The round-trip / reject tests live in a sibling file (kept under the advisory source-line limit);
// `include!` pastes them into this module so their `super::*` resolves to the writer above.
#[cfg(test)]
include!("msdo_tests.rs");
