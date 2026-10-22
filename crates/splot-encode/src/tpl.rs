// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal dependency model of the lookahead window.
//!
//! Each planned frame's block takes the smaller of its intra cost and its inter
//! cost from the closest coded frames before and after it in display order,
//! the references the § 7.7 map ranks first. Walking the plan backwards, a
//! block passes `(intra + received) * (intra - best) / intra` to the blocks its
//! vector covers in that reference, split by overlap area, and split evenly
//! between both references when they tie. A frame's
//! importance is `(intra + received) / intra` over all its blocks. Its qindex
//! offset is `-12 * log2(importance)`, bounded: the § 7.14.2 quantizer step
//! doubles every 24 qindex, so the step shrinks by `sqrt(importance)` and
//! `lambda`, which goes with the squared step, by `importance`. Deeper layers
//! of the mini-GOP add [`LAYER_QINDEX_OFFSET`] per layer.

use std::collections::BTreeMap;

use crate::gop::GopFrame;
use crate::lookahead::{BLOCK, Lookahead};
use crate::rate_control::FrameType;
use crate::rd::log2_q8;

/// The largest qindex decrease the model gives a frame.
pub(crate) const MAX_TPL_QINDEX_DECREASE: i32 = 32;
/// The qindex increase per mini-GOP layer below the top one.
pub(crate) const LAYER_QINDEX_OFFSET: i32 = 3;
/// The smallest complexity a block adds: even a perfect prediction costs
/// mode and vector bits.
const MIN_BLOCK_COMPLEXITY: u32 = 128;
/// qindex change per octave of importance: half of the 24 per step octave.
const QINDEX_PER_IMPORTANCE_OCTAVE: u64 = 12;

/// The model's view of one planned frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TplFrame {
    /// Zero-based input (display) index.
    pub(crate) display_index: u64,
    /// Sum of the blocks' best costs, each at least [`MIN_BLOCK_COMPLEXITY`];
    /// intra costs in a key frame.
    pub(crate) complexity: u64,
    /// The suggested change to the base qindex.
    pub(crate) qindex_offset: i16,
}

/// A reference display index and the vector into it.
type Source = (u64, (i32, i32));

/// One frame's block choices.
struct Node {
    frame: GopFrame,
    intra: Vec<u32>,
    best: Vec<u32>,
    /// The references and vectors of each block that predict better than
    /// intra, two on a tie.
    source: Vec<[Option<Source>; 2]>,
}

/// Runs the model over `plan`, the coded frames of the window in coding
/// order; `coded_before` is the last frame coded before the window. Returns
/// one entry per frame of `plan`, in the same order.
pub(crate) fn analyze(
    lookahead: &mut Lookahead,
    plan: &[GopFrame],
    coded_before: Option<u64>,
) -> Vec<TplFrame> {
    let (cols, rows) = lookahead.grid();
    let blocks = cols * rows;
    let mut coded: Vec<u64> = coded_before.into_iter().collect();
    let mut nodes = Vec::with_capacity(plan.len());
    for frame in plan {
        let display = frame.display_index;
        if frame.frame_type == FrameType::Key {
            coded.clear();
        }
        let past = coded.iter().filter(|&&d| d < display).max().copied();
        let future = coded.iter().filter(|&&d| d > display).min().copied();
        coded.push(display);
        let intra = lookahead
            .intra(display)
            .map_or_else(|| vec![0; blocks], <[u32]>::to_vec);
        let mut best = intra.clone();
        let mut source = vec![[None; 2]; blocks];
        for reference in past.into_iter().chain(future) {
            let Some(motion) = lookahead.motion(display, reference) else {
                continue;
            };
            for (index, block) in motion.iter().enumerate() {
                let choice = Some((reference, block.mv));
                if block.cost < best[index] {
                    best[index] = block.cost;
                    source[index] = [choice, None];
                } else if block.cost == best[index] && source[index][0].is_some() {
                    source[index][1] = choice;
                }
            }
        }
        nodes.push(Node {
            frame: *frame,
            intra,
            best,
            source,
        });
    }

    let mut received: BTreeMap<u64, Vec<u64>> = plan
        .iter()
        .map(|frame| (frame.display_index, vec![0; blocks]))
        .collect();
    for node in nodes.iter().rev() {
        let own = received
            .get(&node.frame.display_index)
            .cloned()
            .unwrap_or_default();
        for (index, sources) in node.source.iter().enumerate() {
            let count = sources.iter().flatten().count() as u64;
            for &(reference, mv) in sources.iter().flatten() {
                let Some(target) = received.get_mut(&reference) else {
                    continue;
                };
                let intra = u64::from(node.intra[index]);
                let kept = intra - u64::from(node.best[index]);
                let amount = (intra + own.get(index).copied().unwrap_or(0)) * kept / intra;
                let (col, row) = ((index % cols) as i32, (index / cols) as i32);
                spread(target, (cols, rows), (col, row), mv, amount / count);
            }
        }
    }

    nodes
        .iter()
        .map(|node| {
            let intra: u64 = node.intra.iter().map(|&c| u64::from(c)).sum();
            let best: u64 = node
                .best
                .iter()
                .map(|&c| u64::from(c.max(MIN_BLOCK_COMPLEXITY)))
                .sum();
            let gained: u64 = received
                .get(&node.frame.display_index)
                .map_or(0, |values| values.iter().sum());
            let importance_q8 = (((intra + gained) << 8) / intra.max(1)).max(1 << 8);
            let octaves_q8 =
                u64::from(log2_q8(u32::try_from(importance_q8).unwrap_or(u32::MAX))) - (8 << 8);
            let decrease = ((octaves_q8 * QINDEX_PER_IMPORTANCE_OCTAVE + 128) >> 8) as i32;
            let offset = -decrease.min(MAX_TPL_QINDEX_DECREASE)
                + i32::from(node.frame.layer) * LAYER_QINDEX_OFFSET;
            TplFrame {
                display_index: node.frame.display_index,
                complexity: if node.frame.frame_type == FrameType::Key {
                    intra
                } else {
                    best
                },
                qindex_offset: offset as i16,
            }
        })
        .collect()
}

/// Adds `amount` to the blocks of `target` under block `(col, row)` moved by
/// `mv`, in proportion to the overlap.
fn spread(
    target: &mut [u64],
    (cols, rows): (usize, usize),
    (col, row): (i32, i32),
    mv: (i32, i32),
    amount: u64,
) {
    let side = BLOCK as i32;
    let (x, y) = (col * side + mv.0, row * side + mv.1);
    let (c0, r0) = (x.div_euclid(side), y.div_euclid(side));
    let (fx, fy) = (x.rem_euclid(side) as u64, y.rem_euclid(side) as u64);
    let side = BLOCK as u64;
    for (dc, dr, area) in [
        (0, 0, (side - fx) * (side - fy)),
        (1, 0, fx * (side - fy)),
        (0, 1, (side - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let (c, r) = (c0 + dc, r0 + dr);
        if area > 0 && (0..cols as i32).contains(&c) && (0..rows as i32).contains(&r) {
            target[r as usize * cols + c as usize] += amount * area / (side * side);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use core::num::NonZeroU32;

    use super::*;
    use crate::config::Config;
    use crate::gop::{GopPlanner, GopStep};
    use crate::lookahead::tests::frame;

    fn model(shift: usize) -> Vec<TplFrame> {
        let config = Config::new(64, 64);
        let mut lookahead = Lookahead::new(&config);
        for index in 0..5 {
            lookahead.push(&frame(&config, index, shift * index as usize, 0), index);
        }
        let mut planner = GopPlanner::new(NonZeroU32::new(100).unwrap(), 4);
        let plan: Vec<GopFrame> = [1, 4]
            .into_iter()
            .flat_map(|len| planner.plan(len).unwrap())
            .filter_map(|step| match step {
                GopStep::Code(frame) => Some(frame),
                GopStep::ShowExisting { .. } => None,
            })
            .collect();
        analyze(&mut lookahead, &plan, None)
    }

    #[test]
    fn referenced_frames_get_lower_qindex_on_static_and_moving_content() {
        for shift in [0, 2] {
            let frames = model(shift);
            let offset = |display: u64| {
                frames
                    .iter()
                    .find(|frame| frame.display_index == display)
                    .unwrap()
                    .qindex_offset
            };
            let leaf = offset(1).max(offset(3));
            assert_eq!(offset(1), offset(3), "shift {shift}");
            assert!(
                offset(2) < leaf && offset(4) < offset(2),
                "shift {shift}: {frames:?}"
            );
            assert!(offset(0) < leaf, "shift {shift}: {frames:?}");
            assert_eq!(leaf, 2 * LAYER_QINDEX_OFFSET as i16);
            assert!(
                frames[1].complexity < frames[0].complexity / 2,
                "{frames:?}"
            );
        }
        assert!(model(0)[0].qindex_offset < model(2)[0].qindex_offset);
    }
}
