// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Staged mode decision: MDS0 class budgets and the inter-frame intra
//! shortlist.

use super::*;
use crate::mode_decision::{MDS0_PRUNED, MDS1_PRUNED, PD0_PASSES};
use crate::preset::PresetParams;

/// Preset 6 searched in one pass, with MDS0 `budgets` and an inter-frame
/// intra shortlist of `intra`.
fn staged(budgets: [u8; 3], intra: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        mds0_budgets: budgets,
        mds_inter_frame_intra: intra,
        pd0_row: u8::MAX,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A noisy two-tile 10-bit mini-GOP stream at `qp` with `preset` on
/// `threads` threads: the units, the recon, the headers, and the candidates
/// MDS0 dropped (counted on one thread).
fn staged_stream(
    preset: &'static PresetParams,
    qp: u16,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    let mut config = config(136, 72, BitDepth::Ten, 6, qp);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    MDS0_PRUNED.set(0);
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, super::compound::noisy_panning_frame, qp);
    (units, expected, cores, MDS0_PRUNED.get())
}

/// Tight MDS0 budgets drop inter candidates in frames that later frames
/// reference, with deblocking and CDEF on; the stream decodes to the encoder
/// recon and codes the same bytes on 1, 2 and 8 threads.
#[test]
fn staged_streams_decode_on_any_thread_count() {
    let preset = staged([1, 1, 1], 1);
    let (one, expected, cores, pruned) = staged_stream(preset, 200, 1);
    assert!(pruned > 0, "MDS0 dropped no candidate");
    assert!(cores.len() > 2 && cores.iter().any(|core| core.inter.is_some()));
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert!(
        cores
            .iter()
            .any(|core| core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable))
    );
    assert_same_samples(&decode(&one), &expected, "staged mode decision");
    for threads in [2, 8] {
        assert!(
            staged_stream(preset, 200, threads).0 == one,
            "{threads} threads"
        );
    }
}

/// The length and FNV-1a hash of the p5 stream of [`staged_stream`] at qp
/// 200, recorded from main e20662669 before the staged mode decision.
const P5_GOLDEN: (usize, u64) = (3028, 0xb309_6d3f_f798_d7c8);

/// The p5 row, whose MDS0 budgets are open and whose inter-frame intra
/// shortlist is `intra_modes`, codes the stream of the unstaged search.
#[test]
fn open_budgets_code_the_unstaged_stream() {
    let p5 = Preset::new(5).unwrap().params();
    assert_eq!(p5.mds0_budgets, [u8::MAX; 3]);
    assert_eq!(p5.mds_inter_frame_intra, p5.intra_modes);
    let (units, _, _, pruned) = staged_stream(p5, 200, 1);
    assert_eq!(pruned, 0);
    assert_eq!((units.len(), fnv(&units)), P5_GOLDEN);
}

/// Open MDS0 limits drop no candidate, and tight limits code another
/// stream.
#[test]
fn open_limits_drop_no_candidate() {
    let (units, _, _, pruned) = staged_stream(staged([u8::MAX; 3], 6), 200, 1);
    assert_eq!(pruned, 0);
    assert_ne!(staged_stream(staged([1, 1, 1], 1), 200, 1).0, units);
}

/// Preset 6 with two full trials per choice.
fn full_staged() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        full_trials: 2,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// MDS1 gives only the two cheapest candidates of a choice by a `DCT_DCT`
/// deadzone trial the full trial, in frames that later frames reference,
/// with deblocking and CDEF on; the stream decodes to the encoder recon and
/// codes the same bytes on 1, 2 and 8 threads.
#[test]
fn mds1_streams_decode_on_any_thread_count() {
    MDS1_PRUNED.set(0);
    let (one, expected, cores, _) = staged_stream(full_staged(), 200, 1);
    assert!(MDS1_PRUNED.get() > 0, "MDS1 dropped no candidate");
    assert!(cores.len() > 2 && cores.iter().any(|core| core.inter.is_some()));
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert!(
        cores
            .iter()
            .any(|core| core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable))
    );
    assert_same_samples(&decode(&one), &expected, "MDS1");
    for threads in [2, 8] {
        assert!(
            staged_stream(full_staged(), 200, threads).0 == one,
            "{threads} threads"
        );
    }
}

/// The length and FNV-1a hash of the [`staged_stream`] at qp 200 of preset
/// 6 with every candidate on a full trial, recorded from main 36451046e with
/// `full_trials: true`.
const FULL_GOLDEN: (usize, u64) = (2963, 0x19dd_6acc_4a12_1993);

/// `full_trials` 255 gives every candidate a full trial: it codes the
/// stream of the old `full_trials: true`, and MDS1 drops nothing.
#[test]
fn full_trials_255_code_the_old_full_trial_stream() {
    let preset = Box::leak(Box::new(PresetParams {
        full_trials: u8::MAX,
        pd0_row: u8::MAX,
        ..*Preset::new(6).unwrap().params()
    }));
    MDS1_PRUNED.set(0);
    let (units, ..) = staged_stream(preset, 200, 1);
    assert_eq!(MDS1_PRUNED.get(), 0);
    assert_eq!((units.len(), fnv(&units)), FULL_GOLDEN);
}

/// FNV-1a over `bytes`.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The two-pass partition decision of preset 6 runs a first pass in every
/// inter-frame superblock, in frames that later frames reference, with
/// deblocking and CDEF on; the stream decodes to the encoder recon and codes
/// the same bytes on 1, 2 and 8 threads.
#[test]
fn two_pass_streams_decode_on_any_thread_count() {
    let p6 = Preset::new(6).unwrap().params();
    assert!(p6.pd0_row != u8::MAX);
    PD0_PASSES.set(0);
    let (one, expected, cores, _) = staged_stream(p6, 200, 1);
    assert!(PD0_PASSES.get() > 0, "no first pass ran");
    assert!(cores.len() > 2 && cores.iter().any(|core| core.inter.is_some()));
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert!(
        cores
            .iter()
            .any(|core| core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable))
    );
    assert_same_samples(&decode(&one), &expected, "two-pass partition");
    for threads in [2, 8] {
        assert!(
            staged_stream(p6, 200, threads).0 == one,
            "{threads} threads"
        );
    }
    let single = Box::leak(Box::new(PresetParams {
        pd0_row: u8::MAX,
        ..*p6
    }));
    assert_ne!(staged_stream(single, 200, 1).0, one);
}

/// AVM decodes the staged stream like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_staged_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    MDS1_PRUNED.set(0);
    let (units, _, _, pruned) = staged_stream(
        Box::leak(Box::new(PresetParams {
            mds0_budgets: [1, 1, 1],
            ..*full_staged()
        })),
        200,
        1,
    );
    assert!(pruned > 0 && MDS1_PRUNED.get() > 0);
    let avm = avm_decode(
        &avmdec,
        std::slice::from_ref(&units),
        (136, 72),
        BitDepth::Ten,
        "staged",
    );
    assert!(avm == decode(&units), "AVM output differs");
}
