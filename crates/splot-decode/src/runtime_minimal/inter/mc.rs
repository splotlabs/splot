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
    ReferencePlaneView, SubpelPredictParams, VisibleRows, subpel_predict_block,
};

use super::mv_scaling::derive_plane_scaling;
use super::{Mv, SPEC_MC, unsupported_at};
use crate::Result;
use crate::error::DecodeError;
use splot_core::span::ByteOffset;

/// A motion-compensated block's luma-space rectangle (the § 7.13.3.18 region the
/// block covers). For the full-frame single block this is the whole frame; for a
/// multi-block partition each leaf block carries its own rect.
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
    // 4:2:0 luma full-resolution; chroma half-resolution (subsampling 1/1).
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

    // Gather the reference's visible rows (storage padding excluded) into a packed
    // u16 buffer for the §7.13.3.18 convolution view (the whole reference plane;
    // the kernel clips block sampling to it).
    let mut samples: Vec<u16> = Vec::with_capacity(ref_width.saturating_mul(ref_height));
    let rows: VisibleRows<'_, u8> = ref_plane.visible_rows();
    for row in rows {
        samples.extend(row.iter().map(|&s| u16::from(s)));
    }
    let view = ReferencePlaneView::new(&samples, ref_width, ref_height)
        .map_err(|source| DecodeError::Reconstruction { source })?;

    // The reference MI grid: luma MI units of the (square) frame. RefMiCols/Rows are
    // the luma mode-info dimensions, used by §7.13.3.18 to derive lastX/lastY with
    // the plane subsampling. For the same-size reference, that is the luma plane
    // dimensions in 4-sample MI units.
    let luma_visible = reference.y().visible_size();
    let ref_mi_cols = (luma_visible.width() as i64) / 4;
    let ref_mi_rows = (luma_visible.height() as i64) / 4;

    // The block's plane-space position + size (luma rect subsampled per plane).
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
    let predicted = subpel_predict_block(&view, &params)
        .map_err(|source| DecodeError::Reconstruction { source })?;

    // §7.13.3 single-reference write: CurrFrame[plane][y][x] = Clip1(Preds[0]). The
    // kernel already applied Clip1 to 8-bit, so narrow back to u8 (every value is in
    // 0..=255 by construction).
    let packed: Vec<u8> = predicted
        .iter()
        .map(|&v| u8::try_from(v).unwrap_or(u8::MAX))
        .collect();

    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)
        .map_err(|source| DecodeError::Reconstruction { source })?;
    workspace
        .write_rect(plane, rect, &packed, block_w)
        .map_err(|source| DecodeError::Reconstruction { source })?;
    Ok(())
}
