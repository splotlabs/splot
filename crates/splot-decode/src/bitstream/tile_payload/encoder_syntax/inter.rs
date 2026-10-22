// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter-frame state of the encoder facade: the decoder's § 7.23 reference
//! buffer over the encoder's reconstructed frames, and the § 5.20.7.3
//! `is_inter` syntax of intra blocks in an inter frame.

use splot_core::headers::frame::{FrameHeaderCore, FrameReferenceStateView, FrameType};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol_encoder::SymbolSink;
use splot_core::types::{EmbeddedLayerId, ObuType};
use splot_recon::{DecodedFrame, SharedFrame};

use super::super::cdf::TileCdfSelector;
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, rule};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{frame_ref_update_from_core, is_key_or_switch};
use crate::prediction::inter::{
    BlockPrecisionRecord, InterReferenceState, MvBlockContext, NON_INTER_FLAG_SYNTAX,
    NeighbourFlagSyntax, NeighbourMvGrid, frame_mv_precision, interp_filter_no_neighbour_ctx,
    is_inter_ctx,
};
use crate::reference::buffer::RuntimeReferenceBuffer;
use crate::tile::SbRowWindow;

/// The decoder's § 7.23 reference buffer, filled with the encoder's filtered
/// reconstructions in coding order.
pub struct EncoderReferences {
    buffer: RuntimeReferenceBuffer,
    frames: Vec<Option<SharedFrame<u16>>>,
    state: InterReferenceState<u16>,
}

impl EncoderReferences {
    /// An empty buffer with the reference slots of `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an incomplete sequence header.
    pub fn new(seq: &SequenceHeader) -> Result<Self> {
        let slots = seq
            .inter
            .as_ref()
            .ok_or(EncoderSyntaxError::Unsupported(
                "incomplete sequence header",
            ))?
            .num_ref_frames;
        let buffer =
            RuntimeReferenceBuffer::new(usize::from(slots)).map_err(rule("reference buffer"))?;
        let state = settled_state(&buffer, &[])?;
        Ok(Self {
            buffer,
            frames: Vec::new(),
            state,
        })
    }

    /// Stores the coded frame `core` with its filtered reconstruction `frame`
    /// in the slots its `refresh_frame_flags` name (§ 7.20).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a header without the § 7.20 inputs.
    pub fn update(&mut self, core: &FrameHeaderCore, frame: DecodedFrame<u16>) -> Result<()> {
        self.buffer.prepare_for_frame(core.obu_type, true);
        let update =
            frame_ref_update_from_core(core, ByteOffset::new(0), EmbeddedLayerId::from_bits(0))
                .map_err(rule("reference update"))?;
        let index = (0..=self.frames.len())
            .find(|&index| !self.buffer.retains(index))
            .unwrap_or(self.frames.len());
        if index == self.frames.len() {
            self.frames.push(None);
        }
        self.frames[index] = Some(SharedFrame::new(frame));
        self.buffer.update(index, &update, is_key_or_switch(core));
        self.buffer
            .note_frame(core.obu_type, true, &update, &core.ref_long_term_ids);
        for (index, frame) in self.frames.iter_mut().enumerate() {
            if !self.buffer.retains(index) {
                *frame = None;
            }
        }
        self.state = settled_state(&self.buffer, &self.frames)?;
        Ok(())
    }

    /// Records a show-existing frame of `slot` with `derive_sef_order_hint = 1`
    /// in its own temporal unit (§ 7.21).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] when `slot` holds no hidden frame that is
    /// still waiting for output.
    pub fn show_existing(&mut self, slot: u32) -> Result<()> {
        self.buffer.prepare_for_frame(ObuType::RegularSef, true);
        self.buffer
            .mark_sef_derive_output(slot, false)
            .map_err(rule("show-existing frame"))?;
        self.buffer.note_show_existing();
        Ok(())
    }

    /// The reference state the next frame header parses with.
    #[must_use]
    pub fn header_view(&self) -> FrameReferenceStateView<'_> {
        self.state.header_view()
    }
}

/// The reference state of `buffer` over settled `frames`.
fn settled_state(
    buffer: &RuntimeReferenceBuffer,
    frames: &[Option<SharedFrame<u16>>],
) -> Result<InterReferenceState<u16>> {
    let (store, meta) = buffer
        .build_store_from(|slot, index| {
            let frame = frames
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(crate::error::DecodeReferenceStateError::MissingFrame { slot })?;
            Ok((RefFrameSlot::completed(frame.share()), None))
        })
        .map_err(rule("reference state"))?;
    Ok(InterReferenceState::from_metadata(store, meta))
}

/// The § 5.20.7 neighbour flags of an inter frame's tile.
pub(super) struct InterLeafState {
    grid: NeighbourMvGrid,
    precision: u8,
}

impl InterLeafState {
    /// `None` for an intra frame.
    pub(super) fn new(
        core: &FrameHeaderCore,
        (mi_rows, mi_cols): (usize, usize),
    ) -> Result<Option<Self>> {
        if core.frame_is_intra == Some(true) {
            return Ok(None);
        }
        let skip_mode = core
            .inter_tail
            .as_ref()
            .ok_or(EncoderSyntaxError::Unsupported(
                "incomplete inter frame header",
            ))?
            .skip_mode_present;
        if core.frame_type != Some(FrameType::Inter) || skip_mode {
            return Err(EncoderSyntaxError::Unsupported(
                "skip mode or a switch frame",
            ));
        }
        let mut grid = NeighbourMvGrid::default();
        grid.reset_for_tile(0..mi_rows, 0..mi_cols, SbRowWindow::WHOLE_TILE_SB_H4)
            .map_err(|_| EncoderSyntaxError::Unsupported("neighbour grid size"))?;
        let precision = frame_mv_precision(core).map_err(rule("MV precision"))?;
        Ok(Some(Self { grid, precision }))
    }
}

impl IntraTileSyntax {
    /// Whether `leaf` may code an intra block: an inter frame infers
    /// `is_inter = 1` for a block whose chroma is coded with another block.
    #[must_use]
    pub fn intra_allowed(&self, leaf: &LeafBlock) -> bool {
        self.inter.is_none() || !leaf.frontier.shared_mixed_chroma_ref_forces_inter()
    }

    /// Writes `is_inter = 0` of `leaf` in an inter frame (§ 5.20.7.3); an intra
    /// frame codes no `is_inter`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when the decoder infers
    /// `is_inter = 1` for `leaf`, and [`EncoderSyntaxError::Rule`] for a symbol
    /// write failure.
    pub fn write_is_inter(
        &mut self,
        leaf: &LeafBlock,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        if !self.intra_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported(
                "an intra block where the decoder infers is_inter",
            ));
        }
        let Some(inter) = &self.inter else {
            return Ok(());
        };
        if leaf.frontier.is_luma_part() || leaf.frontier.is_chroma_part() {
            return Ok(());
        }
        let ctx = is_inter_ctx(&inter.grid, &self.mv_block(leaf)?);
        self.put(symbols, TileCdfSelector::IsInter { ctx }, 0)
    }

    /// Publishes the neighbour flags of the intra leaf `leaf` of an inter frame.
    pub(super) fn publish_inter_neighbours(&mut self, leaf: &LeafBlock) -> Result<()> {
        if self.inter.is_none() || leaf.frontier.is_chroma_part() {
            return Ok(());
        }
        let block = self.mv_block(leaf)?;
        if let Some(inter) = &mut self.inter {
            inter.grid.record_flags(
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                NeighbourFlagSyntax {
                    interp_filter: interp_filter_no_neighbour_ctx(false) as u8,
                    precision: BlockPrecisionRecord::explicit(inter.precision),
                    ..NON_INTER_FLAG_SYNTAX
                },
            );
        }
        Ok(())
    }

    fn mv_block(&self, leaf: &LeafBlock) -> Result<MvBlockContext> {
        let size = leaf.frontier.b_size;
        let (mi_rows, mi_cols) = self.mi_dimensions();
        Ok(MvBlockContext {
            mi_row: leaf.frontier.r,
            mi_col: leaf.frontier.c,
            bw4: size.num_4x4_wide().map_err(rule("block size"))?,
            bh4: size.num_4x4_high().map_err(rule("block size"))?,
            sb_h4: self.sb_size4,
            ref_frame0: 0,
            ref_frame1: None,
            mi_rows,
            mi_cols,
        })
    }
}
