// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, KeyFrameTxParams,
    build_encoder_sequence_header, build_key_frame_header, encode_key_frame_temporal_unit,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use std::collections::HashSet;

use splot_core::headers::frame::TxMode;
use splot_core::tables::conversion::TX_WIDTH;
use splot_decode::encoder_syntax::{
    IntraMode, IntraTileSyntax, LeafBlock, MAX_CFL_ALPHA, PartitionNode, TX_PARTITIONS,
    TransformBlock,
};
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
    tx: KeyFrameTxParams,
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
                cfl: true,
                mhccp: false,
                screen_content: false,
                ibp: true,
                mrls: false,
                fsc: false,
                dip: false,
                idtx_intra: false,
                intra_edge_filter: true,
                intra_ist: false,
                cctx: false,
                tcq: false,
                parity_hiding: false,
            },
            max_level: 12,
            tx: KeyFrameTxParams {
                tx_mode: TxMode::Largest,
                reduced_tx_set: 2,
            },
        }
    }

    /// `TX_MODE_SELECT` with every intra luma type, over every block shape.
    const fn with_tx_search(mut self) -> Self {
        self.tx = KeyFrameTxParams {
            tx_mode: TxMode::Select,
            reduced_tx_set: 0,
        };
        self.tools.ext_partitions = true;
        self.tools.uneven_4way_partitions = true;
        self
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

/// The luma transform choices one frame offered and coded.
#[derive(Default)]
struct TxCoverage {
    /// `(tx_size, offered types)`.
    offered: HashSet<(usize, Vec<u8>)>,
    /// `(tx_size, type)` coded with at least two nonzero levels, and how often.
    coded: std::collections::HashMap<(usize, u8), usize>,
    partitions: HashSet<u8>,
    /// `code` calls for luma: the writer's `eob == 1` recodes.
    recodes: usize,
    /// Pick luma modes that offer uncoded types.
    steer: bool,
}

/// Codes every transform block with random levels, the least-coded luma type,
/// and a random transform partition. When `coverage.steer` is set, it also
/// replaces the planned luma mode with one that offers uncoded luma types.
struct RandomLevels {
    rng: Rng,
    max_level: i32,
    coverage: TxCoverage,
}

impl RandomLevels {
    /// The number of luma types `tx` offers that this frame has not coded.
    fn uncoded(&self, syntax: &mut IntraTileSyntax, tx: &TransformBlock) -> Result<usize> {
        Ok(syntax
            .allowed_luma_tx_types(tx)?
            .into_iter()
            .filter(|&t| !self.coverage.coded.contains_key(&(tx.tx_size, t as u8)))
            .count())
    }
}

impl ResidualCoder for RandomLevels {
    fn luma_modes(
        &mut self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        planned: IntraModes,
    ) -> Result<IntraModes> {
        let mut best = (0, planned);
        if !self.coverage.steer {
            return Ok(best.1);
        }
        for (y_mode, angle_delta) in luma_modes() {
            let modes = IntraModes {
                y_mode,
                angle_delta,
                ..planned
            };
            let tx = syntax.plan_leaf(leaf, modes)?[0];
            let uncoded = self.uncoded(syntax, &tx)?;
            if uncoded > best.0 {
                best = (uncoded, modes);
            }
        }
        Ok(best.1)
    }

    fn tx_partition(
        &mut self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<TxPartition> {
        let options = syntax.luma_tx_partitions(leaf)?;
        let mut partition = options[self.rng.below(options.len() as u64) as usize];
        if self.coverage.steer && self.rng.below(2) == 0 {
            let mut best = 0;
            for &option in &options {
                let tx = syntax.plan_leaf_partitioned(leaf, modes, option)?[0];
                let uncoded = self.uncoded(syntax, &tx)?;
                if uncoded > best {
                    (best, partition) = (uncoded, option);
                }
            }
        }
        self.coverage.partitions.insert(partition as u8);
        Ok(partition)
    }

    fn code_luma(
        &mut self,
        tx: &TransformBlock,
        _: &[u16],
        tx_types: &[TxType],
    ) -> Result<TransformDecision> {
        let coded = &self.coverage.coded;
        let start = self.rng.below(tx_types.len() as u64) as usize;
        let tx_type = *tx_types
            .iter()
            .cycle()
            .skip(start)
            .take(tx_types.len())
            .min_by_key(|&&t| coded.get(&(tx.tx_size, t as u8)).copied().unwrap_or(0))
            .unwrap();
        let mut levels = random_levels(&mut self.rng, tx.tx_size, self.max_level);
        match self.rng.below(4) {
            0 => {
                levels.fill(0);
                levels[0] = 1;
            }
            1 | 2 => {
                levels[0] = levels[0].max(1);
                levels[1] = levels[1].min(-1);
            }
            _ => {}
        }
        let types = tx_types.iter().map(|&t| t as u8).collect();
        self.coverage.offered.insert((tx.tx_size, types));
        if levels.iter().filter(|&&level| level != 0).count() >= 2 {
            *self
                .coverage
                .coded
                .entry((tx.tx_size, tx_type as u8))
                .or_default() += 1;
        }
        Ok(TransformDecision { tx_type, levels })
    }

    fn code(&mut self, tx: &TransformBlock, _: &[u16]) -> Result<TransformDecision> {
        self.coverage.recodes += usize::from(tx.plane == 0);
        Ok(TransformDecision {
            tx_type: tx.tx_type,
            levels: random_levels(&mut self.rng, tx.tx_size, self.max_level),
        })
    }
}

/// Every `(YMode, AngleDeltaY)` pair: the 61 § 5.20.5.5 joint modes.
fn luma_modes() -> Vec<(IntraMode, i8)> {
    INTRA_MODES
        .into_iter()
        .flat_map(|mode| {
            let deltas = if mode.is_directional() { -3..=3 } else { 0..=0 };
            deltas.map(move |delta| (mode, delta))
        })
        .collect()
}

/// Cycles through every luma mode and every chroma mode; CFL alphas are random.
struct ModePicker {
    rng: Rng,
    next: usize,
    luma: Vec<(IntraMode, i8)>,
    coded_luma: HashSet<(u8, i8)>,
    coded_chroma: HashSet<u8>,
}

impl ModePicker {
    fn new(seed: u64) -> Self {
        let luma = luma_modes();
        Self {
            next: seed as usize * 23 % luma.len(),
            rng: Rng(seed.rotate_left(31) | 1),
            luma,
            coded_luma: HashSet::new(),
            coded_chroma: HashSet::new(),
        }
    }

    fn pick(&mut self, cfl_allowed: bool) -> IntraModes {
        let (y_mode, angle_delta) = self.luma[self.next % self.luma.len()];
        let uv = self.next % (INTRA_MODES.len() + 2);
        self.next += 1;
        let uv_mode = match uv {
            _ if uv < INTRA_MODES.len() => ChromaMode::Intra(INTRA_MODES[uv]),
            _ if !cfl_allowed => ChromaMode::Intra(INTRA_MODES[self.rng.below(13) as usize]),
            13 => loop {
                let span = 2 * MAX_CFL_ALPHA as u64 + 1;
                let alpha_u = self.rng.below(span) as i8 - MAX_CFL_ALPHA;
                let alpha_v = self.rng.below(span) as i8 - MAX_CFL_ALPHA;
                if alpha_u != 0 || alpha_v != 0 {
                    break ChromaMode::Cfl { alpha_u, alpha_v };
                }
            },
            _ => ChromaMode::CflDerived,
        };
        self.coded_luma.insert((y_mode as u8, angle_delta));
        self.coded_chroma.insert(match uv_mode {
            ChromaMode::Intra(mode) => mode as u8,
            ChromaMode::Cfl { .. } => 13,
            ChromaMode::CflDerived => 14,
        });
        IntraModes {
            y_mode,
            angle_delta,
            uv_mode,
        }
    }
}

fn random_tree(
    syntax: &IntraTileSyntax,
    node: PartitionNode,
    rng: &mut Rng,
    modes: &mut ModePicker,
) -> PartitionTree {
    if !syntax.in_frame(node) {
        return PartitionTree::Leaf(IntraModes::DC);
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
        let leaf = syntax.leaf(node).unwrap();
        let cfl_allowed = syntax.cfl_allowed(&leaf);
        return PartitionTree::Leaf(modes.pick(cfl_allowed));
    }
    let children = syntax
        .children(node, partition)
        .unwrap()
        .into_iter()
        .map(|child| random_tree(syntax, child, rng, modes))
        .collect();
    PartitionTree::Split {
        kind: partition,
        children,
    }
}

/// Encodes one random key frame; returns the Annex B temporal unit, the recon
/// frame, and the modes it coded.
/// Encodes one random key frame; `coverage` carries the luma type coverage
/// across frames.
fn encode_case(
    case: Case,
    seed: u64,
    coverage: TxCoverage,
) -> (Vec<u8>, DecodedFrame<u16>, ModePicker, TxCoverage) {
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
    let core = build_key_frame_header(
        &view,
        case.base_q_idx,
        0,
        &FrameFilterParams::default(),
        case.tx,
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core).unwrap();
    let mut rng = Rng(seed | 1);
    let mut modes = ModePicker::new(seed);
    let mut coder = RandomLevels {
        rng: Rng(seed.rotate_left(17) | 1),
        max_level: case.max_level,
        coverage: TxCoverage {
            steer: case.tx.tx_mode == TxMode::Select,
            ..coverage
        },
    };
    while let Some(root) = writer.next_superblock_root() {
        let tree = random_tree(writer.syntax(), root, &mut rng, &mut modes);
        writer
            .write_superblock(&SuperblockDecision { tree }, &mut coder)
            .unwrap();
    }
    let (tile, recon) = writer.finish().unwrap();
    let unit = encode_key_frame_temporal_unit(&seq, &core, true, &tile).unwrap();
    (unit, recon, modes, coder.coverage)
}

fn assert_round_trip(case: Case, seed: u64) -> Vec<u8> {
    assert_round_trip_modes(case, seed, TxCoverage::default()).0
}

fn assert_round_trip_modes(
    case: Case,
    seed: u64,
    coverage: TxCoverage,
) -> (Vec<u8>, ModePicker, TxCoverage) {
    let (unit, recon, modes, coverage) = encode_case(case, seed, coverage);
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
    (unit, modes, coverage)
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
fn every_intra_mode_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
            let mut luma = HashSet::new();
            let mut chroma = HashSet::new();
            for seed in 0..4 {
                let case = Case::new(136, 72, bit_depth, sb_size);
                let (_, modes, _) = assert_round_trip_modes(case, seed, TxCoverage::default());
                luma.extend(modes.coded_luma);
                chroma.extend(modes.coded_chroma);
            }
            assert_eq!(luma.len(), 61, "{bit_depth:?} {sb_size:?}");
            assert_eq!(chroma.len(), 15, "{bit_depth:?} {sb_size:?}");
        }
    }
}

/// Every luma type each transform size offers, every transform partition, and
/// the `eob == 1` recode round-trip at both bit depths.
#[test]
fn every_luma_tx_type_and_partition_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut coverage = TxCoverage::default();
        for seed in 0..8 {
            let sb_size = if seed % 2 == 0 {
                SuperblockSize::Block64x64
            } else {
                SuperblockSize::Block128x128
            };
            let case = Case::new(264, 200, bit_depth, sb_size).with_tx_search();
            coverage = assert_round_trip_modes(case, 0x7e00 + seed, coverage).2;
        }
        let sizes: HashSet<usize> = coverage.offered.iter().map(|(size, _)| *size).collect();
        assert_eq!(sizes.len(), TX_WIDTH.len(), "{bit_depth:?}: {sizes:?}");
        for (size, types) in &coverage.offered {
            for &tx_type in types {
                assert!(
                    coverage.coded.contains_key(&(*size, tx_type)),
                    "{bit_depth:?}: tx size {size} type {tx_type} never coded"
                );
            }
        }
        assert!(coverage.offered.iter().any(|(_, types)| types.len() == 7));
        assert_eq!(
            coverage.partitions.len(),
            TX_PARTITIONS.len(),
            "{bit_depth:?}"
        );
        assert!(coverage.recodes > 0, "{bit_depth:?}: no eob == 1 recode");
    }
}

#[test]
fn reduced_transform_sets_round_trip() {
    for reduced_tx_set in 0..=3 {
        let mut case = Case::new(130, 66, BitDepthIdc::Ten, SuperblockSize::Block64x64);
        case.tx.reduced_tx_set = reduced_tx_set;
        case.tx.tx_mode = TxMode::Select;
        assert_round_trip(case, u64::from(reduced_tx_set) + 40);
    }
}

#[test]
fn intra_prediction_tools_off_round_trip() {
    let mut case = Case::new(130, 66, BitDepthIdc::Ten, SuperblockSize::Block128x128);
    case.tools.cfl = false;
    case.tools.ibp = false;
    case.tools.intra_edge_filter = false;
    assert_round_trip(case, 11);
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
        KeyFrameTxParams::default(),
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core).unwrap();
    let tree = PartitionTree::Leaf(IntraModes::DC);
    assert!(matches!(
        writer.write_superblock(
            &SuperblockDecision { tree },
            &mut RandomLevels {
                rng: Rng(1),
                max_level: 1,
                coverage: TxCoverage::default(),
            }
        ),
        Err(TileWriterError::Partition { .. })
    ));

    let mut tools = case.tools;
    tools.mhccp = true;
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
        KeyFrameTxParams::default(),
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
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
            cases.push(Case::new(200, 136, bit_depth, sb_size).with_tx_search());
        }
        for reduced_tx_set in [1, 3] {
            let mut case = Case::new(130, 66, bit_depth, SuperblockSize::Block64x64);
            case.tx.tx_mode = TxMode::Select;
            case.tx.reduced_tx_set = reduced_tx_set;
            cases.push(case);
        }
    }
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
