// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! RD mode decision, intra, single-reference, compound and skip-mode inter,
//! and the trial coder of the partition search.
//!
//! A trial codes through the tile writer's own [`Walk`] into a [`RateSink`],
//! so its rate is the exact symbol cost of the final stream, and its
//! distortion is the squared error of the facade's reconstruction. The search
//! rolls trials back with facade snapshots; the symbol encoder never sees a
//! trial.

use core::ops::Range;

use splot_decode::encoder_syntax::{
    ChromaMode, EncoderSyntaxError, IntraModes, IntraTileSyntax, LeafBlock, MAX_CFL_ALPHA,
    PartitionNode, SyntaxSnapshot,
};

use crate::intra_frame::SourceResidualCoder;
use crate::intra_frame::codable_children;
use crate::motion_search::MotionSearch;
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::rd::partition::PartitionSearch;
use crate::rd::{Bits, Lambda, RdCost};
use crate::tile_writer::{
    CompoundMode, CompoundModes, INTRA_MODES, InterMode, InterModes, InterPrediction, LeafChoice,
    MotionVector, PARTITIONS, Partition, Result, TileWriterError, Walk, snap_mv, snap_prediction,
};

/// The largest `|AngleDeltaY|` (§ 5.20.5.5).
const MAX_ANGLE_DELTA: i8 = 3;
/// References whose pairs compound candidates take: the § 7.7 map ranks the
/// closest references first.
const COMPOUND_REFS: usize = 3;
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
        Ok(self.plane_satd(leaf, modes, planes)?.iter().sum())
    }

    /// The SATD of each plane in `planes` with one plan of `leaf`.
    fn plane_satd(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        planes: Range<usize>,
    ) -> Result<[u64; 3]> {
        let mut cost = [0; 3];
        for (index, tx) in self.syntax.plan_leaf(leaf, modes)?.iter().enumerate() {
            if let Some(slot) = cost.get_mut(tx.plane)
                && planes.contains(&tx.plane)
            {
                *slot += self.coder.source.satd(tx, &self.syntax.predict(index)?)?;
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
            self.coder.trial = true;
            let cost = self.trial(leaf, choice, planes.clone());
            (self.coder.candidate, self.coder.trial) = (false, false);
            self.syntax.set_scalar_trial(false);
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
            let [_, u, v] = self.plane_satd(leaf, both, 1..3)?;
            alpha_u = alpha_u.min((u, alpha));
            alpha_v = alpha_v.min((v, alpha));
        }
        candidates.push(with(ChromaMode::Cfl {
            alpha_u: alpha_u.1,
            alpha_v: alpha_v.1,
        }));
        Ok(candidates)
    }
}

impl TrialCoder<'_, '_> {
    /// The inter candidates of `leaf`: for every reference `GLOBALMV`,
    /// `NEARMV` of each DRL index, and `NEWMV` with the vector searched from
    /// the whole § 7.12 stack over the DRL base whose mode and difference
    /// syntax costs least (candidates with a reference and vector already
    /// listed are dropped); then the skip-mode blocks and the compound
    /// shortlist.
    fn inter_candidates(&mut self, leaf: &LeafBlock) -> Result<Vec<LeafChoice>> {
        let mut out: Vec<((usize, MotionVector), LeafChoice)> = Vec::new();
        let mut push = |mv: MotionVector, modes: InterModes| {
            let key = (modes.ref_frame, mv);
            if out.iter().all(|&(seen, _)| seen != key) {
                out.push((key, LeafChoice::inter(InterPrediction::Single(modes))));
            }
        };
        let mut searched = Vec::new();
        for ref_frame in 0..self.syntax.reference_count() {
            let mode = |mode, drl| InterModes {
                ref_frame,
                mode,
                drl,
                mv: MotionVector::default(),
            };
            let mvs = self.syntax.single_mvs(leaf, ref_frame)?;
            push(mvs.global, mode(InterMode::Global, 0));
            let mut seeds = Vec::new();
            for (drl, &mv) in mvs.near.iter().enumerate() {
                seeds.push(mv);
                push(mv, mode(InterMode::Near, drl));
            }
            let Some(motion) = self.motion else {
                continue;
            };
            for &mv in &mvs.stack {
                if !seeds.contains(&mv) {
                    seeds.push(mv);
                }
            }
            let bases = mvs.new_bases;
            let pred = bases.first().copied().unwrap_or_default();
            let found = motion.search(self.syntax, ref_frame, leaf_rect(leaf)?, &seeds, pred)?;
            searched.push((found, pred));
            let mut best: Option<(Bits, InterModes)> = None;
            for (drl, &base) in bases.iter().enumerate() {
                let modes = InterModes {
                    mv: snap_mv(found, base, self.syntax.mv_step()),
                    ..mode(InterMode::New, drl)
                };
                let mut sink = RateSink::adapting();
                self.syntax
                    .price_single_modes(leaf, (modes, base), &mut sink)?;
                if best.is_none_or(|(bits, _)| sink.bits < bits) {
                    best = Some((sink.bits, modes));
                }
            }
            if let Some((_, modes)) = best {
                push(modes.mv, modes);
            }
        }
        let mut choices: Vec<LeafChoice> = out.into_iter().map(|(_, choice)| choice).collect();
        if self.preset.compound_candidates > 0 && self.syntax.skip_mode_allowed(leaf) {
            choices.extend(
                (0..self.syntax.drl_count())
                    .map(|drl| LeafChoice::inter(InterPrediction::SkipMode { drl })),
            );
        }
        choices.extend(self.compound_candidates(leaf, &searched)?);
        Ok(choices)
    }

    /// The `preset.compound_candidates` compound predictions of `leaf` with
    /// the lowest luma SATD plus vector rate, over the reference pairs of the
    /// first [`COMPOUND_REFS`] references. Each pair tries `NEAR_NEARMV` over
    /// the DRL indices, and a pair of distinct references also
    /// `GLOBAL_GLOBALMV` and the `NEWMV` modes, whose lists take the
    /// single-reference search results `searched` (`(vector, base)` per
    /// reference).
    fn compound_candidates(
        &mut self,
        leaf: &LeafBlock,
        searched: &[(MotionVector, MotionVector)],
    ) -> Result<Vec<LeafChoice>> {
        let (Some(motion), true) = (self.motion, self.syntax.compound_allowed(leaf)) else {
            return Ok(Vec::new());
        };
        if self.preset.compound_candidates == 0 {
            return Ok(Vec::new());
        }
        let drls = self.syntax.drl_count();
        let mut ranked: Vec<(u64, [MotionVector; 2], CompoundModes)> = Vec::new();
        for refs in self.syntax.compound_pairs() {
            if refs.1 >= COMPOUND_REFS {
                continue;
            }
            let modes = |mode, drl, mvs| CompoundModes {
                refs,
                mode,
                drl,
                mvs,
                jmvd_scale_mode: 0,
            };
            let zero = [MotionVector::default(); 2];
            let mut trials: Vec<CompoundModes> = (0..drls * drls)
                .map(|i| modes(CompoundMode::NearNear, [i / drls, i % drls], zero))
                .collect();
            if refs.0 != refs.1 {
                trials.push(modes(CompoundMode::GlobalGlobal, [0; 2], zero));
                if let (Some(&(first, _)), Some(&(second, _))) =
                    (searched.get(refs.0), searched.get(refs.1))
                {
                    for mode in [
                        CompoundMode::NewNew,
                        CompoundMode::NearNew,
                        CompoundMode::NewNear,
                        CompoundMode::JointNew,
                    ] {
                        trials.push(modes(mode, [0; 2], [first, second]));
                    }
                }
            }
            for trial in trials {
                let Some(InterPrediction::Compound(trial)) = codable(snap_prediction(
                    self.syntax,
                    leaf,
                    InterPrediction::Compound(trial),
                ))?
                else {
                    continue;
                };
                let Some(mvs) = codable(
                    self.syntax
                        .inter_mvs(leaf, InterPrediction::Compound(trial))
                        .map_err(Into::into),
                )?
                else {
                    continue;
                };
                if ranked
                    .iter()
                    .any(|(_, seen, other)| *seen == mvs && other.refs == refs)
                {
                    continue;
                }
                let lists = match trial.mode {
                    CompoundMode::NewNew => [true; 2],
                    CompoundMode::NearNew => [false, true],
                    CompoundMode::NewNear | CompoundMode::JointNew => [true, false],
                    CompoundMode::NearNear | CompoundMode::GlobalGlobal => [false; 2],
                };
                let rate: u64 = [refs.0, refs.1]
                    .iter()
                    .zip(mvs)
                    .zip(lists)
                    .filter(|&(_, coded)| coded)
                    .map(|((&r, mv), _)| {
                        let pred = searched.get(r).map_or(mv, |&(_, pred)| pred);
                        motion.rate(mv, pred)
                    })
                    .sum();
                let satd = motion.compound_satd(
                    self.syntax,
                    [(refs.0, mvs[0]), (refs.1, mvs[1])],
                    leaf_rect(leaf)?,
                )?;
                ranked.push((satd + rate, mvs, trial));
            }
        }
        ranked.sort_by_key(|&(cost, ..)| cost);
        ranked.truncate(usize::from(self.preset.compound_candidates));
        Ok(ranked
            .into_iter()
            .map(|(.., trial)| LeafChoice::inter(InterPrediction::Compound(trial)))
            .collect())
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
            self.coder.scalar = !self.preset.tcq_type_trellis;
            let luma = self.trial(leaf, choice, LUMA);
            self.coder.scalar = false;
            luma?;
            let luma = self.save(node, None)?;
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

/// The luma rectangle `(x, y, width, height)` of `leaf`.
fn leaf_rect(leaf: &LeafBlock) -> Result<(usize, usize, usize, usize)> {
    let (w, h) = leaf.dimensions()?;
    Ok((leaf.col4() * 4, leaf.row4() * 4, w, h))
}

/// `Some` result, `None` for a candidate the leaf cannot code.
fn codable<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(TileWriterError::Syntax(EncoderSyntaxError::Unsupported(_))) => Ok(None),
        Err(error) => Err(error),
    }
}

impl PartitionSearch for TrialCoder<'_, '_> {
    type Node = PartitionNode;
    type Saved = SyntaxSnapshot;
    type Error = TileWriterError;

    fn in_frame(&self, node: PartitionNode) -> bool {
        self.syntax.in_frame(node)
    }

    /// The allowed partitions of `node`, without those with a leaf that can
    /// code neither an intra nor an inter block.
    fn partitions(&self, node: PartitionNode) -> Result<Vec<Partition>> {
        let options = self.syntax.partition_options(node)?;
        if let Some(implied) = options.implied().filter(|&p| options.allows(p)) {
            return Ok(vec![implied]);
        }
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
        let start = self.save(node, None)?;
        let mut candidates = Vec::new();
        if self.syntax.intra_allowed(&leaf) {
            candidates.push(self.intra_choice(node, &leaf, &start)?);
        }
        if self.syntax.inter_allowed(&leaf) {
            candidates.extend(self.inter_candidates(&leaf)?);
        }
        let choice = self.best(&leaf, candidates, ALL, &start)?;
        Ok((self.code_leaf(node, choice)?, choice))
    }

    fn code_leaf(&mut self, node: PartitionNode, choice: LeafChoice) -> Result<RdCost> {
        let leaf = self.syntax.leaf(node)?;
        self.coder.scalar = !self.preset.tcq_search_trellis;
        let cost = self.trial(&leaf, choice, ALL);
        self.coder.scalar = false;
        cost
    }

    fn partition_rate(&mut self, node: PartitionNode, partition: Partition) -> Result<Bits> {
        let mut sink = RateSink::adapting();
        Walk::trial(self.syntax, &mut sink).write_partition(node, partition)?;
        Ok(sink.bits)
    }

    fn save(
        &mut self,
        node: PartitionNode,
        start: Option<&SyntaxSnapshot>,
    ) -> Result<SyntaxSnapshot> {
        Ok(match start {
            Some(start) => self.syntax.snapshot_since(start, node)?,
            None => self.syntax.snapshot(node)?,
        })
    }

    fn restore(&mut self, saved: &SyntaxSnapshot) -> Result<()> {
        Ok(self.syntax.restore(saved)?)
    }
}
