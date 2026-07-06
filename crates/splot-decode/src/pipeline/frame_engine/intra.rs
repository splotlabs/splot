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

use crate::bitstream::tile_payload::{FrameCdfSubset, FrameQmScope};
use crate::pipeline::general_intra::general_intra_unsupported;
use crate::pipeline::reconstruct::new_general_intra_workspace;
use crate::pipeline::{
    deblock_quant_deltas, derive_tile_plan, ensure_runtime_limits, unsupported_at,
};
use crate::prediction::inter::{
    InterReferenceState, decode_inter_blocks, effective_quantizer_deltas_are_zero,
};
use crate::support::capability::missing_capability_message;
use crate::{DecodeOptions, DecodePlannedObu, DecodeStreamPlan, Result};

/// Decodes one intra frame through the unified block engine, returning the
/// reconstructed frame and the end-of-frame CDF subset.
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
) -> Result<(DecodedFrame<T>, FrameCdfSubset)> {
    let offset = frame_envelope.offset;
    if core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.has_lossless_segment)
    {
        return Err(general_intra_unsupported(
            "general_intra_lossless_segment_unimplemented",
            Some(offset),
            missing_capability_message!("intra.segmentation", lossless = "segment"),
            "5.18.2",
        ));
    }
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "frame_engine_intra_missing_frame_size",
            offset,
            "intra frame decode requires a parsed frame size",
        )
    })?;
    let frame_width = frame_size.width as usize;
    let frame_height = frame_size.height as usize;
    let qindex = core
        .quantization_params
        .map(|quant| quant.base_q_idx)
        .ok_or_else(|| {
            unsupported_at(
                "frame_engine_intra_missing_base_q",
                offset,
                "intra frame decode requires a parsed base_q_idx",
            )
        })?;
    let luma_use_tcq = core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.allow_tcq);
    if core
        .gdf_params
        .as_ref()
        .is_some_and(|gdf| gdf.gdf_frame_enable)
    {
        return Err(general_intra_unsupported(
            "general_intra_gdf_frame_unimplemented",
            Some(offset),
            missing_capability_message!("filters.gdf", frame = "enabled"),
            "7.20.5",
        ));
    }
    if core.setup_qm_params.is_some_and(|qm| qm.using_qmatrix)
        && core
            .segmentation_params
            .as_ref()
            .is_some_and(|seg| seg.segmentation_enabled && seg.last_active_seg_id > 0)
    {
        return Err(general_intra_unsupported(
            "general_intra_qmatrix_segmented_blocks_unimplemented",
            Some(offset),
            missing_capability_message!("intra.qmatrix.segmentation", segment = "active"),
            "7.14.4",
        ));
    }
    if core
        .quantization_params
        .as_ref()
        .is_some_and(|quantization| !effective_quantizer_deltas_are_zero(sequence, quantization))
    {
        return Err(general_intra_unsupported(
            "general_intra_nonzero_quantizer_delta_unimplemented",
            Some(offset),
            missing_capability_message!("intra.residual.nonzero_quantizer_delta"),
            "7.14.2",
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
        let mut tile_plan = derive_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            core,
            options,
        )?;
        let [tile] = tile_plan.work_units_mut() else {
            return Err(unsupported_at(
                "frame_engine_intra_tile_work_units",
                offset,
                "intra frame decode requires exactly one tile work unit",
            ));
        };
        tile.tile_size()
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
    let reference = InterReferenceState {
        store: &store,
        ref_valid: Vec::new(),
        ref_order_hint: Vec::new(),
        ref_frame_width: Vec::new(),
        ref_frame_height: Vec::new(),
        ref_base_q_idx: Vec::new(),
        ref_is_inter: Vec::new(),
        ref_adapted: Vec::new(),
        lr_frame_filter_class_counts: Vec::new(),
        lr_frame_filter_taps: Vec::new(),
        ref_frame_cdfs: Vec::new(),
    };

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
    Ok((frame, frame_cdfs))
}

/// The frame's § 7.14.4 built-in quantization-matrix levels for the general-intra
/// dequant, or `None` when `using_qmatrix == 0`. `levels_gt8` is `qm_y/u/v[0]` (used
/// when `tw > 8 || th > 8`); `levels_le8` is `SegQMLevel[Y/U/V][0]`. Multi-segment
/// QM frames fail closed before decode until per-block `segment_id` reaches dequant.
fn build_frame_qm_levels(core: &FrameHeaderCore) -> Option<QmFrameLevels> {
    let qm = core.setup_qm_params.filter(|qm| qm.using_qmatrix)?;
    let levels_le8 = core
        .lossless_info
        .as_ref()
        .map_or([0u8; 3], |lossless| lossless.seg_qm_levels[0]);
    Some(QmFrameLevels {
        levels_gt8: [qm.levels[0].qm_y, qm.levels[0].qm_u, qm.levels[0].qm_v],
        levels_le8,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use splot_core::obu::{ParsedObu, PayloadStatus};
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
    use splot_core::types::ObuType;
    use splot_parallel::ThreadCount;

    use super::*;
    use crate::error::DecodeError;
    use crate::pipeline::parse_frame_core;
    use crate::{DecodeContext, DecodeRuntimeConfig};

    const Q80_FIXTURE: &[u8] = include_bytes!(
        "../../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf"
    );

    fn unsupported_reason(error: DecodeError) -> &'static str {
        let DecodeError::UnsupportedFeature { unsupported } = error else {
            panic!("expected unsupported-feature");
        };
        unsupported.reason()
    }

    fn decode_intra_fixture_with_core(mutate: impl FnOnce(&mut FrameHeaderCore)) -> DecodeError {
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let options = DecodeOptions::default();
        let plan = context.plan_bytes(Q80_FIXTURE, options).expect("plan");
        let candidate = plan.frame_candidates().next().expect("candidate").clone();
        let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(Q80_FIXTURE) else {
            panic!("fixture is IVF");
        };
        let obus = || parsed.frames.iter().flat_map(|frame| frame.obus.iter());
        let sequence = obus()
            .find_map(
                |envelope| match envelope.payload_status().expect("payload status") {
                    PayloadStatus::Parsed(ParsedObu::SequenceHeader(sequence)) => {
                        Some((*sequence).clone())
                    }
                    _ => None,
                },
            )
            .expect("sequence");
        let key = obus()
            .find(|envelope| envelope.header.obu_type == ObuType::ClosedLoopKey)
            .copied()
            .expect("key");
        let mut core = parse_frame_core(key, &sequence).expect("core");
        mutate(&mut core);
        context
            .pool()
            .install(|| {
                decode_intra_frame::<u8>(
                    &plan,
                    &candidate,
                    Q80_FIXTURE,
                    key,
                    &core,
                    &sequence,
                    &options,
                    BitDepth::Eight,
                )
            })
            .expect_err("mutated fixture must fail closed")
    }

    #[test]
    fn intra_gate_rejects_gdf_enabled_frame() {
        let error = decode_intra_fixture_with_core(|core| {
            core.gdf_params
                .as_mut()
                .expect("gdf params")
                .gdf_frame_enable = true;
        });
        assert_eq!(
            unsupported_reason(error),
            "general_intra_gdf_frame_unimplemented"
        );
    }

    #[test]
    fn intra_gate_rejects_qmatrix_with_multiple_active_segments() {
        let error = decode_intra_fixture_with_core(|core| {
            core.setup_qm_params
                .as_mut()
                .expect("setup qm")
                .using_qmatrix = true;
            let seg = core
                .segmentation_params
                .as_mut()
                .expect("segmentation params");
            seg.segmentation_enabled = true;
            seg.last_active_seg_id = 1;
        });
        assert_eq!(
            unsupported_reason(error),
            "general_intra_qmatrix_segmented_blocks_unimplemented"
        );
    }

    #[test]
    fn intra_gate_rejects_nonzero_effective_quantizer_deltas() {
        let error = decode_intra_fixture_with_core(|core| {
            core.quantization_params
                .as_mut()
                .expect("quantization params")
                .delta_q_y_dc = 1;
        });
        assert_eq!(
            unsupported_reason(error),
            "general_intra_nonzero_quantizer_delta_unimplemented"
        );
    }
}
