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

use std::sync::Arc;

use splot_core::headers::frame::{FrameHeaderCore, FrameReferenceStateView};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::symbol_encoder::SymbolSink;
use splot_core::types::{EmbeddedLayerId, ObuType};
use splot_recon::{
    BitDepth, DecodedFrame, InterpolationFilter, ReferencePlaneView, SharedFrame,
    SubpelPredictParams,
};

use super::super::cdf::{
    FrameCdfSubset, TileCdfPolicyInput, TileCdfSelector, tile_cdf_save_policy,
};
use super::super::general_intra_residual::{
    inter_chroma_plane_tx_type, luma_transform_records_for_partition,
};
use super::super::partition_traversal::{DecodedLeafPublication, GeneralIntraTreeWalkError};
use super::compound::InterPrediction;
use super::{
    EncoderSyntaxError, FrameStart, IntraTileSyntax, LeafBlock, Result, TileEnd, TransformBlock,
    TxPartition, TxType, rule,
};
use crate::pipeline::inflight::RefFrameSlot;
use crate::pipeline::{frame_ref_update_from_core, is_key_or_switch};
use crate::prediction::inter::read_mv::AMVD_INDEX_TO_MVD;
use splot_recon::InterIntraMode;

pub use crate::prediction::inter::Bawp;

use crate::prediction::inter::{
    EncoderInterCheckpoint, EncoderInterResidual, EncoderInterTile, EncoderMotion,
    EncoderSingleMotion, EncoderSingleMvs, FrameCdfHandle, InterIntraPrediction,
    InterReferenceState, MotionFieldHandle, Mv, SINGLE_MODE_GLOBALMV, SINGLE_MODE_NEARMV,
    SINGLE_MODE_NEWMV, TIP_REF_FRAME, TemporalMvContext, chroma_inter_tx_type,
    encoder_motion_field, encoder_temporal_context, inter_residual_units, non_intra_leaf_mode,
    resolve_initial_frame_cdfs,
};
use crate::reference::buffer::RuntimeReferenceBuffer;

#[path = "tip_output.rs"]
mod tip_output;

/// `MV_PRECISION_EIGHTH_PEL`: motion vectors are in units of `1/8` sample.
pub const MV_PRECISION_EIGHTH_PEL: u8 = 6;
/// The nonzero magnitudes, in `1/8` samples, of an adaptive-MVD component
/// (§ 5.20.7.21).
pub const AMVD_STEPS: [i32; 8] = {
    let mut steps = [0; 8];
    let mut i = 0;
    while i < 8 {
        steps[i] = AMVD_INDEX_TO_MVD[i + 1];
        i += 1;
    }
    steps
};
/// The `interp_filter` values a block codes, in symbol order (§ 6.19.7.5).
pub const INTERP_FILTERS: [InterpolationFilter; 3] = [
    InterpolationFilter::EightTap,
    InterpolationFilter::EightTapSmooth,
    InterpolationFilter::EightTapSharp,
];

/// The decoder's § 7.23 reference buffer, filled with the encoder's filtered
/// reconstructions, saved CDFs and § 7.19 motion fields in coding order.
pub struct EncoderReferences {
    sequence: SequenceHeader,
    buffer: RuntimeReferenceBuffer,
    frames: Vec<Option<SettledFrame>>,
    state: InterReferenceState<u16>,
}

/// One coded frame: its filtered reconstruction, saved CDFs and motion field.
type SettledFrame = (
    SharedFrame<u16>,
    Arc<FrameCdfSubset>,
    Option<MotionFieldHandle>,
);

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
            sequence: seq.clone(),
            buffer,
            frames: Vec::new(),
            state,
        })
    }

    /// Stores the coded frame `core` with its filtered reconstruction `frame`,
    /// and the saved CDFs and motion field of its final `tiles`, in tile
    /// order, in the slots its `refresh_frame_flags` name (§ 7.23). `initial`
    /// is the [`Self::frame_start`] of `core`. A frame that refreshes no slot,
    /// or a sequence without `enable_ref_frame_mvs`, stores no motion field.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a header without the § 7.23 inputs or
    /// a tile count the header does not have.
    pub fn update(
        &mut self,
        core: &FrameHeaderCore,
        frame: DecodedFrame<u16>,
        initial: &FrameStart,
        tiles: &[TileEnd],
    ) -> Result<()> {
        let cdfs = Arc::new(saved_cdfs(&self.sequence, core, initial, tiles)?);
        let stores_motion = core.refresh_frame_flags.is_some_and(|flags| flags != 0)
            && self
                .sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_ref_frame_mvs);
        let motion = stores_motion
            .then(|| {
                encoder_motion_field(
                    &self.sequence,
                    core,
                    &self.state,
                    frame.bit_depth(),
                    tiles.iter().map(|tile| tile.motion.as_slice()),
                )
            })
            .transpose()
            .map_err(rule("motion field"))?;
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
        self.frames[index] = Some((SharedFrame::new(frame), cdfs, motion));
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

    /// Whether the frame `core` has the past/future reference pair TIP needs
    /// (§ 7.12.2); a frame without one must code `TIP_FRAME_DISABLED`, since
    /// the decoder reads `tip_mode` in every TIP frame.
    #[must_use]
    pub fn tip_pair_available(&self, core: &FrameHeaderCore) -> bool {
        crate::prediction::inter::tip_pair_available(core, &self.state)
    }

    /// The CDFs and projected motion field the frame `core` starts from
    /// (§ 5.18.2, § 7.9), which every tile of the frame and [`Self::update`]
    /// take.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] when the primary reference holds
    /// no saved CDFs or a reference holds no motion field.
    pub fn frame_start(&self, core: &FrameHeaderCore) -> Result<FrameStart> {
        Ok(FrameStart {
            cdfs: resolve_initial_frame_cdfs(
                core,
                &self.sequence,
                &self.state,
                None,
                ByteOffset::new(0),
            )
            .map_err(rule("initial CDFs"))?,
            temporal: encoder_temporal_context(
                &self.sequence,
                core,
                &self.state,
                super::intra::sequence_bit_depth(&self.sequence),
            )
            .map_err(rule("temporal motion field"))?,
        })
    }

    /// A fresh reference state for one tile.
    fn tile_state(&self) -> Result<InterReferenceState<u16>> {
        settled_state(&self.buffer, &self.frames)
    }
}

/// The § 7.23 saved CDFs of the frame `core` of `sequence` that started from
/// `initial` and whose tiles ended with `tiles`.
fn saved_cdfs(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    initial: &FrameStart,
    tiles: &[TileEnd],
) -> Result<FrameCdfSubset> {
    let info = core
        .tile_info
        .as_ref()
        .ok_or(EncoderSyntaxError::Unsupported("missing tile info"))?;
    let (enable_avg_cdf, avg_cdf_type) = sequence
        .transform_quant_entropy
        .as_ref()
        .map_or((false, false), |tq| {
            (tq.enable_avg_cdf, tq.avg_cdf_type != 0)
        });
    let input = TileCdfPolicyInput::new(
        info.tile_cols,
        info.tile_rows,
        enable_avg_cdf,
        avg_cdf_type,
        info.context_update_tile_id,
    );
    if tiles.len() != super::tile_count(core) {
        return Err(EncoderSyntaxError::Unsupported(
            "a tile count the frame does not have",
        ));
    }
    let policies = (0..tiles.len() as u32)
        .map(|tile| tile_cdf_save_policy(input, tile).map(|policy| (tile, policy)))
        .collect::<core::result::Result<Vec<_>, _>>()
        .map_err(rule("tile CDF save policy"))?;
    let mut saved = FrameCdfSubset::from_defaults();
    saved.fold_frame_end(
        &initial.cdfs,
        policies
            .into_iter()
            .zip(tiles)
            .map(|((tile, policy), end)| (tile, &end.cdfs, policy)),
    );
    Ok(saved)
}

/// The reference state of `buffer` over settled `frames` with their saved
/// CDFs and motion fields.
fn settled_state(
    buffer: &RuntimeReferenceBuffer,
    frames: &[Option<SettledFrame>],
) -> Result<InterReferenceState<u16>> {
    let (store, mut meta) = buffer
        .build_store_from(|slot, index| {
            let (frame, ..) = frames
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(crate::error::DecodeReferenceStateError::MissingFrame { slot })?;
            Ok((RefFrameSlot::completed(frame.share()), None))
        })
        .map_err(rule("reference state"))?;
    for (slot, (cdfs, motion)) in meta
        .ref_frame_cdfs
        .iter_mut()
        .zip(meta.ref_motion_fields.iter_mut())
        .enumerate()
    {
        if let Some((_, saved, field)) = buffer
            .frame_index_for_slot(slot as u32)
            .ok()
            .and_then(|index| frames.get(index)?.as_ref())
        {
            *cdfs = Some(FrameCdfHandle::settled(Arc::clone(saved)));
            motion.clone_from(field);
        }
    }
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
    /// `interp_filter`, one of [`INTERP_FILTERS`]; a block that reads no
    /// filter (see [`IntraTileSyntax::price_interp_filter`]) ignores it.
    pub filter: InterpolationFilter,
    /// The block `MvPrecision` of a `NEWMV` below the frame precision, one of
    /// [`IntraTileSyntax::newmv_precisions`]; `None` codes the frame
    /// precision. A `NEWMV` vector is its base plus a multiple of
    /// [`IntraTileSyntax::newmv_step`] per component.
    pub precision: Option<u8>,
    /// `use_amvd` of a `NEWMV` (see [`IntraTileSyntax::adaptive_mvd`]): each
    /// difference component is zero or `±` an [`AMVD_STEPS`] entry over the
    /// unrounded base.
    pub amvd: bool,
    /// The inter-intra prediction, where [`IntraTileSyntax::interintra_reads`]
    /// says the block reads it.
    pub interintra: Option<InterIntra>,
    /// The block adaptive weighted prediction, where
    /// [`IntraTileSyntax::bawp_reads`] says the block reads it; a BAWP block
    /// codes no inter-intra.
    pub bawp: Option<Bawp>,
}

/// `INTERINTRA_MODES` (§ 3): the `interintra_mode` values.
pub const INTERINTRA_MODES: u8 = 4;

/// The inter-intra prediction of a single-reference block (§ 5.20.7.15).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InterIntra {
    /// `interintra_mode`, below [`INTERINTRA_MODES`]: `II_DC_PRED`,
    /// `II_V_PRED`, `II_H_PRED` or `II_SMOOTH_PRED`.
    pub mode: u8,
    /// `WedgeIndex` of a block that codes `wedge_interintra = 1`.
    pub wedge: Option<u8>,
}

impl InterIntra {
    fn prediction(self) -> Result<InterIntraPrediction> {
        let mode = match self.mode {
            0 => InterIntraMode::Dc,
            1 => InterIntraMode::Vertical,
            2 => InterIntraMode::Horizontal,
            3 => InterIntraMode::Smooth,
            _ => return Err(EncoderSyntaxError::Unsupported("interintra_mode")),
        };
        Ok(match self.wedge {
            Some(wedge_index) => InterIntraPrediction::WedgeMask { mode, wedge_index },
            None => InterIntraPrediction::SmoothMask { mode },
        })
    }
}

/// The single-reference vectors of one leaf and list reference, see
/// [`IntraTileSyntax::single_mvs`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SingleMvs {
    /// The `GLOBALMV` vector.
    pub global: MotionVector,
    /// The `NEARMV` vector of each `RefMvIdx` below
    /// [`IntraTileSyntax::drl_count`].
    pub near: Vec<MotionVector>,
    /// The `NEWMV` base of each `RefMvIdx`, see [`IntraTileSyntax::newmv_base`].
    pub new_bases: Vec<MotionVector>,
    /// The § 7.12 stack in DRL order: spatial, temporal and bank candidates.
    pub stack: Vec<MotionVector>,
}

/// The inter state of one tile, `None` in an intra frame without intra block
/// copy.
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
    /// `use_intrabc`: an IntraBC block transforms without DDT (§ 7.15.4).
    use_intrabc: bool,
}

/// The saved inter state of one region.
pub(super) type InterSnapshot = Option<EncoderInterCheckpoint>;

impl InterLeafState {
    /// The motion of the planned inter leaf.
    pub(super) fn planned_motion(&self) -> Option<EncoderMotion> {
        self.planned.as_ref().map(|planned| planned.motion)
    }

    /// `None` for an intra frame.
    pub(super) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        refs: &EncoderReferences,
        temporal: Option<Arc<TemporalMvContext>>,
        tile: (core::ops::Range<usize>, core::ops::Range<usize>),
        (qindex, bit_depth): (u32, BitDepth),
    ) -> Result<Option<Self>> {
        if core.frame_is_intra == Some(true) {
            if core.allow_intrabc != Some(true) {
                return Ok(None);
            }
            let tile = EncoderInterTile::new_intrabc(
                seq,
                core,
                refs.tile_state()?,
                tile,
                (qindex, bit_depth),
            )
            .map_err(EncoderSyntaxError::Unsupported)?;
            return Ok(Some(Self {
                tile,
                planned: None,
            }));
        }
        let tile = EncoderInterTile::new(
            seq,
            core,
            refs.tile_state()?,
            temporal,
            tile,
            (qindex, bit_depth),
        )
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
        !self.inter_frame() || !leaf.frontier.shared_mixed_chroma_ref_forces_inter()
    }

    /// Whether `leaf` may code a single-reference inter block.
    #[must_use]
    pub fn inter_allowed(&self, leaf: &LeafBlock) -> bool {
        self.inter_frame() && !leaf.frontier.is_luma_part() && !leaf.frontier.is_chroma_part()
    }

    /// Whether `leaf` may predict from the TIP frame (§ 7.10.6): an
    /// [`InterModes`] whose `ref_frame` is [`Self::reference_count`] then
    /// codes `tip_mode` with `NEARMV` or `NEWMV` over the TIP stack.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid block size.
    pub fn tip_allowed(&self, leaf: &LeafBlock) -> Result<bool> {
        match &self.inter {
            Some(inter) if self.inter_allowed(leaf) => inter
                .tile
                .tip_allowed(&leaf.frontier)
                .map_err(rule("TIP block size")),
            _ => Ok(false),
        }
    }

    /// The luma transform blocks of the TIP `NEARMV` prediction of `leaf` with
    /// `tip_global_wtd_index = weight` (§ 7.13.3.1), each with its row-major
    /// samples, for a frame-level weight choice. The prediction overwrites the
    /// leaf's samples and the planned leaf, so call it before the search of
    /// the tile.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] where `leaf` cannot code TIP or for a
    /// weight index above 7.
    pub fn tip_prediction(
        &mut self,
        leaf: &LeafBlock,
        weight: u8,
    ) -> Result<Vec<(TransformBlock, Vec<u16>)>> {
        if !self.tip_allowed(leaf)? || weight > 7 {
            return Err(EncoderSyntaxError::Unsupported("TIP prediction here"));
        }
        let modes = InterModes {
            ref_frame: self.reference_count(),
            mode: InterMode::Near,
            drl: 0,
            mv: MotionVector::default(),
            filter: InterpolationFilter::EightTap,
            precision: None,
            amvd: false,
            interintra: None,
            bawp: None,
        };
        let previous = self.inter_state()?.tile.set_tip_weight(Some(weight));
        let predicted = self.luma_prediction(leaf, InterPrediction::Single(modes));
        self.inter_state()?.tile.set_tip_weight(previous);
        predicted
    }

    /// The luma transform blocks of the decoder's prediction of `prediction`
    /// at `leaf`, refinement included, each with its row-major samples. The
    /// prediction overwrites the leaf's samples and clears the planned leaf.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction `leaf` cannot code.
    pub fn luma_prediction(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
    ) -> Result<Vec<(TransformBlock, Vec<u16>)>> {
        let planned = self.plan_inter_leaf(leaf, prediction);
        let mut out = Vec::new();
        for tx in planned?.into_iter().filter(|tx| tx.plane == 0) {
            out.push((tx, self.reconstruction(&tx)?));
        }
        self.clear_inter_plan();
        Ok(out)
    }

    /// Whether a single-reference leaf `leaf` reads `inter_intra`, and
    /// whether it reads `wedge_interintra` (§ 5.20.7.14, § 5.20.7.15).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] in an intra frame.
    pub fn interintra_reads(&self, leaf: &LeafBlock) -> Result<(bool, bool)> {
        self.inter_ref()?
            .tile
            .interintra_reads(&leaf.frontier)
            .map_err(rule("inter_intra"))
    }

    /// Whether the single-reference leaf `leaf` coding `modes` reads
    /// `use_bawp` (§ 5.20.7.6); `modes` itself is coded without BAWP.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code.
    pub fn bawp_reads(&mut self, leaf: &LeafBlock, modes: InterModes) -> Result<bool> {
        let single = self.single_motion(
            leaf,
            InterModes {
                bawp: None,
                interintra: None,
                ..modes
            },
        )?;
        self.inter_ref()?
            .tile
            .bawp_reads(&leaf.frontier, single)
            .map_err(rule("use_bawp"))
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

    /// The `NEWMV` difference step of `modes` in `1/8` samples: the step of
    /// its block precision.
    #[must_use]
    pub fn newmv_step(&self, modes: &InterModes) -> i32 {
        let precision = modes.precision.unwrap_or_else(|| self.mv_precision());
        1 << MV_PRECISION_EIGHTH_PEL.saturating_sub(precision)
    }

    /// The block precisions below the frame precision that a `NEWMV` at
    /// `leaf` can code, finest first; empty without `enable_flex_mvres`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] in an intra frame.
    pub fn newmv_precisions(&self, leaf: &LeafBlock) -> Result<Vec<u8>> {
        self.inter_ref()?
            .tile
            .newmv_precisions(&leaf.frontier)
            .map_err(rule("block MV precision"))
    }

    /// Whether a `NEWMV` may code adaptive MVD (`enable_adaptive_mvd`).
    #[must_use]
    pub fn adaptive_mvd(&self) -> bool {
        self.inter
            .as_ref()
            .is_some_and(|inter| inter.tile.enable_adaptive_mvd())
    }

    /// The `NEWMV` base vector of the reference, DRL index, block precision
    /// and `amvd` of `modes` at `leaf`: a `NEWMV` vector is this plus a
    /// multiple of [`Self::newmv_step`] per component, or with `amvd` plus
    /// zero or `±` an [`AMVD_STEPS`] entry per component.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a reference or index `leaf` cannot code.
    pub fn newmv_base(&mut self, leaf: &LeafBlock, modes: InterModes) -> Result<MotionVector> {
        let near = InterModes {
            mode: InterMode::Near,
            ..modes
        };
        let motion = EncoderSingleMotion {
            mode: SINGLE_MODE_NEWMV,
            precision: modes.precision,
            use_amvd: modes.amvd,
            ..self.single_motion(leaf, near)?
        };
        let tile = &mut self.inter_state()?.tile;
        tile.motion_vector(&leaf.frontier, motion)
            .map(MotionVector::from)
            .map_err(rule("motion vector"))
    }

    /// The vectors the single-reference modes code at `leaf` from list
    /// reference `ref_frame`, and the § 7.12 stack they come from, as the
    /// decoder resolves them from one stack.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a reference `leaf` cannot code.
    pub fn single_mvs(&mut self, leaf: &LeafBlock, ref_frame: usize) -> Result<SingleMvs> {
        let ref_frame = if ref_frame == self.reference_count() && self.tip_allowed(leaf)? {
            TIP_REF_FRAME
        } else {
            i8::try_from(ref_frame)
                .ok()
                .filter(|_| ref_frame < self.reference_count())
                .ok_or(EncoderSyntaxError::Unsupported("reference index"))?
        };
        let drls = self.drl_count();
        let tile = &mut self.inter_state()?.tile;
        let mvs: EncoderSingleMvs = tile
            .single_mvs(&leaf.frontier, ref_frame, drls)
            .map_err(rule("motion vector stack"))?;
        let list = |mvs: Vec<Mv>| mvs.into_iter().map(MotionVector::from).collect();
        Ok(SingleMvs {
            global: mvs.global.into(),
            near: list(mvs.near),
            new_bases: list(mvs.new_bases),
            stack: list(mvs.stack),
        })
    }

    /// Plans the inter leaf `leaf` under `prediction`: predicts it into the
    /// frame and returns its transform blocks in § 5.20.7.23 order, each 64x64
    /// luma chunk before its chroma. Chroma blocks hold the type `DCT_DCT`
    /// luma blocks imply; see [`Self::inter_chroma_tx_type`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction `leaf` cannot code.
    pub fn plan_inter_leaf(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
    ) -> Result<Vec<TransformBlock>> {
        self.plan_inter_leaf_partitioned(leaf, prediction, TxPartition::None)
    }

    /// [`Self::plan_inter_leaf`] with the luma transform split by `partition`,
    /// one of [`Self::luma_tx_partitions`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction or a partition the leaf
    /// cannot code.
    pub fn plan_inter_leaf_partitioned(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
        partition: TxPartition,
    ) -> Result<Vec<TransformBlock>> {
        let split = self.partitioned_tx_size(leaf, partition)?;
        if !self.inter_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported("an inter block here"));
        }
        let motion = self.motion(leaf, prediction)?;
        self.plan_motion_leaf(leaf, (motion, None), (partition, split))
    }

    /// Plans the leaf `leaf` of `motion` with the luma transform split by
    /// `partition` into `split`: an inter prediction, or with `intrabc` the
    /// intra block copy of that block vector, whose `motion` names the
    /// IntraBC reference.
    pub(super) fn plan_motion_leaf(
        &mut self,
        leaf: &LeafBlock,
        (motion, intrabc): (EncoderMotion, Option<Mv>),
        (partition, split): (TxPartition, Option<usize>),
    ) -> Result<Vec<TransformBlock>> {
        let (mi_rows, mi_cols) = self.mi_dimensions();
        self.recon.leaf = None;
        self.plan_cctx(leaf);
        self.fsc_leaf = false;
        let chroma = self.chroma;
        let state = self.inter.as_mut().ok_or(INTRA_FRAME)?;
        state.planned = None;
        let workspace = &mut self.recon.workspace;
        let mvs = match intrabc {
            None => state.tile.predict(
                &leaf.frontier,
                motion,
                workspace,
                (&self.recon.block_decoded, self.tools.enable_ibp),
            ),
            Some(mv) => (state.tile.predict_intrabc(&leaf.frontier, mv, workspace))
                .map(|()| [mv, Mv::default()]),
        }
        .map_err(rule("inter prediction"))?;
        let units = inter_residual_units(&leaf.frontier, chroma).map_err(rule("inter residual"))?;
        let (sub_x, sub_y) = super::super::tile_frontier::chroma_subsampling(chroma);
        let mut blocks: Vec<TransformBlock> = units
            .into_iter()
            .filter(|&(plane, x, y, ..)| {
                let (sx, sy) = if plane > 0 {
                    (sub_x, sub_y)
                } else {
                    (false, false)
                };
                x < (mi_cols * 4) >> u8::from(sx) && y < (mi_rows * 4) >> u8::from(sy)
            })
            .map(|(plane, x, y, tx_size, fills_block)| TransformBlock {
                tx_type: TxType::DctDct,
                plane,
                x,
                y,
                tx_size,
                fills_block,
                ist: super::Ist::OFF,
                cctx: 0,
            })
            .collect();
        if let (Some(tx_size), true) = (split, partition != TxPartition::None) {
            let records = luma_transform_records_for_partition(
                leaf.frontier.c * 4,
                leaf.frontier.r * 4,
                tx_size,
                partition,
            )
            .map_err(rule("transform partition"))?;
            let luma = records
                .into_iter()
                .filter(|record| record.y / 4 < mi_rows && record.x / 4 < mi_cols)
                .map(|record| TransformBlock {
                    tx_type: TxType::DctDct,
                    plane: 0,
                    x: record.x,
                    y: record.y,
                    tx_size: record.tx_size,
                    fills_block: false,
                    ist: super::Ist::OFF,
                    cctx: 0,
                });
            blocks.splice(..1, luma);
        }
        state.planned = Some(PlannedInter {
            leaf: *leaf,
            motion,
            mvs,
            residual: EncoderInterResidual::default(),
            use_intrabc: intrabc.is_some(),
        });
        for index in 0..blocks.len() {
            if blocks[index].plane > 0 {
                blocks[index].tx_type =
                    self.inter_chroma_tx_type(leaf, &blocks[..index], &blocks[index])?;
            }
        }
        Ok(blocks)
    }

    /// The type of chroma block `chroma` of the planned inter leaf `leaf`
    /// after its blocks `before`, whose luma blocks each code their `tx_type`
    /// (`DCT_DCT` when all zero).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid block.
    pub fn inter_chroma_tx_type(
        &self,
        leaf: &LeafBlock,
        before: &[TransformBlock],
        chroma: &TransformBlock,
    ) -> Result<TxType> {
        let collocated = chroma_inter_tx_type(
            &leaf.frontier,
            self.chroma,
            before
                .iter()
                .filter(|tx| tx.plane == 0)
                .map(|tx| (tx.x, tx.y, tx.tx_size, tx.tx_type as usize)),
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
        let mut residual = Vec::with_capacity(blocks.len());
        for (tx, levels) in blocks.iter().zip(levels) {
            residual.push((self.decoder_quant(tx, levels)?, self.uses_tcq(tx)));
        }
        let state = self.inter.as_mut().ok_or(INTRA_FRAME)?;
        let planned = state.planned.as_mut().ok_or(NO_INTER_LEAF)?;
        planned.residual.clear();
        for (tx, (levels, use_tcq)) in blocks.iter().zip(residual) {
            let eob = super::coefficient_scan(tx)?
                .iter()
                .rposition(|&pos| levels.get(usize::from(pos)).is_some_and(|&l| l != 0))
                .map_or(0, |last| last + 1);
            let tx_type = if eob == 0 { TxType::DctDct } else { tx.tx_type };
            planned
                .residual
                .push(
                    (tx.plane, tx.x, tx.y, tx.tx_size),
                    &levels,
                    eob,
                    (tx_type as usize, use_tcq),
                    tx.ist.syntax(true).filter(|_| eob > 0),
                )
                .map_err(rule("inter residual"))?;
        }
        state
            .tile
            .add_residual(
                &planned.residual,
                planned.use_intrabc,
                &mut self.recon.workspace,
            )
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
        let (true, Some(inter)) = (self.inter_frame(), &self.inter) else {
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

    /// Prices into `symbols` the mode syntax and the motion vector difference
    /// that the single-reference `modes` code at `leaf`, as
    /// [`Self::write_inter_modes`] and [`Self::write_mvds`] write them for a
    /// planned leaf, then puts the CDFs back. The leaf is not planned. A
    /// `NEWMV` takes `base`, its [`SingleMvs::new_bases`] entry.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code.
    pub fn price_single_modes(
        &mut self,
        leaf: &LeafBlock,
        (modes, base): (InterModes, MotionVector),
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let mark = self.journal.mark();
        let written = self.write_single_modes(leaf, (modes, base), symbols);
        self.journal.rollback(&mut self.cdfs, mark)?;
        written
    }

    fn write_single_modes(
        &mut self,
        leaf: &LeafBlock,
        (modes, base): (InterModes, MotionVector),
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let motion =
            EncoderMotion::Single(self.single_motion_over(leaf, modes, Some(base.into()))?);
        self.write_motion_modes(leaf, motion, symbols)
    }

    /// Prices into `symbols` the mode syntax and the motion vector
    /// differences that `prediction` codes at `leaf`, as
    /// [`Self::write_inter_modes`] and [`Self::write_mvds`] write them for a
    /// planned leaf, then puts the CDFs back. The leaf is not planned.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction `leaf` cannot code.
    pub fn price_inter_modes(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let mark = self.journal.mark();
        let written = self
            .motion(leaf, prediction)
            .and_then(|motion| self.write_motion_modes(leaf, motion, symbols));
        self.journal.rollback(&mut self.cdfs, mark)?;
        written
    }

    fn write_motion_modes(
        &mut self,
        leaf: &LeafBlock,
        motion: EncoderMotion,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let coded = self
            .inter_ref()?
            .tile
            .mode_symbols(&leaf.frontier, motion)
            .map_err(rule("inter mode syntax"))?;
        for (selector, value) in coded {
            self.put(symbols, selector, usize::from(value))?;
        }
        let mvds: Vec<MotionVector> = motion
            .coded_mvds()
            .into_iter()
            .map(MotionVector::from)
            .collect();
        let coding = self
            .inter_ref()?
            .tile
            .mvd_coding(motion)
            .map_err(rule("motion vector difference"))?;
        super::mv::write_mvds((&mut self.cdfs, &mut self.journal), coding, &mvds, symbols)
    }

    /// Prices into `symbols` the `use_refinemv`, the compound type and the
    /// `interp_filter` that `prediction` codes at `leaf` (§ 5.20.7.17,
    /// § 5.20.7.16, § 5.20.7.6), or the warp
    /// deltas and `warp_inter_intra` of a warp block (§ 5.20.7.7,
    /// § 5.20.7.15), as [`Self::write_interp_filter`] writes them, then puts
    /// the CDFs back. A warp block reads no `interp_filter`.
    /// Returns whether the decoder reads `interp_filter`: a skip-mode block,
    /// a TIP block, a refined compound block, a large global block and a
    /// frame without `SWITCHABLE` filters read none.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a prediction `leaf` cannot code or a
    /// filter outside [`INTERP_FILTERS`].
    pub fn price_interp_filter(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
        symbols: &mut impl SymbolSink,
    ) -> Result<bool> {
        let (refs, global, filter) = match prediction {
            InterPrediction::Single(modes) if modes.ref_frame == self.reference_count() => {
                return Ok(false);
            }
            InterPrediction::Single(modes) => (
                (modes.ref_frame, false),
                modes.mode == InterMode::Global,
                modes.filter,
            ),
            InterPrediction::Compound(_) => {
                return self.price_compound_tail(leaf, prediction, symbols);
            }
            InterPrediction::SkipMode { .. } => return Ok(false),
            InterPrediction::Warp(_) => {
                let motion = self.motion(leaf, prediction)?;
                let EncoderMotion::Warp(warp) = motion else {
                    return Err(NO_INTER_LEAF);
                };
                let tail = (self.inter_ref()?.tile)
                    .warp_tail_symbols(&leaf.frontier, warp)
                    .map_err(rule("warp syntax"))?;
                let mark = self.journal.mark();
                let written = tail.into_iter().try_for_each(|(selector, value)| {
                    self.put(symbols, selector, usize::from(value))
                });
                self.journal.rollback(&mut self.cdfs, mark)?;
                return written.map(|()| false);
            }
        };
        let ref_frame0 =
            i8::try_from(refs.0).map_err(|_| EncoderSyntaxError::Unsupported("reference index"))?;
        let selector = self
            .inter_ref()?
            .tile
            .interp_filter_selector(&leaf.frontier, (ref_frame0, refs.1), global)
            .map_err(rule("interp_filter context"))?;
        let Some(selector) = selector else {
            return Ok(false);
        };
        let value = INTERP_FILTERS
            .iter()
            .position(|&f| f == filter)
            .ok_or(EncoderSyntaxError::Unsupported("interpolation filter"))?;
        let mark = self.journal.mark();
        let written = self.put(symbols, selector, value);
        self.journal.rollback(&mut self.cdfs, mark)?;
        written.map(|()| true)
    }

    /// Writes the `use_refinemv`, the compound type and the `interp_filter`
    /// of the planned inter leaf `leaf`, or the warp deltas and
    /// `warp_inter_intra` of a warp leaf, after its motion vector
    /// differences (§ 5.20.7.17, § 5.20.7.16, § 5.20.7.6,
    /// § 5.20.7.7, § 5.20.7.15); a leaf that reads none writes nothing.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned inter leaf or for a
    /// write failure.
    pub fn write_interp_filter(
        &mut self,
        leaf: &LeafBlock,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let state = self.inter_ref()?;
        let motion = state.planned.as_ref().ok_or(NO_INTER_LEAF)?.motion;
        let refinemv = state
            .tile
            .refinemv_symbol(&leaf.frontier, motion)
            .map_err(rule("use_refinemv"))?;
        let (_, interp) = state
            .tile
            .interp_filter(&leaf.frontier, motion)
            .map_err(rule("interp_filter"))?;
        let compound_type = state
            .tile
            .compound_type_tail(&leaf.frontier, motion)
            .map_err(rule("compound type"))?;
        let tail = match motion {
            EncoderMotion::Warp(warp) => state
                .tile
                .warp_tail_symbols(&leaf.frontier, warp)
                .map_err(rule("warp syntax"))?,
            _ => Vec::new(),
        };
        let coded = refinemv
            .map(|(selector, value)| (Some(selector), value))
            .into_iter()
            .chain(compound_type)
            .chain(interp.map(|(selector, value)| (Some(selector), value)))
            .chain(
                tail.into_iter()
                    .map(|(selector, value)| (Some(selector), value)),
            );
        for element in coded {
            self.put_element(symbols, element)?;
        }
        Ok(())
    }

    /// Writes one `S()` symbol, or with no selector one `L(1)` literal.
    fn put_element(
        &mut self,
        symbols: &mut impl SymbolSink,
        (selector, value): (Option<TileCdfSelector>, u8),
    ) -> Result<()> {
        match selector {
            Some(selector) => self.put(symbols, selector, usize::from(value)),
            None => symbols
                .literal(u32::from(value), 1)
                .map_err(rule("symbol write")),
        }
    }

    /// [`Self::price_interp_filter`] of a compound `prediction`.
    fn price_compound_tail(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
        symbols: &mut impl SymbolSink,
    ) -> Result<bool> {
        let motion = self.motion(leaf, prediction)?;
        let tile = &self.inter_ref()?.tile;
        let refinemv = tile
            .refinemv_symbol(&leaf.frontier, motion)
            .map_err(rule("use_refinemv"))?;
        let compound_type = tile
            .compound_type_tail(&leaf.frontier, motion)
            .map_err(rule("compound type"))?;
        let (_, interp) = tile
            .interp_filter(&leaf.frontier, motion)
            .map_err(rule("interp_filter"))?;
        let mut coded = refinemv
            .map(|(selector, value)| (Some(selector), value))
            .into_iter()
            .chain(compound_type)
            .chain(interp.map(|(selector, value)| (Some(selector), value)));
        let mark = self.journal.mark();
        let written = coded.try_for_each(|element| self.put_element(symbols, element));
        self.journal.rollback(&mut self.cdfs, mark)?;
        written.map(|()| interp.is_some())
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
            .publish(
                &leaf.frontier,
                planned.motion,
                skip,
                &mut self.recon.workspace,
            )
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
    /// list reference `ref_frame` with `mv` and `filter`; `splot_recon`'s
    /// sub-pixel kernels take both.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] in an intra frame or for a missing reference.
    pub fn with_luma_subpel<R>(
        &self,
        ref_frame: usize,
        rect: (usize, usize, usize, usize),
        (mv, filter): (MotionVector, InterpolationFilter),
        read: impl FnOnce(&ReferencePlaneView<'_, u16>, &SubpelPredictParams) -> R,
    ) -> Result<R> {
        let ref_frame = i8::try_from(ref_frame)
            .map_err(|_| EncoderSyntaxError::Unsupported("reference index"))?;
        self.inter_ref()?
            .tile
            .with_luma_subpel(ref_frame, rect, (mv.into(), filter), read)
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

    /// Whether the planned inter leaf is an IntraBC block.
    pub(super) fn leaf_uses_intrabc(&self) -> bool {
        self.inter
            .as_ref()
            .and_then(|inter| inter.planned.as_ref())
            .is_some_and(|planned| planned.use_intrabc)
    }

    /// Whether the planned leaf is an inter leaf.
    #[must_use]
    pub fn leaf_is_inter(&self) -> bool {
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
            .record_filters(&leaf.frontier, (planned.motion, residual), records)
            .map_err(rule("filter records"))
    }

    fn publish_non_intra_modes(&mut self, leaf: &LeafBlock) -> Result<()> {
        self.publish_leaf_modes(leaf, non_intra_leaf_mode(&leaf.frontier))
    }

    /// Publishes the leaf modes `mode` of the non-intra leaf `leaf`.
    pub(super) fn publish_leaf_modes(
        &mut self,
        leaf: &LeafBlock,
        mode: super::super::partition_traversal::GeneralIntraLeafMode,
    ) -> Result<()> {
        super::super::partition_traversal::publish_intra_leaf_state::<core::convert::Infallible>(
            self.frame,
            leaf.call,
            &leaf.frontier,
            mode,
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
        self.single_motion_over(leaf, modes, None)
    }

    /// [`Self::single_motion`] whose `NEWMV` takes `base`, else the base the
    /// decoder resolves.
    fn single_motion_over(
        &mut self,
        leaf: &LeafBlock,
        modes: InterModes,
        base: Option<Mv>,
    ) -> Result<EncoderSingleMotion> {
        let tip = modes.ref_frame == self.reference_count();
        if tip && (modes.mode == InterMode::Global || !self.tip_allowed(leaf)?) {
            return Err(EncoderSyntaxError::Unsupported("TIP here"));
        }
        let ref_frame = if tip {
            TIP_REF_FRAME
        } else {
            i8::try_from(modes.ref_frame)
                .ok()
                .filter(|_| modes.ref_frame < self.reference_count())
                .ok_or(EncoderSyntaxError::Unsupported("reference index"))?
        };
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
            interp: modes.filter,
            precision: modes.precision.filter(|_| mode == SINGLE_MODE_NEWMV),
            use_amvd: modes.amvd && mode == SINGLE_MODE_NEWMV,
            tip,
            interintra: modes.interintra.map(InterIntra::prediction).transpose()?,
            bawp: modes.bawp,
        };
        if mode == SINGLE_MODE_NEWMV {
            let step = self.newmv_step(&modes);
            let base = match base {
                Some(base) => base,
                None => self
                    .inter_state()?
                    .tile
                    .motion_vector(&leaf.frontier, motion)
                    .map_err(rule("motion vector"))?,
            };
            let mvd = Mv {
                row: modes.mv.row - base.row,
                col: modes.mv.col - base.col,
            };
            let amvd = |d: i32| d == 0 || AMVD_STEPS.contains(&d.abs());
            let on_lattice = if modes.amvd {
                amvd(mvd.row) && amvd(mvd.col)
            } else {
                mvd.row % step == 0 && mvd.col % step == 0
            };
            if !on_lattice {
                return Err(EncoderSyntaxError::Unsupported(
                    "a NEWMV vector off the precision lattice",
                ));
            }
            motion.mvd = mvd;
        }
        Ok(motion)
    }

    /// Whether the frame is an inter frame.
    pub(super) fn inter_frame(&self) -> bool {
        self.inter
            .as_ref()
            .is_some_and(|inter| !inter.tile.is_intra_frame())
    }

    /// Takes the planned IntraBC leaf `leaf` and returns its block vector.
    pub(super) fn take_planned_intrabc(&mut self, leaf: &LeafBlock) -> Result<Mv> {
        let state = self.inter_state()?;
        match state.planned.take() {
            Some(planned) if planned.use_intrabc && planned.leaf == *leaf => Ok(planned.mvs[0]),
            _ => Err(NO_INTER_LEAF),
        }
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
