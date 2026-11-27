// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IntraBC in inter frames (§ 5.18.3.4: the local area only; § 5.20.7.3
//! `use_intrabc` after `is_inter = 0`), with [`PresetParams::intrabc_inter`],
//! and in `intra_only` frames, which follow the key-frame rules
//! (`FrameIsIntra`).

use super::intra_tools::tiled_text;
use super::*;
use crate::preset::PresetParams;
use splot_core::headers::frame::{FrameHeaderCore, FrameType as CoreFrameType};

/// Preset 9 with IntraBC in inter frames and SDP off, and IntraBC off in key
/// frames: each IntraBC leaf lies in an inter frame.
fn inter_intrabc_preset() -> &'static PresetParams {
    let mut preset = *Preset::new(9).unwrap().params();
    for (field, value) in [("intrabc_local", "1"), ("intrabc_inter", "1"), ("sdp", "0")] {
        preset.set(field, value).unwrap();
    }
    preset.validate().unwrap();
    Box::leak(Box::new(preset))
}

/// A mini-GOP [`tiled_text`] stream with IntraBC in its inter frames, with
/// IntraBC the only candidate where it has one when `forced`: the units, the
/// recon, the headers and the IntraBC leaves the final passes coded.
fn inter_intrabc_stream(
    case: (usize, PixelFormat, BitDepth),
    forced: bool,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    intrabc_stream(inter_intrabc_preset(), case, forced, (tiled_text, 5))
}

/// The `frames` frames of `source` coded with `preset` in mini-GOPs; see
/// [`inter_intrabc_stream`].
fn intrabc_stream(
    preset: &'static PresetParams,
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    forced: bool,
    (source, frames): (fn(&Config, u32) -> SourceFrame, usize),
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    let mut config = config(128, 64, bit_depth, 9, 120);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    crate::mode_decision::FORCE_INTRABC.set(forced);
    crate::tile_writer::CODED_INTRABC.set(0);
    let (units, recon, cores, _) =
        super::carry::planned_stream_frames(&config, coder, source, 120, frames);
    crate::mode_decision::FORCE_INTRABC.set(false);
    (units, recon, cores, crate::tile_writer::CODED_INTRABC.get())
}

/// The inter frames code `allow_intrabc = 1`, the forced streams code
/// IntraBC leaves in them, a later frame codes `use_ref_frame_mvs = 1`, and
/// the streams decode to the encoder recon in 4:2:0 8-bit and 4:4:4 10-bit.
/// Without the forcing, the RD search codes IntraBC leaves in them too, and
/// 1, 2 and 8 threads write the same bytes (the forcing hook is
/// thread-local: this test sets it on its own thread).
#[test]
fn inter_frames_code_intrabc_leaves_that_decode_to_the_recon() {
    for case in [
        (1, PixelFormat::Yuv420, BitDepth::Eight),
        (1, PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let (units, recon, cores, coded) = inter_intrabc_stream(case, true);
        let inter: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .collect();
        assert!(inter.iter().any(|inter| inter.allow_intrabc == Some(true)));
        assert!(
            inter.iter().all(|inter| inter.allow_intrabc != Some(false)),
            "{case:?}"
        );
        assert!(
            inter
                .iter()
                .any(|inter| inter.use_ref_frame_mvs == Some(true))
        );
        assert!(coded > 0, "{case:?}");
        assert_same_samples(&decode(&units), &recon, &format!("{case:?}"));
    }
    let case = |threads| (threads, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, recon, _, coded) = inter_intrabc_stream(case(1), false);
    assert!(coded > 0, "unforced");
    assert_same_samples(&decode(&one), &recon, "unforced");
    for threads in [2, 8] {
        assert!(
            inter_intrabc_stream(case(threads), false).0 == one,
            "{threads} threads"
        );
    }
}

/// AVM decodes the forced streams of
/// [`inter_frames_code_intrabc_leaves_that_decode_to_the_recon`] to the
/// encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_inter_frame_intrabc_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let (units, recon, ..) = inter_intrabc_stream((1, chroma, bit_depth), true);
        let name = format!("inter-intrabc-{chroma:?}-{bit_depth:?}");
        let avm = avm_decode(&avmdec, &[units], (128, 64), bit_depth, &name);
        assert!(avm == recon, "{name}: AVM output differs");
    }
}

/// Preset 9 with IntraBC and its local area, SDP off and `intra_only = n`.
fn intra_only_intrabc_preset(n: &str) -> &'static PresetParams {
    let mut preset = *Preset::new(9).unwrap().params();
    for (field, value) in [
        ("intrabc", "1"),
        ("intrabc_local", "1"),
        ("sdp", "0"),
        ("intra_only", n),
    ] {
        preset.set(field, value).unwrap();
    }
    preset.validate().unwrap();
    Box::leak(Box::new(preset))
}

/// [`tiled_text`] with noise on the key frame, so that the block hashes find
/// no copy there.
fn noisy_key_text(config: &Config, index: u32) -> SourceFrame {
    let text = tiled_text(config, index);
    match index {
        0 => super::grain_streams::with_noise(config, &text, 1),
        _ => text,
    }
}

/// The 9 frames of [`noisy_key_text`] with `intra_only = n`.
fn intra_only_intrabc_stream(
    n: &str,
    case: (usize, PixelFormat, BitDepth),
    forced: bool,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    intrabc_stream(
        intra_only_intrabc_preset(n),
        case,
        forced,
        (noisy_key_text, 9),
    )
}

/// `intra_only` frames code `allow_intrabc = 1` with the local area as key
/// frames do (§ 5.18.3.4 with `FrameIsIntra = 1`), and IntraBC leaves in
/// them: the noisy key frame codes none, and the inter frames do not code
/// `allow_intrabc`. The streams decode to the encoder recon in 4:2:0 8-bit
/// and 4:4:4 10-bit, and unforced on 1, 2 and 8 threads with the same bytes.
#[test]
fn intra_only_frames_code_intrabc_leaves_that_decode_to_the_recon() {
    let case = (1, PixelFormat::Yuv420, BitDepth::Eight);
    assert_eq!(intra_only_intrabc_stream("0", case, true).3, 0, "key frame");
    for case in [case, (1, PixelFormat::Yuv444, BitDepth::Ten)] {
        let (units, recon, cores, coded) = intra_only_intrabc_stream("1", case, true);
        let intra_only: Vec<_> = cores
            .iter()
            .filter(|core| core.frame_type == Some(CoreFrameType::IntraOnly))
            .collect();
        assert!(intra_only.len() >= 2, "{case:?}");
        assert!(
            intra_only.iter().all(|core| core
                .intrabc
                .is_some_and(|ibc| ibc.allow_local_intrabc == Some(true))),
            "{case:?}"
        );
        assert!(
            cores
                .iter()
                .filter_map(|core| core.inter.as_ref())
                .all(|inter| inter.allow_intrabc != Some(true))
        );
        assert!(coded > 0, "{case:?}");
        assert_same_samples(&decode(&units), &recon, &format!("{case:?}"));
    }
    let case = |threads| (threads, PixelFormat::Yuv420, BitDepth::Ten);
    let (one, recon, _, coded) = intra_only_intrabc_stream("1", case(1), false);
    assert!(coded > 0, "unforced");
    assert_same_samples(&decode(&one), &recon, "unforced");
    for threads in [2, 8] {
        assert!(
            intra_only_intrabc_stream("1", case(threads), false).0 == one,
            "{threads} threads"
        );
    }
}

/// AVM decodes the forced streams of
/// [`intra_only_frames_code_intrabc_leaves_that_decode_to_the_recon`] to the
/// encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_intra_only_intrabc_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let (units, recon, ..) = intra_only_intrabc_stream("1", (1, chroma, bit_depth), true);
        let name = format!("intra-only-intrabc-{chroma:?}-{bit_depth:?}");
        let avm = avm_decode(&avmdec, &[units], (128, 64), bit_depth, &name);
        assert!(avm == recon, "{name}: AVM output differs");
    }
}
