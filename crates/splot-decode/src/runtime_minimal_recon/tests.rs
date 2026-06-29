// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the minimal traced reconstruction handoff ([`super`]).

#![allow(clippy::unwrap_used)]

use splot_recon::DecodedFrameHashInput;

use super::*;

const EXPECTED_DIGEST: &str = "dd244844938e78b226240de27e9c0acd39fc7ec2c1631319d13250fbe5f08496";

fn reconstruct() -> DecodedFrame<u8> {
    reconstruct_minimal_traced_frame(
        MinimalRuntimeReconstructionTrace::LumaDcNoResidual8Bit420_64x64,
    )
    .unwrap()
}

#[test]
fn traced_luma_dc_chroma_h_pred_reconstruction_predicts_visible_samples() {
    let frame = reconstruct();

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(32, 32).unwrap()
    );
    assert_eq!(
        frame.v().unwrap().visible_size(),
        PlaneSize::new(32, 32).unwrap()
    );
    assert!(frame.y().samples().iter().all(|sample| *sample == 128));
    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
    );
    assert!(
        frame
            .v()
            .unwrap()
            .samples()
            .iter()
            .all(|sample| *sample == TOP_LEFT_CHROMA_H_PRED_LEFT_FALLBACK_SAMPLE)
    );
    assert!(!frame.y().samples().contains(&0));
    assert!(!frame.u().unwrap().samples().contains(&0));
    assert!(!frame.v().unwrap().samples().contains(&0));
}

#[test]
fn traced_luma_dc_chroma_h_pred_reconstruction_hash_matches_minimal_contract() {
    let frame = reconstruct();
    let hash = DecodedFrameHashInput::new(&frame).compute_hash();

    assert_eq!(hash.to_hex(), EXPECTED_DIGEST);
}

/// An `all_zero` (`txb_skip`) luma block: reconstruction writes the bare
/// §7.13.2 prediction (zero residual), the only kind these cardinal
/// rect/transpose guards exercise.
fn all_zero_luma_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        plane_tx_type: 0,
    }
}

/// Lays an `above_row` pattern (length `width`) so that workspace row `edge_y` is
/// that pattern over `x[0, width)`. Writes a `width x 4` block at `(0, edge_y-3)`
/// whose every row carries the pattern (so its bottom row `edge_y` does too).
fn lay_above_row(ws: &mut CurrentFrameWorkspace<u8>, edge_y: usize, log2_w: u8, pattern: &[u8]) {
    let width = 1usize << log2_w;
    let samples: Vec<u8> = (0..4).flat_map(|_| pattern.iter().copied()).collect();
    let size = IntraRectBlockSize::new(log2_w, 2).unwrap();
    ws.write_rect_block(PlaneId::Y, 0, edge_y - 3, size, &samples)
        .unwrap();
    debug_assert_eq!(width, pattern.len());
}

/// Lays a `left_col` pattern (length `height`) so that workspace column `edge_x`
/// is that pattern over `y[0, height)`. Writes a `4 x height` block at
/// `(edge_x-3, 0)` whose every column carries the pattern (so its rightmost
/// column `edge_x` does too).
fn lay_left_col(ws: &mut CurrentFrameWorkspace<u8>, edge_x: usize, log2_h: u8, pattern: &[u8]) {
    let height = 1usize << log2_h;
    let mut samples = vec![0u8; 4 * height];
    for (row, &v) in pattern.iter().enumerate() {
        for col in 0..4 {
            samples[row * 4 + col] = v;
        }
    }
    let size = IntraRectBlockSize::new(2, log2_h).unwrap();
    ws.write_rect_block(PlaneId::Y, edge_x - 3, 0, size, &samples)
        .unwrap();
}

/// §7.13.2 ZONE-1 MULTI-REFERENCE-LINE GUARD — a D45 (`shift == 0`, the IDIF
/// reduces to the copy `AboveRow[base]`) 4x4 zone-1 leaf at interior `(8, 8)` with
/// `MrlIndex == 2`. The §7.13.2.1 above row is read from `CurrFrame[y - 1 -
/// aboveMrlIndex] == CurrFrame[5]` (`aboveMrlIndex == MrlIndex == 2`, not a
/// superblock-row boundary), NOT the immediate `CurrFrame[y - 1] == CurrFrame[7]`.
/// Row 5 carries an ASCENDING pattern and rows 6/7 a DISTINCT constant (`200`), so a
/// read of the adjacent line would copy `200` while the offset-line read copies the
/// ascending values — making the reference-line offset observable. With D45 the
/// projection `base = (i + 1 + MrlIndex) + j` so `pred[i][j] == AboveRow[i + 3 + j]`
/// at the offset row.
#[test]
fn zone1_d45_mrl_index_2_reads_the_offset_above_reference_line() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();
    let mut block = vec![200u8; 32 * 4];
    for c in 0..32 {
        block[32 + c] = 10 + 4 * c as u8; // row index 1 within the block == row 5
    }
    ws.write_rect_block(
        PlaneId::Y,
        0,
        4,
        IntraRectBlockSize::new(5, 2).unwrap(),
        &block,
    )
    .unwrap();
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 8, 5).unwrap(), 42);
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 8, 7).unwrap(), 200);

    reconstruct_general_intra_one_sided_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        45,
        PlaneId::Y,
        8,
        8,
        2,
        2,
        0,
        2, // num4_above_right: cover the maxBase = 11 above-right reads (x up to 19)
        OneSidedAboveMrl {
            mrl_index: 2,
            above_mrl_index: 2,
        },
        false,
        BitDepth::Eight,
        OneSidedEdgeFilter::default(),
    )
    .unwrap();

    let got: Vec<u8> = (0..4)
        .flat_map(|r| (0..4).map(move |c| (r, c)))
        .map(|(r, c)| ws.reconstructed_sample(PlaneId::Y, 8 + c, 8 + r).unwrap())
        .collect();
    let expected: Vec<u8> = (0..4)
        .flat_map(|i| (0..4).map(move |j| 54 + 4 * (i + j) as u8))
        .collect();
    assert_eq!(got, expected);
    assert!(!got.contains(&200));
}

/// §7.13.2.8 ZONE-2 TWO-SIDED GUARD — a non-canonical `pAngle == 132` middle leaf
/// over an 8x8 block at interior (8, 8) with a REAL, NON-FLAT above row + left
/// column + DISTINCT diagonal corner, the §7.13.2.7 edge filter a NO-OP
/// (`TwoSidedMiddleEdgeFilters` of defaults). The z2 IDIF reads the above edge
/// (`base_x >= -1`) and falls to the left edge otherwise; the expected samples are
/// the VERBATIM AVM `av2_highbd_dr_prediction_z2_idif_c` output (`dx =
/// dr_intra_derivative[180 - 132] == 56`, `dy = dr_intra_derivative[132 - 90] ==
/// 73`). The asymmetric above/left/corner make a swapped above↔left branch, a wrong
/// `(dx, dy)`, or a corner mix-up observable — a flat block would MASK them.
#[test]
fn zone2_two_sided_p132_interior_leaf_matches_avm_z2_idif() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        4,
        4,
        IntraRectBlockSize::new(2, 2).unwrap(),
        &[190u8; 16],
    )
    .unwrap();
    let above: [u8; 8] = [100, 104, 108, 112, 116, 120, 124, 128];
    let above_block: Vec<u8> = (0..4).flat_map(|_| above.iter().copied()).collect();
    ws.write_rect_block(
        PlaneId::Y,
        8,
        4,
        IntraRectBlockSize::new(3, 2).unwrap(),
        &above_block,
    )
    .unwrap();
    let left: [u8; 8] = [60, 64, 68, 72, 76, 80, 84, 88];
    let mut col = vec![0u8; 4 * 8];
    for (r, &v) in left.iter().enumerate() {
        for c in 0..4 {
            col[r * 4 + c] = v;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        4,
        8,
        IntraRectBlockSize::new(2, 3).unwrap(),
        &col,
    )
    .unwrap();

    reconstruct_general_intra_two_sided_middle_luma_block_into(
        &mut ws,
        &all_zero_luma_block(),
        132,
        8,
        8,
        3,
        3,
        0,
        false,
        BitDepth::Eight,
        TwoSidedMiddleEdgeFilters {
            above: OneSidedEdgeFilter::default(),
            left: OneSidedEdgeFilter::default(),
        },
    )
    .unwrap();

    let got: Vec<u8> = (0..8)
        .flat_map(|r| (0..8).map(move |c| (r, c)))
        .map(|(r, c)| ws.reconstructed_sample(PlaneId::Y, 8 + c, 8 + r).unwrap())
        .collect();
    assert_eq!(
        got,
        [
            181, 96, 104, 108, 112, 116, 120, 125, 77, 169, 94, 105, 109, 113, 117, 121, 58, 93,
            157, 93, 106, 110, 114, 118, 67, 55, 115, 145, 93, 106, 110, 114, 71, 67, 51, 134, 132,
            95, 107, 111, 75, 71, 66, 49, 156, 120, 97, 107, 79, 75, 70, 66, 50, 173, 109, 101, 83,
            79, 74, 70, 65, 53, 190, 100
        ]
    );
}

/// §7.13.2.1 ZONE-3 CORNER GUARD — a D203 (`pAngle == 203`) zone-3 8x8 leaf at an
/// INTERIOR position (8, 8) with `have_above == true`. The §7.13.2.1 corner
/// `LeftCol[-1]` must be the DIAGONAL above-left `CurrFrame[y - 1][x - 1] =
/// (7, 7)`, NOT the left-column top `CurrFrame[y][x - 1] = (7, 8)`. The DISTINCT
/// corner (`200`) vs the left column (`50..120`) makes the choice observable: the
/// expected top row is the VERBATIM AVM `av2_highbd_dr_prediction_z3_idif_c` output
/// reading the diagonal corner (`dy = dr_intra_derivative[270 - 203] == 24`, the
/// 4-tap IDIF, no edge filter). Reading the column top instead would give `row0[0]
/// == 53` not `39`. This pins the zone-3 corner fix (the prior code read `(x - 1,
/// y)`, correct only for the frame-top `have_above == false` leaf).
#[test]
fn zone3_d203_interior_leaf_reads_diagonal_above_left_corner() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        4,
        4,
        IntraRectBlockSize::new(2, 2).unwrap(),
        &[200u8; 16],
    )
    .unwrap();
    let left: [u8; 8] = [50, 60, 70, 80, 90, 100, 110, 120];
    let mut col = vec![0u8; 4 * 8];
    for (r, &v) in left.iter().enumerate() {
        for c in 0..4 {
            col[r * 4 + c] = v;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        4,
        8,
        IntraRectBlockSize::new(2, 3).unwrap(),
        &col,
    )
    .unwrap();
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 7, 7).unwrap(), 200);
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 7, 8).unwrap(), 50);

    reconstruct_general_intra_one_sided_left_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        203,
        PlaneId::Y,
        8,
        8,
        3,
        3,
        0,
        0,
        true,
        0,
        false,
        BitDepth::Eight,
        OneSidedEdgeFilter::default(),
    )
    .unwrap();

    let row0: Vec<u8> = (0..8)
        .map(|c| ws.reconstructed_sample(PlaneId::Y, 8 + c, 8).unwrap())
        .collect();
    assert_eq!(row0, [39, 48, 61, 65, 69, 72, 76, 80]);
}

/// §7.13.2 ZONE-3 MULTI-REFERENCE-LINE DIAGNOSTIC — a D203 (`dy ==
/// dr_intra_derivative[270 - 203] == 24`) 8x8 zone-3 leaf at interior `(16, 16)`
/// with `MrlIndex == 1`. The §7.13.2.1 left column is read from the offset column
/// `CurrFrame[..][x - 1 - MrlIndex] == 14`, the corner from `CurrFrame[y - 1][14]`.
/// The expected samples are computed INLINE from the VERBATIM AVM
/// `av2_highbd_dr_prediction_z3_idif_c` over the same prepared edge, where `y_c ==
/// dy * (1 + MrlIndex + c)`, `base == (y_c >> 6) + r`, `shift == (y_c >> 1) & 0x1F`,
/// the 4-tap `Dr_Interp_Filter`, and the clamp `maxBase == w + h - 1 +
/// (MrlIndex << 1) == 17`. This pins the zone-3 MRL geometry and edge independently
/// of any partial-frame reconstruction; a primitive that read the immediate column
/// (15), the wrong base shift, or the wrong maxBase would diverge from the reference.
/// `FILTER` is the §7.13.2.8 `Dr_Interp_Filter` table.
#[test]
fn zone3_d203_mrl_index_1_matches_inline_avm_z3_idif_reference() {
    const FILTER: [[i32; 4]; 32] = [
        [0, 128, 0, 0],
        [-2, 127, 4, -1],
        [-3, 125, 8, -2],
        [-5, 123, 13, -3],
        [-6, 121, 17, -4],
        [-7, 118, 22, -5],
        [-9, 116, 27, -6],
        [-9, 112, 32, -7],
        [-10, 109, 37, -8],
        [-11, 106, 41, -8],
        [-11, 102, 46, -9],
        [-12, 98, 52, -10],
        [-12, 94, 56, -10],
        [-12, 90, 61, -11],
        [-12, 85, 66, -11],
        [-12, 81, 71, -12],
        [-12, 76, 76, -12],
        [-12, 71, 81, -12],
        [-11, 66, 85, -12],
        [-11, 61, 90, -12],
        [-10, 56, 94, -12],
        [-10, 52, 98, -12],
        [-9, 46, 102, -11],
        [-8, 41, 106, -11],
        [-8, 37, 109, -10],
        [-7, 32, 112, -9],
        [-6, 27, 116, -9],
        [-5, 22, 118, -7],
        [-4, 17, 121, -6],
        [-3, 13, 123, -5],
        [-2, 8, 125, -3],
        [-1, 4, 127, -2],
    ];
    const DY: i64 = 24; // dr_intra_derivative[270 - 203]
    const MRL: i64 = 1;
    const W: usize = 8;
    const H: usize = 8;
    let corner: i64 = 200;
    let left_col: Vec<i64> = (0..32).map(|i| 40 + 3 * i as i64).collect(); // ascending col 14
    let max_base = W + H - 1 + (MRL as usize) * 2; // logical edge: slot 0 = -2, slot 1 = corner
    let mut edge = vec![0i64; max_base + 5];
    edge[1] = corner;
    for i in 0..=max_base {
        edge[i + 2] = *left_col.get(i).unwrap_or(left_col.last().unwrap());
    }
    edge[0] = edge[1];
    edge[max_base + 3] = edge[max_base + 2];
    edge[max_base + 4] = edge[max_base + 2];
    let edge_at = |logical: i64| edge[(logical + 2) as usize];

    let reference: Vec<u8> = (0..H)
        .flat_map(|r| (0..W).map(move |c| (r, c)))
        .map(|(r, c)| {
            let y_c = DY * (1 + MRL + c as i64);
            let base = (y_c >> 6) + r as i64;
            let shift = ((y_c >> 1) & 0x1F) as usize;
            if base <= max_base as i64 {
                let taps = FILTER[shift];
                let mut sum = 0i64;
                for (t, &tap) in taps.iter().enumerate() {
                    sum += i64::from(tap) * edge_at(base + t as i64 - 1);
                }
                (((sum + 64) >> 7).clamp(0, 255)) as u8
            } else {
                edge_at(max_base as i64) as u8
            }
        })
        .collect();

    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        12,
        12,
        IntraRectBlockSize::new(2, 2).unwrap(),
        &[200u8; 16],
    )
    .unwrap();
    let mut col_block = vec![0u8; 4 * 32];
    for (r, &v) in left_col.iter().enumerate() {
        for c in 0..4 {
            col_block[r * 4 + c] = v as u8;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        12,
        16,
        IntraRectBlockSize::new(2, 5).unwrap(),
        &col_block,
    )
    .unwrap();
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 14, 15).unwrap(), 200);
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 14, 16).unwrap(), 40);

    reconstruct_general_intra_one_sided_left_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        203,
        PlaneId::Y,
        16,
        16,
        3,
        3,
        0,
        2, // num4_below_left: cover the maxBase = 17 below-left reads (rows up to 39)
        true,
        1, // mrl_index
        false,
        BitDepth::Eight,
        OneSidedEdgeFilter::default(),
    )
    .unwrap();

    let got: Vec<u8> = (0..H)
        .flat_map(|r| (0..W).map(move |c| (r, c)))
        .map(|(r, c)| ws.reconstructed_sample(PlaneId::Y, 16 + c, 16 + r).unwrap())
        .collect();
    assert_eq!(got, reference);
}

/// STRIDE/TRANSPOSE GUARD — V_PRED over a NON-SQUARE 64x32 (`W == 64`,
/// `H == 32`) block with a REAL, NON-FLAT above row. §7.13.2.8 V_PRED copies the
/// 64-wide above row into every one of the 32 rows; a width/height swap or a
/// `stride == height`-instead-of-`width` bug would corrupt the layout and fail.
/// The asymmetric edge is the key: a flat block (the ac0ej3 all-68 oracle) would
/// MASK a transpose.
#[test]
fn rect_cardinal_vertical_64x32_copies_wide_above_row_per_row() {
    let mut ws = new_general_intra_workspace::<u8>(64, 128, BitDepth::Eight).unwrap();
    let above_row: Vec<u8> = (0..64).map(|x| 100 + x as u8).collect();
    lay_above_row(&mut ws, 63, 6, &above_row);

    reconstruct_general_intra_cardinal_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        IntraCardinalDirection::Vertical,
        PlaneId::Y,
        0,
        64,
        6, // log2_width = 6 -> 64
        5, // log2_height = 5 -> 32
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for row in 0..32 {
        for col in 0..64 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, col, 64 + row).unwrap(),
                100 + col as u8,
                "V_PRED 64x32 sample ({col},{}) must copy above_row[{col}]",
                64 + row,
            );
        }
    }
}

/// STRIDE/TRANSPOSE GUARD — H_PRED over a NON-SQUARE 32x64 (`W == 32`,
/// `H == 64`) block with a REAL, NON-FLAT left column. §7.13.2.8 H_PRED fills
/// each of the 64 rows with one of the 64 left samples; a width/height swap would
/// read past the 64-tall left column or mis-stride and fail.
#[test]
fn rect_cardinal_horizontal_32x64_fills_each_row_from_tall_left_column() {
    let mut ws = new_general_intra_workspace::<u8>(128, 64, BitDepth::Eight).unwrap();
    let left_col: Vec<u8> = (0..64).map(|y| 50 + y as u8).collect();
    lay_left_col(&mut ws, 63, 6, &left_col);

    reconstruct_general_intra_cardinal_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        IntraCardinalDirection::Horizontal,
        PlaneId::Y,
        64,
        0,
        5, // log2_width = 5 -> 32
        6, // log2_height = 6 -> 64
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for row in 0..64 {
        for col in 0..32 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, 64 + col, row).unwrap(),
                50 + row as u8,
                "H_PRED 32x64 sample ({},{row}) must fill row from left_col[{row}]",
                64 + col,
            );
        }
    }
}

/// §7.13.2.1 NO-ABOVE FALLBACK GUARD — the ac0ej3 MI(64,0) case: a NON-SQUARE
/// 64x32 V_PRED block at the frame TOP (`y == 0`, `haveAbove == 0`) with a
/// NON-FLAT reconstructed left column. §7.13.2.1 synthesizes
/// `AboveRow[i] = CurrFrame[plane][y][x-1]` — the block's top-left left neighbour
/// (`left[0]`), repeated across the whole synthesized above row — so the V_PRED
/// copy is a FLAT block equal to `left[0]`, NOT `left[i]`. A non-flat left column
/// proves the fallback reads ONLY `left[0]` (a bug reading `left[i]` row-wise
/// would produce a vertical gradient and fail).
#[test]
fn rect_cardinal_vertical_64x32_no_above_fallback_is_flat_left_corner() {
    let mut ws = new_general_intra_workspace::<u8>(128, 64, BitDepth::Eight).unwrap();
    let left_col: Vec<u8> = (0..32).map(|y| 70 + y as u8).collect();
    lay_left_col(&mut ws, 63, 5, &left_col);

    reconstruct_general_intra_cardinal_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        IntraCardinalDirection::Vertical,
        PlaneId::Y,
        64,
        0,
        6, // log2_width = 6 -> 64
        5, // log2_height = 5 -> 32
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for row in 0..32 {
        for col in 0..64 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, 64 + col, row).unwrap(),
                70,
                "no-above V_PRED 64x32 sample ({},{row}) must be the flat left corner left[0]=70",
                64 + col,
            );
        }
    }
}

/// §7.13.2.1 NO-LEFT FALLBACK GUARD — the symmetric H_PRED case at the frame
/// LEFT edge (`x == 0`, `haveLeft == 0`) with a NON-FLAT reconstructed above row.
/// §7.13.2.1 synthesizes `LeftCol[i] = CurrFrame[plane][y-1][x]` (`above[0]`),
/// so the H_PRED copy is FLAT equal to `above[0]`, NOT `above[j]`.
#[test]
fn rect_cardinal_horizontal_32x64_no_left_fallback_is_flat_above_corner() {
    let mut ws = new_general_intra_workspace::<u8>(64, 128, BitDepth::Eight).unwrap();
    let above_row: Vec<u8> = (0..32).map(|x| 80 + x as u8).collect();
    lay_above_row(&mut ws, 63, 5, &above_row);

    reconstruct_general_intra_cardinal_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        IntraCardinalDirection::Horizontal,
        PlaneId::Y,
        0,
        64,
        5, // log2_width = 5 -> 32
        6, // log2_height = 6 -> 64
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for row in 0..64 {
        for col in 0..32 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, col, 64 + row).unwrap(),
                80,
                "no-left H_PRED 32x64 sample ({col},{}) must be the flat above corner above[0]=80",
                64 + row,
            );
        }
    }
}

/// Reference §7.13.2.2 Paeth sample (independent of the splot-recon primitive):
/// pick whichever of `left` / `above` / `top_left` is closest to
/// `above + left - top_left`, ties favouring left then above.
fn ref_paeth(left: i32, above: i32, top_left: i32) -> u8 {
    let base = above + left - top_left;
    let p_left = (base - left).abs();
    let p_top = (base - above).abs();
    let p_top_left = (base - top_left).abs();
    let v = if p_left <= p_top && p_left <= p_top_left {
        left
    } else if p_top <= p_top_left {
        above
    } else {
        top_left
    };
    u8::try_from(v).unwrap()
}

/// STRIDE / CORNER GUARD — §7.13.2.2 PAETH over a NON-SQUARE 8x16 (`W == 8`,
/// `H == 16`) block with a REAL, NON-FLAT above row, a REAL, NON-FLAT left
/// column, AND a DISTINCT corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]`.
/// Paeth genuinely depends on all three (`base = AboveRow[j] + LeftCol[i] -
/// AboveRow[-1]`), so a width/height swap, a wrong stride, or reading the corner
/// from the above row / left column instead of `CurrFrame[y-1][x-1]` would
/// corrupt the output and fail. The asymmetric edges are the key: the flat
/// ac0ej3 oracle (all `68`) would MASK every one of those mix-ups.
#[test]
fn rect_paeth_8x16_uses_above_left_and_distinct_corner() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();

    let above: Vec<u8> = (0..8).map(|j| 30 + 7 * j as u8).collect();
    let above_block: Vec<u8> = (0..4).flat_map(|_| above.iter().copied()).collect();
    ws.write_rect_block(
        PlaneId::Y,
        16,
        12,
        IntraRectBlockSize::new(3, 2).unwrap(),
        &above_block,
    )
    .unwrap();

    let corner: u8 = 200;
    let left: Vec<u8> = (0..16).map(|i| 40 + 5 * i as u8).collect();
    let mut left_block = vec![0u8; 4 * 32];
    for col in 0..4 {
        left_block[15 * 4 + col] = corner;
        for (i, &v) in left.iter().enumerate() {
            left_block[(16 + i) * 4 + col] = v;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        12,
        0,
        IntraRectBlockSize::new(2, 5).unwrap(),
        &left_block,
    )
    .unwrap();

    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 15, 15).unwrap(), corner);
    assert_eq!(
        ws.reconstructed_sample(PlaneId::Y, 16, 15).unwrap(),
        above[0]
    );
    assert_eq!(
        ws.reconstructed_sample(PlaneId::Y, 15, 16).unwrap(),
        left[0]
    );

    reconstruct_general_intra_luma_paeth_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        PlaneId::Y,
        16,
        16,
        3, // log2_width = 3 -> 8
        4, // log2_height = 4 -> 16
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for (i, &left_i) in left.iter().enumerate() {
        for (j, &above_j) in above.iter().enumerate() {
            let want = ref_paeth(i32::from(left_i), i32::from(above_j), i32::from(corner));
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, 16 + j, 16 + i).unwrap(),
                want,
                "PAETH 8x16 sample (col {j}, row {i}) must be Paeth(left[{i}], above[{j}], corner)"
            );
        }
    }
}

/// FRAME-EDGE FALLBACK GUARD — §7.13.2.1 `haveAbove == 0 && haveLeft == 1` PAETH
/// over an 8x16 block at the TOP frame edge (`y == 0`, so there is no above row)
/// with `x > 0` (a real, NON-FLAT reconstructed left column). The §7.13.2.1
/// reference build synthesizes every `AboveRow[j]` and the corner `AboveRow[-1]`
/// from `CurrFrame[plane][y][x-1] == left[0]`, so the Paeth reference is
/// `ref_paeth(left[i], left[0], left[0])` for each row. An ASYMMETRIC left column
/// (`left[0] != left[i]`) is load-bearing: a flat column would mask reading the
/// wrong synthesized above/corner value. Matches AVM `av2_build_intra_predictors_high`
/// (`above_row[i] = above_row[-1] = left_ref[0]` when `n_top_px == 0`).
#[test]
fn rect_paeth_8x16_top_edge_synthesizes_above_from_left() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();

    let left: Vec<u8> = (0..16).map(|i| 40 + 5 * i as u8).collect();
    lay_left_col(&mut ws, 15, 4, &left);
    assert_eq!(ws.reconstructed_sample(PlaneId::Y, 15, 0).unwrap(), left[0]);

    reconstruct_general_intra_luma_paeth_neighbour_block_into(
        &mut ws,
        &all_zero_luma_block(),
        PlaneId::Y,
        16,
        0,
        3, // log2_width = 3 -> 8
        4, // log2_height = 4 -> 16
        0,
        false,
        BitDepth::Eight,
    )
    .unwrap();

    for (i, &left_i) in left.iter().enumerate() {
        let want = ref_paeth(i32::from(left_i), i32::from(left[0]), i32::from(left[0]));
        for j in 0..8 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, 16 + j, i).unwrap(),
                want,
                "top-edge PAETH sample (col {j}, row {i}) must be Paeth(left[{i}], left[0], left[0])"
            );
        }
    }
}

/// RESIDUAL-ADD GUARD — §7.13.2.2 PAETH over a NON-SQUARE 8x16 block with REAL,
/// NON-FLAT above row / left column / DISTINCT corner (so the Paeth prediction is
/// itself NON-FLAT), carrying a NON-`all_zero` `ADST_ADST` residual with several
/// asymmetric coefficients. The reconstructed output must equal the §7.14.3
/// `Clip1(paethPred + inverse-transform(residual))` — proven by independently
/// computing the Paeth prediction (the verbatim `ref_paeth` over the laid edges)
/// and reconstructing the SAME residual onto it through the shared
/// `reconstruct_general_intra_block_rect_with_prediction` (the §7.14.3 helper every
/// residual path uses). A path that dropped the residual (writing the bare
/// prediction) or added it onto the wrong predictor would diverge from this
/// reference; the asymmetric coeffs + non-flat prediction make any such mix-up
/// observable (a flat prediction or DC-only residual would MASK it).
#[test]
fn rect_paeth_8x16_adds_residual_onto_the_paeth_prediction() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();

    let above: Vec<u8> = (0..8).map(|j| 30 + 7 * j as u8).collect();
    let above_block: Vec<u8> = (0..4).flat_map(|_| above.iter().copied()).collect();
    ws.write_rect_block(
        PlaneId::Y,
        16,
        12,
        IntraRectBlockSize::new(3, 2).unwrap(),
        &above_block,
    )
    .unwrap();

    let corner: u8 = 200;
    let left: Vec<u8> = (0..16).map(|i| 40 + 5 * i as u8).collect();
    let mut left_block = vec![0u8; 4 * 32];
    for col in 0..4 {
        left_block[15 * 4 + col] = corner;
        for (i, &v) in left.iter().enumerate() {
            left_block[(16 + i) * 4 + col] = v;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        12,
        0,
        IntraRectBlockSize::new(2, 5).unwrap(),
        &left_block,
    )
    .unwrap();

    let mut paeth_pred = vec![0u8; 8 * 16];
    for (i, &left_i) in left.iter().enumerate() {
        for (j, &above_j) in above.iter().enumerate() {
            paeth_pred[i * 8 + j] =
                ref_paeth(i32::from(left_i), i32::from(above_j), i32::from(corner));
        }
    }

    let mut quant = vec![0i32; 128];
    quant[0] = -96;
    quant[1] = 41;
    quant[8] = -23;
    quant[9] = 12;
    let block = LumaCoeffBlock {
        all_zero: false,
        eob: 10,
        quant,
        intra_ist: None,
        plane_tx_type: 3, // ADST_ADST
    };

    let want = reconstruct_general_intra_block_rect_with_prediction(
        &block.quant,
        &paeth_pred,
        149,
        PlaneId::Y,
        3,
        4,
        block.plane_tx_type,
        true,
        BitDepth::Eight,
    )
    .unwrap();

    reconstruct_general_intra_luma_paeth_neighbour_block_into(
        &mut ws,
        &block,
        PlaneId::Y,
        16,
        16,
        3,
        4,
        149,
        true,
        BitDepth::Eight,
    )
    .unwrap();

    let mut differs_from_prediction = false;
    for i in 0..16 {
        for j in 0..8 {
            let got = ws.reconstructed_sample(PlaneId::Y, 16 + j, 16 + i).unwrap();
            assert_eq!(
                got,
                want[i * 8 + j],
                "PAETH+residual 8x16 sample (col {j}, row {i}) must be Clip1(paethPred + residual)"
            );
            if u32::from(got) != u32::from(paeth_pred[i * 8 + j]) {
                differs_from_prediction = true;
            }
        }
    }
    assert!(
        differs_from_prediction,
        "the residual must actually move samples off the bare Paeth prediction"
    );
}

/// END-TO-END §7.13.2.9 useIBP — an 8x8 zone-1 `pAngle=45` `all_zero` leaf at
/// (8, 8) with a REAL, NON-FLAT above row, left column, and a DISTINCT corner,
/// the §7.13.2.7 edge filter a NO-OP (`OneSidedEdgeFilter::default()` on BOTH
/// edges), `num4_far == 0`. The output is the §7.13.2.8 primary (zone-1 above)
/// blended with the secondary (`secondAngle = 225`, zone-3 left) per §7.13.2.9.
/// The expected 8x8 samples are computed offline from the VERBATIM AVM IDIF
/// primary + secondary predictors and the §7.13.2.9 weights/blend (asymmetric
/// above/left values, so a primary<->secondary swap, a missing transpose, or a
/// wrong secondAngle changes the pinned bytes). This exercises the whole IBP
/// reconstructor end to end, since no ac0ej3 leaf reaches it (the decode-order
/// cascade is config-blocked; see the recon-region tests).
///
/// Neighbour layout: above row `y=7` over `x[8,16)` is `100+i` (laid via an 8x4
/// block at `(8,4)`); left column `x=7` is `corner(7,7)=200` then `50+2i` over
/// `y[8,16)` (laid via a 4x16 block at `(4,0)`).
#[test]
fn one_sided_ibp_8x8_p45_blends_primary_and_secondary_bit_exact() {
    let mut ws = new_general_intra_workspace::<u8>(64, 64, BitDepth::Eight).unwrap();
    let above_in: Vec<u8> = (0..8).map(|i| 100 + i as u8).collect();
    let above_block: Vec<u8> = (0..4).flat_map(|_| above_in.iter().copied()).collect();
    ws.write_rect_block(
        PlaneId::Y,
        8,
        4,
        IntraRectBlockSize::new(3, 2).unwrap(),
        &above_block,
    )
    .unwrap();
    let corner: u8 = 200;
    let left_in: Vec<u8> = (0..8).map(|i| 50 + 2 * i as u8).collect();
    let mut left_block = vec![1u8; 4 * 16];
    for col in 0..4 {
        left_block[7 * 4 + col] = corner;
        for (i, &v) in left_in.iter().enumerate() {
            left_block[(8 + i) * 4 + col] = v;
        }
    }
    ws.write_rect_block(
        PlaneId::Y,
        4,
        0,
        IntraRectBlockSize::new(2, 4).unwrap(),
        &left_block,
    )
    .unwrap();

    reconstruct_general_intra_one_sided_ibp_luma_block_into(
        &mut ws,
        &all_zero_luma_block(),
        45, // pAngle (zone-1)
        8,
        8,
        3, // log2_width = 3 -> 8
        3, // log2_height = 3 -> 8
        0, // qindex (unused for all_zero)
        0, // primary_num4_far (above-right clamps to above_in[7])
        OneSidedEdgeFilter::default(),
        IbpSecondary {
            second_angle: 225,
            edge_filter: OneSidedEdgeFilter::default(),
            num4_far: 0, // below-left clamps to left_in[7]
        },
        false,
        BitDepth::Eight,
    )
    .unwrap();

    #[rustfmt::skip]
    let expected: [u8; 64] = [
        77, 86, 91, 95, 98, 100, 102, 102,
        70, 80, 86, 90, 94,  96,  98,  99,
        68, 76, 83, 87, 91,  93,  94,  95,
        67, 75, 81, 86, 88,  90,  91,  93,
        67, 75, 80, 83, 86,  88,  89,  91,
        68, 75, 78, 81, 83,  86,  87,  89,
        69, 73, 77, 80, 82,  84,  86,  87,
        69, 73, 76, 78, 80,  82,  84,  86,
    ];
    for row in 0..8 {
        for col in 0..8 {
            assert_eq!(
                ws.reconstructed_sample(PlaneId::Y, 8 + col, 8 + row)
                    .unwrap(),
                expected[row * 8 + col],
                "IBP 8x8 p45 sample (col {col}, row {row}) must match the AVM blend"
            );
        }
    }
}
