// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.3.30 mask blends of two compound predictors, and the § 7.13.3.28
//! difference-weight mask made in the same pass: `i16` predictors into `u8`
//! samples at eight bits, `i32` predictors into `u8` or `u16` samples at
//! either bit depth.
//!
//! Each blend has a checked entry with strided inputs and a `_packed` entry,
//! the decoder's, whose predictors and mask are packed (`w` elements per
//! row) and whose caller validates the shapes. The packed entries do not
//! panic and write only inside their output buffers for any input; for
//! inputs outside their contract the written values are unspecified.

use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16, u16x8};

use crate::{BitDepth, ReconError, ReconSample, Result};

const LANES: usize = 8;

/// A chroma mask, its stride and `(sub_x, sub_y)`.
type ChromaMask<'a> = Option<(&'a mut [u8], usize, (u32, u32))>;

/// Eight lanes of each predictor and of the output.
type Chunk<'a, P, O> = ((&'a [P; LANES], &'a [P; LANES]), &'a mut [O; LANES]);

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
    let packed = pred_stride == w && mask_stride == w;
    row_groups(packed, h, 1, |y, rows| {
        let output = rest_mut(output, y * output_stride);
        let preds = preds.map(|pred| rest(pred, y * pred_stride));
        blend_mask_u8_packed(
            output,
            output_stride,
            preds,
            rest(mask, y * mask_stride),
            (w, rows),
        );
    });
    Ok(())
}

/// [`blend_mask_u8`] of packed predictors and mask, without checks: the
/// decoder's entry.
///
/// Contract (the caller validates it; an arch-specific body keeps this
/// signature and gives the same bytes):
/// - value ranges, arithmetic and `w` as [`blend_mask_u8`];
/// - `preds[0]`, `preds[1]` and `mask` are packed: row `y` starts at
///   `y * w`, and each holds at least `w * h` elements;
/// - `output_stride >= w`, row `y` of `output` starts at
///   `y * output_stride`, `output` holds at least
///   `(h - 1) * output_stride + w` elements, and only the `w` leading
///   samples of each row are written;
/// - no alignment; `output` does not alias the inputs (`&mut`).
pub fn blend_mask_u8_packed(
    output: &mut [u8],
    output_stride: usize,
    preds: [&[i16]; 2],
    mask: &[u8],
    (w, h): (usize, usize),
) {
    baseline_simd!(simd => mask_block((output, output_stride), preds, mask, (w, h), |p0, p1, m, out| {
        let (p0, p1) = (i16x8::load_array_ref(simd, p0), i16x8::load_array_ref(simd, p1));
        *out = blend(p1, p0 - p1, load_bytes(simd, *m));
    }));
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
    let mut chroma = check_diff_args(
        (output.len(), output_stride),
        preds.map(<[i16]>::len),
        pred_stride,
        (w, h),
        chroma_mask,
    )?;
    let group = chroma.as_ref().map_or(1, |&(_, _, (_, sub_y))| 1 << sub_y);
    row_groups(pred_stride == w, h, group, |y, rows| {
        let output = rest_mut(output, y * output_stride);
        let preds = preds.map(|pred| rest(pred, y * pred_stride));
        let chroma = chroma_rows(&mut chroma, y);
        blend_diff_weighted_u8_packed(output, output_stride, preds, (w, rows), inverse, chroma);
    });
    Ok(())
}

/// [`blend_diff_weighted_u8`] of packed predictors, without checks: the
/// decoder's entry.
///
/// Contract (the caller validates it; an arch-specific body keeps this
/// signature and gives the same bytes): as [`blend_diff_weighted_u8`] for
/// the value ranges, `w`, `h`, the subsampling and the chroma mask (whose
/// rows start at `y * mask_stride`), and as [`blend_mask_u8_packed`] for the
/// packed `preds` and for `output`.
pub fn blend_diff_weighted_u8_packed(
    output: &mut [u8],
    output_stride: usize,
    preds: [&[i16]; 2],
    (w, h): (usize, usize),
    inverse: bool,
    chroma_mask: Option<(&mut [u8], usize, (u32, u32))>,
) {
    baseline_simd!(simd => {
        let flip = i16x8::splat(simd, if inverse { 64 } else { 0 });
        diff_block((output, output_stride), preds, (w, h), chroma_mask, |p0, p1, out| {
            narrow_weighted(simd, (p0, p1), out, flip)
        });
    });
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
    check_wide_output::<O>(bit_depth)?;
    let packed = pred_stride == w && mask_stride == w;
    row_groups(packed, h, 1, |y, rows| {
        let output = rest_mut(output, y * output_stride);
        let preds = preds.map(|pred| rest(pred, y * pred_stride));
        let mask = rest(mask, y * mask_stride);
        blend_mask_wide_packed(output, output_stride, preds, mask, (w, rows), bit_depth);
    });
    Ok(())
}

/// [`blend_mask_wide`] of packed predictors and mask, without checks: the
/// decoder's entry.
///
/// Contract (the caller validates it; an arch-specific body keeps this
/// signature and gives the same bytes): as [`blend_mask_wide`] for the value
/// ranges and `O` (`u8` only at eight bits; another `O` writes nothing), and
/// as [`blend_mask_u8_packed`] for the shapes.
pub fn blend_mask_wide_packed<O: ReconSample>(
    output: &mut [O],
    output_stride: usize,
    preds: [&[i32]; 2],
    mask: &[u8],
    (w, h): (usize, usize),
    bit_depth: BitDepth,
) {
    let max = i32::from(bit_depth.max_sample());
    macro_rules! run {
        ($output:expr) => {
            baseline_simd!(simd => mask_block(($output, output_stride), preds, mask, (w, h), |p0, p1, m, out| {
                WideSample::put(wide_blend(simd, (p0, p1), load_bytes(simd, *m), max), out);
            }))
        };
    }
    if let Some(output) = O::u8_slice_mut(&mut *output) {
        run!(output);
    } else if let Some(output) = O::u16_slice_mut(output) {
        run!(output);
    }
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
    let mut chroma = check_diff_args(
        (output.len(), output_stride),
        preds.map(<[i32]>::len),
        pred_stride,
        (w, h),
        chroma_mask,
    )?;
    check_wide_output::<O>(bit_depth)?;
    let group = chroma.as_ref().map_or(1, |&(_, _, (_, sub_y))| 1 << sub_y);
    row_groups(pred_stride == w, h, group, |y, rows| {
        let output = rest_mut(output, y * output_stride);
        let preds = preds.map(|pred| rest(pred, y * pred_stride));
        let (shape, chroma) = ((inverse, bit_depth), chroma_rows(&mut chroma, y));
        blend_diff_weighted_wide_packed(output, output_stride, preds, (w, rows), shape, chroma);
    });
    Ok(())
}

/// [`blend_diff_weighted_wide`] of packed predictors, without checks: the
/// decoder's entry.
///
/// Contract (the caller validates it; an arch-specific body keeps this
/// signature and gives the same bytes): as [`blend_diff_weighted_u8_packed`]
/// for the shapes and as [`blend_mask_wide_packed`] for `preds` and `O`.
pub fn blend_diff_weighted_wide_packed<O: ReconSample>(
    output: &mut [O],
    output_stride: usize,
    preds: [&[i32]; 2],
    (w, h): (usize, usize),
    (inverse, bit_depth): (bool, BitDepth),
    chroma_mask: Option<(&mut [u8], usize, (u32, u32))>,
) {
    let max = i32::from(bit_depth.max_sample());
    let diff_round = u32::from(bit_depth.bits() - 8) + 4;
    let weights = (1 << (diff_round - 1), diff_round + 4);
    macro_rules! run {
        ($output:expr) => {
            baseline_simd!(simd => {
                let flip = i16x8::splat(simd, if inverse { 64 } else { 0 });
                diff_block(($output, output_stride), preds, (w, h), chroma_mask, |p0, p1, out| {
                    let m = wide_weight(simd, (p0, p1), flip, weights);
                    WideSample::put(wide_blend(simd, (p0, p1), m, max), out);
                    m
                })
            })
        };
    }
    if let Some(output) = O::u8_slice_mut(&mut *output) {
        run!(output);
    } else if let Some(output) = O::u16_slice_mut(output) {
        run!(output);
    }
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

/// Checks the shapes of a difference-weight blend; returns the chroma mask.
fn check_diff_args(
    output: (usize, usize),
    preds: [usize; 2],
    pred_stride: usize,
    (w, h): (usize, usize),
    chroma_mask: ChromaMask<'_>,
) -> Result<ChromaMask<'_>> {
    check_rect(output.0, output.1, w, h)?;
    check_rect(preds[0], pred_stride, w, h)?;
    check_rect(preds[1], pred_stride, w, h)?;
    let shape_error = |context| Err(ReconError::ArithmeticOverflow { context });
    if w % LANES != 0 {
        return shape_error("diff-weighted blend width");
    }
    let Some((mask, stride, sub @ (sub_x, sub_y))) = chroma_mask else {
        return Ok(None);
    };
    if sub_x > 1 || sub_y > sub_x {
        return shape_error("diff-weighted chroma mask subsampling");
    }
    if sub_y == 1 && h % 2 != 0 {
        return shape_error("diff-weighted chroma mask height");
    }
    check_rect(mask.len(), stride, w >> sub_x, h >> sub_y)?;
    Ok(Some((mask, stride, sub)))
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

fn check_wide_output<O: ReconSample>(bit_depth: BitDepth) -> Result<()> {
    let eight = O::u8_slice(&[]).is_some();
    if (eight && bit_depth != BitDepth::Eight) || (!eight && O::u16_slice(&[]).is_none()) {
        return Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: O::TYPE_NAME,
            bit_depth,
        });
    }
    Ok(())
}

/// Runs `blend(y, rows)` on the whole block when `packed`, else on each
/// group of `group` rows from row `y`.
fn row_groups(packed: bool, h: usize, group: usize, mut blend: impl FnMut(usize, usize)) {
    if packed {
        return blend(0, h);
    }
    for y in (0..h).step_by(group) {
        blend(y, group.min(h - y));
    }
}

fn rest<V>(buffer: &[V], start: usize) -> &[V] {
    buffer.get(start..).unwrap_or_default()
}

fn rest_mut<V>(buffer: &mut [V], start: usize) -> &mut [V] {
    buffer.get_mut(start..).unwrap_or_default()
}

/// The chroma mask from the row of luma row `y`.
fn chroma_rows<'a>(chroma: &'a mut ChromaMask<'_>, y: usize) -> ChromaMask<'a> {
    let (mask, stride, sub) = chroma.as_mut()?;
    Some((rest_mut(mask, (y >> sub.1) * *stride), *stride, *sub))
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
        let values: i16x8<S> = values[0].saturating_narrow(values[1]);
        *output = to_bytes(values.bitcast());
    }
}

impl WideSample for u16 {
    #[inline]
    fn put<S: Simd>(values: [i32x4<S>; 2], output: &mut [u16; LANES]) {
        let values: i16x8<S> = values[0].saturating_narrow(values[1]);
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

/// Runs `chunk` on every eight-lane chunk of a `w x h` block whose
/// predictors and mask are packed: whole chunks in place, an overlapping
/// last chunk when `w % 8 != 0`, and `8 / w` rows in one chunk when `w` is
/// 1, 2 or 4.
#[inline]
fn mask_block<P: Copy + Default, O: Copy + Default>(
    (output, stride): (&mut [O], usize),
    preds: [&[P]; 2],
    mask: &[u8],
    (w, h): (usize, usize),
    chunk: impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    let len = w.wrapping_mul(h);
    let (Some(p0), Some(p1), Some(mask)) =
        (preds[0].get(..len), preds[1].get(..len), mask.get(..len))
    else {
        return;
    };
    if stride < w {
        return;
    }
    let (output, preds) = ((output, stride), [p0, p1]);
    match w {
        1 => small_block::<P, O, 1>(output, preds, mask, chunk),
        2 => small_block::<P, O, 2>(output, preds, mask, chunk),
        4 => small_block::<P, O, 4>(output, preds, mask, chunk),
        8 => rows_block::<P, O, 1>(output, preds, mask, w, chunk),
        16 => rows_block::<P, O, 2>(output, preds, mask, w, chunk),
        32 => rows_block::<P, O, 4>(output, preds, mask, w, chunk),
        9.. => rows_block::<P, O, 0>(output, preds, mask, w, chunk),
        _ => {}
    }
}

/// [`mask_block`] of a block at least 8 wide: `8 * N` wide, or `w` wide when
/// `N == 0`.
#[inline]
fn rows_block<P: Copy, O: Copy, const N: usize>(
    (output, stride): (&mut [O], usize),
    [p0, p1]: [&[P]; 2],
    mask: &[u8],
    w: usize,
    chunk: impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    let w = if N == 0 { w } else { LANES * N };
    let mut output = output;
    let rows = p0.chunks_exact(w).zip(p1.chunks_exact(w));
    for ((row0, row1), masks) in rows.zip(mask.chunks_exact(w)) {
        let row_len = stride.min(output.len());
        let (out, rest) = core::mem::take(&mut output).split_at_mut(row_len);
        output = rest;
        let Some(out) = out.get_mut(..w) else {
            return;
        };
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

/// [`mask_block`] of a `W`-wide block: each chunk is `8 / W` packed rows,
/// and a last part chunk is padded.
#[inline]
fn small_block<P: Copy + Default, O: Copy + Default, const W: usize>(
    (output, stride): (&mut [O], usize),
    [p0, p1]: [&[P]; 2],
    mask: &[u8],
    chunk: impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    let mut output = output;
    let ((chunks0, tail0), (chunks1, tail1)) = (p0.as_chunks(), p1.as_chunks());
    let (masks, tail_mask) = mask.as_chunks();
    for ((p0, p1), m) in chunks0.iter().zip(chunks1).zip(masks) {
        let mut values = [O::default(); LANES];
        chunk(p0, p1, m, &mut values);
        output = store_rows::<O, W>((output, stride), &values, LANES / W);
    }
    if !tail0.is_empty() {
        small_tail::<P, O, W>((output, stride), [tail0, tail1], tail_mask, &chunk);
    }
}

/// Stores `count` rows of `W` values; returns the output after them.
#[inline]
fn store_rows<'a, O: Copy, const W: usize>(
    (mut output, stride): (&'a mut [O], usize),
    values: &[O; LANES],
    count: usize,
) -> &'a mut [O] {
    for values in values.as_chunks::<W>().0.iter().take(count) {
        let (out, rest) = output.split_at_mut(stride.min(output.len()));
        output = rest;
        if let Some(out) = out.first_chunk_mut::<W>() {
            *out = *values;
        }
    }
    output
}

/// The last part chunk of a [`small_block`], padded to eight lanes.
#[cold]
#[inline(never)]
fn small_tail<P: Copy + Default, O: Copy + Default, const W: usize>(
    (output, stride): (&mut [O], usize),
    preds: [&[P]; 2],
    mask: &[u8],
    chunk: &impl Fn(&[P; LANES], &[P; LANES], &[u8; LANES], &mut [O; LANES]),
) {
    fn pad<V: Copy + Default>(tail: &[V]) -> [V; LANES] {
        let mut lanes = [V::default(); LANES];
        if let Some(lanes) = lanes.get_mut(..tail.len()) {
            lanes.copy_from_slice(tail); // splot-copy-ok: a short last chunk into one vector
        }
        lanes
    }
    let mut values = [O::default(); LANES];
    chunk(&pad(preds[0]), &pad(preds[1]), &pad(mask), &mut values);
    store_rows::<O, W>((output, stride), &values, preds[0].len() / W);
}

/// Runs `weighted` on every eight-lane chunk of a packed `w x h` block (`w %
/// 8 == 0`) and stores the weights it returns as the chroma mask.
#[inline]
fn diff_block<S: Simd, P: Copy, O: Copy>(
    (output, stride): (&mut [O], usize),
    preds: [&[P]; 2],
    (w, h): (usize, usize),
    chroma: ChromaMask<'_>,
    weighted: impl Fn(&[P; LANES], &[P; LANES], &mut [O; LANES]) -> i16x8<S>,
) {
    let len = w.wrapping_mul(h);
    let (Some(p0), Some(p1)) = (preds[0].get(..len), preds[1].get(..len)) else {
        return;
    };
    if w < LANES || stride < w || chroma.as_ref().is_some_and(|c| c.1 < (w >> c.2.0)) {
        return;
    }
    let (output, preds) = ((output, stride), [p0, p1]);
    match w {
        8 => diff_rows::<S, P, O, 1>(output, preds, w, chroma, weighted),
        16 => diff_rows::<S, P, O, 2>(output, preds, w, chroma, weighted),
        32 => diff_rows::<S, P, O, 4>(output, preds, w, chroma, weighted),
        _ => diff_rows::<S, P, O, 0>(output, preds, w, chroma, weighted),
    }
}

/// [`diff_block`] of a block `8 * N` wide, or `w` wide when `N == 0`.
#[inline]
fn diff_rows<S: Simd, P: Copy, O: Copy, const N: usize>(
    (output, stride): (&mut [O], usize),
    [p0, p1]: [&[P]; 2],
    w: usize,
    chroma: ChromaMask<'_>,
    weighted: impl Fn(&[P; LANES], &[P; LANES], &mut [O; LANES]) -> i16x8<S>,
) {
    let w = if N == 0 { w } else { LANES * N };
    let weights = |((p0, p1), out): Chunk<'_, P, O>| weighted(p0, p1, out);
    let rows = p0
        .chunks_exact(w)
        .zip(p1.chunks_exact(w))
        .zip(output.chunks_mut(stride));
    let Some((mask, mask_stride, sub)) = chroma else {
        for ((row0, row1), out) in rows {
            let Some(chunks) = row(row0, row1, out) else {
                return;
            };
            chunks.for_each(|chunk| {
                weights(chunk);
            });
        }
        return;
    };
    let masks = mask
        .chunks_mut(mask_stride)
        .map(|mask| mask.get_mut(..w >> sub.0));
    match sub {
        (0, 0) => {
            for (((row0, row1), out), mask) in rows.zip(masks) {
                let (Some(chunks), Some(mask)) = (row(row0, row1, out), mask) else {
                    return;
                };
                for (chunk, mask) in chunks.zip(mask.as_chunks_mut().0) {
                    *mask = to_bytes(weights(chunk).bitcast());
                }
            }
        }
        (1, 0) => {
            for (((row0, row1), out), mask) in rows.zip(masks) {
                let (Some(chunks), Some(mask)) = (row(row0, row1, out), mask) else {
                    return;
                };
                for (chunk, mask) in chunks.zip(mask.as_chunks_mut().0) {
                    let sums = weights(chunk).pairwise_widening_add();
                    store_quarter((sums + 1) >> 1, mask);
                }
            }
        }
        (1, 1) => {
            let pairs = p0.chunks_exact(2 * w).zip(p1.chunks_exact(2 * w));
            let outputs = output.chunks_mut(2 * stride);
            for ((rows0, rows1), (out, mask)) in pairs.zip(outputs.zip(masks)) {
                let ((upper0, lower0), (upper1, lower1)) = (rows0.split_at(w), rows1.split_at(w));
                let (upper_out, lower_out) = out.split_at_mut(stride.min(out.len()));
                let (Some(upper), Some(lower), Some(mask)) = (
                    row(upper0, upper1, upper_out),
                    row(lower0, lower1, lower_out),
                    mask,
                ) else {
                    return;
                };
                for ((upper, lower), mask) in upper.zip(lower).zip(mask.as_chunks_mut().0) {
                    let sums = weights(upper).pairwise_widening_add()
                        + weights(lower).pairwise_widening_add();
                    store_quarter((sums + 2) >> 2, mask);
                }
            }
        }
        _ => {}
    }
}

/// The eight-lane chunks of one row of two packed predictors and the first
/// `row0.len()` samples of an output row; `None` when the output row is
/// shorter.
#[inline]
fn row<'a, P, O>(
    row0: &'a [P],
    row1: &'a [P],
    out: &'a mut [O],
) -> Option<impl Iterator<Item = Chunk<'a, P, O>>> {
    let chunks = row0.as_chunks().0.iter().zip(row1.as_chunks().0);
    Some(chunks.zip(out.get_mut(..row0.len())?.as_chunks_mut().0))
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

    #[test]
    fn packed_mask_blend_matches_the_spec_formula_and_never_panics() {
        let shapes = [
            (1, 8),
            (2, 4),
            (4, 3),
            (4, 4),
            (4, 16),
            (8, 8),
            (12, 3),
            (16, 4),
        ];
        for (w, h) in shapes.into_iter().chain([(32, 8), (64, 2), (128, 2)]) {
            let stride = w + 5;
            let preds = [samples(w * h, w), samples(w * h, h + 1)];
            let mask: Vec<u8> = (0..w * h).map(|i| ((i * 29 + h) % 65) as u8).collect();
            let mut output = vec![0x5a; h * stride + 3];
            blend_mask_u8_packed(&mut output, stride, [&preds[0], &preds[1]], &mask, (w, h));
            for (i, &got) in output.iter().enumerate() {
                let (y, x) = (i / stride, i % stride);
                let at = y * w + x;
                let want = if y < h && x < w {
                    spec_blend(preds[0][at], preds[1][at], i32::from(mask[at]))
                } else {
                    0x5a
                };
                assert_eq!(got, want, "{w}x{h} sample {i}");
            }
        }
        let (preds, wide, mut output) = (
            [&[0i16; 64][..], &[0; 64]],
            [&[0i32; 64][..], &[0; 64]],
            [0u8; 64],
        );
        for (stride, len, mask, size) in
            [(0, 64, 64, (8, 8)), (8, 64, 3, (4, 4)), (8, 64, 64, (3, 8))]
                .into_iter()
                .chain([(4, 64, 64, (8, 8)), (8, 9, 64, (8, 8)), (8, 64, 64, (8, 9))])
        {
            blend_mask_u8_packed(&mut output[..len], stride, preds, &[0; 64][..mask], size);
            blend_mask_wide_packed(
                &mut output[..len],
                stride,
                wide,
                &[0; 64][..mask],
                size,
                BitDepth::Ten,
            );
            for sub in [(0, 0), (1, 0), (1, 1), (0, 1), (2, 2)] {
                for mask_stride in [0, 2, 4, 8] {
                    let chroma = Some((&mut [0; 16][..], mask_stride, sub));
                    blend_diff_weighted_u8_packed(
                        &mut output[..len],
                        stride,
                        preds,
                        size,
                        false,
                        chroma,
                    );
                    let chroma = Some((&mut [0; 16][..], mask_stride, sub));
                    let shape = (true, BitDepth::Eight);
                    blend_diff_weighted_wide_packed(
                        &mut output[..len],
                        stride,
                        wide,
                        size,
                        shape,
                        chroma,
                    );
                }
            }
        }
    }
}
