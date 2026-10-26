// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDEF on skipped transform blocks (§ 5.18.7.10) in key and inter frames.

use super::*;
use crate::preset::PresetParams;

/// `preset` with the CDEF-on-skip search `on`.
fn with_cdef_on_skip(preset: &PresetParams, on: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        cdef_on_skip: on,
        ..*preset
    }))
}

/// A key frame and three inter frames of a two-tile pan on `threads`
/// threads: the units, the encoder recon and `cdef_on_skip_txfm_frame_enable`
/// of each frame.
fn cdef_skip_stream(
    threads: usize,
    preset: &'static PresetParams,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>) {
    let mut config = config(200, 136, BitDepth::Eight, 9, 200);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon, mut flags) = (Vec::new(), Vec::new(), Vec::new());
    for index in 0..4 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let mut coded = coder
            .code_frame(&panning_frame(&config, index), &plan, 200, &[])
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        flags.push(
            core.cdef_params
                .as_ref()
                .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(true)),
        );
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, flags)
}

/// Some frame codes CDEF on skipped blocks; the stream decodes to the encoder
/// recon and 1, 2 and 8 threads write the same bytes. Off, no frame codes it.
#[test]
fn cdef_on_skip_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let base = Preset::new(9).unwrap().params();
    let on = with_cdef_on_skip(base, true);
    let (one, recon, flags) = cdef_skip_stream(1, on);
    assert!(flags.contains(&true), "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "cdef on skip");
    for threads in [2, 8] {
        assert!(cdef_skip_stream(threads, on).0 == one, "{threads} threads");
    }
    let (_, _, off_flags) = cdef_skip_stream(1, with_cdef_on_skip(base, false));
    assert!(!off_flags.contains(&true), "{off_flags:?}");
}

/// AVM decodes frames with CDEF on skipped blocks to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cdef_on_skip_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let on = with_cdef_on_skip(Preset::new(9).unwrap().params(), true);
    let (units, recon, flags) = cdef_skip_stream(2, on);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Eight, "cdef-skip");
    assert!(avm == recon, "AVM output differs");
}
