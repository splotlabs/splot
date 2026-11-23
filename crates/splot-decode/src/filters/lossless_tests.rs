// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_recon::PlaneId;

fn block(r: usize, c: usize, n4w: usize, n4h: usize, lossless: bool) -> DeblockBlock {
    DeblockBlock {
        r: r as u16,
        c: c as u16,
        prediction: crate::filters::deblock::DeblockPredictionUnit::new(r, c, 0),
        n4w: n4w as u8,
        n4h: n4h as u8,
        tx: 0,
        sub_pu_size: None,
        qindex: 0,
        skip: false,
        lossless,
    }
}

#[test]
fn grid_marks_luma_and_subsampled_chroma_cells() {
    let luma = [block(1, 2, 2, 2, true)];
    let chroma = [block(4, 6, 2, 2, true)];
    let grid = LosslessBlockGrid::from_deblock_blocks(8, 8, &luma, &chroma).unwrap();

    assert!(grid.cdef_luma_lossless(0, 2));
    assert!(!grid.cdef_luma_lossless(0, 0));

    assert!(grid.plane_sample_lossless(PlaneId::U, 12, 8, 1, 1));
    assert!(grid.plane_sample_lossless(PlaneId::V, 12, 8, 1, 1));
    assert!(!grid.plane_sample_lossless(PlaneId::V, 4, 4, 1, 1));
    assert!(grid.cdef_chroma_lossless(4, 6));
    assert!(!grid.cdef_chroma_lossless(4, 4));

    let cells = grid.into_cells();
    let retained = cells.each_ref().map(Vec::as_ptr);
    let reused =
        LosslessBlockGrid::from_deblock_blocks_reusing((8, 8), &chroma, &luma, cells).unwrap();
    assert!(reused.cdef_luma_lossless(4, 6) && reused.cdef_chroma_lossless(0, 2));
    let reused = reused.into_cells();
    assert_eq!(reused.each_ref().map(Vec::as_ptr), retained);
}
