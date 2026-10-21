// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
    parse_frame_header_core,
};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::BitDepth;

use super::cdef_rate;
use crate::config::{Config, RateControlMode};
use crate::encoder::{CodedFrame, FrameCoder};
use crate::frame::SourceFrame;
use crate::intra_frame::IntraFrameCoder;
use crate::preset::Preset;
use crate::rate_control::{FramePlanInfo, FrameType};
use crate::test_support::{assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes};

/// `(width, height, bit depth, preset, qp)`: both superblock sizes (presets 0
/// and 6 code 128x128, 10 codes 64x64), odd sizes, and qps where filters pay.
const CASES: [(u32, u32, BitDepth, u8, u8); 8] = [
    (64, 64, BitDepth::Eight, 6, 160),
    (130, 66, BitDepth::Ten, 0, 220),
    (200, 136, BitDepth::Eight, 10, 180),
    (257, 131, BitDepth::Ten, 6, 240),
    (352, 288, BitDepth::Eight, 6, 140),
    (96, 200, BitDepth::Ten, 10, 250),
    (33, 47, BitDepth::Eight, 0, 200),
    (160, 96, BitDepth::Eight, 3, 120),
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

fn code(case: (u32, u32, BitDepth, u8, u8), seed: u32) -> (Config, CodedFrame) {
    let (width, height, bit_depth, preset, qp) = case;
    let mut config = Config::new(width, height);
    config.bit_depth = bit_depth;
    config.preset = Preset::new(preset).unwrap();
    config.rate_control = RateControlMode::ConstantQp { qp };
    let plan = FramePlanInfo {
        frame_index: 0,
        frame_type: FrameType::Key,
    };
    let coded = IntraFrameCoder::new(&config)
        .unwrap()
        .encode_frame(&source(&config, seed), &plan, qp)
        .unwrap();
    (config, coded)
}

/// The frame header of a coded temporal unit.
fn frame_header(unit: &[u8]) -> FrameHeaderCore {
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
    let frame_payload = obu(ObuType::ClosedLoopKey);
    parse_frame_header_core(
        &mut BitReader::new(frame_payload, ByteOffset::new(0)),
        &FrameHeaderParseInput {
            obu_type: ObuType::ClosedLoopKey,
            first_picture_in_tu: true,
            active_sequence: Some(&seq),
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        },
    )
    .unwrap()
}

#[test]
fn filtered_frames_decode_to_the_encoder_recon() {
    let mut used = [0; 3];
    for (index, case) in CASES.into_iter().enumerate() {
        let (config, coded) = code(case, index as u32);
        let name = format!("case {index} {case:?}");
        assert_same_samples(
            &decode(&coded.temporal_unit),
            &raw_bytes(coded.recon.iter().map(Vec::as_slice), config.bit_depth),
            &name,
        );
        let core = frame_header(&coded.temporal_unit);
        let deblock = core
            .deblocking_filter_params
            .unwrap()
            .apply_deblocking_filter;
        used[0] += usize::from(deblock.contains(&true));
        used[1] += usize::from(core.cdef_params.unwrap().cdef_frame_enable);
        used[2] += usize::from(core.lr_params.unwrap().uses_lr);
    }
    assert!(
        used.iter().all(|&count| count >= 2),
        "filters used {used:?}"
    );
}

#[test]
fn more_cdef_sets_cost_more_bits() {
    let rates: Vec<f64> = (1..=8).map(|sets| cdef_rate(sets, 10)).collect();
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
