// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The partition search against a synthetic square-frame coder.

use core::result::Result;

use super::*;
use crate::preset::{PRESETS, PresetParams};
use crate::rd::partition::{PartitionSearch, search_partition};
use crate::tile_writer::LeafChoice;

/// A block: left, top, log2 width, log2 height.
type Rect = (u32, u32, u32, u32);

/// Quad, binary, `HORZ_3`, `HORZ_4A` and `NONE` partitions down to 4x4 over a
/// `size x size` frame; the live state is the list of coded leaves.
struct Synthetic<F> {
    size: u32,
    leaf_cost: F,
    coded: Vec<Rect>,
    leaf_evaluations: usize,
    evaluated: std::collections::HashMap<Rect, usize>,
}

impl<F: Fn(Rect) -> RdCost> Synthetic<F> {
    fn new(size: u32, leaf_cost: F) -> Self {
        Self {
            size,
            leaf_cost,
            coded: Vec::new(),
            leaf_evaluations: 0,
            evaluated: std::collections::HashMap::new(),
        }
    }
}

impl<F: Fn(Rect) -> RdCost> PartitionSearch for Synthetic<F> {
    type Node = Rect;
    type Saved = Vec<Rect>;
    type Error = crate::tile_writer::TileWriterError;

    fn in_frame(&self, (x, y, ..): Rect) -> bool {
        x < self.size && y < self.size
    }

    fn sdp_roots(&self, _: Rect) -> Option<(Rect, Rect)> {
        None
    }

    fn follow_luma(&mut self, _: Option<(Rect, &PartitionTree)>) -> Result<(), Self::Error> {
        Ok(())
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
        if h > 3 && w > 2 {
            out.push(Partition::Horz3);
        }
        if h > 4 {
            out.push(Partition::Horz4A);
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
            Partition::Horz3 => vec![
                (x, y, w, h - 2),
                (x, y + hh / 2, w - 1, h - 1),
                (x + hw, y + hh / 2, w - 1, h - 1),
                (x, y + 3 * hh / 2, w, h - 2),
            ],
            Partition::Horz4A => vec![
                (x, y, w, h - 3),
                (x, y + hh / 4, w, h - 2),
                (x, y + 3 * hh / 4, w, h - 1),
                (x, y + 7 * hh / 4, w, h - 3),
            ],
            _ => Vec::new(),
        })
    }

    fn key(&self, (x, y, w, h): Rect) -> u64 {
        u64::from(x) << 48 | u64::from(y) << 32 | u64::from(w) << 16 | u64::from(h)
    }

    fn block_log2(&self, (_, _, w, h): Rect) -> Result<(u32, u32), Self::Error> {
        Ok((w, h))
    }

    fn origin(&self, (x, y, ..): Rect) -> (usize, usize) {
        (x as usize, y as usize)
    }

    fn evaluate_leaf(&mut self, node: Rect) -> Result<(RdCost, LeafChoice), Self::Error> {
        self.coded.push(node);
        self.leaf_evaluations += 1;
        let searches = self.evaluated.entry(node).or_default();
        *searches += 1;
        assert!(*searches <= 2, "{node:?} searched again in one mode");
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
        min_block_log2: crate::preset::Levels::all(2),
        max_block_log2: 7,
        partition_early_exit: crate::preset::Levels::all(false),
        ext_three_way_margin: u8::MAX,
        ext_four_way_margin: u8::MAX,
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
        PartitionTree::Sdp { .. } => unreachable!("the synthetic coder has no SDP node"),
    }
}

/// The optimum `J` by plain recursion, without the search's state handling;
/// the children of an extended shape take no extended shape.
fn optimum<F: Fn(Rect) -> RdCost>(coder: &Synthetic<F>, node: Rect, lambda: Lambda) -> u128 {
    optimum_in(coder, node, lambda, true)
}

fn optimum_in<F: Fn(Rect) -> RdCost>(
    coder: &Synthetic<F>,
    node: Rect,
    lambda: Lambda,
    extended: bool,
) -> u128 {
    if !coder.in_frame(node) {
        return 0;
    }
    let is_extended = |p: Partition| matches!(p, Partition::Horz3 | Partition::Horz4A);
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
        .filter(|&partition| extended || !is_extended(partition))
        .map(|partition| {
            rate(partition)
                + if partition == Partition::None {
                    (coder.leaf_cost)(node).cost(lambda)
                } else {
                    coder
                        .children(node, partition)
                        .unwrap()
                        .into_iter()
                        .map(|child| {
                            optimum_in(coder, child, lambda, extended && !is_extended(partition))
                        })
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
        min_block_log2: crate::preset::Levels::all(4),
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
        partition_early_exit: crate::preset::Levels::all(true),
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

/// With zero margins, the extended shapes are skipped unless `NONE` is the
/// best cost so far; the optimum needs `HORZ_3` here, so only the open
/// margins find it.
#[test]
fn extended_margins_gate_the_three_and_four_way_trials() {
    let bands = |(x, y, w, h): Rect| {
        let (top, bottom) = (y, y + (1 << h));
        let flat = [(0, 8), (8, 24), (24, 32)]
            .iter()
            .any(|&(a, b)| top >= a && bottom <= b)
            && (h < 4 || x % 16 + (1 << w) <= 16);
        RdCost {
            distortion: if flat { 0 } else { 1 << 24 },
            rate: Bits::whole(10),
        }
    };
    let root = (0, 0, 5, 5);
    let mut open = Synthetic::new(32, bands);
    let (tree, cost) = search_partition(&mut open, root, &full_search(), unit_lambda()).unwrap();
    assert!(
        matches!(
            tree,
            PartitionTree::Split {
                kind: Partition::Horz3,
                ..
            }
        ),
        "{tree:?}"
    );
    assert_eq!(
        cost.cost(unit_lambda()),
        optimum(&open, root, unit_lambda())
    );
    let closed = PresetParams {
        ext_three_way_margin: 0,
        ext_four_way_margin: 0,
        ..full_search()
    };
    let mut gated = Synthetic::new(32, bands);
    let (tree, _) = search_partition(&mut gated, root, &closed, unit_lambda()).unwrap();
    assert!(
        !matches!(
            tree,
            PartitionTree::Split {
                kind: Partition::Horz3,
                ..
            }
        ),
        "{tree:?}"
    );
    assert!(gated.leaf_evaluations < open.leaf_evaluations);
}

/// The early exit skips the remaining binary shapes but leaves the extended
/// shapes to their margins: with `NONE` best, `HORZ_3` is still tried.
#[test]
fn early_exit_leaves_extended_shapes_to_the_margins() {
    let flat = |_: Rect| RdCost {
        distortion: 0,
        rate: Bits::whole(1),
    };
    let preset = PresetParams {
        partition_early_exit: crate::preset::Levels::all(true),
        ext_three_way_margin: 0,
        ..full_search()
    };
    let mut coder = Synthetic::new(32, flat);
    let root = (0, 0, 5, 5);
    let (tree, _) = search_partition(&mut coder, root, &preset, unit_lambda()).unwrap();
    assert!(matches!(tree, PartitionTree::Leaf(_)));
    assert!(
        coder.evaluated.contains_key(&(0, 8, 4, 4)),
        "HORZ_3 was not tried"
    );
    assert!(
        !coder.evaluated.contains_key(&(0, 0, 5, 4)),
        "HORZ was tried"
    );
}

/// The first-pass levels limit a guided search to blocks near them: with
/// window 0 and the level of a 16x16 block over the frame, the search codes
/// blocks of that level, and the blocks next to it that only an extended
/// shape reaches as leaves; a wider window evaluates more, the full search
/// most.
#[test]
fn levels_limit_the_searched_blocks() {
    use crate::rd::partition::{Levels, search_guided};
    let cost = |(x, y, w, h): Rect| RdCost {
        distortion: u64::from(((x * 7 + y * 13) % 29 + 3) << (w + h)),
        rate: Bits::whole(w + h),
    };
    let levels = |window| Levels {
        origin: (0, 0),
        stride: 4,
        cells: vec![8; 16],
        window,
    };
    let root = (0, 0, 5, 5);
    let evaluate = |window: Option<u8>| {
        let mut coder = Synthetic::new(32, cost);
        let levels = window.map(levels);
        search_guided(
            &mut coder,
            root,
            &full_search(),
            unit_lambda(),
            levels.as_ref(),
        )
        .unwrap();
        let mut sizes: Vec<u32> = coder.evaluated.keys().map(|&(_, _, w, h)| w + h).collect();
        sizes.sort_unstable();
        sizes.dedup();
        (coder.leaf_evaluations, sizes)
    };
    let (exact, exact_sizes) = evaluate(Some(0));
    let (near, near_sizes) = evaluate(Some(1));
    let (full, full_sizes) = evaluate(None);
    assert!(exact_sizes.contains(&8) && exact_sizes.iter().all(|size| (6..=9).contains(size)));
    assert!(near_sizes.iter().all(|size| (5..=10).contains(size)));
    assert_eq!(full_sizes.first(), Some(&4));
    assert!(exact < near && near < full, "{exact} {near} {full}");
}

/// One open-loop field over a 64x64 superblock whose block of side
/// `8 << size` at `(col, row)` has SAD `sad(size, col, row)`.
fn field(sad: impl Fn(usize, usize, usize) -> u32) -> std::sync::Arc<crate::hme::FrameMotion> {
    std::sync::Arc::new(crate::hme::FrameMotion::from_fn(
        (1, 1),
        |size, col, row| crate::kernels::motion::Motion {
            sad: sad(size, col, row),
            mv: [0, 0],
        },
    ))
}

/// The first pass splits the one quadrant whose 16x16 blocks predict far
/// better than it, keeps the others whole, and gives up without motion.
#[test]
fn first_pass_levels_split_where_the_quadrants_cost_less() {
    use crate::rd::partition::first_pass_levels;
    let preset = PresetParams {
        pd_window: 2,
        pd0_leaf_bits: 8,
        ..PRESETS[6]
    };
    let in_first = |size: usize, col: usize, row: usize| {
        let side = 8usize << size;
        col * side < 32 && row * side < 32
    };
    let fields = [Some(field(|size, col, row| {
        match (size, in_first(size, col, row)) {
            (3, _) => 1_000_000,
            (2, true) => 100_000,
            (2, false) => 1000,
            (1, true) => 10,
            (1, false) => 400,
            _ => 200,
        }
    }))];
    let levels = first_pass_levels(&fields, (0, 0, 64), (unit_lambda(), 0), &preset).unwrap();
    assert_eq!(
        (levels.origin, levels.stride, levels.window),
        ((0, 0), 8, 2)
    );
    for (index, &level) in levels.cells.iter().enumerate() {
        let (col, row) = (index % 8, index / 8);
        let expected = if col < 4 && row < 4 { 8 } else { 10 };
        assert_eq!(level, expected, "cell ({col}, {row})");
    }
    let missing = [Some(field(
        |size, _, _| if size == 0 { u32::MAX } else { 50 },
    ))];
    assert!(first_pass_levels(&missing, (0, 0, 64), (unit_lambda(), 0), &preset).is_none());
    assert!(first_pass_levels(&[None], (0, 0, 64), (unit_lambda(), 0), &preset).is_none());
}

/// A block whose only partition falls outside the window still codes it:
/// the frame-edge root of a 24x24 frame can only split, which window 0 over
/// the level of a 32x32 block rejects.
#[test]
fn a_forced_partition_outside_the_window_still_codes() {
    use crate::rd::partition::{Levels, search_guided};
    let cost = |(_, _, w, h): Rect| RdCost {
        distortion: u64::from(3u32 << (w + h)),
        rate: Bits::whole(1),
    };
    let levels = Levels {
        origin: (0, 0),
        stride: 4,
        cells: vec![10; 16],
        window: 0,
    };
    let mut coder = Synthetic::new(24, cost);
    let (tree, _) = search_guided(
        &mut coder,
        (0, 0, 5, 5),
        &full_search(),
        unit_lambda(),
        Some(&levels),
    )
    .unwrap();
    assert!(
        matches!(
            tree,
            PartitionTree::Split {
                kind: Partition::Split,
                ..
            }
        ),
        "{tree:?}"
    );
}
