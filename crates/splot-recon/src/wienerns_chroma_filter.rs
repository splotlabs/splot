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

use crate::intra_dc_math::validate_sample_type;
use crate::math::round2_i32;
use crate::workspace::u16_samples_exceed;
use crate::{BitDepth, ReconError, ReconSample, Result};
use std::simd::{Simd, cmp::SimdOrd, num::SimdInt, num::SimdUint};

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
/// companion window extending twice this many luma samples beyond the
/// luma-scaled block; the `get_luma_sample` downsample neighborhood stays
/// inside that reach.
pub const WIENER_NS_CHROMA_TAP_RADIUS: usize = 2;

/// Chroma-to-chroma Wiener NS taps use coefficient slots `0..6`; AV2 § 7.20.3
/// indexes luma-tap coefficients as raw tap index `i + 6`.
const WIENER_NS_CHROMA_COEFF_SLOTS: usize = 6;

const MI_SIZE: usize = 4;

/// AV2 § 7.20.3 `Wiener_Ns_Config_Uv`: `(dy, dx, coeff_index)`.
const WIENER_NS_CONFIG_UV: [(isize, isize, usize); WIENER_NS_CHROMA_TAPS] = [
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

/// Contiguous § 7.20.2 source windows for 4:2:0 chroma Wiener NS filtering.
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
    /// The chroma window extends by two chroma samples and the luma window by
    /// four luma samples on every side.
    ///
    /// # Errors
    /// Returns a typed error when either stride or sample slice cannot cover
    /// the required padded window.
    pub fn new(
        chroma_samples: &'a [T],
        chroma_stride: usize,
        luma_samples: &'a [T],
        luma_stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        validate_padded_window(
            chroma_samples.len(),
            chroma_stride,
            width,
            height,
            WIENER_NS_CHROMA_TAP_RADIUS,
        )?;
        let luma_width = width.checked_mul(2).ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma width",
        })?;
        let luma_height = height
            .checked_mul(2)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "Wiener NS chroma padded luma height",
            })?;
        validate_padded_window(
            luma_samples.len(),
            luma_stride,
            luma_width,
            luma_height,
            2 * WIENER_NS_CHROMA_TAP_RADIUS,
        )?;
        Ok(Self {
            chroma_samples,
            chroma_stride,
            luma_samples,
            luma_stride,
        })
    }
}

/// Reusable storage for contiguous 4:2:0 chroma Wiener NS filtering.
#[derive(Debug, Default)]
pub struct WienerNsChromaScratch<T> {
    luma_ds: Vec<u16>,
    filtered: Vec<T>,
}

fn validate_padded_window(
    len: usize,
    stride: usize,
    width: usize,
    height: usize,
    radius: usize,
) -> Result<()> {
    let padded_width = width
        .checked_add(2 * radius)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded source width",
        })?;
    if stride < padded_width {
        return Err(ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: stride,
            width: padded_width,
        });
    }
    let required = height
        .checked_add(2 * radius)
        .and_then(|rows| rows.checked_sub(1))
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
/// The caller output is not modified unless all validation and filtering
/// succeeds.
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
        // splot-copy-ok: publish fail-atomic Wiener NS scratch row into caller output
        output[dst_start..dst_end].copy_from_slice(&filtered[src_start..src_end]);
    }

    Ok(())
}

/// Applies 4:2:0 chroma Wiener NS filtering from contiguous padded source windows.
///
/// This is bit-identical to [`wiener_ns_filter_chroma_block`] when the caller's
/// callbacks read the same pre-resolved windows. `scratch` retains allocations
/// between calls and the output remains fail-atomic.
///
/// # Errors
/// Returns the same parameter, source-range, output-shape, and allocation
/// errors as the callback-based filter.
pub fn wiener_ns_filter_chroma_block_padded_420_into<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsChromaFilter<'_>,
    source: &WienerNsChromaPaddedSource<'_, T>,
    scratch: &mut WienerNsChromaScratch<T>,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let context = validate_chroma_params(output.len(), params)?;
    if context.subsampling_x != 1 || context.subsampling_y != 1 {
        return Err(ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x: context.subsampling_x,
            subsampling_y: context.subsampling_y,
        });
    }
    WienerNsChromaPaddedSource::new(
        source.chroma_samples,
        source.chroma_stride,
        source.luma_samples,
        source.luma_stride,
        params.width,
        params.height,
    )?;

    let max_sample = params.bit_depth.max_sample();
    validate_padded_samples(
        source.chroma_samples,
        source.chroma_stride,
        params.width + 2 * WIENER_NS_CHROMA_TAP_RADIUS,
        params.height + 2 * WIENER_NS_CHROMA_TAP_RADIUS,
        max_sample,
    )?;
    validate_padded_samples(
        source.luma_samples,
        source.luma_stride,
        2 * params.width + 4 * WIENER_NS_CHROMA_TAP_RADIUS,
        2 * params.height + 4 * WIENER_NS_CHROMA_TAP_RADIUS,
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
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma downsample scratch allocation",
        })?;
    scratch.luma_ds.resize(ds_len, 0);
    downsample_luma_420(
        &mut scratch.luma_ds,
        ds_width,
        source.luma_samples,
        source.luma_stride,
        context.cfl_ds_filter_index,
        params,
        &context,
    )?;

    scratch.filtered.clear();
    scratch
        .filtered
        .try_reserve(context.sample_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma filtered scratch allocation",
        })?;
    scratch.filtered.resize(context.sample_count, T::default());
    if let (Some(chroma), Some(filtered)) = (
        T::u16_slice(source.chroma_samples),
        T::u16_slice_mut(&mut scratch.filtered),
    ) {
        filter_chroma_padded_u16(
            filtered,
            params,
            chroma,
            source.chroma_stride,
            &scratch.luma_ds,
            ds_width,
            max_sample,
        );
    } else {
        filter_chroma_padded_scalar(
            &mut scratch.filtered,
            params,
            source.chroma_samples,
            source.chroma_stride,
            &scratch.luma_ds,
            ds_width,
            max_sample,
        )?;
    }

    for row in 0..params.height {
        let src = row * params.width;
        let dst = row * params.output_stride;
        output[dst..dst + params.width].copy_from_slice(&scratch.filtered[src..src + params.width]); // splot-copy-ok: publish fail-atomic padded chroma Wiener NS row
    }
    Ok(())
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

fn downsample_luma_420<T: ReconSample>(
    output: &mut [u16],
    output_stride: usize,
    source: &[T],
    source_stride: usize,
    filter_index: usize,
    params: &WienerNsChromaFilter<'_>,
    context: &ChromaFilterContext,
) -> Result<()> {
    let radius = WIENER_NS_CHROMA_TAP_RADIUS as isize;
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
    let luma_origin_x = chroma_origin_x
        .checked_mul(2)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma origin x",
        })?;
    let luma_origin_y = chroma_origin_y
        .checked_mul(2)
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
                .saturating_mul(2),
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
                .saturating_mul(2),
        )
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS chroma padded luma last y",
        })?;
    if luma_origin_x >= context.luma_start_x
        && luma_last_x <= context.luma_last_x
        && luma_origin_y >= 0
        && luma_last_y <= context.luma_last_y
    {
        downsample_luma_420_unclipped(output, output_stride, source, source_stride, filter_index);
        return Ok(());
    }

    for (row, output) in output.chunks_exact_mut(output_stride).enumerate() {
        let luma_y = luma_origin_y
            .checked_add(
                isize::try_from(row)
                    .map_err(|_| ReconError::ArithmeticOverflow {
                        context: "Wiener NS chroma padded luma y",
                    })?
                    .saturating_mul(2),
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
        for (col, slot) in output.iter_mut().enumerate() {
            let luma_x = luma_origin_x
                .checked_add(
                    isize::try_from(col)
                        .map_err(|_| ReconError::ArithmeticOverflow {
                            context: "Wiener NS chroma padded luma x",
                        })?
                        .saturating_mul(2),
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
            if filter_index == 2 {
                *slot = top_left.to_u16();
                continue;
            }
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

fn downsample_luma_420_unclipped<T: ReconSample>(
    output: &mut [u16],
    output_stride: usize,
    source: &[T],
    source_stride: usize,
    filter_index: usize,
) {
    for (row, output) in output.chunks_exact_mut(output_stride).enumerate() {
        let top = &source[2 * row * source_stride..];
        let bottom = &source[(2 * row + 1) * source_stride..];
        for (col, slot) in output.iter_mut().enumerate() {
            let x = 2 * col;
            if filter_index == 2 {
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

const WIENER_NS_CONFIG_UV_PAIRS: [(isize, isize, usize); WIENER_NS_CHROMA_COEFF_SLOTS] = [
    (1, 0, 0),
    (0, 1, 1),
    (1, 1, 2),
    (-1, 1, 3),
    (2, 0, 4),
    (0, 2, 5),
];

fn padded_tap_offset(stride: usize, dy: isize, dx: isize) -> usize {
    (dy + WIENER_NS_CHROMA_TAP_RADIUS as isize) as usize * stride
        + (dx + WIENER_NS_CHROMA_TAP_RADIUS as isize) as usize
}

#[allow(clippy::too_many_arguments)]
fn filter_chroma_padded_u16(
    filtered: &mut [u16],
    params: &WienerNsChromaFilter<'_>,
    chroma: &[u16],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    max_sample: u16,
) {
    const LANES: usize = 64;
    let center_offset = padded_tap_offset(chroma_stride, 0, 0);
    let luma_center_offset = padded_tap_offset(luma_stride, 0, 0);
    for r in 0..params.height {
        let chroma_base = r * chroma_stride;
        let luma_base = r * luma_stride;
        let output = &mut filtered[r * params.width..][..params.width];
        let vector_width = params.width - params.width % LANES;
        for c in (0..vector_width).step_by(LANES) {
            let center = Simd::<u16, LANES>::from_slice(&chroma[chroma_base + center_offset + c..])
                .cast::<i32>();
            let mut sum = center << WIENER_NS_PREC_BITS as i32;
            for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_UV_PAIRS {
                let coeff = i32::from(params.coeffs[coeff_index]);
                let plus = Simd::<u16, LANES>::from_slice(
                    &chroma[chroma_base + padded_tap_offset(chroma_stride, dy, dx) + c..],
                )
                .cast::<i32>();
                let minus = Simd::<u16, LANES>::from_slice(
                    &chroma[chroma_base + padded_tap_offset(chroma_stride, -dy, -dx) + c..],
                )
                .cast::<i32>();
                sum += (plus + minus - center * Simd::splat(2)) * Simd::splat(coeff);
            }
            let luma_center =
                Simd::<u16, LANES>::from_slice(&luma_ds[luma_base + luma_center_offset + c..])
                    .cast::<i32>();
            for (tap_index, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
                let coeff = i32::from(params.coeffs[tap_index + WIENER_NS_CHROMA_COEFF_SLOTS]);
                if coeff != 0 {
                    let tap = Simd::<u16, LANES>::from_slice(
                        &luma_ds[luma_base + padded_tap_offset(luma_stride, dy, dx) + c..],
                    )
                    .cast::<i32>();
                    sum += (tap - luma_center) * Simd::splat(coeff);
                }
            }
            let values = ((sum + Simd::splat(1 << (WIENER_NS_PREC_BITS - 1)))
                >> WIENER_NS_PREC_BITS as i32)
                .simd_max(Simd::splat(0))
                .simd_min(Simd::splat(i32::from(max_sample)))
                .cast::<u16>()
                .to_array();
            output[c..c + LANES].copy_from_slice(&values); // splot-copy-ok: publish SIMD chroma Wiener NS samples
        }
        for (c, slot) in output[vector_width..].iter_mut().enumerate() {
            *slot = filter_chroma_padded_sample(
                params,
                chroma,
                chroma_stride,
                luma_ds,
                luma_stride,
                r,
                vector_width + c,
                max_sample,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_chroma_padded_scalar<T: ReconSample>(
    filtered: &mut [T],
    params: &WienerNsChromaFilter<'_>,
    chroma: &[T],
    chroma_stride: usize,
    luma_ds: &[u16],
    luma_stride: usize,
    max_sample: u16,
) -> Result<()> {
    for r in 0..params.height {
        for c in 0..params.width {
            let value = filter_chroma_padded_sample(
                params,
                chroma,
                chroma_stride,
                luma_ds,
                luma_stride,
                r,
                c,
                max_sample,
            );
            filtered[r * params.width + c] = T::try_from_u16(value)?;
        }
    }
    Ok(())
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
    let luma_base = r * luma_stride + c;
    let luma_center = i32::from(luma_ds[luma_base + padded_tap_offset(luma_stride, 0, 0)]);
    for (tap_index, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
        let coeff = i32::from(params.coeffs[tap_index + WIENER_NS_CHROMA_COEFF_SLOTS]);
        if coeff != 0 {
            let tap = i32::from(luma_ds[luma_base + padded_tap_offset(luma_stride, dy, dx)]);
            sum += (tap - luma_center) * coeff;
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

    let value = round2_i32(s, WIENER_NS_PREC_BITS).clamp(0, i32::from(max_sample));
    T::try_from_u16(value as u16)
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
mod tests {
    use super::*;

    const ZERO_CHROMA: [i16; WIENER_NS_CHROMA_COEFFS] = [0; WIENER_NS_CHROMA_COEFFS];

    #[test]
    fn chroma_tap_radius_covers_config_table() {
        let max_offset = WIENER_NS_CONFIG_UV
            .iter()
            .map(|&(dy, dx, _)| dy.unsigned_abs().max(dx.unsigned_abs()))
            .max()
            .unwrap();
        assert_eq!(WIENER_NS_CHROMA_TAP_RADIUS, max_offset);
    }

    fn chroma_params(
        width: usize,
        height: usize,
        output_stride: usize,
        bit_depth: BitDepth,
        coeffs: &[i16; WIENER_NS_CHROMA_COEFFS],
    ) -> WienerNsChromaFilter<'_> {
        WienerNsChromaFilter {
            x: 0,
            y: 0,
            width,
            height,
            output_stride,
            bit_depth,
            coeffs,
            subsampling_x: 0,
            subsampling_y: 0,
            luma_start_x: 0,
            luma_end_x: 15,
            mi_rows: 4,
            cfl_ds_filter_index: 0,
        }
    }

    #[test]
    fn zero_coefficients_copy_source_samples() {
        let mut output = [9u8; 6];
        let params = chroma_params(2, 2, 3, BitDepth::Eight, &ZERO_CHROMA);

        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |x, y| u8::try_from(20 + y * 10 + x).unwrap(),
            |_x, _y| 0,
        )
        .unwrap();

        assert_eq!(output, [20, 21, 9, 30, 31, 9]);
    }

    #[test]
    fn hand_computed_chroma_tap_accumulation() {
        let mut coeffs = ZERO_CHROMA;
        coeffs[0] = 4;
        let mut output = [0u8; 1];
        let params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);

        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |x, y| {
                if x == 0 && y == 1 { 120 } else { 100 }
            },
            |_x, _y| 0,
        )
        .unwrap();

        assert_eq!(output, [101]);
    }

    #[test]
    fn hand_computed_luma_tap_contribution_without_subsampling() {
        let mut coeffs = ZERO_CHROMA;
        coeffs[6] = 4;
        let mut output = [0u8; 1];
        let params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);

        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |_x, _y| 100,
            |x, y| {
                if x == 0 && y == 1 { 70 } else { 50 }
            },
        )
        .unwrap();

        assert_eq!(output, [101]);
    }

    /// Runs the 4:2:0 `get_luma_sample` single-output case for one
    /// `cfl_ds_filter_index`, asserting the filtered sample lands on 101.
    fn assert_luma_420_filters_to_101(
        cfl_ds_filter_index: u8,
        luma_at: impl Fn(isize, isize) -> u8,
    ) {
        let mut coeffs = ZERO_CHROMA;
        coeffs[6] = 4;
        let mut output = [0u8; 1];
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);
        params.subsampling_x = 1;
        params.subsampling_y = 1;
        params.luma_end_x = 7;
        params.mi_rows = 2;
        params.cfl_ds_filter_index = cfl_ds_filter_index;

        wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 100, luma_at).unwrap();

        assert_eq!(output, [101]);
    }

    fn two_by_two_luma_at(x: isize, y: isize) -> u8 {
        match (x, y) {
            (0, 0) => 10,
            (1, 0) => 14,
            (0, 1) => 18,
            (1, 1) => 22,
            (0, 2) => 30,
            (1, 2) => 34,
            (0, 3) => 38,
            (1, 3) => 42,
            _ => 0,
        }
    }

    #[test]
    fn luma_420_filter_index_zero_averages_two_by_two() {
        assert_luma_420_filters_to_101(0, two_by_two_luma_at);
    }

    #[test]
    fn luma_420_filter_index_one_uses_vertical_left_column() {
        assert_luma_420_filters_to_101(1, |x, y| match (x, y) {
            (0, 0) => 10,
            (0, 1) => 30,
            (0, 2) => 50,
            (0, 3) => 70,
            _ => 0,
        });
    }

    #[test]
    fn luma_420_filter_index_three_maps_to_zero() {
        assert_luma_420_filters_to_101(3, two_by_two_luma_at);
    }

    #[test]
    fn luma_422_reads_scaled_direct_luma_sample_without_420_filter() {
        let mut coeffs = ZERO_CHROMA;
        coeffs[6] = 4;
        let mut output = [0u8; 1];
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &coeffs);
        params.subsampling_x = 1;
        params.subsampling_y = 0;
        params.luma_end_x = 7;

        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |_x, _y| 100,
            |x, y| {
                if x == 0 && y == 1 { 70 } else { 50 }
            },
        )
        .unwrap();

        assert_eq!(output, [101]);
    }

    /// A multi-sample block (shared downsampled-luma cache) must be
    /// bit-identical to filtering every sample as its own 1x1 block (fresh
    /// cache per sample), across chroma and luma taps at 4:2:0 subsampling.
    #[test]
    fn block_filtering_matches_per_sample_blocks_bit_exactly() {
        let width = 9;
        let height = 5;
        let mut coeffs = ZERO_CHROMA;
        coeffs[0] = 13;
        coeffs[2] = -7;
        coeffs[5] = 3;
        coeffs[6] = 21;
        coeffs[10] = -9;
        coeffs[17] = 5;
        let chroma_at =
            |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
        let luma_at =
            |x: isize, y: isize| -> u16 { ((x * 13 + y * 41 + 700).rem_euclid(1024)) as u16 };
        let mut params = chroma_params(width, height, width, BitDepth::Ten, &coeffs);
        params.x = 6;
        params.y = 4;
        params.subsampling_x = 1;
        params.subsampling_y = 1;
        params.luma_start_x = 0;
        params.luma_end_x = 63;
        params.mi_rows = 16;
        params.cfl_ds_filter_index = 0;

        let mut block_output = vec![0u16; width * height];
        wiener_ns_filter_chroma_block(&mut block_output, &params, chroma_at, luma_at).unwrap();

        for r in 0..height {
            for c in 0..width {
                let mut single = [0u16; 1];
                let mut single_params = params;
                single_params.x = params.x + c;
                single_params.y = params.y + r;
                single_params.width = 1;
                single_params.height = 1;
                single_params.output_stride = 1;
                wiener_ns_filter_chroma_block(&mut single, &single_params, chroma_at, luma_at)
                    .unwrap();
                assert_eq!(
                    block_output[r * width + c],
                    single[0],
                    "cached block filtering diverged at ({c}, {r})"
                );
            }
        }
    }

    #[test]
    fn padded_420_filter_matches_callback_path() {
        let width = 9;
        let height = 5;
        let mut coeffs = ZERO_CHROMA;
        for (index, coeff) in coeffs.iter_mut().enumerate() {
            *coeff = (index as i16 * 7 % 23) - 11;
        }
        let chroma_at =
            |x: isize, y: isize| -> u16 { ((x * 31 + y * 17 + 512).rem_euclid(1024)) as u16 };
        let luma_at =
            |x: isize, y: isize| -> u16 { ((x * 13 + y * 41 + 700).rem_euclid(1024)) as u16 };
        let mut params = chroma_params(width, height, width, BitDepth::Ten, &coeffs);
        params.x = 0;
        params.y = 0;
        params.subsampling_x = 1;
        params.subsampling_y = 1;
        params.luma_start_x = 0;
        params.luma_end_x = 64;
        params.mi_rows = 16;

        let chroma_stride = width + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
        let chroma_height = height + 2 * WIENER_NS_CHROMA_TAP_RADIUS;
        let chroma_origin_x = params.x as isize - WIENER_NS_CHROMA_TAP_RADIUS as isize;
        let chroma_origin_y = params.y as isize - WIENER_NS_CHROMA_TAP_RADIUS as isize;
        let chroma: Vec<u16> = (0..chroma_height)
            .flat_map(|row| {
                (0..chroma_stride).map(move |col| {
                    chroma_at(
                        chroma_origin_x + col as isize,
                        chroma_origin_y + row as isize,
                    )
                })
            })
            .collect();
        let luma_radius = 2 * WIENER_NS_CHROMA_TAP_RADIUS;
        let luma_stride = 2 * width + 2 * luma_radius;
        let luma_height = 2 * height + 2 * luma_radius;
        let luma_origin_x = 2 * params.x as isize - luma_radius as isize;
        let luma_origin_y = 2 * params.y as isize - luma_radius as isize;
        let luma: Vec<u16> = (0..luma_height)
            .flat_map(|row| {
                (0..luma_stride).map(move |col| {
                    luma_at(luma_origin_x + col as isize, luma_origin_y + row as isize)
                })
            })
            .collect();
        let source = WienerNsChromaPaddedSource::new(
            &chroma,
            chroma_stride,
            &luma,
            luma_stride,
            width,
            height,
        )
        .unwrap();
        let mut scratch = WienerNsChromaScratch::default();

        for filter_index in [0, 1, 2, 3] {
            params.cfl_ds_filter_index = filter_index;
            let mut callback = vec![0u16; width * height];
            wiener_ns_filter_chroma_block(&mut callback, &params, chroma_at, luma_at).unwrap();
            let mut padded = vec![0u16; width * height];
            wiener_ns_filter_chroma_block_padded_420_into(
                &mut padded,
                &params,
                &source,
                &mut scratch,
            )
            .unwrap();
            assert_eq!(padded, callback, "cfl_ds_filter_index={filter_index}");
        }
    }

    #[test]
    fn clip1_clamps_ten_bit_output() {
        let mut coeffs = ZERO_CHROMA;
        coeffs[0] = 512;
        let mut output = [0u16; 1];
        let params = chroma_params(1, 1, 1, BitDepth::Ten, &coeffs);

        wiener_ns_filter_chroma_block(
            &mut output,
            &params,
            |x, y| {
                if x == 0 && y == 1 { 1023 } else { 1000 }
            },
            |_x, _y| 0,
        )
        .unwrap();

        assert_eq!(output, [1023]);
    }

    #[test]
    fn rejects_invalid_output_stride_fail_atomically() {
        let params = chroma_params(2, 2, 1, BitDepth::Eight, &ZERO_CHROMA);
        let mut output = [77u8; 4];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

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
        let params = chroma_params(2, 2, 3, BitDepth::Eight, &ZERO_CHROMA);
        let mut output = [77u8; 4];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

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
    fn rejects_invalid_subsampling_fail_atomically() {
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
        params.subsampling_x = 2;
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidSubsampling {
                subsampling_x: 2,
                subsampling_y: 0,
            }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_inverted_luma_x_bounds_fail_atomically() {
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
        params.luma_start_x = 8;
        params.luma_end_x = 7;
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "Wiener NS chroma luma x range",
            }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_luma_last_x_underflow_fail_atomically() {
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
        params.subsampling_x = 1;
        params.luma_end_x = 0;
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "Wiener NS chroma luma last x",
            }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_luma_last_y_underflow_fail_atomically() {
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
        params.subsampling_y = 1;
        params.mi_rows = 0;
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "Wiener NS chroma luma last y",
            }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_invalid_cfl_filter_index_fail_atomically() {
        let mut params = chroma_params(1, 1, 1, BitDepth::Eight, &ZERO_CHROMA);
        params.cfl_ds_filter_index = 4;
        let mut output = [77u8; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 0, |_x, _y| 0)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterInvalidCflDsFilterIndex { index: 4 }
        );
        assert_eq!(output, [77]);
    }

    #[test]
    fn rejects_source_sample_out_of_range_fail_atomically() {
        let params = chroma_params(1, 1, 1, BitDepth::Ten, &ZERO_CHROMA);
        let mut output = [77u16; 1];

        let err = wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 1024u16, |_x, _y| 0)
            .unwrap_err();

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
    fn rejects_luma_sample_out_of_range_fail_atomically() {
        let mut coeffs = ZERO_CHROMA;
        coeffs[6] = 1;
        let params = chroma_params(1, 1, 1, BitDepth::Ten, &coeffs);
        let mut output = [77u16; 1];

        let err =
            wiener_ns_filter_chroma_block(&mut output, &params, |_x, _y| 100u16, |_x, _y| 1024u16)
                .unwrap_err();

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
}
