// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, IntraRectBlockSize, PlaneId, ReconSample,
    ReferencePlaneView,
};

use super::super::Result;
use super::{BawpSyntax, Mv, PlacedInterBlock, unsupported_at};

const SHIFT: u32 = 8;

/// § 7.13.3.25 `to_fullmv`.
const fn to_fullmv(mv: i32) -> i32 {
    (mv + 3 + if mv >= 0 { 1 } else { 0 }) >> 3
}

/// Applies § 7.13.3.25 to the motion-compensated prediction: luma always,
/// chroma when `use_bawp_chroma`, deriving the implicit scale from the
/// above/left templates (or the explicit-scale arm) and `Clip1`-scaling the
/// block in place. Runs after motion compensation and before the residual;
/// BAWP blocks never carry interintra or warp (§ 5.20.7.14 excludes them).
/// Template availability is frame-origin-based: the decode entry enforces a
/// single tile, so the § 5.20.7.15 tile-relative `AvailU`/`AvailL` reduce to
/// the frame origin here.
pub(super) fn apply_bawp<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: Mv,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut luma_alpha = 1i64 << SHIFT;
    for (plane, sub_x, sub_y) in super::mc::YUV420_MC_PLANES {
        if plane != PlaneId::Y && !bawp.chroma {
            break;
        }
        luma_alpha = apply_bawp_plane(
            workspace,
            reference,
            placed,
            bawp,
            mv,
            plane,
            sub_x,
            sub_y,
            luma_alpha,
            tile_offset,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_bawp_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: Mv,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    luma_alpha: i64,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let bounds_error = || {
        inter_diag!(
            "inter_bawp_reference_bounds",
            tile_offset,
            "inter.bawp reference template out of bounds",
            "7.13.3.25"
        )
    };
    let plane_x = placed.luma_x >> sub_x;
    let plane_y = placed.luma_y >> sub_y;
    let plane_w = placed.luma_w >> sub_x;
    let plane_h = placed.luma_h >> sub_y;
    let (ref_samples, ref_width, ref_height) = reference_plane(reference, plane, tile_offset)?;
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let ref_y =
        (i64::try_from(placed.luma_y).map_err(|_| bounds_error())? + i64::from(dy)) >> sub_y;
    let ref_x =
        (i64::try_from(placed.luma_x).map_err(|_| bounds_error())? + i64::from(dx)) >> sub_x;
    let plane_width = i64::try_from(ref_width).map_err(|_| bounds_error())?;
    let plane_height = i64::try_from(ref_height).map_err(|_| bounds_error())?;
    let (bw, bh) = (
        i64::try_from(plane_w)
            .map_err(|_| bounds_error())?
            .min(plane_width - i64::try_from(plane_x).map_err(|_| bounds_error())?),
        i64::try_from(plane_h)
            .map_err(|_| bounds_error())?
            .min(plane_height - i64::try_from(plane_y).map_err(|_| bounds_error())?),
    );
    if ref_x < 1 || ref_y < 1 || ref_x + bw > plane_width || ref_y + bh > plane_height {
        return Err(bounds_error());
    }

    if !plane_w.is_power_of_two() || !plane_h.is_power_of_two() {
        return Err(bounds_error());
    }
    let size = IntraRectBlockSize::new(
        u8::try_from(plane_w.trailing_zeros()).map_err(|_| bounds_error())?,
        u8::try_from(plane_h.trailing_zeros()).map_err(|_| bounds_error())?,
    )
    .map_err(|_| bounds_error())?;

    let avail_up = plane_y > 0;
    let avail_left = plane_x > 0;
    let (width, height, num_up, num_left) = bawp_template_counts(
        usize::try_from(bw).map_err(|_| bounds_error())?,
        usize::try_from(bh).map_err(|_| bounds_error())?,
        plane == PlaneId::Y,
        avail_up,
        avail_left,
    );

    let edges = workspace
        .intra_dc_edges_for_rect(plane, plane_x, plane_y, size)
        .map_err(|_| bounds_error())?;
    let ref_at = |row: i64, col: i64| -> Result<i64> {
        let row = usize::try_from(row).map_err(|_| bounds_error())?;
        let col = usize::try_from(col).map_err(|_| bounds_error())?;
        Ok(ref_samples.sample(row, col))
    };

    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut sum_xx = 0i64;
    let mut sum_xy = 0i64;
    let mut count = 0i64;
    if let Some(step) = width.checked_div(num_up).filter(|_| num_up > 0) {
        let above = edges.above_samples().ok_or_else(bounds_error)?;
        let mut i = step >> 1;
        while i < width {
            let recon = i64::from(above.get(i).ok_or_else(bounds_error)?.to_u16());
            let reference_sample = ref_at(
                ref_y - 1,
                ref_x + i64::try_from(i).map_err(|_| bounds_error())?,
            )?;
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        count += i64::try_from(num_up).map_err(|_| bounds_error())?;
    }
    if let Some(step) = height.checked_div(num_left).filter(|_| num_left > 0) {
        let left = edges.left_samples().ok_or_else(bounds_error)?;
        let mut i = step >> 1;
        while i < height {
            let recon = i64::from(left.get(i).ok_or_else(bounds_error)?.to_u16());
            let reference_sample = ref_at(
                ref_y + i64::try_from(i).map_err(|_| bounds_error())?,
                ref_x - 1,
            )?;
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        count += i64::try_from(num_left).map_err(|_| bounds_error())?;
    }

    let mut alpha = 1i64 << SHIFT;
    if plane != PlaneId::Y {
        alpha = if count == 0 { 1 << SHIFT } else { luma_alpha };
    } else if bawp.explicit {
        let mut scale = i64::from(bawp.list_index) + 1;
        if bawp.ref_dist_gt4 {
            scale += 1;
        }
        if !bawp.explicit_scale_positive {
            scale = -scale;
        }
        alpha = 256 + 16 * scale;
    } else if count > 0 {
        let nor = sum_xy - sum_x * sum_y / count;
        let der = sum_xx - sum_x * sum_x / count;
        if der != 0 && nor != 0 {
            alpha = i64::from(splot_recon::math::resolve_division(nor, der, SHIFT as u8));
            if alpha == 0 {
                alpha = 1 << SHIFT;
            }
        }
    }
    let beta = if count > 0 {
        ((sum_y << SHIFT) - sum_x * alpha) / count
    } else {
        -(1 << (SHIFT - 1))
    };

    workspace
        .apply_bawp_rect(plane, plane_x, plane_y, size, alpha, beta)
        .map_err(|_| bounds_error())?;
    Ok(if plane == PlaneId::Y {
        alpha
    } else {
        luma_alpha
    })
}

fn reference_plane<'a, T: ReconSample>(
    reference: &'a DecodedFrame<T>,
    plane: PlaneId,
    tile_offset: ByteOffset,
) -> Result<(ReferencePlaneView<'a, T>, usize, usize)> {
    let (view, _, _) = super::mc::reference_plane_view(reference, plane, tile_offset)?;
    let (width, height) = (view.width(), view.height());
    Ok((view, width, height))
}

/// § 7.13.3.25 template geometry: the in-plane clamped block size (`bw`,
/// `bh`) selects the sampled template extents and counts; the `12 -> 8`
/// arms serve exactly the clamped frame-edge sizes.
fn bawp_template_counts(
    bw: usize,
    bh: usize,
    luma: bool,
    avail_up: bool,
    avail_left: bool,
) -> (usize, usize, usize, usize) {
    let cap = if luma { 16 } else { 8 };
    let bw2 = cap.min(bw);
    let bh2 = cap.min(bh);
    let width = if bw2 == 12 { 8 } else { bw2 };
    let height = if bh2 == 12 { 8 } else { bh2 };
    let (num_up, num_left) = if avail_up && avail_left {
        if width == 16 && height == 16 {
            (16, 16)
        } else if width > 4 && height > 4 {
            (8, 8)
        } else if width < 16 && height < 16 {
            (4, 4)
        } else if width == 16 {
            (16, 0)
        } else {
            (0, 16)
        }
    } else if avail_up {
        (width, 0)
    } else if avail_left {
        (0, height)
    } else {
        (0, 0)
    };
    (width, height, num_up, num_left)
}

#[cfg(test)]
mod tests {
    use super::bawp_template_counts;

    #[test]
    fn template_counts_follow_the_clamped_size_table() {
        for (case, expected) in [
            ((16, 16, true, true, true), (16, 16, 16, 16)),
            ((12, 16, true, true, true), (8, 16, 8, 8)),
            ((16, 12, true, true, true), (16, 8, 8, 8)),
            ((4, 4, true, true, true), (4, 4, 4, 4)),
            ((16, 4, true, true, true), (16, 4, 16, 0)),
            ((4, 16, true, true, true), (4, 16, 0, 16)),
            ((32, 8, true, true, false), (16, 8, 16, 0)),
            ((8, 32, true, false, true), (8, 16, 0, 16)),
            ((32, 32, false, true, true), (8, 8, 8, 8)),
            ((12, 12, false, true, true), (8, 8, 8, 8)),
            ((64, 64, true, false, false), (16, 16, 0, 0)),
        ] {
            let (bw, bh, luma, up, left) = case;
            assert_eq!(
                bawp_template_counts(bw, bh, luma, up, left),
                expected,
                "bw={bw} bh={bh} luma={luma} up={up} left={left}"
            );
        }
    }
}
