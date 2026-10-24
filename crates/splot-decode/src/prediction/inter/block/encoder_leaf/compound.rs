// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mode syntax of encoder inter leaves: `comp_mode`, the single and compound
//! references, the modes, the DRL indices and `interp_filter` (§ 5.20.7.6,
//! § 5.20.7.10, § 5.20.7.11), produced by the decoder's own reads answering
//! for a target.

use super::super::super::compound::{
    CompoundParseInput, compound_reference_pair, compound_y_mode, non_joint_compound_mode,
    same_refs_compound_mode,
};
use super::super::compound_path::{
    compound_joint_mv_projection, compound_needs_interp_filter, compound_ref_contexts,
    compound_ref_distance_signs, skip_mode_reference_pair,
};
use super::super::resolve::CompoundMotionSyntax;
use super::super::syntax::interp_filter_symbol;
use super::*;

/// `JOINT_NEWMV_SCALE_FACTOR_CNT` (§ 3): the `jmvd_scale_mode` values without
/// adaptive MVD.
pub(crate) const JMVD_SCALE_MODES: u8 = 5;

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
    /// The filter the decoder predicts `motion` at `frontier` with, and the
    /// `interp_filter` symbol it reads for it (§ 5.20.7.6), if any.
    pub(crate) fn interp_filter(
        &self,
        frontier: &DecodeBlockFrontier,
        motion: EncoderMotion,
    ) -> Result<(ReconInterpolationFilter, Option<(TileCdfSelector, u8)>)> {
        if motion.skip_mode() {
            return Ok((ReconInterpolationFilter::EightTapSharp, None));
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
            None,
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
        let new_mv = find_mode_ctx(&self.grid, &block).new_mv_context;
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
                if single.mode != SINGLE_MODE_GLOBALMV {
                    let selector = |idx| TileCdfSelector::DrlMode { idx, ctx: new_mv };
                    drl_symbols(&mut out, selector, 0, max_drl, single.ref_mv_idx)?;
                }
            }
            EncoderMotion::Compound(compound) => {
                self.compound_symbols(&mut out, &neighbour, (new_mv, max_drl), compound)?;
            }
        }
        Ok(out)
    }

    fn compound_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        neighbour: &BlockNeighbourContext,
        (new_mv, max_drl): (usize, usize),
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
        if y_mode == CompoundYMode::JointNew {
            if compound.jmvd_scale_mode >= JMVD_SCALE_MODES {
                return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
            }
            out.push((TileCdfSelector::JmvdScaleMode, compound.jmvd_scale_mode));
        }
        if !y_mode.reads_drl_idx() {
            return Ok(());
        }
        let selector = |idx| TileCdfSelector::DrlMode { idx, ctx: new_mv };
        match compound.independent_idx {
            Some([first, second]) => {
                drl_symbols(out, selector, 0, max_drl, first)?;
                let min = if pair.0 == pair.1 && y_mode == CompoundYMode::NearNear {
                    first + 1
                } else {
                    0
                };
                drl_symbols(out, selector, min, max_drl, second)
            }
            None => drl_symbols(out, selector, 0, max_drl, compound.ref_mv_idx),
        }
    }

    /// The parse record of `compound` at `block`, as the decoder builds it
    /// with every masked, weighted, warped and refined tool off.
    pub(super) fn compound_syntax(
        &self,
        block: &MvBlockContext,
        compound: EncoderCompoundMotion,
    ) -> Result<CompoundMotionSyntax> {
        let (ref0, ref1) = compound.ref_frames;
        let global = compound.y_mode == CompoundYMode::GlobalGlobal;
        let warp =
            |ref_frame| global_motion_warp(&self.core, ref_frame, false, block.bw4, block.bh4);
        Ok(CompoundMotionSyntax {
            y_mode: compound.y_mode,
            ref_frame1: ref1,
            skip_mode: compound.skip_mode,
            use_optflow: false,
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
            use_refinemv: false,
            refinemv_switchable: false,
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
