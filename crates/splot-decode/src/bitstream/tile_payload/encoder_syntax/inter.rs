// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter-frame state of the encoder facade: the decoder's § 7.23 reference
//! buffer over the encoder's reconstructed frames, and the single-reference,
//! compound and skip-mode inter leaves of § 5.20.7.
//!
//! An inter leaf is planned with [`IntraTileSyntax::plan_inter_leaf`], which
//! runs the decoder's § 7.12 stack and § 7.11.3 motion compensation into the
//! frame. The encoder codes the residual of the planned transform blocks, adds
//! it with [`IntraTileSyntax::reconstruct_inter`], writes the leaf syntax, and
//! publishes the leaf; publication runs the decoder's § 7.12 resolution and
//! fails when it derives other motion vectors than the plan.

use splot_core::headers::frame::{FrameHeaderCore, FrameReferenceStateView, TxMode};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol_encoder::SymbolSink;
use splot_core::tables::conversion::SIZE_TO_TX_PART_GROUP_LOOKUP;
use splot_core::types::{EmbeddedLayerId, ObuType};
use splot_recon::{BitDepth, DecodedFrame, ReferencePlaneView, SharedFrame, SubpelPredictParams};

use super::super::cdf::TileCdfSelector;
use super::super::general_intra_residual::inter_chroma_plane_tx_type;
use super::super::partition_traversal::{DecodedLeafPublication, GeneralIntraTreeWalkError};
use super::compound::InterPrediction;
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, TransformBlock, TxType, rule};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{frame_ref_update_from_core, is_key_or_switch};
use crate::prediction::inter::{
    EncoderInterCheckpoint, EncoderInterResidual, EncoderInterTile, EncoderMotion,
    EncoderSingleMotion, InterReferenceState, Mv, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, chroma_inter_tx_type, non_intra_leaf_mode, single_transform_residual,
};
use crate::reference::buffer::RuntimeReferenceBuffer;

/// `BLOCK_4X4` (§ 9.3), which codes no inter transform partition.
const BLOCK_4X4: usize = 0;
/// `MV_PRECISION_EIGHTH_PEL`: motion vectors are in units of `1/8` sample.
pub const MV_PRECISION_EIGHTH_PEL: u8 = 6;

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

    /// A fresh reference state for one tile.
    fn tile_state(&self) -> Result<InterReferenceState<u16>> {
        settled_state(&self.buffer, &self.frames)
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

/// A motion vector in `1/8` luma samples (§ 7.12).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct MotionVector {
    /// Vertical component.
    pub row: i32,
    /// Horizontal component.
    pub col: i32,
}

impl From<Mv> for MotionVector {
    fn from(mv: Mv) -> Self {
        Self {
            row: mv.row,
            col: mv.col,
        }
    }
}

impl From<MotionVector> for Mv {
    fn from(mv: MotionVector) -> Self {
        Self {
            row: mv.row,
            col: mv.col,
        }
    }
}

/// A single-reference `YMode` (§ 5.20.7.11).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum InterMode {
    /// `NEARMV`: the DRL candidate.
    Near,
    /// `GLOBALMV`: the global motion vector.
    Global,
    /// `NEWMV`: the DRL candidate plus a coded difference.
    New,
}

/// The single-reference prediction of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InterModes {
    /// The reference list index, below [`IntraTileSyntax::reference_count`].
    pub ref_frame: usize,
    /// `YMode`.
    pub mode: InterMode,
    /// `RefMvIdx`, below [`IntraTileSyntax::drl_count`]; `GLOBALMV` ignores it.
    pub drl: usize,
    /// The block motion vector of `NEWMV`: the DRL candidate plus a multiple of
    /// [`IntraTileSyntax::mv_step`]. The other modes ignore it.
    pub mv: MotionVector,
}

/// The inter state of one tile, `None` in an intra frame.
pub(super) struct InterLeafState {
    pub(super) tile: EncoderInterTile,
    planned: Option<PlannedInter>,
}

/// The inter leaf being coded.
struct PlannedInter {
    leaf: LeafBlock,
    motion: EncoderMotion,
    mvs: [Mv; 2],
    residual: EncoderInterResidual,
}

/// The saved inter state of one region.
pub(super) type InterSnapshot = Option<EncoderInterCheckpoint>;

impl InterLeafState {
    /// `None` for an intra frame.
    pub(super) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        tile: (core::ops::Range<usize>, core::ops::Range<usize>),
        (qindex, bit_depth): (u32, BitDepth),
    ) -> Result<Option<Self>> {
        if core.frame_is_intra == Some(true) {
            return Ok(None);
        }
        let tile = EncoderInterTile::new(seq, core, refs.tile_state()?, tile, (qindex, bit_depth))
            .map_err(EncoderSyntaxError::Unsupported)?;
        Ok(Some(Self {
            tile,
            planned: None,
        }))
    }
}

impl IntraTileSyntax {
    /// Whether `leaf` may code an intra block: an inter frame infers
    /// `is_inter = 1` for a block whose chroma is coded with another block.
    #[must_use]
    pub fn intra_allowed(&self, leaf: &LeafBlock) -> bool {
        self.inter.is_none() || !leaf.frontier.shared_mixed_chroma_ref_forces_inter()
    }

    /// Whether `leaf` may code a single-reference inter block.
    ///
    /// shortcut: only leaves whose planes each take one transform (at most
    /// 64x64) are inter; lift with the chunked § 5.20.7.23 residual walk.
    #[must_use]
    pub fn inter_allowed(&self, leaf: &LeafBlock) -> bool {
        self.inter.is_some()
            && !leaf.frontier.is_luma_part()
            && !leaf.frontier.is_chroma_part()
            && single_transform_residual(&leaf.frontier, self.chroma)
                .is_ok_and(|units| units.is_some())
    }

    /// `NumTotalRefs`; `0` in an intra frame.
    #[must_use]
    pub fn reference_count(&self) -> usize {
        self.inter
            .as_ref()
            .map_or(0, |inter| inter.tile.num_total_refs())
    }

    /// The number of `RefMvIdx` values `NEARMV` and `NEWMV` can code.
    #[must_use]
    pub fn drl_count(&self) -> usize {
        self.inter
            .as_ref()
            .map_or(0, |inter| inter.tile.max_drl_bits_minus_1() as usize + 2)
    }

    /// The frame `MvPrecision`; see [`MV_PRECISION_EIGHTH_PEL`].
    #[must_use]
    pub fn mv_precision(&self) -> u8 {
        self.inter
            .as_ref()
            .map_or(MV_PRECISION_EIGHTH_PEL, |inter| inter.tile.precision())
    }

    /// The `NEWMV` difference step in `1/8` samples.
    #[must_use]
    pub fn mv_step(&self) -> i32 {
        1 << MV_PRECISION_EIGHTH_PEL.saturating_sub(self.mv_precision())
    }

    /// The motion vector the decoder derives for `modes` at `leaf`. A `NEWMV`
    /// whose `mv` is off the step lattice of its candidate is an error.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code.
    pub fn inter_mv(&mut self, leaf: &LeafBlock, modes: InterModes) -> Result<MotionVector> {
        let motion = self.single_motion(leaf, modes)?;
        let tile = &mut self.inter_state()?.tile;
        tile.motion_vector(&leaf.frontier, motion)
            .map(MotionVector::from)
            .map_err(rule("motion vector"))
    }

    /// The `NEWMV` base vector of DRL candidate `drl` from list reference
    /// `ref_frame` at `leaf`: a `NEWMV` vector is this plus a multiple of
    /// [`Self::mv_step`] per component.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a reference or index `leaf` cannot code.
    pub fn newmv_base(
        &mut self,
        leaf: &LeafBlock,
        ref_frame: usize,
        drl: usize,
    ) -> Result<MotionVector> {
        let modes = InterModes {
            ref_frame,
            mode: InterMode::Near,
            drl,
            mv: MotionVector::default(),
        };
        let motion = EncoderSingleMotion {
            mode: SINGLE_MODE_NEWMV,
            ..self.single_motion(leaf, modes)?
        };
        let tile = &mut self.inter_state()?.tile;
        tile.motion_vector(&leaf.frontier, motion)
            .map(MotionVector::from)
            .map_err(rule("motion vector"))
    }

    /// Plans the inter leaf `leaf` under `prediction`: predicts it into the
    /// frame and returns its transform blocks, luma then chroma. Chroma blocks
    /// hold the type a `DCT_DCT` luma block implies; see
    /// [`Self::inter_chroma_tx_type`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction `leaf` cannot code.
    pub fn plan_inter_leaf(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
    ) -> Result<Vec<TransformBlock>> {
        if !self.inter_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported("an inter block here"));
        }
        let motion = self.motion(leaf, prediction)?;
        self.recon.leaf = None;
        let chroma = self.chroma;
        let state = self.inter.as_mut().ok_or(INTRA_FRAME)?;
        state.planned = None;
        let mvs = state
            .tile
            .predict(&leaf.frontier, motion, &mut self.recon.workspace)
            .map_err(rule("inter prediction"))?;
        let units = single_transform_residual(&leaf.frontier, chroma)
            .map_err(rule("inter residual"))?
            .ok_or(EncoderSyntaxError::Unsupported("an inter block here"))?;
        let mut blocks: Vec<TransformBlock> = units
            .into_iter()
            .map(|(plane, x, y, tx_size)| TransformBlock {
                tx_type: TxType::DctDct,
                plane,
                x,
                y,
                tx_size,
                fills_block: true,
            })
            .collect();
        state.planned = Some(PlannedInter {
            leaf: *leaf,
            motion,
            mvs,
            residual: EncoderInterResidual::default(),
        });
        if let Some(&luma) = blocks.first() {
            for chroma in blocks.iter_mut().skip(1) {
                chroma.tx_type = self.inter_chroma_tx_type(leaf, &luma, chroma)?;
            }
        }
        Ok(blocks)
    }

    /// The type of chroma block `chroma` of the planned inter leaf `leaf` whose
    /// luma block `luma` codes `luma.tx_type` (`DCT_DCT` when all zero).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid block.
    pub fn inter_chroma_tx_type(
        &self,
        leaf: &LeafBlock,
        luma: &TransformBlock,
        chroma: &TransformBlock,
    ) -> Result<TxType> {
        let collocated = chroma_inter_tx_type(
            &leaf.frontier,
            self.chroma,
            (luma.tx_size, luma.tx_type as usize),
            (chroma.x, chroma.y),
        )
        .map_err(rule("chroma transform type"))?;
        TxType::from_value(
            inter_chroma_plane_tx_type(
                chroma.plane,
                chroma.tx_size,
                collocated,
                self.tools.enable_chroma_dctonly,
            )
            .map_err(rule("chroma transform type"))?,
        )
    }

    /// Adds the residual of the planned inter leaf: `levels[i]` holds the
    /// row-major adjusted levels of `blocks[i]`, coded with its `tx_type`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned inter leaf.
    pub fn reconstruct_inter(
        &mut self,
        blocks: &[TransformBlock],
        levels: &[Vec<i32>],
    ) -> Result<()> {
        let state = self.inter.as_mut().ok_or(INTRA_FRAME)?;
        let planned = state.planned.as_mut().ok_or(NO_INTER_LEAF)?;
        planned.residual.clear();
        for (tx, levels) in blocks.iter().zip(levels) {
            let eob = super::coefficient_scan(tx)?
                .iter()
                .rposition(|&pos| levels.get(usize::from(pos)).is_some_and(|&l| l != 0))
                .map_or(0, |last| last + 1);
            let tx_type = if eob == 0 { TxType::DctDct } else { tx.tx_type };
            planned
                .residual
                .push(
                    (tx.plane, tx.x, tx.y, tx.tx_size),
                    levels,
                    eob,
                    tx_type as usize,
                )
                .map_err(rule("inter residual"))?;
        }
        state
            .tile
            .add_residual(&planned.residual, &mut self.recon.workspace)
            .map_err(rule("inter residual"))
    }

    /// Writes `skip_mode` (§ 5.20.5.9) and `is_inter` (§ 5.20.7.3) of `leaf`;
    /// an intra frame codes neither, an inter frame infers `is_inter = 1` for
    /// a skip-mode block and for a block whose chroma another block codes, and
    /// only a planned skip-mode leaf codes `skip_mode = 1`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when `leaf` cannot code the
    /// value, and [`EncoderSyntaxError::Rule`] for a symbol write failure.
    pub fn write_is_inter(
        &mut self,
        leaf: &LeafBlock,
        is_inter: bool,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let allowed = if is_inter {
            self.inter_allowed(leaf)
        } else {
            self.intra_allowed(leaf)
        };
        if !allowed {
            return Err(EncoderSyntaxError::Unsupported(
                "an is_inter value this block cannot code",
            ));
        }
        let Some(inter) = &self.inter else {
            return Ok(());
        };
        let frontier = &leaf.frontier;
        if frontier.is_luma_part() || frontier.is_chroma_part() {
            return Ok(());
        }
        let skip_mode = is_inter
            && inter
                .planned
                .as_ref()
                .is_some_and(|planned| planned.motion.skip_mode());
        let contexts = inter
            .tile
            .contexts(frontier, false)
            .map_err(rule("is_inter context"))?;
        let (_, reads_skip_mode) = inter
            .tile
            .reads_compound_syntax(frontier)
            .map_err(rule("skip_mode context"))?;
        if reads_skip_mode {
            let ctx = contexts.skip_mode;
            self.put(
                symbols,
                TileCdfSelector::SkipMode { ctx },
                usize::from(skip_mode),
            )?;
        } else if skip_mode {
            return Err(EncoderSyntaxError::Unsupported("skip mode here"));
        }
        if skip_mode || frontier.shared_mixed_chroma_ref_forces_inter() {
            return Ok(());
        }
        let ctx = contexts.is_inter;
        self.put(
            symbols,
            TileCdfSelector::IsInter { ctx },
            usize::from(is_inter),
        )
    }

    /// Writes `skip_txfm` of the planned inter leaf `leaf` (§ 5.20.7.4).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned inter leaf or for a
    /// write failure.
    pub fn write_skip_txfm(
        &mut self,
        leaf: &LeafBlock,
        skip: bool,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let state = self.inter_ref()?;
        let skip_mode = state
            .planned
            .as_ref()
            .ok_or(NO_INTER_LEAF)?
            .motion
            .skip_mode();
        let ctx = state
            .tile
            .contexts(&leaf.frontier, skip_mode)
            .map_err(rule("skip_txfm context"))?
            .skip;
        self.put(symbols, TileCdfSelector::Skip { ctx }, usize::from(skip))
    }

    /// Writes the mode syntax of the planned inter leaf `leaf`: `comp_mode`,
    /// the references, the mode, `jmvd_scale_mode` and the DRL indices
    /// (§ 5.20.7.6, § 5.20.7.11), or the skip-mode DRL index. Returns the
    /// motion vector differences the caller writes next, in coding order
    /// (§ 5.20.7.7, § 5.20.7.20).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned inter leaf.
    pub fn write_inter_modes(
        &mut self,
        leaf: &LeafBlock,
        symbols: &mut impl SymbolSink,
    ) -> Result<Vec<MotionVector>> {
        let state = self.inter_ref()?;
        let motion = state.planned.as_ref().ok_or(NO_INTER_LEAF)?.motion;
        let coded = state
            .tile
            .mode_symbols(&leaf.frontier, motion)
            .map_err(rule("inter mode syntax"))?;
        for (selector, value) in coded {
            self.put(symbols, selector, usize::from(value))?;
        }
        Ok(motion
            .coded_mvds()
            .into_iter()
            .map(MotionVector::from)
            .collect())
    }

    /// Writes the § 5.20.6.3 `tx_do_partition = 0` an inter leaf with a coded
    /// residual reads in a `TX_MODE_SELECT` frame.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid block or a write failure.
    pub fn write_inter_tx_partition(
        &mut self,
        leaf: &LeafBlock,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let size = leaf.frontier.b_size.index();
        if self.tools.tx_mode != Some(TxMode::Select) || size == BLOCK_4X4 {
            return Ok(());
        }
        let txfm_split_group = SIZE_TO_TX_PART_GROUP_LOOKUP
            .get(size)
            .and_then(|&group| usize::try_from(group).ok())
            .ok_or(EncoderSyntaxError::Unsupported("block size"))?;
        self.put(
            symbols,
            TileCdfSelector::TxDoPartition {
                fsc_mode: 0,
                is_inter: 1,
                txfm_split_group,
            },
            0,
        )
    }

    /// Publishes the coded inter leaf `leaf` into the neighbour, motion and
    /// `BlockDecoded` state.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when the decoder resolves
    /// another motion vector than the plan.
    pub fn publish_inter_leaf(&mut self, leaf: &LeafBlock, skip: bool) -> Result<()> {
        let state = self.inter.as_mut().ok_or(INTRA_FRAME)?;
        let planned = state.planned.take().ok_or(NO_INTER_LEAF)?;
        if planned.leaf != *leaf {
            return Err(NO_INTER_LEAF);
        }
        if skip {
            state
                .tile
                .reset_skip_contexts(&leaf.frontier, &mut self.coeff)
                .map_err(rule("coefficient contexts"))?;
        }
        let resolved = state
            .tile
            .publish(&leaf.frontier, planned.motion, skip)
            .map_err(rule("motion resolution"))?;
        if resolved != planned.mvs {
            return Err(EncoderSyntaxError::Unsupported(
                "a motion vector the decoder resolves differently",
            ));
        }
        self.publish_non_intra_modes(leaf)
    }

    /// Calls `read` with the reference plane view and the § 7.11.3.3
    /// parameters that predict the luma rectangle `(x, y, width, height)` from
    /// list reference `ref_frame` with `mv`; `splot_recon`'s sub-pixel kernels
    /// take both.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] in an intra frame or for a missing reference.
    pub fn with_luma_subpel<R>(
        &self,
        ref_frame: usize,
        rect: (usize, usize, usize, usize),
        mv: MotionVector,
        read: impl FnOnce(&ReferencePlaneView<'_, u16>, &SubpelPredictParams) -> R,
    ) -> Result<R> {
        let ref_frame = i8::try_from(ref_frame)
            .map_err(|_| EncoderSyntaxError::Unsupported("reference index"))?;
        self.inter_ref()?
            .tile
            .with_luma_subpel(ref_frame, rect, mv.into(), read)
            .map_err(rule("sub-pixel prediction"))
    }

    /// Publishes an intra leaf of an inter frame into the motion state.
    pub(super) fn publish_inter_neighbours(&mut self, leaf: &LeafBlock) -> Result<()> {
        match &mut self.inter {
            Some(inter) => inter
                .tile
                .publish_intra(&leaf.frontier)
                .map_err(rule("neighbour state")),
            None => Ok(()),
        }
    }

    /// Whether the planned leaf is an inter leaf.
    pub(super) fn leaf_is_inter(&self) -> bool {
        self.inter
            .as_ref()
            .is_some_and(|inter| inter.planned.is_some())
    }

    /// Drops the planned inter leaf.
    pub(super) fn clear_inter_plan(&mut self) {
        if let Some(inter) = &mut self.inter {
            inter.planned = None;
        }
    }

    /// Enters the superblock row of `row` before its first leaf.
    pub(super) fn enter_inter_superblock(&mut self, row: usize) {
        if let Some(inter) = &mut self.inter {
            inter.tile.enter_superblock(row);
        }
    }

    /// Saves the inter state of the luma rectangle `(x, y, width, height)`.
    pub(super) fn inter_snapshot(
        &self,
        (x, y, w, h): (usize, usize, usize, usize),
        since: Option<&InterSnapshot>,
    ) -> InterSnapshot {
        self.inter.as_ref().map(|inter| {
            inter.tile.checkpoint(
                (y / 4, x / 4, w.div_ceil(4), h.div_ceil(4)),
                since.and_then(Option::as_ref),
            )
        })
    }

    /// Puts an [`Self::inter_snapshot`] back.
    pub(super) fn inter_restore(&mut self, snapshot: &InterSnapshot) {
        if let (Some(inter), Some(checkpoint)) = (&mut self.inter, snapshot) {
            inter.tile.rollback(checkpoint);
            inter.planned = None;
        }
    }

    /// The deblocking and restoration records of the planned inter leaf.
    pub(super) fn record_inter_filters(
        &self,
        leaf: &LeafBlock,
        skip: bool,
        records: (
            &mut Vec<crate::filters::deblock::DeblockBlock>,
            &mut crate::filters::deblock::ChromaDeblockRecords,
            &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
        ),
    ) -> Result<()> {
        let state = self.inter_ref()?;
        let planned = state.planned.as_ref().ok_or(NO_INTER_LEAF)?;
        let residual = (!skip && !planned.residual.is_empty()).then_some(&planned.residual);
        state
            .tile
            .record_filters(&leaf.frontier, residual, records)
            .map_err(rule("filter records"))
    }

    fn publish_non_intra_modes(&mut self, leaf: &LeafBlock) -> Result<()> {
        super::super::partition_traversal::publish_intra_leaf_state::<core::convert::Infallible>(
            self.frame,
            leaf.call,
            &leaf.frontier,
            non_intra_leaf_mode(&leaf.frontier),
            leaf.call.b_size,
            &mut self.joint_modes,
            &mut self.fsc_modes,
            &mut self.use_dip,
            &mut self.uses_mrls,
            &mut self.palette_y,
            &mut self.uv_cfls,
            &mut self.y_modes,
            &mut self.mi_sizes,
        )
        .map_err(|error| match error {
            GeneralIntraTreeWalkError::Traversal(source) => rule("leaf publication")(source),
            GeneralIntraTreeWalkError::MiSize(source) => rule("leaf publication")(source),
            GeneralIntraTreeWalkError::Leaf(never) => match never {},
        })?;
        DecodedLeafPublication::new(leaf.call, leaf.call.b_size, self.sb_size4 - 1)
            .publish_block_decoded(&mut self.recon.block_decoded)
            .map_err(rule("BlockDecoded update"))
    }

    /// The decoder's single-reference syntax of `modes` at `leaf`.
    pub(super) fn single_motion(
        &mut self,
        leaf: &LeafBlock,
        modes: InterModes,
    ) -> Result<EncoderSingleMotion> {
        let ref_frame = i8::try_from(modes.ref_frame)
            .ok()
            .filter(|_| modes.ref_frame < self.reference_count())
            .ok_or(EncoderSyntaxError::Unsupported("reference index"))?;
        if modes.mode != InterMode::Global && modes.drl >= self.drl_count() {
            return Err(EncoderSyntaxError::Unsupported("RefMvIdx"));
        }
        let mode = match modes.mode {
            InterMode::Near => SINGLE_MODE_NEARMV,
            InterMode::Global => SINGLE_MODE_GLOBALMV,
            InterMode::New => SINGLE_MODE_NEWMV,
        };
        let mut motion = EncoderSingleMotion {
            ref_frame,
            mode,
            ref_mv_idx: if mode == SINGLE_MODE_GLOBALMV {
                0
            } else {
                modes.drl
            },
            mvd: Mv::default(),
        };
        if mode == SINGLE_MODE_NEWMV {
            let step = self.mv_step();
            let tile = &mut self.inter_state()?.tile;
            let base = tile
                .motion_vector(&leaf.frontier, motion)
                .map_err(rule("motion vector"))?;
            let mvd = Mv {
                row: modes.mv.row - base.row,
                col: modes.mv.col - base.col,
            };
            if mvd.row % step != 0 || mvd.col % step != 0 {
                return Err(EncoderSyntaxError::Unsupported(
                    "a NEWMV vector off the precision lattice",
                ));
            }
            motion.mvd = mvd;
        }
        Ok(motion)
    }

    pub(super) fn inter_ref(&self) -> Result<&InterLeafState> {
        self.inter.as_ref().ok_or(INTRA_FRAME)
    }

    pub(super) fn inter_state(&mut self) -> Result<&mut InterLeafState> {
        self.inter.as_mut().ok_or(INTRA_FRAME)
    }
}

const INTRA_FRAME: EncoderSyntaxError =
    EncoderSyntaxError::Unsupported("inter syntax in an intra frame");
const NO_INTER_LEAF: EncoderSyntaxError = EncoderSyntaxError::Unsupported("no planned inter leaf");
