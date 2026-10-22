// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use splot_core::headers::frame::build_key_frame_header;
use splot_decode::encoder_syntax::LeafBlock;

use crate::config::RateControlMode;
use crate::encoder::Encoder;
use crate::gop::GopFrame;
use crate::preset::Preset;
use crate::rate_control::FrameType;
use crate::test_support::{
    assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes, refs_for,
};
use crate::tile_writer::{
    InterMode, InterModes, LeafChoice, MotionVector, PARTITIONS, PartitionTree,
};

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

/// Key frames every third frame, with inter frames of unrelated content between them.
#[test]
fn inter_frames_decode_to_the_encoder_recon_over_several_gops() {
    for (index, (width, height, bit_depth, preset, qp)) in CASES.into_iter().enumerate() {
        let mut config = config(width, height, bit_depth, preset, qp);
        config.keyint = core::num::NonZeroU32::new(3).unwrap();
        let seeds: Vec<u32> = (0..4).map(|frame| index as u32 * 4 + frame).collect();
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
        let mut writer =
            KeyFrameTileWriter::new(&frame.sequence, &core, &refs_for(&frame.sequence)).unwrap();
        let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
        let sb = writer.syntax().superblock_size4() * 4;
        let padded = PaddedSource::new(&source, mi_cols * 4 + sb, mi_rows * 4 + sb);
        let lambda = Lambda::from_qindex(90, config.bit_depth).unwrap();
        let mut coder =
            SourceResidualCoder::new(&padded, 90, config.bit_depth, frame.preset, lambda);
        let mut search = || {
            writer
                .search_superblock(|syntax, root| {
                    let mut trial = TrialCoder::new(syntax, &mut coder, frame.preset, lambda, None);
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

/// Records the raw decoder output of every coded frame by display index.
struct Recording {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
}

impl FrameCoder for Recording {
    fn encode_frame(
        &mut self,
        source: &SourceFrame,
        plan: &GopFrame,
        qp: u8,
    ) -> Result<CodedFrame> {
        let coded = self.coder.encode_frame(source, plan, qp)?;
        let recon = recon_bytes(&coded, self.bit_depth);
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
        bit_depth: config.bit_depth,
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
    let expected = recons.lock().unwrap().values().flatten().copied().collect();
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

/// A stateless texture: a ripple, a ramp, and hashed noise at `(x, y)`.
fn texture(x: i64, y: i64, plane: usize, max: u32) -> u16 {
    let hash = (x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B)) as u32;
    let ripple = ((x * x / 7 + y * 5).rem_euclid(61)) as u32 * max / 160;
    let ramp = ((x + 2 * y + plane as i64 * 17).rem_euclid(300)) as u32 * max / 600;
    ((ripple + ramp + (hash >> 7) % (max / 10 + 1)) % (max + 1)) as u16
}

/// Frame `index` of a texture panning by `(4, 2)` luma samples per frame.
fn panning_frame(config: &Config, index: u32) -> SourceFrame {
    let max = u32::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        let shift = if plane == 0 { 1 } else { 2 };
        let (dx, dy) = (i64::from(4 * index) / shift, i64::from(2 * index) / shift);
        (0..size.height() as i64)
            .flat_map(|y| (0..size.width() as i64).map(move |x| (x, y)))
            .map(|(x, y)| texture(x + dx, y + dy, plane, max))
            .collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// A panning texture codes inter frames far below the key frame, and the
/// stream decodes to the encoder recon at both bit depths and superblock sizes.
#[test]
fn panning_content_codes_cheap_inter_frames() {
    for (width, height, bit_depth, preset) in
        [(96, 64, BitDepth::Eight, 8), (72, 40, BitDepth::Ten, 6)]
    {
        let config = config(width, height, bit_depth, preset, 100);
        let mut coder = IntraFrameCoder::new(&config).unwrap();
        let coded: Vec<CodedFrame> = (0..3)
            .map(|index| {
                let plan = crate::gop::plan(u64::from(index), config.keyint);
                coder
                    .encode_frame(&panning_frame(&config, index), &plan, 100)
                    .unwrap()
            })
            .collect();
        let name = format!("{width}x{height} {bit_depth:?} preset {preset}");
        let key = coded[0].temporal_unit.len();
        for frame in &coded[1..] {
            assert!(
                frame.temporal_unit.len() * 3 < key,
                "{name}: inter frame of {} bytes, key frame {key}",
                frame.temporal_unit.len()
            );
        }
        let stream: Vec<u8> = coded.iter().flat_map(|f| f.temporal_unit.clone()).collect();
        let expected: Vec<u8> = coded
            .iter()
            .flat_map(|f| recon_bytes(f, bit_depth))
            .collect();
        assert_same_samples(&decode(&stream), &expected, &name);
    }
}

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n
    }
}

/// Leaf counts of a random tree: intra, each inter mode, and inter leaves
/// with a reference other than the first.
type Coverage = [usize; 5];

/// A random partition tree of `node`: inter leaves of every mode, DRL index
/// and random vectors of up to 40 samples, and `DC_PRED` intra leaves.
fn random_tree(
    syntax: &mut IntraTileSyntax,
    node: PartitionNode,
    rng: &mut Rng,
    coverage: &mut Coverage,
) -> core::result::Result<PartitionTree, TileWriterError> {
    let options = syntax.partition_options(node)?;
    let mut partitions = Vec::new();
    for partition in PARTITIONS {
        if options.allows(partition) && codable_children(syntax, node, partition)? {
            partitions.push(partition);
        }
    }
    if let Some(implied) = options.implied().filter(|&p| options.allows(p)) {
        partitions = vec![implied];
    }
    let partition = if partitions.contains(&Partition::None) && rng.below(3) == 0 {
        Partition::None
    } else {
        partitions[rng.below(partitions.len() as u64) as usize]
    };
    if partition == Partition::None {
        let leaf = syntax.leaf(node)?;
        return Ok(PartitionTree::Leaf(random_leaf(
            syntax, &leaf, rng, coverage,
        )));
    }
    let mut children = Vec::new();
    for child in syntax.children(node, partition)? {
        children.push(if syntax.in_frame(child) {
            random_tree(syntax, child, rng, coverage)?
        } else {
            PartitionTree::Leaf(LeafChoice::DC)
        });
    }
    Ok(PartitionTree::Split {
        kind: partition,
        children,
    })
}

fn random_leaf(
    syntax: &IntraTileSyntax,
    leaf: &LeafBlock,
    rng: &mut Rng,
    coverage: &mut Coverage,
) -> LeafChoice {
    if !syntax.inter_allowed(leaf) || (syntax.intra_allowed(leaf) && rng.below(4) == 0) {
        coverage[0] += 1;
        return LeafChoice::DC;
    }
    let mode = [InterMode::Near, InterMode::Global, InterMode::New][rng.below(3) as usize];
    coverage[1 + mode as usize] += 1;
    let step = syntax.mv_step();
    let drl = rng.below(syntax.drl_count() as u64) as usize;
    let ref_frame = rng.below(syntax.reference_count() as u64) as usize;
    coverage[4] += usize::from(ref_frame > 0);
    let mut component = || (rng.below(81) as i32 - 40) * 8 + (rng.below(4) as i32) * step;
    LeafChoice::inter(InterModes {
        ref_frame,
        mode,
        drl,
        mv: MotionVector {
            row: component(),
            col: component(),
        },
    })
}

/// The first frames of a four-frame mini-GOP with frame 1, which references
/// frames 0, 2 and 4, coded from random partitions, modes, references and
/// vectors: the temporal units and the samples a decoder outputs.
fn random_inter_stream(config: &Config, seed: u64) -> (Vec<Vec<u8>>, Vec<u8>, Coverage) {
    let RateControlMode::ConstantQp { qp } = config.rate_control;
    let qindex = u32::from(qp);
    let mut coder = IntraFrameCoder::new(config).unwrap();
    let mut planner = crate::gop::GopPlanner::new(config.keyint, 4);
    let mut steps = planner.plan(1).unwrap();
    steps.extend(planner.plan(4).unwrap());
    let mut units = Vec::new();
    let mut expected = Vec::new();
    let mut plan = None;
    for step in steps {
        let crate::gop::GopStep::Code(frame) = step else {
            continue;
        };
        if frame.display_index == 1 {
            plan = Some(frame);
            break;
        }
        let coded = coder
            .encode_frame(
                &panning_frame(config, frame.display_index as u32),
                &frame,
                qp,
            )
            .unwrap();
        if frame.show_frame {
            expected.extend(recon_bytes(&coded, config.bit_depth));
        }
        units.push(coded.temporal_unit);
    }
    let plan = plan.unwrap();
    let tx = tx_choice::key_frame_tx_params(coder.preset);
    let filters = FrameFilterParams::default();
    let core = plan
        .header(&coder.view, &coder.references, qindex, &filters, tx)
        .unwrap();
    let mut writer = KeyFrameTileWriter::new(&coder.sequence, &core, &coder.references).unwrap();
    let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
    let sb = writer.syntax().superblock_size4() * 4;
    let source = panning_frame(config, 1);
    let padded = PaddedSource::new(&source, mi_cols * 4 + sb, mi_rows * 4 + sb);
    let lambda = Lambda::from_qindex(qindex, config.bit_depth).unwrap();
    let mut residual =
        SourceResidualCoder::new(&padded, qindex, config.bit_depth, coder.preset, lambda);
    let mut rng = Rng(seed);
    let mut coverage = Coverage::default();
    while writer.next_superblock_root().is_some() {
        let tree = writer
            .search_superblock(|syntax, root| random_tree(syntax, root, &mut rng, &mut coverage))
            .unwrap();
        writer
            .write_superblock(&SuperblockDecision { tree }, &mut residual)
            .unwrap();
    }
    let (tile, recon) = writer.finish().unwrap();
    let unit = plan
        .temporal_unit(
            &coder.sequence,
            &coder.view,
            &coder.references,
            (qindex, &filters, tx),
            &core,
            &tile,
        )
        .unwrap();
    let planes = visible_planes(&recon).unwrap();
    expected.extend(raw_bytes(
        planes.iter().map(Vec::as_slice),
        config.bit_depth,
    ));
    units.push(unit);
    (units, expected, coverage)
}

/// `(width, height, bit depth, preset, qp)` of the random inter streams: both
/// superblock sizes, both bit depths, frame edges, and a qp that skips.
const RANDOM_INTER_CASES: [(u32, u32, BitDepth, u8, u8); 3] = [
    (200, 104, BitDepth::Eight, 10, 90),
    (96, 72, BitDepth::Ten, 6, 160),
    (33, 17, BitDepth::Eight, 9, 250),
];

/// Random inter leaves over three references, with NEWMV vectors coded by the
/// MV writer, decode to the writer's reconstruction.
#[test]
fn random_inter_leaves_decode_to_the_writer_recon() {
    let mut total = Coverage::default();
    for (index, (width, height, bit_depth, preset, qp)) in
        RANDOM_INTER_CASES.into_iter().enumerate()
    {
        let config = config(width, height, bit_depth, preset, qp);
        let (units, expected, coverage) = random_inter_stream(&config, 0x5EED + index as u64);
        let name = format!("{width}x{height} {bit_depth:?} preset {preset} qp {qp}");
        assert_same_samples(&decode(&units.concat()), &expected, &name);
        for (sum, count) in total.iter_mut().zip(coverage) {
            *sum += count;
        }
    }
    assert!(total.iter().all(|&count| count >= 5), "coverage {total:?}");
}

/// AVM decodes random and searched inter frames like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_inter_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, (width, height, bit_depth, preset, qp)) in
        RANDOM_INTER_CASES.into_iter().enumerate()
    {
        let config = config(width, height, bit_depth, preset, qp);
        let (units, expected, _) = random_inter_stream(&config, 0x5EED + index as u64);
        let avm = avm_decode(
            &avmdec,
            &units,
            (width, height),
            bit_depth,
            &format!("random{index}"),
        );
        assert!(avm == expected, "random case {index}: AVM output differs");
        let mut coder = IntraFrameCoder::new(&config).unwrap();
        let units: Vec<Vec<u8>> = (0..4)
            .map(|frame| {
                let plan = crate::gop::plan(u64::from(frame), config.keyint);
                coder
                    .encode_frame(&panning_frame(&config, frame), &plan, qp)
                    .unwrap()
                    .temporal_unit
            })
            .collect();
        let avm = avm_decode(
            &avmdec,
            &units,
            (width, height),
            bit_depth,
            &format!("pan{index}"),
        );
        assert!(
            avm == decode(&units.concat()),
            "panning case {index}: AVM output differs"
        );
    }
}

/// An inter-frame superblock search from one state finds the same tree twice
/// and picks inter leaves for panning content: inter trials roll back too.
#[test]
fn inter_superblock_search_is_deterministic_and_rolls_back() {
    let config = config(64, 40, BitDepth::Ten, 9, 90);
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    coder
        .encode_frame(
            &panning_frame(&config, 0),
            &crate::gop::plan(0, config.keyint),
            90,
        )
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let tx = tx_choice::key_frame_tx_params(coder.preset);
    let core = plan
        .header(
            &coder.view,
            &coder.references,
            90,
            &FrameFilterParams::default(),
            tx,
        )
        .unwrap();
    let mut writer = KeyFrameTileWriter::new(&coder.sequence, &core, &coder.references).unwrap();
    let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
    let sb = writer.syntax().superblock_size4() * 4;
    let source = panning_frame(&config, 1);
    let padded = PaddedSource::new(&source, mi_cols * 4 + sb, mi_rows * 4 + sb);
    let lambda = Lambda::from_qindex(90, config.bit_depth).unwrap();
    let motion = MotionSearch::new(writer.syntax(), &padded, coder.preset, lambda)
        .unwrap()
        .unwrap();
    let mut residual =
        SourceResidualCoder::new(&padded, 90, config.bit_depth, coder.preset, lambda);
    let mut search = || {
        writer
            .search_superblock(|syntax, root| {
                let mut trial =
                    TrialCoder::new(syntax, &mut residual, coder.preset, lambda, Some(&motion));
                search_partition(&mut trial, root, coder.preset, lambda)
            })
            .unwrap()
    };
    let first = search();
    assert_eq!(first, search());
    assert!(inter_leaves(&first.0) > 0, "{:?}", first.0);
}

/// The inter leaves of `tree`.
fn inter_leaves(tree: &PartitionTree) -> usize {
    match tree {
        PartitionTree::Leaf(choice) => usize::from(choice.inter.is_some()),
        PartitionTree::Split { children, .. } => children.iter().map(inter_leaves).sum(),
    }
}
