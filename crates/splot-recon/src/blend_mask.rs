// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.3.30 mask blends of two compound predictors, and the § 7.13.3.28
//! difference-weight mask made in the same pass: `i16` predictors into `u8`
//! samples at eight bits, `i32` predictors into `u8` or `u16` samples at
//! either bit depth.

use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16, u16x8};

use crate::{BitDepth, ReconError, ReconSample, Result};

const LANES: usize = 8;

/// Blends two eight-bit compound predictors with a caller mask (§ 7.13.3.30):
/// `output[y][x] = Clip1(Round2(m * p0 + (64 - m) * p1, 6 + InterPostRound))`
/// with `m = mask[y][x]`, `p0 = preds[0][y][x]`, `p1 = preds[1][y][x]` and
/// `InterPostRound = 4` (§ 7.13.3.16), so the shift is 10 and `Clip1` clips
/// to `0..=255`.
///
/// It serves every non-uniform mask: the § 7.13.3.27 wedge mask, the chroma
/// mask of [`blend_diff_weighted_u8`], and the § 7.13.3.14 implicit mask as
/// the map `32 * get_mask` (`0`, `32` or `64`), because
/// `Round2(m * p0 + (2 - m) * p1, 5) == Round2(32m * p0 + (64 - 32m) * p1, 10)`.
///
/// Contract (the generic body and every arch-specific body give the same bytes):
/// - `preds` are § 7.13.3.18 compound predictors of an eight-bit reference
///   (`InterRound0 = 3`, compound `InterRound1 = 7`), stored as `i16` with no
///   offset: each value lies in `-5132..=9212`, the range that
///   `subpel_predict_block_compound_intermediate_into::<_, i16>` writes.
///   `mask` values lie in `0..=64`. For values outside these ranges the
///   output bytes are unspecified, but the call does not panic.
/// - The products are exact in `i32`; the rounding is `(v + 512) >> 10`,
///   then the result is clipped to `0..=255`.
/// - `w` and `h` are in samples; `w` is 1, 2, 4 or at least 8 (AV2 plane
///   blocks are powers of two). Strides are in
///   elements and at least `w`. Row `y` of each buffer starts at
///   `y * stride`; each buffer holds at least `(h - 1) * stride + w`
///   elements. No alignment is required. Only the `w` leading samples of
///   each output row are written. `output` does not alias the inputs.
///
/// # Errors
///
/// Returns [`ReconError::StrideTooSmall`] when a stride is less than `w`,
/// [`ReconError::BufferLengthMismatch`] when a buffer is too short, and
/// [`ReconError::ArithmeticOverflow`] when `w` is 3, 5, 6 or 7. Nothing is
/// written then.
pub fn blend_mask_u8(
    output: &mut [u8],
    output_stride: usize,
    preds: [&[i16]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    mask: &[u8],
    mask_stride: usize,
) -> Result<()> {
    check_mask_args(
        (output.len(), output_stride),
        preds.map(<[i16]>::len),
        pred_stride,
        (w, h),
        (mask.len(), mask_stride),
    )?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    baseline_simd!(simd => mask_rows((output, output_stride), preds, pred_stride, (w, h), (mask, mask_stride), |p0, p1, m, out| {
        let (p0, p1) = (i16x8::load_array_ref(simd, p0), i16x8::load_array_ref(simd, p1));
        *out = blend(p1, p0 - p1, load_bytes(simd, *m));
    }));
    Ok(())
}

/// Derives the § 7.13.3.28 difference-weight mask of two eight-bit compound
/// predictors and blends the luma block with it (§ 7.13.3.30), in one pass:
/// `m = Clip3(0, 64, 38 + Round2(Abs(p0 - p1), 4) / 16)`, `64 - m` when
/// `inverse` (`mask_type`), then the [`blend_mask_u8`] blend with `m`.
///
/// With `chroma_mask = Some((mask, mask_stride, (sub_x, sub_y)))` it also
/// writes the mask that § 7.13.3.30 gives the chroma planes, a
/// `(w >> sub_x) x (h >> sub_y)` block for [`blend_mask_u8`]: `m` itself for
/// `(0, 0)`, `Round2(m[y][2x] + m[y][2x + 1], 1)` for `(1, 0)` and the
/// `Round2` of the 2x2 sum by 2 for `(1, 1)`.
///
/// Contract: as [`blend_mask_u8`] for `preds`, `output` and the strides, and:
/// - `w` is a multiple of 8 (a masked compound block has `Min(w, h) >= 8`,
///   `is_comp_ref_allowed` and `is_thin_4xn_nx4_block` in § 5.20.5),
///   `h >= 1`, and `h` is even when `sub_y == 1`;
/// - `(sub_x, sub_y)` is `(0, 0)`, `(1, 0)` or `(1, 1)`;
/// - the chroma mask holds at least `((h >> sub_y) - 1) * mask_stride +
///   (w >> sub_x)` bytes, `mask_stride >= w >> sub_x`, every value is in
///   `0..=64`, and it does not alias the other buffers.
///
/// # Errors
///
/// Returns [`ReconError::StrideTooSmall`] or
/// [`ReconError::BufferLengthMismatch`] as [`blend_mask_u8`], and
/// [`ReconError::ArithmeticOverflow`] when `w`, `h` or the subsampling break
/// the contract. Nothing is written then.
pub fn blend_diff_weighted_u8(
    output: &mut [u8],
    output_stride: usize,
    preds: [&[i16]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    inverse: bool,
    chroma_mask: Option<(&mut [u8], usize, (u32, u32))>,
) -> Result<()> {
    let chroma = check_diff_args(
        (output.len(), output_stride),
        preds.map(<[i16]>::len),
        pred_stride,
        (w, h),
        chroma_mask,
    )?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    baseline_simd!(simd => {
        let flip = i16x8::splat(simd, if inverse { 64 } else { 0 });
        diff_rows((output, output_stride), preds, pred_stride, (w, h), chroma, |p0, p1, out| {
            narrow_weighted(simd, (p0, p1), out, flip)
        });
    });
    Ok(())
}

/// [`blend_mask_u8`] of `i32` compound predictors at either bit depth, into
/// `u8` or `u16` samples: `Clip1` clips to `0..=bit_depth.max_sample()`.
///
/// Contract: as [`blend_mask_u8`], except that `preds` are § 7.13.3.18
/// compound predictors of a `bit_depth` reference stored as `i32` with no
/// offset (the storage of the `i32` compound kernels and of the warp
/// kernels): `-5132..=9212` at eight bits and `-20588..=36956` at ten bits
/// (horizontal pass `Round2(-56 * 1023, 3)..=Round2(184 * 1023, 3)`, then the
/// vertical pass of the same taps with `Round2(.., 7)`), which `i16` cannot
/// hold. `O` is `u8` only at eight bits.
///
/// # Errors
///
/// As [`blend_mask_u8`], and [`ReconError::SampleTypeUnsupportedBitDepth`]
/// when `O` cannot hold `bit_depth` samples.
pub fn blend_mask_wide<O: ReconSample>(
    output: &mut [O],
    output_stride: usize,
    preds: [&[i32]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    (mask, mask_stride): (&[u8], usize),
    bit_depth: BitDepth,
) -> Result<()> {
    check_mask_args(
        (output.len(), output_stride),
        preds.map(<[i32]>::len),
        pred_stride,
        (w, h),
        (mask.len(), mask_stride),
    )?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    let max = i32::from(bit_depth.max_sample());
    macro_rules! run {
        ($output:expr) => {
            baseline_simd!(simd => mask_rows(($output, output_stride), preds, pred_stride, (w, h), (mask, mask_stride), |p0, p1, m, out| {
                WideSample::put(wide_blend(simd, (p0, p1), load_bytes(simd, *m), max), out);
            }))
        };
    }
    match wide_output(output, bit_depth)? {
        WideOutput::Eight(output) => run!(output),
        WideOutput::Sixteen(output) => run!(output),
    }
    Ok(())
}

/// [`blend_diff_weighted_u8`] of `i32` compound predictors at either bit
/// depth, into `u8` or `u16` samples: `diff = Round2(Abs(p0 - p1),
/// (BitDepth - 8) + InterPostRound)` (§ 7.13.3.28) and the `Clip1` of
/// `bit_depth`.
///
/// Contract: as [`blend_diff_weighted_u8`] for the shapes and the chroma mask,
/// and as [`blend_mask_wide`] for `preds` and `O`.
///
/// # Errors
///
/// As [`blend_diff_weighted_u8`] and [`blend_mask_wide`].
pub fn blend_diff_weighted_wide<O: ReconSample>(
    output: &mut [O],
    output_stride: usize,
    preds: [&[i32]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    (inverse, bit_depth): (bool, BitDepth),
    chroma_mask: Option<(&mut [u8], usize, (u32, u32))>,
) -> Result<()> {
    let chroma = check_diff_args(
        (output.len(), output_stride),
        preds.map(<[i32]>::len),
        pred_stride,
        (w, h),
        chroma_mask,
    )?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    let max = i32::from(bit_depth.max_sample());
    let diff_round = u32::from(bit_depth.bits() - 8) + 4;
    let weights = (1 << (diff_round - 1), diff_round + 4);
    macro_rules! run {
        ($output:expr) => {
            baseline_simd!(simd => {
                let flip = i16x8::splat(simd, if inverse { 64 } else { 0 });
                diff_rows(($output, output_stride), preds, pred_stride, (w, h), chroma, |p0, p1, out| {
                    let m = wide_weight(simd, (p0, p1), flip, weights);
                    WideSample::put(wide_blend(simd, (p0, p1), m, max), out);
                    m
                })
            })
        };
    }
    match wide_output(output, bit_depth)? {
        WideOutput::Eight(output) => run!(output),
        WideOutput::Sixteen(output) => run!(output),
    }
    Ok(())
}

fn check_mask_args(
    output: (usize, usize),
    preds: [usize; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    mask: (usize, usize),
) -> Result<()> {
    check_rect(output.0, output.1, w, h)?;
    check_rect(preds[0], pred_stride, w, h)?;
    check_rect(preds[1], pred_stride, w, h)?;
    check_rect(mask.0, mask.1, w, h)?;
    if w < LANES && !matches!(w, 0 | 1 | 2 | 4) {
        return Err(ReconError::ArithmeticOverflow {
            context: "mask blend width",
        });
    }
    Ok(())
}

/// Checks the shapes of a difference-weight blend; returns the chroma mask,
/// its stride and `sub_x + sub_y`.
fn check_diff_args(
    output: (usize, usize),
    preds: [usize; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    chroma_mask: Option<(&mut [u8], usize, (u32, u32))>,
) -> Result<Option<(&mut [u8], usize, u32)>> {
    check_rect(output.0, output.1, w, h)?;
    check_rect(preds[0], pred_stride, w, h)?;
    check_rect(preds[1], pred_stride, w, h)?;
    let shape_error = |context| Err(ReconError::ArithmeticOverflow { context });
    if w % LANES != 0 {
        return shape_error("diff-weighted blend width");
    }
    let Some((mask, stride, (sub_x, sub_y))) = chroma_mask else {
        return Ok(None);
    };
    if sub_x > 1 || sub_y > sub_x {
        return shape_error("diff-weighted chroma mask subsampling");
    }
    if sub_y == 1 && h % 2 != 0 {
        return shape_error("diff-weighted chroma mask height");
    }
    check_rect(mask.len(), stride, w >> sub_x, h >> sub_y)?;
    Ok(Some((mask, stride, sub_x + sub_y)))
}

fn check_rect(len: usize, stride: usize, w: usize, h: usize) -> Result<()> {
    if w == 0 || h == 0 {
        return Ok(());
    }
    if stride < w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: stride,
            storage_width: w,
        });
    }
    let expected = (h - 1)
        .checked_mul(stride)
        .and_then(|rows| rows.checked_add(w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "mask blend rectangle",
        })?;
    if len < expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: len,
        });
    }
    Ok(())
}

enum WideOutput<'a> {
    Eight(&'a mut [u8]),
    Sixteen(&'a mut [u16]),
}

fn wide_output<O: ReconSample>(output: &mut [O], bit_depth: BitDepth) -> Result<WideOutput<'_>> {
    let unsupported = ReconError::SampleTypeUnsupportedBitDepth {
        sample_type: O::TYPE_NAME,
        bit_depth,
    };
    if O::u8_slice(&[]).is_some() {
        return match (bit_depth, O::u8_slice_mut(output)) {
            (BitDepth::Eight, Some(output)) => Ok(WideOutput::Eight(output)),
            _ => Err(unsupported),
        };
    }
    O::u16_slice_mut(output)
        .map(WideOutput::Sixteen)
        .ok_or(unsupported)
}

/// `Clip1(Round2(64 * p1 + m * (p0 - p1), 10))` of eight lanes, `diff = p0 - p1`.
#[inline]
fn blend<S: Simd>(p1: i16x8<S>, diff: i16x8<S>, mask: i16x8<S>) -> [u8; LANES] {
    let (low, high) = mask.widening_mul(diff);
    let (base_low, base_high) = p1.widen();
    let round = |product: i32x4<S>, base: i32x4<S>| ((base << 6) + product + 512) >> 10;
    let value: i16x8<S> = round(low, base_low).narrow(round(high, base_high));
    to_bytes(value.max(0).min(255).bitcast())
}

/// [`blend`] of `i32` predictors, clipped to `0..=max`.
#[inline]
fn wide_blend<S: Simd>(
    simd: S,
    (p0, p1): (&[i32; LANES], &[i32; LANES]),
    mask: i16x8<S>,
    max: i32,
) -> [i32x4<S>; 2] {
    let (low, high) = mask.widen();
    let half = |half: usize, mask: i32x4<S>| {
        let p0 = i32x4::from_slice(simd, &p0[4 * half..4 * half + 4]);
        let p1 = i32x4::from_slice(simd, &p1[4 * half..4 * half + 4]);
        (((p1 << 6) + mask * (p0 - p1) + 512) >> 10).max(0).min(max)
    };
    [half(0, low), half(1, high)]
}

/// The § 7.13.3.28 weight `|flip - Min(38 + ((|p0 - p1| + round) >> shift), 64)|`
/// of eight `i32` lanes.
#[inline]
fn wide_weight<S: Simd>(
    simd: S,
    (p0, p1): (&[i32; LANES], &[i32; LANES]),
    flip: i16x8<S>,
    (round, shift): (i32, u32),
) -> i16x8<S> {
    let half = |half: usize| {
        let p0 = i32x4::from_slice(simd, &p0[4 * half..4 * half + 4]);
        let p1 = i32x4::from_slice(simd, &p1[4 * half..4 * half + 4]);
        ((((p0 - p1).abs() + round) >> shift) + 38).min(64)
    };
    (flip - half(0).narrow(half(1))).abs()
}

/// A sample type that [`blend_mask_wide`] stores.
trait WideSample: Copy + Default {
    fn put<S: Simd>(values: [i32x4<S>; 2], output: &mut [Self; LANES]);
}

impl WideSample for u8 {
    #[inline]
    fn put<S: Simd>(values: [i32x4<S>; 2], output: &mut [u8; LANES]) {
        let values: i16x8<S> = values[0].narrow(values[1]);
        *output = to_bytes(values.bitcast());
    }
}

impl WideSample for u16 {
    #[inline]
    fn put<S: Simd>(values: [i32x4<S>; 2], output: &mut [u16; LANES]) {
        let values: i16x8<S> = values[0].narrow(values[1]);
        *output = values.bitcast::<u16x8<S>>().to_array();
    }
}

#[inline]
fn load_bytes<S: Simd>(simd: S, bytes: [u8; LANES]) -> i16x8<S> {
    let mut lanes = [0; 2 * LANES];
    lanes[..LANES].copy_from_slice(&bytes); // splot-copy-ok: eight mask bytes into one vector
    let (low, _) = u8x16::load_array(simd, lanes).widen();
    low.bitcast()
}

#[inline]
fn to_bytes<S: Simd>(values: u16x8<S>) -> [u8; LANES] {
    let bytes = values.narrow(values).to_array();
    core::array::from_fn(|lane| bytes[lane])
}

/// Runs `chunk` on every eight-lane chunk of the `w x h` block: whole chunks
/// in place, an overlapping last chunk when `w % 8 != 0`, and `8 / w` rows
/// gathered into one chunk when `w` is 1, 2 or 4.
#[inline]
fn mask_rows<P: Copy + Default, O: Copy + Default>(
    (output, output_stride): (&mut [O], usize),
    preds: [&[P]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    (mask, mask_stride): (&[u8], usize),
    chunk: impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    let rows = preds[0]
        .chunks(pred_stride)
        .zip(preds[1].chunks(pred_stride))
        .zip(
            mask.chunks(mask_stride)
                .zip(output.chunks_mut(output_stride)),
        )
        .take(h);
    match w {
        1 => small_rows::<P, O, 1, 8>(rows, chunk),
        2 => small_rows::<P, O, 2, 4>(rows, chunk),
        4 => small_rows::<P, O, 4, 2>(rows, chunk),
        _ => {
            for ((row0, row1), (masks, out)) in rows {
                let (row0, row1, masks, out) = (&row0[..w], &row1[..w], &masks[..w], &mut out[..w]);
                let chunks = row0.as_chunks().0.iter().zip(row1.as_chunks().0);
                let outputs = masks.as_chunks().0.iter().zip(out.as_chunks_mut().0);
                for ((p0, p1), (m, out)) in chunks.zip(outputs) {
                    chunk(p0, p1, m, out);
                }
                if w % LANES != 0
                    && let (Some(p0), Some(p1), Some(m), Some(out)) = (
                        row0.last_chunk(),
                        row1.last_chunk(),
                        masks.last_chunk(),
                        out.last_chunk_mut(),
                    )
                {
                    chunk(p0, p1, m, out);
                }
            }
        }
    }
}

/// [`mask_rows`] of a `W`-wide block, `R = 8 / W` rows per chunk.
#[inline]
fn small_rows<
    'a,
    P: Copy + Default + 'a,
    O: Copy + Default + 'a,
    const W: usize,
    const R: usize,
>(
    rows: impl Iterator<Item = ((&'a [P], &'a [P]), (&'a [u8], &'a mut [O]))>,
    chunk: impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    let mut rows = rows.peekable();
    while rows.peek().is_some() {
        let (mut p0, mut p1, mut m) = ([P::default(); LANES], [P::default(); LANES], [0; LANES]);
        let mut outputs: [Option<&mut [O; W]>; R] = [const { None }; R];
        for (lane, ((row0, row1), (masks, out))) in (0..LANES).step_by(W).zip(rows.by_ref().take(R))
        {
            if let (Some(a), Some(b), Some(c)) = (
                row0.first_chunk::<W>(),
                row1.first_chunk::<W>(),
                masks.first_chunk::<W>(),
            ) {
                p0[lane..lane + W].copy_from_slice(a); // splot-copy-ok: gather a short row
                p1[lane..lane + W].copy_from_slice(b); // splot-copy-ok: gather a short row
                m[lane..lane + W].copy_from_slice(c); // splot-copy-ok: gather a short row
            }
            outputs[lane / W] = out.first_chunk_mut::<W>();
        }
        let mut values = [O::default(); LANES];
        chunk(&p0, &p1, &m, &mut values);
        for (out, values) in outputs.into_iter().zip(values.as_chunks::<W>().0) {
            if let Some(out) = out {
                *out = *values;
            }
        }
    }
}

/// Runs `weighted` on every eight-lane chunk of the `w x h` block (`w % 8 ==
/// 0`) and stores the weights it returns as the chroma mask of `chroma`: the
/// mask, its stride and `sub_x + sub_y`.
#[inline]
fn diff_rows<S: Simd, P: Copy, O: Copy>(
    (output, output_stride): (&mut [O], usize),
    preds: [&[P]; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    chroma: Option<(&mut [u8], usize, u32)>,
    weighted: impl Fn(&[P; LANES], &[P; LANES], &mut [O; LANES]) -> i16x8<S>,
) {
    let rows = preds[0]
        .chunks(pred_stride)
        .zip(preds[1].chunks(pred_stride))
        .map(|(row0, row1)| chunks(row0, w).iter().zip(chunks(row1, w)))
        .zip(
            output
                .chunks_mut(output_stride)
                .map(|out| chunks_mut(out, w)),
        )
        .take(h);
    let weights = |(p0, p1): (&[P; LANES], &[P; LANES]), out| weighted(p0, p1, out);
    match chroma {
        None => {
            for (preds, out) in rows {
                for (preds, out) in preds.zip(out) {
                    weights(preds, out);
                }
            }
        }
        Some((mask, stride, 0)) => {
            for ((preds, out), mask) in rows.zip(mask.chunks_mut(stride)) {
                for ((preds, out), mask) in preds.zip(out).zip(chunks_mut(mask, w)) {
                    *mask = to_bytes(weights(preds, out).bitcast());
                }
            }
        }
        Some((mask, stride, 1)) => {
            for ((preds, out), mask) in rows.zip(mask.chunks_mut(stride)) {
                let masks = mask[..w / 2].as_chunks_mut::<{ LANES / 2 }>().0;
                for ((preds, out), mask) in preds.zip(out).zip(masks) {
                    let sums = weights(preds, out).pairwise_widening_add();
                    store_quarter((sums + 1) >> 1, mask);
                }
            }
        }
        Some((mask, stride, _)) => {
            let mut rows = rows;
            for mask in mask.chunks_mut(stride).take(h / 2) {
                let (Some((upper, top)), Some((lower, bottom))) = (rows.next(), rows.next()) else {
                    return;
                };
                let masks = mask[..w / 2].as_chunks_mut::<{ LANES / 2 }>().0;
                let pairs = upper.zip(top).zip(lower.zip(bottom));
                for ((upper, lower), mask) in pairs.zip(masks) {
                    let sums = weights(upper.0, upper.1).pairwise_widening_add()
                        + weights(lower.0, lower.1).pairwise_widening_add();
                    store_quarter((sums + 2) >> 2, mask);
                }
            }
        }
    }
}

/// The § 7.13.3.28 weight of eight `i16` lanes, `|flip - m|` (`flip` is 64
/// for `mask_type`, else 0), after blending them into `out`.
#[inline]
fn narrow_weighted<S: Simd>(
    simd: S,
    (p0, p1): (&[i16; LANES], &[i16; LANES]),
    out: &mut [u8; LANES],
    flip: i16x8<S>,
) -> i16x8<S> {
    let (p0, p1) = (
        i16x8::load_array_ref(simd, p0),
        i16x8::load_array_ref(simd, p1),
    );
    let diff = p0 - p1;
    let m = (flip - (((diff.abs() + 8) >> 8) + 38).min(64)).abs();
    *out = blend(p1, diff, m);
    m
}

#[inline]
fn chunks<P>(row: &[P], w: usize) -> &[[P; LANES]] {
    row[..w].as_chunks().0
}

#[inline]
fn chunks_mut<O>(row: &mut [O], w: usize) -> &mut [[O; LANES]] {
    row[..w].as_chunks_mut().0
}

#[inline]
fn store_quarter<S: Simd>(values: i32x4<S>, output: &mut [u8; LANES / 2]) {
    let bytes = to_bytes(values.narrow(values).bitcast());
    *output = core::array::from_fn(|lane| bytes[lane]);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn spec_blend(p0: i16, p1: i16, m: i32) -> u8 {
        let value = m * i32::from(p0) + (64 - m) * i32::from(p1);
        ((value + 512) >> 10).clamp(0, 255) as u8
    }

    fn spec_weight(p0: i16, p1: i16, inverse: bool) -> i32 {
        let diff = (i32::from(p0) - i32::from(p1)).abs();
        let m = (38 + ((diff + 8) >> 4) / 16).clamp(0, 64);
        if inverse { 64 - m } else { m }
    }

    fn samples(len: usize, seed: usize) -> Vec<i16> {
        let edges = [-5132, 9212, 0, -1, 4080, 255 * 16];
        (0..len)
            .map(|i| match (i * 7 + seed) % 11 {
                0..=5 => edges[(i + seed) % edges.len()],
                k => ((i * 2_654_435_761 + seed * 97 + k) % 14_345) as i16 - 5132,
            })
            .collect()
    }

    #[test]
    fn mask_blend_matches_the_spec_formula_at_every_width() {
        for w in [1, 2, 4].into_iter().chain(8..=20).chain([31, 32, 40]) {
            for h in [1, 2, 3, 5, 8] {
                let (stride, len) = (w + 2, (h - 1) * (w + 2) + w);
                let preds = [samples(len, w), samples(len, h + 3)];
                let mask: Vec<u8> = (0..len).map(|i| ((i * 13 + w) % 65) as u8).collect();
                let mut output = vec![0x5a; len + 2];
                let preds_ref = [preds[0].as_slice(), preds[1].as_slice()];
                blend_mask_u8(
                    &mut output,
                    stride,
                    preds_ref,
                    stride,
                    (w, h),
                    &mask,
                    stride,
                )
                .expect("blend");
                for (i, &got) in output.iter().enumerate() {
                    let want = if i < len && i % stride < w {
                        spec_blend(preds[0][i], preds[1][i], i32::from(mask[i]))
                    } else {
                        0x5a
                    };
                    assert_eq!(got, want, "{w}x{h} sample {i}");
                }
            }
        }
    }

    #[test]
    fn diff_weighted_blend_matches_the_spec_formula() {
        for (w, h) in [(8, 2), (8, 8), (16, 6), (24, 4), (64, 2)] {
            for inverse in [false, true] {
                for sub @ (sub_x, sub_y) in [(0, 0), (1, 0), (1, 1)] {
                    let preds = [samples(w * h, w + h), samples(w * h, 5)];
                    let m = |i: usize| spec_weight(preds[0][i], preds[1][i], inverse);
                    let mut output = vec![0; w * h];
                    let mut chroma = vec![0; (w >> sub_x) * (h >> sub_y)];
                    let preds_ref = [preds[0].as_slice(), preds[1].as_slice()];
                    let mask = Some((chroma.as_mut_slice(), w >> sub_x, sub));
                    blend_diff_weighted_u8(&mut output, w, preds_ref, w, (w, h), inverse, mask)
                        .expect("blend");
                    for (i, &got) in output.iter().enumerate() {
                        assert_eq!(got, spec_blend(preds[0][i], preds[1][i], m(i)));
                    }
                    for (i, &got) in chroma.iter().enumerate() {
                        let (x, y) = ((i % (w >> sub_x)) << sub_x, (i / (w >> sub_x)) << sub_y);
                        let sum: i32 = (0..=sub_y as usize)
                            .flat_map(|dy| (0..=sub_x as usize).map(move |dx| (dx, dy)))
                            .map(|(dx, dy)| m((y + dy) * w + x + dx))
                            .sum();
                        let shift = sub_x + sub_y;
                        let want = (sum + ((1 << shift) >> 1)) >> shift;
                        assert_eq!(i32::from(got), want, "{w}x{h} {inverse} {sub:?} {i}");
                    }
                }
            }
        }
    }

    fn wide_samples(len: usize, seed: usize, bit_depth: BitDepth) -> Vec<i32> {
        let (low, high) = if bit_depth == BitDepth::Eight {
            (-5132, 9212)
        } else {
            (-20588, 36956)
        };
        let edges = [low, high, 0, -1, high / 3, low / 2];
        (0..len)
            .map(|i| match (i * 7 + seed) % 11 {
                0..=5 => edges[(i + seed) % edges.len()],
                k => ((i * 2_654_435_761 + seed * 97 + k) % (high - low + 1) as usize) as i32 + low,
            })
            .collect()
    }

    fn spec_wide(p0: i32, p1: i32, m: i32, bit_depth: BitDepth) -> u16 {
        let value = (m * p0 + (64 - m) * p1 + 512) >> 10;
        value.clamp(0, i32::from(bit_depth.max_sample())) as u16
    }

    fn wide_case<O: ReconSample>(bit_depth: BitDepth) {
        for (w, h) in [(1, 8), (2, 3), (4, 6), (8, 2), (12, 3), (16, 4), (64, 2)] {
            let preds = [
                wide_samples(w * h, w, bit_depth),
                wide_samples(w * h, h + 7, bit_depth),
            ];
            let refs = [preds[0].as_slice(), preds[1].as_slice()];
            let mask: Vec<u8> = (0..w * h).map(|i| ((i * 13 + w) % 65) as u8).collect();
            let mut output = vec![O::default(); w * h];
            blend_mask_wide(&mut output, w, refs, w, (w, h), (&mask, w), bit_depth).expect("blend");
            for (i, got) in output.iter().enumerate() {
                let want = spec_wide(preds[0][i], preds[1][i], i32::from(mask[i]), bit_depth);
                assert_eq!(got.to_u16(), want, "mask {bit_depth:?} {w}x{h} {i}");
            }
            if w % LANES != 0 {
                continue;
            }
            let diff_round = u32::from(bit_depth.bits() - 8) + 4;
            for inverse in [false, true] {
                for sub @ (sub_x, sub_y) in [(0, 0), (1, 0), (1, 1)] {
                    let m = |i: usize| {
                        let diff = (preds[0][i] - preds[1][i]).abs();
                        let diff = (diff + (1 << (diff_round - 1))) >> diff_round;
                        let m = (38 + diff / 16).clamp(0, 64);
                        if inverse { 64 - m } else { m }
                    };
                    let cw = w >> sub_x;
                    let mut chroma = vec![0; cw * (h >> sub_y)];
                    let mask = Some((chroma.as_mut_slice(), cw, sub));
                    let shape = (inverse, bit_depth);
                    blend_diff_weighted_wide(&mut output, w, refs, w, (w, h), shape, mask)
                        .expect("diff-weighted blend");
                    for (i, got) in output.iter().enumerate() {
                        let want = spec_wide(preds[0][i], preds[1][i], m(i), bit_depth);
                        assert_eq!(got.to_u16(), want, "diff {bit_depth:?} {w}x{h} {i}");
                    }
                    for (i, &got) in chroma.iter().enumerate() {
                        let (x, y) = ((i % cw) << sub_x, (i / cw) << sub_y);
                        let sum: i32 = (0..=sub_y as usize)
                            .flat_map(|dy| (0..=sub_x as usize).map(move |dx| (dx, dy)))
                            .map(|(dx, dy)| m((y + dy) * w + x + dx))
                            .sum();
                        let shift = sub_x + sub_y;
                        assert_eq!(i32::from(got), (sum + ((1 << shift) >> 1)) >> shift);
                    }
                }
            }
        }
    }

    #[test]
    fn wide_blends_match_the_spec_formula_at_both_bit_depths() {
        wide_case::<u8>(BitDepth::Eight);
        wide_case::<u16>(BitDepth::Eight);
        wide_case::<u16>(BitDepth::Ten);
        let mut output = [0u8; 8];
        let preds = [[0i32; 8].as_slice(), [0; 8].as_slice()];
        let ten = blend_mask_wide(
            &mut output,
            8,
            preds,
            8,
            (8, 1),
            (&[0; 8], 8),
            BitDepth::Ten,
        );
        assert!(ten.is_err());
    }
}
