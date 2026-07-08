// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra (key / intra-only) frame decode through the unified block engine.
//!
//! An intra frame runs the same shared block walk as inter frames
//! ([`decode_inter_blocks`]) with a null reference set (`num_total_refs == 0`), so
//! every block takes the `is_inter == 0` arm and reconstructs through the shared
//! [`crate::pipeline::general_intra::decode_one_general_intra_block`] callback. The
//! frame-level loop filters run through the same shared `into_filtered_frame` sink
//! as the inter path (deblock, then CDEF over the walk-parsed strength grid, then
//! CCSO and loop-restoration), so intra and inter share one final-filter stage.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, InterpolationFilter};
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{
    BitDepth, DecodedFrame, PixelFormat, QmFrameLevels, ReconSample, ReferenceFrameStore,
};

use crate::bitstream::tile_payload::{FrameCdfSubset, FrameQmScope, FrameQuantizerDeltasScope};
use crate::pipeline::general_intra::general_intra_unsupported;
use crate::pipeline::reconstruct::new_general_intra_workspace;
use crate::pipeline::{
    deblock_quant_deltas, derive_tile_plan, ensure_runtime_limits, unsupported_at,
};
use crate::prediction::inter::{
    InterReferenceState, decode_inter_blocks, effective_quantizer_deltas,
};
use crate::support::capability::missing_capability_message;
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_intra_frame<T: ReconSample>(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    bit_depth: BitDepth,
) -> Result<(
    DecodedFrame<T>,
    FrameCdfSubset,
    Option<crate::filters::ccso::CcsoUnitGrid>,
)> {
    let offset = frame_envelope.offset;
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "frame_engine_intra_missing_frame_size",
            offset,
            "intra frame decode requires a parsed frame size",
        )
    })?;
    let frame_width = frame_size.width as usize;
    let frame_height = frame_size.height as usize;
    let quantization = core.quantization_params.ok_or_else(|| {
        unsupported_at(
            "frame_engine_intra_missing_base_q",
            offset,
            "intra frame decode requires a parsed base_q_idx",
        )
    })?;
    let qindex = quantization.base_q_idx;
    let quantizer_deltas =
        effective_quantizer_deltas(sequence, &quantization).ok_or_else(|| {
            unsupported_at(
                "frame_engine_intra_missing_sequence_quant",
                offset,
                "intra frame decode requires parsed sequence quantizer offsets",
            )
        })?;
    let luma_use_tcq = core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.allow_tcq);
    if core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.has_lossless_segment && !lossless.coded_lossless)
        && mixed_lossless_filters_active(core)
    {
        return Err(general_intra_unsupported(
            "general_intra_mixed_lossless_filters_unimplemented",
            Some(offset),
            missing_capability_message!("filters.lossless_segments", mode = "mixed"),
            "7.17",
        ));
    }
    if core
        .gdf_params
        .as_ref()
        .is_some_and(|gdf| gdf.gdf_frame_enable && gdf.gdf_per_block == Some(true))
    {
        return Err(general_intra_unsupported(
            "general_intra_gdf_per_block_unimplemented",
            Some(offset),
            missing_capability_message!("filters.gdf", per_block = "enabled"),
            "7.20.5",
        ));
    }
    let initial_cdfs = FrameCdfSubset::default_for_base_q(qindex).map_err(|_| {
        unsupported_at(
            "frame_engine_intra_cdf_default_init",
            offset,
            "intra frame decode requires default CDFs",
        )
    })?;

    // Enforce DecodeLimits before allocating the workspace, as the inter path does.
    let tile_size = {
        let tile_plan = derive_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            core,
            options,
        )?;
        tile_plan
            .work_units()
            .iter()
            .map(crate::bitstream::tile_payload::DecodeTileWorkUnit::tile_size)
            .max()
            .ok_or_else(|| {
                unsupported_at(
                    "frame_engine_intra_tile_work_units",
                    offset,
                    "intra frame decode requires at least one tile work unit",
                )
            })?
    };
    ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        tile_size,
        bit_depth,
        sequence.general.chroma_format_idc,
    )?;

    let pixel_format =
        PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?;
    let mut workspace =
        new_general_intra_workspace::<T>(frame_width, frame_height, bit_depth, pixel_format)?;

    let store = ReferenceFrameStore::<&DecodedFrame<T>>::with_capacity(1).map_err(|_| {
        unsupported_at(
            "frame_engine_intra_reference_store",
            offset,
            "intra frame decode requires a reference store",
        )
    })?;
    let reference = InterReferenceState::empty(&store);

    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let _qm_scope = FrameQmScope::install(build_frame_qm_levels(core));

    let (frame_cdfs, filter_inputs) = decode_inter_blocks::<T>(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        core,
        options,
        InterpolationFilter::Eighttap,
        0,
        false,
        None,
        0,
        &[],
        &reference,
        &mut workspace,
        qindex,
        luma_use_tcq,
        false,
        bit_depth,
        initial_cdfs,
    )?;

    let mut filter_sink = crate::filters::wienerns_lr::recon_final_filter_sink(
        workspace,
        frame_width,
        frame_height,
        bit_depth,
    );
    filter_sink.set_deblock_blocks(
        filter_inputs.deblock_blocks,
        filter_inputs.chroma_deblock_blocks,
    );
    filter_sink.set_cdef_grid(Some(filter_inputs.cdef_grid));
    let ccso_grid = filter_inputs.ccso_grid.clone();
    filter_sink.set_ccso_grid(filter_inputs.ccso_grid);
    filter_sink.set_cfl_ds_filter_index(
        sequence
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index),
    );
    filter_sink.set_tx_skip_records(filter_inputs.tx_skip_records);
    filter_sink.set_lr_source_blocks(filter_inputs.lr_source_blocks);
    filter_sink.set_lr_unit_filters(filter_inputs.lr_unit_filters);
    let frame =
        filter_sink.into_filtered_frame(core, deblock_quant_deltas(sequence, core), offset)?;
    Ok((frame, frame_cdfs, ccso_grid))
}

fn mixed_lossless_filters_active(core: &FrameHeaderCore) -> bool {
    core.deblocking_filter_params
        .as_ref()
        .is_some_and(|filter| filter.apply_deblocking_filter.iter().any(|active| *active))
        || core
            .gdf_params
            .as_ref()
            .is_some_and(|gdf| gdf.gdf_frame_enable)
        || core
            .cdef_params
            .as_ref()
            .is_some_and(|cdef| cdef.cdef_frame_enable)
        || core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr)
        || core.lr_params_partial.as_ref().is_some_and(|lr| lr.uses_lr)
        || core
            .ccso_params
            .as_ref()
            .is_some_and(|ccso| ccso.planes.iter().any(|plane| plane.ccso_planes))
}

fn build_frame_qm_levels(core: &FrameHeaderCore) -> Option<QmFrameLevels> {
    let qm = core.setup_qm_params.filter(|qm| qm.using_qmatrix)?;
    let levels_le8 = core
        .lossless_info
        .as_ref()
        .map_or([[0u8; 3]; 16], |lossless| lossless.seg_qm_levels);
    Some(QmFrameLevels {
        levels_gt8: [qm.levels[0].qm_y, qm.levels[0].qm_u, qm.levels[0].qm_v],
        levels_le8,
    })
}

#[cfg(test)]
#[path = "intra_tests.rs"]
mod tests;
