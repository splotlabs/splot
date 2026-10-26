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

use splot_decode::encoder_syntax::dct_recon_params;
use splot_recon::{BitDepth, PlaneId};

use crate::gop::GopFrame;
use crate::kernels::kernels;
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
/// The share of the uniform quantization noise [`recon_noise`] adds to an
/// inter cost, in percent.
const RECON_NOISE_PERCENT: u64 = 50;

/// The model's view of one planned frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TplFrame {
    /// Zero-based input (display) index.
    pub(crate) display_index: u64,
    /// Sum of the blocks' best costs, intra costs in a key frame, each at
    /// least [`MIN_BLOCK_COMPLEXITY`].
    pub(crate) complexity: u64,
    /// The complexity of a frame whose blocks all cost the minimum.
    pub(crate) min_complexity: u64,
    /// The suggested change to the base qindex.
    pub(crate) qindex_offset: i16,
    /// `log2` of each block's importance over the frame's, Q8, in raster
    /// order; empty for a frame outside the [`BlockModel`].
    pub(crate) block_log2_beta: Vec<i16>,
}

/// The block-level pass of the model: the first `frames` frames of the plan,
/// with inter costs raised by `noise` for a coded reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockModel {
    pub(crate) frames: usize,
    pub(crate) noise: u32,
}

/// The block importances the encoder hands its frame coder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BlockImportance {
    /// Blocks per row and per column, and the luma side of a block.
    pub(crate) grid: (usize, usize, usize),
    /// The `tpl_delta_q` strength in percent.
    pub(crate) strength: u8,
    /// [`TplFrame::block_log2_beta`] by display index.
    pub(crate) frames: BTreeMap<u64, Vec<i16>>,
}

/// The lookahead SATD a block of `block_luma` luma samples gains from the
/// quantization noise of a reference coded at `qindex`: uniform noise of the
/// § 7.14.2 luma AC step `q` has deviation `q / sqrt(12)`, averaging down to
/// the lookahead scale divides it by `block_luma / 8`, and an 8x8 Hadamard
/// SATD of white noise is about 410 deviations. Scaled by
/// [`RECON_NOISE_PERCENT`].
pub(crate) fn recon_noise(qindex: u16, bit_depth: BitDepth, block_luma: usize) -> u32 {
    let Ok((params, _)) = dct_recon_params(u32::from(qindex), PlaneId::Y, 3, 3, bit_depth) else {
        return 0;
    };
    let step_q6 = u64::from(params.ac_quant) >> (bit_depth.bits() - 8);
    let noise = step_q6 * 410 * 8 * RECON_NOISE_PERCENT / (64 * 346 * block_luma as u64);
    u32::try_from(noise).unwrap_or(u32::MAX)
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
/// one entry per frame of `plan`, in the same order, with block importances
/// for the frames of `blocks_model`.
pub(crate) fn analyze(
    lookahead: &mut Lookahead,
    plan: &[GopFrame],
    coded_before: Option<u64>,
    blocks_model: Option<BlockModel>,
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

    let received = propagate(&nodes, (cols, rows), 0);
    let block_received = blocks_model.map(|model| {
        let frames = model.frames.min(nodes.len());
        propagate(&nodes[..frames], (cols, rows), model.noise)
    });
    nodes
        .iter()
        .map(|node| {
            let intra: u64 = node.intra.iter().map(|&c| u64::from(c)).sum();
            let floored = |costs: &[u32]| -> u64 {
                costs
                    .iter()
                    .map(|&c| u64::from(c.max(MIN_BLOCK_COMPLEXITY)))
                    .sum()
            };
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
                complexity: floored(if node.frame.frame_type == FrameType::Key {
                    &node.intra
                } else {
                    &node.best
                }),
                min_complexity: blocks as u64 * u64::from(MIN_BLOCK_COMPLEXITY),
                qindex_offset: offset as i16,
                block_log2_beta: block_received
                    .as_ref()
                    .and_then(|received| received.get(&node.frame.display_index))
                    .map_or_else(Vec::new, |received| block_log2_beta(&node.intra, received)),
            }
        })
        .collect()
}

/// The amount each block of `nodes` receives from the later nodes that
/// predict from it, walking them backwards, with `noise` added to every
/// inter cost.
fn propagate(nodes: &[Node], (cols, rows): (usize, usize), noise: u32) -> BTreeMap<u64, Vec<u64>> {
    let blocks = cols * rows;
    let mut received: BTreeMap<u64, Vec<u64>> = nodes
        .iter()
        .map(|node| (node.frame.display_index, vec![0; blocks]))
        .collect();
    let mut amounts = vec![0; blocks];
    for node in nodes.iter().rev() {
        let own = received
            .get(&node.frame.display_index)
            .cloned()
            .unwrap_or_else(|| vec![0; blocks]);
        kernels().tpl_propagate(&node.intra, &node.best, &own, noise, &mut amounts);
        for (index, sources) in node.source.iter().enumerate() {
            let count = sources.iter().flatten().count() as u64;
            for &(reference, mv) in sources.iter().flatten() {
                let Some(target) = received.get_mut(&reference) else {
                    continue;
                };
                let (col, row) = ((index % cols) as i32, (index / cols) as i32);
                spread(target, (cols, rows), (col, row), mv, amounts[index] / count);
            }
        }
    }
    received
}

/// `log2` of `(intra + received) / intra` of each block over the same ratio
/// of the frame's sums, Q8, with each intra cost at least
/// [`MIN_BLOCK_COMPLEXITY`].
fn block_log2_beta(intra: &[u32], received: &[u64]) -> Vec<i16> {
    let floored = |cost: u32| u64::from(cost.max(MIN_BLOCK_COMPLEXITY));
    let log2_ratio = |num: u64, den: u64| {
        i32::try_from(log2_q8(
            u32::try_from(num.saturating_mul(256) / den).unwrap_or(u32::MAX),
        ))
        .unwrap_or(i32::MAX)
    };
    let intra_sum: u64 = intra.iter().map(|&cost| floored(cost)).sum();
    let received_sum: u64 = received.iter().sum();
    let frame = log2_ratio(intra_sum + received_sum, intra_sum.max(1));
    intra
        .iter()
        .zip(received)
        .map(|(&cost, &received)| {
            let cost = floored(cost);
            (log2_ratio(cost + received, cost) - frame).clamp(-32_768, 32_767) as i16
        })
        .collect()
}

/// The `(qindex offset, lambda log2 Q8)` of each superblock of a
/// `(sb_cols, sb_rows)` grid of `sb_luma` samples: from the mean
/// `log2_beta` over the `(cols, rows)` grid of `block_luma` blocks it covers,
/// scaled by `strength` percent, the lambda moves by `-log2_beta` and the
/// qindex by 12 per octave, both bounded by [`MAX_TPL_QINDEX_DECREASE`].
pub(crate) fn superblock_quant(
    log2_beta: &[i16],
    (cols, rows, block_luma): (usize, usize, usize),
    (sb_cols, sb_rows, sb_luma): (usize, usize, usize),
    strength: u8,
) -> Vec<(i16, i16)> {
    let per_sb = (sb_luma / block_luma).max(1);
    let max_log2 = MAX_TPL_QINDEX_DECREASE * 256 / QINDEX_PER_IMPORTANCE_OCTAVE as i32;
    (0..sb_rows * sb_cols)
        .map(|index| {
            let (row, col) = (index / sb_cols * per_sb, index % sb_cols * per_sb);
            let (mut sum, mut count) = (0i64, 0i64);
            for r in row..(row + per_sb).min(rows) {
                for c in col..(col + per_sb).min(cols) {
                    sum += i64::from(log2_beta.get(r * cols + c).copied().unwrap_or(0));
                    count += 1;
                }
            }
            let mean = if count == 0 { 0 } else { sum / count };
            let lambda = (-mean * i64::from(strength) / 100) as i32;
            let lambda = lambda.clamp(-max_log2, max_log2);
            let per_octave = QINDEX_PER_IMPORTANCE_OCTAVE as i32;
            let offset = (lambda * per_octave + 128).div_euclid(256);
            (offset as i16, lambda as i16)
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
        analyze(&mut lookahead, &plan, None, None)
    }

    /// A block that later frames predict from is more important than the
    /// frame, one that nothing predicts from less, and the superblock offsets
    /// and lambdas follow the mean, scaled and bounded.
    #[test]
    fn superblocks_follow_the_importance_of_their_blocks() {
        let beta = block_log2_beta(
            &[1000, 1000, 1000, 1000, 40, 1000],
            &[7000, 0, 0, 1000, 0, 0],
        );
        assert!(beta[0] > 0 && beta[3] < 0 && beta[3] > beta[1], "{beta:?}");
        assert_eq!((beta[1], beta[2], beta[5]), (beta[2], beta[5], beta[4]));
        let grid = (3, 2, 16);
        let quant = superblock_quant(&beta, grid, (2, 1, 32), 100);
        let (left, right) = (quant[0], quant[1]);
        assert!(
            left.0 < right.0 && left.1 < right.1 && right.0 > 0,
            "{quant:?}"
        );
        let mean = [0, 1, 3, 4].map(|i| i32::from(beta[i])).iter().sum::<i32>() / 4;
        assert_eq!(i32::from(left.1), -mean);
        assert_eq!(i32::from(left.0), (-mean * 12 + 128).div_euclid(256));
        let half = superblock_quant(&beta, grid, (2, 1, 32), 50);
        assert_eq!(i32::from(half[0].1), -mean / 2);
        let strong = superblock_quant(&[-4096, 4096], (2, 1, 16), (2, 1, 16), 200);
        assert_eq!(strong, [(32, 682), (-32, -682)]);
        assert_eq!(superblock_quant(&beta, grid, (2, 1, 32), 0), [(0, 0); 2]);
    }

    #[test]
    fn recon_noise_grows_with_qindex_and_shrinks_with_the_block() {
        let noise = |qindex, bit_depth, block| recon_noise(qindex, bit_depth, block);
        assert!(noise(60, BitDepth::Eight, 16) < noise(200, BitDepth::Eight, 16));
        assert!(noise(200, BitDepth::Eight, 32) < noise(200, BitDepth::Eight, 16));
        let ten = noise(248, BitDepth::Ten, 16);
        assert!(
            ten.abs_diff(noise(200, BitDepth::Eight, 16)) * 20 < ten,
            "{ten}"
        );
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
