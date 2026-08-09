// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra (key / intra-only) frame decode through the unified block engine.
//!
//! An intra frame runs the same shared block walk as inter frames
//! ([`decode_inter_blocks`]) with a null reference set (`num_total_refs == 0`), so
//! every block takes the `is_inter == 0` arm and reconstructs through the shared
//! general-intra block parser. The
//! frame-level loop filters run through the same shared `into_filtered_frame` sink
//! as the inter path (deblock, then CDEF over the walk-parsed strength grid, then
//! CCSO and loop-restoration), so intra and inter share one final-filter stage.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, InterpolationFilter};
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, PixelFormat, QmFrameLevels, ReconSample};

use crate::bitstream::tile_payload::{FrameQmScope, FrameQuantizerDeltasScope};
use crate::pipeline::frame_engine::finish::{FilterSinkSetup, FrameWalk};
use crate::pipeline::reconstruct::new_general_intra_workspace_with_visible_rect;
use crate::pipeline::{
    deblock_quant_deltas, derive_tile_plan, derive_visible_luma_rect, ensure_runtime_limits,
    unsupported_at,
};
use crate::prediction::inter::{
    InterReferenceState, decode_inter_blocks, effective_quantizer_deltas,
};
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_intra_frame<T: ReconSample>(
    scratch: &mut crate::prediction::inter::InterDecodeScratch<T>,
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    core: FrameHeaderCore,
    sequence: &SequenceHeader,
    options: &DecodeOptions,
    bit_depth: BitDepth,
) -> Result<FrameWalk<T>> {
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
    // Enforce DecodeLimits before allocating the workspace, as the inter path does.
    let tile_plan = derive_tile_plan(plan, candidate, bytes, sequence, &core, options)?;
    let tile_size = tile_plan
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
        })?;
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
    let visible_luma_rect =
        derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    let mut workspace = new_general_intra_workspace_with_visible_rect::<T>(
        frame_width,
        frame_height,
        bit_depth,
        pixel_format,
        visible_luma_rect,
    )?;

    let reference = InterReferenceState::<T>::empty().map_err(|_| {
        unsupported_at(
            "frame_engine_intra_reference_store",
            offset,
            "intra frame decode requires a reference store",
        )
    })?;

    let _quantizer_delta_scope = FrameQuantizerDeltasScope::install(quantizer_deltas);
    let _qm_scope = FrameQmScope::install(build_frame_qm_levels(&core));

    let (frame_cdfs, filter_inputs, segment_ids) = decode_inter_blocks::<T>(
        scratch,
        tile_plan,
        frame_envelope,
        sequence,
        &core,
        options,
        crate::prediction::inter::InterBlockFacts {
            frame_interpolation_filter: InterpolationFilter::Eighttap,
            num_total_refs: 0,
            reference_select: false,
            num_same_ref_compound: 0,
            qindex,
            luma_use_tcq,
            residual_use_ddt: false,
            bit_depth,
        },
        &[],
        &reference,
        &mut workspace,
    )?;

    let setup = FilterSinkSetup {
        luma_width: frame_width,
        luma_height: frame_height,
        bit_depth,
        gdf_reference: None,
        cfl_ds_filter_index: sequence
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index),
        disable_loopfilters_across_tiles: sequence
            .filter
            .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
        deblock_quant_deltas: deblock_quant_deltas(sequence, &core),
        offset,
    };
    let core = std::sync::Arc::new(core);
    Ok(setup.frame_walk(
        workspace,
        filter_inputs,
        core,
        frame_cdfs,
        segment_ids,
        false,
    ))
}

pub(crate) fn build_frame_qm_levels(core: &FrameHeaderCore) -> Option<QmFrameLevels> {
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
