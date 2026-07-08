// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, IntraRectBlockSize, PlaneId, PlaneRect, ReconSample,
    ReferencePlaneView,
};

use super::{BawpSyntax, Mv, PlacedInterBlock};
use crate::Result;

const SHIFT: u32 = 8;
const MAX_BAWP_RECT_DIM: usize = 64;

const fn to_fullmv(mv: i32) -> i32 {
    (mv + 3 + if mv >= 0 { 1 } else { 0 }) >> 3
}

pub(crate) fn apply_bawp<T: ReconSample>(
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
    let Some(plane_width) = i64::try_from(plane.storage_size().width()).ok() else {
        return Ok(());
    };
    let Some(plane_height) = i64::try_from(plane.storage_size().height()).ok() else {
        return Ok(());
    };
    let Some(bw) = i64::try_from(target.width()).ok() else {
        return Ok(());
    };
    let Some(bh) = i64::try_from(target.height()).ok() else {
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

    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut sum_xx = 0i64;
    let mut sum_xy = 0i64;
    let mut count = 0i64;
    if let Some(step) = width.checked_div(num_up).filter(|_| num_up > 0) {
        let Some(above) = edges.above_samples() else {
            return Ok(());
        };
        let mut i = step >> 1;
        while i < width {
            let Some(recon) = above.get(i).map(|sample| i64::from(sample.to_u16())) else {
                return Ok(());
            };
            let Some(sample_col) = i64::try_from(i)
                .ok()
                .and_then(|offset| ref_x.checked_add(offset))
            else {
                return Ok(());
            };
            let Some(reference_sample) = intrabc_morph_sample(workspace, ref_y - 1, sample_col)
            else {
                return Ok(());
            };
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        let Some(delta_count) = i64::try_from(num_up).ok() else {
            return Ok(());
        };
        let Some(next_count) = count.checked_add(delta_count) else {
            return Ok(());
        };
        count = next_count;
    }
    if let Some(step) = height.checked_div(num_left).filter(|_| num_left > 0) {
        let Some(left) = edges.left_samples() else {
            return Ok(());
        };
        let mut i = step >> 1;
        while i < height {
            let Some(recon) = left.get(i).map(|sample| i64::from(sample.to_u16())) else {
                return Ok(());
            };
            let Some(sample_row) = i64::try_from(i)
                .ok()
                .and_then(|offset| ref_y.checked_add(offset))
            else {
                return Ok(());
            };
            let Some(reference_sample) = intrabc_morph_sample(workspace, sample_row, ref_x - 1)
            else {
                return Ok(());
            };
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        let Some(delta_count) = i64::try_from(num_left).ok() else {
            return Ok(());
        };
        let Some(next_count) = count.checked_add(delta_count) else {
            return Ok(());
        };
        count = next_count;
    }

    let mut alpha = 1i64 << SHIFT;
    if count > 0 {
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

    apply_bawp_region(workspace, PlaneId::Y, target, alpha, beta)?;
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
    let plane_x = placed.luma_x >> sub_x;
    let plane_y = placed.luma_y >> sub_y;
    let plane_w = placed.luma_w >> sub_x;
    let plane_h = placed.luma_h >> sub_y;
    let (ref_samples, ref_width, ref_height) = reference_plane(reference, plane, tile_offset)?;
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(placed.luma_y, dy).map(|y| y >> sub_y) else {
        return Ok(luma_alpha);
    };
    let Some(ref_x) = fullpel_ref_pos(placed.luma_x, dx).map(|x| x >> sub_x) else {
        return Ok(luma_alpha);
    };
    let Some(plane_width) = i64::try_from(ref_width).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_height) = i64::try_from(ref_height).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_x_i64) = i64::try_from(plane_x).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_y_i64) = i64::try_from(plane_y).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_w_i64) = i64::try_from(plane_w).ok() else {
        return Ok(luma_alpha);
    };
    let Some(plane_h_i64) = i64::try_from(plane_h).ok() else {
        return Ok(luma_alpha);
    };
    let (bw, bh) = (
        plane_w_i64.min(plane_width - plane_x_i64),
        plane_h_i64.min(plane_height - plane_y_i64),
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
    let ref_at = |row: i64, col: i64| -> Option<i64> {
        let row = usize::try_from(row).ok()?;
        let col = usize::try_from(col).ok()?;
        Some(ref_samples.sample(row, col))
    };

    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut sum_xx = 0i64;
    let mut sum_xy = 0i64;
    let mut count = 0i64;
    if let Some(step) = width.checked_div(num_up).filter(|_| num_up > 0) {
        let Some(above) = edges.above_samples() else {
            return Ok(luma_alpha);
        };
        let mut i = step >> 1;
        while i < width {
            let Some(recon) = above.get(i).map(|sample| i64::from(sample.to_u16())) else {
                return Ok(luma_alpha);
            };
            let Some(sample_col) = i64::try_from(i)
                .ok()
                .and_then(|offset| ref_x.checked_add(offset))
            else {
                return Ok(luma_alpha);
            };
            let Some(reference_sample) = ref_at(ref_y - 1, sample_col) else {
                return Ok(luma_alpha);
            };
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        let Some(delta_count) = i64::try_from(num_up).ok() else {
            return Ok(luma_alpha);
        };
        let Some(next_count) = count.checked_add(delta_count) else {
            return Ok(luma_alpha);
        };
        count = next_count;
    }
    if let Some(step) = height.checked_div(num_left).filter(|_| num_left > 0) {
        let Some(left) = edges.left_samples() else {
            return Ok(luma_alpha);
        };
        let mut i = step >> 1;
        while i < height {
            let Some(recon) = left.get(i).map(|sample| i64::from(sample.to_u16())) else {
                return Ok(luma_alpha);
            };
            let Some(sample_row) = i64::try_from(i)
                .ok()
                .and_then(|offset| ref_y.checked_add(offset))
            else {
                return Ok(luma_alpha);
            };
            let Some(reference_sample) = ref_at(sample_row, ref_x - 1) else {
                return Ok(luma_alpha);
            };
            sum_x += reference_sample;
            sum_y += recon;
            sum_xy += reference_sample * recon;
            sum_xx += reference_sample * reference_sample;
            i += step;
        }
        let Some(delta_count) = i64::try_from(num_left).ok() else {
            return Ok(luma_alpha);
        };
        let Some(next_count) = count.checked_add(delta_count) else {
            return Ok(luma_alpha);
        };
        count = next_count;
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
    alpha: i64,
    beta: i64,
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

fn fullpel_ref_pos(origin: usize, delta: i32) -> Option<i64> {
    i64::try_from(origin)
        .ok()
        .and_then(|origin| origin.checked_add(i64::from(delta)))
}

fn intrabc_morph_sample<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    row: i64,
    col: i64,
) -> Option<i64> {
    let row = usize::try_from(row).ok()?;
    let col = usize::try_from(col).ok()?;
    workspace
        .reconstructed_sample(PlaneId::Y, col, row)
        .ok()
        .map(|sample| i64::from(sample.to_u16()))
}

fn reference_plane<T: ReconSample>(
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    tile_offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, usize, usize)> {
    let (view, _, _) = super::mc::reference_plane_view(reference, plane, tile_offset)?;
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
