// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! RD mode decision, intra and single-reference inter, and the trial coder of
//! the partition search.
//!
//! A trial codes through the tile writer's own [`Walk`] into a [`RateSink`],
//! so its rate is the exact symbol cost of the final stream, and its
//! distortion is the squared error of the facade's reconstruction. The search
//! rolls trials back with facade snapshots; the symbol encoder never sees a
//! trial.

use core::ops::Range;

use splot_decode::encoder_syntax::{
    ChromaMode, IntraModes, IntraTileSyntax, LeafBlock, MAX_CFL_ALPHA, PartitionNode,
    SyntaxSnapshot,
};

use crate::intra_frame::SourceResidualCoder;
use crate::intra_frame::codable_children;
use crate::motion_search::MotionSearch;
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::rd::partition::PartitionSearch;
use crate::rd::{Bits, Lambda, RdCost};
use crate::tile_writer::{
    INTRA_MODES, InterMode, InterModes, LeafChoice, MotionVector, PARTITIONS, Partition, Result,
    TileWriterError, Walk, snap_mv,
};

/// The largest `|AngleDeltaY|` (§ 5.20.5.5).
const MAX_ANGLE_DELTA: i8 = 3;
/// Luma planes, chroma planes, and all planes of a leaf trial.
const LUMA: Range<usize> = 0..1;
const CHROMA: Range<usize> = 1..3;
const ALL: Range<usize> = 0..3;

/// Codes partition and leaf trials into the facade of one superblock search.
pub(crate) struct TrialCoder<'a, 'b> {
    syntax: &'a mut IntraTileSyntax,
    coder: &'a mut SourceResidualCoder<'b>,
    preset: &'a PresetParams,
    lambda: Lambda,
    motion: Option<&'a MotionSearch>,
}

impl<'a, 'b> TrialCoder<'a, 'b> {
    pub(crate) const fn new(
        syntax: &'a mut IntraTileSyntax,
        coder: &'a mut SourceResidualCoder<'b>,
        preset: &'a PresetParams,
        lambda: Lambda,
        motion: Option<&'a MotionSearch>,
    ) -> Self {
        Self {
            syntax,
            coder,
            preset,
            lambda,
            motion,
        }
    }

    /// Codes the `planes` of `leaf` under `choice` into the live state.
    fn trial(
        &mut self,
        leaf: &LeafBlock,
        choice: LeafChoice,
        planes: Range<usize>,
    ) -> Result<RdCost> {
        let mut sink = RateSink::adapting();
        let blocks = Walk::trial(self.syntax, &mut sink).write_leaf(
            leaf,
            choice,
            self.coder,
            planes.clone(),
        )?;
        let mut distortion = 0;
        for tx in blocks.iter().filter(|tx| planes.contains(&tx.plane)) {
            distortion += self
                .coder
                .source
                .sse(tx, &self.syntax.reconstruction(tx)?)?;
        }
        Ok(RdCost {
            distortion,
            rate: sink.bits,
        })
    }

    /// The prediction SATD of the `planes` of `leaf` under `modes`.
    fn satd(&mut self, leaf: &LeafBlock, modes: IntraModes, planes: Range<usize>) -> Result<u64> {
        let mut cost = 0;
        for (index, tx) in self.syntax.plan_leaf(leaf, modes)?.iter().enumerate() {
            if planes.contains(&tx.plane) {
                cost += self.coder.source.satd(tx, &self.syntax.predict(index)?)?;
            }
        }
        Ok(cost)
    }

    /// `candidates` with their SATD over `planes`, appended to `ranked`.
    fn rank(
        &mut self,
        leaf: &LeafBlock,
        candidates: impl IntoIterator<Item = IntraModes>,
        planes: Range<usize>,
        ranked: &mut Vec<(u64, IntraModes)>,
    ) -> Result<()> {
        for modes in candidates {
            ranked.push((self.satd(leaf, modes, planes.clone())?, modes));
        }
        Ok(())
    }

    /// The `preset.intra_modes` entries of `ranked` with the lowest SATD, best
    /// first; ties keep candidate order, so the ranking is deterministic.
    fn shortlist(&self, mut ranked: Vec<(u64, IntraModes)>) -> Vec<LeafChoice> {
        ranked.sort_by_key(|&(satd, _)| satd);
        ranked.truncate(usize::from(self.preset.intra_modes.max(1)));
        ranked
            .into_iter()
            .map(|(_, modes)| LeafChoice::largest(modes))
            .collect()
    }

    /// The luma shortlist: every mode without an angle delta, then the angle
    /// deltas of the shortlisted directional modes.
    fn luma_shortlist(&mut self, leaf: &LeafBlock) -> Result<Vec<LeafChoice>> {
        let base = INTRA_MODES.map(|y_mode| IntraModes {
            y_mode,
            ..IntraModes::DC
        });
        let mut ranked = Vec::new();
        self.rank(leaf, base, LUMA, &mut ranked)?;
        let deltas: Vec<IntraModes> = self
            .shortlist(ranked.clone())
            .into_iter()
            .filter(|choice| choice.modes.y_mode.is_directional())
            .flat_map(|choice| {
                (-MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA)
                    .filter(|&delta| delta != 0)
                    .map(move |angle_delta| IntraModes {
                        angle_delta,
                        ..choice.modes
                    })
            })
            .collect();
        self.rank(leaf, deltas, LUMA, &mut ranked)?;
        Ok(self.shortlist(ranked))
    }

    /// The candidate with the lowest `D + lambda * R` over `planes`; each
    /// trial starts from `start`, and the state is `start` after.
    fn best(
        &mut self,
        leaf: &LeafBlock,
        candidates: Vec<LeafChoice>,
        planes: Range<usize>,
        start: &SyntaxSnapshot,
    ) -> Result<LeafChoice> {
        if let [only] = candidates.as_slice() {
            return Ok(*only);
        }
        let mut best: Option<(u128, LeafChoice)> = None;
        for choice in candidates {
            self.syntax.restore(start)?;
            self.coder.candidate = !self.preset.full_trials;
            let cost = self.trial(leaf, choice, planes.clone());
            self.coder.candidate = false;
            let j = cost?.cost(self.lambda);
            if best.is_none_or(|(best_j, _)| j < best_j) {
                best = Some((j, choice));
            }
        }
        self.syntax.restore(start)?;
        best.map(|(_, choice)| choice)
            .ok_or(TileWriterError::Shape("no leaf candidate"))
    }

    /// Every chroma mode, CFL with derived alphas, and CFL with the explicit
    /// alphas of the lowest SATD per plane.
    fn chroma_candidates(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<Vec<IntraModes>> {
        let with = |uv_mode| IntraModes { uv_mode, ..modes };
        let mut candidates: Vec<IntraModes> = INTRA_MODES
            .map(|mode| with(ChromaMode::Intra(mode)))
            .to_vec();
        if !self.syntax.cfl_allowed(leaf) {
            return Ok(candidates);
        }
        candidates.push(with(ChromaMode::CflDerived));
        let mut alpha_u = (u64::MAX, 0);
        let mut alpha_v = (u64::MAX, 0);
        for alpha in (-MAX_CFL_ALPHA..=MAX_CFL_ALPHA).filter(|&alpha| alpha != 0) {
            let both = with(ChromaMode::Cfl {
                alpha_u: alpha,
                alpha_v: alpha,
            });
            alpha_u = alpha_u.min((self.satd(leaf, both, 1..2)?, alpha));
            alpha_v = alpha_v.min((self.satd(leaf, both, 2..3)?, alpha));
        }
        candidates.push(with(ChromaMode::Cfl {
            alpha_u: alpha_u.1,
            alpha_v: alpha_v.1,
        }));
        Ok(candidates)
    }
}

impl TrialCoder<'_, '_> {
    /// The inter candidates of `leaf` for every reference: `GLOBALMV`, `NEARMV`
    /// of each DRL index, and `NEWMV` with the searched vector over the DRL
    /// base nearest to it; candidates with a vector already listed are dropped.
    fn inter_candidates(&mut self, leaf: &LeafBlock) -> Result<Vec<LeafChoice>> {
        let mut out: Vec<(MotionVector, LeafChoice)> = Vec::new();
        let mut push = |mv: MotionVector, modes: InterModes| {
            if out.iter().all(|&(seen, _)| seen != mv) {
                out.push((mv, LeafChoice::inter(modes)));
            }
        };
        for ref_frame in 0..self.syntax.reference_count() {
            let mode = |mode, drl| InterModes {
                ref_frame,
                mode,
                drl,
                mv: MotionVector::default(),
            };
            push(
                self.syntax.inter_mv(leaf, mode(InterMode::Global, 0))?,
                mode(InterMode::Global, 0),
            );
            let mut seeds = Vec::new();
            for drl in 0..self.syntax.drl_count() {
                let mv = self.syntax.inter_mv(leaf, mode(InterMode::Near, drl))?;
                seeds.push(mv);
                push(mv, mode(InterMode::Near, drl));
            }
            let Some(motion) = self.motion else {
                continue;
            };
            let bases = (0..self.syntax.drl_count())
                .map(|drl| self.syntax.newmv_base(leaf, ref_frame, drl))
                .collect::<core::result::Result<Vec<_>, _>>()?;
            let pred = bases.first().copied().unwrap_or_default();
            let (w, h) = leaf.dimensions()?;
            let rect = (leaf.col4() * 4, leaf.row4() * 4, w, h);
            let found = motion.search(self.syntax, ref_frame, rect, &seeds, pred)?;
            let distance = |base: MotionVector| {
                (found.row - base.row).unsigned_abs() + (found.col - base.col).unsigned_abs()
            };
            let (drl, base) = bases
                .iter()
                .copied()
                .enumerate()
                .min_by_key(|&(_, base)| distance(base))
                .unwrap_or((0, pred));
            let mv = snap_mv(found, base, self.syntax.mv_step());
            push(
                mv,
                InterModes {
                    mv,
                    ..mode(InterMode::New, drl)
                },
            );
        }
        Ok(out.into_iter().map(|(_, choice)| choice).collect())
    }

    /// The best intra choice of `leaf`: luma modes on luma alone, then the
    /// luma transform partition, then the chroma mode over the coded luma.
    fn intra_choice(
        &mut self,
        node: PartitionNode,
        leaf: &LeafBlock,
        start: &SyntaxSnapshot,
    ) -> Result<LeafChoice> {
        let luma = self.luma_shortlist(leaf)?;
        let mut choice = self.best(leaf, luma, LUMA, start)?;
        if self.preset.tx_partition_search {
            let partitions = self.syntax.luma_tx_partitions(leaf)?;
            let candidates = partitions
                .into_iter()
                .map(|tx_partition| LeafChoice {
                    tx_partition,
                    ..choice
                })
                .collect();
            choice = self.best(leaf, candidates, LUMA, start)?;
        }
        if leaf.has_chroma() {
            self.trial(leaf, choice, LUMA)?;
            let luma = self.save(node)?;
            let modes = self.chroma_candidates(leaf, choice.modes)?;
            let mut ranked = Vec::new();
            self.rank(leaf, modes, CHROMA, &mut ranked)?;
            let candidates = self.shortlist(ranked);
            let chroma = candidates
                .into_iter()
                .map(|candidate| LeafChoice {
                    tx_partition: choice.tx_partition,
                    ..candidate
                })
                .collect();
            choice = self.best(leaf, chroma, CHROMA, &luma)?;
            self.syntax.restore(start)?;
        }
        Ok(choice)
    }
}

impl PartitionSearch for TrialCoder<'_, '_> {
    type Node = PartitionNode;
    type Saved = SyntaxSnapshot;
    type Error = TileWriterError;

    fn in_frame(&self, node: PartitionNode) -> bool {
        self.syntax.in_frame(node)
    }

    fn partitions(&self, node: PartitionNode) -> Result<Vec<Partition>> {
        let options = self.syntax.partition_options(node)?;
        if let Some(implied) = options.implied().filter(|&p| options.allows(p)) {
            return Ok(vec![implied]);
        }
        // Drop the partitions with a leaf that can code neither an intra block
        // nor an inter block.
        let mut allowed = Vec::with_capacity(PARTITIONS.len());
        for partition in PARTITIONS.into_iter().filter(|&p| options.allows(p)) {
            if codable_children(self.syntax, node, partition)? {
                allowed.push(partition);
            }
        }
        Ok(allowed)
    }

    fn children(&self, node: PartitionNode, partition: Partition) -> Result<Vec<PartitionNode>> {
        Ok(self.syntax.children(node, partition)?)
    }

    fn key(&self, node: PartitionNode) -> u64 {
        let (w, h) = node.dimensions().unwrap_or_default();
        (node.row4() as u64) << 40 | (node.col4() as u64) << 16 | (w as u64) << 8 | h as u64
    }

    fn block_log2(&self, node: PartitionNode) -> Result<(u32, u32)> {
        let (w, h) = node.dimensions()?;
        Ok((w.ilog2(), h.ilog2()))
    }

    /// RD-chooses the intra choice of the leaf, then compares it with the
    /// inter candidates over all planes, then codes the whole leaf.
    fn evaluate_leaf(&mut self, node: PartitionNode) -> Result<(RdCost, LeafChoice)> {
        let leaf = self.syntax.leaf(node)?;
        let start = self.save(node)?;
        let mut candidates = Vec::new();
        if self.syntax.intra_allowed(&leaf) {
            candidates.push(self.intra_choice(node, &leaf, &start)?);
        }
        if self.syntax.inter_allowed(&leaf) {
            candidates.extend(self.inter_candidates(&leaf)?);
        }
        let choice = self.best(&leaf, candidates, ALL, &start)?;
        Ok((self.trial(&leaf, choice, ALL)?, choice))
    }

    fn code_leaf(&mut self, node: PartitionNode, choice: LeafChoice) -> Result<RdCost> {
        let leaf = self.syntax.leaf(node)?;
        self.trial(&leaf, choice, ALL)
    }

    fn partition_rate(&mut self, node: PartitionNode, partition: Partition) -> Result<Bits> {
        let mut sink = RateSink::adapting();
        Walk::trial(self.syntax, &mut sink).write_partition(node, partition)?;
        Ok(sink.bits)
    }

    fn save(&mut self, node: PartitionNode) -> Result<SyntaxSnapshot> {
        let (w, h) = node.dimensions()?;
        Ok(self
            .syntax
            .snapshot((node.col4() * 4, node.row4() * 4, w, h))?)
    }

    fn restore(&mut self, saved: &SyntaxSnapshot) -> Result<()> {
        Ok(self.syntax.restore(saved)?)
    }
}
