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

        // m = 100, s = 100 << 7. The first tap adds (120 - 100) * 4; the
        // paired (-1, 0, 0) tap contributes zero. Round2(12880, 7) = 101.
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
