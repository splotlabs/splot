// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.3 reconstruct residual-addition step.
//!
//! This module implements the scheduler-free final step of the AV2 § 7.14.3
//! reconstruct process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-3`): adding the inverse-transform residual to the predicted samples
//! and clamping with § 4.8 `Clip1`,
//! `CurrFrame[plane][y + i][x + j] = Clip1(CurrFrame[plane][y + i][x + j] + Residual[i][j])`.
//!
//! Feature tracking: `RECON-RESIDUAL-ADDITION`.

#[cfg(target_arch = "x86_64")]
use fearless_simd::Level;
use fearless_simd::{
    Bytes as _, Simd, SimdBase as _, SimdInt as _, SimdNarrow as _, SimdWiden as _, i16x16, i32x4,
    i32x8, u8x16, u16x8, u16x16, u32x4, u32x8, u64x2,
};

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{
    __m128i, __m256i, _mm_adds_epi16, _mm_cvtepu8_epi16, _mm_min_epu8, _mm_packs_epi32,
    _mm_packus_epi16, _mm256_adds_epi16, _mm256_castsi256_si128, _mm256_cvtepu8_epi16,
    _mm256_extracti128_si256, _mm256_packs_epi32, _mm256_packus_epi16, _mm256_permute4x64_epi64,
};
#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, SimdInto as _};

use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// Applies the AV2 § 7.14.3 residual-addition step to a block of predicted
/// samples, writing `out`.
///
/// Each output is `Clip1(prediction[i] + residual[i])`, where § 4.8
/// `Clip1(x) = Clip3(0, (1 << BitDepth) - 1, x)` clamps to the active decoded bit
/// depth. `prediction` holds the intra/inter predicted samples (the values
/// § 7.14.3 reads from `CurrFrame` before adding the residual) and `residual`
/// holds the signed inverse-transform output.
///
/// The sum uses saturating `i32` addition, so it is total and never panics; because
/// `Clip1` bounds the result to `0..=max_sample` for the active bit depth (and
/// the sample type is validated to represent that depth), every written value
/// fits the storage type.
///
/// Prediction samples are validated up front against the active bit depth — like
/// the other `splot-recon` caller-buffer primitives — and a wider storage type
/// holding an out-of-range value (e.g. a `u16` above 255 at 8-bit) is rejected
/// rather than silently folded by `Clip1`. § 7.14.3 reads in-range prediction
/// samples from `CurrFrame`, so a conformant caller never trips this check.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::ReconstructLengthMismatch`] if `prediction`,
/// `residual`, and `out` do not all have the same length, and
/// [`ReconError::ReconstructPredictionOutOfRange`] if a prediction sample exceeds
/// the active bit depth. All inputs are validated before any output is written.
pub fn reconstruct_add_residual<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    bit_depth: BitDepth,
    out: &mut [T],
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if prediction.len() != residual.len() || out.len() != prediction.len() {
        return Err(ReconError::ReconstructLengthMismatch {
            prediction_len: prediction.len(),
            residual_len: residual.len(),
            out_len: out.len(),
        });
    }

    let max_sample = bit_depth.max_sample();
    validate_prediction_range(prediction, max_sample)?;
    add_residual_row(prediction, residual, i32::from(max_sample), out)
}

/// Applies the AV2 § 7.14.3 residual-addition step of one contiguous prediction
/// block directly to strided destination rows.
///
/// `out` starts at the destination rectangle's first sample and spans through
/// its final row, advancing `out_stride` samples per row; `prediction` and
/// `residual` hold `width * height` samples in block raster order. Callers own
/// the `out_stride >= width` geometry invariant, which the current-frame
/// rectangle view establishes before building `out`.
///
/// This is [`reconstruct_add_residual`] without the block staging buffer: it
/// writes `Clip1(prediction + residual)` where the copy-based path would first
/// fill a block, range-scan it, and copy it row by row into the destination.
/// Every input is validated before the first destination sample changes, and
/// `Clip1` bounds each written value to the active bit depth, so no destination
/// write can fail once those checks pass.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::ZeroDimension`] for an empty rectangle,
/// [`ReconError::ReconstructLengthMismatch`] if `prediction` or `residual` is
/// not `width * height` samples, [`ReconError::ArithmeticOverflow`] or
/// [`ReconError::BufferLengthMismatch`] if the destination rows do not fit
/// `out`, and [`ReconError::ReconstructPredictionOutOfRange`] if a prediction
/// sample exceeds the active bit depth.
pub(crate) fn add_block_residual_into_rows<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    bit_depth: BitDepth,
    out: &mut [T],
    out_stride: usize,
    width: usize,
    height: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if width == 0 || height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "residual destination rectangle",
        });
    }
    debug_assert!(out_stride >= width);
    let samples = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "residual destination sample count",
        })?;
    if prediction.len() != samples || residual.len() != samples {
        return Err(ReconError::ReconstructLengthMismatch {
            prediction_len: prediction.len(),
            residual_len: residual.len(),
            out_len: samples,
        });
    }
    let span = (height - 1)
        .checked_mul(out_stride)
        .and_then(|offset| offset.checked_add(width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "residual destination row span",
        })?;
    if out.len() < span {
        return Err(ReconError::BufferLengthMismatch {
            expected: span,
            actual: out.len(),
        });
    }

    let max_sample = bit_depth.max_sample();
    validate_prediction_range(prediction, max_sample)?;
    let max = i32::from(max_sample);
    for row in 0..height {
        let source = row * width;
        let target = row * out_stride;
        add_residual_row(
            &prediction[source..source + width],
            &residual[source..source + width],
            max,
            &mut out[target..target + width],
        )?;
    }
    Ok(())
}

/// Rejects a prediction sample the active bit depth cannot represent, scanning
/// `u16` storage a lane group at a time.
fn validate_prediction_range<T: ReconSample>(prediction: &[T], max_sample: u16) -> Result<()> {
    if let Some(samples) = T::u16_slice(prediction)
        && !crate::workspace::u16_samples_exceed(samples, max_sample)
    {
        return Ok(());
    }
    for (sample_index, &pred) in prediction.iter().enumerate() {
        let value = pred.to_u16();
        if value > max_sample {
            return Err(ReconError::ReconstructPredictionOutOfRange {
                sample_index,
                value,
                max: max_sample,
            });
        }
    }
    Ok(())
}

/// Writes `Clip1(prediction + residual)` over one destination row.
fn add_residual_row<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    max: i32,
    out: &mut [T],
) -> Result<()> {
    if let Some(prediction) = T::u16_slice(prediction)
        && let Some(out) = T::u16_slice_mut(out)
    {
        add_residual_u16(prediction, residual, max, out);
        return Ok(());
    }
    if max <= i32::from(u8::MAX)
        && let Some(prediction) = T::u8_slice(prediction)
        && let Some(out) = T::u8_slice_mut(out)
    {
        add_residual_u8(Some(prediction), residual, max, out);
        return Ok(());
    }
    for ((slot, &pred), &res) in out.iter_mut().zip(prediction).zip(residual) {
        let reconstructed = i32::from(pred.to_u16()).saturating_add(res).clamp(0, max);
        *slot = T::try_from_u16(reconstructed as u16)?;
    }
    Ok(())
}

/// Adds the § 7.14.3 residual to `samples` in place: `Clip1(sample + residual)`
/// with saturating `i32` addition, over the samples `residual` covers.
///
/// `u16` storage takes 8-lane groups, and `u8` storage with an 8-bit `max`
/// takes a plain loop that vectorizes; every value they write is the one the
/// checked conversion of the generic loop writes.
///
/// # Errors
/// Returns the [`ReconSample::try_from_u16`] error when a clipped value does not
/// fit `T`.
pub(crate) fn add_residual_in_place<T: ReconSample>(
    samples: &mut [T],
    residual: &[i32],
    max: i32,
) -> Result<()> {
    if let Some(samples) = T::u16_slice_mut(samples) {
        let len = samples.len().min(residual.len());
        let (samples, residual) = (&mut samples[..len], &residual[..len]);
        baseline_simd!(simd => {
            let (groups, samples) = samples.as_chunks_mut::<8>();
            let (residual_groups, residual) = residual.as_chunks::<8>();
            for (group, residual) in groups.iter_mut().zip(residual_groups) {
                *group = add_residual_lanes(simd, *group, residual, max);
            }
            add_residual_u16_tail(simd, None, residual, max, samples);
        });
        return Ok(());
    }
    if max <= i32::from(u8::MAX)
        && let Some(samples) = T::u8_slice_mut(samples)
    {
        add_residual_u8(None, residual, max, samples);
        return Ok(());
    }
    for (sample, &res) in samples.iter_mut().zip(residual) {
        let value = i32::from(sample.to_u16()).saturating_add(res).clamp(0, max);
        *sample = T::try_from_u16(value as u16)?;
    }
    Ok(())
}

/// Adds the § 7.14.3 residual over `u16` storage in 8- and 4-lane groups.
///
/// Each lane repeats the scalar `Clip1(prediction + residual)` step with the
/// same saturating `i32` addition, so every output sample is bit-identical;
/// `Clip1` bounds the result to `0..=max`, which the `u16` storage represents
/// exactly.
fn add_residual_u16(prediction: &[u16], residual: &[i32], max: i32, out: &mut [u16]) {
    let len = out.len().min(prediction.len()).min(residual.len());
    let (out, prediction, residual) = (&mut out[..len], &prediction[..len], &residual[..len]);
    baseline_simd!(simd => add_residual_u16_lanes(simd, prediction, residual, max, out));
}

#[inline]
fn add_residual_u16_lanes<S: Simd>(
    simd: S,
    prediction: &[u16],
    residual: &[i32],
    max: i32,
    out: &mut [u16],
) {
    let (out_groups, out) = out.as_chunks_mut::<8>();
    let (prediction_groups, prediction) = prediction.as_chunks::<8>();
    let (residual_groups, residual) = residual.as_chunks::<8>();
    for ((slot, &pred), res) in out_groups
        .iter_mut()
        .zip(prediction_groups)
        .zip(residual_groups)
    {
        *slot = add_residual_lanes(simd, pred, res, max);
    }
    add_residual_u16_tail(simd, Some(prediction), residual, max, out);
}

/// [`add_residual_lanes`] over the fewer than eight samples of `out`: one
/// 4-lane group, then the rest as one zero-padded group; `prediction` is
/// `out` itself when `None`.
#[inline]
fn add_residual_u16_tail<S: Simd>(
    simd: S,
    prediction: Option<&[u16]>,
    residual: &[i32],
    max: i32,
    out: &mut [u16],
) {
    let len = out.len().min(residual.len());
    let mut start = 0;
    if let (Some(slot), Some(res)) = (out.first_chunk::<4>(), residual.first_chunk::<4>()) {
        let pred = prediction
            .and_then(<[u16]>::first_chunk::<4>)
            .unwrap_or(slot);
        let sum = add_residual_lanes(simd, *pred, res, max);
        if let Some(slot) = out.first_chunk_mut::<4>() {
            *slot = sum;
        }
        start = 4;
    }
    if start >= len {
        return;
    }
    let n = len - start;
    let (mut pred, mut res) = ([0u16; 4], [0i32; 4]);
    pred[..n].copy_from_slice(&prediction.unwrap_or(&*out)[start..len]); // splot-copy-ok: zero-padded lane group
    res[..n].copy_from_slice(&residual[start..len]); // splot-copy-ok: zero-padded lane group
    let sum = add_residual_lanes(simd, pred, &res, max);
    out[start..len].copy_from_slice(&sum[..n]); // splot-copy-ok: the group's own lanes
}

/// `Clip1(prediction + residual)` over `u8` storage, sixteen, eight, then
/// four lanes at a time, for `0 <= max <= 255`; `prediction` is `out` itself
/// when `None`.
#[allow(clippy::inline_always, reason = "per-row residual add hot path")]
#[inline(always)]
fn add_residual_u8(prediction: Option<&[u8]>, residual: &[i32], max: i32, out: &mut [u8]) {
    baseline_simd!(simd => add_residual_u8_lanes(simd, prediction, residual, max, out));
}

#[allow(clippy::inline_always, reason = "per-row residual add hot path")]
#[inline(always)]
fn add_residual_u8_lanes<S: Simd>(
    simd: S,
    prediction: Option<&[u8]>,
    residual: &[i32],
    max: i32,
    out: &mut [u8],
) {
    let mut done = 0;
    macro_rules! lanes {
        ($lanes:literal) => {
            while let (Some(slot), Some(res)) = (
                out.get_mut(done..done + $lanes),
                residual.get(done..done + $lanes),
            ) {
                let pred = match prediction {
                    Some(prediction) => prediction.get(done..done + $lanes),
                    None => Some(&*slot),
                };
                let (Some(pred), Ok(res)) = (
                    pred.and_then(|p| p.first_chunk::<$lanes>()),
                    <&[i32; $lanes]>::try_from(res),
                ) else {
                    break;
                };
                let value = clip_add_u8(simd, pred, res, max);
                slot.copy_from_slice(&value); // splot-copy-ok: one residual lane group
                done += $lanes;
            }
        };
    }
    lanes!(16);
    lanes!(8);
    lanes!(4);
    let len = out.len().min(residual.len());
    if let Some(source) = prediction.unwrap_or(&*out).get(done..len)
        && !source.is_empty()
    {
        let n = len - done;
        let (mut pred, mut res) = ([0u8; 4], [0i32; 4]);
        pred[..n].copy_from_slice(source); // splot-copy-ok: zero-padded lane group
        res[..n].copy_from_slice(&residual[done..len]); // splot-copy-ok: zero-padded lane group
        let sum = clip_add_u8(simd, &pred, &res, max);
        out[done..len].copy_from_slice(&sum[..n]); // splot-copy-ok: the group's own lanes
    }
}

/// Copies `src` into the as long `dst` as `u16`: `u16` samples as they are,
/// `u8` samples widened 16 lanes at a time, a short last group zero-padded.
pub fn widen_to_u16<T: ReconSample>(src: &[T], dst: &mut [u16]) {
    if let Some(src) = T::u16_slice(src) {
        let len = src.len().min(dst.len());
        dst[..len].copy_from_slice(&src[..len]); // splot-copy-ok: u16 rows need no widening
        return;
    }
    let Some(src) = T::u8_slice(src) else {
        return;
    };
    baseline_simd!(simd => {
        let widen = |lanes: [u8; 16]| -> [u16; 16] {
            let (low, high) = simd.widen_u8x16(u8x16::load_array(simd, lanes));
            simd.combine_u16x8(low, high).into()
        };
        let (groups, tail) = src.as_chunks::<16>();
        let (out, out_tail) = dst.as_chunks_mut::<16>();
        for (out, &group) in out.iter_mut().zip(groups) {
            *out = widen(group);
        }
        let len = tail.len().min(out_tail.len());
        if len > 0 && groups.len() <= out.len() {
            let mut lanes = [0u8; 16];
            lanes[..len].copy_from_slice(&tail[..len]); // splot-copy-ok: zero-padded lane group
            out_tail[..len].copy_from_slice(&widen(lanes)[..len]); // splot-copy-ok: the group's own lanes
        }
    });
}

/// `Clip1(sample + residual)` over the 8-bit `width x height` block from
/// `base` of `samples`, rows `stride` apart, as saturating byte adds of the
/// constant `residual`: 16 lanes at a time, a row's short last group
/// zero-padded.
pub(crate) fn add_constant_u8_rows(
    samples: &mut [u8],
    (base, stride, width, height): (usize, usize, usize, usize),
    residual: i32,
) {
    let magnitude = residual.unsigned_abs().min(255) as u8;
    let (plus, minus) = if residual < 0 {
        (0, magnitude)
    } else {
        (magnitude, 0)
    };
    baseline_simd!(simd => {
        let (plus, minus) = (u8x16::splat(simd, plus), u8x16::splat(simd, minus));
        let add = |v: [u8; 16]| -> [u8; 16] {
            let v = simd.saturating_add_u8x16(u8x16::load_array(simd, v), plus);
            simd.saturating_sub_u8x16(v, minus).into()
        };
        macro_rules! rows {
            ($width:expr) => {
                for row in 0..height {
                    let Some(row) = samples.get_mut(base + row * stride..).and_then(|r| r.get_mut(..$width)) else {
                        break;
                    };
                    let (groups, tail) = row.as_chunks_mut::<16>();
                    for group in groups {
                        *group = add(*group);
                    }
                    if !tail.is_empty() {
                        let mut lanes = [0u8; 16];
                        lanes[..tail.len()].copy_from_slice(tail); // splot-copy-ok: zero-padded lane group
                        let len = tail.len();
                        tail.copy_from_slice(&add(lanes)[..len]); // splot-copy-ok: the group's own lanes
                    }
                }
            };
        }
        match width {
            4 => rows!(4),
            8 => rows!(8),
            16 => rows!(16),
            32 => rows!(32),
            _ => rows!(width),
        }
    });
}

/// One lane group of [`add_residual_u8`], `LANES <= 16`. The residual
/// saturates to `i16` and the sum saturates in `i16`: a residual past `i16`
/// drives the `i32` sum past `0..=max` the same way, so every clipped value
/// matches the saturating `i32` addition.
#[inline]
fn clip_add_u8<S: Simd, const LANES: usize>(
    simd: S,
    prediction: &[u8; LANES],
    residual: &[i32; LANES],
    max: i32,
) -> [u8; LANES] {
    let (pred, res) = (&prediction[..], &residual[..]);
    #[cfg(target_arch = "x86_64")]
    if let (Some(token), Ok(pred), Some((lo, hi))) = (
        Level::baseline().as_avx2(),
        <&[u8; 16]>::try_from(pred),
        res.split_at_checked(8),
    ) {
        let sum = clip_add_u8x16_avx2(
            token,
            u8x16::from_slice(token, pred).into(),
            i32x8::from_slice(token, lo).into(),
            i32x8::from_slice(token, hi).into(),
            u8x16::splat(token, max as u8).into(),
        );
        let sum: u8x16<Avx2> = sum.simd_into(token);
        let mut out = [0u8; LANES];
        out.copy_from_slice(&sum.as_slice()[..LANES]); // splot-copy-ok: one residual lane group
        return out;
    }
    #[cfg(target_arch = "x86_64")]
    if let (Some(token), Ok(pred), Ok(res)) = (
        Level::baseline().as_avx2(),
        <[u8; 8]>::try_from(pred),
        <&[i32; 8]>::try_from(res),
    ) {
        let sum = clip_add_u8x8_avx2(
            token,
            u64x2::from_slice(token, &[u64::from_le_bytes(pred), 0]).into(),
            i32x8::from_slice(token, res).into(),
            u8x16::splat(token, max as u8).into(),
        );
        let sum: u8x16<Avx2> = sum.simd_into(token);
        let mut out = [0u8; LANES];
        out.copy_from_slice(&sum.as_slice()[..LANES]); // splot-copy-ok: one residual lane group
        return out;
    }
    let (pred, res) = if let (Ok(pred), Some((lo, hi))) =
        (<&[u8; 16]>::try_from(pred), res.split_at_checked(8))
    {
        (
            u8x16::from_slice(simd, pred),
            i32x8::from_slice(simd, lo).saturating_narrow(i32x8::from_slice(simd, hi)),
        )
    } else if let (Ok(pred), Ok(res)) = (<[u8; 8]>::try_from(pred), <&[i32; 8]>::try_from(res)) {
        (
            u64x2::from_slice(simd, &[u64::from_le_bytes(pred), 0]).bitcast(),
            i32x8::from_slice(simd, res).saturating_narrow(i32x8::splat(simd, 0)),
        )
    } else if let (Ok(pred), Ok(res)) = (<[u8; 4]>::try_from(pred), <&[i32; 4]>::try_from(res)) {
        let res = i32x4::from_slice(simd, res);
        (
            u32x4::from_slice(simd, &[u32::from_le_bytes(pred), 0, 0, 0]).bitcast(),
            simd.combine_i32x4(res, res)
                .saturating_narrow(i32x8::splat(simd, 0)),
        )
    } else {
        return [0; LANES];
    };
    let (lo, hi) = pred.widen();
    let pred: i16x16<S> = simd.combine_u16x8(lo, hi).bitcast();
    let sum = pred
        .saturating_add(res)
        .max(i16x16::splat(simd, 0))
        .min(i16x16::splat(simd, max as i16))
        .bitcast::<u16x16<S>>();
    let sum: [u8; 32] = sum.narrow(sum).into();
    let mut out = [0u8; LANES];
    out.copy_from_slice(&sum[..LANES]); // splot-copy-ok: one residual lane group
    out
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// Sixteen lanes of [`clip_add_u8`]: `packssdw` saturates the residual
    /// to `i16`, `paddsw` adds the widened prediction, and `packuswb` clips
    /// to `0..=255` before the `max` bound.
    #[inline]
    fn clip_add_u8x16_avx2(
        token: Avx2,
        prediction: __m128i,
        lo: __m256i,
        hi: __m256i,
        max: __m128i,
    ) -> __m128i {
        let residual = _mm256_permute4x64_epi64::<0xd8>(_mm256_packs_epi32(lo, hi));
        let sum = _mm256_adds_epi16(_mm256_cvtepu8_epi16(prediction), residual);
        let packed = _mm256_packus_epi16(sum, sum);
        let packed = _mm256_castsi256_si128(_mm256_permute4x64_epi64::<0x08>(packed));
        _mm_min_epu8(packed, max)
    }
);

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// Eight lanes of [`clip_add_u8`], as [`clip_add_u8x16_avx2`] on halves.
    #[inline]
    fn clip_add_u8x8_avx2(
        token: Avx2,
        prediction: __m128i,
        residual: __m256i,
        max: __m128i,
    ) -> __m128i {
        let residual = _mm_packs_epi32(
            _mm256_castsi256_si128(residual),
            _mm256_extracti128_si256::<1>(residual),
        );
        let sum = _mm_adds_epi16(_mm_cvtepu8_epi16(prediction), residual);
        _mm_min_epu8(_mm_packus_epi16(sum, sum), max)
    }
);

/// `Clip1(prediction + residual)` over one lane group of 8 or 4 lanes, for
/// `max >= 0`.
#[inline]
fn add_residual_lanes<S: Simd, const LANES: usize>(
    simd: S,
    prediction: [u16; LANES],
    residual: &[i32; LANES],
    max: i32,
) -> [u16; LANES] {
    let (pred, res) = (&prediction[..], &residual[..]);
    let (pred, res) =
        if let (Ok(pred), Ok(res)) = (<&[u16; 8]>::try_from(pred), <&[i32; 8]>::try_from(res)) {
            (u16x8::from_slice(simd, pred), i32x8::from_slice(simd, res))
        } else if let (Ok(&[p0, p1, p2, p3]), Ok(res)) =
            (<&[u16; 4]>::try_from(pred), <&[i32; 4]>::try_from(res))
        {
            let res = i32x4::from_slice(simd, res);
            (
                u16x8::from_slice(simd, &[p0, p1, p2, p3, 0, 0, 0, 0]),
                simd.combine_i32x4(res, res),
            )
        } else {
            return [0; LANES];
        };
    let (lo, hi) = pred.widen();
    let pred: i32x8<S> = simd.combine_u32x4(lo, hi).bitcast();
    let sum = pred
        .saturating_add(res)
        .max(i32x8::splat(simd, 0))
        .min(i32x8::splat(simd, max))
        .bitcast::<u32x8<S>>();
    let sum: [u16; 16] = sum.narrow(sum).into();
    let mut out = [0u16; LANES];
    out.copy_from_slice(&sum[..LANES]); // splot-copy-ok: one residual lane group
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn in_place_residual_add_clips_both_storage_types() {
        let residual = [i32::MIN, -300, -1, 0, 1, 300, i32::MAX, 5];
        let mut narrow = [10u8, 10, 0, 255, 254, 10, 10, 250];
        add_residual_in_place(&mut narrow, &residual, 255).unwrap();
        assert_eq!(narrow, [0, 0, 0, 255, 255, 255, 255, 255]);
        let mut wide = [10u16, 10, 0, 1023, 1022, 10, 10, 1020];
        add_residual_in_place(&mut wide, &residual, 1023).unwrap();
        assert_eq!(wide, [0, 0, 0, 1023, 1023, 310, 1023, 1023]);
    }

    #[test]
    fn u8_lane_groups_match_the_saturating_scalar_add() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let extremes = [
            i32::MIN,
            i32::MIN + 1,
            -70_000,
            -32_769,
            -256,
            256,
            32_768,
            i32::MAX,
        ];
        for len in 0..=40 {
            for max in [255, 200, 0] {
                let prediction: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                let residual: Vec<i32> = (0..len)
                    .map(|_| match next() % 4 {
                        0 => extremes[(next() % 8) as usize],
                        _ => (next() % 701) as i32 - 350,
                    })
                    .collect();
                let expected: Vec<u8> = prediction
                    .iter()
                    .zip(&residual)
                    .map(|(&pred, &res)| i32::from(pred).saturating_add(res).clamp(0, max) as u8)
                    .collect();
                let mut out = vec![0u8; len];
                add_residual_u8(Some(&prediction), &residual, max, &mut out);
                assert_eq!(out, expected, "len {len} max {max}");
                let mut in_place = prediction.clone();
                add_residual_u8(None, &residual, max, &mut in_place);
                assert_eq!(in_place, expected, "in place len {len} max {max}");
            }
        }
    }

    /// The constant byte adds match the scalar `Clip1` step for every width,
    /// leave the samples between rows alone, and stop at a short buffer.
    #[test]
    fn constant_u8_rows_match_the_scalar_add() {
        for width in 1..=40usize {
            for residual in [i32::MIN, -300, -255, -7, 0, 9, 255, 256, i32::MAX] {
                let (stride, height) = (width + 3, 5);
                let start: Vec<u8> = (0..stride * height).map(|i| (i * 37 % 256) as u8).collect();
                let mut expected = start.clone();
                for (i, sample) in expected.iter_mut().enumerate().skip(2) {
                    if (i - 2) % stride < width && (i - 2) / stride < height - 1 {
                        *sample = i32::from(*sample).saturating_add(residual).clamp(0, 255) as u8;
                    }
                }
                let mut got = start.clone();
                let span = 2 + (height - 2) * stride + width;
                add_constant_u8_rows(&mut got[..span], (2, stride, width, height), residual);
                assert_eq!(got, expected, "width {width} residual {residual}");
            }
        }
    }

    #[test]
    fn adds_residual_to_prediction() {
        let mut out = [0u8; 4];
        reconstruct_add_residual(
            &[100u8, 100, 100, 100],
            &[10, -10, 0, 5],
            BitDepth::Eight,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [110, 90, 100, 105]);
    }

    #[test]
    fn clip1_clamps_below_zero_and_above_max() {
        let mut out = [0u8; 3];
        reconstruct_add_residual(&[10u8, 250, 0], &[-50, 50, 255], BitDepth::Eight, &mut out)
            .unwrap();
        assert_eq!(out, [0, 255, 255]);
    }

    #[test]
    fn ten_bit_uses_u16_and_clamps_to_1023() {
        let mut out = [0u16; 2];
        reconstruct_add_residual(&[1000u16, 5], &[100, -50], BitDepth::Ten, &mut out).unwrap();
        assert_eq!(out, [1023, 0]);
    }

    #[test]
    fn is_total_for_extreme_residual() {
        let mut out = [0u8; 2];
        reconstruct_add_residual(
            &[128u8, 128],
            &[i32::MAX, i32::MIN],
            BitDepth::Eight,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [255, 0]);
    }

    /// The `u16` lane groups, also in place, must reproduce the scalar `Clip1`
    /// step exactly for every block length, over randomized predictions and
    /// residuals including the `i32` extremes.
    #[test]
    fn u16_lane_groups_match_the_scalar_reference() {
        let mut state = 0x1234_5678_9abc_def1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for len in 1..=64usize {
            for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
                let max = i32::from(bit_depth.max_sample());
                let prediction: Vec<u16> = (0..len)
                    .map(|_| (next() % (max as u64 + 1)) as u16)
                    .collect();
                let residual: Vec<i32> = (0..len)
                    .map(|index| match index % 8 {
                        0 => i32::MAX,
                        1 => i32::MIN,
                        _ => (next() as i32) >> (next() % 20) as i32,
                    })
                    .collect();
                let expected: Vec<u16> = prediction
                    .iter()
                    .zip(&residual)
                    .map(|(&pred, &res)| i32::from(pred).saturating_add(res).clamp(0, max) as u16)
                    .collect();
                let mut actual = vec![0u16; len];
                add_residual_u16(&prediction, &residual, max, &mut actual);
                assert_eq!(actual, expected, "len {len} {bit_depth:?}");
                let mut in_place = prediction.clone();
                add_residual_in_place(&mut in_place, &residual, max).unwrap();
                assert_eq!(in_place, expected, "in place, len {len} {bit_depth:?}");
            }
        }
    }

    #[test]
    fn rejects_sample_type_too_narrow_for_bit_depth() {
        let mut out = [0u8; 1];
        assert!(matches!(
            reconstruct_add_residual(&[0u8], &[0], BitDepth::Ten, &mut out),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn rejects_length_mismatch() {
        let mut out = [0u8; 3];
        assert!(matches!(
            reconstruct_add_residual(&[0u8; 4], &[0; 4], BitDepth::Eight, &mut out),
            Err(ReconError::ReconstructLengthMismatch {
                prediction_len: 4,
                residual_len: 4,
                out_len: 3
            })
        ));
    }

    #[test]
    fn rejects_prediction_sample_above_bit_depth() {
        let mut out = [0u16; 2];
        assert!(matches!(
            reconstruct_add_residual(&[10u16, 300], &[0, -100], BitDepth::Eight, &mut out),
            Err(ReconError::ReconstructPredictionOutOfRange {
                sample_index: 1,
                value: 300,
                max: 255
            })
        ));
        assert_eq!(out, [0, 0]);
    }
}
