// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Single-reference, compound and skip-mode inter leaves of the encoder
//! syntax facade.
//!
//! [`EncoderInterTile`] keeps the decoder's neighbour grid, banks and § 7.9
//! projected motion field for one tile and runs the decoder's own § 7.12
//! stack, § 7.12 resolution, § 7.11.3 motion compensation, inter residual
//! reconstruction and filter records, so the facade never derives motion,
//! prediction or residual geometry itself.

use super::super::compound::CompoundYMode;
use super::super::find_mv_stack::{
    GridCheckpoint, ModeContext, OrderHintMvContext, RefMvBank, WarpParamBank,
    reference_order_hints,
};
use splot_core::headers::sequence::{SequencePartitionConfig, SuperblockSize};

use super::filter_records::record_inter_deblock_geometry;
use super::residual::{InterLumaTxTypeMap, push_inter_residual_block};
use super::resolve::{
    InterBlockSyntax, InterMotionSyntax, MvResolutionState, ResolvedInterBlock,
    compound_inter_block, compound_motion, predict_by_translation_under_integer_mv,
    resolve_inter_block, resolve_single, single_inter_block, single_mode_mv, single_mv_stack,
    warp_motion,
};
use super::temporal::temporal_motion_block;
#[allow(clippy::wildcard_imports)]
use super::*;

mod blend;
mod compound;
mod intrabc;
mod warp;

pub use blend::CompoundType;
pub(crate) use blend::MAX_CWP_NUM;
pub(crate) use compound::{EncoderCompoundMotion, JMVD_SCALE_MODES, MvdCoding};
pub(crate) use warp::{EncoderWarpMotion, EncoderWarpSource};

/// The motion of one inter leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EncoderMotion {
    /// A single-reference block (§ 5.20.7.11).
    Single(EncoderSingleMotion),
    /// A compound or skip-mode block (§ 5.20.7.6, § 5.20.7.10).
    Compound(EncoderCompoundMotion),
    /// A `WARPMV` or `WARP_NEWMV` block (§ 5.20.7.11).
    Warp(EncoderWarpMotion),
}

impl EncoderMotion {
    /// `(RefFrame[0], RefFrame[1])` as list indices.
    pub(crate) const fn refs(self) -> (i8, Option<i8>) {
        match self {
            Self::Single(single) => (single.ref_frame, None),
            Self::Warp(warp) => (warp.ref_frame, None),
            Self::Compound(compound) => (compound.ref_frames.0, Some(compound.ref_frames.1)),
        }
    }

    /// Whether the block is a skip-mode block.
    pub(crate) const fn skip_mode(self) -> bool {
        matches!(
            self,
            Self::Compound(EncoderCompoundMotion {
                skip_mode: true,
                ..
            })
        )
    }
}

/// The single-reference choice of one leaf (§ 5.20.7.11).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EncoderSingleMotion {
    /// The reference list index, `ref_frame0`.
    pub(crate) ref_frame: i8,
    /// `SINGLE_MODE_NEARMV`, `SINGLE_MODE_GLOBALMV` or `SINGLE_MODE_NEWMV`.
    pub(crate) mode: u8,
    /// `RefMvIdx`.
    pub(crate) ref_mv_idx: usize,
    /// The signed motion vector difference in 1/8 samples.
    pub(crate) mvd: Mv,
    /// `interp_filter`, where the decoder reads it.
    pub(crate) interp: ReconInterpolationFilter,
    /// The block `MvPrecision` of a `NEWMV` coded with
    /// `use_most_probable_precision = 0`; `None` takes `FrameMvPrecision`.
    pub(crate) precision: Option<u8>,
    /// `use_amvd` of a `NEWMV`.
    pub(crate) use_amvd: bool,
    /// `tip_mode`: the block predicts from the TIP frame (§ 7.11.3); its
    /// `ref_frame` is `TIP_REF_FRAME` and its mode `NEARMV` or `NEWMV`.
    pub(crate) tip: bool,
    /// The inter-intra prediction (§ 5.20.7.15) of a block that codes
    /// `inter_intra = 1`.
    pub(crate) interintra: Option<InterIntraPrediction>,
}

/// Whether `motion` predicts from the TIP frame.
const fn is_tip(motion: EncoderMotion) -> bool {
    matches!(
        motion,
        EncoderMotion::Single(EncoderSingleMotion { tip: true, .. })
    )
}

/// The single-reference vectors of one leaf and reference, see
/// [`EncoderInterTile::single_mvs`].
pub(crate) struct EncoderSingleMvs {
    pub(crate) global: Mv,
    pub(crate) near: Vec<Mv>,
    pub(crate) new_bases: Vec<Mv>,
    pub(crate) stack: Vec<Mv>,
}

/// The § 8.3.2 contexts of one inter leaf's syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EncoderInterContexts {
    pub(crate) is_inter: usize,
    pub(crate) skip_mode: usize,
    pub(crate) skip: usize,
}

/// A saved [`EncoderInterTile`] region.
pub(crate) struct EncoderInterCheckpoint {
    grid: GridCheckpoint,
    warp_bank: WarpParamBank,
    ref_mv_bank: Option<RefMvBank>,
    /// The TIP records before the region, and those appended after it that
    /// a rollback puts back, as [`GridCheckpoint`] keeps its leaves.
    tip_records: (usize, Vec<TemporalMotionBlock>),
    intrabc: Option<crate::filters::wienerns_lr::intrabc_records::IntrabcCheckpoint>,
}

/// The inter residual of one leaf: its transform blocks and their levels.
#[derive(Default)]
pub(crate) struct EncoderInterResidual {
    blocks: Vec<InterResidualBlock>,
    coeffs: Vec<i32>,
}

/// One tile's single-reference inter state.
pub(crate) struct EncoderInterTile {
    grid: NeighbourMvGrid,
    warp_bank: WarpParamBank,
    ref_mv_bank: Option<RefMvBank>,
    /// The projected motion field of a frame with `use_ref_frame_mvs`.
    temporal: Option<Arc<TemporalMvContext>>,
    reference: InterReferenceState<u16>,
    ref_frame_idx: Vec<u32>,
    order_hints: Vec<Option<u32>>,
    current_order_hint: u32,
    temporal_first: bool,
    core: FrameHeaderCore,
    sequence: SequenceHeader,
    precision: u8,
    max_drl_bits_minus_1: u32,
    drl_reorder: DrlReorder,
    /// `reference_select` and `skip_mode_present`.
    compound_tools: (bool, bool),
    /// The first luma 4x4 row and column of the tile.
    tile_start: (usize, usize),
    /// `enable_cwp || enable_masked_compound` and `enable_imp_msk_bld`.
    blend_tools: (bool, bool),
    /// `interpolation_filter == SWITCHABLE`.
    interp_switchable: bool,
    num_same_ref_compound: u8,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    chroma: ChromaFormatIdc,
    qindex: u32,
    bit_depth: BitDepth,
    residual_scratch: super::super::InterResidualReconScratch<u16>,
    interintra_scratch: super::interintra::InterIntraScratch<u16>,
    /// `TipFrameMode == TIP_FRAME_AS_REF` with the TIP reference pair.
    tip_ref_pair: Option<(i8, i8)>,
    /// The § 7.22 records of the published TIP leaves, which the decoder
    /// derives per TIP unit.
    tip_records: Vec<TemporalMotionBlock>,
    /// The unit records a TIP trial derives and drops, kept for reuse.
    tip_trial_records: Vec<TemporalMotionBlock>,
    tip_scratch: super::tip::TipReconstructScratch<u16>,
    /// The intra block copy state of an intra frame with `allow_intrabc`;
    /// `None` in an inter frame.
    intrabc: Option<TileIntrabcPreludeState>,
}

impl EncoderInterTile {
    /// The state of the tile over luma 4x4 `rows` and `cols` of the inter
    /// frame `core`, over the references in `reference`; `Err` names a tool
    /// the facade does not model.
    /// `temporal` is the [`encoder_temporal_context`] of the frame.
    pub(crate) fn new(
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        reference: InterReferenceState<u16>,
        temporal: Option<Arc<TemporalMvContext>>,
        (rows, cols): (core::ops::Range<usize>, core::ops::Range<usize>),
        (qindex, bit_depth): (u32, BitDepth),
    ) -> core::result::Result<Self, &'static str> {
        let (Some(inter), Some(tail), Some(seq_inter)) = (
            core.inter.as_ref(),
            core.inter_tail.as_ref(),
            sequence.inter.as_ref(),
        ) else {
            return Err("incomplete inter frame header");
        };
        let refused = [
            (tail.allow_bawp, "BAWP"),
            (
                seq_inter.enable_ref_frame_mvs && seq_inter.enable_global_motion,
                "global motion with reference frame motion vectors",
            ),
            (
                !matches!(
                    inter.interpolation_filter,
                    Some(FrameInterpolationFilter::Eighttap | FrameInterpolationFilter::Switchable)
                ),
                "an interpolation filter other than EIGHTTAP or SWITCHABLE",
            ),
            (
                inter
                    .tip_frame_mode
                    .is_some_and(|mode| mode == TipFrameMode::AsOutput)
                    || seq_inter.enable_mv_traj,
                "TIP output or motion vector trajectories",
            ),
            (
                inter.frame_enabled_motion_modes.is_some_and(|modes| {
                    !modes[splot_core::headers::frame::DELTAWARP]
                        && modes[splot_core::headers::frame::LOCALWARP..].contains(&true)
                }),
                "a warp mode without DELTAWARP",
            ),
            (core.frame_type == Some(FrameType::Switch), "a switch frame"),
        ];
        if let Some(&(_, what)) = refused.iter().find(|&&(on, _)| on) {
            return Err(what);
        }
        let ref_frame_idx = inter.ref_frame_idx.to_vec();
        if ref_frame_idx.is_empty() || inter.num_total_refs != Some(ref_frame_idx.len() as u32) {
            return Err("an inter frame without references");
        }
        let sb_h4 = frame_superblock_h4(
            sequence.partition.as_ref().map_or(
                SuperblockSize::Block64x64,
                SequencePartitionConfig::seq_sb_size,
            ),
            false,
        );
        let mut grid = NeighbourMvGrid::default();
        let size = core.frame_size.ok_or("incomplete inter frame header")?;
        let (mi_rows, mi_cols) = (
            (size.height as usize).div_ceil(8) * 2,
            (size.width as usize).div_ceil(8) * 2,
        );
        let tile_start = (rows.start, cols.start);
        grid.reset_for_tile(rows, cols, sb_h4)
            .map_err(|_| "neighbour grid size")?;
        let current_order_hint = core.display_order_hint().unwrap_or(0);
        let order_hints = reference_order_hints(
            &ref_frame_idx,
            &reference.ref_valid,
            &reference.ref_order_hint,
        )
        .to_vec();
        let tip_ref_pair = (inter.tip_frame_mode == Some(TipFrameMode::AsRef))
            .then(|| derived_tip_reference_pair(core, &order_hints))
            .flatten()
            .map(|pair| (pair.past_ref, pair.future_ref));
        let temporal_first = frame_temporal_first(
            sequence,
            core,
            &ref_frame_idx,
            &reference,
            current_order_hint,
            ref_frame_idx.len(),
        );
        Ok(Self {
            grid,
            warp_bank: WarpParamBank::new(),
            ref_mv_bank: seq_inter.enable_refmvbank.then(RefMvBank::new),
            temporal,
            temporal_first,
            ref_frame_idx,
            order_hints,
            current_order_hint,
            reference,
            core: core.clone(),
            sequence: sequence.clone(),
            precision: frame_mv_precision(core).map_err(|_| "MV precision")?,
            max_drl_bits_minus_1: inter
                .max_drl_bits_minus_1
                .ok_or("incomplete inter frame tools")?,
            drl_reorder: sequence_drl_reorder(sequence),
            compound_tools: (tail.reference_select, tail.skip_mode_present),
            tile_start,
            blend_tools: (
                seq_inter.enable_cwp || seq_inter.enable_masked_compound,
                seq_inter.enable_imp_msk_bld,
            ),
            interp_switchable: inter.interpolation_filter
                == Some(FrameInterpolationFilter::Switchable),
            num_same_ref_compound: seq_inter.num_same_ref_compound,
            sb_h4,
            mi_rows,
            mi_cols,
            chroma: sequence.general.chroma_format_idc,
            qindex,
            bit_depth,
            residual_scratch: super::super::InterResidualReconScratch::default(),
            interintra_scratch: super::interintra::InterIntraScratch::default(),
            tip_ref_pair,
            tip_records: Vec::new(),
            tip_trial_records: Vec::new(),
            tip_scratch: super::tip::TipReconstructScratch::default(),
            intrabc: None,
        })
    }

    /// Sets `tip_global_wtd_index` and returns the previous value.
    pub(crate) fn set_tip_weight(&mut self, weight: Option<u8>) -> Option<u8> {
        self.core
            .inter
            .as_mut()
            .and_then(|inter| core::mem::replace(&mut inter.tip_global_wtd_index, weight))
    }

    /// The TIP reference pair of a frame whose blocks may predict from TIP.
    pub(crate) const fn tip_ref_pair(&self) -> Option<(i8, i8)> {
        self.tip_ref_pair
    }

    /// Whether a block at `frontier` may code `tip_mode`.
    pub(crate) fn tip_allowed(&self, frontier: &DecodeBlockFrontier) -> Result<bool> {
        let (n4w, n4h) = block_size4(frontier)?;
        Ok(self.tip_ref_pair.is_some() && super::tip::tip_block_allowed(frontier, n4w, n4h))
    }

    /// `NumTotalRefs`.
    pub(crate) fn num_total_refs(&self) -> usize {
        self.ref_frame_idx.len()
    }

    /// `MaxDrlBits - 1`.
    pub(crate) const fn max_drl_bits_minus_1(&self) -> u32 {
        self.max_drl_bits_minus_1
    }

    /// The frame `MvPrecision`.
    pub(crate) const fn precision(&self) -> u8 {
        self.precision
    }

    /// Sets the `CurrentQIndex` the following leaves dequantize and deblock with.
    pub(crate) const fn set_qindex(&mut self, qindex: u32) {
        self.qindex = qindex;
    }

    /// Enters the superblock row holding `row`, before any leaf of it is coded.
    pub(crate) fn enter_superblock(&mut self, row: usize) {
        self.grid.enter_sb_row(row);
        if let Some(intrabc) = &mut self.intrabc {
            intrabc.enter_sb_row(row);
        }
    }

    /// Saves the state a leaf inside `footprint` can change; with `since`, an
    /// older live checkpoint, it stays valid after a rollback to `since`.
    pub(crate) fn checkpoint(
        &self,
        footprint: (usize, usize, usize, usize),
        since: Option<&EncoderInterCheckpoint>,
    ) -> EncoderInterCheckpoint {
        let kept = since.map_or(self.tip_records.len(), |since| since.tip_records.0);
        EncoderInterCheckpoint {
            grid: self
                .grid
                .checkpoint(footprint, since.map(|since| &since.grid)),
            warp_bank: self.warp_bank.clone(),
            ref_mv_bank: self.ref_mv_bank.clone(),
            tip_records: (
                kept,
                self.tip_records.get(kept..).unwrap_or_default().to_vec(),
            ),
            intrabc: self.intrabc.as_ref().map(|intrabc| {
                let (r, c, n4w, n4h) = footprint;
                intrabc.checkpoint(r..r + n4h, c..c + n4w)
            }),
        }
    }

    /// The reference MV bank, for the debug rollback check.
    #[cfg(debug_assertions)]
    pub(crate) const fn ref_mv_bank(&self) -> Option<&RefMvBank> {
        self.ref_mv_bank.as_ref()
    }

    /// The published TIP unit records, for the debug rollback check.
    #[cfg(debug_assertions)]
    pub(crate) fn tip_records(&self) -> &[TemporalMotionBlock] {
        &self.tip_records
    }

    /// Puts a [`Self::checkpoint`] back.
    pub(crate) fn rollback(&mut self, checkpoint: &EncoderInterCheckpoint) {
        self.grid.rollback(&checkpoint.grid);
        self.warp_bank.clone_from(&checkpoint.warp_bank);
        self.ref_mv_bank.clone_from(&checkpoint.ref_mv_bank);
        self.tip_records.truncate(checkpoint.tip_records.0);
        self.tip_records
            .extend_from_slice(&checkpoint.tip_records.1);
        if let (Some(intrabc), Some(saved)) = (&mut self.intrabc, &checkpoint.intrabc) {
            intrabc.rollback(saved);
        }
    }

    /// The `is_inter`, `skip_mode` and `skip_txfm` contexts of `frontier`,
    /// whose block codes `skip_mode`.
    pub(crate) fn contexts(
        &self,
        frontier: &DecodeBlockFrontier,
        skip_mode: bool,
    ) -> Result<EncoderInterContexts> {
        let block = self.block(frontier, (SINGLE_REF_FRAME0, None))?;
        let neighbour = block_neighbour_ctx(&self.grid, &block);
        Ok(EncoderInterContexts {
            is_inter: neighbour.is_inter_ctx,
            skip_mode: neighbour.skip_mode_ctx,
            skip: inter_skip_txfm_ctx(neighbour.skip_ctx, skip_mode),
        })
    }

    /// The motion vector the decoder resolves for `motion` at `frontier`.
    pub(crate) fn motion_vector(
        &mut self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderSingleMotion,
    ) -> Result<Mv> {
        Ok(self.motion_vectors(frontier, EncoderMotion::Single(motion))?[0])
    }

    /// The motion vectors the decoder resolves for `motion` at `frontier`.
    pub(crate) fn motion_vectors(
        &mut self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
    ) -> Result<[Mv; 2]> {
        let syntax = self.inter_syntax(frontier, motion, true)?;
        Ok(self.resolve_motion(&syntax)?.mv)
    }

    /// The vectors `GLOBALMV`, then `NEARMV` and the `NEWMV` bases of the
    /// first `drls` DRL indices code from list reference `ref_frame` at
    /// `frontier`, and the § 7.12 stack they come from, all from one stack.
    pub(crate) fn single_mvs(
        &mut self,
        frontier: &DecodeBlockFrontier,
        ref_frame: i8,
        drls: usize,
    ) -> Result<EncoderSingleMvs> {
        let block = self.block(frontier, (ref_frame, None))?;
        let precision = BlockPrecisionRecord::most_probable(self.precision);
        let tip = ref_frame == TIP_REF_FRAME;
        let single = |mode, ref_mv_idx| {
            self.single_syntax(
                &block,
                EncoderSingleMotion {
                    ref_frame,
                    mode,
                    ref_mv_idx,
                    mvd: Mv::ZERO,
                    interp: ReconInterpolationFilter::EightTap,
                    precision: None,
                    use_amvd: false,
                    tip,
                    interintra: None,
                },
            )
        };
        let global = single(SINGLE_MODE_GLOBALMV, 0);
        let near: Vec<_> = (0..drls)
            .map(|idx| single(SINGLE_MODE_NEARMV, idx))
            .collect();
        let new: Vec<_> = (0..drls)
            .map(|idx| single(SINGLE_MODE_NEWMV, idx))
            .collect();
        let temporal_first = !tip && self.temporal_first(ref_frame);
        let state = self.resolution_state(&block);
        let stack = single_mv_stack(&block, tip, temporal_first, &state);
        let mv = |single: &SingleMotionSyntax| {
            single_mode_mv(single, &stack, (precision, false), &block, &state)
        };
        Ok(EncoderSingleMvs {
            global: mv(&global),
            near: near.iter().map(mv).collect(),
            new_bases: new.iter().map(mv).collect(),
            stack: stack.candidates().collect(),
        })
    }

    /// Predicts `frontier` with `motion` into `workspace` (§ 7.11.3) and
    /// returns its motion vectors.
    pub(crate) fn predict(
        &mut self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
        workspace: &mut CurrentFrameWorkspace<u16>,
        (block_decoded, enable_ibp): (&TileBlockDecodedState, bool),
    ) -> Result<[Mv; 2]> {
        let syntax = self.inter_syntax(frontier, motion, true)?;
        let refine = refinemv_flags(&syntax.motion);
        let mut resolved = self.resolve_motion(&syntax)?;
        predict_by_translation_under_integer_mv(&syntax, &self.core, &mut resolved);
        let mvs = resolved.mv;
        let placed = self.placed(frontier, syntax, &resolved)?;
        if is_tip(motion) {
            let mut records = core::mem::take(&mut self.tip_trial_records);
            records.clear();
            let predicted = self.tip_motion(&placed, workspace, &mut records, true);
            self.tip_trial_records = records;
            predicted?;
            return Ok(mvs);
        }
        if placed.block.interintra.is_some() {
            prediction::reconstruct_placed_inter_block(
                &mut self.interintra_scratch,
                &mut self.residual_scratch,
                workspace,
                &placed,
                &[],
                &[],
                None,
                false,
                false,
                block_decoded,
                &self.ref_frame_idx,
                &self.reference,
                self.qindex,
                false,
                false,
                self.bit_depth,
                enable_ibp,
                ByteOffset::new(0),
            )?;
            return Ok(mvs);
        }
        let grid = self.compound_grid(&placed, refine, workspace)?;
        prediction::reconstruct_pure_inter_block(
            &mut mc::WorkspaceSink::Frame(workspace),
            &mut self.residual_scratch,
            &placed,
            &[],
            &[],
            grid,
            refine.0,
            refine.1,
            &self.ref_frame_idx,
            &self.reference,
            self.qindex,
            false,
            false,
            self.bit_depth,
            ByteOffset::new(0),
        )?;
        Ok(mvs)
    }

    /// Adds `residual` to the predicted samples of `frontier` in `workspace`;
    /// an IntraBC block (`use_intrabc`) transforms without DDT.
    pub(crate) fn add_residual(
        &mut self,
        residual: &EncoderInterResidual,
        use_intrabc: bool,
        workspace: &mut CurrentFrameWorkspace<u16>,
    ) -> Result<()> {
        super::super::add_inter_residual_to_workspace(
            &mut self.residual_scratch,
            &mut mc::WorkspaceSink::Frame(workspace),
            &residual.range(),
            &residual.blocks,
            &residual.coeffs,
            self.qindex,
            true,
            self.sequence
                .transform_quant_entropy
                .is_some_and(|tq| tq.enable_inter_ddt),
            use_intrabc,
            self.bit_depth,
        )
    }

    /// Publishes a coded inter leaf: its flags, then its § 7.12 resolution.
    /// Returns the motion vectors the decoder resolves.
    pub(crate) fn publish(
        &mut self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
        skip: bool,
        workspace: &mut CurrentFrameWorkspace<u16>,
    ) -> Result<[Mv; 2]> {
        let syntax = self.inter_syntax(frontier, motion, skip)?;
        let block = syntax.block_ctx;
        self.grid.record_flags(
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            syntax.flag_syntax(),
        );
        let mut state = self.resolution_state(&block);
        let resolved = resolve_inter_block(&syntax, &mut state)?;
        let refined = match syntax.motion {
            InterMotionSyntax::Compound(compound)
                if compound.use_refinemv || compound.use_optflow =>
            {
                Some(compound)
            }
            _ => None,
        };
        let wedge = match syntax.motion {
            InterMotionSyntax::Compound(compound)
                if matches!(syntax.blend, mc::CompoundBlend::Wedge { .. }) =>
            {
                Some(compound)
            }
            _ => None,
        };
        if is_tip(motion) {
            let placed = self.placed(frontier, syntax, &resolved)?;
            let mut records = core::mem::take(&mut self.tip_records);
            let derived = self.tip_motion(&placed, workspace, &mut records, false);
            self.tip_records = records;
            derived?;
        } else if let Some(compound) = refined {
            let refine = (
                compound.use_refinemv,
                compound.refinemv_switchable,
                compound.use_optflow,
            );
            let placed = self.placed(frontier, syntax, &resolved)?;
            if let Some(grid) = self.compound_grid(&placed, refine, workspace)? {
                self.append_compound_records((&placed, &block), compound, &resolved, Some(&grid))?;
            }
        } else if let Some(compound) = wedge {
            let placed = self.placed(frontier, syntax, &resolved)?;
            self.append_compound_records((&placed, &block), compound, &resolved, None)?;
        }
        Ok(resolved.mv)
    }

    /// Appends the § 7.19 temporal records of the compound block `placed`
    /// of `block` as 8x8 units, as the decoder records a refined or wedge
    /// block.
    fn append_compound_records(
        &mut self,
        (placed, block): (&PlacedInterBlock, &MvBlockContext),
        compound: CompoundMotionSyntax,
        resolved: &ResolvedInterBlock,
        grid: Option<&mc::CompoundMotionGrid>,
    ) -> Result<()> {
        super::compound_path::append_compound_temporal_motion(
            &mut self.tip_records,
            &self.reference,
            &self.ref_frame_idx,
            placed,
            super::super::compound::CompoundBlockSyntax {
                y_mode: compound.y_mode,
                use_optflow: compound.use_optflow,
                ref_frame0: block.ref_frame0,
                ref_frame1: compound.ref_frame1,
                mv0: resolved.mv[0],
                mv1: resolved.mv[1],
            },
            resolved.warp_params,
            grid,
            block.mi_row,
            block.mi_col,
            self.mi_rows,
            self.mi_cols,
            self.current_order_hint,
        )
    }

    /// Whether the sequence enables `enable_tip_refinemv`.
    fn tip_refinemv(&self) -> bool {
        self.sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_tip_refinemv)
    }

    /// The decoder's refinement grid of the compound block `placed` with
    /// `(use_refinemv, refinemv_switchable, use_optflow)`, `None` where it
    /// predicts without one.
    fn compound_grid(
        &self,
        placed: &PlacedInterBlock,
        (use_refinemv, switchable, use_optflow): (bool, bool, bool),
        workspace: &mut CurrentFrameWorkspace<u16>,
    ) -> Result<Option<mc::CompoundMotionGrid>> {
        if !use_refinemv && !use_optflow {
            return Ok(None);
        }
        let held = super::super::hold_inter_block_references(
            &self.ref_frame_idx,
            &self.reference,
            placed,
        )?;
        mc::inter_block_motion_grid(
            &mc::WorkspaceSink::Frame(workspace),
            held.block_params(placed, placed.motion_compensation_rect())?
                .with_refinemv(use_refinemv)
                .with_switchable_refinemv(switchable),
            None,
            ByteOffset::new(0),
        )
    }

    /// The decoder's § 7.11.3 TIP motion of `placed`, whose unit records it
    /// appends to `records`; with `predict`, also its prediction into
    /// `workspace`.
    fn tip_motion(
        &mut self,
        placed: &PlacedInterBlock,
        workspace: &mut CurrentFrameWorkspace<u16>,
        records: &mut Vec<TemporalMotionBlock>,
        predict: bool,
    ) -> Result<()> {
        let temporal = self
            .temporal
            .clone()
            .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let mut sink = mc::WorkspaceSink::Frame(workspace);
        let shared = (
            &self.sequence,
            &self.core,
            &self.ref_frame_idx,
            &self.reference,
        );
        let grid = super::tip::motion(
            &mut self.tip_scratch,
            records,
            &sink,
            placed,
            &temporal,
            shared.0,
            shared.1,
            shared.2,
            shared.3,
            ByteOffset::new(0),
        )?;
        if predict {
            super::tip::predict(
                &mut self.tip_scratch,
                &mut self.residual_scratch,
                &mut sink,
                grid,
                placed,
                &[],
                &[],
                &temporal,
                shared.0,
                shared.1,
                shared.2,
                shared.3,
                self.qindex,
                false,
                false,
                self.bit_depth,
                ByteOffset::new(0),
            )?;
        }
        Ok(())
    }

    /// The decoder's § 7.12 motion of `syntax`, not published.
    fn resolve_motion(&mut self, syntax: &InterBlockSyntax) -> Result<ResolvedInterBlock> {
        let state = self.resolution_state(&syntax.block_ctx);
        match &syntax.motion {
            InterMotionSyntax::Single(single) => Ok(resolve_single(syntax, single, &state)),
            InterMotionSyntax::Compound(compound) => compound_motion(syntax, compound, &state),
            InterMotionSyntax::Warp(warp) => warp_motion(syntax, warp, &state),
        }
    }

    /// The resolution state of a leaf at `block`, with the banks reset for it
    /// as the resolve pass does.
    fn resolution_state(&mut self, block: &MvBlockContext) -> MvResolutionState<'_> {
        self.reset_banks(block);
        MvResolutionState {
            grid: &mut self.grid,
            ref_mv_bank: &mut self.ref_mv_bank,
            warp_param_bank: &mut self.warp_bank,
            core: &self.core,
            temporal: self.temporal.as_deref(),
            order_hints: OrderHintMvContext::new(self.current_order_hint, &self.order_hints),
            drl_reorder: self.drl_reorder,
            max_drl_bits_minus_1: self.max_drl_bits_minus_1,
            frame_precision: self.precision,
            tile_offset: ByteOffset::new(0),
        }
    }

    /// Resets the banks for a leaf at `block` as the resolve pass does.
    fn reset_banks(&mut self, block: &MvBlockContext) {
        let (sb_h4, inter_frame) = (self.bank_sb_h4(), self.intrabc.is_none());
        if let Some(bank) = &mut self.ref_mv_bank {
            bank.reset_for_leaf(&self.grid, block.mi_row, block.mi_col, sb_h4, inter_frame);
        }
        self.warp_bank
            .reset_for_leaf(&self.grid, block.mi_row, block.mi_col, self.sb_h4);
    }

    /// The § 7.19 motion records of the published leaves, in coding order,
    /// which the decoder stores into the frame's motion field. A
    /// single-reference block stores a warp model only as a warp block, and
    /// a warp block of a `force_integer_mv` frame stores its translation
    /// (§ 7.13.3.14).
    pub(crate) fn temporal_records(&self) -> Vec<TemporalMotionBlock> {
        if self.intrabc.is_some() {
            return Vec::new();
        }
        let integer_mv = effective_force_integer_mv(&self.core);
        let mut unit_leaves: Vec<(usize, usize)> = self
            .tip_records
            .iter()
            .map(TemporalMotionBlock::origin)
            .collect();
        unit_leaves.sort_unstable();
        self.grid
            .published_leaves()
            .filter(|(_, refs, ..)| refs.is_none_or(|(ref_frame0, _)| ref_frame0 != TIP_REF_FRAME))
            .filter(|((r, c, ..), ..)| unit_leaves.binary_search(&(*r, *c)).is_err())
            .map(|((r, c, n4w, n4h), refs, mvs, splat)| {
                let (ref_frame0, ref_frame1) = refs.unwrap_or((-1, None));
                if refs.is_none() {
                    return TemporalMotionBlock::new(
                        r,
                        c,
                        n4w,
                        n4h,
                        self.mi_rows,
                        self.mi_cols,
                        self.current_order_hint,
                        [None, None],
                        [Mv::ZERO; 2],
                        [None, None],
                    );
                }
                temporal_motion_block(
                    &self.reference,
                    &self.ref_frame_idx,
                    r,
                    c,
                    n4w,
                    n4h,
                    self.mi_rows,
                    self.mi_cols,
                    self.current_order_hint,
                    ref_frame0,
                    ref_frame1,
                    mvs[0],
                    mvs[1],
                    if integer_mv && ref_frame1.is_none() {
                        [None, None]
                    } else {
                        splat
                    },
                )
            })
            .chain(self.tip_records.iter().copied())
            .collect()
    }

    /// The parse record of `motion` at `frontier` coded with `skip_txfm = skip`.
    fn inter_syntax(
        &self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
        skip: bool,
    ) -> Result<InterBlockSyntax> {
        let block = self.block(frontier, motion.refs())?;
        let mut blend = mc::CompoundBlend::default();
        let (syntax, optflow_distances) = match motion {
            EncoderMotion::Single(single) => (
                InterMotionSyntax::Single(self.single_syntax(&block, single)),
                None,
            ),
            EncoderMotion::Warp(warp) => (
                InterMotionSyntax::Warp(self.warp_syntax(&block, warp)?),
                None,
            ),
            EncoderMotion::Compound(compound) => {
                let syntax;
                (syntax, blend) = self.compound_syntax(frontier, &block, compound)?;
                let distances = super::compound_path::compound_optflow_distances(
                    &self.core,
                    &self.reference,
                    &self.ref_frame_idx,
                    block.ref_frame0,
                    &syntax,
                )?;
                (InterMotionSyntax::Compound(syntax), distances)
            }
        };
        Ok(InterBlockSyntax {
            block_ctx: block,
            motion: syntax,
            interp: self.interp_filter(frontier, motion)?.0,
            precision: self.block_precision(motion),
            skip,
            use_amvd: matches!(motion, EncoderMotion::Single(single) if single.use_amvd),
            tip_size_16x16: matches!(motion, EncoderMotion::Single(single) if single.tip)
                && super::tip::reference_uses_16x16_units(
                    block.bw4,
                    block.bh4,
                    self.tip_refinemv(),
                ),
            blend,
            bawp: BawpSyntax::default(),
            interintra: match motion {
                EncoderMotion::Single(single) => single.interintra,
                EncoderMotion::Compound(_) | EncoderMotion::Warp(_) => None,
            },
            optflow_distances,
            residual: None,
        })
    }

    /// Publishes an intra leaf of an inter frame as the parse and resolve
    /// passes do: its flags, then zero motion.
    pub(crate) fn publish_intra(&mut self, frontier: &DecodeBlockFrontier) -> Result<()> {
        if frontier.is_chroma_part() {
            return Ok(());
        }
        let block = self.block(frontier, (SINGLE_REF_FRAME0, None))?;
        self.grid.record_flags(
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            NeighbourFlagSyntax {
                interp_filter: interp_filter_no_neighbour_ctx(false) as u8,
                precision: BlockPrecisionRecord::explicit(self.precision),
                ..NON_INTER_FLAG_SYNTAX
            },
        );
        self.reset_banks(&block);
        self.grid.record_motion(
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            ZERO_NEIGHBOUR_MOTION_VALUES,
        );
        let sb_h4 = self.bank_sb_h4();
        if let Some(bank) = &mut self.ref_mv_bank {
            bank.update_count_for_non_inter(
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                sb_h4,
            );
        }
        self.record_intrabc_prelude(
            frontier,
            IntrabcBlockPrelude::from_use_skip(IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: false,
            }),
        )
    }

    /// Calls `read` with the plane view and the § 7.11.3.3 parameters that
    /// predict the luma rectangle `(x, y, w, h)` from `ref_frame` with `mv`
    /// and `filter`.
    pub(crate) fn with_luma_subpel<R>(
        &self,
        ref_frame: i8,
        rect: (usize, usize, usize, usize),
        (mv, filter): (Mv, ReconInterpolationFilter),
        read: impl FnOnce(
            &splot_recon::ReferencePlaneView<'_, u16>,
            &splot_recon::SubpelPredictParams,
        ) -> R,
    ) -> Result<R> {
        let slot = self
            .ref_frame_idx
            .get(usize::try_from(ref_frame).unwrap_or(usize::MAX))
            .copied()
            .ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
        let held = super::super::hold_reference_slot(&self.reference, slot)?;
        let (view, _, params) = mc::plane_prediction(
            self.frame_info()?,
            held.samples()?,
            ReconPlaneId::Y,
            mc::McBlockRect::from_luma_rect(rect.0, rect.1, rect.2, rect.3),
            mv,
            filter,
            0,
            0,
            ByteOffset::new(0),
        )?;
        Ok(read(&view, &params))
    }

    /// Records the deblocking and restoration facts of a coded inter leaf.
    pub(crate) fn record_filters(
        &self,
        frontier: &DecodeBlockFrontier,
        (motion, residual): (EncoderMotion, Option<&EncoderInterResidual>),
        (deblock, chroma, tx_skip): (
            &mut Vec<crate::filters::deblock::DeblockBlock>,
            &mut crate::filters::deblock::ChromaDeblockRecords,
            &mut Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
        ),
    ) -> Result<()> {
        let (n4w, n4h) = block_size4(frontier)?;
        let (blocks, range) = residual.map_or((&[][..], None), |residual| {
            (residual.blocks.as_slice(), Some(residual.range()))
        });
        let sub_pu_size = match motion {
            EncoderMotion::Single(single) if single.tip => {
                let units16 = super::tip::reference_uses_16x16_units(n4w, n4h, self.tip_refinemv());
                Some(crate::filters::deblock::DeblockSubPuSize::square(
                    if units16 { 16 } else { 8 },
                ))
            }
            EncoderMotion::Single(_) | EncoderMotion::Warp(_) => None,
            EncoderMotion::Compound(compound) => {
                let refinement = self.compound_refinement(frontier, compound)?;
                super::compound_path::compound_deblock_sub_pu_size(
                    refinement.use_optflow,
                    refinement.use_refinemv(),
                    n4w * MI_SIZE,
                    n4h * MI_SIZE,
                )
            }
        };
        record_inter_deblock_geometry(
            deblock,
            chroma,
            tx_skip,
            frontier,
            (n4w, n4h),
            self.chroma,
            range.as_ref(),
            blocks,
            sub_pu_size,
            self.qindex,
            false,
        )
    }

    /// Clears the coefficient contexts a skipped leaf covers (§ 5.20.4).
    pub(crate) fn reset_skip_contexts(
        &self,
        frontier: &DecodeBlockFrontier,
        coeff_ctx: &mut TileCoeffContextState,
    ) -> Result<()> {
        let (n4w, n4h) = block_size4(frontier)?;
        residual::reset_inter_skip_coeff_contexts(
            coeff_ctx,
            frontier,
            n4w,
            n4h,
            block_chroma_subsampling(self.chroma),
        )
    }

    fn frame_info(&self) -> Result<splot_recon::DecodedFrameInfo> {
        let size = self
            .core
            .frame_size
            .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
        let (width, height) = (size.width as usize, size.height as usize);
        let info = splot_recon::PlaneSize::new(width, height)
            .and_then(|plane| {
                splot_recon::DecodedFrameInfo::new(
                    splot_recon::OutputIndex::new(0),
                    self.bit_depth,
                    splot_recon::PixelFormat::from_av2_chroma_format_idc(self.chroma.get())?,
                    plane,
                    splot_recon::PlaneRect::new(0, 0, width, height)?,
                )
            })
            .and_then(|info| {
                info.with_storage_luma_size(splot_recon::PlaneSize::new(
                    self.mi_cols * 4,
                    self.mi_rows * 4,
                )?)
            })?;
        Ok(info)
    }

    fn block(
        &self,
        frontier: &DecodeBlockFrontier,
        (ref_frame0, ref_frame1): (i8, Option<i8>),
    ) -> Result<MvBlockContext> {
        let (bw4, bh4) = block_size4(frontier)?;
        Ok(MvBlockContext {
            mi_row: frontier.r,
            mi_col: frontier.c,
            bw4,
            bh4,
            sb_h4: self.sb_h4,
            ref_frame0,
            ref_frame1,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
        })
    }

    fn single_syntax(
        &self,
        block: &MvBlockContext,
        motion: EncoderSingleMotion,
    ) -> SingleMotionSyntax {
        SingleMotionSyntax {
            mode: motion.mode,
            tip_ref: motion.tip,
            ref_mv_idx: motion.ref_mv_idx,
            mvd: motion.mvd,
            use_temporal_first: !motion.tip && self.temporal_first(motion.ref_frame),
            global_warp: (!motion.tip && motion.mode == SINGLE_MODE_GLOBALMV)
                .then(|| {
                    global_motion_warp(&self.core, motion.ref_frame, false, block.bw4, block.bh4)
                })
                .flatten(),
        }
    }

    /// The single-reference temporal-first priority of list reference `ref_frame`.
    fn temporal_first(&self, ref_frame: i8) -> bool {
        self.temporal_first
            && block_ref_within_temporal_distance(
                &self.reference,
                &self.ref_frame_idx,
                self.current_order_hint,
                ref_frame,
            )
    }

    /// The block of `syntax`, resolved to `resolved`, placed at `frontier`.
    fn placed(
        &self,
        frontier: &DecodeBlockFrontier,
        syntax: InterBlockSyntax,
        resolved: &ResolvedInterBlock,
    ) -> Result<PlacedInterBlock> {
        let geometry = prediction::placed_inter_geometry_at(
            frontier,
            self.tile_start,
            (syntax.block_ctx.bw4, syntax.block_ctx.bh4),
            self.chroma != ChromaFormatIdc::Monochrome,
        )?;
        Ok(match &syntax.motion {
            &InterMotionSyntax::Compound(compound) => prediction::PlacedInterGeometry {
                interintra_chroma: false,
                ..geometry
            }
            .place(compound_inter_block(syntax, &compound, resolved)),
            _ => geometry.place(single_inter_block(syntax, resolved)),
        })
    }
}

/// The § 7.9 projected motion field of frame `core` of `sequence` over
/// `reference`, which every tile of the frame shares; `None` without
/// `use_ref_frame_mvs`.
pub(crate) fn encoder_temporal_context(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<u16>,
    bit_depth: BitDepth,
) -> Result<Option<Arc<TemporalMvContext>>> {
    if !frame_uses_temporal_mvs(core) {
        return Ok(None);
    }
    let geometry = super::super::FrameDecodeGeometry::new(core, sequence, bit_depth, false)?;
    let ref_frame_idx = core
        .inter
        .as_ref()
        .map_or(&[][..], |inter| &inter.ref_frame_idx[..]);
    let hints = reference_order_hints(
        ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
    );
    let mut context = TemporalMvContext::empty();
    frame_temporal_context(
        &mut context,
        core,
        geometry.sb_h4(),
        geometry.mi_dimensions(),
        core.display_order_hint().unwrap_or(0),
        temporal_projection_config(sequence, core, geometry),
        ref_frame_idx,
        reference,
        derived_tip_reference_pair(core, &hints),
    )?;
    Ok(Some(Arc::new(context)))
}

/// The § 7.19 motion field the decoder stores for frame `core` of `sequence`,
/// coded over `reference`, whose tiles published `records`.
pub(crate) fn encoder_motion_field<'a>(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<u16>,
    bit_depth: BitDepth,
    records: impl IntoIterator<Item = &'a [TemporalMotionBlock]>,
) -> Result<MotionFieldHandle> {
    let intra = core.frame_is_intra == Some(true);
    let geometry = super::super::FrameDecodeGeometry::new(core, sequence, bit_depth, intra)?;
    let ref_frame_idx = core
        .inter
        .as_ref()
        .map_or(&[][..], |inter| &inter.ref_frame_idx[..]);
    let hints = reference_order_hints(
        ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
    );
    let mut field = geometry
        .new_motion_field(&hints)
        .ok_or(inter_allocation!("encoder motion field"))?;
    for records in records {
        field.record_blocks(records);
    }
    Ok(MotionFieldHandle::settled(field))
}

impl EncoderInterResidual {
    /// Empties the residual for another leaf.
    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.coeffs.clear();
    }

    /// Whether no transform block is held.
    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Appends one transform block whose row-major adjusted `levels` code `eob`
    /// coefficients with `tx_type`.
    pub(crate) fn push(
        &mut self,
        (plane, x, y, tx_size): (usize, usize, usize, usize),
        levels: &[i32],
        eob: usize,
        (tx_type, use_tcq): (usize, bool),
        intra_ist: Option<crate::bitstream::tile_payload::IntraIstSyntax>,
    ) -> Result<()> {
        let plane = [ReconPlaneId::Y, ReconPlaneId::U, ReconPlaneId::V]
            .get(plane)
            .copied()
            .ok_or(DecodeHeaderStateError::InvalidInterResidualCoefficientState)?;
        let nonzero = levels
            .iter()
            .rposition(|&q| q != 0)
            .map_or(0, |last| last + 1);
        let start = self.coeffs.len();
        self.coeffs.extend_from_slice(&levels[..nonzero]);
        let coeffs = LumaCoeffBlock {
            eob,
            quant_range: start..start + nonzero,
            zero_tail: levels.len() - nonzero,
            use_tcq,
            intra_ist,
            ..LumaCoeffBlock::empty(tx_type, false)
        };
        push_inter_residual_block(
            &mut self.blocks,
            plane,
            x / MI_SIZE,
            y / MI_SIZE,
            tx_size,
            coeffs,
        )
        .map(drop)
    }

    fn range(&self) -> InterResidual {
        InterResidual {
            block_range: 0..self.blocks.len(),
        }
    }
}

/// The `chroma_inter_tx_type` the decoder reads for the chroma transform at
/// plane position `(x, y)` of `frontier`, after the luma transforms `luma`,
/// each `(x, y, tx_size, tx_type)` in luma samples.
pub(crate) fn chroma_inter_tx_type(
    frontier: &DecodeBlockFrontier,
    chroma: ChromaFormatIdc,
    luma: impl IntoIterator<Item = (usize, usize, usize, usize)>,
    (x, y): (usize, usize),
) -> Result<usize> {
    let (n4w, n4h) = block_size4(frontier)?;
    let mut map = InterLumaTxTypeMap::default();
    map.reset(frontier.r, frontier.c, n4h, n4w)?;
    for (luma_x, luma_y, tx_size, tx_type) in luma {
        map.update(luma_y / MI_SIZE, luma_x / MI_SIZE, tx_size, tx_type)?;
    }
    Ok(map.chroma_inter_tx_type(
        frontier.r,
        frontier.c,
        y / MI_SIZE,
        x / MI_SIZE,
        chroma_subsampling(chroma),
        false,
    ))
}

fn block_size4(frontier: &DecodeBlockFrontier) -> Result<(usize, usize)> {
    Ok((
        frontier
            .b_size
            .num_4x4_wide()
            .map_err(|_| DecodeHeaderStateError::InvalidBlockGeometry)?,
        frontier
            .b_size
            .num_4x4_high()
            .map_err(|_| DecodeHeaderStateError::InvalidBlockGeometry)?,
    ))
}

/// `(use_refinemv, refinemv_switchable, use_optflow)` of `motion`.
fn refinemv_flags(motion: &InterMotionSyntax) -> (bool, bool, bool) {
    match motion {
        InterMotionSyntax::Compound(compound) => (
            compound.use_refinemv,
            compound.refinemv_switchable,
            compound.use_optflow,
        ),
        _ => (false, false, false),
    }
}
