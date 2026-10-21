// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! RD intra mode decision and the trial coder of the partition search.
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
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::rd::partition::PartitionSearch;
use crate::rd::{Bits, Lambda, RdCost};
use crate::tile_writer::{INTRA_MODES, PARTITIONS, Partition, Result, TileWriterError, Walk};

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
}

impl<'a, 'b> TrialCoder<'a, 'b> {
    pub(crate) const fn new(
        syntax: &'a mut IntraTileSyntax,
        coder: &'a mut SourceResidualCoder<'b>,
        preset: &'a PresetParams,
        lambda: Lambda,
    ) -> Self {
        Self {
            syntax,
            coder,
            preset,
            lambda,
        }
    }

    /// Codes the `planes` of `leaf` under `modes` into the live state.
    fn trial(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        planes: Range<usize>,
    ) -> Result<RdCost> {
        let mut sink = RateSink::adapting();
        let blocks = Walk::trial(self.syntax, &mut sink).write_leaf(
            leaf,
            modes,
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

    /// The `preset.intra_modes` candidates with the lowest SATD, best first;
    /// ties keep candidate order, so the ranking is deterministic.
    fn shortlist(
        &mut self,
        leaf: &LeafBlock,
        candidates: Vec<IntraModes>,
        planes: Range<usize>,
    ) -> Result<Vec<IntraModes>> {
        let mut ranked = Vec::with_capacity(candidates.len());
        for modes in candidates {
            ranked.push((self.satd(leaf, modes, planes.clone())?, modes));
        }
        ranked.sort_by_key(|&(satd, _)| satd);
        ranked.truncate(usize::from(self.preset.intra_modes.max(1)));
        Ok(ranked.into_iter().map(|(_, modes)| modes).collect())
    }

    /// The shortlisted candidate with the lowest `D + lambda * R` over
    /// `planes`; each trial starts from `start`, and the state is `start` after.
    fn best(
        &mut self,
        leaf: &LeafBlock,
        candidates: Vec<IntraModes>,
        planes: Range<usize>,
        start: &SyntaxSnapshot,
    ) -> Result<IntraModes> {
        let shortlist = self.shortlist(leaf, candidates, planes.clone())?;
        let mut best: Option<(u128, IntraModes)> = None;
        for modes in shortlist {
            self.syntax.restore(start)?;
            let j = self.trial(leaf, modes, planes.clone())?.cost(self.lambda);
            if best.is_none_or(|(best_j, _)| j < best_j) {
                best = Some((j, modes));
            }
        }
        self.syntax.restore(start)?;
        best.map(|(_, modes)| modes)
            .ok_or(TileWriterError::Shape("no intra mode candidate"))
    }

    /// Every luma mode and angle delta, with the chroma mode of `planned`.
    fn luma_candidates(planned: IntraModes) -> Vec<IntraModes> {
        INTRA_MODES
            .into_iter()
            .flat_map(|y_mode| {
                let deltas = if y_mode.is_directional() {
                    -MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA
                } else {
                    0..=0
                };
                deltas.map(move |angle_delta| IntraModes {
                    y_mode,
                    angle_delta,
                    ..planned
                })
            })
            .collect()
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
        Ok(PARTITIONS
            .into_iter()
            .filter(|&p| options.allows(p))
            .collect())
    }

    fn children(&self, node: PartitionNode, partition: Partition) -> Result<Vec<PartitionNode>> {
        Ok(self.syntax.children(node, partition)?)
    }

    fn key(&self, node: PartitionNode) -> u64 {
        let (w, h) = node.dimensions().unwrap_or_default();
        (node.row4() << 40 | node.col4() << 16 | w << 8 | h) as u64
    }

    fn block_log2(&self, node: PartitionNode) -> Result<(u32, u32)> {
        let (w, h) = node.dimensions()?;
        Ok((w.ilog2(), h.ilog2()))
    }

    /// RD-chooses the luma modes on luma alone, then the chroma mode over the
    /// coded luma, then codes the whole leaf.
    fn evaluate_leaf(&mut self, node: PartitionNode) -> Result<(RdCost, IntraModes)> {
        let leaf = self.syntax.leaf(node)?;
        let start = self.save(node)?;
        let mut modes = self.best(&leaf, Self::luma_candidates(IntraModes::DC), LUMA, &start)?;
        if leaf.has_chroma() {
            self.trial(&leaf, modes, LUMA)?;
            let luma = self.save(node)?;
            let candidates = self.chroma_candidates(&leaf, modes)?;
            modes = self.best(&leaf, candidates, CHROMA, &luma)?;
            self.syntax.restore(&start)?;
        }
        Ok((self.trial(&leaf, modes, ALL)?, modes))
    }

    fn code_leaf(&mut self, node: PartitionNode, modes: IntraModes) -> Result<RdCost> {
        let leaf = self.syntax.leaf(node)?;
        self.trial(&leaf, modes, ALL)
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
