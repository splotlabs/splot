// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height).unwrap()
}

fn symbol(symbol_id: u16) -> PlanSymbolId {
    PlanSymbolId::new(symbol_id)
}

fn coefficient(index: u16, value: i32) -> (CoefficientIndex, i32) {
    (CoefficientIndex::new(index), value)
}

fn sample_block(index: u32) -> BlockDecision {
    let coefficients =
        QuantizedCoefficients::new(vec![coefficient(0, 7), coefficient(3, -2)]).unwrap();
    BlockDecision::new(
        BlockIndex::new(index),
        PredictionDecision::intra(symbol(1)),
        TransformDecision::new(size(4, 4), symbol(2)),
        coefficients,
    )
    .unwrap()
}

fn sample_tile(index: u32) -> TilePlan {
    let tile = TileIndex::new(index);
    let superblock = SuperBlockPlan::new(SuperBlockIndex::new(0), vec![sample_block(0)]).unwrap();
    let events = vec![
        SyntaxEvent::Sequence {
            index: SyntaxEventIndex::new(0),
            symbol: symbol(0),
        },
        SyntaxEvent::Frame {
            index: SyntaxEventIndex::new(1),
            frame: FrameId::new(0),
            symbol: symbol(1),
        },
        SyntaxEvent::Tile {
            index: SyntaxEventIndex::new(2),
            tile,
            symbol: symbol(2),
        },
        SyntaxEvent::Block {
            index: SyntaxEventIndex::new(3),
            tile,
            superblock: SuperBlockIndex::new(0),
            block: BlockIndex::new(0),
            symbol: symbol(3),
        },
        SyntaxEvent::Token {
            index: SyntaxEventIndex::new(4),
            tile,
            superblock: SuperBlockIndex::new(0),
            block: BlockIndex::new(0),
            coefficient: CoefficientIndex::new(3),
            symbol: symbol(4),
        },
    ];
    TilePlan::new(tile, vec![superblock], events).unwrap()
}

fn sample_frame(frame_id: u64) -> FramePlan {
    let info = FrameInfo::yuv420_8bit(FrameId::new(frame_id), size(16, 16));
    FramePlan::new(info, vec![sample_tile(0)]).unwrap()
}

fn sample_sequence() -> SequencePlan {
    SequencePlan::new(
        size(16, 16),
        BitDepth::Eight,
        ChromaSubsampling::Yuv420,
        vec![sample_frame(0)],
    )
    .unwrap()
}

#[test]
fn builds_nested_syntax_plan() {
    let plan = sample_sequence();
    assert_eq!(plan.coded_luma_size(), size(16, 16));
    assert_eq!(plan.bit_depth(), BitDepth::Eight);
    assert_eq!(plan.chroma_subsampling(), ChromaSubsampling::Yuv420);
    assert_eq!(plan.frames().len(), 1);
    assert_eq!(plan.event_count(), 5);

    let frame = &plan.frames()[0];
    assert_eq!(frame.info().id(), FrameId::new(0));
    assert_eq!(frame.tiles().len(), 1);

    let tile = &frame.tiles()[0];
    assert_eq!(tile.index().get(), 0);
    assert_eq!(tile.superblocks().len(), 1);
    assert_eq!(tile.syntax_events().len(), 5);

    let block = &tile.superblocks()[0].blocks()[0];
    assert_eq!(block.index().get(), 0);
    assert_eq!(block.coefficients().entries().len(), 2);
    assert_eq!(block.coefficients().eob(), 4);
}

#[test]
fn rejects_out_of_order_plan_children() {
    let err = FramePlan::new(
        FrameInfo::yuv420_8bit(FrameId::new(0), size(16, 16)),
        vec![sample_tile(1), sample_tile(0)],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::OutOfOrder {
            collection: "frame tiles",
            previous: PlanOrderKey::Tile(TileIndex(1)),
            next: PlanOrderKey::Tile(TileIndex(0)),
        }
    ));

    let err = SuperBlockPlan::new(
        SuperBlockIndex::new(0),
        vec![sample_block(1), sample_block(0)],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::OutOfOrder {
            collection: "superblock blocks",
            previous: PlanOrderKey::Block(BlockIndex(1)),
            next: PlanOrderKey::Block(BlockIndex(0)),
        }
    ));
}

#[test]
fn rejects_non_contiguous_plan_child_indices() {
    let tile = TilePlan::new(TileIndex::new(1), Vec::new(), Vec::new()).unwrap();
    let err = FramePlan::new(
        FrameInfo::yuv420_8bit(FrameId::new(0), size(16, 16)),
        vec![tile],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::NonContiguousIndex {
            collection: "frame tiles",
            actual: PlanOrderKey::Tile(TileIndex(1)),
            ..
        }
    ));
}

#[test]
fn rejects_sequence_frame_format_mismatches() {
    let frame = FramePlan::new(
        FrameInfo::new(
            FrameId::new(0),
            size(16, 16),
            BitDepth::Ten,
            ChromaSubsampling::Yuv420,
        ),
        Vec::new(),
    )
    .unwrap();
    let err = SequencePlan::new(
        size(16, 16),
        BitDepth::Eight,
        ChromaSubsampling::Yuv420,
        vec![frame],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::FrameFormatMismatch {
            frame,
            field: "bit depth",
        } if frame == FrameId::new(0)
    ));

    let frame = FramePlan::new(
        FrameInfo::yuv420_8bit(FrameId::new(1), size(17, 16)),
        Vec::new(),
    )
    .unwrap();
    let err = SequencePlan::new(
        size(16, 16),
        BitDepth::Eight,
        ChromaSubsampling::Yuv420,
        vec![frame],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::FrameLumaSizeOutOfBounds {
            frame,
            ..
        } if frame == FrameId::new(1)
    ));
}

#[test]
fn rejects_out_of_order_syntax_events_before_returning_plan() {
    let result = TilePlan::new(
        TileIndex::new(0),
        Vec::new(),
        vec![
            SyntaxEvent::Tile {
                index: SyntaxEventIndex::new(1),
                tile: TileIndex::new(0),
                symbol: symbol(1),
            },
            SyntaxEvent::Frame {
                index: SyntaxEventIndex::new(0),
                frame: FrameId::new(0),
                symbol: symbol(0),
            },
        ],
    );

    assert!(matches!(
        result,
        Err(SyntaxIrError::OutOfOrder {
            collection: "tile syntax events",
            previous: PlanOrderKey::Event(SyntaxEventIndex(1)),
            next: PlanOrderKey::Event(SyntaxEventIndex(0)),
        })
    ));
}

#[test]
fn rejects_duplicate_and_zero_quantized_coefficients() {
    let err = QuantizedCoefficients::new(vec![coefficient(2, 1), coefficient(2, -1)]).unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::DuplicateCoefficient {
            index: CoefficientIndex(2),
        }
    ));

    let err = QuantizedCoefficients::new(vec![coefficient(2, 0)]).unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::ZeroCoefficient {
            index: CoefficientIndex(2),
        }
    ));

    let err = QuantizedCoefficients::new(vec![coefficient(3, 1), coefficient(2, 1)]).unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::OutOfOrder {
            collection: "quantized coefficients",
            previous: PlanOrderKey::Coefficient(CoefficientIndex(3)),
            next: PlanOrderKey::Coefficient(CoefficientIndex(2)),
        }
    ));
}

#[test]
fn rejects_invalid_event_references_before_returning_tile_plan() {
    let tile = TileIndex::new(0);
    let superblock = SuperBlockPlan::new(SuperBlockIndex::new(0), vec![sample_block(0)]).unwrap();
    let err = TilePlan::new(
        tile,
        vec![superblock.clone()],
        vec![SyntaxEvent::Tile {
            index: SyntaxEventIndex::new(0),
            tile: TileIndex::new(1),
            symbol: symbol(0),
        }],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::ReferenceMismatch {
            event: SyntaxEventIndex(0),
            expected: PlanOrderKey::Tile(TileIndex(0)),
            actual: PlanOrderKey::Tile(TileIndex(1)),
        }
    ));

    let err = TilePlan::new(
        tile,
        vec![superblock],
        vec![SyntaxEvent::Token {
            index: SyntaxEventIndex::new(0),
            tile,
            superblock: SuperBlockIndex::new(0),
            block: BlockIndex::new(0),
            coefficient: CoefficientIndex::new(2),
            symbol: symbol(0),
        }],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::InvalidReference {
            event: SyntaxEventIndex(0),
            reference: PlanOrderKey::Coefficient(CoefficientIndex(2)),
            scope: "tile syntax events",
        }
    ));
}

#[test]
fn rejects_coefficients_outside_transform_area() {
    let coefficients = QuantizedCoefficients::new(vec![coefficient(16, 1)]).unwrap();
    let err = BlockDecision::new(
        BlockIndex::new(0),
        PredictionDecision::intra(symbol(1)),
        TransformDecision::new(size(4, 4), symbol(2)),
        coefficients,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::CoefficientsOutsideTransform {
            block: BlockIndex(0),
            eob: 17,
            area: 16,
        }
    ));
}

#[test]
fn rejects_planning_count_limit_and_overflow() {
    let too_many_events = (0..=MAX_SYNTAX_EVENTS_PER_TILE as u32)
        .map(|index| SyntaxEvent::Sequence {
            index: SyntaxEventIndex::new(index),
            symbol: symbol(0),
        })
        .collect();
    let err = TilePlan::new(TileIndex::new(0), Vec::new(), too_many_events).unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::CountLimitExceeded {
            field: "syntax events per tile",
            actual,
            limit: MAX_SYNTAX_EVENTS_PER_TILE,
        } if actual == MAX_SYNTAX_EVENTS_PER_TILE + 1
    ));

    let err = checked_add_count("syntax events", usize::MAX, 1).unwrap_err();
    assert!(matches!(
        err,
        SyntaxIrError::CountOverflow {
            field: "syntax events",
        }
    ));
}

#[test]
fn repeated_construction_has_stable_debug_output_and_event_order() {
    let first = sample_sequence();
    let second = sample_sequence();

    assert_eq!(format!("{first:#?}"), format!("{second:#?}"));

    let first_events: Vec<_> = first.frames()[0].tiles()[0]
        .syntax_events()
        .iter()
        .map(SyntaxEvent::index)
        .collect();
    let second_events: Vec<_> = second.frames()[0].tiles()[0]
        .syntax_events()
        .iter()
        .map(SyntaxEvent::index)
        .collect();
    assert_eq!(first_events, second_events);
    assert_eq!(
        first_events,
        vec![
            SyntaxEventIndex::new(0),
            SyntaxEventIndex::new(1),
            SyntaxEventIndex::new(2),
            SyntaxEventIndex::new(3),
            SyntaxEventIndex::new(4),
        ]
    );
}
