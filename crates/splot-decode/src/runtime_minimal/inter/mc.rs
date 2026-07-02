// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
use splot_recon::BitDepth;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, InterpolationFilter, PlaneId, PlaneRect, ReconSample,
    ReferencePlaneView, SubpelPredictParams, VisibleRows, WARPED_BLOCK_SIZE,
    WarpPredictBlockParams, blend_compound_average_equal, subpel_predict_block,
    subpel_predict_block_compound_intermediate, warp_predict_block,
};

use super::mv_scaling::derive_plane_scaling;
use super::{Mv, SPEC_MC, unsupported_at};
use crate::Result;
use splot_core::span::ByteOffset;
use splot_recon::math::clip3;

pub(super) const YUV420_MC_PLANES: [(PlaneId, u32, u32); 3] =
    [(PlaneId::Y, 0, 0), (PlaneId::U, 1, 1), (PlaneId::V, 1, 1)];

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct McBlockRect {
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    pub(super) luma_w: usize,
    pub(super) luma_h: usize,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct InterBlockParams<'a, T: ReconSample> {
    rect: McBlockRect,
    prediction: InterPrediction<'a, T>,
    interp: InterpolationFilter,
}

impl<'a, T: ReconSample> InterBlockParams<'a, T> {
    pub(super) const fn single(
        reference: &'a DecodedFrame<T>,
        rect: McBlockRect,
        mv: Mv,
        interp: InterpolationFilter,
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::Single { reference, mv },
            interp,
        }
    }
    pub(super) const fn compound_average(
        reference0: &'a DecodedFrame<T>,
        reference1: &'a DecodedFrame<T>,
        rect: McBlockRect,
        mv0: Mv,
        mv1: Mv,
        interp: InterpolationFilter,
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::CompoundAverage {
                reference0,
                reference1,
                mv0,
                mv1,
            },
            interp,
        }
    }
    pub(super) const fn single_warp(
        reference: &'a DecodedFrame<T>,
        rect: McBlockRect,
        warp_params: [i64; 6],
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::SingleWarp {
                reference,
                warp_params,
            },
            interp: InterpolationFilter::EightTap,
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum InterPrediction<'a, T: ReconSample> {
    Single {
        reference: &'a DecodedFrame<T>,
        mv: Mv,
    },
    SingleWarp {
        reference: &'a DecodedFrame<T>,
        warp_params: [i64; 6],
    },
    CompoundAverage {
        reference0: &'a DecodedFrame<T>,
        reference1: &'a DecodedFrame<T>,
        mv0: Mv,
        mv1: Mv,
    },
}
#[derive(Clone, Copy, Debug)]
struct CompoundMcBlock<'a, T: ReconSample> {
    reference0: &'a DecodedFrame<T>,
    reference1: &'a DecodedFrame<T>,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
}
pub(super) fn motion_compensate_inter_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: InterBlockParams<'_, T>,
    offset: ByteOffset,
) -> Result<()> {
    match block.prediction {
        InterPrediction::Single { reference, mv } => motion_compensate_single_block_into(
            workspace,
            reference,
            block.rect,
            mv,
            block.interp,
            offset,
        ),
        InterPrediction::SingleWarp {
            reference,
            warp_params,
        } => motion_compensate_single_warp_block_into(
            workspace,
            reference,
            block.rect,
            warp_params,
            offset,
        ),
        InterPrediction::CompoundAverage {
            reference0,
            reference1,
            mv0,
            mv1,
        } => motion_compensate_compound_average_block_into(
            workspace,
            CompoundMcBlock {
                reference0,
                reference1,
                rect: block.rect,
                mv0,
                mv1,
                interp: block.interp,
            },
            offset,
        ),
    }
}

fn motion_compensate_single_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(workspace, |workspace, plane, sub_x, sub_y| {
        predict_plane(
            workspace, reference, plane, rect, mv, interp, sub_x, sub_y, offset,
        )
    })
}

fn motion_compensate_planes<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    mut predict: impl FnMut(&mut CurrentFrameWorkspace<T>, PlaneId, u32, u32) -> Result<()>,
) -> Result<()> {
    for (plane, sub_x, sub_y) in YUV420_MC_PLANES {
        predict(workspace, plane, sub_x, sub_y)?;
    }
    Ok(())
}

fn motion_compensate_compound_average_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    offset: ByteOffset,
) -> Result<()> {
    for (plane, sub_x, sub_y) in YUV420_MC_PLANES {
        predict_compound_plane(
            workspace,
            block.reference0,
            block.reference1,
            plane,
            block.rect,
            block.mv0,
            block.mv1,
            block.interp,
            sub_x,
            sub_y,
            offset,
        )?;
    }
    Ok(())
}
fn motion_compensate_single_warp_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    warp_params: [i64; 6],
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(workspace, |workspace, plane, sub_x, sub_y| {
        predict_warp_plane(
            workspace,
            reference,
            plane,
            rect,
            warp_params,
            sub_x,
            sub_y,
            offset,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn predict_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (samples, ref_width, ref_height, ref_mi_cols, ref_mi_rows) =
        reference_plane_samples(reference, plane, offset)?;
    let view = ReferencePlaneView::new(&samples, ref_width, ref_height)?;

    let plane_x = rect.luma_x >> sub_x;
    let plane_y = rect.luma_y >> sub_y;
    let block_w = rect.luma_w >> sub_x;
    let block_h = rect.luma_h >> sub_y;

    let scaling = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv.row),
        i64::from(mv.col),
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        block_w as i64,
        block_h as i64,
    );

    let params = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling.start_x,
        start_y: scaling.start_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: scaling.first_x,
        first_y: scaling.first_y,
        last_x: scaling.last_x,
        last_y: scaling.last_y,
        bit_depth: workspace.info().bit_depth(),
    };
    let predicted = subpel_predict_block(&view, &params)?;

    let packed: Vec<T> = predicted
        .iter()
        .map(|&v| T::try_from_u16(v))
        .collect::<splot_recon::Result<Vec<T>>>()?;

    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    workspace.write_rect(plane, rect, &packed, block_w)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_warp_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: [i64; 6],
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (samples, ref_width, ref_height, ref_mi_cols, ref_mi_rows) =
        reference_plane_samples(reference, plane, offset)?;
    let view = ReferencePlaneView::new(&samples, ref_width, ref_height)?;

    let plane_x = rect.luma_x >> sub_x;
    let plane_y = rect.luma_y >> sub_y;
    let block_w = rect.luma_w >> sub_x;
    let block_h = rect.luma_h >> sub_y;
    let bit_depth = workspace.info().bit_depth();
    let skip_pred = !splot_recon::warp_shear_is_valid(warp_params)
        || block_w < WARPED_BLOCK_SIZE
        || block_h < WARPED_BLOCK_SIZE;
    if skip_pred {
        for i4 in 0..block_h.div_euclid(4) {
            for j4 in 0..block_w.div_euclid(4) {
                let unit_x = (plane_x + (j4 & !1) * 4) as i64;
                let unit_y = (plane_y + (i4 & !1) * 4) as i64;
                let (first_x, first_y, last_x, last_y) = ext_warp_unit_bounds(
                    rect,
                    warp_params,
                    unit_x,
                    unit_y,
                    block_w.min(8) as i64,
                    block_h.min(8) as i64,
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                );
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: plane_x as i64,
                    block_y: plane_y as i64,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    first_x,
                    first_y,
                    last_x,
                    last_y,
                    bit_depth,
                };
                let predicted = splot_recon::ext_warp_predict_unit(&view, &params, i4, j4)?;
                let packed: Vec<T> = predicted
                    .iter()
                    .map(|&v| T::try_from_u16(v))
                    .collect::<splot_recon::Result<Vec<T>>>()?;
                let rect = PlaneRect::new(plane_x + j4 * 4, plane_y + i4 * 4, 4, 4)?;
                workspace.write_rect(plane, rect, &packed, 4)?;
            }
        }
        return Ok(());
    }

    for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
        for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
            let params = WarpPredictBlockParams {
                warp_params,
                block_x: (plane_x + local_x) as i64,
                block_y: (plane_y + local_y) as i64,
                subsampling_x: sub_x as u8,
                subsampling_y: sub_y as u8,
                first_x: 0,
                first_y: 0,
                last_x: ref_width as i64 - 1,
                last_y: ref_height as i64 - 1,
                bit_depth,
            };
            let predicted = warp_predict_block(&view, &params)?;
            let write_w = (block_w - local_x).min(WARPED_BLOCK_SIZE);
            let write_h = (block_h - local_y).min(WARPED_BLOCK_SIZE);
            let mut packed: Vec<T> = Vec::with_capacity(write_w.saturating_mul(write_h));
            for row in 0..write_h {
                for col in 0..write_w {
                    packed.push(T::try_from_u16(predicted[row * WARPED_BLOCK_SIZE + col])?);
                }
            }
            let rect = PlaneRect::new(plane_x + local_x, plane_y + local_y, write_w, write_h)?;
            workspace.write_rect(plane, rect, &packed, write_w)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_compound_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference0: &DecodedFrame<T>,
    reference1: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (samples0, ref_width0, ref_height0, ref_mi_cols0, ref_mi_rows0) =
        reference_plane_samples(reference0, plane, offset)?;
    let (samples1, ref_width1, ref_height1, ref_mi_cols1, ref_mi_rows1) =
        reference_plane_samples(reference1, plane, offset)?;
    let view0 = ReferencePlaneView::new(&samples0, ref_width0, ref_height0)?;
    let view1 = ReferencePlaneView::new(&samples1, ref_width1, ref_height1)?;

    let plane_x = rect.luma_x >> sub_x;
    let plane_y = rect.luma_y >> sub_y;
    let block_w = rect.luma_w >> sub_x;
    let block_h = rect.luma_h >> sub_y;

    let scaling0 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv0.row),
        i64::from(mv0.col),
        sub_x,
        sub_y,
        ref_mi_cols0,
        ref_mi_rows0,
        block_w as i64,
        block_h as i64,
    );
    let scaling1 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv1.row),
        i64::from(mv1.col),
        sub_x,
        sub_y,
        ref_mi_cols1,
        ref_mi_rows1,
        block_w as i64,
        block_h as i64,
    );

    let params0 = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling0.start_x,
        start_y: scaling0.start_y,
        step_x: scaling0.step_x,
        step_y: scaling0.step_y,
        first_x: scaling0.first_x,
        first_y: scaling0.first_y,
        last_x: scaling0.last_x,
        last_y: scaling0.last_y,
        bit_depth: workspace.info().bit_depth(),
    };
    let params1 = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling1.start_x,
        start_y: scaling1.start_y,
        step_x: scaling1.step_x,
        step_y: scaling1.step_y,
        first_x: scaling1.first_x,
        first_y: scaling1.first_y,
        last_x: scaling1.last_x,
        last_y: scaling1.last_y,
        bit_depth: workspace.info().bit_depth(),
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0)?;
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1)?;
    let blended = blend_compound_average_equal(&pred0, &pred1, workspace.info().bit_depth())?;

    let packed: Vec<T> = blended
        .iter()
        .map(|&v| T::try_from_u16(v))
        .collect::<splot_recon::Result<Vec<T>>>()?;
    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    workspace.write_rect(plane, rect, &packed, block_w)?;
    Ok(())
}

/// § 7.13.3.20 per-8x8 bounding box for the fixed-phase ext-warp kernel:
/// derives the unit's translational MV (`get_sub_block_warp_mv` with
/// `rnd == 0`), applies the § 5.20.9.4 / § 5.20.9.5 MV clamps and the
/// § 7.13.3.17 unscaled scaling, and narrows the reference read window to
/// the projected span with -3/+4 tap margins. Bounds derive from the
/// visible reference geometry on the admitted equal-size unscaled-reference
/// surface; the § 7.13.3.15 `is_scaled` arm and non-8-aligned mi padding
/// are beyond the frontier (upstream gates defer both).
#[allow(clippy::too_many_arguments)]
fn ext_warp_unit_bounds(
    rect: McBlockRect,
    warp_params: [i64; 6],
    unit_x: i64,
    unit_y: i64,
    bbox_w: i64,
    bbox_h: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
) -> (i64, i64, i64, i64) {
    const WARPEDMODEL_PREC_BITS: u32 = 16;
    const MV_BOUND: i64 = 1 << 16;
    const MV_BORDER: i64 = 128;
    let src_x = (unit_x + (bbox_w >> 1)) << sub_x;
    let src_y = (unit_y + (bbox_h >> 1)) << sub_y;
    let dst_x = warp_params[2] * src_x + warp_params[3] * src_y + warp_params[0];
    let dst_y = warp_params[4] * src_x + warp_params[5] * src_y + warp_params[1];
    let mv_row = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_y - (src_y << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    );
    let mv_col = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_x - (src_x << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    );
    let mi_row = (rect.luma_y / 4) as i64;
    let mi_col = (rect.luma_x / 4) as i64;
    let bh4 = (rect.luma_h / 4) as i64;
    let bw4 = (rect.luma_w / 4) as i64;
    let mv_row = clip3(
        -(mi_row + bh4) * 32 - MV_BORDER,
        (ref_mi_rows - mi_row) * 32 + MV_BORDER,
        mv_row,
    );
    let mv_col = clip3(
        -(mi_col + bw4) * 32 - MV_BORDER,
        (ref_mi_cols - mi_col) * 32 + MV_BORDER,
        mv_col,
    );
    let scaling = derive_plane_scaling(
        unit_x,
        unit_y,
        mv_row,
        mv_col,
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        bbox_w,
        bbox_h,
    );
    let first_x = clip3(0, scaling.last_x, (scaling.start_x >> 10) - 3);
    let first_y = clip3(0, scaling.last_y, (scaling.start_y >> 10) - 3);
    let last_x = clip3(
        0,
        scaling.last_x,
        ((scaling.start_x + scaling.step_x * (bbox_w - 1)) >> 10) + 4,
    );
    let last_y = clip3(
        0,
        scaling.last_y,
        ((scaling.start_y + scaling.step_y * (bbox_h - 1)) >> 10) + 4,
    );
    (first_x, first_y, last_x, last_y)
}

pub(super) fn reference_plane_samples<T: ReconSample>(
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    offset: ByteOffset,
) -> Result<(Vec<u16>, usize, usize, i64, i64)> {
    let Some(ref_plane) = reference.plane(plane) else {
        return Err(unsupported_at(
            "inter_reference_missing_plane",
            offset,
            "minimal inter motion compensation requires the reference frame to carry every plane",
            SPEC_MC,
        ));
    };
    let visible = ref_plane.visible_size();
    let ref_width = visible.width();
    let ref_height = visible.height();

    let mut samples: Vec<u16> = Vec::with_capacity(ref_width.saturating_mul(ref_height));
    let rows: VisibleRows<'_, T> = ref_plane.visible_rows();
    for row in rows {
        samples.extend(row.iter().map(|&s| s.to_u16()));
    }

    let luma_visible = reference.y().visible_size();
    let ref_mi_cols = (luma_visible.width() as i64) / 4;
    let ref_mi_rows = (luma_visible.height() as i64) / 4;

    Ok((samples, ref_width, ref_height, ref_mi_cols, ref_mi_rows))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use splot_recon::{DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane, PlaneSize};

    #[test]
    fn dispatcher_zero_mv_copies_single_reference_planes() {
        let reference = patterned_frame(8, 8);
        let mut workspace = workspace(8, 8);

        motion_compensate_inter_block_into(
            &mut workspace,
            InterBlockParams::single(
                &reference,
                rect(0, 0, 8, 8),
                Mv { row: 0, col: 0 },
                InterpolationFilter::EightTap,
            ),
            ByteOffset::new(0),
        )
        .expect("single-reference dispatcher");

        let decoded = workspace.freeze().expect("freeze dispatched workspace");
        assert_eq!(
            visible_samples(&decoded, PlaneId::Y),
            visible_samples(&reference, PlaneId::Y)
        );
        assert_eq!(
            visible_samples(&decoded, PlaneId::U),
            visible_samples(&reference, PlaneId::U)
        );
        assert_eq!(
            visible_samples(&decoded, PlaneId::V),
            visible_samples(&reference, PlaneId::V)
        );
    }

    #[test]
    fn dispatcher_blends_compound_average_planes() {
        let reference0 = flat_frame(8, 8, 40, 90, 120);
        let reference1 = flat_frame(8, 8, 80, 110, 140);
        let mut workspace = workspace(8, 8);

        motion_compensate_inter_block_into(
            &mut workspace,
            InterBlockParams::compound_average(
                &reference0,
                &reference1,
                rect(0, 0, 8, 8),
                Mv { row: 0, col: 0 },
                Mv { row: 0, col: 0 },
                InterpolationFilter::EightTap,
            ),
            ByteOffset::new(0),
        )
        .expect("compound-average dispatcher");

        let decoded = workspace.freeze().expect("freeze compound workspace");
        assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
        assert_eq!(visible_samples(&decoded, PlaneId::U), vec![100; 16]);
        assert_eq!(visible_samples(&decoded, PlaneId::V), vec![130; 16]);
    }

    fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u8> {
        let luma_size = PlaneSize::new(width, height).expect("luma size");
        let visible = PlaneRect::new(0, 0, width, height).expect("visible rect");
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            visible,
        )
        .expect("frame info");
        CurrentFrameWorkspace::new(info, 0).expect("workspace")
    }

    fn patterned_frame(width: usize, height: usize) -> DecodedFrame<u8> {
        let y: Vec<u8> = (0..width * height)
            .map(|sample| u8::try_from(sample).expect("luma sample"))
            .collect();
        let chroma_width = width / 2;
        let chroma_height = height / 2;
        let u: Vec<u8> = (0..chroma_width * chroma_height)
            .map(|sample| 100 + u8::try_from(sample).expect("u sample"))
            .collect();
        let v: Vec<u8> = (0..chroma_width * chroma_height)
            .map(|sample| 150 + u8::try_from(sample).expect("v sample"))
            .collect();
        frame(width, height, y, u, v)
    }

    fn flat_frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> DecodedFrame<u8> {
        let chroma_width = width / 2;
        let chroma_height = height / 2;
        frame(
            width,
            height,
            vec![y; width * height],
            vec![u; chroma_width * chroma_height],
            vec![v; chroma_width * chroma_height],
        )
    }

    fn frame(width: usize, height: usize, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> DecodedFrame<u8> {
        let luma_size = PlaneSize::new(width, height).expect("luma size");
        let luma_rect = PlaneRect::new(0, 0, width, height).expect("luma rect");
        let chroma_width = width / 2;
        let chroma_height = height / 2;
        let chroma_size = PlaneSize::new(chroma_width, chroma_height).expect("chroma size");
        let chroma_rect = PlaneRect::new(0, 0, chroma_width, chroma_height).expect("chroma rect");
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            luma_size,
            luma_rect,
        )
        .expect("frame info");

        DecodedFrame::try_new(
            info,
            FramePlanes::new(
                plane(luma_size, width, luma_rect, y),
                Some(plane(chroma_size, chroma_width, chroma_rect, u)),
                Some(plane(chroma_size, chroma_width, chroma_rect, v)),
            ),
        )
        .expect("decoded frame")
    }

    fn plane(size: PlaneSize, stride: usize, visible: PlaneRect, samples: Vec<u8>) -> Plane<u8> {
        Plane::from_vec(size, stride, visible, samples).expect("plane")
    }

    fn visible_samples(frame: &DecodedFrame<u8>, plane: PlaneId) -> Vec<u8> {
        frame
            .plane(plane)
            .expect("frame plane")
            .visible_rows()
            .flat_map(|row| row.iter().copied())
            .collect()
    }

    const fn rect(luma_x: usize, luma_y: usize, luma_w: usize, luma_h: usize) -> McBlockRect {
        McBlockRect {
            luma_x,
            luma_y,
            luma_w,
            luma_h,
        }
    }
}
