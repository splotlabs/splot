// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Optical flow and vector refinement (§ 5.18.3.2, § 5.20.7.6 `use_optflow`,
//! § 5.20.7.17 `use_refinemv`) in coded streams. Random refined leaves are
//! covered by `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use crate::preset::PresetParams;

/// Preset 6 with optical flow and vector refinement.
fn with_refinement() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        opfl: true,
        refinemv: true,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A panning mini-GOP stream with refinement at qp 200, where the encoder
/// deblocks.
fn refined_stream(config: &Config) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let coder = IntraFrameCoder::with_preset(config, with_refinement()).unwrap();
    super::carry::planned_stream_of(config, coder, panning_frame, 200)
}

/// Mini-GOP streams with refinement code `REFINE_SWITCHABLE` frames whose
/// `NEAR_NEARMV` blocks refine their vectors, with the loop filters the
/// encoder picks and later frames predicting from the refined motion field,
/// and decode to the encoder recon in 4:2:0 10-bit, 4:2:2 8-bit and 4:4:4
/// 10-bit.
#[test]
fn refined_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 40, bit_depth, 6, 200);
        config.chroma = chroma;
        let (units, expected, cores, coder) = refined_stream(&config);
        let inter = coder.sequence.inter.unwrap();
        assert!(inter.enable_refinemv && inter.enable_opfl_refine == 3);
        let switchable = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .filter(|inter| inter.opfl_refine_type == Some(1))
            .count();
        assert!(switchable > 0, "{chroma:?}");
        assert!(
            cores.iter().any(|core| core
                .deblocking_filter_params
                .as_ref()
                .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))),
            "{chroma:?}: no deblocked frame"
        );
        assert_same_samples(&decode(&units), &expected, &format!("{chroma:?}"));
    }
}

/// A tiled refined stream codes the same bytes on 1, 2 and 8 threads and
/// decodes to the encoder recon.
#[test]
fn refined_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 200);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        refined_stream(&config)
    };
    let (one, expected, ..) = stream(1);
    assert_same_samples(&decode(&one), &expected, "tiled");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// AVM decodes refined panning streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_refined_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for chroma in [
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        let mut config = config(72, 40, BitDepth::Ten, 6, 200);
        config.chroma = chroma;
        let (units, expected, ..) = refined_stream(&config);
        let avm = avm_decode(
            &avmdec,
            &[units],
            (config.width, config.height),
            config.bit_depth,
            &format!("refined{chroma:?}"),
        );
        assert!(avm == expected, "{chroma:?}: AVM output differs");
    }
}
