// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 film grain OBU syntax model (AV2 v1.0.0 § 5.14, § 5.18.10.2).
//!
//! [`parse_film_grain`] reads `film_grain_obu()` (§ 5.14): an 8-bit
//! `fgm_update_flags` bitmap selecting which of the `MAX_FILM_GRAIN` model slots are
//! updated, an `fgm_chroma_idc` selecting the chroma format, and one
//! [`FilmGrainModel`] (`film_grain_model()`, § 5.18.10.2) per set slot bit.
//!
//! This module models syntax only: it preserves every parsed field so the inspector
//! and future frame `apply_grain` / `fgm_id` reference checks can use it; it does not
//! synthesize grain. Reserved/out-of-range values (for example `fgm_chroma_idc > 3`,
//! a § 6.13 conformance violation) are preserved rather than rejected so the validator
//! can report them with offsets. Field order follows the AV2 spec and the AVM oracle
//! `read_fgm_obu` / `read_film_grain_model` (`av2/decoder/obu_fgm.c`); no AV1 tables
//! or code are copied.

use crate::bitio::BitReader;
use crate::error::Result;

/// `MAX_FILM_GRAIN`: number of film grain model slots (AV2 v1.0.0 § 3).
pub const MAX_FILM_GRAIN: usize = 8;

/// `CHROMA_FORMAT_420` (AV2 v1.0.0 § 6.4.2, chroma format table).
const CHROMA_FORMAT_420: u32 = 0;
/// `CHROMA_FORMAT_400` (monochrome).
const CHROMA_FORMAT_400: u32 = 1;
/// `CHROMA_FORMAT_444`.
const CHROMA_FORMAT_444: u32 = 2;
/// `CHROMA_FORMAT_422`.
const CHROMA_FORMAT_422: u32 = 3;

/// Parsed `film_grain_obu()` syntax (AV2 v1.0.0 § 5.14).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilmGrainObu {
    /// `fgm_update_flags` (`f(8)`): bitmap of updated model slots. `0` is a § 6.13
    /// conformance violation but is preserved for the validator.
    pub update_flags: u8,
    /// `fgm_chroma_idc` (`uvlc()`): chroma format selector. Values greater than `3`
    /// are a § 6.13 conformance violation but are preserved for the validator.
    pub chroma_idc: u32,
    /// `subX` derived from `fgm_chroma_idc` (§ 5.14).
    pub sub_x: bool,
    /// `subY` derived from `fgm_chroma_idc` (§ 5.14).
    pub sub_y: bool,
    /// `monochrome = fgm_chroma_idc == CHROMA_FORMAT_400`.
    pub monochrome: bool,
    /// One entry per set bit of `update_flags`, in ascending slot order.
    pub models: Vec<FilmGrainSlotUpdate>,
}

impl FilmGrainObu {
    /// Returns the updated slot indices as a bitmap (equal to `update_flags`).
    #[must_use]
    pub const fn updated_slot_bitmap(&self) -> u8 {
        self.update_flags
    }
}

/// One updated film grain model slot within a `film_grain_obu()` (AV2 v1.0.0 § 5.14).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilmGrainSlotUpdate {
    /// Slot index `i` in `0..MAX_FILM_GRAIN` (bit `i` of `fgm_update_flags`).
    pub slot: u8,
    /// The parsed `film_grain_model()` for this slot.
    pub model: FilmGrainModel,
}

/// One scaling-function point: `(value, scaling)` (AV2 v1.0.0 § 5.18.10.2).
///
/// `value` is the cumulative point coordinate (`point_*_value`, the per-point
/// increments summed); `scaling` is `point_*_scaling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilmGrainScalingPoint {
    /// Cumulative point coordinate.
    pub value: u32,
    /// Point scaling value.
    pub scaling: u32,
}

/// Parsed `film_grain_model()` syntax (AV2 v1.0.0 § 5.18.10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilmGrainModel {
    /// `chroma_scaling_from_luma` (`f(1)`; forced `0` when monochrome).
    pub chroma_scaling_from_luma: bool,
    /// `num_y_points` (`f(4)`).
    pub num_y_points: u8,
    /// Luma scaling-function points (`point_y_value` / `point_y_scaling`).
    pub point_y: Vec<FilmGrainScalingPoint>,
    /// `num_cb_points` (`f(4)`; `0` when monochrome or `chroma_scaling_from_luma`).
    pub num_cb_points: u8,
    /// Cb scaling-function points.
    pub point_cb: Vec<FilmGrainScalingPoint>,
    /// `num_cr_points` (`f(4)`; `0` when monochrome or `chroma_scaling_from_luma`).
    pub num_cr_points: u8,
    /// Cr scaling-function points.
    pub point_cr: Vec<FilmGrainScalingPoint>,
    /// `grain_scaling_minus_8` (`f(2)`).
    pub grain_scaling_minus_8: u8,
    /// `ar_coeff_lag` (`f(2)`).
    pub ar_coeff_lag: u8,
    /// `ar_coeffs_y[i]` (each de-biased by `1 << (bitsCoef - 1)`).
    pub ar_coeffs_y: Vec<i32>,
    /// `ar_coeffs_cb[i]`.
    pub ar_coeffs_cb: Vec<i32>,
    /// `ar_coeffs_cr[i]`.
    pub ar_coeffs_cr: Vec<i32>,
    /// `ar_coeff_shift_minus_6` (`f(2)`).
    pub ar_coeff_shift_minus_6: u8,
    /// `grain_scale_shift` (`f(2)`).
    pub grain_scale_shift: u8,
    /// `cb_mult` (`f(8)`; present when `num_cb_points > 0`).
    pub cb_mult: Option<u8>,
    /// `cb_luma_mult` (`f(8)`; present when `num_cb_points > 0`).
    pub cb_luma_mult: Option<u8>,
    /// `cb_offset` (`f(9)`; present when `num_cb_points > 0`).
    pub cb_offset: Option<u16>,
    /// `cr_mult` (`f(8)`; present when `num_cr_points > 0`).
    pub cr_mult: Option<u8>,
    /// `cr_luma_mult` (`f(8)`; present when `num_cr_points > 0`).
    pub cr_luma_mult: Option<u8>,
    /// `cr_offset` (`f(9)`; present when `num_cr_points > 0`).
    pub cr_offset: Option<u16>,
    /// `overlap_flag` (`f(1)`).
    pub overlap_flag: bool,
    /// `clip_to_restricted_range` (`f(1)`).
    pub clip_to_restricted_range: bool,
    /// `fg_mc_identity` (`f(1)`; forced `0` unless `clip_to_restricted_range`).
    pub mc_identity: bool,
    /// `film_grain_block_size` (`f(1)`).
    pub film_grain_block_size: bool,
}

/// Returns `(subX, subY)` for an `fgm_chroma_idc` (AV2 v1.0.0 § 5.14).
///
/// Out-of-range values (`> 3`, a § 6.13 conformance violation handled by the
/// validator) fall back to `(false, false)`; `subX` / `subY` are not used by
/// `film_grain_model()` itself, so the fallback only affects the preserved summary.
const fn chroma_subsampling(chroma_idc: u32) -> (bool, bool) {
    // The CHROMA_FORMAT_444 arm is kept distinct from the out-of-range `_` fallback to
    // document the § 5.14 don't-care semantics, even though both yield (false, false).
    #[allow(clippy::match_same_arms)]
    match chroma_idc {
        // AV2 § 5.14 explicitly assigns subX = subY = 1 to both CHROMA_FORMAT_420 and
        // CHROMA_FORMAT_400 (monochrome). The fields are a don't-care for the model
        // syntax but drive the validator's § 6.17.10.2 4:2:0 chroma-pairing check.
        CHROMA_FORMAT_420 | CHROMA_FORMAT_400 => (true, true),
        CHROMA_FORMAT_422 => (true, false),
        CHROMA_FORMAT_444 => (false, false),
        // Out-of-range (`> 3`, a § 6.13 conformance violation the validator reports).
        _ => (false, false),
    }
}

/// Parses a `film_grain_obu()` (AV2 v1.0.0 § 5.14).
///
/// The defining layer ids (`obu_tlayer_id` / `obu_mlayer_id`, used for per-slot HLS
/// availability, § 6.13) come from the OBU header rather than the payload, so the
/// validator reads them from the OBU envelope instead of threading them through here.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof),
/// [`Error::InvalidUvlc`](crate::error::Error::InvalidUvlc), or
/// [`Error::BitWidthTooLarge`](crate::error::Error::BitWidthTooLarge) from
/// [`BitReader`] when the input is truncated or a descriptor is malformed.
pub fn parse_film_grain(reader: &mut BitReader<'_>) -> Result<FilmGrainObu> {
    let update_flags = reader.read_bits_u8(8)?;
    let chroma_idc = reader.read_uvlc()?;
    let monochrome = chroma_idc == CHROMA_FORMAT_400;
    let (sub_x, sub_y) = chroma_subsampling(chroma_idc);

    let mut models = Vec::new();
    for slot in 0..MAX_FILM_GRAIN {
        if update_flags & (1 << slot) == 0 {
            continue;
        }
        let model = parse_film_grain_model(reader, monochrome)?;
        // slot < MAX_FILM_GRAIN (8), so it fits in u8.
        models.push(FilmGrainSlotUpdate {
            slot: slot as u8,
            model,
        });
    }

    Ok(FilmGrainObu {
        update_flags,
        chroma_idc,
        sub_x,
        sub_y,
        monochrome,
        models,
    })
}

/// Reads one scaling function's points: a count `f(4)`, then per-point cumulative
/// `point_*_value` and `point_*_scaling` (AV2 v1.0.0 § 5.18.10.2).
fn read_scaling_points(reader: &mut BitReader<'_>) -> Result<(u8, Vec<FilmGrainScalingPoint>)> {
    let num_points = reader.read_bits_u8(4)?;
    if num_points == 0 {
        return Ok((0, Vec::new()));
    }

    let bits_incr = reader.read_bits(3)? + 1; // point_value_increment_bits_minus_1 + 1
    let bits_scal = reader.read_bits(2)? + 5; // point_scaling_bits_minus_5 + 5
    let mut points = Vec::with_capacity(num_points as usize);
    let mut value = 0u32;
    for i in 0..num_points {
        let increment = reader.read_bits(bits_incr)?;
        // AV2 § 5.18.10.2: point_*_value[i] += point_*_value[i - 1] for i > 0.
        value = if i == 0 { increment } else { value + increment };
        let scaling = reader.read_bits(bits_scal)?;
        points.push(FilmGrainScalingPoint { value, scaling });
    }
    Ok((num_points, points))
}

/// Reads `count` de-biased AR coefficients of width `bits_per_coeff_minus_5 + 5`
/// (AV2 v1.0.0 § 5.18.10.2): `ar_coeffs[i] = f(bitsCoef) - (1 << (bitsCoef - 1))`.
fn read_ar_coeffs(reader: &mut BitReader<'_>, count: usize) -> Result<Vec<i32>> {
    let bits_coef = reader.read_bits(2)? + 5;
    // bits_coef is in 5..=8, so the midpoint fits comfortably in i32.
    let midpoint = 1i32 << (bits_coef - 1);
    let mut coeffs = Vec::with_capacity(count);
    for _ in 0..count {
        // The read value is at most 2^8 - 1, so `as i32` is exact.
        let raw = reader.read_bits(bits_coef)? as i32;
        coeffs.push(raw - midpoint);
    }
    Ok(coeffs)
}

/// Parses a `film_grain_model()` (AV2 v1.0.0 § 5.18.10.2).
///
/// `subX` / `subY` are parameters of the spec syntax but are not referenced in its
/// body, so they are not threaded here.
fn parse_film_grain_model(reader: &mut BitReader<'_>, monochrome: bool) -> Result<FilmGrainModel> {
    let chroma_scaling_from_luma = if monochrome {
        false
    } else {
        reader.read_flag()?
    };

    let (num_y_points, point_y) = read_scaling_points(reader)?;

    let (num_cb_points, point_cb, num_cr_points, point_cr) =
        if monochrome || chroma_scaling_from_luma {
            (0, Vec::new(), 0, Vec::new())
        } else {
            let (num_cb, cb) = read_scaling_points(reader)?;
            let (num_cr, cr) = read_scaling_points(reader)?;
            (num_cb, cb, num_cr, cr)
        };

    let grain_scaling_minus_8 = reader.read_bits_u8(2)?;
    let ar_coeff_lag = reader.read_bits_u8(2)?;
    let num_pos_luma = 2 * (ar_coeff_lag as usize) * (ar_coeff_lag as usize + 1);

    let mut num_pos_chroma = num_pos_luma;
    let ar_coeffs_y = if num_y_points > 0 {
        num_pos_chroma = num_pos_luma + 1;
        read_ar_coeffs(reader, num_pos_luma)?
    } else {
        Vec::new()
    };

    let ar_coeffs_cb = if chroma_scaling_from_luma || num_cb_points > 0 {
        read_ar_coeffs(reader, num_pos_chroma)?
    } else {
        Vec::new()
    };
    let ar_coeffs_cr = if chroma_scaling_from_luma || num_cr_points > 0 {
        read_ar_coeffs(reader, num_pos_chroma)?
    } else {
        Vec::new()
    };

    let ar_coeff_shift_minus_6 = reader.read_bits_u8(2)?;
    let grain_scale_shift = reader.read_bits_u8(2)?;

    let (cb_mult, cb_luma_mult, cb_offset) = if num_cb_points > 0 {
        let mult = reader.read_bits_u8(8)?;
        let luma_mult = reader.read_bits_u8(8)?;
        // cb_offset is f(9), at most 511, so `as u16` is exact.
        let offset = reader.read_bits(9)? as u16;
        (Some(mult), Some(luma_mult), Some(offset))
    } else {
        (None, None, None)
    };
    let (cr_mult, cr_luma_mult, cr_offset) = if num_cr_points > 0 {
        let mult = reader.read_bits_u8(8)?;
        let luma_mult = reader.read_bits_u8(8)?;
        let offset = reader.read_bits(9)? as u16;
        (Some(mult), Some(luma_mult), Some(offset))
    } else {
        (None, None, None)
    };

    let overlap_flag = reader.read_flag()?;
    let clip_to_restricted_range = reader.read_flag()?;
    let mc_identity = if clip_to_restricted_range {
        reader.read_flag()?
    } else {
        false
    };
    let film_grain_block_size = reader.read_flag()?;

    Ok(FilmGrainModel {
        chroma_scaling_from_luma,
        num_y_points,
        point_y,
        num_cb_points,
        point_cb,
        num_cr_points,
        point_cr,
        grain_scaling_minus_8,
        ar_coeff_lag,
        ar_coeffs_y,
        ar_coeffs_cb,
        ar_coeffs_cr,
        ar_coeff_shift_minus_6,
        grain_scale_shift,
        cb_mult,
        cb_luma_mult,
        cb_offset,
        cr_mult,
        cr_luma_mult,
        cr_offset,
        overlap_flag,
        clip_to_restricted_range,
        mc_identity,
        film_grain_block_size,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn parse(bytes: &[u8]) -> Result<FilmGrainObu> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_film_grain(&mut reader)
    }

    /// Appends the smallest valid `film_grain_model()` for a non-monochrome,
    /// no-points configuration (no scaling points, `ar_coeff_lag == 0`).
    fn write_minimal_model(bits: &mut Bits) {
        bits.bit(0); // chroma_scaling_from_luma = 0
        bits.f(0, 4); // num_y_points = 0
        bits.f(0, 4); // num_cb_points = 0
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0 -> numPosLuma = 0; no AR coeffs read
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range = 0 -> mc_identity inferred 0
        bits.bit(0); // film_grain_block_size
    }

    #[test]
    fn film_grain_single_slot_minimal_model_parses() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
        bits.uvlc(CHROMA_FORMAT_420);
        write_minimal_model(&mut bits);
        let data = bits.into_bytes();
        let fg = parse(&data).unwrap();
        assert_eq!(fg.update_flags, 1);
        assert_eq!(fg.chroma_idc, CHROMA_FORMAT_420);
        assert!(!fg.monochrome);
        assert_eq!(fg.models.len(), 1);
        assert_eq!(fg.models[0].slot, 0);
        let model = &fg.models[0].model;
        assert_eq!(model.num_y_points, 0);
        assert!(!model.clip_to_restricted_range);
        assert!(!model.mc_identity);
    }

    #[test]
    fn film_grain_with_luma_points_and_ar_coeffs_parses() {
        let mut bits = Bits::default();
        bits.f(0b0000_0010, 8); // fgm_update_flags: slot 1
        bits.uvlc(CHROMA_FORMAT_400); // monochrome
        // model: monochrome -> chroma_scaling_from_luma forced 0 (no bit read).
        bits.f(2, 4); // num_y_points = 2
        bits.f(0, 3); // point_value_increment_bits_minus_1 = 0 -> bitsIncr = 1
        bits.f(0, 2); // point_scaling_bits_minus_5 = 0 -> bitsScal = 5
        bits.f(1, 1); // point_y_value[0] = 1
        bits.f(3, 5); // point_y_scaling[0] = 3
        bits.f(1, 1); // point_y_value[1] increment = 1 -> cumulative 2
        bits.f(4, 5); // point_y_scaling[1] = 4
        // monochrome -> num_cb_points = num_cr_points = 0 (no reads).
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(1, 2); // ar_coeff_lag = 1 -> numPosLuma = 2*1*2 = 4
        bits.f(0, 2); // bits_per_ar_coeff_y_minus_5 = 0 -> bitsCoef = 5, midpoint 16
        bits.f(16, 5); // ar_coeffs_y[0] = 16 - 16 = 0
        bits.f(17, 5); // ar_coeffs_y[1] = 17 - 16 = 1
        bits.f(15, 5); // ar_coeffs_y[2] = 15 - 16 = -1
        bits.f(16, 5); // ar_coeffs_y[3] = 0
        // chroma_scaling_from_luma = 0 and num_cb/cr = 0 -> no chroma AR coeffs.
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        // num_cb/cr = 0 -> no mult/offset.
        bits.bit(1); // overlap_flag
        bits.bit(1); // clip_to_restricted_range = 1 -> read mc_identity
        bits.bit(1); // mc_identity
        bits.bit(0); // film_grain_block_size
        let data = bits.into_bytes();
        let fg = parse(&data).unwrap();
        assert!(fg.monochrome);
        assert_eq!(fg.models[0].slot, 1);
        let model = &fg.models[0].model;
        assert_eq!(model.num_y_points, 2);
        assert_eq!(model.point_y[0].value, 1);
        assert_eq!(model.point_y[1].value, 2); // cumulative
        assert_eq!(model.point_y[1].scaling, 4);
        assert_eq!(model.ar_coeffs_y, vec![0, 1, -1, 0]);
        assert!(model.overlap_flag);
        assert!(model.clip_to_restricted_range);
        assert!(model.mc_identity);
    }

    #[test]
    fn film_grain_zero_update_flags_parses_with_no_models() {
        let mut bits = Bits::default();
        bits.f(0, 8); // fgm_update_flags = 0 (a §6.13 violation, but parses)
        bits.uvlc(CHROMA_FORMAT_420);
        let data = bits.into_bytes();
        let fg = parse(&data).unwrap();
        assert_eq!(fg.update_flags, 0);
        assert!(fg.models.is_empty());
    }

    #[test]
    fn film_grain_out_of_range_chroma_idc_is_preserved() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(4); // fgm_chroma_idc = 4 (>3, a §6.13 violation)
        write_minimal_model(&mut bits); // not monochrome (4 != 1)
        let data = bits.into_bytes();
        let fg = parse(&data).unwrap();
        assert_eq!(fg.chroma_idc, 4);
        assert!(!fg.monochrome);
        assert_eq!(fg.models.len(), 1);
    }

    #[test]
    fn truncated_film_grain_is_error_not_panic() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(CHROMA_FORMAT_444);
        bits.bit(0); // chroma_scaling_from_luma
        bits.f(5, 4); // num_y_points = 5, but no point payload follows -> EOF
        let data = bits.into_bytes();
        assert!(parse(&data).is_err());
    }

    #[test]
    fn chroma_subsampling_table_matches_spec() {
        assert_eq!(chroma_subsampling(CHROMA_FORMAT_420), (true, true));
        assert_eq!(chroma_subsampling(CHROMA_FORMAT_400), (true, true));
        assert_eq!(chroma_subsampling(CHROMA_FORMAT_444), (false, false));
        assert_eq!(chroma_subsampling(CHROMA_FORMAT_422), (true, false));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The film-grain parser must never panic on arbitrary input.
        #[test]
        fn film_grain_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_film_grain(&mut reader);
        }
    }
}
