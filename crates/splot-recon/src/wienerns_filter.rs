// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.3 non-separable Wiener filter sample math.
//!
//! This module implements the scheduler-free luma portion of the AV2 § 7.20.3
//! non-separable Wiener filter process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-3)).
//! The caller resolves the § 7.20.2 source-sample process, frame/restoration-unit
//! traversal, frame-vs-unit coefficient selection, and any § 7.20.4
//! pixel-classified Wiener subclass mapping. Temporal/reference filter state and
//! runtime decode wiring stay outside this primitive.
//!
//! Feature tracking: `RECON-WIENERNS-FILTER-PRIMITIVE`.

use std::ops::Range;
use std::simd::{Simd, ToBytes, cmp::SimdOrd, num::SimdInt, num::SimdUint, simd_swizzle};

use crate::PlaneId;
use crate::intra_dc_math::validate_sample_type;
use crate::loop_restoration::PaddedRows;
use crate::math::round2_i32;
use crate::workspace::u16_samples_exceed;
use crate::{BitDepth, ReconError, ReconSample, Result};

#[path = "wienerns_flat.rs"]
mod flat;
pub(crate) use flat::FlatChunks;
use flat::LumaFlatGroup;

/// AV2 § 3 `WIENER_NS_PREC_BITS`, used by § 7.20.3 for the accumulator scale.
const WIENER_NS_PREC_BITS: u32 = 7;

/// Number of luma Wiener NS coefficient slots (`WIENER_NS_LUMA_COEFFS`) consumed
/// by AV2 § 7.20.3 `Wiener_Ns_Config_Y`.
pub const WIENER_NS_LUMA_COEFFS: usize = 16;

/// Number of luma Wiener NS taps (`WIENER_NS_TAPS_Y`) in AV2 § 7.20.3.
pub const WIENER_NS_LUMA_TAPS: usize = 32;

/// Maximum absolute § 7.20.3 `Wiener_Ns_Config_Y` tap offset in either axis.
///
/// Callers that pre-resolve § 7.20.2 source samples may materialize a window
/// extending this many samples beyond the output block on every side; the
/// filter never reads farther.
pub const WIENER_NS_LUMA_TAP_RADIUS: usize = 4;

/// AV2 § 7.20.3 `Wiener_Ns_Config_Y`: `(dy, dx, coeff_index)`.
const WIENER_NS_CONFIG_Y: [(isize, isize, usize); WIENER_NS_LUMA_TAPS] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 1),
    (0, -1, 1),
    (2, 0, 2),
    (-2, 0, 2),
    (0, 2, 3),
    (0, -2, 3),
    (1, 1, 4),
    (-1, -1, 4),
    (-1, 1, 5),
    (1, -1, 5),
    (2, 1, 6),
    (-2, -1, 6),
    (2, -1, 7),
    (-2, 1, 7),
    (1, 2, 8),
    (-1, -2, 8),
    (1, -2, 9),
    (-1, 2, 9),
    (3, 0, 10),
    (-3, 0, 10),
    (0, 3, 11),
    (0, -3, 11),
    (4, 0, 12),
    (-4, 0, 12),
    (0, 4, 13),
    (0, -4, 13),
    (3, 3, 14),
    (-3, -3, 14),
    (3, -3, 15),
    (-3, 3, 15),
];

/// First tap `(dy, dx, coeff_index)` of each § 7.20.3 `Wiener_Ns_Config_Y`
/// symmetric pair; the partner tap is `(-dy, -dx, coeff_index)`.
pub const WIENER_NS_CONFIG_Y_PAIRS: [(isize, isize, usize); WIENER_NS_LUMA_COEFFS] = {
    let mut pairs = [(0isize, 0isize, 0usize); WIENER_NS_LUMA_COEFFS];
    let mut j = 0;
    while j < WIENER_NS_LUMA_COEFFS {
        pairs[j] = WIENER_NS_CONFIG_Y[2 * j];
        j += 1;
    }
    pairs
};

/// Caller-resolved parameters for AV2 § 7.20.3 luma Wiener NS filtering.
///
/// `width` and `height` describe the output block in samples. `output_stride`
/// is the distance in samples between adjacent output rows. `coeffs_by_class`
/// supplies one or more caller-resolved luma coefficient classes indexed by the
/// optional `subclasses` map. When `subclasses` is `None`, every output sample
/// uses class `0`; otherwise the first `width * height` entries are read in
/// row-major order. The caller is responsible for deriving those classes from
/// `FilterClass` / `SubclassLookup` when frame-level classified filters are used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WienerNsLumaFilter<'a> {
    /// Output block width in samples.
    pub width: usize,
    /// Output block height in samples.
    pub height: usize,
    /// Distance in samples between adjacent output rows.
    pub output_stride: usize,
    /// Active decoded bit depth used for source validation and `Clip1`.
    pub bit_depth: BitDepth,
    /// Caller-resolved luma coefficient classes.
    pub coeffs_by_class: &'a [[i16; WIENER_NS_LUMA_COEFFS]],
    /// Optional row-major per-output-sample subclass map.
    pub subclasses: Option<&'a [usize]>,
}

/// Reusable working storage for padded luma Wiener NS filtering.
#[derive(Debug, Default)]
pub struct WienerNsLumaScratch<T> {
    clean_rows: Vec<bool>,
    filtered: Vec<T>,
    prepared_classes: Vec<PreparedLumaClass>,
}

#[derive(Clone, Copy, Debug)]
struct PreparedLumaClass {
    center_scale: i32,
    coeffs: [i16; WIENER_NS_LUMA_COEFFS],
    nonzero: u16,
    /// The coefficients as `pmaddwd` pairs.
    madd_pairs: [u32; MADD_PAIRS],
}

/// `pmaddwd` pairs over the 16 symmetric tap sums.
const MADD_PAIRS: usize = WIENER_NS_LUMA_COEFFS / 2;

/// Padded source rows one § 7.20.3 luma output row reads.
const LUMA_WINDOW_ROWS: usize = 2 * WIENER_NS_LUMA_TAP_RADIUS + 1;

#[derive(Clone, Copy)]
enum LumaSubclassLayout<'a> {
    Uniform,
    Samples(&'a [usize]),
    Cells { values: &'a [u8], cols: usize },
}

struct PreparedLumaFilter {
    max_sample: u16,
    direct: bool,
}

/// Applies AV2 § 7.20.3 luma non-separable Wiener filtering to a block.
///
/// `source_sample(x, y)` is called with block-relative coordinates for the
/// center sample and each § 7.20.3 tap. The callback must implement the caller's
/// selected source-sample behavior, including § 7.20.2 clipping/stripe handling
/// and any frame-coordinate offset. The function writes the filtered block into
/// `output` using `params.output_stride`; any samples outside the block but
/// inside the strided buffer are left unchanged.
///
/// # Errors
///
/// Returns typed [`ReconError`] values for unsupported sample storage, zero block
/// dimensions, output shape errors, missing coefficient classes, too-short or
/// out-of-range subclass maps, and source samples outside the active bit-depth
/// range. The caller output is not modified unless all validation and filtering
/// succeeds.
#[inline]
pub fn wiener_ns_filter_luma_block<T, F>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    mut source_sample: F,
) -> Result<()>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    validate_sample_type::<T>(params.bit_depth)?;
    let sample_count = validate_luma_params(output.len(), params)?;
    validate_subclasses(params, sample_count)?;

    let max_sample = params.bit_depth.max_sample();
    let mut filtered = Vec::with_capacity(sample_count);
    for r in 0..params.height {
        for c in 0..params.width {
            let sample_index = r * params.width + c;
            let coeffs = coeffs_for_sample(params, sample_index);
            filtered.push(filter_sample(coeffs, c, r, max_sample, &mut source_sample)?);
        }
    }

    for row_index in 0..params.height {
        let src_start = row_index * params.width;
        let src_end = src_start + params.width;
        let dst_start = row_index * params.output_stride;
        let dst_end = dst_start + params.width;
        // splot-copy-ok: publish fail-atomic Wiener NS scratch row into caller output
        output[dst_start..dst_end].copy_from_slice(&filtered[src_start..src_end]);
    }

    Ok(())
}

/// § 7.20.2-pre-resolved source samples for [`wiener_ns_filter_luma_block_padded`].
///
/// Row-major samples covering the output block extended by
/// [`WIENER_NS_LUMA_TAP_RADIUS`] on every side: index `0` is the sample at
/// block-relative `(-WIENER_NS_LUMA_TAP_RADIUS, -WIENER_NS_LUMA_TAP_RADIUS)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WienerNsLumaPaddedSource<'a, T> {
    rows: PaddedRows<'a, T>,
}

impl<'a, T: ReconSample> WienerNsLumaPaddedSource<'a, T> {
    /// Wraps a padded source buffer for a `width` x `height` output block.
    ///
    /// `stride` is the distance in samples between adjacent padded rows.
    ///
    /// # Errors
    /// Returns typed [`ReconError`] values when the stride or length cannot
    /// cover the block plus the § 7.20.3 luma tap reach.
    pub fn new(samples: &'a [T], stride: usize, width: usize, height: usize) -> Result<Self> {
        let padded_width = width.checked_add(2 * WIENER_NS_LUMA_TAP_RADIUS).ok_or(
            ReconError::ArithmeticOverflow {
                context: "Wiener NS padded source width",
            },
        )?;
        if stride < padded_width {
            return Err(ReconError::WienerNsFilterOutputStrideTooSmall {
                stride_samples: stride,
                width: padded_width,
            });
        }
        let required = height
            .checked_add(2 * WIENER_NS_LUMA_TAP_RADIUS)
            .and_then(|rows| rows.checked_sub(1))
            .and_then(|prefix_rows| prefix_rows.checked_mul(stride))
            .and_then(|prefix| prefix.checked_add(padded_width))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS padded source length",
            })?;
        if samples.len() < required {
            return Err(ReconError::WienerNsFilterOutputTooSmall {
                expected: required,
                actual: samples.len(),
            });
        }
        Ok(Self {
            rows: PaddedRows::Strided { samples, stride },
        })
    }

    /// Wraps padded rows: padded row `i` starts at column `col` of
    /// `rows[i]`. Filtering checks the sample range as for [`Self::new`].
    ///
    /// # Errors
    /// Returns a typed error when a row cannot cover the block plus the
    /// § 7.20.3 luma tap reach.
    pub fn from_rows(rows: &'a [&'a [T]], col: usize, width: usize, height: usize) -> Result<Self> {
        Self::check_table(PaddedRows::table(rows, col), width, height)
    }

    fn check_table(rows: PaddedRows<'a, T>, width: usize, height: usize) -> Result<Self> {
        let padded_width = width.checked_add(2 * WIENER_NS_LUMA_TAP_RADIUS);
        let padded_rows = height.checked_add(2 * WIENER_NS_LUMA_TAP_RADIUS);
        let (Some(padded_width), Some(padded_rows)) = (padded_width, padded_rows) else {
            return Err(ReconError::ArithmeticOverflow {
                context: "Wiener NS padded source rows",
            });
        };
        if !rows.covers(padded_rows, padded_width) {
            return Err(ReconError::WienerNsFilterOutputTooSmall {
                expected: padded_width,
                actual: 0,
            });
        }
        Ok(Self { rows })
    }

    fn validate_cover(&self, width: usize, height: usize) -> Result<()> {
        match self.rows {
            PaddedRows::Strided { samples, stride } => {
                Self::new(samples, stride, width, height).map(drop)
            }
            rows @ PaddedRows::Table { .. } => Self::check_table(rows, width, height).map(drop),
        }
    }
}

/// Applies AV2 § 7.20.3 luma non-separable Wiener filtering from a padded
/// pre-resolved source.
///
/// Identical filter math and output as [`wiener_ns_filter_luma_block`]; the
/// § 7.20.2 source-sample process is resolved by the caller into `source`
/// instead of a per-tap callback. Tap positions are precomputed relative to
/// the padded block origin — `(dy + radius) * stride + dx + radius` is
/// non-negative for every config tap — so the tap loop is pure index addition
/// off each output sample's padded-row base.
///
/// # Errors
/// Returns the same typed [`ReconError`] values as
/// [`wiener_ns_filter_luma_block`], including source samples outside the
/// active bit-depth range.
#[inline]
pub fn wiener_ns_filter_luma_block_padded<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
) -> Result<()> {
    let mut scratch = WienerNsLumaScratch::default();
    wiener_ns_filter_luma_block_padded_into(output, params, source, &mut scratch)
}

/// Applies padded luma Wiener NS filtering with reusable working storage.
///
/// `scratch` retains its allocations between calls, and `output` is not
/// modified unless filtering succeeds.
///
/// # Errors
/// Returns the same errors as [`wiener_ns_filter_luma_block_padded`], plus a
/// typed allocation error when the scratch buffers cannot reserve enough
/// storage.
#[inline]
pub fn wiener_ns_filter_luma_block_padded_into<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    let subclasses = params
        .subclasses
        .map_or(LumaSubclassLayout::Uniform, LumaSubclassLayout::Samples);
    wiener_ns_filter_luma_block_padded_layout_into(output, params, source, subclasses, scratch)
}

/// Applies padded luma Wiener NS filtering with one subclass per 4x4 cell.
///
/// This is equivalent to expanding each cell subclass across its covered
/// output samples before calling [`wiener_ns_filter_luma_block_padded_into`].
/// Edge cells cover the remaining partial width or height.
///
/// # Errors
/// Returns the same errors as [`wiener_ns_filter_luma_block_padded_into`], plus
/// a subclass-map length error when `cell_subclasses` does not cover the block.
#[inline]
pub fn wiener_ns_filter_luma_block_padded_cells_into<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    cell_subclasses: &[u8],
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    wiener_ns_filter_luma_block_padded_layout_into(
        output,
        params,
        source,
        LumaSubclassLayout::Cells {
            values: cell_subclasses,
            cols: params.width.div_ceil(4),
        },
        scratch,
    )
}

/// Applies padded luma Wiener NS filtering directly into strided `u16` storage.
///
/// Both supported source storage types share the same SIMD arithmetic and may
/// write an exact rectangle within a wider destination. Output geometry,
/// source geometry, subclass selection, and every fallible allocation are
/// validated before the first destination write.
///
/// # Errors
/// Returns the same parameter, source-range, output-shape, subclass, and
/// allocation errors as [`wiener_ns_filter_luma_block_padded_into`].
#[inline]
pub fn wiener_ns_filter_luma_block_padded_u16_into<T: ReconSample>(
    output: &mut [u16],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    let subclasses = params
        .subclasses
        .map_or(LumaSubclassLayout::Uniform, LumaSubclassLayout::Samples);
    wiener_ns_filter_luma_block_padded_layout_u16_into(output, params, source, subclasses, scratch)
}

/// Applies padded luma Wiener NS filtering with one subclass per 4x4 cell
/// directly into strided `u16` storage.
///
/// # Errors
/// Returns the same errors as
/// [`wiener_ns_filter_luma_block_padded_cells_into`].
#[inline]
pub fn wiener_ns_filter_luma_block_padded_cells_u16_into<T: ReconSample>(
    output: &mut [u16],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    cell_subclasses: &[u8],
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    wiener_ns_filter_luma_block_padded_layout_u16_into(
        output,
        params,
        source,
        LumaSubclassLayout::Cells {
            values: cell_subclasses,
            cols: params.width.div_ceil(4),
        },
        scratch,
    )
}

/// Applies padded luma Wiener NS filtering directly into strided `u8` storage.
///
/// Output geometry, source geometry, subclass selection, and every fallible
/// allocation are validated before the first destination write.
///
/// # Errors
/// Returns the same parameter, source-range, output-shape, subclass, and
/// allocation errors as [`wiener_ns_filter_luma_block_padded_into`].
#[inline]
pub fn wiener_ns_filter_luma_block_padded_u8_into<T: ReconSample>(
    output: &mut [u8],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    let subclasses = params
        .subclasses
        .map_or(LumaSubclassLayout::Uniform, LumaSubclassLayout::Samples);
    wiener_ns_filter_luma_block_padded_layout_u8_into(output, params, source, subclasses, scratch)
}

/// Applies padded luma Wiener NS filtering with one subclass per 4x4 cell
/// directly into strided `u8` storage.
///
/// # Errors
/// Returns the same errors as
/// [`wiener_ns_filter_luma_block_padded_cells_into`].
#[inline]
pub fn wiener_ns_filter_luma_block_padded_cells_u8_into<T: ReconSample>(
    output: &mut [u8],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    cell_subclasses: &[u8],
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    wiener_ns_filter_luma_block_padded_layout_u8_into(
        output,
        params,
        source,
        LumaSubclassLayout::Cells {
            values: cell_subclasses,
            cols: params.width.div_ceil(4),
        },
        scratch,
    )
}

fn wiener_ns_filter_luma_block_padded_layout_into<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    subclasses: LumaSubclassLayout<'_>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    let context = prepare_luma_padded(output.len(), params, source, subclasses, scratch)?;
    if context.direct {
        return filter_padded_luma_rows_in_range(
            output,
            params.output_stride,
            source.rows,
            0..params.height,
            params,
            &scratch.prepared_classes,
            subclasses,
            context.max_sample,
        );
    }
    filter_luma_rows_to_scratch(params, source, subclasses, scratch, &context)?;
    for row_index in 0..params.height {
        let src_start = row_index * params.width;
        let src_end = src_start + params.width;
        let dst_start = row_index * params.output_stride;
        let dst_end = dst_start + params.width;
        // splot-copy-ok: publish fail-atomic Wiener NS scratch row into caller output
        output[dst_start..dst_end].copy_from_slice(&scratch.filtered[src_start..src_end]);
    }

    Ok(())
}

fn prepare_luma_padded<T: ReconSample>(
    output_len: usize,
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    subclasses: LumaSubclassLayout<'_>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<PreparedLumaFilter> {
    scratch.clean_rows.clear();
    scratch.filtered.clear();
    scratch.prepared_classes.clear();
    validate_sample_type::<T>(params.bit_depth)?;
    let sample_count = validate_luma_params(output_len, params)?;
    validate_subclass_layout(params, sample_count, subclasses)?;
    source.validate_cover(params.width, params.height)?;

    let max_sample = params.bit_depth.max_sample();
    let padded_width = params.width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let padded_rows = params.height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    scratch
        .clean_rows
        .try_reserve_exact(padded_rows)
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "Wiener NS clean-row scratch",
        })?;
    scratch
        .prepared_classes
        .try_reserve_exact(params.coeffs_by_class.len())
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "Wiener NS prepared-class scratch",
        })?;
    scratch
        .prepared_classes
        .extend(params.coeffs_by_class.iter().map(|&coeffs| {
            let center_scale = (1 << WIENER_NS_PREC_BITS)
                - 2 * coeffs.iter().map(|&coeff| i32::from(coeff)).sum::<i32>();
            PreparedLumaClass {
                center_scale,
                coeffs,
                nonzero: coeffs
                    .iter()
                    .rev()
                    .fold(0, |mask, &coeff| (mask << 1) | u16::from(coeff != 0)),
                madd_pairs: core::array::from_fn(|pair| {
                    u32::from(coeffs[2 * pair] as u16)
                        | u32::from(coeffs[2 * pair + 1] as u16) << 16
                }),
            }
        }));
    for row_index in 0..padded_rows {
        let row = padded_row(source.rows, row_index, padded_width)?;
        scratch
            .clean_rows
            .push(T::u16_slice(row).is_none_or(|samples| !u16_samples_exceed(samples, max_sample)));
    }

    let direct = scratch.clean_rows.iter().all(|&clean| clean);
    if !direct {
        scratch
            .filtered
            .try_reserve_exact(sample_count)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "Wiener NS filtered scratch",
            })?;
        scratch.filtered.resize(sample_count, T::default());
    }
    Ok(PreparedLumaFilter { max_sample, direct })
}

fn wiener_ns_filter_luma_block_padded_layout_u16_into<T: ReconSample>(
    output: &mut [u16],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    subclasses: LumaSubclassLayout<'_>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    if let Some(output) = T::from_u16_slice_mut(output) {
        return wiener_ns_filter_luma_block_padded_layout_into(
            output, params, source, subclasses, scratch,
        );
    }

    let context = prepare_luma_padded(output.len(), params, source, subclasses, scratch)?;
    filter_padded_luma_rows_simd(
        output,
        params.output_stride,
        source.rows,
        T::u8_slice,
        0..params.height,
        params,
        &scratch.prepared_classes,
        subclasses,
        context.max_sample,
    )
}

fn wiener_ns_filter_luma_block_padded_layout_u8_into<T: ReconSample>(
    output: &mut [u8],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    subclasses: LumaSubclassLayout<'_>,
    scratch: &mut WienerNsLumaScratch<T>,
) -> Result<()> {
    validate_sample_type::<u8>(params.bit_depth)?;
    let context = prepare_luma_padded(output.len(), params, source, subclasses, scratch)?;
    if !context.direct {
        filter_luma_rows_to_scratch(params, source, subclasses, scratch, &context)?;
        for row_index in 0..params.height {
            let src_start = row_index * params.width;
            let dst_start = row_index * params.output_stride;
            for (dst, src) in output[dst_start..dst_start + params.width]
                .iter_mut()
                .zip(&scratch.filtered[src_start..src_start + params.width])
            {
                *dst = src.to_u16() as u8;
            }
        }
        return Ok(());
    }
    macro_rules! filter_source {
        ($cast:expr) => {
            filter_padded_luma_rows_simd(
                output,
                params.output_stride,
                source.rows,
                $cast,
                0..params.height,
                params,
                &scratch.prepared_classes,
                subclasses,
                context.max_sample,
            )
        };
    }
    if T::u8_slice(&[]).is_some() {
        filter_source!(T::u8_slice)
    } else if T::u16_slice(&[]).is_some() {
        filter_source!(T::u16_slice)
    } else {
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth: params.bit_depth,
        })
    }
}

fn filter_luma_rows_to_scratch<T: ReconSample>(
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
    subclasses: LumaSubclassLayout<'_>,
    scratch: &mut WienerNsLumaScratch<T>,
    context: &PreparedLumaFilter,
) -> Result<()> {
    for r in 0..params.height {
        let filtered = &mut scratch.filtered[r * params.width..(r + 1) * params.width];
        let window_in_range = scratch
            .clean_rows
            .get(r..r + 2 * WIENER_NS_LUMA_TAP_RADIUS + 1)
            .is_some_and(|rows| rows.iter().all(|&clean| clean));
        if window_in_range {
            filter_padded_luma_rows_in_range(
                filtered,
                params.width,
                source.rows,
                r..r + 1,
                params,
                &scratch.prepared_classes,
                subclasses,
                context.max_sample,
            )?;
        } else {
            filter_padded_luma_row_validated(
                filtered,
                source.rows,
                r,
                params,
                subclasses,
                context.max_sample,
            )?;
        }
    }
    Ok(())
}

fn padded_row<T>(rows: PaddedRows<'_, T>, row_index: usize, padded_width: usize) -> Result<&[T]> {
    rows.row(row_index, padded_width)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: padded_width,
            actual: 0,
        })
}

/// Filters output `rows` whose full tap windows are known in range; row `r`
/// lands at `(r - rows.start) * output_stride` in `output`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_rows_in_range<T: ReconSample>(
    output: &mut [T],
    output_stride: usize,
    source: PaddedRows<'_, T>,
    rows: Range<usize>,
    params: &WienerNsLumaFilter<'_>,
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    if let Some(output) = T::u16_slice_mut(output) {
        return filter_padded_luma_rows_simd(
            output,
            output_stride,
            source,
            T::u16_slice,
            rows,
            params,
            prepared_classes,
            subclasses,
            max_sample,
        );
    }
    if let Some(output) = T::u8_slice_mut(output) {
        return filter_padded_luma_rows_simd(
            output,
            output_stride,
            source,
            T::u8_slice,
            rows,
            params,
            prepared_classes,
            subclasses,
            max_sample,
        );
    }
    Err(ReconError::SampleTypeUnsupportedBitDepth {
        sample_type: T::TYPE_NAME,
        bit_depth: params.bit_depth,
    })
}

fn for_each_luma_segment(
    r: usize,
    width: usize,
    subclasses: LumaSubclassLayout<'_>,
    mut filter: impl FnMut(usize, usize, usize) -> Result<()>,
) -> Result<()> {
    let row_start = r.checked_mul(width).ok_or(ReconError::ArithmeticOverflow {
        context: "Wiener NS luma filter row start",
    })?;
    match subclasses {
        LumaSubclassLayout::Uniform => filter(0, width, 0)?,
        LumaSubclassLayout::Samples(subclasses) => {
            let row_subclasses = subclasses
                .get(row_start..row_start + width)
                .ok_or_else(|| luma_segment_error(width))?;
            let mut segment_start = 0usize;
            while segment_start < width {
                let subclass = row_subclasses[segment_start];
                let mut segment_end = segment_start + 1;
                while segment_end < width && row_subclasses[segment_end] == subclass {
                    segment_end += 1;
                }
                filter(segment_start, segment_end - segment_start, subclass)?;
                segment_start = segment_end;
            }
        }
        LumaSubclassLayout::Cells { values, cols } => {
            let cell_row = (r / 4) * cols;
            let mut cell_start = 0;
            while cell_start < cols {
                let subclass = values[cell_row + cell_start];
                let mut cell_end = cell_start + 1;
                while cell_end < cols && values[cell_row + cell_end] == subclass {
                    cell_end += 1;
                }
                let segment_start = cell_start * 4;
                let segment_end = (cell_end * 4).min(width);
                filter(
                    segment_start,
                    segment_end - segment_start,
                    usize::from(subclass),
                )?;
                cell_start = cell_end;
            }
        }
    }
    Ok(())
}

/// Filters output `rows` like [`filter_padded_luma_rows_in_range`], finding the
/// subclass segments once per 4x4 cell row instead of once per row.
///
/// `cast` views each source row as the SIMD lane source type.
#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_rows_simd<'a, S, T: LumaSimdSource + 'a, O: LumaSimdOutput>(
    output: &mut [O],
    output_stride: usize,
    source_rows: PaddedRows<'a, S>,
    cast: impl Fn(&'a [S]) -> Option<&'a [T]>,
    rows: Range<usize>,
    params: &WienerNsLumaFilter<'_>,
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    let padded_width = params.width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let mut flat = FlatChunks::new();
    let mut first = rows.start;
    while first < rows.end {
        let end = match subclasses {
            LumaSubclassLayout::Samples(_) => first + 1,
            _ => ((first / 4 + 1) * 4).min(rows.end),
        };
        let mut source: [&[T]; LUMA_WINDOW_ROWS + 3] = [&[]; LUMA_WINDOW_ROWS + 3];
        for (dy, row) in source
            .iter_mut()
            .take(end - first + 2 * WIENER_NS_LUMA_TAP_RADIUS)
            .enumerate()
        {
            *row = cast(padded_row(source_rows, first + dy, padded_width)?)
                .ok_or_else(|| luma_segment_error(params.width))?;
        }
        let window = &source[..end - first + 2 * WIENER_NS_LUMA_TAP_RADIUS];
        flat.update(window, first, params.width);
        if flat.any() {
            let group = LumaFlatGroup {
                window,
                rows: (first, first - rows.start),
                output_stride,
                max_sample,
                flat: &flat,
            };
            group.filter(output, params, prepared_classes, subclasses)?;
            first = end;
            continue;
        }
        let cells_done = filter_luma_cell_groups(
            output,
            output_stride,
            &source,
            (first, end, rows.start),
            params.width,
            prepared_classes,
            subclasses,
            max_sample,
        )?;
        for_each_luma_segment(
            first,
            params.width,
            subclasses,
            |segment_start, len, subclass| {
                let (segment_start, len) = if segment_start + len <= cells_done {
                    return Ok(());
                } else if segment_start < cells_done {
                    (cells_done, segment_start + len - cells_done)
                } else {
                    (segment_start, len)
                };
                let class = prepared_classes
                    .get(subclass)
                    .ok_or_else(|| luma_segment_error(params.width))?;
                for r in first..end {
                    let start = (r - rows.start) * output_stride + segment_start;
                    let (Some(window), Some(filtered)) = (
                        source[r - first..].first_chunk::<LUMA_WINDOW_ROWS>(),
                        output.get_mut(start..start + len),
                    ) else {
                        return Err(luma_segment_error(params.width));
                    };
                    filter_luma_segment_simd::<_, _, 0>(
                        filtered,
                        window,
                        segment_start,
                        class,
                        max_sample,
                    );
                }
                Ok(())
            },
        )?;
        first = end;
    }
    Ok(())
}

/// Filters the rows `first..end` of the 4x4 cell row they share in groups of
/// four cells, 16 columns at a time, with each cell's class weights; returns
/// the columns done. Sample-subclass layouts are left to the segment filter.
#[allow(clippy::too_many_arguments)]
fn filter_luma_cell_groups<T: LumaSimdSource, O: LumaSimdOutput>(
    output: &mut [O],
    output_stride: usize,
    source: &[&[T]; LUMA_WINDOW_ROWS + 3],
    (first, end, origin): (usize, usize, usize),
    width: usize,
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<usize> {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let cell_class = |cell: usize| -> Option<&[u32; MADD_PAIRS]> {
        let class = match subclasses {
            LumaSubclassLayout::Uniform => 0,
            LumaSubclassLayout::Cells { values, cols } => {
                usize::from(*values.get((first / 4) * cols + cell)?)
            }
            LumaSubclassLayout::Samples(_) => return None,
        };
        prepared_classes.get(class).map(|class| &class.madd_pairs)
    };
    let mut x = 0;
    while x + 16 <= width {
        let classes: [Option<&[u32; MADD_PAIRS]>; 4] =
            core::array::from_fn(|cell| cell_class(x / 4 + cell));
        let [Some(c0), Some(c1), Some(c2), Some(c3)] = classes else {
            break;
        };
        let weights: [[Simd<i16, 16>; 2]; MADD_PAIRS] = core::array::from_fn(|pair| {
            let halves = |a: u32, b: u32| {
                Simd::<i16, 16>::from_ne_bytes(
                    Simd::<u32, 8>::from_array([a, a, a, a, b, b, b, b]).to_ne_bytes(),
                )
            };
            [halves(c0[pair], c2[pair]), halves(c1[pair], c3[pair])]
        });
        for r in first..end {
            let start = (r - origin) * output_stride + x;
            let (Some(window), Some(filtered)) = (
                source[r - first..].first_chunk::<LUMA_WINDOW_ROWS>(),
                output.get_mut(start..start + 16),
            ) else {
                return Err(luma_segment_error(width));
            };
            let window = window.map(|row| row.get(x..x + 16 + 2 * R).unwrap_or(&[]));
            if window.iter().any(|row| row.len() != 16 + 2 * R) {
                return Err(luma_segment_error(width));
            }
            filter_luma_cells16(filtered, &window, &weights, max_sample);
        }
        x += 16;
    }
    Ok(x)
}

/// Lanes `0..4, 8..12` and `4..8, 12..16` of two vectors interleaved, as an
/// in-lane AVX2 unpack gives them.
#[inline]
fn interleave_pairs(a: Simd<i16, 16>, b: Simd<i16, 16>) -> [Simd<i16, 16>; 2] {
    [
        simd_swizzle!(
            a,
            b,
            [0, 16, 1, 17, 2, 18, 3, 19, 8, 24, 9, 25, 10, 26, 11, 27]
        ),
        simd_swizzle!(
            a,
            b,
            [4, 20, 5, 21, 6, 22, 7, 23, 12, 28, 13, 29, 14, 30, 15, 31]
        ),
    ]
}

/// Sums of adjacent products, written so that LLVM emits one `pmaddwd`.
#[inline]
fn multiply_add_pairs(x: Simd<i16, 16>, w: Simd<i16, 16>) -> Simd<i32, 8> {
    let product = x.cast::<i32>() * w.cast::<i32>();
    simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15])
}

/// Symmetric tap sum `J` of [`filter_luma_cells16`] less twice the center.
#[allow(clippy::inline_always, reason = "measured Wiener NS luma hot path")]
#[inline(always)]
fn luma_term<const J: usize, T: LumaSimdSource>(
    window: &[&[T]; LUMA_WINDOW_ROWS],
    center: Simd<i16, 16>,
) -> Simd<i16, 16> {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let (dy, dx, _) = WIENER_NS_CONFIG_Y_PAIRS[J];
    let plus = T::load::<16>(window[R.wrapping_add_signed(dy)], R.wrapping_add_signed(dx));
    let minus = T::load::<16>(
        window[R.wrapping_add_signed(-dy)],
        R.wrapping_add_signed(-dx),
    );
    plus.cast::<i16>() + minus.cast::<i16>() - center - center
}

/// Filters 16 samples of four 4-sample cells: `weights` holds per pair the
/// interleaved weights of cells 0 and 2, then of cells 1 and 3, matching
/// `interleave_pairs`. The center scale `128 - 2 * sum(coeffs)` folds into
/// the taps: the sum is `center << 7` plus each coefficient times its tap sum
/// less twice the center, whose 12 bits and the coefficients' fit `pmaddwd`.
#[allow(clippy::inline_always, reason = "measured Wiener NS luma hot path")]
#[inline(always)]
fn filter_luma_cells16<T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    window: &[&[T]; LUMA_WINDOW_ROWS],
    weights: &[[Simd<i16, 16>; 2]; MADD_PAIRS],
    max_sample: u16,
) {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let center = T::load::<16>(window[R], R).cast::<i16>();
    let scaled = (center.cast::<i32>() << WIENER_NS_PREC_BITS as i32)
        + Simd::splat(1 << (WIENER_NS_PREC_BITS - 1));
    let mut sums = [
        simd_swizzle!(scaled, [0, 1, 2, 3, 8, 9, 10, 11]),
        simd_swizzle!(scaled, [4, 5, 6, 7, 12, 13, 14, 15]),
    ];
    macro_rules! pairs {
        ($($pair:literal)*) => {$({
            let halves = interleave_pairs(
                luma_term::<{ 2 * $pair }, T>(window, center),
                luma_term::<{ 2 * $pair + 1 }, T>(window, center),
            );
            for ((sum, half), weight) in sums.iter_mut().zip(halves).zip(&weights[$pair]) {
                *sum += multiply_add_pairs(half, *weight);
            }
        })*};
    }
    pairs!(0 1 2 3 4 5 6 7);
    let [low, high] = sums.map(|sum| sum >> Simd::splat(WIENER_NS_PREC_BITS as i32));
    let values = simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    )
    .simd_max(Simd::splat(0))
    .simd_min(Simd::splat(i32::from(max_sample)))
    .cast::<u16>();
    O::write(filtered, values);
}

trait LumaSimdOutput: Copy {
    fn from_u16(value: u16) -> Self;

    fn write<const LANES: usize>(output: &mut [Self], values: Simd<u16, LANES>);
}

impl LumaSimdOutput for u16 {
    #[inline]
    fn from_u16(value: u16) -> Self {
        value
    }

    #[inline]
    fn write<const LANES: usize>(output: &mut [Self], values: Simd<u16, LANES>) {
        output[..LANES].copy_from_slice(&values.to_array()); // splot-copy-ok: publish Wiener NS SIMD lanes
    }
}

impl LumaSimdOutput for u8 {
    #[inline]
    fn from_u16(value: u16) -> Self {
        value as u8
    }

    #[inline]
    fn write<const LANES: usize>(output: &mut [Self], values: Simd<u16, LANES>) {
        output[..LANES].copy_from_slice(&values.cast::<u8>().to_array()); // splot-copy-ok: publish Wiener NS SIMD lanes
    }
}

pub(crate) trait LumaSimdSource: Copy {
    fn load<const LANES: usize>(samples: &[Self], start: usize) -> Simd<u16, LANES>;

    fn scalar(samples: &[Self], index: usize) -> u16;
}

impl LumaSimdSource for u16 {
    #[inline]
    fn load<const LANES: usize>(samples: &[Self], start: usize) -> Simd<u16, LANES> {
        Simd::from_slice(&samples[start..])
    }

    #[inline]
    fn scalar(samples: &[Self], index: usize) -> u16 {
        samples[index]
    }
}

impl LumaSimdSource for u8 {
    #[inline]
    fn load<const LANES: usize>(samples: &[Self], start: usize) -> Simd<u16, LANES> {
        Simd::<u8, LANES>::from_slice(&samples[start..]).cast()
    }

    #[inline]
    fn scalar(samples: &[Self], index: usize) -> u16 {
        u16::from(samples[index])
    }
}

/// Filters output columns `c0..c0 + filtered.len()` of one row from its nine
/// padded source `rows`.
///
/// Every row is first sliced to the segment's exact reach, so each lane
/// group's reslice is proven in bounds. A center scale outside `i16`,
/// reachable only with coefficients outside the § 5.20.2.1
/// `Wiener_Ns_Taps_Min` ranges, takes the scalar loop. Each `COPY` has one
/// caller, so the flat-chunk path keeps the other caller's inlined loop.
fn filter_luma_segment_simd<T: LumaSimdSource, O: LumaSimdOutput, const COPY: u8>(
    filtered: &mut [O],
    rows: &[&[T]; LUMA_WINDOW_ROWS],
    c0: usize,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let len = filtered.len();
    let rows = &rows.map(|row| &row[c0..c0 + len + 2 * R]);
    let mut col = 0usize;
    if let Ok(center_scale) = i16::try_from(class.center_scale) {
        macro_rules! filter_lane_group {
            ($lanes:literal) => {
                while col + $lanes <= len {
                    filter_luma_lanes::<$lanes, T, O>(
                        &mut filtered[col..],
                        rows,
                        col,
                        center_scale,
                        class,
                        max_sample,
                    );
                    col += $lanes;
                }
            };
        }
        filter_lane_group!(32);
        filter_lane_group!(16);
        filter_lane_group!(8);
        filter_lane_group!(4);
    }
    for (offset, slot) in filtered[col..].iter_mut().enumerate() {
        let x = col + offset + R;
        let tap = |dy: isize, dx: isize| {
            i32::from(T::scalar(
                rows[R.wrapping_add_signed(dy)],
                x.wrapping_add_signed(dx),
            ))
        };
        let mut sum = class.center_scale * tap(0, 0);
        for (&(dy, dx, _), &coeff) in WIENER_NS_CONFIG_Y_PAIRS.iter().zip(&class.coeffs) {
            sum += i32::from(coeff) * (tap(dy, dx) + tap(-dy, -dx));
        }
        *slot = O::from_u16(
            round2_i32(sum, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample)) as u16,
        );
    }
}

/// One `LANES`-wide group of [`filter_luma_segment_simd`] at padded column
/// `col`.
///
/// Each row is sliced once to the group's `LANES + 2 * WIENER_NS_LUMA_TAP_RADIUS`
/// reach, so the unrolled constant tap offsets need no further bounds checks.
/// `plus + minus` stays in `i16` lanes because § 6 Table 6.3 caps `BitDepth` at
/// 10 and this path only runs over rows validated against `max_sample`, so
/// every multiply factor, the center scale included, is 16-bit and the
/// widening folds into the multiply. That bounds `|sum|` near `2^30`, so
/// `Round2` as `(sum + 64) >> 7` cannot overflow and equals [`round2_i32`].
#[allow(clippy::inline_always, reason = "measured Wiener NS luma hot path")]
#[inline(always)]
fn filter_luma_lanes<const LANES: usize, T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    rows: &[&[T]; LUMA_WINDOW_ROWS],
    col: usize,
    center_scale: i16,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let window = rows.map(|row| &row[col..col + LANES + 2 * R]);
    let mut sum = T::load::<LANES>(window[R], R).cast::<i16>().cast::<i32>()
        * Simd::<i16, LANES>::splat(center_scale).cast::<i32>();
    let coeffs = Simd::from_array(class.coeffs);
    macro_rules! tap_pairs {
        ($($j:literal)*) => {
            $(add_luma_tap_pair::<LANES, $j, T>(&mut sum, &window, class.nonzero, coeffs);)*
        };
    }
    tap_pairs!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
    let shift = WIENER_NS_PREC_BITS as i32;
    let values = ((sum + Simd::splat(1 << (shift - 1))) >> Simd::splat(shift))
        .simd_clamp(Simd::splat(0), Simd::splat(i32::from(u16::MAX)))
        .cast::<u16>()
        .simd_min(Simd::splat(max_sample));
    O::write(filtered, values);
}

/// Adds § 7.20.3 symmetric tap pair `J` of a [`filter_luma_lanes`] group,
/// skipping a zero coefficient by its bit in [`PreparedLumaClass::nonzero`].
#[allow(clippy::inline_always, reason = "measured Wiener NS luma hot path")]
#[inline(always)]
fn add_luma_tap_pair<const LANES: usize, const J: usize, T: LumaSimdSource>(
    sum: &mut Simd<i32, LANES>,
    window: &[&[T]; LUMA_WINDOW_ROWS],
    nonzero: u16,
    coeffs: Simd<i16, WIENER_NS_LUMA_COEFFS>,
) {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    if nonzero & (1 << J) == 0 {
        return;
    }
    let coeff = coeffs[J];
    let (dy, dx, _) = WIENER_NS_CONFIG_Y_PAIRS[J];
    let plus = T::load::<LANES>(window[R.wrapping_add_signed(dy)], R.wrapping_add_signed(dx));
    let minus = T::load::<LANES>(
        window[R.wrapping_add_signed(-dy)],
        R.wrapping_add_signed(-dx),
    );
    *sum += (plus.cast::<i16>() + minus.cast::<i16>()).cast::<i32>()
        * Simd::<i16, LANES>::splat(coeff).cast::<i32>();
}

const fn luma_segment_error(width: usize) -> ReconError {
    ReconError::BufferLengthMismatch {
        expected: width,
        actual: 0,
    }
}

/// Per-sample § 7.20.3 filtering for rows whose padded window may hold
/// out-of-range samples, preserving the original read-order error identity.
#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_row_validated<T: ReconSample>(
    filtered: &mut [T],
    source: PaddedRows<'_, T>,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let padded_width = params.width + 2 * R;
    for (c, slot) in filtered.iter_mut().enumerate().take(params.width) {
        let subclass = subclass_for_position(subclasses, params.width, r, c);
        let coeffs = &params.coeffs_by_class[subclass];
        let sample = |dy: isize, dx: isize| {
            padded_row(source, (r + R).wrapping_add_signed(dy), padded_width).and_then(|row| {
                validated_padded_sample(
                    row,
                    (c + R).wrapping_add_signed(dx),
                    c as isize + dx,
                    r as isize + dy,
                    max_sample,
                )
            })
        };
        let m = sample(0, 0)?;
        let mut s = i32::from(m) << WIENER_NS_PREC_BITS;
        for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_Y {
            let tap = sample(dy, dx)?;
            let diff = i32::from(tap) - i32::from(m);
            s += diff * i32::from(coeffs[coeff_index]);
        }
        let value = round2_i32(s, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample));
        *slot = T::try_from_u16(value as u16)?;
    }
    Ok(())
}

fn validated_padded_sample<T: ReconSample>(
    samples: &[T],
    index: usize,
    x: isize,
    y: isize,
    max_sample: u16,
) -> Result<u16> {
    let value = samples
        .get(index)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: index.saturating_add(1),
            actual: samples.len(),
        })?
        .to_u16();
    if value > max_sample {
        return Err(ReconError::WienerNsFilterSourceSampleOutOfRange {
            x,
            y,
            value,
            max: max_sample,
        });
    }
    Ok(value)
}

fn validate_luma_params(output_len: usize, params: &WienerNsLumaFilter<'_>) -> Result<usize> {
    if params.width == 0 {
        return Err(ReconError::ZeroDimension {
            field: "wiener NS luma filter width",
        });
    }
    if params.height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "wiener NS luma filter height",
        });
    }
    if params.output_stride < params.width {
        return Err(ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: params.output_stride,
            width: params.width,
        });
    }
    if params.coeffs_by_class.is_empty() {
        return Err(ReconError::WienerNsFilterMissingClasses);
    }

    let sample_count =
        params
            .width
            .checked_mul(params.height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS luma filter sample count",
            })?;
    let expected = (params.height - 1)
        .checked_mul(params.output_stride)
        .and_then(|prefix| prefix.checked_add(params.width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS luma filter output length",
        })?;
    if output_len < expected {
        return Err(ReconError::WienerNsFilterOutputTooSmall {
            expected,
            actual: output_len,
        });
    }
    Ok(sample_count)
}

fn validate_subclasses(params: &WienerNsLumaFilter<'_>, sample_count: usize) -> Result<()> {
    let Some(subclasses) = params.subclasses else {
        return Ok(());
    };
    if subclasses.len() < sample_count {
        return Err(ReconError::WienerNsFilterSubclassMapTooShort {
            expected: sample_count,
            actual: subclasses.len(),
        });
    }
    for (sample_index, &subclass) in subclasses.iter().take(sample_count).enumerate() {
        if subclass >= params.coeffs_by_class.len() {
            return Err(ReconError::WienerNsFilterSubclassOutOfRange {
                sample_index,
                subclass,
                classes: params.coeffs_by_class.len(),
            });
        }
    }
    Ok(())
}

fn validate_subclass_layout(
    params: &WienerNsLumaFilter<'_>,
    sample_count: usize,
    subclasses: LumaSubclassLayout<'_>,
) -> Result<()> {
    let LumaSubclassLayout::Cells { values, cols } = subclasses else {
        return validate_subclasses(params, sample_count);
    };
    let rows = params.height.div_ceil(4);
    let expected = cols
        .checked_mul(rows)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS luma cell subclass count",
        })?;
    if values.len() < expected {
        return Err(ReconError::WienerNsFilterSubclassMapTooShort {
            expected,
            actual: values.len(),
        });
    }
    let classes = params.coeffs_by_class.len();
    if usize::from(
        values[..expected]
            .iter()
            .fold(0, |max, &value| max.max(value)),
    ) < classes
    {
        return Ok(());
    }
    for (cell_index, subclass) in values[..expected]
        .iter()
        .map(|&value| usize::from(value))
        .enumerate()
    {
        if subclass >= classes {
            let cell_row = cell_index / cols;
            let cell_col = cell_index % cols;
            return Err(ReconError::WienerNsFilterSubclassOutOfRange {
                sample_index: cell_row * 4 * params.width + cell_col * 4,
                subclass,
                classes: params.coeffs_by_class.len(),
            });
        }
    }
    Ok(())
}

fn subclass_for_position(
    subclasses: LumaSubclassLayout<'_>,
    width: usize,
    row: usize,
    col: usize,
) -> usize {
    match subclasses {
        LumaSubclassLayout::Uniform => 0,
        LumaSubclassLayout::Samples(values) => values[row * width + col],
        LumaSubclassLayout::Cells { values, cols } => {
            usize::from(values[(row / 4) * cols + col / 4])
        }
    }
}

fn coeffs_for_sample<'a>(
    params: &'a WienerNsLumaFilter<'_>,
    sample_index: usize,
) -> &'a [i16; WIENER_NS_LUMA_COEFFS] {
    let subclass = match params.subclasses {
        Some(subclasses) => subclasses[sample_index],
        None => 0,
    };
    &params.coeffs_by_class[subclass]
}

#[allow(clippy::many_single_char_names)]
fn filter_sample<T, F>(
    coeffs: &[i16; WIENER_NS_LUMA_COEFFS],
    c: usize,
    r: usize,
    max_sample: u16,
    source_sample: &mut F,
) -> Result<T>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    let x = c as isize;
    let y = r as isize;
    let m = validated_source_sample(source_sample, x, y, max_sample)?;

    let mut s = i32::from(m) << WIENER_NS_PREC_BITS;
    for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_Y {
        let tap = validated_source_sample(source_sample, x + dx, y + dy, max_sample)?;
        let diff = i32::from(tap) - i32::from(m);
        s += diff * i32::from(coeffs[coeff_index]);
    }
    let value = round2_i32(s, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample));
    T::try_from_u16(value as u16)
}

pub(super) fn validated_source_sample<T, F>(
    source_sample: &mut F,
    x: isize,
    y: isize,
    max_sample: u16,
) -> Result<u16>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    let value = source_sample(x, y).to_u16();
    if value > max_sample {
        return Err(ReconError::WienerNsFilterSourceSampleOutOfRange {
            x,
            y,
            value,
            max: max_sample,
        });
    }
    Ok(value)
}

#[cfg(test)]
#[path = "wienerns_filter_tests.rs"]
mod tests;
