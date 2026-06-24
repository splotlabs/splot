// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.4 pixel-classified Wiener skip-filter classification.
//!
//! This module implements the scheduler-free classification portion of the AV2
//! § 7.20.4 pixel-classified Wiener filter process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-4)).
//! The caller resolves § 7.20.2 source-sample selection, frame/restoration-unit
//! traversal, and the `LrTxSkip` grid storage. This primitive derives the
//! skip-filter class value from caller-provided source samples and `LrTxSkip`
//! values; it does not store `FilterClass`, derive `SubclassLookup`, invoke
//! § 7.20.3 filtering, or wire runtime decode output.
//!
//! Feature tracking: `RECON-PC-WIENER-CLASSIFICATION`.

use splot_tables::tables::loop_restoration::PC_WIENER_LUT_TO_CLASS;

use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// AV2 § 3 `PC_WIENER_NUM_FEATURES`.
pub const PC_WIENER_NUM_FEATURES: usize = 4;

/// Number of feature points in one dimension of AV2 § 7.20.4 `get_box_features`.
pub const PC_WIENER_FEATURE_WINDOW_SIDE: usize = 6;

/// Number of entries in AV2 § 9.8 `Pc_Wiener_Lut_To_Class`.
pub const PC_WIENER_LUT_INPUTS: usize = 4096;

/// AV2 § 3 `PC_WIENER_PREC_FEATURE`.
const PC_WIENER_PREC_FEATURE: u32 = 14;
/// AV2 § 3 `QUANT_TABLE_BITS`.
const QUANT_TABLE_BITS: u32 = 3;
/// AV2 § 7.20.4 `PC_WIENER_LEAD`.
const PC_WIENER_LEAD: isize = 1;
/// AV2 § 7.20.4 `PC_WIENER_LAG`.
const PC_WIENER_LAG: isize = 4;
/// AV2 § 7.20.4 `Pc_Wiener_Normalizer`.
const PC_WIENER_NORMALIZER: [i64; PC_WIENER_NUM_FEATURES + 1] = [0, 3739, 3273, 3074, 7];
/// AV2 § 7.20.4 `Mode_Weights`.
const MODE_WEIGHTS: [[i64; 3]; PC_WIENER_NUM_FEATURES] = [
    [-527, 15325, 321],
    [26436, -17705, 17905],
    [366, -147, -194],
    [202, -267, -179],
];
/// AV2 § 7.20.4 `Mode_Offsets`.
const MODE_OFFSETS: [i64; PC_WIENER_NUM_FEATURES] = [-547, -21565, -573, -680];

/// Caller-resolved parameters for AV2 § 7.20.4 skip-filter classification.
///
/// `x` and `y` identify the luma sample being classified in current-plane sample
/// coordinates. The `block_*`, stripe, and tile bounds are caller-derived from
/// the active restoration block and tile facts; this primitive uses them only for
/// the § 7.20.4 `get_features` x clipping and `get_tx_skip` lookup clipping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerClassifyParams {
    /// Luma sample x coordinate of the classified sample.
    pub x: isize,
    /// Luma sample y coordinate of the classified sample.
    pub y: isize,
    /// Active decoded bit depth used for source validation and feature scaling.
    pub bit_depth: BitDepth,
    /// Frame `base_q_idx` used by § 7.20.4 `get_qval_given_tskip`.
    pub base_q_idx: u32,
    /// § 7.20.4 `BlockStartX`, in luma samples.
    pub block_start_x: usize,
    /// § 7.20.4 `BlockEndX`, in luma samples.
    pub block_end_x: usize,
    /// Caller-resolved `LumaStripeStartY`, in luma samples.
    pub luma_stripe_start_y: usize,
    /// Caller-resolved `LumaStripeEndY`, in luma samples.
    pub luma_stripe_end_y: usize,
    /// Tile start y coordinate, in luma samples.
    pub tile_start_y: usize,
    /// Tile end y coordinate, in luma samples.
    pub tile_end_y: usize,
}

/// A clipped AV2 § 7.20.4 `LrTxSkip[y >> 2][x >> 2]` lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerTxSkipLookup {
    /// Luma sample x coordinate after § 7.20.4 clipping.
    pub x: usize,
    /// Luma sample y coordinate after § 7.20.4 clipping.
    pub y: usize,
    /// Zero-based `LrTxSkip` row.
    pub row: usize,
    /// Zero-based `LrTxSkip` column.
    pub col: usize,
}

/// AV2 § 7.20.4 skip-filter classification result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerClassification {
    /// Raw unnormalized accumulated feature values from the 6x6 feature window.
    pub raw_features: [i64; PC_WIENER_NUM_FEATURES],
    /// Normalized feature values returned by `get_box_features`.
    pub features: [i64; PC_WIENER_NUM_FEATURES],
    /// Raw accumulated `LrTxSkip` values over the 6x6 feature window.
    pub raw_tx_skip_sum: i64,
    /// Normalized `tskip` value returned by `get_box_features`.
    pub tx_skip: i64,
    /// AV2 § 7.20.4 `lutInput` in `0..4096`.
    pub lut_input: u16,
    /// AV2 § 7.20.4 `cls = Pc_Wiener_Lut_To_Class[lutInput]`.
    pub class: u8,
}

/// Derives an AV2 § 7.20.4 pixel-classified Wiener skip-filter class.
///
/// `source_sample(x, y)` is called with current-plane luma source coordinates
/// after the § 7.20.4 `x = Min(BlockEndX + 2, x)` adjustment performed inside
/// `get_features`. The callback must implement the caller's selected
/// § 7.20.2 source-sample behavior, including frame selection and clipping.
/// `tx_skip(lookup)` is called with a fully clipped `LrTxSkip` grid lookup.
///
/// # Errors
///
/// Returns typed [`ReconError`] values for unsupported sample storage, invalid
/// caller-resolved bounds, source samples outside the active bit-depth range,
/// `LrTxSkip` values outside `0..=1`, and arithmetic overflow in coordinate or
/// LUT derivation. The function mutates no caller-owned output or grid state.
pub fn pc_wiener_classify<T, FS, FT>(
    params: &PcWienerClassifyParams,
    mut source_sample: FS,
    mut tx_skip: FT,
) -> Result<PcWienerClassification>
where
    T: ReconSample,
    FS: FnMut(isize, isize) -> T,
    FT: FnMut(PcWienerTxSkipLookup) -> i32,
{
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;

    let mut raw_features = [0i64; PC_WIENER_NUM_FEATURES];
    let mut raw_tx_skip_sum = 0i64;
    for dy in -PC_WIENER_LEAD..=PC_WIENER_LAG {
        for dx in -PC_WIENER_LEAD..=PC_WIENER_LAG {
            let feature_x = coordinate_add(params.x, dx, "PC-Wiener feature x")?;
            let feature_y = coordinate_add(params.y, dy, "PC-Wiener feature y")?;
            let feature = pc_wiener_features(params, feature_x, feature_y, &mut source_sample)?;
            for (dst, src) in raw_features.iter_mut().zip(feature.values) {
                *dst = dst.checked_add(src).ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener feature accumulation",
                })?;
            }
            let lookup = tx_skip_lookup(params, feature.x, feature_y)?;
            let skip = tx_skip(lookup);
            if !(0..=1).contains(&skip) {
                return Err(ReconError::PcWienerInvalidTxSkip {
                    x: lookup.x,
                    y: lookup.y,
                    row: lookup.row,
                    col: lookup.col,
                    value: skip,
                });
            }
            raw_tx_skip_sum = raw_tx_skip_sum.checked_add(i64::from(skip)).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "PC-Wiener tx-skip accumulation",
                },
            )?;
        }
    }

    let scale_shift = u32::from(params.bit_depth.bits() - 8);
    let mut features = [0i64; PC_WIENER_NUM_FEATURES];
    for (i, feature) in features.iter_mut().enumerate() {
        *feature = round2(
            raw_features[i].checked_mul(PC_WIENER_NORMALIZER[i]).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "PC-Wiener feature normalization",
                },
            )?,
            scale_shift,
        );
    }
    let normalized_tx_skip = raw_tx_skip_sum
        .checked_mul(PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES])
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip normalization",
        })?;
    let lut_input = pc_wiener_lut_input(
        features,
        normalized_tx_skip,
        params.base_q_idx,
        params.bit_depth,
    )?;
    let class = u8::try_from(PC_WIENER_LUT_TO_CLASS[usize::from(lut_input)]).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "PC-Wiener LUT class",
        }
    })?;

    Ok(PcWienerClassification {
        raw_features,
        features,
        raw_tx_skip_sum,
        tx_skip: normalized_tx_skip,
        lut_input,
        class,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PcWienerFeatureValues {
    x: isize,
    values: [i64; PC_WIENER_NUM_FEATURES],
}

fn pc_wiener_features<T, F>(
    params: &PcWienerClassifyParams,
    x: isize,
    y: isize,
    source_sample: &mut F,
) -> Result<PcWienerFeatureValues>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    let block_end_plus_two = usize_to_isize(
        params
            .block_end_x
            .checked_add(2)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener block end plus two",
            })?,
        "PC-Wiener block end plus two",
    )?;
    let x = x.min(block_end_plus_two);

    let m = source_value(source_sample, x, y, params.bit_depth)?;
    let up = source_value(
        source_sample,
        x,
        coordinate_add(y, -1, "PC-Wiener up y")?,
        params.bit_depth,
    )?;
    let down = source_value(
        source_sample,
        x,
        coordinate_add(y, 1, "PC-Wiener down y")?,
        params.bit_depth,
    )?;
    let upright = source_value(
        source_sample,
        coordinate_add(x, 1, "PC-Wiener right x")?,
        coordinate_add(y, -1, "PC-Wiener up y")?,
        params.bit_depth,
    )?;
    let downleft = source_value(
        source_sample,
        coordinate_add(x, -1, "PC-Wiener left x")?,
        coordinate_add(y, 1, "PC-Wiener down y")?,
        params.bit_depth,
    )?;
    let downright = source_value(
        source_sample,
        coordinate_add(x, 1, "PC-Wiener right x")?,
        coordinate_add(y, 1, "PC-Wiener down y")?,
        params.bit_depth,
    )?;
    let upleft = source_value(
        source_sample,
        coordinate_add(x, -1, "PC-Wiener left x")?,
        coordinate_add(y, -1, "PC-Wiener up y")?,
        params.bit_depth,
    )?;

    let vert = up - 2 * m + down;
    let anti_diag = upright - 2 * m + downleft;
    let diag = upleft - 2 * m + downright;
    Ok(PcWienerFeatureValues {
        x,
        values: [0, vert.abs(), anti_diag.abs(), diag.abs()],
    })
}

fn source_value<T, F>(source_sample: &mut F, x: isize, y: isize, bit_depth: BitDepth) -> Result<i64>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    let value = source_sample(x, y).to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        return Err(ReconError::PcWienerSourceSampleOutOfRange { x, y, value, max });
    }
    Ok(i64::from(value))
}

fn tx_skip_lookup(
    params: &PcWienerClassifyParams,
    x: isize,
    y: isize,
) -> Result<PcWienerTxSkipLookup> {
    let x = clip_isize_to_usize_range(
        x,
        params.block_start_x,
        params.block_end_x,
        "PC-Wiener tx-skip x bounds",
    )?;
    let y = clip_isize_to_usize_range(
        y,
        params.luma_stripe_start_y,
        params.luma_stripe_end_y,
        "PC-Wiener tx-skip stripe y bounds",
    )?;
    let y = y.clamp(params.tile_start_y, params.tile_end_y);
    Ok(PcWienerTxSkipLookup {
        x,
        y,
        row: y >> 2,
        col: x >> 2,
    })
}

fn pc_wiener_lut_input(
    features: [i64; PC_WIENER_NUM_FEATURES],
    tx_skip: i64,
    qindex: u32,
    bit_depth: BitDepth,
) -> Result<u16> {
    let mut lut_input = 0i64;
    for (i, feature) in features.iter().enumerate() {
        let qval = round2_signed(
            feature
                .checked_add(qval_given_tx_skip(qindex, tx_skip, i, bit_depth)?)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener feature qval",
                })?,
            PC_WIENER_PREC_FEATURE,
        )
        .clamp(0, 255)
            >> 5;
        lut_input =
            lut_input
                .checked_add(qval << (3 * (3 - i)))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener LUT input",
                })?;
    }
    u16::try_from(lut_input).map_err(|_| ReconError::ArithmeticOverflow {
        context: "PC-Wiener LUT input",
    })
}

fn qval_given_tx_skip(
    qindex: u32,
    tx_skip: i64,
    feature_index: usize,
    bit_depth: BitDepth,
) -> Result<i64> {
    let mut qstep = i64::from(quantizer_value(qindex, 0, bit_depth));
    let qstep_shift = QUANT_TABLE_BITS + 10;
    qstep = round2(qstep, u32::from(bit_depth.bits() - 8));
    let diff_shift = qstep_shift - 8;
    let prod = round2(
        tx_skip
            .checked_mul(qstep)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener tx-skip quantizer product",
            })?,
        8,
    );
    let qval = MODE_WEIGHTS[feature_index][0]
        .checked_mul(
            tx_skip
                .checked_shl(diff_shift)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener tx-skip shift",
                })?,
        )
        .and_then(|v| v.checked_add(MODE_WEIGHTS[feature_index][1].checked_mul(qstep)?))
        .and_then(|v| v.checked_add(MODE_WEIGHTS[feature_index][2].checked_mul(prod)?))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip qval",
        })?;
    MODE_OFFSETS[feature_index]
        .checked_add(round2_signed(qval, qstep_shift))
        .and_then(|v| v.checked_mul(255))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip qval offset",
        })
}

fn validate_params(params: &PcWienerClassifyParams) -> Result<()> {
    if params.block_start_x > params.block_end_x {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "block x range",
        });
    }
    if params.luma_stripe_start_y > params.luma_stripe_end_y {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "stripe y range",
        });
    }
    if params.tile_start_y > params.tile_end_y {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "tile y range",
        });
    }
    let _ = usize_to_isize(params.block_end_x, "PC-Wiener block end x")?;
    Ok(())
}

fn clip_isize_to_usize_range(
    value: isize,
    low: usize,
    high: usize,
    field: &'static str,
) -> Result<usize> {
    if low > high {
        return Err(ReconError::PcWienerInvalidBounds { field });
    }
    let low_i = usize_to_isize(low, field)?;
    let high_i = usize_to_isize(high, field)?;
    let clipped = value.clamp(low_i, high_i);
    usize::try_from(clipped).map_err(|_| ReconError::PcWienerInvalidBounds { field })
}

fn usize_to_isize(value: usize, context: &'static str) -> Result<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn coordinate_add(value: isize, delta: isize, context: &'static str) -> Result<isize> {
    value
        .checked_add(delta)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

/// AV2 § 4.8 `Round2Signed(value, n)`.
const fn round2_signed(value: i64, n: u32) -> i64 {
    if value >= 0 {
        round2(value, n)
    } else {
        -round2(-value, n)
    }
}

/// AV2 § 4.8 `Round2(value, n) = (value + (1 << (n - 1))) >> n` for `n > 0`,
/// and `value` for `n == 0`.
const fn round2(value: i64, n: u32) -> i64 {
    if n == 0 {
        value
    } else {
        (value + (1i64 << (n - 1))) >> n
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn params(bit_depth: BitDepth) -> PcWienerClassifyParams {
        PcWienerClassifyParams {
            x: 4,
            y: 4,
            bit_depth,
            base_q_idx: 0,
            block_start_x: 0,
            block_end_x: 63,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 63,
            tile_start_y: 0,
            tile_end_y: 63,
        }
    }

    #[test]
    fn flat_source_without_skips_classifies_to_lut_zero() {
        let result =
            pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| 12, |_| 0).unwrap();

        assert_eq!(result.raw_features, [0, 0, 0, 0]);
        assert_eq!(result.features, [0, 0, 0, 0]);
        assert_eq!(result.raw_tx_skip_sum, 0);
        assert_eq!(result.tx_skip, 0);
        assert_eq!(result.lut_input, 0);
        assert_eq!(result.class, 83);
    }

    #[test]
    fn quadratic_ten_bit_source_accumulates_features_and_tx_skip() {
        let result = pc_wiener_classify::<u16, _, _>(
            &params(BitDepth::Ten),
            |x, y| {
                let x = i64::try_from(x).unwrap();
                let y = i64::try_from(y).unwrap();
                u16::try_from(100 + x * x + 2 * y * y + 3 * x * y).unwrap()
            },
            |_| 1,
        )
        .unwrap();

        assert_eq!(result.raw_features, [0, 144, 0, 432]);
        assert_eq!(result.features, [0, 134_604, 0, 331_992]);
        assert_eq!(result.raw_tx_skip_sum, 36);
        assert_eq!(result.tx_skip, 252);
        assert_eq!(result.lut_input, 128);
        assert_eq!(result.class, 243);
    }

    #[test]
    fn tx_skip_lookup_clips_to_block_stripe_and_tile_bounds() {
        let params = PcWienerClassifyParams {
            x: 70,
            y: -4,
            bit_depth: BitDepth::Eight,
            base_q_idx: 0,
            block_start_x: 64,
            block_end_x: 80,
            luma_stripe_start_y: 4,
            luma_stripe_end_y: 20,
            tile_start_y: 8,
            tile_end_y: 16,
        };
        let mut first = None;
        pc_wiener_classify::<u8, _, _>(
            &params,
            |_, _| 0,
            |lookup| {
                first.get_or_insert(lookup);
                0
            },
        )
        .unwrap();

        assert_eq!(
            first,
            Some(PcWienerTxSkipLookup {
                x: 69,
                y: 8,
                row: 2,
                col: 17,
            })
        );
    }

    #[test]
    fn rejects_source_samples_outside_bit_depth() {
        let err = pc_wiener_classify::<u16, _, _>(&params(BitDepth::Eight), |_, _| 256, |_| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::PcWienerSourceSampleOutOfRange {
                x: 3,
                y: 3,
                value: 256,
                max: 255,
            }
        );
    }

    #[test]
    fn rejects_non_boolean_tx_skip_values() {
        let err =
            pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| 0, |_| 2).unwrap_err();

        assert_eq!(
            err,
            ReconError::PcWienerInvalidTxSkip {
                x: 3,
                y: 3,
                row: 0,
                col: 0,
                value: 2,
            }
        );
    }

    #[test]
    fn rejects_u8_storage_for_ten_bit_classification() {
        let err =
            pc_wiener_classify::<u8, _, _>(&params(BitDepth::Ten), |_, _| 0, |_| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            }
        );
    }

    #[test]
    fn rejects_inverted_bounds() {
        let mut params = params(BitDepth::Eight);
        params.block_start_x = 12;
        params.block_end_x = 10;
        let err = pc_wiener_classify::<u8, _, _>(&params, |_, _| 0, |_| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::PcWienerInvalidBounds {
                field: "block x range",
            }
        );
    }
}
