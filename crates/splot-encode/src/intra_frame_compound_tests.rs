// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound types (§ 5.20.7.16, § 5.20.7.6 `cwp_idx`) and the implicit mask
//! blend (§ 7.13.3.16). Random wedge, difference-weighted and weighted leaves
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
    assert!(!inter.enable_masked_compound && inter.enable_cwp);
}
