// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mode syntax of encoder inter leaves: `comp_mode`, the single and compound
//! references, the modes, the DRL indices and `interp_filter` (§ 5.20.7.6,
//! § 5.20.7.10, § 5.20.7.11), produced by the decoder's own reads answering
//! for a target.

use super::super::super::compound::CompoundBlockSyntax;
use super::super::super::compound::{
    CompoundParseInput, compound_reference_pair, compound_y_mode, non_joint_compound_mode,
    same_refs_compound_mode,
};
use super::super::compound_path::{
    compound_all_opfl_reachable, compound_joint_mv_projection, compound_needs_interp_filter,
    compound_ref_contexts, compound_ref_distance_signs, compound_refinemv_is_switchable,
    compound_refinemv_reachable, compound_switchable_opfl_reachable, compound_use_amvd_selector,
    skip_mode_reference_pair, use_optflow_ctx, use_refinemv_ctx,
};
use super::super::resolve::CompoundMotionSyntax;
use super::super::syntax::{
    block_mv_precision_with, interp_filter_symbol, single_use_amvd_selector,
};
use super::super::warp::{
    inter_mv_read_config, inter_mvd_sign_derivation_allowed,
    mvd_sign_derivation_block_scope_allowed,
};
use super::*;

/// `JOINT_NEWMV_SCALE_FACTOR_CNT` (§ 3): the `jmvd_scale_mode` values without
/// adaptive MVD.
pub(crate) const JMVD_SCALE_MODES: u8 = 5;

/// The `pb_mv_precision` values a block codes.
const PB_MV_PRECISION_SYMBOLS: u8 =
    crate::bitstream::tile_payload::PB_MV_PRECISION_CDF_ROW_LEN as u8 - 1;

/// How the decoder reads the differences of one leaf (§ 5.20.7.13):
/// `MvPrecision`, `use_amvd`, and the derived-sign rule with its threshold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MvdCoding {
    pub(crate) precision: u8,
    pub(crate) use_amvd: bool,
    pub(crate) derive_sign: bool,
    pub(crate) derive_threshold: usize,
}

/// The compound choice of one leaf (§ 5.20.7.12), or a skip-mode leaf
/// (§ 5.20.7.10) whose `ref_frames` are the `skip_mode_frames` pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EncoderCompoundMotion {
    /// `(RefFrame[0], RefFrame[1])` as list indices.
    pub(crate) ref_frames: (i8, i8),
    /// `YMode`; a skip-mode block codes `NEAR_NEARMV`.
    pub(crate) y_mode: CompoundYMode,
    /// `skip_mode`.
    pub(crate) skip_mode: bool,
    /// `RefMvIdx` of the paired stack.
    pub(crate) ref_mv_idx: usize,
    /// The two second-DRL indices of `NEAR_NEARMV` and `NEAR_NEWMV`.
    pub(crate) independent_idx: Option<[usize; 2]>,
    /// The signed differences, in the slots the decoder reads them into.
    pub(crate) mvd: [Mv; 2],
    /// `jmvd_scale_mode`.
    pub(crate) jmvd_scale_mode: u8,
    /// `interp_filter`, where the decoder reads it.
    pub(crate) interp: ReconInterpolationFilter,
    /// `use_optflow`, where the decoder reads it (§ 5.20.7.6).
    pub(crate) use_optflow: bool,
    /// `use_refinemv`, where the decoder reads it (§ 5.20.7.17); where it
    /// infers the flag, the block refines whenever refinement is reachable.
    pub(crate) use_refinemv: bool,
}

/// The decoder's refinement of a compound block (§ 5.20.7.6, § 5.20.7.17).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompoundRefinement {
    /// Whether the block reads `use_optflow`.
    pub(crate) reads_optflow: bool,
    /// The `use_refinemv` selector, where the block reads the flag.
    pub(crate) refinemv_selector: Option<TileCdfSelector>,
    /// Whether the block predicts with optical flow, `REFINE_ALL` included.
    pub(crate) use_optflow: bool,
    /// Whether `use_refinemv` is read as `1` or inferred.
    pub(crate) refinemv_signalled: bool,
    /// `refinemv_switchable` of the decoder's motion syntax.
    pub(crate) refinemv_switchable: bool,
}

impl EncoderMotion {
    /// The motion vector differences in coding order (§ 5.20.7.7).
    pub(crate) fn coded_mvds(self) -> Vec<Mv> {
        match self {
            Self::Single(single) if single.mode == SINGLE_MODE_NEWMV => vec![single.mvd],
            Self::Compound(compound) if !compound.skip_mode => match compound.y_mode {
                CompoundYMode::NearNew => vec![compound.mvd[1]],
                CompoundYMode::NewNear | CompoundYMode::JointNew => vec![compound.mvd[0]],
                CompoundYMode::NewNew => compound.mvd.to_vec(),
                CompoundYMode::NearNear | CompoundYMode::GlobalGlobal => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

impl EncoderInterTile {
    /// The block precision record of `motion`.
    pub(crate) fn block_precision(&self, motion: EncoderMotion) -> BlockPrecisionRecord {
        match motion {
            EncoderMotion::Single(EncoderSingleMotion {
                precision: Some(precision),
                ..
            }) => BlockPrecisionRecord::explicit(precision),
            _ => BlockPrecisionRecord::most_probable(self.precision),
        }
    }

    /// How the decoder reads the differences of `motion`.
    pub(crate) fn mvd_coding(&self, motion: EncoderMotion) -> Result<MvdCoding> {
        let precision = self.block_precision(motion).mv_precision;
        let frame_config = inter_mv_read_config(&self.core)?;
        let allowed = |use_amvd| {
            inter_mvd_sign_derivation_allowed(
                &self.sequence,
                &self.core,
                SINGLE_MODE_NEWMV,
                use_amvd,
                frame_config,
                MvReadConfig::inter(precision),
            )
        };
        let (use_amvd, derive_sign, derive_threshold) = match motion {
            EncoderMotion::Single(single) => (
                single.use_amvd,
                single.mode == SINGLE_MODE_NEWMV
                    && mvd_sign_derivation_block_scope_allowed(MotionMode::Simple, false, None)
                    && allowed(single.use_amvd),
                1,
            ),
            EncoderMotion::Compound(compound) => (
                false,
                matches!(
                    compound.y_mode,
                    CompoundYMode::JointNew | CompoundYMode::NewNew
                ) && mvd_sign_derivation_block_scope_allowed(
                    MotionMode::Simple,
                    false,
                    Some(compound.ref_mv_idx),
                ) && allowed(false),
                compound.y_mode.mvd_sign_derivation_threshold(),
            ),
        };
        Ok(MvdCoding {
            precision,
            use_amvd,
            derive_sign,
            derive_threshold,
        })
    }

    /// Whether the sequence enables adaptive MVD.
    pub(crate) fn enable_adaptive_mvd(&self) -> bool {
        self.sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_adaptive_mvd)
    }

    /// The explicit block precisions a `NEWMV` at `frontier` can code,
    /// finest first.
    pub(crate) fn newmv_precisions(&self, frontier: &DecodeBlockFrontier) -> Result<Vec<u8>> {
        let neighbour = block_neighbour_ctx(
            &self.grid,
            &self.block(frontier, (SINGLE_REF_FRAME0, None))?,
        );
        let mut out = Vec::new();
        for pb in 0..PB_MV_PRECISION_SYMBOLS {
            let read = block_mv_precision_with(
                &self.sequence,
                &self.core,
                &neighbour,
                self.precision,
                (true, false),
                |selector| {
                    Ok(match selector {
                        TileCdfSelector::UseMostProbablePrecision { .. } => 0,
                        _ => pb,
                    })
                },
            );
            if let Ok(read) = read
                && !read.use_most_probable_precision
            {
                out.push(read.mv_precision);
            }
        }
        out.sort_unstable_by(|a, b| b.cmp(a));
        out.dedup();
        Ok(out)
    }

    /// The `use_most_probable_precision` and `pb_mv_precision` symbols that
    /// code `target` for a block that codes `NEWMV` as `has_newmv`, answered
    /// through the decoder's own reads.
    fn precision_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        neighbour: &BlockNeighbourContext,
        has_newmv: bool,
        (target, use_amvd): (BlockPrecisionRecord, bool),
    ) -> Result<()> {
        for pb in 0..PB_MV_PRECISION_SYMBOLS {
            let mut coded = Vec::new();
            let read = block_mv_precision_with(
                &self.sequence,
                &self.core,
                neighbour,
                self.precision,
                (has_newmv, use_amvd),
                |selector| {
                    let value = match selector {
                        TileCdfSelector::UseMostProbablePrecision { .. } => {
                            u8::from(target.use_most_probable_precision)
                        }
                        _ => pb,
                    };
                    coded.push((selector, value));
                    Ok(value)
                },
            );
            if read.is_ok_and(|read| read == target) {
                out.extend(coded);
                return Ok(());
            }
        }
        Err(DecodeHeaderStateError::InvalidInterBlockModeState.into())
    }

    /// The refinement the decoder derives for `compound` at `frontier`.
    ///
    /// # Errors
    /// Returns the decoder's diagnostic where `compound` sets a flag the
    /// block does not read.
    pub(crate) fn compound_refinement(
        &self,
        frontier: &DecodeBlockFrontier,
        compound: EncoderCompoundMotion,
    ) -> Result<CompoundRefinement> {
        let invalid =
            || -> crate::DecodeError { DecodeHeaderStateError::InvalidInterBlockModeState.into() };
        if compound.skip_mode {
            if compound.use_optflow || compound.use_refinemv {
                return Err(invalid());
            }
            return Ok(CompoundRefinement::default());
        }
        let (n4w, n4h) = block_size4(frontier)?;
        let opfl = self
            .core
            .inter
            .as_ref()
            .and_then(|inter| inter.opfl_refine_type)
            .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
        let enable_refinemv = self
            .sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refinemv);
        let mut block = CompoundBlockSyntax {
            y_mode: compound.y_mode,
            use_optflow: false,
            ref_frame0: compound.ref_frames.0,
            ref_frame1: compound.ref_frames.1,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
        };
        let refs = (&self.core, &self.reference, self.ref_frame_idx.as_slice());
        let reads_optflow =
            compound_switchable_opfl_reachable(opfl, refs.0, refs.1, refs.2, block, n4w, n4h)?;
        if compound.use_optflow && !reads_optflow {
            return Err(invalid());
        }
        block.use_optflow = compound.use_optflow;
        let refinemv_switchable = compound_refinemv_is_switchable(block, opfl);
        let reached = compound_refinemv_reachable(
            enable_refinemv,
            opfl,
            refs.0,
            refs.1,
            refs.2,
            block,
            n4w,
            n4h,
        )?;
        let reads_refinemv = reached && refinemv_switchable;
        if compound.use_refinemv && !reads_refinemv {
            return Err(invalid());
        }
        let all = compound_all_opfl_reachable(
            opfl,
            refs.0,
            refs.1,
            refs.2,
            block,
            n4w,
            n4h,
            super::super::mc::CompoundBlend::default(),
        )?;
        Ok(CompoundRefinement {
            reads_optflow,
            refinemv_selector: reads_refinemv.then(|| TileCdfSelector::UseRefinemv {
                ctx: use_refinemv_ctx(compound.y_mode, compound.use_optflow),
            }),
            use_optflow: compound.use_optflow || all,
            refinemv_signalled: reached && (!refinemv_switchable || compound.use_refinemv),
            refinemv_switchable,
        })
    }

    /// The `use_refinemv` symbol `motion` at `frontier` codes after its
    /// differences (§ 5.20.7.17), if the decoder reads one.
    pub(crate) fn refinemv_symbol(
        &self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
    ) -> Result<Option<(TileCdfSelector, u8)>> {
        let EncoderMotion::Compound(compound) = motion else {
            return Ok(None);
        };
        Ok(self
            .compound_refinement(frontier, compound)?
            .refinemv_selector
            .map(|selector| (selector, u8::from(compound.use_refinemv))))
    }

    /// The filter the decoder predicts `motion` at `frontier` with, and the
    /// `interp_filter` symbol it reads for it (§ 5.20.7.6), if any.
    pub(crate) fn interp_filter(
        &self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
    ) -> Result<(ReconInterpolationFilter, Option<(TileCdfSelector, u8)>)> {
        if motion.skip_mode() || matches!(motion, EncoderMotion::Single(single) if single.tip) {
            return Ok((ReconInterpolationFilter::EightTapSharp, None));
        }
        if let EncoderMotion::Compound(compound) = motion {
            let refinement = self.compound_refinement(frontier, compound)?;
            if refinement.use_optflow || refinement.refinemv_signalled {
                return Ok((ReconInterpolationFilter::EightTapSharp, None));
            }
        }
        let (global, interp) = match motion {
            EncoderMotion::Single(single) => (single.mode == SINGLE_MODE_GLOBALMV, single.interp),
            EncoderMotion::Compound(compound) => (
                compound.y_mode == CompoundYMode::GlobalGlobal,
                compound.interp,
            ),
        };
        let (ref_frame0, ref_frame1) = motion.refs();
        match self.interp_filter_selector(frontier, (ref_frame0, ref_frame1.is_some()), global)? {
            None => Ok((ReconInterpolationFilter::EightTap, None)),
            Some(_) if interp == ReconInterpolationFilter::Bilinear => {
                Err(DecodeHeaderStateError::InvalidInterpolationFilter.into())
            }
            Some(selector) => Ok((interp, Some((selector, interp_filter_symbol(interp))))),
        }
    }

    /// The `interp_filter` selector of a block at `frontier` over list
    /// reference `ref_frame0`, compound or not, that codes `GLOBALMV` or
    /// `GLOBAL_GLOBALMV` or not; `None` where the decoder reads no filter.
    pub(crate) fn interp_filter_selector(
        &self,
        frontier: &DecodeBlockFrontier,
        (ref_frame0, compound): (i8, bool),
        global: bool,
    ) -> Result<Option<TileCdfSelector>> {
        let (n4w, n4h) = block_size4(frontier)?;
        let needs = if compound {
            let y_mode = if global {
                CompoundYMode::GlobalGlobal
            } else {
                CompoundYMode::NearNear
            };
            compound_needs_interp_filter(n4w, n4h, y_mode, false)
        } else {
            let mode = if global {
                SINGLE_MODE_GLOBALMV
            } else {
                SINGLE_MODE_NEARMV
            };
            single_inter_needs_interp_filter(n4w, n4h, mode)
        };
        if !self.interp_switchable || !needs {
            return Ok(None);
        }
        let neighbour = block_neighbour_ctx(
            &self.grid,
            &self.block(frontier, (SINGLE_REF_FRAME0, None))?,
        );
        Ok(Some(TileCdfSelector::InterpFilter {
            ctx: neighbour.interp_filter_ctx(ref_frame0, compound),
        }))
    }

    /// Whether the decoder reads `comp_mode` (§ 5.20.7.6) and `skip_mode`
    /// (§ 5.20.5.9) of a block at `frontier`.
    pub(crate) fn reads_compound_syntax(
        &self,
        frontier: &DecodeBlockFrontier,
    ) -> Result<(bool, bool)> {
        let (n4w, n4h) = block_size4(frontier)?;
        let allowed = is_comp_ref_allowed(n4w, n4h);
        let (reference_select, skip_mode_present) = self.compound_tools;
        let part = frontier.is_luma_part() || frontier.is_chroma_part();
        Ok((
            reference_select && allowed,
            skip_mode_present && allowed && !part,
        ))
    }

    /// The reference pairs `read_compound_ref` can code.
    pub(crate) fn compound_pairs(&self) -> Vec<(i8, i8)> {
        let refs = self.num_total_refs() as i8;
        let input = self.compound_input([0; 7], [true; 7]);
        let mut pairs = Vec::new();
        for first in 0..refs {
            for second in first..refs {
                let pair = (first, second);
                if compound_reference_pair(input, |selector| Ok(comp_ref_bit(selector, pair)))
                    .is_ok_and(|coded| coded == pair)
                {
                    pairs.push(pair);
                }
            }
        }
        pairs
    }

    /// The `skip_mode_frames` pair of a skip-mode block at `frontier`.
    pub(crate) fn skip_mode_pair(&self, frontier: &DecodeBlockFrontier) -> Result<(i8, i8)> {
        let block = self.block(frontier, (SINGLE_REF_FRAME0, None))?;
        skip_mode_reference_pair(
            &self.core,
            (&self.reference, &self.ref_frame_idx, self.num_total_refs()),
            &block_neighbour_ctx(&self.grid, &block),
            self.tip_ref_pair(),
            ByteOffset::new(0),
        )
    }

    /// Whether list `0` and list `1` of `compound` code a difference.
    pub(crate) fn coded_lists(&self, compound: &EncoderCompoundMotion) -> Result<[bool; 2]> {
        Ok(match compound.y_mode {
            _ if compound.skip_mode => [false; 2],
            CompoundYMode::NearNew => [false, true],
            CompoundYMode::NewNear => [true, false],
            CompoundYMode::NewNew => [true; 2],
            CompoundYMode::JointNew => {
                let base_list = self.joint_projection(compound.ref_frames)?.base_list;
                [base_list == 0, base_list == 1]
            }
            CompoundYMode::NearNear | CompoundYMode::GlobalGlobal => [false; 2],
        })
    }

    /// The differences that move the coded lists of `compound` at `frontier`
    /// to `targets`; the other lists keep their candidates.
    pub(crate) fn compound_mvd(
        &mut self,
        frontier: &DecodeBlockFrontier,
        compound: EncoderCompoundMotion,
        targets: [Mv; 2],
    ) -> Result<[Mv; 2]> {
        let lists = self.coded_lists(&compound)?;
        let base = self.motion_vectors(
            frontier,
            EncoderMotion::Compound(EncoderCompoundMotion {
                mvd: [Mv::ZERO; 2],
                ..compound
            }),
        )?;
        let mut mvd = [Mv::ZERO; 2];
        for list in (0..2).filter(|&list| lists[list]) {
            let slot = if compound.y_mode == CompoundYMode::JointNew {
                0
            } else {
                list
            };
            mvd[slot] = Mv {
                row: targets[list].row - base[list].row,
                col: targets[list].col - base[list].col,
            };
        }
        Ok(mvd)
    }

    /// The mode symbols of `motion` at `frontier` in coding order, from
    /// `comp_mode` to the DRL indices, with the values the decoder reads.
    pub(crate) fn mode_symbols(
        &self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
    ) -> Result<Vec<(TileCdfSelector, u8)>> {
        let mut out = Vec::new();
        let refs = self.num_total_refs();
        let max_drl = self.max_drl_bits_minus_1 as usize + 1;
        let block = self.block(frontier, motion.refs())?;
        let neighbour = block_neighbour_ctx(
            &self.grid,
            &self.block(frontier, (SINGLE_REF_FRAME0, None))?,
        );
        let tip_pair = match motion {
            EncoderMotion::Compound(_) => self.tip_ref_pair(),
            EncoderMotion::Single(_) => None,
        };
        let new_mv = find_mode_ctx_with_tip(&self.grid, &block, tip_pair).new_mv_context;
        if let EncoderMotion::Compound(compound) = motion
            && compound.skip_mode
        {
            drl_symbols(
                &mut out,
                |idx| TileCdfSelector::SkipDrlMode { idx },
                0,
                max_drl,
                compound.ref_mv_idx,
            )?;
            return Ok(out);
        }
        let tip = matches!(motion, EncoderMotion::Single(single) if single.tip);
        if self.tip_allowed(frontier)? {
            let ctx = neighbour.tip_mode_ctx();
            out.push((TileCdfSelector::TipMode { ctx }, u8::from(tip)));
        } else if tip {
            return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
        }
        if let (true, EncoderMotion::Single(single)) = (tip, motion) {
            let new = single.mode == SINGLE_MODE_NEWMV;
            if !new && single.mode != SINGLE_MODE_NEARMV {
                return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
            }
            out.push((TileCdfSelector::TipPredMode, u8::from(new)));
            let amvd_ctx = neighbour.amvd_ctx(TIP_REF_FRAME);
            if let Some(selector) =
                single_use_amvd_selector(self.enable_adaptive_mvd(), single.mode, amvd_ctx)
            {
                out.push((selector, u8::from(single.use_amvd)));
            } else if single.use_amvd {
                return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
            }
            let selector = |idx| TileCdfSelector::TipDrlMode { idx };
            drl_symbols(&mut out, selector, 0, max_drl, single.ref_mv_idx)?;
            self.precision_symbols(
                &mut out,
                &neighbour,
                new,
                (self.block_precision(motion), single.use_amvd),
            )?;
            return Ok(out);
        }
        if self.reads_compound_syntax(frontier)?.0 {
            let ctx = neighbour.comp_mode_ctx(
                &self.ref_frame_idx,
                &self.reference.ref_order_hint,
                super::super::super::CompoundOrderHint::current(self.current_order_hint),
            );
            let compound = matches!(motion, EncoderMotion::Compound(_));
            out.push((TileCdfSelector::CompMode { ctx }, u8::from(compound)));
        }
        match motion {
            EncoderMotion::Single(single) => {
                for ref_idx in 0..refs.saturating_sub(1) {
                    let ctx = neighbour
                        .single_ref_ctx(ref_idx, refs)
                        .ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
                    let value = ref_idx == single.ref_frame as usize;
                    out.push((TileCdfSelector::SingleRef { ctx, ref_idx }, u8::from(value)));
                    if value {
                        break;
                    }
                }
                out.push((TileCdfSelector::SingleMode { ctx: new_mv }, single.mode));
                let amvd_ctx = neighbour.amvd_ctx(single.ref_frame);
                if let Some(selector) =
                    single_use_amvd_selector(self.enable_adaptive_mvd(), single.mode, amvd_ctx)
                {
                    out.push((selector, u8::from(single.use_amvd)));
                } else if single.use_amvd {
                    return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
                }
                if single.mode != SINGLE_MODE_GLOBALMV {
                    let selector = |idx| TileCdfSelector::DrlMode { idx, ctx: new_mv };
                    drl_symbols(&mut out, selector, 0, max_drl, single.ref_mv_idx)?;
                }
                self.precision_symbols(
                    &mut out,
                    &neighbour,
                    single.mode == SINGLE_MODE_NEWMV,
                    (self.block_precision(motion), single.use_amvd),
                )?;
            }
            EncoderMotion::Compound(compound) => {
                let refinement = self.compound_refinement(frontier, compound)?;
                self.compound_symbols(
                    &mut out,
                    &neighbour,
                    (new_mv, max_drl, refinement.reads_optflow),
                    compound,
                )?;
            }
        }
        Ok(out)
    }

    fn compound_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        neighbour: &BlockNeighbourContext,
        (new_mv, max_drl, reads_optflow): (usize, usize, bool),
        compound: EncoderCompoundMotion,
    ) -> Result<()> {
        let refs = self.num_total_refs();
        let input = self.compound_input(
            compound_ref_contexts(neighbour, refs)?,
            compound_ref_distance_signs(
                &self.ref_frame_idx,
                &self.reference,
                self.current_order_hint,
                refs,
            )?,
        );
        let target = compound.ref_frames;
        let pair = compound_reference_pair(input, |selector| {
            let value = comp_ref_bit(selector, target);
            out.push((selector, value));
            Ok(value)
        })?;
        let is_joint = super::super::super::compound_is_joint_context(
            &self.ref_frame_idx,
            &self.reference.ref_order_hint,
            pair,
            self.current_order_hint,
        )?;
        let y_mode = compound_y_mode(pair, (new_mv, is_joint), |selector| {
            let value = compound_mode_symbol(selector, compound.y_mode);
            out.push((selector, value));
            Ok(value)
        })?;
        if pair != target || y_mode != compound.y_mode {
            return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
        }
        if reads_optflow {
            out.push((
                TileCdfSelector::UseOptflow {
                    ctx: use_optflow_ctx(y_mode),
                },
                u8::from(compound.use_optflow),
            ));
        }
        let amvd_ctx = neighbour.amvd_ctx(pair.0);
        if let Some(selector) = compound_use_amvd_selector(
            self.enable_adaptive_mvd(),
            y_mode,
            compound.use_optflow,
            amvd_ctx,
        ) {
            out.push((selector, 0));
        }
        if y_mode == CompoundYMode::JointNew {
            if compound.jmvd_scale_mode >= JMVD_SCALE_MODES {
                return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
            }
            out.push((TileCdfSelector::JmvdScaleMode, compound.jmvd_scale_mode));
        }
        let selector = |idx| TileCdfSelector::DrlMode { idx, ctx: new_mv };
        match compound.independent_idx {
            _ if !y_mode.reads_drl_idx() => {}
            Some([first, second]) => {
                drl_symbols(out, selector, 0, max_drl, first)?;
                let min = if pair.0 == pair.1 && y_mode == CompoundYMode::NearNear {
                    first + 1
                } else {
                    0
                };
                drl_symbols(out, selector, min, max_drl, second)?;
            }
            None => drl_symbols(out, selector, 0, max_drl, compound.ref_mv_idx)?,
        }
        let precision = BlockPrecisionRecord::most_probable(self.precision);
        self.precision_symbols(out, neighbour, y_mode.has_newmv(), (precision, false))
    }

    /// The parse record of `compound` at `frontier` and `block`, as the
    /// decoder builds it with every masked, weighted and warped tool off.
    pub(super) fn compound_syntax(
        &self,
        frontier: &DecodeBlockFrontier,
        block: &MvBlockContext,
        compound: EncoderCompoundMotion,
    ) -> Result<CompoundMotionSyntax> {
        let refinement = self.compound_refinement(frontier, compound)?;
        let (ref0, ref1) = compound.ref_frames;
        let global = compound.y_mode == CompoundYMode::GlobalGlobal;
        let warp =
            |ref_frame| global_motion_warp(&self.core, ref_frame, false, block.bw4, block.bh4);
        Ok(CompoundMotionSyntax {
            y_mode: compound.y_mode,
            ref_frame1: ref1,
            skip_mode: compound.skip_mode,
            use_optflow: refinement.use_optflow,
            local_warp: false,
            global_warp: if global {
                [warp(ref0), warp(ref1)]
            } else {
                [None, None]
            },
            ref_mv_idx: compound.ref_mv_idx,
            independent_idx: compound.independent_idx,
            mvd: compound.mvd,
            joint: (compound.y_mode == CompoundYMode::JointNew)
                .then(|| self.joint_projection(compound.ref_frames))
                .transpose()?,
            jmvd_scale_mode: compound.jmvd_scale_mode,
            temporal_allowed: compound.skip_mode || ref0 != ref1,
            temporal_first: if compound.skip_mode {
                [false; 2]
            } else {
                [self.temporal_first(ref0), self.temporal_first(ref1)]
            },
            use_refinemv: refinement.refinemv_signalled,
            refinemv_switchable: refinement.refinemv_switchable,
        })
    }

    fn joint_projection(
        &self,
        (ref0, ref1): (i8, i8),
    ) -> Result<super::super::resolve::CompoundJointMvProjection> {
        compound_joint_mv_projection(&self.core, &self.reference, &self.ref_frame_idx, ref0, ref1)
    }

    fn compound_input(
        &self,
        ref_contexts: [usize; 7],
        ref_distance_nonnegative: [bool; 7],
    ) -> CompoundParseInput {
        let refs = self.num_total_refs();
        CompoundParseInput {
            num_total_refs: refs,
            num_same_ref_compound: self
                .num_same_ref_compound
                .min(u8::try_from(refs).unwrap_or(u8::MAX)),
            ref_contexts,
            ref_distance_nonnegative,
        }
    }
}

/// The `comp_ref` value that keeps the § 5.20.7.11 walk on `pair`.
fn comp_ref_bit(selector: TileCdfSelector, pair: (i8, i8)) -> u8 {
    let chosen = match selector {
        TileCdfSelector::CompRef0 { ref_idx, .. } => ref_idx == pair.0 as usize,
        TileCdfSelector::CompRef1 { ref_idx, .. } => ref_idx == pair.1 as usize,
        _ => false,
    };
    u8::from(chosen)
}

/// The `is_joint` or `compound_mode` value that reads as `y_mode`.
fn compound_mode_symbol(selector: TileCdfSelector, y_mode: CompoundYMode) -> u8 {
    match selector {
        TileCdfSelector::IsJoint { .. } => u8::from(y_mode == CompoundYMode::JointNew),
        TileCdfSelector::CompoundModeSameRefs { .. } => (0..4)
            .find(|&v| same_refs_compound_mode(v) == y_mode)
            .unwrap_or(0),
        _ => (0..5)
            .find(|&v| non_joint_compound_mode(v) == y_mode)
            .unwrap_or(0),
    }
}

/// The `drl_mode` symbols that read `value` from index `min` on, below `max`
/// (`read_drl_idx`).
fn drl_symbols(
    out: &mut Vec<(TileCdfSelector, u8)>,
    selector: impl Fn(usize) -> TileCdfSelector,
    min: usize,
    max: usize,
    value: usize,
) -> Result<()> {
    let first = min.min(max);
    if value < first || value > max {
        return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
    }
    for idx in first..max {
        let more = value > idx;
        out.push((selector(idx.min(2)), u8::from(more)));
        if !more {
            break;
        }
    }
    Ok(())
}
