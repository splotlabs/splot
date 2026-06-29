// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter motion compensation (AV2 § 7.13.3.18) for the verified single-reference
//! sub-pel block.
//!
//! For the full-frame single 64x64 block this runs the § 7.13.3.18 separable
//! interpolation-filter convolution (via [`splot_recon::subpel_predict_block`])
//! over every plane of the unscaled reference, using the § 7.13.3.17 scaling and
//! the decoded motion vector. The zero-fraction (zero-MV) case reduces inside the
//! kernel to a straight reference-sample copy, so the existing zero-MV inter
//! fixture stays byte-identical.
//!
//! Chroma uses the same luma motion vector adjusted for 4:2:0 subsampling inside
//! the § 7.13.3.17 scaling (`(2 * mv) >> subsampling`), and the small-block 4-tap
//! filter substitution is applied by the kernel per plane dimension.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, InterpolationFilter, PlaneId, PlaneRect,
    ReferencePlaneView, SubpelPredictParams, VisibleRows, blend_compound_average_equal,
    subpel_predict_block, subpel_predict_block_compound_intermediate,
};

use super::mv_scaling::derive_plane_scaling;
use super::{Mv, SPEC_MC, unsupported_at};
use crate::Result;
use splot_core::span::ByteOffset;

/// A motion-compensated block's luma-space rectangle (the § 7.13.3.18 region the
/// block covers). For the full-frame single block this is the whole frame; for a
/// multi-block partition each leaf block carries its own rect.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct McBlockRect {
    /// Luma-space top-left x (samples).
    pub(super) luma_x: usize,
    /// Luma-space top-left y (samples).
    pub(super) luma_y: usize,
    /// Block width in luma samples.
    pub(super) luma_w: usize,
    /// Block height in luma samples.
    pub(super) luma_h: usize,
}

/// Inputs for one compound-average MC block.
#[derive(Clone, Copy, Debug)]
pub(super) struct CompoundMcBlock<'a> {
    /// List-0 decoded reference frame.
    pub(super) reference0: &'a DecodedFrame<u8>,
    /// List-1 decoded reference frame.
    pub(super) reference1: &'a DecodedFrame<u8>,
    /// Luma-space block rectangle.
    pub(super) rect: McBlockRect,
    /// List-0 motion vector.
    pub(super) mv0: Mv,
    /// List-1 motion vector.
    pub(super) mv1: Mv,
    /// Shared interpolation filter for both references.
    pub(super) interp: InterpolationFilter,
}

/// Motion-compensates one block (§ 7.13.3.18) into an existing workspace, for
/// every plane. `rect` is the block's luma-space rectangle; chroma uses the
/// 4:2:0-subsampled rectangle and the same luma MV (the § 7.13.3.17 scaling does
/// the `(2 * mv) >> subsampling` adjustment). Used by both the full-frame single
/// block and each leaf block of a multi-block partition.
pub(super) fn motion_compensate_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    reference: &DecodedFrame<u8>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    offset: ByteOffset,
) -> Result<()> {
    predict_plane(
        workspace,
        reference,
        PlaneId::Y,
        rect,
        mv,
        interp,
        0,
        0,
        offset,
    )?;
    predict_plane(
        workspace,
        reference,
        PlaneId::U,
        rect,
        mv,
        interp,
        1,
        1,
        offset,
    )?;
    predict_plane(
        workspace,
        reference,
        PlaneId::V,
        rect,
        mv,
        interp,
        1,
        1,
        offset,
    )?;
    Ok(())
}

/// Motion-compensates one COMPOUND_AVERAGE / CWP_EQUAL block (§ 7.13.3.16 +
/// § 7.13.3.18) into an existing workspace, for every plane. Both references are
/// unscaled and same-size in the caller-gated subset; each list uses its own MV
/// but the same block interpolation filter.
pub(super) fn motion_compensate_compound_average_block_into(
    workspace: &mut CurrentFrameWorkspace<u8>,
    block: CompoundMcBlock<'_>,
    offset: ByteOffset,
) -> Result<()> {
    predict_compound_plane(
        workspace,
        block.reference0,
        block.reference1,
        PlaneId::Y,
        block.rect,
        block.mv0,
        block.mv1,
        block.interp,
        0,
        0,
        offset,
    )?;
    predict_compound_plane(
        workspace,
        block.reference0,
        block.reference1,
        PlaneId::U,
        block.rect,
        block.mv0,
        block.mv1,
        block.interp,
        1,
        1,
        offset,
    )?;
    predict_compound_plane(
        workspace,
        block.reference0,
        block.reference1,
        PlaneId::V,
        block.rect,
        block.mv0,
        block.mv1,
        block.interp,
        1,
        1,
        offset,
    )?;
    Ok(())
}

/// Motion-compensates one plane of one block: gathers the unscaled reference
/// plane samples, derives the § 7.13.3.17 scaling / § 7.13.3.18 clip bounds for
/// the block's plane-space rectangle, runs the convolution, and writes the
/// predicted block into the workspace plane at its plane-space position.
#[allow(clippy::too_many_arguments)]
fn predict_plane(
    workspace: &mut CurrentFrameWorkspace<u8>,
    reference: &DecodedFrame<u8>,
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
        bit_depth: BitDepth::Eight,
    };
    let predicted = subpel_predict_block(&view, &params)?;

    let packed: Vec<u8> = predicted
        .iter()
        .map(|&v| u8::try_from(v).unwrap_or(u8::MAX))
        .collect();

    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    workspace.write_rect(plane, rect, &packed, block_w)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_compound_plane(
    workspace: &mut CurrentFrameWorkspace<u8>,
    reference0: &DecodedFrame<u8>,
    reference1: &DecodedFrame<u8>,
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
        bit_depth: BitDepth::Eight,
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
        bit_depth: BitDepth::Eight,
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0)?;
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1)?;
    let blended = blend_compound_average_equal(&pred0, &pred1, BitDepth::Eight)?;

    let packed: Vec<u8> = blended
        .iter()
        .map(|&v| u8::try_from(v).unwrap_or(u8::MAX))
        .collect();
    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    workspace.write_rect(plane, rect, &packed, block_w)?;
    Ok(())
}

fn reference_plane_samples(
    reference: &DecodedFrame<u8>,
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
    let rows: VisibleRows<'_, u8> = ref_plane.visible_rows();
    for row in rows {
        samples.extend(row.iter().map(|&s| u16::from(s)));
    }

    let luma_visible = reference.y().visible_size();
    let ref_mi_cols = (luma_visible.width() as i64) / 4;
    let ref_mi_rows = (luma_visible.height() as i64) / 4;

    Ok((samples, ref_width, ref_height, ref_mi_cols, ref_mi_rows))
}
