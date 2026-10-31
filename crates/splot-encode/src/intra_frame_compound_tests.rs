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
pub(super) fn noisy_panning_frame(config: &Config, index: u32) -> SourceFrame {
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
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    let (one, expected, cores, _) = stream(1);
    let [weighted, wedges, diffs, masked_near, ..] = crate::tile_writer::CODED_TYPES.get();
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
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, qp);
    let [weighted, wedges, diffs, ..] = crate::tile_writer::CODED_TYPES.get();
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

/// A noisy mini-GOP stream with the inter-intra search codes inter-intra
/// leaves through the filtered frames that later frames reference, decodes
/// to the encoder recon, and codes the same bytes on 1, 2 and 8 threads.
#[test]
fn interintra_streams_decode_on_any_thread_count() {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        interintra: true,
        ..*Preset::new(6).unwrap().params()
    }));
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Ten, 6, 200);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200)
    };
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    let (one, expected, cores, coder) = stream(1);
    assert!(coder.sequence.inter.unwrap().seq_enabled_motion_modes[1]);
    let interintra = crate::tile_writer::CODED_TYPES.get()[4];
    assert!(interintra > 0, "no inter-intra leaf");
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert_same_samples(&decode(&one), &expected, "inter-intra");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// The inter-intra preset of the tiled stream tests.
fn interintra_preset() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        interintra: true,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A two-tile 10-bit noisy mini-GOP stream at qp 200 whose leaves code their
/// inter-intra candidate wherever they have one: inter-intra leaves at the
/// left edge of the second tile predict without the left edge, as the
/// decoder does (§ 7.13.3.30 edge availability inside the tile), and the
/// stream decodes to the encoder recon; AVM decodes it like splot where a
/// decoder build is set.
#[test]
fn forced_interintra_at_tile_edges_decodes() {
    let mut config = config(136, 72, BitDepth::Ten, 6, 200);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(1);
    let coder = IntraFrameCoder::with_preset(&config, interintra_preset()).unwrap();
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    crate::mode_decision::FORCE_INTERINTRA.set(true);
    let (units, expected, ..) =
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200);
    crate::mode_decision::FORCE_INTERINTRA.set(false);
    let [.., interintra, at_tile_edge, _] = crate::tile_writer::CODED_TYPES.get();
    assert!(
        at_tile_edge > 0,
        "{interintra} inter-intra leaves, none at a tile edge"
    );
    assert_same_samples(&decode(&units), &expected, "inter-intra at tile edges");
    if let Some(avmdec) = avm_decoder() {
        let avm = avm_decode(
            &avmdec,
            std::slice::from_ref(&units),
            (136, 72),
            BitDepth::Ten,
            "interintra-tiles",
        );
        assert!(avm == decode(&units), "AVM output differs");
    }
}

/// AVM decodes the two-tile inter-intra stream, which codes inter-intra
/// leaves, like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_interintra_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let mut config = config(136, 72, BitDepth::Ten, 6, 200);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(1);
    let coder = IntraFrameCoder::with_preset(&config, interintra_preset()).unwrap();
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    let (units, ..) = super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200);
    assert!(
        crate::tile_writer::CODED_TYPES.get()[4] > 0,
        "no inter-intra leaf"
    );
    let avm = avm_decode(
        &avmdec,
        std::slice::from_ref(&units),
        (136, 72),
        BitDepth::Ten,
        "interintra",
    );
    assert!(avm == decode(&units), "AVM output differs");
}

/// A noisy 10-bit mini-GOP stream of `width`x`height` in two tiles with the
/// BAWP search on `threads` threads: the units, the recon, the headers and
/// the BAWP leaves of the final pass (counted on one thread).
fn bawp_stream(
    (width, height): (u32, u32),
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, usize) {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        bawp: true,
        ..*Preset::new(6).unwrap().params()
    }));
    let mut config = config(width, height, BitDepth::Ten, 6, 200);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    assert!(coder.sequence.inter.unwrap().enable_bawp);
    crate::tile_writer::CODED_TYPES.set([0; 7]);
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, noisy_panning_frame, 200);
    (
        units,
        expected,
        cores,
        crate::tile_writer::CODED_TYPES.get()[6],
    )
}

/// The BAWP search codes BAWP leaves in a deblocked two-tile stream that
/// decodes to the encoder recon with the same bytes on 1, 2 and 8 threads,
/// and in a 130x70 stream, whose planes end inside a 4x4 unit.
#[test]
fn bawp_streams_decode_on_any_thread_count() {
    let (one, expected, cores, bawp) = bawp_stream((136, 72), 1);
    assert!(bawp > 0, "no BAWP leaf");
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert_same_samples(&decode(&one), &expected, "BAWP");
    for threads in [2, 8] {
        assert!(
            bawp_stream((136, 72), threads).0 == one,
            "{threads} threads"
        );
    }
    let (odd, expected, _, bawp) = bawp_stream((130, 70), 1);
    assert!(bawp > 0, "no BAWP leaf at 130x70");
    assert_same_samples(&decode(&odd), &expected, "BAWP 130x70");
}

/// AVM decodes the BAWP streams, which code BAWP leaves, like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_bawp_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for size in [(136, 72), (130, 70)] {
        let (units, _, _, bawp) = bawp_stream(size, 1);
        assert!(bawp > 0, "no BAWP leaf at {size:?}");
        let avm = avm_decode(
            &avmdec,
            std::slice::from_ref(&units),
            size,
            BitDepth::Ten,
            "bawp",
        );
        assert!(avm == decode(&units), "{size:?}: AVM output differs");
    }
}

/// A BAWP `NEWMV` at a superblock root whose luma template starts at
/// `refX = refY = 1` fits without chroma weighting but not with it, as the
/// 4:2:0 chroma template starts at 0 (§ 7.13.3.25 applies the requirement
/// to chroma only with `use_bawp_chroma`); `snap_prediction` drops the BAWP
/// that no longer fits instead of failing the frame.
#[test]
fn bawp_templates_fit_only_inside_the_reference() {
    let mut config = config(272, 200, BitDepth::Eight, 6, 140);
    config.chroma = PixelFormat::Yuv420;
    let mut checked = 0;
    let tree = |syntax: &mut IntraTileSyntax,
                node: PartitionNode,
                rng: &mut Rng,
                coverage: &mut Coverage| {
        let leaf = syntax.leaf(node)?;
        if checked == 0 && leaf.row4() == 0 && leaf.col4() == 0 && syntax.inter_allowed(&leaf) {
            let base = InterModes {
                ref_frame: 0,
                mode: InterMode::New,
                drl: 0,
                mv: MotionVector { row: 8, col: 8 },
                filter: splot_recon::InterpolationFilter::EightTap,
                precision: None,
                amvd: false,
                interintra: None,
                bawp: None,
            };
            let InterPrediction::Single(snapped) =
                crate::tile_writer::snap_prediction(syntax, &leaf, InterPrediction::Single(base))?
            else {
                unreachable!("a single-reference prediction snaps to one")
            };
            assert!(syntax.bawp_reads(&leaf, snapped)?);
            let with = |chroma| InterModes {
                bawp: Some(Bawp {
                    explicit_scale: None,
                    chroma,
                }),
                ..snapped
            };
            assert!(syntax.bawp_fits(&leaf, with(false))?);
            assert!(!syntax.bawp_fits(&leaf, with(true))?);
            let dropped = crate::tile_writer::snap_prediction(
                syntax,
                &leaf,
                InterPrediction::Single(with(true)),
            )?;
            assert_eq!(dropped, InterPrediction::Single(snapped));
            checked += 1;
        }
        super::random_tree(syntax, node, rng, coverage)
    };
    super::inter_stream_with(&config, 0xBA3F, tree);
    assert_eq!(checked, 1);
}

/// The compound search derives the `NEAR_NEARMV` vectors of a reference
/// pair in one batch; every batched pair is checked against the decoder's
/// one-at-a-time derivation while the stream codes.
#[test]
fn batched_near_near_vectors_match_the_decoder() {
    crate::mode_decision::NEAR_NEAR_CHECKED.set(0);
    let (units, expected, ..) = searched_stream(150, 1);
    assert!(crate::mode_decision::NEAR_NEAR_CHECKED.get() > 0);
    assert_same_samples(&decode(&units), &expected, "batched NEAR_NEARMV");
}
