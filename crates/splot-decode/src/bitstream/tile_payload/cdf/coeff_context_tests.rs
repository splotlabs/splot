// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

const PAD: usize = super::super::super::coeff_state::LEVEL_GRID_PAD;

/// Re-lays a flat `txw` x `txh` level grid into the padded-stride layout the
/// production contexts read.
fn padded(flat: &[u8], txw: usize, txh: usize) -> Vec<u8> {
    let stride = txw + PAD;
    let mut out = vec![0u8; stride * (txh + PAD)];
    for row in 0..txh {
        for col in 0..txw {
            out[row * stride + col] = flat[row * txw + col];
        }
    }
    out
}

#[test]
fn coeff_base_eob_partitions_the_scan_position() {
    let (bwl, height) = (5u32, 32usize);
    assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0, "c == 0");
    assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
    assert_eq!(
        coeff_base_eob_ctx(128, bwl, height),
        1,
        "boundary numCoeffs/8"
    );
    assert_eq!(coeff_base_eob_ctx(129, bwl, height), 2);
    assert_eq!(
        coeff_base_eob_ctx(256, bwl, height),
        2,
        "boundary numCoeffs/4"
    );
    assert_eq!(coeff_base_eob_ctx(257, bwl, height), 3);
    assert_eq!(coeff_base_eob_ctx(1023, bwl, height), 3, "last position");
}

#[test]
fn coeff_base_eob_smallest_block() {
    let (bwl, height) = (2u32, 4usize);
    assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0);
    assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
    assert_eq!(coeff_base_eob_ctx(2, bwl, height), 1, "boundary 16/8");
    assert_eq!(coeff_base_eob_ctx(3, bwl, height), 2);
    assert_eq!(coeff_base_eob_ctx(4, bwl, height), 2, "boundary 16/4");
    assert_eq!(coeff_base_eob_ctx(5, bwl, height), 3);
}

#[test]
fn coeff_base_eob_is_total_for_out_of_range_shift() {
    assert_eq!(coeff_base_eob_ctx(0, u32::MAX, 32), 0);
    assert_eq!(coeff_base_eob_ctx(1, u32::MAX, 32), 1);
}

#[test]
fn coeff_base_bob_partitions_the_begin_position() {
    let seg_eob = 64usize;
    assert_eq!(coeff_base_bob_ctx(0, seg_eob), 0);
    assert_eq!(coeff_base_bob_ctx(8, seg_eob), 0, "boundary segEob>>3");
    assert_eq!(coeff_base_bob_ctx(9, seg_eob), 1);
    assert_eq!(coeff_base_bob_ctx(16, seg_eob), 1, "boundary segEob>>2");
    assert_eq!(coeff_base_bob_ctx(17, seg_eob), 2);
    assert_eq!(coeff_base_bob_ctx(64, seg_eob), 2, "bob == segEob");
}

#[test]
fn coeff_base_bob_zero_segment_eob() {
    assert_eq!(coeff_base_bob_ctx(0, 0), 0);
    assert_eq!(coeff_base_bob_ctx(1, 0), 2);
}

fn br(pos: usize, plane: usize, is_lf: bool, tx_class: usize) -> CoeffBrContext {
    CoeffBrContext {
        row: pos >> 2,
        col: pos & 3,
        stride: 4 + PAD,
        plane,
        is_lf,
        tx_class,
    }
}

#[test]
fn coeff_br_contexts_cover_neighbour_and_plane_rules() {
    let mut level = [0u8; 16];
    level[1] = 7;
    level[4] = 2;
    level[5] = 10;
    assert_eq!(br(0, 0, false, 0).ctx(&padded(&level, 4, 4)), 6);

    level.fill(0);
    level[6] = 5;
    level[9] = 5;
    level[10] = 5;
    assert_eq!(br(5, 0, false, 0).ctx(&padded(&level, 4, 4)), 6);

    let zero = padded(&[0u8; 16], 4, 4);
    assert_eq!(br(0, 0, false, 2).ctx(&zero), 7);
    assert_eq!(br(5, 0, true, 0).ctx(&zero), 7);
    assert_eq!(br(5, 0, false, 0).ctx(&zero), 0);

    level.fill(0);
    level[1] = 5;
    level[4] = 5;
    level[5] = 5;
    assert_eq!(br(0, 1, false, 0).ctx(&padded(&level, 4, 4)), 3);

    level.fill(0);
    level[6] = 1;
    level[9] = 1;
    level[13] = 4;
    assert_eq!(br(5, 1, false, 2).ctx(&padded(&level, 4, 4)), 1);
}

#[test]
fn coeff_br_is_total_for_out_of_bounds_and_short_slices() {
    let full = padded(&[9u8; 16], 4, 4);
    assert_eq!(br(15, 0, false, 0).ctx(&full), 0);
    let short = [0u8, 9, 0, 0];
    assert_eq!(br(0, 0, false, 0).ctx(&short), 3);
}

#[test]
fn coeff_br_is_total_for_pathological_geometry() {
    let level = [0u8; 16];
    let _ = CoeffBrContext {
        row: usize::MAX,
        col: usize::MAX,
        stride: usize::MAX,
        plane: 0,
        is_lf: false,
        tx_class: 9,
    }
    .ctx(&level);
    let _ = CoeffBrContext {
        row: usize::MAX,
        col: usize::MAX,
        stride: usize::MAX,
        plane: 1,
        is_lf: true,
        tx_class: 2,
    }
    .ctx(&level);
}

#[test]
fn coeff_base_idtx_sums_clamped_left_and_above() {
    let mut lvl = [0u8; 16];
    lvl[4] = 1; // (1,0) = left of (1,1)
    lvl[1] = 9; // (0,1) = above of (1,1)
    assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 1, 4), 4);
}

#[test]
fn coeff_idtx_contexts_cover_missing_neighbours_and_clamps() {
    let lvl = [7u8; 16];
    assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 0, 4), 0);
    assert_eq!(coeff_base_idtx_ctx(&lvl, 0, 1, 4), 3);
    assert_eq!(coeff_base_idtx_ctx(&lvl, 1, 0, 4), 3);

    let lvl = [9u8; 16];
    assert_eq!(coeff_br_idtx_ctx(&lvl, 1, 1, 4), 6);
    assert_eq!(coeff_br_idtx_ctx(&lvl, 0, 1, 4), 5);
}

#[test]
fn coeff_idtx_is_total_for_short_slice_and_pathological_geometry() {
    let short = [3u8, 3];
    assert_eq!(coeff_base_idtx_ctx(&short, 1, 1, 4), 3);
    let lvl = [0u8; 4];
    let _ = coeff_base_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
    let _ = coeff_br_idtx_ctx(&lvl, usize::MAX, usize::MAX, usize::MAX);
}

fn cb8(
    pos: usize,
    plane: usize,
    is_lf: bool,
    is_hidden: bool,
    c: usize,
    tx_class: usize,
) -> CoeffBaseContext {
    CoeffBaseContext {
        row: pos >> 3,
        col: pos & 7,
        stride: 8 + PAD,
        plane,
        is_lf,
        is_hidden,
        c,
        tx_class,
    }
}

#[test]
fn coeff_base_luma_hf_2d_position_buckets() {
    let z = padded(&[0u8; 64], 8, 8);
    assert_eq!(
        cb8(0, 0, false, false, 0, 0).select(&z),
        CoeffBaseSelection::Hf { ctx: 0 }
    ); // (0,0) sum 0
    assert_eq!(
        cb8(27, 0, false, false, 5, 0).select(&z),
        CoeffBaseSelection::Hf { ctx: 5 }
    ); // (3,3) sum 6
    assert_eq!(
        cb8(36, 0, false, false, 5, 0).select(&z),
        CoeffBaseSelection::Hf { ctx: 10 }
    ); // (4,4) sum 8
}

#[test]
fn coeff_base_luma_hf_non_2d_adds_fifteen() {
    let z = padded(&[0u8; 64], 8, 8);
    assert_eq!(
        cb8(0, 0, false, false, 1, 2).select(&z),
        CoeffBaseSelection::Hf { ctx: 15 }
    );
}

#[test]
fn coeff_base_luma_lf_covers_2d_and_directional_branches() {
    let z = padded(&[0u8; 64], 8, 8);
    let cases = [
        ((0, 0, 0), 0),
        ((1, 1, 0), 9),
        ((9, 1, 0), 16),
        ((0, 1, 1), 21),
        ((1, 1, 1), 28),
        ((0, 1, 2), 21),
        ((9, 1, 2), 28),
    ];
    for ((pos, c, tx_class), ctx) in cases {
        assert_eq!(
            cb8(pos, 0, true, false, c, tx_class).select(&z),
            CoeffBaseSelection::Lf { ctx }
        );
    }
}

#[test]
fn coeff_base_chroma_uv_branches() {
    let z = padded(&[0u8; 64], 8, 8);
    assert_eq!(
        cb8(0, 1, false, false, 1, 0).select(&z),
        CoeffBaseSelection::Uv { ctx: 0 }
    );
    assert_eq!(
        cb8(0, 2, false, false, 1, 0).select(&z),
        CoeffBaseSelection::Uv { ctx: 4 }
    );
    assert_eq!(
        cb8(0, 1, false, false, 1, 2).select(&z),
        CoeffBaseSelection::Uv { ctx: 8 }
    );
    assert_eq!(
        cb8(0, 1, true, false, 1, 0).select(&z),
        CoeffBaseSelection::LfUv { ctx: 0 }
    );
}

#[test]
fn coeff_base_sums_clamped_neighbours_into_hf() {
    let mut lvl = [0u8; 64];
    for f in [1, 8, 9, 2, 16] {
        lvl[f] = 9;
    }
    assert_eq!(
        cb8(0, 0, false, false, 0, 0).select(&padded(&lvl, 8, 8)),
        CoeffBaseSelection::Hf { ctx: 4 }
    );
}

#[test]
fn coeff_base_low_frequency_maglimit_raises_to_five() {
    let mut lvl = [0u8; 64];
    lvl[1] = 9;
    assert_eq!(
        cb8(0, 0, true, false, 0, 0).select(&padded(&lvl, 8, 8)),
        CoeffBaseSelection::Lf { ctx: 3 }
    );
}

#[test]
fn coeff_base_parity_hidden_overrides_and_caps_maglimit() {
    let mut lvl = [0u8; 64];
    lvl[1] = 9;
    assert_eq!(
        cb8(0, 0, true, true, 0, 0).select(&padded(&lvl, 8, 8)),
        CoeffBaseSelection::Ph { ctx: 2 }
    );
}

#[test]
fn coeff_base_chroma_2d_reads_three_neighbours_not_five() {
    let mut lvl = [0u8; 64];
    lvl[9] = 9;
    lvl[2] = 9;
    assert_eq!(
        cb8(0, 1, false, false, 1, 0).select(&padded(&lvl, 8, 8)),
        CoeffBaseSelection::Uv { ctx: 2 }
    );
}

#[test]
fn coeff_base_is_total_for_short_slice_and_pathological_geometry() {
    let short = [0u8, 9];
    assert_eq!(
        cb8(0, 0, false, false, 0, 0).select(&short),
        CoeffBaseSelection::Hf { ctx: 2 }
    );
    let z = [0u8; 4];
    let _ = CoeffBaseContext {
        row: usize::MAX,
        col: usize::MAX,
        stride: usize::MAX,
        plane: 0,
        is_lf: true,
        is_hidden: false,
        c: 0,
        tx_class: 9,
    }
    .select(&z);
}

#[test]
fn dc_sign_ctx_nets_above_and_left_votes() {
    let above = [2u8, 2];
    let left = [1u8, 1];
    assert_eq!(dc_sign_ctx(&above, &left, 0, 0, 2, 2), 0);
    let above_neg = [1u8, 0];
    let z2 = [0u8, 0];
    assert_eq!(dc_sign_ctx(&above_neg, &z2, 0, 0, 2, 2), 1);
    let pos = [2u8, 2];
    assert_eq!(dc_sign_ctx(&z2, &pos, 0, 0, 2, 2), 2);
    let zeros = [0u8, 0];
    assert_eq!(dc_sign_ctx(&zeros, &zeros, 0, 0, 2, 2), 0);
}

#[test]
fn dc_sign_ctx_honours_the_position_offset_and_max_bounds() {
    let above = [1u8, 2, 2]; // index 0 = -1 (skipped), 1,2 = +1 each
    let z = [0u8; 4];
    assert_eq!(dc_sign_ctx(&above, &z, 1, 0, 2, 0), 2); // +1+1 = +2 -> ctx 2
    let short = [2u8]; // only index 0 in range
    assert_eq!(dc_sign_ctx(&short, &z, 0, 0, 4, 0), 2); // only above[0]=+1 -> ctx 2
}

#[test]
fn dc_sign_ctx_is_total_for_pathological_geometry() {
    let a = [2u8; 4];
    let l = [1u8; 4];
    let _ = dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, usize::MAX, usize::MAX);
    assert_eq!(dc_sign_ctx(&a, &l, usize::MAX, usize::MAX, 4, 4), 0); // all out of range -> 0
}

#[test]
fn idtx_sign_ctx_maps_signc_to_base_context() {
    let zl = [0u8; 16];
    let p3 = [1i8, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&p3, &zl, 1, 1, 4), 5);
    let n3 = [-1i8, -1, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&n3, &zl, 1, 1, 4), 6);
    let p1 = [0i8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&p1, &zl, 1, 1, 4), 1);
    let n1 = [0i8, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&n1, &zl, 1, 1, 4), 2);
    assert_eq!(idtx_sign_ctx(&[0i8; 16], &zl, 1, 1, 4), 0);
}

#[test]
fn idtx_sign_ctx_level_threshold_raises_nonzero_context() {
    let p1 = [0i8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let hi = [0u8, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // 4 > 3
    assert_eq!(idtx_sign_ctx(&p1, &hi, 1, 1, 4), 3);
    let eq = [0u8, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&p1, &eq, 1, 1, 4), 1);
    assert_eq!(idtx_sign_ctx(&[0i8; 16], &hi, 1, 1, 4), 0);
}

#[test]
fn idtx_sign_ctx_skips_missing_edge_neighbours() {
    let zl = [0u8; 16];
    let q = [1i8; 16];
    assert_eq!(idtx_sign_ctx(&q, &zl, 0, 0, 4), 0);
    let only_left = [1i8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&only_left, &zl, 0, 1, 4), 1);
    let only_above = [1i8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(idtx_sign_ctx(&only_above, &zl, 1, 0, 4), 1);
}

#[test]
fn idtx_sign_ctx_is_total_for_short_slices_and_pathological_geometry() {
    let q = [1i8, 1];
    let l = [9u8];
    let _ = idtx_sign_ctx(&q, &l, 1, 1, 4);
    let _ = idtx_sign_ctx(&q, &l, usize::MAX, usize::MAX, usize::MAX);
}
