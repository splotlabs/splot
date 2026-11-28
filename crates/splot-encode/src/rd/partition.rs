// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Recursive rate-distortion partition search, independent of leaf coding.

use std::collections::HashMap;

use std::sync::Arc;

use super::{Bits, Lambda, RdCost};
use crate::hme::FrameMotion;
use crate::preset::{PresetContext, PresetParams};
use crate::tile_writer::{LeafChoice, Partition, PartitionTree, TileWriterError};
use crate::trace::{PartitionOutcome, PartitionPruning};

/// The luma child nodes and the chroma node of an `INTRA_REGION`.
pub(crate) type RegionNodes<N> = (Vec<N>, N);

/// What the partition search needs from a coder. Trials code into live state:
/// [`Self::evaluate_leaf`] leaves its leaf coded, and the search rolls back
/// through [`Self::save`] and [`Self::restore`].
pub(crate) trait PartitionSearch {
    /// One `decode_partition` call.
    type Node: Copy + PartialEq;
    /// A saved coder state.
    type Saved;
    /// The coder's error.
    type Error: From<TileWriterError>;

    /// Whether `node` starts inside the frame; nodes outside code nothing.
    fn in_frame(&self, node: Self::Node) -> bool;
    /// The luma and chroma roots that code `node` in turn when it is an SDP node.
    fn sdp_roots(&self, node: Self::Node) -> Option<(Self::Node, Self::Node)>;
    /// Limits the chroma tree to merges of the coded luma `tree` at `root`;
    /// `None` lifts the limit.
    fn follow_luma(
        &mut self,
        luma: Option<(Self::Node, &PartitionTree)>,
    ) -> Result<(), Self::Error>;
    /// The partitions `node` can code: the implied one alone when the position
    /// implies one, else the allowed set.
    fn partitions(&self, node: Self::Node) -> Result<Vec<Partition>, Self::Error>;
    /// The child nodes of a non-`NONE` partition, in syntax order.
    fn children(
        &self,
        node: Self::Node,
        partition: Partition,
    ) -> Result<Vec<Self::Node>, Self::Error>;
    /// A key of `node`'s block; equal nodes have equal keys.
    fn key(&self, node: Self::Node) -> u64;
    /// Luma `(log2 width, log2 height)` of `node`.
    fn block_log2(&self, node: Self::Node) -> Result<(u32, u32), Self::Error>;
    /// The luma `(left, top)` sample of `node`.
    fn origin(&self, node: Self::Node) -> (usize, usize);
    /// Codes `node` as a leaf into the live state; returns its cost and decision.
    fn evaluate_leaf(&mut self, node: Self::Node) -> Result<(RdCost, LeafChoice), Self::Error>;
    /// Codes `node` as a leaf with `choice` into the live state; returns its cost.
    fn code_leaf(&mut self, node: Self::Node, choice: LeafChoice) -> Result<RdCost, Self::Error>;
    /// Whether the leaf coded last codes a nonzero coefficient in any plane.
    fn leaf_coded(&self) -> bool {
        true
    }
    /// Whether a memoized `tree` codes the same choices wherever it is coded
    /// again; otherwise a later visit searches its node again.
    fn replays(&self, _tree: &PartitionTree) -> bool {
        true
    }
    /// The rate of the partition symbols that select `partition` at `node`.
    fn partition_rate(
        &mut self,
        node: Self::Node,
        partition: Partition,
    ) -> Result<Bits, Self::Error>;
    /// The rate of the `region_type` that `node` codes after `partition`,
    /// `INTRA_REGION` when `intra`; `None` when it codes none.
    fn region_rate(
        &mut self,
        _node: Self::Node,
        _partition: Partition,
        _intra: bool,
    ) -> Result<Option<Bits>, Self::Error> {
        Ok(None)
    }
    /// The luma child nodes and the chroma node of an `INTRA_REGION` at
    /// `node` coded with `partition`.
    fn region_children(
        &self,
        _node: Self::Node,
        _partition: Partition,
    ) -> Result<RegionNodes<Self::Node>, Self::Error> {
        Err(TileWriterError::Shape("no extended SDP region").into())
    }
    /// Saves the state that coding `node` can change; with `start`, a live
    /// save of the same node, the state a trial reached from it.
    fn save(
        &mut self,
        node: Self::Node,
        start: Option<&Self::Saved>,
    ) -> Result<Self::Saved, Self::Error>;
    /// Restores a state saved for the same node.
    fn restore(&mut self, saved: &Self::Saved) -> Result<(), Self::Error>;
    /// The frame context of the preset's [`crate::preset::Levels`] fields.
    fn context(&self) -> PresetContext {
        PresetContext::default()
    }
    /// Records for the decision trace that the search of `node` considered
    /// `partition`, with its cost when it has one; returns the record.
    fn trace_partition(
        &mut self,
        _node: Self::Node,
        _partition: Partition,
        _cost: Option<RdCost>,
        _outcome: PartitionOutcome,
    ) -> Option<usize> {
        None
    }
    /// Marks the record `trial` as the shape the search of its node returned.
    fn trace_selected(&mut self, _trial: usize) {}
}

/// Finds the partition tree of `node` with the lowest `J = D + lambda * R` and
/// leaves the coder in the state of that tree.
///
/// A leaf is tried only up to `max_block_log2` per side, a split only when every
/// child keeps both sides at least `min_block_log2`; when the limits leave no
/// candidate, every codable partition is tried. `NONE` is tried first and
/// `SPLIT` second, then the binary shape that halves the longer side; with
/// `partition_early_exit`, the search stops once a split is coded and the
/// unsplit cost does not exceed the best split cost, so a wide block decides
/// the exit on `VERT`. A
/// tolerance that also stops when the split wins by a margin compounds over the
/// levels of the tree: 10% cost 20 points of BD-rate on CIF content.
///
/// An SDP node searches its luma tree on luma alone, then its chroma tree
/// over the coded luma, where each node tries `NONE` and the luma partition
/// at the same node only: the chroma tree merges luma leaves and never splits
/// deeper.
///
/// Partition paths reach many nodes more than once. A node keeps the tree its
/// first search chose, and a later visit codes that tree again in the new
/// context instead of searching it again.
///
/// With the preset's `none_breakout` level, a node whose `NONE` leaf codes
/// no coefficient and whose log2 area is at most `breakout_area`, or whose
/// `NONE` distortion and rate per sample are under the level's thresholds
/// ([`cheap`]), tries no other shape.
///
/// Each shape after the first searches under a budget, the lower of the best
/// cost so far and the node's own budget: its children search under what the
/// budget leaves, and the shape stops once its cost reaches the budget. A node
/// with no tree under its budget reports none and keeps the budget as a
/// bound: a later visit under a budget not above it reports none at once, a
/// later visit under a larger budget searches again and codes the `NONE`
/// choice found before. The shape whose cost decides the early exit searches
/// under the best cost alone, and three-way shapes without a budget, as the
/// exit and the four-way gate read their exact costs.
///
/// A three-way shape is tried only when the `NONE` cost is within the
/// preset's margin of the best cost so far, an uneven four-way shape only
/// when the three-way shape of its axis is; the early exit skips the other
/// remaining partitions but leaves extended shapes to these margins. The children of an extended shape
/// search without extended shapes, and a node keeps one tree per mode.
///
/// # Errors
/// Returns the coder's errors.
#[cfg(test)]
pub(crate) fn search_partition<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
) -> Result<(PartitionTree, RdCost), S::Error> {
    search_guided(coder, node, preset, lambda, None)
}

/// [`search_partition`] limited to the partitions `levels` allows, when
/// given.
///
/// # Errors
/// Returns the coder's errors.
pub(crate) fn search_guided<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    levels: Option<&Levels>,
) -> Result<(PartitionTree, RdCost), S::Error> {
    let mut memo = Memo::new();
    let found = search(
        coder,
        node,
        preset,
        lambda,
        (&mut memo, levels),
        (true, None),
    )?;
    Ok(found.unwrap_or((PartitionTree::Leaf(LeafChoice::DC), RdCost::default())))
}

/// Searches `node` once per budget of `budgets` over one memo; `None` for a
/// visit that found no tree under its budget.
///
/// # Errors
/// Returns the coder's errors.
#[cfg(test)]
pub(crate) fn search_visits<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    budgets: &[Option<u128>],
) -> Result<Vec<Option<RdCost>>, S::Error> {
    let mut memo = Memo::new();
    let mut out = Vec::with_capacity(budgets.len());
    for &budget in budgets {
        let found = search(
            coder,
            node,
            preset,
            lambda,
            (&mut memo, None),
            (true, budget),
        )?;
        out.push(found.map(|(_, cost)| cost));
    }
    Ok(out)
}

/// The partition levels of a first pass (PD0) over a superblock: each 8x8
/// luma cell holds twice the log2 side of the square that covers it, and a
/// guided search tries a block of `log2 width + log2 height` level `l` only
/// when `l` is within `window` of the levels of its cells, and splits it only
/// while a deeper block can be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Levels {
    /// The luma sample of cell 0.
    pub(crate) origin: (usize, usize),
    /// Cells per row.
    pub(crate) stride: usize,
    /// One level per cell, row-major.
    pub(crate) cells: Vec<u8>,
    /// The binary partition levels tried around the cell levels.
    pub(crate) window: u8,
}

impl Levels {
    /// Whether a block at `origin` of luma log2 size `(w, h)` may code
    /// `NONE` (`children` is `None`), or a split whose largest child has
    /// level `children`; a block over no cell may code any.
    fn allows(&self, (x, y): (usize, usize), (w, h): (u32, u32), children: Option<u8>) -> bool {
        let (Some(left), Some(top)) = (x.checked_sub(self.origin.0), y.checked_sub(self.origin.1))
        else {
            return true;
        };
        let rows = self.cells.len() / self.stride.max(1);
        let (cols, rows) = (
            (left >> 3)..((left + (1 << w)).div_ceil(8)).min(self.stride),
            (top >> 3)..((top + (1 << h)).div_ceil(8)).min(rows),
        );
        let mut range: Option<(u8, u8)> = None;
        for row in rows {
            for col in cols.clone() {
                let level = self.cells[row * self.stride + col];
                range =
                    Some(range.map_or((level, level), |(lo, hi)| (lo.min(level), hi.max(level))));
            }
        }
        let Some((lo, hi)) = range else {
            return true;
        };
        let level = (w + h) as u8;
        match children {
            None => {
                level <= hi.saturating_add(self.window) && level.saturating_add(self.window) >= lo
            }
            Some(child) => child.saturating_add(self.window) >= lo,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Nodes on this thread that the `NONE` breakout ended.
    pub(crate) static BREAKOUTS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
    /// `INTRA_REGION` trials on this thread that beat their shared tree.
    pub(crate) static REGIONS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// What a searched node found: its tree, or a budget it found no tree under
/// and the choice of its `NONE` leaf when it tried one.
enum Seen {
    Tree(PartitionTree),
    Bound(u128, Option<LeafChoice>),
}

/// What each searched node found, by node key and extended-shape mode.
type Memo<N> = HashMap<(u64, bool), Vec<(N, Seen)>>;

/// The tree of a search and its cost; `None` when no tree costs less than
/// the search's budget.
type Found = Option<(PartitionTree, RdCost)>;

/// Records `seen` for `node`, replacing what an earlier visit recorded.
fn remember<N: PartialEq>(memo: &mut Memo<N>, key: (u64, bool), node: N, seen: Seen) {
    let entries = memo.entry(key).or_default();
    match entries.iter_mut().find(|(at, _)| *at == node) {
        Some(entry) => entry.1 = seen,
        None => entries.push((node, seen)),
    }
}

/// The lower of two optional bounds; `None` is no bound.
fn lower(a: Option<u128>, b: Option<u128>) -> Option<u128> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn search<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    (memo, levels): (&mut Memo<S::Node>, Option<&Levels>),
    (extended, budget): (bool, Option<u128>),
) -> Result<Found, S::Error> {
    if !coder.in_frame(node) {
        return Ok(Some((
            PartitionTree::Leaf(LeafChoice::DC),
            RdCost::default(),
        )));
    }
    let key = (coder.key(node), extended);
    let mut leaf = None;
    match memo
        .get(&key)
        .and_then(|seen| seen.iter().find(|(seen, _)| *seen == node))
    {
        Some((_, Seen::Tree(tree))) if coder.replays(tree) => {
            let tree = tree.clone();
            let cost = recode(coder, node, &tree)?;
            if let Some(shape) = tree.root_partition() {
                coder.trace_partition(node, shape, Some(cost), PartitionOutcome::Replayed);
            }
            return Ok(Some((tree, cost)));
        }
        Some((_, Seen::Bound(bound, _))) if budget.is_some_and(|budget| budget <= *bound) => {
            return Ok(None);
        }
        Some((_, Seen::Bound(_, known))) => {
            leaf = known.filter(|choice| coder.replays(&PartitionTree::Leaf(*choice)));
        }
        Some((_, Seen::Tree(_))) | None => {}
    }
    if let Some((luma_root, chroma_root)) = coder.sdp_roots(node) {
        let lumas = search(
            coder,
            luma_root,
            preset,
            lambda,
            (memo, levels),
            (extended, budget),
        )?;
        let Some((luma, luma_cost)) = lumas else {
            return Ok(None);
        };
        let spent = luma_cost.cost(lambda);
        if budget.is_some_and(|budget| spent >= budget) {
            return Ok(None);
        }
        let rest = budget.map(|budget| budget - spent);
        coder.follow_luma(Some((luma_root, &luma)))?;
        let chroma = search(
            coder,
            chroma_root,
            preset,
            lambda,
            (memo, levels),
            (extended, rest),
        );
        coder.follow_luma(None)?;
        let Some((chroma, chroma_cost)) = chroma? else {
            return Ok(None);
        };
        let tree = PartitionTree::Sdp {
            luma: Box::new(luma),
            chroma: Box::new(chroma),
        };
        remember(memo, key, node, Seen::Tree(tree.clone()));
        return Ok(Some((tree, luma_cost + chroma_cost)));
    }
    let mut candidates = candidates(coder, node, preset, extended)?;
    if let Some(levels) = levels {
        let (origin, size) = (coder.origin(node), coder.block_log2(node)?);
        let mut allowed = Vec::with_capacity(candidates.len());
        for (partition, children) in &candidates {
            let mut largest = None;
            for &child in children {
                let (w, h) = coder.block_log2(child)?;
                largest = largest.max(Some((w + h) as u8));
            }
            allowed.push(levels.allows(
                origin,
                size,
                largest.filter(|_| *partition != Partition::None),
            ));
        }
        let none_only = !allowed.contains(&true);
        let mut allowed = allowed.into_iter();
        candidates.retain(|&(partition, _)| {
            allowed.next().unwrap_or(false) || none_only && partition == Partition::None
        });
        if candidates.is_empty() {
            candidates = self::candidates(coder, node, preset, extended)?;
        }
    }
    let start = if candidates.len() > 1 || preset.extended_sdp > 0 {
        Some(coder.save(node, None)?)
    } else {
        None
    };
    let mut best: Option<(u128, PartitionTree, RdCost, Option<S::Saved>)> = None;
    let mut best_trial = None;
    let mut unsplit = None;
    let mut best_split = None;
    let mut three_way = [None; 2];
    let mut exited = false;
    let early_exit = preset.partition_early_exit.at(coder.context());
    let breakout = preset.none_breakout.at(coder.context());
    let area = preset.breakout_area.at(coder.context());
    for (index, (partition, children)) in candidates.iter().enumerate() {
        let best_j = best.as_ref().map(|(j, ..)| *j);
        let gate = match extended_shape(*partition) {
            Some((_, false)) => Some((unsplit, preset.ext_three_way_margin)),
            Some((axis, true)) => Some((three_way[axis], preset.ext_four_way_margin)),
            None => None,
        };
        let gated = gate.map(|(j, margin)| within_margin(j, best_j, margin));
        if gated.map_or(exited, |accepted| !accepted) {
            let pruning = match gated {
                Some(_) => PartitionPruning::ExtendedMargin,
                None => PartitionPruning::EarlyExit,
            };
            coder.trace_partition(node, *partition, None, PartitionOutcome::Pruned(pruning));
            continue;
        }
        if index > 0
            && let Some(start) = &start
        {
            coder.restore(start)?;
        }
        let mut rate = coder.partition_rate(node, *partition)?;
        if *partition != Partition::None {
            rate += coder
                .region_rate(node, *partition, false)?
                .unwrap_or_default();
        }
        let three_way_shape = matches!(extended_shape(*partition), Some((_, false)));
        let exits = early_exit && unsplit.is_some() && best_split.is_none();
        let bound = match (three_way_shape, exits) {
            (true, _) => None,
            (false, true) => best_j,
            (false, false) => lower(best_j, budget),
        };
        let mut ends = false;
        let found = if *partition == Partition::None {
            let (mut cost, choice) = match leaf {
                Some(choice) => (coder.code_leaf(node, choice)?, choice),
                None => coder.evaluate_leaf(node)?,
            };
            leaf = Some(choice);
            cost.rate += rate;
            let (w, h) = coder.block_log2(node)?;
            ends = breakout > 0
                && (!coder.leaf_coded() && w + h <= u32::from(area)
                    || cheap(cost, (w, h), lambda, breakout));
            Some((PartitionTree::Leaf(choice), cost))
        } else {
            let nested = extended && extended_shape(*partition).is_none();
            let mut trees = Vec::with_capacity(children.len());
            let mut cost = RdCost {
                distortion: 0,
                rate,
            };
            for &child in children {
                let spent = cost.cost(lambda);
                if bound.is_some_and(|bound| spent >= bound) {
                    break;
                }
                let rest = bound.map(|bound| bound - spent);
                let found = search(coder, child, preset, lambda, (memo, levels), (nested, rest))?;
                let Some((tree, child_cost)) = found else {
                    break;
                };
                trees.push(tree);
                cost = cost + child_cost;
            }
            let split = (trees.len() == children.len()).then(|| {
                let tree = PartitionTree::Split {
                    kind: *partition,
                    children: trees,
                };
                (tree, cost)
            });
            let all_intra = split
                .as_ref()
                .is_some_and(|(tree, _)| tree.count_leaves(&|choice| choice.inter.is_some()) == 0);
            let level = preset.extended_sdp;
            match &start {
                Some(start) if level == 2 || level == 1 && all_intra => better_region(
                    coder,
                    (node, *partition),
                    (preset, lambda),
                    (memo, levels),
                    (nested, bound),
                    (start, split),
                )?,
                _ => split,
            }
        };
        let (cost, outcome) = match &found {
            Some((_, cost)) => (Some(*cost), PartitionOutcome::Rejected),
            None => (None, PartitionOutcome::OverBound),
        };
        let trial = coder.trace_partition(node, *partition, cost, outcome);
        let j = match found {
            Some((tree, cost)) => {
                let j = cost.cost(lambda);
                if best_j.is_none_or(|best_j| j < best_j) {
                    best_trial = trial;
                    let end = if index + 1 < candidates.len() && !ends {
                        Some(coder.save(node, start.as_ref())?)
                    } else {
                        None
                    };
                    best = Some((j, tree, cost, end));
                }
                j
            }
            None => bound.unwrap_or(u128::MAX),
        };
        if *partition == Partition::None {
            unsplit = Some(j);
            if ends {
                #[cfg(test)]
                BREAKOUTS.set(BREAKOUTS.get() + 1);
                let breakout = PartitionOutcome::Pruned(PartitionPruning::NoneBreakout);
                for (partition, _) in candidates.iter().skip(index + 1) {
                    coder.trace_partition(node, *partition, None, breakout);
                }
                break;
            }
            continue;
        }
        if let Some((axis, false)) = extended_shape(*partition) {
            three_way[axis] = Some(j);
        }
        let split = best_split.map_or(j, |b: u128| b.min(j));
        best_split = Some(split);
        exited |= early_exit && unsplit.is_some_and(|none| none <= split);
    }
    let Some((_, tree, cost, end)) = best.filter(|(j, ..)| budget.is_none_or(|budget| *j < budget))
    else {
        if let Some(budget) = budget {
            remember(memo, key, node, Seen::Bound(budget, leaf));
            return Ok(None);
        }
        return Ok(Some((
            PartitionTree::Leaf(LeafChoice::DC),
            RdCost::default(),
        )));
    };
    if let Some(end) = &end {
        coder.restore(end)?;
    }
    if let Some(trial) = best_trial {
        coder.trace_selected(trial);
    }
    remember(memo, key, node, Seen::Tree(tree.clone()));
    Ok(Some((tree, cost)))
}

/// The cheaper of the shared-tree result `split` of `partition` at `node`
/// and an `INTRA_REGION` coded from `start` with the luma children searched
/// as the shared children are, under `bound`; leaves the coder in the state
/// of the result. `split` when the node codes no `region_type`.
fn better_region<S: PartitionSearch>(
    coder: &mut S,
    (node, partition): (S::Node, Partition),
    (preset, lambda): (&PresetParams, Lambda),
    (memo, levels): (&mut Memo<S::Node>, Option<&Levels>),
    (nested, bound): (bool, Option<u128>),
    (start, split): (&S::Saved, Found),
) -> Result<Found, S::Error> {
    let split_end = coder.save(node, Some(start))?;
    coder.restore(start)?;
    let rate = coder.partition_rate(node, partition)?;
    let Some(region_rate) = coder.region_rate(node, partition, true)? else {
        coder.restore(&split_end)?;
        return Ok(split);
    };
    let bound = lower(bound, split.as_ref().map(|(_, cost)| cost.cost(lambda)));
    let (children, chroma) = coder.region_children(node, partition)?;
    let mut cost = RdCost {
        distortion: 0,
        rate: rate + region_rate,
    };
    let mut luma = Vec::with_capacity(children.len());
    for &child in &children {
        let spent = cost.cost(lambda);
        if bound.is_some_and(|bound| spent >= bound) {
            break;
        }
        let rest = bound.map(|bound| bound - spent);
        let found = search(coder, child, preset, lambda, (memo, levels), (nested, rest))?;
        let Some((tree, child_cost)) = found else {
            break;
        };
        luma.push(tree);
        cost = cost + child_cost;
    }
    if luma.len() == children.len() && bound.is_none_or(|bound| cost.cost(lambda) < bound) {
        let (chroma_cost, choice) = coder.evaluate_leaf(chroma)?;
        cost = cost + chroma_cost;
        if bound.is_none_or(|bound| cost.cost(lambda) < bound) {
            #[cfg(test)]
            REGIONS.set(REGIONS.get() + 1);
            let tree = PartitionTree::Region {
                kind: partition,
                luma,
                chroma: choice,
            };
            return Ok(Some((tree, cost)));
        }
    }
    coder.restore(&split_end)?;
    Ok(split)
}

/// Whether the `NONE` cost `cost` of a block of luma log2 size `(w, h)` is
/// under the breakout thresholds of `level`: a distortion per luma sample of
/// at most `(level * sqrt(lambda) / 16)^2` and a rate per luma sample of at
/// most `level / 256` bits.
fn cheap(cost: RdCost, (w, h): (u32, u32), lambda: Lambda, level: u8) -> bool {
    let samples = 1u128 << (w + h);
    let scale = u128::from(lambda.qp_scaled(u64::from(level)));
    u128::from(cost.distortion) << 24 <= scale * scale * samples
        && u128::from(cost.rate.0) << 8 <= (u128::from(level) * samples) << Bits::FRAC_BITS
}

/// Codes `tree` at `node` into the live state, partition symbols before the
/// children as the bitstream orders them; returns its cost.
fn recode<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    tree: &PartitionTree,
) -> Result<RdCost, S::Error> {
    if !coder.in_frame(node) {
        return Ok(RdCost::default());
    }
    let partition = match (tree, coder.sdp_roots(node)) {
        (PartitionTree::Sdp { luma, chroma }, Some((luma_root, chroma_root))) => {
            return Ok(recode(coder, luma_root, luma)? + recode(coder, chroma_root, chroma)?);
        }
        (PartitionTree::Sdp { .. }, None) | (_, Some(_)) => {
            return Err(TileWriterError::Shape("SDP tree at a shared node or none").into());
        }
        (PartitionTree::Leaf(_), None) => Partition::None,
        (PartitionTree::Split { kind, .. } | PartitionTree::Region { kind, .. }, None) => *kind,
    };
    let mut rate = coder.partition_rate(node, partition)?;
    if partition != Partition::None {
        let intra = matches!(tree, PartitionTree::Region { .. });
        match coder.region_rate(node, partition, intra)? {
            Some(region) => rate += region,
            None if intra => {
                return Err(
                    TileWriterError::Shape("a region at a node without region_type").into(),
                );
            }
            None => {}
        }
    }
    let mut cost = match tree {
        PartitionTree::Split { kind, children } => {
            let mut cost = RdCost::default();
            for (child, tree) in coder.children(node, *kind)?.into_iter().zip(children) {
                cost = cost + recode(coder, child, tree)?;
            }
            cost
        }
        PartitionTree::Leaf(choice) => coder.code_leaf(node, *choice)?,
        PartitionTree::Region { kind, luma, chroma } => {
            let (children, chroma_node) = coder.region_children(node, *kind)?;
            let mut cost = RdCost::default();
            for (child, tree) in children.into_iter().zip(luma) {
                cost = cost + recode(coder, child, tree)?;
            }
            cost + coder.code_leaf(chroma_node, *chroma)?
        }
        PartitionTree::Sdp { .. } => RdCost::default(),
    };
    cost.rate += rate;
    Ok(cost)
}

/// The first-pass (PD0) partition levels of the square superblock `(left,
/// top, side)` from the lookahead motion `fields` alone: each square of side
/// 64 down to 8 costs its least open-loop SAD over the references plus
/// `preset.pd0_leaf_bits` at the SAD-domain `lambda` (scaled to 8-bit SAD by
/// `depth` bits), and splits when its four quadrants cost less. `None` when
/// a reference lacks motion for a square.
pub(crate) fn first_pass_levels(
    fields: &[Option<Arc<FrameMotion>>],
    (left, top, side): (usize, usize, usize),
    (lambda, depth): (Lambda, u32),
    (preset, context): (&PresetParams, PresetContext),
) -> Option<Levels> {
    let fields: Vec<&FrameMotion> = fields.iter().flatten().map(AsRef::as_ref).collect();
    if fields.is_empty() {
        return None;
    }
    let leaf_cost = lambda.qp_scaled(u64::from(preset.pd0_leaf_bits)) >> (8 + depth);
    let stride = side / 8;
    let mut levels = Levels {
        origin: (left, top),
        stride,
        cells: vec![0; stride * stride],
        window: preset.pd_window.at(context),
    };
    let quarter = side.min(64);
    for y in (top..top + side).step_by(quarter) {
        for x in (left..left + side).step_by(quarter) {
            first_pass_square(&fields, (x, y, quarter), leaf_cost, &mut levels)?;
        }
    }
    Some(levels)
}

/// The first-pass cost of the square `(x, y, s)` and its best subdivision
/// into squares down to 8, whose levels it writes into `levels` (a leaf
/// overwrites what its quadrants wrote); `None` when a reference lacks
/// motion for a square.
fn first_pass_square(
    fields: &[&FrameMotion],
    (x, y, s): (usize, usize, usize),
    leaf_cost: u64,
    levels: &mut Levels,
) -> Option<u64> {
    let size = s.ilog2() as usize - 3;
    let mut sad = u64::MAX;
    for field in fields {
        let (stride, blocks) = field.blocks(size);
        let found = blocks
            .get((y / s) * stride + x / s)
            .filter(|_| x / s < stride)?;
        if found.sad != u32::MAX {
            sad = sad.min(u64::from(found.sad));
        }
    }
    let leaf = sad.checked_add(leaf_cost);
    if s > 8 {
        let half = s / 2;
        let mut split = 0u64;
        for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
            split = split.saturating_add(first_pass_square(
                fields,
                (x + dx, y + dy, half),
                leaf_cost,
                levels,
            )?);
        }
        if leaf.is_none_or(|leaf| split < leaf) {
            return Some(split);
        }
    }
    let leaf = leaf?;
    let (origin, stride) = (levels.origin, levels.stride);
    let level = 2 * s.ilog2() as u8;
    for row in (y - origin.1) / 8..(y - origin.1 + s) / 8 {
        for col in (x - origin.0) / 8..(x - origin.0 + s) / 8 {
            levels.cells[row * stride + col] = level;
        }
    }
    Some(leaf)
}

type Candidates<N> = Vec<(Partition, Vec<N>)>;

/// The codable partitions of `node` inside the preset's block limits, `NONE`
/// first and `SPLIT` second, with their children; extended shapes only when
/// `extended`.
fn candidates<S: PartitionSearch>(
    coder: &S,
    node: S::Node,
    preset: &PresetParams,
    extended: bool,
) -> Result<Candidates<S::Node>, S::Error> {
    let mut all = Vec::new();
    for partition in coder.partitions(node)? {
        let children = if partition == Partition::None {
            Vec::new()
        } else {
            coder.children(node, partition)?
        };
        all.push((partition, children));
    }
    let (w, h) = coder.block_log2(node)?;
    let halves_long_side = if w > h {
        Partition::Vert
    } else {
        Partition::Horz
    };
    all.sort_by_key(|&(partition, _)| match partition {
        Partition::None => 0,
        Partition::Split => 1,
        _ if partition == halves_long_side => 2,
        _ => 3,
    });
    if !extended {
        all.retain(|(partition, _)| extended_shape(*partition).is_none());
    }
    let mut within = Vec::new();
    for (partition, children) in &all {
        let fits = if *partition == Partition::None {
            let (w, h) = coder.block_log2(node)?;
            w.max(h) <= u32::from(preset.max_block_log2)
        } else {
            let mut fits = true;
            for &child in children.iter().filter(|&&child| coder.in_frame(child)) {
                let (w, h) = coder.block_log2(child)?;
                fits &= w.min(h) >= u32::from(preset.min_block_log2.at(coder.context()));
            }
            fits
        };
        if fits {
            within.push((*partition, children.clone()));
        }
    }
    Ok(if within.is_empty() { all } else { within })
}

/// The axis (`0` rows, `1` columns) of an extended shape, and whether it is
/// an uneven four-way shape.
fn extended_shape(partition: Partition) -> Option<(usize, bool)> {
    Some(match partition {
        Partition::Horz3 => (0, false),
        Partition::Vert3 => (1, false),
        Partition::Horz4A | Partition::Horz4B => (0, true),
        Partition::Vert4A | Partition::Vert4B => (1, true),
        _ => return None,
    })
}

/// Whether cost `j` is within `margin`/256 of `best`; `u8::MAX` accepts any
/// cost, and a missing cost passes only while nothing is coded yet.
fn within_margin(j: Option<u128>, best: Option<u128>, margin: u8) -> bool {
    match (j, best) {
        _ if margin == u8::MAX => true,
        (Some(j), Some(best)) => j * 256 <= best * (256 + u128::from(margin)),
        (_, None) => true,
        (None, Some(_)) => false,
    }
}
