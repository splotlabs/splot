// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::grid::{DeblockGridStorage, RowOrder};
use super::tests::{
    EMPTY_CHROMA_RECORDS, assert_candidate_mask_superset, deblock_blocks, prediction,
};
use super::*;

#[test]
fn candidate_mask_covers_sub_pu_lines_of_offset_prediction_units() {
    let block =
        |(r, c, n4w, n4h), luma: (usize, usize), chroma: (usize, usize), (w, h)| DeblockBlock {
            r,
            c,
            luma_prediction: prediction(luma.0, luma.1, 3),
            chroma_prediction: prediction(chroma.0, chroma.1, 2),
            chroma_base_r: r,
            chroma_base_c: c,
            n4w,
            n4h,
            luma_tx: 3,
            chroma_tx: Some(2),
            sub_pu_size: Some(DeblockSubPuSize::new(w, h)),
            chroma_transform_only: false,
            qindex: 100,
            skip: false,
            lossless: false,
        };
    let blocks = [
        block((0, 0, 7, 5), (0, 0), (0, 0), (8, 16)),
        block((0, 7, 9, 3), (0, 6), (0, 5), (16, 4)),
        block((3, 7, 9, 5), (1, 5), (2, 6), (4, 8)),
        block((5, 0, 7, 3), (4, 1), (3, 0), (8, 8)),
    ];
    for subsampling in [(1, 1), (1, 0), (0, 0)] {
        let mut storage = DeblockGridStorage::default();
        let mut order = RowOrder::default();
        order.sort(blocks.iter(), 8).unwrap();
        let mut base = MiGridStorage::new(16, subsampling, &mut storage);
        base.fill(&blocks, &order, 8, &(0..8)).unwrap();
        let grid = MiGrid::new(&base, None, &blocks, &EMPTY_CHROMA_RECORDS);
        for (plane, (sub_x, sub_y)) in [(0, (0, 0)), (1, subsampling)] {
            for pass in 0..2 {
                assert_candidate_mask_superset(&grid, 8, 16, plane, pass, sub_x, sub_y);
            }
        }
    }
}

/// Counts the positions along one line where [`same_record_decision`]
/// filters, asserting it decides as [`edge_decision`] does for a twin record.
fn same_record_decisions_match<const PLANE: usize, const PASS: usize>(
    record: &DeblockBlock,
    (plane_sub_x, plane_sub_y): (usize, usize),
) -> usize {
    let twin = *record;
    let side = |block| EdgeBlock {
        block,
        chroma_transform: None,
    };
    let strengths = StrengthCache::new(0, 0, BitDepth::Ten);
    let project = |decision: Option<EdgeDecision>| {
        decision.map(|d| {
            let key = (d.key.curr_qindex, d.key.prev_qindex, d.key.sub_pu_only);
            (
                d.filter_size,
                d.q_thr,
                d.side,
                d.prev_lossless,
                d.curr_lossless,
                key,
            )
        })
    };
    let mut filtered = 0;
    for at in 1..64 {
        let (x_p, y_p) = if PASS == 0 { (at, 3) } else { (3, at) };
        let ctx = EdgeContext {
            row: 0,
            col: 0,
            plane_sub_x,
            plane_sub_y,
            bit_depth: BitDepth::Ten,
            allow_df_sub_pu: true,
            tile_edge: false,
        };
        let fast = same_record_decision::<PLANE, PASS>(side(record), x_p, y_p, ctx, &strengths);
        let general =
            edge_decision::<PLANE, PASS>(side(record), side(&twin), x_p, y_p, ctx, &strengths);
        assert_eq!(
            project(fast),
            project(general.0),
            "plane {PLANE} pass {PASS} at {at}"
        );
        filtered += usize::from(fast.is_some());
    }
    filtered
}

#[test]
fn same_record_decisions_match_the_two_record_derivation() {
    let mut record = deblock_blocks(1, 1)[0];
    record.qindex = 180;
    record.luma_prediction = prediction(1, 1, 3);
    record.chroma_prediction = prediction(2, 0, 2);
    let mut filtered = 0;
    for (sub_pu, luma_tx) in [((8, 16), 3), ((16, 8), 0), ((4, 4), 4), ((16, 16), 11)] {
        record.sub_pu_size = Some(DeblockSubPuSize::new(sub_pu.0, sub_pu.1));
        record.luma_tx = luma_tx;
        filtered += same_record_decisions_match::<0, 0>(&record, (0, 0));
        filtered += same_record_decisions_match::<0, 1>(&record, (0, 0));
        for subsampling in [(1, 1), (1, 0), (0, 0)] {
            filtered += same_record_decisions_match::<1, 0>(&record, subsampling);
            filtered += same_record_decisions_match::<1, 1>(&record, subsampling);
        }
    }
    assert!(filtered > 0);
}
