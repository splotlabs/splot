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
use crate::DecodeLimits;
use crate::Result;
use crate::error::DecodeError;
use splot_core::span::ByteOffset;

/// Builds the current inter frame by § 7.13.3.18 motion-compensating every plane
/// of `reference` with the decoded `mv` and `interp` filter. The reference must
/// be unscaled and the same size as the current frame (checked by the caller).
pub(super) fn motion_compensate_inter_block(
    reference: &DecodedFrame<u8>,
    mv: Mv,
    interp: InterpolationFilter,
    frame_width: u32,
    frame_height: u32,
    offset: ByteOffset,
    _limits: DecodeLimits,
) -> Result<DecodedFrame<u8>> {
    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace(
        frame_width as usize,
        frame_height as usize,
    )?;

    // 4:2:0 luma full-resolution; chroma half-resolution (subsampling 1/1).
    predict_plane(
        &mut workspace,
        reference,
        PlaneId::Y,
        mv,
        interp,
        0,
        0,
        offset,
    )?;
    predict_plane(
        &mut workspace,
        reference,
        PlaneId::U,
        mv,
        interp,
        1,
        1,
        offset,
    )?;
    predict_plane(
        &mut workspace,
        reference,
        PlaneId::V,
        mv,
        interp,
        1,
        1,
        offset,
    )?;

    Ok(workspace.freeze()?)
}

/// Motion-compensates one plane: gathers the unscaled reference plane samples,
/// derives the § 7.13.3.17 scaling / § 7.13.3.18 clip bounds, runs the
/// convolution, and writes the predicted block into the workspace plane.
#[allow(clippy::too_many_arguments)]
fn predict_plane(
    workspace: &mut CurrentFrameWorkspace<u8>,
    reference: &DecodedFrame<u8>,
    plane: PlaneId,
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
    let width = visible.width();
    let height = visible.height();

    // Gather the reference's visible rows (storage padding excluded) into a packed
    // u16 buffer for the §7.13.3.18 convolution view.
    let mut samples: Vec<u16> = Vec::with_capacity(width.saturating_mul(height));
    let rows: VisibleRows<'_, u8> = ref_plane.visible_rows();
    for row in rows {
        samples.extend(row.iter().map(|&s| u16::from(s)));
    }
    let view = ReferencePlaneView::new(&samples, width, height)
        .map_err(|source| DecodeError::Reconstruction { source })?;

    // The reference MI grid: luma MI units of the (square) frame. RefMiCols/Rows are
    // the luma mode-info dimensions, used by §7.13.3.18 to derive lastX/lastY with
    // the plane subsampling. For the same-size reference, that is the luma plane
    // dimensions in 4-sample MI units.
    let luma_visible = reference.y().visible_size();
    let ref_mi_cols = (luma_visible.width() as i64) / 4;
    let ref_mi_rows = (luma_visible.height() as i64) / 4;

    // The block covers the full plane at plane-space (0, 0).
    let scaling = derive_plane_scaling(
        0,
        0,
        i64::from(mv.row),
        i64::from(mv.col),
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        width as i64,
        height as i64,
    );

    let params = SubpelPredictParams {
        interp,
        w: width,
        h: height,
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

    let rect = PlaneRect::new(0, 0, width, height)
        .map_err(|source| DecodeError::Reconstruction { source })?;
    workspace
        .write_rect(plane, rect, &packed, width)
        .map_err(|source| DecodeError::Reconstruction { source })?;
    Ok(())
}
