// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.25 block adaptive weighted prediction.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFramePlane, CurrentFrameWorkspace, IntraRectBlockSize, PlaneId, PlaneRect, ReconSample,
    ReferencePlaneView,
};

use super::reference::ReferenceSamples;
use super::{BawpSyntax, Mv, PlacedInterBlock};
use crate::Result;

const SHIFT: u32 = 8;

/// Rows below its top-left reference position a § 7.13.3.25 template reaches:
/// [`bawp_template_counts`] caps the template side at 16 luma samples.
const MAX_TEMPLATE_EXTENT: usize = 16;

pub(super) const fn to_fullmv(mv: i32) -> i32 {
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

impl TemplateStats {
    fn implicit_alpha(&self) -> i16 {
        let mut alpha = 1i16 << SHIFT;
        if self.count > 0 {
            let nor = self.sum_xy - self.sum_x * self.sum_y / self.count;
            let der = self.sum_xx - self.sum_x * self.sum_x / self.count;
            if der != 0 && nor != 0 {
                alpha = splot_recon::math::resolve_division(nor, der, SHIFT as u8);
                if alpha == 0 {
                    alpha = 1 << SHIFT;
                }
            }
        }
        alpha
    }

    fn beta(&self, alpha: i16) -> i32 {
        if self.count > 0 {
            ((self.sum_y << SHIFT) - self.sum_x * i32::from(alpha)) / self.count
        } else {
            -(1 << (SHIFT - 1))
        }
    }
}

impl TemplateStats {
    /// Adds the `(reference, current)` sample pairs at `step >> 1`,
    /// `step + (step >> 1)`, ... below `len`.
    fn add_arm(
        &mut self,
        len: usize,
        num: usize,
        mut pair: impl FnMut(usize) -> Option<(i32, i32)>,
    ) -> Option<()> {
        let step = len.checked_div(num)?;
        let mut i = step >> 1;
        while i < len {
            let (x, y) = pair(i)?;
            self.sum_x += x;
            self.sum_y += y;
            self.sum_xy += x * y;
            self.sum_xx += x * x;
            i += step;
        }
        self.count = self.count.checked_add(i32::try_from(num).ok()?)?;
        Some(())
    }
}

/// The current-frame samples above and left of a block's template, clamped
/// to storage like the DC edges: indices past the stored extent repeat the
/// last stored sample.
struct CurrentTemplate<'a, T> {
    rows: &'a [T],
    stride: usize,
    x: usize,
    up: usize,
    last_col: usize,
    last_row: usize,
}

impl<'a, T: ReconSample> CurrentTemplate<'a, T> {
    /// `None` when an arm the template uses lies outside the frame.
    fn new(
        plane: &'a CurrentFramePlane<T>,
        (x, y): (usize, usize),
        (width, height, num_up, num_left): (usize, usize, usize, usize),
    ) -> Result<Option<Self>> {
        let storage = plane.storage_size();
        let up = usize::from(num_up > 0);
        let (Some(top), Some(stored_cols), Some(stored_rows)) = (
            y.checked_sub(up),
            storage.width().checked_sub(x),
            storage.height().checked_sub(y),
        ) else {
            return Ok(None);
        };
        let (Some(last_col), Some(last_row)) = (
            width.min(stored_cols).checked_sub(1),
            height.min(stored_rows).checked_sub(1),
        ) else {
            return Ok(None);
        };
        if num_left > 0 && x == 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            rows: plane.rows(top, y + last_row + 1)?,
            stride: plane.stride_samples(),
            x,
            up,
            last_col,
            last_row,
        }))
    }

    fn above(&self, i: usize) -> Option<i32> {
        let sample = self.rows.get(self.x + i.min(self.last_col))?;
        Some(i32::from(sample.to_u16()))
    }

    fn left(&self, i: usize) -> Option<i32> {
        let row = self.up + i.min(self.last_row);
        let sample = self
            .rows
            .get((row * self.stride + self.x).checked_sub(1)?)?;
        Some(i32::from(sample.to_u16()))
    }
}

/// The § 7.13.3.25 template sums: `above_ref(i)` is the reference sample
/// above block column `i`, `left_ref(i)` the one left of block row `i`.
fn collect_template_stats<T: ReconSample>(
    current: &CurrentTemplate<'_, T>,
    (width, height, num_up, num_left): (usize, usize, usize, usize),
    mut above_ref: impl FnMut(usize) -> Option<i32>,
    mut left_ref: impl FnMut(usize) -> Option<i32>,
) -> Option<TemplateStats> {
    let mut stats = TemplateStats::default();
    if num_up > 0 {
        stats.add_arm(width, num_up, |i| Some((above_ref(i)?, current.above(i)?)))?;
    }
    if num_left > 0 {
        stats.add_arm(height, num_left, |i| Some((left_ref(i)?, current.left(i)?)))?;
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
    (avail_up, avail_left): (bool, bool),
    mv: Mv,
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

    let (width, height, num_up, num_left) =
        bawp_template_counts(target.width(), target.height(), true, avail_up, avail_left);
    let dims = (width, height, num_up, num_left);
    if morph_dimensions_size(width, height).is_none() {
        return Ok(());
    }
    let Some(current) =
        CurrentTemplate::new(workspace.plane(PlaneId::Y)?, (target.x(), target.y()), dims)?
    else {
        return Ok(());
    };

    let (Ok(ref_x), Ok(ref_y)) = (usize::try_from(ref_x), usize::try_from(ref_y)) else {
        return Ok(());
    };
    let recon = |row: usize, col: usize| {
        workspace
            .reconstructed_sample(PlaneId::Y, col, row)
            .ok()
            .map(|sample| i32::from(sample.to_u16()))
    };
    let Some(stats) = collect_template_stats(
        &current,
        dims,
        |i| recon(ref_y - 1, ref_x + i),
        |i| recon(ref_y + i, ref_x - 1),
    ) else {
        return Ok(());
    };

    let alpha = stats.implicit_alpha();
    let beta = stats.beta(alpha);

    workspace.apply_bawp_rect(PlaneId::Y, target, alpha, beta)?;
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
    let plane_x = placed.luma_x() >> sub_x;
    let plane_y = placed.luma_y() >> sub_y;
    let plane_w = placed.luma_w() >> sub_x;
    let plane_h = placed.luma_h() >> sub_y;
    let last_row = (bawp_reference_luma_rows(placed.luma_y(), placed.luma_h(), mv.row) >> sub_y)
        .saturating_sub(1) as i32;
    let (ref_samples, ref_width, ref_height) =
        reference_plane(reference, plane, last_row, tile_offset)?;
    let dy = to_fullmv(mv.row);
    let dx = to_fullmv(mv.col);
    let Some(ref_y) = fullpel_ref_pos(placed.luma_y(), dy).map(|y| y >> sub_y) else {
        return Ok(luma_alpha);
    };
    let Some(ref_x) = fullpel_ref_pos(placed.luma_x(), dx).map(|x| x >> sub_x) else {
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
        bawp.avail_up,
        bawp.avail_left,
    );
    let dims = (width, height, num_up, num_left);
    if morph_dimensions_size(width, height).is_none() {
        return Ok(luma_alpha);
    }
    let Some(current) = CurrentTemplate::new(workspace.plane(plane)?, (plane_x, plane_y), dims)?
    else {
        return Ok(luma_alpha);
    };
    let (Ok(ref_x), Ok(ref_y)) = (usize::try_from(ref_x), usize::try_from(ref_y)) else {
        return Ok(luma_alpha);
    };
    let above_row = ref_samples.row(ref_y - 1).get(ref_x..).unwrap_or_default();
    let Some(stats) = collect_template_stats(
        &current,
        dims,
        |i| above_row.get(i).map(|sample| i32::from(sample.to_u16())),
        |i| Some(ref_samples.sample(ref_y + i, ref_x - 1)),
    ) else {
        return Ok(luma_alpha);
    };

    let alpha = if plane != PlaneId::Y {
        if stats.count == 0 {
            1 << SHIFT
        } else {
            luma_alpha
        }
    } else if bawp.explicit {
        let mut scale = i16::from(bawp.list_index) + 1;
        if bawp.ref_dist_gt4 {
            scale += 1;
        }
        if !bawp.explicit_scale_positive {
            scale = -scale;
        }
        256 + 16 * scale
    } else {
        stats.implicit_alpha()
    };
    let beta = stats.beta(alpha);

    workspace.apply_bawp_rect(
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

fn fullpel_ref_pos(origin: usize, delta: i32) -> Option<i32> {
    i32::try_from(origin)
        .ok()
        .and_then(|origin| origin.checked_add(delta))
}

fn reference_plane<T: ReconSample>(
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    last_row: i32,
    tile_offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, usize, usize)> {
    let view = reference.plane_view(plane, last_row, tile_offset)?;
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
