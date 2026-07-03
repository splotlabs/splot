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
use crate::math::round2;
use crate::{BitDepth, ReconError, ReconSample, Result};

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
        if let Some(slot) = self.slot(x, y) {
            *slot = Some(value);
        }
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

    let mut s = i64::from(m) << WIENER_NS_PREC_BITS;
    for &(dy, dx, coeff_index) in &WIENER_NS_CONFIG_UV {
        let tap_x = offset_coord(x, dx, "Wiener NS chroma tap x")?;
        let tap_y = offset_coord(y, dy, "Wiener NS chroma tap y")?;
        let tap = validated_source_sample(chroma_source_sample, tap_x, tap_y, max_sample)?;
        let diff = i64::from(tap) - i64::from(m);
        s += diff * i64::from(params.coeffs[coeff_index]);
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
        let diff = i64::from(tap_luma) - i64::from(m_luma);
        s += diff * i64::from(coeff);
    }

    let value = round2(s, WIENER_NS_PREC_BITS).clamp(0, i64::from(max_sample));
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
    let x = clip3(x, context.luma_start_x, context.luma_last_x);
    let y = clip3(y, 0, context.luma_last_y);

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

const fn clip3(value: isize, min: isize, max: isize) -> isize {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
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
