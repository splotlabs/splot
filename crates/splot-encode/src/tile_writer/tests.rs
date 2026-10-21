// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams,
    build_encoder_sequence_header, build_key_frame_header, encode_key_frame_temporal_unit,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use splot_decode::encoder_syntax::{IntraTileSyntax, PartitionNode, TransformBlock};
use splot_recon::{BitDepth, DecodedFrame, PlaneId};

use super::*;
use crate::test_support::{assert_same_samples, avm_decode, avm_decoder, decode, raw_bytes};

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Clone, Copy)]
struct Case {
    width: u32,
    height: u32,
    bit_depth: BitDepthIdc,
    sb_size: SuperblockSize,
    base_q_idx: u32,
    tools: EncoderToolFlags,
    max_level: i32,
}

impl Case {
    const fn new(width: u32, height: u32, bit_depth: BitDepthIdc, sb_size: SuperblockSize) -> Self {
        Self {
            width,
            height,
            bit_depth,
            sb_size,
            base_q_idx: 90,
            tools: EncoderToolFlags {
                cdef: false,
                loop_restoration: false,
                ccso: false,
                gdf: false,
                sdp: false,
                extended_sdp: false,
                ext_partitions: false,
                uneven_4way_partitions: false,
                cfl: false,
                mhccp: false,
                screen_content: false,
                ibp: false,
                mrls: false,
                fsc: false,
                dip: false,
                idtx_intra: false,
                intra_edge_filter: false,
                intra_ist: false,
                cctx: false,
                tcq: false,
                parity_hiding: false,
            },
            max_level: 12,
        }
    }

    fn bit_depth(self) -> BitDepth {
        match self.bit_depth {
            BitDepthIdc::Eight => BitDepth::Eight,
            BitDepthIdc::Ten => BitDepth::Ten,
        }
    }
}

fn random_levels(rng: &mut Rng, tx_size: usize, max_level: i32) -> Vec<i32> {
    let (log2_w, log2_h) = coeffs::adjusted_log2(tx_size).unwrap();
    let count = 1usize << (log2_w + log2_h);
    let mut levels = vec![0; count];
    if rng.below(2) == 0 {
        return levels;
    }
    let density = 1 + rng.below(3);
    for (pos, level) in levels.iter_mut().enumerate() {
        let near_dc = pos < 8;
        if rng.below(16) < density || (near_dc && rng.below(2) == 0) {
            let magnitude = if rng.below(8) == 0 {
                1 + rng.below(max_level as u64) as i32
            } else {
                1 + rng.below(3) as i32
            };
            *level = if rng.below(2) == 0 {
                magnitude
            } else {
                -magnitude
            };
        }
    }
    levels
}

/// Codes every transform block with random levels.
struct RandomLevels {
    rng: Rng,
    max_level: i32,
}

impl ResidualCoder for RandomLevels {
    fn code(&mut self, tx: &TransformBlock, _: &[u16]) -> Result<TransformDecision> {
        Ok(TransformDecision {
            tx_type: TxType::DctDct,
            levels: random_levels(&mut self.rng, tx.tx_size, self.max_level),
        })
    }
}

fn random_tree(syntax: &IntraTileSyntax, node: PartitionNode, rng: &mut Rng) -> PartitionTree {
    if !syntax.in_frame(node) {
        return PartitionTree::Leaf(BlockDecision::DC);
    }
    let options = syntax.partition_options(node).unwrap();
    let allowed: Vec<Partition> = PARTITIONS
        .into_iter()
        .filter(|&p| options.allows(p))
        .collect();
    let partition = match options.implied().filter(|&p| options.allows(p)) {
        Some(implied) => implied,
        None if allowed.len() == 1 => allowed[0],
        None if node.block_size() >= 9 && options.allows(Partition::Split) && rng.below(3) != 0 => {
            Partition::Split
        }
        None => allowed[rng.below(allowed.len() as u64) as usize],
    };
    if partition == Partition::None {
        return PartitionTree::Leaf(BlockDecision::DC);
    }
    let children = syntax
        .children(node, partition)
        .unwrap()
        .into_iter()
        .map(|child| random_tree(syntax, child, rng))
        .collect();
    PartitionTree::Split {
        kind: partition,
        children,
    }
}

/// Encodes one random key frame; returns the Annex B temporal unit and the recon frame.
fn encode_case(case: Case, seed: u64) -> (Vec<u8>, DecodedFrame<u16>) {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: case.width,
        height: case.height,
        bit_depth: case.bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: case.sb_size,
        tools: case.tools,
    })
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core =
        build_key_frame_header(&view, case.base_q_idx, 0, &FrameFilterParams::default()).unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core).unwrap();
    let mut rng = Rng(seed | 1);
    let mut coder = RandomLevels {
        rng: Rng(seed.rotate_left(17) | 1),
        max_level: case.max_level,
    };
    while let Some(root) = writer.next_superblock_root() {
        let tree = random_tree(writer.syntax(), root, &mut rng);
        writer
            .write_superblock(&SuperblockDecision { tree }, &mut coder)
            .unwrap();
    }
    let (tile, recon) = writer.finish().unwrap();
    let unit = encode_key_frame_temporal_unit(&seq, &core, true, &tile).unwrap();
    (unit, recon)
}

fn assert_round_trip(case: Case, seed: u64) -> Vec<u8> {
    let (unit, recon) = encode_case(case, seed);
    let view = recon.as_frame_ref();
    let expected = raw_bytes(
        [PlaneId::Y, PlaneId::U, PlaneId::V]
            .into_iter()
            .flat_map(|plane| view.plane(plane).unwrap().visible_rows()),
        case.bit_depth(),
    );
    let name = format!(
        "{}x{} {:?} sb {:?} seed {seed}",
        case.width, case.height, case.bit_depth, case.sb_size
    );
    assert_same_samples(&decode(&unit), &expected, &name);
    unit
}

const SIZES: [(u32, u32); 5] = [(1, 1), (17, 9), (64, 64), (65, 65), (130, 66)];

#[test]
fn random_key_frames_decode_to_the_encoder_recon() {
    for (index, (width, height)) in SIZES.into_iter().enumerate() {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
                let case = Case::new(width, height, bit_depth, sb_size);
                assert_round_trip(case, 0x5eed_0000 + index as u64);
            }
        }
    }
}

#[test]
fn quantizer_and_golomb_extremes_round_trip() {
    for (base_q_idx, max_level) in [(1, 3000), (40, 400), (200, 20), (255, 6)] {
        let mut case = Case::new(40, 24, BitDepthIdc::Eight, SuperblockSize::Block64x64);
        case.base_q_idx = base_q_idx;
        case.max_level = max_level;
        assert_round_trip(case, u64::from(base_q_idx));
    }
    let mut case = Case::new(72, 40, BitDepthIdc::Ten, SuperblockSize::Block128x128);
    case.base_q_idx = 500;
    assert_round_trip(case, 7);
}

#[test]
fn extended_partitions_and_frame_filter_tools_round_trip() {
    let mut case = Case::new(130, 66, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    case.tools.ext_partitions = true;
    case.tools.uneven_4way_partitions = true;
    case.tools.cdef = true;
    case.tools.gdf = true;
    case.tools.ccso = true;
    case.tools.loop_restoration = true;
    case.tools.intra_edge_filter = true;
    for seed in 1..3 {
        assert_round_trip(case, seed);
    }
}

#[test]
fn rejects_unsupported_decisions() {
    let case = Case::new(16, 16, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: case.width,
        height: case.height,
        bit_depth: case.bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: case.sb_size,
        tools: case.tools,
    })
    .unwrap();
    let core = build_key_frame_header(
        &CoreSeqView::from_sequence(&seq).unwrap(),
        50,
        0,
        &FrameFilterParams::default(),
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core).unwrap();
    let tree = PartitionTree::Leaf(BlockDecision::DC);
    assert!(matches!(
        writer.write_superblock(
            &SuperblockDecision { tree },
            &mut RandomLevels {
                rng: Rng(1),
                max_level: 1
            }
        ),
        Err(TileWriterError::Partition { .. })
    ));

    let mut tools = case.tools;
    tools.cfl = true;
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 16,
        height: 16,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools,
    })
    .unwrap();
    let core = build_key_frame_header(
        &CoreSeqView::from_sequence(&seq).unwrap(),
        50,
        0,
        &FrameFilterParams::default(),
    )
    .unwrap();
    assert!(matches!(
        KeyFrameTileWriter::new(&seq, &core),
        Err(TileWriterError::Unsupported(_))
    ));
}

/// Decodes random streams with AVM and compares them with splot's output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_random_key_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let mut cases: Vec<Case> = Vec::new();
    for (width, height) in SIZES.into_iter().chain([(200, 136), (352, 288)]) {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
                cases.push(Case::new(width, height, bit_depth, sb_size));
            }
        }
    }
    let mut extended = Case::new(150, 98, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    extended.tools.ext_partitions = true;
    extended.tools.uneven_4way_partitions = true;
    cases.push(extended);
    for (base_q_idx, max_level) in [(1, 3000), (40, 400), (200, 20), (255, 6)] {
        let mut case = Case::new(40, 24, BitDepthIdc::Eight, SuperblockSize::Block64x64);
        case.base_q_idx = base_q_idx;
        case.max_level = max_level;
        cases.push(case);
    }
    for (index, case) in cases.into_iter().enumerate() {
        let unit = assert_round_trip(case, 0xa0 + index as u64);
        let avm = avm_decode(
            &avmdec,
            std::slice::from_ref(&unit),
            (case.width, case.height),
            case.bit_depth(),
            &format!("writer{index}"),
        );
        assert!(
            avm == decode(&unit),
            "case {index} ({}x{} {:?} {:?}): AVM output differs",
            case.width,
            case.height,
            case.bit_depth,
            case.sb_size
        );
    }
}
