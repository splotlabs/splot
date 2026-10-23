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

/// A key frame and two inter frames of a two-tile pan on `threads` threads:
/// the units, the encoder recon, whether each frame codes CCSO, and whether
/// the sequence enables it.
fn ccso_stream(
    threads: usize,
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    let mut config = config(200, 96, BitDepth::Eight, 9, 170);
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
            .code_frame(&panning_frame(&config, index), &plan, 170)
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        flags.push(
            core.ccso_params
                .as_ref()
                .is_some_and(|params| params.ccso_frame_flag == Some(true)),
        );
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
    let (one, recon, flags, enabled) = ccso_stream(1, on);
    assert!(
        enabled && flags[0] && flags[1..].contains(&true),
        "{flags:?}"
    );
    assert_same_samples(&decode(&one.concat()), &recon, "ccso");
    for threads in [2, 8] {
        assert!(ccso_stream(threads, on).0 == one, "{threads} threads");
    }
    let (off, off_recon, off_flags, off_enabled) = ccso_stream(1, without_ccso(on));
    assert!(!off_enabled && !off_flags.contains(&true), "{off_flags:?}");
    assert_same_samples(&decode(&off.concat()), &off_recon, "no ccso");
    assert_ne!(off, one);
}

/// AVM decodes the CCSO key and inter frames to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_ccso_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags, _) = ccso_stream(2, Preset::new(9).unwrap().params());
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 96), BitDepth::Eight, "ccso");
    assert!(avm == recon, "AVM output differs");
}
