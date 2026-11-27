// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use proptest::prelude::*;

use super::*;
use crate::{Config, Encoder, SourceFrame};

/// `fast` is at most `slow` in every context.
fn at_most<T: PartialOrd + Copy>(fast: &Levels<T>, slow: &Levels<T>) -> bool {
    fast.values().iter().zip(slow.values()).all(|(f, s)| f <= s)
}

#[test]
fn every_field_is_monotone_from_slow_to_fast() {
    for pair in PRESETS.windows(2) {
        let (slow, fast) = (&pair[0], &pair[1]);
        assert!(at_most(&slow.min_block_log2, &fast.min_block_log2));
        assert!(fast.max_block_log2 <= slow.max_block_log2);
        assert!(
            fast.min_block_log2
                .values()
                .iter()
                .all(|&m| m <= fast.max_block_log2)
        );
        assert!(at_most(&fast.intra_modes, &slow.intra_modes));
        assert!(at_most(
            &slow.partition_early_exit,
            &fast.partition_early_exit
        ));
        assert!(at_most(&slow.none_breakout, &fast.none_breakout));
        assert!(at_most(&slow.breakout_area, &fast.breakout_area));
        assert!(at_most(&slow.leaf_reuse, &fast.leaf_reuse));
        assert!(fast.sdp <= slow.sdp);
        assert!(fast.sdp_chroma_search <= slow.sdp_chroma_search);
        assert!(fast.extended_sdp <= slow.extended_sdp);
        assert!(fast.rdoq <= slow.rdoq);
        assert!(fast.rdoq_every_type <= slow.rdoq_every_type);
        assert!(fast.parity_hiding <= slow.parity_hiding);
        assert!(at_most(&fast.tcq, &slow.tcq));
        assert!(fast.tcq_search_trellis <= slow.tcq_search_trellis);
        assert!(fast.tcq_type_trellis <= slow.tcq_type_trellis);
        assert!(fast.tcq_search_trellis || !fast.tcq_type_trellis);
        assert!(fast.rdoq || !fast.rdoq_every_type);
        assert!(fast.full_trials <= slow.full_trials);
        let type_work = |p: &PresetParams| {
            let offered = crate::tx_choice::tx_type_candidates(p);
            match p.tx_type_model_keep {
                0 => 3 * offered,
                keep => offered + 3 * offered.min(usize::from(keep)),
            }
        };
        assert!(type_work(fast) <= type_work(slow));
        assert!(at_most(
            &fast.tx_partition_search,
            &slow.tx_partition_search
        ));
        assert!(at_most(&fast.motion_range, &slow.motion_range));
        assert!(fast.hme_range <= slow.hme_range && fast.me_range <= slow.me_range);
        assert!(slow.me_seeds == 0 || fast.me_seeds >= slow.me_seeds);
        assert!(at_most(&fast.newmv_ref_margin, &slow.newmv_ref_margin));
        assert!(at_most(&fast.light_leaf, &slow.light_leaf));
        assert!(slow.compound_skip_sad == 0 || fast.compound_skip_sad >= slow.compound_skip_sad);
        assert!(fast.subpel_depth <= slow.subpel_depth);
        assert!(fast.eighth_pel_max_qindex <= slow.eighth_pel_max_qindex);
        assert!(fast.loop_filter_search <= slow.loop_filter_search);
        assert!(fast.mini_gop_size <= slow.mini_gop_size);
        assert!(fast.bitrate_mini_gop_size <= slow.bitrate_mini_gop_size);
        assert!(fast.lookahead_frames <= slow.lookahead_frames);
        assert!(fast.tpl_delta_q <= slow.tpl_delta_q);
        assert!(fast.ssim_aq <= slow.ssim_aq);
        assert!(fast.segmentation <= slow.segmentation);
        assert!(fast.psy_rd <= slow.psy_rd);
        assert!(at_most(&fast.tf_frames, &slow.tf_frames));
        assert!(fast.compound_candidates <= slow.compound_candidates);
        assert!(fast.compound_refs <= slow.compound_refs);
        assert!(fast.jmvd_scales <= slow.jmvd_scales);
        assert!(fast.same_ref_modes <= slow.same_ref_modes);
        assert!(fast.compound_amvd <= slow.compound_amvd);
        assert!(fast.tip_amvd <= slow.tip_amvd);
        assert!(fast.warp_interintra <= slow.warp_interintra);
        assert!(fast.compound_local_warp <= slow.compound_local_warp);
        assert!(fast.cdf_carry <= slow.cdf_carry);
        assert!(fast.mv_prediction <= slow.mv_prediction);
        assert!(fast.drl_bits <= slow.drl_bits);
        assert!(fast.interp_filter_search <= slow.interp_filter_search);
        assert!(
            fast.mvd_tools
                .iter()
                .zip(slow.mvd_tools)
                .all(|(&f, s)| f <= s)
        );
        assert!(fast.ext_partitions <= slow.ext_partitions);
        assert!(fast.ext_three_way_margin <= slow.ext_three_way_margin);
        assert!(fast.ext_four_way_margin <= slow.ext_four_way_margin);
        assert!(at_most(&fast.pd_window, &slow.pd_window));
        assert!(fast.pd0_leaf_bits >= slow.pd0_leaf_bits);
        assert!(fast.ref_intra_margin <= slow.ref_intra_margin);
        assert!(fast.ref_intra_refresh >= slow.ref_intra_refresh);
        assert!(fast.ccso_candidates <= slow.ccso_candidates);
        assert!(fast.wiener_ns <= slow.wiener_ns);
        assert!(fast.wiener_ns_chroma <= slow.wiener_ns_chroma);
        assert!(fast.wiener_ns_classes <= slow.wiener_ns_classes);
        assert!(fast.mrl_trials <= slow.mrl_trials);
        assert!(fast.ist_sets <= slow.ist_sets);
        assert!(fast.intra_ist_rd <= slow.intra_ist_rd);
        assert!(fast.cctx <= slow.cctx);
        assert!(fast.cctx_candidates <= slow.cctx_candidates);
        assert!(fast.mhccp <= slow.mhccp);
        assert!(fast.fsc <= slow.fsc);
        assert!(fast.palette <= slow.palette);
        assert!(fast.intrabc <= slow.intrabc);
        assert!(fast.integer_mv <= slow.integer_mv);
        assert!(fast.screen_detect <= slow.screen_detect);
        assert!(fast.cfl_filter <= slow.cfl_filter);
        assert!(fast.cfl_zero_alpha <= slow.cfl_zero_alpha);
        assert!(fast.dip <= slow.dip);
        assert!(fast.dip_trials <= slow.dip_trials);
        assert!(fast.large_inter <= slow.large_inter);
        assert!(fast.gdf_candidates <= slow.gdf_candidates);
        assert!(fast.cdef_strengths <= slow.cdef_strengths);
        assert!(fast.cdef_on_skip <= slow.cdef_on_skip);
        assert!(fast.inter_ddt <= slow.inter_ddt);
        assert!(fast.inter_ist <= slow.inter_ist);
        assert!(fast.inter_ist_rd <= slow.inter_ist_rd);
        assert!(fast.tip <= slow.tip);
        assert!(fast.warp <= slow.warp);
        assert!(fast.global_motion <= slow.global_motion);
        assert!(fast.global_motion_predict <= slow.global_motion_predict);
        assert!(fast.compound_types <= slow.compound_types);
        assert!(fast.implicit_mask <= slow.implicit_mask);
        assert!(fast.interintra <= slow.interintra);
        assert!(fast.interintra_search <= slow.interintra_search);
        assert!(fast.bawp <= slow.bawp);
        assert!(fast.bawp_search <= slow.bawp_search);
        assert!(fast.tip_newmv <= slow.tip_newmv);
        assert!(fast.wavefront >= slow.wavefront);
        assert!(
            fast.mds0_budgets
                .values()
                .as_flattened()
                .iter()
                .zip(slow.mds0_budgets.values().as_flattened())
                .all(|(f, s)| f <= s)
        );
        assert!(fast.opfl <= slow.opfl);
        assert!(fast.tip_refinemv <= slow.tip_refinemv);
        assert!(
            fast.inter_tx_split == 0
                || (slow.inter_tx_split != 0 && fast.inter_tx_split >= slow.inter_tx_split)
        );
    }
    for params in PRESETS {
        assert!((1..=32).contains(&params.mini_gop_size));
        assert!(params.bitrate_mini_gop_size <= params.mini_gop_size);
        assert!(params.lookahead_frames >= params.mini_gop_size);
        assert!((1..=5).contains(&params.drl_bits));
    }
}

/// The measured placement of the interpolation filter search: off in
/// every row (see `ENC-INTERP-FILTER`); a merge must not change it
/// silently.
#[test]
fn interp_filter_search_placement() {
    assert_eq!(PRESETS.map(|params| params.interp_filter_search), [0; 11]);
}

/// The measured placement of inter IST and DDT: off in every row (see
/// `ENC-INTER-IST-DDT`).
#[test]
fn inter_ist_and_ddt_placement() {
    assert_eq!(
        PRESETS.map(|params| (params.inter_ist, params.inter_ddt)),
        [(false, false); 11]
    );
}

/// The measured placement of the MVD tools: off in every row (see
/// `ENC-MV-CODING`).
#[test]
fn mvd_tools_placement() {
    assert_eq!(PRESETS.map(|params| params.mvd_tools), [[false; 3]; 11]);
}

#[test]
fn set_overrides_one_field() -> core::result::Result<(), PresetError> {
    let mut params = PRESETS[6];
    params.set("gdf_candidates", "4")?;
    params.set("rdoq", "false")?;
    params.set("tcq", "true")?;
    params.set("intra_modes", "5,4,3,2")?;
    params.set("mvd_tools", "1,0,true")?;
    params.set("motion_range", "48,48,32,24")?;
    params.set("mds0_budgets", "4,2,2/4,2,2/2,1,1/2,1,1")?;
    params.set("pd_window", "3")?;
    let expected = PresetParams {
        gdf_candidates: 4,
        rdoq: false,
        tcq: Levels::all(true),
        intra_modes: Levels::by_frame([5, 4, 3, 2]),
        mvd_tools: [true, false, true],
        motion_range: Levels::by_frame([48, 48, 32, 24]),
        mds0_budgets: Levels::by_frame([[4, 2, 2], [4, 2, 2], [2, 1, 1], [2, 1, 1]]),
        pd_window: Levels::all(3),
        ..PRESETS[6]
    };
    assert_eq!(params, expected);
    Ok(())
}

#[test]
fn set_rejects_unknown_fields_and_bad_values() {
    let mut params = PRESETS[6];
    let unknown = PresetError::UnknownField { name: "gdf".into() };
    assert_eq!(params.set("gdf", "4"), Err(unknown.clone()));
    let message = unknown.to_string();
    assert!(
        PresetParams::FIELDS
            .iter()
            .all(|field| message.contains(field))
    );
    for (field, value) in [
        ("gdf_candidates", "256"),
        ("gdf_candidates", "-1"),
        ("motion_range", "65536"),
        ("rdoq", "2"),
        ("full_trials", "true"),
        ("rdoq", "yes"),
        ("tcq", "base"),
        ("tcq", "1,1"),
        ("intra_modes", "1,2,3,4,5"),
        ("mvd_tools", "1,0"),
        ("mvd_tools", "1,0,1,1"),
        ("mds0_budgets", "4,2,2/2,1,1"),
        ("mds0_budgets", "4,2,2,1"),
        ("motion_range", "48,32"),
    ] {
        assert!(
            matches!(
                params.set(field, value),
                Err(PresetError::InvalidValue { .. })
            ),
            "{field}={value}"
        );
    }
    assert_eq!(params, PRESETS[6]);
}

/// Each context axis splits where its type documents.
#[test]
fn contexts_classify_frames() {
    use FrameType::{Inter, Key};
    assert_eq!(ResolutionClass::of(854, 480), ResolutionClass::Sd);
    assert_eq!(ResolutionClass::of(1280, 720), ResolutionClass::Hd);
    assert_eq!(ResolutionClass::of(1920, 1088), ResolutionClass::Hd);
    assert_eq!(ResolutionClass::of(2048, 1088), ResolutionClass::Uhd);
    assert_eq!(FrameKind::of(Key, 0, 0xff), FrameKind::Key);
    assert_eq!(FrameKind::of(Inter, 0, 1), FrameKind::Base);
    assert_eq!(FrameKind::of(Inter, 0, 0), FrameKind::Base);
    assert_eq!(FrameKind::of(Inter, 1, 2), FrameKind::Mid);
    assert_eq!(FrameKind::of(Inter, 2, 0), FrameKind::Leaf);
    assert_eq!(CoeffLevel::of(128), CoeffLevel::High);
    assert_eq!(CoeffLevel::of(129), CoeffLevel::Normal);
    assert_eq!(CoeffLevel::of(193), CoeffLevel::Low);
    let mut values = [0u8; 36];
    for (i, value) in values.iter_mut().enumerate() {
        *value = i as u8;
    }
    let levels = Levels(values);
    let context = PresetContext {
        resolution: ResolutionClass::Uhd,
        frame: FrameKind::Mid,
        coeff: CoeffLevel::Low,
    };
    assert_eq!(levels.at(context), 2 * 9 + 2 * 3);
    assert_eq!(Levels::by_frame([1, 2, 3, 4]).at(context), 3);
    let split = frames_sd([[1; 11]; 4], [[2; 11]; 4])[5];
    assert_eq!(split.at(context), 1);
    let sd = PresetContext {
        resolution: ResolutionClass::Sd,
        ..context
    };
    assert_eq!(split.at(sd), 2);
}

#[test]
fn every_row_passes_validate() {
    for params in PRESETS.iter().chain(&SCREEN_PRESETS) {
        assert_eq!(params.validate(), Ok(()));
    }
}

#[test]
fn validate_rejects_unsupported_ranges() {
    for (overrides, field) in [
        (
            &[("min_block_log2", "7"), ("max_block_log2", "6")][..],
            "min_block_log2",
        ),
        (&[("max_block_log2", "9")], "max_block_log2"),
        (&[("sb256_min_height", "63")], "sb256_min_height"),
        (&[("drl_bits", "6")], "drl_bits"),
        (&[("lossless_fsc", "3")], "lossless_fsc"),
        (&[("drl_bits", "0")], "drl_bits"),
        (&[("lookahead_frames", "3")], "lookahead_frames"),
        (&[("mini_gop_size", "0")], "mini_gop_size"),
        (&[("mini_gop_size", "33")], "mini_gop_size"),
        (&[("bitrate_mini_gop_size", "0")], "bitrate_mini_gop_size"),
        (&[("bitrate_mini_gop_size", "9")], "bitrate_mini_gop_size"),
        (&[("intra_modes", "0")], "intra_modes"),
        (&[("mds0_budgets", "4,0,2")], "mds0_budgets"),
        (&[("intra_modes", "3,3,0,3")], "intra_modes"),
        (&[("tf_frames", "9")], "tf_frames"),
        (&[("gdf_candidates", "17")], "gdf_candidates"),
        (&[("ssim_aq", "201")], "ssim_aq"),
        (&[("segmentation", "2")], "segmentation"),
        (&[("psy_rd", "201")], "psy_rd"),
        (&[("cdef_strengths", "0")], "cdef_strengths"),
        (&[("ccso_candidates", "113")], "ccso_candidates"),
        (&[("ccso_bo_band_log2", "7")], "ccso_bo_band_log2"),
        (&[("ccso_reuse", "3")], "ccso_reuse"),
        (&[("lr_unit_sizes", "0")], "lr_unit_sizes"),
        (&[("lr_chroma_unit_sizes", "5")], "lr_chroma_unit_sizes"),
        (&[("deblock_offset_bits", "6")], "deblock_offset_bits"),
        (&[("cdef_pairs", "65")], "cdef_pairs"),
        (&[("compound_refs", "0")], "compound_refs"),
        (&[("compound_refs", "8")], "compound_refs"),
        (&[("jmvd_scales", "0")], "jmvd_scales"),
        (&[("jmvd_scales", "6")], "jmvd_scales"),
        (&[("interintra_search", "2")], "interintra_search"),
        (&[("sdp_chroma_search", "2")], "sdp_chroma_search"),
        (&[("extended_sdp", "3")], "extended_sdp"),
        (&[("warp_interintra", "3")], "warp_interintra"),
        (&[("bawp_search", "3")], "bawp_search"),
        (&[("tip", "3")], "tip"),
        (&[("cdf_update_tile", "2")], "cdf_update_tile"),
        (&[("tip_output", "201")], "tip_output"),
        (&[("tip_output_search", "3")], "tip_output_search"),
        (&[("quant_matrices", "4")], "quant_matrices"),
        (&[("opfl", "false")], "tip_refinemv"),
        (&[("warp", "3")], "warp"),
        (&[("global_motion", "3")], "global_motion"),
        (&[("compound_types", "4")], "compound_types"),
        (&[("intrabc_morph", "1"), ("bawp", "0")], "intrabc_morph"),
        (&[("intrabc_inter", "1")], "intrabc_inter"),
    ] {
        let mut params = PRESETS[6];
        for (name, value) in overrides {
            assert_eq!(params.set(name, value), Ok(()));
        }
        assert!(
            matches!(params.validate(), Err(PresetError::OutOfRange { field: f, .. }) if f == field),
            "{overrides:?}"
        );
    }
}

/// Values at the type extremes and around the supported bounds.
const PROBE_VALUES: [&str; 16] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "16", "17", "255", "65535", "1,1,0,0",
    "1,1,1",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A random set of overrides either fails validation or encodes two
    /// tiny frames.
    #[test]
    fn random_overrides_validate_or_encode(
        preset in 0..=Preset::MAX,
        picks in prop::collection::vec(
            (0..PresetParams::FIELDS.len(), 0..PROBE_VALUES.len()),
            1..7,
        ),
    ) {
        let mut config = Config::new(24, 16);
        config.preset = Preset::new(preset).unwrap_or_default();
        config.threads = splot_parallel::ThreadCount::from_count_or_auto(1);
        let mut scratch = PRESETS[0];
        for (field, start) in picks {
            let name = PresetParams::FIELDS[field];
            let value = (start..start + PROBE_VALUES.len())
                .map(|i| PROBE_VALUES[i % PROBE_VALUES.len()])
                .find(|value| scratch.set(name, value).is_ok());
            config.preset_overrides.extend(value.map(|value| (name.into(), value.into())));
        }
        if config.validate().is_ok() {
            let encode = || -> crate::Result<usize> {
                let mut encoder = Encoder::new(config.clone())?;
                for pts in 0..2 {
                    let luma = (0..24 * 16).map(|i| ((i * 37 + i / 24 * 91 + pts * 13) % 97) as u16 + 60);
                    let planes = [luma.collect(), vec![128; 12 * 8], vec![120; 12 * 8]];
                    encoder.send_frame(SourceFrame::new(&config, pts as u64, planes)?)?;
                }
                encoder.flush()?;
                let mut bytes = 0;
                while let Some(packet) = encoder.receive_packet()? {
                    bytes += packet.data.len();
                }
                Ok(bytes)
            };
            let result = encode();
            prop_assert!(result.is_ok(), "{:?}: {:?}", config.preset_overrides, result);
        }
    }
}

#[test]
fn every_field_is_settable_by_name() {
    let mut params = PRESETS[10];
    for &field in PresetParams::FIELDS {
        assert!(
            ["1", "1,1,1"]
                .iter()
                .any(|value| params.set(field, value).is_ok()),
            "{field}"
        );
    }
    assert_ne!(params, PRESETS[10]);
}

#[test]
fn rejects_out_of_range_preset() {
    assert!(Preset::new(Preset::MAX).is_ok());
    assert!(matches!(
        Preset::new(Preset::MAX + 1),
        Err(Error::InvalidPreset { preset: 11 })
    ));
}
