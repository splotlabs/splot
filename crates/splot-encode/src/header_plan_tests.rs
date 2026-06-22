// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height).unwrap()
}

fn info(
    visible_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
) -> FrameInfo {
    FrameInfo::new(
        FrameId::new(3),
        visible_luma_size,
        bit_depth,
        chroma_subsampling,
    )
}

fn config(
    width: u32,
    height: u32,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
) -> EncoderConfig {
    EncoderConfig {
        width,
        height,
        bit_depth,
        chroma_subsampling,
        qp: crate::config::DEFAULT_QP,
    }
}

#[test]
fn valid_current_subset_header_plan_is_deterministic() {
    let config = EncoderConfig::new(4, 4);
    let frame_info = FrameInfo::yuv420_8bit(FrameId::new(3), size(4, 4));

    let plan = MinimalHeaderPlan::new(&config, frame_info).unwrap();
    let plan_again = MinimalHeaderPlan::new(&config, frame_info).unwrap();

    assert_eq!(plan, plan_again);
    assert_eq!(plan.sequence().coded_luma_size(), size(4, 4));
    assert_eq!(plan.sequence().bit_depth(), BitDepth::Eight);
    assert_eq!(
        plan.sequence().chroma_subsampling(),
        ChromaSubsampling::Yuv420
    );
    assert_eq!(plan.frame().source_frame(), FrameId::new(3));
    assert_eq!(plan.frame().visible_luma_size(), size(4, 4));
    assert_eq!(plan.frame().bit_depth(), BitDepth::Eight);
    assert_eq!(plan.frame().chroma_subsampling(), ChromaSubsampling::Yuv420);
    assert_eq!(plan.frame().kind(), FrameHeaderIntentKind::FirstFrame);
    assert_eq!(plan.tile_group().first_tile(), TileIndex::new(0));
    assert_eq!(plan.tile_group().last_tile(), TileIndex::new(0));
    assert!(plan.tile_group().is_first_tile_group());
}

#[test]
fn valid_header_plan_has_stable_debug_shape() {
    let plan = MinimalHeaderPlan::new(
        &EncoderConfig::new(4, 4),
        FrameInfo::yuv420_8bit(FrameId::new(3), size(4, 4)),
    )
    .unwrap();

    assert_eq!(
        format!("{plan:?}"),
        "MinimalHeaderPlan { sequence: SequenceHeaderIntent { coded_luma_size: PlaneSize { width: 4, height: 4 }, bit_depth: Eight, chroma_subsampling: Yuv420 }, frame: FrameHeaderIntent { source_frame: FrameId(3), visible_luma_size: PlaneSize { width: 4, height: 4 }, bit_depth: Eight, chroma_subsampling: Yuv420, kind: FirstFrame }, tile_group: TileGroupHeaderIntent { first_tile: TileIndex(0), last_tile: TileIndex(0), is_first_tile_group: true } }"
    );
}

#[test]
fn zero_config_dimensions_are_rejected_before_plan_construction() {
    let frame_info = FrameInfo::yuv420_8bit(FrameId::new(3), size(4, 4));

    assert!(matches!(
        MinimalHeaderPlan::new(&EncoderConfig::new(0, 4), frame_info),
        Err(HeaderPlanError::ZeroDimension { field: "width" })
    ));
    assert!(matches!(
        MinimalHeaderPlan::new(&EncoderConfig::new(4, 0), frame_info),
        Err(HeaderPlanError::ZeroDimension { field: "height" })
    ));
}

#[test]
fn unsupported_config_formats_are_rejected() {
    let ten_bit = config(4, 4, BitDepth::Ten, ChromaSubsampling::Yuv420);
    assert!(matches!(
        MinimalHeaderPlan::new(
            &ten_bit,
            info(size(4, 4), BitDepth::Ten, ChromaSubsampling::Yuv420),
        ),
        Err(HeaderPlanError::UnsupportedBitDepth {
            bit_depth: BitDepth::Ten
        })
    ));

    let yuv444 = config(4, 4, BitDepth::Eight, ChromaSubsampling::Yuv444);
    assert!(matches!(
        MinimalHeaderPlan::new(
            &yuv444,
            info(size(4, 4), BitDepth::Eight, ChromaSubsampling::Yuv444),
        ),
        Err(HeaderPlanError::UnsupportedChromaSubsampling {
            chroma_subsampling: ChromaSubsampling::Yuv444
        })
    ));
}

#[test]
fn frame_metadata_mismatches_are_rejected() {
    let config = EncoderConfig::new(4, 4);

    assert!(matches!(
        MinimalHeaderPlan::new(
            &config,
            FrameInfo::yuv420_8bit(FrameId::new(3), size(2, 4)),
        ),
        Err(HeaderPlanError::FrameSizeMismatch {
            frame,
            expected,
            actual,
        }) if frame == FrameId::new(3) && expected == size(4, 4) && actual == size(2, 4)
    ));
    assert!(matches!(
        MinimalHeaderPlan::new(
            &config,
            info(size(4, 4), BitDepth::Ten, ChromaSubsampling::Yuv420),
        ),
        Err(HeaderPlanError::FrameBitDepthMismatch {
            frame,
            expected: BitDepth::Eight,
            actual: BitDepth::Ten,
        }) if frame == FrameId::new(3)
    ));
    assert!(matches!(
        MinimalHeaderPlan::new(
            &config,
            info(size(4, 4), BitDepth::Eight, ChromaSubsampling::Yuv444),
        ),
        Err(HeaderPlanError::FrameChromaSubsamplingMismatch {
            frame,
            expected: ChromaSubsampling::Yuv420,
            actual: ChromaSubsampling::Yuv444,
        }) if frame == FrameId::new(3)
    ));
}

#[test]
fn non_minimal_tile_group_shapes_are_rejected() {
    assert!(matches!(
        TileGroupHeaderIntent::new(TileIndex::new(0), TileIndex::new(0), false),
        Err(HeaderPlanError::UnsupportedTileGroupContinuation)
    ));
    assert!(matches!(
        TileGroupHeaderIntent::new(TileIndex::new(0), TileIndex::new(1), true),
        Err(HeaderPlanError::UnsupportedTileRange {
            first_tile,
            last_tile,
        }) if first_tile == TileIndex::new(0) && last_tile == TileIndex::new(1)
    ));
}
