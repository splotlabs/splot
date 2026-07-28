// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The frame-level parse/reconstruct seam of the inter walk.
//!
//! [`derive_inter_walk_prologue`] settles everything one inter frame's walk
//! reads from its header before any tile syntax: the tile plan, the
//! reconstruction workspace, the § 7.2 filter-sink facts and the frame's
//! quantizer state. The fused walk goes straight on to its tile phase from
//! there; [`parse_inter_frame`] instead runs only the entropy pass and returns
//! a [`DeferredInterWalk`], which owns everything the reconstruction still
//! needs so the driver can run it after it has moved on.

use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, PixelFormat, ReconSample};

use super::*;
use crate::bitstream::tile_payload::{DecodeTilePayloadPlan, FrameQuantizerSnapshot};
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::pipeline::frame_engine::finish::WalkedFrame;

/// One inter frame's header-derived walk state, shared by the fused walk and
/// the split parse pass so both enter their tile phase identically.
pub(super) struct InterWalkPrologue<'payload, T: ReconSample> {
    pub(super) tile_plan: DecodeTilePayloadPlan<'payload>,
    pub(super) workspace: CurrentFrameWorkspace<T>,
    pub(super) setup: FilterSinkSetup,
    pub(super) facts: InterBlockFacts,
    pub(super) ref_frame_idx: Vec<u32>,
    pub(super) quantizer_deltas: splot_recon::QuantizerDeltas,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn derive_inter_walk_prologue<'payload, T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &'payload [u8],
    frame_envelope: ObuEnvelope<'payload>,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<InterWalkPrologue<'payload, T>> {
    let offset = frame_envelope.offset;
    let initial_cdfs = resolve_initial_frame_cdfs(core, sequence, reference, offset)?;
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "inter_missing_frame_size",
            offset,
            "inter.frame_size",
            SPEC_HEADER
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;
    let inter = core.inter.as_ref().ok_or_else(|| {
        inter_missing!(
            "inter_missing_control_region",
            offset,
            "inter.control_region",
            SPEC_HEADER
        )
    })?;
    let tail = core
        .inter_tail
        .as_ref()
        .ok_or_else(|| inter_missing!("inter_missing_tail", offset, "inter.tail", SPEC_HEADER))?;
    let num_total_refs = inter.num_total_refs.unwrap_or(0);
    if !(1..=7).contains(&num_total_refs) {
        return Err(inter_cap!(
            "inter_unsupported_num_total_refs",
            offset,
            "inter.single_ref.num_total_refs not in 1..=7",
            SPEC_MODE_INFO
        ));
    }
    let ref_frame_idx = &inter.ref_frame_idx;
    if ref_frame_idx.len() != num_total_refs as usize || ref_frame_idx.is_empty() {
        return Err(inter_missing!(
            "inter_missing_ref_frame_idx",
            offset,
            "inter.ref_frame_idx",
            SPEC_HEADER
        ));
    }
    let block_reference_select = tail.reference_select;
    if block_reference_select {
        validate_compound_sequence_subset(sequence, core, offset)?;
    }
    let tile_plan = crate::pipeline::derive_inter_tile_plan(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
        &initial_cdfs,
    )?;
    let tile_size = tile_plan
        .work_units()
        .iter()
        .map(crate::bitstream::tile_payload::DecodeTileWorkUnit::tile_size)
        .max()
        .ok_or_else(|| {
            inter_missing!(
                "inter_missing_tile_work_units",
                offset,
                "inter.tile_count > 0",
                "5.20.1"
            )
        })?;
    ensure_runtime_limits(
        options.limits(),
        frame_width,
        frame_height,
        tile_size,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let interpolation_filter = inter.interpolation_filter.ok_or_else(|| {
        inter_missing!(
            "inter_missing_interpolation_filter",
            offset,
            "inter.interpolation_filter",
            SPEC_MC
        )
    })?;
    let visible_luma_rect = derive_visible_luma_rect(sequence, frame_width, frame_height)?;
    let workspace = crate::pipeline::reconstruct::new_general_intra_workspace_with_visible_rect::<T>(
        frame_width as usize,
        frame_height as usize,
        bit_depth,
        PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?,
        visible_luma_rect,
    )?;
    let quantization = core.quantization_params.as_ref().ok_or_else(|| {
        unsupported_at(
            "inter_missing_base_q",
            offset,
            "minimal inter residual decode requires a parsed base_q_idx",
            SPEC_HEADER,
        )
    })?;
    let qindex = quantization.base_q_idx;
    let quantizer_deltas = effective_quantizer_deltas(sequence, quantization).ok_or_else(|| {
        inter_missing!(
            "inter_missing_quantizer_delta_state",
            offset,
            "sequence.transform_quant_entropy",
            SPEC_HEADER
        )
    })?;
    let setup = FilterSinkSetup {
        luma_width: frame_width as usize,
        luma_height: frame_height as usize,
        bit_depth,
        gdf_reference: Some(
            crate::filters::gdf::GdfReferenceContext::from_reference_list(
                core.display_order_hint().unwrap_or(0),
                ref_frame_idx,
                &reference.ref_order_hint,
            ),
        ),
        cfl_ds_filter_index: sequence
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index),
        disable_loopfilters_across_tiles: sequence
            .filter
            .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
        deblock_quant_deltas: crate::pipeline::deblock_quant_deltas(sequence, core),
        offset,
    };
    Ok(InterWalkPrologue {
        tile_plan,
        workspace,
        setup,
        facts: InterBlockFacts {
            frame_interpolation_filter: interpolation_filter,
            num_total_refs: num_total_refs as usize,
            reference_select: block_reference_select,
            num_same_ref_compound: sequence
                .inter
                .as_ref()
                .map_or(0, |seq_inter| seq_inter.num_same_ref_compound)
                .min(u8::try_from(num_total_refs).unwrap_or(u8::MAX)),
            qindex,
            luma_use_tcq: core
                .lossless_info
                .as_ref()
                .is_some_and(|lossless| lossless.allow_tcq),
            residual_use_ddt: sequence
                .transform_quant_entropy
                .as_ref()
                .is_some_and(|tq| tq.enable_inter_ddt),
            bit_depth,
        },
        ref_frame_idx: ref_frame_idx.to_vec(),
        quantizer_deltas,
    })
}

/// Whether one frame's walk can run as an entropy pass now and a deferred
/// reconstruction later.
///
/// The split serves exactly one tile — a multi-tile frame already parses its
/// tiles in parallel — needs a pool wide enough for the superblock prepass, and
/// rules out frame-level intra block copy, whose reconstruction the walk order
/// feeds back into. Every other frame keeps the fused walk.
#[must_use]
pub(crate) fn splittable_inter_frame(obu_type: ObuType, core: &FrameHeaderCore) -> bool {
    matches!(
        obu_type,
        ObuType::LeadingTileGroup | ObuType::RegularTileGroup | ObuType::Switch | ObuType::RasFrame
    ) && !block::global_intrabc_enabled(core.intrabc)
        && core
            .tile_info
            .as_ref()
            .is_some_and(|tiles| tiles.tile_cols == 1 && tiles.tile_rows == 1)
        && splot_parallel::current_pool_width() >= 4
        && splot_parallel::on_multiworker_pool()
}

/// One inter frame whose entropy pass is done and whose reconstruction is
/// still owed.
///
/// Everything the reconstruction reads is owned here, so the driver can record
/// the frame's reference update from the parse products and only then run the
/// reconstruction — after the next frame's entropy pass has already started.
pub(crate) struct DeferredInterWalk<T: ReconSample> {
    /// The frame header the walk consumed, shared with the filter phase.
    pub(crate) core: Arc<FrameHeaderCore>,
    /// The frame's end-of-walk CDF subset, settled by the entropy pass.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    /// The walk-parsed CCSO unit grid, retained for the reference update.
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    /// The geometry the finished frame will report, known from the workspace.
    pub(crate) info: DecodedFrameInfo,
    /// The § 7.9 motion field the reconstruction publishes.
    pub(crate) motion: MotionFieldHandle,
    parse: InterFrameParse,
    workspace: CurrentFrameWorkspace<T>,
    setup: FilterSinkSetup,
    sequence: Arc<SequenceHeader>,
    reference: InterReferenceState<T>,
    ref_frame_idx: Vec<u32>,
    quantizer: FrameQuantizerSnapshot,
}

/// Runs one inter frame's entropy pass and leaves its reconstruction owed.
///
/// The pass reads no reference sample and no projected motion field, so it
/// waits on nothing and can run on a worker while the driver reconstructs the
/// previous frame. It carries the frame's quantizer state explicitly, since
/// those scopes are thread-local to whichever thread installed them.
///
/// # Errors
///
/// Returns the walk's own diagnostic when the header, tile plan, or entropy
/// pass fails.
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_inter_frame<T: ReconSample>(
    records: FrameFilterRecords,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: Arc<SequenceHeader>,
    options: &DecodeOptions,
    reference: InterReferenceState<T>,
    bit_depth: BitDepth,
) -> Result<DeferredInterWalk<T>> {
    let InterWalkPrologue {
        mut tile_plan,
        workspace,
        setup,
        facts,
        ref_frame_idx,
        quantizer_deltas,
    } = derive_inter_walk_prologue(
        plan,
        candidate,
        bytes,
        frame_envelope,
        &core,
        &sequence,
        options,
        &reference,
        bit_depth,
    )?;
    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let quantizer = FrameQuantizerSnapshot::capture();
    let started = crate::timing::start();
    let parse = parse_inter_frame_blocks(
        &mut tile_plan,
        records,
        frame_envelope,
        &sequence,
        &core,
        options,
        facts,
        &ref_frame_idx,
        &reference,
        &workspace,
    )?;
    if started.is_some() {
        crate::timing::report_detail(
            "pass1_parse",
            started,
            &format!("units={}", parse.unit_count()),
        );
    }
    Ok(DeferredInterWalk {
        core: Arc::new(core),
        frame_cdfs: Arc::clone(&parse.frame_cdfs),
        ccso_grid: parse.ccso_grid.clone(),
        info: workspace.info(),
        motion: MotionFieldHandle::pending(),
        parse,
        workspace,
        setup,
        sequence,
        reference,
        ref_frame_idx,
        quantizer,
    })
}

impl<T: ReconSample> DeferredInterWalk<T> {
    /// Runs the frame's § 7.9 prelude, § 7.12 resolve pass and reconstruction,
    /// publishes its motion field as soon as the walk's last unit lands, and
    /// yields the walked frame the § 7.2 filter chain consumes.
    ///
    /// The frame's quantizer scopes are reinstalled here, since the driver's
    /// thread-local state has already moved on to the next frame.
    ///
    /// # Errors
    ///
    /// Returns the reconstruction's own diagnostic.
    pub(crate) fn reconstruct(self, scratch: &mut InterDecodeScratch<T>) -> Result<WalkedFrame<T>> {
        let Self {
            core,
            frame_cdfs: _,
            ccso_grid: _,
            info: _,
            motion,
            parse,
            mut workspace,
            setup,
            sequence,
            reference,
            ref_frame_idx,
            quantizer,
        } = self;
        let _quantizer_scopes = quantizer.install_frame();
        let filter_inputs = parse.reconstruct(
            scratch,
            &sequence,
            &core,
            &ref_frame_idx,
            &reference,
            &mut workspace,
            motion,
        )?;
        Ok(setup.walked_frame(workspace, filter_inputs, core))
    }
}
