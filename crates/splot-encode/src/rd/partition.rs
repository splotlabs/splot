// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Recursive rate-distortion partition search, independent of leaf coding.

use std::collections::HashMap;

use std::sync::Arc;

use super::{Bits, Lambda, RdCost};
use crate::hme::FrameMotion;
use crate::preset::{PresetContext, PresetParams};
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
    /// The luma `(left, top)` sample of `node`.
    fn origin(&self, node: Self::Node) -> (usize, usize);
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
    /// The frame context of the preset's [`crate::preset::Levels`] fields.
    fn context(&self) -> PresetContext {
        PresetContext::default()
    }
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
    search(coder, node, preset, lambda, (&mut memo, levels), true)
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

/// The tree each searched node chose, by node key and extended-shape mode.
type Memo<N> = HashMap<(u64, bool), Vec<(N, PartitionTree)>>;

fn search<S: PartitionSearch>(
    coder: &mut S,
    node: S::Node,
    preset: &PresetParams,
    lambda: Lambda,
    (memo, levels): (&mut Memo<S::Node>, Option<&Levels>),
    extended: bool,
) -> Result<(PartitionTree, RdCost), S::Error> {
    if !coder.in_frame(node) {
        return Ok((PartitionTree::Leaf(LeafChoice::DC), RdCost::default()));
    }
    let key = (coder.key(node), extended);
    if let Some((_, tree)) = memo
        .get(&key)
        .and_then(|seen| seen.iter().find(|(seen, _)| *seen == node))
    {
        let tree = tree.clone();
        let cost = recode(coder, node, &tree)?;
        return Ok((tree, cost));
    }
    if let Some((luma_root, chroma_root)) = coder.sdp_roots(node) {
        let (luma, luma_cost) = search(coder, luma_root, preset, lambda, (memo, levels), extended)?;
        coder.follow_luma(Some((luma_root, &luma)))?;
        let chroma = search(coder, chroma_root, preset, lambda, (memo, levels), extended);
        coder.follow_luma(None)?;
        let (chroma, chroma_cost) = chroma?;
        let tree = PartitionTree::Sdp {
            luma: Box::new(luma),
            chroma: Box::new(chroma),
        };
        memo.entry(key).or_default().push((node, tree.clone()));
        return Ok((tree, luma_cost + chroma_cost));
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
            for &child in children {
                let nested = extended && extended_shape(*partition).is_none();
                let (tree, child_cost) =
                    search(coder, child, preset, lambda, (memo, levels), nested)?;
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
        exited |= preset.partition_early_exit.at(coder.context())
            && unsplit.is_some_and(|none| none <= split);
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
    preset: &PresetParams,
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
        window: preset.pd_window,
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
