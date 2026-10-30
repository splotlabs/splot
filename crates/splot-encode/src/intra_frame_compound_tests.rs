// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound types (§ 5.20.7.16, § 5.20.7.6 `cwp_idx`) and the implicit mask
//! blend (§ 7.13.3.14). Random wedge, difference-weighted and weighted leaves
//! are covered by `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use crate::preset::PresetParams;

/// Preset 6 with `compound_types` and `implicit_mask`.
fn with_compound(compound_types: u8, implicit_mask: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        compound_types,
        implicit_mask,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A tiled mini-GOP stream at preset 6 with every compound tool on: the
/// sequence enables them, the loop filters run, later frames reference the
/// compound frames, the stream decodes to the encoder recon and codes the
/// same bytes on 1, 2 and 8 threads.
#[test]
fn compound_type_streams_decode_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Ten, 6, 200);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, with_compound(2, true)).unwrap();
        super::carry::planned_stream_of(&config, coder, panning_frame, 200)
    };
    let (one, expected, cores, coder) = stream(1);
    let inter = coder.sequence.inter.unwrap();
    assert!(inter.enable_masked_compound && inter.enable_cwp && inter.enable_imp_msk_bld);
    assert!(cores.iter().any(|core| {
        core.inter_tail
            .as_ref()
            .is_some_and(|tail| tail.reference_select)
    }));
    assert!(
        cores.iter().any(|core| {
            core.deblocking_filter_params
                .as_ref()
                .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
        }),
        "deblocking off"
    );
    assert_same_samples(&decode(&one), &expected, "compound types");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// `compound_types = 0` without the implicit mask codes the sequence with
/// every compound tool off.
#[test]
fn compound_types_off_disable_the_sequence_tools() {
    let config = config(72, 40, BitDepth::Eight, 6, 110);
    let coder = IntraFrameCoder::with_preset(&config, with_compound(0, false)).unwrap();
    let inter = coder.sequence.inter.unwrap();
    assert!(!inter.enable_masked_compound && !inter.enable_cwp && !inter.enable_imp_msk_bld);
    let coder = IntraFrameCoder::with_preset(&config, with_compound(1, false)).unwrap();
    let inter = coder.sequence.inter.unwrap();
    assert!(inter.enable_masked_compound && !inter.enable_cwp);
}

/// A panning frame with noise, which averaging two references removes.
fn noisy_panning_frame(config: &Config, index: u32) -> SourceFrame {
    super::grain_streams::with_noise(config, &panning_frame(config, index), 3 + index)
}

/// A tiled mini-GOP stream at a coarse quantizer whose compound candidates
/// are forced to wedges, difference-weighted masks and `cwp_idx` weights,
/// with vector refinement on: masked `NEAR_NEARMV` blocks drop the refinement
/// the decoder infers (`DecidedAgainstRefinemv`, with its deblock sub-PU
/// size), the loop filters run, later frames take the motion and weights of
/// the earlier ones, the stream decodes to the encoder recon and codes the
/// same bytes on 1, 2 and 8 threads.
#[test]
fn forced_compound_types_decode_on_any_thread_count() {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        compound_types: crate::mode_decision::FORCED_COMPOUND_TYPES,
        implicit_mask: true,
        refinemv: true,
        ..*Preset::new(6).unwrap().params()
    }));
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 200);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200)
    };
    crate::tile_writer::CODED_TYPES.set([0; 4]);
    let (one, expected, cores, _) = stream(1);
    let [weighted, wedges, diffs, masked_near] = crate::tile_writer::CODED_TYPES.get();
    assert!(
        weighted > 0 && wedges > 0 && diffs > 0 && masked_near > 0,
        "{weighted} {wedges} {diffs} {masked_near}"
    );
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert_same_samples(&decode(&one), &expected, "forced compound types");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// AVM decodes the forced compound type stream like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_compound_types_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        compound_types: crate::mode_decision::FORCED_COMPOUND_TYPES,
        implicit_mask: true,
        refinemv: true,
        ..*Preset::new(6).unwrap().params()
    }));
    let config = config(136, 72, BitDepth::Eight, 6, 200);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (units, ..) = super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200);
    let avm = avm_decode(
        &avmdec,
        std::slice::from_ref(&units),
        (136, 72),
        BitDepth::Eight,
        "compound-types",
    );
    assert!(avm == decode(&units), "AVM output differs");
}

/// A tiled 10-bit noisy mini-GOP stream with the compound type search at
/// level 2 at `qp` on `threads` threads: the units, the recon, the headers
/// and the weighted, wedge and difference-weighted leaves of the final pass.
fn searched_stream(
    qp: u16,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, [usize; 3]) {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        compound_types: 2,
        ..*Preset::new(6).unwrap().params()
    }));
    let mut config = config(136, 72, BitDepth::Ten, 6, qp);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    crate::tile_writer::CODED_TYPES.set([0; 4]);
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, qp);
    let [weighted, wedges, diffs, _] = crate::tile_writer::CODED_TYPES.get();
    (units, expected, cores, [weighted, wedges, diffs])
}

/// The compound type search at level 2 codes weighted, wedge and
/// difference-weighted leaves in frames that later frames reference with
/// compound blocks, with CDEF on; at a coarser quantizer it codes masked
/// leaves under deblocking and loop restoration. Both streams decode to the
/// encoder recon, the first with the same bytes on 1, 2 and 8 threads.
#[test]
fn searched_compound_types_decode_on_any_thread_count() {
    let (one, expected, cores, coded) = searched_stream(150, 1);
    assert!(coded.iter().all(|&count| count > 0), "{coded:?}");
    let compound_frames = cores
        .iter()
        .filter(|core| {
            core.inter_tail
                .as_ref()
                .is_some_and(|tail| tail.reference_select)
        })
        .count();
    assert!(compound_frames > 1, "{compound_frames} compound frames");
    assert!(
        cores
            .iter()
            .any(|core| core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable))
    );
    assert_same_samples(&decode(&one), &expected, "searched compound types");
    for threads in [2, 8] {
        assert!(searched_stream(150, threads).0 == one, "{threads} threads");
    }
    let (coarse, expected, cores, coded) = searched_stream(240, 1);
    assert!(coded[1] + coded[2] > 0, "{coded:?}");
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert!(
        cores
            .iter()
            .any(|core| core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr))
    );
    assert_same_samples(
        &decode(&coarse),
        &expected,
        "coarse searched compound types",
    );
}
