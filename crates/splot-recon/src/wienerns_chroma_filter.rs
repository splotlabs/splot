// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20.3 chroma non-separable Wiener filter sample math.
//!
//! This module implements the scheduler-free chroma branch of the AV2 § 7.20.3
//! non-separable Wiener filter process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-3)).
//! The caller resolves the § 7.20.2 source-sample process, frame/restoration-unit
//! traversal, frame-vs-unit coefficient selection, source-frame selection, and
//! runtime decode wiring.
//!
//! Feature tracking: `RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE`.

use crate::PlaneId;
use crate::intra_dc_math::validate_sample_type;
use crate::math::round2_i32;
use crate::wienerns_filter::{LumaSimdSource, validated_source_sample};
use crate::workspace::u16_samples_exceed;
use crate::{BitDepth, ReconError, ReconSample, Result};
#[cfg(target_arch = "x86_64")]
use fearless_simd::Level;
use fearless_simd::{Simd, SimdIntWiden, i16x8, i16x16, i32x4, i32x8, prelude::*, u16x16};

/// AV2 § 3 `WIENER_NS_PREC_BITS`, used by § 7.20.3 for the accumulator scale.
const WIENER_NS_PREC_BITS: u32 = 7;

/// Number of chroma Wiener NS coefficient slots consumed by AV2 § 7.20.3.
pub const WIENER_NS_CHROMA_COEFFS: usize = 18;

/// Number of chroma Wiener NS taps (`WIENER_NS_TAPS_UV`) in AV2 § 7.20.3.
pub const WIENER_NS_CHROMA_TAPS: usize = 12;

/// Maximum absolute § 7.20.3 `Wiener_Ns_Config_Uv` tap offset in either axis.
///
/// Callers that pre-resolve § 7.20.2 source samples may materialize a chroma
/// window extending this many samples beyond the output block, and a luma
/// companion window extending this radius scaled by the per-axis chroma
/// subsampling beyond the luma-scaled block; the `get_luma_sample` downsample
/// neighborhood stays inside that reach.
pub const WIENER_NS_CHROMA_TAP_RADIUS: usize = 2;

/// Chroma-to-chroma Wiener NS taps use coefficient slots `0..6`; AV2 § 7.20.3
/// indexes luma-tap coefficients as raw tap index `i + 6`.
const WIENER_NS_CHROMA_COEFF_SLOTS: usize = 6;

const MI_SIZE: usize = 4;

/// AV2 § 7.20.3 `Wiener_Ns_Config_Uv`: `(dy, dx, coeff_index)`.
pub const WIENER_NS_CONFIG_UV: [(isize, isize, usize); WIENER_NS_CHROMA_TAPS] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 1),
    (0, -1, 1),
    (1, 1, 2),
    (-1, -1, 2),
    (-1, 1, 3),
    (1, -1, 3),
    (2, 0, 4),
    (-2, 0, 4),
    (0, 2, 5),
    (0, -2, 5),
];

/// AV2 § 7.20.3 `Wiener_Filters_420`.
const WIENER_FILTERS_420: [[[u16; 2]; 2]; 2] = [[[1, 1], [1, 1]], [[2, 0], [2, 0]]];

/// Caller-resolved parameters for AV2 § 7.20.3 chroma Wiener NS filtering.
///
/// `x` and `y` are the current-plane coordinates of the output block. Chroma
/// source callbacks receive current-plane chroma coordinates; luma source
/// callbacks receive luma-plane coordinates after the spec `get_luma_sample`
/// clipping/downsampling process. `luma_start_x`, `luma_end_x`, and `mi_rows`
/// are the caller-resolved luma frame/source bounds used by that process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WienerNsChromaFilter<'a> {
    /// Output block x coordinate in current-plane samples.
    pub x: usize,
    /// Output block y coordinate in current-plane samples.
    pub y: usize,
    /// Output block width in samples.
    pub width: usize,
    /// Output block height in samples.
    pub height: usize,
    /// Distance in samples between adjacent output rows.
    pub output_stride: usize,
    /// Active decoded bit depth used for source validation and `Clip1`.
    pub bit_depth: BitDepth,
    /// Caller-resolved chroma/luma coefficient slots.
    pub coeffs: &'a [i16; WIENER_NS_CHROMA_COEFFS],
    /// AV2 sequence `SubsamplingX`.
    pub subsampling_x: u8,
    /// AV2 sequence `SubsamplingY`.
    pub subsampling_y: u8,
    /// Caller-resolved luma source lower x bound.
    pub luma_start_x: usize,
    /// Caller-resolved luma source upper x bound.
    pub luma_end_x: usize,
    /// AV2 frame height in 4x4 luma units (`MiRows`).
    pub mi_rows: usize,
    /// AV2 `cfl_ds_filter_index`; value `3` maps to filter index `0`.
    pub cfl_ds_filter_index: u8,
}

impl WienerNsChromaFilter<'_> {
    /// Returns `true` when any cross-component luma tap coefficient is nonzero.
    ///
    /// The filters read luma source samples only in that case.
    #[must_use]
    pub fn reads_luma(&self) -> bool {
        self.coeffs[WIENER_NS_CHROMA_COEFF_SLOTS..]
            .iter()
            .any(|&coeff| coeff != 0)
    }
}

/// Contiguous § 7.20.2 source windows for padded chroma Wiener NS filtering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WienerNsChromaPaddedSource<'a, T> {
    chroma_samples: &'a [T],
    chroma_stride: usize,
    luma_samples: &'a [T],
    luma_stride: usize,
}

impl<'a, T: ReconSample> WienerNsChromaPaddedSource<'a, T> {
    /// Wraps padded chroma and luma windows for a `width` by `height` chroma block.
    ///
    /// The chroma window extends by [`WIENER_NS_CHROMA_TAP_RADIUS`] chroma
    /// samples on every side; the luma window covers the chroma window scaled
    /// by the AV2 chroma subsampling on each axis.
    ///
    /// # Errors
    /// Returns a typed error when either stride or sample slice cannot cover
    /// the required padded window, or subsampling is outside the AV2 domain.
    pub fn new(
        chroma_samples: &'a [T],
        chroma_stride: usize,
        luma_samples: &'a [T],
        luma_stride: usize,
        width: usize,
        height: usize,
        (subsampling_x, subsampling_y): (u8, u8),
    ) -> Result<Self> {
        if subsampling_x > 1 || subsampling_y > 1 {
            return Err(ReconError::LoopRestorationSourceInvalidSubsampling {
                subsampling_x,
                subsampling_y,
            });
        }
        let chroma = Self::chroma_only(chroma_samples, chroma_stride, width, height)?;
        let (luma_width, luma_rows) =
            padded_window_shape(width, height, subsampling_x, subsampling_y)?;
        validate_padded_window(luma_samples.len(), luma_stride, luma_width, luma_rows)?;
        Ok(Self {
            luma_samples,
            luma_stride,
            ..chroma
        })
    }

    /// Wraps a padded chroma window with no luma companion.
    ///
    /// Filtering accepts this source only when
    /// [`WienerNsChromaFilter::reads_luma`] is `false`, and rejects it
    /// otherwise.
    ///
    /// # Errors
    /// Returns a typed error when the stride or sample slice cannot cover the
    /// padded chroma window.
    pub fn chroma_only(
        chroma_samples: &'a [T],
        chroma_stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        let (chroma_width, chroma_rows) = padded_window_shape(width, height, 0, 0)?;
        validate_padded_window(
            chroma_samples.len(),
            chroma_stride,
            chroma_width,
            chroma_rows,
        )?;
        Ok(Self {
            chroma_samples,
            chroma_stride,
            luma_samples: &[],
            luma_stride: 0,
        })
    }
}

/// Reusable storage for contiguous padded chroma Wiener NS filtering.
#[derive(Debug, Default)]
pub struct WienerNsChromaScratch {
    luma_ds: Vec<u16>,
}

/// Padded-window shape for a chroma block scaled by per-axis subsampling:
/// `((width + 2 * radius) << sub_x, (height + 2 * radius) << sub_y)`.
fn padded_window_shape(
    width: usize,
    height: usize,
    sub_x: u8,
    sub_y: u8,
) -> Result<(usize, usize)> {
    const CONTEXT: &str = "Wiener NS chroma padded window shape";
    let padded_width = width
        .checked_add(2 * WIENER_NS_CHROMA_TAP_RADIUS)
        .and_then(|padded| padded.checked_shl(u32::from(sub_x)))
        .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
    let padded_rows = height
        .checked_add(2 * WIENER_NS_CHROMA_TAP_RADIUS)
        .and_then(|padded| padded.checked_shl(u32::from(sub_y)))
        .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
    Ok((padded_width, padded_rows))
}

fn validate_padded_window(
    len: usize,
    stride: usize,
    padded_width: usize,
    padded_rows: usize,
) -> Result<()> {
    if stride < padded_width {
        return Err(ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: stride,
            width: padded_width,
        });
    }
    let required = padded_rows
        .checked_sub(1)
        .and_then(|rows| rows.checked_mul(stride))
        .and_then(|prefix| prefix.checked_add(padded_width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded source length",
        })?;
    if len < required {
        return Err(ReconError::WienerNsFilterOutputTooSmall {
            expected: required,
            actual: len,
        });
    }
    Ok(())
}

/// Applies AV2 § 7.20.3 chroma non-separable Wiener filtering to a block.
///
/// `chroma_source_sample(x, y)` is called with current-plane chroma coordinates
/// for the center sample and each `Wiener_Ns_Config_Uv` chroma tap.
/// `luma_source_sample(x, y)` is called with luma-plane coordinates selected by
/// the spec `get_luma_sample` process. The caller owns §7.20.2 source-frame
/// selection and any frame-coordinate mapping outside these per-plane
/// coordinates.
///
/// # Errors
///
/// Returns typed [`ReconError`] values for unsupported sample storage, zero block
/// dimensions, output shape errors, invalid subsampling or luma bounds, invalid
/// `cfl_ds_filter_index`, and source samples outside the active bit-depth range.
/// Luma source samples are read and checked only when
/// [`WienerNsChromaFilter::reads_luma`] is true. The caller output is not
/// modified unless all validation and filtering succeeds.
pub fn wiener_ns_filter_chroma_block<T, C, L>(
    output: &mut [T],
    params: &WienerNsChromaFilter<'_>,
    mut chroma_source_sample: C,
    mut luma_source_sample: L,
) -> Result<()>
where
    T: ReconSample,
    C: FnMut(isize, isize) -> T,
    L: FnMut(isize, isize) -> T,
{
    validate_sample_type::<T>(params.bit_depth)?;
    let context = validate_chroma_params(output.len(), params)?;

    let max_sample = params.bit_depth.max_sample();
    let mut luma_ds = LumaDsCache::new(params)?;
    let mut filtered = Vec::with_capacity(context.sample_count);
    for r in 0..params.height {
        for c in 0..params.width {
            let x = block_coord(params.x, c, "Wiener NS chroma filter x")?;
            let y = block_coord(params.y, r, "Wiener NS chroma filter y")?;
            filtered.push(filter_chroma_sample(
                params,
                &context,
                x,
                y,
                max_sample,
                &mut chroma_source_sample,
                &mut luma_source_sample,
                &mut luma_ds,
            )?);
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

/// Applies chroma Wiener NS filtering from padded source windows directly into `u16` storage.
///
/// Both supported source storage types use the same SIMD arithmetic and may
/// write an exact rectangle within a wider strided destination. All output and
/// source geometry and every fallible source transformation are validated
/// before the first destination write.
///
/// # Errors
/// Returns the same parameter, source-range, output-shape, and allocation
/// errors as [`wiener_ns_filter_chroma_block`].
pub fn wiener_ns_filter_chroma_block_padded_u16_into<T: ReconSample>(
    output: &mut [u16],
    params: &WienerNsChromaFilter<'_>,
    source: &WienerNsChromaPaddedSource<'_, T>,
    scratch: &mut WienerNsChromaScratch,
) -> Result<()> {
    prepare_chroma_padded(output.len(), params, source, scratch)?;
    filter_chroma_padded_output(output, params, source, scratch);
    Ok(())
}

/// Applies 8-bit chroma Wiener NS filtering directly into strided `u8` storage.
///
/// This uses the same rounded and clipped kernel as
/// [`wiener_ns_filter_chroma_block_padded_u16_into`]. All output and source
/// geometry and every fallible source transformation are validated before the
/// first destination write.
///
/// # Errors
/// Returns the same errors as the `u16` output path, and rejects a bit depth
/// that cannot be represented by `u8`.
pub fn wiener_ns_filter_chroma_block_padded_u8_into<T: ReconSample>(
    output: &mut [u8],
    params: &WienerNsChromaFilter<'_>,
    source: &WienerNsChromaPaddedSource<'_, T>,
    scratch: &mut WienerNsChromaScratch,
) -> Result<()> {
    validate_sample_type::<u8>(params.bit_depth)?;
    prepare_chroma_padded(output.len(), params, source, scratch)?;
    filter_chroma_padded_output(output, params, source, scratch);
    Ok(())
}

fn filter_chroma_padded_output<O: ChromaOutputSample, T: ReconSample>(
    output: &mut [O],
    params: &WienerNsChromaFilter<'_>,
    source: &WienerNsChromaPaddedSource<'_, T>,
    scratch: &WienerNsChromaScratch,
) {
    let max_sample = params.bit_depth.max_sample();
    let luma_stride = params.width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    if let Some(chroma) = T::u16_slice(source.chroma_samples) {
        filter_chroma_padded_simd(
            output,
            params,
            chroma,
            source.chroma_stride,
            &scratch.luma_ds,
            luma_stride,
            max_sample,
        );
    } else if let Some(chroma) = T::u8_slice(source.chroma_samples) {
        filter_chroma_padded_simd(
            output,
            params,
            chroma,
            source.chroma_stride,
            &scratch.luma_ds,
            luma_stride,
            max_sample,
        );
    } else {
        filter_chroma_padded_u16_scalar(
            output,
            params,
            source.chroma_samples,
            source.chroma_stride,
            &scratch.luma_ds,
            luma_stride,
            max_sample,
        );
    }
}

fn prepare_chroma_padded<T: ReconSample>(
    output_len: usize,
    params: &WienerNsChromaFilter<'_>,
    source: &WienerNsChromaPaddedSource<'_, T>,
    scratch: &mut WienerNsChromaScratch,
) -> Result<ChromaFilterContext> {
    validate_sample_type::<T>(params.bit_depth)?;
    let context = validate_chroma_params(output_len, params)?;
    let reads_luma = params.reads_luma();
    if reads_luma {
        WienerNsChromaPaddedSource::new(
            source.chroma_samples,
            source.chroma_stride,
            source.luma_samples,
            source.luma_stride,
            params.width,
            params.height,
            (params.subsampling_x, params.subsampling_y),
        )?;
    } else {
        WienerNsChromaPaddedSource::chroma_only(
            source.chroma_samples,
            source.chroma_stride,
            params.width,
            params.height,
        )?;
    }

    let max_sample = params.bit_depth.max_sample();
    let (chroma_padded_width, chroma_padded_rows) =
        padded_window_shape(params.width, params.height, 0, 0)?;
    validate_padded_samples(
        source.chroma_samples,
        source.chroma_stride,
        chroma_padded_width,
        chroma_padded_rows,
        max_sample,
    )?;
    if !reads_luma {
        scratch.luma_ds.clear();
        return Ok(context);
    }
    let (luma_padded_width, luma_padded_rows) = padded_window_shape(
        params.width,
        params.height,
        params.subsampling_x,
        params.subsampling_y,
    )?;
    validate_padded_samples(
        source.luma_samples,
        source.luma_stride,
        luma_padded_width,
        luma_padded_rows,
        max_sample,
    )?;

    let ds_width = params.width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let ds_height = params.height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
    let ds_len = ds_width
        .checked_mul(ds_height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma downsample scratch length",
        })?;
    scratch.luma_ds.clear();
    scratch
        .luma_ds
        .try_reserve(ds_len)
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::U,
            context: "Wiener NS chroma downsample scratch",
        })?;
    scratch.luma_ds.resize(ds_len, 0);
    downsample_luma(
        &mut scratch.luma_ds,
        ds_width,
        source.luma_samples,
        source.luma_stride,
        params,
        &context,
    )?;

    Ok(context)
}

fn validate_padded_samples<T: ReconSample>(
    samples: &[T],
    stride: usize,
    width: usize,
    height: usize,
    max_sample: u16,
) -> Result<()> {
    for row in 0..height {
        let start = row * stride;
        let values = &samples[start..start + width];
        if let Some(values) = T::u16_slice(values)
            && u16_samples_exceed(values, max_sample)
            && let Some((col, &value)) = values
                .iter()
                .enumerate()
                .find(|&(_, &value)| value > max_sample)
        {
            return Err(ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: col as isize,
                y: row as isize,
                value,
                max: max_sample,
            });
        }
    }
    Ok(())
}

/// Fills the chroma-geometry downsampled-luma window per § 7.20.3
/// `get_luma_sample`: coordinates scale by the per-axis subsampling, and the
/// 2x2 `Wiener_Filters_420` kernel applies only when both axes subsample and
/// `filterIdx <= 1`; every other format reads the scaled sample directly.
fn downsample_luma<T: ReconSample>(
    output: &mut [u16],
    output_stride: usize,
    source: &[T],
    source_stride: usize,
    params: &WienerNsChromaFilter<'_>,
    context: &ChromaFilterContext,
) -> Result<()> {
    let filter_index = context.cfl_ds_filter_index;
    let radius = WIENER_NS_CHROMA_TAP_RADIUS as isize;
    let scale_x = 1isize << u32::from(params.subsampling_x);
    let scale_y = 1isize << u32::from(params.subsampling_y);
    let filter_420 = (params.subsampling_x == 1 && params.subsampling_y == 1)
        .then_some(filter_index.min(PICK_420));
    let chroma_origin_x = checked_isize(params.x, "Wiener NS chroma padded luma origin x")?
        .checked_sub(radius)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma origin x",
        })?;
    let chroma_origin_y = checked_isize(params.y, "Wiener NS chroma padded luma origin y")?
        .checked_sub(radius)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma origin y",
        })?;
    let luma_origin_x =
        chroma_origin_x
            .checked_mul(scale_x)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma padded luma origin x",
            })?;
    let luma_origin_y =
        chroma_origin_y
            .checked_mul(scale_y)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma padded luma origin y",
            })?;
    let rows = output.len() / output_stride;
    let luma_last_x = luma_origin_x
        .checked_add(
            isize::try_from(output_stride.saturating_sub(1))
                .map_err(|_| ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma last x",
                })?
                .saturating_mul(scale_x),
        )
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma last x",
        })?;
    let luma_last_y = luma_origin_y
        .checked_add(
            isize::try_from(rows.saturating_sub(1))
                .map_err(|_| ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma last y",
                })?
                .saturating_mul(scale_y),
        )
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma last y",
        })?;
    if luma_origin_x >= context.luma_start_x
        && luma_last_x <= context.luma_last_x
        && luma_origin_y >= 0
        && luma_last_y <= context.luma_last_y
    {
        downsample_luma_unclipped(
            output,
            output_stride,
            source,
            source_stride,
            (scale_x as usize, scale_y as usize),
            filter_420,
        );
        return Ok(());
    }

    let step = scale_x as usize;
    let blend_420 = filter_420.filter(|&index| index != PICK_420);
    let middle = usize::try_from(context.luma_last_x - luma_origin_x)
        .ok()
        .map(|last| {
            let start = usize::try_from(context.luma_start_x - luma_origin_x).unwrap_or(0);
            start.div_ceil(step)..(last / step + 1).min(output_stride)
        })
        .filter(|middle| !middle.is_empty());
    for (row, output) in output.chunks_exact_mut(output_stride).enumerate() {
        let luma_y = luma_origin_y
            .checked_add(
                isize::try_from(row)
                    .map_err(|_| ReconError::ArithmeticOverflow {
                        context: "Wiener NS chroma padded luma y",
                    })?
                    .saturating_mul(scale_y),
            )
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma padded luma y",
            })?
            .clamp(0, context.luma_last_y);
        let source_y = usize::try_from(luma_y - luma_origin_y).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma padded luma source y",
            }
        })?;
        let mut skip = 0..0;
        if let Some(middle) = middle.clone() {
            let span =
                |start: usize| source.get(start + step * middle.start..start + step * middle.end);
            let top = source_y.saturating_mul(source_stride);
            let output = &mut output[middle.clone()];
            match (
                filter_420,
                span(top),
                span(top.saturating_add(source_stride)),
            ) {
                (Some(filter_index), Some(top), Some(bottom))
                    if downsample_luma_420_row(output, top, bottom, filter_index) =>
                {
                    skip = middle;
                }
                (None, Some(top), _) => {
                    for (slot, sample) in output.iter_mut().zip(top.iter().step_by(step)) {
                        *slot = sample.to_u16();
                    }
                    skip = middle;
                }
                _ => {}
            }
        }
        let (head, rest) = output.split_at_mut(skip.start);
        let edges = head.iter_mut().enumerate().chain(
            rest.iter_mut()
                .enumerate()
                .skip(skip.len())
                .map(|(col, slot)| (skip.start + col, slot)),
        );
        for (col, slot) in edges {
            let luma_x = luma_origin_x
                .checked_add(
                    isize::try_from(col)
                        .map_err(|_| ReconError::ArithmeticOverflow {
                            context: "Wiener NS chroma padded luma x",
                        })?
                        .saturating_mul(scale_x),
                )
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma x",
                })?
                .clamp(context.luma_start_x, context.luma_last_x);
            let source_x = usize::try_from(luma_x - luma_origin_x).map_err(|_| {
                ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma source x",
                }
            })?;
            let top = source_y
                .checked_mul(source_stride)
                .and_then(|start| start.checked_add(source_x))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma source index",
                })?;
            let bottom = top
                .checked_add(source_stride)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "Wiener NS chroma padded luma source index",
                })?;
            let top_left = source
                .get(top)
                .ok_or(ReconError::WienerNsFilterOutputTooSmall {
                    expected: top.saturating_add(1),
                    actual: source.len(),
                })?;
            let Some(filter_index) = blend_420 else {
                *slot = top_left.to_u16();
                continue;
            };
            let bottom_left =
                source
                    .get(bottom)
                    .ok_or(ReconError::WienerNsFilterOutputTooSmall {
                        expected: bottom.saturating_add(1),
                        actual: source.len(),
                    })?;
            let left = u32::from(top_left.to_u16()) + u32::from(bottom_left.to_u16());
            let sum = if filter_index == 1 {
                left * 2
            } else {
                let top_right =
                    source
                        .get(top + 1)
                        .ok_or(ReconError::WienerNsFilterOutputTooSmall {
                            expected: top.saturating_add(2),
                            actual: source.len(),
                        })?;
                let bottom_right =
                    source
                        .get(bottom + 1)
                        .ok_or(ReconError::WienerNsFilterOutputTooSmall {
                            expected: bottom.saturating_add(2),
                            actual: source.len(),
                        })?;
                left + u32::from(top_right.to_u16()) + u32::from(bottom_right.to_u16())
            };
            *slot = (sum >> 2) as u16;
        }
    }
    Ok(())
}

fn downsample_luma_unclipped<T: ReconSample>(
    output: &mut [u16],
    output_stride: usize,
    source: &[T],
    source_stride: usize,
    (scale_x, scale_y): (usize, usize),
    filter_420: Option<usize>,
) {
    if let Some(filter_index) = filter_420 {
        if let Some(source) = T::u16_slice(source) {
            return downsample_luma_420(output, output_stride, source, source_stride, filter_index);
        }
        if let Some(source) = T::u8_slice(source) {
            return downsample_luma_420(output, output_stride, source, source_stride, filter_index);
        }
    }
    for (row, output) in output.chunks_exact_mut(output_stride).enumerate() {
        let top = &source[scale_y * row * source_stride..];
        let Some(filter_index) = filter_420 else {
            for (col, slot) in output.iter_mut().enumerate() {
                *slot = top[scale_x * col].to_u16();
            }
            continue;
        };
        let bottom = &source[(scale_y * row + 1) * source_stride..];
        for (col, slot) in output.iter_mut().enumerate() {
            let x = scale_x * col;
            if filter_index == PICK_420 {
                *slot = top[x].to_u16();
                continue;
            }
            let left = u32::from(top[x].to_u16()) + u32::from(bottom[x].to_u16());
            let sum = if filter_index == 1 {
                left * 2
            } else {
                left + u32::from(top[x + 1].to_u16()) + u32::from(bottom[x + 1].to_u16())
            };
            *slot = (sum >> 2) as u16;
        }
    }
}

/// [`downsample_luma_unclipped`] for 4:2:0 `Wiener_Filters_420`, one output
/// row at a time.
fn downsample_luma_420<T: ReconSample + LumaSimdSource>(
    output: &mut [u16],
    output_stride: usize,
    source: &[T],
    source_stride: usize,
    filter_index: usize,
) {
    for (row, output) in output.chunks_exact_mut(output_stride).enumerate() {
        let top = &source[2 * row * source_stride..];
        let bottom = match filter_index {
            PICK_420 => top,
            _ => &source[(2 * row + 1) * source_stride..],
        };
        downsample_420_lanes(output, top, bottom, filter_index);
    }
}

/// One 4:2:0 output row from luma rows `top` and `bottom` that start at its
/// first sample's luma column and reach `2 * output.len()` samples, or `false`
/// when the storage has no lane load.
fn downsample_luma_420_row<T: ReconSample>(
    output: &mut [u16],
    top: &[T],
    bottom: &[T],
    filter_index: usize,
) -> bool {
    if let (Some(top), Some(bottom)) = (T::u16_slice(top), T::u16_slice(bottom)) {
        downsample_420_lanes(output, top, bottom, filter_index);
        return true;
    }
    if let (Some(top), Some(bottom)) = (T::u8_slice(top), T::u8_slice(bottom)) {
        downsample_420_lanes(output, top, bottom, filter_index);
        return true;
    }
    false
}

/// Sixteen outputs per step: the two luma rows add lane-wise, then even and
/// odd columns add pairwise (or the even column doubles for `filterIdx` 1).
fn downsample_420_lanes<S: ReconSample + LumaSimdSource>(
    output: &mut [u16],
    top: &[S],
    bottom: &[S],
    filter_index: usize,
) {
    let col = crate::lr_madd::lr_simd!(simd => downsample_420_simd(simd, output, top, bottom, filter_index));
    for (x, slot) in output
        .iter_mut()
        .enumerate()
        .skip(col)
        .map(|(col, slot)| (2 * col, slot))
    {
        if filter_index == PICK_420 {
            *slot = top[x].to_u16();
            continue;
        }
        let left = u32::from(top[x].to_u16()) + u32::from(bottom[x].to_u16());
        let sum = if filter_index == 1 {
            left * 2
        } else {
            left + u32::from(top[x + 1].to_u16()) + u32::from(bottom[x + 1].to_u16())
        };
        *slot = (sum >> 2) as u16;
    }
}

#[inline]
fn downsample_420_simd<S: Simd, T: LumaSimdSource>(
    simd: S,
    output: &mut [u16],
    top: &[T],
    bottom: &[T],
    filter_index: usize,
) -> usize {
    let load = |row: &[T], at: usize| {
        row.get(at..)
            .and_then(<[T]>::first_chunk::<16>)
            .map(|lanes| T::widen16(simd, lanes))
    };
    let Some(last) = output.len().checked_sub(16) else {
        return 0;
    };
    for chunk in 0..output.len().div_ceil(16) {
        let col = (16 * chunk).min(last);
        let (Some(slot), Some(a), Some(b)) = (
            output
                .get_mut(col..)
                .and_then(<[u16]>::first_chunk_mut::<16>),
            load(top, 2 * col),
            load(top, 2 * col + 16),
        ) else {
            return col;
        };
        if filter_index == PICK_420 {
            a.deinterleave(b).0.store_array(slot);
            continue;
        }
        let (Some(c), Some(d)) = (load(bottom, 2 * col), load(bottom, 2 * col + 16)) else {
            return col;
        };
        let (even, odd) = (a + c).deinterleave(b + d);
        let pair = if filter_index == 1 { even } else { odd };
        ((even + pair) >> 2).store_array(slot);
    }
    output.len()
}

/// The `filter_420` index of a 4:2:0 `get_luma_sample` that takes its
/// top-left luma sample unfiltered (§ 7.20.3 `filterIdx > 1`).
const PICK_420: usize = 2;

const WIENER_NS_CONFIG_UV_PAIRS: [(isize, isize, usize); WIENER_NS_CHROMA_COEFF_SLOTS] = [
    (1, 0, 0),
    (0, 1, 1),
    (1, 1, 2),
    (-1, 1, 3),
    (2, 0, 4),
    (0, 2, 5),
];

trait ChromaOutputSample: Copy {
    fn from_clamped_u16(value: u16) -> Self;

    fn write_lanes<const LANES: usize>(output: &mut [Self], values: [u16; LANES]) {
        for (output, value) in output.iter_mut().zip(values) {
            *output = Self::from_clamped_u16(value);
        }
    }
}

impl ChromaOutputSample for u16 {
    fn from_clamped_u16(value: u16) -> Self {
        value
    }

    fn write_lanes<const LANES: usize>(output: &mut [Self], values: [u16; LANES]) {
        output.copy_from_slice(&values); // splot-copy-ok: publish SIMD chroma Wiener NS samples
    }
}

impl ChromaOutputSample for u8 {
    fn from_clamped_u16(value: u16) -> Self {
        value as u8
    }
}

fn padded_tap_offset(stride: usize, dy: isize, dx: isize) -> usize {
    (dy + WIENER_NS_CHROMA_TAP_RADIUS as isize) as usize * stride
        + (dx + WIENER_NS_CHROMA_TAP_RADIUS as isize) as usize
}

/// Filters every output row in 16-lane groups, finishing each row's last
/// few samples with the scalar kernel.
#[allow(clippy::too_many_arguments)]
fn filter_chroma_padded_simd<O: ChromaOutputSample, T: ReconSample + LumaSimdSource>(
    output: &mut [O],
    params: &WienerNsChromaFilter<'_>,
    chroma: &[T],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    max_sample: u16,
) {
    crate::lr_madd::lr_simd!(simd => filter_chroma_rows(simd, output, params, chroma, chroma_stride, luma_ds, luma_stride, max_sample));
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn filter_chroma_rows<S: Simd, O: ChromaOutputSample, T: ReconSample + LumaSimdSource>(
    simd: S,
    output: &mut [O],
    params: &WienerNsChromaFilter<'_>,
    chroma: &[T],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    max_sample: u16,
) {
    let reads_luma = params.reads_luma();
    #[cfg(target_arch = "x86_64")]
    let wide = Level::baseline().as_avx2().zip(T::u16_slice(chroma));
    #[cfg(target_arch = "x86_64")]
    let words: [i32; 9] = core::array::from_fn(|pair| {
        let [low, high] = match pair {
            0..3 => [
                params.coeffs[WIENER_NS_CONFIG_UV_PAIRS[2 * pair].2],
                params.coeffs[WIENER_NS_CONFIG_UV_PAIRS[2 * pair + 1].2],
            ],
            _ => [
                params.coeffs[2 * (pair - 3) + WIENER_NS_CHROMA_COEFF_SLOTS],
                params.coeffs[2 * (pair - 3) + 1 + WIENER_NS_CHROMA_COEFF_SLOTS],
            ],
        };
        i32::from(low as u16) | (i32::from(high) << 16)
    });
    #[cfg(target_arch = "x86_64")]
    let vectors = wide.map(|(token, _)| avx2::pair_vectors(token, &words));
    #[cfg(target_arch = "x86_64")]
    let narrow_done = match (wide, vectors.as_ref()) {
        (Some((token, chroma)), Some(vectors)) if (8..16).contains(&params.width) => {
            filter_chroma_narrow(
                token,
                output,
                params,
                (chroma, chroma_stride),
                (luma_ds, luma_stride),
                vectors,
                max_sample,
            )
        }
        _ => false,
    };
    #[cfg(not(target_arch = "x86_64"))]
    let narrow_done = false;
    for r in (0..params.height).filter(|_| !narrow_done) {
        let output = &mut output[r * params.output_stride..][..params.width];
        let mut c = 0;
        while c < params.width && params.width >= 16 {
            c = c.min(params.width - 16);
            #[cfg(target_arch = "x86_64")]
            if let (Some((token, chroma)), Some(vectors)) = (wide, vectors.as_ref())
                && let Some(chroma) = avx2::window(chroma, r * chroma_stride + c, chroma_stride)
                && let Some(luma) = if reads_luma {
                    avx2::window(luma_ds, r * luma_stride + c, luma_stride)
                } else {
                    Some(chroma)
                }
            {
                let values = avx2::filter16(token, &chroma, &luma, vectors, reads_luma, max_sample);
                O::write_lanes::<16>(&mut output[c..c + 16], values);
                c += 16;
                continue;
            }
            let chroma_base = r * chroma_stride + c;
            let load_chroma = |dy: isize, dx: isize| -> i16x16<S> {
                chroma
                    .get(chroma_base + padded_tap_offset(chroma_stride, dy, dx)..)
                    .and_then(<[T]>::first_chunk::<16>)
                    .map_or_else(
                        || i16x16::splat(simd, 0),
                        |lanes| T::widen16(simd, lanes).bitcast(),
                    )
            };
            let luma_base = r * luma_stride + c;
            let load_luma = |dy: isize, dx: isize| -> i16x16<S> {
                luma_ds
                    .get(luma_base + padded_tap_offset(luma_stride, dy, dx)..)
                    .and_then(<[u16]>::first_chunk::<16>)
                    .map_or_else(
                        || i16x16::splat(simd, 0),
                        |lanes| u16x16::load_array_ref(simd, lanes).bitcast(),
                    )
            };
            let center = load_chroma(0, 0);
            let round = i32x4::splat(simd, 1 << (WIENER_NS_PREC_BITS - 1));
            let (c_lo, c_hi) = simd.split_i16x16(center);
            let w_center = i16x8::splat(simd, 1 << WIENER_NS_PREC_BITS);
            let (c0, c1) = c_lo.widening_mul(w_center);
            let (c2, c3) = c_hi.widening_mul(w_center);
            let mut sums = [c0 + round, c1 + round, c2 + round, c3 + round];
            let mut add_term = |a: i16x16<S>, coeff: i16| {
                let w = i16x8::splat(simd, coeff);
                let (a_lo, a_hi) = simd.split_i16x16(a);
                let (p0, p1) = a_lo.widening_mul(w);
                let (p2, p3) = a_hi.widening_mul(w);
                sums[0] += p0;
                sums[1] += p1;
                sums[2] += p2;
                sums[3] += p3;
            };
            let twice_center = center + center;
            for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_UV_PAIRS {
                let coeff = params.coeffs[coeff_index];
                if coeff != 0 {
                    add_term(
                        load_chroma(dy, dx) + load_chroma(-dy, -dx) - twice_center,
                        coeff,
                    );
                }
            }
            if reads_luma {
                let luma_center = load_luma(0, 0);
                for (j, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
                    let coeff = params.coeffs[j + WIENER_NS_CHROMA_COEFF_SLOTS];
                    if coeff != 0 {
                        add_term(load_luma(dy, dx) - luma_center, coeff);
                    }
                }
            }
            let sums = [
                simd.combine_i32x4(sums[0], sums[1]),
                simd.combine_i32x4(sums[2], sums[3]),
            ];
            let (zero, max) = (
                i32x8::splat(simd, 0),
                i32x8::splat(simd, i32::from(max_sample)),
            );
            let [low, high] = sums.map(|sum| (sum >> WIENER_NS_PREC_BITS).clamp(zero, max));
            let values: u16x16<S> = low.narrow(high).bitcast();
            O::write_lanes::<16>(&mut output[c..c + 16], values.into());
            c += 16;
        }
        for (offset, slot) in output[c..].iter_mut().enumerate() {
            *slot = O::from_clamped_u16(filter_chroma_padded_sample(
                params,
                chroma,
                chroma_stride,
                luma_ds,
                luma_stride,
                r,
                c + offset,
                max_sample,
            ));
        }
    }
}

/// [`filter_chroma_rows`] for a block 8 to 15 samples wide: two rows per
/// vector, the last 8 columns overlapping the first when the width is not 8.
/// Returns `false`, having written nothing, when a window is short.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
fn filter_chroma_narrow<O: ChromaOutputSample>(
    token: fearless_simd::Avx2,
    output: &mut [O],
    params: &WienerNsChromaFilter<'_>,
    (chroma, chroma_stride): (&[u16], usize),
    (luma_ds, luma_stride): (&[u16], usize),
    vectors: &avx2::PairVectors,
    max_sample: u16,
) -> bool {
    let reads_luma = params.reads_luma();
    let window = |r: usize, c: usize| {
        let chroma = avx2::half_window(chroma, r * chroma_stride + c, chroma_stride)?;
        let luma = if reads_luma {
            avx2::half_window(luma_ds, r * luma_stride + c, luma_stride)?
        } else {
            chroma
        };
        Some((chroma, luma))
    };
    let columns = [0, params.width - 8];
    let rows = params.height;
    let all_present = (0..rows).all(|r| columns.iter().all(|&c| window(r, c).is_some()));
    let output_fits = output.len() >= (rows - 1) * params.output_stride + params.width;
    if !all_present || !output_fits {
        return false;
    }
    for first in (0..rows).step_by(2) {
        let second = (first + 1).min(rows - 1);
        for c in columns {
            let (Some(a), Some(b)) = (window(first, c), window(second, c)) else {
                return false;
            };
            let values = avx2::filter8x2(
                token,
                [&a.0, &b.0],
                [&a.1, &b.1],
                vectors,
                reads_luma,
                max_sample,
            );
            let (low, high) = values.split_at(8);
            O::write_lanes::<8>(
                &mut output[first * params.output_stride + c..][..8],
                low.try_into().unwrap_or([0; 8]),
            );
            O::write_lanes::<8>(
                &mut output[second * params.output_stride + c..][..8],
                high.try_into().unwrap_or([0; 8]),
            );
        }
    }
    true
}

#[inline(never)]
fn filter_chroma_padded_u16_scalar<O: ChromaOutputSample, T: ReconSample>(
    output: &mut [O],
    params: &WienerNsChromaFilter<'_>,
    chroma: &[T],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    max_sample: u16,
) {
    for r in 0..params.height {
        let output = &mut output[r * params.output_stride..][..params.width];
        for (c, slot) in output.iter_mut().enumerate() {
            *slot = O::from_clamped_u16(filter_chroma_padded_sample(
                params,
                chroma,
                chroma_stride,
                luma_ds,
                luma_stride,
                r,
                c,
                max_sample,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_chroma_padded_sample<T: ReconSample>(
    params: &WienerNsChromaFilter<'_>,
    chroma: &[T],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    r: usize,
    c: usize,
    max_sample: u16,
) -> u16 {
    let chroma_base = r * chroma_stride + c;
    let center = i32::from(chroma[chroma_base + padded_tap_offset(chroma_stride, 0, 0)].to_u16());
    let mut sum = center << WIENER_NS_PREC_BITS;
    for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_UV {
        let tap =
            i32::from(chroma[chroma_base + padded_tap_offset(chroma_stride, dy, dx)].to_u16());
        sum += (tap - center) * i32::from(params.coeffs[coeff_index]);
    }
    if params.reads_luma() {
        let luma_base = r * luma_stride + c;
        let luma_center = i32::from(luma_ds[luma_base + padded_tap_offset(luma_stride, 0, 0)]);
        for (tap_index, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
            let coeff = i32::from(params.coeffs[tap_index + WIENER_NS_CHROMA_COEFF_SLOTS]);
            if coeff != 0 {
                let tap = i32::from(luma_ds[luma_base + padded_tap_offset(luma_stride, dy, dx)]);
                sum += (tap - luma_center) * coeff;
            }
        }
    }
    round2_i32(sum, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample)) as u16
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChromaFilterContext {
    sample_count: usize,
    luma_start_x: isize,
    luma_last_x: isize,
    luma_last_y: isize,
    subsampling_x: u8,
    subsampling_y: u8,
    cfl_ds_filter_index: usize,
}

fn validate_chroma_params(
    output_len: usize,
    params: &WienerNsChromaFilter<'_>,
) -> Result<ChromaFilterContext> {
    if params.width == 0 {
        return Err(ReconError::ZeroDimension {
            field: "wiener NS chroma filter width",
        });
    }
    if params.height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "wiener NS chroma filter height",
        });
    }
    if params.output_stride < params.width {
        return Err(ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: params.output_stride,
            width: params.width,
        });
    }

    let sample_count =
        params
            .width
            .checked_mul(params.height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma filter sample count",
            })?;
    let expected = (params.height - 1)
        .checked_mul(params.output_stride)
        .and_then(|prefix| prefix.checked_add(params.width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma filter output length",
        })?;
    if output_len < expected {
        return Err(ReconError::WienerNsFilterOutputTooSmall {
            expected,
            actual: output_len,
        });
    }
    if params.subsampling_x > 1 || params.subsampling_y > 1 {
        return Err(ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x: params.subsampling_x,
            subsampling_y: params.subsampling_y,
        });
    }
    if params.luma_start_x > params.luma_end_x {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma x range",
        });
    }
    if params.cfl_ds_filter_index > 3 {
        return Err(ReconError::WienerNsFilterInvalidCflDsFilterIndex {
            index: params.cfl_ds_filter_index,
        });
    }

    let luma_last_x = params
        .luma_end_x
        .checked_sub(usize::from(params.subsampling_x))
        .ok_or(ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma last x",
        })?;
    if params.luma_start_x > luma_last_x {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma x clip range",
        });
    }
    let luma_rows = params
        .mi_rows
        .checked_mul(MI_SIZE)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma luma rows",
        })?;
    let luma_last_y = luma_rows
        .checked_sub(1)
        .and_then(|value| value.checked_sub(usize::from(params.subsampling_y)))
        .ok_or(ReconError::LoopRestorationSourceInvalidBounds {
            field: "Wiener NS chroma luma last y",
        })?;

    let mut cfl_ds_filter_index = params.cfl_ds_filter_index;
    if cfl_ds_filter_index == 3 {
        cfl_ds_filter_index = 0;
    }

    Ok(ChromaFilterContext {
        sample_count,
        luma_start_x: checked_isize(params.luma_start_x, "Wiener NS chroma luma start x")?,
        luma_last_x: checked_isize(luma_last_x, "Wiener NS chroma luma last x")?,
        luma_last_y: checked_isize(luma_last_y, "Wiener NS chroma luma last y")?,
        subsampling_x: params.subsampling_x,
        subsampling_y: params.subsampling_y,
        cfl_ds_filter_index: usize::from(cfl_ds_filter_index),
    })
}

/// Memoized § 7.20.3 `get_luma_sample` values keyed by chroma coordinate.
///
/// Adjacent chroma samples tap the same downsampled-luma positions up to 13
/// times each; the cache resolves each position once. Entries are computed
/// lazily on first read, so which positions reach the luma source — and the
/// first out-of-range read that errors — match uncached filtering exactly.
struct LumaDsCache {
    values: Vec<Option<u16>>,
    origin_x: isize,
    origin_y: isize,
    width: usize,
}

impl LumaDsCache {
    fn new(params: &WienerNsChromaFilter<'_>) -> Result<Self> {
        const CONTEXT: &str = "Wiener NS chroma luma cache geometry";
        let width = params
            .width
            .checked_add(2 * WIENER_NS_CHROMA_TAP_RADIUS)
            .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
        let height = params
            .height
            .checked_add(2 * WIENER_NS_CHROMA_TAP_RADIUS)
            .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
        let cells = width
            .checked_mul(height)
            .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
        let radius = WIENER_NS_CHROMA_TAP_RADIUS as isize;
        let origin_x = checked_isize(params.x, CONTEXT)?
            .checked_sub(radius)
            .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
        let origin_y = checked_isize(params.y, CONTEXT)?
            .checked_sub(radius)
            .ok_or(ReconError::ArithmeticOverflow { context: CONTEXT })?;
        Ok(Self {
            values: vec![None; cells],
            origin_x,
            origin_y,
            width,
        })
    }

    fn slot(&mut self, x: isize, y: isize) -> Option<&mut Option<u16>> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        if col >= self.width {
            return None;
        }
        let index = row.checked_mul(self.width)?.checked_add(col)?;
        self.values.get_mut(index)
    }

    fn get_or_compute<T, F>(
        &mut self,
        context: &ChromaFilterContext,
        x: isize,
        y: isize,
        max_sample: u16,
        luma_source_sample: &mut F,
    ) -> Result<u16>
    where
        T: ReconSample,
        F: FnMut(isize, isize) -> T,
    {
        let Some(slot) = self.slot(x, y) else {
            return get_luma_sample(context, x, y, max_sample, luma_source_sample);
        };
        if let Some(value) = *slot {
            return Ok(value);
        }
        let value = get_luma_sample(context, x, y, max_sample, luma_source_sample)?;
        *slot = Some(value);
        Ok(value)
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_chroma_sample<T, C, L>(
    params: &WienerNsChromaFilter<'_>,
    context: &ChromaFilterContext,
    x: isize,
    y: isize,
    max_sample: u16,
    chroma_source_sample: &mut C,
    luma_source_sample: &mut L,
    luma_ds: &mut LumaDsCache,
) -> Result<T>
where
    T: ReconSample,
    C: FnMut(isize, isize) -> T,
    L: FnMut(isize, isize) -> T,
{
    let m = validated_source_sample(chroma_source_sample, x, y, max_sample)?;

    let mut s = i32::from(m) << WIENER_NS_PREC_BITS;
    for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_UV {
        let tap_x = offset_coord(x, dx, "Wiener NS chroma tap x")?;
        let tap_y = offset_coord(y, dy, "Wiener NS chroma tap y")?;
        let tap = validated_source_sample(chroma_source_sample, tap_x, tap_y, max_sample)?;
        let diff = i32::from(tap) - i32::from(m);
        s += diff * i32::from(params.coeffs[coeff_index]);
    }

    if params.reads_luma() {
        let m_luma = luma_ds.get_or_compute(context, x, y, max_sample, luma_source_sample)?;
        for (tap_index, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
            let coeff = params.coeffs[tap_index + WIENER_NS_CHROMA_COEFF_SLOTS];
            if coeff == 0 {
                continue;
            }
            let tap_x = offset_coord(x, dx, "Wiener NS chroma luma tap x")?;
            let tap_y = offset_coord(y, dy, "Wiener NS chroma luma tap y")?;
            let tap_luma =
                luma_ds.get_or_compute(context, tap_x, tap_y, max_sample, luma_source_sample)?;
            let diff = i32::from(tap_luma) - i32::from(m_luma);
            s += diff * i32::from(coeff);
        }
    }

    let value = round2_i32(s, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample));
    T::try_from_u16(value as u16)
}

/// The § 7.20.3 `get_luma_sample` value at every position of a `width x
/// height` chroma plane, from the luma plane `luma` with row stride `stride`,
/// with the frame's `subsampling`, `cfl_ds_filter_index`, `LumaEndX` (the
/// frame's last luma column, `LumaStartX = 0`) and `MiRows`.
///
/// # Errors
/// Returns [`ReconError`] for a subsampling above `1`, a filter index above
/// `3`, an empty luma range, or a luma sample outside `luma` or above
/// `max_sample`.
pub fn wiener_ns_chroma_luma_plane(
    (luma, stride): (&[u16], usize),
    (width, height): (usize, usize),
    (subsampling_x, subsampling_y): (u8, u8),
    cfl_ds_filter_index: u8,
    (luma_end_x, mi_rows): (usize, usize),
    max_sample: u16,
) -> Result<Vec<u16>> {
    let invalid = |field| ReconError::LoopRestorationSourceInvalidBounds { field };
    if subsampling_x > 1 || subsampling_y > 1 {
        return Err(ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x,
            subsampling_y,
        });
    }
    if cfl_ds_filter_index > 3 {
        return Err(ReconError::WienerNsFilterInvalidCflDsFilterIndex {
            index: cfl_ds_filter_index,
        });
    }
    let last = |end: Option<usize>, sub: u8, field| {
        end.and_then(|end| end.checked_sub(usize::from(sub)))
            .ok_or(invalid(field))
            .and_then(|value| checked_isize(value, field))
    };
    let context = ChromaFilterContext {
        sample_count: 0,
        luma_start_x: 0,
        luma_last_x: last(
            Some(luma_end_x),
            subsampling_x,
            "Wiener NS chroma luma last x",
        )?,
        luma_last_y: last(
            (mi_rows * MI_SIZE).checked_sub(1),
            subsampling_y,
            "Wiener NS chroma luma last y",
        )?,
        subsampling_x,
        subsampling_y,
        cfl_ds_filter_index: usize::from(cfl_ds_filter_index % 3),
    };
    let mut source = |x: isize, y: isize| {
        luma.get(y as usize * stride + x as usize)
            .copied()
            .unwrap_or(u16::MAX)
    };
    let mut plane = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            plane.push(get_luma_sample::<u16, _>(
                &context,
                checked_isize(x, "Wiener NS chroma x")?,
                checked_isize(y, "Wiener NS chroma y")?,
                max_sample,
                &mut source,
            )?);
        }
    }
    Ok(plane)
}

fn get_luma_sample<T, F>(
    context: &ChromaFilterContext,
    x: isize,
    y: isize,
    max_sample: u16,
    luma_source_sample: &mut F,
) -> Result<u16>
where
    T: ReconSample,
    F: FnMut(isize, isize) -> T,
{
    let x = scale_coord(x, context.subsampling_x, "Wiener NS chroma luma sample x")?;
    let y = scale_coord(y, context.subsampling_y, "Wiener NS chroma luma sample y")?;
    let x = x.clamp(context.luma_start_x, context.luma_last_x);
    let y = y.clamp(0, context.luma_last_y);

    if context.subsampling_x == 1 && context.subsampling_y == 1 && context.cfl_ds_filter_index <= 1
    {
        let mut sum = 0u16;
        for (dy, row) in WIENER_FILTERS_420[context.cfl_ds_filter_index]
            .iter()
            .enumerate()
        {
            for (dx, &weight) in row.iter().enumerate() {
                let sample_x = offset_coord(x, dx as isize, "Wiener NS chroma luma filter x")?;
                let sample_y = offset_coord(y, dy as isize, "Wiener NS chroma luma filter y")?;
                let value =
                    validated_source_sample(luma_source_sample, sample_x, sample_y, max_sample)?;
                sum = sum
                    .checked_add(weight * value)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "Wiener NS chroma luma filter sum",
                    })?;
            }
        }
        Ok(sum >> 2)
    } else {
        validated_source_sample(luma_source_sample, x, y, max_sample)
    }
}

fn block_coord(base: usize, offset: usize, context: &'static str) -> Result<isize> {
    let value = base
        .checked_add(offset)
        .ok_or(ReconError::ArithmeticOverflow { context })?;
    checked_isize(value, context)
}

fn checked_isize(value: usize, context: &'static str) -> Result<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn offset_coord(value: isize, offset: isize, context: &'static str) -> Result<isize> {
    value
        .checked_add(offset)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn scale_coord(value: isize, shift: u8, context: &'static str) -> Result<isize> {
    match shift {
        0 => Ok(value),
        1 => value
            .checked_mul(2)
            .ok_or(ReconError::ArithmeticOverflow { context }),
        _ => Err(ReconError::ArithmeticOverflow { context }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
#[path = "wienerns_chroma_filter_tests.rs"]
mod tests;

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    use fearless_simd::{Avx2, SimdBase, SimdInto, u16x8, u16x16};

    use super::{
        WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_CONFIG_UV, WIENER_NS_CONFIG_UV_PAIRS,
        WIENER_NS_PREC_BITS,
    };

    const R: usize = WIENER_NS_CHROMA_TAP_RADIUS;
    const REACH: usize = 16 + 2 * R;

    /// The padded rows of one 16-sample group: row `R + dy` from column
    /// `R + dx` holds the taps at offset `(dy, dx)`.
    pub(super) type Window<'a> = [&'a [u16; REACH]; 2 * R + 1];

    /// The [`Window`] whose first sample is `samples[base]`, rows `stride`
    /// apart.
    #[inline]
    pub(super) fn window(samples: &[u16], base: usize, stride: usize) -> Option<Window<'_>> {
        let row = |dy: usize| samples.get(base + dy * stride..)?.first_chunk::<REACH>();
        Some([row(0)?, row(1)?, row(2)?, row(3)?, row(4)?])
    }

    #[inline]
    fn at<'a>(window: &Window<'a>, dy: isize, dx: isize) -> &'a [u16; 16] {
        window[R.wrapping_add_signed(dy)][R.wrapping_add_signed(dx)..]
            .first_chunk::<16>()
            .unwrap_or(&[0; 16])
    }

    /// The three chroma and six luma coefficient pairs as `pmaddwd` weights.
    pub(super) struct PairVectors {
        weights: [__m256i; 9],
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn pair_vectors(t: Avx2, words: &[i32; 9]) -> PairVectors {
            let _ = t;
            PairVectors {
                weights: words.map(|word| _mm256_set1_epi32(word)),
            }
        }
    );

    /// The § 7.20.3 chroma sum over tap loaders `chroma(dy, dx)` and
    /// `luma(dy, dx)` that return 16 lanes, with the in-lane unpack the final
    /// saturating pack restores.
    macro_rules! chroma_lanes {
        ($chroma:expr, $luma:expr, $pairs:expr, $reads_luma:expr, $max_sample:expr) => {{
            let (chroma, luma, pairs) = ($chroma, $luma, $pairs);
            let mut sums = [_mm256_setzero_si256(); 2];
            let mut add = |a: __m256i, b: __m256i, weights: __m256i| {
                sums[0] = _mm256_add_epi32(
                    sums[0],
                    _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights),
                );
                sums[1] = _mm256_add_epi32(
                    sums[1],
                    _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
                );
            };
            let center = chroma(0, 0);
            add(
                center,
                _mm256_set1_epi16(1),
                _mm256_set1_epi32(
                    (1 << WIENER_NS_PREC_BITS) | (1 << (WIENER_NS_PREC_BITS - 1 + 16)),
                ),
            );
            let twice_center = _mm256_add_epi16(center, center);
            let diff = |j: usize| {
                let (dy, dx, _) = WIENER_NS_CONFIG_UV_PAIRS[j];
                _mm256_sub_epi16(
                    _mm256_add_epi16(chroma(dy, dx), chroma(-dy, -dx)),
                    twice_center,
                )
            };
            for (pair, &weights) in pairs.weights[..3].iter().enumerate() {
                add(diff(2 * pair), diff(2 * pair + 1), weights);
            }
            if $reads_luma {
                let luma_center = luma(0, 0);
                let diff = |j: usize| {
                    let (dy, dx, _) = WIENER_NS_CONFIG_UV[j];
                    _mm256_sub_epi16(luma(dy, dx), luma_center)
                };
                for (pair, &weights) in pairs.weights[3..].iter().enumerate() {
                    add(diff(2 * pair), diff(2 * pair + 1), weights);
                }
            }
            _mm256_min_epu16(
                _mm256_packus_epi32(
                    _mm256_srai_epi32::<{ WIENER_NS_PREC_BITS as i32 }>(sums[0]),
                    _mm256_srai_epi32::<{ WIENER_NS_PREC_BITS as i32 }>(sums[1]),
                ),
                _mm256_set1_epi16($max_sample as i16),
            )
        }};
    }

    fearless_simd::kernel!(
        /// One 16-sample group of one row of `filter_chroma_rows`.
        #[inline]
        pub(super) fn filter16(
            t: Avx2,
            chroma_rows: &Window<'_>,
            luma_rows: &Window<'_>,
            pairs: &PairVectors,
            reads_luma: bool,
            max_sample: u16,
        ) -> [u16; 16] {
            let chroma = |dy: isize, dx: isize| -> __m256i {
                u16x16::load_array_ref(t, at(chroma_rows, dy, dx)).into()
            };
            let luma = |dy: isize, dx: isize| -> __m256i {
                u16x16::load_array_ref(t, at(luma_rows, dy, dx)).into()
            };
            let values = chroma_lanes!(chroma, luma, pairs, reads_luma, max_sample);
            let values: u16x16<Avx2> = values.simd_into(t);
            values.into()
        }
    );

    /// The padded rows of one 8-sample group, as [`Window`] for 8 lanes.
    pub(super) type HalfWindow<'a> = [&'a [u16; HALF_REACH]; 2 * R + 1];
    const HALF_REACH: usize = 8 + 2 * R;

    /// The [`HalfWindow`] whose first sample is `samples[base]`.
    #[inline]
    pub(super) fn half_window(
        samples: &[u16],
        base: usize,
        stride: usize,
    ) -> Option<HalfWindow<'_>> {
        let row = |dy: usize| {
            samples
                .get(base + dy * stride..)?
                .first_chunk::<HALF_REACH>()
        };
        Some([row(0)?, row(1)?, row(2)?, row(3)?, row(4)?])
    }

    #[inline]
    fn half_at<'a>(window: &HalfWindow<'a>, dy: isize, dx: isize) -> &'a [u16; 8] {
        window[R.wrapping_add_signed(dy)][R.wrapping_add_signed(dx)..]
            .first_chunk::<8>()
            .unwrap_or(&[0; 8])
    }

    fearless_simd::kernel!(
        /// One 8-sample group of two rows of `filter_chroma_rows`, the first
        /// row in the low lane halves and the second in the high ones.
        #[inline]
        pub(super) fn filter8x2(
            t: Avx2,
            chroma_rows: [&HalfWindow<'_>; 2],
            luma_rows: [&HalfWindow<'_>; 2],
            pairs: &PairVectors,
            reads_luma: bool,
            max_sample: u16,
        ) -> [u16; 16] {
            let pair = |rows: [&HalfWindow<'_>; 2], dy: isize, dx: isize| -> __m256i {
                _mm256_set_m128i(
                    u16x8::load_array_ref(t, half_at(rows[1], dy, dx)).into(),
                    u16x8::load_array_ref(t, half_at(rows[0], dy, dx)).into(),
                )
            };
            let chroma = |dy: isize, dx: isize| pair(chroma_rows, dy, dx);
            let luma = |dy: isize, dx: isize| pair(luma_rows, dy, dx);
            let values = chroma_lanes!(chroma, luma, pairs, reads_luma, max_sample);
            let values: u16x16<Avx2> = values.simd_into(t);
            values.into()
        }
    );
}
