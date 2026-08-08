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
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneSize,
    QuantizerDeltas, ReconSample,
};

use super::*;
use crate::bitstream::tile_payload::{DecodeTilePayloadPlan, FrameQuantizerSnapshot};
use crate::filters::wienerns_lr::FrameFilterRecords;

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

fn invalid_inter_reference_map(offset: splot_core::span::ByteOffset) -> DecodeError {
    inter_missing!(
        "inter_missing_ref_frame_idx",
        offset,
        "inter.num_total_refs in 0..=7 with matching inter.ref_frame_idx",
        SPEC_HEADER
    )
}

/// Derives the pending slot's header-known geometry without parsing tile syntax.
pub(crate) fn inter_frame_info(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    bit_depth: BitDepth,
    offset: splot_core::span::ByteOffset,
) -> Result<DecodedFrameInfo> {
    let frame_size = core.frame_size.ok_or_else(|| {
        inter_missing!(
            "inter_pending_missing_frame_size",
            offset,
            "inter.frame_size",
            SPEC_HEADER
        )
    })?;
    let pixel_format =
        PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?;
    let visible = derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    Ok(DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        PlaneSize::new(frame_size.width as usize, frame_size.height as usize)?,
        visible,
    )?)
}

/// Derives the fixed motion-band completion layout before entropy admission.
pub(crate) fn motion_field_layout(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    info: DecodedFrameInfo,
    offset: splot_core::span::ByteOffset,
) -> Result<MotionFieldLayout> {
    let sb_h4 = block::superblock_h4(sequence, core).ok_or_else(|| {
        inter_cap!(
            "inter_temporal_motion_layout_superblock",
            offset,
            "inter.temporal_motion_field",
            SPEC_MODE_INFO
        )
    })?;
    let luma = info.coded_luma_size();
    MotionFieldLayout::new(luma.height().div_ceil(4), luma.width().div_ceil(4), sb_h4).ok_or_else(
        || {
            inter_cap!(
                "inter_frame_temporal_motion_layout",
                offset,
                "inter.temporal_motion_field",
                SPEC_MODE_INFO
            )
        },
    )
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
            "inter_walk_missing_frame_size",
            offset,
            "inter.frame_size",
            SPEC_HEADER
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let tail = core
        .inter_tail
        .as_ref()
        .ok_or_else(|| inter_missing!("inter_missing_tail", offset, "inter.tail", SPEC_HEADER))?;
    let num_total_refs = inter
        .num_total_refs
        .filter(|count| *count <= 7)
        .ok_or_else(|| invalid_inter_reference_map(offset))?;
    let ref_frame_idx = &inter.ref_frame_idx;
    if ref_frame_idx.len() != num_total_refs as usize {
        return Err(invalid_inter_reference_map(offset));
    }
    let block_reference_select = tail.reference_select;
    let tile_plan = crate::pipeline::derive_inter_tile_plan(
        plan,
        candidate,
        bytes,
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
    let quantizer_deltas = required_inter_quantizer_deltas(sequence, quantization)?;
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

pub(super) fn required_inter_quantizer_deltas(
    sequence: &SequenceHeader,
    quantization: &splot_core::headers::frame::QuantizationParams,
) -> Result<QuantizerDeltas> {
    effective_quantizer_deltas(sequence, quantization)
        .ok_or_else(|| DecodeHeaderStateError::MissingSequenceTransformQuantEntropy.into())
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
        && splot_parallel::current_pool_width() >= 2
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

/// One deferred frame whose reconstruction units are scheduler-owned.
pub(crate) struct ScheduledInterWalk<T: ReconSample> {
    reconstruction: block::ScheduledInterReconstruction<T>,
}

impl<T: ReconSample> ScheduledInterWalk<T> {
    /// Number of independently admitted reconstruction units.
    pub(crate) const fn len(&self) -> usize {
        self.reconstruction.len()
    }

    /// Number of final-filter jobs joined before terminal freeze.
    pub(crate) const fn filter_count(&self) -> usize {
        self.reconstruction.filter_count()
    }

    pub(crate) fn resolve_len(&self) -> usize {
        self.reconstruction.resolve_len()
    }

    pub(crate) fn resolve_conditions(&self, index: usize) -> Vec<splot_parallel::Condition<'_>> {
        self.reconstruction.resolve_conditions(index)
    }

    pub(crate) fn resolve(&self, index: usize) -> Result<core::ops::Range<usize>> {
        self.reconstruction.resolve(index)
    }

    pub(crate) fn fail_temporal(&self) {
        self.reconstruction.fail_temporal();
    }

    /// Cross-frame conditions for one reconstruction unit.
    pub(crate) fn conditions(&self, index: usize) -> Vec<splot_parallel::Condition<'_>> {
        self.reconstruction.conditions(index)
    }

    /// Precomputes one admitted reconstruction unit.
    pub(crate) fn precompute(&self, index: usize) -> Result<()> {
        self.reconstruction.precompute(index)
    }

    /// Commits one precomputed unit and returns the frontier links its
    /// canonical rows released.
    pub(crate) fn commit(&self, index: usize) -> Result<block::ScheduledCommitProgress> {
        self.reconstruction.commit(index)
    }

    pub(crate) fn take_scheduled_scratch(&self) -> Result<InterDecodeScratch<T>> {
        self.reconstruction.take_scheduled_scratch()
    }

    /// Number of ordered links in this frame's § 7.17 frontier chain.
    pub(crate) const fn frontier_len(&self) -> usize {
        self.reconstruction.frontier_len()
    }

    /// Advances the § 7.17 frontier over one sealed superblock row and returns
    /// the walked frame after the final link.
    pub(crate) fn frontier(&self, row: usize) -> Result<block::ScheduledFrameProgress<T>> {
        self.reconstruction.frontier(row)
    }
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
    motion: MotionFieldHandle,
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
    motion.publish_metadata(parse.motion_field_metadata());
    Ok(DeferredInterWalk {
        core: Arc::new(core),
        frame_cdfs: Arc::clone(&parse.frame_cdfs),
        ccso_grid: parse.ccso_grid.clone(),
        motion,
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
    /// Shares the reference motion handles that gate this frame's temporal
    /// prelude.
    pub(crate) fn motion_dependencies(&self) -> Vec<MotionFieldHandle> {
        self.reference.motion_dependencies(&self.ref_frame_idx)
    }

    /// Runs the temporal prelude and resolve/motion half-pass, returning the
    /// per-unit reconstruction state consumed by the admission scheduler.
    pub(crate) fn prepare_scheduled(
        self,
        mut decode_scratch: InterDecodeScratch<T>,
        temporal_scratch: super::find_mv_stack::TemporalMvScratch,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    ) -> Result<(
        ScheduledInterWalk<T>,
        super::find_mv_stack::TemporalMvScratch,
    )> {
        let Self {
            core,
            frame_cdfs: _,
            ccso_grid: _,
            motion,
            parse,
            workspace,
            setup,
            sequence,
            reference,
            ref_frame_idx,
            quantizer,
        } = self;
        let _quantizer_scopes = quantizer.install_frame();
        let core_for_reconstruction = Arc::clone(&core);
        decode_scratch.install_temporal_scratch(temporal_scratch);
        let (reconstruction, temporal_scratch) = parse.prepare_scheduled(
            decode_scratch,
            setup,
            progress,
            sequence,
            core_for_reconstruction,
            Arc::from(ref_frame_idx),
            Arc::new(reference),
            workspace,
            motion,
        )?;
        Ok((ScheduledInterWalk { reconstruction }, temporal_scratch))
    }
}
