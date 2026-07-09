// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlacedInterGeometry {
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    pub(super) luma_w: usize,
    pub(super) luma_h: usize,
    pub(super) chroma_luma_x: usize,
    pub(super) chroma_luma_y: usize,
    pub(super) chroma_luma_w: usize,
    pub(super) chroma_luma_h: usize,
    pub(super) has_chroma: bool,
    pub(super) interintra_chroma: bool,
}

pub(super) fn placed_inter_geometry(
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<PlacedInterGeometry> {
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma_w = n4w * 4;
    let luma_h = n4h * 4;
    let (chroma_luma_x, chroma_luma_y, chroma_luma_w, chroma_luma_h) = if frontier.has_chroma {
        let chroma_ref = frontier.chroma_ref_geometry();
        let chroma_n4w = chroma_ref.size().num_4x4_wide().map_err(|_| {
            inter_diag!(
                "inter_chroma_ref_width",
                tile_offset,
                "invalid inter chroma reference width",
                "5.20.4.1"
            )
        })?;
        let chroma_n4h = chroma_ref.size().num_4x4_high().map_err(|_| {
            inter_diag!(
                "inter_chroma_ref_height",
                tile_offset,
                "invalid inter chroma reference height",
                "5.20.4.1"
            )
        })?;
        (
            chroma_ref.col() * 4,
            chroma_ref.row() * 4,
            chroma_n4w * 4,
            chroma_n4h * 4,
        )
    } else {
        (luma_x, luma_y, luma_w, luma_h)
    };
    let mixed_offset_chroma = !frontier.is_luma_part()
        && !frontier.is_chroma_part()
        && frontier.is_mixed_region()
        && frontier.chroma_offset;
    Ok(PlacedInterGeometry {
        luma_x,
        luma_y,
        luma_w,
        luma_h,
        chroma_luma_x,
        chroma_luma_y,
        chroma_luma_w,
        chroma_luma_h,
        has_chroma: frontier.has_chroma,
        interintra_chroma: frontier.has_chroma && !mixed_offset_chroma,
    })
}
