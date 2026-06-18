// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private deterministic syntax-planning IR for future encoder passes.
//!
//! This module advances `ENC-SYNTAX-IR`. It is intentionally not re-exported
//! from the crate root and it does not emit bytes, own a bit writer, or produce
//! [`crate::Packet`] values. Future writer integration will consume this staging
//! model once exact AV2 syntax mappings are implemented.

#![allow(dead_code)]

use splot_recon::PlaneSize;

use crate::config::{BitDepth, ChromaSubsampling};
use crate::frame::{FrameId, FrameInfo};

const MAX_FRAMES_PER_SEQUENCE: usize = 4096;
const MAX_TILES_PER_FRAME: usize = 4096;
const MAX_SUPERBLOCKS_PER_TILE: usize = 4096;
const MAX_BLOCKS_PER_SUPERBLOCK: usize = 4096;
const MAX_COEFFICIENTS_PER_BLOCK: usize = 4096;
const MAX_SYNTAX_EVENTS_PER_TILE: usize = 4096;
const MAX_SYNTAX_EVENTS_PER_FRAME: usize = 1_048_576;
const MAX_SYNTAX_EVENTS_PER_SEQUENCE: usize = 16_777_216;

type SyntaxIrResult<T> = core::result::Result<T, SyntaxIrError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SyntaxIrError {
    #[error("{field} count {actual} exceeds planning limit {limit}")]
    CountLimitExceeded {
        field: &'static str,
        actual: usize,
        limit: usize,
    },

    #[error("{field} count arithmetic overflowed")]
    CountOverflow { field: &'static str },

    #[error("{collection} order is not strictly increasing: previous {previous:?}, next {next:?}")]
    OutOfOrder {
        collection: &'static str,
        previous: PlanOrderKey,
        next: PlanOrderKey,
    },

    #[error("duplicate quantized coefficient index {index:?}")]
    DuplicateCoefficient { index: CoefficientIndex },

    #[error("quantized coefficient {index:?} has zero value")]
    ZeroCoefficient { index: CoefficientIndex },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanOrderKey {
    Frame(FrameId),
    Tile(TileIndex),
    SuperBlock(SuperBlockIndex),
    Block(BlockIndex),
    Coefficient(CoefficientIndex),
    Event(SyntaxEventIndex),
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TileIndex(u32);

impl TileIndex {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SuperBlockIndex(u32);

impl SuperBlockIndex {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BlockIndex(u32);

impl BlockIndex {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CoefficientIndex(u16);

impl CoefficientIndex {
    pub(crate) const fn new(index: u16) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SyntaxEventIndex(u32);

impl SyntaxEventIndex {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PlanSymbolId(u16);

impl PlanSymbolId {
    pub(crate) const fn new(symbol_id: u16) -> Self {
        Self(symbol_id)
    }

    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SequencePlan {
    coded_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
    frames: Vec<FramePlan>,
    syntax_event_count: usize,
}

impl SequencePlan {
    pub(crate) fn new(
        coded_luma_size: PlaneSize,
        bit_depth: BitDepth,
        chroma_subsampling: ChromaSubsampling,
        frames: Vec<FramePlan>,
    ) -> SyntaxIrResult<Self> {
        check_count("frames per sequence", frames.len(), MAX_FRAMES_PER_SEQUENCE)?;
        validate_strict_order(
            "sequence frames",
            &frames,
            |frame| frame.info.id(),
            PlanOrderKey::Frame,
        )?;
        let syntax_event_count = sum_counts(
            "sequence syntax events",
            frames.iter().map(FramePlan::event_count),
        )?;
        check_count(
            "syntax events per sequence",
            syntax_event_count,
            MAX_SYNTAX_EVENTS_PER_SEQUENCE,
        )?;

        Ok(Self {
            coded_luma_size,
            bit_depth,
            chroma_subsampling,
            frames,
            syntax_event_count,
        })
    }

    pub(crate) const fn coded_luma_size(&self) -> PlaneSize {
        self.coded_luma_size
    }

    pub(crate) const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    pub(crate) const fn chroma_subsampling(&self) -> ChromaSubsampling {
        self.chroma_subsampling
    }

    pub(crate) fn frames(&self) -> &[FramePlan] {
        &self.frames
    }

    pub(crate) const fn event_count(&self) -> usize {
        self.syntax_event_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FramePlan {
    info: FrameInfo,
    tiles: Vec<TilePlan>,
    syntax_event_count: usize,
}

impl FramePlan {
    pub(crate) fn new(info: FrameInfo, tiles: Vec<TilePlan>) -> SyntaxIrResult<Self> {
        check_count("tiles per frame", tiles.len(), MAX_TILES_PER_FRAME)?;
        validate_strict_order("frame tiles", &tiles, |tile| tile.index, PlanOrderKey::Tile)?;
        let syntax_event_count = sum_counts(
            "frame syntax events",
            tiles.iter().map(TilePlan::event_count),
        )?;
        check_count(
            "syntax events per frame",
            syntax_event_count,
            MAX_SYNTAX_EVENTS_PER_FRAME,
        )?;

        Ok(Self {
            info,
            tiles,
            syntax_event_count,
        })
    }

    pub(crate) const fn info(&self) -> FrameInfo {
        self.info
    }

    pub(crate) fn tiles(&self) -> &[TilePlan] {
        &self.tiles
    }

    pub(crate) const fn event_count(&self) -> usize {
        self.syntax_event_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TilePlan {
    index: TileIndex,
    superblocks: Vec<SuperBlockPlan>,
    syntax_events: Vec<SyntaxEvent>,
}

impl TilePlan {
    pub(crate) fn new(
        index: TileIndex,
        superblocks: Vec<SuperBlockPlan>,
        syntax_events: Vec<SyntaxEvent>,
    ) -> SyntaxIrResult<Self> {
        check_count(
            "superblocks per tile",
            superblocks.len(),
            MAX_SUPERBLOCKS_PER_TILE,
        )?;
        validate_strict_order(
            "tile superblocks",
            &superblocks,
            |superblock| superblock.index,
            PlanOrderKey::SuperBlock,
        )?;
        check_count(
            "syntax events per tile",
            syntax_events.len(),
            MAX_SYNTAX_EVENTS_PER_TILE,
        )?;
        validate_strict_order(
            "tile syntax events",
            &syntax_events,
            SyntaxEvent::index,
            PlanOrderKey::Event,
        )?;

        Ok(Self {
            index,
            superblocks,
            syntax_events,
        })
    }

    pub(crate) const fn index(&self) -> TileIndex {
        self.index
    }

    pub(crate) fn superblocks(&self) -> &[SuperBlockPlan] {
        &self.superblocks
    }

    pub(crate) fn syntax_events(&self) -> &[SyntaxEvent] {
        &self.syntax_events
    }

    pub(crate) const fn event_count(&self) -> usize {
        self.syntax_events.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SuperBlockPlan {
    index: SuperBlockIndex,
    blocks: Vec<BlockDecision>,
}

impl SuperBlockPlan {
    pub(crate) fn new(index: SuperBlockIndex, blocks: Vec<BlockDecision>) -> SyntaxIrResult<Self> {
        check_count(
            "blocks per superblock",
            blocks.len(),
            MAX_BLOCKS_PER_SUPERBLOCK,
        )?;
        validate_strict_order(
            "superblock blocks",
            &blocks,
            |block| block.index,
            PlanOrderKey::Block,
        )?;
        Ok(Self { index, blocks })
    }

    pub(crate) const fn index(&self) -> SuperBlockIndex {
        self.index
    }

    pub(crate) fn blocks(&self) -> &[BlockDecision] {
        &self.blocks
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockDecision {
    index: BlockIndex,
    prediction: PredictionDecision,
    transform: TransformDecision,
    coefficients: QuantizedCoefficients,
}

impl BlockDecision {
    pub(crate) const fn new(
        index: BlockIndex,
        prediction: PredictionDecision,
        transform: TransformDecision,
        coefficients: QuantizedCoefficients,
    ) -> Self {
        Self {
            index,
            prediction,
            transform,
            coefficients,
        }
    }

    pub(crate) const fn index(&self) -> BlockIndex {
        self.index
    }

    pub(crate) const fn prediction(&self) -> PredictionDecision {
        self.prediction
    }

    pub(crate) const fn transform(&self) -> TransformDecision {
        self.transform
    }

    pub(crate) const fn coefficients(&self) -> &QuantizedCoefficients {
        &self.coefficients
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PredictionDecision {
    Intra { predictor: PlanSymbolId },
}

impl PredictionDecision {
    pub(crate) const fn intra(predictor: PlanSymbolId) -> Self {
        Self::Intra { predictor }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransformDecision {
    block_size: PlaneSize,
    transform: PlanSymbolId,
}

impl TransformDecision {
    pub(crate) const fn new(block_size: PlaneSize, transform: PlanSymbolId) -> Self {
        Self {
            block_size,
            transform,
        }
    }

    pub(crate) const fn block_size(self) -> PlaneSize {
        self.block_size
    }

    pub(crate) const fn transform(self) -> PlanSymbolId {
        self.transform
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QuantizedCoefficients {
    entries: Vec<QuantizedCoefficient>,
}

impl QuantizedCoefficients {
    pub(crate) fn new(entries: Vec<(CoefficientIndex, i32)>) -> SyntaxIrResult<Self> {
        check_count(
            "quantized coefficients per block",
            entries.len(),
            MAX_COEFFICIENTS_PER_BLOCK,
        )?;

        let mut coefficients = Vec::with_capacity(entries.len());
        let mut previous_index = None;
        for (index, value) in entries {
            if value == 0 {
                return Err(SyntaxIrError::ZeroCoefficient { index });
            }
            if let Some(previous) = previous_index {
                if index == previous {
                    return Err(SyntaxIrError::DuplicateCoefficient { index });
                }
                if index < previous {
                    return Err(SyntaxIrError::OutOfOrder {
                        collection: "quantized coefficients",
                        previous: PlanOrderKey::Coefficient(previous),
                        next: PlanOrderKey::Coefficient(index),
                    });
                }
            }
            previous_index = Some(index);
            coefficients.push(QuantizedCoefficient { index, value });
        }

        Ok(Self {
            entries: coefficients,
        })
    }

    pub(crate) fn entries(&self) -> &[QuantizedCoefficient] {
        &self.entries
    }

    pub(crate) fn eob(&self) -> usize {
        self.entries
            .last()
            .map_or(0, |entry| usize::from(entry.index.get()) + 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuantizedCoefficient {
    index: CoefficientIndex,
    value: i32,
}

impl QuantizedCoefficient {
    pub(crate) const fn index(self) -> CoefficientIndex {
        self.index
    }

    pub(crate) const fn value(self) -> i32 {
        self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxEvent {
    Sequence {
        index: SyntaxEventIndex,
        symbol: PlanSymbolId,
    },
    Frame {
        index: SyntaxEventIndex,
        frame: FrameId,
        symbol: PlanSymbolId,
    },
    Tile {
        index: SyntaxEventIndex,
        tile: TileIndex,
        symbol: PlanSymbolId,
    },
    Block {
        index: SyntaxEventIndex,
        tile: TileIndex,
        superblock: SuperBlockIndex,
        block: BlockIndex,
        symbol: PlanSymbolId,
    },
    Token {
        index: SyntaxEventIndex,
        tile: TileIndex,
        superblock: SuperBlockIndex,
        block: BlockIndex,
        coefficient: CoefficientIndex,
        symbol: PlanSymbolId,
    },
}

impl SyntaxEvent {
    pub(crate) const fn index(&self) -> SyntaxEventIndex {
        match self {
            Self::Sequence { index, .. }
            | Self::Frame { index, .. }
            | Self::Tile { index, .. }
            | Self::Block { index, .. }
            | Self::Token { index, .. } => *index,
        }
    }
}

fn check_count(field: &'static str, actual: usize, limit: usize) -> SyntaxIrResult<()> {
    if actual > limit {
        return Err(SyntaxIrError::CountLimitExceeded {
            field,
            actual,
            limit,
        });
    }
    Ok(())
}

fn checked_add_count(field: &'static str, left: usize, right: usize) -> SyntaxIrResult<usize> {
    left.checked_add(right)
        .ok_or(SyntaxIrError::CountOverflow { field })
}

fn sum_counts(
    field: &'static str,
    counts: impl IntoIterator<Item = usize>,
) -> SyntaxIrResult<usize> {
    let mut total = 0_usize;
    for count in counts {
        total = checked_add_count(field, total, count)?;
    }
    Ok(total)
}

fn validate_strict_order<T, K>(
    collection: &'static str,
    items: &[T],
    key: impl Fn(&T) -> K,
    order_key: impl Fn(K) -> PlanOrderKey,
) -> SyntaxIrResult<()>
where
    K: Copy + Ord,
{
    let Some((first, rest)) = items.split_first() else {
        return Ok(());
    };

    let mut previous = key(first);
    for item in rest {
        let next = key(item);
        if next <= previous {
            return Err(SyntaxIrError::OutOfOrder {
                collection,
                previous: order_key(previous),
                next: order_key(next),
            });
        }
        previous = next;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
    }

    fn sample_tile(index: u32) -> TilePlan {
        let tile = TileIndex::new(index);
        let superblock =
            SuperBlockPlan::new(SuperBlockIndex::new(0), vec![sample_block(0)]).unwrap();
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
        let err =
            QuantizedCoefficients::new(vec![coefficient(2, 1), coefficient(2, -1)]).unwrap_err();
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

        let err =
            QuantizedCoefficients::new(vec![coefficient(3, 1), coefficient(2, 1)]).unwrap_err();
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
}
