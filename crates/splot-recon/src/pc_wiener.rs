// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.4 pixel-classified Wiener filtering.
//!
//! This module implements scheduler-free primitives for the AV2 § 7.20.4
//! pixel-classified Wiener filter process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-4)).
//! The caller resolves § 7.20.2 source-sample selection, frame/restoration-unit
//! traversal, and the `LrTxSkip` grid storage. The primitives derive the
//! skip-filter class and apply the fixed § 9.8 filter selected by the caller;
//! they do not store `FilterClass` or wire runtime decode output.
//!
//! Feature tracking: `RECON-PC-WIENER-CLASSIFICATION`.

use splot_tables::tables::loop_restoration::{
    PC_WIENER_FILTERS, PC_WIENER_LUT_TO_CLASS, PC_WIENER_SUB_CLASSIFY, PC_WIENER_SUB_CLASSIFY2,
};

use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::math::{round2_i32, round2_signed_i32};
use crate::{BitDepth, ReconError, ReconSample, Result};

/// AV2 § 3 `PC_WIENER_NUM_FEATURES`.
pub const PC_WIENER_NUM_FEATURES: usize = 4;

/// Number of feature points in one dimension of AV2 § 7.20.4 `get_box_features`.
pub const PC_WIENER_FEATURE_WINDOW_SIDE: usize = 6;

/// Number of entries in AV2 § 9.8 `Pc_Wiener_Lut_To_Class`.
pub const PC_WIENER_LUT_INPUTS: usize = 4096;
/// Number of AV2 § 9.8 PC-Wiener LUT classes.
pub const PC_WIENER_LUT_CLASSES: usize = 256;
/// Number of AV2 § 9.8 PC-Wiener filter classes in the full classifier.
pub const PC_WIENER_FULL_CLASSES: usize = 64;

/// Distance between adjacent § 7.20.4 classification positions.
const PC_WIENER_BLOCK_SIZE: usize = 4;

/// Maximum absolute § 7.20.4 PC-Wiener filter-tap offset in either axis.
pub const PC_WIENER_FILTER_TAP_RADIUS: usize = 3;

/// Maximum distance of any § 7.20.4 classification source read from the
/// classified sample, in either axis (`PC_WIENER_LAG` plus the one-sample
/// feature neighborhood).
///
/// Callers that pre-resolve § 7.20.2 source samples may materialize a window
/// extending this many samples beyond the classified positions; classification
/// never reads farther (the § 7.20.4 x clipping only lowers coordinates).
pub const PC_WIENER_CLASSIFY_READ_RADIUS: usize = 5;

/// AV2 § 3 `PC_WIENER_PREC_FEATURE`.
const PC_WIENER_PREC_FEATURE: u32 = 14;
/// AV2 § 3 `QUANT_TABLE_BITS`.
const QUANT_TABLE_BITS: u32 = 3;
/// AV2 § 7.20.4 `PC_WIENER_LEAD`.
const PC_WIENER_LEAD: isize = 1;
/// AV2 § 7.20.4 `PC_WIENER_LAG`.
const PC_WIENER_LAG: isize = 4;
/// AV2 § 7.20.4 `Pc_Wiener_Normalizer`.
const PC_WIENER_NORMALIZER: [i32; PC_WIENER_NUM_FEATURES + 1] = [0, 3739, 3273, 3074, 7];
/// AV2 § 3 `PC_WIENER_PREC_BITS`.
const PC_WIENER_PREC_BITS: u32 = 7;
/// One representative from each symmetric pair in § 7.20.4 `Pc_Wiener_Config`.
const PC_WIENER_CONFIG: [(isize, isize); 12] = [
    (1, 0),
    (0, 1),
    (2, 0),
    (0, 2),
    (1, 1),
    (-1, 1),
    (2, 1),
    (2, -1),
    (1, 2),
    (1, -2),
    (3, 0),
    (0, 3),
];
/// AV2 § 7.20.4 `Mode_Weights`.
const MODE_WEIGHTS: [[i32; 3]; PC_WIENER_NUM_FEATURES] = [
    [-527, 15325, 321],
    [26436, -17705, 17905],
    [366, -147, -194],
    [202, -267, -179],
];
/// AV2 § 7.20.4 `Mode_Offsets`.
const MODE_OFFSETS: [i32; PC_WIENER_NUM_FEATURES] = [-547, -21565, -573, -680];

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
    pub raw_features: [i32; PC_WIENER_NUM_FEATURES],
    /// Normalized feature values returned by `get_box_features`.
    pub features: [i32; PC_WIENER_NUM_FEATURES],
    /// Raw accumulated `LrTxSkip` values over the 6x6 feature window.
    pub raw_tx_skip_sum: i32,
    /// Normalized `tskip` value returned by `get_box_features`.
    pub tx_skip: i32,
    /// AV2 § 7.20.4 `lutInput` in `0..4096`.
    pub lut_input: u16,
    /// AV2 § 7.20.4 `cls = Pc_Wiener_Lut_To_Class[lutInput]`.
    pub class: u8,
}

/// Reusable working storage for padded PC-Wiener grid classification.
#[derive(Debug, Default)]
pub struct PcWienerClassifyScratch {
    source_cache: Vec<u16>,
    features: Vec<CachedPcWienerFeature>,
    classifications: Vec<PcWienerClassification>,
}

/// § 7.20.2-pre-resolved source samples for [`pc_wiener_classify_grid_padded`].
///
/// A padded contiguous buffer covering the classified block extended on every
/// side. `origin_x` and `origin_y` are the current-plane luma coordinates of
/// buffer index `0`; a sample at absolute `(x, y)` is
/// `samples[(y - origin_y) * stride + (x - origin_x)]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerClassifyPaddedSource<'a, T> {
    samples: &'a [T],
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

impl<'a, T: ReconSample> PcWienerClassifyPaddedSource<'a, T> {
    /// Wraps a padded source buffer whose index `0` is current-plane luma
    /// coordinate `(origin_x, origin_y)` and whose rows are `stride` apart.
    #[must_use]
    pub fn new(samples: &'a [T], stride: usize, origin_x: isize, origin_y: isize) -> Self {
        Self {
            samples,
            stride,
            origin_x,
            origin_y,
        }
    }
}

/// Caller-resolved parameters for AV2 § 7.20.4 PC-Wiener block filtering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerFilter<'a> {
    /// Output block width in samples.
    pub width: usize,
    /// Output block height in samples.
    pub height: usize,
    /// Distance in samples between adjacent output rows.
    pub output_stride: usize,
    /// Active decoded bit depth used for source validation and `Clip1`.
    pub bit_depth: BitDepth,
    /// Caller-selected § 9.8 filter set.
    pub filter_set_index: usize,
    /// Row-major per-output-sample filter indices.
    pub subclasses: &'a [usize],
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
/// caller-resolved bounds, caller source-sample or `LrTxSkip` lookup failures,
/// source samples outside the active bit-depth range, `LrTxSkip` values outside
/// `0..=1`, and arithmetic overflow in coordinate or LUT derivation. The
/// function mutates no caller-owned output or grid state.
#[inline]
pub fn pc_wiener_classify<T, FS, FT>(
    params: &PcWienerClassifyParams,
    mut source_sample: FS,
    mut tx_skip: FT,
) -> Result<PcWienerClassification>
where
    T: ReconSample,
    FS: FnMut(isize, isize) -> Result<T>,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;

    let mut raw_features = [0i32; PC_WIENER_NUM_FEATURES];
    let mut raw_tx_skip_sum = 0i32;
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
            let skip = tx_skip(lookup)?;
            if !(0..=1).contains(&skip) {
                return Err(ReconError::PcWienerInvalidTxSkip {
                    x: lookup.x,
                    y: lookup.y,
                    row: lookup.row,
                    col: lookup.col,
                    value: skip,
                });
            }
            raw_tx_skip_sum =
                raw_tx_skip_sum
                    .checked_add(skip)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "PC-Wiener tx-skip accumulation",
                    })?;
        }
    }

    finish_pc_wiener_classification(
        raw_features,
        raw_tx_skip_sum,
        params.base_q_idx,
        params.bit_depth,
    )
}

/// Classifies a row-major grid of four-sample-spaced PC-Wiener cells.
///
/// `params.x` and `params.y` identify the first cell. All cells must share the
/// supplied block, stripe, and tile bounds. The implementation evaluates each
/// source sample and overlapping feature point once, then reuses them across
/// the 6x6 windows.
///
/// # Errors
///
/// Returns the same typed failures as [`pc_wiener_classify`], plus geometry or
/// allocation-size overflow for an unrepresentable grid.
#[inline]
pub fn pc_wiener_classify_grid<T, FS, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    mut source_sample: FS,
    tx_skip: FT,
) -> Result<Vec<PcWienerClassification>>
where
    T: ReconSample,
    FS: FnMut(isize, isize) -> Result<T>,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;
    if cell_cols == 0 || cell_rows == 0 {
        return Ok(Vec::new());
    }

    let geo = classify_grid_geometry(params, cell_cols, cell_rows)?;
    let mut source_cache = Vec::new();
    source_cache
        .try_reserve_exact(geo.source_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener source-grid allocation",
        })?;
    for row in 0..geo.source_height {
        let y = coordinate_add(
            geo.source_start_y,
            isize::try_from(row).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener source-grid row",
            })?,
            "PC-Wiener source-grid y",
        )?;
        for col in 0..geo.source_width {
            let x = coordinate_add(
                geo.source_start_x,
                isize::try_from(col).map_err(|_| ReconError::ArithmeticOverflow {
                    context: "PC-Wiener source-grid column",
                })?,
                "PC-Wiener source-grid x",
            )?;
            source_cache.push(source_sample_u16(
                &mut source_sample,
                x,
                y,
                params.bit_depth,
            )?);
        }
    }
    let mut features = Vec::new();
    let mut classifications = Vec::new();
    classify_grid_from_cache(
        params,
        cell_cols,
        cell_rows,
        &geo,
        &source_cache,
        &mut features,
        &mut classifications,
        tx_skip,
    )?;
    Ok(classifications)
}

/// Row-major grid of four-sample-spaced PC-Wiener cells from a padded
/// pre-resolved source.
///
/// Identical classification math and output as [`pc_wiener_classify_grid`]; the
/// § 7.20.2 source-sample process is resolved by the caller into `source`
/// instead of a per-sample callback. The `tx_skip` grid lookup stays a callback.
/// An up-front fits check proves the whole source region is in range, so the
/// feature cache is built by pure index addition off the padded-window base.
///
/// # Errors
/// Returns the same typed failures as [`pc_wiener_classify_grid`], plus a typed
/// [`ReconError`] when the padded source cannot cover the classification region.
#[inline]
pub fn pc_wiener_classify_grid_padded<T, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    tx_skip: FT,
) -> Result<Vec<PcWienerClassification>>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    let mut scratch = PcWienerClassifyScratch::default();
    pc_wiener_classify_grid_padded_into(
        params,
        cell_cols,
        cell_rows,
        source,
        tx_skip,
        &mut scratch,
    )?;
    Ok(core::mem::take(&mut scratch.classifications))
}

/// Classifies a padded PC-Wiener grid into reusable buffers.
///
/// `scratch` retains its allocations between calls. The returned slice contains
/// exactly `cell_cols * cell_rows` row-major classifications. Its internal
/// output is cleared before validation and remains empty on error.
///
/// # Errors
///
/// Returns the same errors as [`pc_wiener_classify_grid_padded`].
#[inline]
pub fn pc_wiener_classify_grid_padded_into<'a, T, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    tx_skip: FT,
    scratch: &'a mut PcWienerClassifyScratch,
) -> Result<&'a [PcWienerClassification]>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    scratch.source_cache.clear();
    scratch.features.clear();
    scratch.classifications.clear();
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;
    if cell_cols == 0 || cell_rows == 0 {
        return Ok(&scratch.classifications);
    }
    let geo = classify_grid_geometry(params, cell_cols, cell_rows)?;
    build_padded_source_cache_into(source, &geo, params.bit_depth, &mut scratch.source_cache)?;
    let result = classify_grid_from_cache(
        params,
        cell_cols,
        cell_rows,
        &geo,
        &scratch.source_cache,
        &mut scratch.features,
        &mut scratch.classifications,
        tx_skip,
    );
    if let Err(error) = result {
        scratch.classifications.clear();
        return Err(error);
    }
    Ok(&scratch.classifications)
}

/// Caller-resolved geometry shared by the callback and padded classify grids.
struct ClassifyGridGeometry {
    feature_width: usize,
    feature_height: usize,
    feature_start_x: isize,
    feature_start_y: isize,
    block_end_plus_two: isize,
    source_start_x: isize,
    source_start_y: isize,
    source_width: usize,
    source_height: usize,
    source_count: usize,
    feature_count: usize,
}

fn classify_grid_geometry(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
) -> Result<ClassifyGridGeometry> {
    let feature_width = (cell_cols - 1)
        .checked_mul(PC_WIENER_BLOCK_SIZE)
        .and_then(|span| span.checked_add(PC_WIENER_FEATURE_WINDOW_SIDE))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener feature-grid width",
        })?;
    let feature_height = (cell_rows - 1)
        .checked_mul(PC_WIENER_BLOCK_SIZE)
        .and_then(|span| span.checked_add(PC_WIENER_FEATURE_WINDOW_SIDE))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener feature-grid height",
        })?;
    let feature_count =
        feature_width
            .checked_mul(feature_height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener feature-grid sample count",
            })?;
    let feature_start_x = coordinate_add(params.x, -PC_WIENER_LEAD, "PC-Wiener grid x")?;
    let feature_start_y = coordinate_add(params.y, -PC_WIENER_LEAD, "PC-Wiener grid y")?;
    let feature_last_x = coordinate_add(
        feature_start_x,
        isize::try_from(feature_width - 1).map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener feature-grid last column",
        })?,
        "PC-Wiener feature-grid last x",
    )?;
    let block_end_plus_two = usize_to_isize(
        params
            .block_end_x
            .checked_add(2)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener block end plus two",
            })?,
        "PC-Wiener block end plus two",
    )?;
    let source_start_x = coordinate_add(
        feature_start_x.min(block_end_plus_two),
        -1,
        "PC-Wiener source-grid start x",
    )?;
    let source_last_x = coordinate_add(
        feature_last_x.min(block_end_plus_two),
        1,
        "PC-Wiener source-grid last x",
    )?;
    let source_width = source_last_x
        .checked_sub(source_start_x)
        .and_then(|span| usize::try_from(span).ok())
        .and_then(|span| span.checked_add(1))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener source-grid width",
        })?;
    let source_start_y = coordinate_add(feature_start_y, -1, "PC-Wiener source-grid start y")?;
    let source_height = feature_height
        .checked_add(2)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener source-grid height",
        })?;
    let source_count =
        source_width
            .checked_mul(source_height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener source-grid sample count",
            })?;
    Ok(ClassifyGridGeometry {
        feature_width,
        feature_height,
        feature_start_x,
        feature_start_y,
        block_end_plus_two,
        source_start_x,
        source_start_y,
        source_width,
        source_height,
        source_count,
        feature_count,
    })
}

fn build_padded_source_cache_into<T: ReconSample>(
    source: &PcWienerClassifyPaddedSource<'_, T>,
    geo: &ClassifyGridGeometry,
    bit_depth: BitDepth,
    source_cache: &mut Vec<u16>,
) -> Result<()> {
    let region_col = geo
        .source_start_x
        .checked_sub(source.origin_x)
        .and_then(|col| usize::try_from(col).ok())
        .ok_or(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener padded source column",
        })?;
    let region_row = geo
        .source_start_y
        .checked_sub(source.origin_y)
        .and_then(|row| usize::try_from(row).ok())
        .ok_or(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener padded source row",
        })?;
    let row_span =
        region_col
            .checked_add(geo.source_width)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener padded source row span",
            })?;
    if row_span > source.stride {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener padded source width",
        });
    }
    let required = region_row
        .checked_add(geo.source_height)
        .and_then(|rows| rows.checked_sub(1))
        .and_then(|last_row| last_row.checked_mul(source.stride))
        .and_then(|prefix| prefix.checked_add(row_span))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener padded source length",
        })?;
    if source.samples.len() < required {
        return Err(ReconError::BufferLengthMismatch {
            expected: required,
            actual: source.samples.len(),
        });
    }
    let max_sample = bit_depth.max_sample();
    source_cache
        .try_reserve_exact(geo.source_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener source-grid allocation",
        })?;
    for row in 0..geo.source_height {
        let base = (region_row + row) * source.stride + region_col;
        for col in 0..geo.source_width {
            let value = source.samples[base + col].to_u16();
            if value > max_sample {
                return Err(ReconError::PcWienerSourceSampleOutOfRange {
                    x: geo.source_start_x + col as isize,
                    y: geo.source_start_y + row as isize,
                    value,
                    max: max_sample,
                });
            }
            source_cache.push(value);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn classify_grid_from_cache<FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    geo: &ClassifyGridGeometry,
    source_cache: &[u16],
    cached: &mut Vec<CachedPcWienerFeature>,
    classifications: &mut Vec<PcWienerClassification>,
    mut tx_skip: FT,
) -> Result<()>
where
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    cached
        .try_reserve_exact(geo.feature_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener feature-grid allocation",
        })?;
    for row in 0..geo.feature_height {
        let y = coordinate_add(
            geo.feature_start_y,
            isize::try_from(row).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener feature-grid row",
            })?,
            "PC-Wiener feature-grid y",
        )?;
        for col in 0..geo.feature_width {
            let x = coordinate_add(
                geo.feature_start_x,
                isize::try_from(col).map_err(|_| ReconError::ArithmeticOverflow {
                    context: "PC-Wiener feature-grid column",
                })?,
                "PC-Wiener feature-grid x",
            )?;
            let x = x.min(geo.block_end_plus_two);
            let center_col = x
                .checked_sub(geo.source_start_x)
                .and_then(|col| usize::try_from(col).ok())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener source-grid center column",
                })?;
            let center_index = (row + 1) * geo.source_width + center_col;
            let m = i32::from(source_cache[center_index]);
            let up = i32::from(source_cache[center_index - geo.source_width]);
            let down = i32::from(source_cache[center_index + geo.source_width]);
            let upright = i32::from(source_cache[center_index - geo.source_width + 1]);
            let downleft = i32::from(source_cache[center_index + geo.source_width - 1]);
            let downright = i32::from(source_cache[center_index + geo.source_width + 1]);
            let upleft = i32::from(source_cache[center_index - geo.source_width - 1]);
            let values = [
                0,
                (up - 2 * m + down).abs(),
                (upright - 2 * m + downleft).abs(),
                (upleft - 2 * m + downright).abs(),
            ];
            let lookup = tx_skip_lookup(params, x, y)?;
            let skip = tx_skip(lookup)?;
            if !(0..=1).contains(&skip) {
                return Err(ReconError::PcWienerInvalidTxSkip {
                    x: lookup.x,
                    y: lookup.y,
                    row: lookup.row,
                    col: lookup.col,
                    value: skip,
                });
            }
            cached.push(CachedPcWienerFeature {
                values,
                tx_skip: skip,
            });
        }
    }

    let cell_count = cell_cols
        .checked_mul(cell_rows)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener classification-grid cell count",
        })?;
    classifications
        .try_reserve_exact(cell_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener classification-grid allocation",
        })?;
    for cell_row in 0..cell_rows {
        let feature_row =
            cell_row
                .checked_mul(PC_WIENER_BLOCK_SIZE)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener classification-grid row",
                })?;
        for cell_col in 0..cell_cols {
            let feature_col = cell_col.checked_mul(PC_WIENER_BLOCK_SIZE).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "PC-Wiener classification-grid column",
                },
            )?;
            let mut raw_features = [0i32; PC_WIENER_NUM_FEATURES];
            let mut raw_tx_skip_sum = 0i32;
            for row in 0..PC_WIENER_FEATURE_WINDOW_SIDE {
                let start = feature_row
                    .checked_add(row)
                    .and_then(|row| row.checked_mul(geo.feature_width))
                    .and_then(|start| start.checked_add(feature_col))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "PC-Wiener classification-grid feature index",
                    })?;
                let end = start.checked_add(PC_WIENER_FEATURE_WINDOW_SIDE).ok_or(
                    ReconError::ArithmeticOverflow {
                        context: "PC-Wiener classification-grid feature end",
                    },
                )?;
                let Some(feature_row) = cached.get(start..end) else {
                    return Err(ReconError::BufferLengthMismatch {
                        expected: end,
                        actual: cached.len(),
                    });
                };
                for feature in feature_row {
                    for (dst, src) in raw_features.iter_mut().zip(feature.values) {
                        *dst = dst.checked_add(src).ok_or(ReconError::ArithmeticOverflow {
                            context: "PC-Wiener feature accumulation",
                        })?;
                    }
                    raw_tx_skip_sum = raw_tx_skip_sum.checked_add(feature.tx_skip).ok_or(
                        ReconError::ArithmeticOverflow {
                            context: "PC-Wiener tx-skip accumulation",
                        },
                    )?;
                }
            }
            classifications.push(finish_pc_wiener_classification(
                raw_features,
                raw_tx_skip_sum,
                params.base_q_idx,
                params.bit_depth,
            )?);
        }
    }
    Ok(())
}

fn finish_pc_wiener_classification(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: i32,
    base_q_idx: u32,
    bit_depth: BitDepth,
) -> Result<PcWienerClassification> {
    let scale_shift = u32::from(bit_depth.bits() - 8);
    let mut features = [0i32; PC_WIENER_NUM_FEATURES];
    for (i, feature) in features.iter_mut().enumerate() {
        *feature = round2_i32(
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
    let lut_input = pc_wiener_lut_input(features, normalized_tx_skip, base_q_idx, bit_depth)?;
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

/// Returns the AVM/AV2 PC-Wiener filter-set index for a frame quantizer.
///
/// The AVM helper accepts an additional qindex offset used by some runtime
/// contexts; the normative § 7.20.4 classifier uses `base_q_idx`.
#[must_use]
#[inline]
pub const fn pc_wiener_filter_set_index(base_q_idx: u32) -> usize {
    if base_q_idx < 130 {
        0
    } else if base_q_idx < 190 {
        1
    } else if base_q_idx < 220 {
        2
    } else {
        3
    }
}

/// Maps a § 7.20.4 full PC-Wiener class to a caller-requested subclass count.
///
/// `num_classes` accepts the AV2-supported target counts 1, 2, 3, 4, 6, 8, 12,
/// 16, and 64. `filter_set_index` is from [`pc_wiener_filter_set_index`].
///
/// # Errors
/// Returns [`ReconError`] if any index or class count is outside the generated
/// § 9.8 table domain.
#[inline]
pub fn pc_wiener_subclass(
    num_classes: usize,
    filter_set_index: usize,
    full_class: u8,
) -> Result<usize> {
    let class_index = usize::from(full_class);
    if class_index >= PC_WIENER_LUT_CLASSES || filter_set_index >= PC_WIENER_SUB_CLASSIFY.len() {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener subclass table index",
        });
    }
    let value = if num_classes == PC_WIENER_FULL_CLASSES {
        PC_WIENER_SUB_CLASSIFY[filter_set_index][class_index]
    } else {
        let Some(target_index) = pc_wiener_subclass_target_index(num_classes) else {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener subclass target count",
            });
        };
        PC_WIENER_SUB_CLASSIFY2[filter_set_index][target_index][class_index]
    };
    usize::try_from(value).map_err(|_| ReconError::PcWienerInvalidBounds {
        field: "PC-Wiener subclass table value",
    })
}

/// Applies the fixed AV2 § 7.20.4 PC-Wiener filter to a luma block.
///
/// `subclasses` is a row-major per-output-sample map into the selected § 9.8
/// filter set. `source_sample(x, y)` receives block-relative coordinates after
/// the caller resolves § 7.20.2 source selection and frame offsets.
///
/// # Errors
/// Returns typed [`ReconError`] values for unsupported sample storage, invalid
/// block geometry or table indices, too-small buffers, source lookup failures,
/// and source samples outside the active bit-depth range. Output is fail-atomic.
#[inline]
pub fn pc_wiener_filter_block<T, F>(
    output: &mut [T],
    params: &PcWienerFilter<'_>,
    mut source_sample: F,
) -> Result<()>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> Result<T>,
{
    validate_sample_type::<T>(params.bit_depth)?;
    let (sample_count, filters) = validate_pc_wiener_filter(output.len(), params)?;

    let max_sample = i32::from(params.bit_depth.max_sample());
    let mut filtered = Vec::with_capacity(sample_count);
    for row in 0..params.height {
        for col in 0..params.width {
            let row = isize::try_from(row).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener filter row",
            })?;
            let col = isize::try_from(col).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener filter column",
            })?;
            let index = filtered.len();
            let coeffs = &filters[params.subclasses[index]];
            let center = source_value(&mut source_sample, col, row, params.bit_depth)?;
            let mut sum = (center << PC_WIENER_PREC_BITS) + center * coeffs[12];
            for (&(dy, dx), &coeff) in PC_WIENER_CONFIG.iter().zip(coeffs) {
                let positive = source_value(
                    &mut source_sample,
                    coordinate_add(col, dx, "PC-Wiener filter x")?,
                    coordinate_add(row, dy, "PC-Wiener filter y")?,
                    params.bit_depth,
                )?;
                let negative = source_value(
                    &mut source_sample,
                    coordinate_add(col, -dx, "PC-Wiener filter x")?,
                    coordinate_add(row, -dy, "PC-Wiener filter y")?,
                    params.bit_depth,
                )?;
                sum += (positive + negative) * coeff;
            }
            let sample = round2_i32(sum, PC_WIENER_PREC_BITS).clamp(0, max_sample);
            filtered.push(T::try_from_u16(sample as u16)?);
        }
    }

    write_pc_wiener_block(output, &filtered, params);
    Ok(())
}

/// § 7.20.2-pre-resolved source samples for [`pc_wiener_filter_block_padded`].
///
/// Row-major samples covering the output block extended by
/// [`PC_WIENER_FILTER_TAP_RADIUS`] on every side: index `0` is the sample at
/// block-relative `(-PC_WIENER_FILTER_TAP_RADIUS, -PC_WIENER_FILTER_TAP_RADIUS)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerPaddedSource<'a, T> {
    samples: &'a [T],
    stride: usize,
}

impl<'a, T: ReconSample> PcWienerPaddedSource<'a, T> {
    /// Wraps a padded source buffer for a `width` x `height` output block.
    ///
    /// `stride` is the distance in samples between adjacent padded rows.
    ///
    /// # Errors
    /// Returns typed [`ReconError`] values when the stride or length cannot
    /// cover the block plus the § 7.20.4 filter tap reach.
    pub fn new(samples: &'a [T], stride: usize, width: usize, height: usize) -> Result<Self> {
        let padded_width = width.checked_add(2 * PC_WIENER_FILTER_TAP_RADIUS).ok_or(
            ReconError::ArithmeticOverflow {
                context: "PC-Wiener padded source width",
            },
        )?;
        if stride < padded_width {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener padded source stride",
            });
        }
        let required = height
            .checked_add(2 * PC_WIENER_FILTER_TAP_RADIUS)
            .and_then(|rows| rows.checked_sub(1))
            .and_then(|prefix_rows| prefix_rows.checked_mul(stride))
            .and_then(|prefix| prefix.checked_add(padded_width))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener padded source length",
            })?;
        if samples.len() < required {
            return Err(ReconError::BufferLengthMismatch {
                expected: required,
                actual: samples.len(),
            });
        }
        Ok(Self { samples, stride })
    }
}

/// Applies the fixed AV2 § 7.20.4 PC-Wiener filter from a padded pre-resolved
/// source.
///
/// Identical filter math and output as [`pc_wiener_filter_block`]; the § 7.20.2
/// source-sample process is resolved by the caller into `source` instead of a
/// per-tap callback. The up-front [`PcWienerPaddedSource::new`] fits check
/// proves every tap read in range, so the tap loop is pure index addition off
/// each output sample's padded-window base.
///
/// # Errors
/// Returns the same typed [`ReconError`] values as [`pc_wiener_filter_block`],
/// including source samples outside the active bit-depth range.
#[inline]
pub fn pc_wiener_filter_block_padded<T: ReconSample>(
    output: &mut [T],
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let (sample_count, filters) = validate_pc_wiener_filter(output.len(), params)?;
    PcWienerPaddedSource::new(source.samples, source.stride, params.width, params.height)?;

    let stride = source.stride;
    let center_offset = padded_filter_offset(stride, 0, 0)?;
    let mut pos_offsets = [0usize; PC_WIENER_CONFIG.len()];
    let mut neg_offsets = [0usize; PC_WIENER_CONFIG.len()];
    for (i, &(dy, dx)) in PC_WIENER_CONFIG.iter().enumerate() {
        pos_offsets[i] = padded_filter_offset(stride, dy, dx)?;
        neg_offsets[i] = padded_filter_offset(stride, -dy, -dx)?;
    }

    let max_sample = params.bit_depth.max_sample();
    let padded_width = params.width + 2 * PC_WIENER_FILTER_TAP_RADIUS;
    let padded_rows = params.height + 2 * PC_WIENER_FILTER_TAP_RADIUS;
    // Invalid input still takes the original access order, preserving its exact error.
    let source_is_valid = (0..padded_rows).all(|row| {
        let start = row * stride;
        source.samples[start..start + padded_width]
            .iter()
            .all(|sample| sample.to_u16() <= max_sample)
    });
    if !source_is_valid {
        return pc_wiener_filter_block(output, params, |x, y| {
            let index = padded_filter_offset(stride, y, x)?;
            Ok(source.samples[index])
        });
    }

    let mut filtered = Vec::with_capacity(sample_count);
    let max_sample = i32::from(max_sample);
    for row in 0..params.height {
        for col in 0..params.width {
            let window_base = row * stride + col;
            let index = filtered.len();
            let coeffs = &filters[params.subclasses[index]];
            let center = i32::from(source.samples[window_base + center_offset].to_u16());
            let mut sum = (center << PC_WIENER_PREC_BITS) + center * coeffs[12];
            for i in 0..PC_WIENER_CONFIG.len() {
                let positive = i32::from(source.samples[window_base + pos_offsets[i]].to_u16());
                let negative = i32::from(source.samples[window_base + neg_offsets[i]].to_u16());
                sum += (positive + negative) * coeffs[i];
            }
            let sample = round2_i32(sum, PC_WIENER_PREC_BITS).clamp(0, max_sample);
            filtered.push(T::try_from_u16(sample as u16)?);
        }
    }

    write_pc_wiener_block(output, &filtered, params);
    Ok(())
}

fn validate_pc_wiener_filter(
    output_len: usize,
    params: &PcWienerFilter<'_>,
) -> Result<(usize, &'static [[i32; 13]; PC_WIENER_FULL_CLASSES])> {
    if params.width == 0 || params.height == 0 || params.output_stride < params.width {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter geometry",
        });
    }
    let sample_count =
        params
            .width
            .checked_mul(params.height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener filter sample count",
            })?;
    let output_expected = (params.height - 1)
        .checked_mul(params.output_stride)
        .and_then(|prefix| prefix.checked_add(params.width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener filter output length",
        })?;
    if output_len < output_expected {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_expected,
            actual: output_len,
        });
    }
    if params.subclasses.len() < sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: params.subclasses.len(),
        });
    }
    let Some(filters) = PC_WIENER_FILTERS.get(params.filter_set_index) else {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter set index",
        });
    };
    if params.subclasses[..sample_count]
        .iter()
        .any(|&subclass| subclass >= filters.len())
    {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter index",
        });
    }
    Ok((sample_count, filters))
}

fn write_pc_wiener_block<T: ReconSample>(
    output: &mut [T],
    filtered: &[T],
    params: &PcWienerFilter<'_>,
) {
    for row in 0..params.height {
        for col in 0..params.width {
            output[row * params.output_stride + col] = filtered[row * params.width + col];
        }
    }
}

fn padded_filter_offset(stride: usize, dy: isize, dx: isize) -> Result<usize> {
    let radius = PC_WIENER_FILTER_TAP_RADIUS as isize;
    let row = usize::try_from(dy + radius)
        .ok()
        .and_then(|row| row.checked_mul(stride))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener padded tap offset",
        })?;
    usize::try_from(dx + radius)
        .ok()
        .and_then(|col| row.checked_add(col))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener padded tap offset",
        })
}

const fn pc_wiener_subclass_target_index(num_classes: usize) -> Option<usize> {
    match num_classes {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        4 => Some(3),
        6 => Some(4),
        8 => Some(5),
        12 => Some(6),
        16 => Some(7),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PcWienerFeatureValues {
    x: isize,
    values: [i32; PC_WIENER_NUM_FEATURES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CachedPcWienerFeature {
    values: [i32; PC_WIENER_NUM_FEATURES],
    tx_skip: i32,
}

#[inline]
fn pc_wiener_features<T, F>(
    params: &PcWienerClassifyParams,
    x: isize,
    y: isize,
    source_sample: &mut F,
) -> Result<PcWienerFeatureValues>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> Result<T>,
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

#[inline]
fn source_value<T, F>(source_sample: &mut F, x: isize, y: isize, bit_depth: BitDepth) -> Result<i32>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> Result<T>,
{
    Ok(i32::from(source_sample_u16(
        source_sample,
        x,
        y,
        bit_depth,
    )?))
}

#[inline]
fn source_sample_u16<T, F>(
    source_sample: &mut F,
    x: isize,
    y: isize,
    bit_depth: BitDepth,
) -> Result<u16>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> Result<T>,
{
    let value = source_sample(x, y)?.to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        return Err(ReconError::PcWienerSourceSampleOutOfRange { x, y, value, max });
    }
    Ok(value)
}

#[inline]
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

#[inline]
fn pc_wiener_lut_input(
    features: [i32; PC_WIENER_NUM_FEATURES],
    tx_skip: i32,
    qindex: u32,
    bit_depth: BitDepth,
) -> Result<u16> {
    let terms = QvalTxSkipTerms::new(qindex, tx_skip, bit_depth)?;
    let mut lut_input = 0i32;
    for (i, feature) in features.iter().enumerate() {
        let qval = round2_signed_i32(
            feature.checked_add(qval_given_tx_skip(&terms, i)?).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "PC-Wiener feature qval",
                },
            )?,
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

/// Feature-independent § 7.20.4 `get_qval_given_tskip` terms, derived once per
/// classification instead of once per feature.
struct QvalTxSkipTerms {
    shifted_tx_skip: i32,
    qstep: i32,
    prod: i32,
}

impl QvalTxSkipTerms {
    fn new(qindex: u32, tx_skip: i32, bit_depth: BitDepth) -> Result<Self> {
        let mut qstep = i32::try_from(quantizer_value(qindex, 0, bit_depth)).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "PC-Wiener quantizer value",
            }
        })?;
        let qstep_shift = QUANT_TABLE_BITS + 10;
        qstep = round2_i32(qstep, u32::from(bit_depth.bits() - 8));
        let diff_shift = qstep_shift - 8;
        let prod = round2_i32(
            tx_skip
                .checked_mul(qstep)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener tx-skip quantizer product",
                })?,
            8,
        );
        let shifted_tx_skip =
            tx_skip
                .checked_shl(diff_shift)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener tx-skip shift",
                })?;
        Ok(Self {
            shifted_tx_skip,
            qstep,
            prod,
        })
    }
}

#[inline]
fn qval_given_tx_skip(terms: &QvalTxSkipTerms, feature_index: usize) -> Result<i32> {
    let qstep_shift = QUANT_TABLE_BITS + 10;
    let qval = MODE_WEIGHTS[feature_index][0]
        .checked_mul(terms.shifted_tx_skip)
        .and_then(|v| v.checked_add(MODE_WEIGHTS[feature_index][1].checked_mul(terms.qstep)?))
        .and_then(|v| v.checked_add(MODE_WEIGHTS[feature_index][2].checked_mul(terms.prod)?))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip qval",
        })?;
    MODE_OFFSETS[feature_index]
        .checked_add(round2_signed_i32(qval, qstep_shift))
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

#[inline]
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

#[inline]
fn usize_to_isize(value: usize, context: &'static str) -> Result<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

#[inline]
fn coordinate_add(value: isize, delta: isize, context: &'static str) -> Result<isize> {
    value
        .checked_add(delta)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn classify_read_radius_covers_lead_lag_neighborhood() {
        assert_eq!(
            PC_WIENER_CLASSIFY_READ_RADIUS,
            (PC_WIENER_LAG + 1).unsigned_abs()
        );
        assert!(PC_WIENER_CLASSIFY_READ_RADIUS >= (PC_WIENER_LEAD + 1).unsigned_abs());
    }

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
            pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| Ok(12), |_| Ok(0))
                .unwrap();

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
                let x = i32::try_from(x).unwrap();
                let y = i32::try_from(y).unwrap();
                Ok(u16::try_from(100 + x * x + 2 * y * y + 3 * x * y).unwrap())
            },
            |_| Ok(1),
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
    fn maximum_ten_bit_feature_window_fits_i32_classification_state() {
        let result = pc_wiener_classify::<u16, _, _>(
            &params(BitDepth::Ten),
            |_, y| Ok(if y & 1 == 0 { 1023 } else { 0 }),
            |_| Ok(1),
        )
        .unwrap();

        assert_eq!(result.raw_features, [0, 73_656, 73_656, 73_656]);
        assert_eq!(result.features, [0, 68_849_946, 60_269_022, 56_604_636]);
        assert_eq!(result.raw_tx_skip_sum, 36);
        assert_eq!(result.tx_skip, 252);
    }

    #[test]
    fn grid_classification_matches_scalar_cells_and_reuses_features() {
        let mut params = params(BitDepth::Eight);
        params.x = 52;
        params.block_end_x = 61;
        let cell_cols = 3;
        let cell_rows = 3;
        let source = |x: isize, y: isize| {
            let value = (3 * x + 5 * y + x * y).rem_euclid(200) + 20;
            Ok(u8::try_from(value).unwrap())
        };
        let tx_skip = |lookup: PcWienerTxSkipLookup| {
            Ok(i32::try_from((lookup.row + lookup.col) & 1).unwrap())
        };
        let mut grid_source_calls = 0;
        let grid = pc_wiener_classify_grid::<u8, _, _>(
            &params,
            cell_cols,
            cell_rows,
            |x, y| {
                grid_source_calls += 1;
                source(x, y)
            },
            tx_skip,
        )
        .unwrap();

        let mut scalar_source_calls = 0;
        let mut scalar = Vec::new();
        for row in 0..cell_rows {
            for col in 0..cell_cols {
                let mut cell = params;
                cell.x += isize::try_from(col * PC_WIENER_BLOCK_SIZE).unwrap();
                cell.y += isize::try_from(row * PC_WIENER_BLOCK_SIZE).unwrap();
                scalar.push(
                    pc_wiener_classify::<u8, _, _>(
                        &cell,
                        |x, y| {
                            scalar_source_calls += 1;
                            source(x, y)
                        },
                        tx_skip,
                    )
                    .unwrap(),
                );
            }
        }

        assert_eq!(grid, scalar);
        assert_eq!(grid_source_calls, 240);
        assert!(grid_source_calls * 7 < scalar_source_calls);
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
            |_, _| Ok(0),
            |lookup| {
                first.get_or_insert(lookup);
                Ok(0)
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
        let err =
            pc_wiener_classify::<u16, _, _>(&params(BitDepth::Eight), |_, _| Ok(256), |_| Ok(0))
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
    fn propagates_source_sample_errors() {
        let err = pc_wiener_classify::<u8, _, _>(
            &params(BitDepth::Eight),
            |_, _| {
                Err(ReconError::ArithmeticOverflow {
                    context: "test source sample",
                })
            },
            |_| Ok(0),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::ArithmeticOverflow {
                context: "test source sample",
            }
        );
    }

    #[test]
    fn propagates_tx_skip_lookup_errors() {
        let err = pc_wiener_classify::<u8, _, _>(
            &params(BitDepth::Eight),
            |_, _| Ok(0),
            |_| {
                Err(ReconError::ArithmeticOverflow {
                    context: "test tx-skip lookup",
                })
            },
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::ArithmeticOverflow {
                context: "test tx-skip lookup",
            }
        );
    }

    #[test]
    fn rejects_non_boolean_tx_skip_values() {
        let err = pc_wiener_classify::<u8, _, _>(&params(BitDepth::Eight), |_, _| Ok(0), |_| Ok(2))
            .unwrap_err();

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
        let err = pc_wiener_classify::<u8, _, _>(&params(BitDepth::Ten), |_, _| Ok(0), |_| Ok(0))
            .unwrap_err();

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
        let err = pc_wiener_classify::<u8, _, _>(&params, |_, _| Ok(0), |_| Ok(0)).unwrap_err();

        assert_eq!(
            err,
            ReconError::PcWienerInvalidBounds {
                field: "block x range",
            }
        );
    }

    #[test]
    fn fixed_filter_matches_hand_computed_quadratic_sample() {
        let mut output = [0u16; 1];
        let params = PcWienerFilter {
            width: 1,
            height: 1,
            output_stride: 1,
            bit_depth: BitDepth::Ten,
            filter_set_index: 0,
            subclasses: &[2],
        };
        pc_wiener_filter_block(&mut output, &params, |x, y| {
            u16::try_from(500 + 10 * x * x + 20 * y * y + 5 * x * y).map_err(|_| {
                ReconError::ArithmeticOverflow {
                    context: "test PC-Wiener source",
                }
            })
        })
        .unwrap();

        assert_eq!(output, [499]);
    }

    #[test]
    fn fixed_filter_rejects_out_of_range_subclass_without_writing() {
        let mut output = [7u8; 1];
        let params = PcWienerFilter {
            width: 1,
            height: 1,
            output_stride: 1,
            bit_depth: BitDepth::Eight,
            filter_set_index: 0,
            subclasses: &[PC_WIENER_FULL_CLASSES],
        };
        let err = pc_wiener_filter_block(&mut output, &params, |_, _| Ok(0)).unwrap_err();

        assert_eq!(
            err,
            ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener filter index",
            }
        );
        assert_eq!(output, [7]);
    }

    #[test]
    fn padded_and_callback_filters_match_bit_exactly() {
        let width = 6;
        let height = 5;
        let radius = PC_WIENER_FILTER_TAP_RADIUS;
        let stride = width + 2 * radius + 1;
        let subclasses: Vec<usize> = (0..width * height).map(|i| i % 5).collect();
        let source_at =
            |x: isize, y: isize| -> u16 { ((x * 23 + y * 11 + 400).rem_euclid(1024)) as u16 };
        let params = PcWienerFilter {
            width,
            height,
            output_stride: width,
            bit_depth: BitDepth::Ten,
            filter_set_index: 1,
            subclasses: &subclasses,
        };

        let mut callback_output = vec![0u16; width * height];
        pc_wiener_filter_block(&mut callback_output, &params, |x, y| Ok(source_at(x, y))).unwrap();

        let rows = height + 2 * radius;
        let mut padded = Vec::with_capacity(stride * rows);
        for row in 0..rows {
            for col in 0..stride {
                padded.push(source_at(
                    col as isize - radius as isize,
                    row as isize - radius as isize,
                ));
            }
        }
        let source = PcWienerPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut padded_output = vec![0u16; width * height];
        pc_wiener_filter_block_padded(&mut padded_output, &params, &source).unwrap();

        assert_eq!(callback_output, padded_output);
    }

    #[test]
    fn padded_filter_preserves_out_of_range_error_order_and_output() {
        let width = 1;
        let height = 1;
        let radius = PC_WIENER_FILTER_TAP_RADIUS;
        let stride = width + 2 * radius;
        let rows = height + 2 * radius;
        let params = PcWienerFilter {
            width,
            height,
            output_stride: width,
            bit_depth: BitDepth::Eight,
            filter_set_index: 0,
            subclasses: &[0],
        };

        for (index, x, y) in [(radius * stride + radius, 0, 0), (radius * stride, -3, 0)] {
            let mut padded = vec![0_u16; stride * rows];
            padded[index] = 256;
            let source = PcWienerPaddedSource::new(&padded, stride, width, height).unwrap();
            let mut output = [77_u16];
            let error = pc_wiener_filter_block_padded(&mut output, &params, &source).unwrap_err();

            assert_eq!(
                error,
                ReconError::PcWienerSourceSampleOutOfRange {
                    x,
                    y,
                    value: 256,
                    max: 255,
                }
            );
            assert_eq!(output, [77]);
        }
    }

    #[test]
    fn padded_and_callback_classify_grids_match_bit_exactly() {
        let mut params = params(BitDepth::Ten);
        params.x = 40;
        params.y = 24;
        params.base_q_idx = 96;
        let cell_cols = 3;
        let cell_rows = 4;
        let source_at =
            |x: isize, y: isize| -> u16 { ((x * 37 + y * 19 + 512).rem_euclid(1024)) as u16 };

        let callback = pc_wiener_classify_grid::<u16, _, _>(
            &params,
            cell_cols,
            cell_rows,
            |x, y| Ok(source_at(x, y)),
            alternating_tx_skip,
        )
        .unwrap();

        let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
        let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
        let padded = pc_wiener_classify_grid_padded::<u16, _>(
            &params,
            cell_cols,
            cell_rows,
            &source,
            alternating_tx_skip,
        )
        .unwrap();
        let mut scratch = PcWienerClassifyScratch::default();
        let reused = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            cell_cols,
            cell_rows,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap();

        assert_eq!(callback, padded);
        assert_eq!(padded, reused);
    }

    fn padded_classify_fixture(params: &PcWienerClassifyParams) -> (Vec<u16>, usize, isize, isize) {
        let origin_x = params.x - PC_WIENER_CLASSIFY_READ_RADIUS as isize;
        let origin_y = params.y - PC_WIENER_CLASSIFY_READ_RADIUS as isize;
        let stride = 64;
        let rows = 64;
        let mut buffer = Vec::with_capacity(stride * rows);
        for row in 0..rows {
            for col in 0..stride {
                let x = origin_x + col as isize;
                let y = origin_y + row as isize;
                buffer.push(((x * 37 + y * 19 + 512).rem_euclid(1024)) as u16);
            }
        }
        (buffer, stride, origin_x, origin_y)
    }

    fn alternating_tx_skip(lookup: PcWienerTxSkipLookup) -> Result<i32> {
        i32::try_from((lookup.row * 3 + lookup.col) & 1).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "test PC-Wiener tx-skip",
            }
        })
    }

    #[test]
    fn padded_into_reuses_capacity_from_large_to_small_grid() {
        let mut params = params(BitDepth::Ten);
        params.x = 40;
        params.y = 24;
        let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
        let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
        let mut scratch = PcWienerClassifyScratch::default();
        let large_output_ptr = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            8,
            8,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap()
        .as_ptr();
        let source_ptr = scratch.source_cache.as_ptr();
        let feature_ptr = scratch.features.as_ptr();
        let capacities = (
            scratch.source_cache.capacity(),
            scratch.features.capacity(),
            scratch.classifications.capacity(),
        );

        let small_output_ptr = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            1,
            1,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap()
        .as_ptr();

        assert_eq!(scratch.source_cache.as_ptr(), source_ptr);
        assert_eq!(scratch.features.as_ptr(), feature_ptr);
        assert_eq!(small_output_ptr, large_output_ptr);
        assert_eq!(
            (
                scratch.source_cache.capacity(),
                scratch.features.capacity(),
                scratch.classifications.capacity(),
            ),
            capacities
        );
    }

    #[test]
    fn padded_into_clears_output_after_error_and_empty_grid() {
        let mut params = params(BitDepth::Ten);
        params.x = 40;
        params.y = 24;
        let (buffer, stride, origin_x, origin_y) = padded_classify_fixture(&params);
        let source = PcWienerClassifyPaddedSource::new(&buffer, stride, origin_x, origin_y);
        let mut scratch = PcWienerClassifyScratch::default();
        pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            3,
            3,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap();

        let error = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            2,
            2,
            &source,
            |_| Ok(2),
            &mut scratch,
        )
        .unwrap_err();
        assert!(matches!(error, ReconError::PcWienerInvalidTxSkip { .. }));
        assert!(scratch.classifications.is_empty());

        let recovered = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            1,
            1,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap();
        assert_eq!(recovered.len(), 1);
        let capacity = scratch.classifications.capacity();

        let empty = pc_wiener_classify_grid_padded_into::<u16, _>(
            &params,
            0,
            3,
            &source,
            alternating_tx_skip,
            &mut scratch,
        )
        .unwrap();

        assert!(empty.is_empty());
        assert_eq!(scratch.classifications.capacity(), capacity);
    }

    #[test]
    fn classify_padded_source_rejects_origin_past_region() {
        let mut params = params(BitDepth::Ten);
        params.x = 40;
        params.y = 24;
        let buffer = vec![0u16; 64];
        let source = PcWienerClassifyPaddedSource::new(&buffer, 8, params.x + 100, params.y + 100);
        let err = pc_wiener_classify_grid_padded::<u16, _>(
            &params,
            2,
            2,
            &source,
            |lookup: PcWienerTxSkipLookup| Ok(i32::from((lookup.row + lookup.col) as u8 & 1)),
        )
        .unwrap_err();
        assert!(matches!(err, ReconError::PcWienerInvalidBounds { .. }));
    }
}
