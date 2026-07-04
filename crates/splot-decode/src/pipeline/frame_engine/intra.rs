// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra (key / intra-only) frame decode through the unified block engine.
//!
//! An intra frame runs the same shared block walk as inter frames
//! ([`decode_inter_blocks`]) with a null reference set (`num_total_refs == 0`), so
//! every block takes the `is_inter == 0` arm and reconstructs through the shared
//! [`crate::pipeline::general_intra::decode_one_general_intra_block`] callback. The
//! frame-level loop filters stay on the intra sink (deblock + CDEF then freeze);
//! unifying the sink with the inter `into_filtered_frame` path is a later step.

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, InterpolationFilter};
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, DecodedFrame, ReconSample, ReferenceFrameStore};

use crate::bitstream::tile_payload::{FrameCdfSubset, frame_mi_dimensions};
use crate::pipeline::general_intra::{cdef_frame_params, general_intra_unsupported};
use crate::pipeline::reconstruct::new_general_intra_workspace;
use crate::pipeline::{deblock_quant_deltas, unsupported_at};
use crate::prediction::inter::{InterReferenceState, decode_inter_blocks};
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
        .segmentation_params
        .as_ref()
        .is_some_and(|seg| seg.segmentation_enabled)
    {
        return Err(general_intra_unsupported(
            "general_intra_segment_id_unimplemented",
            Some(offset),
            missing_capability_message!("intra.segmentation", segment_id = "enabled"),
            "5.20.5.7",
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
    let (mi_rows, mi_cols) = frame_mi_dimensions(core).map_err(|_| {
        unsupported_at(
            "frame_engine_intra_mi_dimensions",
            offset,
            "intra frame decode requires frame mi dimensions",
        )
    })?;
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
    let initial_cdfs = FrameCdfSubset::default_for_base_q(qindex).map_err(|_| {
        unsupported_at(
            "frame_engine_intra_cdf_default_init",
            offset,
            "intra frame decode requires default CDFs",
        )
    })?;

    let mut workspace = new_general_intra_workspace::<T>(frame_width, frame_height, bit_depth)?;

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

    if let Some(filter) = core.deblocking_filter_params {
        crate::filters::deblock::deblock_general_intra_frame(
            &mut workspace,
            &filter_inputs.deblock_blocks,
            [
                &filter_inputs.chroma_deblock_blocks[0],
                &filter_inputs.chroma_deblock_blocks[1],
            ],
            mi_rows,
            mi_cols,
            filter,
            deblock_quant_deltas(sequence, core),
            bit_depth,
        )
        .map_err(|_| unsupported_at("frame_engine_intra_deblock", offset, "intra frame deblock"))?;
    }
    if let Some(params) = cdef_frame_params(core) {
        crate::filters::cdef::cdef_general_intra_frame(
            &mut workspace,
            params,
            mi_rows,
            mi_cols,
            bit_depth,
        )
        .map_err(|_| unsupported_at("frame_engine_intra_cdef", offset, "intra frame cdef"))?;
    }
    let frame = workspace.freeze()?;
    Ok((frame, frame_cdfs))
}
