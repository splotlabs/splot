// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    CcsoOffsets, CcsoPlaneParams, CoreSeqView, FrameFilterParams, KeyFrameTxParams, UniformTiles,
    build_key_frame_header,
};
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
    parse_frame_header_core,
};
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_decode::encoder_syntax::{CcsoGrid, FilterRecorder, apply_loop_filters, ccso_offset};
use splot_recon::BitDepth;

use super::ccso::{CcsoInput, class_sums};
use super::cdef_rate;
use crate::config::{Config, RateControlMode};
use crate::encoder::{CodedFrame, FrameCoder};
use crate::frame::SourceFrame;
use crate::intra_frame::{IntraFrameCoder, visible_planes};
use crate::preset::Preset;
use crate::rd::Lambda;
use crate::test_support::{assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes};

/// `(width, height, bit depth, preset, qp)`: both superblock sizes (presets 0
/// to 6 code 128x128, 9 and 10 code 64x64), odd sizes, and qps where filters pay.
/// The slow presets code small frames, so debug test runs stay short.
const CASES: [(u32, u32, BitDepth, u8, u16); 8] = [
    (64, 64, BitDepth::Eight, 6, 160),
    (130, 34, BitDepth::Ten, 6, 220),
    (200, 136, BitDepth::Eight, 6, 200),
    (257, 131, BitDepth::Ten, 10, 240),
    (352, 288, BitDepth::Eight, 9, 120),
    (96, 200, BitDepth::Ten, 9, 250),
    (33, 23, BitDepth::Eight, 0, 200),
    (40, 24, BitDepth::Eight, 3, 120),
];

/// Smooth shapes with sharp edges and light noise, which blocky coding damages.
fn source(config: &Config, seed: u32) -> SourceFrame {
    let max = f64::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        let scale = if plane == 0 { 1.0 } else { 2.0 };
        let mut samples = Vec::with_capacity(size.width() * size.height());
        for y in 0..size.height() {
            for x in 0..size.width() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let (fx, fy) = (x as f64 * scale, y as f64 * scale);
                let wave = (fx / 9.0).sin() * (fy / 13.0).cos();
                let disc = f64::from(u8::from((fx - 60.0).hypot(fy - 50.0) < 35.0));
                let noise = f64::from(state % 64) / 64.0 - 0.5;
                let value = 0.5 + 0.2 * wave + 0.2 * disc + 0.02 * noise + plane as f64 * 0.03;
                samples.push((value.clamp(0.0, 1.0) * max) as u16);
            }
        }
        samples
    });
    SourceFrame::new(config, u64::from(seed), planes).unwrap()
}

fn code(case: (u32, u32, BitDepth, u8, u16), seed: u32) -> (Config, CodedFrame) {
    let (width, height, bit_depth, preset, qp) = case;
    let mut config = Config::new(width, height);
    config.bit_depth = bit_depth;
    config.preset = Preset::new(preset).unwrap();
    config.rate_control = RateControlMode::ConstantQp { qp };
    let coded = IntraFrameCoder::new(&config)
        .unwrap()
        .encode_frame(
            &source(&config, seed),
            &crate::gop::plan(0, config.keyint),
            qp,
        )
        .unwrap();
    (config, coded)
}

/// The sequence and frame headers of a coded temporal unit.
fn headers(unit: &[u8]) -> (SequenceHeader, FrameHeaderCore) {
    let parsed = parse_annex_b_obus_partial(unit);
    let obu = |obu_type| {
        parsed
            .obus
            .iter()
            .find(|obu| obu.header.obu_type == obu_type)
            .unwrap()
            .payload
    };
    let seq_payload = obu(ObuType::SequenceHeader);
    let seq = parse_sequence_header(&mut BitReader::new(seq_payload, ByteOffset::new(0))).unwrap();
    let mut reader = BitReader::new(obu(ObuType::ClosedLoopKey), ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let core = parse_frame_header_core(
        &mut reader,
        &FrameHeaderParseInput {
            obu_type: ObuType::ClosedLoopKey,
            first_picture_in_tu: true,
            active_sequence: Some(&seq),
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        },
    )
    .unwrap();
    (seq, core)
}

#[test]
fn filtered_frames_decode_to_the_encoder_recon() {
    let mut used = [0; 4];
    for (index, case) in CASES.into_iter().enumerate() {
        let (config, coded) = code(case, index as u32);
        let name = format!("case {index} {case:?}");
        assert_same_samples(
            &decode(&coded.temporal_unit),
            &raw_bytes(coded.recon.iter().map(Vec::as_slice), config.bit_depth),
            &name,
        );
        let (_, core) = headers(&coded.temporal_unit);
        let deblock = core
            .deblocking_filter_params
            .unwrap()
            .apply_deblocking_filter;
        used[0] += usize::from(deblock.contains(&true));
        used[1] += usize::from(core.cdef_params.unwrap().cdef_frame_enable);
        used[2] += usize::from(core.lr_params.unwrap().uses_lr);
        used[3] += usize::from(core.ccso_params.unwrap().ccso_frame_flag == Some(true));
    }
    assert!(
        used[..3].iter().all(|&count| count >= 2) && used[3] >= 1,
        "filters used {used:?}"
    );
}

/// Every GDF pair is tried once; the first takes the largest
/// `gdf_pic_qc_idx` and the scale index of its qindex at either bit depth.
#[test]
fn gdf_candidates_start_from_the_qindex_scale() {
    for (qindex, bit_depth, first) in [
        (100, 8, (3, 1)),
        (140, 8, (3, 2)),
        (200, 8, (3, 3)),
        (188, 10, (3, 2)),
    ] {
        let order = super::gdf::candidates(qindex, bit_depth);
        assert_eq!(order[0], first, "q{qindex} {bit_depth}-bit");
        let mut sorted = order;
        sorted.sort_unstable();
        assert_eq!(sorted, core::array::from_fn(|i| (i as u8 / 4, i as u8 % 4)));
    }
}

/// A frame with a lossless segment keeps GDF off: the search skips it and
/// the facade refuses its errors, since GDF leaves lossless samples alone.
#[test]
fn gdf_skips_frames_with_a_lossless_segment() {
    let mut config = Config::new(64, 64);
    config.preset = Preset::new(9).unwrap();
    let coded = IntraFrameCoder::new(&config)
        .unwrap()
        .encode_wave(&[(&source(&config, 3), crate::gop::plan(0, config.keyint), 160)])
        .unwrap()
        .pop()
        .unwrap();
    let (seq, mut core) = headers(&coded.temporal_unit);
    assert!(!super::gdf::has_lossless_segment(&core));
    if let Some(info) = core.lossless_info.as_mut() {
        info.has_lossless_segment = true;
    }
    assert!(super::gdf::has_lossless_segment(&core));
    let (_, recon, _, _) = coded.reference.unwrap();
    let refs = splot_decode::encoder_syntax::EncoderReferences::new(&seq).unwrap();
    let flags = splot_decode::encoder_syntax::GdfFlags {
        blocks: &[],
        refs: &refs,
    };
    assert!(
        splot_decode::encoder_syntax::gdf_errors(&seq, &core, (&recon, &recon), flags).is_err()
    );
}

#[test]
fn more_cdef_sets_cost_more_bits() {
    let rates: Vec<_> = (1..=8).map(|sets| cdef_rate(sets, 10)).collect();
    assert!(rates.windows(2).all(|pair| pair[0] < pair[1]));
}

/// AVM decodes every filtered frame to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_filtered_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, case) in CASES.into_iter().enumerate() {
        let units: Vec<Vec<u8>> = (0..2)
            .map(|seed| code(case, index as u32 * 2 + seed).1.temporal_unit)
            .collect();
        let avm = avm_decode(
            &avmdec,
            &units,
            (case.0, case.1),
            case.2,
            &format!("filters{index}"),
        );
        assert!(
            avm == decode(&units.concat()),
            "case {index} {case:?}: AVM output differs"
        );
    }
}

/// One plane of `ccso_params()` with offsets `offset_idx`.
fn ccso_plane(
    (bo_only, ext_filter, edge_clf): (bool, u8, bool),
    (scale_idx, quant_idx, band_log2): (u8, u8, u8),
    offset_idx: impl Fn(usize) -> u8,
) -> CcsoPlaneParams {
    let interval = match (bo_only, edge_clf) {
        (true, _) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    CcsoPlaneParams {
        ccso_planes: true,
        ccso_bo_only: Some(bo_only),
        ccso_scale_idx: Some(scale_idx),
        ccso_quant_idx: Some(quant_idx),
        ccso_ext_filter: Some(ext_filter),
        ccso_edge_clf: Some(edge_clf),
        ccso_max_band_log2: Some(band_log2),
        ccso_offset_idx: CcsoOffsets::from_iter_checked(
            (0..(interval * interval) << band_log2).map(offset_idx),
        )
        .unwrap(),
        ..CcsoPlaneParams::default()
    }
}

/// The search classes are the decoder's: on a frame with every `ccso_blk`
/// on and deblocking and CDEF off, the decoder output minus its input sums,
/// over each class the search counts, to the count times the class offset,
/// in every plane and for edge, edge-classifier and band-only parameters.
#[test]
fn ccso_search_classes_match_the_decoder_output() {
    let (width, height, qp) = (300, 270, 150);
    let mut config = Config::new(width, height);
    config.bit_depth = BitDepth::Ten;
    config.preset = Preset::new(9).unwrap();
    config.preset_overrides = vec![("wavefront".into(), "false".into())];
    let mut coded = IntraFrameCoder::new(&config)
        .unwrap()
        .encode_wave(&[(&source(&config, 5), crate::gop::plan(0, config.keyint), qp)])
        .unwrap();
    let coded = coded.pop().unwrap();
    let (seq, _) = headers(&coded.temporal_unit);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let (_, recon, start, _) = coded.reference.unwrap();
    let cases = [
        ccso_plane((false, 2, false), (1, 0, 3), |k| ((k * 5 + 3) % 8) as u8),
        ccso_plane((false, 5, true), (0, 1, 1), |k| (7 - k % 8) as u8),
        ccso_plane((true, 0, false), (0, 0, 2), |k| (k * 3 % 8) as u8),
        ccso_plane((false, 6, false), (2, 2, 0), |k| ((k + 1) % 8) as u8),
    ];
    for (index, params) in cases.into_iter().enumerate() {
        let filters = FrameFilterParams {
            ccso: [params; 3],
            ..FrameFilterParams::default()
        };
        let core = build_key_frame_header(
            &view,
            u32::from(qp),
            0,
            &filters,
            KeyFrameTxParams::default(),
            UniformTiles::default(),
        )
        .unwrap();
        let grid = CcsoGrid::new(&seq, &core).unwrap();
        assert!(grid.rows * grid.cols > 1, "{grid:?}");
        let on = vec![vec![1; grid.rows * grid.cols]; 3];
        let recorder = FilterRecorder::new(&seq, &core).unwrap();
        let out =
            apply_loop_filters(&seq, &core, &recorder, &recon, (&[], &on, None), &[]).unwrap();
        let source = SourceFrame::new(&config, 0, visible_planes(&out).unwrap()).unwrap();
        let input = CcsoInput {
            seq: &seq,
            core: &core,
            start: &start,
            deblocked: &recon,
            filtered: &recon,
            source: &source,
            lambda: Lambda::from_qindex(u32::from(qp), config.bit_depth).unwrap(),
            pool: None,
        };
        for plane in 0..3 {
            let sums = class_sums(&input, plane, &params).unwrap();
            let samples = source.plane_sizes()[plane];
            assert_eq!(
                sums.iter().map(|&(_, count)| count as usize).sum::<usize>(),
                samples.width() * samples.height()
            );
            let mut moved = 0;
            for (class, &(sum, count)) in sums.iter().enumerate() {
                let offset = ccso_offset(params.ccso_offset_idx[class], 0)
                    .map(|base| base * (i32::from(params.ccso_scale_idx.unwrap()) + 1))
                    .unwrap();
                assert_eq!(
                    sum,
                    i64::from(offset) * i64::from(count),
                    "case {index} plane {plane} class {class}"
                );
                moved += usize::from(sum != 0);
            }
            assert!(moved >= 2, "case {index} plane {plane}: {sums:?}");
        }
    }
}
