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
use std::collections::HashMap;

use splot_decode::encoder_syntax::{
    ChromaMode, Dip, EncoderSyntaxError, INTERP_FILTERS, IntraModes, IntraTileSyntax, LeafBlock,
    MAX_CFL_ALPHA, Mrl, PALETTE_COLORS, PartitionNode, SyntaxSnapshot,
};
use splot_recon::InterpolationFilter;

use crate::intra_frame::codable_children;
use crate::intra_frame::{PaddedSource, SourceResidualCoder};
use crate::kernels::{BlockDims, kernels};
use crate::motion_search::MotionSearch;
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::rd::partition::PartitionSearch;
use crate::rd::{Bits, Lambda, RdCost};
use crate::tile_writer::{
    CompoundMode, CompoundModes, CompoundType, INTRA_MODES, InterMode, InterModes, InterPrediction,
    LeafChoice, MotionVector, PARTITIONS, Partition, PartitionTree, Result, TileWriterError, Walk,
    snap_mv, snap_prediction,
};

/// The largest `|AngleDeltaY|` (§ 5.20.5.5).
const MAX_ANGLE_DELTA: i8 = 3;
/// A `NEWMV` tries one block precision coarser than the search justifies
/// when its difference rate exceeds `1 / COARSER_RATE_SHARE` of its cost.
const COARSER_RATE_SHARE: u64 = 4;
/// `(SATD + rate, rate, modes)` of one `NEWMV` coding.
type NewmvCost = (u64, u64, InterModes);
/// The best directional modes of the luma shortlist whose MRL lines are ranked.
const MRL_MODES: usize = 2;
/// Every MRL line past the adjacent one, without and with the secondary blend.
const MRL_LINES: [Mrl; 6] = {
    let mut lines = [Mrl::OFF; 6];
    let mut i = 0;
    while i < 6 {
        lines[i] = Mrl {
            index: 1 + i as u8 / 2,
            secondary: i % 2 == 1,
        };
        i += 1;
    }
    lines
};
/// A test-only [`PresetParams::compound_types`] that forces compound types
/// on the compound candidates by leaf position and makes them a leaf's only
/// candidates when it has any.
#[cfg(test)]
pub(crate) const FORCED_COMPOUND_TYPES: u8 = 3;
#[cfg(test)]
thread_local! {
    /// Makes the inter-intra candidate of a leaf its only inter candidate on
    /// this thread.
    pub(crate) static FORCE_INTERINTRA: core::cell::Cell<bool> =
        const { core::cell::Cell::new(false) };
}
/// The compound shortlist prices the syntax of this many times as many of
/// its lowest-SATD candidates as it keeps.
const PRICED_COMPOUND_SHARE: usize = 2;
/// One luma sample in `1/8` units.
const FULL_PEL: i32 = 8;
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
    /// The partition of each node of the coded luma tree, by key, while a
    /// chroma tree is searched.
    luma: HashMap<u64, Partition>,
    compound: compound_search::CompoundScratch,
}

impl<'a, 'b> TrialCoder<'a, 'b> {
    pub(crate) fn new(
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
            luma: HashMap::new(),
            compound: compound_search::CompoundScratch::default(),
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
    /// deltas of the shortlisted directional modes; then the
    /// `preset.mrl_trials` MRL lines of the [`MRL_MODES`] best directional
    /// modes with the lowest SATD, appended so they displace no line-0 mode.
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
        let angular = self.preset.dip.then(|| {
            ranked
                .iter()
                .filter(|(_, modes)| modes.y_mode.is_directional())
                .map(|&(satd, _)| satd)
                .min()
        });
        let mut shortlist = self.shortlist(ranked);
        if let Some(Some(angular)) = angular {
            shortlist.extend(self.dip_candidate(leaf, angular)?);
        }
        if self.preset.mrl_trials > 0 {
            let blend = leaf.dimensions()? != (4, 4); // § 7.13.2.7 `not4x4`
            let lines: Vec<IntraModes> = shortlist
                .iter()
                .filter(|choice| choice.modes.y_mode.is_directional())
                .take(MRL_MODES)
                .flat_map(|choice| {
                    MRL_LINES
                        .into_iter()
                        .filter(move |mrl| blend || !mrl.secondary)
                        .map(move |mrl| IntraModes {
                            mrl,
                            ..choice.modes
                        })
                })
                .collect();
            let mut ranked = Vec::new();
            self.rank(leaf, lines, LUMA, &mut ranked)?;
            ranked.sort_by_key(|&(satd, _)| satd);
            ranked.truncate(usize::from(self.preset.mrl_trials));
            shortlist.extend(
                ranked
                    .into_iter()
                    .map(|(_, modes)| LeafChoice::largest(modes)),
            );
        }
        Ok(shortlist)
    }

    /// The DIP prediction of `leaf` with the lowest SATD, when it beats the
    /// best angular SATD `angular`.
    fn dip_candidate(&mut self, leaf: &LeafBlock, angular: u64) -> Result<Option<LeafChoice>> {
        if !self.syntax.dip_allowed(leaf, IntraModes::DC) {
            return Ok(None);
        }
        let candidates: Vec<IntraModes> = (0..self.syntax.dip_modes()?)
            .flat_map(|mode| {
                [false, true].map(|transpose| IntraModes {
                    dip: Some(Dip { mode, transpose }),
                    ..IntraModes::DC
                })
            })
            .collect();
        let mut ranked = Vec::new();
        self.rank(leaf, candidates, LUMA, &mut ranked)?;
        Ok(ranked
            .into_iter()
            .min_by_key(|&(satd, _)| satd)
            .filter(|&(satd, _)| satd < angular)
            .map(|(_, modes)| LeafChoice::largest(modes)))
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
        Ok(self.best_scored(leaf, candidates, planes, start)?.1)
    }

    /// [`Self::best`] with its `D + lambda * R`; `None` for a lone candidate,
    /// which gets no trial.
    fn best_scored(
        &mut self,
        leaf: &LeafBlock,
        candidates: Vec<LeafChoice>,
        planes: Range<usize>,
        start: &SyntaxSnapshot,
    ) -> Result<(Option<u128>, LeafChoice)> {
        if let [only] = candidates.as_slice() {
            return Ok((None, *only));
        }
        let mut best: Option<(u128, LeafChoice)> = None;
        for choice in candidates {
            let j = self.trial_cost(leaf, choice, planes.clone(), start)?;
            if best.is_none_or(|(best_j, _)| j < best_j) {
                best = Some((j, choice));
            }
        }
        self.syntax.restore(start)?;
        best.map(|(j, choice)| (Some(j), choice))
            .ok_or(TileWriterError::Shape("no leaf candidate"))
    }

    /// The candidate-trial `D + lambda * R` of `choice` over `planes` from
    /// `start`; the state is the trial's after.
    fn trial_cost(
        &mut self,
        leaf: &LeafBlock,
        choice: LeafChoice,
        planes: Range<usize>,
        start: &SyntaxSnapshot,
    ) -> Result<u128> {
        self.syntax.restore(start)?;
        self.coder.candidate = !self.preset.full_trials;
        self.coder.trial = true;
        let cost = self.trial(leaf, choice, planes);
        (self.coder.candidate, self.coder.trial) = (false, false);
        self.syntax.set_scalar_trial(false);
        Ok(cost?.cost(self.lambda))
    }

    /// Every chroma mode, CFL with derived alphas, CFL with the explicit
    /// alphas of the lowest SATD per plane, and the MHCCP directions.
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
        if self.preset.mhccp && self.syntax.mhccp_allowed(leaf) {
            candidates.extend((0..3).map(|direction| with(ChromaMode::Mhccp { direction })));
        }
        Ok(candidates)
    }
}

impl TrialCoder<'_, '_> {
    /// The inter candidates of `leaf`: for every reference `GLOBALMV`,
    /// `NEARMV` of each DRL index, and `NEWMV` with the vector searched from
    /// the whole § 7.12 stack over the DRL base whose mode and difference
    /// syntax costs least (candidates with a reference and vector already
    /// listed are dropped); then, per distinct skip-mode vector pair, the
    /// skip-DRL index of the lowest exact price, and the compound shortlist.
    fn inter_candidates(&mut self, leaf: &LeafBlock) -> Result<Vec<LeafChoice>> {
        let mut out: Vec<((usize, MotionVector), LeafChoice)> = Vec::new();
        let mut push = |mv: MotionVector, modes: InterModes| {
            let key = (modes.ref_frame, mv);
            if out.iter().all(|&(seen, _)| seen != key) {
                out.push((key, LeafChoice::inter(InterPrediction::Single(modes))));
            }
        };
        let mut searched = Vec::new();
        let mut warp_base: Option<(u64, usize, MotionVector)> = None;
        let mut interintra_base: Option<InterModes> = None;
        for ref_frame in 0..self.syntax.reference_count() {
            let mode = |mode, drl| InterModes {
                ref_frame,
                mode,
                drl,
                mv: MotionVector::default(),
                filter: InterpolationFilter::EightTap,
                precision: None,
                amvd: false,
                interintra: None,
                bawp: None,
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
            searched.push(found);
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
                let coded = self.mvd_choice(leaf, modes, found)?;
                push(coded.mv, coded);
                if (self.preset.warp > 0 || self.preset.interintra || self.preset.bawp)
                    && let Some((cost, ..)) = self.newmv_cost(leaf, coded)?
                {
                    let mut filter = RateSink::adapting();
                    self.syntax.price_interp_filter(
                        leaf,
                        InterPrediction::Single(coded),
                        &mut filter,
                    )?;
                    let cost = cost + motion.bits_cost(filter.bits);
                    if warp_base.is_none_or(|(best, ..)| cost < best) {
                        warp_base = Some((cost, ref_frame, found));
                        interintra_base = Some(coded);
                    }
                }
            }
        }
        if self.preset.tip > 0 && self.syntax.tip_allowed(leaf)? {
            let ref_frame = self.syntax.reference_count();
            for (drl, &mv) in self
                .syntax
                .single_mvs(leaf, ref_frame)?
                .near
                .iter()
                .enumerate()
            {
                push(
                    mv,
                    InterModes {
                        ref_frame,
                        mode: InterMode::Near,
                        drl,
                        mv: MotionVector::default(),
                        filter: InterpolationFilter::EightTap,
                        precision: None,
                        amvd: false,
                        interintra: None,
                        bawp: None,
                    },
                );
            }
        }
        let mut choices: Vec<LeafChoice> = out.into_iter().map(|(_, choice)| choice).collect();
        if let (Some(motion), Some(base)) = (self.motion, interintra_base) {
            if self.preset.interintra {
                let interintra = self
                    .interintra_choice(leaf, motion, base)?
                    .map(|modes| LeafChoice::inter(InterPrediction::Single(modes)));
                #[cfg(test)]
                if let (Some(choice), true) = (interintra, FORCE_INTERINTRA.get()) {
                    return Ok(vec![choice]);
                }
                choices.extend(interintra);
            }
            if self.preset.bawp {
                choices.extend(
                    self.bawp_choice(leaf, motion, base)?
                        .map(|modes| LeafChoice::inter(InterPrediction::Single(modes))),
                );
            }
        }
        let warp_base = warp_base.filter(|_| self.preset.warp > 0);
        if let (Some(motion), Some((cost, ref_frame, found))) = (self.motion, warp_base) {
            let warps = crate::warp::search(
                self.syntax,
                motion,
                leaf,
                (ref_frame, found, cost),
                usize::from(self.preset.warp),
            )?;
            choices.extend(warps.into_iter().map(|(_, warp)| LeafChoice::inter(warp)));
        }
        if self.preset.compound_candidates > 0 && self.syntax.skip_mode_allowed(leaf) {
            let mut cheapest: Vec<([MotionVector; 2], Bits, InterPrediction)> = Vec::new();
            for drl in 0..self.syntax.drl_count() {
                let prediction = InterPrediction::SkipMode { drl };
                let mvs = self.syntax.inter_mvs(leaf, prediction)?;
                let mut sink = RateSink::adapting();
                self.syntax.price_inter_modes(leaf, prediction, &mut sink)?;
                match cheapest.iter_mut().find(|(seen, ..)| *seen == mvs) {
                    Some(entry) if sink.bits < entry.1 => *entry = (mvs, sink.bits, prediction),
                    Some(_) => {}
                    None => cheapest.push((mvs, sink.bits, prediction)),
                }
            }
            choices.extend(
                cheapest
                    .into_iter()
                    .map(|(.., prediction)| LeafChoice::inter(prediction)),
            );
        }
        let compound = self.compound_candidates(leaf, &searched)?;
        #[cfg(test)]
        if self.preset.compound_types == FORCED_COMPOUND_TYPES && !compound.is_empty() {
            return Ok(compound);
        }
        choices.extend(compound);
        Ok(choices)
    }

    /// The cheapest coding, by luma SATD plus exact mode and difference
    /// rate, of the frame-precision `NEWMV` `modes` searched to `found`: the
    /// coarsest block precision that codes `found` exactly, one precision
    /// coarser only when the difference rate exceeds [`COARSER_RATE_SHARE`]
    /// of the cost, and adaptive-MVD steps.
    fn mvd_choice(
        &mut self,
        leaf: &LeafBlock,
        modes: InterModes,
        found: MotionVector,
    ) -> Result<InterModes> {
        let precisions = self.syntax.newmv_precisions(leaf)?;
        if precisions.is_empty() && !self.syntax.adaptive_mvd() {
            return Ok(modes);
        }
        let Some(mut best) = self.newmv_cost(leaf, modes)? else {
            return Ok(modes);
        };
        let mut exact = None;
        for (index, &precision) in precisions.iter().enumerate() {
            if let Some(candidate) = self.newmv_at(leaf, modes, found, (Some(precision), false))?
                && candidate.2.mv == found
            {
                exact = Some((index, candidate));
            }
        }
        let mut coarser = 0;
        if let Some((index, candidate)) = exact {
            coarser = index + 1;
            best = cheaper(best, candidate);
        }
        if best.1 * COARSER_RATE_SHARE > best.0
            && let Some(&precision) = precisions.get(coarser)
            && let Some(candidate) = self.newmv_at(leaf, modes, found, (Some(precision), false))?
        {
            best = cheaper(best, candidate);
        }
        if self.syntax.adaptive_mvd()
            && let Some(candidate) = self.newmv_at(leaf, modes, found, (None, true))?
        {
            best = cheaper(best, candidate);
        }
        Ok(best.2)
    }

    /// `modes` coded at `(precision, amvd)` with the vector nearest `found`
    /// that it codes (see [`snap_prediction`]), with its
    /// [`Self::newmv_cost`].
    fn newmv_at(
        &mut self,
        leaf: &LeafBlock,
        modes: InterModes,
        found: MotionVector,
        (precision, amvd): (Option<u8>, bool),
    ) -> Result<Option<NewmvCost>> {
        let modes = InterModes {
            precision,
            amvd,
            mv: found,
            ..modes
        };
        let Some(InterPrediction::Single(modes)) = codable(snap_prediction(
            self.syntax,
            leaf,
            InterPrediction::Single(modes),
        ))?
        else {
            return Ok(None);
        };
        self.newmv_cost(leaf, modes)
    }

    /// `(SATD + rate, rate, modes)` of the `NEWMV` `modes` at `leaf`, in SAD
    /// units; `None` when the leaf cannot code it.
    fn newmv_cost(&mut self, leaf: &LeafBlock, modes: InterModes) -> Result<Option<NewmvCost>> {
        let Some(motion) = self.motion else {
            return Ok(None);
        };
        let mut sink = RateSink::adapting();
        let priced = self
            .syntax
            .price_inter_modes(leaf, InterPrediction::Single(modes), &mut sink);
        if codable(priced.map_err(Into::into))?.is_none() {
            return Ok(None);
        }
        let rate = motion.bits_cost(sink.bits);
        let satd = motion.prediction_satd(
            self.syntax,
            &[(modes.ref_frame, modes.mv)],
            InterpolationFilter::EightTap,
            leaf_rect(leaf)?,
        )?;
        Ok(Some((satd + rate, rate, modes)))
    }

    /// Sets the filter of `prediction` at `leaf`, which reads one, to the
    /// [`INTERP_FILTERS`] entry whose luma prediction SATD at the decoded
    /// vectors plus `interp_filter` rate is lowest; ties keep the earlier.
    /// Full-pel vectors predict luma alike with every filter, so their SATD
    /// is not computed.
    fn choose_filter(&mut self, leaf: &LeafBlock, prediction: &mut InterPrediction) -> Result<()> {
        let Some(motion) = self.motion else {
            return Ok(());
        };
        let refs = match *prediction {
            InterPrediction::Single(modes) => vec![modes.ref_frame],
            InterPrediction::Compound(modes) => vec![modes.refs.0, modes.refs.1],
            InterPrediction::SkipMode { .. } | InterPrediction::Warp(_) => return Ok(()),
        };
        let mvs = self.syntax.inter_mvs(leaf, *prediction)?;
        let lists: Vec<_> = refs.into_iter().zip(mvs).collect();
        let full_pel = lists
            .iter()
            .all(|(_, mv)| mv.row % FULL_PEL == 0 && mv.col % FULL_PEL == 0);
        let mut best: Option<(u64, InterpolationFilter)> = None;
        for filter in INTERP_FILTERS {
            let candidate = with_filter(*prediction, filter);
            let mut sink = RateSink::adapting();
            if !self
                .syntax
                .price_interp_filter(leaf, candidate, &mut sink)?
            {
                return Ok(());
            }
            let satd = if full_pel {
                0
            } else {
                motion.prediction_satd(self.syntax, &lists, filter, leaf_rect(leaf)?)?
            };
            let cost = satd + motion.bits_cost(sink.bits);
            if best.is_none_or(|(best_cost, _)| cost < best_cost) {
                best = Some((cost, filter));
            }
        }
        if let Some((_, filter)) = best {
            *prediction = with_filter(*prediction, filter);
        }
        Ok(())
    }

    /// `choice`, the leaf's winner of trial cost `cost`, with the filter of
    /// [`Self::choose_filter`] when it is an inter leaf that reads
    /// `interp_filter`. With `interp_filter_search` 2 a filter other than
    /// the winner's must also lower the trial cost. The state is `start`
    /// after.
    fn refine_filter(
        &mut self,
        leaf: &LeafBlock,
        (cost, choice): (Option<u128>, LeafChoice),
        start: &SyntaxSnapshot,
    ) -> Result<LeafChoice> {
        let Some(prediction) = choice
            .inter
            .filter(|_| self.preset.interp_filter_search > 0)
        else {
            return Ok(choice);
        };
        let mut picked = prediction;
        self.choose_filter(leaf, &mut picked)?;
        let other = LeafChoice::inter(picked);
        if picked == prediction || self.preset.interp_filter_search < 2 {
            return Ok(other);
        }
        let cost = match cost {
            Some(cost) => cost,
            None => self.trial_cost(leaf, choice, ALL, start)?,
        };
        let other_cost = self.trial_cost(leaf, other, ALL, start)?;
        self.syntax.restore(start)?;
        Ok(if other_cost < cost { other } else { choice })
    }

    /// Whether the final trial of `choice` measures the quadrant energy of its
    /// luma prediction residual: an inter leaf of a frame with luma transform
    /// partitions, both sides at least 8, under `preset.inter_tx_split`.
    fn measures_quadrants(&self, leaf: &LeafBlock, choice: LeafChoice) -> Result<bool> {
        let (w, h) = leaf.dimensions()?;
        Ok(choice.inter.is_some()
            && self.preset.inter_tx_split > 0
            && w.min(h) >= 8
            && self.syntax.luma_tx_partitions(leaf)?.len() > 1)
    }

    /// The best luma transform partition of the coded inter `choice` when one
    /// quadrant of its prediction residual holds more than
    /// `preset.inter_tx_split / 64` of the energy; `None` keeps `choice`.
    fn uneven_split(
        &mut self,
        leaf: &LeafBlock,
        choice: LeafChoice,
        quadrants: Option<[u64; 4]>,
        start: &SyntaxSnapshot,
    ) -> Result<Option<LeafChoice>> {
        let Some(quadrants) = quadrants else {
            return Ok(None);
        };
        let total: u64 = quadrants.iter().sum();
        let max = quadrants.iter().copied().max().unwrap_or(0);
        if max * 64 <= total * u64::from(self.preset.inter_tx_split) {
            return Ok(None);
        }
        self.syntax.restore(start)?;
        let candidates = self
            .syntax
            .luma_tx_partitions(leaf)?
            .into_iter()
            .map(|tx_partition| LeafChoice {
                tx_partition,
                ..choice
            })
            .collect();
        Ok(Some(self.best(leaf, candidates, ALL, start)?))
    }

    /// The `preset.compound_candidates` compound predictions of `leaf` with
    /// the lowest luma SATD plus the exact rate of their mode and difference
    /// syntax, DRL indices included (priced for the [`PRICED_COMPOUND_SHARE`]
    /// times as many lowest-SATD ones; a candidate with the references and
    /// vectors of an earlier one is dropped, which measured better than
    /// keeping the cheapest), over the reference pairs of the
    /// first [`COMPOUND_REFS`] references. Each pair tries `NEAR_NEARMV` over
    /// the DRL indices, and a pair of distinct references also
    /// `GLOBAL_GLOBALMV` and the `NEWMV` modes, whose lists take the
    /// single-reference search results `searched` (one vector per reference).
    fn compound_candidates(
        &mut self,
        leaf: &LeafBlock,
        searched: &[MotionVector],
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
                filter: InterpolationFilter::EightTap,
                optflow: false,
                refinemv: false,
                blend: CompoundType::default(),
            };
            let zero = [MotionVector::default(); 2];
            let mut trials: Vec<CompoundModes> = (0..drls * drls)
                .map(|i| modes(CompoundMode::NearNear, [i / drls, i % drls], zero))
                .collect();
            if refs.0 != refs.1 {
                trials.push(modes(CompoundMode::GlobalGlobal, [0; 2], zero));
                if let (Some(&first), Some(&second)) = (searched.get(refs.0), searched.get(refs.1))
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
                let satd = motion.prediction_satd(
                    self.syntax,
                    &[(refs.0, mvs[0]), (refs.1, mvs[1])],
                    InterpolationFilter::EightTap,
                    leaf_rect(leaf)?,
                )?;
                ranked.push((satd, mvs, trial));
            }
        }
        let keep = usize::from(self.preset.compound_candidates);
        ranked.sort_by_key(|&(satd, ..)| satd);
        ranked.truncate(keep * PRICED_COMPOUND_SHARE);
        let mut priced = Vec::with_capacity(ranked.len());
        for (mut cost, _, trial) in ranked {
            let mut sink = RateSink::adapting();
            self.syntax
                .price_inter_modes(leaf, InterPrediction::Compound(trial), &mut sink)?;
            cost += motion.bits_cost(sink.bits);
            let (average, typed) = self.typed_variants(leaf, motion, trial)?;
            if let Some(best) = typed.iter().map(|&(sse, _)| sse).min() {
                cost = scaled_cost(cost, best, average);
            }
            priced.push((cost, trial, typed));
        }
        priced.sort_by_key(|&(cost, ..)| cost);
        priced.truncate(keep);
        let mut out = Vec::with_capacity(priced.len());
        for (_, trial, typed) in priced {
            let trial = self.refined(leaf, motion, trial, &typed)?;
            #[cfg(test)]
            let trial = self.forced_type(leaf, trial)?;
            out.push(LeafChoice::inter(InterPrediction::Compound(trial)));
        }
        Ok(out)
    }

    /// With [`FORCED_COMPOUND_TYPES`], `trial` without refinement flags and
    /// with a wedge, a difference-weighted mask or a `cwp_idx` weight by the
    /// leaf position, where the block reads it.
    #[cfg(test)]
    fn forced_type(&mut self, leaf: &LeafBlock, trial: CompoundModes) -> Result<CompoundModes> {
        if self.preset.compound_types != FORCED_COMPOUND_TYPES {
            return Ok(trial);
        }
        let base = CompoundModes {
            optflow: false,
            refinemv: false,
            ..trial
        };
        let Some(types) = codable(self.syntax.compound_types(leaf, base).map_err(Into::into))?
        else {
            return Ok(trial);
        };
        let key = leaf.row4() + leaf.col4();
        let blends = [
            types.wedge.then_some(CompoundType::Wedge {
                index: (key % 68) as u8,
                sign: key % 2 == 1,
            }),
            types.masked.then_some(CompoundType::DiffWeighted {
                inverse: key.is_multiple_of(2),
            }),
            types.cwp.is_some().then_some(CompoundType::Average {
                cwp: 1 + (key % 4) as u8,
            }),
        ];
        Ok((0..3)
            .find_map(|i| blends[(key + i) % 3])
            .map_or(trial, |blend| CompoundModes { blend, ..base }))
    }

    /// `trial` with the `use_optflow` and `use_refinemv` it reads, or one of
    /// its [`Self::typed_variants`], whose decoder prediction has the least
    /// luma SATD plus mode and filter rate; each variant is predicted once,
    /// by the decoder's own refinement and blend.
    fn refined(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        trial: CompoundModes,
        typed: &[(i64, CompoundModes)],
    ) -> Result<CompoundModes> {
        let mut variants = vec![trial];
        if self.preset.opfl || self.preset.refinemv {
            let (reads_optflow, _) = self.syntax.compound_refinements(leaf, trial)?;
            if reads_optflow {
                variants.push(CompoundModes {
                    optflow: true,
                    ..trial
                });
            }
            let base = variants.len();
            for index in 0..base {
                let variant = variants[index];
                if self.syntax.compound_refinements(leaf, variant)?.1 {
                    variants.push(CompoundModes {
                        refinemv: true,
                        ..variant
                    });
                }
            }
        }
        variants.extend(typed.iter().map(|&(_, variant)| variant));
        if variants.len() == 1 {
            return Ok(trial);
        }
        let mut best = (u64::MAX, trial);
        for variant in variants {
            let prediction = InterPrediction::Compound(variant);
            let Some(blocks) = codable(
                self.syntax
                    .luma_prediction(leaf, prediction)
                    .map_err(Into::into),
            )?
            else {
                continue;
            };
            let mut cost = 0;
            for (tx, samples) in &blocks {
                cost += self.coder.source.satd(tx, samples)?;
            }
            let mut sink = RateSink::adapting();
            self.syntax.price_inter_modes(leaf, prediction, &mut sink)?;
            self.syntax
                .price_interp_filter(leaf, prediction, &mut sink)?;
            cost += motion.bits_cost(sink.bits);
            let cost = if variant.refinemv {
                cost.saturating_mul(100) / REFINEMV_MARGIN_PCT
            } else {
                cost
            };
            if cost < best.0 {
                best = (cost, variant);
            }
        }
        Ok(best.1)
    }

    /// The best chroma mode of the `CHROMA_PART` leaf `leaf` over its
    /// collocated luma: the shortlist of chroma SATD, then RD trials.
    fn chroma_choice(&mut self, leaf: &LeafBlock, start: &SyntaxSnapshot) -> Result<LeafChoice> {
        let modes = self.chroma_candidates(leaf, IntraModes::DC)?;
        let mut ranked = Vec::new();
        self.rank(leaf, modes, CHROMA, &mut ranked)?;
        let candidates = self.shortlist(ranked);
        self.best(leaf, candidates, CHROMA, start)
    }

    /// The partitions of each node of the luma `tree` at `node`, by key.
    fn record_luma(&mut self, node: PartitionNode, tree: &PartitionTree) -> Result<()> {
        let PartitionTree::Split { kind, children } = tree else {
            self.luma.insert(self.key(node), Partition::None);
            return Ok(());
        };
        self.luma.insert(self.key(node), *kind);
        for (child, tree) in self.syntax.children(node, *kind)?.into_iter().zip(children) {
            self.record_luma(child, tree)?;
        }
        Ok(())
    }

    /// The best intra choice of `leaf`: luma modes on luma alone, the winner
    /// against a palette, the winner with FSC, then the luma transform
    /// partition, then the chroma mode over the coded luma.
    fn intra_choice(
        &mut self,
        node: PartitionNode,
        leaf: &LeafBlock,
        start: &SyntaxSnapshot,
    ) -> Result<LeafChoice> {
        let luma = self.luma_shortlist(leaf)?;
        let mut choice = self.best(leaf, luma, LUMA, start)?;
        if self.preset.palette
            && self.syntax.palette_allowed(leaf)
            && self.coder.palette.count(self.coder.source, leaf)? >= 2
        {
            let mut candidates = vec![choice];
            for size in (2..=PALETTE_COLORS).rev() {
                if let Some(palette) = self.coder.palette.palette(size) {
                    let palette = LeafChoice::largest(IntraModes {
                        palette: Some(palette),
                        ..IntraModes::DC
                    });
                    if !candidates.contains(&palette) {
                        candidates.push(palette);
                    }
                }
            }
            choice = self.best(leaf, candidates, LUMA, start)?;
        }
        if self.preset.fsc && self.syntax.fsc_allowed(leaf) {
            let fsc = LeafChoice::largest(IntraModes {
                fsc: true,
                ..choice.modes
            });
            choice = self.best(leaf, vec![choice, fsc], LUMA, start)?;
        }
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

/// The squared error between the luma source and the row-major `prediction`
/// of the luma rectangle `(x, y, w, h)` in each quadrant, in raster order.
pub(crate) fn quadrant_sse(
    source: &PaddedSource,
    (x, y, w, h): (usize, usize, usize, usize),
    prediction: &[u16],
) -> Result<[u64; 4]> {
    let (luma, stride, ..) = source.luma();
    let dims = BlockDims::new(w / 2, h / 2).map_err(TileWriterError::Kernel)?;
    let mut out = [0; 4];
    for (index, sse) in out.iter_mut().enumerate() {
        let (dx, dy) = (index % 2 * w / 2, index / 2 * h / 2);
        let at = (y + dy) * stride + x + dx;
        let (Some(source), Some(predicted)) = (luma.get(at..), prediction.get(dy * w + dx..))
        else {
            return Err(TileWriterError::Shape("quadrant outside the source"));
        };
        *sse = kernels().sse(source, stride, predicted, w, dims);
    }
    Ok(out)
}

/// The luma rectangle `(x, y, width, height)` of `leaf`.
pub(crate) fn leaf_rect(leaf: &LeafBlock) -> Result<(usize, usize, usize, usize)> {
    let (w, h) = leaf.dimensions()?;
    Ok((leaf.col4() * 4, leaf.row4() * 4, w, h))
}

/// The shortlist cost `cost` of a compound candidate scaled by the ratio of
/// the squared error `best` of its best compound type to that of its
/// average, `average`.
fn scaled_cost(cost: u64, best: i64, average: i64) -> u64 {
    match (u128::try_from(best), u128::try_from(average)) {
        (Ok(best), Ok(average)) if average > 0 => {
            u64::try_from(u128::from(cost) * best / average).unwrap_or(cost)
        }
        _ => cost,
    }
}

/// The cheaper of `a` and `b`; a tie keeps `a`.
fn cheaper(a: NewmvCost, b: NewmvCost) -> NewmvCost {
    if b.0 < a.0 { b } else { a }
}

/// `prediction` with `interp_filter = filter`.
fn with_filter(prediction: InterPrediction, filter: InterpolationFilter) -> InterPrediction {
    match prediction {
        InterPrediction::Single(modes) => InterPrediction::Single(InterModes { filter, ..modes }),
        InterPrediction::Compound(modes) => {
            InterPrediction::Compound(CompoundModes { filter, ..modes })
        }
        other @ (InterPrediction::SkipMode { .. } | InterPrediction::Warp(_)) => other,
    }
}

/// A `use_refinemv` variant wins only below this percentage of the cost of
/// the best other variant.
const REFINEMV_MARGIN_PCT: u64 = 97;

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

    fn sdp_roots(&self, node: PartitionNode) -> Option<(PartitionNode, PartitionNode)> {
        self.syntax.sdp_roots(node)
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
        if node.is_chroma_part() && !self.luma.is_empty() {
            let luma = self.luma.get(&self.key(node)).copied();
            let merges: Vec<Partition> = allowed
                .iter()
                .copied()
                .filter(|&p| p == Partition::None || Some(p) == luma)
                .collect();
            if merges.is_empty() {
                return Err(TileWriterError::Shape(
                    "a chroma node that cannot merge luma",
                ));
            }
            return Ok(merges);
        }
        Ok(allowed)
    }

    fn follow_luma(&mut self, luma: Option<(PartitionNode, &PartitionTree)>) -> Result<()> {
        self.luma.clear();
        match luma {
            Some((root, tree)) => self.record_luma(root, tree),
            None => Ok(()),
        }
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
        if leaf.is_chroma_part() {
            let choice = self.chroma_choice(&leaf, &start)?;
            return Ok((self.code_leaf(node, choice)?, choice));
        }
        let mut candidates = Vec::new();
        if self.syntax.intra_allowed(&leaf) {
            candidates.push(self.intra_choice(node, &leaf, &start)?);
        }
        candidates.extend(crate::intrabc::intrabc_choice(
            self.syntax,
            self.coder,
            &leaf,
        )?);
        let (w, h) = leaf.dimensions()?;
        if self.syntax.inter_allowed(&leaf) && (self.preset.large_inter || w.max(h) <= 64) {
            let inter = self.inter_candidates(&leaf)?;
            #[cfg(test)]
            if self.preset.compound_types == FORCED_COMPOUND_TYPES
                && inter
                    .iter()
                    .any(|choice| matches!(choice.inter, Some(InterPrediction::Compound(_))))
                || FORCE_INTERINTRA.get()
                    && inter.iter().any(|choice| {
                        matches!(choice.inter, Some(InterPrediction::Single(single)) if single.interintra.is_some())
                    })
            {
                candidates.clear();
            }
            candidates.extend(inter);
        }
        let scored = self.best_scored(&leaf, candidates, ALL, &start)?;
        let choice = self.refine_filter(&leaf, scored, &start)?;
        self.coder.quadrants = self.measures_quadrants(&leaf, choice)?.then_some([0; 4]);
        let cost = self.code_leaf(node, choice);
        let quadrants = self.coder.quadrants.take();
        let cost = cost?;
        match self.uneven_split(&leaf, choice, quadrants, &start)? {
            Some(split) => Ok((self.code_leaf(node, split)?, split)),
            None => Ok((cost, choice)),
        }
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

#[path = "compound_search.rs"]
mod compound_search;
#[path = "interintra_search.rs"]
mod interintra_search;
