// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Recursive rate-distortion partition search, independent of leaf coding.

use std::collections::HashMap;

use super::{Bits, Lambda, RdCost};
use crate::preset::PresetParams;
use crate::tile_writer::{LeafChoice, Partition, PartitionTree, TileWriterError};

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
    /// Codes `node` as a leaf into the live state; returns its cost and decision.
    fn evaluate_leaf(&mut self, node: Self::Node) -> Result<(RdCost, LeafChoice), Self::Error>;
    /// Codes `node` as a leaf with `choice` into the live state; returns its cost.
    fn code_leaf(&mut self, node: Self::Node, choice: LeafChoice) -> Result<RdCost, Self::Error>;
    /// The rate of the partition symbols that select `partition` at `node`.
    fn partition_rate(
        &mut self,
        node: Self::Node,
        partition: Partition,
    ) -> Result<Bits, Self::Error>;
    /// Saves the state that coding `node` can change; with `start`, a live
    /// save of the same node, the state a trial reached from it.
    fn save(
        &mut self,
        node: Self::Node,
        start: Option<&Self::Saved>,
    ) -> Result<Self::Saved, Self::Error>;
    /// Restores a state saved for the same node.
    fn restore(&mut self, saved: &Self::Saved) -> Result<(), Self::Error>;
}

/// Finds the partition tree of `node` with the lowest `J = D + lambda * R` and
/// leaves the coder in the state of that tree.
///
/// A leaf is tried only up to `max_block_log2` per side, a split only when every
/// child keeps both sides at least `min_block_log2`; when the limits leave no
/// candidate, every codable partition is tried. `NONE` is tried first and
/// `SPLIT` second; with `partition_early_exit`, the search stops once a split
/// is coded and the unsplit cost does not exceed the best split cost. A
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
/// A three-way shape is tried only when the `NONE` cost is within the
/// preset's margin of the best cost so far, an uneven four-way shape only
/// when the three-way shape of its axis is; the early exit skips the other
/// remaining partitions but leaves extended shapes to these margins. The children of an extended shape
/// search without extended shapes, and a node keeps one tree per mode.
///
/// # Errors
/// Returns the coder's errors.
pub(crate) fn search_partition<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
) -> Result<(PartitionTree, RdCost), S::Error> {
    search_guided(coder, node, preset, lambda, Guide::Free(u8::MAX))
}

/// [`search_partition`] limited by `guide`, the tree of a first pass (PD0):
/// a node follows the guide's partition, and tries the others only within
/// `preset.pd_window` binary levels of a guide leaf, below or above it.
///
/// # Errors
/// Returns the coder's errors.
pub(crate) fn search_guided<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    guide: Guide<'_>,
) -> Result<(PartitionTree, RdCost), S::Error> {
    search(coder, node, preset, lambda, &mut Memo::new(), true, guide)
}

/// What a guided search may try at a node.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Guide<'t> {
    /// The first-pass tree of the node.
    Tree(&'t PartitionTree),
    /// Every partition, with this many binary levels of splits below; `u8::MAX`
    /// is unlimited.
    Free(u8),
}

impl Guide<'_> {
    /// The guide of child `index` of `partition`, or `None` when the node may
    /// not try `partition`.
    fn child(self, partition: Partition, index: usize, window: u8) -> Option<Self> {
        let free = |levels: u8| match levels {
            u8::MAX => Some(Guide::Free(u8::MAX)),
            0 => None,
            levels => Some(Guide::Free(levels - 1)),
        };
        match self {
            Guide::Free(levels) => free(levels),
            Guide::Tree(PartitionTree::Split { kind, children }) if *kind == partition => Some(
                children
                    .get(index)
                    .map_or(Guide::Free(u8::MAX), Guide::Tree),
            ),
            Guide::Tree(tree @ PartitionTree::Split { .. }) => {
                (leaf_depth(tree) <= u32::from(window)).then_some(Guide::Free(0))
            }
            Guide::Tree(PartitionTree::Leaf(_)) => free(window),
            Guide::Tree(PartitionTree::Sdp { .. }) => Some(Guide::Free(u8::MAX)),
        }
    }

    /// A key that tells the guides of a node apart: the first-pass subtree
    /// by address, a level budget by value.
    fn memo_key(self) -> usize {
        match self {
            Guide::Tree(tree) => core::ptr::from_ref(tree).addr(),
            Guide::Free(levels) => usize::from(levels),
        }
    }

    /// Whether the node may code `NONE`.
    fn allows_none(self, window: u8) -> bool {
        match self {
            Guide::Tree(tree @ PartitionTree::Split { .. }) => {
                leaf_depth(tree) <= u32::from(window)
            }
            _ => true,
        }
    }
}

/// The binary levels from `tree` to its shallowest leaf.
fn leaf_depth(tree: &PartitionTree) -> u32 {
    match tree {
        PartitionTree::Leaf(_) => 0,
        PartitionTree::Split { children, .. } => {
            1 + children.iter().map(leaf_depth).min().unwrap_or(0)
        }
        PartitionTree::Sdp { luma, .. } => leaf_depth(luma),
    }
}

/// The tree each searched node chose, by node key, extended-shape mode and
/// guide.
type Memo<N> = HashMap<(u64, bool, usize), Vec<(N, PartitionTree)>>;

fn search<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    memo: &mut Memo<S::Node>,
    extended: bool,
    guide: Guide<'_>,
) -> Result<(PartitionTree, RdCost), S::Error> {
    if !coder.in_frame(node) {
        return Ok((PartitionTree::Leaf(LeafChoice::DC), RdCost::default()));
    }
    let key = (coder.key(node), extended, guide.memo_key());
    if let Some((_, tree)) = memo
        .get(&key)
        .and_then(|seen| seen.iter().find(|(seen, _)| *seen == node))
    {
        let tree = tree.clone();
        let cost = recode(coder, node, &tree)?;
        return Ok((tree, cost));
    }
    if let Some((luma_root, chroma_root)) = coder.sdp_roots(node) {
        let (luma_guide, chroma_guide) = match guide {
            Guide::Tree(PartitionTree::Sdp { luma, chroma }) => {
                (Guide::Tree(luma), Guide::Tree(chroma))
            }
            _ => (guide, guide),
        };
        let (luma, luma_cost) =
            search(coder, luma_root, preset, lambda, memo, extended, luma_guide)?;
        coder.follow_luma(Some((luma_root, &luma)))?;
        let chroma = search(
            coder,
            chroma_root,
            preset,
            lambda,
            memo,
            extended,
            chroma_guide,
        );
        coder.follow_luma(None)?;
        let (chroma, chroma_cost) = chroma?;
        let tree = PartitionTree::Sdp {
            luma: Box::new(luma),
            chroma: Box::new(chroma),
        };
        memo.entry(key).or_default().push((node, tree.clone()));
        return Ok((tree, luma_cost + chroma_cost));
    }
    let candidates = guided(
        candidates(coder, node, preset, extended)?,
        guide,
        preset.pd_window,
    );
    let start = if candidates.len() > 1 {
        Some(coder.save(node, None)?)
    } else {
        None
    };
    let mut best: Option<(u128, PartitionTree, RdCost, Option<S::Saved>)> = None;
    let mut unsplit = None;
    let mut best_split = None;
    let mut three_way = [None; 2];
    let mut exited = false;
    for (index, (partition, children)) in candidates.iter().enumerate() {
        let best_j = best.as_ref().map(|(j, ..)| *j);
        let gate = match extended_shape(*partition) {
            Some((_, false)) => Some((unsplit, preset.ext_three_way_margin)),
            Some((axis, true)) => Some((three_way[axis], preset.ext_four_way_margin)),
            None => None,
        };
        let gated = gate.map(|(j, margin)| within_margin(j, best_j, margin));
        if gated.map_or(exited, |accepted| !accepted) {
            continue;
        }
        if index > 0
            && let Some(start) = &start
        {
            coder.restore(start)?;
        }
        let rate = coder.partition_rate(node, *partition)?;
        let (tree, mut cost) = if *partition == Partition::None {
            let (cost, leaf) = coder.evaluate_leaf(node)?;
            (PartitionTree::Leaf(leaf), cost)
        } else {
            let mut trees = Vec::with_capacity(children.len());
            let mut cost = RdCost::default();
            for (index, &child) in children.iter().enumerate() {
                let nested = extended && extended_shape(*partition).is_none();
                let child_guide = guide
                    .child(*partition, index, preset.pd_window)
                    .unwrap_or(Guide::Free(0));
                let (tree, child_cost) =
                    search(coder, child, preset, lambda, memo, nested, child_guide)?;
                trees.push(tree);
                cost = cost + child_cost;
            }
            let tree = PartitionTree::Split {
                kind: *partition,
                children: trees,
            };
            (tree, cost)
        };
        cost.rate += rate;
        let j = cost.cost(lambda);
        if best.as_ref().is_none_or(|(best_j, ..)| j < *best_j) {
            let end = if index + 1 < candidates.len() {
                Some(coder.save(node, start.as_ref())?)
            } else {
                None
            };
            best = Some((j, tree, cost, end));
        }
        if *partition == Partition::None {
            unsplit = Some(j);
            continue;
        }
        if let Some((axis, false)) = extended_shape(*partition) {
            three_way[axis] = Some(j);
        }
        let split = best_split.map_or(j, |b: u128| b.min(j));
        best_split = Some(split);
        exited |= preset.partition_early_exit && unsplit.is_some_and(|none| none <= split);
    }
    let Some((_, tree, cost, end)) = best else {
        return Ok((PartitionTree::Leaf(LeafChoice::DC), RdCost::default()));
    };
    if let Some(end) = &end {
        coder.restore(end)?;
    }
    memo.entry(key).or_default().push((node, tree.clone()));
    Ok((tree, cost))
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
        (PartitionTree::Split { kind, .. }, None) => *kind,
    };
    let rate = coder.partition_rate(node, partition)?;
    let mut cost = match tree {
        PartitionTree::Split { kind, children } => {
            let mut cost = RdCost::default();
            for (child, tree) in coder.children(node, *kind)?.into_iter().zip(children) {
                cost = cost + recode(coder, child, tree)?;
            }
            cost
        }
        PartitionTree::Leaf(choice) => coder.code_leaf(node, *choice)?,
        PartitionTree::Sdp { .. } => RdCost::default(),
    };
    cost.rate += rate;
    Ok(cost)
}

type Candidates<N> = Vec<(Partition, Vec<N>)>;

/// The `candidates` that `guide` allows, `NONE` first and the guide's
/// partition second so that the early exit weighs it, or all of them when
/// it allows none or its partition is not among them.
fn guided<N>(candidates: Candidates<N>, guide: Guide<'_>, window: u8) -> Candidates<N> {
    if let Guide::Tree(PartitionTree::Split { kind, .. }) = guide
        && !candidates.iter().any(|(partition, _)| partition == kind)
    {
        return candidates;
    }
    let allowed = |partition: Partition| match partition {
        Partition::None => guide.allows_none(window),
        _ => guide.child(partition, 0, window).is_some(),
    };
    if !candidates.iter().any(|&(partition, _)| allowed(partition)) {
        return candidates;
    }
    let mut kept: Candidates<N> = candidates
        .into_iter()
        .filter(|&(partition, _)| allowed(partition))
        .collect();
    if let Guide::Tree(PartitionTree::Split { kind, .. }) = guide {
        kept.sort_by_key(|(partition, _)| match partition {
            Partition::None => 0,
            partition if partition == kind => 1,
            _ => 2,
        });
    }
    kept
}

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
    all.sort_by_key(|(partition, _)| match partition {
        Partition::None => 0,
        Partition::Split => 1,
        _ => 2,
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
                fits &= w.min(h) >= u32::from(preset.min_block_log2);
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
