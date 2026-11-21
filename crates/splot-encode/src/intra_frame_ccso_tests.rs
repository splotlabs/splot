// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CCSO (§ 7.19) in key and inter frames.

use super::*;
use crate::preset::PresetParams;

/// `preset` without CCSO.
fn without_ccso(preset: &PresetParams) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        ccso_candidates: 0,
        ..*preset
    }))
}

/// A key frame and two inter frames of a two-tile `chroma` pan on `threads` threads:
/// the units, the encoder recon, whether each frame codes CCSO, and whether
/// the sequence enables it.
fn ccso_stream(
    (threads, chroma): (usize, PixelFormat),
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    ccso_stream_with((threads, chroma, 170), preset, |core| {
        core.ccso_params
            .as_ref()
            .is_some_and(|params| params.ccso_frame_flag == Some(true))
    })
}

/// [`ccso_stream`] at `qp` with `flag` of each frame header.
fn ccso_stream_with(
    (threads, chroma, qp): (usize, PixelFormat, u16),
    preset: &'static PresetParams,
    flag: fn(&FrameHeaderCore) -> bool,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    let mut config = config(200, 96, BitDepth::Eight, 9, qp);
    config.chroma = chroma;
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon, mut flags) = (Vec::new(), Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let mut coded = coder
            .code_frame(&panning_frame(&config, index), &plan, qp, &[])
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        flags.push(flag(core));
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    let enabled = coder.sequence.filter.unwrap().enable_ccso;
    (units, recon, flags, enabled)
}

/// Key and inter frames code CCSO, decode to the encoder recon, and 1, 2 and
/// 8 threads write the same bytes. Without CCSO the sequence does not enable
/// it and no frame codes it.
#[test]
fn ccso_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let on = Preset::new(9).unwrap().params();
    let (one, recon, flags, enabled) = ccso_stream((1, PixelFormat::Yuv420), on);
    assert!(
        enabled && flags[0] && flags[1..].contains(&true),
        "{flags:?}"
    );
    assert_same_samples(&decode(&one.concat()), &recon, "ccso");
    for threads in [2, 8] {
        assert!(
            ccso_stream((threads, PixelFormat::Yuv420), on).0 == one,
            "{threads} threads"
        );
    }
    let (off, off_recon, off_flags, off_enabled) =
        ccso_stream((1, PixelFormat::Yuv420), without_ccso(on));
    assert!(!off_enabled && !off_flags.contains(&true), "{off_flags:?}");
    assert_same_samples(&decode(&off.concat()), &off_recon, "no ccso");
    assert_ne!(off, one);
}

/// Two-tile 4:2:2 and 4:4:4 frames code CCSO and decode to the encoder recon:
/// the tiles merge with the chroma subsampling of the stream.
#[test]
fn ccso_422_and_444_tiles_decode_to_the_encoder_recon() {
    let on = Preset::new(9).unwrap().params();
    for chroma in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let (units, recon, flags, _) = ccso_stream((2, chroma), on);
        assert!(flags.contains(&true), "{chroma:?}: {flags:?}");
        assert_same_samples(&decode(&units.concat()), &recon, &format!("{chroma:?}"));
    }
}

/// AVM decodes the CCSO key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ccso_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags, _) =
        ccso_stream((2, PixelFormat::Yuv420), Preset::new(9).unwrap().params());
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Eight, "ccso");
    assert!(avm == recon, "AVM output differs");
}

/// Preset 9 with every CCSO triple of the ladder.
fn with_full_ladder() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        ccso_candidates: 112,
        ..*Preset::new(9).unwrap().params()
    }))
}

/// Whether some plane of `core` codes CCSO with a triple outside the first 8
/// of the ladder (`ccso_ext_filter` above 3, `ccso_scale_idx` or
/// `ccso_quant_idx` above 1, or band offsets at a scale above 0).
fn new_ccso_choice(core: &FrameHeaderCore) -> bool {
    core.ccso_params.as_ref().is_some_and(|params| {
        params
            .planes
            .iter()
            .filter(|plane| plane.ccso_planes)
            .any(|plane| {
                let (ext, scale, quant) = (
                    plane.ccso_ext_filter.unwrap_or(0),
                    plane.ccso_scale_idx.unwrap_or(0),
                    plane.ccso_quant_idx.unwrap_or(0),
                );
                ext > 3 || quant > 1 || (scale > 1 || plane.ccso_bo_only == Some(true)) && scale > 0
            })
    })
}

/// With all 112 triples some frame of a qp-90 pan codes a new CCSO choice; the stream
/// decodes to the encoder recon and 1, 2 and 8 threads write the same bytes.
/// With the ladder of preset 9, no frame does.
#[test]
fn full_ccso_ladder_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let yuv = PixelFormat::Yuv420;
    let (one, recon, flags, _) =
        ccso_stream_with((1, yuv, 90), with_full_ladder(), new_ccso_choice);
    assert!(flags.contains(&true), "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "full ccso ladder");
    for threads in [2, 8] {
        assert!(
            ccso_stream_with((threads, yuv, 90), with_full_ladder(), new_ccso_choice).0 == one,
            "{threads} threads"
        );
    }
    let base = Preset::new(9).unwrap().params();
    let (_, _, off, _) = ccso_stream_with((1, yuv, 90), base, new_ccso_choice);
    assert!(!off.contains(&true), "{off:?}");
}

/// AVM decodes frames of the full CCSO ladder to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_full_ccso_ladder_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let yuv = PixelFormat::Yuv420;
    let (units, recon, flags, _) =
        ccso_stream_with((2, yuv, 90), with_full_ladder(), new_ccso_choice);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Eight, "ccso-ladder");
    assert!(avm == recon, "AVM output differs");
}
