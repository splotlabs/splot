// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `content_interpretation_obu()` writer (AV2 v1.0.0 § 5.15,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-15`) — the inverse of
//! [`crate::headers::content_interpretation::parse_content_interpretation`].
//!
//! The OBU is `ci_scan_type_idc` `f(2)`, four optional-structure present flags
//! (`ci_color_description_present_flag`, `ci_chroma_sample_position_present_flag`,
//! `ci_aspect_ratio_info_present_flag`, `ci_timing_info_present_flag`), then
//! `ci_reserved_2bit` `f(2)`, followed by the four optional structures in that
//! order. Each present flag is the model's matching `Option` presence, so the writer
//! derives the flag bit from `Option::is_some()` rather than from a stored field.
//!
//! `ci_color_description()` is `ci_color_description_idc` `rg(2)`, the explicit
//! `(ci_color_primaries, ci_transfer_characteristics, ci_matrix_coefficients)`
//! triple iff that idc is `0`, then `ci_full_range_flag` `f(1)`.
//! `ci_chroma_sample_position()` codes `ci_chroma_sample_position_top` `uvlc()` and —
//! only when `ci_scan_type_idc != 1` — `ci_chroma_sample_position_bottom` `uvlc()`
//! (otherwise the bottom is inferred equal to the top). `ci_aspect_ratio_info()` is
//! `ci_aspect_ratio_idc` `f(8)` plus the explicit `ci_sar_width` / `ci_sar_height`
//! `uvlc()` pair iff that idc is `255`. `ci_timing_info()` is the § 5.4.12
//! `timing_info()` structure shared with [`crate::headers::sequence`], inverted by
//! the private `write_timing_info` helper below.
//!
//! `ci_color_description_idc` in `6..=127`, `ci_aspect_ratio_idc` in `17..=254`, a
//! reserved `ci_scan_type_idc`, and a non-zero `ci_reserved_2bit` are values the
//! § 5.15 parser preserves verbatim (§ 6.14 flags them, but the parser returns
//! `Ok`), so the writer reproduces each verbatim and never rejects it.
//! `OBU_CONTENT_INTERPRETATION` is an **extensible** OBU type (§ 5.2.1), so the OBU
//! tail is the dispatch's generic extensible tail (`obu_extension_flag = 0` then
//! `trailing_bits()`); this writer emits the body, not the tail.

use crate::headers::content_interpretation::{
    AspectRatioInfo, ChromaSamplePosition, ColorDescription, ContentInterpretation,
};
use crate::headers::sequence::TimingInfo;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `ci_scan_type_idc` and `ci_reserved_2bit` are `f(2)`.
const F2: u32 = 2;
/// `ci_color_description_idc` is `rg(2)`.
const COLOR_DESCRIPTION_RG: u32 = 2;
/// The explicit color triple fields (`ci_color_primaries`,
/// `ci_transfer_characteristics`, `ci_matrix_coefficients`) and `ci_aspect_ratio_idc`
/// are `f(8)`.
const F8: u32 = 8;
/// `num_units_in_display_tick` / `time_scale` are `f(32)`.
const TIMING_F32: u32 = 32;
/// `ci_aspect_ratio_idc == 255` selects the explicit `ci_sar_width` / `ci_sar_height`
/// pair; every other value uses the `Aspect_Ratio_Width` / `Aspect_Ratio_Height`
/// table.
const ASPECT_RATIO_EXTENDED_SAR: u8 = 255;

/// Writes a `content_interpretation_obu()` body (AV2 v1.0.0 § 5.15), the inverse of
/// [`crate::headers::content_interpretation::parse_content_interpretation`]. The OBU
/// header and the extensible OBU tail are the dispatch's job
/// ([`crate::write::write_complete_obu`]); this writes the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU
///   payload begins on a byte boundary).
/// - [`WriteError::NonCanonicalContentInterpretation`] for a constructed model the
///   § 5.15 parser could never produce, so it would not round-trip. The `what` label
///   names the offending field:
///   - `"color_primaries_idc"`: `ColorDescription::primaries` is `Some` while
///     `color_description_idc != 0`, or `None` while it is `0` (the parser reads the
///     triple iff the idc is `0`).
///   - `"chroma_bottom_progressive"`: `ci_scan_type_idc == 1` (progressive) but the
///     chroma `top` and `bottom` differ (the parser infers `bottom == top` and codes
///     no bottom, so a differing pair could never have been parsed).
///   - `"extended_sar_idc"`: `AspectRatioInfo::extended_sar` is `Some` while
///     `aspect_ratio_idc != 255`, or `None` while it is `255` (the parser reads the
///     explicit SAR iff the idc is `255`).
///   - `"timing_num_ticks_gate"`: `TimingInfo::num_ticks_per_picture_minus_1`
///     presence disagrees with `equal_picture_interval` (the parser reads the value
///     iff the flag is set).
///   - `"timing_display_tick_zero"` / `"timing_time_scale_zero"`: a zero
///     `num_units_in_display_tick` / `time_scale`, which the § 6.4.12 parser rejects
///     (so the value is parser-unproducible and its bytes would not reparse).
/// - [`WriteError::ValueTooWide`] / [`WriteError::ValueOutOfRange`] from the
///   primitive writers for a field value outside its descriptor's domain (e.g. a
///   `ci_color_description_idc` whose `rg(2)` quotient is `>= 32`, which the parser
///   could not have produced).
///
/// Values the parser tolerates verbatim — a reserved `ci_color_description_idc`
/// (`6..=127`), a reserved `ci_aspect_ratio_idc` (`17..=254`), a reserved
/// `ci_scan_type_idc`, and a non-zero `ci_reserved_2bit` — are reproduced exactly,
/// never rejected.
///
/// All checks run before any bit reaches `writer` (the body is drafted into a
/// scratch and appended only on full success), so a rejected model leaves `writer`
/// unchanged and the writer never panics.
pub fn write_content_interpretation(
    writer: &mut BitWriter,
    ci: &ContentInterpretation,
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let mut scratch = BitWriter::new();
    // § 5.15: ci_scan_type_idc f(2). The value (including a reserved 0/3) is preserved
    // verbatim — the parser returns Ok for any 2-bit value, the validator flags it.
    scratch.write_bits_u8(ci.scan_type_idc.get(), F2)?;
    // § 5.15: the four optional-structure present flags. Each flag IS the model's
    // matching Option presence, so it is derived here rather than stored separately.
    scratch.write_bit(u8::from(ci.color_description.is_some()))?;
    scratch.write_bit(u8::from(ci.chroma_sample_position.is_some()))?;
    scratch.write_bit(u8::from(ci.aspect_ratio.is_some()))?;
    scratch.write_bit(u8::from(ci.timing_info.is_some()))?;
    // § 5.15: ci_reserved_2bit f(2). § 6.14 requires 0, but the parser preserves any
    // 0..=3 value, so the writer reproduces it verbatim.
    scratch.write_bits_u8(ci.reserved_2bit, F2)?;

    if let Some(color) = &ci.color_description {
        write_color_description(&mut scratch, color)?;
    }
    if let Some(chroma) = &ci.chroma_sample_position {
        write_chroma_sample_position(
            &mut scratch,
            ci.scan_type_idc.is_progressive_frame(),
            chroma,
        )?;
    }
    if let Some(aspect) = &ci.aspect_ratio {
        write_aspect_ratio_info(&mut scratch, aspect)?;
    }
    if let Some(timing) = &ci.timing_info {
        write_timing_info(&mut scratch, timing)?;
    }

    writer.append(&scratch)
}

/// Writes `ci_color_description()` (AV2 v1.0.0 § 5.15): `ci_color_description_idc`
/// `rg(2)`, the explicit color triple iff that idc is `0`, then `ci_full_range_flag`
/// `f(1)`. A reserved idc (`6..=127`) is reproduced verbatim; only a
/// `primaries`-vs-idc disagreement is rejected.
fn write_color_description(scratch: &mut BitWriter, color: &ColorDescription) -> WriteResult<()> {
    // § 6.14: ci_color_description_idc has the rg(2) domain 0..=127; the writer
    // reproduces a reserved 6..=127 idc verbatim (write_rg rejects only a quotient
    // that could not terminate in 32 bits, i.e. a value the parser never produced).
    scratch.write_rg(color.color_description_idc, COLOR_DESCRIPTION_RG)?;
    if color.color_description_idc == 0 {
        // § 5.15: the explicit (color_primaries, transfer, matrix) triple.
        let primaries = color
            .primaries
            .ok_or_else(|| non_canonical("color_primaries_idc"))?;
        scratch.write_bits_u8(primaries.color_primaries, F8)?;
        scratch.write_bits_u8(primaries.transfer_characteristics, F8)?;
        scratch.write_bits_u8(primaries.matrix_coefficients, F8)?;
    } else if color.primaries.is_some() {
        // The parser reads the triple ONLY when idc == 0, so a non-zero idc that
        // stores primaries is parser-unproducible.
        return Err(non_canonical("color_primaries_idc"));
    }
    scratch.write_bit(u8::from(color.full_range_flag))
}

/// Writes `ci_chroma_sample_position()` (AV2 v1.0.0 § 5.15):
/// `ci_chroma_sample_position_top` `uvlc()` and — only when `ci_scan_type_idc != 1` —
/// `ci_chroma_sample_position_bottom` `uvlc()`. For a progressive frame the bottom is
/// inferred equal to the top and not coded, so a model whose progressive bottom
/// differs from its top could not have been parsed.
fn write_chroma_sample_position(
    scratch: &mut BitWriter,
    is_progressive_frame: bool,
    chroma: &ChromaSamplePosition,
) -> WriteResult<()> {
    scratch.write_uvlc(chroma.top)?;
    if is_progressive_frame {
        // § 5.15: the parser infers bottom == top for ci_scan_type_idc == 1 and codes
        // no bottom; a differing pair is parser-unproducible.
        if chroma.bottom != chroma.top {
            return Err(non_canonical("chroma_bottom_progressive"));
        }
    } else {
        scratch.write_uvlc(chroma.bottom)?;
    }
    Ok(())
}

/// Writes `ci_aspect_ratio_info()` (AV2 v1.0.0 § 5.15): `ci_aspect_ratio_idc` `f(8)`
/// plus the explicit `ci_sar_width` / `ci_sar_height` `uvlc()` pair iff that idc is
/// `255`. A reserved idc (`17..=254`) is reproduced verbatim; only an
/// `extended_sar`-vs-idc disagreement is rejected.
fn write_aspect_ratio_info(scratch: &mut BitWriter, aspect: &AspectRatioInfo) -> WriteResult<()> {
    // § 5.15: ci_aspect_ratio_idc f(8). A reserved 17..=254 idc (a § 6.14 violation
    // the validator flags) is preserved verbatim by the parser, so reproduce it.
    scratch.write_bits_u8(aspect.aspect_ratio_idc, F8)?;
    if aspect.aspect_ratio_idc == ASPECT_RATIO_EXTENDED_SAR {
        // § 5.15: the explicit ci_sar_width / ci_sar_height pair.
        let sar = aspect
            .extended_sar
            .ok_or_else(|| non_canonical("extended_sar_idc"))?;
        scratch.write_uvlc(sar.sar_width)?;
        scratch.write_uvlc(sar.sar_height)?;
    } else if aspect.extended_sar.is_some() {
        // The parser reads the explicit SAR ONLY when idc == 255, so a non-255 idc
        // that stores one is parser-unproducible.
        return Err(non_canonical("extended_sar_idc"));
    }
    Ok(())
}

/// Writes `timing_info()` (AV2 v1.0.0 § 5.4.12,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-12`), the inverse of
/// [`crate::headers::sequence::parse_timing_info`]: `num_units_in_display_tick`
/// `f(32)`, `time_scale` `f(32)`, `equal_picture_interval` `f(1)`, then —
/// only when that flag is set — `num_ticks_per_picture_minus_1` `uvlc()`.
///
/// The § 6.4.12 ranges the parser enforces map to writer rejects: a zero
/// `num_units_in_display_tick` or `time_scale` is rejected up front (the `f(32)` primitive
/// accepts any `u32`, so the non-zero bound is NOT enforced by the domain and is checked
/// explicitly), the `uvlc()` bound caps `num_ticks_per_picture_minus_1` to the parser's
/// `(1 << 32) - 2`, and the `num_ticks_per_picture_minus_1`-vs-`equal_picture_interval`
/// presence gate is enforced. All three reject before any bit reaches `scratch`, so a
/// parser-unproducible model never produces bytes that fail to reparse.
fn write_timing_info(scratch: &mut BitWriter, timing: &TimingInfo) -> WriteResult<()> {
    // § 6.4.12: `parse_timing_info` rejects a zero num_units_in_display_tick / time_scale
    // (Error::InvalidSequenceHeader), so a model carrying a zero is parser-unproducible and
    // its bytes would not reparse — reject it before any bit (f(32) accepts any u32).
    if timing.num_units_in_display_tick == 0 {
        return Err(non_canonical("timing_display_tick_zero"));
    }
    if timing.time_scale == 0 {
        return Err(non_canonical("timing_time_scale_zero"));
    }
    scratch.write_bits(timing.num_units_in_display_tick, TIMING_F32)?;
    scratch.write_bits(timing.time_scale, TIMING_F32)?;
    scratch.write_bit(u8::from(timing.equal_picture_interval))?;
    // § 5.4.12: num_ticks_per_picture_minus_1 is read iff equal_picture_interval, so
    // the parser ties Some(..) to the flag. Reject a model that stores one without the
    // other.
    if timing.equal_picture_interval {
        let ticks = timing
            .num_ticks_per_picture_minus_1
            .ok_or_else(|| non_canonical("timing_num_ticks_gate"))?;
        scratch.write_uvlc(ticks)?;
    } else if timing.num_ticks_per_picture_minus_1.is_some() {
        return Err(non_canonical("timing_num_ticks_gate"));
    }
    Ok(())
}

/// Helper constructing the content-interpretation-specific non-canonical reject with
/// a stable `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalContentInterpretation { what }
}

// The round-trip / reject tests live in a sibling file (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writer above.
#[cfg(test)]
include!("content_interpretation_tests.rs");
