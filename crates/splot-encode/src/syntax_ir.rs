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

    #[error(
        "{collection} index is not zero-based and contiguous: expected position {expected}, actual {actual:?}"
    )]
    NonContiguousIndex {
        collection: &'static str,
        expected: usize,
        actual: PlanOrderKey,
    },

    #[error("frame {frame:?} {field} does not match the sequence plan")]
    FrameFormatMismatch { frame: FrameId, field: &'static str },

    #[error(
        "frame {frame:?} visible luma size {visible_luma_size:?} exceeds coded luma size {coded_luma_size:?}"
    )]
    FrameLumaSizeOutOfBounds {
        frame: FrameId,
        coded_luma_size: PlaneSize,
        visible_luma_size: PlaneSize,
    },

    #[error("syntax event {event:?} references {actual:?}, expected {expected:?}")]
    ReferenceMismatch {
        event: SyntaxEventIndex,
        expected: PlanOrderKey,
        actual: PlanOrderKey,
    },

    #[error("syntax event {event:?} references missing {reference:?} in {scope}")]
    InvalidReference {
        event: SyntaxEventIndex,
        reference: PlanOrderKey,
        scope: &'static str,
    },

    #[error("duplicate quantized coefficient index {index:?}")]
    DuplicateCoefficient { index: CoefficientIndex },

    #[error("quantized coefficient {index:?} has zero value")]
    ZeroCoefficient { index: CoefficientIndex },

    #[error("quantized coefficient index {index:?} exceeds planning limit {limit}")]
    CoefficientIndexOutOfRange {
        index: CoefficientIndex,
        limit: usize,
    },

    #[error("block {block:?} coefficient eob {eob} exceeds transform area {area}")]
    CoefficientsOutsideTransform {
        block: BlockIndex,
        eob: usize,
        area: usize,
    },
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
        validate_sequence_frame_info(coded_luma_size, bit_depth, chroma_subsampling, &frames)?;
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
        validate_contiguous_order(
            "frame tiles",
            &tiles,
            |tile| tile.index,
            PlanOrderKey::Tile,
            |index| usize::try_from(index.get()).ok(),
        )?;
        validate_frame_event_references(info.id(), &tiles)?;
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
        validate_contiguous_order(
            "tile superblocks",
            &superblocks,
            |superblock| superblock.index,
            PlanOrderKey::SuperBlock,
            |index| usize::try_from(index.get()).ok(),
        )?;
        check_count(
            "syntax events per tile",
            syntax_events.len(),
            MAX_SYNTAX_EVENTS_PER_TILE,
        )?;
        validate_contiguous_order(
            "tile syntax events",
            &syntax_events,
            SyntaxEvent::index,
            PlanOrderKey::Event,
            |index| usize::try_from(index.get()).ok(),
        )?;
        validate_tile_event_references(index, &superblocks, &syntax_events)?;

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
        validate_contiguous_order(
            "superblock blocks",
            &blocks,
            |block| block.index,
            PlanOrderKey::Block,
            |index| usize::try_from(index.get()).ok(),
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
    pub(crate) fn new(
        index: BlockIndex,
        prediction: PredictionDecision,
        transform: TransformDecision,
        coefficients: QuantizedCoefficients,
    ) -> SyntaxIrResult<Self> {
        let area = transform
            .block_size()
            .width()
            .checked_mul(transform.block_size().height())
            .ok_or(SyntaxIrError::CountOverflow {
                field: "transform coefficient capacity",
            })?;
        let eob = coefficients.eob();
        if eob > area {
            return Err(SyntaxIrError::CoefficientsOutsideTransform {
                block: index,
                eob,
                area,
            });
        }

        Ok(Self {
            index,
            prediction,
            transform,
            coefficients,
        })
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
            if usize::from(index.get()) >= MAX_COEFFICIENTS_PER_BLOCK {
                return Err(SyntaxIrError::CoefficientIndexOutOfRange {
                    index,
                    limit: MAX_COEFFICIENTS_PER_BLOCK,
                });
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

    fn contains(&self, index: CoefficientIndex) -> bool {
        self.entries
            .binary_search_by_key(&index, |entry| entry.index)
            .is_ok()
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

fn validate_sequence_frame_info(
    coded_luma_size: PlaneSize,
    bit_depth: BitDepth,
    chroma_subsampling: ChromaSubsampling,
    frames: &[FramePlan],
) -> SyntaxIrResult<()> {
    for frame in frames {
        let info = frame.info();
        if info.bit_depth() != bit_depth {
            return Err(SyntaxIrError::FrameFormatMismatch {
                frame: info.id(),
                field: "bit depth",
            });
        }
        if info.chroma_subsampling() != chroma_subsampling {
            return Err(SyntaxIrError::FrameFormatMismatch {
                frame: info.id(),
                field: "chroma subsampling",
            });
        }
        let visible_luma_size = info.visible_luma_size();
        if visible_luma_size.width() > coded_luma_size.width()
            || visible_luma_size.height() > coded_luma_size.height()
        {
            return Err(SyntaxIrError::FrameLumaSizeOutOfBounds {
                frame: info.id(),
                coded_luma_size,
                visible_luma_size,
            });
        }
    }
    Ok(())
}

fn validate_frame_event_references(frame_id: FrameId, tiles: &[TilePlan]) -> SyntaxIrResult<()> {
    for tile in tiles {
        for event in tile.syntax_events() {
            if let SyntaxEvent::Frame { index, frame, .. } = event {
                validate_reference_match(
                    *index,
                    PlanOrderKey::Frame(frame_id),
                    PlanOrderKey::Frame(*frame),
                )?;
            }
        }
    }
    Ok(())
}

fn validate_tile_event_references(
    tile_index: TileIndex,
    superblocks: &[SuperBlockPlan],
    events: &[SyntaxEvent],
) -> SyntaxIrResult<()> {
    for event in events {
        match event {
            SyntaxEvent::Sequence { .. } | SyntaxEvent::Frame { .. } => {}
            SyntaxEvent::Tile { index, tile, .. } => {
                validate_reference_match(
                    *index,
                    PlanOrderKey::Tile(tile_index),
                    PlanOrderKey::Tile(*tile),
                )?;
            }
            SyntaxEvent::Block {
                index,
                tile,
                superblock,
                block,
                ..
            } => {
                validate_reference_match(
                    *index,
                    PlanOrderKey::Tile(tile_index),
                    PlanOrderKey::Tile(*tile),
                )?;
                let superblock_plan = find_superblock(superblocks, *superblock).ok_or(
                    SyntaxIrError::InvalidReference {
                        event: *index,
                        reference: PlanOrderKey::SuperBlock(*superblock),
                        scope: "tile syntax events",
                    },
                )?;
                find_block(superblock_plan, *block).ok_or(SyntaxIrError::InvalidReference {
                    event: *index,
                    reference: PlanOrderKey::Block(*block),
                    scope: "tile syntax events",
                })?;
            }
            SyntaxEvent::Token {
                index,
                tile,
                superblock,
                block,
                coefficient,
                ..
            } => {
                validate_reference_match(
                    *index,
                    PlanOrderKey::Tile(tile_index),
                    PlanOrderKey::Tile(*tile),
                )?;
                let superblock_plan = find_superblock(superblocks, *superblock).ok_or(
                    SyntaxIrError::InvalidReference {
                        event: *index,
                        reference: PlanOrderKey::SuperBlock(*superblock),
                        scope: "tile syntax events",
                    },
                )?;
                let block_plan =
                    find_block(superblock_plan, *block).ok_or(SyntaxIrError::InvalidReference {
                        event: *index,
                        reference: PlanOrderKey::Block(*block),
                        scope: "tile syntax events",
                    })?;
                if !block_plan.coefficients().contains(*coefficient) {
                    return Err(SyntaxIrError::InvalidReference {
                        event: *index,
                        reference: PlanOrderKey::Coefficient(*coefficient),
                        scope: "tile syntax events",
                    });
                }
            }
        }
    }
    Ok(())
}

fn find_superblock(
    superblocks: &[SuperBlockPlan],
    index: SuperBlockIndex,
) -> Option<&SuperBlockPlan> {
    superblocks
        .iter()
        .find(|superblock| superblock.index == index)
}

fn find_block(superblock: &SuperBlockPlan, index: BlockIndex) -> Option<&BlockDecision> {
    superblock
        .blocks()
        .iter()
        .find(|block| block.index == index)
}

fn validate_reference_match(
    event: SyntaxEventIndex,
    expected: PlanOrderKey,
    actual: PlanOrderKey,
) -> SyntaxIrResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(SyntaxIrError::ReferenceMismatch {
            event,
            expected,
            actual,
        })
    }
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

fn validate_contiguous_order<T, K>(
    collection: &'static str,
    items: &[T],
    key: impl Fn(&T) -> K,
    order_key: impl Fn(K) -> PlanOrderKey,
    to_usize: impl Fn(K) -> Option<usize>,
) -> SyntaxIrResult<()>
where
    K: Copy + Ord,
{
    validate_strict_order(collection, items, &key, &order_key)?;
    for (expected, item) in items.iter().enumerate() {
        let actual = key(item);
        if to_usize(actual) != Some(expected) {
            return Err(SyntaxIrError::NonContiguousIndex {
                collection,
                expected,
                actual: order_key(actual),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "syntax_ir_tests.rs"]
mod tests;
