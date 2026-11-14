// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use fearless_simd::{
    Bytes as _, Select as _, Simd, SimdBase as _, SimdNarrow as _, i32x4, u16x8, u32x4,
};

/// The widest luma motion-compensated block: a 256x256 superblock side.
const MAX_MC_LUMA_DIM: usize = 256;

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_compound_average<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    blend: CompoundBlend,
    luma_w: usize,
    luma_h: usize,
    motion: Option<&CompoundMotionGrid>,
    plane_x: usize,
    plane_y: usize,
    scaling0: PlaneScaling,
    scaling1: PlaneScaling,
    frame_w: usize,
    frame_h: usize,
    luma_diff_weighted_mask: Option<&[u16]>,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
    stride: usize,
) -> splot_recon::Result<()> {
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "compound blend sample count",
    })?;
    let required = h
        .checked_sub(1)
        .map_or(Some(0), |rows| rows.checked_mul(stride)?.checked_add(w));
    if stride < w || required != Some(output.len()) {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: output.len(),
        });
    }
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    if pred0.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = blend
    else {
        return blend_compound_diff_weighted::<T>(
            pred0,
            pred1,
            bit_depth,
            w,
            h,
            blend,
            luma_w,
            luma_h,
            luma_diff_weighted_mask,
            sub_x,
            sub_y,
            output,
            stride,
        );
    };
    let scaling_templates = [scaling0, scaling1];
    let uniform_scalings =
        compound_uniform_scalings(motion, plane_x, plane_y, scaling_templates, sub_x, sub_y);
    if compound_average_weights_are_uniform(
        implicit_mask,
        cwp_weight,
        w,
        h,
        scaling_templates,
        uniform_scalings,
        (frame_w, frame_h),
    ) {
        return blend_rows(pred0, pred1, w, output, stride, |pred0, pred1, output| {
            blend_compound_average_weighted_samples(pred0, pred1, bit_depth, cwp_weight, output)
        });
    }

    blend_nonuniform_implicit_mask(
        pred0,
        pred1,
        bit_depth,
        w,
        h,
        motion,
        plane_x,
        plane_y,
        scaling_templates,
        frame_w,
        frame_h,
        sub_x,
        sub_y,
        output,
        stride,
    )
}

/// Runs `blend` over each of the `w`-sample rows of the packed predictions
/// and of `output`, whose rows are `stride` samples apart.
fn blend_rows<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    w: usize,
    output: &mut [T],
    stride: usize,
    mut blend: impl FnMut(&[i32], &[i32], &mut [T]) -> splot_recon::Result<()>,
) -> splot_recon::Result<()> {
    let preds = pred0
        .chunks_exact(w.max(1))
        .zip(pred1.chunks_exact(w.max(1)));
    for (output, (pred0, pred1)) in output.chunks_mut(stride.max(1)).zip(preds) {
        blend(pred0, pred1, &mut output[..w])?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_compound_diff_weighted<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    blend: CompoundBlend,
    luma_w: usize,
    luma_h: usize,
    luma_diff_weighted_mask: Option<&[u16]>,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
    stride: usize,
) -> splot_recon::Result<()> {
    if let CompoundBlend::Wedge { index, sign } = blend {
        return blend_compound_wedge::<T>(
            pred0, pred1, bit_depth, w, h, luma_w, luma_h, index, sign, sub_x, sub_y, output,
            stride,
        );
    }
    let CompoundBlend::DiffWeighted { inverse } = blend else {
        return blend_rows(pred0, pred1, w, output, stride, |pred0, pred1, output| {
            blend_compound_average_weighted_samples(pred0, pred1, bit_depth, CWP_EQUAL, output)
        });
    };
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "diff-weighted compound mask sample count",
    })?;
    if pred0.len() < sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let scales = luma_diff_weighted_mask
        .map(|mask| diff_weighted_luma_mask_scales(mask, w, h, sub_x, sub_y))
        .transpose()?;
    let max_sample = i32::from(bit_depth.max_sample().min(T::MAX_VALUE));
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    let mut mask_row = [0i32; MAX_MC_LUMA_DIM];
    let mask_row = mask_row
        .get_mut(..w)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: w,
            actual: MAX_MC_LUMA_DIM,
        })?;
    let rows = output
        .chunks_mut(stride.max(1))
        .zip(pred0.chunks_exact(w).zip(pred1.chunks_exact(w)));
    for (y, (output, (pred0, pred1))) in rows.enumerate() {
        let output = &mut output[..w];
        if let (Some(luma_mask), Some((scale_x, scale_y, luma_w))) =
            (luma_diff_weighted_mask, scales)
        {
            let luma_rows = &luma_mask[y * scale_y * luma_w..][..scale_y * luma_w];
            match (scale_x, scale_y) {
                (1, 1) => {
                    let masks = luma_rows.iter().map(|&mask| i32::from(mask));
                    blend_mask_row(output, masks, pred0, pred1, max_sample)?;
                    continue;
                }
                (2, 2) => {
                    let (top, bottom) = luma_rows.split_at(luma_w);
                    let masks = top.as_chunks::<2>().0.iter().zip(bottom.as_chunks::<2>().0);
                    let masks = masks.map(|(top, bottom)| {
                        let sum = i32::from(top[0]) + i32::from(top[1]);
                        (sum + i32::from(bottom[0]) + i32::from(bottom[1]) + 2) >> 2
                    });
                    blend_mask_row(output, masks, pred0, pred1, max_sample)?;
                    continue;
                }
                _ => {}
            }
            mask_row.fill(0);
            for luma_row in luma_rows.chunks_exact(luma_w) {
                for (mask, luma) in mask_row.iter_mut().zip(luma_row.chunks_exact(scale_x)) {
                    *mask += luma.iter().map(|&value| i32::from(value)).sum::<i32>();
                }
            }
            let average_shift = sub_x + sub_y;
            for mask in mask_row.iter_mut() {
                *mask = (*mask + ((1 << average_shift) >> 1)) >> average_shift;
            }
        } else {
            for (mask, (&left, &right)) in mask_row.iter_mut().zip(pred0.iter().zip(pred1)) {
                *mask = i32::from(difference_weight(left, right, diff_round, inverse));
            }
        }
        blend_mask_row(output, mask_row.iter().copied(), pred0, pred1, max_sample)?;
    }
    Ok(())
}

/// The § 7.13.3.30 blend of one row with the mask values `masks`.
#[allow(
    clippy::inline_always,
    reason = "the mask iterator must fuse with the blend loop"
)]
#[inline(always)]
fn blend_mask_row<T: ReconSample>(
    output: &mut [T],
    masks: impl Iterator<Item = i32>,
    pred0: &[i32],
    pred1: &[i32],
    max_sample: i32,
) -> splot_recon::Result<()> {
    let shift = 6 + compound_inter_post_round();
    let blended = masks
        .zip(pred0.iter().zip(pred1))
        .map(|(mask, (&left, &right))| {
            (mask * left + (64 - mask) * right + (1 << (shift - 1))) >> shift
        });
    store_clamped_samples(output, max_sample, blended)
}

/// Clamps each sample to `0..=max_sample` and stores it. When `max_sample`
/// fits the storage type, the store is a plain `u8` or `u16` write, so the
/// caller's loop vectorizes.
#[allow(
    clippy::inline_always,
    reason = "the loop must fuse with the caller's iterator"
)]
#[inline(always)]
pub(super) fn store_clamped_samples<T: ReconSample>(
    output: &mut [T],
    max_sample: i32,
    samples: impl Iterator<Item = i32>,
) -> splot_recon::Result<()> {
    let samples = samples.map(|sample| sample.clamp(0, max_sample));
    if max_sample <= i32::from(u8::MAX)
        && let Some(output) = T::u8_slice_mut(output)
    {
        for (slot, sample) in output.iter_mut().zip(samples) {
            *slot = sample as u8;
        }
    } else if max_sample <= i32::from(u16::MAX)
        && let Some(output) = T::u16_slice_mut(output)
    {
        for (slot, sample) in output.iter_mut().zip(samples) {
            *slot = sample as u16;
        }
    } else {
        for (slot, sample) in output.iter_mut().zip(samples) {
            *slot = T::try_from_u16(sample as u16)?;
        }
    }
    Ok(())
}

/// § 7.13.3.28 difference weight of one sample pair, rounded without overflow.
#[inline]
fn difference_weight(left: i32, right: i32, diff_round: u32, inverse: bool) -> u16 {
    let diff = ((left.abs_diff(right) >> (diff_round - 1)) + 1) >> 1;
    let base = (38 + (diff >> 4)).min(64) as u16;
    if inverse { 64 - base } else { base }
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_wedge<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    luma_w: usize,
    luma_h: usize,
    wedge_index: u8,
    sign: bool,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
    stride: usize,
) -> splot_recon::Result<()> {
    let max_sample = i32::from(bit_depth.max_sample());
    let shift = 6 + compound_inter_post_round();
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let mask = wedge_mask_plane_sample(
                luma_w,
                luma_h,
                usize::from(wedge_index),
                sign,
                sub_x,
                sub_y,
                x,
                y,
            )?;
            let blended = round2_i32(
                i32::from(mask) * pred0[idx] + i32::from(64 - mask) * pred1[idx],
                shift,
            );
            output[y * stride + x] = T::try_from_u16(blended.clamp(0, max_sample) as u16)?;
        }
    }
    Ok(())
}

pub(super) fn diff_weighted_mask_into(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    inverse: bool,
    mask: &mut Vec<u16>,
) -> splot_recon::Result<()> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "diff-weighted compound mask sample count",
    })?;
    if pred0.len() < sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    mask.clear();
    mask.extend(
        pred0[..sample_count]
            .iter()
            .zip(&pred1[..sample_count])
            .map(|(&left, &right)| difference_weight(left, right, diff_round, inverse)),
    );
    Ok(())
}

fn diff_weighted_luma_mask_scales(
    luma_mask: &[u16],
    w: usize,
    h: usize,
    sub_x: u32,
    sub_y: u32,
) -> splot_recon::Result<(usize, usize, usize)> {
    let scale_x = 1usize
        .checked_shl(sub_x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask horizontal subsampling",
        })?;
    let scale_y = 1usize
        .checked_shl(sub_y)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask vertical subsampling",
        })?;
    let luma_w = w
        .checked_mul(scale_x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask width",
        })?;
    let luma_h = h
        .checked_mul(scale_y)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask height",
        })?;
    let expected = luma_w
        .checked_mul(luma_h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask sample count",
        })?;
    if luma_mask.len() < expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: luma_mask.len(),
        });
    }

    Ok((scale_x, scale_y, luma_w))
}

#[cfg(test)]
#[test]
fn difference_weight_matches_the_rounded_spec_formula_at_the_extremes() {
    let values = [
        i32::MIN,
        -70_000,
        -1023,
        -17,
        -1,
        0,
        1,
        15,
        16,
        1023,
        70_000,
        i32::MAX,
    ];
    for diff_round in [4, 6] {
        for left in values {
            for right in values {
                let diff = round2_i32(
                    i32::try_from(left.abs_diff(right)).unwrap_or(i32::MAX),
                    diff_round,
                );
                let base = (38 + diff / 16).clamp(0, 64) as u16;
                for inverse in [false, true] {
                    let want = if inverse { 64 - base } else { base };
                    let got = difference_weight(left, right, diff_round, inverse);
                    assert_eq!(got, want, "{left} {right} {diff_round} {inverse}");
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_nonuniform_implicit_mask<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    width: usize,
    height: usize,
    motion: Option<&CompoundMotionGrid>,
    plane_x: usize,
    plane_y: usize,
    scaling_templates: [PlaneScaling; 2],
    frame_w: usize,
    frame_h: usize,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
    output_stride: usize,
) -> splot_recon::Result<()> {
    if output.is_empty() {
        return Ok(());
    }
    let blend = ImplicitMaskBlend::new(bit_depth, frame_w, frame_h);
    if let Some(motion) = motion {
        return blend_implicit_mask_region(
            [pred0, pred1],
            width,
            [0, 0, width, height],
            motion,
            (plane_x, plane_y),
            scaling_templates,
            blend,
            (sub_x, sub_y),
            output,
            output_stride,
        );
    }
    let reference_starts =
        scaling_templates.map(|scaling| (scaling.start_x >> 10, scaling.start_y >> 10));
    for (row, ((output, pred0), pred1)) in output
        .chunks_mut(output_stride.max(1))
        .zip(pred0.chunks(width))
        .zip(pred1.chunks(width))
        .enumerate()
    {
        let starts = reference_starts.map(|(x, y)| (x, y + row as i32));
        blend.row(pred0, pred1, starts, &mut output[..width])?;
    }
    Ok(())
}

/// The implicit-mask weights: a reference whose sample position falls outside
/// the frame yields its whole weight to the other reference.
#[derive(Clone, Copy)]
pub(super) struct ImplicitMaskBlend {
    last: (i32, i32),
    max_sample: i32,
}

impl ImplicitMaskBlend {
    pub(super) fn new(bit_depth: splot_recon::BitDepth, frame_w: usize, frame_h: usize) -> Self {
        Self {
            last: (frame_w as i32 - 1, frame_h as i32 - 1),
            max_sample: i32::from(bit_depth.max_sample()),
        }
    }

    #[allow(
        clippy::inline_always,
        reason = "per-sample blend; must inline in every crate's copy of the decoder"
    )]
    #[inline(always)]
    pub(super) fn sample<T: ReconSample>(
        self,
        left: i32,
        right: i32,
        starts: [(i32, i32); 2],
    ) -> splot_recon::Result<T> {
        let onscreen =
            starts.map(|(x, y)| (0..=self.last.0).contains(&x) && (0..=self.last.1).contains(&y));
        let mask = match onscreen {
            [true, false] => 2,
            [false, true] => 0,
            _ => 1,
        };
        let sample = round2_i32(
            mask * left + (2 - mask) * right,
            1 + compound_inter_post_round(),
        );
        T::try_from_u16(sample.clamp(0, self.max_sample) as u16)
    }

    /// [`Self::sample`] over one row whose first sample reads the reference
    /// positions `starts` and whose later samples step one column right.
    fn row<T: ReconSample>(
        self,
        left: &[i32],
        right: &[i32],
        starts: [(i32, i32); 2],
        output: &mut [T],
    ) -> splot_recon::Result<()> {
        let bounds = starts.map(|(_, y)| {
            if (0..=self.last.1).contains(&y) {
                self.last.0 as u32 + 1
            } else {
                0
            }
        });
        let width = output.len() as i32;
        let row_weight = |reference: usize| {
            let (first, bound) = (starts[reference].0, bounds[reference] as i32);
            let end = first + width;
            if bound == 0 || end <= 0 || first >= bound {
                Some(0)
            } else {
                (first >= 0 && end <= bound).then_some(32)
            }
        };
        baseline_simd!(simd => {
            if let (Some(weight0), Some(weight1)) = (row_weight(0), row_weight(1)) {
                let mask = i32x4::splat(simd, weight0 - weight1 + 32);
                return masked_row(simd, (left, right), self.max_sample, output, |_| mask);
            }
            let columns = i32x4::from_slice(simd, &[0, 1, 2, 3]);
            let weight = |reference: usize, col: usize| {
                let x = columns + (starts[reference].0 + col as i32);
                x.bitcast::<u32x4<_>>()
                    .simd_lt(bounds[reference])
                    .select(i32x4::splat(simd, 32), i32x4::splat(simd, 0))
            };
            let masks = |col: usize| weight(0, col) - weight(1, col) + 32;
            masked_row(simd, (left, right), self.max_sample, output, masks)
        })
    }
}

/// The § 7.13.3.16 `cwpWeight` blend of a row: [`masked_row`] with the
/// mask `4 * cwp_weight`, the same rounding at four times the scale.
pub(super) fn weighted_row<T: ReconSample>(
    left: &[i32],
    right: &[i32],
    bit_depth: splot_recon::BitDepth,
    cwp_weight: i16,
    output: &mut [T],
) -> splot_recon::Result<()> {
    let max_sample = i32::from(bit_depth.max_sample());
    let weight = 4 * i32::from(cwp_weight);
    baseline_simd!(simd => masked_row(simd, (left, right), max_sample, output, |_| {
        i32x4::splat(simd, weight)
    }))
}

/// `Round2(mask * left + (64 - mask) * right, 6 + InterPostRound)` of each
/// sample of `output`, clipped to `0..=max_sample` (§ 7.13.3.30), four at a
/// time with `masks(col)` the masks of columns `col..col + 4`, paired into
/// eight-sample stores. A ragged end repeats the last four columns; a row
/// under four samples is padded.
#[inline]
fn masked_row<S: Simd, T: ReconSample>(
    simd: S,
    (left, right): (&[i32], &[i32]),
    max_sample: i32,
    output: &mut [T],
    masks: impl Fn(usize) -> i32x4<S>,
) -> splot_recon::Result<()> {
    let width = output.len();
    let (Some(left), Some(right)) = (left.get(..width), right.get(..width)) else {
        return Err(ReconError::BufferLengthMismatch {
            expected: width,
            actual: left.len().min(right.len()),
        });
    };
    let shift = 6 + compound_inter_post_round();
    let blend = |col: usize, left: &[i32], right: &[i32]| {
        let (mask, l, r) = (
            masks(col),
            i32x4::from_slice(simd, &left[col..col + 4]),
            i32x4::from_slice(simd, &right[col..col + 4]),
        );
        let value = (mask * l + (i32x4::splat(simd, 64) - mask) * r + (1 << (shift - 1))) >> shift;
        value.clamp(0, max_sample).bitcast::<u32x4<S>>()
    };
    let store = |values: u16x8<S>, output: &mut [T]| {
        if let Some(output) = T::u8_slice_mut(output) {
            output.copy_from_slice(&values.narrow(values).to_array()[..output.len()]);
        } else if let Some(output) = T::u16_slice_mut(output) {
            output.copy_from_slice(&values.to_array()[..output.len()]);
        } else {
            for (slot, value) in output.iter_mut().zip(values.to_array()) {
                *slot = T::try_from_u16(value)?;
            }
        }
        Ok(())
    };
    if width < 4 {
        let pad = |samples: &[i32]| -> [i32; 4] {
            core::array::from_fn(|col| samples.get(col).copied().unwrap_or(0))
        };
        let value = blend(0, &pad(left), &pad(right));
        return store(value.narrow(value), output);
    }
    let mut start = 0;
    while start + 8 <= width {
        let values = blend(start, left, right).narrow(blend(start + 4, left, right));
        store(values, &mut output[start..start + 8])?;
        start += 8;
    }
    for col in (start..width).step_by(4).map(|col| col.min(width - 4)) {
        let value = blend(col, left, right);
        store(value.narrow(value), &mut output[col..col + 4])?;
    }
    Ok(())
}

/// Blends the `[x, y, width, height]` region of a motion-grid plane, whose
/// predictions and output are region-local with their own row strides, with
/// each sample's implicit mask taken from its own motion-grid cell. With
/// unscaled references a start is the sample position plus one per-cell
/// offset, so only scaled references derive it per sample.
#[allow(clippy::too_many_arguments)]
pub(super) fn blend_implicit_mask_region<T: ReconSample>(
    preds: [&[i32]; 2],
    pred_stride: usize,
    [x, y, width, height]: [usize; 4],
    motion: &CompoundMotionGrid,
    (plane_x, plane_y): (usize, usize),
    scaling_templates: [PlaneScaling; 2],
    blend: ImplicitMaskBlend,
    (sub_x, sub_y): (u32, u32),
    output: &mut [T],
    output_stride: usize,
) -> splot_recon::Result<()> {
    let unit_width = (motion.unit_size() >> sub_x).max(4);
    let unit_height = (motion.unit_size() >> sub_y).max(4);
    let unscaled = !scaling_templates.iter().any(|scaling| scaling.is_scaled());
    for cell_y in (y..y + height).step_by(unit_height) {
        for cell_x in (x..x + width).step_by(unit_width) {
            let mvs = motion.at_luma_offset(cell_x << sub_x, cell_y << sub_y)?;
            let start_at = |reference: usize, col: usize, row: usize| {
                let scaling = scaling_templates[reference].with_prescaled_mv(
                    (plane_x + col) as i32,
                    (plane_y + row) as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                );
                (scaling.start_x >> 10, scaling.start_y >> 10)
            };
            let offsets: [(i32, i32); 2] = core::array::from_fn(|reference| {
                let (start_x, start_y) = start_at(reference, cell_x, cell_y);
                (
                    start_x - (plane_x + cell_x) as i32,
                    start_y - (plane_y + cell_y) as i32,
                )
            });
            let cols = cell_x - x..(cell_x + unit_width).min(x + width) - x;
            for row in cell_y..(cell_y + unit_height).min(y + height) {
                let source = (row - y) * pred_stride;
                let destination = (row - y) * output_stride;
                if unscaled {
                    let starts = offsets.map(|(offset_x, offset_y)| {
                        (
                            (plane_x + x + cols.start) as i32 + offset_x,
                            (plane_y + row) as i32 + offset_y,
                        )
                    });
                    blend.row(
                        &preds[0][source + cols.start..source + cols.end],
                        &preds[1][source + cols.start..source + cols.end],
                        starts,
                        &mut output[destination + cols.start..destination + cols.end],
                    )?;
                    continue;
                }
                for col in cols.clone() {
                    let starts =
                        core::array::from_fn(|reference| start_at(reference, x + col, row));
                    output[destination + col] =
                        blend.sample(preds[0][source + col], preds[1][source + col], starts)?;
                }
            }
        }
    }
    Ok(())
}
