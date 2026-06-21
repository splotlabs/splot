// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Zero-fraction inter motion compensation (AV2 § 7.13.3.18).
//!
//! For the verified zero-MV full-frame block, § 7.13.3.18 reduces to a straight
//! per-sample copy of the co-located reference block: `CurrFrame[plane][y][x] =
//! RefFrame[plane][y][x]`. The whole 64x64 block covers the frame, so this copies
//! every plane of the unscaled reference into the current frame.

use splot_core::span::ByteOffset;
use splot_recon::{CurrentFrameWorkspace, DecodedFrame, PlaneId, PlaneRect, VisibleRows};

use super::{SPEC_MC, unsupported_at};
use crate::DecodeLimits;
use crate::Result;

/// Builds the current inter frame by copying every plane of `reference` (zero-MV,
/// zero-fraction § 7.13.3.18 motion compensation). The reference must be unscaled
/// and the same size as the current frame (checked by the caller).
pub(super) fn motion_compensate_zero_mv_copy(
    reference: &DecodedFrame<u8>,
    frame_width: u32,
    frame_height: u32,
    offset: ByteOffset,
    _limits: DecodeLimits,
) -> Result<DecodedFrame<u8>> {
    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace(
        frame_width as usize,
        frame_height as usize,
    )?;

    // §7.13.3.18 zero-fraction copy for each plane. Luma is full resolution; 4:2:0
    // chroma is half resolution. Each plane's visible rectangle is copied row by row
    // from the reference's reconstructed samples (skipping any storage padding).
    copy_plane(&mut workspace, reference, PlaneId::Y, offset)?;
    copy_plane(&mut workspace, reference, PlaneId::U, offset)?;
    copy_plane(&mut workspace, reference, PlaneId::V, offset)?;

    Ok(workspace.freeze()?)
}

/// Copies one plane's visible samples from the reference frame into the workspace at
/// the same position (zero MV). The reference plane and workspace plane share the
/// same visible geometry (verified by the caller's resolution check).
fn copy_plane(
    workspace: &mut CurrentFrameWorkspace<u8>,
    reference: &DecodedFrame<u8>,
    plane: PlaneId,
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
    let rect = PlaneRect::new(0, 0, width, height)?;

    // Gather the reference's visible rows (padding excluded) into a contiguous
    // buffer, then write the rectangle into the workspace plane. `write_rect` accepts
    // a row-strided source; using `width` as the stride keeps it a packed copy.
    let mut packed = Vec::with_capacity(width.saturating_mul(height));
    let rows: VisibleRows<'_, u8> = ref_plane.visible_rows();
    for row in rows {
        packed.extend_from_slice(row);
    }
    workspace.write_rect(plane, rect, &packed, width)?;
    Ok(())
}
