// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use splot_core::headers::frame::build_key_frame_header;

use crate::config::RateControlMode;
use crate::gop::GopFrame;
use crate::preset::Preset;
use crate::rate_control::FrameType;
use crate::test_support::{assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes};
use crate::tile_writer::PartitionTree;

/// Gradients, a ripple, and seeded noise, so blocks of every size split.
fn synthetic_frame(config: &Config, seed: u32) -> SourceFrame {
    let max = u32::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        let mut samples = Vec::with_capacity(size.width() * size.height());
        for y in 0..size.height() {
            for x in 0..size.width() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let ripple = ((x * x + y * 3) % 61) as u32 * max / 120;
                let ramp = ((x + 2 * y + plane * 17) as u32 * max / 300) % (max / 2);
                let noise = if (x / 16 + y / 16) % 3 == 0 {
                    state % (max / 8)
                } else {
                    0
                };
                samples.push(((ripple + ramp + noise) % (max + 1)) as u16);
            }
        }
        samples
    });
    SourceFrame::new(config, u64::from(seed), planes).unwrap()
}

fn config(width: u32, height: u32, bit_depth: BitDepth, preset: u8, qp: u8) -> Config {
    let mut config = Config::new(width, height);
    config.bit_depth = bit_depth;
    config.preset = Preset::new(preset).unwrap();
    config.rate_control = RateControlMode::ConstantQp { qp };
    config
}

fn code(config: &Config, seed: u32) -> CodedFrame {
    code_stream(config, &[seed]).pop().unwrap()
}

/// Codes one frame per seed in order, with key frames every `config.keyint`.
fn code_stream(config: &Config, seeds: &[u32]) -> Vec<CodedFrame> {
    let RateControlMode::ConstantQp { qp } = config.rate_control;
    let mut coder = IntraFrameCoder::new(config).unwrap();
    seeds
        .iter()
        .enumerate()
        .map(|(index, &seed)| {
            let plan = crate::gop::plan(index as u64, config.keyint);
            coder
                .encode_frame(&synthetic_frame(config, seed), &plan, qp)
                .unwrap()
        })
        .collect()
}

fn recon_bytes(coded: &CodedFrame, bit_depth: BitDepth) -> Vec<u8> {
    raw_bytes(coded.recon.iter().map(Vec::as_slice), bit_depth)
}

/// `(width, height, bit depth, preset, qp)`: odd sizes, both superblock sizes
/// (presets 0 to 6 code 128x128, 10 codes 64x64), and the qp extremes. The
/// slow presets code small frames, so debug test runs stay short.
const CASES: [(u32, u32, BitDepth, u8, u8); 8] = [
    (1, 1, BitDepth::Eight, 6, 80),
    (17, 9, BitDepth::Ten, 0, 1),
    (64, 64, BitDepth::Eight, 10, 255),
    (65, 65, BitDepth::Ten, 6, 120),
    (130, 34, BitDepth::Eight, 6, 40),
    (200, 136, BitDepth::Ten, 10, 200),
    (352, 288, BitDepth::Eight, 10, 100),
    (33, 17, BitDepth::Ten, 3, 60),
];

#[test]
fn coded_frames_decode_to_the_encoder_recon() {
    for (index, (width, height, bit_depth, preset, qp)) in CASES.into_iter().enumerate() {
        let config = config(width, height, bit_depth, preset, qp);
        let coded = code(&config, index as u32);
        let name = format!("{width}x{height} {bit_depth:?} preset {preset} qp {qp}");
        assert_same_samples(
            &decode(&coded.temporal_unit),
            &recon_bytes(&coded, bit_depth),
            &name,
        );
    }
}

/// Key frames every third frame; the inter frames between them code intra blocks.
#[test]
fn inter_frames_decode_to_the_encoder_recon_over_several_gops() {
    for (index, (width, height, bit_depth, preset, qp)) in CASES.into_iter().enumerate() {
        let mut config = config(width, height, bit_depth, preset, qp);
        config.keyint = core::num::NonZeroU32::new(3).unwrap();
        let seeds: Vec<u32> = (0..5).map(|frame| index as u32 * 5 + frame).collect();
        let coded = code_stream(&config, &seeds);
        let types: Vec<_> = coded.iter().map(|frame| frame.frame_type).collect();
        assert_eq!(
            types[..4],
            [
                FrameType::Key,
                FrameType::Inter,
                FrameType::Inter,
                FrameType::Key
            ]
        );
        let stream: Vec<u8> = coded
            .iter()
            .flat_map(|frame| frame.temporal_unit.clone())
            .collect();
        let expected: Vec<u8> = coded
            .iter()
            .flat_map(|frame| recon_bytes(frame, bit_depth))
            .collect();
        let name = format!("{width}x{height} {bit_depth:?} preset {preset} qp {qp}");
        assert_same_samples(&decode(&stream), &expected, &name);
    }
}

#[test]
fn coding_is_deterministic_and_lower_qp_costs_more_bits() {
    let fine = code(&config(96, 80, BitDepth::Eight, 6, 40), 7);
    let again = code(&config(96, 80, BitDepth::Eight, 6, 40), 7);
    let coarse = code(&config(96, 80, BitDepth::Eight, 6, 200), 7);
    assert_eq!(fine.temporal_unit, again.temporal_unit);
    assert!(fine.temporal_unit.len() > coarse.temporal_unit.len());
}

/// A superblock search from one state finds the same tree and cost twice:
/// trials roll back completely and no decision depends on hidden state.
#[test]
fn superblock_search_is_deterministic_and_rolls_back() {
    for preset in [3, 10] {
        let config = config(40, 24, BitDepth::Ten, preset, 90);
        let source = synthetic_frame(&config, 5);
        let frame = IntraFrameCoder::new(&config).unwrap();
        let tx = tx_choice::key_frame_tx_params(frame.preset);
        let header = FrameFilterParams::default();
        let core = build_key_frame_header(&frame.view, 90, 0, &header, tx).unwrap();
        let mut writer = KeyFrameTileWriter::new(&frame.sequence, &core).unwrap();
        let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
        let sb = writer.syntax().superblock_size4() * 4;
        let padded = PaddedSource::new(&source, mi_cols * 4 + sb, mi_rows * 4 + sb);
        let lambda = Lambda::from_qindex(90, config.bit_depth).unwrap();
        let mut coder =
            SourceResidualCoder::new(&padded, 90, config.bit_depth, frame.preset, lambda);
        let mut search = || {
            writer
                .search_superblock(|syntax, root| {
                    let mut trial = TrialCoder::new(syntax, &mut coder, frame.preset, lambda);
                    search_partition(&mut trial, root, frame.preset, lambda)
                })
                .unwrap()
        };
        let first = search();
        assert_eq!(first, search(), "preset {preset}");
        assert!(
            matches!(first.0, PartitionTree::Split { .. }),
            "preset {preset}"
        );
    }
}

/// AVM decodes every coded frame to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_coded_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, (width, height, bit_depth, preset, qp)) in CASES.into_iter().enumerate() {
        let mut config = config(width, height, bit_depth, preset, qp);
        config.keyint = core::num::NonZeroU32::new(3).unwrap();
        let seeds: Vec<u32> = (0..4).map(|seed| index as u32 * 4 + seed).collect();
        let units: Vec<Vec<u8>> = code_stream(&config, &seeds)
            .into_iter()
            .map(|frame| frame.temporal_unit)
            .collect();
        let avm = avm_decode(
            &avmdec,
            &units,
            (width, height),
            bit_depth,
            &format!("frame{index}"),
        );
        assert!(
            avm == decode(&units.concat()),
            "case {index}: AVM output differs"
        );
    }
}

/// A key frame, a hidden inter frame, and a show-existing frame of it, with
/// the samples a decoder outputs.
fn hidden_frame_stream(config: &Config) -> (Vec<Vec<u8>>, Vec<u8>) {
    use splot_core::headers::frame::{
        encode_show_existing_frame_obu, encode_temporal_delimiter_obu,
    };
    let RateControlMode::ConstantQp { qp } = config.rate_control;
    let mut coder = IntraFrameCoder::new(config).unwrap();
    let key_plan = crate::gop::plan(0, config.keyint);
    let key = coder
        .encode_frame(&synthetic_frame(config, 1), &key_plan, qp)
        .unwrap();
    let hidden_plan = GopFrame {
        show_frame: false,
        ..crate::gop::plan(1, config.keyint)
    };
    let hidden = coder
        .encode_frame(&synthetic_frame(config, 2), &hidden_plan, qp)
        .unwrap();
    let mut show = encode_temporal_delimiter_obu().unwrap();
    show.extend(encode_show_existing_frame_obu(&coder.view, 0).unwrap());
    coder.references.show_existing(0).unwrap();
    assert!(coder.references.show_existing(0).is_err(), "shown twice");
    let expected = [
        recon_bytes(&key, config.bit_depth),
        recon_bytes(&hidden, config.bit_depth),
    ]
    .concat();
    (
        vec![key.temporal_unit, hidden.temporal_unit, show],
        expected,
    )
}

#[test]
fn hidden_frames_decode_through_show_existing_frames() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let config = config(72, 40, bit_depth, 6, 90);
        let (units, expected) = hidden_frame_stream(&config);
        assert_same_samples(&decode(&units.concat()), &expected, "hidden frame");
    }
}

/// AVM shows the hidden frame like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_shows_hidden_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let config = config(72, 40, BitDepth::Ten, 6, 90);
    let (units, expected) = hidden_frame_stream(&config);
    let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Ten, "hidden");
    assert!(avm == expected, "AVM output differs");
}
