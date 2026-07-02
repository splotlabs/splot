// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra decode tests for the minimal runtime.

use std::{collections::BTreeSet, fmt::Debug};

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize, ReconSample};

use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

const Q80_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");

const Q80_LUMA: u8 = 100;
const Q80_CHROMA_U: u8 = 120;
const Q80_CHROMA_V: u8 = 130;

const Q80_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-10bit-q80.ivf"
);

const Q180_COS_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-cos-intra-64x64-10bit-q180.ivf"
);

const TWO_SB_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2sb-intra-128x64-10bit-q80.ivf"
);

const Q160_SMCHROMA_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-smchroma-intra-64x64-10bit-q160.ivf"
);

const SMCHROMA_2SB_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2sb-smchroma-intra-128x64-10bit-q160.ivf"
);

const Q80_10BIT_LUMA: u16 = 400;
const Q80_10BIT_CHROMA_U: u16 = 480;
const Q80_10BIT_CHROMA_V: u16 = 520;

const Q180_COS_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-cos-intra-64x64-q180.ivf");

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

fn decode_context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context")
}

fn decode_eight(fixture: &[u8]) -> DecodedFrame<u8> {
    let options = DecodeOptions::default();
    let context = decode_context();
    let plan = context.plan_bytes(fixture, options).expect("plan");
    context
        .pool()
        .install(|| decode_minimal_frame_from_plan(fixture, &options, &plan))
        .expect("decode")
        .into_frame_eight()
}

fn decode_general_intra_luma(fixture: &[u8]) -> DecodedFrame<u8> {
    decode_eight(fixture)
}

fn decode_ten(fixture: &[u8]) -> DecodedFrame<u16> {
    let options = DecodeOptions::default();
    let context = decode_context();
    let plan = context.plan_bytes(fixture, options).expect("plan");
    context
        .pool()
        .install(|| decode_minimal_frame_from_plan(fixture, &options, &plan))
        .expect("decode")
        .into_frame_ten()
}

fn assert_yuv420_frame<T: ReconSample>(
    frame: &DecodedFrame<T>,
    bit_depth: BitDepth,
    width: usize,
    height: usize,
) {
    assert_eq!(frame.bit_depth(), bit_depth);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(
        frame.y().visible_size(),
        PlaneSize::new(width, height).unwrap()
    );
}

fn assert_chroma_size<T: ReconSample>(frame: &DecodedFrame<T>, width: usize, height: usize) {
    let size = PlaneSize::new(width, height).unwrap();
    assert_eq!(frame.u().unwrap().visible_size(), size);
    assert_eq!(frame.v().unwrap().visible_size(), size);
}

fn frame_hash<T: ReconSample>(frame: &DecodedFrame<T>) -> String {
    DecodedFrameHashInput::new(frame).compute_hash().to_hex()
}

fn assert_hash<T: ReconSample>(frame: &DecodedFrame<T>, expected: &str) {
    assert_eq!(frame_hash(frame), expected);
}

fn assert_all_samples_eq<T>(samples: &[T], expected: T, label: &str)
where
    T: Copy + Debug + PartialEq,
{
    let preview_len = samples.len().min(8);
    assert!(
        samples.iter().all(|&s| s == expected),
        "{label} expected {expected:?}; first samples: {:?}",
        &samples[..preview_len]
    );
}

fn assert_chroma_eq<T>(frame: &DecodedFrame<T>, u: T, v: T)
where
    T: Copy + Debug + PartialEq + ReconSample,
{
    assert_all_samples_eq(frame.u().unwrap().samples(), u, "U");
    assert_all_samples_eq(frame.v().unwrap().samples(), v, "V");
}

fn distinct_count<T: Ord>(samples: &[T]) -> usize {
    samples.iter().collect::<BTreeSet<_>>().len()
}

fn assert_distinct_gt<T: Ord>(samples: &[T], min: usize, label: &str) {
    let distinct = distinct_count(samples);
    assert!(distinct > min, "{label}; distinct={distinct}");
}

fn single_frame_ivf_from_first(stream: &[u8]) -> Vec<u8> {
    const IVF_HEADER_LEN: usize = 32;
    const IVF_FRAME_HEADER_LEN: usize = 12;
    const FRAME_COUNT_OFFSET: usize = 24;

    let size0 = u32::from_le_bytes([
        stream[IVF_HEADER_LEN],
        stream[IVF_HEADER_LEN + 1],
        stream[IVF_HEADER_LEN + 2],
        stream[IVF_HEADER_LEN + 3],
    ]) as usize;
    let mut out = stream[..IVF_HEADER_LEN].to_vec();
    out[FRAME_COUNT_OFFSET..FRAME_COUNT_OFFSET + 4].copy_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&stream[IVF_HEADER_LEN..IVF_HEADER_LEN + IVF_FRAME_HEADER_LEN + size0]);
    out
}

#[test]
fn two_frame_key_frame_reconstructs_flat_h_pred_chroma() {
    let key_stream = single_frame_ivf_from_first(TWO_FRAME_INTER_FIXTURE);
    let frame = decode_eight(&key_stream);

    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);
    assert_all_samples_eq(frame.y().samples(), Q80_LUMA, "luma");
    assert_chroma_eq(&frame, Q80_CHROMA_U, Q80_CHROMA_V);
}

#[test]
fn q80_intra_frame_reconstructs_flat_planes() {
    let frame = decode_eight(Q80_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);
    assert_all_samples_eq(frame.y().samples(), Q80_LUMA, "luma");
    assert_chroma_eq(&frame, Q80_CHROMA_U, Q80_CHROMA_V);
}

#[test]
fn q80_intra_frame_hash_is_stable() {
    let frame = decode_eight(Q80_FIXTURE);
    assert_hash(
        &frame,
        "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
    );
}

#[test]
fn q80_10bit_intra_frame_reconstructs_flat_planes() {
    let frame = decode_ten(Q80_10BIT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Ten, 64, 64);
    assert_chroma_size(&frame, 32, 32);
    assert_all_samples_eq(frame.y().samples(), Q80_10BIT_LUMA, "10-bit luma");
    assert_chroma_eq(&frame, Q80_10BIT_CHROMA_U, Q80_10BIT_CHROMA_V);
}

#[test]
fn q80_10bit_intra_frame_hash_is_stable() {
    let frame = decode_ten(Q80_10BIT_FIXTURE);
    assert_hash(
        &frame,
        "973eb3fc4b112c865f939dc1339824ca0b2a1522ca2b5ec70311afb459436e2d",
    );
}

#[test]
fn q180_cos_10bit_intra_frame_decodes_ac_residual_luma() {
    let frame = decode_ten(Q180_COS_10BIT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Ten, 64, 64);

    let y = frame.y().samples();
    assert_distinct_gt(y, 4, "10-bit luma non-flat AC reconstruction");
    assert_hash(
        &frame,
        "bfec72ffcddf982499eebfa21bdfb400fc66aa96b40281298387420ef2124649",
    );
}

#[test]
fn two_superblock_10bit_intra_frame_decodes_to_oracle() {
    let frame = decode_ten(TWO_SB_10BIT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Ten, 128, 64);

    let y = frame.y().samples();
    assert_eq!(y[0], 400, "left-superblock luma must be 400");
    assert_eq!(y[64], 460, "right-superblock luma (column 64) must be 460");
    assert_hash(
        &frame,
        "ceff974fde25c8d05c9010d2a7f414845dc3a626ab3c45a9dabb08634c29dd66",
    );
}

#[test]
fn ten_bit_dc_luma_smooth_chroma_decodes_to_oracle() {
    let frame = decode_ten(Q160_SMCHROMA_10BIT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Ten, 64, 64);
    assert_hash(
        &frame,
        "4fe932e5e5dea4a1830eae4853b198c738e8d1919049736d2f4a234c491d5397",
    );
}

const SMOOTH_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-smooth-intra-64x64-10bit-q80.ivf"
);

const SPLIT_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-split-intra-64x64-10bit-q110.ivf"
);

const FLAT_Q255_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-10bit-q255.ivf"
);

fn assert_decode_rejects(fixture: &[u8], reason: &str) {
    use crate::error::DecodeError;

    let options = DecodeOptions::default();
    let context = decode_context();
    let plan = context.plan_bytes(fixture, options).expect("plan");
    match context
        .pool()
        .install(|| decode_minimal_frame_from_plan(fixture, &options, &plan))
    {
        Ok(_) => panic!("expected an unsupported-feature rejection for reason {reason}, decoded"),
        Err(DecodeError::UnsupportedFeature { unsupported }) => {
            assert_eq!(unsupported.reason(), reason);
        }
        Err(other) => panic!("expected an unsupported-feature rejection, got {other:?}"),
    }
}

#[test]
fn ten_bit_smooth_luma_fails_closed_non_dc() {
    assert_decode_rejects(
        SMOOTH_10BIT_FIXTURE,
        "general_intra_transform_tool_residual",
    );
}

#[test]
fn ten_bit_split_leaf_reaches_transform_tool_residual_frontier() {
    assert_decode_rejects(SPLIT_10BIT_FIXTURE, "general_intra_transform_tool_residual");
}

#[test]
fn ten_bit_base_q255_fails_closed_frozen_tier() {
    assert_decode_rejects(
        FLAT_Q255_10BIT_FIXTURE,
        "unsupported_10bit_frozen_minimal_tier",
    );
}

#[test]
fn ten_bit_multi_sb_smooth_chroma_fails_closed_non_dc() {
    assert_decode_rejects(SMCHROMA_2SB_10BIT_FIXTURE, "unsupported_10bit_non_dc_intra");
}

#[test]
fn q180_cos_intra_frame_decodes_multi_coefficient_luma() {
    let frame = decode_eight(Q180_COS_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert_distinct_gt(y, 4, "luma non-flat AC reconstruction");
    assert_hash(
        &frame,
        "8a6751d4517073bad0bbe71f4b5537df8e8b0bfee85fcd6af1ac2d5878dd59e8",
    );
}

const QUAD_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-quad-intra-64x64-q80.ivf");

#[test]
fn quad_multiblock_intra_frame_reaches_transform_tool_residual_frontier() {
    assert_decode_rejects(QUAD_FIXTURE, "general_intra_transform_tool_residual");
}

const DEEP_SPLIT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-deep-intra-64x64-q120.ivf");

#[test]
fn deep_split_sub_32x32_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(DEEP_SPLIT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    let at = |col: usize, row: usize| y[row * 64 + col];
    assert_eq!(
        (at(8, 8), at(24, 8), at(8, 24), at(24, 24)),
        (240, 21, 21, 240)
    );
    assert_eq!((at(48, 16), at(16, 48), at(48, 48)), (130, 70, 200));

    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == 120 || s == 121)
    );
    assert_all_samples_eq(frame.v().unwrap().samples(), 130, "V");
    assert_hash(
        &frame,
        "73123e51c66787b59fb6b93a6221e9d78a550c6e0d1c4e0c1adfd21a41ed39ab",
    );
}

const TWO_SB_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-2sb-intra-128x64-q80.ivf");

#[test]
fn two_superblock_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(TWO_SB_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    assert!(
        (0..64).all(|r| (0..64).all(|c| y[r * 128 + c] == 80)),
        "left superblock luma must be flat 80"
    );
    assert!(
        (0..64).all(|r| (64..128).all(|c| y[r * 128 + c] == 180)),
        "right superblock luma must be flat 180"
    );
    assert_chroma_eq(&frame, 120, 130);
    assert_hash(
        &frame,
        "18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f",
    );
}

const COL_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-2sbcol-intra-64x128-q80.ivf");

#[test]
fn multi_row_superblock_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(COL_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 128);
    assert_chroma_size(&frame, 32, 64);

    let y = frame.y().samples();
    assert!(
        (0..64).all(|r| (0..64).all(|c| y[r * 64 + c] == 80)),
        "top superblock luma must be flat 80"
    );
    assert!(
        (64..128).all(|r| (0..64).all(|c| y[r * 64 + c] == 180)),
        "bottom superblock luma must be flat 180"
    );
    assert_chroma_eq(&frame, 120, 130);
    assert_hash(
        &frame,
        "3ee739a805e13597ff7d75659dd1e0150113bf4782c4d69e1d27ae942d6c10a0",
    );
}

const GRID_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-grid-intra-128x128-q80.ivf");

#[test]
fn grid_2d_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(GRID_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 128);
    assert_chroma_size(&frame, 64, 64);

    assert_all_samples_eq(frame.y().samples(), 100, "luma");

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

    let u_bl = quad(u, 1, 0);
    let v_bl = quad(v, 1, 0);
    assert_distinct_gt(&u_bl, 1, "U bottom-left SMOOTH gradient");
    assert_distinct_gt(&v_bl, 1, "V bottom-left SMOOTH gradient");
    assert_eq!(
        u_bl[0], 110,
        "U bottom-left top-left corner == own top edge"
    );
    assert!(
        u_bl[31] > u_bl[0],
        "U bottom-left top-right corner must be pulled toward the above-right (200), proving the above-right read"
    );
    assert_hash(
        &frame,
        "42bd99faae1ac0acb15c3e24fbededd8fc670612d08987bebb8942de5f4f4874",
    );
}

const VSMOOTH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-vsmooth-intra-64x64-q120.ivf");
const HSMOOTH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hsmooth-intra-64x64-q120.ivf");
const SMOOTH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-smooth-intra-64x64-q124.ivf");

#[test]
fn vsmooth_single_block_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(VSMOOTH_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert!(
        y[0..64].iter().all(|&s| s == y[0]),
        "top row should be constant across columns"
    );
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "3aebe2eb215d4878bbc40aa2f97e2178b6140ef51c03afaaae478e69dbbf6bcd",
    );
}

#[test]
fn hsmooth_single_block_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(HSMOOTH_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert!(
        (0..64).all(|r| y[r * 64] == y[0]),
        "left column should be constant across rows"
    );
    assert!(y[0] < y[63], "luma should increase left-to-right");
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "cfc6debd26760cdebf1d1a4497792461f0f68bc7e7773741ddf2cbc34561e702",
    );
}

#[test]
fn smooth_single_block_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(SMOOTH_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert!(
        !y[0..64].iter().all(|&s| s == y[0]),
        "plain SMOOTH must vary along the top row (not SMOOTH_V)"
    );
    assert!(
        !(0..64).all(|r| y[r * 64] == y[0]),
        "plain SMOOTH must vary down the left column (not SMOOTH_H)"
    );
    assert!(
        y[0] < y[63],
        "luma should increase left-to-right along the top row"
    );
    assert!(
        y[0] < y[63 * 64],
        "luma should increase top-to-bottom along the left column"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat 2-D reconstruction");

    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    assert!(
        u.iter().any(|&s| s != u[0]),
        "chroma U carries a DC-mode residual"
    );
    assert!(
        v.iter().any(|&s| s != v[0]),
        "chroma V carries a DC-mode residual"
    );

    assert_hash(
        &frame,
        "9b054c6fff47397fbe88a9eb45a34fac018efc7748fc697edebddd3f14bd88d3",
    );
}

const SMOOTH_NONDC_CHROMA_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-smoothnondc-intra-64x64-q132.ivf"
);

#[test]
fn smooth_luma_with_non_dc_h_pred_chroma_decodes_to_oracle() {
    let frame = decode_eight(SMOOTH_NONDC_CHROMA_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    assert_hash(
        &frame,
        "f1621607dfcd2737e8a4c308fc26cd1596cb001444437f0440e34883a59b519b",
    );
}

const SHSPLIT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-shsplit-intra-64x64-q80.ivf");

#[test]
fn smooth_h_split_subblock_reaches_transform_tool_residual_frontier() {
    assert_decode_rejects(SHSPLIT_FIXTURE, "general_intra_transform_tool_residual");
}

const SVSPLIT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-svsplit-intra-64x64-q140.ivf");

#[test]
fn smooth_chroma_split_subblock_still_rejects() {
    assert_decode_rejects(SVSPLIT_FIXTURE, "general_intra_transform_tool_residual");
}

const HEDGE_DIR_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hedge-intra-64x64-q80.ivf");

#[test]
fn hedge_directional_d135_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(HEDGE_DIR_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "b15f267ec6e99ca4d96a70f38bffe5f798ee4c33ad3aaec23761a1ea74b0be33",
    );
}

const DFCHROMA_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-dfchroma-intra-64x64-q80.ivf");

#[test]
fn dfchroma_directional_follow_chroma_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(DFCHROMA_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);
    assert_chroma_size(&frame, 32, 32);

    let y = frame.y().samples();
    assert!(y[0] < y[63 * 64], "luma should increase top-to-bottom");

    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    let at = |p: &[u8], r: usize, c: usize| p[r * 32 + c];
    assert_distinct_gt(u, 4, "U chroma non-flat directional reconstruction");
    assert_distinct_gt(v, 4, "V chroma non-flat directional reconstruction");
    assert!(
        at(u, 2, 28) < at(u, 28, 2),
        "U upper-right (c>r) must be below lower-left (r>c) for D135"
    );
    assert!(
        at(v, 2, 28) < at(v, 28, 2),
        "V upper-right (c>r) must be below lower-left (r>c) for D135"
    );

    assert_hash(
        &frame,
        "628b759dcb63356ad3174063652c54d7ebf6f54d1566ab9f1b64b3a74542154f",
    );
}

const MBVG_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mbvg-intra-128x64-q80.ivf");

#[test]
fn mbvg_multiblock_smooth_v_neighbour_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(MBVG_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
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
    assert_ne!(
        at(32, 32),
        at(32, 96),
        "right superblock must read the real neighbour edge, not duplicate the left"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "269b4969800751c63f7f0605f1f7b8f178f7bf85590ec62fe64313ff394d6dfd",
    );
}

const DIRNEIGH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-dirneigh-intra-128x64-q80.ivf");

#[test]
fn dirneigh_directional_neighbour_ctx_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(DIRNEIGH_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        (1..64).any(|c| at(0, c) != at(0, 0)),
        "left superblock top row must vary across columns (directional, not flat)"
    );
    assert!(at(0, 0) < at(63, 0), "left darkens top-to-bottom");
    assert!(
        (64..128).all(|c| at(20, c) == at(20, 64)),
        "right superblock must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(0, 64) < at(63, 64),
        "right increases top-to-bottom (SMOOTH_V)"
    );
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "ad1515885df5620a31c37f855934ae2432167edbf1b1b62081552b9df3957426",
    );
}

const VGRID_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-vgrid-intra-192x128-q120.ivf");

#[test]
fn vgrid_multirow_smooth_v_above_row_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(VGRID_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 192, 128);
    assert_chroma_size(&frame, 96, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 192 + c];

    assert!(
        (64..128).all(|c| at(80, c) == at(80, 64)),
        "middle superblock column must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(63, 96) < at(64, 96) && at(64, 96) < at(65, 96),
        "bottom middle superblock top must continue the above superblock gradient (real above row read)"
    );
    assert!(
        at(64, 96) < 140,
        "bottom superblock top must read the real (low) above row, not the 127 fallback"
    );
    assert!(
        at(0, 96) < at(127, 96),
        "middle superblock column increases top-to-bottom across both superblock rows"
    );
    assert_ne!(
        at(64, 32),
        at(64, 96),
        "left (DC) and middle (SMOOTH_V) bottom superblocks must reconstruct distinct values"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "c62dd0eb74ab1129e9cd4d6a326cfef9026f62ab4144a378b38cb325b45462d2",
    );
}

const SHGRID_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-shgrid-intra-128x128-q80.ivf");

#[test]
fn shgrid_multirow_smooth_h_above_right_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(SHGRID_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 128);
    assert_chroma_size(&frame, 64, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];

    assert!(
        (64..128).all(|r| at(r, 0) == at(64, 0)),
        "SMOOTH_H bottom-left superblock must be constant down column 0"
    );
    assert!(
        (64..128).all(|r| at(r, 32) == at(64, 32)),
        "SMOOTH_H bottom-left superblock must be constant down column 32"
    );
    assert!(
        at(96, 0) < at(96, 16) && at(96, 16) < at(96, 32) && at(96, 32) < at(96, 48),
        "SMOOTH_H bottom-left superblock must increase left-to-right (horizontal gradient)"
    );
    assert_eq!(
        at(63, 64),
        200,
        "top-right superblock bottom row (the above-right source) must reconstruct to 200"
    );
    assert!(
        at(96, 63) > 200,
        "SMOOTH_H rightmost column must blend toward the real above-right sentinel (200), not the clamp (100)"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");

    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    let cat = |p: &[u8], r: usize, c: usize| p[r * 64 + c];
    assert!(
        (32..64).all(|r| (0..32).all(|c| cat(u, r, c) == cat(u, 32, 0))),
        "bottom-left superblock U chroma must be a single DC value"
    );
    assert!(
        (32..64).all(|r| (0..32).all(|c| cat(v, r, c) == cat(v, 32, 0))),
        "bottom-left superblock V chroma must be a single DC value"
    );

    assert_hash(
        &frame,
        "d1ce39cc3d79f5c46fdea67ad57ec4edd5dfed088ee39fd7029fda1bbb11e0e8",
    );
}

const RDIR_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-rdir-intra-128x64-q80.ivf");

#[test]
fn rdir_neighbour_directional_d135_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(RDIR_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        (0..64).all(|c| at(20, c) == at(20, 0)),
        "left superblock must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(0, 0) < at(63, 0),
        "left superblock increases top-to-bottom"
    );
    assert!(
        (1..64).any(|c| at(0, 64 + c) != at(0, 64)),
        "right superblock top row must vary across columns (directional D135, not flat)"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "9ea9254abc7d7507558099d5ae3e78eaf5d88625e1cc8184038321650b2b54a4",
    );
}

const VPRED_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-vpred-intra-64x128-q160.ivf");

#[test]
fn vpred_cardinal_multirow_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(VPRED_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 128);
    assert_chroma_size(&frame, 32, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 64 + c];

    for c in (0..64).step_by(8) {
        assert!(
            (64..128).all(|r| at(r, c) == at(64, c)),
            "bottom superblock column {c} must be constant down the block (V_PRED vertical copy)"
        );
    }
    assert!(
        at(64, 0) < at(64, 63),
        "bottom superblock must vary across columns (V_PRED copies the column-varying above row), not flat DC"
    );
    assert!(
        at(64, 0).abs_diff(at(63, 0)) < 16,
        "bottom superblock top row must continue the real above row, not jump to the 127 fallback"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    assert!(u.iter().all(|&s| s == u[0]), "chroma U must be uniform");
    assert!(v.iter().all(|&s| s == v[0]), "chroma V must be uniform");

    assert_hash(
        &frame,
        "5b2761c0d2eb2502af5cbe544b2cadbb676a4b84b60953d86a3e42d7df910e39",
    );
}

const HPRED_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hpred-intra-128x64-q180.ivf");

#[test]
fn hpred_cardinal_multicolumn_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(HPRED_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];

    for r in (0..64).step_by(8) {
        assert!(
            (64..128).all(|c| at(r, c) == at(r, 64)),
            "right superblock row {r} must be constant across columns (H_PRED horizontal copy)"
        );
    }
    assert!(
        at(0, 64) < at(63, 64),
        "right superblock must vary down rows (H_PRED copies the row-varying left column), not flat DC"
    );
    assert!(
        at(0, 64).abs_diff(at(0, 63)) < 16,
        "right superblock left column must continue the real left neighbour, not jump to a fallback"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    let u = frame.u().unwrap().samples();
    let v = frame.v().unwrap().samples();
    assert!(u.iter().all(|&s| s == u[0]), "chroma U must be uniform");
    assert!(v.iter().all(|&s| s == v[0]), "chroma V must be uniform");

    assert_hash(
        &frame,
        "826cea4e59f8280b538c3efc26e7be72cd1912aa19f235ebf3f862fc8832a885",
    );
}

const D157_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-d157-intra-128x64-q80.ivf");

#[test]
fn d157_neighbour_directional_idif_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(D157_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        (0..64).all(|c| at(20, c) == at(20, 0)),
        "left superblock must be constant across columns within a row (SMOOTH_V)"
    );
    assert!(
        at(0, 0) < at(63, 0),
        "left superblock increases top-to-bottom"
    );
    assert!(
        (1..64).any(|c| at(63, 64 + c) != at(63, 64)),
        "right superblock bottom row must vary across columns (directional D157, not flat / H_PRED)"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "bf93ca6b8f55e1fb7db2584f3e3821ad67f21018b774c6e326634362ee5ef046",
    );
}

const D135ROW_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-d135row-intra-128x128-q80.ivf");

#[test]
fn d135row_neighbour_directional_row_gt0_corner_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(D135ROW_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 128);
    assert_chroma_size(&frame, 64, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        (0..64).all(|r| (0..128).all(|c| at(r, c) == 100)),
        "top superblock row must reconstruct flat 100 (DC)"
    );
    for k in 0..64 {
        assert_eq!(
            at(64 + k, 64 + k),
            100,
            "bottom-right D135 main diagonal must copy the real corner (100)"
        );
    }
    assert!(
        at(127, 64) != 100,
        "bottom-right block left-branch must propagate the real left column (not flat DC)"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "85583e5a46ac6a2db97854b86f643735c1b9710bee2c2d2bc65d1aa5a16fe3a1",
    );
}

const D113_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-d113-intra-128x128-q80.ivf");

#[test]
fn d113_neighbour_directional_idif_above_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(D113_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 128);
    assert_chroma_size(&frame, 64, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        (0..64).all(|r| (0..128).all(|c| at(r, c) == 100)),
        "top superblock row must reconstruct flat 100 (DC)"
    );
    assert_eq!(
        at(64, 64),
        100,
        "bottom-right D113 top-left sample must copy the real corner / flat above (100)"
    );
    assert!(
        (1..64).any(|c| at(127, 64 + c) != at(127, 64)),
        "bottom-right D113 bottom row must vary across columns (directional, not flat / H_PRED)"
    );
    assert!(
        at(127, 64) != 100,
        "bottom-right D113 left-branch must propagate the real left column (not flat DC)"
    );
    assert_distinct_gt(y, 4, "luma should be a non-flat reconstruction");
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "d32bc2b11585e7ea55f0d2401f18402c55e781c0a861bb613b55f5dc26a2a395",
    );
}

const D45_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-d45-intra-192x128-q80.ivf");

#[test]
fn d45_neighbour_one_sided_above_right_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(D45_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 192, 128);
    assert_chroma_size(&frame, 96, 64);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 192 + c];
    assert_eq!(
        at(64, 64),
        128,
        "D45 block top-left must copy the flat above-middle row (128)"
    );
    assert!(
        at(64, 127) != 128,
        "D45 block top-right must read the non-flat real above-right (not the flat above-middle 128)"
    );
    assert!(
        at(64, 127) < 100,
        "D45 block top-right must propagate the real above-right gradient low end (~42)"
    );
    let block: Vec<u8> = (0..64)
        .flat_map(|i| (0..64).map(move |j| at(64 + i, 64 + j)))
        .collect();
    assert_distinct_gt(&block, 4, "D45 block non-flat above-right reconstruction");

    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "d08056c0d1ed3f379e3072c7f1ebced04da0f6df994efd0b5f8d39b76c0b683f",
    );
}

const SKIP_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");

#[test]
fn luma_skip_fixture_decodes_skip_branch_through_general_path() {
    let frame = decode_eight(SKIP_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    assert!(
        y.iter().all(|&s| s == 128),
        "luma must be the flat 128 skip block; first samples: {:?}",
        &y[..8]
    );

    let u = frame.u().unwrap().samples();
    assert!(
        u.iter().any(|&s| s != u[0]),
        "U must carry a coded (non-flat) residual; first samples: {:?}",
        &u[..8]
    );

    assert_hash(
        &frame,
        "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af",
    );
}

const D203_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-d203-intra-128x64-q80.ivf");

#[test]
fn d203_neighbour_one_sided_left_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(D203_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
    assert_chroma_size(&frame, 64, 32);

    let y = frame.y().samples();
    let at = |r: usize, c: usize| y[r * 128 + c];
    assert!(
        at(0, 64) < 60,
        "D203 block top-left must read the real left column gradient low end (~31)"
    );
    assert!(
        at(0, 127) > at(0, 64) + 30,
        "D203 block top-right must project down-and-left into the lower left column (a higher gradient value than the top-left)"
    );
    let block: Vec<u8> = (0..64)
        .flat_map(|i| (0..64).map(move |j| at(i, 64 + j)))
        .collect();
    assert_distinct_gt(&block, 4, "D203 block non-flat left-column reconstruction");

    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 120));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 130));

    assert_hash(
        &frame,
        "3b95907f8808cc9d0bdd2eb376c8726019f7a4490cf8ecfcccab883fb11f8a3f",
    );
}

const HRECT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-hrect-intra-64x64-q120.ivf");

#[test]
fn horz_rectangular_partition_intra_frame_decodes_to_oracle() {
    let frame = decode_eight(HRECT_FIXTURE);
    assert_yuv420_frame(&frame, BitDepth::Eight, 64, 64);

    let y = frame.y().samples();
    let at = |col: usize, row: usize| y[row * 64 + col];
    assert_eq!((at(32, 16), at(32, 48)), (60, 200));
    assert!(frame.u().unwrap().samples().iter().all(|&s| s == 128));
    assert!(frame.v().unwrap().samples().iter().all(|&s| s == 128));

    assert_hash(
        &frame,
        "6d2e94d795d46cae62d1e2cf06cf4fe5b727b0917742745af998b002a7686142",
    );
}

const DEBLOCK_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q100.ivf"
);
const DEBLOCK_Q98_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q98.ivf"
);
const DEBLOCK_WIDE_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2sb-deblockwide-intra-128x64-q100.ivf"
);
const DEBLOCK_GRID_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-grid-deblock-intra-128x128-q100.ivf"
);

#[test]
fn deblock_active_intra_frame_reaches_transform_tool_residual_frontier() {
    assert_decode_rejects(
        DEBLOCK_Q100_FIXTURE,
        "general_intra_transform_tool_residual",
    );
    assert_decode_rejects(DEBLOCK_Q98_FIXTURE, "general_intra_transform_tool_residual");
    assert_decode_rejects(
        DEBLOCK_WIDE_Q100_FIXTURE,
        "general_intra_transform_tool_residual",
    );
    assert_decode_rejects(
        DEBLOCK_GRID_Q100_FIXTURE,
        "general_intra_transform_tool_residual",
    );
}

#[test]
fn deblock_off_frame_is_byte_identical() {
    let frame = decode_eight(TWO_SB_FIXTURE);
    assert_eq!(
        frame_hash(&frame),
        "18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f",
        "deblock-off fixture must stay byte-identical (the §7.17 pass is a no-op)"
    );
}

mod general_intra_cdef_tests;
