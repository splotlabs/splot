// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `film_grain_obu()` writer (AV2 v1.0.0 § 5.14,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-14`, and the `film_grain_model()`
//! syntax § 5.18.10.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-2`) — the
//! inverse of [`crate::headers::film_grain::parse_film_grain`].
//!
//! The OBU is `fgm_update_flags` `f(8)` (a bitmap of updated model slots),
//! `fgm_chroma_idc` `uvlc()`, then — for each of the `MAX_FILM_GRAIN` (`8`) slots
//! whose bit is set in `fgm_update_flags`, in ascending slot order — one
//! `film_grain_model()` (§ 5.18.10.2). The derived `subX` / `subY` / `monochrome`
//! the parser computes from `fgm_chroma_idc` are not wire fields; the writer
//! re-derives them and rejects a model that disagrees. `OBU_FILM_GRAIN` is **not**
//! an extensible OBU type (§ 5.2.1), so the OBU tail is `trailing_bits()` only (the
//! dispatch's generic tail with `is_extensible == false`); this writer emits the
//! body, not the tail.
//!
//! ## The canonicalization (the model is lossy versus the wire)
//!
//! The model stores only the *cumulative* scaling-point coordinate `value` and the
//! `scaling` per point, and the *de-biased* AR coefficient `coeffs` — it does **not**
//! store the wire bit-widths the parser read (`point_value_increment_bits_minus_1`,
//! `point_scaling_bits_minus_5`, `bits_per_ar_coeff_*_minus_5`) nor the per-point
//! increments. So this writer **re-derives** the bit widths the way the leb128 writer
//! re-derives its minimal length: it picks, per array, the smallest in-range width
//! that fits every value, recomputes the wire form (increments from the cumulative
//! values, biased AR coefficients), and writes that. A reparse re-reads the same
//! cumulative values / scalings / de-biased coefficients regardless of the widths
//! chosen (the widths are not in the model's [`PartialEq`]), so the **semantic**
//! round-trip `read(write(x)) == x` holds — but **byte-exactness is not guaranteed**
//! (a producer that used wider-than-minimal widths is re-emitted minimally).
//!
//! - **Scaling points** (`read_scaling_points`): the increments are `point[0].value`
//!   then `point[i].value - point[i - 1].value`; the writer rejects non-monotonic
//!   points (the parser only produces non-decreasing cumulative values). `bitsIncr`
//!   is the smallest width in `1..=8` such that every increment `< (1 << bitsIncr)`;
//!   `bitsScal` is the smallest in `5..=8` such that every scaling `< (1 << bitsScal)`.
//!   If no in-range width fits (an increment `>= 256`, or a scaling `>= 256`) the
//!   model is rejected.
//! - **AR coeffs** (`read_ar_coeffs`): `bitsCoef` is the smallest in `5..=8` such that
//!   every coeff is in `[-(1 << (bitsCoef - 1)), (1 << (bitsCoef - 1)))` (so the biased
//!   `raw = coeff + (1 << (bitsCoef - 1))` fits `f(bitsCoef)`). If no width fits, the
//!   model is rejected.

use crate::headers::film_grain::{
    FilmGrainModel, FilmGrainObu, FilmGrainScalingPoint, MAX_FILM_GRAIN,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `CHROMA_FORMAT_420` (AV2 v1.0.0 § 6.4.2).
const CHROMA_FORMAT_420: u32 = 0;
/// `CHROMA_FORMAT_400` (monochrome).
const CHROMA_FORMAT_400: u32 = 1;
/// `CHROMA_FORMAT_444`.
const CHROMA_FORMAT_444: u32 = 2;
/// `CHROMA_FORMAT_422`.
const CHROMA_FORMAT_422: u32 = 3;

/// `fgm_update_flags` is `f(8)`.
const UPDATE_FLAGS_BITS: u32 = 8;
/// `num_y_points` / `num_cb_points` / `num_cr_points` are `f(4)`.
const NUM_POINTS_BITS: u32 = 4;
/// `point_value_increment_bits_minus_1` is `f(3)`.
const POINT_INCR_BITS_MINUS_1_BITS: u32 = 3;
/// `point_scaling_bits_minus_5` / `bits_per_ar_coeff_*_minus_5` and the various 2-bit
/// fields (`grain_scaling_minus_8`, `ar_coeff_lag`, `ar_coeff_shift_minus_6`,
/// `grain_scale_shift`) are `f(2)`.
const F2: u32 = 2;
/// `cb_mult` / `cb_luma_mult` / `cr_mult` / `cr_luma_mult` are `f(8)`.
const MULT_BITS: u32 = 8;
/// `cb_offset` / `cr_offset` are `f(9)`.
const OFFSET_BITS: u32 = 9;

/// `bitsIncr = point_value_increment_bits_minus_1 + 1` is in `1..=8`.
const INCR_WIDTH_MIN: u32 = 1;
const INCR_WIDTH_MAX: u32 = 8;
/// `bitsScal = point_scaling_bits_minus_5 + 5` is in `5..=8`.
const SCAL_WIDTH_MIN: u32 = 5;
const SCAL_WIDTH_MAX: u32 = 8;
/// `bitsCoef = bits_per_ar_coeff_*_minus_5 + 5` is in `5..=8`.
const COEF_WIDTH_MIN: u32 = 5;
const COEF_WIDTH_MAX: u32 = 8;

/// Writes a `film_grain_obu()` body (AV2 v1.0.0 § 5.14), the inverse of
/// [`crate::headers::film_grain::parse_film_grain`]. The OBU header and the
/// `trailing_bits()` tail are the dispatch's job ([`crate::write::write_complete_obu`]);
/// this writes the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU
///   payload begins on a byte boundary).
/// - [`WriteError::NonCanonicalFilmGrain`] for a constructed model the § 5.14 /
///   § 5.18.10.2 parser could never produce, so it would not round-trip. The `what`
///   label names the offending invariant:
///   - `"chroma_subsampling"`: `sub_x` / `sub_y` / `monochrome` disagree with
///     re-deriving them from `chroma_idc` (the parser derives them, they are not wire
///     fields).
///   - `"slot_update_flags"`: the `models` slots do not match the set bits of
///     `update_flags` (each `slot == bit index`, ascending, `len == popcount`).
///   - `"monochrome_chroma_scaling"`: `chroma_scaling_from_luma` is `true` while
///     `monochrome` (the parser forces it `0` when monochrome).
///   - `"chroma_points_gate"`: `num_cb_points` / `num_cr_points` is non-zero (or its
///     point `Vec` non-empty) while `monochrome || chroma_scaling_from_luma` (the
///     parser reads no chroma points then).
///   - `"num_y_points_len"` / `"num_cb_points_len"` / `"num_cr_points_len"`: a
///     `num_*_points` that disagrees with its point `Vec`'s length.
///   - `"ar_coeffs_y_len"` / `"ar_coeffs_cb_len"` / `"ar_coeffs_cr_len"`: an AR-coeff
///     `Vec` length that disagrees with its gate (`num_y_points > 0` ⇒ `numPosLuma`;
///     `chroma_scaling_from_luma || num_*_points > 0` ⇒ `numPosChroma`; else `0`).
///   - `"cb_mult_gate"` / `"cb_offset_gate"` / `"cr_mult_gate"` / `"cr_offset_gate"`:
///     a `cb_*` / `cr_*` `Option` presence that disagrees with `num_cb_points > 0` /
///     `num_cr_points > 0`.
///   - `"mc_identity_clip"`: `mc_identity` is `true` while `!clip_to_restricted_range`
///     (the parser forces it `0` then).
///   - `"non_monotonic_points"`: a scaling-point `value` is below its predecessor (the
///     parser only produces non-decreasing cumulative values).
///   - `"point_increment_width"` / `"point_scaling_width"`: a per-point increment /
///     scaling fits no in-range bit width (`>= 256`).
///   - `"ar_coeff_width"`: an AR coeff fits no in-range bit width.
/// - [`WriteError::ValueTooWide`] from the primitive writers for a field value outside
///   its descriptor's domain (`num_*_points >= 16`, `cb_offset >= 512`, …).
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch
/// and appended only on full success), so a rejected model leaves `writer` unchanged
/// and the writer never panics.
pub fn write_film_grain(writer: &mut BitWriter, fg: &FilmGrainObu) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    // § 5.14: subX/subY/monochrome are derived from fgm_chroma_idc, not wire fields, so
    // a stored value that disagrees could never have been parsed. Locally decidable.
    let (sub_x, sub_y) = chroma_subsampling(fg.chroma_idc);
    let monochrome = fg.chroma_idc == CHROMA_FORMAT_400;
    if fg.sub_x != sub_x || fg.sub_y != sub_y || fg.monochrome != monochrome {
        return Err(non_canonical("chroma_subsampling"));
    }

    // § 5.14: the parser emits one model per set bit of fgm_update_flags, in ascending
    // slot order (slot == bit index). A models Vec that does not match that exact set /
    // order / length could never have been parsed.
    let expected_slots: Vec<u8> = (0..MAX_FILM_GRAIN)
        .filter(|i| fg.update_flags & (1u8 << i) != 0)
        // i < MAX_FILM_GRAIN (8), so it fits in u8.
        .map(|i| i as u8)
        .collect();
    if fg.models.len() != expected_slots.len() {
        return Err(non_canonical("slot_update_flags"));
    }

    let mut scratch = BitWriter::new();
    scratch.write_bits_u8(fg.update_flags, UPDATE_FLAGS_BITS)?;
    scratch.write_uvlc(fg.chroma_idc)?;
    for (update, &expected_slot) in fg.models.iter().zip(&expected_slots) {
        if update.slot != expected_slot {
            return Err(non_canonical("slot_update_flags"));
        }
        write_film_grain_model(&mut scratch, &update.model, monochrome)?;
    }

    writer.append(&scratch)
}

/// Writes one `film_grain_model()` (AV2 v1.0.0 § 5.18.10.2), the inverse of the
/// parser's `parse_film_grain_model`. `monochrome` is the value the OBU derived from
/// `fgm_chroma_idc`; it gates `chroma_scaling_from_luma` and the chroma scaling points.
fn write_film_grain_model(
    scratch: &mut BitWriter,
    model: &FilmGrainModel,
    monochrome: bool,
) -> WriteResult<()> {
    // § 5.18.10.2: chroma_scaling_from_luma is read iff !monochrome (forced 0 when
    // monochrome). A monochrome model storing `true` could never have been parsed.
    if monochrome {
        if model.chroma_scaling_from_luma {
            return Err(non_canonical("monochrome_chroma_scaling"));
        }
    } else {
        scratch.write_flag(model.chroma_scaling_from_luma)?;
    }

    // § 5.18.10.2: luma scaling points (always present).
    if usize::from(model.num_y_points) != model.point_y.len() {
        return Err(non_canonical("num_y_points_len"));
    }
    scratch.write_bits_u8(model.num_y_points, NUM_POINTS_BITS)?;
    write_scaling_points(scratch, &model.point_y)?;

    // § 5.18.10.2: cb/cr scaling points are read iff !monochrome && !chroma_scaling_from_luma;
    // otherwise num_cb_points == num_cr_points == 0 and the point Vecs are empty.
    let chroma_points_coded = !monochrome && !model.chroma_scaling_from_luma;
    if chroma_points_coded {
        if usize::from(model.num_cb_points) != model.point_cb.len() {
            return Err(non_canonical("num_cb_points_len"));
        }
        scratch.write_bits_u8(model.num_cb_points, NUM_POINTS_BITS)?;
        write_scaling_points(scratch, &model.point_cb)?;

        if usize::from(model.num_cr_points) != model.point_cr.len() {
            return Err(non_canonical("num_cr_points_len"));
        }
        scratch.write_bits_u8(model.num_cr_points, NUM_POINTS_BITS)?;
        write_scaling_points(scratch, &model.point_cr)?;
    } else {
        // The parser sets both counts to 0 and leaves both point Vecs empty here.
        if model.num_cb_points != 0
            || model.num_cr_points != 0
            || !model.point_cb.is_empty()
            || !model.point_cr.is_empty()
        {
            return Err(non_canonical("chroma_points_gate"));
        }
    }

    scratch.write_bits_u8(model.grain_scaling_minus_8, F2)?;
    scratch.write_bits_u8(model.ar_coeff_lag, F2)?;
    // numPosLuma = 2 * ar_coeff_lag * (ar_coeff_lag + 1). ar_coeff_lag <= 3 (f(2)), so
    // numPosLuma <= 24; no overflow.
    let num_pos_luma = 2 * usize::from(model.ar_coeff_lag) * (usize::from(model.ar_coeff_lag) + 1);

    // § 5.18.10.2: ar_coeffs_y is read (length numPosLuma) iff num_y_points > 0; when it
    // is, numPosChroma = numPosLuma + 1, else numPosChroma = numPosLuma.
    let (num_pos_chroma, code_y) = if model.num_y_points > 0 {
        (num_pos_luma + 1, true)
    } else {
        (num_pos_luma, false)
    };
    let expected_y_len = if code_y { num_pos_luma } else { 0 };
    if model.ar_coeffs_y.len() != expected_y_len {
        return Err(non_canonical("ar_coeffs_y_len"));
    }
    if code_y {
        write_ar_coeffs(scratch, &model.ar_coeffs_y)?;
    }

    // § 5.18.10.2: ar_coeffs_cb is read (length numPosChroma) iff
    // chroma_scaling_from_luma || num_cb_points > 0.
    let code_cb = model.chroma_scaling_from_luma || model.num_cb_points > 0;
    let expected_cb_len = if code_cb { num_pos_chroma } else { 0 };
    if model.ar_coeffs_cb.len() != expected_cb_len {
        return Err(non_canonical("ar_coeffs_cb_len"));
    }
    if code_cb {
        write_ar_coeffs(scratch, &model.ar_coeffs_cb)?;
    }

    // § 5.18.10.2: ar_coeffs_cr is read (length numPosChroma) iff
    // chroma_scaling_from_luma || num_cr_points > 0.
    let code_cr = model.chroma_scaling_from_luma || model.num_cr_points > 0;
    let expected_cr_len = if code_cr { num_pos_chroma } else { 0 };
    if model.ar_coeffs_cr.len() != expected_cr_len {
        return Err(non_canonical("ar_coeffs_cr_len"));
    }
    if code_cr {
        write_ar_coeffs(scratch, &model.ar_coeffs_cr)?;
    }

    scratch.write_bits_u8(model.ar_coeff_shift_minus_6, F2)?;
    scratch.write_bits_u8(model.grain_scale_shift, F2)?;

    // § 5.18.10.2: cb_mult/cb_luma_mult/cb_offset are read iff num_cb_points > 0.
    write_mult_offset(
        scratch,
        model.num_cb_points > 0,
        model.cb_mult,
        model.cb_luma_mult,
        model.cb_offset,
        ("cb_mult_gate", "cb_offset_gate"),
    )?;
    // § 5.18.10.2: cr_mult/cr_luma_mult/cr_offset are read iff num_cr_points > 0.
    write_mult_offset(
        scratch,
        model.num_cr_points > 0,
        model.cr_mult,
        model.cr_luma_mult,
        model.cr_offset,
        ("cr_mult_gate", "cr_offset_gate"),
    )?;

    scratch.write_flag(model.overlap_flag)?;
    scratch.write_flag(model.clip_to_restricted_range)?;
    // § 5.18.10.2: fg_mc_identity is read iff clip_to_restricted_range (forced 0 else).
    if model.clip_to_restricted_range {
        scratch.write_flag(model.mc_identity)?;
    } else if model.mc_identity {
        return Err(non_canonical("mc_identity_clip"));
    }
    scratch.write_flag(model.film_grain_block_size)?;
    Ok(())
}

/// Writes one scaling function's points (the part after the `f(4)` count, which the
/// caller writes): when there are points, `point_value_increment_bits_minus_1` `f(3)`
/// and `point_scaling_bits_minus_5` `f(2)`, then per point the increment `f(bitsIncr)`
/// and `scaling` `f(bitsScal)`. The widths are re-derived minimally (see the module
/// docs); an empty `points` writes nothing.
fn write_scaling_points(
    scratch: &mut BitWriter,
    points: &[FilmGrainScalingPoint],
) -> WriteResult<()> {
    if points.is_empty() {
        return Ok(());
    }

    // Re-derive the per-point increments from the cumulative values, rejecting a
    // non-monotonic sequence the parser could not have produced (its cumulative values
    // are non-decreasing). increments[0] = value[0]; increments[i] = value[i] - value[i-1].
    let mut increments: Vec<u32> = Vec::with_capacity(points.len());
    let mut prev = 0u32;
    for (i, point) in points.iter().enumerate() {
        let increment = if i == 0 {
            point.value
        } else {
            // Reject below-predecessor (non-monotonic) before the subtraction underflows.
            point
                .value
                .checked_sub(prev)
                .ok_or_else(|| non_canonical("non_monotonic_points"))?
        };
        increments.push(increment);
        prev = point.value;
    }

    // bitsIncr is the smallest width in 1..=8 with every increment < (1 << bitsIncr);
    // bitsScal is the smallest in 5..=8 with every scaling < (1 << bitsScal).
    let max_increment = increments.iter().copied().max().unwrap_or(0);
    let bits_incr = minimal_width(max_increment, INCR_WIDTH_MIN, INCR_WIDTH_MAX)
        .ok_or_else(|| non_canonical("point_increment_width"))?;
    let max_scaling = points.iter().map(|p| p.scaling).max().unwrap_or(0);
    let bits_scal = minimal_width(max_scaling, SCAL_WIDTH_MIN, SCAL_WIDTH_MAX)
        .ok_or_else(|| non_canonical("point_scaling_width"))?;

    scratch.write_bits(bits_incr - 1, POINT_INCR_BITS_MINUS_1_BITS)?;
    scratch.write_bits(bits_scal - 5, F2)?;
    for (increment, point) in increments.iter().zip(points) {
        scratch.write_bits(*increment, bits_incr)?;
        scratch.write_bits(point.scaling, bits_scal)?;
    }
    Ok(())
}

/// Writes the AR-coefficient array of one scaling function: `bits_per_ar_coeff_minus_5`
/// `f(2)` then each biased coefficient `f(bitsCoef)`. `bitsCoef` is re-derived minimally
/// (the smallest in `5..=8` such that every coeff is in
/// `[-(1 << (bitsCoef - 1)), (1 << (bitsCoef - 1)))`). The caller only calls this when
/// the array is present, so an empty array (`numPos == 0`) still writes the width.
fn write_ar_coeffs(scratch: &mut BitWriter, coeffs: &[i32]) -> WriteResult<()> {
    // The smallest width whose signed range holds every coeff. An empty array needs no
    // range, so the minimal width COEF_WIDTH_MIN is used (the parser would read the same
    // f(2) = 0 and a zero-length loop, recovering an empty Vec).
    let bits_coef = minimal_coeff_width(coeffs).ok_or_else(|| non_canonical("ar_coeff_width"))?;
    scratch.write_bits(bits_coef - 5, F2)?;
    // midpoint = 1 << (bitsCoef - 1); raw = coeff + midpoint is in [0, 1 << bitsCoef).
    let midpoint = 1i64 << (bits_coef - 1);
    for &coeff in coeffs {
        // coeff is in [-midpoint, midpoint) by construction of bits_coef, so raw fits
        // f(bitsCoef); write_bits re-checks the field width defensively.
        let raw = (i64::from(coeff) + midpoint) as u32;
        scratch.write_bits(raw, bits_coef)?;
    }
    Ok(())
}

/// Writes a `(mult, luma_mult, offset)` triple gated on `present` (`num_*_points > 0`):
/// each value `f(8)`/`f(8)`/`f(9)`. Rejects an `Option` presence that disagrees with the
/// gate (`mult_gate` for `mult`/`luma_mult`, `offset_gate` for `offset`).
fn write_mult_offset(
    scratch: &mut BitWriter,
    present: bool,
    mult: Option<u8>,
    luma_mult: Option<u8>,
    offset: Option<u16>,
    labels: (&'static str, &'static str),
) -> WriteResult<()> {
    let (mult_gate, offset_gate) = labels;
    if present {
        let mult = mult.ok_or_else(|| non_canonical(mult_gate))?;
        let luma_mult = luma_mult.ok_or_else(|| non_canonical(mult_gate))?;
        let offset = offset.ok_or_else(|| non_canonical(offset_gate))?;
        scratch.write_bits_u8(mult, MULT_BITS)?;
        scratch.write_bits_u8(luma_mult, MULT_BITS)?;
        scratch.write_bits(u32::from(offset), OFFSET_BITS)?;
    } else {
        if mult.is_some() || luma_mult.is_some() {
            return Err(non_canonical(mult_gate));
        }
        if offset.is_some() {
            return Err(non_canonical(offset_gate));
        }
    }
    Ok(())
}

/// Returns `(subX, subY)` for an `fgm_chroma_idc` (AV2 v1.0.0 § 5.14), matching the
/// parser's `chroma_subsampling` table (out-of-range falls back to `(false, false)`).
const fn chroma_subsampling(chroma_idc: u32) -> (bool, bool) {
    // The CHROMA_FORMAT_444 arm is kept distinct from the out-of-range `_` fallback to
    // mirror the parser's documented § 5.14 table, even though both yield (false, false).
    #[allow(clippy::match_same_arms)]
    match chroma_idc {
        CHROMA_FORMAT_420 | CHROMA_FORMAT_400 => (true, true),
        CHROMA_FORMAT_422 => (true, false),
        CHROMA_FORMAT_444 => (false, false),
        _ => (false, false),
    }
}

/// Returns the smallest width `w` in `min_width..=max_width` such that `value` fits an
/// unsigned `f(w)` field (`value < (1 << w)`), or `None` if even `max_width` is too
/// narrow. `max_width <= 8`, so `1 << max_width` never overflows.
fn minimal_width(value: u32, min_width: u32, max_width: u32) -> Option<u32> {
    (min_width..=max_width).find(|&w| u64::from(value) < (1u64 << w))
}

/// Returns the smallest `bitsCoef` in `5..=8` whose signed range
/// `[-(1 << (bitsCoef - 1)), (1 << (bitsCoef - 1)))` holds every coeff, or `None` if even
/// width `8` is too narrow. An empty slice fits the minimum width.
fn minimal_coeff_width(coeffs: &[i32]) -> Option<u32> {
    (COEF_WIDTH_MIN..=COEF_WIDTH_MAX).find(|&w| {
        // half = 1 << (w - 1); the range is [-half, half). w <= 8, so half <= 128.
        let half = 1i64 << (w - 1);
        coeffs
            .iter()
            .all(|&c| i64::from(c) >= -half && i64::from(c) < half)
    })
}

/// Helper constructing the film-grain non-canonical reject with a stable `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalFilmGrain { what }
}

// The round-trip / reject tests live in a sibling file (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writer above.
#[cfg(test)]
include!("film_grain_tests.rs");
