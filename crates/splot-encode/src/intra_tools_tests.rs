// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra coding tools in coded streams: MRL (§ 7.13.2), IST (§ 7.15.3), CCTX
//! (§ 7.14.6), MHCCP (§ 5.20.7.32), FSC (§ 5.20.7.27) and the CfL
//! down-sampling filter (§ 7.13.5).

use super::*;

/// A key frame and two inter frames of a two-tile `chroma` pan at `bit_depth`
/// on `threads` threads: the units, the encoder recon, and the sequence.
fn tool_stream(
    case: (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader) {
    cfl_stream(case, preset, 0)
}

/// [`tool_stream`] with `cfl_ds_filter_index` `cfl_filter`.
fn cfl_stream(
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
    cfl_filter: u8,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader) {
    let mut config = config(136, 72, bit_depth, 9, 120);
    config.chroma = chroma;
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    coder.set_cfl_filter(cfl_filter).unwrap();
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let coded = coder
            .encode_frame(&panning_frame(&config, index), &plan, 120)
            .unwrap();
        recon.extend(recon_bytes(&coded, bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, coder.sequence)
}

/// Every `cfl_ds_filter_index` reaches the sequence header, and its CfL and
/// chroma Wiener-NS samples decode to the encoder recon in every chroma
/// format and both bit depths.
#[test]
fn cfl_filters_decode_to_the_encoder_recon() {
    for chroma in CHROMAS {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            for index in 1..3 {
                let (units, recon, seq) = cfl_stream((2, chroma, bit_depth), &PRESETS[9], index);
                assert_eq!(seq.intra.unwrap().cfl_ds_filter_index, index);
                let name = format!("{chroma:?} {bit_depth:?} filter {index}");
                assert_same_samples(&decode(&units.concat()), &recon, &name);
            }
        }
    }
}

/// AVM decodes the CfL and chroma Wiener-NS samples of every filter like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cfl_filters_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, bit_depth) in [(1, BitDepth::Eight), (2, BitDepth::Ten)] {
        let (units, recon, _) = cfl_stream((2, PixelFormat::Yuv420, bit_depth), &PRESETS[9], index);
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, "cfl filter");
        assert!(avm == recon, "filter {index}: AVM output differs");
    }
}

/// Preset 9, fast enough for debug tests, with `set` applied.
fn preset_with(set: fn(&mut PresetParams)) -> &'static PresetParams {
    let mut preset = PRESETS[9];
    set(&mut preset);
    Box::leak(Box::new(preset))
}

/// Preset 9 with the MRL trials of preset 6.
fn mrl_preset() -> &'static PresetParams {
    preset_with(|p| p.mrl_trials = PRESETS[6].mrl_trials)
}

/// Preset 9 with every IST set.
fn ist_preset() -> &'static PresetParams {
    preset_with(|p| p.ist_sets = splot_decode::encoder_syntax::IST_SETS)
}

/// Streams of `on` in every chroma format and both bit depths decode to the
/// encoder recon with `enabled` set, and 1, 2 and 8 threads write the same
/// bytes; `off` clears `enabled` and changes the stream.
fn assert_tool_streams(
    on: &'static PresetParams,
    off: &'static PresetParams,
    enabled: fn(&SequenceHeader) -> bool,
) {
    for chroma in CHROMAS {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon, seq) = tool_stream((2, chroma, bit_depth), on);
            assert!(enabled(&seq));
            let name = format!("{chroma:?} {bit_depth:?}");
            assert_same_samples(&decode(&units.concat()), &recon, &name);
        }
    }
    let case = (1, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, ..) = tool_stream(case, on);
    for threads in [2, 8] {
        assert!(
            tool_stream((threads, case.1, case.2), on).0 == one,
            "{threads} threads"
        );
    }
    let (units, recon, seq) = tool_stream(case, off);
    assert!(!enabled(&seq));
    assert_same_samples(&decode(&units.concat()), &recon, "off");
    assert_ne!(units, one);
}

/// AVM decodes the key and inter frames of `preset` to the encoder recon.
fn assert_avm_decodes(preset: &'static PresetParams, name: &str) {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (units, recon, _) = tool_stream((2, PixelFormat::Yuv420, bit_depth), preset);
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, name);
        assert!(avm == recon, "{name} {bit_depth:?}: AVM output differs");
    }
}

/// MRL streams decode to the encoder recon on any thread count.
#[test]
fn mrl_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(mrl_preset(), preset_with(|p| p.mrl_trials = 0), |seq| {
        seq.intra.unwrap().enable_mrls
    });
}

/// The key-frame search codes leaves on every MRL line.
#[test]
fn search_picks_mrl_lines() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, mrl_preset()).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    for index in 1..=3 {
        let line = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.mrl.index == index;
        let count: usize = trees.iter().map(|tree| leaves(tree, line)).sum();
        assert!(count > 0, "no leaf on MRL line {index}");
    }
}

/// AVM decodes the MRL key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_mrl_frames_like_splot() {
    assert_avm_decodes(mrl_preset(), "mrl");
}

/// IST streams decode to the encoder recon on any thread count.
#[test]
fn ist_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(ist_preset(), preset_with(|p| p.ist_sets = 0), |seq| {
        seq.transform_quant_entropy.unwrap().enable_intra_ist
    });
}

/// CCTX streams decode to the encoder recon on any thread count.
#[test]
fn cctx_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(preset_with(|p| p.cctx = true), preset_with(|_| {}), |seq| {
        seq.transform_quant_entropy.unwrap().enable_cctx
    });
}

/// MHCCP streams decode to the encoder recon on any thread count.
#[test]
fn mhccp_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.mhccp = true),
        preset_with(|_| {}),
        |seq| seq.intra.unwrap().enable_mhccp,
    );
}

/// FSC streams decode to the encoder recon on any thread count.
#[test]
fn fsc_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    assert_tool_streams(
        preset_with(|p| p.fsc = true),
        preset_with(|p| p.fsc = false),
        |seq| seq.transform_quant_entropy.unwrap().enable_fsc,
    );
}

/// The key-frame search codes FSC leaves.
#[test]
fn search_picks_fsc_leaves() {
    let config = config(136, 72, BitDepth::Eight, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, preset_with(|p| p.fsc = true)).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let trees = search_frame(
        &coder,
        &plan,
        &synthetic_frame(&config, 3),
        config.bit_depth,
    );
    let fsc = |choice: &LeafChoice| choice.inter.is_none() && choice.modes.fsc;
    let count: usize = trees.iter().map(|tree| leaves(tree, fsc)).sum();
    assert!(count > 0);
}

/// AVM decodes the FSC key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_fsc_frames_like_splot() {
    assert_avm_decodes(preset_with(|p| p.fsc = true), "fsc");
}

/// The key-frame search codes CCTX pairs and MHCCP leaves.
#[test]
fn search_picks_cctx_pairs_and_mhccp_leaves() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let preset = preset_with(|p| (p.cctx, p.mhccp) = (true, true));
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let source = wave_frame(&config);
    let (trees, [_, cctx]) = search_frame_counting(&coder, &plan, &source, config.bit_depth);
    let mhccp = |choice: &LeafChoice| {
        choice.inter.is_none()
            && matches!(
                choice.modes.uv_mode,
                splot_decode::encoder_syntax::ChromaMode::Mhccp { .. }
            )
    };
    let leaves: usize = trees.iter().map(|tree| leaves(tree, mhccp)).sum();
    assert!(cctx > 0 && leaves > 0, "cctx {cctx} mhccp {leaves}");
}

/// AVM decodes CCTX and MHCCP key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cctx_and_mhccp_frames_like_splot() {
    assert_avm_decodes(
        preset_with(|p| (p.cctx, p.mhccp) = (true, true)),
        "cctx-mhccp",
    );
}

/// An oblique sine wave: directional residual that IST compacts.
fn wave_frame(config: &Config) -> SourceFrame {
    let max = f64::from(config.bit_depth.max_sample());
    let planes = config.plane_sizes().unwrap().map(|size| {
        (0..size.height())
            .flat_map(|y| (0..size.width()).map(move |x| (x as f64, y as f64)))
            .map(|(x, y)| (max * (0.5 + 0.3 * ((x * 0.8 + y * 0.6) / 7.0).sin())) as u16)
            .collect()
    });
    SourceFrame::new(config, 0, planes).unwrap()
}

/// The key-frame search codes luma blocks with a secondary transform.
#[test]
fn search_picks_ist_blocks() {
    let config = config(64, 64, BitDepth::Eight, 9, 90);
    let coder = IntraFrameCoder::with_preset(&config, ist_preset()).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let (_, [ist, _]) =
        search_frame_counting(&coder, &plan, &wave_frame(&config), config.bit_depth);
    assert!(ist > 0);
}

/// AVM decodes the IST key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ist_frames_like_splot() {
    assert_avm_decodes(ist_preset(), "ist");
}
