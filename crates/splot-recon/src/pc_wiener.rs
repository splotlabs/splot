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

use fearless_simd::{Simd, SimdBase, i16x16, i32x4, i32x8, i64x2, prelude::*, u16x8, u16x16};
use splot_tables::tables::loop_restoration::{
    PC_WIENER_FILTERS, PC_WIENER_LUT_TO_CLASS, PC_WIENER_SUB_CLASSIFY, PC_WIENER_SUB_CLASSIFY2,
};

use crate::PlaneId;
use crate::dequant::quantizer_value;
use crate::intra_dc_math::validate_sample_type;
use crate::loop_restoration::PaddedRows;
use crate::math::{round2_i32, round2_signed_i32};
use crate::wienerns_filter::eight_bit::ByteLaneOutput;
use crate::wienerns_filter::{FlatChunks, LumaSimdOutput, LumaSimdSource};
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

/// A run of `len` clipped AV2 § 7.20.4 `LrTxSkip` lookups: grid row `row`,
/// columns `col..col + len`.
///
/// The first lookup reads luma column `x`, and lookup `i > 0` reads luma
/// column `4 * (col + i)`; every lookup reads luma row `y`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerTxSkipRun {
    /// Luma sample x coordinate of the first lookup after § 7.20.4 clipping.
    pub x: usize,
    /// Luma sample y coordinate after § 7.20.4 clipping.
    pub y: usize,
    /// Zero-based `LrTxSkip` row.
    pub row: usize,
    /// Zero-based `LrTxSkip` column of the first lookup.
    pub col: usize,
    /// Number of lookups in the run.
    pub len: usize,
}

/// AV2 § 7.20.4 skip-filter classification result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
    feature_rows: Vec<u16>,
    skip_sums: Vec<u8>,
    qval_cache_key: Option<(u32, BitDepth)>,
    qval_offsets: QvalOffsetsCache,
    /// The two most recent subclass tables, each composed with
    /// `Pc_Wiener_Lut_To_Class`; a frame uses at most two.
    luts: [SubclassLut; 2],
    classifications: Vec<PcWienerClassification>,
}

impl Default for PcWienerClassifyScratch {
    fn default() -> Self {
        Self {
            feature_rows: Vec::new(),
            skip_sums: Vec::new(),
            qval_cache_key: None,
            qval_offsets: [[0; PC_WIENER_NUM_FEATURES]; QVAL_CACHE_LEN],
            luts: [(None, [0; PC_WIENER_LUT_INPUTS]); 2],
            classifications: Vec::new(),
        }
    }
}

/// § 7.20.2-pre-resolved source samples for [`pc_wiener_classify_grid_padded_into`].
///
/// Padded rows covering the classified block extended on every side, whose
/// first sample is current-plane luma coordinate `(origin_x, origin_y)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerClassifyPaddedSource<'a, T> {
    rows: PaddedRows<'a, T>,
    origin_x: isize,
    origin_y: isize,
}

impl<'a, T: ReconSample> PcWienerClassifyPaddedSource<'a, T> {
    /// Wraps padded rows: row `i` starts at column `col` of `rows[i]`, and its
    /// first sample is current-plane luma coordinate `(origin_x, origin_y + i)`.
    /// Classification checks the sample range.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample storage cannot represent
    /// `bit_depth`.
    #[doc(hidden)]
    pub fn from_rows(
        rows: &'a [&'a [T]],
        col: usize,
        origin_x: isize,
        origin_y: isize,
        bit_depth: BitDepth,
    ) -> Result<Self> {
        validate_sample_type::<T>(bit_depth)?;
        Ok(Self {
            rows: PaddedRows::table(rows, col),
            origin_x,
            origin_y,
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
    pub subclasses: &'a [u8],
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
            usize_to_isize(row, "PC-Wiener source-grid row")?,
            "PC-Wiener source-grid y",
        )?;
        for col in 0..geo.source_width {
            let x = coordinate_add(
                geo.source_start_x,
                usize_to_isize(col, "PC-Wiener source-grid column")?,
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
    let mut feature_rows = Vec::new();
    let mut skip_sums = Vec::new();
    let mut classifications = Vec::new();
    let mut qval_offsets: QvalOffsetsCache = [[0; PC_WIENER_NUM_FEATURES]; QVAL_CACHE_LEN];
    prepare_qval_offsets_cache(params.base_q_idx, params.bit_depth, &mut qval_offsets)?;
    classify_grid_from_cache(
        params,
        cell_cols,
        cell_rows,
        &geo,
        |row| cache_row(&source_cache, row, geo.source_width),
        &mut feature_rows,
        &mut skip_sums,
        &qval_offsets,
        &PC_WIENER_LUT_TO_CLASS,
        &mut classifications,
        tx_skip_cells(tx_skip),
        false,
    )?;
    Ok(classifications)
}

/// Row-major grid of four-sample-spaced PC-Wiener cells from a padded
/// pre-resolved source, into reusable buffers.
///
/// Identical classification math and output as [`pc_wiener_classify_grid`]; the
/// § 7.20.2 source-sample process is resolved by the caller into `source`
/// instead of a per-sample callback. The `tx_skip` grid lookup stays a callback.
/// `scratch` retains its allocations between calls. The returned slice contains
/// exactly `cell_cols * cell_rows` row-major classifications. Its internal
/// output is cleared before validation and remains empty on error.
///
/// # Errors
/// Returns the same typed failures as [`pc_wiener_classify_grid`], plus a typed
/// [`ReconError`] when the padded source cannot cover the classification region.
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
        tx_skip_cells(tx_skip),
        &mut scratch.feature_rows,
        &mut scratch.skip_sums,
        &mut scratch.qval_cache_key,
        &mut scratch.qval_offsets,
        &PC_WIENER_LUT_TO_CLASS,
        &mut scratch.classifications,
        false,
    )
}

/// Compact runtime form of [`pc_wiener_classify_grid_padded_into`] that writes
/// only `subclasses[cls]` for each cell's class `cls` into `output`.
///
/// `tx_skip(run)` answers a whole [`PcWienerTxSkipRun`] instead of one lookup
/// at a time: it returns `LrTxSkip` row `run.row` from column `run.col`, at
/// least `run.len` values long; values past `run.len` are ignored.
///
/// The cells may span several § 7.20.4 64-sample blocks: the cell at luma `x`
/// uses `BlockStartX = (x >> 6) << 6` and `BlockEndX = Min(params.block_end_x,
/// BlockStartX + 63)`. `params.x` must be a multiple of four and
/// `params.block_start_x` the first cell's `BlockStartX`; then only the last
/// block can reach its `get_features` clip, so one feature grid serves all.
/// # Errors
/// Returns the same errors as [`pc_wiener_classify_grid_padded_into`], and
/// [`ReconError::PcWienerInvalidBounds`] for a span that breaks these rules.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn pc_wiener_classify_grid_padded_classes_into<'a, 'g, T, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    mut tx_skip: FT,
    subclasses: &'static [u8; PC_WIENER_LUT_CLASSES],
    scratch: &mut PcWienerClassifyScratch,
    output: &'a mut Vec<u8>,
) -> Result<&'a [u8]>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipRun) -> Result<&'g [u8]>,
{
    let cached = |lut: &SubclassLut| lut.0.is_some_and(|key| core::ptr::eq(key, subclasses));
    if !cached(&scratch.luts[0]) {
        scratch.luts.swap(0, 1);
        if !cached(&scratch.luts[0]) {
            let (key, lut) = &mut scratch.luts[0];
            for (entry, &class) in lut.iter_mut().zip(&PC_WIENER_LUT_TO_CLASS) {
                *entry = subclasses[usize::from(class)];
            }
            *key = Some(subclasses);
        }
    }
    classify_grid_padded_mapped_into(
        params,
        cell_cols,
        cell_rows,
        source,
        |run, _| tx_skip(run).map(Some),
        &mut scratch.feature_rows,
        &mut scratch.skip_sums,
        &mut scratch.qval_cache_key,
        &mut scratch.qval_offsets,
        &scratch.luts[0].1,
        output,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn classify_grid_padded_mapped_into<'a, 'g, T, FT, O>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    source: &PcWienerClassifyPaddedSource<'_, T>,
    tx_skip: FT,
    feature_rows: &mut Vec<u16>,
    skip_sums: &mut Vec<u8>,
    qval_cache_key: &mut Option<(u32, BitDepth)>,
    qval_offsets: &mut QvalOffsetsCache,
    lut: &[u8; PC_WIENER_LUT_INPUTS],
    output: &'a mut Vec<O>,
    split_blocks: bool,
) -> Result<&'a [O]>
where
    T: ReconSample,
    FT: FnMut(PcWienerTxSkipRun, &mut Vec<u8>) -> Result<Option<&'g [u8]>>,
    O: ClassifiedCell,
{
    output.clear();
    validate_sample_type::<T>(params.bit_depth)?;
    validate_params(params)?;
    if cell_cols == 0 || cell_rows == 0 {
        return Ok(output);
    }
    let geo = classify_grid_geometry(params, cell_cols, cell_rows)?;
    let region = padded_source_region(source, &geo)?;
    validate_padded_u16_source(region, &geo, params.bit_depth)?;
    let key = (params.base_q_idx, params.bit_depth);
    if *qval_cache_key != Some(key) {
        prepare_qval_offsets_cache(params.base_q_idx, params.bit_depth, qval_offsets)?;
        *qval_cache_key = Some(key);
    }
    macro_rules! classify_rows {
        ($cast:expr) => {
            classify_grid_from_cache(
                params,
                cell_cols,
                cell_rows,
                &geo,
                |row| {
                    region.row(row, geo.source_width).and_then($cast).ok_or(
                        ReconError::BufferLengthMismatch {
                            expected: geo.source_width,
                            actual: 0,
                        },
                    )
                },
                feature_rows,
                skip_sums,
                qval_offsets,
                lut,
                output,
                tx_skip,
                split_blocks,
            )
        };
    }
    let result = if T::u8_slice(&[]).is_some() {
        classify_rows!(T::u8_slice)
    } else {
        classify_rows!(T::u16_slice)
    };
    if let Err(error) = result {
        output.clear();
        return Err(error);
    }
    Ok(output)
}

/// Caller-resolved geometry shared by the callback and padded classify grids.
struct ClassifyGridGeometry {
    feature_width: usize,
    feature_start_x: isize,
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
        usize_to_isize(feature_width - 1, "PC-Wiener feature-grid last column")?,
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
        feature_start_x,
        block_end_plus_two,
        source_start_x,
        source_start_y,
        source_width,
        source_height,
        source_count,
    })
}

/// Row `row` of a contiguous `width`-wide source cache.
fn cache_row(cache: &[u16], row: usize, width: usize) -> Result<&[u16]> {
    let start = row * width;
    cache
        .get(start..start + width)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: start + width,
            actual: cache.len(),
        })
}

/// The padded rows of `source` starting at the classification region,
/// checked to cover all of it.
fn padded_source_region<'a, T: ReconSample>(
    source: &PcWienerClassifyPaddedSource<'a, T>,
    geo: &ClassifyGridGeometry,
) -> Result<PaddedRows<'a, T>> {
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
    source
        .rows
        .offset(region_row, region_col)
        .filter(|region| region.covers(geo.source_height, geo.source_width))
        .ok_or(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener padded source rows",
        })
}

fn validate_padded_u16_source<T: ReconSample>(
    region: PaddedRows<'_, T>,
    geo: &ClassifyGridGeometry,
    bit_depth: BitDepth,
) -> Result<()> {
    let max_sample = bit_depth.max_sample();
    for row in 0..geo.source_height {
        let Some(row_samples) = region.row(row, geo.source_width).and_then(T::u16_slice) else {
            continue;
        };
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
fn classify_grid_from_cache<'s, 'g, T, FR, FT, O>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    geo: &ClassifyGridGeometry,
    source_row: FR,
    feature_rows: &mut Vec<u16>,
    skip_sums: &mut Vec<u8>,
    offsets_cache: &QvalOffsetsCache,
    lut: &[u8; PC_WIENER_LUT_INPUTS],
    output: &mut Vec<O>,
    mut tx_skip: FT,
    split_blocks: bool,
) -> Result<()>
where
    T: LumaSimdSource + 's,
    FR: Fn(usize) -> Result<&'s [T]>,
    FT: FnMut(PcWienerTxSkipRun, &mut Vec<u8>) -> Result<Option<&'g [u8]>>,
    O: ClassifiedCell,
{
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
    output.resize(cell_count, O::default());
    let first_x = usize::try_from(params.x)
        .ok()
        .filter(|x| x.is_multiple_of(4));
    if split_blocks
        && first_x.is_none_or(|x| {
            (x >> 6) << 6 != params.block_start_x
                || params.block_end_x < ((x + 4 * (cell_cols - 1)) >> 6) << 6
        })
    {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener block span",
        });
    }
    let skip_sums = tx_skip::window_sums(
        params,
        cell_cols,
        cell_rows,
        split_blocks,
        &mut tx_skip,
        skip_sums,
    )?;
    let skip_stride = tx_skip::sums_stride(cell_cols);

    let pooled_width = geo.feature_width / 2;
    let plane_width = 16 * cell_cols.div_ceil(8) + 2;
    let row_count = 9 * plane_width;
    feature_rows
        .try_reserve_exact(row_count.saturating_sub(feature_rows.len()))
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "PC-Wiener feature rows",
        })?;
    feature_rows.resize(row_count, 0);
    let rows_missing = || ReconError::BufferLengthMismatch {
        expected: row_count,
        actual: 0,
    };
    let columns = pooled_feature_columns(geo)?;
    let build_row = |feature_rows: &mut [u16], pooled_row: usize| -> Result<()> {
        let slot = pooled_row % 3 * 3 * plane_width;
        let rows = [
            source_row(2 * pooled_row)?,
            source_row(2 * pooled_row + 1)?,
            source_row(2 * pooled_row + 2)?,
            source_row(2 * pooled_row + 3)?,
        ];
        let planes = feature_rows
            .get_mut(slot..slot + 3 * plane_width)
            .ok_or_else(rows_missing)?;
        let (vertical, rest) = planes.split_at_mut(plane_width);
        let (anti_diag, diag) = rest.split_at_mut(plane_width);
        pooled_feature_row(rows, [vertical, anti_diag, diag], pooled_width, columns);
        Ok(())
    };
    let mut slots = [0, 1, 2];
    build_row(feature_rows, 0)?;
    for (cell_row, (cells, row_sums)) in output
        .chunks_mut(cell_cols)
        .zip(skip_sums.chunks(skip_stride))
        .enumerate()
    {
        build_row(feature_rows, 2 * cell_row + 1)?;
        build_row(feature_rows, 2 * cell_row + 2)?;
        let mut planes: [[&[u16]; 3]; 3] = [[&[]; 3]; 3];
        for (feature, rows) in planes.iter_mut().enumerate() {
            for (slot, row) in slots.iter().zip(rows) {
                let start = (3 * slot + feature) * plane_width;
                *row = feature_rows
                    .get(start..start + plane_width)
                    .ok_or_else(rows_missing)?;
            }
        }
        slots = [slots[2], slots[0], slots[1]];
        crate::lr_madd::lr_simd!(simd => classify_cells(
            simd,
            planes,
            row_sums,
            (offsets_cache, lut),
            params.bit_depth,
            cells,
        ))
        .ok_or_else(rows_missing)?;
    }
    Ok(())
}

/// Finishes up to eight cells of a grid row from the pooled feature planes
/// `planes[feature][row]` of its window rows: cell `c` sums pooled columns
/// `2c..2c + 3` of all three rows, as one pair sum and one even lane.
#[allow(clippy::inline_always)]
#[inline(always)]
fn classify_cells<S: Simd, O: ClassifiedCell>(
    simd: S,
    planes: [[&[u16]; 3]; 3],
    skips: &[u8],
    tables: (&QvalOffsetsCache, &[u8; PC_WIENER_LUT_INPUTS]),
    bit_depth: BitDepth,
    cells: &mut [O],
) -> Option<()> {
    const EVENS: [i16; 16] = [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0];
    let weights: [_; 3] = core::array::from_fn(|feature| {
        let scale = if O::SCALED {
            PC_WIENER_NORMALIZER[feature + 1] as i16
        } else {
            1
        };
        let evens = EVENS.map(|even| even * scale);
        (i16x16::splat(simd, scale), i16x16::from_slice(simd, &evens))
    });
    for (group, (cells, skips)) in cells.chunks_mut(8).zip(skips.chunks(8)).enumerate() {
        let base = 16 * group;
        let mut sums = [i32x8::splat(simd, 0); 3];
        for feature in 0..3 {
            let [mut pairs, mut next] = [u16x16::splat(simd, 0); 2];
            for row in planes[feature] {
                let window = row.get(base..)?.first_chunk::<18>()?;
                pairs += u16x16::from_slice(simd, &window[..16]);
                next += u16x16::from_slice(simd, &window[2..]);
            }
            let (pair_weight, even_weight) = weights[feature];
            sums[feature] = simd.pairwise_dot_product_i16x16(pairs.bitcast(), pair_weight)
                + simd.pairwise_dot_product_i16x16(next.bitcast(), even_weight);
        }
        O::finish_lanes(
            simd,
            sums,
            *skips.first_chunk::<8>()?,
            tables,
            bit_depth,
            cells,
        );
    }
    Some(())
}

/// A subclass table and its `lutInput` composition.
type SubclassLut = (
    Option<&'static [u8; PC_WIENER_LUT_CLASSES]>,
    [u8; PC_WIENER_LUT_INPUTS],
);

pub(super) fn check_tx_skip_cells(run: PcWienerTxSkipRun, cells: &[u8]) -> Result<()> {
    match cells.iter().position(|&value| value > 1) {
        Some(index) => Err(ReconError::PcWienerInvalidTxSkip {
            x: run_x(run, index),
            y: run.y,
            row: run.row,
            col: run.col + index,
            value: i32::from(cells[index]),
        }),
        None => Ok(()),
    }
}

/// The § 7.20.4 `get_features` column clip as pooled-row build inputs: the
/// count of feature columns whose center stays linear, and the source column
/// every other center clips to.
fn pooled_feature_columns(geo: &ClassifyGridGeometry) -> Result<[usize; 2]> {
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
    Ok([linear_cols, clamped_center])
}

/// Builds one row of the 2x2-pooled feature planes from its four source
/// `rows`: entry `j` of plane `i` sums the § 7.20.4 `get_features` value `i`
/// of the row's two feature rows and feature columns `2j..2j + 2`.
///
/// Every 6x6 `get_box_features` window starts on an even feature row and
/// column, so it is exactly 3x3 pooled entries. A pooled feature is at most
/// `4 * 2 * 1023` under § 6 Table 6.3's 10-bit cap, so three pooled rows still
/// fit `i16` lanes. The § 7.20.4 column clip keeps its center column inside the
/// source cache, so a row's linear span reaches exactly `linear_cols + 2`
/// cached samples. A row whose columns are all unclipped ends with a SIMD
/// chunk that overlaps the previous one instead of a scalar tail.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_feature_row<'a, T: LumaSimdSource>(
    rows: [&'a [T]; 4],
    planes: [&mut [u16]; 3],
    pooled_width: usize,
    [linear_cols, clamped_center]: [usize; 2],
) {
    let [Some(vertical), Some(anti_diag), Some(diag)] =
        planes.map(|plane| plane.get_mut(..pooled_width))
    else {
        return;
    };
    let mut planes = [vertical, anti_diag, diag];
    let linear_cols = linear_cols.min(2 * pooled_width);
    let full = linear_cols / 16;
    let overlap = linear_cols == 2 * pooled_width && pooled_width >= 8 && 8 * full < pooled_width;
    let (starts, reach) = match (overlap, full) {
        (true, _) => (full + 1, 2 * pooled_width + 2),
        (false, 0) => (0, 0),
        (false, _) => (full, 16 * full + 2),
    };
    let [Some(r0), Some(r1), Some(r2), Some(r3)] = rows.map(|row| row.get(..reach)) else {
        return;
    };
    let pairs = full / 2;
    let windows = |row: &'a [T]| {
        row.get(..32 * pairs + 2)
            .unwrap_or_default()
            .windows(34)
            .step_by(32)
    };
    let [vertical, anti_diag, diag] = &mut planes;
    let sources = windows(r0)
        .zip(windows(r1))
        .zip(windows(r2))
        .zip(windows(r3));
    let targets = pooled_lanes(vertical, 16 * pairs)
        .zip(pooled_lanes(anti_diag, 16 * pairs))
        .zip(pooled_lanes(diag, 16 * pairs));
    for ((((r0, r1), r2), r3), ((vertical, anti_diag), diag)) in sources.zip(targets) {
        if let (
            Some(r0),
            Some(r1),
            Some(r2),
            Some(r3),
            Some(vertical),
            Some(anti_diag),
            Some(diag),
        ) = (
            r0.first_chunk::<34>(),
            r1.first_chunk::<34>(),
            r2.first_chunk::<34>(),
            r3.first_chunk::<34>(),
            vertical.first_chunk_mut::<16>(),
            anti_diag.first_chunk_mut::<16>(),
            diag.first_chunk_mut::<16>(),
        ) {
            crate::lr_madd::lr_simd!(simd => pooled_features_lanes(simd, [r0, r1, r2, r3], [vertical, anti_diag, diag]));
        }
    }
    for index in 2 * pairs..starts {
        let start = (8 * index).min(pooled_width.saturating_sub(8));
        if !pooled_chunk([r0, r1, r2, r3], &mut planes, start) {
            return;
        }
    }
    let j = if overlap { pooled_width } else { 8 * full };
    for j in j..pooled_width {
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
        for (plane, sum) in planes.iter_mut().zip(sums) {
            if let Some(entry) = plane.get_mut(j) {
                *entry = sum as u16;
            }
        }
    }
}

/// The 16-entry chunks of the first `len` entries of a pooled plane.
fn pooled_lanes(plane: &mut [u16], len: usize) -> core::slice::ChunksExactMut<'_, u16> {
    plane
        .get_mut(..len)
        .unwrap_or_default()
        .chunks_exact_mut(16)
}

/// Pools 16 feature columns from `start` into 8 entries of each plane, from
/// the four source rows of the pooled row; returns `false` when a row or
/// plane is short.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_chunk<'a, T: LumaSimdSource>(
    rows: [&'a [T]; 4],
    planes: &mut [&mut [u16]; 3],
    start: usize,
) -> bool {
    let chunk = |row: &'a [T]| row.get(2 * start..).and_then(<[T]>::first_chunk::<18>);
    let [vertical, anti_diag, diag] = planes;
    if let (Some(r0), Some(r1), Some(r2), Some(r3), Some(vertical), Some(anti_diag), Some(diag)) = (
        chunk(rows[0]),
        chunk(rows[1]),
        chunk(rows[2]),
        chunk(rows[3]),
        vertical
            .get_mut(start..)
            .and_then(<[u16]>::first_chunk_mut::<8>),
        anti_diag
            .get_mut(start..)
            .and_then(<[u16]>::first_chunk_mut::<8>),
        diag.get_mut(start..)
            .and_then(<[u16]>::first_chunk_mut::<8>),
    ) {
        crate::lr_madd::lr_simd!(simd => pooled_features_lanes(simd, [r0, r1, r2, r3], [vertical, anti_diag, diag]));
        return true;
    }
    false
}

/// Pools `2 * N` feature columns of two feature rows into `N` (8 or 16)
/// entries of each plane; `rows` are the four source rows of the pooled row,
/// each sliced to the chunk's `2 * N + 2`-sample reach.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_features_lanes<S: Simd, T: LumaSimdSource, const REACH: usize, const N: usize>(
    simd: S,
    rows: [&[T; REACH]; 4],
    planes: [&mut [u16; N]; 3],
) {
    let low = pooled_sums(simd, &rows, 0);
    if N == 16 {
        let high = pooled_sums(simd, &rows, 16);
        for ((plane, low), high) in planes.into_iter().zip(low).zip(high) {
            if let Some(plane) = plane.as_mut_slice().first_chunk_mut::<16>() {
                let lanes: u16x16<S> = simd.saturating_narrow_i32x8(low, high).bitcast();
                lanes.store_array(plane);
            }
        }
        return;
    }
    let (vertical, anti_diag) = simd.saturating_narrow_i32x8(low[0], low[1]).split();
    let (diag, _) = simd.saturating_narrow_i32x8(low[2], low[2]).split();
    for (plane, lanes) in planes.into_iter().zip([vertical, anti_diag, diag]) {
        if let Some(plane) = plane.as_mut_slice().first_chunk_mut::<8>() {
            lanes.bitcast::<u16x8<S>>().store_array(plane);
        }
    }
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn pooled_sums<S: Simd, T: LumaSimdSource, const REACH: usize>(
    simd: S,
    rows: &[&[T; REACH]; 4],
    at: usize,
) -> [i32x8<S>; 3] {
    let cut = |row: usize| rows[row].get(at..).unwrap_or_default();
    let top = second_derivative_lanes(simd, cut(0), cut(1), cut(2));
    let bottom = second_derivative_lanes(simd, cut(1), cut(2), cut(3));
    let ones = i16x16::splat(simd, 1);
    [
        simd.pairwise_dot_product_i16x16((top[0] + bottom[0]).bitcast(), ones),
        simd.pairwise_dot_product_i16x16((top[1] + bottom[1]).bitcast(), ones),
        simd.pairwise_dot_product_i16x16((top[2] + bottom[2]).bitcast(), ones),
    ]
}

/// § 7.20.4 `get_features` absolute second derivatives (vertical, anti-diagonal,
/// diagonal) for sixteen consecutive centers `up[1..17]`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn second_derivative_lanes<S: Simd, T: LumaSimdSource>(
    simd: S,
    up: &[T],
    cur: &[T],
    down: &[T],
) -> [u16x16<S>; 3] {
    let load = |row: &[T], start: usize| -> i16x16<S> {
        row.get(start..)
            .and_then(<[T]>::first_chunk::<16>)
            .map_or_else(
                || i16x16::splat(simd, 0),
                |lanes| T::widen16(simd, lanes).bitcast(),
            )
    };
    let center = load(cur, 1);
    let twice_center = center + center;
    [
        (load(up, 1) - twice_center + load(down, 1)).abs().bitcast(),
        (load(up, 2) - twice_center + load(down, 0)).abs().bitcast(),
        (load(up, 0) - twice_center + load(down, 2)).abs().bitcast(),
    ]
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn second_derivative_features<T: LumaSimdSource>(
    up: &[T],
    cur: &[T],
    down: &[T],
    center: usize,
) -> [i32; 3] {
    let at = |row: &[T], col: usize| i32::from(T::scalar(row, col));
    let m2 = 2 * at(cur, center);
    [
        (at(up, center) - m2 + at(down, center)).abs(),
        (at(up, center + 1) - m2 + at(down, center - 1)).abs(),
        (at(up, center - 1) - m2 + at(down, center + 1)).abs(),
    ]
}

/// Luma column of lookup `index` of `run`.
fn run_x(run: PcWienerTxSkipRun, index: usize) -> usize {
    if index == 0 {
        run.x
    } else {
        4 * (run.col + index)
    }
}

/// Answers whole [`PcWienerTxSkipRun`]s with a per-lookup `tx_skip`, in the
/// same lookup order and with the same `0..=1` check.
fn tx_skip_cells<'g, FT>(
    mut tx_skip: FT,
) -> impl FnMut(PcWienerTxSkipRun, &mut Vec<u8>) -> Result<Option<&'g [u8]>>
where
    FT: FnMut(PcWienerTxSkipLookup) -> Result<i32>,
{
    move |run, cells| {
        cells.clear();
        cells
            .try_reserve(run.len)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "PC-Wiener tx-skip run",
            })?;
        for index in 0..run.len {
            cells.push(checked_tx_skip(
                &mut tx_skip,
                run_x(run, index),
                run.y,
                run.row,
            )?);
        }
        Ok(None)
    }
}

#[inline]
fn checked_tx_skip<FT>(tx_skip: &mut FT, x: usize, y: usize, row: usize) -> Result<u8>
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
    Ok(value as u8)
}

/// `qval_given_tskip` offsets keyed by the raw 6x6 window tx-skip sum, folded
/// into the rounding terms of [`rounded_lut_input`]. Any byte indexes it;
/// sums past [`PC_WIENER_WINDOW_POINTS`] are rejected before a lookup.
type QvalOffsetsCache = [[i32; PC_WIENER_NUM_FEATURES]; QVAL_CACHE_LEN];
const QVAL_CACHE_LEN: usize = 256;

fn prepare_qval_offsets_cache(
    base_q_idx: u32,
    bit_depth: BitDepth,
    cache: &mut QvalOffsetsCache,
) -> Result<()> {
    let scale_shift = u32::from(bit_depth.bits() - 8);
    let max_product = PC_WIENER_WINDOW_POINTS as i32
        * 4
        * i32::from(bit_depth.max_sample())
        * PC_WIENER_NORMALIZER[1];
    for (raw_tx_skip_sum, rounding) in cache
        .iter_mut()
        .take(PC_WIENER_WINDOW_POINTS + 1)
        .enumerate()
    {
        let normalized_tx_skip = i32::try_from(raw_tx_skip_sum)
            .ok()
            .and_then(|sum| sum.checked_mul(PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES]))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "PC-Wiener tx-skip normalization",
            })?;
        let offsets = qval_tx_skip_offsets(base_q_idx, normalized_tx_skip, bit_depth)?;
        for (term, offset) in rounding.iter_mut().zip(offsets) {
            *term = offset
                .checked_add(1 << (PC_WIENER_PREC_FEATURE - 1))
                .and_then(|value| value.checked_mul(1 << scale_shift))
                .and_then(|value| value.checked_add((1 << scale_shift) >> 1))
                .filter(|value| value.checked_add(max_product).is_some())
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PC-Wiener qval rounding term",
                })?;
        }
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
    Ok(finish_pc_wiener_classification_with_lut_input(
        raw_features,
        raw_tx_skip_sum,
        normalized_tx_skip,
        bit_depth,
        pc_wiener_lut_input(normalized_features(raw_features, bit_depth), &offsets),
    ))
}

/// One cell of a grid classification, finished from its raw 6x6 window sums
/// and the [`QvalOffsetsCache`] entry of its tx-skip sum.
trait ClassifiedCell: Copy + Default {
    /// Whether [`Self::finish_lanes`] takes the sums times `Pc_Wiener_Normalizer`.
    const SCALED: bool;

    /// Finishes `cells` (at most eight) from the window sums of features
    /// `1..4` and the tx-skip sum of each lane; a class byte is `lut[lutInput]`.
    fn finish_lanes<S: Simd>(
        simd: S,
        sums: [i32x8<S>; 3],
        skips: [u8; 8],
        tables: (&QvalOffsetsCache, &[u8; PC_WIENER_LUT_INPUTS]),
        bit_depth: BitDepth,
        cells: &mut [Self],
    );
}

impl ClassifiedCell for PcWienerClassification {
    const SCALED: bool = false;

    fn finish_lanes<S: Simd>(
        _simd: S,
        sums: [i32x8<S>; 3],
        skips: [u8; 8],
        (cache, _): (&QvalOffsetsCache, &[u8; PC_WIENER_LUT_INPUTS]),
        bit_depth: BitDepth,
        cells: &mut [Self],
    ) {
        let [first, second, third]: [[i32; 8]; 3] = sums.map(Into::into);
        for (lane, (cell, skip)) in cells.iter_mut().zip(skips).enumerate() {
            let raw_features = [0, first[lane], second[lane], third[lane]];
            let raw_tx_skip_sum = i32::from(skip);
            *cell = finish_pc_wiener_classification_with_lut_input(
                raw_features,
                raw_tx_skip_sum,
                raw_tx_skip_sum * PC_WIENER_NORMALIZER[PC_WIENER_NUM_FEATURES],
                bit_depth,
                rounded_lut_input(raw_features, &cache[usize::from(skip)], bit_depth),
            );
        }
    }
}

impl ClassifiedCell for u8 {
    const SCALED: bool = true;

    /// [`rounded_lut_input`] in eight lanes, then the class of each.
    #[allow(
        clippy::inline_always,
        reason = "measured PC-Wiener classification hot path"
    )]
    #[inline(always)]
    fn finish_lanes<S: Simd>(
        simd: S,
        sums: [i32x8<S>; 3],
        skips: [u8; 8],
        (cache, lut): (&QvalOffsetsCache, &[u8; PC_WIENER_LUT_INPUTS]),
        bit_depth: BitDepth,
        cells: &mut [Self],
    ) {
        let shift = PC_WIENER_PREC_FEATURE + 5 + u32::from(bit_depth.bits() - 8);
        let row = |lane: usize| i32x4::from_slice(simd, &cache[usize::from(skips[lane])]);
        let low = transpose4([row(0), row(1), row(2), row(3)]);
        let high = transpose4([row(4), row(5), row(6), row(7)]);
        let rounding = |feature: usize| low[feature].combine(high[feature]);
        let qval = |value: i32x8<S>| {
            (value >> shift)
                .max(i32x8::splat(simd, 0))
                .min(i32x8::splat(simd, 7))
        };
        let mut input = qval(rounding(0)) << 9;
        for (feature, product) in (1..).zip(sums) {
            input += qval(product + rounding(feature)) << (9 - 3 * feature as u32);
        }
        let inputs: [i32; 8] = input.into();
        let class = |input: i32| lut[input as usize % PC_WIENER_LUT_INPUTS];
        match cells.first_chunk_mut::<8>() {
            Some(full) => {
                for (cell, input) in full.iter_mut().zip(inputs) {
                    *cell = class(input);
                }
            }
            None => {
                for (cell, input) in cells.iter_mut().zip(inputs) {
                    *cell = class(input);
                }
            }
        }
    }
}

/// The columns of the 4x4 matrix whose rows are `rows`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn transpose4<S: Simd>([r0, r1, r2, r3]: [i32x4<S>; 4]) -> [i32x4<S>; 4] {
    let wide = |rows: i32x4<S>| rows.bitcast::<i64x2<S>>();
    let ((ab, ab_high), (cd, cd_high)) = (r0.interleave(r1), r2.interleave(r3));
    let (c0, c1) = wide(ab).interleave(wide(cd));
    let (c2, c3) = wide(ab_high).interleave(wide(cd_high));
    [c0.bitcast(), c1.bitcast(), c2.bitcast(), c3.bitcast()]
}

/// § 7.20.4 `lutInput` from raw box features and one [`QvalOffsetsCache`]
/// entry.
///
/// For `v = nf[i] + get_qval_given_tskip(..)`, `Clip3(0, 255,
/// Round2Signed(v, 14)) >> 5` equals `Clip3(0, 7, (v + 2^13) >> 19)`: a
/// negative `v` yields zero either way. `nf[i]` is `(f * N + h) >> s`, and
/// adding `(offset + 2^13) << s` before that shift is exact, so the cache
/// holds `((offset + 2^13) << s) + h` and one `>> (19 + s)` remains.
#[allow(clippy::inline_always)]
#[inline(always)]
fn rounded_lut_input(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    rounding: &[i32; PC_WIENER_NUM_FEATURES],
    bit_depth: BitDepth,
) -> u16 {
    let shift = (PC_WIENER_PREC_FEATURE + 5) as i32 + i32::from(bit_depth.bits() - 8);
    let mut input = 0;
    for (feature, (&raw, &rounding)) in raw_features.iter().zip(rounding).enumerate() {
        let product = raw.wrapping_mul(PC_WIENER_NORMALIZER[feature]);
        let qval = (product.wrapping_add(rounding) >> shift).clamp(0, 7);
        input += qval << (9 - 3 * feature);
    }
    input as u16
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
    core::array::from_fn(|feature| {
        raw_features[feature]
            .wrapping_mul(PC_WIENER_NORMALIZER[feature])
            .wrapping_add((1 << scale_shift) >> 1)
            >> scale_shift
    })
}

#[allow(
    clippy::inline_always,
    reason = "measured PC-Wiener classification hot path"
)]
#[inline(always)]
fn finish_pc_wiener_classification_with_lut_input(
    raw_features: [i32; PC_WIENER_NUM_FEATURES],
    raw_tx_skip_sum: i32,
    normalized_tx_skip: i32,
    bit_depth: BitDepth,
    lut_input: u16,
) -> PcWienerClassification {
    PcWienerClassification {
        raw_features,
        features: normalized_features(raw_features, bit_depth),
        raw_tx_skip_sum,
        tx_skip: normalized_tx_skip,
        lut_input,
        class: PC_WIENER_LUT_TO_CLASS[usize::from(lut_input)],
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
            let row = usize_to_isize(row, "PC-Wiener filter row")?;
            let col = usize_to_isize(col, "PC-Wiener filter column")?;
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
/// Rows covering the output block extended by [`PC_WIENER_FILTER_TAP_RADIUS`]
/// on every side: sample `0` of row `0` is at block-relative
/// `(-PC_WIENER_FILTER_TAP_RADIUS, -PC_WIENER_FILTER_TAP_RADIUS)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcWienerPaddedSource<'a, T> {
    rows: PaddedRows<'a, T>,
}

impl<'a, T: ReconSample> PcWienerPaddedSource<'a, T> {
    /// Wraps padded rows: padded row `i` starts at column `col` of
    /// `rows[i]`. Filtering checks the sample range.
    ///
    /// # Errors
    /// Returns a typed error when a row cannot cover the block plus the
    /// § 7.20.4 filter tap reach.
    pub fn from_rows(rows: &'a [&'a [T]], col: usize, width: usize, height: usize) -> Result<Self> {
        Self::check_table(PaddedRows::table(rows, col), width, height)
    }

    fn check_table(rows: PaddedRows<'a, T>, width: usize, height: usize) -> Result<Self> {
        let covered = width
            .checked_add(2 * PC_WIENER_FILTER_TAP_RADIUS)
            .zip(height.checked_add(2 * PC_WIENER_FILTER_TAP_RADIUS))
            .is_some_and(|(padded_width, padded_rows)| rows.covers(padded_rows, padded_width));
        if !covered {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener padded source rows",
            });
        }
        Ok(Self { rows })
    }

    fn validate_cover(&self, width: usize, height: usize) -> Result<()> {
        Self::check_table(self.rows, width, height).map(drop)
    }

    fn sample(&self, x: isize, y: isize, width: usize) -> Result<T> {
        const R: isize = PC_WIENER_FILTER_TAP_RADIUS as isize;
        let index = |value: isize| {
            usize::try_from(value + R).map_err(|_| ReconError::ArithmeticOverflow {
                context: "PC-Wiener padded tap offset",
            })
        };
        let (row, col) = (index(y)?, index(x)?);
        self.rows
            .row(row, width + 2 * PC_WIENER_FILTER_TAP_RADIUS)
            .and_then(|row| row.get(col))
            .copied()
            .ok_or(ReconError::PcWienerInvalidBounds {
                field: "PC-Wiener padded source rows",
            })
    }
}

struct PcWienerPaddedFilterSetup {
    sample_count: usize,
    filters: &'static [[i32; 13]; PC_WIENER_FULL_CLASSES],
    classes: &'static [LaneClass; PC_WIENER_FULL_CLASSES],
    max_sample: u16,
    /// Whether the cell-lane kernel applies: 4x4 subclass cells over a source
    /// inside the active bit depth.
    lanes: bool,
}

fn prepare_pc_wiener_padded_filter<T: ReconSample>(
    output_len: usize,
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<PcWienerPaddedFilterSetup> {
    validate_sample_type::<T>(params.bit_depth)?;
    let (sample_count, filters) = validate_pc_wiener_filter(output_len, params)?;
    let classes = PC_WIENER_CLASSES.get(params.filter_set_index).ok_or(
        ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter set index",
        },
    )?;
    source.validate_cover(params.width, params.height)?;

    let max_sample = params.bit_depth.max_sample();
    let padded_width = params.width + 2 * PC_WIENER_FILTER_TAP_RADIUS;
    let padded_rows = params.height + 2 * PC_WIENER_FILTER_TAP_RADIUS;
    let lanes = params.subclass_block_size == PC_WIENER_BLOCK_SIZE
        && (T::MAX_VALUE <= max_sample
            || (0..padded_rows).all(|row| {
                source
                    .rows
                    .row(row, padded_width)
                    .is_some_and(|samples| !crate::workspace::samples_exceed(samples, max_sample))
            }));
    Ok(PcWienerPaddedFilterSetup {
        sample_count,
        filters,
        classes,
        max_sample,
        lanes,
    })
}

/// Applies the fixed AV2 § 7.20.4 PC-Wiener filter from a padded pre-resolved
/// source.
///
/// Identical filter math and output as [`pc_wiener_filter_block`]; the § 7.20.2
/// source-sample process is resolved by the caller into `source` instead of a
/// per-tap callback. The up-front [`PcWienerPaddedSource::from_rows`] fits check
/// proves every tap read in range, so the tap loop is pure index addition off
/// each output sample's padded-window base.
///
/// # Errors
/// Returns the same typed [`ReconError`] values as [`pc_wiener_filter_block`],
/// including source samples outside the active bit-depth range.
#[inline(never)]
pub fn pc_wiener_filter_block_padded<T: ReconSample>(
    output: &mut [T],
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<()> {
    let setup = prepare_pc_wiener_padded_filter(output.len(), params, source)?;
    if !setup.lanes {
        return pc_wiener_filter_block(output, params, |x, y| source.sample(x, y, params.width));
    }

    if let Some(destination) = T::u16_slice_mut(output) {
        return filter_pc_wiener_padded(destination, params, source.rows, T::u16_slice, &setup);
    }
    let destination = T::u8_slice_mut(output).ok_or(ReconError::SampleTypeUnsupportedBitDepth {
        sample_type: T::TYPE_NAME,
        bit_depth: params.bit_depth,
    })?;
    filter_pc_wiener_padded(destination, params, source.rows, T::u8_slice, &setup)
}

/// [`pc_wiener_filter_block_padded`] into an 8-bit `output` from any source
/// sample type.
///
/// # Errors
/// As [`pc_wiener_filter_block_padded`], plus a non-8-bit `params.bit_depth`.
#[inline(never)]
pub fn pc_wiener_filter_block_padded_u8_into<T: ReconSample>(
    output: &mut [u8],
    params: &PcWienerFilter<'_>,
    source: &PcWienerPaddedSource<'_, T>,
) -> Result<()> {
    validate_sample_type::<u8>(params.bit_depth)?;
    let setup = prepare_pc_wiener_padded_filter(output.len(), params, source)?;
    if !setup.lanes {
        let mut filtered = vec![T::default(); setup.sample_count];
        let packed = PcWienerFilter {
            output_stride: params.width,
            ..*params
        };
        pc_wiener_filter_block(&mut filtered, &packed, |x, y| {
            source.sample(x, y, params.width)
        })?;
        let rows = output.chunks_mut(params.output_stride);
        for (output, filtered) in rows.zip(filtered.chunks(params.width)) {
            for (output, sample) in output.iter_mut().zip(filtered) {
                *output = sample.to_u16() as u8;
            }
        }
        return Ok(());
    }
    if T::u8_slice(&[]).is_some() {
        filter_pc_wiener_padded(output, params, source.rows, T::u8_slice, &setup)
    } else {
        filter_pc_wiener_padded(output, params, source.rows, T::u16_slice, &setup)
    }
}

/// Filters a cell row in column groups of up to four 4x4 cells, each lane with
/// its cell's taps; a 32-column chunk whose whole tap reach in the group holds
/// one value takes it, as every `Pc_Wiener_Filters` row sums to zero (checked
/// at compile time below).
fn filter_pc_wiener_padded<
    'a,
    S,
    T: ReconSample + LumaSimdSource + 'a,
    O: LumaSimdOutput + ByteLaneOutput,
>(
    destination: &mut [O],
    params: &PcWienerFilter<'_>,
    source: PaddedRows<'a, S>,
    cast: impl Fn(&'a [S]) -> Option<&'a [T]>,
    setup: &PcWienerPaddedFilterSetup,
) -> Result<()> {
    const R: usize = PC_WIENER_FILTER_TAP_RADIUS;
    let padded_width = params.width + 2 * R;
    let subclass_cols = params.width.div_ceil(PC_WIENER_BLOCK_SIZE);
    let rows_error = || ReconError::PcWienerInvalidBounds {
        field: "PC-Wiener padded source rows",
    };
    let mut flat = FlatChunks::<R>::new();
    let mut window = [&[][..]; PC_WIENER_BLOCK_SIZE + 2 * R];
    #[cfg(target_arch = "x86_64")]
    let bytes = fearless_simd::Level::baseline()
        .as_avx2()
        .filter(|_| setup.max_sample == 255)
        .zip(bytes::BYTE_CLASSES.get(params.filter_set_index))
        .and_then(|(t, classes)| {
            let row = |index| source.row(index, padded_width).and_then(&cast);
            let staged = bytes::stage(row, padded_width, params.height + 2 * R)?;
            Some((t, classes, staged))
        });
    #[cfg(target_arch = "x86_64")]
    let mut byte_window = [&[0; bytes::STAGE_STRIDE]; PC_WIENER_BLOCK_SIZE + 2 * R];
    let mut row = 0;
    while row < params.height {
        let group = PC_WIENER_BLOCK_SIZE.min(params.height - row);
        if row % PC_WIENER_BLOCK_SIZE == 0 {
            for (dy, slot) in window.iter_mut().enumerate() {
                *slot = source
                    .row(row + dy.min(group + 2 * R - 1), padded_width)
                    .and_then(&cast)
                    .ok_or_else(rows_error)?;
            }
            #[cfg(target_arch = "x86_64")]
            if let Some((_, _, staged)) = &bytes {
                for (dy, slot) in byte_window.iter_mut().enumerate() {
                    *slot = staged
                        .get(row + dy.min(group + 2 * R - 1))
                        .ok_or_else(rows_error)?;
                }
            }
            flat.update(&window[..group + 2 * R], row, params.width);
        }
        let group_rows = row..row + group;
        let subclass_row = row / PC_WIENER_BLOCK_SIZE;
        let row_subclasses =
            &params.subclasses[subclass_row * subclass_cols..(subclass_row + 1) * subclass_cols];
        let mut c0 = 0usize;
        let (mut end, mut run) = flat.split(0, params.width);
        while c0 < params.width {
            if c0 == end
                && let Some((value, run_end)) = run
            {
                for row in group_rows.clone() {
                    let output = &mut destination[row * params.output_stride..];
                    output[c0..run_end].fill(O::from_u16(value.min(setup.max_sample)));
                }
                c0 = run_end;
                (end, run) = flat.split(c0, params.width);
                continue;
            }
            #[cfg(target_arch = "x86_64")]
            if let Some((t, classes, _)) = &bytes {
                c0 = bytes::filter_groups(
                    *t,
                    &byte_window,
                    group,
                    (c0, end),
                    (row_subclasses, classes),
                    (
                        &mut destination[row * params.output_stride..],
                        params.output_stride,
                    ),
                );
            }
            let full = (end - c0) / 16;
            let tail = full > 0
                && !(end - c0).is_multiple_of(16)
                && end.is_multiple_of(PC_WIENER_BLOCK_SIZE);
            let mut groups = (0..full)
                .map(|group| c0 + 16 * group)
                .chain(tail.then(|| end - 16))
                .peekable();
            let cells_at = |col: usize| {
                row_subclasses
                    .get(col / PC_WIENER_BLOCK_SIZE..)
                    .and_then(<[u8]>::first_chunk::<4>)
                    .copied()
            };
            while let Some(group_col) = groups.next() {
                let cells = cells_at(group_col).ok_or_else(rows_error)?;
                let uniform = cells == [cells[0]; 4];
                let mut run_end = group_col + 16;
                while groups.peek() == Some(&run_end)
                    && cells_at(run_end).is_some_and(|next| {
                        if uniform {
                            next == cells
                        } else {
                            next != [next[0]; 4]
                        }
                    })
                {
                    groups.next();
                    run_end += 16;
                }
                let classes = if uniform {
                    lr_madd::Classes::Uniform(
                        &setup.classes[usize::from(cells[0]) % PC_WIENER_FULL_CLASSES],
                    )
                } else {
                    lr_madd::Classes::Cells {
                        cells: row_subclasses
                            .get(group_col / PC_WIENER_BLOCK_SIZE..run_end / PC_WIENER_BLOCK_SIZE)
                            .ok_or_else(rows_error)?,
                        table: setup.classes,
                    }
                };
                if !lr_madd::filter_run::<PcTaps, T, O, PC_WIENER_WINDOW_ROWS, 22>(
                    &window,
                    group,
                    (group_col, run_end),
                    classes,
                    (
                        &mut destination[row * params.output_stride..],
                        params.output_stride,
                    ),
                    setup.max_sample,
                ) {
                    return Err(rows_error());
                }
            }
            let col = if tail { end } else { c0 + 16 * full };
            for row in group_rows.clone() {
                let output = &mut destination[row * params.output_stride..][..params.width];
                let rows = window[row % PC_WIENER_BLOCK_SIZE..]
                    .first_chunk::<PC_WIENER_WINDOW_ROWS>()
                    .ok_or_else(rows_error)?;
                for (x, slot) in output.iter_mut().enumerate().take(end).skip(col) {
                    let coeffs =
                        &setup.filters[usize::from(row_subclasses[x / PC_WIENER_BLOCK_SIZE])];
                    let tap = |dy: isize, dx: isize| {
                        i32::from(
                            rows[R.wrapping_add_signed(dy)][(x + R).wrapping_add_signed(dx)]
                                .to_u16(),
                        )
                    };
                    let center = tap(0, 0);
                    let mut sum = (center << PC_WIENER_PREC_BITS) + center * coeffs[12];
                    for (&(dy, dx), &coeff) in PC_WIENER_CONFIG.iter().zip(coeffs) {
                        sum += (tap(dy, dx) + tap(-dy, -dx)) * coeff;
                    }
                    *slot = O::from_u16(
                        round2_i32(sum, PC_WIENER_PREC_BITS).clamp(0, i32::from(setup.max_sample))
                            as u16,
                    );
                }
            }
            c0 = end;
            (end, run) = flat.split(c0, params.width);
        }
        row = group_rows.end;
    }
    Ok(())
}

/// Every `Pc_Wiener_Filters` row sums to zero over its 25 taps, so a window
/// that holds one value filters to that value.
const _: () = {
    let mut index = 0;
    while index < PC_WIENER_FILTERS.len() * PC_WIENER_FULL_CLASSES {
        let taps =
            &PC_WIENER_FILTERS[index / PC_WIENER_FULL_CLASSES][index % PC_WIENER_FULL_CLASSES];
        let mut sum = taps[12];
        let mut tap = 0;
        while tap < 12 {
            sum += 2 * taps[tap];
            tap += 1;
        }
        assert!(sum == 0);
        index += 1;
    }
};

/// Padded source rows one § 7.20.4 PC-Wiener output row reads.
const PC_WIENER_WINDOW_ROWS: usize = 2 * PC_WIENER_FILTER_TAP_RADIUS + 1;

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
    if usize::from(
        params.subclasses[..subclass_count]
            .iter()
            .fold(0, |max, &subclass| max.max(subclass)),
    ) >= filters.len()
    {
        return Err(ReconError::PcWienerInvalidBounds {
            field: "PC-Wiener filter index",
        });
    }
    Ok((sample_count, filters))
}

fn pc_wiener_subclass_at(params: &PcWienerFilter<'_>, row: usize, col: usize) -> usize {
    let subclass_cols = params.width.div_ceil(params.subclass_block_size);
    usize::from(
        params.subclasses
            [(row / params.subclass_block_size) * subclass_cols + col / params.subclass_block_size],
    )
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
    let mut input = 0;
    for (feature, (&value, &offset)) in features.iter().zip(qval_offsets).enumerate() {
        let adjusted = value.wrapping_add(offset);
        let rounded = adjusted
            .wrapping_add(1 << (PC_WIENER_PREC_FEATURE - 1))
            .wrapping_add(adjusted >> 31)
            >> PC_WIENER_PREC_FEATURE;
        input += (rounded.clamp(0, 255) >> 5) << (9 - 3 * feature);
    }
    input as u16
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

#[cfg(target_arch = "x86_64")]
mod bytes;
mod cells16;
mod tx_skip;
use crate::lr_madd::{self, LaneClass};
use cells16::{PC_WIENER_CLASSES, PcTaps};
