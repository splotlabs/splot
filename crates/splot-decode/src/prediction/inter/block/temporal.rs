// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::ReconSample;

use super::super::find_mv_stack::{TemporalMotionBlock, TemporalMotionField};
use super::super::{InterReferenceState, Mv};

pub(super) fn block_ref_within_temporal_distance<T: ReconSample>(
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    current_order_hint: u32,
    ref_frame0: i8,
) -> bool {
    let Some(hint) = usize::try_from(ref_frame0)
        .ok()
        .and_then(|list_ref| ref_frame_idx.get(list_ref))
        .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
    else {
        return false;
    };
    let dist = super::super::get_relative_dist(
        current_order_hint as i32,
        i32::try_from(*hint).unwrap_or(i32::MAX),
    );
    dist.abs() <= 2
}

fn temporal_ref_order_hint<T: ReconSample>(
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
) -> Option<u32> {
    usize::try_from(ref_frame)
        .ok()
        .and_then(|list_ref| ref_frame_idx.get(list_ref))
        .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
        .copied()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_temporal_motion_block<T: ReconSample>(
    motion_field: &mut TemporalMotionField,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    ref_frame0: i8,
    ref_frame1: Option<i8>,
    mv0: Mv,
    mv1: Mv,
    warp_params: [Option<[i64; 6]>; 2],
) {
    motion_field.record_block(TemporalMotionBlock {
        mi_row,
        mi_col,
        n4w,
        n4h,
        mi_rows,
        mi_cols,
        current_order_hint,
        ref_order_hints: [
            temporal_ref_order_hint(reference, ref_frame_idx, ref_frame0),
            ref_frame1.and_then(|ref_frame1| {
                temporal_ref_order_hint(reference, ref_frame_idx, ref_frame1)
            }),
        ],
        mvs: [mv0, mv1],
        warp_params,
    });
}
