// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;
use splot_parallel::ThreadCount;
use splot_recon::DecodedFrame;

use super::*;
use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::error::DecodeError;
use crate::pipeline::frame_engine::finish::finish_walked_frame;
use crate::pipeline::parse_frame_core;
use crate::{DecodeContext, DecodeRuntimeConfig};

const Q80_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");

fn unsupported_reason(error: DecodeError) -> &'static str {
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported-feature");
    };
    unsupported.reason()
}

fn decode_intra_fixture_with_core(
    mutate: impl FnOnce(&mut FrameHeaderCore),
) -> crate::Result<(
    DecodedFrame<u8>,
    std::sync::Arc<FrameCdfSubset>,
    Option<crate::filters::ccso::CcsoUnitGrid>,
)> {
    decode_intra_fixture_with_core_on_threads(ThreadCount::from(1usize), mutate)
}

fn decode_intra_fixture_with_core_on_threads(
    threads: ThreadCount,
    mutate: impl FnOnce(&mut FrameHeaderCore),
) -> crate::Result<(
    DecodedFrame<u8>,
    std::sync::Arc<FrameCdfSubset>,
    Option<crate::filters::ccso::CcsoUnitGrid>,
)> {
    let context = DecodeContext::new(DecodeRuntimeConfig::new(threads)).expect("context");
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
    let walk = context.pool().install(|| {
        crate::pipeline::frame_engine::walk_frame::<u8>(
            &mut crate::prediction::inter::InterDecodeScratch::default(),
            &plan,
            &candidate,
            Q80_FIXTURE,
            key,
            core,
            &sequence,
            &options,
            &crate::pipeline::frame_engine::FrameSetup::Intra,
            BitDepth::Eight,
        )
    })?;
    let WalkStage::Pending(walked) = walk.stage else {
        panic!("an intra frame always owes its filter phase");
    };
    let finished = context.pool().install(|| finish_walked_frame(*walked))?;
    Ok((finished.frame, walk.frame_cdfs, walk.ccso_grid))
}

#[test]
fn frame_gdf_is_bit_exact_across_worker_counts() {
    let decode = |threads| {
        decode_intra_fixture_with_core_on_threads(threads, |core| {
            let gdf = core.gdf_params.as_mut().expect("gdf params");
            gdf.gdf_frame_enable = true;
            gdf.gdf_per_block = Some(false);
            gdf.gdf_pic_qc_idx = Some(0);
            gdf.gdf_pic_scale_idx = Some(0);
        })
        .expect("active frame GDF decode")
        .0
    };
    let serial = decode(ThreadCount::from(1usize));
    let parallel = decode(ThreadCount::from(4usize));

    assert_eq!(parallel.y().samples(), serial.y().samples());
    assert_eq!(
        parallel.u().expect("parallel U").samples(),
        serial.u().expect("serial U").samples()
    );
    assert_eq!(
        parallel.v().expect("parallel V").samples(),
        serial.v().expect("serial V").samples()
    );
}

#[test]
fn intra_gate_rejects_gdf_per_block_frame() {
    let error = decode_intra_fixture_with_core(|core| {
        let gdf = core.gdf_params.as_mut().expect("gdf params");
        gdf.gdf_frame_enable = true;
        gdf.gdf_per_block = Some(true);
    })
    .expect_err("fixture without a use_gdf symbol must fail closed");
    assert_eq!(unsupported_reason(error), "inter_exit_symbol");
}

#[test]
fn intra_frame_allows_nonzero_effective_quantizer_deltas() {
    decode_intra_fixture_with_core(|core| {
        core.quantization_params
            .as_mut()
            .expect("quantization params")
            .delta_q_y_dc = 1;
    })
    .expect("nonzero effective quantizer deltas are installed for dequant");
}
