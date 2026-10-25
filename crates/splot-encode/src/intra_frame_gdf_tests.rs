// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! GDF (§ 7.20.5) in key and inter frames.

use super::*;
use crate::preset::PresetParams;

/// `preset` with `candidates` GDF candidates.
fn with_gdf(preset: &PresetParams, candidates: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        gdf_candidates: candidates,
        ..*preset
    }))
}

/// A key frame and two inter frames of a 2x2-tile pan on `threads` threads:
/// the units, the encoder recon, whether each frame codes GDF, and whether
/// the sequence enables it.
fn gdf_stream(
    threads: usize,
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>, bool) {
    let mut config = config(200, 136, BitDepth::Ten, 9, 200);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon, mut flags) = (Vec::new(), Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let mut coded = coder
            .code_frame(&panning_frame(&config, index), &plan, 200)
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        flags.push(core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable));
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    let enabled = coder.sequence.filter.unwrap().enable_gdf;
    (units, recon, flags, enabled)
}

/// Key and inter frames code GDF, decode to the encoder recon, and 1, 2 and
/// 8 threads write the same bytes. Without GDF the sequence does not enable
/// it and the stream differs.
#[test]
fn gdf_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let on = Preset::new(9).unwrap().params();
    let (one, recon, flags, enabled) = gdf_stream(1, on);
    assert!(
        enabled && flags[0] && flags[1..].contains(&true),
        "{flags:?}"
    );
    assert_same_samples(&decode(&one.concat()), &recon, "gdf");
    for threads in [2, 8] {
        assert!(gdf_stream(threads, on).0 == one, "{threads} threads");
    }
    let (off, off_recon, off_flags, off_enabled) = gdf_stream(1, with_gdf(on, 0));
    assert!(!off_enabled && !off_flags.contains(&true), "{off_flags:?}");
    assert_same_samples(&decode(&off.concat()), &off_recon, "no gdf");
    assert_ne!(off, one);
}

/// AVM decodes GDF key and inter frames to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_gdf_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags, _) = gdf_stream(2, Preset::new(9).unwrap().params());
    assert!(flags.iter().all(|&on| on), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Ten, "gdf");
    assert!(avm == recon, "AVM output differs");
}
