// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The partition search against a synthetic square-frame coder.

use core::result::Result;

use super::*;
use crate::preset::{PRESETS, PresetParams};
use crate::rd::partition::{PartitionSearch, SourceProfile, search_partition};
use crate::tile_writer::LeafChoice;

/// A block: left, top, log2 width, log2 height.
type Rect = (u32, u32, u32, u32);

/// Quad, binary, and `NONE` partitions down to 4x4 over a `size x size` frame;
/// the live state is the list of coded leaves.
struct Synthetic<F> {
    size: u32,
    leaf_cost: F,
    coded: Vec<Rect>,
    leaf_evaluations: usize,
    evaluated: std::collections::HashSet<Rect>,
}

impl<F: Fn(Rect) -> RdCost> Synthetic<F> {
    fn new(size: u32, leaf_cost: F) -> Self {
        Self {
            size,
            leaf_cost,
            coded: Vec::new(),
            leaf_evaluations: 0,
            evaluated: std::collections::HashSet::new(),
        }
    }
}

impl<F: Fn(Rect) -> RdCost> PartitionSearch for Synthetic<F> {
    type Node = Rect;
    type Saved = Vec<Rect>;
    type Error = core::convert::Infallible;

    fn in_frame(&self, (x, y, ..): Rect) -> bool {
        x < self.size && y < self.size
    }

    fn partitions(&self, (x, y, w, h): Rect) -> Result<Vec<Partition>, Self::Error> {
        if x + (1 << w) > self.size || y + (1 << h) > self.size {
            return Ok(vec![Partition::Split]);
        }
        let mut out = vec![Partition::None];
        if w > 2 && h > 2 && w == h {
            out.push(Partition::Split);
        }
        if h > 2 {
            out.push(Partition::Horz);
        }
        if w > 2 {
            out.push(Partition::Vert);
        }
        Ok(out)
    }

    fn children(&self, (x, y, w, h): Rect, partition: Partition) -> Result<Vec<Rect>, Self::Error> {
        let (hw, hh) = (1 << (w - 1), 1 << (h - 1));
        Ok(match partition {
            Partition::Split => vec![
                (x, y, w - 1, h - 1),
                (x + hw, y, w - 1, h - 1),
                (x, y + hh, w - 1, h - 1),
                (x + hw, y + hh, w - 1, h - 1),
            ],
            Partition::Horz => vec![(x, y, w, h - 1), (x, y + hh, w, h - 1)],
            Partition::Vert => vec![(x, y, w - 1, h), (x + hw, y, w - 1, h)],
            _ => Vec::new(),
        })
    }

    fn key(&self, (x, y, w, h): Rect) -> u64 {
        u64::from(x) << 48 | u64::from(y) << 32 | u64::from(w) << 16 | u64::from(h)
    }

    fn block_log2(&self, (_, _, w, h): Rect) -> Result<(u32, u32), Self::Error> {
        Ok((w, h))
    }

    fn source_profile(&self, _: Rect) -> Result<SourceProfile, Self::Error> {
        Ok(SourceProfile::default())
    }

    fn evaluate_leaf(&mut self, node: Rect) -> Result<(RdCost, LeafChoice), Self::Error> {
        self.coded.push(node);
        self.leaf_evaluations += 1;
        assert!(self.evaluated.insert(node), "{node:?} searched twice");
        Ok(((self.leaf_cost)(node), LeafChoice::DC))
    }

    fn code_leaf(&mut self, node: Rect, _: LeafChoice) -> Result<RdCost, Self::Error> {
        self.coded.push(node);
        Ok((self.leaf_cost)(node))
    }

    fn partition_rate(&mut self, _: Rect, partition: Partition) -> Result<Bits, Self::Error> {
        Ok(Bits::whole(if partition == Partition::None {
            1
        } else {
            2
        }))
    }

    fn save(&mut self, _: Rect, _: Option<&Vec<Rect>>) -> Result<Vec<Rect>, Self::Error> {
        Ok(self.coded.clone())
    }

    fn restore(&mut self, saved: &Vec<Rect>) -> Result<(), Self::Error> {
        self.coded.clone_from(saved);
        Ok(())
    }
}

fn unit_lambda() -> Lambda {
    Lambda {
        scale_q16: 1 << 16,
        step_sq_q12: 1 << 12,
    }
}

fn full_search() -> PresetParams {
    PresetParams {
        min_block_log2: 2,
        max_block_log2: 7,
        partition_early_exit: false,
        ..PRESETS[0]
    }
}

/// The coded leaves of `tree` at `node`, in coding order.
fn leaves<F: Fn(Rect) -> RdCost>(
    coder: &Synthetic<F>,
    node: Rect,
    tree: &PartitionTree,
    out: &mut Vec<Rect>,
) {
    if !coder.in_frame(node) {
        return;
    }
    match tree {
        PartitionTree::Leaf(_) => out.push(node),
        PartitionTree::Split { kind, children } => {
            for (child, tree) in coder
                .children(node, *kind)
                .unwrap()
                .into_iter()
                .zip(children)
            {
                leaves(coder, child, tree, out);
            }
        }
    }
}

/// The optimum `J` by plain recursion, without the search's state handling.
fn optimum<F: Fn(Rect) -> RdCost>(coder: &Synthetic<F>, node: Rect, lambda: Lambda) -> u128 {
    if !coder.in_frame(node) {
        return 0;
    }
    let rate = |partition| {
        RdCost {
            distortion: 0,
            rate: Bits::whole(if partition == Partition::None { 1 } else { 2 }),
        }
        .cost(lambda)
    };
    coder
        .partitions(node)
        .unwrap()
        .into_iter()
        .map(|partition| {
            rate(partition)
                + if partition == Partition::None {
                    (coder.leaf_cost)(node).cost(lambda)
                } else {
                    coder
                        .children(node, partition)
                        .unwrap()
                        .into_iter()
                        .map(|child| optimum(coder, child, lambda))
                        .sum()
                }
        })
        .min()
        .unwrap()
}

#[test]
fn finds_a_known_optimum_tree_and_leaves_its_state() {
    let target: [Rect; 9] = [
        (0, 0, 4, 4),
        (16, 0, 4, 3),
        (16, 8, 4, 3),
        (0, 16, 3, 4),
        (8, 16, 3, 4),
        (16, 16, 3, 3),
        (24, 16, 3, 3),
        (16, 24, 3, 3),
        (24, 24, 3, 3),
    ];
    let inside = move |(x, y, w, h): Rect| {
        target.iter().any(|&(tx, ty, tw, th)| {
            x >= tx && y >= ty && x + (1 << w) <= tx + (1 << tw) && y + (1 << h) <= ty + (1 << th)
        })
    };
    let mut coder = Synthetic::new(32, |node| RdCost {
        distortion: if inside(node) { 0 } else { 1 << 20 },
        rate: Bits::whole(10),
    });
    let root = (0, 0, 5, 5);
    let (tree, cost) = search_partition(&mut coder, root, &full_search(), unit_lambda()).unwrap();
    let mut found = Vec::new();
    leaves(&coder, root, &tree, &mut found);
    let mut expected = target.to_vec();
    let mut sorted = found.clone();
    sorted.sort_unstable();
    expected.sort_unstable();
    assert_eq!(sorted, expected);
    assert_eq!(coder.coded, found, "the live state is the chosen tree");
    assert_eq!(cost.distortion, 0);
    assert_eq!(
        cost.cost(unit_lambda()),
        optimum(&coder, root, unit_lambda())
    );
}

#[test]
fn frame_edge_children_are_not_coded() {
    let mut coder = Synthetic::new(24, |(.., w, h)| RdCost {
        distortion: u64::from(w + h),
        rate: Bits::whole(1),
    });
    let root = (0, 0, 5, 5);
    let (tree, _) = search_partition(&mut coder, root, &full_search(), unit_lambda()).unwrap();
    let mut found = Vec::new();
    leaves(&coder, root, &tree, &mut found);
    assert_eq!(coder.coded, found);
    assert!(
        found
            .iter()
            .all(|&(x, y, w, h)| x + (1 << w) <= 24 && y + (1 << h) <= 24)
    );
    let area: u32 = found.iter().map(|&(.., w, h)| 1 << (w + h)).sum();
    assert_eq!(area, 24 * 24);
}

#[test]
fn block_limits_bound_the_searched_sizes() {
    let flat = |_: Rect| RdCost {
        distortion: 0,
        rate: Bits::whole(1),
    };
    let preset = PresetParams {
        max_block_log2: 3,
        ..full_search()
    };
    let mut coder = Synthetic::new(32, flat);
    let (tree, _) = search_partition(&mut coder, (0, 0, 5, 5), &preset, unit_lambda()).unwrap();
    let mut found = Vec::new();
    leaves(&coder, (0, 0, 5, 5), &tree, &mut found);
    assert!(found.iter().all(|&(.., w, h)| w.max(h) <= 3));

    let busy = |(.., w, h): Rect| RdCost {
        distortion: 1 << (2 * (w + h)),
        rate: Bits::whole(1),
    };
    let preset = PresetParams {
        min_block_log2: 4,
        ..full_search()
    };
    let mut coder = Synthetic::new(32, busy);
    let (tree, _) = search_partition(&mut coder, (0, 0, 5, 5), &preset, unit_lambda()).unwrap();
    let mut found = Vec::new();
    leaves(&coder, (0, 0, 5, 5), &tree, &mut found);
    assert!(found.iter().all(|&(.., w, h)| w.min(h) >= 4));
    assert!(found.len() > 1);
}

#[test]
fn early_exit_skips_rectangles_when_unsplit_is_close() {
    let near_flat = |(x, y, w, h): Rect| RdCost {
        distortion: u64::from((x * 7 + y * 3) % 5) << (w + h),
        rate: Bits::whole(4),
    };
    let mut exhaustive = Synthetic::new(32, near_flat);
    let root = (0, 0, 5, 5);
    let (_, full) = search_partition(&mut exhaustive, root, &full_search(), unit_lambda()).unwrap();
    let preset = PresetParams {
        partition_early_exit: true,
        ..full_search()
    };
    let mut fast = Synthetic::new(32, near_flat);
    let (tree, cost) = search_partition(&mut fast, root, &preset, unit_lambda()).unwrap();
    assert!(fast.leaf_evaluations < exhaustive.leaf_evaluations);
    assert!(cost.cost(unit_lambda()) >= full.cost(unit_lambda()));
    let mut found = Vec::new();
    leaves(&fast, root, &tree, &mut found);
    assert_eq!(fast.coded, found);
}

proptest! {
    #[test]
    fn exhaustive_search_is_optimal(seed in any::<u64>(), size in prop::sample::select(vec![16u32, 24, 32])) {
        let cost = move |(x, y, w, h): Rect| {
            let hash = (u64::from(x) << 40 | u64::from(y) << 20 | u64::from(w) << 8 | u64::from(h))
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ seed;
            RdCost {
                distortion: (hash >> 40) % (64 << (w + h)),
                rate: Bits(hash % (64 << 8)),
            }
        };
        let mut coder = Synthetic::new(size, cost);
        let root = (0, 0, 5, 5);
        let (tree, found) = search_partition(&mut coder, root, &full_search(), unit_lambda()).unwrap();
        prop_assert_eq!(found.cost(unit_lambda()), optimum(&coder, root, unit_lambda()));
        let mut coded = Vec::new();
        leaves(&coder, root, &tree, &mut coded);
        prop_assert_eq!(&coder.coded, &coded);
        let leaf_sum = coded.iter().map(|&leaf| cost(leaf)).fold(RdCost::default(), |a, b| a + b);
        prop_assert_eq!(leaf_sum.distortion, found.distortion);
    }
}

/// A `size x size` plane whose rows take `levels` in bands of `eighths`.
fn banded(size: usize, eighths: &[usize], levels: &[u16]) -> Vec<u16> {
    let mut plane = Vec::with_capacity(size * size);
    for (&band, &level) in eighths.iter().zip(levels) {
        plane.resize(plane.len() + band * size / 8 * size, level);
    }
    plane
}

fn profile(plane: &[u16], size: usize) -> SourceProfile {
    SourceProfile::measure(plane, size, (0, 0, size, size)).unwrap()
}

#[test]
fn source_gate_tries_extended_shapes_that_fit_the_bands() {
    let three = profile(&banded(32, &[2, 4, 2], &[40, 200, 40]), 32);
    assert!(three.worth_trying(Partition::Horz3, 200));
    assert!(!three.worth_trying(Partition::Vert3, 1));
    assert!(three.worth_trying(Partition::Horz, u8::MAX));
    let uneven = profile(&banded(32, &[1, 2, 4, 1], &[10, 90, 170, 250]), 32);
    assert!(uneven.worth_trying(Partition::Horz4A, 100));
    assert!(!uneven.worth_trying(Partition::Horz4B, 100));
    let halves = profile(&banded(16, &[4, 4], &[10, 250]), 16);
    assert!(!halves.worth_trying(Partition::Horz3, 0));
    let flat = profile(&[77; 64 * 64], 64);
    for partition in [Partition::Horz3, Partition::Vert4A, Partition::Horz4B] {
        assert!(!flat.worth_trying(partition, 0));
    }
}

proptest! {
    #[test]
    fn source_profile_matches_direct_sums(
        seed in any::<u64>(),
        (log2_w, log2_h) in (2u32..=7, 2u32..=7),
        (x, y) in (0usize..9, 0usize..5),
        max in prop::sample::select(vec![255u16, 1023]),
    ) {
        let (width, height, stride) = (1usize << log2_w, 1usize << log2_h, 141);
        let plane: Vec<u16> = (0..stride * (y + height))
            .map(|at| (((at as u64 * 0x9e37_79b9) ^ seed) >> 7) as u16 % (max + 1))
            .collect();
        let profile = SourceProfile::measure(&plane, stride, (x, y, width, height)).unwrap();
        let sample = |r: usize, c: usize| u64::from(plane[(y + r) * stride + x + c]);
        let (strip_h, strip_w) = (height / profile.strips.0, width / profile.strips.1);
        for strip in 0..profile.strips.0 {
            let sum = (strip * strip_h..(strip + 1) * strip_h)
                .flat_map(|r| (0..width).map(move |c| (r, c)))
                .map(|(r, c)| sample(r, c))
                .sum::<u64>();
            prop_assert_eq!(profile.rows[strip], sum);
        }
        for strip in 0..profile.strips.1 {
            let sum = (0..height)
                .flat_map(|r| (strip * strip_w..(strip + 1) * strip_w).map(move |c| (r, c)))
                .map(|(r, c)| sample(r, c))
                .sum::<u64>();
            prop_assert_eq!(profile.cols[strip], sum);
        }
        let squares = (0..height)
            .flat_map(|r| (0..width).map(move |c| (r, c)))
            .map(|(r, c)| sample(r, c).pow(2))
            .sum::<u64>();
        prop_assert_eq!(profile.squares, squares);
        prop_assert_eq!(profile.samples, (width * height) as u64);
    }
}
