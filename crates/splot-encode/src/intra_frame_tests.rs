// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use splot_core::headers::frame::build_key_frame_header;

use crate::config::RateControlMode;
use crate::encoder::Encoder;
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

/// Records the reconstruction of every coded frame by display index.
struct Recording {
    coder: IntraFrameCoder,
    recons: Arc<Mutex<BTreeMap<u64, [Vec<u16>; 3]>>>,
}

impl FrameCoder for Recording {
    fn encode_frame(
        &mut self,
        source: &SourceFrame,
        plan: &GopFrame,
        qp: u8,
    ) -> Result<CodedFrame> {
        let coded = self.coder.encode_frame(source, plan, qp)?;
        let recon = coded.recon.clone();
        self.recons
            .lock()
            .unwrap()
            .insert(plan.display_index, recon);
        Ok(coded)
    }

    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
        self.coder.show_existing_frame(slot)
    }
}

/// Codes `frames` frames through the [`Encoder`]; returns the temporal units
/// and the encoder reconstructions in display order.
fn encode_stream(config: &Config, frames: u32) -> (Vec<Vec<u8>>, Vec<u8>) {
    let recons = Arc::default();
    let coder = Recording {
        coder: IntraFrameCoder::new(config).unwrap(),
        recons: Arc::clone(&recons),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder));
    let mut packets = Vec::new();
    for seed in 0..=frames {
        if seed == frames {
            encoder.flush().unwrap();
        } else {
            encoder.send_frame(synthetic_frame(config, seed)).unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            packets.push(packet);
        }
    }
    let pts: Vec<u64> = packets.iter().map(|packet| packet.pts).collect();
    assert_eq!(pts, (0..u64::from(frames)).collect::<Vec<_>>());
    let coded: usize = packets.iter().map(|packet| packet.stats.len()).sum();
    assert_eq!(coded, frames as usize);
    let expected = recons
        .lock()
        .unwrap()
        .values()
        .flat_map(|recon| raw_bytes(recon.iter().map(Vec::as_slice), config.bit_depth))
        .collect();
    let units = packets.into_iter().map(|packet| packet.data).collect();
    (units, expected)
}

/// `(preset, keyint, frames, width, height, bit depth)`: presets 9, 6 and 3
/// code mini-GOPs of 2, 4 and 8 frames; no keyint is a multiple of them, and
/// the last mini-GOP of each stream is short.
const MINI_GOP_CASES: [(u8, u32, u32, u32, u32, BitDepth); 4] = [
    (9, 6, 9, 40, 24, BitDepth::Ten),
    (6, 7, 11, 33, 17, BitDepth::Eight),
    (6, 10, 8, 24, 16, BitDepth::Ten),
    (3, 11, 13, 24, 16, BitDepth::Eight),
];

fn mini_gop_stream(case: (u8, u32, u32, u32, u32, BitDepth)) -> (Vec<Vec<u8>>, Vec<u8>) {
    let (preset, keyint, frames, width, height, bit_depth) = case;
    let mut config = config(width, height, bit_depth, preset, 120);
    config.keyint = core::num::NonZeroU32::new(keyint).unwrap();
    encode_stream(&config, frames)
}

#[test]
fn mini_gops_decode_to_the_encoder_recon_in_display_order() {
    for case in MINI_GOP_CASES {
        let (units, expected) = mini_gop_stream(case);
        assert_same_samples(&decode(&units.concat()), &expected, &format!("{case:?}"));
    }
}

/// AVM decodes mini-GOP streams to the encoder reconstructions.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_mini_gops_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, case) in MINI_GOP_CASES.into_iter().enumerate() {
        let (units, expected) = mini_gop_stream(case);
        let (_, _, _, width, height, bit_depth) = case;
        let name = format!("gop{index}");
        let avm = avm_decode(&avmdec, &units, (width, height), bit_depth, &name);
        assert!(avm == expected, "{case:?}: AVM output differs");
    }
}
