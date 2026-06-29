// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! First inter-frame decode frontier for the shared minimal-tier runtime.
//!
//! Decodes the verified minimal inter subset: an `OBU_REGULAR_TILE_GROUP` frame
//! with one reference, `skip_mode == 0`, `is_inter == 1`, the single-reference
//! NEARMV / GLOBALMV / NEWMV modes, `skip ∈ {0, 1}`, and § 7.13.3.17 / § 7.13.3.18
//! block inter prediction (zero-fraction reduces to a straight copy; sub-pel runs
//! the interpolation-filter convolution). The frame may now be a MULTI-SUPERBLOCK
//! single superblock ROW (height 64, width a positive multiple of 64) OR single
//! superblock COLUMN (width 64, height a positive multiple of 64) of 64x64
//! superblocks: the § 5.20.2.1 raster loop (in the shared partition walker)
//! decodes each superblock in turn, and a later superblock's block predicts its
//! motion vector from the immediately-prior superblock's reconstructed-edge
//! neighbour via the frame-wide § 7.11 / § 7.12 `find_mv_stack` grid. Everything
//! outside that subset is rejected with a structured `decode/unsupported-feature`
//! diagnostic BEFORE any wrong output, so splot never emits a
//! confident-but-unverified inter frame.
//!
//! Feature tracking: `DECODE-FIRST-INTER-FRAME-FRONTIER`,
//! `DECODE-INTER-MODE-INFO`, `DECODE-INTER-ZERO-MV`,
//! `DECODE-INTER-MOTION-COMPENSATION`, `DECODE-INTER-MULTI-SB-SPATIAL`.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CoreSeqQuantView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, QuantizationParams, TipFrameMode, TxMode,
    parse_frame_header_core,
};
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    BitDepth, DecodedFrame, InterpolationFilter as ReconInterpolationFilter,
    PlaneId as ReconPlaneId, ReferenceFrameStore, ReferenceSlot,
};

use super::{
    DecodeOptions, DecodePlannedObu, DecodeStreamPlan, IvfHeader, MinimalRuntimeDecodedFrame,
    MinimalRuntimeFrame, Result, ensure_runtime_limits,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature};

/// Feature id for the inter decode frontier diagnostics.
const FEATURE_ID: &str = "DECODE-FIRST-INTER-FRAME-FRONTIER";
const MATRIX_ROW: &str = "first-inter-frame-frontier";
const TIER_ID: &str = "general-inter-8bit420-frontier-v1";
const REMEDIATION: &str = "Inter decode is limited to the verified single-reference zero-MV skip subset; track DECODE-FIRST-INTER-FRAME-FRONTIER.";
const COMPOUND_FEATURE_ID: &str = "DECODE-INTER-COMPOUND-AVERAGE";
const COMPOUND_MATRIX_ROW: &str = "inter-compound-average";
const COMPOUND_REMEDIATION: &str = "Compound inter decode is limited to the verified two-reference COMPOUND_AVERAGE/CWP_EQUAL skipped 64x64 fixture; track DECODE-INTER-COMPOUND-AVERAGE.";

const SPEC_HEADER: &str = "5.18.2";
const SPEC_MODE_INFO: &str = "5.20.7.6";
const SPEC_MV: &str = "7.11";
const SPEC_MC: &str = "7.13.3.18";
const SPEC_REFERENCE: &str = "7.23";

/// AV2 § 5.20.7.6 `single_mode == 0` -> `YMode = NEARMV` (the zero-MV mode over an
/// empty no-neighbour § 7.10 MV stack this frontier reconstructs).
const SINGLE_MODE_NEARMV: u8 = 0;
/// AV2 § 5.20.7.6 `single_mode == 1` -> `YMode = GLOBALMV` (the zero-MV mode over
/// identity global motion).
const SINGLE_MODE_GLOBALMV: u8 = 1;
/// AV2 § 5.20.7.6 `single_mode == 2` -> `YMode = NEWMV` (reads a § 5.20.7.20
/// SHELL-coded MV delta over the no-neighbour zero predictor).
const SINGLE_MODE_NEWMV: u8 = 2;

/// A modelled motion vector in eighth-pel units (AV2 § 7.11). The verified subset
/// only produces the zero vector; the newtype keeps the MV result explicit at the
/// motion-compensation boundary rather than a bare integer pair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Mv {
    /// Row (vertical) component in eighth-pel units.
    pub(super) row: i32,
    /// Column (horizontal) component in eighth-pel units.
    pub(super) col: i32,
}

impl Mv {
    /// The zero motion vector (`GLOBALMV` over identity global motion / the
    /// no-neighbour zero predictor, AV2 § 7.11).
    const ZERO: Self = Self { row: 0, col: 0 };
}

/// Decodes the minimal inter frame, given the post-key reference store, and returns
/// its displayed frame. The reference for slot `ref_frame_idx[0]` must be present.
///
/// The caller provides the already-validated § 5.18.2 inter frame header core
/// (parsed to [`FrameHeaderParseStatus::InterHeaderComplete`]) so the runtime can
/// charge decoded-frame byte limits before reconstruction without parsing the
/// same header twice. This function walks the § 5.20.3 partition tree, reads the
/// § 5.20.7.6 inter `mode_info` (`is_inter` / `skip` / `single_mode`) from the
/// tile arithmetic stream, derives the § 7.11 zero motion vector, runs the
/// § 7.13.3.18 zero-fraction motion-compensation copy, and validates § 8.2.4
/// `exit_symbol()`. No step is hardcoded: a wrong symbol read fails `exit_symbol()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_minimal_inter_frame(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: DecodeOptions,
    header: IvfHeader,
    reference: &InterReferenceState<'_>,
) -> Result<(MinimalRuntimeFrame, FrameHeaderCore)> {
    let offset = frame_envelope.offset;

    if frame_envelope.header.obu_type != ObuType::RegularTileGroup {
        return Err(unsupported_at(
            "inter_unexpected_obu_type",
            offset,
            "minimal inter decode requires an OBU_REGULAR_TILE_GROUP inter frame",
            SPEC_HEADER,
        ));
    }

    let current_base_q_idx = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let current_order_hint = i32::try_from(core.order_hint_lsb.unwrap_or(0)).unwrap_or(i32::MAX);
    if let Some(inter_ctrl) = core.inter.as_ref() {
        let (enable_avg_cdf, avg_cdf_type) = sequence
            .transform_quant_entropy
            .as_ref()
            .map_or((false, 1u8), |tq| (tq.enable_avg_cdf, tq.avg_cdf_type));
        let load = resolve_cdf_load(
            inter_ctrl.signal_primary_ref_frame,
            inter_ctrl.primary_ref_frame,
            inter_ctrl.disable_cross_frame_cdf_init,
            &inter_ctrl.ref_frame_idx,
            &reference.ref_is_inter,
            &reference.ref_base_q_idx,
            &reference.ref_order_hint,
            &reference.ref_frame_width,
            &reference.ref_frame_height,
            current_base_q_idx,
            current_order_hint,
            enable_avg_cdf,
            avg_cdf_type,
        );
        if let ResolvedCdfLoad::OutOfRangePrimary = load {
            return Err(unsupported_at(
                "inter_primary_ref_out_of_range",
                offset,
                "minimal multi-reference decode rejects a non-conformant signalled primary_ref_frame that names a reference >= NumTotalRefs (out of ref_frame_idx bounds, §6.17.2) before any output",
                SPEC_HEADER,
            ));
        }
        if let ResolvedCdfLoad::LoadSlot { primary, blend } = load {
            if reference.ref_adapted.get(primary as usize).copied() == Some(true) {
                return Err(unsupported_at(
                    "inter_cdf_inheritance_unmodeled",
                    offset,
                    "minimal multi-reference decode does not model §7.23 cross-frame CDF save/load; an inter frame whose §5-resolved primary_ref_frame loads a prior ADAPTED reference slot's CDFs is rejected before any output",
                    SPEC_HEADER,
                ));
            }
            if let Some(blend_slot) = blend
                && reference.ref_adapted.get(blend_slot as usize).copied() == Some(true)
            {
                return Err(unsupported_at(
                    "inter_blend_cdf_unmodeled",
                    offset,
                    "minimal multi-reference decode does not model the §5 blend_cdfs secondary CDF load; an inter frame that blends a prior ADAPTED reference slot's CDFs (enable_avg_cdf && avg_cdf_type == 0) is rejected before any output",
                    SPEC_HEADER,
                ));
            }
        }
    }

    let order_hint_bits = sequence
        .inter
        .as_ref()
        .map_or(0, |seq_inter| u32::from(seq_inter.order_hint_bits));
    let this_order_hint = core.order_hint_lsb.unwrap_or(0);
    if !order_hint_history_unwrapped(
        &reference.ref_valid,
        &reference.ref_order_hint,
        order_hint_bits,
        this_order_hint,
    ) {
        return Err(unsupported_at(
            "inter_order_hint_wrapped",
            offset,
            "minimal multi-reference decode stores RefOrderHint as OrderHintLsbs (the unwrapped OrderHint only while the GOP fits in one OrderHintBits window); an order-hint-wrapped reference history would mis-order the §7.7 ranking, so it is rejected before any output",
            SPEC_REFERENCE,
        ));
    }

    let uses_temporal_mvs = core
        .inter
        .as_ref()
        .and_then(|inter| inter.use_ref_frame_mvs)
        == Some(true);
    let has_retained_inter_reference = reference.ref_is_inter.iter().any(|&is_inter| is_inter);
    if uses_temporal_mvs && has_retained_inter_reference {
        return Err(unsupported_at(
            "inter_temporal_mvs_unmodeled",
            offset,
            "minimal multi-reference decode does not model §7.23 SavedMvs; an inter frame using temporal MVs (enable_ref_frame_mvs / use_ref_frame_mvs) after an inter reference has been retained is rejected before any output",
            SPEC_MV,
        ));
    }

    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "inter_missing_frame_size",
            offset,
            "minimal inter decode requires a parsed frame size",
            SPEC_HEADER,
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    let inter = core.inter.as_ref().ok_or_else(|| {
        unsupported_at(
            "inter_missing_control_region",
            offset,
            "minimal inter decode requires a parsed inter control region",
            SPEC_HEADER,
        )
    })?;
    let tail = core.inter_tail.as_ref().ok_or_else(|| {
        unsupported_at(
            "inter_missing_tail",
            offset,
            "minimal inter decode requires the parsed §5.18.2 inter tail",
            SPEC_HEADER,
        )
    })?;
    let num_total_refs = inter.num_total_refs.unwrap_or(0);
    if tail.reference_select {
        if num_total_refs != 2 {
            return Err(unsupported_compound_at(
                "compound_unsupported_num_total_refs",
                offset,
                "minimal compound-average decode requires NumTotalRefs == 2 so read_compound_ref selects implicit RefFrame[0,1] without comp_ref symbols",
                SPEC_MODE_INFO,
            ));
        }
    } else if num_total_refs != 1 && num_total_refs != 2 {
        return Err(unsupported_at(
            "inter_unsupported_num_total_refs",
            offset,
            "minimal inter decode supports single-reference prediction with NumTotalRefs ∈ {1, 2}; NumTotalRefs > 2 would read additional single_ref symbols this subset does not model",
            SPEC_MODE_INFO,
        ));
    }
    let ref_frame_idx = inter.ref_frame_idx.clone();
    if ref_frame_idx.len() != num_total_refs as usize || ref_frame_idx.is_empty() {
        return Err(unsupported_at(
            "inter_missing_ref_frame_idx",
            offset,
            "minimal inter decode requires a derived ref_frame_idx[] of length NumTotalRefs",
            SPEC_HEADER,
        ));
    }

    let compound_is_joint_ctx = if tail.reference_select {
        validate_compound_sequence_subset(sequence, &core, offset)?;
        Some(compound_is_joint_context(
            &ref_frame_idx,
            reference,
            current_order_hint,
            offset,
        )?)
    } else {
        None
    };
    if tail.use_global_motion {
        return Err(unsupported_at(
            "inter_use_global_motion",
            offset,
            "minimal inter decode only supports identity global motion (use_global_motion == 0), so GLOBALMV is the zero vector",
            SPEC_MV,
        ));
    }

    // Every reference the §7.7 map names must be present in the §7.23 store and unscaled
    // (reference size == current frame size): a block's single_ref read can select ANY of
    // the NumTotalRefs slots, so validate them all up front before any output.
    for (r, &slot) in ref_frame_idx.iter().enumerate() {
        let ref_frame = reference.frame_for_slot(slot).ok_or_else(|| {
            unsupported_at(
                "inter_missing_reference_frame",
                offset,
                "minimal inter decode requires every §7.7-mapped reference to be present in the §7.23 store",
                SPEC_REFERENCE,
            )
        })?;
        let ref_luma = ref_frame.y();
        if ref_luma.visible_size().width() != frame_width as usize
            || ref_luma.visible_size().height() != frame_height as usize
        {
            let _ = r;
            return Err(unsupported_at(
                "inter_reference_resolution_mismatch",
                offset,
                "minimal inter decode only supports unscaled references (every §7.7-mapped reference size must equal the current frame size)",
                SPEC_MC,
            ));
        }
    }

    let limits = options.limits();
    let tile_size = {
        let mut tile_plan = super::derive_inter_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            &core,
            options,
        )?;
        let [tile] = tile_plan.work_units_mut() else {
            return Err(unsupported_at(
                "inter_unexpected_tile_work_units",
                offset,
                "minimal inter decode requires exactly one tile work unit",
                SPEC_HEADER,
            ));
        };
        tile.tile_size()
    };
    ensure_runtime_limits(
        limits,
        frame_width,
        frame_height,
        tile_size,
        BitDepth::Eight,
    )?;

    let interpolation_filter = inter.interpolation_filter.ok_or_else(|| {
        unsupported_at(
            "inter_missing_interpolation_filter",
            offset,
            "minimal inter decode requires a parsed §5.18.5.1 interpolation_filter",
            SPEC_MC,
        )
    })?;

    let blocks = decode_inter_blocks(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        &core,
        options,
        interpolation_filter,
        num_total_refs as usize,
        tail.reference_select,
        compound_is_joint_ctx,
        sequence
            .inter
            .as_ref()
            .map_or(0, |seq_inter| seq_inter.num_same_ref_compound),
    )?;

    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace::<u8>(
        frame_width as usize,
        frame_height as usize,
        splot_recon::BitDepth::Eight,
    )?;
    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            unsupported_at(
                "inter_missing_base_q",
                offset,
                "minimal inter residual decode requires a parsed base_q_idx",
                SPEC_HEADER,
            )
        })?;
    let luma_use_tcq = core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.allow_tcq);

    for placed in &blocks {
        let rect = mc::McBlockRect {
            luma_x: placed.luma_x,
            luma_y: placed.luma_y,
            luma_w: placed.luma_w,
            luma_h: placed.luma_h,
        };
        let ref_frame0 = resolve_block_reference_frame(
            &ref_frame_idx,
            reference,
            placed.block.ref_frame0,
            offset,
        )?;
        if let Some(ref_frame1) = placed.block.ref_frame1 {
            let ref_frame1 =
                resolve_block_reference_frame(&ref_frame_idx, reference, ref_frame1, offset)?;
            mc::motion_compensate_compound_average_block_into(
                &mut workspace,
                mc::CompoundMcBlock {
                    reference0: ref_frame0,
                    reference1: ref_frame1,
                    rect,
                    mv0: placed.block.mv,
                    mv1: placed.block.mv1,
                    interp: placed.block.interp,
                },
                offset,
            )?;
        } else {
            mc::motion_compensate_block_into(
                &mut workspace,
                ref_frame0,
                rect,
                placed.block.mv,
                placed.block.interp,
                offset,
            )?;
        }
        if let Some(residual) = placed.block.residual.as_ref() {
            add_inter_residual_to_workspace(
                &mut workspace,
                residual,
                placed.luma_x,
                placed.luma_y,
                qindex,
                luma_use_tcq,
                offset,
            )?;
        }
    }

    let frame = workspace.freeze()?;

    Ok((
        MinimalRuntimeFrame {
            frame: MinimalRuntimeDecodedFrame::Eight(frame),
            frame_rate_numerator: header.timebase_denominator,
            frame_rate_denominator: header.timebase_numerator,
        },
        core,
    ))
}

fn resolve_block_reference_frame<'a>(
    ref_frame_idx: &[u32],
    reference: &'a InterReferenceState<'a>,
    ref_frame: i8,
    offset: ByteOffset,
) -> Result<&'a DecodedFrame<u8>> {
    let ref_slot = ref_frame_idx
        .get(ref_frame as usize)
        .copied()
        .ok_or_else(|| {
            unsupported_at(
                "inter_block_ref_frame_out_of_range",
                offset,
                "a decoded block's RefFrame indexed past the §7.7 ref_frame_idx map",
                SPEC_MODE_INFO,
            )
        })?;
    reference.frame_for_slot(ref_slot).ok_or_else(|| {
        unsupported_at(
            "inter_missing_reference_frame",
            offset,
            "the block's selected §7.23 reference is not present in the store",
            SPEC_REFERENCE,
        )
    })
}

fn validate_compound_sequence_subset(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    let Some(seq_inter) = sequence.inter.as_ref() else {
        return Err(unsupported_compound_at(
            "compound_missing_sequence_inter",
            offset,
            "minimal compound-average decode requires the parsed inter sequence tools",
            SPEC_MODE_INFO,
        ));
    };
    if seq_inter.enable_masked_compound {
        return Err(unsupported_compound_at(
            "compound_masked_compound_enabled",
            offset,
            "minimal compound-average decode requires enable_masked_compound == 0 so §5.20.7.16 cannot signal wedge or difference-weighted compound masks",
            SPEC_MODE_INFO,
        ));
    }
    if seq_inter.enable_cwp {
        return Err(unsupported_compound_at(
            "compound_cwp_enabled",
            offset,
            "minimal compound-average decode requires enable_cwp == 0; compound weighted prediction is deferred to a separate fixture-proven brick",
            SPEC_MODE_INFO,
        ));
    }
    if seq_inter.enable_imp_msk_bld {
        return Err(unsupported_compound_at(
            "compound_implicit_mask_enabled",
            offset,
            "minimal compound-average decode requires enable_imp_msk_bld == 0 so §7.13.3.16 cannot enter implicit masked blending",
            SPEC_MC,
        ));
    }
    if seq_inter.enable_opfl_refine != 0 {
        return Err(unsupported_compound_at(
            "compound_opfl_refine_enabled",
            offset,
            "minimal compound-average decode requires enable_opfl_refine == REFINE_NONE; optical-flow refinement is not modelled",
            SPEC_MODE_INFO,
        ));
    }
    if seq_inter.enable_refinemv {
        return Err(unsupported_compound_at(
            "compound_refinemv_enabled",
            offset,
            "minimal compound-average decode requires enable_refinemv == 0 so §5.20.7.17 reads no refine-MV syntax and cannot adjust compound MVs",
            SPEC_MODE_INFO,
        ));
    }
    if seq_inter.enable_tip {
        return Err(unsupported_compound_at(
            "compound_tip_enabled",
            offset,
            "minimal compound-average decode requires enable_tip == 0 so read_ref_frames does not enter TIP syntax before comp_mode",
            SPEC_MODE_INFO,
        ));
    }
    let tip_frame_mode = core.inter.as_ref().and_then(|inter| inter.tip_frame_mode);
    if tip_frame_mode != Some(TipFrameMode::Disabled) {
        return Err(unsupported_compound_at(
            "compound_active_tip_frame_mode",
            offset,
            "minimal compound-average decode requires no active TIP frame mode so read_ref_frames does not read a tip_mode symbol before comp_mode",
            SPEC_MODE_INFO,
        ));
    }
    Ok(())
}

fn compound_is_joint_context(
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_>,
    current_order_hint: i32,
    offset: ByteOffset,
) -> Result<usize> {
    if ref_frame_idx.len() != 2 {
        return Err(unsupported_compound_at(
            "compound_missing_ref_frame_idx",
            offset,
            "minimal compound-average decode requires exactly two §7.7 reference-map entries",
            SPEC_MODE_INFO,
        ));
    }
    let ref_order_hint = |ref_idx: usize| -> Result<i32> {
        let slot = *ref_frame_idx.get(ref_idx).ok_or_else(|| {
            unsupported_compound_at(
                "compound_ref_frame_idx_out_of_range",
                offset,
                "minimal compound-average decode could not resolve a compound RefFrame through ref_frame_idx[]",
                SPEC_MODE_INFO,
            )
        })?;
        reference
            .ref_order_hint
            .get(slot as usize)
            .copied()
            .map(|hint| i32::try_from(hint).unwrap_or(i32::MAX))
            .ok_or_else(|| {
                unsupported_compound_at(
                    "compound_reference_order_hint",
                    offset,
                    "minimal compound-average decode requires RefOrderHint for both compound references",
                    SPEC_REFERENCE,
                )
            })
    };
    let first_order_hint = ref_order_hint(0)?;
    let second_order_hint = ref_order_hint(1)?;
    Ok(compound_is_joint_context_from_order_hints(
        first_order_hint,
        second_order_hint,
        current_order_hint,
    ))
}

fn compound_is_joint_context_from_order_hints(
    first_order_hint: i32,
    second_order_hint: i32,
    current_order_hint: i32,
) -> usize {
    let first_dist = get_relative_dist(first_order_hint, current_order_hint).abs();
    let second_dist = get_relative_dist(second_order_hint, current_order_hint).abs();
    let first_side = get_relative_dist(first_order_hint, current_order_hint);
    let second_side = get_relative_dist(second_order_hint, current_order_hint);
    let same_side = (first_side < 0 && second_side < 0) || (first_side > 0 && second_side > 0);
    usize::from(same_side || first_dist != second_dist)
}

fn get_relative_dist(a: i32, b: i32) -> i32 {
    (a - b).clamp(-127, 127)
}

/// AV2 § 9.2 transform log2 sides for the verified single 64x64 inter block: the
/// luma TX_64X64 (`log2 == 6`) and the 4:2:0 chroma TX_32X32 (`log2 == 5`).
const INTER_LUMA_LOG2_SIDE: u32 = 6;
const INTER_CHROMA_LOG2_SIDE: u32 = 5;

/// Adds the decoded § 5.20.7.27 inter residual to the § 7.13.3.18 MC prediction
/// already in `workspace`, per plane (Y/U/V), at the block's luma-space position
/// `(luma_x, luma_y)`. The luma DCT_DCT block uses the § 7.14.4 TCQ `dqDenom`
/// term when `luma_use_tcq` (the frame's `allow_tcq`); chroma never does. An
/// `all_zero` plane is a no-op (the MC prediction stands).
///
/// The residual is read for the verified single 64x64 block only (TX_64X64 luma
/// / TX_32X32 chroma), so the only `skip == 0` block reaches here at the frame
/// origin. A multi-block `skip == 0` residual (per-block TX sizes) is a future
/// brick; the block decode rejects it before this point.
#[allow(clippy::too_many_arguments)]
fn add_inter_residual_to_workspace(
    workspace: &mut splot_recon::CurrentFrameWorkspace<u8>,
    residual: &InterResidual,
    luma_x: usize,
    luma_y: usize,
    qindex: u32,
    luma_use_tcq: bool,
    offset: ByteOffset,
) -> Result<()> {
    let map_recon = |error| {
        let _ = error;
        unsupported_at(
            "inter_residual_reconstruct",
            offset,
            "minimal inter residual reconstruction failed",
            SPEC_MC,
        )
    };
    let chroma_x = luma_x >> 1;
    let chroma_y = luma_y >> 1;
    crate::runtime_minimal_recon::reconstruct_inter_block_residual_into(
        workspace,
        &residual.luma,
        ReconPlaneId::Y,
        luma_x,
        luma_y,
        INTER_LUMA_LOG2_SIDE,
        qindex,
        luma_use_tcq,
        splot_recon::BitDepth::Eight,
    )
    .map_err(map_recon)?;
    crate::runtime_minimal_recon::reconstruct_inter_block_residual_into(
        workspace,
        &residual.u,
        ReconPlaneId::U,
        chroma_x,
        chroma_y,
        INTER_CHROMA_LOG2_SIDE,
        qindex,
        false,
        splot_recon::BitDepth::Eight,
    )
    .map_err(map_recon)?;
    crate::runtime_minimal_recon::reconstruct_inter_block_residual_into(
        workspace,
        &residual.v,
        ReconPlaneId::V,
        chroma_x,
        chroma_y,
        INTER_CHROMA_LOG2_SIDE,
        qindex,
        false,
        splot_recon::BitDepth::Eight,
    )
    .map_err(map_recon)?;
    Ok(())
}

/// The decoded inter block: its reference index or indices, §7.11 motion vector
/// or vectors, the §5.20.7.6 block interpolation filter (mapped to the recon-side
/// §7.13.3.18 selector), and — for a `skip == 0` block — the §5.20.7.27
/// per-plane coded residual.
#[derive(Clone, Debug)]
pub(super) struct InterBlock {
    /// §5.20.7.12 `RefFrame[0]`: the block's reference index into the §7.7
    /// `ref_frame_idx[]` map (0 for the NumTotalRefs == 1 single reference, or the
    /// decoded `single_ref` selection 0/1 for NumTotalRefs == 2). The caller resolves
    /// the §7.23 store slot as `ref_frame_idx[ref_frame0]`.
    pub(super) ref_frame0: i8,
    /// §5.20.7.11 `RefFrame[1]` for the compound subset; `None` for single
    /// prediction.
    pub(super) ref_frame1: Option<i8>,
    /// The decoded list-0 §7.11 motion vector (eighth-pel units).
    pub(super) mv: Mv,
    /// The decoded list-1 §7.11 motion vector (eighth-pel units), used only when
    /// `ref_frame1` is present.
    pub(super) mv1: Mv,
    /// The §7.13.3.18 interpolation filter the block uses for motion compensation.
    pub(super) interp: ReconInterpolationFilter,
    /// The decoded §5.20.7.27 residual for the Y/U/V planes when `skip == 0`;
    /// `None` for a `skip == 1` block (no residual is read or added). Each entry
    /// is the plane's single transform-block coefficient decode (luma first).
    pub(super) residual: Option<InterResidual>,
}

/// A decoded inter block placed in the frame: its luma-space rectangle plus the
/// decoded [`InterBlock`]. A multi-block partition returns one per §5.20.3 leaf
/// block in decode (DFS) order; each is motion-compensated (and residual-added)
/// independently into the shared frame workspace.
#[derive(Clone, Debug)]
pub(super) struct PlacedInterBlock {
    /// Luma-space top-left x (samples).
    pub(super) luma_x: usize,
    /// Luma-space top-left y (samples).
    pub(super) luma_y: usize,
    /// Block width in luma samples.
    pub(super) luma_w: usize,
    /// Block height in luma samples.
    pub(super) luma_h: usize,
    /// The decoded block (MV, interp filter, optional residual).
    pub(super) block: InterBlock,
}

/// The decoded §5.20.7.27 inter residual for one 64x64 block: the per-plane
/// Y/U/V transform-block coefficient decodes, in plane (decode) order.
#[derive(Clone, Debug)]
pub(super) struct InterResidual {
    /// Luma (Y) transform-block coefficients.
    pub(super) luma: crate::tile_payload::LumaCoeffBlock,
    /// Chroma U transform-block coefficients.
    pub(super) u: crate::tile_payload::LumaCoeffBlock,
    /// Chroma V transform-block coefficients.
    pub(super) v: crate::tile_payload::LumaCoeffBlock,
}

/// The post-key reference state the inter decode consumes: the §7.23
/// [`ReferenceFrameStore`] of borrowed decoded reference frames plus the modelled
/// §7.7/§7.23 reference metadata (`RefValid` / `RefOrderHint` / dims) the §5.18.2
/// inter header parse reads through [`FrameReferenceStateView`].
///
/// The store borrows the already-decoded key frame (it does not copy pixels): a
/// `ReferenceFrameStore<&DecodedFrame<u8>>` retains a reference handle per slot the
/// §7.23 `refresh_frame_flags` selected.
pub(super) struct InterReferenceState<'a> {
    /// §7.23 reference store, one borrowed decoded frame per refreshed slot.
    pub(super) store: &'a ReferenceFrameStore<&'a DecodedFrame<u8>>,
    /// `RefValid[i]` per slot.
    pub(super) ref_valid: Vec<bool>,
    /// `RefOrderHint[i]` per slot.
    pub(super) ref_order_hint: Vec<u32>,
    /// `RefFrameWidth[i]` per slot.
    pub(super) ref_frame_width: Vec<u32>,
    /// `RefFrameHeight[i]` per slot.
    pub(super) ref_frame_height: Vec<u32>,
    /// `RefBaseQIdx[i]` per slot (§7.23). The §7.7 `get_ref_frames()` ranking scores
    /// this when two or more slots are valid (the multi-reference case); a single valid
    /// slot ignores it. Threaded into the §5.18.2 header parse via
    /// [`FrameReferenceStateView::from_slots_with_base_q_idx`] so the two-valid-slot
    /// derivation is exact.
    pub(super) ref_base_q_idx: Vec<u32>,
    /// `RefFrameType[i] == INTER_FRAME` per slot (§7.23 :14110). The §5
    /// `choose_primary_secondary_ref_frame` CHOOSE-resolution loop (mirror :5468-5495)
    /// scores ONLY inter-typed slots, so this gates which slot a `signal_primary_ref_frame
    /// == 0` (PRIMARY_REF_CHOOSE) frame can resolve its primary reference to.
    pub(super) ref_is_inter: Vec<bool>,
    /// Whether the frame stored in slot `i` ADAPTED its CDFs (`disable_cdf_update == 0`).
    /// The §7.23 cross-frame CDF-load reject fires only when the RESOLVED loaded slot's
    /// saved CDFs are adapted (the minimal decoder does not model `load_cdfs`).
    pub(super) ref_adapted: Vec<bool>,
}

impl InterReferenceState<'_> {
    /// Returns the decoded reference frame stored in `slot`, if any.
    fn frame_for_slot(&self, slot: u32) -> Option<&DecodedFrame<u8>> {
        let slot = ReferenceSlot::new(slot as usize).ok()?;
        self.store.get(slot).ok().flatten().copied()
    }

    /// Builds the §5.18.2 frame-header reference-state view borrowing this state,
    /// modeling `RefBaseQIdx` so the §7.7 two-valid-slot ranking resolves exactly.
    fn header_view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.ref_valid,
            &self.ref_order_hint,
            &self.ref_frame_width,
            &self.ref_frame_height,
            &self.ref_base_q_idx,
        )
    }
}

/// Parses and validates the §5.18.2 inter frame header using the post-key
/// reference state.
pub(super) fn parse_validated_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_>,
) -> Result<FrameHeaderCore> {
    let core = parse_inter_frame_core(envelope, sequence, reference)?;
    validate_inter_frame_core(&core, sequence, envelope.offset)?;
    Ok(core)
}

/// Parses the §5.18.2 inter frame header to [`FrameHeaderParseStatus::InterHeaderComplete`]
/// using the post-key reference state.
fn parse_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        unsupported_at(
            "inter_tile_group_prefix_parse",
            envelope.offset,
            "minimal inter decode requires a parseable first tile-group prefix",
            SPEC_HEADER,
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(unsupported_at(
            "inter_non_first_tile_group",
            envelope.offset,
            "minimal inter decode requires the frame header in the first tile group",
            SPEC_HEADER,
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu: false,
        active_sequence: Some(sequence),
        mfh_record: None,
        reference_state: reference.header_view(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "inter_frame_header_parse",
            envelope.offset,
            "minimal inter decode requires a fully parseable §5.18.2 inter frame header",
            SPEC_HEADER,
        )
    })
}

/// Gates the parsed inter frame header to the verified minimal subset (no tools, no
/// filters, no grain, one tile, an immediate-output inter frame) whose geometry is a
/// single superblock row or single superblock column of 64x64 superblocks.
fn validate_inter_frame_core(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    if core.status != FrameHeaderParseStatus::InterHeaderComplete {
        return Err(unsupported_at(
            "inter_incomplete_frame_header",
            offset,
            "minimal inter decode requires a complete §5.18.2 inter frame header (InterHeaderComplete)",
            SPEC_HEADER,
        ));
    }
    if core.frame_is_intra != Some(false) || core.is_key_frame {
        return Err(unsupported_at(
            "inter_not_inter_frame",
            offset,
            "minimal inter decode requires a non-intra, non-key inter frame",
            SPEC_HEADER,
        ));
    }
    if core.show_existing_frame != Some(false) || core.immediate_output_frame != Some(true) {
        return Err(unsupported_at(
            "inter_unsupported_output_control",
            offset,
            "minimal inter decode requires one immediate-output inter frame (no show-existing-frame indirection)",
            SPEC_HEADER,
        ));
    }
    let Some(frame_size) = core.frame_size else {
        return Err(unsupported_at(
            "inter_unsupported_frame_size",
            offset,
            "minimal inter decode requires a parsed frame size",
            SPEC_HEADER,
        ));
    };
    let width = frame_size.width;
    let height = frame_size.height;
    let superblock_grid = width != 0
        && height != 0
        && width.is_multiple_of(super::MINIMAL_WIDTH)
        && height.is_multiple_of(super::MINIMAL_HEIGHT);
    if !superblock_grid {
        return Err(unsupported_at(
            "inter_unsupported_frame_size",
            offset,
            "minimal inter decode accepts a 2-D grid of 64x64 superblocks (width and height each a positive multiple of 64); a partial (non-multiple-of-64) frame is not yet fixtured",
            SPEC_HEADER,
        ));
    }
    if sequence
        .partition
        .is_none_or(|partition| partition.seq_sb_size() != SuperblockSize::Block64x64)
    {
        return Err(unsupported_at(
            "inter_unsupported_superblock_size",
            offset,
            "minimal inter decode requires 64x64 sequence superblocks; 128x128 / 256x256 superblocks are not yet modelled",
            SPEC_HEADER,
        ));
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(unsupported_at(
            "inter_missing_tile_info",
            offset,
            "minimal inter decode requires a parsed one-tile frame layout",
            SPEC_HEADER,
        ));
    };
    if tile_info.tile_cols != 1 || tile_info.tile_rows != 1 {
        return Err(unsupported_at(
            "inter_multi_tile_frame",
            offset,
            "minimal inter decode supports one tile",
            SPEC_HEADER,
        ));
    }
    if core
        .quantization_params
        .is_none_or(|quant| quant.base_q_idx == 0)
        || core
            .segmentation_params
            .as_ref()
            .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .delta_q_params
            .is_none_or(|delta| delta.delta_q_present)
        || core
            .lossless_info
            .as_ref()
            .is_none_or(|lossless| lossless.coded_lossless)
        || core
            .deblocking_filter_params
            .is_none_or(|filter| filter.apply_deblocking_filter != [false; 4])
        || core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable)
        || core
            .cdef_params
            .as_ref()
            .is_none_or(|cdef| cdef.cdef_frame_enable)
        || core.lr_params.as_ref().is_none_or(|lr| lr.uses_lr)
        || core
            .ccso_params
            .as_ref()
            .is_none_or(|ccso| ccso.ccso_frame_flag.is_some() || !ccso.planes.is_empty())
        || core.inter_tail.as_ref().is_none_or(|tail| {
            tail.apply_grain || tail.tx_mode != TxMode::Largest || tail.skip_mode_present
        })
        || core.inter.as_ref().is_none_or(|inter| {
            inter
                .frame_enabled_motion_modes
                .is_some_and(|modes| modes.iter().any(|&enabled| enabled))
        })
        || sequence.inter.as_ref().is_none_or(|seq_inter| {
            seq_inter.enable_flex_mvres || seq_inter.enable_adaptive_mvd || seq_inter.enable_bawp
        })
    {
        return Err(unsupported_at(
            "inter_unsupported_frame_tools",
            offset,
            "minimal inter decode requires the verified no-tool, no-filter, no-grain, TX_MODE_LARGEST inter frame header",
            SPEC_HEADER,
        ));
    }
    Ok(())
}

fn effective_quantizer_deltas_are_zero(
    sequence: &SequenceHeader,
    quantization: &QuantizationParams,
) -> bool {
    let Some(tq) = sequence.transform_quant_entropy.as_ref() else {
        return false;
    };
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);

    quantization.delta_q_y_dc + seq_quant.base_y_dc_delta_q == 0
        && (seq_quant.num_planes == 1
            || (quantization.delta_q_u_dc + seq_quant.base_uv_dc_delta_q == 0
                && quantization.delta_q_v_dc + seq_quant.base_uv_dc_delta_q == 0
                && quantization.delta_q_u_ac + seq_quant.base_uv_ac_delta_q == 0
                && quantization.delta_q_v_ac + seq_quant.base_uv_ac_delta_q == 0))
}

mod block;
mod compound;
// AV2 § 5 / § 7.7 cross-frame reference-state resolution (CDF-load decision incl. the
// PRIMARY_REF_CHOOSE resolution, and the order-hint wrap check) used by the
// verified-subset rejects below (`DECODE-INTER-MULTIREF-RUNTIME`).
mod cross_frame;
mod find_mv_stack;
mod mc;
pub(in crate::runtime_minimal) mod mv_scaling;
pub(super) mod read_mv;
// AV2 § 5.20.7.12 `read_single_ref` entropy element (`DECODE-INTER-MULTIREF-RUNTIME`):
// the block decode reads `single_ref` when § 7.7 yields `NumTotalRefs == 2` (the
// § 8.3.2 ctx comes from `find_mv_stack::BlockNeighbourContext::single_ref_ctx`).
mod single_ref;

use block::decode_inter_blocks;
use cross_frame::{ResolvedCdfLoad, order_hint_history_unwrapped, resolve_cdf_load};

#[cfg(test)]
mod lr_live_storage_tests;
#[cfg(test)]
mod lr_source_read_tests;
#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            TIER_ID,
            MATRIX_ROW,
            FEATURE_ID,
            spec_section,
            message,
            REMEDIATION,
            Some(byte_offset),
        )),
    }
}

fn unsupported_compound_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            TIER_ID,
            COMPOUND_MATRIX_ROW,
            COMPOUND_FEATURE_ID,
            spec_section,
            message,
            COMPOUND_REMEDIATION,
            Some(byte_offset),
        )),
    }
}
