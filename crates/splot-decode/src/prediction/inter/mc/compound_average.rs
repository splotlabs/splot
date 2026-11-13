// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_recon::{
    PlaneId, PlaneRect, ReconSample, UnscaledPut, subpel_compound_block,
    subpel_predict_block_compound_average_unscaled_strided_into,
};

use super::super::mv_scaling::reference_is_scaled;
use super::{
    CWP_EQUAL, CompoundAverageOutput, CompoundBlend, CompoundMcBlock, TranslationalCompoundPlane,
    WorkspaceSink, ceil_shift, compound_average_weights_are_uniform, luma_view, mc_planes,
    predict_compound_average_into, translational_compound_plane, unscaled_put,
};
use crate::Result;

/// Predicts a translational COMPOUND_AVERAGE block straight into `sink` and
/// returns whether it did; `false` for any other block, one whose weights
/// are not uniform or one with a plane the frame edge clips, which the
/// caller predicts through the general path. A declined block may have had
/// planes written; the general path rewrites them.
pub(super) fn predict_translational_direct<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    offset: ByteOffset,
) -> Result<bool> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = block.blend
    else {
        return Ok(false);
    };
    if T::u16_slice(&[]).is_none() && T::u8_slice(&[]).is_none() {
        return Ok(false);
    }
    if block.sub8x8_chroma || block.warp_params.iter().any(Option::is_some) {
        return Ok(false);
    }
    let frame = sink.info().coded_luma_size();
    let scaled = [block.reference0, block.reference1]
        .iter()
        .any(|reference| {
            let size = reference.info().coded_luma_size();
            reference_is_scaled(
                size.width() as i32,
                size.height() as i32,
                frame.width() as i32,
                frame.height() as i32,
            )
        });
    let done = if scaled {
        [false; 2]
    } else {
        match predict_unscaled(sink, block, (implicit_mask, cwp_weight), offset)? {
            Some(done) => done,
            None => return Ok(false),
        }
    };
    if done[0] && (done[1] || !block.has_chroma) {
        return Ok(true);
    }
    predict_planes(sink, block, (implicit_mask, cwp_weight), done, offset)
}

/// The unscaled descriptor path: both references' windows of luma and of
/// both chroma planes resolved once and stored by one recon call. Returns
/// which of luma and chroma it stored, or `None` without writing when the
/// weights of a plane are not uniform.
fn predict_unscaled<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    (implicit_mask, cwp_weight): (bool, i16),
    offset: ByteOffset,
) -> Result<Option<[bool; 2]>> {
    let info = sink.info();
    let references = [block.reference0, block.reference1];
    let storages = references.map(|reference| reference.info().storage_luma_size());
    let frame = info.storage_luma_size();
    let puts = |plane: PlaneId, sub: (u32, u32)| -> Option<[UnscaledPut; 2]> {
        Some([
            unscaled_put(block.rect, plane, sub, block.mv0, storages[0])?,
            unscaled_put(block.rect, plane, sub, block.mv1, storages[1])?,
        ])
    };
    let uniform = |puts: &[UnscaledPut; 2], (sub_x, sub_y): (u32, u32)| {
        if !implicit_mask || cwp_weight != CWP_EQUAL {
            return true;
        }
        let last_x = ceil_shift(frame.width(), sub_x) as i32 - 1;
        let last_y = ceil_shift(frame.height(), sub_y) as i32 - 1;
        puts.iter().all(|put| {
            let [x, y] = put.start;
            let (w, h) = (put.rect.width() as i32, put.rect.height() as i32);
            x >> 10 >= 0
                && y >> 10 >= 0
                && (x + ((w - 1) << 10)) >> 10 <= last_x
                && (y + ((h - 1) << 10)) >> 10 <= last_y
        })
    };
    let [_, (_, sub_x, sub_y), _] = mc_planes(info.pixel_format());
    let (Some(luma), chroma) = (
        puts(PlaneId::Y, (0, 0)),
        block
            .has_chroma
            .then(|| puts(PlaneId::U, (sub_x, sub_y)))
            .flatten(),
    ) else {
        return Ok(Some([false; 2]));
    };
    if !uniform(&luma, (0, 0)) || chroma.is_some_and(|puts| !uniform(&puts, (sub_x, sub_y))) {
        return Ok(None);
    }
    let views = [
        luma_view(references[0], luma[0], offset)?,
        luma_view(references[1], luma[1], offset)?,
    ];
    let chroma_views = |plane| {
        Some([
            references[0].settled_plane_view(plane)?,
            references[1].settled_plane_view(plane)?,
        ])
    };
    let chroma = chroma
        .and_then(|puts| Some(([chroma_views(PlaneId::U)?, chroma_views(PlaneId::V)?], puts)));
    Ok(Some(subpel_compound_block(
        sink,
        block.interp,
        cwp_weight,
        ([&views[0], &views[1]], luma),
        chroma
            .as_ref()
            .map(|([u, v], puts)| ([[&u[0], &u[1]], [&v[0], &v[1]]], *puts)),
    )))
}

/// Predicts the planes of `block` that `done` does not mark, each through
/// its translational plane, when every one has uniform weights and a
/// contiguous target; `false` without writing them otherwise.
fn predict_planes<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    (implicit_mask, cwp_weight): (bool, i16),
    done: [bool; 2],
    offset: ByteOffset,
) -> Result<bool> {
    let storage_luma_size = sink.info().storage_luma_size();
    let mut translations = [None, None, None];
    let mut translation_count = 0;
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if done[usize::from(plane != PlaneId::Y)] || (plane != PlaneId::Y && !block.has_chroma) {
            continue;
        }
        let translation =
            translational_compound_plane(sink.info(), block, plane, sub_x, sub_y, offset)?;
        let frame_w = storage_luma_size.width().div_ceil(1 << sub_x);
        let frame_h = storage_luma_size.height().div_ceil(1 << sub_y);
        if !compound_average_weights_are_uniform(
            implicit_mask,
            cwp_weight,
            translation.plane.block_w,
            translation.plane.block_h,
            translation.plane.scalings,
            Some(translation.plane.scalings),
            (frame_w, frame_h),
        ) {
            return Ok(false);
        }
        let target = PlaneRect::new(
            translation.plane.plane_x,
            translation.plane.plane_y,
            translation.plane.block_w,
            translation.plane.block_h,
        )?;
        translations[translation_count] = Some((plane, target, translation));
        translation_count += 1;
    }
    for (plane, target, _) in translations.iter().flatten() {
        if sink.rect_mut(*plane, *target).is_none() {
            return Ok(false);
        }
    }
    for (plane, target, translation) in translations.into_iter().flatten() {
        if T::u16_slice(&[]).is_some() {
            sink.with_contiguous_u16_rect_mut(plane, target, |output, stride| {
                predict_unscaled_or_two_call(&translation, cwp_weight, output, stride)
            })?;
        } else {
            sink.with_contiguous_u8_rect_mut(plane, target, |output, stride| {
                predict_unscaled_or_two_call(&translation, cwp_weight, output, stride)
            })?;
        }
    }
    Ok(true)
}

fn predict_unscaled_or_two_call<T: ReconSample, O: CompoundAverageOutput>(
    translation: &TranslationalCompoundPlane<'_, T>,
    cwp_weight: i16,
    output: &mut [O],
    stride: usize,
) -> splot_recon::Result<()> {
    let ([view0, view1], [params0, params1]) = (&translation.plane.views, &translation.params);
    if subpel_predict_block_compound_average_unscaled_strided_into(
        view0, params0, view1, params1, cwp_weight, output, stride,
    )? {
        return Ok(());
    }
    predict_compound_average_into(
        &translation.plane,
        &translation.params,
        cwp_weight,
        None,
        None,
        output,
        stride,
    )
}
