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

use std::simd::{Simd, cmp::SimdOrd, num::SimdInt, num::SimdUint};

use crate::PlaneId;
use crate::intra_dc_math::validate_sample_type;
use crate::math::round2_i32;
use crate::workspace::u16_samples_exceed;
use crate::{BitDepth, ReconError, ReconSample, Result};

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
const WIENER_NS_CONFIG_Y_PAIRS: [(isize, isize, usize); WIENER_NS_LUMA_COEFFS] = {
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
    acc: Vec<i32>,
    prepared_classes: Vec<PreparedLumaClass>,
}

#[derive(Clone, Copy, Debug)]
struct PreparedLumaClass {
    center_scale: i32,
    pairs: [(usize, usize, usize, usize, i32); WIENER_NS_LUMA_COEFFS],
    flat_pairs: [(usize, usize, i16); WIENER_NS_LUMA_COEFFS],
    pair_count: usize,
}

#[derive(Clone, Copy)]
enum LumaSubclassLayout<'a> {
    Uniform,
    Samples(&'a [usize]),
    Cells { values: &'a [usize], cols: usize },
}

struct PreparedLumaFilter {
    tap_offsets: [usize; WIENER_NS_LUMA_TAPS],
    center_offset: usize,
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
    samples: &'a [T],
    stride: usize,
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
        Ok(Self { samples, stride })
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
    cell_subclasses: &[usize],
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
    cell_subclasses: &[usize],
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
    cell_subclasses: &[usize],
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

fn padded_luma_offsets(stride: usize) -> Result<([usize; WIENER_NS_LUMA_TAPS], usize)> {
    let center = WIENER_NS_LUMA_TAP_RADIUS
        .checked_mul(stride)
        .and_then(|row| row.checked_add(WIENER_NS_LUMA_TAP_RADIUS))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS padded source stride",
        })?;
    let mut taps = [0usize; WIENER_NS_LUMA_TAPS];
    for (offset, &(dy, dx, _)) in taps.iter_mut().zip(&WIENER_NS_CONFIG_Y) {
        let row = usize::try_from(dy + WIENER_NS_LUMA_TAP_RADIUS as isize)
            .map_err(|_| ReconError::ArithmeticOverflow {
                context: "Wiener NS padded tap offset",
            })?
            .checked_mul(stride)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS padded tap offset",
            })?;
        *offset = usize::try_from(dx + WIENER_NS_LUMA_TAP_RADIUS as isize)
            .ok()
            .and_then(|col| row.checked_add(col))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS padded tap offset",
            })?;
    }
    Ok((taps, center))
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
        for r in 0..params.height {
            let filtered = &mut output[r * params.output_stride..][..params.width];
            filter_padded_luma_row_in_range(
                filtered,
                &mut scratch.acc,
                source.samples,
                source.stride,
                r,
                params,
                &scratch.prepared_classes,
                subclasses,
                context.max_sample,
            )?;
        }
        return Ok(());
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
    scratch.acc.clear();
    scratch.prepared_classes.clear();
    validate_sample_type::<T>(params.bit_depth)?;
    let sample_count = validate_luma_params(output_len, params)?;
    validate_subclass_layout(params, sample_count, subclasses)?;
    WienerNsLumaPaddedSource::new(source.samples, source.stride, params.width, params.height)?;

    let stride = source.stride;
    let (tap_offsets, center_offset) = padded_luma_offsets(stride)?;
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
    scratch.acc.try_reserve_exact(params.width).map_err(|_| {
        ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "Wiener NS accumulator scratch",
        }
    })?;
    scratch.acc.resize(params.width, 0);
    scratch
        .prepared_classes
        .try_reserve_exact(params.coeffs_by_class.len())
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::Y,
            context: "Wiener NS prepared-class scratch",
        })?;
    scratch
        .prepared_classes
        .extend(params.coeffs_by_class.iter().map(|coeffs| {
            let center_scale = (1 << WIENER_NS_PREC_BITS)
                - 2 * coeffs.iter().map(|&coeff| i32::from(coeff)).sum::<i32>();
            let mut pairs = [(0, 0, 0, 0, 0); WIENER_NS_LUMA_COEFFS];
            let mut flat_pairs = [(0, 0, 0); WIENER_NS_LUMA_COEFFS];
            let mut pair_count = 0;
            for (&(dy, dx, _), &coeff) in WIENER_NS_CONFIG_Y_PAIRS.iter().zip(coeffs) {
                if coeff != 0 {
                    let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
                    pairs[pair_count] = (
                        (dy + radius) as usize,
                        (dx + radius) as usize,
                        (-dy + radius) as usize,
                        (-dx + radius) as usize,
                        i32::from(coeff),
                    );
                    flat_pairs[pair_count] = (
                        (dy + radius) as usize * stride + (dx + radius) as usize,
                        (-dy + radius) as usize * stride + (-dx + radius) as usize,
                        coeff,
                    );
                    pair_count += 1;
                }
            }
            PreparedLumaClass {
                center_scale,
                pairs,
                flat_pairs,
                pair_count,
            }
        }));
    // Scanning the stride gaps too only ever falls back to the per-row scan.
    let region_clean = (padded_rows - 1)
        .checked_mul(stride)
        .and_then(|prefix| prefix.checked_add(padded_width))
        .and_then(|span| source.samples.get(..span))
        .is_some_and(|region| {
            T::u16_slice(region).is_none_or(|samples| !u16_samples_exceed(samples, max_sample))
        });
    if region_clean {
        scratch.clean_rows.resize(padded_rows, true);
    } else {
        for row_index in 0..padded_rows {
            let row = padded_row(source.samples, stride, row_index, padded_width)?;
            scratch.clean_rows.push(
                T::u16_slice(row).is_none_or(|samples| !u16_samples_exceed(samples, max_sample)),
            );
        }
    }

    let direct = region_clean || scratch.clean_rows.iter().all(|&clean| clean);
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
    Ok(PreparedLumaFilter {
        tap_offsets,
        center_offset,
        max_sample,
        direct,
    })
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
    let Some(samples) = T::u8_slice(source.samples) else {
        return Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth: params.bit_depth,
        });
    };
    for r in 0..params.height {
        let output_row = &mut output[r * params.output_stride..][..params.width];
        filter_padded_luma_row_u8_source_u16_output(
            output_row,
            samples,
            source.stride,
            r,
            params,
            &scratch.prepared_classes,
            subclasses,
            context.max_sample,
        )?;
    }
    Ok(())
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
        ($samples:expr) => {
            for r in 0..params.height {
                let output_row = &mut output[r * params.output_stride..][..params.width];
                filter_padded_luma_row_u8_source_u16_output(
                    output_row,
                    $samples,
                    source.stride,
                    r,
                    params,
                    &scratch.prepared_classes,
                    subclasses,
                    context.max_sample,
                )?;
            }
        };
    }
    if let Some(samples) = T::u8_slice(source.samples) {
        filter_source!(samples);
    } else if let Some(samples) = T::u16_slice(source.samples) {
        filter_source!(samples);
    } else {
        return Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth: params.bit_depth,
        });
    }
    Ok(())
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
            filter_padded_luma_row_in_range(
                filtered,
                &mut scratch.acc,
                source.samples,
                source.stride,
                r,
                params,
                &scratch.prepared_classes,
                subclasses,
                context.max_sample,
            )?;
        } else {
            filter_padded_luma_row_validated(
                filtered,
                source.samples,
                source.stride,
                &context.tap_offsets,
                context.center_offset,
                r,
                params,
                subclasses,
                context.max_sample,
            )?;
        }
    }
    Ok(())
}

fn padded_row<T: ReconSample>(
    samples: &[T],
    stride: usize,
    row_index: usize,
    padded_width: usize,
) -> Result<&[T]> {
    let start = row_index
        .checked_mul(stride)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS padded row start",
        })?;
    let end = start
        .checked_add(padded_width)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS padded row end",
        })?;
    samples
        .get(start..end)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: end,
            actual: samples.len(),
        })
}

/// Filters one output row whose full tap window is known in range, as
/// row-slice arithmetic: taps outer, samples inner, over per-subclass
/// segments. The i32 accumulation adds the same § 7.20.3 tap terms in config
/// order, so the result is bit-identical to the per-sample path.
#[inline]
#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_row_in_range<T: ReconSample>(
    filtered: &mut [T],
    acc: &mut [i32],
    samples: &[T],
    stride: usize,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    const RADIUS: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let width = params.width;
    let padded_width = width + 2 * RADIUS;
    let mut rows: [&[T]; 2 * RADIUS + 1] = [&[]; 2 * RADIUS + 1];
    for (dy, row) in rows.iter_mut().enumerate() {
        *row = padded_row(samples, stride, r + dy, padded_width)?;
    }
    let center = rows[RADIUS]
        .get(RADIUS..RADIUS + width)
        .ok_or_else(|| luma_segment_error(width))?;
    let flat = LumaFlatSource {
        samples,
        row_base: r * stride,
        center_offset: RADIUS * stride + RADIUS,
    };

    for_each_luma_segment(r, width, subclasses, |segment_start, len, subclass| {
        filter_padded_luma_segment(
            filtered,
            acc,
            &rows,
            center,
            &flat,
            segment_start,
            len,
            subclass,
            prepared_classes,
            params.width,
            max_sample,
        )
    })?;

    Ok(())
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
                filter(segment_start, segment_end - segment_start, subclass)?;
                cell_start = cell_end;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_row_u8_source_u16_output<T: LumaSimdSource, O: LumaSimdOutput>(
    output: &mut [O],
    samples: &[T],
    stride: usize,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    let flat = LumaFlatSource {
        samples,
        row_base: r * stride,
        center_offset: WIENER_NS_LUMA_TAP_RADIUS * stride + WIENER_NS_LUMA_TAP_RADIUS,
    };
    for_each_luma_segment(
        r,
        params.width,
        subclasses,
        |segment_start, len, subclass| {
            let class = prepared_classes
                .get(subclass)
                .ok_or_else(|| luma_segment_error(params.width))?;
            let filtered = output
                .get_mut(segment_start..segment_start + len)
                .ok_or_else(|| luma_segment_error(params.width))?;
            filter_luma_segment_u8(
                filtered,
                flat.samples,
                flat.row_base + segment_start,
                flat.center_offset,
                class,
                max_sample,
            );
            Ok(())
        },
    )
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn filter_padded_luma_segment<T: ReconSample>(
    filtered: &mut [T],
    acc: &mut [i32],
    rows: &[&[T]; 2 * WIENER_NS_LUMA_TAP_RADIUS + 1],
    center: &[T],
    flat: &LumaFlatSource<'_, T>,
    c0: usize,
    len: usize,
    subclass: usize,
    prepared_classes: &[PreparedLumaClass],
    width: usize,
    max_sample: u16,
) -> Result<()> {
    let class = prepared_classes
        .get(subclass)
        .ok_or_else(|| luma_segment_error(width))?;
    let seg = acc
        .get_mut(c0..c0 + len)
        .ok_or_else(|| luma_segment_error(width))?;
    let center_seg = center
        .get(c0..c0 + len)
        .ok_or_else(|| luma_segment_error(width))?;
    let filtered = filtered
        .get_mut(c0..c0 + len)
        .ok_or_else(|| luma_segment_error(width))?;
    if let Some(samples) = T::u16_slice(flat.samples)
        && let Some(filtered) = T::u16_slice_mut(filtered)
    {
        filter_luma_segment_u16(
            filtered,
            samples,
            flat.row_base + c0,
            flat.center_offset,
            class,
            max_sample,
        );
        return Ok(());
    }
    for (a, &m) in seg.iter_mut().zip(center_seg) {
        *a = class.center_scale * i32::from(m.to_u16());
    }
    for &(plus_row, plus_offset, minus_row, minus_offset, coeff) in &class.pairs[..class.pair_count]
    {
        let plus = tap_segment(rows, c0, len, plus_row, plus_offset, width)?;
        let minus = tap_segment(rows, c0, len, minus_row, minus_offset, width)?;
        for ((a, &tp), &tm) in seg.iter_mut().zip(plus).zip(minus) {
            *a += coeff * (i32::from(tp.to_u16()) + i32::from(tm.to_u16()));
        }
    }
    for (slot, &s) in filtered.iter_mut().zip(seg.iter()) {
        let value = round2_i32(s, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample));
        *slot = T::try_from_u16(value as u16)?;
    }
    Ok(())
}

/// Padded § 7.20.3 source addressed by flat tap offsets off one output row.
///
/// `row_base` is the padded index of the row's first tap column and
/// `center_offset` the flat displacement of its center tap, so a segment
/// sample at column `c` reads `row_base + c + offset` for each
/// [`PreparedLumaClass::flat_pairs`] entry. Every such index stays inside a
/// [`WienerNsLumaPaddedSource`]: the largest offset is
/// `2 * WIENER_NS_LUMA_TAP_RADIUS * stride + 2 * WIENER_NS_LUMA_TAP_RADIUS`,
/// which is exactly the reach the constructor validates.
struct LumaFlatSource<'a, T> {
    samples: &'a [T],
    row_base: usize,
    center_offset: usize,
}

/// Filters one `u16`-storage segment with the § 7.20.3 taps carried in
/// registers, widest lane group first.
///
/// Each lane repeats the accumulator path of [`filter_padded_luma_segment`] in
/// § 7.20.3 config order, so every output sample is bit-identical.
fn filter_luma_segment_u16(
    filtered: &mut [u16],
    samples: &[u16],
    base: usize,
    center_offset: usize,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    filter_luma_segment_simd(filtered, samples, base, center_offset, class, max_sample);
}

fn filter_luma_segment_u8<T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    samples: &[T],
    base: usize,
    center_offset: usize,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    filter_luma_segment_simd(filtered, samples, base, center_offset, class, max_sample);
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

trait LumaSimdSource: Copy {
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

fn filter_luma_segment_simd<T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    samples: &[T],
    base: usize,
    center_offset: usize,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    let pairs = &class.flat_pairs[..class.pair_count];
    let len = filtered.len();
    let mut col = 0usize;
    macro_rules! filter_lane_group {
        ($lanes:literal) => {
            while col + $lanes <= len {
                filter_luma_lanes::<$lanes, T, O>(
                    &mut filtered[col..],
                    samples,
                    base + col,
                    center_offset,
                    pairs,
                    class.center_scale,
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
    for (offset, slot) in filtered[col..].iter_mut().enumerate() {
        let sample = base + col + offset;
        let mut sum = class.center_scale * i32::from(T::scalar(samples, sample + center_offset));
        for &(plus, minus, coeff) in pairs {
            sum += i32::from(coeff)
                * (i32::from(T::scalar(samples, sample + plus))
                    + i32::from(T::scalar(samples, sample + minus)));
        }
        *slot = O::from_u16(
            round2_i32(sum, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample)) as u16,
        );
    }
}

/// One `LANES`-wide group of [`filter_luma_segment_u16`].
///
/// `Round2` is expanded as `(sum >> n) + ((sum >> (n - 1)) & 1)`, which is the
/// definition of [`round2_i32`] for every `i32` and cannot overflow. `plus +
/// minus` stays in `i16` lanes because § 6 Table 6.3 caps `BitDepth` at 10 and
/// this path only runs over rows validated against `max_sample`, so both
/// multiply factors are 16-bit and the widening folds into the accumulate.
#[allow(clippy::inline_always, reason = "measured Wiener NS luma hot path")]
#[inline(always)]
fn filter_luma_lanes<const LANES: usize, T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    samples: &[T],
    base: usize,
    center_offset: usize,
    pairs: &[(usize, usize, i16)],
    center_scale: i32,
    max_sample: u16,
) {
    let mut sum = T::load(samples, base + center_offset).cast::<i32>() * Simd::splat(center_scale);
    for &(plus, minus, coeff) in pairs {
        let plus = T::load(samples, base + plus).cast::<i16>();
        let minus = T::load(samples, base + minus).cast::<i16>();
        sum += (plus + minus).cast::<i32>() * Simd::<i16, LANES>::splat(coeff).cast::<i32>();
    }
    let shift = WIENER_NS_PREC_BITS as i32;
    let rounded = (sum >> Simd::splat(shift)) + ((sum >> Simd::splat(shift - 1)) & Simd::splat(1));
    let values = rounded
        .simd_clamp(Simd::splat(0), Simd::splat(i32::from(max_sample)))
        .cast::<u16>();
    O::write(filtered, values);
}

/// Resolves the padded-row segment for one § 7.20.3 tap `(dy, dx)`, preserving
/// the config-order row-index, offset, and slice bounds checks.
#[inline]
fn tap_segment<'r, T: ReconSample>(
    rows: &[&'r [T]; 2 * WIENER_NS_LUMA_TAP_RADIUS + 1],
    c0: usize,
    len: usize,
    row: usize,
    offset: usize,
    width: usize,
) -> Result<&'r [T]> {
    let row = rows.get(row).ok_or_else(|| luma_segment_error(width))?;
    row.get(c0 + offset..c0 + offset + len)
        .ok_or_else(|| luma_segment_error(width))
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
    samples: &[T],
    stride: usize,
    tap_offsets: &[usize; WIENER_NS_LUMA_TAPS],
    center_offset: usize,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    subclasses: LumaSubclassLayout<'_>,
    max_sample: u16,
) -> Result<()> {
    for (c, slot) in filtered.iter_mut().enumerate().take(params.width) {
        let subclass = subclass_for_position(subclasses, params.width, r, c);
        let coeffs = &params.coeffs_by_class[subclass];
        let base = r * stride + c;
        let m = validated_padded_sample(
            samples,
            base + center_offset,
            c as isize,
            r as isize,
            max_sample,
        )?;
        let mut s = i32::from(m) << WIENER_NS_PREC_BITS;
        for (&offset, &(dy, dx, coeff_index)) in tap_offsets.iter().zip(&WIENER_NS_CONFIG_Y) {
            let tap = validated_padded_sample(
                samples,
                base + offset,
                c as isize + dx,
                r as isize + dy,
                max_sample,
            )?;
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
    for (cell_index, &subclass) in values.iter().take(expected).enumerate() {
        if subclass >= params.coeffs_by_class.len() {
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
        LumaSubclassLayout::Cells { values, cols } => values[(row / 4) * cols + col / 4],
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

fn validated_source_sample<T, F>(
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
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const ZERO: [i16; WIENER_NS_LUMA_COEFFS] = [0; WIENER_NS_LUMA_COEFFS];

    fn padded_from<T: ReconSample>(
        height: usize,
        stride: usize,
        source_at: impl Fn(isize, isize) -> T,
    ) -> Vec<T> {
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let source_at = &source_at;
        (0..height + 2 * radius)
            .flat_map(move |row| {
                (0..stride).map(move |col| {
                    source_at(
                        col as isize - radius as isize,
                        row as isize - radius as isize,
                    )
                })
            })
            .collect()
    }

    /// Covers every lane group and tail of the `u16` SIMD segment kernel
    /// against the per-sample callback reference, over randomized samples,
    /// coefficients, and subclass runs.
    #[test]
    fn padded_u16_lane_groups_match_the_callback_reference() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
            let max = u64::from(bit_depth.max_sample()) + 1;
            for &(width, height) in &[
                (1, 1),
                (3, 2),
                (4, 4),
                (7, 5),
                (16, 3),
                (31, 2),
                (32, 2),
                (37, 3),
                (64, 2),
                (67, 2),
                (100, 2),
                (128, 2),
            ] {
                let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = (0..3)
                    .map(|_| core::array::from_fn(|_| (next() % 65) as i16 - 32))
                    .collect();
                let subclasses: Vec<usize> = (0..width * height)
                    .map(|_| (next() % coeffs.len() as u64) as usize)
                    .collect();
                let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + (next() % 3) as usize;
                let padded: Vec<u16> = (0..(height + 2 * WIENER_NS_LUMA_TAP_RADIUS) * stride)
                    .map(|index| match index % 4 {
                        0 => bit_depth.max_sample(),
                        1 => 0,
                        _ => (next() % max) as u16,
                    })
                    .collect();
                let source_at = |x: isize, y: isize| -> u16 {
                    let radius = WIENER_NS_LUMA_TAP_RADIUS as isize;
                    padded[((y + radius) as usize) * stride + (x + radius) as usize]
                };
                for map in [None, Some(subclasses.as_slice())] {
                    let params = params(width, height, width, bit_depth, &coeffs, map);
                    let mut reference = vec![0u16; width * height];
                    wiener_ns_filter_luma_block(&mut reference, &params, source_at).unwrap();
                    let mut actual = vec![0u16; width * height];
                    let source =
                        WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
                    wiener_ns_filter_luma_block_padded(&mut actual, &params, &source).unwrap();
                    assert_eq!(actual, reference, "{bit_depth:?} {width}x{height}");
                }
            }
        }
    }

    fn packed_and_strided_u16_luma_match<T: ReconSample>(
        bit_depth: BitDepth,
        width: usize,
        height: usize,
        class_count: usize,
    ) {
        let mins = [
            -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
        ];
        let maxs = [39, 39, 17, 17, 15, 15, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
        let coeffs: Vec<_> = (0..class_count)
            .map(|class| if class % 2 == 0 { mins } else { maxs })
            .collect();
        let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + 3;
        let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let source_samples: Vec<T> = (0..source_stride * source_rows)
            .map(|index| {
                T::try_from_u16(
                    ((index * 37 + index / source_stride * 53 + 17)
                        % (usize::from(bit_depth.max_sample()) + 1)) as u16,
                )
                .unwrap()
            })
            .collect();
        let source =
            WienerNsLumaPaddedSource::new(&source_samples, source_stride, width, height).unwrap();
        let cells: Vec<usize> = (0..width.div_ceil(4) * height.div_ceil(4))
            .map(|index| index % class_count)
            .collect();

        let packed_params = params(width, height, width, bit_depth, &coeffs, None);
        let mut packed = vec![T::default(); width * height];
        let mut packed_scratch = WienerNsLumaScratch::default();
        if class_count == 1 {
            wiener_ns_filter_luma_block_padded_into(
                &mut packed,
                &packed_params,
                &source,
                &mut packed_scratch,
            )
            .unwrap();
        } else {
            wiener_ns_filter_luma_block_padded_cells_into(
                &mut packed,
                &packed_params,
                &source,
                &cells,
                &mut packed_scratch,
            )
            .unwrap();
        }

        let output_stride = width + 11;
        let direct_params = params(width, height, output_stride, bit_depth, &coeffs, None);
        let mut direct = vec![u16::MAX; output_stride * height];
        let mut direct_scratch = WienerNsLumaScratch::default();
        if class_count == 1 {
            wiener_ns_filter_luma_block_padded_u16_into(
                &mut direct,
                &direct_params,
                &source,
                &mut direct_scratch,
            )
            .unwrap();
        } else {
            wiener_ns_filter_luma_block_padded_cells_u16_into(
                &mut direct,
                &direct_params,
                &source,
                &cells,
                &mut direct_scratch,
            )
            .unwrap();
        }

        for row in 0..height {
            assert_eq!(
                &direct[row * output_stride..row * output_stride + width],
                &packed[row * width..(row + 1) * width]
                    .iter()
                    .map(|sample| sample.to_u16())
                    .collect::<Vec<_>>(),
                "{} {bit_depth:?} {width}x{height} classes={class_count}",
                T::TYPE_NAME,
            );
            assert!(
                direct[row * output_stride + width..(row + 1) * output_stride]
                    .iter()
                    .all(|&sample| sample == u16::MAX)
            );
        }
    }

    #[test]
    fn packed_and_strided_u16_luma_match_runtime_class_counts_and_shapes() {
        let class_counts = [1, 2, 3, 4, 6, 8, 12, 16];
        let shapes = [
            (1, 1),
            (3, 3),
            (31, 8),
            (32, 56),
            (37, 64),
            (64, 8),
            (67, 56),
            (128, 64),
            (255, 3),
            (256, 56),
        ];
        for class_count in class_counts {
            for &(width, height) in &shapes {
                packed_and_strided_u16_luma_match::<u8>(
                    BitDepth::Eight,
                    width,
                    height,
                    class_count,
                );
                packed_and_strided_u16_luma_match::<u16>(
                    BitDepth::Eight,
                    width,
                    height,
                    class_count,
                );
                packed_and_strided_u16_luma_match::<u16>(BitDepth::Ten, width, height, class_count);
            }
        }
    }

    #[test]
    fn direct_u8_and_u16_staging_match_runtime_class_counts_and_shapes() {
        for class_count in [1, 2, 4, 8, 16] {
            for (width, height) in [(1, 1), (3, 3), (31, 8), (37, 63), (128, 56)] {
                let coeffs: Vec<_> = (0..class_count)
                    .map(|class| {
                        core::array::from_fn(|index| ((class * 11 + index * 7) % 33) as i16 - 16)
                    })
                    .collect();
                let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS + 3;
                let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
                let source_samples: Vec<u8> = (0..source_stride * source_rows)
                    .map(|index| ((index * 37 + index / source_stride * 53 + 17) % 256) as u8)
                    .collect();
                let source =
                    WienerNsLumaPaddedSource::new(&source_samples, source_stride, width, height)
                        .unwrap();
                let source_samples_u16: Vec<u16> = source_samples
                    .iter()
                    .map(|&sample| u16::from(sample))
                    .collect();
                let source_u16 = WienerNsLumaPaddedSource::new(
                    &source_samples_u16,
                    source_stride,
                    width,
                    height,
                )
                .unwrap();
                let cells: Vec<usize> = (0..width.div_ceil(4) * height.div_ceil(4))
                    .map(|index| index % class_count)
                    .collect();
                let output_stride = width + 5;
                let params = params(width, height, output_stride, BitDepth::Eight, &coeffs, None);
                let mut staged = vec![u16::MAX; output_stride * height];
                let mut direct = vec![0xa5; output_stride * height];
                let mut direct_from_u16 = vec![0xa5; output_stride * height];
                if class_count == 1 {
                    wiener_ns_filter_luma_block_padded_u16_into(
                        &mut staged,
                        &params,
                        &source,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                    wiener_ns_filter_luma_block_padded_u8_into(
                        &mut direct,
                        &params,
                        &source,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                    wiener_ns_filter_luma_block_padded_u8_into(
                        &mut direct_from_u16,
                        &params,
                        &source_u16,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                } else {
                    wiener_ns_filter_luma_block_padded_cells_u16_into(
                        &mut staged,
                        &params,
                        &source,
                        &cells,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                    wiener_ns_filter_luma_block_padded_cells_u8_into(
                        &mut direct,
                        &params,
                        &source,
                        &cells,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                    wiener_ns_filter_luma_block_padded_cells_u8_into(
                        &mut direct_from_u16,
                        &params,
                        &source_u16,
                        &cells,
                        &mut WienerNsLumaScratch::default(),
                    )
                    .unwrap();
                }
                for row in 0..height {
                    assert_eq!(
                        &direct[row * output_stride..row * output_stride + width],
                        &staged[row * output_stride..row * output_stride + width]
                            .iter()
                            .map(|&sample| sample as u8)
                            .collect::<Vec<_>>(),
                        "{width}x{height} classes={class_count}",
                    );
                    assert_eq!(
                        &direct_from_u16[row * output_stride..row * output_stride + width],
                        &direct[row * output_stride..row * output_stride + width],
                        "u16 source {width}x{height} classes={class_count}",
                    );
                    assert!(
                        direct[row * output_stride + width..(row + 1) * output_stride]
                            .iter()
                            .all(|&sample| sample == 0xa5)
                    );
                    assert!(
                        direct_from_u16[row * output_stride + width..(row + 1) * output_stride]
                            .iter()
                            .all(|&sample| sample == 0xa5)
                    );
                }
            }
        }
    }

    #[test]
    fn u8_source_strided_u16_simd_has_a_mutation_sensitive_oracle() {
        let width = 32;
        let height = 1;
        let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let mut samples = vec![100u8; source_stride * source_rows];
        let center_row = WIENER_NS_LUMA_TAP_RADIUS;
        samples[(center_row + 1) * source_stride + WIENER_NS_LUMA_TAP_RADIUS] = 120;
        let source = WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
        let mut class = ZERO;
        class[0] = 4;
        let coeffs = [class];
        let output_stride = width + 7;
        let params = params(width, height, output_stride, BitDepth::Eight, &coeffs, None);
        let mut output = vec![u16::MAX; output_stride];

        wiener_ns_filter_luma_block_padded_u16_into(
            &mut output,
            &params,
            &source,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap();

        assert_eq!(output[0], 101);
        assert!(output[1..width].iter().all(|&sample| sample == 100));
        assert!(output[width..].iter().all(|&sample| sample == u16::MAX));

        let mut direct = vec![0xa5; output_stride];
        wiener_ns_filter_luma_block_padded_u8_into(
            &mut direct,
            &params,
            &source,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap();
        assert_eq!(direct[0], 101);
        assert!(direct[1..width].iter().all(|&sample| sample == 100));
        assert!(direct[width..].iter().all(|&sample| sample == 0xa5));
    }

    #[test]
    fn strided_u16_luma_validates_every_input_before_writing() {
        let width = 2;
        let height = 2;
        let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let samples = vec![0u8; source_stride * source_rows];
        let valid_source =
            WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
        let short_source = WienerNsLumaPaddedSource {
            samples: &samples[..samples.len() - 1],
            stride: source_stride,
        };
        let coeffs = [ZERO];
        let short_cells: [usize; 0] = [];

        let mut output = [77u16; 8];
        let invalid_stride_params = params(width, height, 1, BitDepth::Eight, &coeffs, None);
        let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
            &mut output,
            &invalid_stride_params,
            &short_source,
            &short_cells,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::WienerNsFilterOutputStrideTooSmall {
                stride_samples: 1,
                width,
            }
        );
        assert_eq!(output, [77; 8]);

        let params = params(width, height, 4, BitDepth::Eight, &coeffs, None);
        let mut output = [77u16; 5];
        let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
            &mut output,
            &params,
            &short_source,
            &short_cells,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::WienerNsFilterOutputTooSmall {
                expected: 6,
                actual: 5,
            }
        );
        assert_eq!(output, [77; 5]);

        let mut output = [77u16; 8];
        let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
            &mut output,
            &params,
            &valid_source,
            &short_cells,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::WienerNsFilterSubclassMapTooShort {
                expected: 1,
                actual: 0,
            }
        );
        assert_eq!(output, [77; 8]);

        let cells = [0usize];
        let error = wiener_ns_filter_luma_block_padded_cells_u16_into(
            &mut output,
            &params,
            &short_source,
            &cells,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::WienerNsFilterOutputTooSmall {
                expected: source_stride * source_rows,
                actual: samples.len() - 1,
            }
        );
        assert_eq!(output, [77; 8]);
    }

    #[test]
    fn strided_luma_u16_source_errors_are_fail_atomic() {
        let width = 2;
        let height = 2;
        let source_stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let source_rows = height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let mut samples = vec![23u16; source_stride * source_rows];
        let invalid = (source_rows - 1) * source_stride + WIENER_NS_LUMA_TAP_RADIUS;
        samples[invalid] = 1024;
        let source = WienerNsLumaPaddedSource::new(&samples, source_stride, width, height).unwrap();
        let coeffs = [ZERO];
        let ten_bit_params = params(width, height, 4, BitDepth::Ten, &coeffs, None);
        let mut output = [77u16; 8];

        let error = wiener_ns_filter_luma_block_padded_u16_into(
            &mut output,
            &ten_bit_params,
            &source,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();

        assert_eq!(
            error,
            ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: 0,
                y: 5,
                value: 1024,
                max: 1023,
            }
        );
        assert_eq!(output, [77; 8]);

        let mut direct = [0xa5u8; 8];
        let error = wiener_ns_filter_luma_block_padded_u8_into(
            &mut direct,
            &ten_bit_params,
            &source,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            }
        );
        assert_eq!(direct, [0xa5; 8]);

        let mut eight_bit_samples = vec![23u16; source_stride * source_rows];
        let center = WIENER_NS_LUMA_TAP_RADIUS * source_stride + WIENER_NS_LUMA_TAP_RADIUS;
        eight_bit_samples[center] = 256;
        let eight_bit_source =
            WienerNsLumaPaddedSource::new(&eight_bit_samples, source_stride, width, height)
                .unwrap();
        let eight_bit_params = params(width, height, 4, BitDepth::Eight, &coeffs, None);
        let error = wiener_ns_filter_luma_block_padded_u8_into(
            &mut direct,
            &eight_bit_params,
            &eight_bit_source,
            &mut WienerNsLumaScratch::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: 0,
                y: 0,
                value: 256,
                max: 255,
            }
        );
        assert_eq!(direct, [0xa5; 8]);
    }
    #[test]
    fn padded_and_callback_filters_match_bit_exactly() {
        let width = 7;
        let height = 5;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius;
        let mut class_a = ZERO;
        class_a[0] = 4;
        class_a[7] = -3;
        class_a[15] = 9;
        let mut class_b = ZERO;
        class_b[3] = -6;
        class_b[10] = 2;
        let coeffs = [class_a, class_b];
        let subclasses: Vec<usize> = (0..width * height).map(|i| i % 2).collect();
        let source_at =
            |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
        let padded: Vec<u16> = padded_from(height, stride, source_at);
        let params = params(
            width,
            height,
            width + 3,
            BitDepth::Ten,
            &coeffs,
            Some(&subclasses),
        );

        let mut callback_output = vec![0u16; (height - 1) * (width + 3) + width];
        wiener_ns_filter_luma_block(&mut callback_output, &params, source_at).unwrap();

        let mut padded_output = vec![0u16; (height - 1) * (width + 3) + width];
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

        assert_eq!(callback_output, padded_output);
    }

    #[test]
    fn padded_and_callback_filters_match_bit_exactly_eight_bit_wide_block() {
        let width = 23;
        let height = 6;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius + 2;
        let mut class_a = ZERO;
        class_a[1] = 7;
        class_a[12] = -5;
        let mut class_b = ZERO;
        class_b[6] = 11;
        class_b[14] = -2;
        let coeffs = [class_a, class_b];
        let subclasses: Vec<usize> = (0..width * height)
            .map(|i| ((i % width) / 4 + i / width / 4) % 2)
            .collect();
        let source_at =
            |x: isize, y: isize| -> u8 { ((x * 7 + y * 13 + 90).rem_euclid(256)) as u8 };
        let padded: Vec<u8> = padded_from(height, stride, source_at);
        let params = params(
            width,
            height,
            width,
            BitDepth::Eight,
            &coeffs,
            Some(&subclasses),
        );

        let mut callback_output = vec![0u8; width * height];
        wiener_ns_filter_luma_block(&mut callback_output, &params, source_at).unwrap();

        let mut padded_output = vec![0u8; width * height];
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

        assert_eq!(callback_output, padded_output);
    }

    #[test]
    fn cell_subclasses_match_per_sample_expansion() {
        let width = 10;
        let height = 6;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius;
        let mut class_a = ZERO;
        class_a[0] = 7;
        class_a[8] = -3;
        let mut class_b = ZERO;
        class_b[4] = -5;
        class_b[15] = 9;
        let coeffs = [class_a, class_b];
        let cell_subclasses = [0usize, 0, 1, 1, 1, 0];
        let cell_cols = width.div_ceil(4);
        let subclasses: Vec<usize> = (0..height)
            .flat_map(|row| {
                (0..width).map(move |col| cell_subclasses[(row / 4) * cell_cols + col / 4])
            })
            .collect();
        let source_at =
            |x: isize, y: isize| -> u8 { ((x * 11 + y * 19 + 120).rem_euclid(256)) as u8 };
        let padded: Vec<u8> = padded_from(height, stride, source_at);
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let per_sample = params(
            width,
            height,
            width,
            BitDepth::Eight,
            &coeffs,
            Some(&subclasses),
        );
        let per_cell = params(width, height, width, BitDepth::Eight, &coeffs, None);
        let mut expected = vec![0u8; width * height];
        let mut actual = vec![0u8; width * height];
        let mut expected_scratch = WienerNsLumaScratch::default();
        let mut actual_scratch = WienerNsLumaScratch::default();

        wiener_ns_filter_luma_block_padded_into(
            &mut expected,
            &per_sample,
            &source,
            &mut expected_scratch,
        )
        .unwrap();
        wiener_ns_filter_luma_block_padded_cells_into(
            &mut actual,
            &per_cell,
            &source,
            &cell_subclasses,
            &mut actual_scratch,
        )
        .unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn out_of_range_sample_in_unread_window_corner_still_filters() {
        let width = 5;
        let height = 4;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius;
        let source_at =
            |x: isize, y: isize| -> u16 { ((x * 19 + y * 5 + 300).rem_euclid(1024)) as u16 };
        let mut padded: Vec<u16> = padded_from(height, stride, source_at);
        padded[0] = u16::MAX;
        let mut class = ZERO;
        class[0] = 6;
        class[13] = -9;
        let coeffs = [class];
        let params = params(width, height, width, BitDepth::Ten, &coeffs, None);

        let mut expected = vec![0u16; width * height];
        wiener_ns_filter_luma_block(&mut expected, &params, source_at).unwrap();

        let mut padded_output = vec![0u16; width * height];
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        wiener_ns_filter_luma_block_padded(&mut padded_output, &params, &source).unwrap();

        assert_eq!(expected, padded_output);
    }

    #[test]
    fn padded_source_rejects_short_buffer() {
        let padded = vec![0u16; 10];
        let err = WienerNsLumaPaddedSource::new(&padded, 9, 1, 1).unwrap_err();
        assert_eq!(
            err,
            ReconError::WienerNsFilterOutputTooSmall {
                expected: 8 * 9 + 9,
                actual: 10,
            }
        );
    }

    #[test]
    fn padded_source_rejects_narrow_stride() {
        let padded = vec![0u16; 100];
        let err = WienerNsLumaPaddedSource::new(&padded, 8, 1, 1).unwrap_err();
        assert_eq!(
            err,
            ReconError::WienerNsFilterOutputStrideTooSmall {
                stride_samples: 8,
                width: 9,
            }
        );
    }

    #[test]
    fn padded_scratch_recovers_after_source_sample_error() {
        let width = 2;
        let height = 2;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius;
        let mut padded = vec![23u16; stride * (height + 2 * radius)];
        let invalid_sample = (height + 2 * radius - 1) * stride + radius;
        padded[invalid_sample] = 1024;
        let coeffs = [ZERO];
        let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut scratch = WienerNsLumaScratch::default();
        let mut output = [77u16; 4];

        let err =
            wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch)
                .unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: 0,
                y: 5,
                value: 1024,
                max: 1023,
            }
        );
        assert_eq!(output, [77; 4]);

        padded[invalid_sample] = 23;
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch)
            .unwrap();
        assert_eq!(output, [23; 4]);
    }

    #[test]
    fn padded_into_reuses_maximum_restoration_stripe_scratch() {
        let width = 512;
        let height = 64;
        let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let mut padded = vec![91u16; stride * (height + 2 * WIENER_NS_LUMA_TAP_RADIUS)];
        padded[0] = 1024;
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let coeffs = [ZERO];
        let large_params = params(width, height, width, BitDepth::Ten, &coeffs, None);
        let mut scratch = WienerNsLumaScratch::default();
        let mut output = vec![0u16; width * height];

        wiener_ns_filter_luma_block_padded_into(&mut output, &large_params, &source, &mut scratch)
            .unwrap();

        assert!(output.iter().all(|&sample| sample == 91));
        assert!(scratch.filtered.capacity() >= width * height);
        assert!(scratch.acc.capacity() >= width);
        assert!(scratch.clean_rows.capacity() >= height + 2 * WIENER_NS_LUMA_TAP_RADIUS);
        assert!(scratch.prepared_classes.capacity() >= coeffs.len());
        let clean_rows_capacity = scratch.clean_rows.capacity();
        let filtered_ptr = scratch.filtered.as_ptr();
        let acc_ptr = scratch.acc.as_ptr();

        let width = 2;
        let height = 2;
        let stride = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
        let padded = vec![37u16; stride * (height + 2 * WIENER_NS_LUMA_TAP_RADIUS)];
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
        let mut output = [u16::MAX; 4];
        wiener_ns_filter_luma_block_padded_into(&mut output, &params, &source, &mut scratch)
            .unwrap();

        assert_eq!(output, [37; 4]);
        assert_eq!(scratch.clean_rows.capacity(), clean_rows_capacity);
        assert_eq!(scratch.filtered.as_ptr(), filtered_ptr);
        assert_eq!(scratch.acc.as_ptr(), acc_ptr);
    }

    #[test]
    fn config_y_is_symmetric_pairs() {
        for (j, &(dy, dx, coeff_index)) in WIENER_NS_CONFIG_Y_PAIRS.iter().enumerate() {
            assert_eq!(WIENER_NS_CONFIG_Y[2 * j], (dy, dx, coeff_index));
            assert_eq!(WIENER_NS_CONFIG_Y[2 * j + 1], (-dy, -dx, coeff_index));
        }
    }

    #[test]
    fn luma_tap_radius_covers_config_table() {
        let max_offset = WIENER_NS_CONFIG_Y
            .iter()
            .map(|&(dy, dx, _)| dy.unsigned_abs().max(dx.unsigned_abs()))
            .max()
            .unwrap();
        assert_eq!(WIENER_NS_LUMA_TAP_RADIUS, max_offset);
    }
    fn params<'a>(
        width: usize,
        height: usize,
        output_stride: usize,
        bit_depth: BitDepth,
        coeffs_by_class: &'a [[i16; WIENER_NS_LUMA_COEFFS]],
        subclasses: Option<&'a [usize]>,
    ) -> WienerNsLumaFilter<'a> {
        WienerNsLumaFilter {
            width,
            height,
            output_stride,
            bit_depth,
            coeffs_by_class,
            subclasses,
        }
    }

    #[test]
    fn zero_coefficients_copy_source_samples() {
        let coeffs = [ZERO];
        let mut output = [9u8; 6];
        let params = params(2, 2, 3, BitDepth::Eight, &coeffs, None);

        wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
            if (0..=1).contains(&x) && (0..=1).contains(&y) {
                u8::try_from(20 + y * 10 + x).unwrap()
            } else {
                0
            }
        })
        .unwrap();

        assert_eq!(output, [20, 21, 9, 30, 31, 9]);
    }

    #[test]
    fn hand_computed_luma_tap_accumulation() {
        let mut class = ZERO;
        class[0] = 4;
        let coeffs = [class];
        let mut output = [0u8; 1];
        let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);

        wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
            if x == 0 && y == 1 { 120 } else { 100 }
        })
        .unwrap();

        assert_eq!(output, [101]);
    }

    #[test]
    fn subclass_selects_coefficients_per_sample() {
        let mut class_one = ZERO;
        class_one[0] = 4;
        let coeffs = [ZERO, class_one];
        let subclasses = [0, 1];
        let mut output = [0u8; 2];
        let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));

        wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
            if y == 1 {
                120
            } else {
                u8::try_from(100 + x).unwrap()
            }
        })
        .unwrap();

        assert_eq!(output, [100, 102]);
    }

    #[test]
    fn clip1_clamps_eight_bit_output() {
        let mut class = ZERO;
        class[0] = 512;
        let coeffs = [class];
        let mut output = [0u8; 1];
        let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);

        wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
            if x == 0 && y == 1 { 255 } else { 250 }
        })
        .unwrap();

        assert_eq!(output, [255]);
    }

    #[test]
    fn ten_bit_u16_output_is_supported_and_clamped() {
        let mut class = ZERO;
        class[0] = 512;
        let coeffs = [class];
        let mut output = [0u16; 1];
        let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);

        wiener_ns_filter_luma_block(&mut output, &params, |x, y| {
            if x == 0 && y == 1 { 1023 } else { 1000 }
        })
        .unwrap();

        assert_eq!(output, [1023]);
    }

    #[test]
    fn rejects_invalid_output_stride_fail_atomically() {
        let coeffs = [ZERO];
        let params = params(2, 2, 1, BitDepth::Eight, &coeffs, None);
        let mut output = [77u8; 4];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterOutputStrideTooSmall {
                stride_samples: 1,
                width: 2,
            }
        );
        assert_eq!(output, [77; 4]);
    }

    #[test]
    fn rejects_short_output_fail_atomically() {
        let coeffs = [ZERO];
        let params = params(2, 2, 3, BitDepth::Eight, &coeffs, None);
        let mut output = [77u8; 4];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterOutputTooSmall {
                expected: 5,
                actual: 4,
            }
        );
        assert_eq!(output, [77; 4]);
    }

    #[test]
    fn rejects_missing_coefficient_classes_fail_atomically() {
        let coeffs: [[i16; WIENER_NS_LUMA_COEFFS]; 0] = [];
        let params = params(1, 1, 1, BitDepth::Eight, &coeffs, None);
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(err, ReconError::WienerNsFilterMissingClasses);
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_short_subclass_map_fail_atomically() {
        let coeffs = [ZERO];
        let subclasses = [0usize];
        let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));
        let mut output = [77u8; 2];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterSubclassMapTooShort {
                expected: 2,
                actual: 1,
            }
        );
        assert_eq!(output, [77; 2]);
    }

    #[test]
    fn rejects_out_of_range_subclass_fail_atomically() {
        let coeffs = [ZERO];
        let subclasses = [0usize, 1];
        let params = params(2, 1, 2, BitDepth::Eight, &coeffs, Some(&subclasses));
        let mut output = [77u8; 2];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterSubclassOutOfRange {
                sample_index: 1,
                subclass: 1,
                classes: 1,
            }
        );
        assert_eq!(output, [77; 2]);
    }

    #[test]
    fn rejects_source_sample_out_of_range_fail_atomically() {
        let coeffs = [ZERO];
        let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);
        let mut output = [77u16; 1];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 1024u16).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: 0,
                y: 0,
                value: 1024,
                max: 1023,
            }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_u8_storage_for_ten_bit() {
        let coeffs = [ZERO];
        let params = params(1, 1, 1, BitDepth::Ten, &coeffs, None);
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_luma_block(&mut output, &params, |_x, _y| 0).unwrap_err();

        assert_eq!(
            err,
            ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            }
        );
        assert_eq!(output, [77]);
    }
}
