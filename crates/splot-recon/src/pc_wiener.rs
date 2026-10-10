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
//! they do not store `FilterClass`.
//!
//! Feature tracking: `RECON-PC-WIENER-CLASSIFICATION`.

use splot_tables::tables::loop_restoration::{
    PC_WIENER_FILTERS, PC_WIENER_LUT_TO_CLASS, PC_WIENER_SUB_CLASSIFY, PC_WIENER_SUB_CLASSIFY2,
};
use std::simd::{Simd, cmp::SimdOrd, num::SimdInt, num::SimdUint, simd_swizzle};

use crate::PlaneId;
use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::math::{round2_i32, round2_signed_i32};
use crate::{BitDepth, ReconError, ReconSample, Result};

/// AV2 § 3 `PC_WIENER_NUM_FEATURES`.
pub const PC_WIENER_NUM_FEATURES: usize = 4;

/// Number of feature points in one dimension of AV2 § 7.20.4 `get_box_features`.
pub const PC_WIENER_FEATURE_WINDOW_SIDE: usize = 6;

/// Number of feature points in one § 7.20.4 `get_box_features` window.
const PC_WIENER_WINDOW_POINTS: usize =
    PC_WIENER_FEATURE_WINDOW_SIDE * PC_WIENER_FEATURE_WINDOW_SIDE;

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
#[derive(Debug)]
pub struct PcWienerClassifyScratch {
    source_cache: Vec<u16>,
    feature_grid: Vec<[u16; 4]>,
    skip_row: Vec<u16>,
    qval_cache_key: Option<(u32, BitDepth)>,
    qval_offsets: QvalOffsetsCache,
    classifications: Vec<PcWienerClassification>,
    classes: Vec<u8>,
}

impl Default for PcWienerClassifyScratch {
    fn default() -> Self {
        Self {
            source_cache: Vec::new(),
            feature_grid: Vec::new(),
            skip_row: Vec::new(),
            qval_cache_key: None,
            qval_offsets: [[0; PC_WIENER_NUM_FEATURES]; PC_WIENER_WINDOW_POINTS + 1],
            classifications: Vec::new(),
            classes: Vec::new(),
        }
    }
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
    validated_bit_depth: Option<BitDepth>,
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
            validated_bit_depth: None,
        }
    }

    /// Wraps decoder-owned samples whose range is guaranteed by reconstruction
    /// before the padded source window is materialized.
    ///
    /// This avoids rescanning the same window before every classification.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample storage cannot represent
    /// `bit_depth` or `stride` is zero.
    #[doc(hidden)]
    pub fn new_prevalidated(
        samples: &'a [T],
        stride: usize,
        origin_x: isize,
        origin_y: isize,
        bit_depth: BitDepth,
    ) -> Result<Self> {
        validate_sample_type::<T>(bit_depth)?;
        if stride == 0 {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener padded source stride",
            });
        }
        Ok(Self {
            samples,
            stride,
            origin_x,
            origin_y,
            validated_bit_depth: Some(bit_depth),
        })
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
    /// Side length covered by one subclass entry: `1` for a per-sample map or
    /// `4` for the normative PC-Wiener classification grid.
    pub subclass_block_size: usize,
    /// Row-major filter indices at [`Self::subclass_block_size`] spacing.
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
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener source-grid",
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
    let mut feature_grid = Vec::new();
    let mut skip_row = Vec::new();
    let mut classifications = Vec::new();
    let mut qval_offsets: QvalOffsetsCache =
        [[0; PC_WIENER_NUM_FEATURES]; PC_WIENER_WINDOW_POINTS + 1];
    prepare_qval_offsets_cache(params.base_q_idx, params.bit_depth, &mut qval_offsets)?;
    classify_grid_from_cache(
        params,
        cell_cols,
        cell_rows,
        &geo,
        &source_cache,
        geo.source_width,
        &mut feature_grid,
        &mut skip_row,
        &qval_offsets,
        &mut classifications,
        tx_skip,
        finish_pc_wiener_classification_cached,
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
    classify_grid_padded_mapped_into(
        params,
        cell_cols,
        cell_rows,
        source,
        tx_skip,
        &mut scratch.source_cache,
        &mut scratch.feature_grid,
        &mut scratch.skip_row,
        &mut scratch.qval_cache_key,
        &mut scratch.qval_offsets,
        &mut scratch.classifications,
        finish_pc_wiener_classification_cached,
    )
}

/// Compact runtime form of [`pc_wiener_classify_grid_padded_into`] that returns
/// only the final class byte for each cell.
/// # Errors
/// Returns the same errors as [`pc_wiener_classify_grid_padded_into`].
#[inline]
pub fn pc_wiener_classify_grid_padded_classes_into<'a, T, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    tx_skip: FT,
    scratch: &'a mut PcWienerClassifyScratch,
) -> Result<&'a [u8]>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    classify_grid_padded_mapped_into(
        params,
        cell_cols,
        cell_rows,
        source,
        tx_skip,
        &mut scratch.source_cache,
        &mut scratch.feature_grid,
        &mut scratch.skip_row,
        &mut scratch.qval_cache_key,
        &mut scratch.qval_offsets,
        &mut scratch.classes,
        finish_pc_wiener_class_cached,
    )
}

#[allow(clippy::too_many_arguments)]
fn classify_grid_padded_mapped_into<'a, T, FT, O, FM>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    tx_skip: FT,
    source_scratch: &mut Vec<u16>,
    feature_grid: &mut Vec<[u16; 4]>,
    skip_row: &mut Vec<u16>,
    qval_cache_key: &mut Option<(u32, BitDepth)>,
    qval_offsets: &mut QvalOffsetsCache,
    output: &'a mut Vec<O>,
    finish: FM,
) -> Result<&'a [O]>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
    FM: FnMut([i32; PC_WIENER_NUM_FEATURES], usize, BitDepth, &QvalOffsetsCache) -> Result<O>,
{
    source_scratch.clear();
    output.clear();
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;
    if cell_cols == 0 || cell_rows == 0 {
        return Ok(output);
    }
    let geo = classify_grid_geometry(params, cell_cols, cell_rows)?;
    let source_stride;
    let source_cache: &[u16];
    if let Some(samples) = T::u16_slice(source.samples) {
        let (start, end) = padded_source_region(source, &geo)?;
        let region = &samples[start..end];
        if source.validated_bit_depth != Some(params.bit_depth) {
            validate_padded_u16_source(region, source.stride, &geo, params.bit_depth)?;
        }
        source_stride = source.stride;
        source_cache = region;
    } else {
        build_padded_source_cache_into(
            source,
            &geo,
            params.bit_depth,
            source.validated_bit_depth != Some(params.bit_depth),
            source_scratch,
        )?;
        source_stride = geo.source_width;
        source_cache = source_scratch;
    }
    let key = (params.base_q_idx, params.bit_depth);
    if *qval_cache_key != Some(key) {
        prepare_qval_offsets_cache(params.base_q_idx, params.bit_depth, qval_offsets)?;
        *qval_cache_key = Some(key);
    }
    let result = classify_grid_from_cache(
        params,
        cell_cols,
        cell_rows,
        &geo,
        source_cache,
        source_stride,
        feature_grid,
        skip_row,
        qval_offsets,
        output,
        tx_skip,
        finish,
    );
    if let Err(error) = result {
        output.clear();
        return Err(error);
    }
    Ok(output)
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
    let source_count = checked_area(
        source_width,
        source_height,
        "PC-Wiener source-grid sample count",
    )?;
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
    })
}

fn build_padded_source_cache_into<T: ReconSample>(
    source: &PcWienerClassifyPaddedSource<'_, T>,
    geo: &ClassifyGridGeometry,
    bit_depth: BitDepth,
    validate: bool,
    source_cache: &mut Vec<u16>,
) -> Result<()> {
    let (region_start, _) = padded_source_region(source, geo)?;
    let max_sample = bit_depth.max_sample();
    source_cache
        .try_reserve_exact(geo.source_count)
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener source-grid",
        })?;
    for row in 0..geo.source_height {
        let base = region_start + row * source.stride;
        let row_samples = &source.samples[base..base + geo.source_width];
        let cached_start = source_cache.len();
        source_cache.extend(row_samples.iter().map(|sample| sample.to_u16()));
        if validate
            && let Some(col) = source_cache[cached_start..]
                .iter()
                .position(|&value| value > max_sample)
        {
            return Err(ReconError::PcWienerSourceSampleOutOfRange {
                x: geo.source_start_x + col as isize,
                y: geo.source_start_y + row as isize,
                value: source_cache[cached_start + col],
                max: max_sample,
            });
        }
    }
    Ok(())
}

fn padded_source_region<T: ReconSample>(
    source: &PcWienerClassifyPaddedSource<'_, T>,
    geo: &ClassifyGridGeometry,
) -> Result<(usize, usize)> {
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
    let start = region_row
        .checked_mul(source.stride)
        .and_then(|prefix| prefix.checked_add(region_col))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener padded source start",
        })?;
    Ok((start, required))
}

fn validate_padded_u16_source(
    source: &[u16],
    stride: usize,
    geo: &ClassifyGridGeometry,
    bit_depth: BitDepth,
) -> Result<()> {
    let max_sample = bit_depth.max_sample();
    for row in 0..geo.source_height {
        let base = row * stride;
        let row_samples = &source[base..base + geo.source_width];
        if crate::workspace::u16_samples_exceed(row_samples, max_sample)
            && let Some(col) = row_samples.iter().position(|&value| value > max_sample)
        {
            return Err(ReconError::PcWienerSourceSampleOutOfRange {
                x: geo.source_start_x + col as isize,
                y: geo.source_start_y + row as isize,
                value: row_samples[col],
                max: max_sample,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn classify_grid_from_cache<FT, O, FM>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    geo: &ClassifyGridGeometry,
    source_cache: &[u16],
    source_stride: usize,
    feature_grid: &mut Vec<[u16; 4]>,
    skip_row: &mut Vec<u16>,
    offsets_cache: &QvalOffsetsCache,
    output: &mut Vec<O>,
    mut tx_skip: FT,
    mut finish: FM,
) -> Result<()>
where
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
    FM: FnMut([i32; PC_WIENER_NUM_FEATURES], usize, BitDepth, &QvalOffsetsCache) -> Result<O>,
{
    build_feature_grid(
        params,
        geo,
        source_cache,
        source_stride,
        feature_grid,
        skip_row,
        &mut tx_skip,
    )?;

    let cell_count = checked_area(
        cell_cols,
        cell_rows,
        "PC-Wiener classification-grid cell count",
    )?;
    output
        .try_reserve_exact(cell_count)
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener classification-grid",
        })?;
    let pooled_width = geo.feature_width / 2;
    for cell_row in 0..cell_rows {
        let mut window_rows: [&[[u16; 8]]; 3] = [&[]; 3];
        let mut leading = Simd::<u16, 4>::splat(0);
        for (row, groups) in window_rows.iter_mut().enumerate() {
            let (head, tail) =
                pooled_row_window(feature_grid, (2 * cell_row + row) * pooled_width, cell_cols)?;
            leading += Simd::from_array(head);
            *groups = tail;
        }
        let mut shared = leading.cast::<i32>();
        for cell_col in 0..cell_cols {
            let mut pair = Simd::<u16, 8>::splat(0);
            for groups in window_rows {
                pair += Simd::from_array(groups[cell_col]);
            }
            let last = simd_swizzle!(pair, [4, 5, 6, 7]).cast::<i32>();
            let sums = (shared + simd_swizzle!(pair, [0, 1, 2, 3]).cast::<i32>() + last).to_array();
            output.push(finish(
                [0, sums[0], sums[1], sums[2]],
                usize::try_from(sums[3]).map_err(|_| ReconError::ArithmeticOverflow {
                    context: "PC-Wiener tx-skip cache index",
                })?,
                params.bit_depth,
                offsets_cache,
            )?);
            shared = last;
        }
    }
    Ok(())
}

/// Splits one pooled-grid row into its leading column and the per-cell
/// two-column groups, validating the row's whole `cell_cols` reach once.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_row_window(
    grid: &[[u16; 4]],
    start: usize,
    cell_cols: usize,
) -> Result<([u16; 4], &[[u16; 8]])> {
    let required = start + 1 + 2 * cell_cols;
    let Some((head, tail)) = grid.get(start..required).and_then(<[_]>::split_first) else {
        return Err(ReconError::BufferLengthMismatch {
            expected: required,
            actual: grid.len(),
        });
    };
    Ok((*head, tail.as_flattened().as_chunks::<8>().0))
}

/// Builds the 2x2-pooled feature grid: entry `(k, j)` sums the § 7.20.4
/// `get_features` values and `LrTxSkip` values of feature rows `2k..2k + 2` and
/// columns `2j..2j + 2`.
///
/// Every 6x6 `get_box_features` window starts on an even feature row and
/// column, so it is exactly 3x3 pooled entries. A pooled feature is at most
/// `4 * 2 * 1023` under § 6 Table 6.3's 10-bit cap, so three pooled rows still
/// fit `u16` lanes. The § 7.20.4 column clip keeps its center column inside the
/// source cache, so a row's linear span reaches exactly `linear_cols + 2`
/// cached samples. `get_tx_skip` clips every column again to `BlockEndX`, so
/// column `c` reads `LrTxSkip` column `Clip3(BlockStartX, BlockEndX, x) >> 2`;
/// the coordinates are kept relative to the first such column and clamped to
/// `feature_width + 8`, which never changes a clip result. Each clipped grid
/// row is fetched once and kept in two pooled slots: clipped grid rows never
/// decrease down the grid, so evicting the older slot never drops a row the
/// current pooled row reads.
fn build_feature_grid<FT>(
    params: &PcWienerClassifyParams,
    geo: &ClassifyGridGeometry,
    source_cache: &[u16],
    source_stride: usize,
    feature_grid: &mut Vec<[u16; 4]>,
    skip_rows: &mut Vec<u16>,
    tx_skip: &mut FT,
) -> Result<()>
where
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    let block_lo = usize_to_isize(params.block_start_x, "PC-Wiener tx-skip x bounds")?;
    let block_hi = usize_to_isize(params.block_end_x, "PC-Wiener tx-skip x bounds")?;
    let stripe_lo = usize_to_isize(
        params.luma_stripe_start_y,
        "PC-Wiener tx-skip stripe y bounds",
    )?;
    let stripe_hi = usize_to_isize(
        params.luma_stripe_end_y,
        "PC-Wiener tx-skip stripe y bounds",
    )?;
    let linear_cols = if geo.feature_start_x > geo.block_end_plus_two {
        0
    } else {
        geo.block_end_plus_two
            .checked_sub(geo.feature_start_x)
            .and_then(|span| usize::try_from(span).ok())
            .and_then(|span| span.checked_add(1))
            .map_or(geo.feature_width, |cols| cols.min(geo.feature_width))
    };
    let clamped_center = geo
        .block_end_plus_two
        .checked_sub(geo.source_start_x)
        .and_then(|col| usize::try_from(col).ok())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener source-grid center column",
        })?;
    let pooled_width = geo.feature_width / 2;
    let pooled_count = checked_area(
        pooled_width,
        geo.feature_height / 2,
        "PC-Wiener feature-grid sample count",
    )?;
    feature_grid
        .try_reserve_exact(pooled_count.saturating_sub(feature_grid.len()))
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener feature-grid",
        })?;
    feature_grid.resize(pooled_count, [0; 4]);
    let last_feature_x = coordinate_add(
        geo.feature_start_x,
        usize_to_isize(geo.feature_width - 1, "PC-Wiener feature-grid last x")?,
        "PC-Wiener feature-grid last x",
    )?;
    let first_x = geo.feature_start_x.clamp(block_lo, block_hi);
    let first_cell = first_x >> 2;
    let cell_count = ((last_feature_x.clamp(block_lo, block_hi) >> 2) - first_cell) as usize + 1;
    let base_x = 4 * first_cell;
    let span = usize_to_isize(geo.feature_width + 8, "PC-Wiener tx-skip span")?;
    let relative = |x: isize| (x - base_x).clamp(-span, span) as i32;
    let (start_x, low_x, high_x) = (
        relative(geo.feature_start_x),
        relative(block_lo),
        relative(block_hi),
    );
    let slot_width = pooled_width.next_multiple_of(8);
    let skip_len = cell_count + 16 + 2 * slot_width + pooled_width;
    skip_rows
        .try_reserve_exact(skip_len.saturating_sub(skip_rows.len()))
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener feature-grid",
        })?;
    skip_rows.resize(skip_len, 0);
    let (cells, pooled_skips) = skip_rows.split_at_mut(cell_count + 16);
    let (slots, pair_skip) = pooled_skips.split_at_mut(2 * slot_width);
    let mut slot_rows: [Option<usize>; 2] = [None; 2];
    let mut pair_rows = None;
    let source_row = |row: usize| {
        let start = row * source_stride;
        source_cache
            .get(start..start + geo.source_width)
            .ok_or(ReconError::BufferLengthMismatch {
                expected: start + geo.source_width,
                actual: source_cache.len(),
            })
    };
    for (pooled_row, grid_row) in feature_grid.chunks_exact_mut(pooled_width).enumerate() {
        let mut skip_grid_rows = [0usize; 2];
        for (half, skip_grid_row) in skip_grid_rows.iter_mut().enumerate() {
            let y = coordinate_add(
                geo.feature_start_y,
                isize::try_from(2 * pooled_row + half).map_err(|_| {
                    ReconError::ArithmeticOverflow {
                        context: "PC-Wiener feature-grid row",
                    }
                })?,
                "PC-Wiener feature-grid y",
            )?;
            let clipped_y = usize::try_from(y.clamp(stripe_lo, stripe_hi))
                .map_err(|_| ReconError::PcWienerInvalidBounds {
                    field: "PC-Wiener tx-skip stripe y bounds",
                })?
                .clamp(params.tile_start_y, params.tile_end_y);
            *skip_grid_row = clipped_y >> 2;
            if slot_rows.contains(&Some(*skip_grid_row)) {
                continue;
            }
            for (index, value) in cells[..cell_count].iter_mut().enumerate() {
                let x = (4 * (first_cell + index as isize)).max(first_x) as usize;
                *value = checked_tx_skip(tx_skip, x, clipped_y, *skip_grid_row)?;
            }
            let slot = match slot_rows {
                [None, _] => 0,
                [_, None] => 1,
                [Some(first), Some(second)] => usize::from(second < first),
            };
            pool_skip_row(
                &mut slots[slot * slot_width..(slot + 1) * slot_width],
                cells,
                start_x,
                [low_x, high_x],
            );
            slot_rows[slot] = Some(*skip_grid_row);
            pair_rows = None;
        }
        if pair_rows != Some(skip_grid_rows) {
            let slot_of = |grid_row| usize::from(slot_rows[0] != Some(grid_row));
            let (top, bottom) = (slot_of(skip_grid_rows[0]), slot_of(skip_grid_rows[1]));
            for ((pooled, &top), &bottom) in pair_skip
                .iter_mut()
                .zip(&slots[top * slot_width..])
                .zip(&slots[bottom * slot_width..])
            {
                *pooled = top + bottom;
            }
            pair_rows = Some(skip_grid_rows);
        }
        let rows = [
            source_row(2 * pooled_row)?,
            source_row(2 * pooled_row + 1)?,
            source_row(2 * pooled_row + 2)?,
            source_row(2 * pooled_row + 3)?,
        ];
        let mut j = 0;
        while 2 * j + 16 <= linear_cols {
            let chunk = 2 * j..2 * j + 18;
            pooled_features_simd(
                [
                    &rows[0][chunk.clone()],
                    &rows[1][chunk.clone()],
                    &rows[2][chunk.clone()],
                    &rows[3][chunk],
                ],
                &pair_skip[j..j + 8],
                &mut grid_row[j..j + 8],
            );
            j += 8;
        }
        for (j, entry) in grid_row.iter_mut().enumerate().skip(j) {
            let mut sums = [0i32; 3];
            for (up, cur, down) in [(rows[0], rows[1], rows[2]), (rows[1], rows[2], rows[3])] {
                for col in [2 * j, 2 * j + 1] {
                    let center = if col < linear_cols {
                        col + 1
                    } else {
                        clamped_center
                    };
                    let values = second_derivative_features(up, cur, down, center);
                    for (sum, value) in sums.iter_mut().zip(values) {
                        *sum += value;
                    }
                }
            }
            *entry = [sums[0] as u16, sums[1] as u16, sums[2] as u16, pair_skip[j]];
        }
    }
    Ok(())
}

/// Sums the `LrTxSkip` values of feature columns `2j` and `2j + 1` into
/// `pooled[j]`, writing whole eight-entry chunks only.
///
/// Feature column `c` reads `cells[(Clip3(low, high, start + c)) >> 2]`, with
/// every coordinate relative to the first fetched cell. Sixteen columns span
/// at most five cells, so each chunk gathers from a sixteen-cell window.
fn pool_skip_row(pooled: &mut [u16], cells: &[u16], start: i32, [low, high]: [i32; 2]) {
    const EVEN: [i32; 8] = [0, 2, 4, 6, 8, 10, 12, 14];
    for (chunk, pooled) in pooled.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let first = Simd::splat(start + 16 * chunk as i32) + Simd::from_array(EVEN);
        let clip = |x: Simd<i32, 8>| x.simd_clamp(Simd::splat(low), Simd::splat(high)) >> 2;
        let (even, odd) = (clip(first), clip(first + Simd::splat(1)));
        let window = even[0] as usize;
        let table = Simd::<u16, 16>::from_slice(&cells[window..]).cast::<u8>();
        let (even, odd) = (
            even - Simd::splat(window as i32),
            odd - Simd::splat(window as i32),
        );
        let indices = simd_swizzle!(
            even,
            odd,
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        )
        .cast::<u8>();
        let values = table.swizzle_dyn(indices);
        *pooled = (simd_swizzle!(values, [0, 1, 2, 3, 4, 5, 6, 7])
            + simd_swizzle!(values, [8, 9, 10, 11, 12, 13, 14, 15]))
        .cast::<u16>()
        .to_array();
    }
}

/// Pools sixteen feature columns of two feature rows into eight entries.
///
/// `rows` are the four source rows of the pooled row, each sliced to the
/// chunk's eighteen-sample reach.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_features_simd(rows: [&[u16]; 4], skip: &[u16], grid: &mut [[u16; 4]]) {
    const ZIP_LOW: [usize; 8] = [0, 8, 1, 9, 2, 10, 3, 11];
    const ZIP_HIGH: [usize; 8] = [4, 12, 5, 13, 6, 14, 7, 15];
    let [top_vertical, top_anti_diag, top_diag] =
        second_derivative_lanes(rows[0], rows[1], rows[2]);
    let [vertical, anti_diag, diag] = second_derivative_lanes(rows[1], rows[2], rows[3]);
    let pool = |top: Simd<u16, 16>, bottom: Simd<u16, 16>| {
        let rows = top + bottom;
        simd_swizzle!(rows, [0, 2, 4, 6, 8, 10, 12, 14])
            + simd_swizzle!(rows, [1, 3, 5, 7, 9, 11, 13, 15])
    };
    let vertical = pool(top_vertical, vertical);
    let anti_diag = pool(top_anti_diag, anti_diag);
    let diag = pool(top_diag, diag);
    let skip = Simd::<u16, 8>::from_slice(skip);
    let first = [
        simd_swizzle!(vertical, anti_diag, ZIP_LOW),
        simd_swizzle!(vertical, anti_diag, ZIP_HIGH),
    ];
    let second = [
        simd_swizzle!(diag, skip, ZIP_LOW),
        simd_swizzle!(diag, skip, ZIP_HIGH),
    ];
    for (half, (first, second)) in grid[..8]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(first.into_iter().zip(second))
    {
        let flat = half.as_flattened_mut();
        let low = simd_swizzle!(first, second, [0, 1, 8, 9, 2, 3, 10, 11]);
        let high = simd_swizzle!(first, second, [4, 5, 12, 13, 6, 7, 14, 15]);
        flat[..8].copy_from_slice(&low.to_array()); // splot-copy-ok: publish interleaved PC-Wiener SIMD features
        flat[8..].copy_from_slice(&high.to_array()); // splot-copy-ok: publish interleaved PC-Wiener SIMD features
    }
}

/// § 7.20.4 `get_features` absolute second derivatives (vertical, anti-diagonal,
/// diagonal) for sixteen consecutive centers `up[1..17]`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn second_derivative_lanes(up: &[u16], cur: &[u16], down: &[u16]) -> [Simd<u16, 16>; 3] {
    let load = |row: &[u16], start: usize| Simd::<u16, 16>::from_slice(&row[start..]).cast::<i16>();
    let twice_center = load(cur, 1) * Simd::splat(2);
    [
        (load(up, 1) - twice_center + load(down, 1)).abs().cast(),
        (load(up, 2) - twice_center + load(down, 0)).abs().cast(),
        (load(up, 0) - twice_center + load(down, 2)).abs().cast(),
    ]
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn second_derivative_features(up: &[u16], cur: &[u16], down: &[u16], center: usize) -> [i32; 3] {
    let m2 = 2 * i32::from(cur[center]);
    [
        (i32::from(up[center]) - m2 + i32::from(down[center])).abs(),
        (i32::from(up[center + 1]) - m2 + i32::from(down[center - 1])).abs(),
        (i32::from(up[center - 1]) - m2 + i32::from(down[center + 1])).abs(),
    ]
}

#[inline]
fn checked_tx_skip<FT>(tx_skip: &mut FT, x: usize, y: usize, row: usize) -> Result<u16>
where
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    let col = x >> 2;
    let value = tx_skip(PcWienerTxSkipLookup { x, y, row, col })?;
    if !(0..=1).contains(&value) {
        return Err(ReconError::PcWienerInvalidTxSkip {
            x,
            y,
            row,
            col,
            value,
        });
    }
    Ok(value as u16)
}

/// `qval_given_tskip` offsets keyed by the raw 6x6 window tx-skip sum.
type QvalOffsetsCache = [[i32; PC_WIENER_NUM_FEATURES]; PC_WIENER_WINDOW_POINTS + 1];

fn prepare_qval_offsets_cache(
    base_q_idx: u32,
    bit_depth: BitDepth,
    cache: &mut QvalOffsetsCache,
) -> Result<()> {
    for (raw_tx_skip_sum, offsets) in cache.iter_mut().enumerate() {
        let normalized_tx_skip = i32::try_from(raw_tx_skip_sum)
            .ok()
            .and_then(|sum| sum.checked_mul(PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES]))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener tx-skip normalization",
            })?;
        *offsets = qval_tx_skip_offsets(base_q_idx, normalized_tx_skip, bit_depth)?;
    }
    Ok(())
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn finish_pc_wiener_classification(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: i32,
    base_q_idx: u32,
    bit_depth: BitDepth,
) -> Result<PcWienerClassification> {
    let normalized_tx_skip = raw_tx_skip_sum
        .checked_mul(PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES])
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip normalization",
        })?;
    let offsets = qval_tx_skip_offsets(base_q_idx, normalized_tx_skip, bit_depth)?;
    Ok(finish_pc_wiener_classification_with_offsets(
        raw_features,
        raw_tx_skip_sum,
        normalized_tx_skip,
        bit_depth,
        &offsets,
    ))
}

#[allow(
    clippy::inline_always,
    reason = "measured PC-Wiener classification hot path"
)]
#[inline(always)]
fn finish_pc_wiener_classification_cached(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: usize,
    bit_depth: BitDepth,
    offsets_cache: &QvalOffsetsCache,
) -> Result<PcWienerClassification> {
    let Some(offsets) = offsets_cache.get(raw_tx_skip_sum) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip cache index",
        });
    };
    let raw_tx_skip_sum =
        i32::try_from(raw_tx_skip_sum).map_err(|_| ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip normalization",
        })?;
    let normalized_tx_skip = raw_tx_skip_sum * PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES];
    Ok(finish_pc_wiener_classification_with_offsets(
        raw_features,
        raw_tx_skip_sum,
        normalized_tx_skip,
        bit_depth,
        offsets,
    ))
}

#[allow(
    clippy::inline_always,
    reason = "measured PC-Wiener classification hot path"
)]
#[inline(always)]
fn finish_pc_wiener_class_cached(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: usize,
    bit_depth: BitDepth,
    offsets_cache: &QvalOffsetsCache,
) -> Result<u8> {
    let Some(offsets) = offsets_cache.get(raw_tx_skip_sum) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "PC-Wiener tx-skip cache index",
        });
    };
    let lut_input = pc_wiener_lut_input(normalized_features(raw_features, bit_depth), offsets);
    Ok(PC_WIENER_LUT_TO_CLASS[usize::from(lut_input)])
}

/// § 7.20.4 `nf[i] = Round2(f[i] * Pc_Wiener_Normalizer[i], BitDepth - 8)`.
///
/// `(x + ((1 << n) >> 1)) >> n` is `Round2` for every shift including zero,
/// and `36 * 4 * 1023 * 3739` keeps the product inside `i32`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn normalized_features(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    bit_depth: BitDepth,
) -> [i32; PC_WIENER_NUM_FEATURES] {
    let scale_shift = i32::from(bit_depth.bits() - 8);
    let [n0, n1, n2, n3, _] = PC_WIENER_NORMALIZER;
    let products = Simd::from_array(raw_features) * Simd::from_array([n0, n1, n2, n3]);
    ((products + Simd::splat((1 << scale_shift) >> 1)) >> Simd::splat(scale_shift)).to_array()
}

#[allow(
    clippy::inline_always,
    reason = "measured PC-Wiener classification hot path"
)]
#[inline(always)]
fn finish_pc_wiener_classification_with_offsets(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: i32,
    normalized_tx_skip: i32,
    bit_depth: BitDepth,
    offsets: &[i32; PC_WIENER_NUM_FEATURES],
) -> PcWienerClassification {
    let features = normalized_features(raw_features, bit_depth);
    let lut_input = pc_wiener_lut_input(features, offsets);
    let class = PC_WIENER_LUT_TO_CLASS[usize::from(lut_input)];

    PcWienerClassification {
        raw_features,
        features,
        raw_tx_skip_sum,
        tx_skip: normalized_tx_skip,
        lut_input,
        class,
    }
}

/// Returns the normative § 7.20.4 PC-Wiener filter-set index for `base_q_idx`.
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

/// Returns the full-class to filter-subclass map for one filter configuration.
///
/// # Errors
/// Returns [`ReconError`] for an unsupported class count or filter-set index.
pub fn pc_wiener_subclass_table(
    num_classes: usize,
    filter_set_index: usize,
) -> Result<&'static [u8; PC_WIENER_LUT_CLASSES]> {
    if filter_set_index >= PC_WIENER_SUB_CLASSIFY.len() {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener subclass table index",
        });
    }
    if num_classes == PC_WIENER_FULL_CLASSES {
        Ok(&PC_WIENER_SUB_CLASSIFY[filter_set_index])
    } else {
        let Some(target_index) = pc_wiener_subclass_target_index(num_classes) else {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener subclass target count",
            });
        };
        Ok(&PC_WIENER_SUB_CLASSIFY2[filter_set_index][target_index])
    }
}

/// Applies the fixed AV2 § 7.20.4 PC-Wiener filter to a luma block.
///
/// `subclasses` is a row-major map into the selected § 9.8 filter set at
/// `subclass_block_size` spacing. `source_sample(x, y)` receives block-relative
/// coordinates after the caller resolves § 7.20.2 source selection and frame
/// offsets.
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
            let coeffs = &filters[pc_wiener_subclass_at(params, row, col)];
            let row = isize::try_from(row).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener filter row",
            })?;
            let col = isize::try_from(col).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener filter column",
            })?;
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
    prevalidated: bool,
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
        Ok(Self {
            samples,
            stride,
            prevalidated: false,
        })
    }

    /// Wraps decoder-owned samples whose range reconstruction already
    /// guarantees, so filtering skips the per-block source range scan.
    ///
    /// # Errors
    /// Returns the same errors as [`Self::new`].
    #[doc(hidden)]
    pub fn new_prevalidated(
        samples: &'a [T],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        Ok(Self {
            prevalidated: true,
            ..Self::new(samples, stride, width, height)?
        })
    }
}

/// § 9.8 `Pc_Wiener_Filters` with each class narrowed to the `i16` lane factors
/// the SIMD row kernel multiplies by, and the center tap pre-folded with the
/// § 3 `PC_WIENER_PREC_BITS` unit scale.
static PC_WIENER_FILTERS_I16: std::sync::LazyLock<Vec<[[i16; 13]; PC_WIENER_FULL_CLASSES]>> =
    std::sync::LazyLock::new(|| {
        PC_WIENER_FILTERS
            .iter()
            .map(|set| {
                set.map(|coeffs| {
                    let mut packed = [0i16; 13];
                    for (slot, &coeff) in packed.iter_mut().zip(&coeffs) {
                        *slot = coeff as i16;
                    }
                    packed[12] = ((1 << PC_WIENER_PREC_BITS) + coeffs[12]) as i16;
                    packed
                })
            })
            .collect()
    });

struct PcWienerPaddedFilterSetup {
    sample_count: usize,
    filters: &'static [[i32; 13]; PC_WIENER_FULL_CLASSES],
    filters16: &'static [[i16; 13]; PC_WIENER_FULL_CLASSES],
    stride: usize,
    center_offset: usize,
    pos_offsets: [usize; PC_WIENER_CONFIG.len()],
    neg_offsets: [usize; PC_WIENER_CONFIG.len()],
    max_sample: u16,
    source_is_valid: bool,
}

fn prepare_pc_wiener_padded_filter<T: ReconSample>(
    output_len: usize,
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<PcWienerPaddedFilterSetup> {
    validate_sample_type::<T>(params.bit_depth)?;
    let (sample_count, filters) = validate_pc_wiener_filter(output_len, params)?;
    let filters16 = PC_WIENER_FILTERS_I16.get(params.filter_set_index).ok_or(
        ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter set index",
        },
    )?;
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
    let source_is_valid = source.prevalidated
        || T::MAX_VALUE <= max_sample
        || (0..padded_rows).all(|row| {
            let start = row * stride;
            !crate::workspace::samples_exceed(
                &source.samples[start..start + padded_width],
                max_sample,
            )
        });
    Ok(PcWienerPaddedFilterSetup {
        sample_count,
        filters,
        filters16,
        stride,
        center_offset,
        pos_offsets,
        neg_offsets,
        max_sample,
        source_is_valid,
    })
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
    let setup = prepare_pc_wiener_padded_filter(output.len(), params, source)?;
    if !setup.source_is_valid {
        return pc_wiener_filter_block(output, params, |x, y| {
            let index = padded_filter_offset(setup.stride, y, x)?;
            Ok(source.samples[index])
        });
    }

    if let Some(samples) = T::u16_slice(source.samples) {
        let Some(destination) = T::u16_slice_mut(output) else {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener sample storage",
            });
        };
        filter_pc_wiener_padded_u16(destination, params, samples, &setup);
        return Ok(());
    }

    let mut filtered = Vec::with_capacity(setup.sample_count);
    let mut acc = vec![0i32; params.width];
    let max_sample = i32::from(setup.max_sample);
    let subclass_cols = params.width.div_ceil(params.subclass_block_size);
    for row in 0..params.height {
        let subclass_row = row / params.subclass_block_size;
        let row_subclasses =
            &params.subclasses[subclass_row * subclass_cols..(subclass_row + 1) * subclass_cols];
        let row_base = row * setup.stride;
        let mut c0 = 0usize;
        while c0 < params.width {
            let subclass_col = c0 / params.subclass_block_size;
            let subclass = row_subclasses[subclass_col];
            let mut subclass_end = subclass_col + 1;
            while subclass_end < subclass_cols && row_subclasses[subclass_end] == subclass {
                subclass_end += 1;
            }
            let c1 = (subclass_end * params.subclass_block_size).min(params.width);
            let coeffs = &setup.filters[subclass];
            let len = c1 - c0;
            let seg_base = row_base + c0;
            let seg = &mut acc[..len];
            let center = &source.samples
                [seg_base + setup.center_offset..seg_base + setup.center_offset + len];
            for (a, &m) in seg.iter_mut().zip(center) {
                let m = i32::from(m.to_u16());
                *a = (m << PC_WIENER_PREC_BITS) + m * coeffs[12];
            }
            for ((&coeff, &pos), &neg) in coeffs
                .iter()
                .zip(&setup.pos_offsets)
                .zip(&setup.neg_offsets)
            {
                let plus = &source.samples[seg_base + pos..seg_base + pos + len];
                let minus = &source.samples[seg_base + neg..seg_base + neg + len];
                for ((a, &tp), &tm) in seg.iter_mut().zip(plus).zip(minus) {
                    *a += (i32::from(tp.to_u16()) + i32::from(tm.to_u16())) * coeff;
                }
            }
            for &sum in seg.iter() {
                let sample = round2_i32(sum, PC_WIENER_PREC_BITS).clamp(0, max_sample);
                filtered.push(T::try_from_u16(sample as u16)?);
            }
            c0 = c1;
        }
    }

    write_pc_wiener_block(output, &filtered, params);
    Ok(())
}

/// Applies padded PC-Wiener filtering directly into strided `u16` storage.
///
/// Both supported source storage types use the same filter arithmetic and SIMD
/// row kernel. Output geometry, source geometry, filter selection, subclasses,
/// and every source sample reached by the filter are validated before the
/// first destination write.
///
/// # Errors
/// Returns the same parameter, source-range, output-shape, and subclass errors
/// as [`pc_wiener_filter_block_padded`].
#[inline]
pub fn pc_wiener_filter_block_padded_u16_into<T: ReconSample>(
    output: &mut [u16],
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<()> {
    let setup = prepare_pc_wiener_padded_filter(output.len(), params, source)?;
    if !setup.source_is_valid {
        let packed_params = PcWienerFilter {
            output_stride: params.width,
            ..*params
        };
        let mut staged = vec![T::default(); setup.sample_count];
        pc_wiener_filter_block_padded(&mut staged, &packed_params, source)?;
        for row in 0..params.height {
            let source = &staged[row * params.width..(row + 1) * params.width];
            let destination =
                &mut output[row * params.output_stride..row * params.output_stride + params.width];
            for (destination, source) in destination.iter_mut().zip(source) {
                *destination = source.to_u16();
            }
        }
        return Ok(());
    }
    if let Some(samples) = T::u16_slice(source.samples) {
        filter_pc_wiener_padded_u16(output, params, samples, &setup);
        return Ok(());
    }
    if let Some(samples) = T::u8_slice(source.samples) {
        filter_pc_wiener_padded_u16(output, params, samples, &setup);
        return Ok(());
    }
    Err(ReconError::PcWienerInvalidBounds {
        field: "PC-Wiener sample storage",
    })
}

trait PcWienerPaddedSample: ReconSample {
    fn load_lanes<const LANES: usize>(samples: &[Self], start: usize) -> Simd<i16, LANES>;
}

impl PcWienerPaddedSample for u16 {
    fn load_lanes<const LANES: usize>(samples: &[Self], start: usize) -> Simd<i16, LANES> {
        Simd::<u16, LANES>::from_slice(&samples[start..]).cast()
    }
}

impl PcWienerPaddedSample for u8 {
    fn load_lanes<const LANES: usize>(samples: &[Self], start: usize) -> Simd<i16, LANES> {
        Simd::<u8, LANES>::from_slice(&samples[start..]).cast()
    }
}

fn filter_pc_wiener_padded_u16<T: PcWienerPaddedSample>(
    destination: &mut [u16],
    params: &PcWienerFilter<'_>,
    samples: &[T],
    setup: &PcWienerPaddedFilterSetup,
) {
    const R: usize = PC_WIENER_FILTER_TAP_RADIUS;
    let padded_width = params.width + 2 * R;
    let subclass_cols = params.width.div_ceil(params.subclass_block_size);
    for row in 0..params.height {
        let subclass_row = row / params.subclass_block_size;
        let row_subclasses =
            &params.subclasses[subclass_row * subclass_cols..(subclass_row + 1) * subclass_cols];
        let output = &mut destination[row * params.output_stride..][..params.width];
        let mut rows: [&[T]; PC_WIENER_WINDOW_ROWS] = [&[]; PC_WIENER_WINDOW_ROWS];
        for (dy, window_row) in rows.iter_mut().enumerate() {
            *window_row = &samples[(row + dy) * setup.stride..][..padded_width];
        }
        let mut c0 = 0usize;
        while c0 < params.width {
            let subclass_col = c0 / params.subclass_block_size;
            let subclass = row_subclasses[subclass_col];
            let mut subclass_end = subclass_col + 1;
            while subclass_end < subclass_cols && row_subclasses[subclass_end] == subclass {
                subclass_end += 1;
            }
            let c1 = (subclass_end * params.subclass_block_size).min(params.width);
            let coeffs = &setup.filters[subclass];
            let coeffs16 = &setup.filters16[subclass];
            let mut col = c0;
            macro_rules! filter_chunks {
                ($lanes:literal) => {
                    while col + $lanes <= c1 {
                        filter_pc_wiener_padded_u16_simd::<$lanes, T>(
                            &mut output[col..],
                            &rows,
                            col,
                            coeffs16,
                            setup.max_sample,
                        );
                        col += $lanes;
                    }
                };
            }
            filter_chunks!(32);
            filter_chunks!(16);
            filter_chunks!(8);
            filter_chunks!(4);
            for (x, slot) in output.iter_mut().enumerate().take(c1).skip(col) {
                let tap = |dy: isize, dx: isize| {
                    i32::from(
                        rows[R.wrapping_add_signed(dy)][(x + R).wrapping_add_signed(dx)].to_u16(),
                    )
                };
                let center = tap(0, 0);
                let mut sum = (center << PC_WIENER_PREC_BITS) + center * coeffs[12];
                for (&(dy, dx), &coeff) in PC_WIENER_CONFIG.iter().zip(coeffs) {
                    sum += (tap(dy, dx) + tap(-dy, -dx)) * coeff;
                }
                *slot = round2_i32(sum, PC_WIENER_PREC_BITS).clamp(0, i32::from(setup.max_sample))
                    as u16;
            }
            c0 = c1;
        }
    }
}

/// Padded source rows one § 7.20.4 PC-Wiener output row reads.
const PC_WIENER_WINDOW_ROWS: usize = 2 * PC_WIENER_FILTER_TAP_RADIUS + 1;

/// One `LANES`-wide group of [`filter_pc_wiener_padded_u16`] at output column
/// `col`, with both § 7.20.4 multiply factors paired as `i16`:
/// `Pc_Wiener_Filters` spans `-208..=127`, and § 6 Table 6.3 caps `BitDepth` at
/// 10 over a source this path has already validated, so `plus + minus` is at
/// most 2046 and `coeffs[12]` carries the folded
/// `(1 << PC_WIENER_PREC_BITS) + centerCoeff` scale. Each row is sliced once to
/// the group's reach, so the unrolled constant tap offsets need no further
/// bounds checks.
#[allow(clippy::inline_always)]
#[inline(always)]
fn filter_pc_wiener_padded_u16_simd<const LANES: usize, T: PcWienerPaddedSample>(
    output: &mut [u16],
    rows: &[&[T]; PC_WIENER_WINDOW_ROWS],
    col: usize,
    coeffs: &[i16; 13],
    max_sample: u16,
) {
    const R: usize = PC_WIENER_FILTER_TAP_RADIUS;
    let window = rows.map(|row| &row[col..col + LANES + 2 * R]);
    let mut sum = T::load_lanes::<LANES>(window[R], R).cast::<i32>()
        * Simd::<i16, LANES>::splat(coeffs[12]).cast::<i32>();
    macro_rules! tap_pairs {
        ($($j:literal)*) => {
            $(add_pc_wiener_tap_pair::<LANES, $j, T>(&mut sum, &window, coeffs);)*
        };
    }
    tap_pairs!(0 1 2 3 4 5 6 7 8 9 10 11);
    let values = ((sum + Simd::splat(1 << (PC_WIENER_PREC_BITS - 1)))
        >> Simd::splat(PC_WIENER_PREC_BITS as i32))
    .simd_clamp(Simd::splat(0), Simd::splat(i32::from(u16::MAX)))
    .cast::<u16>()
    .simd_min(Simd::splat(max_sample))
    .to_array();
    output[..LANES].copy_from_slice(&values); // splot-copy-ok: publish PC-Wiener SIMD lanes
}

/// Adds § 7.20.4 symmetric tap pair `J` of a [`filter_pc_wiener_padded_u16_simd`]
/// group.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_pc_wiener_tap_pair<const LANES: usize, const J: usize, T: PcWienerPaddedSample>(
    sum: &mut Simd<i32, LANES>,
    window: &[&[T]; PC_WIENER_WINDOW_ROWS],
    coeffs: &[i16; 13],
) {
    const R: usize = PC_WIENER_FILTER_TAP_RADIUS;
    let (dy, dx) = PC_WIENER_CONFIG[J];
    let plus = T::load_lanes::<LANES>(window[R.wrapping_add_signed(dy)], R.wrapping_add_signed(dx));
    let minus = T::load_lanes::<LANES>(
        window[R.wrapping_add_signed(-dy)],
        R.wrapping_add_signed(-dx),
    );
    *sum += (plus + minus).cast::<i32>() * Simd::<i16, LANES>::splat(coeffs[J]).cast::<i32>();
}

fn validate_pc_wiener_filter(
    output_len: usize,
    params: &PcWienerFilter<'_>,
) -> Result<(usize, &'static [[i32; 13]; PC_WIENER_FULL_CLASSES])> {
    if params.width == 0
        || params.height == 0
        || params.output_stride < params.width
        || !matches!(params.subclass_block_size, 1 | PC_WIENER_BLOCK_SIZE)
    {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter geometry",
        });
    }
    let sample_count = checked_area(params.width, params.height, "PC-Wiener filter sample count")?;
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
    let subclass_count = params
        .width
        .div_ceil(params.subclass_block_size)
        .checked_mul(params.height.div_ceil(params.subclass_block_size))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "PC-Wiener subclass count",
        })?;
    if params.subclasses.len() < subclass_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: subclass_count,
            actual: params.subclasses.len(),
        });
    }
    let Some(filters) = PC_WIENER_FILTERS.get(params.filter_set_index) else {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter set index",
        });
    };
    if params.subclasses[..subclass_count]
        .iter()
        .any(|&subclass| subclass >= filters.len())
    {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter index",
        });
    }
    Ok((sample_count, filters))
}

fn pc_wiener_subclass_at(params: &PcWienerFilter<'_>, row: usize, col: usize) -> usize {
    let subclass_cols = params.width.div_ceil(params.subclass_block_size);
    params.subclasses
        [(row / params.subclass_block_size) * subclass_cols + col / params.subclass_block_size]
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

/// Derives the four feature-indexed `get_qval_given_tskip` values for one
/// classification's normalized tx-skip.
fn qval_tx_skip_offsets(
    qindex: u32,
    tx_skip: i32,
    bit_depth: BitDepth,
) -> Result<[i32; PC_WIENER_NUM_FEATURES]> {
    let terms = QvalTxSkipTerms::new(qindex, tx_skip, bit_depth)?;
    let mut offsets = [0i32; PC_WIENER_NUM_FEATURES];
    for (i, offset) in offsets.iter_mut().enumerate() {
        *offset = qval_given_tx_skip(&terms, i)?;
    }
    Ok(offsets)
}

#[inline]
fn pc_wiener_lut_input(
    features: [i32; PC_WIENER_NUM_FEATURES],
    qval_offsets: &[i32; PC_WIENER_NUM_FEATURES],
) -> u16 {
    let adjusted = Simd::from_array(features) + Simd::from_array(*qval_offsets);
    let rounded =
        (adjusted + Simd::splat(1 << (PC_WIENER_PREC_FEATURE - 1)) + (adjusted >> Simd::splat(31)))
            >> Simd::splat(PC_WIENER_PREC_FEATURE as i32);
    let qval = rounded.simd_clamp(Simd::splat(0), Simd::splat(255)) >> Simd::splat(5);
    (qval << Simd::from_array([9, 6, 3, 0])).reduce_sum() as u16
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
    usize_to_isize(params.block_end_x, "PC-Wiener block end x")?;
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

fn checked_area(width: usize, height: usize, context: &'static str) -> Result<usize> {
    width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
#[path = "pc_wiener/tests.rs"]
mod tests;
