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
    stream: (usize, PixelFormat, u16),
    preset: &'static PresetParams,
    flag: fn(&FrameHeaderCore) -> bool,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    ccso_stream_tiles(stream, 1, preset, flag)
}

/// [`ccso_stream_with`] with `1 << cols_log2` tile columns.
fn ccso_stream_tiles(
    (threads, chroma, qp): (usize, PixelFormat, u16),
    cols_log2: u8,
    preset: &'static PresetParams,
    flag: fn(&FrameHeaderCore) -> bool,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    let mut config = config(200, 96, BitDepth::Eight, 9, qp);
    config.chroma = chroma;
    config.tiles = TileLayout::Uniform {
        cols_log2,
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

/// Whether some plane of `core` codes band offsets of more than 8 bands.
fn wide_band_offsets(core: &FrameHeaderCore) -> bool {
    core.ccso_params.as_ref().is_some_and(|params| {
        params.planes.iter().any(|plane| {
            plane.ccso_bo_only == Some(true)
                && plane.ccso_max_band_log2.is_some_and(|log2| log2 > 3)
        })
    })
}

/// Band offsets of 16 to 64 bands (`ccso_bo_band_log2` 6, forced by the test
/// hook) code in key and inter frames and decode to the encoder recon. With
/// `ccso_bo_band_log2` 3 the hook finds no variant and no frame codes them.
#[test]
fn wide_band_offsets_decode_to_the_encoder_recon() {
    let wide: &'static PresetParams = Box::leak(Box::new(PresetParams {
        ccso_bo_band_log2: 6,
        ..*Preset::new(9).unwrap().params()
    }));
    let forced = |preset| {
        crate::loop_filter::ccso::FORCE_WIDE_BANDS.with(|force| force.set(true));
        let stream = ccso_stream_with((1, PixelFormat::Yuv420, 170), preset, wide_band_offsets);
        crate::loop_filter::ccso::FORCE_WIDE_BANDS.with(|force| force.set(false));
        stream
    };
    let (units, recon, flags, _) = forced(wide);
    assert!(flags.iter().all(|&flag| flag), "{flags:?}");
    assert_same_samples(&decode(&units.concat()), &recon, "wide band offsets");
    let (_, _, off, _) = forced(Preset::new(9).unwrap().params());
    assert!(!off.contains(&true), "{off:?}");
}

/// Preset 9 with CCSO reuse.
fn with_reuse() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        ccso_reuse: 1,
        ..*Preset::new(9).unwrap().params()
    }))
}

/// Whether some plane of `core` codes `reuse_ccso`.
fn reuses_ccso(core: &FrameHeaderCore) -> bool {
    core.ccso_params
        .as_ref()
        .is_some_and(|params| params.planes.iter().any(|plane| plane.reuse_ccso))
}

/// With `ccso_reuse` and the test hook that takes the cheapest reuse, the
/// inter frames reuse the saved CCSO parameters of a reference (§ 5.18.7.12)
/// and the decoder loads them; the stream decodes to the encoder recon and 1,
/// 2 and 8 threads write the same bytes. Without the hook no frame of this
/// pan or of a still source reuses at qp 110 to 220 (CCSO off or fresh
/// parameters cost less). Without the field the hook finds nothing.
#[test]
fn ccso_reuse_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let yuv = PixelFormat::Yuv420;
    let forced = |threads, preset| {
        crate::loop_filter::ccso::FORCE_REUSE.with(|force| force.set(true));
        let stream = ccso_stream_with((threads, yuv, 170), preset, reuses_ccso);
        crate::loop_filter::ccso::FORCE_REUSE.with(|force| force.set(false));
        stream
    };
    let (one, recon, flags, _) = forced(1, with_reuse());
    assert!(
        !flags[0] && flags[1..].iter().all(|&flag| flag),
        "{flags:?}"
    );
    assert_same_samples(&decode(&one.concat()), &recon, "ccso reuse");
    for threads in [2, 8] {
        assert!(forced(threads, with_reuse()).0 == one, "{threads} threads");
    }
    let (_, _, off, _) = forced(1, Preset::new(9).unwrap().params());
    assert!(!off.contains(&true), "{off:?}");
    let (_, _, natural, _) = ccso_stream_with((1, yuv, 170), with_reuse(), reuses_ccso);
    assert!(
        !natural.contains(&true),
        "a natural pick: drop the hook {natural:?}"
    );
}

/// AVM decodes frames that reuse CCSO parameters to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ccso_reuse_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    crate::loop_filter::ccso::FORCE_REUSE.with(|force| force.set(true));
    let stream = ccso_stream_with((1, PixelFormat::Yuv420, 170), with_reuse(), reuses_ccso);
    crate::loop_filter::ccso::FORCE_REUSE.with(|force| force.set(false));
    let (units, recon, flags, _) = stream;
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Eight, "ccso-reuse");
    assert!(avm == recon, "AVM output differs");
}

/// Preset 9 with CCSO reuse of parameters and `ccso_blk` flags.
fn with_block_reuse() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        ccso_reuse: 2,
        ..*Preset::new(9).unwrap().params()
    }))
}

/// Whether some plane of `core` codes `sb_reuse_ccso`.
fn sb_reuses_ccso(core: &FrameHeaderCore) -> bool {
    core.ccso_params
        .as_ref()
        .is_some_and(|params| params.planes.iter().any(|plane| plane.sb_reuse_ccso))
}

/// `ccso_reuse` 2 on a one-tile pan at qp 140 (`CcsoLumaSizeLog2` 8,
/// § 6.17.7.8): both inter frames pick `sb_reuse_ccso` by RD, so they load
/// the saved `ccso_blk` flags of a reference and code none (§ 5.20.10.2), and
/// the second loads flags the first reused. The stream decodes to the encoder
/// recon and 1, 2 and 8 threads write the same bytes. With `ccso_reuse` 1, or
/// two tiles (`CcsoLumaSizeLog2` 7), no frame codes it.
#[test]
fn ccso_block_reuse_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let stream = |threads, cols_log2, preset| {
        ccso_stream_tiles(
            (threads, PixelFormat::Yuv420, 140),
            cols_log2,
            preset,
            sb_reuses_ccso,
        )
    };
    let (one, recon, flags, _) = stream(1, 0, with_block_reuse());
    assert!(
        !flags[0] && flags[1..].iter().all(|&flag| flag),
        "{flags:?}"
    );
    assert_same_samples(&decode(&one.concat()), &recon, "ccso block reuse");
    for threads in [2, 8] {
        assert!(
            stream(threads, 0, with_block_reuse()).0 == one,
            "{threads} threads"
        );
    }
    for (cols_log2, preset) in [(0, with_reuse()), (1, with_block_reuse())] {
        let (_, _, off, _) = stream(1, cols_log2, preset);
        assert!(!off.contains(&true), "{cols_log2}: {off:?}");
    }
}

/// AVM decodes frames that reuse CCSO `ccso_blk` flags to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ccso_block_reuse_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags, _) = ccso_stream_tiles(
        (1, PixelFormat::Yuv420, 140),
        0,
        with_block_reuse(),
        sb_reuses_ccso,
    );
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(
        &avmdec,
        &units,
        (200, 96),
        BitDepth::Eight,
        "ccso-block-reuse",
    );
    assert!(avm == recon, "AVM output differs");
}

/// The facade keeps the `ccso_params()` of each reference slot as § 7.23
/// saves them: after a key frame slot 0 holds the key frame's parameters and
/// the other slots hold none (only slot 0 is valid, § 7.20), a frame that
/// refreshes slots overwrites exactly those slots, and a frame that refreshes
/// none and a show-existing frame leave every slot as it was.
#[test]
fn facade_saves_the_ccso_parameters_of_each_slot() {
    let mut config = config(200, 96, BitDepth::Eight, 9, 170);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    let mut coder = IntraFrameCoder::with_preset(&config, with_reuse()).unwrap();
    let mut planner = crate::gop::GopPlanner::new(config.keyint, coder.preset.mini_gop_size.min(4));
    let saved = |coder: &IntraFrameCoder| -> Vec<Option<splot_core::headers::frame::CcsoParams>> {
        (0..8)
            .map(|slot| coder.references.ccso_params(slot).cloned())
            .collect()
    };
    let (mut queued, mut cases) = (9, [0usize; 4]);
    while let Some(len) = planner.next_len(queued, true) {
        queued -= len;
        for step in planner.plan(len).unwrap() {
            let before = saved(&coder);
            match step {
                crate::gop::GopStep::Code(plan) => {
                    let source = panning_frame(&config, plan.display_index as u32);
                    let mut coded = coder.code_frame(&source, &plan, 170, &[]).unwrap();
                    let core = coded.reference.as_ref().unwrap().0.ccso_params.clone();
                    coder.commit(&mut coded).unwrap();
                    let after = saved(&coder);
                    for slot in 0..8 {
                        let refreshed = plan.refresh_frame_flags >> slot & 1 == 1;
                        let expected = match (plan.frame_type, refreshed) {
                            (FrameType::Key, _) if slot > 0 => &None,
                            (_, true) => &core,
                            (_, false) => &before[slot],
                        };
                        assert_eq!(&after[slot], expected, "slot {slot} of {plan:?}");
                    }
                    let kind = match (plan.frame_type, plan.refresh_frame_flags) {
                        (FrameType::Key, _) => 0,
                        (_, 0) => 1,
                        _ => 2,
                    };
                    cases[kind] += 1;
                }
                crate::gop::GopStep::ShowExisting { slot, .. } => {
                    coder.show_existing_frame(slot, None, false).unwrap();
                    assert_eq!(saved(&coder), before, "show-existing slot {slot}");
                    cases[3] += 1;
                }
            }
        }
    }
    assert!(
        cases.iter().all(|&count| count > 0),
        "key, no refresh, refresh, show-existing: {cases:?}"
    );
}
