// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_recon::{
    PlaneId, PlaneRect, ReconSample, subpel_predict_block_compound_average_fullpel_strided_into_u8,
};

use super::{
    CompoundBlend, CompoundMcBlock, WorkspaceSink, compound_average_weights_are_uniform, mc_planes,
    predict_compound_average_into, translational_compound_plane,
};
use crate::Result;

#[derive(Clone, Copy)]
enum DirectDestination {
    U8,
    U16,
}

pub(super) fn predict_translational_direct<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    offset: ByteOffset,
) -> Result<bool> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = block.blend
    else {
        return Ok(false);
    };
    let destination = if T::u16_slice(&[]).is_some() {
        DirectDestination::U16
    } else if T::u8_slice(&[]).is_some() {
        DirectDestination::U8
    } else {
        return Ok(false);
    };
    if block.sub8x8_chroma || block.warp_params.iter().any(Option::is_some) {
        return Ok(false);
    }

    let coded_luma_size = sink.info().coded_luma_size();
    let mut translations = [None, None, None];
    let mut translation_count = 0;
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if plane != PlaneId::Y && !block.has_chroma {
            continue;
        }
        let translation = translational_compound_plane(sink, block, plane, sub_x, sub_y, offset)?;
        let frame_w = (coded_luma_size.width().div_ceil(4) * 4) >> sub_x;
        let frame_h = (coded_luma_size.height().div_ceil(4) * 4) >> sub_y;
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
        let available = match destination {
            DirectDestination::U16 => sink
                .with_contiguous_u16_rect_mut(*plane, *target, |_, _| Ok(()))?
                .is_some(),
            DirectDestination::U8 => sink
                .with_contiguous_u8_rect_mut(*plane, *target, |_, _| Ok(()))?
                .is_some(),
        };
        if !available {
            return Ok(false);
        }
    }

    for (plane, target, translation) in translations.into_iter().flatten() {
        match destination {
            DirectDestination::U16 => {
                sink.with_contiguous_u16_rect_mut(plane, target, |output, stride| {
                    predict_compound_average_into(
                        &translation.plane,
                        &translation.params,
                        cwp_weight,
                        None,
                        None,
                        output,
                        stride,
                    )
                })?;
            }
            DirectDestination::U8 => {
                sink.with_contiguous_u8_rect_mut(plane, target, |output, stride| {
                    if subpel_predict_block_compound_average_fullpel_strided_into_u8(
                        &translation.plane.views[0],
                        &translation.params[0],
                        &translation.plane.views[1],
                        &translation.params[1],
                        cwp_weight,
                        output,
                        stride,
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
                })?;
            }
        }
    }
    Ok(true)
}
