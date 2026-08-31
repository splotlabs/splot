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
use splot_core::headers::frame::{FrameHeaderCore, FrameSize};
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
    pub(super) ref_frame_idx: RefIdxBuf,
    pub(super) quantizer_deltas: splot_recon::QuantizerDeltas,
}

/// Validated geometry shared by every stage of one frame decode.
#[derive(Clone, Copy)]
pub(crate) struct FrameDecodeGeometry {
    frame_size: FrameSize,
    info: DecodedFrameInfo,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    motion_layout: MotionFieldLayout,
    frame_is_intra: bool,
}

impl FrameDecodeGeometry {
    /// Derives all frame geometry before pending state or decode storage is created.
    pub(crate) fn new(
        core: &FrameHeaderCore,
        sequence: &SequenceHeader,
        bit_depth: BitDepth,
        frame_is_intra: bool,
    ) -> Result<Self> {
        if core.frame_is_intra != Some(frame_is_intra) {
            return Err(DecodeHeaderStateError::IncompleteInterFrame.into());
        }
        let frame_size = core
            .frame_size
            .ok_or(DecodeHeaderStateError::MissingFrameSize)?;
        if frame_size.width == 0 || frame_size.height == 0 {
            return Err(DecodeHeaderStateError::ZeroFrameSize.into());
        }
        let partition = sequence
            .partition
            .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
        let mi_dimension = |samples: u32| {
            usize::try_from(samples)
                .ok()
                .and_then(|samples| samples.checked_add(7))
                .and_then(|samples| (samples >> 3).checked_mul(2))
                .ok_or(DecodeHeaderStateError::InvalidInterTileConstructionState)
        };
        let mi_cols = mi_dimension(frame_size.width)?;
        let mi_rows = mi_dimension(frame_size.height)?;
        let pixel_format =
            PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?;
        let visible = derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            bit_depth,
            pixel_format,
            PlaneSize::new(frame_size.width as usize, frame_size.height as usize)?,
            visible,
        )?;
        let sb_h4 = block::frame_superblock_h4(partition.seq_sb_size(), frame_is_intra);
        let motion_layout = MotionFieldLayout::new(mi_rows, mi_cols, sb_h4)
            .ok_or(DecodeHeaderStateError::InvalidInterTileConstructionState)?;
        Ok(Self {
            frame_size,
            info,
            mi_rows,
            mi_cols,
            sb_h4,
            motion_layout,
            frame_is_intra,
        })
    }

    pub(crate) const fn frame_size(self) -> FrameSize {
        self.frame_size
    }

    pub(crate) const fn info(self) -> DecodedFrameInfo {
        self.info
    }

    pub(crate) const fn mi_dimensions(self) -> (usize, usize) {
        (self.mi_rows, self.mi_cols)
    }

    pub(crate) const fn sb_h4(self) -> usize {
        self.sb_h4
    }

    pub(crate) const fn motion_layout(self) -> MotionFieldLayout {
        self.motion_layout
    }

    pub(crate) const fn frame_is_intra(self) -> bool {
        self.frame_is_intra
    }

    pub(crate) fn new_motion_field(
        self,
        reference_order_hints: &[Option<u32>],
    ) -> Option<TemporalMotionField> {
        let coded_size = self.info.coded_luma_size();
        let mut field = TemporalMotionField::new_with_metadata(
            self.mi_rows,
            self.mi_cols,
            !self.frame_is_intra,
            (coded_size.width(), coded_size.height()),
            reference_order_hints,
        )?;
        field.set_band_rows8(self.motion_layout.band_rows8());
        Some(field)
    }
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
    geometry: FrameDecodeGeometry,
) -> Result<InterWalkPrologue<'payload, T>> {
    let offset = frame_envelope.offset;
    let initial_cdfs = resolve_initial_frame_cdfs(core, sequence, reference, candidate, offset)?;
    let frame_size = geometry.frame_size();
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let tail = core
        .inter_tail
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterTail)?;
    let num_total_refs = inter
        .num_total_refs
        .filter(|count| *count <= 7)
        .ok_or(DecodeHeaderStateError::InvalidInterReferenceMap)?;
    let ref_frame_idx = &inter.ref_frame_idx;
    if ref_frame_idx.len() != num_total_refs as usize {
        return Err(DecodeHeaderStateError::InvalidInterReferenceMap.into());
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
        .fold(0, u64::max);
    ensure_runtime_limits(
        options.limits(),
        frame_width,
        frame_height,
        tile_size,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;
    let interpolation_filter = inter
        .interpolation_filter
        .ok_or(DecodeHeaderStateError::MissingInterpolationFilter)?;
    let workspace = CurrentFrameWorkspace::<T>::new_recycled(geometry.info())?; // pooled buffer keeps the previous frame's samples: restore the fill if § 7.11/§ 7.13 ever leave a coded sample unwritten
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
                ref_frame_idx.as_slice(),
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
    };
    Ok(InterWalkPrologue {
        tile_plan,
        workspace,
        setup,
        facts: InterBlockFacts {
            geometry,
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
        },
        ref_frame_idx: ref_frame_idx.clone(),
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
    core.status == splot_core::headers::frame::FrameHeaderParseStatus::InterHeaderComplete
        && core.frame_is_intra == Some(false)
        && matches!(
            obu_type,
            ObuType::LeadingTileGroup
                | ObuType::RegularTileGroup
                | ObuType::Switch
                | ObuType::RasFrame
        )
        && !block::global_intrabc_enabled(core.intrabc)
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
/// Everything reconstruction reads is owned here. Entropy products publish to
/// the canonical `PipelineFrame`, while the reference update records header
/// metadata before reconstruction runs after the next frame's entropy pass.
pub(crate) struct DeferredInterWalk<T: ReconSample> {
    parse: InterFrameParse,
    parse_progress: Arc<super::ParseProgress>,
    marker: core::marker::PhantomData<T>,
}

/// The half of an inter walk that is settled before the § 8.2 pass reads its
/// first unit, so the admission scheduler can be built while the pass runs.
pub(crate) struct InterWalkEarly<T: ReconSample> {
    pub(crate) core: Arc<FrameHeaderCore>,
    pub(crate) motion: MotionFieldHandle,
    pub(crate) workspace: CurrentFrameWorkspace<T>,
    pub(crate) setup: FilterSinkSetup,
    pub(crate) sequence: Arc<SequenceHeader>,
    pub(crate) reference: Arc<InterReferenceState<T>>,
    pub(crate) ref_frame_idx: RefIdxBuf,
    pub(crate) quantizer: FrameQuantizerSnapshot,
    pub(crate) parse_progress: Arc<super::ParseProgress>,
    pub(crate) params: block::TileWalkParams,
    pub(crate) prelude: block::TemporalPrelude,
    pub(crate) motion_field: super::find_mv_stack::TemporalMotionField,
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
    sequence: &Arc<SequenceHeader>,
    options: &DecodeOptions,
    reference: InterReferenceState<T>,
    bit_depth: BitDepth,
    geometry: FrameDecodeGeometry,
    motion: &MotionFieldHandle,
    parse_progress: &Arc<super::ParseProgress>,
    publish_early: impl FnOnce(InterWalkEarly<T>),
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
        sequence,
        options,
        &reference,
        bit_depth,
        geometry,
    )?;
    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let quantizer = FrameQuantizerSnapshot::capture();
    let tile_count = tile_plan.work_units().len();
    let [tile] = tile_plan.work_units_mut() else {
        return Err(DecodeHeaderStateError::InvalidSplitTileCount { actual: tile_count }.into());
    };
    let block_setup = super::block::derive_inter_block_setup(
        std::slice::from_mut(tile),
        sequence,
        &core,
        options,
        facts,
        ref_frame_idx.as_slice(),
        &reference,
    )?;
    motion.publish_metadata(block_setup.motion_field_metadata());
    let (parse_setup, params, prelude, motion_field) = block_setup.split();
    block::publish_tile_geometry(
        tile,
        &params,
        sequence,
        &core,
        &reference,
        ref_frame_idx.as_slice(),
        parse_progress,
    )?;
    let core = Arc::new(core);
    let reference = Arc::new(reference);
    publish_early(InterWalkEarly {
        core: Arc::clone(&core),
        motion: motion.clone(),
        workspace,
        setup,
        sequence: Arc::clone(sequence),
        reference: Arc::clone(&reference),
        ref_frame_idx: ref_frame_idx.clone(),
        quantizer,
        parse_progress: Arc::clone(parse_progress),
        params,
        prelude,
        motion_field,
    });
    let parse = match parse_inter_frame_blocks(
        tile,
        records,
        sequence,
        &core,
        ref_frame_idx.as_slice(),
        &reference,
        parse_progress,
        parse_setup,
    ) {
        Ok(parse) => parse,
        Err(error) => {
            parse_progress.fail();
            return Err(error);
        }
    };
    Ok(DeferredInterWalk {
        parse,
        parse_progress: Arc::clone(parse_progress),
        marker: core::marker::PhantomData,
    })
}

impl<T: ReconSample> DeferredInterWalk<T> {
    /// Hands the scheduled frontier the filter state the pass settles last.
    pub(crate) fn attach_filters(
        self,
        pending: block::PendingFilterAttach<T>,
        reconstruction: &block::ScheduledTileRecon<T>,
    ) -> Result<()> {
        self.parse
            .attach_filters(pending, reconstruction, &self.parse_progress)
    }

    /// The frame's end-of-walk CDF subset, settled by the entropy pass.
    pub(crate) const fn frame_cdfs(&self) -> &Arc<FrameCdfSubset> {
        &self.parse.frame_cdfs
    }

    /// The walk-parsed CCSO unit grid published to the canonical `PipelineFrame`.
    pub(crate) const fn ccso_grid(&self) -> Option<&crate::filters::ccso::CcsoUnitGrid> {
        self.parse.ccso_grid.as_ref()
    }

    /// The segment id map published to the canonical `PipelineFrame`.
    pub(crate) const fn segment_ids(
        &self,
    ) -> &Arc<crate::bitstream::tile_payload::FrameSegmentIdMap> {
        &self.parse.segment_ids
    }
}

impl<T: ReconSample> InterWalkEarly<T> {
    /// Shares the reference motion handles that gate this frame's temporal
    /// prelude.
    pub(crate) fn motion_dependencies(&self) -> Vec<MotionFieldHandle> {
        self.reference
            .motion_dependencies(self.ref_frame_idx.as_slice())
    }

    /// Runs the temporal prelude and builds the admission scheduler, which is
    /// everything the § 8.2 pass does not have to have finished for.
    pub(crate) fn prepare_scheduled(
        self,
        mut decode_scratch: InterDecodeScratch<T>,
        temporal_scratch: super::find_mv_stack::TemporalMvScratch,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    ) -> Result<(
        block::ScheduledTileRecon<T>,
        super::find_mv_stack::TemporalMvScratch,
        block::PendingFilterAttach<T>,
    )> {
        let Self {
            core,
            motion,
            workspace,
            setup,
            sequence,
            reference,
            ref_frame_idx,
            quantizer,
            parse_progress,
            params,
            prelude,
            motion_field,
        } = self;
        let _quantizer_scopes = quantizer.install_frame();
        decode_scratch.install_temporal_scratch(temporal_scratch);
        let (reconstruction, temporal_scratch, pending) = block::prepare_scheduled_recon(
            decode_scratch,
            setup,
            progress,
            sequence,
            core,
            ref_frame_idx,
            reference,
            workspace,
            motion,
            &parse_progress,
            &params,
            prelude,
            motion_field,
        )?;
        Ok((reconstruction, temporal_scratch, pending))
    }
}
