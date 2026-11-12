// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_recon::wedge_mask_plane_sample;

use super::super::CompoundOrderHint;
use super::super::compound::{
    CompoundParseInput, CompoundYMode, read_compound_mode_syntax, read_compound_reference_pair,
};
use super::super::find_mv_stack::TemporalMotionBlock;
use super::super::read_mv::apply_inter_mvd_sign_pair;
use super::temporal::temporal_motion_block;
use super::warp::mvd_sign_derivation_block_scope_allowed;
use super::*;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const REFINE_SWITCHABLE: u32 = 1;
const REFINE_ALL: u32 = 2;
const MV_PROJECTION_DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];
/// Logical references `0..=TIP_REF_FRAME`, the indices a block can name.
const FACT_REFS: usize = 8;

/// A failed logical-reference lookup, kept `Copy` so the per-frame table can
/// sit in the tile walk parameters.
#[derive(Clone, Copy, Debug)]
enum RefFactError {
    /// The index is outside `ref_frame_idx`.
    List,
    Slot {
        slot: usize,
        slot_count: usize,
    },
}

/// One logical reference's lookups, each with the error its per-block
/// derivation reported first.
#[derive(Clone, Copy, Debug)]
struct RefFact {
    hint: core::result::Result<CompoundOrderHint, RefFactError>,
    /// Width and height.
    dims: core::result::Result<(u32, u32), RefFactError>,
    /// The raw order hint the temporal-first distance reads.
    temporal_hint: Option<i32>,
    /// The raw order hint `ref_dist_gt4` reads, slot 0 for an absent index.
    bawp_hint: i32,
}

/// The frame-constant reference facts every block derived per block before:
/// order hints, distances and sizes of the logical references, looked up
/// once per frame. A block that names a reference whose lookup failed gets
/// the error the per-block derivation returned.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameRefFacts {
    current_order_hint: u32,
    current: CompoundOrderHint,
    frame_size: Option<(u32, u32)>,
    switch: bool,
    list_len: usize,
    refs: [RefFact; FACT_REFS],
    /// The facts of an index past the table.
    absent: RefFact,
    /// § 7.12 comp-mode context: the reference lies after the current frame.
    backward: [bool; FACT_REFS],
    distance_signs: core::result::Result<[bool; 7], RefFactError>,
    /// The § 8.3.2 furthest future reference, or the index whose order hint
    /// is missing.
    furthest_future: core::result::Result<Option<i8>, i8>,
}

impl FrameRefFacts {
    pub(super) fn new<T: ReconSample>(
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        num_total_refs: usize,
    ) -> Self {
        Self::from_parts(
            core.frame_size.map(|size| (size.width, size.height)),
            core.frame_type == Some(FrameType::Switch),
            core.display_order_hint().unwrap_or(0),
            reference,
            ref_frame_idx,
            num_total_refs,
        )
    }

    fn from_parts<T: ReconSample>(
        frame_size: Option<(u32, u32)>,
        switch: bool,
        current_order_hint: u32,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        num_total_refs: usize,
    ) -> Self {
        let current = CompoundOrderHint::current(current_order_hint);
        let slot_value = |values: &[u32], slot: usize| {
            values.get(slot).copied().ok_or(RefFactError::Slot {
                slot,
                slot_count: values.len(),
            })
        };
        let fact = |index: usize| {
            let slot = ref_frame_idx.get(index).map(|&slot| slot as usize);
            let hint = slot
                .ok_or(RefFactError::List)
                .and_then(|slot| slot_value(&reference.ref_order_hint, slot))
                .map(CompoundOrderHint::reference);
            let dims = slot.ok_or(RefFactError::List).and_then(|slot| {
                Ok((
                    slot_value(&reference.ref_frame_width, slot)?,
                    slot_value(&reference.ref_frame_height, slot)?,
                ))
            });
            let raw_hint = |slot: usize| {
                reference
                    .ref_order_hint
                    .get(slot)
                    .map(|&hint| i32::try_from(hint).unwrap_or(i32::MAX))
            };
            RefFact {
                hint,
                dims,
                temporal_hint: slot.and_then(raw_hint),
                bawp_hint: raw_hint(slot.unwrap_or(0)).unwrap_or(0),
            }
        };
        let refs = core::array::from_fn(fact);
        let backward = core::array::from_fn(|index: usize| {
            ref_frame_idx
                .get(index)
                .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
                .is_some_and(|&hint| {
                    CompoundOrderHint::reference(hint).frame_distance_from(current) < 0
                })
        });
        let mut distance_signs = Ok([true; 7]);
        for index in 0..num_total_refs.min(7) {
            let sign = ref_frame_idx
                .get(index)
                .ok_or(RefFactError::List)
                .and_then(|&slot| slot_value(&reference.ref_order_hint, slot as usize))
                .map(|hint| CompoundOrderHint::reference(hint).frame_distance_from(current) >= 0);
            match (sign, &mut distance_signs) {
                (Ok(sign), Ok(signs)) => signs[index] = sign,
                (Err(error), Ok(_)) => distance_signs = Err(error),
                (_, Err(_)) => {}
            }
        }
        let mut furthest_future = Ok(None::<(i64, i8)>);
        for index in 0..num_total_refs {
            let Ok(best) = furthest_future else { break };
            let Some(Ok(hint)) = refs.get(index).map(|fact: &RefFact| fact.hint) else {
                furthest_future = Err(index as i8);
                break;
            };
            let CompoundOrderHint::Value(order_hint) = hint else {
                continue;
            };
            if hint.relative_dist(current) > 0
                && best.is_none_or(|(best_order_hint, _)| order_hint > best_order_hint)
            {
                furthest_future = Ok(Some((order_hint, index as i8)));
            }
        }
        Self {
            current_order_hint,
            current,
            frame_size,
            switch,
            list_len: ref_frame_idx.len(),
            refs,
            absent: fact(usize::MAX),
            backward,
            distance_signs,
            furthest_future: furthest_future.map(|best| best.map(|(_, index)| index)),
        }
    }

    fn error(&self, index: i8, error: RefFactError) -> crate::error::DecodeError {
        match error {
            RefFactError::List => crate::DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                index,
                list_len: self.list_len,
            }
            .into(),
            RefFactError::Slot { slot, slot_count } => {
                crate::DecodeReferenceStateError::SlotOutOfRange { slot, slot_count }.into()
            }
        }
    }

    fn fact(&self, index: i8) -> &RefFact {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.refs.get(index))
            .unwrap_or(&self.absent)
    }

    fn hint(&self, index: i8) -> Result<CompoundOrderHint> {
        self.fact(index)
            .hint
            .map_err(|error| self.error(index, error))
    }

    /// Whether `ref_frame` is scaled against the current frame.
    pub(super) fn scaled(&self, ref_frame: i8) -> Result<bool> {
        let (frame_width, frame_height) = self
            .frame_size
            .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
        let (width, height) = self
            .fact(ref_frame)
            .dims
            .map_err(|error| self.error(ref_frame, error))?;
        Ok(super::super::mv_scaling::reference_is_scaled(
            width as i32,
            height as i32,
            frame_width as i32,
            frame_height as i32,
        ))
    }

    /// Whether `ref_frame` lies within the temporal-first distance.
    pub(super) fn temporal_near(&self, ref_frame: i8) -> bool {
        self.fact(ref_frame).temporal_hint.is_some_and(|hint| {
            super::super::get_relative_dist(self.current_order_hint as i32, hint).abs() <= 2
        })
    }

    /// `bawp` with the § 7.13.3.25 `ref_dist_gt4` of `ref_frame0`.
    pub(super) fn bawp_with_ref_distance(&self, bawp: BawpSyntax, ref_frame0: i8) -> BawpSyntax {
        if !bawp.enabled {
            return bawp;
        }
        BawpSyntax {
            ref_dist_gt4: super::super::get_relative_dist(
                self.fact(ref_frame0).bawp_hint,
                self.current_order_hint as i32,
            )
            .abs()
                > 4,
            ..bawp
        }
    }

    /// The order hint, width and height of `index`.
    fn size(&self, index: i8) -> Result<(CompoundOrderHint, u32, u32)> {
        let fact = self.fact(index);
        let sized = fact.hint.and_then(|hint| Ok((hint, fact.dims?)));
        let (hint, (width, height)) = sized.map_err(|error| self.error(index, error))?;
        Ok((hint, width, height))
    }

    /// The order hints of a compound pair whose references are unrestricted
    /// and unscaled, `None` otherwise or on a switch frame.
    fn sized_pair(
        &self,
        ref_frame0: i8,
        ref_frame1: i8,
    ) -> Result<Option<(CompoundOrderHint, CompoundOrderHint)>> {
        if self.switch {
            return Ok(None);
        }
        let (width, height) = self
            .frame_size
            .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
        let (ref0, ref1) = (self.size(ref_frame0)?, self.size(ref_frame1)?);
        let full = |(hint, w, h): (CompoundOrderHint, u32, u32)| {
            !hint.is_restricted() && w == width && h == height
        };
        Ok((full(ref0) && full(ref1)).then_some((ref0.0, ref1.0)))
    }

    /// The § 5.20.7.4 per-reference distance signs.
    pub(super) fn distance_signs(&self) -> Result<[bool; 7]> {
        self.distance_signs.map_err(|error| match error {
            RefFactError::List => DecodeHeaderStateError::InvalidInterReferenceMap.into(),
            error @ RefFactError::Slot { .. } => self.error(0, error),
        })
    }

    /// The § 8.3.2 `compound_is_joint` context of a compound pair.
    pub(super) fn is_joint_context(&self, (ref_frame0, ref_frame1): (i8, i8)) -> Result<usize> {
        Ok(super::super::compound_is_joint_context_from_order_hints(
            self.hint(ref_frame0)?,
            self.hint(ref_frame1)?,
            self.current,
        ))
    }

    /// Per logical reference: whether it lies after the current frame.
    pub(super) const fn backward(&self) -> &[bool; FACT_REFS] {
        &self.backward
    }

    fn furthest_future_ref(&self) -> Result<Option<i8>> {
        self.furthest_future
            .or_else(|index| self.hint(index).map(|_| None))
    }
}

pub(super) fn read_reference_mode(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    neighbour_ctx: &BlockNeighbourContext,
    facts: &FrameRefFacts,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let ctx = neighbour_ctx.comp_mode_ctx(facts.backward());
    let mode = cdfs
        .read_block_symbol_trace(TileCdfSelector::CompMode { ctx }, symbols)
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    match mode {
        0 => Ok(false),
        _ => Ok(true),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_compound_inter_block(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    residual_arena: &mut Vec<i32>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    tip_ref_pair: Option<(i8, i8)>,
    block_ctx: &mut MvBlockContext,
    neighbour_ctx: &BlockNeighbourContext,
    filter_records: &mut tile::TileFilterRecords,
    slots: &mut Vec<tile::LeafSlot>,
    intrabc_state: &mut TileIntrabcPreludeState,
    facts: &FrameRefFacts,
    num_total_refs: usize,
    num_same_ref_compound: u8,
    skip: u8,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    temporal_first_frame: bool,
    enable_adaptive_mvd: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, LeafResolveRecord)> {
    let opfl_refine_type = core
        .inter
        .as_ref()
        .and_then(|inter| inter.opfl_refine_type)
        .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
    let enable_refinemv = sequence
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?
        .enable_refinemv;
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let ref_contexts = compound_ref_contexts(neighbour_ctx, num_total_refs)?;
    let ref_distance_nonnegative = facts.distance_signs()?;
    let pair = read_compound_reference_pair(
        cdfs,
        symbols,
        CompoundParseInput {
            num_total_refs,
            num_same_ref_compound,
            ref_contexts,
            ref_distance_nonnegative,
        },
        tile_offset,
    )?;
    let compound_is_joint_ctx = facts.is_joint_context(pair)?;
    block_ctx.ref_frame0 = pair.0;
    block_ctx.ref_frame1 = Some(pair.1);
    let mode_ctx = mode_ctx_from_neighbours(mv_grid, block_ctx, tip_ref_pair, neighbour_ctx);
    let mut compound = read_compound_mode_syntax(
        cdfs,
        symbols,
        pair,
        mode_ctx.new_mv_context,
        compound_is_joint_ctx,
        tile_offset,
    )?;
    if compound_switchable_opfl_reachable(opfl_refine_type, facts, compound, n4w, n4h)? {
        compound.use_optflow =
            read_compound_use_optflow_syntax(cdfs, symbols, compound.y_mode, tile_offset)?;
    }
    let use_amvd = read_compound_use_amvd_syntax(
        cdfs,
        symbols,
        enable_adaptive_mvd,
        compound.y_mode,
        compound.use_optflow,
        neighbour_ctx.amvd_ctx(compound.ref_frame0),
        tile_offset,
    )?;
    let frame_modes = core
        .inter
        .as_ref()
        .and_then(|inter| inter.frame_enabled_motion_modes)
        .unwrap_or([false; splot_core::headers::frame::MOTION_MODES]);
    let signal_local_warp = compound_local_warp_signal_allowed(
        compound,
        n4w,
        n4h,
        effective_force_integer_mv(core),
        opfl_refine_type,
        [mode_ctx.warp_sample_found, mode_ctx.warp_sample_found1],
        frame_modes[splot_core::headers::frame::LOCALWARP],
    );
    let local_warp = read_compound_motion_mode_syntax(
        cdfs,
        symbols,
        signal_local_warp,
        neighbour_ctx,
        tile_offset,
    )?;
    let jmvd_scale_mode = read_compound_jmvd_scale_mode_syntax(
        cdfs,
        symbols,
        compound.y_mode,
        use_amvd,
        tile_offset,
    )?;
    let mut ref_mv_idx = 0;
    let mut ref_mv_idx0 = 0;
    let mut ref_mv_idx1 = 0;
    if compound.y_mode.reads_drl_idx() {
        if compound_reads_second_drl(compound) {
            ref_mv_idx0 = read_drl_idx(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?;
            let second_min_idx = if compound.ref_frame0 == compound.ref_frame1
                && compound.y_mode == CompoundYMode::NearNear
            {
                ref_mv_idx0.saturating_add(1)
            } else {
                0
            };
            ref_mv_idx1 = read_drl_idx_from(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                second_min_idx,
                tile_offset,
            )?;
        } else {
            ref_mv_idx = read_drl_idx(
                cdfs,
                symbols,
                mode_ctx.new_mv_context,
                max_drl_bits_minus_1,
                tile_offset,
            )?;
        }
    }
    let frame_mv_config = inter_mv_read_config(core)?;
    let precision = read_block_mv_precision_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        neighbour_ctx,
        frame_mv_config.precision(),
        compound.y_mode.has_newmv(),
        use_amvd,
        tile_offset,
    )?;
    let joint = (compound.y_mode == CompoundYMode::JointNew)
        .then(|| compound_joint_mv_projection(facts, compound.ref_frame0, compound.ref_frame1))
        .transpose()?;
    let mvd = read_compound_mvd_syntax(
        cdfs,
        symbols,
        sequence,
        core,
        CompoundMvdInput {
            y_mode: compound.y_mode,
            use_amvd,
            ref_mv_idx,
            precision,
            frame_mv_config,
            motion_mode: if local_warp {
                MotionMode::LocalWarp
            } else {
                MotionMode::Simple
            },
        },
        tile_offset,
    )?;
    let refinemv_switchable = compound_refinemv_is_switchable(compound, opfl_refine_type);
    let refinemv_signalled = if !local_warp
        && compound_refinemv_reachable(
            enable_refinemv,
            opfl_refine_type,
            facts,
            compound,
            n4w,
            n4h,
        )? {
        if refinemv_switchable {
            read_compound_use_refinemv_syntax(
                cdfs,
                symbols,
                compound.y_mode,
                compound.use_optflow,
                tile_offset,
            )?
        } else {
            true
        }
    } else {
        false
    };
    let compound_blend_tools = CompoundBlendToolConfig::from_sequence(sequence);
    let compound_blend_thin = compound_blend_is_thin(n4w, n4h);
    let comp_group_idx_ctx = if !compound_blend_tools.masked_enabled || compound_blend_thin {
        0
    } else {
        compound_group_idx_context(
            neighbour_ctx,
            facts,
            compound.ref_frame0,
            compound.ref_frame1,
        )?
    };
    let compound_blend = read_compound_blend_syntax(
        cdfs,
        symbols,
        compound_blend_tools,
        CompoundBlendInput {
            use_optflow: compound.use_optflow,
            joint_amvd: compound.y_mode == CompoundYMode::JointNew && use_amvd,
            switchable_refinemv_on: refinemv_signalled && refinemv_switchable,
            n4w,
            n4h,
            block_size_index: frontier.b_size.index(),
            comp_group_idx_ctx,
        },
        tile_offset,
    )?;
    let use_refinemv = compound_refinemv_active_after_blend(refinemv_signalled, compound_blend);
    let compound_blend = read_compound_cwp_syntax(
        cdfs,
        symbols,
        CompoundCwpContext { sequence, facts },
        CompoundCwpInput {
            y_mode: compound.y_mode,
            jmvd_scale_mode,
            use_optflow: compound.use_optflow,
            use_refinemv,
            motion_simple: !local_warp,
            ref_frame0: compound.ref_frame0,
            ref_frame1: compound.ref_frame1,
            blend: compound_blend,
        },
        tile_offset,
    )?;
    let compound_blend = compound_warp_blend(compound_blend, local_warp, compound.y_mode);
    if compound_all_opfl_reachable(opfl_refine_type, facts, compound, n4w, n4h, compound_blend)? {
        compound.use_optflow = true;
    }
    let interp = resolve_compound_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        compound.use_optflow || refinemv_signalled,
        compound_needs_interp_filter(n4w, n4h, compound.y_mode, local_warp),
        neighbour_ctx.interp_filter_ctx(compound.ref_frame0, true),
        tile_offset,
    )?;
    let temporal_allowed = compound.ref_frame0 != compound.ref_frame1;
    let temporal_first = |ref_frame| temporal_first_frame && facts.temporal_near(ref_frame);
    finish_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        residual_scratch,
        residual_blocks,
        residual_arena,
        sequence,
        core,
        frontier,
        mv_grid,
        frame_mv_config.precision(),
        filter_records,
        slots,
        intrabc_state,
        facts,
        ParsedCompoundBlock {
            block_ctx: *block_ctx,
            motion: CompoundMotionSyntax {
                y_mode: compound.y_mode,
                ref_frame1: compound.ref_frame1,
                skip_mode: false,
                use_optflow: compound.use_optflow,
                local_warp,
                ref_mv_idx,
                independent_idx: compound_reads_second_drl(compound)
                    .then_some([ref_mv_idx0, ref_mv_idx1]),
                mvd,
                joint,
                jmvd_scale_mode,
                temporal_allowed,
                temporal_first: [
                    temporal_first(compound.ref_frame0),
                    temporal_first(compound.ref_frame1),
                ],
                use_refinemv,
                refinemv_switchable,
            },
            blend: compound_blend,
            interp,
            use_amvd,
            precision,
        },
        skip,
        mi_rows,
        mi_cols,
        residual_tool_policy,
        block_qindex,
        tile_offset,
    )
}

fn resolve_compound_interp_filter(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    frame_interpolation_filter: FrameInterpolationFilter,
    force_sharp: bool,
    needs_interp_filter: bool,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<ReconInterpolationFilter> {
    if force_sharp {
        return Ok(ReconInterpolationFilter::EightTapSharp);
    }
    resolve_interp_filter(
        cdfs,
        symbols,
        frame_interpolation_filter,
        needs_interp_filter,
        ctx,
        tile_offset,
    )
}

/// The signed order-hint distances of an optical-flow compound block
/// `motion` over `ref_frame0` to its two references; `None` without optical
/// flow.
pub(super) fn compound_optflow_distances(
    facts: &FrameRefFacts,
    ref_frame0: i8,
    motion: &CompoundMotionSyntax,
) -> Result<Option<[i32; 2]>> {
    if !motion.use_optflow {
        return Ok(None);
    }
    Ok(facts
        .sized_pair(ref_frame0, motion.ref_frame1)?
        .map(|(ref0, ref1)| {
            [
                facts.current.relative_dist(ref0),
                facts.current.relative_dist(ref1),
            ]
        }))
}

pub(super) fn compound_needs_interp_filter(
    n4w: usize,
    n4h: usize,
    y_mode: CompoundYMode,
    local_warp: bool,
) -> bool {
    !local_warp && (n4w < 2 || n4h < 2 || y_mode != CompoundYMode::GlobalGlobal)
}

pub(super) fn compound_local_warp_signal_allowed(
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    force_integer_mv: bool,
    opfl_refine_type: u32,
    warp_sample_found: [bool; 2],
    local_warp_enabled: bool,
) -> bool {
    compound.y_mode == CompoundYMode::NewNew
        && compound.ref_frame0 != compound.ref_frame1
        && n4w >= 2
        && n4h >= 2
        && !force_integer_mv
        && !compound.use_optflow
        && opfl_refine_type != REFINE_ALL
        && warp_sample_found[0]
        && warp_sample_found[1]
        && local_warp_enabled
}

/// AV2 § 7.13.3.23: § 7.12.3 warp-sample search plus least-squares estimation
/// once per reference list. A signalled list can gather no samples because the
/// § 5.20.7.14 `WarpSampleFound` scan is wider than the § 7.12.3.1 gathering.
/// AVM marks that list's warp model invalid, which selects the translational
/// prediction equivalent to the spec's det==0 identity model.
#[allow(clippy::too_many_arguments)]
pub(super) fn compound_local_warp_models(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    ref_frame1: i8,
    mv0: Mv,
    mv1: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
) -> Result<[Option<[i32; 6]>; 2]> {
    let model0 = compound_ref_warp_model(
        mv_grid,
        block_ctx,
        block_ctx.ref_frame0,
        mv0,
        mi_row,
        mi_col,
        n4w,
        n4h,
    )?;
    let model1 = compound_ref_warp_model(
        mv_grid, block_ctx, ref_frame1, mv1, mi_row, mi_col, n4w, n4h,
    )?;
    Ok([model0, model1])
}

#[allow(clippy::too_many_arguments)]
fn compound_ref_warp_model(
    mv_grid: &NeighbourMvGrid,
    block_ctx: &MvBlockContext,
    target_ref: i8,
    mv: Mv,
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
) -> Result<Option<[i32; 6]>> {
    let samples = super::super::find_mv_stack::find_warp_samples(mv_grid, block_ctx, target_ref);
    if samples.is_empty() {
        return Ok(None);
    }
    Ok(Some(local_warp_estimation(
        &samples, mv, mi_row, mi_col, n4w, n4h,
    )?))
}

/// AV2 § 7.13.3.14: compound LOCALWARP (`compoundWarp = 1`) and
/// `GLOBAL_GLOBALMV` disable the implicit-mask average branch, so
/// COMPOUND_AVERAGE collapses to the plain weighted average; wedge and
/// diff-weighted blends are unaffected.
pub(super) fn compound_warp_blend(
    blend: mc::CompoundBlend,
    local_warp: bool,
    y_mode: CompoundYMode,
) -> mc::CompoundBlend {
    if !local_warp && y_mode != CompoundYMode::GlobalGlobal {
        return blend;
    }
    match blend {
        mc::CompoundBlend::Average { cwp_weight, .. } => {
            mc::CompoundBlend::average_with_implicit_mask(false).average_with_cwp_weight(cwp_weight)
        }
        other => other,
    }
}

/// AV2 § 5.20.7.14 `read_motion_mode`: reads `use_local_warp` when LOCALWARP is
/// allowed and returns whether the block uses compound LOCALWARP (AVM WARP_CAUSAL).
fn read_compound_motion_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    signal_local_warp: bool,
    neighbour_ctx: &BlockNeighbourContext,
    tile_offset: ByteOffset,
) -> Result<bool> {
    if !signal_local_warp {
        return Ok(false);
    }
    let use_local_warp = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseLocalWarp {
                ctx: neighbour_ctx.use_local_warp_ctx(),
            },
            symbols,
        )
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    Ok(use_local_warp != 0)
}

/// Parsed § 5.20.7.6 compound syntax handed to the § 7.12 resolution step.
#[derive(Clone, Copy)]
pub(super) struct ParsedCompoundBlock {
    pub(super) block_ctx: MvBlockContext,
    pub(super) motion: CompoundMotionSyntax,
    pub(super) blend: mc::CompoundBlend,
    pub(super) interp: ReconInterpolationFilter,
    pub(super) use_amvd: bool,
    pub(super) precision: BlockPrecisionRecord,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_compound_inter_block(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    residual_arena: &mut Vec<i32>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    frame_precision: u8,
    filter_records: &mut tile::TileFilterRecords,
    slots: &mut Vec<tile::LeafSlot>,
    intrabc_state: &mut TileIntrabcPreludeState,
    facts: &FrameRefFacts,
    parsed: ParsedCompoundBlock,
    skip: u8,
    mi_rows: usize,
    mi_cols: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, LeafResolveRecord)> {
    let ParsedCompoundBlock {
        block_ctx,
        motion,
        blend,
        interp,
        use_amvd,
        precision,
    } = parsed;
    let (mi_row, mi_col, n4w, n4h) = (
        block_ctx.mi_row,
        block_ctx.mi_col,
        block_ctx.bw4,
        block_ctx.bh4,
    );
    let residual = if skip == 0 {
        Some(read_inter_residual(
            work_unit,
            symbols,
            coeff_ctx,
            residual_scratch,
            residual_blocks,
            residual_arena,
            sequence,
            core,
            frontier,
            n4w,
            n4h,
            mi_rows,
            mi_cols,
            current_residual_lossless(work_unit),
            InterResidualLumaTxSizeMode::Inter,
            residual_tool_policy,
            tile_offset,
        )?)
    } else {
        reset_inter_skip_coeff_contexts(
            coeff_ctx,
            frontier,
            n4w,
            n4h,
            super::block_chroma_subsampling(sequence.general.chroma_format_idc),
        )?;
        None
    };
    let sub_pu_size = compound_deblock_sub_pu_size(
        motion.use_optflow,
        motion.use_refinemv,
        n4w * MI_SIZE,
        n4h * MI_SIZE,
    );
    filter_records.defer_inter_leaf(
        frontier,
        (n4w, n4h),
        residual.as_ref(),
        sub_pu_size,
        block_qindex,
        current_residual_lossless(work_unit),
    );
    let geometry = inter_leaf_geometry(
        frontier,
        work_unit,
        (n4w, n4h),
        sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
    )?;
    let optflow_distances = compound_optflow_distances(facts, block_ctx.ref_frame0, &motion)?;
    intrabc_state.record_block(
        frontier.r,
        frontier.c,
        n4w,
        n4h,
        IntrabcBlockPrelude::from_use_skip(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: skip == 1,
        })
        .mark_inter(),
    )?;
    let syntax = InterBlockSyntax {
        block_ctx,
        motion: InterMotionSyntax::Compound(motion),
        interp,
        precision,
        skip: skip == 1,
        use_amvd,
        tip_size_16x16: false,
        blend,
        bawp: BawpSyntax::default(),
        interintra: None,
        optflow_distances,
        residual,
    };
    mv_grid.record_flags(mi_row, mi_col, n4w, n4h, syntax.flag_syntax());
    Ok((
        GeneralIntraLeafMode::no_luma_mode(),
        pending_inter_leaf(
            slots,
            syntax,
            geometry,
            block_qindex,
            frame_precision,
            tile_offset,
        ),
    ))
}

/// Input to the § 5.20.7.7 compound motion-vector difference reads.
#[derive(Clone, Copy)]
struct CompoundMvdInput {
    y_mode: CompoundYMode,
    use_amvd: bool,
    ref_mv_idx: usize,
    precision: BlockPrecisionRecord,
    frame_mv_config: MvReadConfig,
    motion_mode: MotionMode,
}

/// AV2 § 5.20.7.7 compound motion-vector differences: one per NEWMV list, with
/// JOINT_NEWMV reading a single difference the § 7.12.2.4 projection reuses.
fn read_compound_mvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    input: CompoundMvdInput,
    tile_offset: ByteOffset,
) -> Result<[Mv; 2]> {
    if !input.y_mode.has_newmv() {
        return Ok([Mv::ZERO; 2]);
    }
    let config = MvReadConfig::inter(input.precision.mv_precision);
    let threshold = input.y_mode.mvd_sign_derivation_threshold();
    let derive_sign =
        mvd_sign_derivation_block_scope_allowed(input.motion_mode, false, Some(input.ref_mv_idx))
            && inter_mvd_sign_derivation_allowed(
                sequence,
                core,
                SINGLE_MODE_NEWMV,
                input.use_amvd,
                input.frame_mv_config,
                config,
            );
    match input.y_mode {
        CompoundYMode::NearNew | CompoundYMode::NewNear => {
            let magnitude =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let diff =
                apply_inter_mvd_signs(magnitude, symbols, tile_offset, config, false, threshold)?;
            let mut mvd = [Mv::ZERO; 2];
            mvd[usize::from(input.y_mode == CompoundYMode::NearNew)] = diff;
            Ok(mvd)
        }
        CompoundYMode::JointNew => {
            let magnitude =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let diff = apply_inter_mvd_signs(
                magnitude,
                symbols,
                tile_offset,
                config,
                derive_sign,
                threshold,
            )?;
            Ok([diff, Mv::ZERO])
        }
        CompoundYMode::NewNew => {
            let magnitude0 =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let magnitude1 =
                read_compound_mvd_magnitude(cdfs, symbols, input.use_amvd, config, tile_offset)?;
            let (diff0, diff1) = apply_inter_mvd_sign_pair(
                magnitude0,
                magnitude1,
                symbols,
                tile_offset,
                config,
                derive_sign,
                threshold,
            )?;
            Ok([diff0, diff1])
        }
        CompoundYMode::NearNear | CompoundYMode::GlobalGlobal => Ok([Mv::ZERO; 2]),
    }
}

fn read_compound_mvd_magnitude(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    use_amvd: bool,
    config: MvReadConfig,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    if use_amvd {
        read_newmv_amvd_block_mvd(cdfs, symbols, tile_offset)
    } else {
        read_newmv_block_mvd_magnitude(cdfs, symbols, tile_offset, config)
    }
}

pub(super) const fn compound_deblock_sub_pu_size(
    use_optflow: bool,
    use_refinemv: bool,
    luma_width: usize,
    luma_height: usize,
) -> Option<crate::filters::deblock::DeblockSubPuSize> {
    if use_optflow {
        let size = super::super::mc::optflow_unit_size(luma_width, luma_height);
        Some(crate::filters::deblock::DeblockSubPuSize::square(size))
    } else if use_refinemv {
        Some(crate::filters::deblock::DeblockSubPuSize::new(
            if luma_width < 16 { luma_width } else { 16 },
            if luma_height < 16 { luma_height } else { 16 },
        ))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_compound_temporal_motion<T: ReconSample>(
    records: &mut Vec<TemporalMotionBlock>,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    placed: &PlacedInterBlock,
    compound: super::super::compound::CompoundBlockSyntax,
    warp_params: [Option<[i32; 6]>; 2],
    motion_grid: Option<&mc::CompoundMotionGrid>,
    mi_row: usize,
    mi_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
) -> Result<()> {
    let uniform = motion_grid.is_none()
        && warp_params == [None, None]
        && !matches!(placed.block.compound_blend, mc::CompoundBlend::Wedge { .. });
    let (step_w, step_h) = if uniform {
        (placed.luma_w.max(8), placed.luma_h.max(8))
    } else {
        (8, 8)
    };
    let first = records.len();
    for y in (0..placed.luma_h).step_by(step_h) {
        for x in (0..placed.luma_w).step_by(step_w) {
            let mvs = if let Some(grid) = motion_grid {
                grid.temporal_mvs_at_luma_offset(x, y)?
            } else {
                let unit_mv = |params: Option<[i32; 6]>, block_mv: Mv| {
                    params.map_or(block_mv, |params| {
                        super::super::find_mv_stack::warp_sub_mv_at(
                            params,
                            mi_row,
                            mi_col,
                            mi_row + y / 4,
                            mi_col + x / 4,
                        )
                    })
                };
                [
                    unit_mv(warp_params[0], compound.mv0),
                    unit_mv(warp_params[1], compound.mv1),
                ]
            };
            let allowed = wedge_temporal_allowed_lists(
                placed.block.compound_blend,
                placed.luma_w,
                placed.luma_h,
                x,
                y,
            )?;
            let (ref_frame0, ref_frame1, mvs) = match allowed {
                [true, false] => (compound.ref_frame0, None, [mvs[0], Mv::ZERO]),
                [false, true] => (compound.ref_frame1, None, [mvs[1], Mv::ZERO]),
                _ => (compound.ref_frame0, Some(compound.ref_frame1), mvs),
            };
            let record = temporal_motion_block(
                reference,
                ref_frame_idx,
                mi_row + y / 4,
                mi_col + x / 4,
                (placed.luma_w - x).min(step_w).div_ceil(4),
                (placed.luma_h - y).min(step_h).div_ceil(4),
                mi_rows,
                mi_cols,
                current_order_hint,
                ref_frame0,
                ref_frame1,
                mvs[0],
                mvs[1],
                [None, None],
            );
            TemporalMotionBlock::push_merged(records, first, record);
        }
    }
    Ok(())
}

fn wedge_temporal_allowed_lists(
    blend: mc::CompoundBlend,
    luma_width: usize,
    luma_height: usize,
    x: usize,
    y: usize,
) -> Result<[bool; 2]> {
    let mc::CompoundBlend::Wedge { index, sign } = blend else {
        return Ok([true; 2]);
    };
    let mut dominant = [0usize; 2];
    for row in y..y + 8 {
        for col in x..x + 8 {
            let mask = wedge_mask_plane_sample(
                luma_width,
                luma_height,
                usize::from(index),
                sign,
                0,
                0,
                col,
                row,
            )?;
            dominant[0] += usize::from(mask > 60);
            dominant[1] += usize::from(mask < 4);
        }
    }
    Ok(if dominant[0] >= 60 {
        [true, false]
    } else if dominant[1] >= 60 {
        [false, true]
    } else {
        [true; 2]
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_skip_mode_inter_block(
    work_unit: &mut DecodeTileWorkUnit,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    residual_scratch: &mut InterResidualParseScratch,
    residual_blocks: &mut Vec<InterResidualBlock>,
    residual_arena: &mut Vec<i32>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    mv_grid: &mut NeighbourMvGrid,
    block_ctx: &mut MvBlockContext,
    neighbour_ctx: &BlockNeighbourContext,
    tip_ref_pair: Option<(i8, i8)>,
    filter_records: &mut tile::TileFilterRecords,
    slots: &mut Vec<tile::LeafSlot>,
    intrabc_state: &mut TileIntrabcPreludeState,
    facts: &FrameRefFacts,
    num_total_refs: usize,
    skip: u8,
    mi_rows: usize,
    mi_cols: usize,
    max_drl_bits_minus_1: u32,
    residual_tool_policy: TransformToolResidualPolicy,
    block_qindex: u32,
    tile_offset: ByteOffset,
) -> Result<(GeneralIntraLeafMode, LeafResolveRecord)> {
    let (ref_frame0, ref_frame1) = skip_mode_reference_pair(
        facts,
        num_total_refs,
        neighbour_ctx,
        tip_ref_pair,
        tile_offset,
    )?;
    block_ctx.ref_frame0 = ref_frame0;
    block_ctx.ref_frame1 = Some(ref_frame1);
    let ref_mv_idx = read_skip_drl_idx(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        max_drl_bits_minus_1,
        tile_offset,
    )?;
    let precision = frame_mv_precision(core)?;
    finish_compound_inter_block(
        work_unit,
        symbols,
        coeff_ctx,
        residual_scratch,
        residual_blocks,
        residual_arena,
        sequence,
        core,
        frontier,
        mv_grid,
        precision,
        filter_records,
        slots,
        intrabc_state,
        facts,
        ParsedCompoundBlock {
            block_ctx: *block_ctx,
            motion: CompoundMotionSyntax {
                y_mode: CompoundYMode::NearNear,
                ref_frame1,
                skip_mode: true,
                use_optflow: false,
                local_warp: false,
                ref_mv_idx,
                independent_idx: None,
                mvd: [Mv::ZERO; 2],
                joint: None,
                jmvd_scale_mode: 0,
                temporal_allowed: true,
                temporal_first: [false; 2],
                use_refinemv: false,
                refinemv_switchable: false,
            },
            blend: mc::CompoundBlend::average_with_implicit_mask(
                CompoundBlendToolConfig::from_sequence(sequence).implicit_mask,
            ),
            interp: ReconInterpolationFilter::EightTapSharp,
            use_amvd: false,
            precision: BlockPrecisionRecord::most_probable(precision),
        },
        skip,
        mi_rows,
        mi_cols,
        residual_tool_policy,
        block_qindex,
        tile_offset,
    )
}

/// The § 5.20.7.10 `skip_mode_frames` reference pair of a skip-mode block
/// whose neighbours give `neighbour_ctx`; the encoder facade calls it too.
#[allow(
    clippy::inline_always,
    reason = "keeps the parse path of the parent with the facade as second caller"
)]
#[inline(always)]
pub(super) fn skip_mode_reference_pair(
    facts: &FrameRefFacts,
    num_total_refs: usize,
    neighbour_ctx: &BlockNeighbourContext,
    tip_ref_pair: Option<(i8, i8)>,
    tile_offset: ByteOffset,
) -> Result<(i8, i8)> {
    let ref_order_hints = if num_total_refs > 1 {
        Some((facts.hint(0)?, facts.hint(1)?))
    } else {
        None
    };
    let default_pair = skip_mode_default_pair(facts.current, ref_order_hints);
    checked_skip_mode_reference_pair(
        neighbour_ctx.skip_mode_ref_pair(default_pair, tip_ref_pair),
        num_total_refs,
        tile_offset,
    )
}

/// AV2 § 5.9.22 `skip_mode_params` fallback reference pair.
fn skip_mode_default_pair(
    current: CompoundOrderHint,
    order_hints: Option<(CompoundOrderHint, CompoundOrderHint)>,
) -> (i8, i8) {
    let Some((order_hint0, order_hint1)) = order_hints else {
        return (0, 0);
    };
    let distance = |reference: CompoundOrderHint| {
        if reference.is_restricted() {
            0
        } else {
            current.relative_dist(reference).abs()
        }
    };
    let second = i8::from((distance(order_hint0) - distance(order_hint1)).abs() <= 1);
    (0, second)
}

fn checked_skip_mode_reference_pair(
    pair: (i8, i8),
    num_total_refs: usize,
    tile_offset: ByteOffset,
) -> Result<(i8, i8)> {
    for reference in [pair.0, pair.1] {
        if reference < 0 || reference as usize >= num_total_refs {
            return Err(crate::pipeline::malformed_tile_payload(
                tile_offset,
                SPEC_MODE_INFO,
                DecodeReferenceStateError::ReferenceListIndexOutOfRange {
                    index: reference,
                    list_len: num_total_refs,
                },
            ));
        }
    }
    Ok(pair)
}

pub(super) fn compound_switchable_opfl_reachable(
    opfl_refine_type: u32,
    facts: &FrameRefFacts,
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
) -> Result<bool> {
    if compound.y_mode == CompoundYMode::GlobalGlobal {
        return Ok(false);
    }
    if opfl_refine_type != REFINE_SWITCHABLE || !compound_opfl_block_size_allowed(n4w, n4h) {
        return Ok(false);
    }
    compound_opfl_reference_allowed(facts, compound)
}

fn read_compound_use_optflow_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let use_optflow = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseOptflow {
                ctx: use_optflow_ctx(y_mode),
            },
            symbols,
        )
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    Ok(use_optflow.get() != 0)
}

/// The § 8.3.2 `use_refinemv` context `1 + (YMode - NEAR_NEARMV) + 6 *
/// use_optflow`, reduced by one for optical-flow modes past
/// `GLOBAL_GLOBALMV`.
pub(super) const fn use_refinemv_ctx(y_mode: CompoundYMode, use_optflow: bool) -> usize {
    let mode_delta = match y_mode {
        CompoundYMode::NearNear => 0usize,
        CompoundYMode::NearNew => 1,
        CompoundYMode::NewNear => 2,
        CompoundYMode::GlobalGlobal => 3,
        CompoundYMode::NewNew => 4,
        CompoundYMode::JointNew => 5,
    };
    let ctx = 1 + mode_delta + 6 * use_optflow as usize;
    if use_optflow && mode_delta > 3 {
        ctx - 1
    } else {
        ctx
    }
}

/// The § 5.20.7.6 `use_optflow` context: `0` for `NEAR_NEARMV`.
pub(super) fn use_optflow_ctx(y_mode: CompoundYMode) -> usize {
    usize::from(y_mode != CompoundYMode::NearNear)
}

/// AV2 § 5.20.7.17 switchable `use_refinemv` read over
/// `TileUseRefinemvCdf[ctx]` with the [`use_refinemv_ctx`] context.
fn read_compound_use_refinemv_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    use_optflow: bool,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let use_refinemv = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::UseRefinemv {
                ctx: use_refinemv_ctx(y_mode, use_optflow),
            },
            symbols,
        )
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    Ok(use_refinemv.get() != 0)
}

const fn compound_reads_second_drl(compound: super::super::compound::CompoundBlockSyntax) -> bool {
    !compound.use_optflow && compound.y_mode.has_second_drl()
}

pub(super) fn select_near_near_candidates(
    independent_indices: Option<[usize; 2]>,
    paired_idx: usize,
    paired: impl FnOnce(usize) -> [Mv; 2],
    independent: impl FnOnce([usize; 2]) -> [Mv; 2],
) -> [Mv; 2] {
    match independent_indices {
        Some(indices) => independent(indices),
        None => paired(paired_idx),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compound_all_opfl_reachable(
    opfl_refine_type: u32,
    facts: &FrameRefFacts,
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    blend: mc::CompoundBlend,
) -> Result<bool> {
    if opfl_refine_type != REFINE_ALL || !compound_all_opfl_block_allowed(compound, n4w, n4h, blend)
    {
        return Ok(false);
    }
    compound_opfl_reference_allowed(facts, compound)
}

fn compound_all_opfl_block_allowed(
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
    blend: mc::CompoundBlend,
) -> bool {
    compound_opfl_block_size_allowed(n4w, n4h)
        && compound.y_mode != CompoundYMode::GlobalGlobal
        && matches!(blend, mc::CompoundBlend::Average { .. })
        && blend.cwp_weight() == mc::CWP_EQUAL
}

const fn compound_opfl_block_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2
}

fn compound_opfl_reference_allowed(
    facts: &FrameRefFacts,
    compound: super::super::compound::CompoundBlockSyntax,
) -> Result<bool> {
    let Some((ref0, ref1)) = facts.sized_pair(compound.ref_frame0, compound.ref_frame1)? else {
        return Ok(false);
    };
    let d0 = facts.current.relative_dist(ref0);
    let d1 = facts.current.relative_dist(ref1);
    Ok((d0 <= 0) ^ (d1 <= 0))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compound_refinemv_reachable(
    enable_refinemv: bool,
    opfl_refine_type: u32,
    facts: &FrameRefFacts,
    compound: super::super::compound::CompoundBlockSyntax,
    n4w: usize,
    n4h: usize,
) -> Result<bool> {
    if !enable_refinemv || !compound_refinemv_size_allowed(n4w, n4h) {
        return Ok(false);
    }
    if !compound_refinemv_mode_allowed(compound, opfl_refine_type) {
        return Ok(false);
    }
    compound_refinemv_reference_allowed(facts, compound)
}

const fn compound_refinemv_size_allowed(n4w: usize, n4h: usize) -> bool {
    n4w >= 2 && n4h >= 2 && (n4w >= 4 || n4h >= 4)
}

const fn compound_refinemv_mode_allowed(
    compound: super::super::compound::CompoundBlockSyntax,
    opfl_refine_type: u32,
) -> bool {
    !(matches!(compound.y_mode, CompoundYMode::GlobalGlobal)
        || opfl_refine_type == REFINE_SWITCHABLE
            && compound.y_mode.has_newmv()
            && !compound.use_optflow)
}

pub(super) fn compound_refinemv_is_switchable(
    compound: super::super::compound::CompoundBlockSyntax,
    opfl_refine_type: u32,
) -> bool {
    compound.y_mode != CompoundYMode::NearNear
        && !(compound.y_mode == CompoundYMode::JointNew
            && compound.use_optflow
            && opfl_refine_type == REFINE_SWITCHABLE)
}

pub(super) const fn compound_refinemv_active_after_blend(
    refinemv_default: bool,
    blend: mc::CompoundBlend,
) -> bool {
    refinemv_default && matches!(blend, mc::CompoundBlend::Average { .. })
}

fn compound_refinemv_reference_allowed(
    facts: &FrameRefFacts,
    compound: super::super::compound::CompoundBlockSyntax,
) -> Result<bool> {
    let Some((ref0, ref1)) = facts.sized_pair(compound.ref_frame0, compound.ref_frame1)? else {
        return Ok(false);
    };
    let d0 = facts.current.relative_dist(ref0);
    let d1 = facts.current.relative_dist(ref1);
    Ok(d0 != 0 && d0 == -d1)
}

#[inline]
pub(super) fn compound_ref_contexts(
    neighbour_ctx: &BlockNeighbourContext,
    num_total_refs: usize,
) -> Result<[usize; 7]> {
    neighbour_ctx
        .single_ref_ctxs(num_total_refs)
        .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterReferenceMap.into())
}

pub(super) fn compound_group_idx_context(
    neighbour_ctx: &BlockNeighbourContext,
    facts: &FrameRefFacts,
    ref_frame0: i8,
    ref_frame1: i8,
) -> Result<usize> {
    let (ref0_order_hint, ref1_order_hint) = (facts.hint(ref_frame0)?, facts.hint(ref_frame1)?);
    let equal_ref_distance = facts.current.relative_dist(ref0_order_hint).abs()
        == ref1_order_hint.relative_dist(facts.current).abs();
    let furthest_future_ref = facts.furthest_future_ref()?;
    Ok(neighbour_ctx.compound_group_idx_ctx(equal_ref_distance, furthest_future_ref))
}

#[inline]
pub(super) fn compound_joint_mv_projection(
    facts: &FrameRefFacts,
    ref_frame0: i8,
    ref_frame1: i8,
) -> Result<CompoundJointMvProjection> {
    Ok(compound_joint_mv_projection_from_order_hints(
        facts.current,
        facts.hint(ref_frame0)?,
        facts.hint(ref_frame1)?,
    ))
}

fn compound_joint_mv_projection_from_order_hints(
    current: CompoundOrderHint,
    ref0_order_hint: CompoundOrderHint,
    ref1_order_hint: CompoundOrderHint,
) -> CompoundJointMvProjection {
    let rel0 = ref0_order_hint.relative_dist(current);
    let rel1 = ref1_order_hint.relative_dist(current);
    let mut first_dist = rel0.abs();
    let mut second_dist = rel1.abs();
    let base_list = usize::from(
        first_dist < second_dist
            || (!ref0_order_hint.is_restricted() && ref1_order_hint.is_restricted()),
    );
    if base_list == 1 {
        core::mem::swap(&mut first_dist, &mut second_dist);
    }
    let same_side = compound_references_same_side(current, ref0_order_hint, ref1_order_hint);
    if !same_side {
        second_dist = -second_dist;
    }
    CompoundJointMvProjection {
        base_list,
        first_dist,
        second_dist,
    }
}

pub(super) fn project_joint_mvd(diff: Mv, num: i32, den: i32) -> Mv {
    let num = num.clamp(-31, 31);
    let den = den.clamp(1, 31);
    let frac = i64::from(num) * i64::from(MV_PROJECTION_DIV_MULT[den as usize]);
    Mv {
        row: project_joint_mvd_component(diff.row, frac),
        col: project_joint_mvd_component(diff.col, frac),
    }
}

fn project_joint_mvd_component(component: i32, frac: i64) -> i32 {
    let product = i64::from(component) * frac;
    let rounded = (product + 8192 + (product >> 63)) >> 14;
    rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(super) fn scale_joint_projected_mvd(
    mut projected: Mv,
    jmvd_scale_mode: u8,
    use_amvd: bool,
) -> Mv {
    if use_amvd {
        match jmvd_scale_mode {
            1 => {
                projected.row = projected.row.saturating_mul(2);
                projected.col = projected.col.saturating_mul(2);
            }
            2 => {
                projected.row /= 2;
                projected.col /= 2;
            }
            _ => {}
        }
        return projected;
    }
    match jmvd_scale_mode {
        1 => projected.row = projected.row.saturating_mul(2),
        2 => projected.col = projected.col.saturating_mul(2),
        3 => projected.row /= 2,
        4 => projected.col /= 2,
        _ => {}
    }
    projected
}

pub(super) fn add_mv_clamped(pred: Mv, diff: Mv) -> Mv {
    Mv {
        row: mv_clamp_to_integer(pred.row.saturating_add(diff.row)),
        col: mv_clamp_to_integer(pred.col.saturating_add(diff.col)),
    }
}

fn read_compound_use_amvd_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    enable_adaptive_mvd: bool,
    y_mode: CompoundYMode,
    use_optflow: bool,
    ctx: usize,
    tile_offset: ByteOffset,
) -> Result<bool> {
    let Some(selector) = compound_use_amvd_selector(enable_adaptive_mvd, y_mode, use_optflow, ctx)
    else {
        return Ok(false);
    };
    let use_amvd = cdfs
        .read_block_symbol_trace(selector, symbols)
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    Ok(use_amvd.get() != 0)
}

/// The `use_amvd` selector of a compound block coding `y_mode` (§ 5.20.7.6);
/// `None` where the decoder reads no `use_amvd`.
pub(super) fn compound_use_amvd_selector(
    enable_adaptive_mvd: bool,
    y_mode: CompoundYMode,
    use_optflow: bool,
    ctx: usize,
) -> Option<TileCdfSelector> {
    y_mode
        .use_amvd_index(use_optflow)
        .filter(|_| enable_adaptive_mvd)
        .map(|index| TileCdfSelector::UseAmvd { index, ctx })
}

fn read_compound_jmvd_scale_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    y_mode: CompoundYMode,
    use_amvd: bool,
    tile_offset: ByteOffset,
) -> Result<u8> {
    if y_mode != CompoundYMode::JointNew {
        return Ok(0);
    }
    let selector = if use_amvd {
        TileCdfSelector::JmvdAdaptiveScaleMode
    } else {
        TileCdfSelector::JmvdScaleMode
    };
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(splot_core::symbol::Symbol::get)
        .map_err(|error| symbol_read_error(error, tile_offset))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundBlendInput {
    pub(super) use_optflow: bool,
    pub(super) joint_amvd: bool,
    pub(super) switchable_refinemv_on: bool,
    pub(super) n4w: usize,
    pub(super) n4h: usize,
    pub(super) block_size_index: usize,
    pub(super) comp_group_idx_ctx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundBlendToolConfig {
    pub(super) masked_enabled: bool,
    pub(super) implicit_mask: bool,
}

impl CompoundBlendToolConfig {
    pub(super) fn from_sequence(sequence: &SequenceHeader) -> Self {
        Self {
            masked_enabled: sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_masked_compound),
            implicit_mask: sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_imp_msk_bld),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundCwpInput {
    pub(super) y_mode: CompoundYMode,
    pub(super) jmvd_scale_mode: u8,
    pub(super) use_optflow: bool,
    pub(super) use_refinemv: bool,
    pub(super) motion_simple: bool,
    pub(super) ref_frame0: i8,
    pub(super) ref_frame1: i8,
    pub(super) blend: mc::CompoundBlend,
}

#[derive(Clone, Copy)]
struct CompoundCwpContext<'a> {
    sequence: &'a SequenceHeader,
    facts: &'a FrameRefFacts,
}

fn read_compound_blend_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tools: CompoundBlendToolConfig,
    input: CompoundBlendInput,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlend> {
    let average_blend = mc::CompoundBlend::average_with_implicit_mask(tools.implicit_mask);
    if !compound_reads_group_idx(tools, input) {
        return Ok(average_blend);
    }
    let comp_group_idx = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::CompGroupIdx {
                ctx: input.comp_group_idx_ctx,
            },
            symbols,
        )
        .map_err(|error| symbol_read_error(error, tile_offset))?
        .get();
    if comp_group_idx == 0 {
        return Ok(average_blend);
    }
    let compound_type = if wedge_bits(input.block_size_index) == 0 {
        MaskedCompoundType::DiffWeighted
    } else {
        match cdfs
            .read_block_symbol_trace(TileCdfSelector::CompoundType, symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?
            .get()
        {
            0 => MaskedCompoundType::Wedge,
            _ => MaskedCompoundType::DiffWeighted,
        }
    };
    match compound_type {
        MaskedCompoundType::DiffWeighted => {
            let mask_type = symbols.read_literal(1).map_err(|error| {
                symbol_read_error(BlockSymbolTraceReadError::Symbol(error), tile_offset)
            })? != 0;
            Ok(mc::CompoundBlend::DiffWeighted { inverse: mask_type })
        }
        MaskedCompoundType::Wedge => {
            let index = read_wedge_mode_syntax(cdfs, symbols, tile_offset)?;
            let sign = symbols.read_bool().map_err(|error| {
                symbol_read_error(BlockSymbolTraceReadError::Symbol(error), tile_offset)
            })?;
            Ok(mc::CompoundBlend::Wedge { index, sign })
        }
    }
}

/// Whether `read_compound_type` (§ 5.20.7.16) reads `comp_group_idx`.
pub(super) const fn compound_reads_group_idx(
    tools: CompoundBlendToolConfig,
    input: CompoundBlendInput,
) -> bool {
    !(input.use_optflow
        || input.joint_amvd
        || input.switchable_refinemv_on
        || !tools.masked_enabled
        || compound_blend_is_thin(input.n4w, input.n4h))
}

const fn compound_blend_is_thin(n4w: usize, n4h: usize) -> bool {
    (n4w == 1 && n4h >= 4) || (n4h == 1 && n4w >= 4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaskedCompoundType {
    Wedge,
    DiffWeighted,
}

pub(super) fn wedge_bits(block_size_index: usize) -> u8 {
    const WEDGE_BITS: [u8; 29] = [
        0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 4, 4, 0, 0, 4, 4,
    ];
    WEDGE_BITS.get(block_size_index).copied().unwrap_or(0)
}

pub(super) const CWP_WEIGHTING_FACTOR: [[i16; 5]; 2] = [[8, 12, 4, 10, 6], [8, 12, 4, 20, -4]];

fn read_compound_cwp_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    context: CompoundCwpContext<'_>,
    input: CompoundCwpInput,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlend> {
    let cwp_enabled = context
        .sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_cwp);
    if !compound_cwp_signal_allowed(cwp_enabled, input) {
        return Ok(input.blend);
    }
    let mut coding_idx = 0usize;
    for idx in 0..CWP_WEIGHTING_FACTOR[0].len() - 1 {
        let symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::CwpIdx { idx }, symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?
            .get();
        coding_idx = idx + usize::from(symbol != 0);
        if symbol == 0 {
            break;
        }
    }
    let same_side = compound_cwp_same_side(context.facts, input.ref_frame0, input.ref_frame1)?;
    Ok(input
        .blend
        .average_with_cwp_weight(CWP_WEIGHTING_FACTOR[usize::from(same_side)][coding_idx]))
}

pub(super) fn compound_cwp_signal_allowed(cwp_enabled: bool, input: CompoundCwpInput) -> bool {
    cwp_enabled
        && !input.use_optflow
        && !input.use_refinemv
        && input.motion_simple
        && compound_cwp_mode_allowed(input.y_mode, input.jmvd_scale_mode)
        && matches!(input.blend, mc::CompoundBlend::Average { .. })
}

const fn compound_cwp_mode_allowed(y_mode: CompoundYMode, jmvd_scale_mode: u8) -> bool {
    matches!(y_mode, CompoundYMode::NearNear)
        || matches!(y_mode, CompoundYMode::JointNew) && jmvd_scale_mode == 0
}

pub(super) fn compound_cwp_same_side(
    facts: &FrameRefFacts,
    ref_frame0: i8,
    ref_frame1: i8,
) -> Result<bool> {
    Ok(compound_references_same_side(
        facts.current,
        facts.hint(ref_frame0)?,
        facts.hint(ref_frame1)?,
    ))
}

fn compound_references_same_side(
    current: CompoundOrderHint,
    ref0_order_hint: CompoundOrderHint,
    ref1_order_hint: CompoundOrderHint,
) -> bool {
    let d0 = ref0_order_hint.frame_distance_from(current);
    let d1 = ref1_order_hint.frame_distance_from(current);
    (d0 < 0 && d1 < 0) || (d0 > 0 && d1 > 0)
}

#[cfg(test)]
mod tests;
