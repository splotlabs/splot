// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::test_support::refs_for;
use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, KeyFrameTxParams,
    UniformTiles, build_encoder_sequence_header, build_key_frame_header,
    encode_key_frame_temporal_unit,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use std::collections::HashSet;

use splot_core::headers::frame::TxMode;
use splot_core::tables::conversion::TX_WIDTH;
use splot_decode::encoder_syntax::{
    ChromaMode, Dip, EncoderSyntaxError, IST_SETS, IntraMode, IntraModes, IntraTileSyntax, Ist,
    LeafBlock, MAX_CFL_ALPHA, Mrl, PALETTE_COLORS, Palette, PartitionNode, TX_PARTITIONS,
    TransformBlock, coefficient_scan,
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
    chroma: ChromaFormatIdc,
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
                cfl_ds_filter: 0,
                screen_content: false,
                ibp: true,
                mrls: false,
                fsc: false,
                dip: false,
                idtx_intra: false,
                intra_edge_filter: true,
                intra_ist: false,
                inter_ddt: false,
                inter_ist: false,
                tip: false,
                cctx: false,
                tcq: false,
                parity_hiding: false,
                avg_cdf: None,
                ref_frame_mvs: false,
                refmvbank: false,
                drl_reorder: None,
                max_drl_bits_minus_1: 0,
                mvd_tools: [false; 3],
            },
            max_level: 12,
            chroma: ChromaFormatIdc::Yuv420,
            tx: KeyFrameTxParams {
                tx_mode: TxMode::Largest,
                reduced_tx_set: 2,
                allow_tcq: false,
                delta_q_res: None,
                allow_intrabc: false,
            },
        }
    }

    /// `TX_MODE_SELECT` with every intra luma type, over every block shape.
    const fn with_tx_search(mut self) -> Self {
        self.tx = KeyFrameTxParams {
            tx_mode: TxMode::Select,
            reduced_tx_set: 0,
            ..self.tx
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
    /// Luma blocks that hide a parity: `sumAbs1` 0 and 1, a remainder on
    /// the hidden level, a hidden `quant` of 1, and a one-dimensional type.
    hidden: [usize; 5],
    /// `(sec_tx_type, most_probable_stx_set, primary type)` of IST blocks.
    ist: HashSet<(u8, u8, u8)>,
    /// `cctx_type` values of coded chroma pairs.
    cctx: HashSet<u8>,
}

/// Moves the scan index 0 level of `levels` by one when they hide a parity
/// it does not have, and counts the hidden blocks.
fn keep_parity(
    syntax: &IntraTileSyntax,
    tx: &TransformBlock,
    levels: &mut [i32],
    rng: &mut Rng,
    hidden: &mut [usize; 5],
) -> Result<()> {
    let Some(parity) = syntax.parity_hiding(tx) else {
        return Ok(());
    };
    let first = usize::from(coefficient_scan(tx)?[0]);
    if !parity.holds(levels)? {
        let magnitude = levels[first].unsigned_abs();
        let step = if magnitude == 0 || rng.below(2) == 0 {
            1
        } else {
            -1
        };
        let sign = if levels[first] < 0 { -1 } else { 1 };
        levels[first] = sign * (magnitude as i32 + step);
    }
    if let Some(sum) = parity.sum_abs1(levels)? {
        let magnitude = levels[first].unsigned_abs();
        hidden[sum as usize] += 1;
        hidden[2] += usize::from(magnitude >> 1 >= 3);
        hidden[3] += usize::from(magnitude == 1);
        hidden[4] += usize::from(tx.tx_type as u8 > TxType::Idtx as u8);
    }
    Ok(())
}

/// Codes every transform block with random levels, the least-coded luma type,
/// and a random transform partition. When `coverage.steer` is set, it also
/// replaces the planned luma mode with one that offers uncoded luma types.
struct RandomLevels {
    rng: Rng,
    max_level: i32,
    coverage: TxCoverage,
    /// The color map of the last palette leaf.
    map: Vec<u8>,
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

    /// `planned`, steered to a luma mode that offers uncoded types, with a
    /// random or steered transform partition.
    fn leaf_choice(
        &mut self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        planned: IntraModes,
    ) -> Result<LeafChoice> {
        let mut best = (0, planned);
        let steer = self.coverage.steer && planned.palette.is_none() && !leaf.is_chroma_part();
        if steer {
            for (y_mode, angle_delta) in luma_modes() {
                let modes = IntraModes {
                    y_mode,
                    angle_delta,
                    mrl: if y_mode.is_directional() {
                        planned.mrl
                    } else {
                        Mrl::OFF
                    },
                    ..planned
                };
                let tx = syntax.plan_leaf(leaf, modes)?[0];
                let uncoded = self.uncoded(syntax, &tx)?;
                if uncoded > best.0 {
                    best = (uncoded, modes);
                }
            }
        }
        let modes = best.1;
        let options = syntax.luma_tx_partitions(leaf)?;
        let mut partition = options[self.rng.below(options.len() as u64) as usize];
        if steer && self.rng.below(2) == 0 {
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
        Ok(LeafChoice {
            modes,
            tx_partition: partition,
            inter: None,
            intrabc: None,
        })
    }
}

impl ResidualCoder for RandomLevels {
    /// Random lines of indices, some uniform or copies of the line before,
    /// along rows or, for a third of the leaves, along columns.
    fn palette_map(&mut self, leaf: &LeafBlock, palette: &Palette) -> Result<&[u8]> {
        let (w, h) = leaf.dimensions()?;
        let size = u64::from(palette.size);
        let columns = self.rng.below(3) == 0;
        let (outer, inner) = if columns { (w, h) } else { (h, w) };
        let mut lines: Vec<Vec<u8>> = Vec::with_capacity(outer);
        for i in 0..outer {
            let line = match self.rng.below(4) {
                0 if i > 0 => lines[i - 1].clone(),
                1 => vec![self.rng.below(size) as u8; inner],
                _ => (0..inner).map(|_| self.rng.below(size) as u8).collect(),
            };
            lines.push(line);
        }
        self.map = (0..w * h)
            .map(|i| {
                let (y, x) = (i / w, i % w);
                if columns { lines[x][y] } else { lines[y][x] }
            })
            .collect();
        Ok(&self.map)
    }

    fn code_luma(
        &mut self,
        syntax: &mut IntraTileSyntax,
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
        let mut coded_tx = TransformBlock { tx_type, ..*tx };
        if syntax.ist_coded(&coded_tx, 2)? && self.rng.below(2) == 0 {
            coded_tx.ist = Ist {
                sec_tx_type: 1 + self.rng.below(3) as u8,
                set: self.rng.below(u64::from(syntax.ist_sets(&coded_tx)?)) as u8,
            };
            let n = syntax.secondary_transform(&coded_tx)?.map_or(0, |p| p.n);
            let scan = coefficient_scan(&coded_tx)?;
            for &pos in &scan[n..] {
                levels[usize::from(pos)] = 0;
            }
            levels[usize::from(scan[1])] |= 1;
            let ist = (coded_tx.ist.sec_tx_type, coded_tx.ist.set, tx_type as u8);
            self.coverage.ist.insert(ist);
        }
        let hidden = &mut self.coverage.hidden;
        keep_parity(syntax, &coded_tx, &mut levels, &mut self.rng, hidden)?;
        let types = tx_types.iter().map(|&t| t as u8).collect();
        self.coverage.offered.insert((tx.tx_size, types));
        if levels.iter().filter(|&&level| level != 0).count() >= 2 {
            *self
                .coverage
                .coded
                .entry((tx.tx_size, tx_type as u8))
                .or_default() += 1;
        }
        Ok(TransformDecision {
            tx_type,
            ist: coded_tx.ist,
            cctx: 0,
            levels,
        })
    }

    fn code(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        _: &[u16],
    ) -> Result<TransformDecision> {
        self.coverage.recodes += usize::from(tx.plane == 0);
        let mut levels = random_levels(&mut self.rng, tx.tx_size, self.max_level);
        let hidden = &mut self.coverage.hidden;
        keep_parity(syntax, tx, &mut levels, &mut self.rng, hidden)?;
        Ok(TransformDecision {
            tx_type: tx.tx_type,
            ist: tx.ist,
            cctx: 0,
            levels,
        })
    }

    /// A random `cctx_type` with U levels of at least two coefficients when
    /// the pair may code CCTX.
    fn code_chroma_pair(
        &mut self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        [u_prediction, v_prediction]: [&[u16]; 2],
    ) -> Result<[TransformDecision; 2]> {
        let mut decisions = [
            self.code(syntax, u, u_prediction)?,
            self.code(syntax, v, v_prediction)?,
        ];
        if syntax.cctx_coded(u, 2) && u.tx_type == v.tx_type {
            let cctx = self.rng.below(7) as u8;
            let scan = coefficient_scan(u)?;
            let levels = &mut decisions[0].levels;
            for &pos in &scan[..2] {
                levels[usize::from(pos)] |= 1;
            }
            decisions[0].cctx = cctx;
            self.coverage.cctx.insert(cctx);
        }
        Ok(decisions)
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

/// Cycles through every luma mode and every chroma mode; CFL alphas and,
/// with `mrl`, the MRL lines of directional modes are random.
struct ModePicker {
    rng: Rng,
    next: usize,
    luma: Vec<(IntraMode, i8)>,
    coded_luma: HashSet<(u8, i8)>,
    coded_chroma: HashSet<u8>,
    mrl: bool,
    mhccp: bool,
    fsc: bool,
    coded_fsc: bool,
    dip: bool,
    coded_dip: HashSet<Dip>,
    coded_mrl: HashSet<Mrl>,
    palette: bool,
    coded_palette: HashSet<Palette>,
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
            mrl: false,
            mhccp: false,
            fsc: false,
            coded_fsc: false,
            dip: false,
            coded_dip: HashSet::new(),
            coded_mrl: HashSet::new(),
            palette: false,
            coded_palette: HashSet::new(),
        }
    }

    fn pick(
        &mut self,
        cfl_allowed: bool,
        mhccp_allowed: bool,
        fsc_allowed: bool,
        dip_modes: u8,
    ) -> IntraModes {
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
        let uv_mode = if self.mhccp && mhccp_allowed && self.rng.below(3) == 0 {
            ChromaMode::Mhccp {
                direction: self.rng.below(3) as u8,
            }
        } else {
            uv_mode
        };
        self.coded_luma.insert((y_mode as u8, angle_delta));
        self.coded_chroma.insert(match uv_mode {
            ChromaMode::Intra(mode) => mode as u8,
            ChromaMode::Cfl { .. } => 13,
            ChromaMode::CflDerived => 14,
            ChromaMode::Mhccp { direction } => 15 + direction,
        });
        let mut mrl = Mrl::OFF;
        if self.mrl && y_mode.is_directional() {
            let index = self.rng.below(4) as u8;
            mrl = Mrl {
                index,
                secondary: index > 0 && self.rng.below(2) == 0,
            };
            self.coded_mrl.insert(mrl);
        }
        let fsc = self.fsc && fsc_allowed && self.rng.below(2) == 0;
        self.coded_fsc |= fsc;
        if dip_modes > 0 && self.rng.below(2) == 0 {
            let dip = Dip {
                mode: self.rng.below(u64::from(dip_modes)) as u8,
                transpose: self.rng.below(2) == 0,
            };
            self.coded_dip.insert(dip);
            let uv_mode = match uv_mode {
                ChromaMode::Intra(_) => ChromaMode::Intra(IntraMode::Dc),
                cfl => cfl,
            };
            return IntraModes {
                uv_mode,
                fsc,
                dip: Some(dip),
                ..IntraModes::DC
            };
        }
        IntraModes {
            y_mode,
            angle_delta,
            uv_mode,
            mrl,
            fsc,
            dip: None,
            palette: None,
        }
    }
}

fn random_tree(
    syntax: &mut IntraTileSyntax,
    node: PartitionNode,
    rng: &mut Rng,
    modes: &mut ModePicker,
    coder: &mut RandomLevels,
) -> PartitionTree {
    if !syntax.in_frame(node) {
        return PartitionTree::Leaf(LeafChoice::DC);
    }
    if let Some((luma_root, chroma_root)) = syntax.sdp_roots(node) {
        let luma = random_tree(syntax, luma_root, rng, modes, coder);
        let mut sink = crate::rd::coeff_rate::RateSink::adapting();
        Walk::trial(syntax, &mut sink)
            .write_node(luma_root, &luma, coder)
            .unwrap();
        let chroma = random_tree(syntax, chroma_root, rng, modes, coder);
        return PartitionTree::Sdp {
            luma: Box::new(luma),
            chroma: Box::new(chroma),
        };
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
        let dc = IntraModes::DC;
        let dip_modes = if modes.dip && syntax.dip_allowed(&leaf, dc) {
            syntax.dip_modes().unwrap()
        } else {
            0
        };
        let mut planned = modes.pick(
            cfl_allowed,
            syntax.mhccp_allowed(&leaf),
            syntax.fsc_allowed(&leaf),
            dip_modes,
        );
        if modes.palette && syntax.palette_allowed(&leaf) && rng.below(2) == 0 {
            planned = random_palette(rng, planned);
            modes.coded_palette.extend(planned.palette);
        }
        return PartitionTree::Leaf(coder.leaf_choice(syntax, &leaf, planned).unwrap());
    }
    let children = syntax
        .children(node, partition)
        .unwrap()
        .into_iter()
        .map(|child| random_tree(syntax, child, rng, modes, coder))
        .collect();
    PartitionTree::Split {
        kind: partition,
        children,
    }
}

/// A `DC_PRED` palette of two to eight colors from a small pool, so
/// neighbours share colors through the cache, with the chroma and FSC of
/// `planned`.
fn random_palette(rng: &mut Rng, planned: IntraModes) -> IntraModes {
    let mut pool = [3u16, 12, 40, 41, 77, 99, 128, 129, 200, 201, 230, 250];
    let size = 2 + rng.below(7) as usize;
    for i in 0..size {
        let j = i + rng.below((pool.len() - i) as u64) as usize;
        pool.swap(i, j);
    }
    let mut colors = [0; PALETTE_COLORS];
    colors[..size].copy_from_slice(&pool[..size]);
    colors[..size].sort_unstable();
    IntraModes {
        uv_mode: match planned.uv_mode {
            ChromaMode::Intra(_) => ChromaMode::Intra(IntraMode::Dc),
            cfl => cfl,
        },
        fsc: planned.fsc,
        palette: Some(Palette {
            size: size as u8,
            colors,
        }),
        ..IntraModes::DC
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
        chroma: case.chroma,
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
        UniformTiles::default(),
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let mut rng = Rng(seed | 1);
    let mut modes = ModePicker::new(seed);
    modes.mrl = case.tools.mrls;
    modes.mhccp = case.tools.mhccp;
    modes.fsc = case.tools.fsc || case.tools.idtx_intra;
    modes.dip = case.tools.dip;
    modes.palette = case.tools.screen_content;
    let mut coder = RandomLevels {
        rng: Rng(seed.rotate_left(17) | 1),
        max_level: case.max_level,
        coverage: TxCoverage {
            steer: case.tx.tx_mode == TxMode::Select,
            ..coverage
        },
        map: Vec::new(),
    };
    let max_q = splot_recon::max_quantizer_index(case.bit_depth());
    let mut edges = [6, 7, 8, -600, 600, -600].into_iter();
    while writer.next_superblock_root().is_some() {
        if let Some(res) = case.tx.delta_q_res {
            let before = writer.syntax().qindex();
            let target = match edges.next() {
                Some(jump) => before.saturating_add_signed(jump << res),
                None => [1, max_q, rng.below(u64::from(max_q) + 8) as u32][rng.below(3) as usize],
            };
            let qindex = writer.set_superblock_qindex(target).unwrap();
            let step = 1 << res;
            let target = target.clamp(1, max_q) as i32;
            let nearest = (-600..=600)
                .map(|reduced| (before as i32 + reduced * step).clamp(1, max_q as i32))
                .map(|codable| codable.abs_diff(target))
                .min();
            assert_eq!(
                Some((qindex as i32).abs_diff(target)),
                nearest,
                "{before} {target}"
            );
        }
        let tree = writer
            .search_superblock(|syntax, root| {
                Ok(random_tree(syntax, root, &mut rng, &mut modes, &mut coder))
            })
            .unwrap();
        writer
            .write_superblock(&SuperblockDecision { tree }, &mut coder)
            .unwrap();
    }
    let (tile, recon) = writer.finish().unwrap();
    let unit = encode_key_frame_temporal_unit(&seq, &core, true, &[&tile]).unwrap();
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

/// Superblock qindex targets at every `delta_q_res`: `delta_q_abs` 6, 7 and 8
/// around `DELTA_Q_SMALL`, the largest jumps both ways, then random targets: the facade codes each `delta_q_abs`, escape and sign, and
/// dequantizes with the `CurrentQIndex` the decoder derives.
#[test]
fn superblock_qindex_deltas_round_trip() {
    for (seed, res) in (0..4).enumerate() {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            let sb_size = [SuperblockSize::Block64x64, SuperblockSize::Block128x128][seed % 2];
            let mut case = Case::new(200, 136, bit_depth, sb_size);
            case.tx.delta_q_res = Some(res);
            assert_round_trip(case, 0xde17a + seed as u64);
        }
    }
}

/// A superblock whose first leaf is a superblock-sized skipped leaf reads no
/// delta and keeps `CurrentQIndex`; the next superblock's delta is coded
/// from it, and a non-skipped leaf codes the planned qindex.
#[test]
fn superblock_sized_skipped_leaf_keeps_current_qindex() {
    let mut case = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    case.tx.delta_q_res = Some(0);
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: case.bit_depth,
        chroma: case.chroma,
        sb_size: case.sb_size,
        tools: case.tools,
    })
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(
        &view,
        50,
        0,
        &FrameFilterParams::default(),
        case.tx,
        UniformTiles::default(),
    )
    .unwrap();
    let mut syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let root = syntax.superblock_root(0, 0);
    let leaf = syntax.leaf(root).unwrap();
    let mut sink = crate::rd::coeff_rate::RateSink::adapting();
    syntax.begin_superblock(root);
    assert_eq!(syntax.set_superblock_qindex(120).unwrap(), 120);
    assert_eq!(syntax.qindex(), 120);
    syntax.write_delta_qindex(&leaf, true, &mut sink).unwrap();
    assert_eq!((syntax.qindex(), sink.bits.0), (50, 0));
    syntax.begin_superblock(root);
    assert_eq!(syntax.set_superblock_qindex(57).unwrap(), 57);
    syntax.write_delta_qindex(&leaf, false, &mut sink).unwrap();
    assert!(sink.bits.0 > 0);
    assert_eq!(syntax.qindex(), 57);
    syntax.begin_superblock(root);
    assert_eq!(syntax.set_superblock_qindex(1).unwrap(), 1);
}

/// Without `delta_q_present` the facade plans no superblock qindex.
#[test]
fn superblock_qindex_needs_delta_q_present() {
    let case = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let (seq, core) = headers(case.tools);
    let mut syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    assert!(matches!(
        syntax.set_superblock_qindex(100),
        Err(EncoderSyntaxError::Unsupported(_))
    ));
    assert_eq!(syntax.qindex(), 50);
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

/// Random DIP modes and transposes of `DC_PRED` leaves decode to the encoder
/// recon: the facade writes `use_dip`, `dip_transpose` and `dip_mode`, and
/// predicts with the decoder's DIP.
#[test]
fn every_dip_mode_round_trips() {
    let mut dips = HashSet::new();
    for (seed, bit_depth) in [BitDepthIdc::Eight, BitDepthIdc::Ten, BitDepthIdc::Eight]
        .into_iter()
        .enumerate()
    {
        let mut case = Case::new(200, 136, bit_depth, SuperblockSize::Block64x64);
        case.tools.dip = true;
        let (_, modes, _) =
            assert_round_trip_modes(case, 0xd19 + seed as u64, TxCoverage::default());
        dips.extend(modes.coded_dip);
    }
    assert!(dips.len() >= 8, "{dips:?}");
}

/// Random palettes, color maps and transform partitions of `DC_PRED` leaves
/// decode to the encoder recon: the facade writes `palette_mode_info` with
/// the color cache and `palette_tokens` in both scan directions, also for
/// leaves past the frame edge.
#[test]
fn palette_leaves_round_trip() {
    let mut palettes = HashSet::new();
    for (seed, bit_depth) in [BitDepthIdc::Eight, BitDepthIdc::Ten, BitDepthIdc::Eight]
        .into_iter()
        .enumerate()
    {
        let mut case = Case::new(200, 136, bit_depth, SuperblockSize::Block64x64);
        if seed == 2 {
            case = case.with_tx_search();
        }
        case.tools.screen_content = true;
        let (_, modes, _) =
            assert_round_trip_modes(case, 0xba1 + seed as u64, TxCoverage::default());
        palettes.extend(modes.coded_palette);
    }
    assert!(palettes.len() >= 20, "{}", palettes.len());
}

/// Random FSC leaves decode to the encoder recon with IDTX-only and FSC
/// coefficient coding, with and without `enable_fsc`.
#[test]
fn fsc_leaves_round_trip() {
    for (seed, (bit_depth, fsc)) in [
        (BitDepthIdc::Eight, false),
        (BitDepthIdc::Eight, true),
        (BitDepthIdc::Ten, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut case = Case::new(136, 72, bit_depth, SuperblockSize::Block64x64).with_tx_search();
        (case.tools.fsc, case.tools.idtx_intra) = (fsc, !fsc);
        (case.tools.tcq, case.tx.allow_tcq) = (true, true);
        let (_, modes, _) =
            assert_round_trip_modes(case, 0xf5c + seed as u64, TxCoverage::default());
        assert!(modes.coded_fsc, "{bit_depth:?} fsc {fsc}");
    }
}

/// Random MHCCP directions decode to the encoder recon: the facade writes
/// `cfl_mhccp` and `cfl_mh_dir` and predicts with the decoder's MHCCP.
#[test]
fn every_mhccp_direction_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut chroma = HashSet::new();
        for (seed, sb_size) in [SuperblockSize::Block64x64, SuperblockSize::Block128x128]
            .into_iter()
            .enumerate()
        {
            let mut case = Case::new(136, 72, bit_depth, sb_size);
            case.tools.mhccp = true;
            let (_, modes, _) =
                assert_round_trip_modes(case, 0x4cc + seed as u64, TxCoverage::default());
            chroma.extend(modes.coded_chroma);
        }
        assert!(
            (15..18).all(|id| chroma.contains(&id)),
            "{bit_depth:?}: {chroma:?}"
        );
    }
}

/// Random MRL lines and secondary blends decode to the encoder recon, with
/// every transform type and partition: the facade writes `mrl_index` and
/// `mrl_sec_index` in the decoder's contexts and publishes `UsesMrls`.
#[test]
fn every_mrl_line_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut lines = HashSet::new();
        for (seed, sb_size) in [SuperblockSize::Block64x64, SuperblockSize::Block128x128]
            .into_iter()
            .enumerate()
        {
            let mut case = Case::new(136, 72, bit_depth, sb_size);
            case.tools.mrls = true;
            for case in [case, case.with_tx_search()] {
                let (_, modes, _) =
                    assert_round_trip_modes(case, 0x3a1 + seed as u64, TxCoverage::default());
                lines.extend(modes.coded_mrl);
            }
        }
        assert_eq!(lines.len(), 7, "{bit_depth:?}: {lines:?}");
    }
}

/// Random CCTX rotations of chroma pairs decode to the encoder recon in every
/// chroma format: the facade writes `cctx_type` and reconstructs each pair
/// with the decoder's cross-chroma transform.
#[test]
fn every_cctx_type_round_trips() {
    for chroma in [
        ChromaFormatIdc::Yuv420,
        ChromaFormatIdc::Yuv422,
        ChromaFormatIdc::Yuv444,
    ] {
        let mut coverage = TxCoverage::default();
        for (seed, bit_depth) in [BitDepthIdc::Eight, BitDepthIdc::Ten]
            .into_iter()
            .enumerate()
        {
            let mut case = Case::new(136, 72, bit_depth, SuperblockSize::Block64x64);
            case.tools.cctx = true;
            case.chroma = chroma;
            coverage = assert_round_trip_modes(case, 0xcc7 + seed as u64, coverage).2;
        }
        assert_eq!(coverage.cctx.len(), 7, "{chroma:?}: {:?}", coverage.cctx);
    }
}

/// Random IST kernels and sets of DCT and ADST blocks decode to the encoder
/// recon: the facade writes `sec_tx_type` and `most_probable_stx_set`, and
/// reconstructs with the decoder's secondary transform.
#[test]
fn every_ist_kernel_and_set_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut coverage = TxCoverage::default();
        for seed in 0..6 {
            let sb_size = if seed % 2 == 0 {
                SuperblockSize::Block64x64
            } else {
                SuperblockSize::Block128x128
            };
            let mut case = Case::new(200, 136, bit_depth, sb_size).with_tx_search();
            case.tools.intra_ist = true;
            coverage = assert_round_trip_modes(case, 0x157 + seed, coverage).2;
        }
        let types: HashSet<u8> = coverage.ist.iter().map(|&(.., t)| t).collect();
        let kernels: HashSet<(u8, u8)> = coverage.ist.iter().map(|&(k, s, _)| (k, s)).collect();
        assert_eq!(types.len(), 2, "{bit_depth:?}: {types:?}");
        assert_eq!(kernels.len(), 3 * usize::from(IST_SETS), "{bit_depth:?}");
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

/// SDP key frames (§ 5.20.3.1) with random, independent luma and chroma
/// trees decode to the encoder recon in every chroma format, at frame
/// edges, with both superblock sizes and with extended partitions.
#[test]
fn sdp_key_frames_decode_to_the_encoder_recon() {
    for (index, case) in sdp_cases().into_iter().enumerate() {
        assert_round_trip(case, 0x5d9 + index as u64);
    }
}

fn sdp_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (index, chroma) in [
        ChromaFormatIdc::Yuv420,
        ChromaFormatIdc::Yuv422,
        ChromaFormatIdc::Yuv444,
    ]
    .into_iter()
    .enumerate()
    {
        for (width, height) in [(17, 9), (64, 64), (65, 65), (200, 136)] {
            let bit_depth = [BitDepthIdc::Eight, BitDepthIdc::Ten][(index + width as usize) % 2];
            for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
                let mut case = Case::new(width, height, bit_depth, sb_size);
                case.chroma = chroma;
                case.tools.sdp = true;
                cases.push(case);
            }
        }
        let mut case = Case::new(130, 66, BitDepthIdc::Ten, SuperblockSize::Block128x128);
        case.chroma = chroma;
        case.tools.sdp = true;
        case.tools.mrls = true;
        cases.push(case.with_tx_search());
    }
    cases
}

/// The facade gives a 64x64 intra node SDP roots. The 64x64 chroma node
/// follows a luma `HORZ` whose halves split across and then implies `HORZ`,
/// but not a luma `VERT` whose halves split along; a chroma 8x16 node allows
/// no vertical split and a chroma 8x8 node implies `NONE` (§ 5.20.3.2).
#[test]
fn sdp_chroma_tree_follows_the_luma_tree_rules() {
    let mut tools = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64).tools;
    let (seq, core) = headers(tools);
    let syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    assert!(syntax.sdp_roots(syntax.superblock_root(0, 0)).is_none());
    tools.sdp = true;
    let (seq, core) = headers(tools);
    let mut sink = crate::rd::coeff_rate::RateSink::adapting();
    for (top, follows) in [(Partition::Horz, true), (Partition::Vert, false)] {
        let mut syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
        let root = syntax.superblock_root(0, 0);
        syntax.begin_superblock(root);
        let (luma, chroma) = syntax.sdp_roots(root).unwrap();
        assert!(syntax.sdp_roots(luma).is_none() && syntax.sdp_roots(chroma).is_none());
        assert!(chroma.is_chroma_part() && !luma.is_chroma_part());
        syntax.write_partition(luma, top, &mut sink).unwrap();
        for child in syntax.children(luma, top).unwrap() {
            syntax
                .write_partition(child, Partition::Vert, &mut sink)
                .unwrap();
        }
        let options = syntax.partition_options(chroma).unwrap();
        assert_eq!(options.implied(), follows.then_some(Partition::Horz));
        if follows {
            continue;
        }
        let mut node = chroma;
        for (step, partition) in [
            Partition::Horz,
            Partition::Vert,
            Partition::Horz,
            Partition::Vert,
        ]
        .into_iter()
        .enumerate()
        {
            syntax.write_partition(node, partition, &mut sink).unwrap();
            node = syntax.children(node, partition).unwrap()[step % 2];
        }
        assert_eq!(node.dimensions().unwrap(), (16, 16));
        let narrow = syntax.children(node, Partition::Vert).unwrap()[0];
        assert_eq!(narrow.dimensions().unwrap(), (8, 16));
        let options = syntax.partition_options(narrow).unwrap();
        assert!(!options.allows(Partition::Vert) && options.allows(Partition::Horz));
        assert!(matches!(
            syntax.write_partition(narrow, Partition::Vert, &mut sink),
            Err(EncoderSyntaxError::Unsupported(_))
        ));
        let small = syntax.children(narrow, Partition::Horz).unwrap()[1];
        assert_eq!(small.dimensions().unwrap(), (8, 8));
        let options = syntax.partition_options(small).unwrap();
        assert_eq!(options.implied(), Some(Partition::None));
    }
}

/// A shared tree at an SDP node and an SDP tree at a shared node do not code.
#[test]
fn rejects_sdp_tree_mismatches() {
    let mut tools = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64).tools;
    for sdp in [true, false] {
        tools.sdp = sdp;
        let (seq, core) = headers(tools);
        let mut writer = KeyFrameTileWriter::new(&seq, &core, &refs_for(&seq), 0).unwrap();
        let leaf = PartitionTree::Leaf(LeafChoice::DC);
        let tree = if sdp {
            leaf
        } else {
            PartitionTree::Sdp {
                luma: Box::new(leaf.clone()),
                chroma: Box::new(leaf),
            }
        };
        let mut coder = RandomLevels {
            rng: Rng(1),
            max_level: 1,
            coverage: TxCoverage::default(),
            map: Vec::new(),
        };
        assert!(matches!(
            writer.write_superblock(&SuperblockDecision { tree }, &mut coder),
            Err(TileWriterError::Shape(_))
        ));
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
        UniformTiles::default(),
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let tree = PartitionTree::Leaf(LeafChoice::DC);
    assert!(matches!(
        writer.write_superblock(
            &SuperblockDecision { tree },
            &mut RandomLevels {
                rng: Rng(1),
                max_level: 1,
                coverage: TxCoverage::default(),
                map: Vec::new(),
            }
        ),
        Err(TileWriterError::Partition { .. })
    ));
}

fn headers(tools: EncoderToolFlags) -> (SequenceHeader, FrameHeaderCore) {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 64,
        height: 64,
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
        UniformTiles::default(),
    )
    .unwrap();
    (seq, core)
}

/// Parity hiding round-trips over every luma type and partition, with each
/// hidden parity, a hidden level with a remainder, a hidden `quant` of 1, and
/// one-dimensional types.
#[test]
fn parity_hiding_round_trips() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut coverage = TxCoverage::default();
        for seed in 0..4 {
            let mut case =
                Case::new(136, 72, bit_depth, SuperblockSize::Block64x64).with_tx_search();
            case.tools.parity_hiding = true;
            case.max_level = 40;
            coverage = assert_round_trip_modes(case, 0x9a00 + seed, coverage).2;
        }
        assert!(
            coverage.hidden.iter().all(|&n| n > 0),
            "{bit_depth:?}: {:?}",
            coverage.hidden
        );
    }
}

/// TCQ levels round-trip over every luma type and partition at both bit
/// depths: luma 2D blocks code and reconstruct along the TCQ states (with
/// remainders on both sides of the `maxLevel - 1` threshold), chroma and 1D
/// blocks code plain levels in the same frame.
#[test]
fn tcq_levels_round_trip() {
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        for seed in 0..4 {
            let mut case =
                Case::new(136, 72, bit_depth, SuperblockSize::Block64x64).with_tx_search();
            case.tools.tcq = true;
            case.tx.allow_tcq = true;
            case.max_level = 40;
            assert_round_trip_modes(case, 0x7c00 + seed, TxCoverage::default());
        }
    }
}

/// Publication check: the facade's parity rule is the § 5.20.7.27 rule the
/// writer and the quantizer assume, for every class and transform size.
#[test]
fn facade_parity_rule_is_the_spec_rule() {
    let mut tools = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64).tools;
    tools.parity_hiding = true;
    let (seq, core) = headers(tools);
    assert!(core.lossless_info.as_ref().unwrap().allow_parity_hiding);
    let syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let mut rng = Rng(0x9a17);
    let types = [
        TxType::DctDct,
        TxType::AdstFlipadst,
        TxType::Idtx,
        TxType::VDct,
        TxType::HFlipadst,
    ];
    for tx_size in 0..TX_WIDTH.len() {
        for tx_type in types {
            for plane in 0..2 {
                let tx = TransformBlock {
                    tx_type,
                    plane,
                    x: 0,
                    y: 0,
                    tx_size,
                    fills_block: true,
                    ist: splot_decode::encoder_syntax::Ist::OFF,
                    cctx: 0,
                };
                let parity = syntax.parity_hiding(&tx);
                assert_eq!(parity.is_some(), plane == 0 && tx_type != TxType::Idtx);
                let Some(parity) = parity else { continue };
                let scan = coefficient_scan(&tx).unwrap();
                for _ in 0..16 {
                    let levels = random_levels(&mut rng, tx_size, 20);
                    let after: Vec<u32> = scan[1..]
                        .iter()
                        .map(|&pos| levels[usize::from(pos)].unsigned_abs())
                        .filter(|&level| level != 0)
                        .collect();
                    let sum = after.iter().fold(0, |sum, &level| sum ^ (level.min(6) & 1));
                    let expected = (after.len() >= 4).then_some(sum);
                    assert_eq!(
                        parity.sum_abs1(&levels).unwrap(),
                        expected,
                        "{tx_size} {tx_type:?}"
                    );
                    let first = levels[usize::from(scan[0])].unsigned_abs() & 1;
                    assert_eq!(
                        parity.holds(&levels).unwrap(),
                        expected.is_none_or(|s| s == first)
                    );
                }
            }
        }
    }
}

/// The facade itself refuses every tool it does not model, so no caller can
/// code a silently wrong stream.
#[test]
fn facade_rejects_unmodelled_tools() {
    let base = Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64).tools;
    let rejects = |seq: &SequenceHeader, core: &FrameHeaderCore| {
        matches!(
            IntraTileSyntax::new(seq, core, &refs_for(seq), 0),
            Err(EncoderSyntaxError::Unsupported(_))
        ) && matches!(
            KeyFrameTileWriter::new(seq, core, &refs_for(seq), 0),
            Err(TileWriterError::Syntax(EncoderSyntaxError::Unsupported(_)))
        )
    };
    let (seq, core) = headers(base);
    assert!(IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).is_ok());
    let frame_cases: [fn(&mut FrameHeaderCore); 4] = [
        |c| c.lossless_info.as_mut().unwrap().coded_lossless = true,
        |c| c.quantization_params.as_mut().unwrap().base_q_idx = 0,
        |c| c.segmentation_params.as_mut().unwrap().segmentation_enabled = true,
        |c| c.setup_qm_params.as_mut().unwrap().using_qmatrix = true,
    ];
    for set in frame_cases {
        let mut core = core.clone();
        set(&mut core);
        assert!(rejects(&seq, &core));
    }
    let intrabc = |global, local| {
        let mut core = core.clone();
        core.allow_intrabc = Some(true);
        core.intrabc = Some(splot_core::headers::frame::IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(global),
            allow_local_intrabc: local,
            change_bvp_drl: None,
            max_bvp_drl_bits_minus_1: None,
        });
        core
    };
    let global_only = intrabc(true, Some(false));
    assert!(IntraTileSyntax::new(&seq, &global_only, &refs_for(&seq), 0).is_ok());
    assert!(rejects(&seq, &intrabc(false, None)), "local-only IntraBC");
    assert!(rejects(&seq, &intrabc(true, Some(true))), "local IntraBC");
    let mut inter = global_only;
    inter.frame_is_intra = Some(false);
    assert!(rejects(&seq, &inter), "IntraBC in an inter frame");
    let mut core = core;
    core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Only4x4;
    assert!(rejects(&seq, &core));
}

/// A CFL alpha of `i8::MIN` is refused instead of overflowing `abs`.
#[test]
fn rejects_out_of_range_cfl_alphas() {
    let (seq, core) =
        headers(Case::new(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64).tools);
    let mut syntax = IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let root = syntax.superblock_root(0, 0);
    syntax.begin_superblock(root);
    let node = syntax.children(root, Partition::Horz).unwrap()[0];
    let node = syntax.children(node, Partition::Vert).unwrap()[0];
    let leaf = syntax.leaf(node).unwrap();
    assert!(syntax.cfl_allowed(&leaf));
    for (alpha_u, alpha_v) in [(i8::MIN, 1), (1, i8::MIN), (MAX_CFL_ALPHA + 1, 0)] {
        let modes = IntraModes {
            uv_mode: ChromaMode::Cfl { alpha_u, alpha_v },
            ..IntraModes::DC
        };
        assert!(matches!(
            syntax.write_intra_modes(&leaf, modes, &mut SymbolEncoder::new()),
            Err(EncoderSyntaxError::Unsupported("CFL alpha magnitude"))
        ));
    }
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
    cases.extend(sdp_cases());
    let mut extended = Case::new(150, 98, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    extended.tools.ext_partitions = true;
    extended.tools.uneven_4way_partitions = true;
    cases.push(extended);
    let mut fsc =
        Case::new(200, 136, BitDepthIdc::Ten, SuperblockSize::Block64x64).with_tx_search();
    fsc.tools.fsc = true;
    cases.push(fsc);
    let mut dip = Case::new(200, 136, BitDepthIdc::Ten, SuperblockSize::Block128x128);
    dip.tools.dip = true;
    cases.push(dip);
    for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
        let mut palette = Case::new(200, 136, bit_depth, SuperblockSize::Block64x64);
        palette.tools.screen_content = true;
        cases.push(palette.with_tx_search());
    }
    let mut mhccp = Case::new(200, 136, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    mhccp.tools.mhccp = true;
    cases.push(mhccp);
    let mut mrl =
        Case::new(200, 136, BitDepthIdc::Ten, SuperblockSize::Block64x64).with_tx_search();
    mrl.tools.mrls = true;
    cases.push(mrl);
    let mut ist =
        Case::new(200, 136, BitDepthIdc::Ten, SuperblockSize::Block64x64).with_tx_search();
    ist.tools.intra_ist = true;
    cases.push(ist);
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

/// A 128x128 CFL leaf predicts its chroma from all four reconstructed 64x64
/// luma chunks: the decoder defers CFL chroma until the luma is complete.
#[test]
fn cfl_chroma_of_a_128x128_leaf_reads_the_whole_luma() {
    let case = Case::new(128, 128, BitDepthIdc::Eight, SuperblockSize::Block128x128);
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 128,
        height: 128,
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
        UniformTiles::default(),
    )
    .unwrap();
    let mut writer = KeyFrameTileWriter::new(&seq, &core, &refs_for(&seq), 0).unwrap();
    let root = writer.next_superblock_root().unwrap();
    let leaf = writer.syntax().leaf(root).unwrap();
    assert!(writer.syntax().cfl_allowed(&leaf));
    let modes = IntraModes {
        y_mode: IntraMode::D135,
        uv_mode: ChromaMode::Cfl {
            alpha_u: 5,
            alpha_v: -4,
        },
        ..IntraModes::DC
    };
    let mut coder = RandomLevels {
        rng: Rng(0xcf1),
        max_level: 40,
        coverage: TxCoverage::default(),
        map: Vec::new(),
    };
    let tree = PartitionTree::Leaf(LeafChoice::largest(modes));
    writer
        .write_superblock(&SuperblockDecision { tree }, &mut coder)
        .unwrap();
    let (tile, recon) = writer.finish().unwrap();
    let unit = encode_key_frame_temporal_unit(&seq, &core, true, &[&tile]).unwrap();
    let view = recon.as_frame_ref();
    let expected = raw_bytes(
        [PlaneId::Y, PlaneId::U, PlaneId::V]
            .into_iter()
            .flat_map(|plane| view.plane(plane).unwrap().visible_rows()),
        case.bit_depth(),
    );
    assert_same_samples(&decode(&unit), &expected, "128x128 CFL leaf");
}
