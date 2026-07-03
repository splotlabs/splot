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

use crate::intra_dc_math::validate_sample_type;
use crate::math::round2;
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
pub fn wiener_ns_filter_luma_block_padded<T: ReconSample>(
    output: &mut [T],
    params: &WienerNsLumaFilter<'_>,
    source: &WienerNsLumaPaddedSource<'_, T>,
) -> Result<()> {
    validate_sample_type::<T>(params.bit_depth)?;
    let sample_count = validate_luma_params(output.len(), params)?;
    validate_subclasses(params, sample_count)?;
    WienerNsLumaPaddedSource::new(source.samples, source.stride, params.width, params.height)?;

    let stride = source.stride;
    let mut tap_offsets = [0usize; WIENER_NS_LUMA_TAPS];
    let center_offset = WIENER_NS_LUMA_TAP_RADIUS
        .checked_mul(stride)
        .and_then(|row| row.checked_add(WIENER_NS_LUMA_TAP_RADIUS))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "Wiener NS padded source stride",
        })?;
    for (offset, &(dy, dx, _)) in tap_offsets.iter_mut().zip(&WIENER_NS_CONFIG_Y) {
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

    let max_sample = params.bit_depth.max_sample();
    let padded_width = params.width + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let padded_rows = params.height + 2 * WIENER_NS_LUMA_TAP_RADIUS;
    let mut clean_rows = Vec::with_capacity(padded_rows);
    for row_index in 0..padded_rows {
        let row = padded_row(source.samples, stride, row_index, padded_width)?;
        clean_rows.push(row.iter().all(|sample| sample.to_u16() <= max_sample));
    }

    let mut filtered = Vec::with_capacity(sample_count);
    let mut acc = vec![0i64; params.width];
    for r in 0..params.height {
        let window_in_range = clean_rows
            .get(r..r + 2 * WIENER_NS_LUMA_TAP_RADIUS + 1)
            .is_some_and(|rows| rows.iter().all(|&clean| clean));
        if window_in_range {
            filter_padded_luma_row_in_range(
                &mut filtered,
                &mut acc,
                source.samples,
                stride,
                r,
                params,
                max_sample,
            )?;
        } else {
            filter_padded_luma_row_validated(
                &mut filtered,
                source.samples,
                stride,
                &tap_offsets,
                center_offset,
                r,
                params,
                max_sample,
            )?;
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
/// segments. The i64 accumulation adds the same § 7.20.3 tap terms in config
/// order, so the result is bit-identical to the per-sample path.
fn filter_padded_luma_row_in_range<T: ReconSample>(
    filtered: &mut Vec<T>,
    acc: &mut [i64],
    samples: &[T],
    stride: usize,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    max_sample: u16,
) -> Result<()> {
    const RADIUS: usize = WIENER_NS_LUMA_TAP_RADIUS;
    let width = params.width;
    let padded_width = width + 2 * RADIUS;
    let mut rows: [&[T]; 2 * RADIUS + 1] = [&[]; 2 * RADIUS + 1];
    for (dy, row) in rows.iter_mut().enumerate() {
        *row = padded_row(samples, stride, r + dy, padded_width)?;
    }
    let segment_error = || ReconError::BufferLengthMismatch {
        expected: width,
        actual: 0,
    };
    let center = rows[RADIUS]
        .get(RADIUS..RADIUS + width)
        .ok_or_else(segment_error)?;

    let row_start = r.checked_mul(width).ok_or(ReconError::ArithmeticOverflow {
        context: "Wiener NS luma filter row start",
    })?;
    let mut segment_start = 0usize;
    let mut segments: Vec<(usize, usize)> = Vec::new();
    match params.subclasses {
        None => segments.push((0, width)),
        Some(subclasses) => {
            let row_subclasses = subclasses
                .get(row_start..row_start + width)
                .ok_or_else(segment_error)?;
            for run in row_subclasses.chunk_by(|a, b| a == b) {
                segments.push((segment_start, run.len()));
                segment_start += run.len();
            }
        }
    }

    for &(c0, len) in &segments {
        let sample_index = row_start + c0;
        let coeffs = params
            .coeffs_by_class
            .get(match params.subclasses {
                Some(subclasses) => subclasses
                    .get(sample_index)
                    .copied()
                    .ok_or_else(segment_error)?,
                None => 0,
            })
            .ok_or_else(segment_error)?;
        let seg = acc.get_mut(c0..c0 + len).ok_or_else(segment_error)?;
        let center_seg = center.get(c0..c0 + len).ok_or_else(segment_error)?;
        for (a, &m) in seg.iter_mut().zip(center_seg) {
            *a = i64::from(m.to_u16()) << WIENER_NS_PREC_BITS;
        }
        for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_Y {
            let coeff = i64::from(coeffs[coeff_index]);
            let row = rows
                .get(usize::try_from(dy + RADIUS as isize).map_err(|_| segment_error())?)
                .ok_or_else(segment_error)?;
            let offset = usize::try_from(dx + RADIUS as isize).map_err(|_| segment_error())?;
            let taps = row
                .get(c0 + offset..c0 + offset + len)
                .ok_or_else(segment_error)?;
            for ((a, &t), &m) in seg.iter_mut().zip(taps).zip(center_seg) {
                *a += (i64::from(t.to_u16()) - i64::from(m.to_u16())) * coeff;
            }
        }
        for &s in seg.iter() {
            let value = round2(s, WIENER_NS_PREC_BITS).clamp(0, i64::from(max_sample));
            filtered.push(T::try_from_u16(value as u16)?);
        }
    }
    Ok(())
}

/// Per-sample § 7.20.3 filtering for rows whose padded window may hold
/// out-of-range samples, preserving the original read-order error identity.
#[allow(clippy::too_many_arguments)]
fn filter_padded_luma_row_validated<T: ReconSample>(
    filtered: &mut Vec<T>,
    samples: &[T],
    stride: usize,
    tap_offsets: &[usize; WIENER_NS_LUMA_TAPS],
    center_offset: usize,
    r: usize,
    params: &WienerNsLumaFilter<'_>,
    max_sample: u16,
) -> Result<()> {
    for c in 0..params.width {
        let sample_index = r * params.width + c;
        let coeffs = coeffs_for_sample(params, sample_index);
        let base = r * stride + c;
        let m = validated_padded_sample(
            samples,
            base + center_offset,
            c as isize,
            r as isize,
            max_sample,
        )?;
        let mut s = i64::from(m) << WIENER_NS_PREC_BITS;
        for (&offset, &(dy, dx, coeff_index)) in tap_offsets.iter().zip(&WIENER_NS_CONFIG_Y) {
            let tap = validated_padded_sample(
                samples,
                base + offset,
                c as isize + dx,
                r as isize + dy,
                max_sample,
            )?;
            let diff = i64::from(tap) - i64::from(m);
            s += diff * i64::from(coeffs[coeff_index]);
        }
        let value = round2(s, WIENER_NS_PREC_BITS).clamp(0, i64::from(max_sample));
        filtered.push(T::try_from_u16(value as u16)?);
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

    let mut s = i64::from(m) << WIENER_NS_PREC_BITS;
    for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_Y {
        let tap = validated_source_sample(source_sample, x + dx, y + dy, max_sample)?;
        let diff = i64::from(tap) - i64::from(m);
        s += diff * i64::from(coeffs[coeff_index]);
    }
    let value = round2(s, WIENER_NS_PREC_BITS).clamp(0, i64::from(max_sample));
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
    fn padded_filter_rejects_source_sample_out_of_range() {
        let width = 1;
        let height = 1;
        let radius = WIENER_NS_LUMA_TAP_RADIUS;
        let stride = width + 2 * radius;
        let mut padded = vec![0u16; stride * (height + 2 * radius)];
        padded[radius] = 1024;
        let coeffs = [ZERO];
        let params = params(width, height, width, BitDepth::Ten, &coeffs, None);
        let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
        let mut output = [77u16; 1];

        let err = wiener_ns_filter_luma_block_padded(&mut output, &params, &source).unwrap_err();

        assert_eq!(
            err,
            ReconError::WienerNsFilterSourceSampleOutOfRange {
                x: 0,
                y: -4,
                value: 1024,
                max: 1023,
            }
        );
        assert_eq!(output, [77]);
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
