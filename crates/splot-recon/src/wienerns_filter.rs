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

use fearless_simd::{Simd, SimdBase, u8x16, u16x16};
use std::ops::Range;

use crate::PlaneId;
use crate::intra_dc_math::validate_sample_type;
use crate::loop_restoration::PaddedRows;
use crate::lr_madd::{self, MaddPattern, MaddWeights, TapPairs};
#[cfg(test)]
use crate::math::round2_i32;
use crate::workspace::u16_samples_exceed;
use crate::{BitDepth, ReconError, ReconSample, Result};

#[path = "wienerns_luma_u8.rs"]
pub(crate) mod eight_bit;
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
    eight_bit: eight_bit::EightBitLuma,
}

#[derive(Clone, Copy, Debug)]
struct PreparedLumaClass {
    /// The coefficients as `pmaddwd` pairs, then the center scale paired
    /// with the rounding offset.
    madd_pairs: Option<[MaddPattern; MADD_PAIRS + 1]>,
    /// [`PreparedLumaClass::madd_pairs`] as the weights of four cells of
    /// this class, reused whenever four cells share it.
    cell_weights: Option<[MaddWeights; MADD_PAIRS + 1]>,
}

/// The § 7.20.3 luma taps for [`lr_madd`].
struct LumaTaps;

impl TapPairs for LumaTaps {
    #[cfg(target_arch = "x86_64")]
    lr_madd::madd_avx2!(LumaTaps, 9, 24);
    const RADIUS: usize = WIENER_NS_LUMA_TAP_RADIUS;
    const TAPS: &'static [(isize, isize)] = &{
        let mut taps = [(0, 0); WIENER_NS_LUMA_COEFFS];
        let mut j = 0;
        while j < WIENER_NS_LUMA_COEFFS {
            taps[j] = (WIENER_NS_CONFIG_Y_PAIRS[j].0, WIENER_NS_CONFIG_Y_PAIRS[j].1);
            j += 1;
        }
        taps
    };
}

/// Source columns one 16-sample vector row reads.
const CELL_REACH: usize = 16 + 2 * WIENER_NS_LUMA_TAP_RADIUS;

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
    /// The block takes the byte-lane kernel of [`eight_bit`].
    eight_bit: bool,
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
#[cfg(test)]
pub(crate) fn wiener_ns_filter_luma_block<T, F>(
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
        output[dst_start..dst_end].copy_from_slice(&filtered[src_start..src_end]); // splot-copy-ok: publish fail-atomic Wiener NS scratch row into caller output
    }

    Ok(())
}

/// § 7.20.2-pre-resolved source samples for [`wiener_ns_filter_luma_block_padded_into`].
///
/// Rows covering the output block extended by [`WIENER_NS_LUMA_TAP_RADIUS`]
/// on every side: sample `0` of row `0` is at block-relative
/// `(-WIENER_NS_LUMA_TAP_RADIUS, -WIENER_NS_LUMA_TAP_RADIUS)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WienerNsLumaPaddedSource<'a, T> {
    rows: PaddedRows<'a, T>,
}

impl<'a, T: ReconSample> WienerNsLumaPaddedSource<'a, T> {
    /// Wraps padded rows: padded row `i` starts at column `col` of
    /// `rows[i]`. Filtering checks the sample range.
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
        Self::check_table(self.rows, width, height).map(drop)
    }
}

/// Applies AV2 § 7.20.3 luma non-separable Wiener filtering from a padded
/// pre-resolved source.
///
/// `scratch` retains its allocations between calls, and `output` is not
/// modified unless filtering succeeds.
///
/// # Errors
/// Returns typed errors for parameter and output shape errors, missing
/// coefficient classes, source samples outside the active bit-depth range,
/// and scratch buffers that cannot reserve enough storage.
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
            context.eight_bit.then_some(&scratch.eight_bit),
        );
    }
    filter_luma_rows_to_scratch(params, source, subclasses, scratch, &context)?;
    for row_index in 0..params.height {
        let src_start = row_index * params.width;
        let src_end = src_start + params.width;
        let dst_start = row_index * params.output_stride;
        let dst_end = dst_start + params.width;
        output[dst_start..dst_end].copy_from_slice(&scratch.filtered[src_start..src_end]); // splot-copy-ok: publish fail-atomic Wiener NS scratch row into caller output
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
    for &coeffs in params.coeffs_by_class {
        let center_scale = (1 << WIENER_NS_PREC_BITS)
            - 2 * coeffs.iter().map(|&coeff| i32::from(coeff)).sum::<i32>();
        let center = i16::try_from(center_scale)
            .map_err(|_| ReconError::WienerNsFilterCenterScaleOutOfRange { center_scale })?;
        let madd_pairs: [MaddPattern; MADD_PAIRS + 1] = core::array::from_fn(|pair| {
            let (low, high) = coeffs
                .get(2 * pair..2 * pair + 2)
                .map_or((center, 1 << (WIENER_NS_PREC_BITS - 1)), |pair| {
                    (pair[0], pair[1])
                });
            [low, high, low, high, low, high, low, high]
        });
        let cell_weights = madd_pairs.each_ref().map(lr_madd::uniform);
        scratch.prepared_classes.push(PreparedLumaClass {
            madd_pairs: Some(madd_pairs),
            cell_weights: Some(cell_weights),
        });
    }
    let staged = params.bit_depth == BitDepth::Eight
        && T::u16_slice(&[]).is_some()
        && scratch.eight_bit.stage(
            |index| {
                padded_row(source.rows, index, padded_width)
                    .ok()
                    .and_then(T::u16_slice)
            },
            params.width,
            params.height,
        );
    if staged {
        scratch.clean_rows.resize(padded_rows, true);
    } else {
        for row_index in 0..padded_rows {
            let row = padded_row(source.rows, row_index, padded_width)?;
            scratch.clean_rows.push(
                T::u16_slice(row).is_none_or(|samples| !u16_samples_exceed(samples, max_sample)),
            );
        }
    }

    let direct = scratch.clean_rows.iter().all(|&clean| clean);
    let eight_bit = staged
        && scratch
            .eight_bit
            .prepare(params.width, params.coeffs_by_class, subclasses);
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
        max_sample,
        direct,
        eight_bit,
    })
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
                context.eight_bit.then_some(&scratch.eight_bit),
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
        if !window_in_range {
            first_out_of_range_read(source.rows, r, params.width, context.max_sample)?;
        }
        filter_padded_luma_rows_in_range(
            filtered,
            params.width,
            source.rows,
            r..r + 1,
            params,
            &scratch.prepared_classes,
            subclasses,
            context.max_sample,
            None,
        )?;
    }
    Ok(())
}

/// Reports the first sample above `max_sample` that the § 7.20.3 filter of
/// output row `r` reads, in its per-sample read order (columns left to
/// right, the centre and then each tap), as the per-sample filter would.
/// Each tap's source row is compared 16 lanes at a time; only a hit is
/// located sample by sample.
fn first_out_of_range_read<T: ReconSample>(
    rows: PaddedRows<'_, T>,
    r: usize,
    width: usize,
    max_sample: u16,
) -> Result<()> {
    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let padded_width = width + 2 * R;
    let taps =
        core::iter::once((0, 0)).chain(WIENER_NS_CONFIG_Y.iter().map(|&(dy, dx, _)| (dy, dx)));
    let mut first: Option<(usize, isize, isize, u16)> = None;
    for (dy, dx) in taps {
        let row = padded_row(rows, (r + R).wrapping_add_signed(dy), padded_width)?;
        let Some(samples) = T::u16_slice(row) else {
            return Ok(());
        };
        let start = R.wrapping_add_signed(dx);
        let reads = samples
            .get(start..start + width)
            .ok_or(ReconError::BufferLengthMismatch {
                expected: start + width,
                actual: samples.len(),
            })?;
        let limit = first.map_or(width, |(c, ..)| c);
        let Some(reads) = reads.get(..limit) else {
            continue;
        };
        if !u16_samples_exceed(reads, max_sample) {
            continue;
        }
        if let Some(c) = reads.iter().position(|&value| value > max_sample) {
            first = Some((c, dy, dx, reads[c]));
        }
    }
    match first {
        Some((c, dy, dx, value)) => Err(ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: c as isize + dx,
            y: r as isize + dy,
            value,
            max: max_sample,
        }),
        None => Ok(()),
    }
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
    eight_bit: Option<&eight_bit::EightBitLuma>,
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
            eight_bit,
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
            eight_bit,
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
    eight_bit: Option<&eight_bit::EightBitLuma>,
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
        if flat.any() && matches!(subclasses, LumaSubclassLayout::Samples(_)) {
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
        let lanes_done = eight_bit.map_or(0, |eight_bit| {
            eight_bit.filter_cell_rows(
                output,
                output_stride,
                (first, end, rows.start),
                params.width,
                subclasses,
                &flat,
            )
        });
        let cells_done = filter_luma_cell_groups(
            output,
            output_stride,
            &source,
            (first, end, rows.start),
            (lanes_done, params.width),
            prepared_classes,
            subclasses,
            (max_sample, &flat),
        )?;
        if cells_done >= params.width {
            first = end;
            continue;
        }
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
                    filter_luma_segment_simd(filtered, window, segment_start, class, max_sample);
                }
                Ok(())
            },
        )?;
        first = end;
    }
    Ok(())
}

/// Filters the rows `first..end` of the 4x4 cell row they share in groups of
/// four cells, 16 columns at a time from column `x`, with each cell's class
/// weights; returns the columns done. Sample-subclass layouts are left to
/// the segment filter.
#[allow(clippy::too_many_arguments)]
fn filter_luma_cell_groups<'a, T: LumaSimdSource, O: LumaSimdOutput>(
    output: &mut [O],
    output_stride: usize,
    source: &[&'a [T]; LUMA_WINDOW_ROWS + 3],
    (first, end, origin): (usize, usize, usize),
    (mut x, width): (usize, usize),
    prepared_classes: &[PreparedLumaClass],
    subclasses: LumaSubclassLayout<'_>,
    (max_sample, flat): (u16, &FlatChunks<WIENER_NS_LUMA_TAP_RADIUS>),
) -> Result<usize> {
    let cell_row = match subclasses {
        LumaSubclassLayout::Uniform => None,
        LumaSubclassLayout::Cells { values, cols } => {
            Some(values.get((first / 4) * cols..).unwrap_or_default())
        }
        LumaSubclassLayout::Samples(_) => return Ok(x),
    };
    let padded = width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let rows = end - first + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let span = |row: &&'a [T]| row.get(..padded).ok_or_else(|| luma_segment_error(width));
    let mut window = [span(&source[0])?; LUMA_WINDOW_ROWS + 3];
    for (slot, row) in window.iter_mut().zip(source).take(rows) {
        *slot = span(row)?;
    }
    let mut mixed = [[[0; 16]; 2]; MADD_PAIRS + 1];
    while x + 16 <= width {
        if let Some(value) = flat.chunk_value(x / 32) {
            let value = O::from_u16(value.min(max_sample));
            for r in first..end {
                output
                    .get_mut((r - origin) * output_stride + x..)
                    .and_then(<[O]>::first_chunk_mut::<16>)
                    .ok_or_else(|| luma_segment_error(width))?
                    .fill(value);
            }
            x += 16;
            continue;
        }
        let cells = match cell_row {
            None => [0; 4],
            Some(row) => match row.get(x / 4..).and_then(<[u8]>::first_chunk::<4>) {
                Some(&cells) => cells,
                None => break,
            },
        };
        let classes = cells.map(|class| prepared_classes.get(usize::from(class)));
        let [Some(c0), Some(c1), Some(c2), Some(c3)] = classes else {
            break;
        };
        let weights = if cells == [cells[0]; 4] {
            c0.cell_weights.as_ref()
        } else if let [Some(m0), Some(m1), Some(m2), Some(m3)] =
            [c0, c1, c2, c3].map(|class| class.madd_pairs.as_ref())
        {
            for (pair, weights) in mixed.iter_mut().enumerate() {
                lr_madd::arrange_into::<T>(weights, [&m0[pair], &m1[pair], &m2[pair], &m3[pair]]);
            }
            Some(&mixed)
        } else {
            None
        };
        let Some(weights) = weights else {
            break;
        };
        let Some(group) = cell_window(&window, x) else {
            return Err(luma_segment_error(width));
        };
        for r in first..end {
            let start = (r - origin) * output_stride + x;
            let (Some(window), Some(filtered)) = (
                group[r - first..].first_chunk::<LUMA_WINDOW_ROWS>(),
                output
                    .get_mut(start..)
                    .and_then(<[O]>::first_chunk_mut::<16>),
            ) else {
                return Err(luma_segment_error(width));
            };
            O::store16(
                lr_madd::filter16::<LumaTaps, T, LUMA_WINDOW_ROWS, CELL_REACH>(
                    window, weights, max_sample,
                ),
                filtered,
            );
        }
        x += 16;
    }
    Ok(x)
}

/// The [`CELL_REACH`] columns from `x` of up to [`LUMA_WINDOW_ROWS`]` + 3`
/// window rows; unused trailing slots repeat the first row.
#[allow(
    clippy::inline_always,
    reason = "measured: an outlined call per lane group costs more than the window"
)]
#[inline(always)]
fn cell_window<'a, T>(
    window: &[&'a [T]],
    x: usize,
) -> Option<[&'a [T; CELL_REACH]; LUMA_WINDOW_ROWS + 3]> {
    let first = window.first()?.get(x..)?.first_chunk::<CELL_REACH>()?;
    let mut rows = [first; LUMA_WINDOW_ROWS + 3];
    for (index, slot) in rows.iter_mut().enumerate().take(window.len()) {
        *slot = window[index].get(x..)?.first_chunk::<CELL_REACH>()?;
    }
    Some(rows)
}

pub(crate) trait LumaSimdOutput: Copy + eight_bit::ByteLaneOutput {
    fn from_u16(value: u16) -> Self;

    /// Stores 16 values already within the sample range.
    fn store16(values: [u16; 16], output: &mut [Self; 16]);
}

impl LumaSimdOutput for u16 {
    #[inline]
    fn from_u16(value: u16) -> Self {
        value
    }

    #[inline]
    fn store16(values: [u16; 16], output: &mut [Self; 16]) {
        *output = values;
    }
}

impl LumaSimdOutput for u8 {
    #[inline]
    fn from_u16(value: u16) -> Self {
        value as u8
    }

    #[inline]
    fn store16(values: [u16; 16], output: &mut [Self; 16]) {
        *output = values.map(|value| value as u8);
    }
}

pub(crate) trait LumaSimdSource: Copy + Default {
    /// The samples are `u16`, which the x86 tap-pair kernel reads.
    #[cfg(target_arch = "x86_64")]
    const WIDE: bool;

    /// Zero-extends 16 samples.
    fn widen16<S: Simd>(simd: S, samples: &[Self; 16]) -> u16x16<S>;

    /// The rows as `u16` rows when the samples are `u16`.
    #[cfg(target_arch = "x86_64")]
    fn u16_rows<'a, const ROWS: usize, const REACH: usize>(
        rows: &[&'a [Self; REACH]; ROWS],
    ) -> Option<[&'a [u16; REACH]; ROWS]>;

    fn scalar(samples: &[Self], index: usize) -> u16;
}

impl LumaSimdSource for u16 {
    #[cfg(target_arch = "x86_64")]
    const WIDE: bool = true;
    #[inline]
    fn widen16<S: Simd>(simd: S, samples: &[Self; 16]) -> u16x16<S> {
        u16x16::load_array_ref(simd, samples)
    }

    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn u16_rows<'a, const ROWS: usize, const REACH: usize>(
        rows: &[&'a [Self; REACH]; ROWS],
    ) -> Option<[&'a [u16; REACH]; ROWS]> {
        Some(*rows)
    }

    #[inline]
    fn scalar(samples: &[Self], index: usize) -> u16 {
        samples[index]
    }
}

impl LumaSimdSource for u8 {
    #[cfg(target_arch = "x86_64")]
    const WIDE: bool = false;
    #[inline]
    fn widen16<S: Simd>(simd: S, samples: &[Self; 16]) -> u16x16<S> {
        use fearless_simd::prelude::*;
        let (low, high) = u8x16::load_array_ref(simd, samples).widen();
        simd.combine_u16x8(low, high)
    }

    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn u16_rows<'a, const ROWS: usize, const REACH: usize>(
        _rows: &[&'a [Self; REACH]; ROWS],
    ) -> Option<[&'a [u16; REACH]; ROWS]> {
        None
    }

    #[inline]
    fn scalar(samples: &[Self], index: usize) -> u16 {
        u16::from(samples[index])
    }
}

/// Filters output columns `c0..c0 + filtered.len()` of one row from its nine
/// padded source `rows`, 16 columns at a time: the last group of a segment
/// at least 16 wide overlaps the one before it, and a narrower segment, or a
/// group whose window runs past its rows, reads a zero-padded copy of the
/// window and stores only its own columns.
///
/// The lane groups follow [`lr_madd`]: `plus + minus` stays in `i16` lanes
/// because § 6 Table 6.3 caps `BitDepth` at 10 and this path only runs over
/// rows validated against `max_sample`, so every factor is 16-bit; `|sum|`
/// stays near `2^30`, so `Round2` as `(sum + 64) >> 7` cannot overflow.
fn filter_luma_segment_simd<T: LumaSimdSource, O: LumaSimdOutput>(
    filtered: &mut [O],
    rows: &[&[T]; LUMA_WINDOW_ROWS],
    c0: usize,
    class: &PreparedLumaClass,
    max_sample: u16,
) {
    let Some(weights) = class.cell_weights.as_ref() else {
        return;
    };
    let len = filtered.len();
    let mut copy = [[T::default(); CELL_REACH]; LUMA_WINDOW_ROWS];
    let mut col = 0usize;
    while col < len {
        let at = if len >= 16 { col.min(len - 16) } else { 0 };
        let window = cell_window(rows, c0 + at);
        let values = if let Some(window) = window
            .as_ref()
            .and_then(|window| window.first_chunk::<LUMA_WINDOW_ROWS>())
        {
            lr_madd::filter16::<LumaTaps, T, LUMA_WINDOW_ROWS, CELL_REACH>(
                window, weights, max_sample,
            )
        } else {
            for (row, copy) in rows.iter().zip(&mut copy) {
                let source = row.get(c0 + at..).unwrap_or_default();
                let source = &source[..source.len().min(CELL_REACH)];
                *copy = [T::default(); CELL_REACH];
                copy[..source.len()].copy_from_slice(source); // splot-copy-ok: zero-padded window row
            }
            let window = copy.each_ref();
            lr_madd::filter16::<LumaTaps, T, LUMA_WINDOW_ROWS, CELL_REACH>(
                &window, weights, max_sample,
            )
        };
        if let Some(slot) = filtered
            .get_mut(at..)
            .and_then(<[O]>::first_chunk_mut::<16>)
        {
            O::store16(values, slot);
        } else {
            let mut lanes = [O::from_u16(0); 16];
            O::store16(values, &mut lanes);
            let tail = &mut filtered[at..];
            let len = tail.len().min(16);
            tail[..len].copy_from_slice(&lanes[..len]); // splot-copy-ok: the columns of a block narrower than one lane group
        }
        col = at + 16;
    }
}

const fn luma_segment_error(width: usize) -> ReconError {
    ReconError::BufferLengthMismatch {
        expected: width,
        actual: 0,
    }
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

#[cfg(test)]
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

#[cfg(test)]
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
