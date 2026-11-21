// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDEF on skipped transform blocks and the full CDEF strength and damping
//! search (§ 5.18.7.10), the luma loop-restoration unit size search
//! (§ 5.18.7.11), and deblocking of prediction sub-units (§ 5.18.5.2
//! `allow_df_sub_pu`) in key and inter frames.

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
/// threads: the units, the encoder recon and `flag` of each frame header.
fn filter_stream(
    threads: usize,
    preset: &'static PresetParams,
    flag: fn(&FrameHeaderCore) -> bool,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>) {
    tiled_filter_stream(threads, preset, flag, 1)
}

/// [`filter_stream`] in `1 << cols_log2` tile columns.
fn tiled_filter_stream(
    threads: usize,
    preset: &'static PresetParams,
    flag: fn(&FrameHeaderCore) -> bool,
    cols_log2: u8,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>) {
    let mut config = config(200, 136, BitDepth::Eight, 9, 200);
    config.tiles = TileLayout {
        cols_log2,
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
        flags.push(flag(core));
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, flags)
}

/// `cdef_on_skip_txfm_frame_enable` of `core`.
fn on_skip(core: &FrameHeaderCore) -> bool {
    core.cdef_params
        .as_ref()
        .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(true))
}

/// Some frame codes CDEF on skipped blocks; the stream decodes to the encoder
/// recon and 1, 2 and 8 threads write the same bytes. Off, no frame codes it.
#[test]
fn cdef_on_skip_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let base = Preset::new(9).unwrap().params();
    let on = with_cdef_on_skip(base, true);
    let (one, recon, flags) = filter_stream(1, on, on_skip);
    assert!(flags.contains(&true), "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "cdef on skip");
    for threads in [2, 8] {
        assert!(
            filter_stream(threads, on, on_skip).0 == one,
            "{threads} threads"
        );
    }
    let (_, _, off_flags) = filter_stream(1, with_cdef_on_skip(base, false), on_skip);
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
    let (units, recon, flags) = filter_stream(2, on, on_skip);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Eight, "cdef-skip");
    assert!(avm == recon, "AVM output differs");
}

/// Preset 6 with `df_sub_pu`.
fn with_df_sub_pu() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        df_sub_pu: true,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// `allow_df_sub_pu` of `core`.
fn sub_pu(core: &FrameHeaderCore) -> bool {
    core.deblocking_filter_params
        .as_ref()
        .is_some_and(|params| params.allow_df_sub_pu)
}

/// With `df_sub_pu` some inter frame deblocks prediction sub-units; the
/// stream decodes to the encoder recon and 1, 2 and 8 threads write the same
/// bytes. Off, the sequence does not enable it.
#[test]
fn df_sub_pu_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let (one, recon, flags) = filter_stream(1, with_df_sub_pu(), sub_pu);
    assert!(flags.contains(&true), "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "df sub-pu");
    for threads in [2, 8] {
        assert!(
            filter_stream(threads, with_df_sub_pu(), sub_pu).0 == one,
            "{threads} threads"
        );
    }
    let (_, _, off) = filter_stream(1, Preset::new(6).unwrap().params(), sub_pu);
    assert!(!off.contains(&true), "{off:?}");
}

/// AVM decodes frames that deblock prediction sub-units to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_df_sub_pu_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags) = filter_stream(2, with_df_sub_pu(), sub_pu);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Eight, "df-sub-pu");
    assert!(avm == recon, "AVM output differs");
}

/// Preset 9 with the full CDEF search: all 64 strength pairs and every damping.
fn with_full_cdef() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        cdef_pairs: 64,
        cdef_damping_search: true,
        ..*Preset::new(9).unwrap().params()
    }))
}

/// Whether `core` codes CDEF with a strength pair outside the 16-pair ladder
/// or with a damping other than the one its qindex derives.
fn new_cdef_choice(core: &FrameHeaderCore) -> bool {
    let Some(cdef) = core
        .cdef_params
        .as_ref()
        .filter(|cdef| cdef.cdef_frame_enable)
    else {
        return false;
    };
    let qindex = core.quantization_params.map_or(0, |q| q.base_q_idx);
    let derived = crate::loop_filter::cdef_damping(qindex, BitDepth::Eight);
    let ladder = |pair| crate::loop_filter::CDEF_LADDER.contains(&pair);
    cdef.cdef_damping != Some(derived)
        || cdef.strengths.as_slice().iter().any(|set| {
            !ladder((set.y_pri_strength, set.y_sec_strength))
                || !ladder((set.uv_pri_strength, set.uv_sec_strength))
        })
}

/// With `cdef_pairs` 64 and `cdef_damping_search` some frame codes a pair
/// outside the 16-pair ladder or a non-derived damping; the stream decodes to
/// the encoder recon and 1, 2 and 8 threads write the same bytes. Off, no
/// frame does.
#[test]
fn full_cdef_search_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let (one, recon, flags) = filter_stream(1, with_full_cdef(), new_cdef_choice);
    let chosen = flags.iter().filter(|&&flag| flag).count();
    assert!(chosen > 0, "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "full cdef");
    for threads in [2, 8] {
        assert!(
            filter_stream(threads, with_full_cdef(), new_cdef_choice).0 == one,
            "{threads} threads"
        );
    }
    let (_, _, off) = filter_stream(1, Preset::new(9).unwrap().params(), new_cdef_choice);
    assert!(!off.contains(&true), "{off:?}");
}

/// AVM decodes frames of the full CDEF search to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_full_cdef_search_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags) = filter_stream(2, with_full_cdef(), new_cdef_choice);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Eight, "full-cdef");
    assert!(avm == recon, "AVM output differs");
}

/// Preset 6 with every luma restoration unit size.
fn with_lr_sizes() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        lr_unit_sizes: 4,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// Whether `core` codes luma restoration with units of 256 or 512, larger
/// than every superblock of this stream (at most 128x128).
fn large_lr_units(core: &FrameHeaderCore) -> bool {
    core.lr_params
        .as_ref()
        .is_some_and(|lr| lr.uses_lr && lr.loop_restoration_size[0] >= 256)
}

/// With `lr_unit_sizes` 4 some frame of a one-tile pan codes luma
/// restoration units larger than its superblock; the stream decodes to the encoder recon and 1, 2 and 8
/// threads write the same bytes. Off, no frame does.
#[test]
fn lr_unit_size_frames_decode_to_the_encoder_recon_on_any_thread_count() {
    let (one, recon, flags) = tiled_filter_stream(1, with_lr_sizes(), large_lr_units, 0);
    assert!(flags.contains(&true), "{flags:?}");
    assert_same_samples(&decode(&one.concat()), &recon, "lr unit sizes");
    for threads in [2, 8] {
        assert!(
            tiled_filter_stream(threads, with_lr_sizes(), large_lr_units, 0).0 == one,
            "{threads} threads"
        );
    }
    let (_, _, off) = tiled_filter_stream(1, Preset::new(6).unwrap().params(), large_lr_units, 0);
    assert!(!off.contains(&true), "{off:?}");
}

/// AVM decodes frames with large luma restoration units to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_lr_unit_size_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, recon, flags) = tiled_filter_stream(2, with_lr_sizes(), large_lr_units, 0);
    assert!(flags.contains(&true), "{flags:?}");
    let avm = avm_decode(&avmdec, &units, (200, 136), BitDepth::Eight, "lr-sizes");
    assert!(avm == recon, "AVM output differs");
}
