// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.2.8 bilinear directional prediction of small blocks, sample by
//! sample.
//!
//! Feature tracking: `RECON-INTRA-DIRECTIONAL-ANGLE`.

use super::{DirectionalAngleBranch, MiddleDirectionalAngleBranch};
use crate::IntraRectBlockSize;
use crate::intra_dc_math::IntraLaneSample;

/// Blocks of at most this many samples predict bilinear directional samples
/// one by one: their few short lines cost more to set up than to compute.
pub(super) const SMALL_BLOCK_SAMPLES: usize = 32;

/// `Round2(a * (32 - shift) + b * shift, 5)` of the edge samples at `base`
/// and `base + 1`.
#[inline]
fn bilinear_at<P: IntraLaneSample>(edge: &[P], base: usize, shift: u32) -> P {
    let (a, b) = match edge.get(base..base + 2) {
        Some(&[a, b]) => (u32::from(a.to_u16()), u32::from(b.to_u16())),
        _ => (0, 0),
    };
    P::from_lane(((a * (32 - shift) + b * shift + 16) >> 5) as u16)
}

/// [`super::write_prediction`]'s per-sample form for a small block over in-range edges.
pub(super) fn write_one_sided<P: IntraLaneSample>(
    size: IntraRectBlockSize,
    branch: DirectionalAngleBranch,
    edge: &[P],
    output: &mut [P],
    stride: usize,
) {
    let (width, height) = (size.width(), size.height());
    let max_base = width + height - 1;
    let last = edge.get(max_base).copied().unwrap_or_default();
    let (DirectionalAngleBranch::Above { derivative }
    | DirectionalAngleBranch::Left { derivative }) = branch;
    let above = matches!(branch, DirectionalAngleBranch::Above { .. });
    for (row, out) in output.chunks_mut(stride).take(height).enumerate() {
        for (column, slot) in out.iter_mut().take(width).enumerate() {
            let (line, along) = if above { (row, column) } else { (column, row) };
            let idx = (line + 1) * usize::from(derivative);
            let base = (idx >> 6) + along;
            *slot = if base < max_base {
                bilinear_at(edge, base, ((idx >> 1) & 0x1f) as u32)
            } else {
                last
            };
        }
    }
}

/// [`super::write_middle_prediction`]'s per-sample form for a small block whose references
/// `validate_middle_index_bounds` checked.
pub(super) fn write_middle<P: IntraLaneSample>(
    size: IntraRectBlockSize,
    branch: MiddleDirectionalAngleBranch,
    [left, above]: [&[P]; 2],
    output: &mut [P],
    stride: usize,
) {
    let (dx, dy) = (i32::from(branch.dx), i32::from(branch.dy));
    for (row, out) in output.chunks_mut(stride).take(size.height()).enumerate() {
        let row = row as i32;
        for (column, slot) in out.iter_mut().take(size.width()).enumerate() {
            let column = column as i32;
            let above_idx = 64 * column - (row + 1) * dx;
            let (edge, idx) = if above_idx >> 6 >= -1 {
                (above, above_idx)
            } else {
                (left, 64 * row - (column + 1) * dy)
            };
            let base = usize::try_from((idx >> 6) + 1).unwrap_or(usize::MAX - 1);
            *slot = bilinear_at(edge, base, ((idx >> 1) & 0x1f) as u32);
        }
    }
}
