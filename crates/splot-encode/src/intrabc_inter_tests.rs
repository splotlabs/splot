// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IntraBC in inter frames (§ 5.18.3.4: the local area only; § 5.20.7.3
//! `use_intrabc` after `is_inter = 0`), behind the test hook
//! [`crate::gop::INTER_INTRABC`].

use super::intra_tools::tiled_text;
use super::*;
use crate::preset::PresetParams;
use splot_core::headers::frame::FrameHeaderCore;

/// Preset 9 with the IntraBC local area and SDP off, and IntraBC off in key
/// frames: each IntraBC leaf lies in an inter frame.
fn inter_intrabc_preset() -> &'static PresetParams {
    let mut preset = *Preset::new(9).unwrap().params();
    (preset.intrabc, preset.intrabc_local, preset.sdp) = (false, true, false);
    Box::leak(Box::new(preset))
}

/// A mini-GOP [`tiled_text`] stream with IntraBC in its inter frames, with
/// IntraBC the only candidate where it has one when `forced`: the units, the
/// recon, the headers and the IntraBC leaves the final passes coded.
pub(super) fn inter_intrabc_stream(
    (threads, chroma, bit_depth): (usize, PixelFormat, BitDepth),
    forced: bool,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    let mut config = config(128, 64, bit_depth, 9, 120);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, inter_intrabc_preset()).unwrap();
    crate::gop::INTER_INTRABC.set(true);
    crate::mode_decision::FORCE_INTRABC.set(forced);
    crate::tile_writer::CODED_INTRABC.set(0);
    let (units, recon, cores, _) = super::carry::planned_stream_of(&config, coder, tiled_text, 120);
    crate::gop::INTER_INTRABC.set(false);
    crate::mode_decision::FORCE_INTRABC.set(false);
    (units, recon, cores, crate::tile_writer::CODED_INTRABC.get())
}

/// The inter frames code `allow_intrabc = 1`, the forced streams code
/// IntraBC leaves in them, a later frame codes `use_ref_frame_mvs = 1`, and
/// the streams decode to the encoder recon in 4:2:0 8-bit and 4:4:4 10-bit.
/// Without the forcing, 1, 2 and 8 threads write the same bytes (the hooks
/// are thread-local: this test sets them on its own thread).
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
    let (one, recon, ..) = inter_intrabc_stream(case(1), false);
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
