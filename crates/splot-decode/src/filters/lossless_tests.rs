// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use splot_recon::PlaneId;

fn block(r: usize, c: usize, n4w: usize, n4h: usize, lossless: bool) -> DeblockBlock {
    DeblockBlock {
        r: r as u32,
        c: c as u32,
        luma_prediction: crate::filters::deblock::DeblockPredictionUnit::new(r, c, 0),
        chroma_prediction: crate::filters::deblock::DeblockPredictionUnit::new(r, c, 0),
        chroma_base_r: r as u32,
        chroma_base_c: c as u32,
        n4w: n4w as u32,
        n4h: n4h as u32,
        luma_tx: 0,
        chroma_tx: Some(0),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 0,
        skip: false,
        lossless,
    }
}

#[test]
fn grid_marks_luma_and_subsampled_chroma_cells() {
    let luma = [block(1, 2, 2, 2, true)];
    let u = [block(4, 6, 2, 2, true)];
    let v = [block(4, 6, 2, 2, false)];
    let grid = LosslessBlockGrid::from_deblock_blocks(8, 8, &luma, [&u, &v]).unwrap();

    assert!(grid.cdef_luma_lossless(0, 2));
    assert!(!grid.cdef_luma_lossless(0, 0));

    assert!(grid.plane_sample_lossless(PlaneId::U, 12, 8, 1, 1));
    assert!(!grid.plane_sample_lossless(PlaneId::V, 12, 8, 1, 1));
}
