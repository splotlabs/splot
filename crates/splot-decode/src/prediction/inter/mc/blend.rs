// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

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
) -> splot_recon::Result<()> {
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "compound blend sample count",
    })?;
    if output.len() != sample_count {
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
        return blend_compound_average_weighted_samples(
            pred0, pred1, bit_depth, cwp_weight, output,
        );
    }

    optflow::blend_nonuniform_implicit_mask(
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
    )
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_diff_weighted<T: ReconSample>(
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
) -> splot_recon::Result<()> {
    if let CompoundBlend::Wedge { index, sign } = blend {
        return blend_compound_wedge::<T>(
            pred0, pred1, bit_depth, w, h, luma_w, luma_h, index, sign, sub_x, sub_y, output,
        );
    }
    let CompoundBlend::DiffWeighted { inverse } = blend else {
        return blend_compound_average_weighted_samples(pred0, pred1, bit_depth, CWP_EQUAL, output);
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
    if pred0.len() < sample_count || output.len() > sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len().min(output.len()),
        });
    }
    let scales = luma_diff_weighted_mask
        .map(|mask| diff_weighted_luma_mask_scales(mask, w, h, sub_x, sub_y))
        .transpose()?;
    let max_sample = i32::from(bit_depth.max_sample().min(T::MAX_VALUE));
    let blend_shift = 6 + compound_inter_post_round();
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    let mut mask_row = [0i32; MAX_MC_BLOCK_DIM];
    let mask_row = mask_row
        .get_mut(..w)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: w,
            actual: MAX_MC_BLOCK_DIM,
        })?;
    let rows = output
        .chunks_mut(w)
        .zip(pred0.chunks_exact(w).zip(pred1.chunks_exact(w)));
    for (y, (output, (pred0, pred1))) in rows.enumerate() {
        if let (Some(luma_mask), Some((scale_x, scale_y, luma_w))) =
            (luma_diff_weighted_mask, scales)
        {
            mask_row.fill(0);
            for dy in 0..scale_y {
                let luma_row = &luma_mask[(y * scale_y + dy) * luma_w..][..luma_w];
                match scale_x {
                    1 => {
                        for (mask, &luma) in mask_row.iter_mut().zip(luma_row) {
                            *mask += i32::from(luma);
                        }
                    }
                    2 => {
                        for (mask, pair) in mask_row.iter_mut().zip(luma_row.as_chunks::<2>().0) {
                            *mask += i32::from(pair[0]) + i32::from(pair[1]);
                        }
                    }
                    _ => {
                        for (mask, luma) in mask_row.iter_mut().zip(luma_row.chunks_exact(scale_x))
                        {
                            *mask += luma.iter().map(|&value| i32::from(value)).sum::<i32>();
                        }
                    }
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
        let blended =
            mask_row
                .iter()
                .zip(pred0.iter().zip(pred1))
                .map(|(&mask, (&left, &right))| {
                    (mask * left + (64 - mask) * right + (1 << (blend_shift - 1))) >> blend_shift
                });
        store_clamped_samples(output, max_sample, blended)?;
    }
    Ok(())
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
            output[idx] = T::try_from_u16(blended.clamp(0, max_sample) as u16)?;
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
