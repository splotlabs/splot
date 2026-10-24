// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra coding tools in coded streams: MRL (§ 7.13.2).

use super::*;

/// A key frame and two inter frames of a two-tile `chroma` pan at `bit_depth`
/// on `threads` threads: the units, the encoder recon, and the sequence.
fn tool_stream(
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, SequenceHeader) {
    let mut config = config(136, 72, bit_depth, 9, 120);
    config.chroma = chroma;
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
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

/// Whether `seq` enables MRL.
fn mrl_enabled(seq: &SequenceHeader) -> bool {
    seq.intra.unwrap().enable_mrls
}

/// MRL streams of every chroma format and both bit depths decode to the
/// encoder recon, and 1, 2 and 8 threads write the same bytes. Without MRL
/// the sequence does not enable it and the stream changes.
#[test]
fn mrl_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let on = Preset::new(9).unwrap().params();
    for chroma in CHROMAS {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let (units, recon, seq) = tool_stream((2, chroma, bit_depth), on);
            assert!(mrl_enabled(&seq));
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
    let off = Box::leak(Box::new(PresetParams {
        mrl_trials: 0,
        ..*on
    }));
    let (units, recon, seq) = tool_stream(case, off);
    assert!(!mrl_enabled(&seq));
    assert_same_samples(&decode(&units.concat()), &recon, "no mrl");
    assert_ne!(units, one);
}

/// The key-frame search codes leaves on every MRL line.
#[test]
fn search_picks_mrl_lines() {
    let config = config(136, 72, BitDepth::Ten, 9, 90);
    let coder = IntraFrameCoder::new(&config).unwrap();
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
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (units, recon, _) = tool_stream(
            (2, PixelFormat::Yuv420, bit_depth),
            Preset::new(9).unwrap().params(),
        );
        let avm = avm_decode(&avmdec, &units, (136, 72), bit_depth, "mrl");
        assert!(avm == recon, "{bit_depth:?}: AVM output differs");
    }
}
