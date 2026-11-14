// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound blocks that blend through a mask: the § 7.13.3.27 wedge, the
//! § 7.13.3.28 difference weight, and the § 7.13.3.14 implicit mask of a
//! COMPOUND_AVERAGE plane at the frame edge. A plane predicts both
//! references (`i16` at eight bits in `u8` frames, else `i32`) and blends
//! them with one kernel call; the luma difference-weight call also writes the
//! chroma mask.

use std::cell::Cell;

use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, PlaneId, PlaneRect, ReconError, ReconSample, blend_diff_weighted_u8_packed,
    blend_diff_weighted_wide_packed, blend_mask_u8_packed, blend_mask_wide_packed,
    subpel_predict_block_compound_intermediate_into, wedge_mask_plane_sample,
};

#[cfg(test)]
use splot_recon::{
    blend_diff_weighted_u8, blend_diff_weighted_wide, blend_mask_u8, blend_mask_wide,
};

use super::super::mv_scaling::PlaneScaling;
use super::compound_average::predict_unscaled_or_two_call;
use super::{
    CompoundBlend, CompoundMcBlock, RecycledMcSamples, WorkspaceSink,
    compound_average_weights_are_uniform, mc_planes, predict_plane, translational_compound_plane,
};
use crate::Result;

/// The two predictors of a plane and the plane's mask, which the chroma
/// planes of a difference-weight block read from the luma call; buffers
/// only grow.
#[derive(Default)]
struct Scratch {
    preds: [Vec<i16>; 2],
    wide: [Vec<i32>; 2],
    mask: Vec<u8>,
}

std::thread_local! {
    static SCRATCH: Cell<Option<Scratch>> = const { Cell::new(None) };
}

fn with_scratch<R>(f: impl FnOnce(&mut Scratch) -> R) -> R {
    let mut scratch = SCRATCH.take().unwrap_or_default();
    let result = f(&mut scratch);
    SCRATCH.set(Some(scratch));
    result
}

/// The predictors of one plane, `w` samples per row.
#[derive(Clone, Copy)]
enum Preds<'a> {
    Narrow([&'a [i16]; 2]),
    Wide([&'a [i32]; 2]),
}

/// Where a plane lies: its size and subsampling, the reference starts, and
/// the plane size of the frame (for the implicit mask).
pub(super) struct PlaneLayout {
    pub(super) plane: PlaneId,
    pub(super) size: (usize, usize),
    pub(super) sub: (u32, u32),
    pub(super) chroma_sub: (u32, u32),
    pub(super) scalings: [PlaneScaling; 2],
    pub(super) frame: (usize, usize),
}

/// Predicts a compound block without warp or motion grid and returns
/// `true`; returns `false` without writing for a warp block.
pub(super) fn predict<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    offset: ByteOffset,
) -> Result<bool> {
    if block.warp_params.iter().any(Option::is_some) {
        return Ok(false);
    }
    with_scratch(|scratch| predict_planes(sink, block, offset, scratch)).map(|()| true)
}

fn predict_planes<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    offset: ByteOffset,
    scratch: &mut Scratch,
) -> Result<()> {
    let info = sink.info();
    let bit_depth = info.bit_depth();
    let narrow = T::u8_slice(&[]).is_some() && bit_depth == BitDepth::Eight;
    let storage = info.storage_luma_size();
    let planes = mc_planes(info.pixel_format());
    for (plane, sub_x, sub_y) in planes {
        if plane != PlaneId::Y && !block.has_chroma {
            continue;
        }
        if plane != PlaneId::Y && block.sub8x8_chroma {
            predict_plane(
                sink,
                block.reference0,
                plane,
                block.rect,
                block.mv0,
                block.interp,
                sub_x,
                sub_y,
                offset,
            )?;
            continue;
        }
        let translation = translational_compound_plane(info, block, plane, sub_x, sub_y, offset)?;
        let geometry = &translation.plane;
        let (w, h) = (geometry.block_w, geometry.block_h);
        let target = PlaneRect::new(geometry.plane_x, geometry.plane_y, w, h)?;
        let layout = PlaneLayout {
            plane,
            size: (w, h),
            sub: (sub_x, sub_y),
            chroma_sub: (planes[1].1, planes[1].2),
            scalings: geometry.scalings,
            frame: (
                storage.width().div_ceil(1 << sub_x),
                storage.height().div_ceil(1 << sub_y),
            ),
        };
        if let CompoundBlend::Average {
            implicit_mask,
            cwp_weight,
        } = block.blend
            && compound_average_weights_are_uniform(
                implicit_mask,
                cwp_weight,
                w,
                h,
                geometry.scalings,
                Some(geometry.scalings),
                layout.frame,
            )
        {
            write_plane(sink, plane, target, |output, stride| {
                if let Some(output) = T::u8_slice_mut(&mut *output) {
                    return predict_unscaled_or_two_call(&translation, cwp_weight, output, stride);
                }
                let output = T::u16_slice_mut(output).ok_or(unsupported::<T>(bit_depth))?;
                predict_unscaled_or_two_call(&translation, cwp_weight, output, stride)
            })?;
            continue;
        }
        let len = w * h;
        let views = geometry.views.iter().zip(&translation.params);
        let preds = if narrow {
            let [pred0, pred1] = &mut scratch.preds;
            let preds = [grown(pred0, len), grown(pred1, len)];
            for (pred, (view, params)) in preds.into_iter().zip(views) {
                subpel_predict_block_compound_intermediate_into(view, params, None, pred, w)?;
            }
            Preds::Narrow([&scratch.preds[0][..len], &scratch.preds[1][..len]])
        } else {
            let [pred0, pred1] = &mut scratch.wide;
            let preds = [grown(pred0, len), grown(pred1, len)];
            for (pred, (view, params)) in preds.into_iter().zip(views) {
                subpel_predict_block_compound_intermediate_into(view, params, None, pred, w)?;
            }
            Preds::Wide([&scratch.wide[0][..len], &scratch.wide[1][..len]])
        };
        let mask = &mut scratch.mask;
        write_plane(sink, plane, target, |output, stride| {
            blend_plane(block, &layout, preds, mask, (output, stride), bit_depth)
        })?;
    }
    Ok(())
}

/// Blends a non-average plane of a warp compound block, whose `i32`
/// predictors the warp kernels made, into `output`.
pub(super) fn blend_warp_plane<T: ReconSample>(
    block: &CompoundMcBlock<'_, T>,
    layout: &PlaneLayout,
    preds: [&[i32]; 2],
    output: (&mut [T], usize),
    bit_depth: BitDepth,
) -> splot_recon::Result<()> {
    with_scratch(|scratch| {
        let preds = Preds::Wide(preds);
        blend_plane(block, layout, preds, &mut scratch.mask, output, bit_depth)
    })
}

/// Makes the mask of the plane and blends `preds` with it into `output`. The
/// luma plane of a difference-weight block makes the chroma mask in `mask`
/// instead, which its chroma planes then read.
fn blend_plane<T: ReconSample>(
    block: &CompoundMcBlock<'_, T>,
    layout: &PlaneLayout,
    preds: Preds<'_>,
    mask: &mut Vec<u8>,
    (output, stride): (&mut [T], usize),
    bit_depth: BitDepth,
) -> splot_recon::Result<()> {
    let ((w, h), (sub_x, sub_y)) = (layout.size, layout.sub);
    let len = w * h;
    if T::u8_slice(&[]).is_some() && bit_depth != BitDepth::Eight {
        return Err(unsupported::<T>(bit_depth));
    }
    let luma_diff = match block.blend {
        CompoundBlend::DiffWeighted { inverse } if layout.plane == PlaneId::Y => Some(inverse),
        CompoundBlend::DiffWeighted { .. } => None,
        CompoundBlend::Wedge { index, sign } => {
            let luma = (block.rect.luma_w, block.rect.luma_h);
            wedge_mask_map(grown(mask, len), w, luma, (index, sign), (sub_x, sub_y))?;
            None
        }
        CompoundBlend::Average { .. } => {
            implicit_mask_map(grown(mask, len), w, layout.scalings, layout.frame);
            None
        }
    };
    if let Some(inverse) = luma_diff {
        let chroma = block.has_chroma && !block.sub8x8_chroma;
        let chroma_mask = chroma.then(|| {
            let sub @ (chroma_x, chroma_y) = layout.chroma_sub;
            let cw = w >> chroma_x;
            (grown(mask, cw * (h >> chroma_y)), cw, sub)
        });
        return match preds {
            Preds::Narrow(preds) => {
                let output = T::u8_slice_mut(output).ok_or(unsupported::<T>(bit_depth))?;
                blend_diff_weighted_u8_packed(output, stride, preds, (w, h), inverse, chroma_mask);
                Ok(())
            }
            Preds::Wide(preds) => {
                let shape = (inverse, bit_depth);
                blend_diff_weighted_wide_packed(output, stride, preds, (w, h), shape, chroma_mask);
                Ok(())
            }
        };
    }
    let mask = mask.get(..len).ok_or(ReconError::BufferLengthMismatch {
        expected: len,
        actual: mask.len(),
    })?;
    match preds {
        Preds::Narrow(preds) => {
            let output = T::u8_slice_mut(output).ok_or(unsupported::<T>(bit_depth))?;
            blend_mask_u8_packed(output, stride, preds, mask, (w, h));
        }
        Preds::Wide(preds) => {
            blend_mask_wide_packed(output, stride, preds, mask, (w, h), bit_depth);
        }
    }
    Ok(())
}

const fn unsupported<T: ReconSample>(bit_depth: BitDepth) -> ReconError {
    ReconError::SampleTypeUnsupportedBitDepth {
        sample_type: T::TYPE_NAME,
        bit_depth,
    }
}

/// Runs `predict` on the target of `rect`, or on a staged block that is
/// then written, when the frame edge clips the target.
fn write_plane<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    plane: PlaneId,
    rect: PlaneRect,
    predict: impl FnOnce(&mut [T], usize) -> splot_recon::Result<()>,
) -> Result<()> {
    if let Some((output, stride)) = sink.rect_mut(plane, rect) {
        return Ok(predict(output, stride)?);
    }
    let mut staged = RecycledMcSamples::<T>::take();
    staged.clear();
    staged.resize(rect.width() * rect.height(), T::default());
    predict(&mut staged, rect.width())?;
    Ok(sink.write_rect(plane, rect, &staged, rect.width())?)
}

/// The § 7.13.3.14 `get_mask` of each sample times 32, row by row: 64 where
/// only reference 0 reads inside the `frame` plane, 0 where only reference 1
/// does, else 32. The references are unscaled.
fn implicit_mask_map(
    mask: &mut [u8],
    w: usize,
    scalings: [PlaneScaling; 2],
    (frame_w, frame_h): (usize, usize),
) {
    let starts = scalings.map(|scaling| (scaling.start_x >> 10, scaling.start_y >> 10));
    for (row, weights) in mask.chunks_exact_mut(w.max(1)).enumerate() {
        weights.fill(32);
        for (reference, &(x, y)) in starts.iter().enumerate() {
            if !(0..frame_h as i32).contains(&(y + row as i32)) {
                continue;
            }
            let first = (-x).clamp(0, w as i32) as usize;
            let end = (frame_w as i32 - x).clamp(0, w as i32) as usize;
            for weight in weights.get_mut(first..end).unwrap_or_default() {
                *weight = if reference == 0 {
                    *weight + 32
                } else {
                    *weight - 32
                };
            }
        }
    }
}

/// The § 7.13.3.27 wedge mask of a `w`-wide plane of a `luma` block, as
/// § 7.13.3.30 subsamples it for the plane.
fn wedge_mask_map(
    mask: &mut [u8],
    w: usize,
    (luma_w, luma_h): (usize, usize),
    (index, sign): (u8, bool),
    (sub_x, sub_y): (u32, u32),
) -> splot_recon::Result<()> {
    for (y, row) in mask.chunks_exact_mut(w.max(1)).enumerate() {
        for (x, weight) in row.iter_mut().enumerate() {
            let index = usize::from(index);
            *weight =
                wedge_mask_plane_sample(luma_w, luma_h, index, sign, sub_x, sub_y, x, y)? as u8;
        }
    }
    Ok(())
}

fn grown<V: Copy + Default>(buffer: &mut Vec<V>, len: usize) -> &mut [V] {
    if buffer.len() < len {
        buffer.resize(len, V::default());
    }
    &mut buffer[..len]
}

#[cfg(test)]
#[path = "compound_mask_tests.rs"]
mod tests;
