// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFrameIntraEdges, CurrentFrameWorkspace, IntraRectBlockSize, PlaneId, PlaneRect,
    ReconSample, ReferencePlaneView,
};

use super::reference::ReferenceSamples;
use super::{BawpSyntax, Mv, PlacedInterBlock};
use crate::Result;

const SHIFT: u32 = 8;
const MAX_BAWP_RECT_DIM: usize = 64;

/// Rows below its top-left reference position a § 7.13.3.25 template reaches:
/// [`bawp_template_counts`] caps the template side at 16 luma samples.
const MAX_TEMPLATE_EXTENT: usize = 16;

const fn to_fullmv(mv: i32) -> i32 {
    (mv + 3 + if mv >= 0 { 1 } else { 0 }) >> 3
}

/// The luma rows of its reference one § 7.13.3.25 BAWP block's template reads.
///
/// The template samples the reference at the block's full-pel position
/// `refY = lumaY + toFullMv(mvRow)`: one row above it for the above arm, and
/// rows `refY ..= refY + height - 1` for the left arm, where `height` is
/// [`bawp_template_counts`]'s capped block side — at most
/// [`MAX_TEMPLATE_EXTENT`] luma samples, and never more than the block's own
/// height. A chroma arm reads the same window subsampled, whose luma-row
/// requirement `(refY >> subY) + (height >> subY)` shifted back up never passes
/// the luma one, so a single luma-row count covers every plane.
pub(super) fn bawp_reference_luma_rows(luma_y: usize, luma_h: usize, mv_row: i32) -> u32 {
    let top = (luma_y as i64) + i64::from(to_fullmv(mv_row));
    let extent = luma_h.min(MAX_TEMPLATE_EXTENT) as i64;
    top.saturating_add(extent).clamp(0, i64::from(u32::MAX)) as u32
}

#[derive(Default)]
struct TemplateStats {
    sum_x: i32,
    sum_y: i32,
    sum_xx: i32,
    sum_xy: i32,
    count: i32,
}

#[allow(clippy::too_many_arguments)]
fn collect_template_stats<T, F>(
    edges: &CurrentFrameIntraEdges<T>,
    width: usize,
    height: usize,
    num_up: usize,
    num_left: usize,
    ref_x: i32,
    ref_y: i32,
    mut reference_sample: F,
) -> Option<TemplateStats>
where
    T: ReconSample,
    F: FnMut(i32, i32) -> Option<i32>,
{
    let mut stats = TemplateStats::default();
    if num_up > 0 {
        let step = width.checked_div(num_up)?;
        let above = edges.above_samples()?;
        let mut i = step >> 1;
        while i < width {
            let recon = i32::from(above.get(i)?.to_u16());
            let sample_col = i32::try_from(i)
                .ok()
                .and_then(|offset| ref_x.checked_add(offset))?;
            let reference = reference_sample(ref_y - 1, sample_col)?;
            stats.sum_x += reference;
            stats.sum_y += recon;
            stats.sum_xy += reference * recon;
            stats.sum_xx += reference * reference;
            i += step;
        }
        stats.count = stats.count.checked_add(i32::try_from(num_up).ok()?)?;
    }
    if num_left > 0 {
        let step = height.checked_div(num_left)?;
        let left = edges.left_samples()?;
        let mut i = step >> 1;
        while i < height {
            let recon = i32::from(left.get(i)?.to_u16());
            let sample_row = i32::try_from(i)
                .ok()
                .and_then(|offset| ref_y.checked_add(offset))?;
            let reference = reference_sample(sample_row, ref_x - 1)?;
            stats.sum_x += reference;
            stats.sum_y += recon;
            stats.sum_xy += reference * recon;
            stats.sum_xx += reference * reference;
            i += step;
        }
        stats.count = stats.count.checked_add(i32::try_from(num_left).ok()?)?;
    }
    Some(stats)
}

pub(crate) fn apply_bawp<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: ReferenceSamples<'_, T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: Mv,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut luma_alpha = 1i16 << SHIFT;
    for (plane, sub_x, sub_y) in super::mc::mc_planes(workspace.info().pixel_format()) {
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

pub(crate) fn apply_intrabc_morph_pred<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    target: PlaneRect,
    mv: Mv,
    _tile_offset: ByteOffset,
) -> Result<()> {
    if !target.width().is_power_of_two() || !target.height().is_power_of_two() {
        return Ok(());
    }

    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(target.y(), dy) else {
        return Ok(());
    };
    let Some(ref_x) = fullpel_ref_pos(target.x(), dx) else {
        return Ok(());
    };
    if ref_x < 1 || ref_y < 1 {
        return Ok(());
    }

    let plane = workspace.plane(PlaneId::Y)?;
    let Some(plane_width) = i32::try_from(plane.storage_size().width()).ok() else {
        return Ok(());
    };
    let Some(plane_height) = i32::try_from(plane.storage_size().height()).ok() else {
        return Ok(());
    };
    let Some(bw) = i32::try_from(target.width()).ok() else {
        return Ok(());
    };
    let Some(bh) = i32::try_from(target.height()).ok() else {
        return Ok(());
    };
    let Some(ref_right) = ref_x.checked_add(bw) else {
        return Ok(());
    };
    let Some(ref_bottom) = ref_y.checked_add(bh) else {
        return Ok(());
    };
    if ref_right > plane_width || ref_bottom > plane_height {
        return Ok(());
    }

    let avail_up = target.y() > 0;
    let avail_left = target.x() > 0;
    let (width, height, num_up, num_left) =
        bawp_template_counts(target.width(), target.height(), true, avail_up, avail_left);
    let Some(size) = morph_dimensions_size(width, height) else {
        return Ok(());
    };
    let edges = workspace.intra_dc_edges_for_rect(PlaneId::Y, target.x(), target.y(), size)?;

    let Some(TemplateStats {
        sum_x,
        sum_y,
        sum_xx,
        sum_xy,
        count,
    }) = collect_template_stats(
        &edges,
        width,
        height,
        num_up,
        num_left,
        ref_x,
        ref_y,
        |row, col| intrabc_morph_sample(workspace, row, col),
    )
    else {
        return Ok(());
    };

    let mut alpha = 1i16 << SHIFT;
    if count > 0 {
        let nor = sum_xy - sum_x * sum_y / count;
        let der = sum_xx - sum_x * sum_x / count;
        if der != 0 && nor != 0 {
            alpha = splot_recon::math::resolve_division(nor, der, SHIFT as u8);
            if alpha == 0 {
                alpha = 1 << SHIFT;
            }
        }
    }
    let beta = if count > 0 {
        ((sum_y << SHIFT) - sum_x * i32::from(alpha)) / count
    } else {
        -(1 << (SHIFT - 1))
    };

    apply_bawp_region(workspace, PlaneId::Y, target, alpha, beta)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_bawp_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: ReferenceSamples<'_, T>,
    placed: &PlacedInterBlock,
    bawp: BawpSyntax,
    mv: Mv,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    luma_alpha: i16,
    tile_offset: ByteOffset,
) -> Result<i16> {
    let plane_x = placed.luma_x >> sub_x;
    let plane_y = placed.luma_y >> sub_y;
    let plane_w = placed.luma_w >> sub_x;
    let plane_h = placed.luma_h >> sub_y;
    let last_row = (bawp_reference_luma_rows(placed.luma_y, placed.luma_h, mv.row) >> sub_y)
        .saturating_sub(1) as i32;
    let (ref_samples, ref_width, ref_height) =
        reference_plane(reference, plane, last_row, tile_offset)?;
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(placed.luma_y, dy).map(|y| y >> sub_y) else {
        return Ok(luma_alpha);
    };
    let Some(ref_x) = fullpel_ref_pos(placed.luma_x, dx).map(|x| x >> sub_x) else {
        return Ok(luma_alpha);
    };
    let Some(plane_width) = i32::try_from(ref_width).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_height) = i32::try_from(ref_height).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_x_i32) = i32::try_from(plane_x).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_y_i32) = i32::try_from(plane_y).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_w_i32) = i32::try_from(plane_w).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_h_i32) = i32::try_from(plane_h).ok() else {
        return Ok(luma_alpha);
    };
    let (bw, bh) = (
        plane_w_i32.min(plane_width - plane_x_i32),
        plane_h_i32.min(plane_height - plane_y_i32),
    );
    if bw <= 0 || bh <= 0 {
        return Ok(luma_alpha);
    }
    let Some(ref_right) = ref_x.checked_add(bw) else {
        return Ok(luma_alpha);
    };
    let Some(ref_bottom) = ref_y.checked_add(bh) else {
        return Ok(luma_alpha);
    };
    if ref_x < 1 || ref_y < 1 || ref_right > plane_width || ref_bottom > plane_height {
        return Ok(luma_alpha);
    }

    if !plane_w.is_power_of_two() || !plane_h.is_power_of_two() {
        return Ok(luma_alpha);
    }
    let avail_up = plane_y > 0;
    let avail_left = plane_x > 0;
    let Some(width_for_counts) = usize::try_from(bw).ok() else {
        return Ok(luma_alpha);
    };
    let Some(height_for_counts) = usize::try_from(bh).ok() else {
        return Ok(luma_alpha);
    };
    let (width, height, num_up, num_left) = bawp_template_counts(
        width_for_counts,
        height_for_counts,
        plane == PlaneId::Y,
        avail_up,
        avail_left,
    );
    let Some(size) = morph_dimensions_size(width, height) else {
        return Ok(luma_alpha);
    };

    let edges = workspace.intra_dc_edges_for_rect(plane, plane_x, plane_y, size)?;
    let ref_at = |row: i32, col: i32| -> Option<i32> {
        let row = usize::try_from(row).ok()?;
        let col = usize::try_from(col).ok()?;
        Some(ref_samples.sample(row, col))
    };

    let Some(TemplateStats {
        sum_x,
        sum_y,
        sum_xx,
        sum_xy,
        count,
    }) = collect_template_stats(
        &edges, width, height, num_up, num_left, ref_x, ref_y, ref_at,
    )
    else {
        return Ok(luma_alpha);
    };

    let mut alpha = 1i16 << SHIFT;
    if plane != PlaneId::Y {
        alpha = if count == 0 { 1 << SHIFT } else { luma_alpha };
    } else if bawp.explicit {
        let mut scale = i16::from(bawp.list_index) + 1;
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
            alpha = splot_recon::math::resolve_division(nor, der, SHIFT as u8);
            if alpha == 0 {
                alpha = 1 << SHIFT;
            }
        }
    }
    let beta = if count > 0 {
        ((sum_y << SHIFT) - sum_x * i32::from(alpha)) / count
    } else {
        -(1 << (SHIFT - 1))
    };

    apply_bawp_region(
        workspace,
        plane,
        PlaneRect::new(plane_x, plane_y, plane_w, plane_h)?,
        alpha,
        beta,
    )?;
    Ok(if plane == PlaneId::Y {
        alpha
    } else {
        luma_alpha
    })
}

fn morph_dimensions_size(width: usize, height: usize) -> Option<IntraRectBlockSize> {
    if !width.is_power_of_two() || !height.is_power_of_two() {
        return None;
    }
    IntraRectBlockSize::new(
        u8::try_from(width.trailing_zeros()).ok()?,
        u8::try_from(height.trailing_zeros()).ok()?,
    )
    .ok()
}

fn apply_bawp_region<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    rect: PlaneRect,
    alpha: i16,
    beta: i32,
) -> Result<()> {
    let storage = workspace.plane(plane)?.storage_size();
    let x = rect.x();
    let y = rect.y();
    let width = rect.width();
    let height = rect.height();
    if x >= storage.width() || y >= storage.height() {
        return Ok(());
    }
    let visible_width = width.min(storage.width() - x);
    let visible_height = height.min(storage.height() - y);
    let mut y_offset = 0usize;
    while y_offset < height && y_offset < visible_height {
        let chunk_h = (height - y_offset).min(MAX_BAWP_RECT_DIM);
        let Some(chunk_y) = y.checked_add(y_offset) else {
            return Ok(());
        };
        let mut x_offset = 0usize;
        while x_offset < width && x_offset < visible_width {
            let chunk_w = (width - x_offset).min(MAX_BAWP_RECT_DIM);
            let Some(chunk_x) = x.checked_add(x_offset) else {
                return Ok(());
            };
            let Some(size) = morph_dimensions_size(chunk_w, chunk_h) else {
                return Ok(());
            };
            workspace.apply_bawp_rect(plane, chunk_x, chunk_y, size, alpha, beta)?;
            x_offset += chunk_w;
        }
        y_offset += chunk_h;
    }
    Ok(())
}

fn fullpel_ref_pos(origin: usize, delta: i32) -> Option<i32> {
    i32::try_from(origin)
        .ok()
        .and_then(|origin| origin.checked_add(delta))
}

fn intrabc_morph_sample<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    row: i32,
    col: i32,
) -> Option<i32> {
    let row = usize::try_from(row).ok()?;
    let col = usize::try_from(col).ok()?;
    workspace
        .reconstructed_sample(PlaneId::Y, col, row)
        .ok()
        .map(|sample| i32::from(sample.to_u16()))
}

fn reference_plane<T: ReconSample>(
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    last_row: i32,
    tile_offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, usize, usize)> {
    let (view, _, _) = reference.plane_view(plane, last_row, tile_offset)?;
    let (width, height) = (view.width(), view.height());
    Ok((view, width, height))
}

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
#[path = "bawp_tests.rs"]
mod tests;
