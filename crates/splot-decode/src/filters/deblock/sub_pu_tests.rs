// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::tests::{deblock_blocks, prediction};
use super::*;

/// Counts the positions along one line where [`same_record_decision`]
/// filters, asserting it decides as [`edge_decision`] does for a twin record.
fn same_record_decisions_match<const PLANE: usize, const PASS: usize>(
    record: &DeblockBlock,
    (plane_sub_x, plane_sub_y): (usize, usize),
) -> usize {
    let twin = *record;
    let side = |block| EdgeBlock {
        block,
    };
    let strengths = StrengthCache::new(0, 0, BitDepth::Ten);
    let project = |decision: Option<EdgeDecision>| {
        decision.map(|d| {
            (
                d.filter_size,
                d.q_thr,
                d.side,
                d.prev_lossless,
                d.curr_lossless,
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
            allow_df_sub_pu: true,
            tile_edge: false,
        };
        let fast = same_record_decision::<PLANE, PASS>(side(record), x_p, y_p, ctx, &strengths);
        let general =
            edge_decision::<PLANE, PASS>(side(record), side(&twin), x_p, y_p, ctx, &strengths);
        assert_eq!(
            project(fast),
            project(general),
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
