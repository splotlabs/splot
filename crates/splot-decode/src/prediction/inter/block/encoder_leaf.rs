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
    GridCheckpoint, OrderHintMvContext, RefMvBank, WarpParamBank, reference_order_hints,
};
use splot_core::headers::sequence::{SequencePartitionConfig, SuperblockSize};

use super::filter_records::record_inter_deblock_geometry;
use super::residual::{InterLumaTxTypeMap, push_inter_residual_block};
use super::resolve::{
    InterBlockSyntax, InterMotionSyntax, MvResolutionState, ResolvedInterBlock,
    compound_inter_block, compound_motion, resolve_inter_block, resolve_single, single_inter_block,
    single_mode_mv, single_mv_stack,
};
use super::temporal::temporal_motion_block;
#[allow(clippy::wildcard_imports)]
use super::*;

mod compound;

pub(crate) use compound::{EncoderCompoundMotion, JMVD_SCALE_MODES};

/// The motion of one inter leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EncoderMotion {
    /// A single-reference block (§ 5.20.7.11).
    Single(EncoderSingleMotion),
    /// A compound or skip-mode block (§ 5.20.7.10, § 5.20.7.12).
    Compound(EncoderCompoundMotion),
}

impl EncoderMotion {
    /// `(RefFrame[0], RefFrame[1])` as list indices.
    pub(crate) const fn refs(self) -> (i8, Option<i8>) {
        match self {
            Self::Single(single) => (single.ref_frame, None),
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
    precision: u8,
    max_drl_bits_minus_1: u32,
    drl_reorder: DrlReorder,
    /// `reference_select` and `skip_mode_present`.
    compound_tools: (bool, bool),
    num_same_ref_compound: u8,
    sb_h4: usize,
    mi_rows: usize,
    mi_cols: usize,
    chroma: ChromaFormatIdc,
    qindex: u32,
    bit_depth: BitDepth,
    residual_scratch: super::super::InterResidualReconScratch<u16>,
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
            (
                (tail.reference_select || tail.skip_mode_present)
                    && (seq_inter.enable_masked_compound
                        || seq_inter.enable_cwp
                        || seq_inter.enable_imp_msk_bld
                        || seq_inter.enable_refinemv
                        || seq_inter.enable_opfl_refine != 0),
                "masked, weighted, refined or optical-flow compound prediction",
            ),
            (
                tail.allow_warpmv_mode || tail.allow_bawp,
                "warped motion or BAWP",
            ),
            (
                seq_inter.enable_adaptive_mvd
                    || seq_inter.enable_flex_mvres
                    || seq_inter.enable_mvd_sign_derive,
                "adaptive MVD, flexible MV precision, or MVD sign derivation",
            ),
            (
                seq_inter.enable_ref_frame_mvs && seq_inter.enable_global_motion,
                "global motion with reference frame motion vectors",
            ),
            (
                inter.interpolation_filter != Some(FrameInterpolationFilter::Eighttap),
                "an interpolation filter other than EIGHTTAP",
            ),
            (
                inter
                    .tip_frame_mode
                    .is_some_and(|mode| mode != TipFrameMode::Disabled)
                    || seq_inter.enable_mv_traj,
                "TIP or motion vector trajectories",
            ),
            (
                inter
                    .frame_enabled_motion_modes
                    .is_some_and(|modes| modes.iter().any(|&on| on)),
                "inter-intra or a motion mode",
            ),
            (
                effective_force_integer_mv(core) || core.frame_type == Some(FrameType::Switch),
                "integer motion vectors or a switch frame",
            ),
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
        grid.reset_for_tile(rows, cols, sb_h4)
            .map_err(|_| "neighbour grid size")?;
        let current_order_hint = core.display_order_hint().unwrap_or(0);
        let order_hints = reference_order_hints(
            &ref_frame_idx,
            &reference.ref_valid,
            &reference.ref_order_hint,
        )
        .to_vec();
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
            precision: frame_mv_precision(core).map_err(|_| "MV precision")?,
            max_drl_bits_minus_1: inter
                .max_drl_bits_minus_1
                .ok_or("incomplete inter frame tools")?,
            drl_reorder: sequence_drl_reorder(sequence),
            compound_tools: (tail.reference_select, tail.skip_mode_present),
            num_same_ref_compound: seq_inter.num_same_ref_compound,
            sb_h4,
            mi_rows,
            mi_cols,
            chroma: sequence.general.chroma_format_idc,
            qindex,
            bit_depth,
            residual_scratch: super::super::InterResidualReconScratch::default(),
        })
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

    /// Enters the superblock row holding `row`, before any leaf of it is coded.
    pub(crate) fn enter_superblock(&mut self, row: usize) {
        self.grid.enter_sb_row(row);
    }

    /// Saves the state a leaf inside `footprint` can change; with `since`, an
    /// older live checkpoint, it stays valid after a rollback to `since`.
    pub(crate) fn checkpoint(
        &self,
        footprint: (usize, usize, usize, usize),
        since: Option<&EncoderInterCheckpoint>,
    ) -> EncoderInterCheckpoint {
        EncoderInterCheckpoint {
            grid: self
                .grid
                .checkpoint(footprint, since.map(|since| &since.grid)),
            warp_bank: self.warp_bank.clone(),
            ref_mv_bank: self.ref_mv_bank.clone(),
        }
    }

    /// The reference MV bank, for the debug rollback check.
    #[cfg(debug_assertions)]
    pub(crate) const fn ref_mv_bank(&self) -> Option<&RefMvBank> {
        self.ref_mv_bank.as_ref()
    }

    /// Puts a [`Self::checkpoint`] back.
    pub(crate) fn rollback(&mut self, checkpoint: &EncoderInterCheckpoint) {
        self.grid.rollback(&checkpoint.grid);
        self.warp_bank.clone_from(&checkpoint.warp_bank);
        self.ref_mv_bank.clone_from(&checkpoint.ref_mv_bank);
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
        let single = |mode, ref_mv_idx| {
            self.single_syntax(
                &block,
                EncoderSingleMotion {
                    ref_frame,
                    mode,
                    ref_mv_idx,
                    mvd: Mv::ZERO,
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
        let temporal_first = self.temporal_first(ref_frame);
        let state = self.resolution_state(&block);
        let stack = single_mv_stack(&block, false, temporal_first, &state);
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
    ) -> Result<[Mv; 2]> {
        let syntax = self.inter_syntax(frontier, motion, true)?;
        let resolved = self.resolve_motion(&syntax)?;
        let mvs = resolved.mv;
        let placed = self.placed(frontier, syntax, &resolved)?;
        prediction::reconstruct_pure_inter_block(
            &mut mc::WorkspaceSink::Frame(workspace),
            &mut self.residual_scratch,
            &placed,
            &[],
            &[],
            None,
            false,
            false,
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

    /// Adds `residual` to the predicted samples of `frontier` in `workspace`.
    pub(crate) fn add_residual(
        &mut self,
        residual: &EncoderInterResidual,
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
            false,
            false,
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
        Ok(resolve_inter_block(&syntax, &mut state)?.mv)
    }

    /// The decoder's § 7.12 motion of `syntax`, not published.
    fn resolve_motion(&mut self, syntax: &InterBlockSyntax) -> Result<ResolvedInterBlock> {
        let state = self.resolution_state(&syntax.block_ctx);
        match &syntax.motion {
            InterMotionSyntax::Single(single) => Ok(resolve_single(syntax, single, &state)),
            InterMotionSyntax::Compound(compound) => compound_motion(syntax, compound, &state),
            InterMotionSyntax::Warp(_) => {
                Err(DecodeHeaderStateError::InvalidInterBlockModeState.into())
            }
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
        if let Some(bank) = &mut self.ref_mv_bank {
            bank.reset_for_leaf(&self.grid, block.mi_row, block.mi_col, self.sb_h4, true);
        }
        self.warp_bank
            .reset_for_leaf(&self.grid, block.mi_row, block.mi_col, self.sb_h4);
    }

    /// The § 7.19 motion records of the published leaves, in coding order,
    /// which the decoder stores into the frame's motion field.
    pub(crate) fn temporal_records(&self) -> Vec<TemporalMotionBlock> {
        self.grid
            .published_leaves()
            .map(|((r, c, n4w, n4h), refs, mvs)| {
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
                    [None, None],
                )
            })
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
        let (syntax, interp) = match motion {
            EncoderMotion::Single(single) => (
                InterMotionSyntax::Single(self.single_syntax(&block, single)),
                ReconInterpolationFilter::EightTap,
            ),
            EncoderMotion::Compound(compound) => (
                InterMotionSyntax::Compound(self.compound_syntax(&block, compound)?),
                if compound.skip_mode {
                    ReconInterpolationFilter::EightTapSharp
                } else {
                    ReconInterpolationFilter::EightTap
                },
            ),
        };
        Ok(InterBlockSyntax {
            block_ctx: block,
            motion: syntax,
            interp,
            precision: BlockPrecisionRecord::most_probable(self.precision),
            skip,
            use_amvd: false,
            tip_size_16x16: false,
            blend: mc::CompoundBlend::default(),
            bawp: BawpSyntax::default(),
            interintra: None,
            optflow_distances: None,
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
        if let Some(bank) = &mut self.ref_mv_bank {
            bank.update_count_for_non_inter(
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                self.sb_h4,
            );
        }
        Ok(())
    }

    /// Calls `read` with the plane view and the § 7.11.3.3 parameters that
    /// predict the luma rectangle `(x, y, w, h)` from `ref_frame` with `mv`.
    pub(crate) fn with_luma_subpel<R>(
        &self,
        ref_frame: i8,
        rect: (usize, usize, usize, usize),
        mv: Mv,
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
            ReconInterpolationFilter::EightTap,
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
        residual: Option<&EncoderInterResidual>,
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
        record_inter_deblock_geometry(
            deblock,
            chroma,
            tx_skip,
            frontier,
            (n4w, n4h),
            self.chroma,
            range.as_ref(),
            blocks,
            None,
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
            tip_ref: false,
            ref_mv_idx: motion.ref_mv_idx,
            mvd: motion.mvd,
            use_temporal_first: self.temporal_first(motion.ref_frame),
            global_warp: (motion.mode == SINGLE_MODE_GLOBALMV)
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
            (0, 0),
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

/// One transform block of a leaf: `(plane, x, y, tx_size)` in plane samples.
pub(crate) type ResidualUnit = (usize, usize, usize, usize);

/// The transform blocks `(plane, x, y, tx_size)` of a non-lossless inter leaf
/// whose planes each take one transform (§ 5.20.7.23), luma first; `None` for
/// a leaf whose residual spans more than one transform per plane.
pub(crate) fn single_transform_residual(
    frontier: &DecodeBlockFrontier,
    chroma: ChromaFormatIdc,
) -> Result<Option<Vec<ResidualUnit>>> {
    let (n4w, n4h) = block_size4(frontier)?;
    let luma = residual::max_tx_size(frontier.b_size.index())?;
    let covers = |tx_size: usize, w4: usize, h4: usize| -> Result<bool> {
        Ok(
            residual::tx_size_dimension(&TX_WIDTH, tx_size)? == w4 * MI_SIZE
                && residual::tx_size_dimension(&TX_HEIGHT, tx_size)? == h4 * MI_SIZE,
        )
    };
    if n4w > CHUNK_64_N4 || n4h > CHUNK_64_N4 || !covers(luma, n4w, n4h)? {
        return Ok(None);
    }
    let mut units = vec![(0, frontier.c * MI_SIZE, frontier.r * MI_SIZE, luma)];
    if frontier.has_chroma && chroma != ChromaFormatIdc::Monochrome {
        let (sub_x, sub_y) = chroma_subsampling(chroma);
        let chroma_ref = frontier.chroma_ref_geometry();
        let size = get_plane_residual_size(chroma_ref.size(), 1, sub_x, sub_y)
            .map_err(|_| residual::residual_geometry_error())?
            .ok_or_else(residual::residual_geometry_error)?;
        let tx_size = residual::max_tx_size(size.index())?;
        let (w4, h4) = (
            size.num_4x4_wide()
                .map_err(|_| residual::residual_geometry_error())?,
            size.num_4x4_high()
                .map_err(|_| residual::residual_geometry_error())?,
        );
        if !covers(tx_size, w4, h4)? {
            return Ok(None);
        }
        let x = (chroma_ref.col() >> usize::from(sub_x)) * MI_SIZE;
        let y = (chroma_ref.row() >> usize::from(sub_y)) * MI_SIZE;
        units.push((1, x, y, tx_size));
        units.push((2, x, y, tx_size));
    }
    Ok(Some(units))
}

/// The `chroma_inter_tx_type` the decoder reads for the chroma transform at
/// plane position `(x, y)` of `frontier`, whose single luma transform of
/// size `luma_tx_size` has type `luma_tx_type`.
pub(crate) fn chroma_inter_tx_type(
    frontier: &DecodeBlockFrontier,
    chroma: ChromaFormatIdc,
    (luma_tx_size, luma_tx_type): (usize, usize),
    (x, y): (usize, usize),
) -> Result<usize> {
    let (n4w, n4h) = block_size4(frontier)?;
    let mut map = InterLumaTxTypeMap::default();
    map.reset(frontier.r, frontier.c, n4h, n4w)?;
    map.update(frontier.r, frontier.c, luma_tx_size, luma_tx_type)?;
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
