// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-block `segment_id` of a segmentation map coded with
//! `segmentation_update_map = 1` and no temporal update (§ 5.20.5.7,
//! § 5.20.5.8, § 5.20.7.2), and the § 7.12.2 `get_qindex` of each block.

use std::ops::Range;

use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, SegmentationParams, get_qindex,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::TileCdfSelector;
use super::super::{TileSegmentIdState, neg_deinterleave};
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, rule};

/// The segment state of one tile of a frame with `segmentation_enabled`.
pub(super) struct Segments {
    params: SegmentationParams,
    quant: CoreSeqQuantView,
    pub(super) ids: TileSegmentIdState,
    /// The segment the blocks of the current superblock code.
    planned: u8,
    /// The segment of the current leaf: `planned`, or the § 5.20.5.8
    /// prediction that a skipped leaf takes.
    leaf: u8,
    /// `CurrentQIndex` of the current leaf.
    current_qindex: u32,
    /// The `segment_id` symbols coded so far.
    pub(super) coded: u64,
}

impl Segments {
    /// The state of a tile over `tile` of `core`; `None` without
    /// segmentation.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a map the facade does
    /// not code: a feature other than `SEG_LVL_ALT_Q`, a temporal update or
    /// a map that is not updated, or 16 segments (`enable_ext_seg`).
    pub(super) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        tile: (Range<usize>, Range<usize>),
        base_q_idx: u32,
    ) -> Result<Option<Self>> {
        let Some(params) = core
            .segmentation_params
            .filter(|params| params.segmentation_enabled)
        else {
            return Ok(None);
        };
        let ext = seq.segment.as_ref().is_some_and(|s| s.enable_ext_seg);
        let other_feature = params
            .features
            .iter()
            .any(|segment| segment[1..].iter().any(|feature| feature.enabled));
        if ext
            || other_feature
            || !params.segmentation_update_map
            || params.segmentation_temporal_update
        {
            return Err(EncoderSyntaxError::Unsupported(
                "segmentation other than an updated SEG_LVL_ALT_Q map",
            ));
        }
        let tq = seq
            .transform_quant_entropy
            .as_ref()
            .ok_or(EncoderSyntaxError::Unsupported("missing quantizer config"))?;
        Ok(Some(Self {
            params,
            quant: CoreSeqQuantView::from_sequence_configs(&seq.general, tq),
            ids: TileSegmentIdState::new_for_tile(tile.0, tile.1)
                .map_err(rule("segment id state"))?,
            planned: 0,
            leaf: 0,
            current_qindex: base_q_idx,
            coded: 0,
        }))
    }

    fn qindex(&self) -> u32 {
        get_qindex(
            &self.quant,
            self.current_qindex,
            &self.params,
            usize::from(self.leaf),
        )
    }
}

impl IntraTileSyntax {
    /// Sets the segment that the leaves of the following superblocks code
    /// and returns their § 7.12.2 `get_qindex`: `CurrentQIndex` plus the
    /// segment's `SEG_LVL_ALT_Q` data, clipped to `0..=MaxQ`. A skipped
    /// leaf codes no `segment_id` and takes the predicted segment.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when the frame has no
    /// segmentation or `segment` is past `LastActiveSegId`.
    pub fn set_superblock_segment(&mut self, segment: u8) -> Result<u32> {
        let segments = self
            .segments
            .as_mut()
            .ok_or(EncoderSyntaxError::Unsupported(
                "a segment without segmentation",
            ))?;
        if segment > segments.params.last_active_seg_id {
            return Err(EncoderSyntaxError::Unsupported(
                "a segment past LastActiveSegId",
            ));
        }
        segments.planned = segment;
        self.set_leaf_segment(segment);
        Ok(self.tools.qindex)
    }

    /// The `segment_id` symbols that this facade has coded.
    #[must_use]
    pub fn coded_segment_ids(&self) -> u64 {
        self.segments.as_ref().map_or(0, |segments| segments.coded)
    }

    /// Writes `segment_id` of `leaf`, coded with `skip_txfm = skip`, after
    /// its skip syntax: a skipped leaf takes the predicted segment and codes
    /// nothing, every other leaf codes the superblock's segment against the
    /// prediction (§ 5.20.5.8). A `CHROMA_PART` leaf keeps the segment of
    /// its luma leaf. Without segmentation it writes nothing.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a write failure.
    pub fn write_segment_id(
        &mut self,
        leaf: &LeafBlock,
        skip: bool,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let (rows, cols) = (self.tile.0.start, self.tile.1.start);
        let Some(segments) = self.segments.as_mut() else {
            return Ok(());
        };
        let (r, c) = (leaf.row4(), leaf.col4());
        if leaf.is_chroma_part() {
            let segment = segments.ids.cell(r, c).unwrap_or(0);
            self.set_leaf_segment(segment);
            return Ok(());
        }
        let (pred, ctx) = segments.ids.predictor_and_ctx(r, c, r > rows, c > cols);
        let segment = if skip { pred } else { segments.planned };
        let (w, h) = leaf.dimensions()?;
        segments.ids.record_block(r, c, w / 4, h / 4, segment);
        if !skip {
            let max = i32::from(segments.params.last_active_seg_id) + 1;
            let diff = (0..max)
                .find(|&diff| neg_deinterleave(diff, i32::from(pred), max) == i32::from(segment))
                .ok_or(EncoderSyntaxError::Unsupported("an uncodable segment id"))?;
            segments.coded += 1;
            let cdf = self.tile_row(TileCdfSelector::SegmentId { ctx, ext: false })?;
            symbols
                .symbol(cdf, Symbol::new(diff as u8))
                .map_err(rule("segment_id"))?;
        }
        self.set_leaf_segment(segment);
        Ok(())
    }

    /// Ends the current leaf: the next leaf starts on the superblock's
    /// segment.
    pub(super) fn end_leaf_segment(&mut self) {
        if let Some(segment) = self.segments.as_ref().map(|segments| segments.planned) {
            self.set_leaf_segment(segment);
        }
    }

    fn set_leaf_segment(&mut self, segment: u8) {
        if let Some(segments) = self.segments.as_mut() {
            segments.leaf = segment;
            let qindex = segments.current_qindex;
            self.apply_qindex(qindex as i32);
        }
    }

    /// The block qindex at `CurrentQIndex = current`: § 7.12.2 `get_qindex`
    /// of the current leaf's segment, `current` without segmentation.
    pub(super) fn block_qindex(&mut self, current: i32) -> u32 {
        let current = current as u32;
        self.segments.as_mut().map_or(current, |segments| {
            segments.current_qindex = current;
            segments.qindex()
        })
    }

    /// Restores the `(coded, leaf)` state of a snapshot.
    pub(super) fn restore_segment(&mut self, (coded, leaf): (u64, u8)) {
        if let Some(segments) = self.segments.as_mut() {
            segments.coded = coded;
            self.set_leaf_segment(leaf);
        }
    }

    /// The segment of the current leaf, `0` without segmentation.
    pub(super) fn leaf_segment(&self) -> u8 {
        self.segments.as_ref().map_or(0, |segments| segments.leaf)
    }
}
