// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Oracle-verified general minimal-tool intra decode tests for the shared
//! minimal-tier runtime.

use splot_parallel::ThreadCount;

use super::general_intra::full_sb_num4_above_right;
use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

const Q80_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");

// avmdec and dav2d both decode this fixture to flat planes.
const Q80_LUMA: u8 = 100;
const Q80_CHROMA_U: u8 = 120;
const Q80_CHROMA_V: u8 = 130;

// A single-block DC_PRED intra frame whose luma carries multiple (eob > 1) AC
// coefficients from a low-frequency half-cosine input; avmdec's raw output is
// reproduced byte-for-byte (verified locally) and pinned via the frame hash.
const Q180_COS_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-cos-intra-64x64-q180.ivf");

// Drives the q80 fixture through the full general intra runtime path: decode
// modes -> decode luma + chroma coefficients -> dequant -> inverse transform
// -> residual add over the no-neighbour DC prediction -> frame assembly.
fn decode_q80_frame() -> DecodedFrame<u8> {
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(Q80_FIXTURE, options).expect("plan");
    decode_minimal_frame_from_plan(Q80_FIXTURE, options, &plan)
        .expect("decode")
        .frame
}

#[test]
fn q80_intra_frame_reconstructs_flat_planes() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_q80_frame();
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    let y = frame.y().samples();
    assert!(
        y.iter().all(|&s| s == Q80_LUMA),
        "luma must be flat {Q80_LUMA}; first samples: {:?}",
        &y[..8]
    );
    let u = frame.u().unwrap().samples();
    assert!(
        u.iter().all(|&s| s == Q80_CHROMA_U),
        "U must be flat {Q80_CHROMA_U}; first samples: {:?}",
        &u[..8]
    );
    let v = frame.v().unwrap().samples();
    assert!(
        v.iter().all(|&s| s == Q80_CHROMA_V),
        "V must be flat {Q80_CHROMA_V}; first samples: {:?}",
        &v[..8]
    );
}

#[test]
fn q80_intra_frame_hash_is_stable() {
    // Regression pin for the full-frame decode hash. The flat-plane test
    // above is the avmdec/dav2d oracle anchor; this pins the byte layout.
    let frame = decode_q80_frame();
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979"
    );
}

#[test]
fn q180_cos_intra_frame_decodes_multi_coefficient_luma() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(Q180_COS_FIXTURE, options).expect("plan");
    let frame = decode_minimal_frame_from_plan(Q180_COS_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    // The luma is a reconstructed low-frequency cosine: genuinely non-flat
    // (proving the eob > 1 AC coefficient path ran, not just a DC level).
    let y = frame.y().samples();
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(
        distinct > 4,
        "luma should be a non-flat AC reconstruction; distinct={distinct}"
    );

    // Frame hash pins splot's output, which reproduces avmdec's raw output
    // byte-for-byte (verified locally against ~/Devel/avm/build/avmdec).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "8a6751d4517073bad0bbe71f4b5537df8e8b0bfee85fcd6af1ac2d5878dd59e8"
    );
}

// A multi-block intra frame: four flat 32x32 luma quadrants that split
// (Horz -> Vert -> Vert) into four square DC_PRED blocks. Each non-first
// block DC-predicts from its already-reconstructed neighbour. avmdec and
// dav2d agree on the decoded output.
const QUAD_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-quad-intra-64x64-q80.ivf");

#[test]
fn quad_multiblock_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(QUAD_FIXTURE, options).expect("plan");
    let frame = decode_minimal_frame_from_plan(QUAD_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    let y = frame.y().samples();
    let quad = |r: usize, c: usize| y[r * 64 + c];
    // TL / TR / BL / BR quadrant centres, matching the avmdec/dav2d oracle.
    assert_eq!(
        (quad(16, 16), quad(16, 48), quad(48, 16), quad(48, 48)),
        (80, 200, 160, 40)
    );
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "c54ed4e996841e2178e74033d765dda1e1127d5d89c3012be3266c3e24a7fd28"
    );
}

// A 64x64 two-level partition tree: the 64x64 superblock SPLITs into four
// 32x32 quadrants and the top-left 32x32 SPLITs AGAIN into four 16x16 DC_PRED
// blocks (the other three quadrants stay 32x32 DC_PRED). One level deeper than
// the merged `syn-quad` case: each 16x16 leaf DC-predicts from its
// already-reconstructed 16x16 neighbour INSIDE the parent 32x32 sub-block
// (the §5.20.4.1 partition recursion pushes the 16x16 SPLIT children, and the
// §7.13.2.4 DC predictor reads the in-frame left column / above row that those
// sibling 16x16 blocks just wrote). avmdec and dav2d agree on the decoded
// output (md5 5e348413...).
const DEEP_SPLIT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-deep-intra-64x64-q120.ivf");

#[test]
fn deep_split_sub_32x32_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context
        .plan_bytes(DEEP_SPLIT_FIXTURE, options)
        .expect("plan");
    let frame = decode_minimal_frame_from_plan(DEEP_SPLIT_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    // The top-left 32x32 holds four distinct flat 16x16 DC blocks; the other
    // three 32x32 quadrants are flat DC. Sampling each block centre proves the
    // deeper SPLIT decoded the right per-block DC values (each from a
    // reconstructed sibling neighbour), matching the avmdec/dav2d oracle.
    let y = frame.y().samples();
    let at = |col: usize, row: usize| y[row * 64 + col];
    // Top-left 32x32 -> four 16x16 leaves: TL/TR/BL/BR centres.
    assert_eq!(
        (at(8, 8), at(24, 8), at(8, 24), at(24, 24)),
        (240, 21, 21, 240)
    );
    // The remaining three 32x32 quadrants (TR / BL / BR) stay DC.
    assert_eq!((at(48, 16), at(16, 48), at(48, 48)), (130, 70, 200));

    // Chroma is near-flat DC (U around 120, V 130), matching the oracle.
    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == 120 || s == 121)
    );
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's raw
    // output byte-for-byte (verified locally, md5 5e348413...).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "73123e51c66787b59fb6b93a6221e9d78a550c6e0d1c4e0c1adfd21a41ed39ab"
    );
}

// A 128x64 multi-superblock intra frame: two 64x64 DC_PRED superblocks (left
// flat luma 80, right flat luma 180). The right superblock DC-predicts its
// luma from the already-reconstructed left-superblock neighbour, and codes
// its (residual-free) chroma as SMOOTH_PRED over that flat neighbour. avmdec
// and dav2d agree on the decoded output (md5 88cf94a2...).
const TWO_SB_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-2sb-intra-128x64-q80.ivf");

#[test]
fn two_superblock_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(TWO_SB_FIXTURE, options).expect("plan");
    let frame = decode_minimal_frame_from_plan(TWO_SB_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );

    // Left superblock (cols 0..64) is flat luma 80, right superblock
    // (cols 64..128) is flat luma 180, matching the avmdec/dav2d oracle.
    let y = frame.y().samples();
    assert!(
        (0..64).all(|r| (0..64).all(|c| y[r * 128 + c] == 80)),
        "left superblock luma must be flat 80"
    );
    assert!(
        (0..64).all(|r| (64..128).all(|c| y[r * 128 + c] == 180)),
        "right superblock luma must be flat 180"
    );
    // Chroma is flat across both superblocks (U=120, V=130).
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f"
    );
}

// A 64x128 single-column multi-superblock-ROW intra frame: two vertically
// stacked 64x64 DC_PRED superblocks (top flat luma 80, bottom flat luma 180,
// chroma 120/130). Exercises the §5.20.2.1 superblock raster loop across
// multiple ROWS (`clear_left_context()` per superblock row), with the
// second-row superblock DC-predicting its luma from the already-reconstructed
// first-row above neighbour and reconstructing full-superblock SMOOTH chroma
// at row > 0 (a rightmost-column superblock, so no decoded above-right). avmdec
// and dav2d agree on the decoded output (md5 bd09ea82...).
const COL_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-2sbcol-intra-64x128-q80.ivf");

#[test]
fn multi_row_superblock_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(COL_FIXTURE, options).expect("plan");
    let frame = decode_minimal_frame_from_plan(COL_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 128).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(32, 64).unwrap()
    );

    // Top superblock (rows 0..64) flat luma 80; bottom superblock (rows
    // 64..128) flat luma 180, DC-predicted from the reconstructed first-row
    // neighbour. Chroma flat U=120 / V=130. Matches the avmdec/dav2d oracle.
    let y = frame.y().samples();
    assert!(
        (0..64).all(|r| (0..64).all(|c| y[r * 64 + c] == 80)),
        "top superblock luma must be flat 80"
    );
    assert!(
        (64..128).all(|r| (0..64).all(|c| y[r * 64 + c] == 180)),
        "bottom superblock luma must be flat 180"
    );
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "3ee739a805e13597ff7d75659dd1e0150113bf4782c4d69e1d27ae942d6c10a0"
    );
}

// A 128x128 2-D grid of four 64x64 DC_PRED-luma superblocks. Luma is uniform
// (100) so every superblock is DC; chroma is distinct flat per quadrant
// (U: top-left 110 / top-right 200 / bottom-right 130) except the bottom-left
// superblock, whose chroma the encoder codes as SMOOTH_PRED over a real 2-D
// gradient. That bottom-left superblock (raster MI col 0, row > 0) has a
// decoded above-right neighbour (the top-right superblock), so its §7.13.2.13
// top-right sentinel `AboveRow[w]` reads the real reconstructed above-right
// sample (200) per §7.13.2.1 / §5.20.7.25 `count_top_right_avail` — NOT the
// edge-clamped own-top sample (110). avmdec and dav2d agree on the decoded
// output (md5 dd2fa84f...); the old repeat-last sentinel mismatched it.
const GRID_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-grid-intra-128x128-q80.ivf");

#[test]
fn grid_2d_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(GRID_FIXTURE, options).expect("plan");
    let frame = decode_minimal_frame_from_plan(GRID_FIXTURE, options, &plan)
        .expect("decode")
        .frame;

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 128).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 64).unwrap()
    );
    assert_eq!(
        frame.v().unwrap().visible_size(),
        PlaneSize::new(64, 64).unwrap()
    );

    // Uniform luma 100 across the whole 2-D grid (all DC_PRED), matching the
    // avmdec/dav2d oracle.
    assert!(
        frame.y().samples().iter().all(|&s| s == 100),
        "luma must be uniform 100 across the 2-D grid"
    );

    // Chroma quadrant helper (64x64 chroma plane, 32x32 quadrants).
    let quad = |plane: &[u8], qr: usize, qc: usize| -> Vec<u8> {
        let mut out = Vec::new();
        for r in (qr * 32)..(qr * 32 + 32) {
            for c in (qc * 32)..(qc * 32 + 32) {
                out.push(plane[r * 64 + c]);
            }
        }
        out
    };
    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();

    // Three flat distinct quadrants (top-left, top-right, bottom-right).
    assert!(
        quad(u, 0, 0).iter().all(|&s| s == 110),
        "U top-left flat 110"
    );
    assert!(
        quad(u, 0, 1).iter().all(|&s| s == 200),
        "U top-right flat 200"
    );
    assert!(
        quad(u, 1, 1).iter().all(|&s| s == 130),
        "U bottom-right flat 130"
    );
    assert!(
        quad(v, 0, 0).iter().all(|&s| s == 120),
        "V top-left flat 120"
    );
    assert!(
        quad(v, 0, 1).iter().all(|&s| s == 160),
        "V top-right flat 160"
    );
    assert!(
        quad(v, 1, 1).iter().all(|&s| s == 140),
        "V bottom-right flat 140"
    );

    // The bottom-left superblock chroma is SMOOTH_PRED over a real gradient
    // (not flat), so the above-right sentinel actually shapes the prediction.
    let u_bl = quad(u, 1, 0);
    let v_bl = quad(v, 1, 0);
    assert!(
        u_bl.iter().collect::<std::collections::BTreeSet<_>>().len() > 1,
        "U bottom-left superblock must be a SMOOTH gradient, not flat"
    );
    assert!(
        v_bl.iter().collect::<std::collections::BTreeSet<_>>().len() > 1,
        "V bottom-left superblock must be a SMOOTH gradient, not flat"
    );
    // The bottom-left superblock's top edge is its own top-left neighbour
    // (110), but its decoded above-right is the top-right superblock (200);
    // the §7.13.2.1 above-right sentinel pulls the top-right corner toward
    // 200, so the bottom-left's top row rises above its own top edge.
    // `u_bl[0]` is the top-left corner (110); `u_bl[31]` is the top-right
    // corner of the bottom-left superblock.
    assert_eq!(
        u_bl[0], 110,
        "U bottom-left top-left corner == own top edge"
    );
    assert!(
        u_bl[31] > u_bl[0],
        "U bottom-left top-right corner must be pulled toward the above-right (200), proving the above-right read"
    );

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally vs avmdec + dav2d).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "42bd99faae1ac0acb15c3e24fbededd8fc670612d08987bebb8942de5f4f4874"
    );
}

#[test]
fn full_sb_num4_above_right_matches_count_top_right_avail() {
    // 128x128 (mi_cols = 32), full 64x64 superblock (n4w = 16), 4:2:0
    // (sub_x = 1) -> chroma w4 = 8. The bottom-left superblock (c = 0) has an
    // in-frame decoded above-right (the top-right superblock): chroma above
    // row decoded out to (32 - 0) >> 1 = 16 columns, so columns 8..15 are all
    // decoded -> num4AboveRight = 8 (capped at w4).
    assert_eq!(full_sb_num4_above_right(0, 16, 32, 1), 8);
    // The rightmost superblock (c = 16) has no in-frame above-right: chroma
    // above row decoded out to (32 - 16) >> 1 = 8 columns, so columns 8..15
    // are all undecoded -> num4AboveRight = 0 (and the §7.13.2.1 clamp /
    // no-above fallback applies).
    assert_eq!(full_sb_num4_above_right(16, 16, 32, 1), 0);
    // A single-column frame (mi_cols = 16) has only one superblock per row, so
    // the rightmost (only) superblock at c = 0 has no above-right.
    assert_eq!(full_sb_num4_above_right(0, 16, 16, 1), 0);
    // A 3-wide grid (mi_cols = 48): the middle superblock (c = 16) still has a
    // decoded above-right (the right superblock): decoded out to
    // (48 - 16) >> 1 = 16 columns, columns 8..15 decoded -> 8.
    assert_eq!(full_sb_num4_above_right(16, 16, 48, 1), 8);
}

// Single-block non-DC intra: a 64x64 vertical-gradient luma block the encoder
// codes as SMOOTH_V_PRED (DC chroma). The decoder builds the §7.13.2.13
// vertical smooth prediction over the §7.13.2.1 no-neighbour fallback edges
// and adds the AC residual. avmdec and dav2d agree on the decoded output.
const VSMOOTH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-vsmooth-intra-64x64-q120.ivf");
// Companion single-block SMOOTH_H_PRED (horizontal-gradient) block.
const HSMOOTH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hsmooth-intra-64x64-q120.ivf");

fn decode_general_intra_luma(fixture: &[u8]) -> DecodedFrame<u8> {
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(fixture, options).expect("plan");
    decode_minimal_frame_from_plan(fixture, options, &plan)
        .expect("decode")
        .frame
}

#[test]
fn vsmooth_single_block_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(VSMOOTH_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    let y = frame.y().samples();
    // SMOOTH_V over a vertical gradient: each row is constant across columns,
    // and the gradient increases top-to-bottom (proving the non-DC prediction
    // plus AC residual ran, not a flat DC level).
    assert!(
        y[0..64].iter().all(|&s| s == y[0]),
        "top row should be constant across columns"
    );
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "3aebe2eb215d4878bbc40aa2f97e2178b6140ef51c03afaaae478e69dbbf6bcd"
    );
}

#[test]
fn hsmooth_single_block_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(HSMOOTH_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    let y = frame.y().samples();
    // SMOOTH_H over a horizontal gradient: each column is constant across rows,
    // and the gradient increases left-to-right.
    assert!(
        (0..64).all(|r| y[r * 64] == y[0]),
        "left column should be constant across rows"
    );
    assert!(y[0] < y[63], "luma should increase left-to-right");
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "cfc6debd26760cdebf1d1a4497792461f0f68bc7e7773741ddf2cbc34561e702"
    );
}

// Single-block directional intra: a 64x64 block the encoder codes as the
// § 5.20.5.3 `y_mode_offset` escape (`y_mode_set == 0`,
// `y_mode_index == MODE_INDEX_COUNT - 1`, `y_mode_offset == 3`), which
// reconstructs `D135_PRED` (pAngle 135, `AngleDeltaY == 0`). The decoder
// builds the § 7.13.2.8 directional prediction over the § 7.13.2.1
// no-neighbour fallback edges and adds the residual. avmdec and dav2d agree on
// the decoded output (md5 1179bcc873c1d1ac49c2c032f11ca44d, DC chroma).
const HEDGE_DIR_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hedge-intra-64x64-q80.ivf");

#[test]
fn hedge_directional_d135_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(HEDGE_DIR_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());

    // The D135 directional prediction over a top/bottom split residual: the
    // top half reconstructs near 40 and the bottom half near 210 (a genuinely
    // non-flat reconstruction, not a single DC level). Chroma is flat DC.
    let y = frame.y().samples();
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally; md5
    // 1179bcc873c1d1ac49c2c032f11ca44d).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "b15f267ec6e99ca4d96a70f38bffe5f798ee4c33ad3aaec23761a1ea74b0be33"
    );
}

// Single-block directional-follow chroma intra: a 64x64 block whose luma codes
// as `D135_PRED` (the § 5.20.5.3 `y_mode_offset` escape, pAngle 135,
// `AngleDeltaY == 0`) and whose chroma codes with `uv_mode == 0`, so
// § 5.20.5.3 `get_intra_uv_mode_set` returns `YMode` (the directional-follow
// branch) — `UVMode == D135_PRED`, `AngleDeltaUV == 0`. Over the § 7.13.2.1
// no-neighbour fallback edges the chroma § 7.13.2.8 middle-angle prediction is
// the same `enableIdif == 0` bilinear sample copy the luma D135 path uses
// (shift `0`), so both U and V reconstruct as a genuine 135° anti-diagonal
// pattern (not flat, not DC) plus residual. avmdec and dav2d agree on the
// decoded output (md5 09fc23f0bced8ab5b9562d6d2478af1c); the first
// general-intra directional-follow chroma decode target.
const DFCHROMA_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-dfchroma-intra-64x64-q80.ivf");

#[test]
fn dfchroma_directional_follow_chroma_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(DFCHROMA_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(32, 32).unwrap()
    );

    // Luma is the D135 top/bottom split (top near 40, bottom near 210).
    let y = frame.y().samples();
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");

    // Chroma is a GENUINE directional (D135) reconstruction, not flat DC: a
    // 135° anti-diagonal pattern where the upper-right triangle (c > r) sits
    // below the lower-left triangle (r > c). U and V each take many distinct
    // values (not a single DC level), proving the directional chroma predictor
    // ran, not the DC fallback.
    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    let at = |p: &[u8], r: usize, c: usize| p[r * 32 + c];
    let u_distinct = u.iter().collect::<std::collections::BTreeSet<_>>().len();
    let v_distinct = v.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(
        u_distinct > 4,
        "U chroma must be a non-flat directional reconstruction"
    );
    assert!(
        v_distinct > 4,
        "V chroma must be a non-flat directional reconstruction"
    );
    assert!(
        at(u, 2, 28) < at(u, 28, 2),
        "U upper-right (c>r) must be below lower-left (r>c) for D135"
    );
    assert!(
        at(v, 2, 28) < at(v, 28, 2),
        "V upper-right (c>r) must be below lower-left (r>c) for D135"
    );

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally; md5
    // 09fc23f0bced8ab5b9562d6d2478af1c).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "628b759dcb63356ad3174063652c54d7ebf6f54d1566ab9f1b64b3a74542154f"
    );
}

// A 128x64 multi-superblock intra frame whose two 64x64 superblocks are both
// coded as `SMOOTH_V_PRED` (§ 7.13.2.13) over a vertical luma gradient
// (top 30, bottom 210, flat chroma U=120 V=130). The LEFT (top-left,
// no-neighbour) superblock predicts over the § 7.13.2.1 flat fallback edges.
// The RIGHT superblock has a left neighbour, so its § 7.13.2.13 prediction
// reads the REAL reconstructed neighbour edge: § 7.13.2.1 supplies the
// reconstructed left column (the left superblock's right column) and, with no
// above neighbour (`haveAbove == 0, haveLeft == 1`), the repeated-left above
// row `AboveRow[i] = CurrFrame[0][y][x-1]`. Smooth prediction is linear
// interpolation (no IDIF / edge-filter synthesis), so the non-flat neighbour
// edge reconstructs bit-exact. avmdec and dav2d agree on the decoded output
// (md5 3e57ba0c8cbdbe1d3400b0ae365c5d8e); the first general-intra multi-block
// non-DC luma decode over a reconstructed neighbour edge.
const MBVG_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mbvg-intra-128x64-q80.ivf");

#[test]
fn mbvg_multiblock_smooth_v_neighbour_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(MBVG_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    // Each superblock is SMOOTH_V (constant across columns within a row), with
    // a vertical gradient top-to-bottom.
    assert!(
        (0..64).all(|c| at(20, c) == at(20, 0)),
        "left superblock must be constant across columns within a row"
    );
    assert!(
        (64..128).all(|c| at(20, c) == at(20, 64)),
        "right superblock must be constant across columns within a row"
    );
    assert!(
        at(0, 0) < at(63, 0),
        "left superblock increases top-to-bottom"
    );
    assert!(
        at(0, 64) < at(63, 64),
        "right superblock increases top-to-bottom"
    );
    // The right superblock reads the REAL reconstructed left-neighbour edge
    // plus its own residual, so its column profile differs from the left
    // superblock's (it is not a copy): the superblock centres differ.
    assert_ne!(
        at(32, 32),
        at(32, 96),
        "right superblock must read the real neighbour edge, not duplicate the left"
    );
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    // Chroma is flat DC across both superblocks (U=120, V=130).
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally; md5
    // 3e57ba0c8cbdbe1d3400b0ae365c5d8e).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "269b4969800751c63f7f0605f1f7b8f178f7bf85590ec62fe64313ff394d6dfd"
    );
}

// A 128x64 multi-superblock intra frame whose LEFT 64x64 codes as § 7.13.2.8
// `D135_PRED` luma (the § 5.20.5.3 `y_mode_offset` escape, `IntraJointMode 36`,
// 40/210 anti-diagonal hedge) and whose RIGHT 64x64 codes as `SMOOTH_V_PRED`
// luma over a vertical gradient (flat chroma U=120 V=130). The RIGHT block's
// left neighbour stored the directional `IntraJointMode 36`
// (`>= NON_DIRECTIONAL_MODES_COUNT`), so its § 8.3.2 `y_mode_index` ctx is 1 and
// it reads from `TileYModeIndexCdf[1]` — the first general-intra
// directional-neighbour (`ctx != 0`) `y_mode_index` decode (the earlier code
// rejected this exact frame, `general_intra_directional_neighbour_y_mode_index_ctx`).
// Its `modeIdx == y_mode_index == 2 < NON_DIRECTIONAL_MODES_COUNT` passes through
// § 5.20.5.3 `get_intra_y_mode_set` unchanged (the reorder fires only for
// `modeIdx >= 5`) -> `Reordered_Y_Mode[2] == SMOOTH_V_PRED`. avmdec and dav2d
// agree (md5 1a84b6545ee333b98cdf1982fd18310a). Rationale in the matrix row.
const DIRNEIGH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-dirneigh-intra-128x64-q80.ivf");

#[test]
fn dirneigh_directional_neighbour_ctx_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(DIRNEIGH_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    // LEFT superblock: D135 anti-diagonal hedge — a genuine directional pattern
    // (varies across columns in its top row, not flat/row-constant) that darkens
    // top-to-bottom.
    assert!(
        (1..64).any(|c| at(0, c) != at(0, 0)),
        "left superblock top row must vary across columns (directional, not flat)"
    );
    assert!(at(0, 0) < at(63, 0), "left darkens top-to-bottom");
    // RIGHT superblock: SMOOTH_V — constant across columns within a row, with a
    // vertical gradient increasing top-to-bottom (the ctx==1 decode resolved to
    // SMOOTH_V_PRED and ran the non-DC prediction plus AC residual).
    assert!(
        (64..128).all(|c| at(20, c) == at(20, 64)),
        "right superblock must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(0, 64) < at(63, 64),
        "right increases top-to-bottom (SMOOTH_V)"
    );
    // Chroma is flat (U=120, V=130) across both superblocks.
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's raw
    // output byte-for-byte (verified locally; md5
    // 1a84b6545ee333b98cdf1982fd18310a).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "ad1515885df5620a31c37f855934ae2432167edbf1b1b62081552b9df3957426"
    );
}

// A 192x128 full 2-D grid (3 superblock columns x 2 superblock rows) whose
// right two columns code as `SMOOTH_V_PRED` (§ 7.13.2.13) luma over a vertical
// gradient (top 20, bottom 230, with a small per-column tint; flat chroma).
// The decisive block is the row > 0 SMOOTH_V luma superblock at the **middle**
// (non-rightmost) column (`frontier.r == 16`, `frontier.c == 16`): unlike the
// first-row `syn-mbvg` SMOOTH_V block (`haveAbove == 0`, above row a fallback),
// this one has `haveAbove == 1`, so § 7.13.2.1 supplies the **real
// reconstructed above row** `CurrFrame[0][y-1][...]` (the bottom row of the
// already-decoded above superblock); being non-rightmost it also has a decoded
// above-right superblock, so `full_sb_num4_above_right` / the § 7.13.2.1
// `AboveRow[w]` resolver run (SMOOTH_V's predictor reads `AboveRow[j]` and the
// bottom-left sentinel, not the top-right one, but the resolver path is
// exercised). Smooth prediction is linear interpolation over those edges (no
// IDIF / edge-filter synthesis), so the non-flat real above-row edge
// reconstructs bit-exact. The prior brick gated SMOOTH luma neighbour decode to
// the first superblock row and rejected this frame with
// `general_intra_multirow_neighbour_non_dc`. avmdec and dav2d agree on the
// decoded output (md5 136a87190eeecb1ccd32e7cf27861c9c); the first general-intra
// 2-D grid non-DC luma decode reading a real reconstructed above row.
const VGRID_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-vgrid-intra-192x128-q120.ivf");

#[test]
fn vgrid_multirow_smooth_v_above_row_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(VGRID_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(192, 128).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(96, 64).unwrap()
    );

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 192 + c];

    // The middle superblock column (x in 64..128) is SMOOTH_V: constant across
    // columns within a row.
    assert!(
        (64..128).all(|c| at(80, c) == at(80, 64)),
        "middle superblock column must be constant across columns within a row (SMOOTH_V)"
    );
    // The row > 0 (bottom) middle superblock read the REAL reconstructed above
    // row: its top row (row 64) continues the gradient from the bottom row of
    // the above superblock (row 63) rather than jumping toward the §7.13.2.1
    // no-above flat fallback (127). The samples straddle a small monotone step.
    assert!(
        at(63, 96) < at(64, 96) && at(64, 96) < at(65, 96),
        "bottom middle superblock top must continue the above superblock gradient (real above row read)"
    );
    // A flat no-above fallback (≈127) at the SB boundary would break the
    // monotone gradient; the reconstructed boundary samples sit well below 127
    // at the top of the frame and rise monotonically toward 230 at the bottom.
    assert!(
        at(64, 96) < 140,
        "bottom superblock top must read the real (low) above row, not the 127 fallback"
    );
    assert!(
        at(0, 96) < at(127, 96),
        "middle superblock column increases top-to-bottom across both superblock rows"
    );
    // Each superblock column reads its OWN real neighbour edge, so the bottom
    // superblock's reconstruction differs by column (the per-column tint).
    assert_ne!(
        at(64, 32),
        at(64, 96),
        "left (DC) and middle (SMOOTH_V) bottom superblocks must reconstruct distinct values"
    );
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    // Chroma is flat (U=120, V=130) under DC / SMOOTH over a flat plane.
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's
    // raw output byte-for-byte (verified locally; md5
    // 136a87190eeecb1ccd32e7cf27861c9c).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "c62dd0eb74ab1129e9cd4d6a326cfef9026f62ab4144a378b38cb325b45462d2"
    );
}

// A 128x64 frame (two 64x64 superblocks) whose LEFT superblock codes
// `SMOOTH_V_PRED` (§ 7.13.2.13) luma with DC chroma, and whose RIGHT superblock
// codes the § 7.13.2.8 D135_PRED directional luma mode (`uv_mode == 0`
// directional-follow D135 chroma) reading a **real reconstructed** neighbour edge.
//
// The RIGHT block is the first general-intra **neighbour-having directional**
// luma+chroma decode. Its § 5.20.5.3 `y_mode_offset` escape was decoded with a
// non-directional left neighbour (the SMOOTH_V left superblock stored
// `IntraJointMode == 2 < NON_DIRECTIONAL_MODES_COUNT`), so its § 8.3.2 context is
// `0` — the same supported escape path the top-left D135 uses — not the deferred
// directional-neighbour reorder. At `frontier.r == 0`, `haveAbove == 0`, so
// § 7.13.2.1 fills `AboveRow` with the repeated first left sample and `LeftCol`
// with the **real reconstructed left column** of the already-decoded left
// superblock (a non-flat vertical gradient, 34 distinct values). pAngle 135 has
// `dx == dy == Dr_Intra_Derivative[45] == 64`, so every § 7.13.2.8 projection has
// `shift == 0`: the luma IDIF 4-tap (`enableIdif == 1`) collapses to
// `Dr_Interp_Filter[0] == {0, 128, 0, 0}` → `Edge[base]`, bit-identical to the
// chroma bilinear branch (`enableIdif == 0`) over the **non-flat** edge, so the
// shared bilinear middle-angle predictor is bit-exact for D135 in both planes. The
// prior brick rejected this frame (`general_intra_multiblock_directional_luma` for
// luma, `general_intra_directional_chroma_neighbour` for chroma). avmdec and dav2d
// agree on the decoded output (md5 9ff7e4d46c0dd4fa979070ce4ca4dd1c) and splot
// reproduces it byte-for-byte.
const RDIR_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-rdir-intra-128x64-q80.ivf");

#[test]
fn rdir_neighbour_directional_d135_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frame = decode_general_intra_luma(RDIR_FIXTURE);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    // LEFT superblock: SMOOTH_V — constant across columns within a row, increasing
    // top-to-bottom (the non-DC prediction plus AC residual over the real left edge).
    assert!(
        (0..64).all(|c| at(20, c) == at(20, 0)),
        "left superblock must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(0, 0) < at(63, 0),
        "left superblock increases top-to-bottom"
    );
    // RIGHT superblock: D135 — a genuine 135-degree directional pattern (varies
    // across columns within its top row, not flat/row-constant) reconstructed over
    // the real left-neighbour edge plus residual.
    assert!(
        (1..64).any(|c| at(0, 64 + c) != at(0, 64)),
        "right superblock top row must vary across columns (directional D135, not flat)"
    );
    let distinct = y.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert!(distinct > 4, "luma should be a non-flat reconstruction");
    // Chroma is flat (U=120, V=130). The right superblock's chroma still runs the
    // **neighbour-having** directional-follow D135 path
    // (`reconstruct_general_intra_directional_neighbour_block_into`, chroma_x == 32
    // so x > 0), reading the real reconstructed left chroma column via the
    // §7.13.2.8 bilinear branch; over the uniform left chroma edge the D135 sample
    // copy plus the all-zero chroma residual reconstructs flat — the path is
    // exercised, and bit-exactness against the oracle confirms correctness.
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    // Frame hash pins splot's output, which reproduces avmdec's and dav2d's raw
    // output byte-for-byte (verified locally; md5 9ff7e4d46c0dd4fa979070ce4ca4dd1c).
    let hash = splot_recon::DecodedFrameHashInput::new(&frame)
        .compute_hash()
        .to_hex();
    assert_eq!(
        hash,
        "9ea9254abc7d7507558099d5ae3e78eaf5d88625e1cc8184038321650b2b54a4"
    );
}
