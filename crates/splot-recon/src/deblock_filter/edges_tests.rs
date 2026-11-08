// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::tables::conversion::{Q_FIRST, Q_THRESH_MULTS, W_MULT};

use super::super::{
    DeblockFilterChoice, DeblockSampleFilter, deblock_filter_choice, deblock_sample_filter,
};
use super::*;

/// A deterministic xorshift stream.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

const POS: [usize; 5] = [1, 3, 4, 6, 8];
const NEG: [usize; 6] = [1, 2, 3, 4, 6, 8];

/// A random edge whose `maxWidthPos` is at most `widest`.
fn random_edge(noise: &mut Noise, max_sample: i32, widest: usize) -> DeblockEdge {
    if noise.below(8) == 0 {
        return DeblockEdge::NONE;
    }
    let pos = loop {
        let pos = POS[noise.below(5) as usize];
        if pos <= widest {
            break pos;
        }
    };
    let neg = loop {
        let neg = NEG[noise.below(6) as usize];
        if neg <= pos {
            break neg;
        }
    };
    let scale = (max_sample + 1) / 256;
    let q_thr = 1 + noise.below(120) as i32 * scale;
    let side = 1 + noise.below(52) as i32 * scale;
    let lossless = noise.below(6);
    DeblockEdge::new(q_thr, side, neg, pos, lossless == 0, lossless == 1).unwrap()
}

/// A plane of smooth ramps with a step every four samples and a little
/// noise, so that every § 7.17.7.2 width comes up.
fn plane(noise: &mut Noise, len: usize, max_sample: i32) -> Vec<u16> {
    let mut level = max_sample / 2;
    (0..len)
        .map(|_| {
            if noise.below(5) == 0 {
                level = (level + noise.below(17) as i32 - 8).clamp(0, max_sample);
            }
            (level + noise.below(3) as i32 - 1).clamp(0, max_sample) as u16
        })
        .collect()
}

/// The per-edge spec primitives: the § 7.17.7.2 width from lines 0 and 3,
/// then the § 7.17.7.1 filter of every line.
fn reference(
    samples: &mut [u16],
    q0: usize,
    across: usize,
    along: usize,
    edge: DeblockEdge,
    bit_depth: BitDepth,
) -> usize {
    if edge.is_none() {
        return 0;
    }
    let line = |samples: &[u16], index: usize| -> [u16; 16] {
        core::array::from_fn(|i| samples[q0 + index * along + i * across - REACH * across])
    };
    let (max_width_neg, max_width_pos) = (edge.max_width_neg(), edge.max_width_pos());
    let q_thr = i32::from(edge.q_thr());
    let width = deblock_filter_choice(
        &line(samples, 0),
        &line(samples, LINES - 1),
        &DeblockFilterChoice {
            boundary: REACH,
            q_thr,
            side_thr: i32::from(edge.side()),
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        },
    )
    .unwrap();
    if width == 0 {
        return 0;
    }
    let (neg, pos) = (width.min(max_width_neg), width.min(max_width_pos));
    let params = DeblockSampleFilter {
        boundary: REACH,
        q_thr,
        max_width_neg: neg,
        max_width_pos: pos,
        q_thresh_mult: Q_THRESH_MULTS[neg.max(pos) - 1],
        w_mult_neg: W_MULT[neg - 1],
        w_mult_pos: W_MULT[pos - 1],
        prev_lossless: edge.prev_lossless(),
        curr_lossless: edge.curr_lossless(),
        bit_depth,
    };
    for index in 0..LINES {
        let mut values = line(samples, index);
        deblock_sample_filter(&mut values, &params).unwrap();
        for (i, value) in values.into_iter().enumerate() {
            samples[q0 + index * along + i * across - REACH * across] = value;
        }
    }
    width
}

fn check_runs(bit_depth: BitDepth, seed: u64, widths: &mut [usize; REACH + 1]) {
    let max_sample = i32::from(bit_depth.max_sample());
    let mut noise = Noise(seed);
    for edges_len in [1, 2, 3, 4, 5, 7, 8, 9, 12, 16, 17, 23] {
        for vertical in [true, false] {
            let widest = POS[noise.below(5) as usize];
            let edges: Vec<_> = (0..edges_len)
                .map(|_| random_edge(&mut noise, max_sample, widest))
                .collect();
            let (stride, rows) = if vertical {
                (24 + noise.below(9) as usize, edges_len * LINES + 2)
            } else {
                (edges_len * LINES + 3 + noise.below(5) as usize, 18)
            };
            let original = plane(&mut noise, stride * rows, max_sample);
            let first = if vertical {
                stride + REACH + noise.below((stride - 2 * REACH + 1) as u64) as usize
            } else {
                REACH * stride + noise.below((stride - edges_len * LINES + 1) as u64) as usize
            };
            let mut expected = original.clone();
            for (k, &edge) in edges.iter().enumerate() {
                let (q0, across, along) = if vertical {
                    (first + k * LINES * stride, 1, stride)
                } else {
                    (first + k * LINES, stride, 1)
                };
                widths[reference(&mut expected, q0, across, along, edge, bit_depth)] += 1;
            }
            let filter = if vertical {
                deblock_vertical_edges::<u16>
            } else {
                deblock_horizontal_edges::<u16>
            };
            let mut wide = original.clone();
            filter(&mut wide, stride, first, &edges, bit_depth).unwrap();
            assert_eq!(
                wide, expected,
                "u16 {bit_depth:?} vertical={vertical} n={edges_len}"
            );
            if vertical {
                let edge_stride = 4;
                let grid: Vec<_> = (0..edges_len * edge_stride)
                    .map(|at| match at % edge_stride {
                        0 => edges[at / edge_stride],
                        3 => DeblockEdge::NONE,
                        _ => random_edge(&mut noise, max_sample, 8),
                    })
                    .collect();
                let room = (stride - REACH - first % stride) / LINES + 1;
                let live = &[true, false, true][..room.min(3)];
                let mut reference_band = original.clone();
                for k in 0..edges_len {
                    for (column, _) in live.iter().enumerate().filter(|(_, live)| **live) {
                        let q0 = first + LINES * column + k * LINES * stride;
                        let edge = grid[column + k * edge_stride];
                        reference(&mut reference_band, q0, 1, stride, edge, bit_depth);
                    }
                }
                let (runs, band_runs) = (&grid, (edges_len, edge_stride));
                let mut band = original.clone();
                deblock_vertical_edge_band(
                    &mut band, stride, first, runs, band_runs, live, bit_depth,
                )
                .unwrap();
                if bit_depth == BitDepth::Eight {
                    let mut narrow: Vec<u8> = original.iter().map(|&v| v as u8).collect();
                    deblock_vertical_edge_band(
                        &mut narrow,
                        stride,
                        first,
                        runs,
                        band_runs,
                        live,
                        bit_depth,
                    )
                    .unwrap();
                    let narrow: Vec<u16> = narrow.into_iter().map(u16::from).collect();
                    assert_eq!(narrow, reference_band, "u8 band n={edges_len}");
                }
                assert_eq!(band, reference_band, "band {bit_depth:?} n={edges_len}");
            }
            if bit_depth == BitDepth::Eight {
                let filter = if vertical {
                    deblock_vertical_edges::<u8>
                } else {
                    deblock_horizontal_edges::<u8>
                };
                let mut narrow: Vec<u8> = original.iter().map(|&v| v as u8).collect();
                filter(&mut narrow, stride, first, &edges, bit_depth).unwrap();
                let narrow: Vec<u16> = narrow.into_iter().map(u16::from).collect();
                assert_eq!(narrow, expected, "u8 vertical={vertical} n={edges_len}");
            }
            check_portable(&original, stride, first, &edges, vertical, bit_depth);
        }
    }
}

/// The portable chunk, on the 128-bit level, filters every full chunk as the
/// dispatched one does.
fn check_portable(
    original: &[u16],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
    vertical: bool,
    bit_depth: BitDepth,
) {
    let max_sample = bit_depth.max_sample() as i16;
    let Some(simd) = fearless_simd::Level::baseline().as_sse4_2() else {
        return;
    };
    for (index, chunk) in edges.as_chunks::<SEGMENTS>().0.iter().enumerate() {
        let start = if vertical {
            first + index * LANES * stride
        } else {
            first + index * LANES
        };
        let mut portable = original.to_vec(); // splot-copy-ok: per-path test copies
        let mut dispatched = original.to_vec(); // splot-copy-ok: per-path test copies
        if vertical {
            portable_chunk::<_, u16, true>(simd, &mut portable, stride, start, chunk, max_sample);
            u16::chunk::<true>(&mut dispatched, stride, start, chunk, max_sample);
        } else {
            portable_chunk::<_, u16, false>(simd, &mut portable, stride, start, chunk, max_sample);
            u16::chunk::<false>(&mut dispatched, stride, start, chunk, max_sample);
        }
        assert_eq!(portable, dispatched);
    }
}

#[test]
fn edge_runs_match_the_per_edge_spec_primitives_8bit() {
    let mut widths = [0; REACH + 1];
    for seed in 1..400 {
        check_runs(BitDepth::Eight, seed, &mut widths);
    }
    assert!(WIDTHS.iter().all(|&width| widths[width] > 20), "{widths:?}");
}

#[test]
fn edge_runs_match_the_per_edge_spec_primitives_10bit() {
    let mut widths = [0; REACH + 1];
    for seed in 1..400 {
        check_runs(BitDepth::Ten, seed, &mut widths);
    }
    assert!(WIDTHS.iter().all(|&width| widths[width] > 20), "{widths:?}");
}

#[test]
fn edge_packing_rejects_what_section_7_17_never_derives() {
    assert_eq!(
        DeblockEdge::new(0, 9, 1, 1, false, false),
        Some(DeblockEdge::NONE)
    );
    assert_eq!(
        DeblockEdge::new(9, 0, 3, 3, false, false),
        Some(DeblockEdge::NONE)
    );
    assert!(DeblockEdge::new(9, 9, 2, 2, false, false).is_none());
    assert!(DeblockEdge::new(9, 9, 4, 3, false, false).is_none());
    assert!(DeblockEdge::new(9, 9, 5, 6, false, false).is_none());
    assert!(DeblockEdge::new(MAX_Q_THR + 1, 9, 1, 1, false, false).is_none());
    assert!(DeblockEdge::new(9, MAX_SIDE + 1, 1, 1, false, false).is_none());
    assert!(DeblockEdge::new(-1, 9, 1, 1, false, false).is_none());
    let edge = DeblockEdge::new(120, 52, 6, 8, true, false).unwrap();
    assert_eq!(
        (
            edge.q_thr(),
            edge.side(),
            edge.max_width_neg(),
            edge.max_width_pos()
        ),
        (120, 52, 6, 8)
    );
    assert!(edge.prev_lossless() && !edge.curr_lossless());
}

#[test]
fn edge_runs_reject_spans_outside_their_rows_or_samples() {
    let edge = [DeblockEdge::new(9, 9, 1, 1, false, false).unwrap()];
    let mut samples = vec![0u8; 32 * 20];
    let eight = BitDepth::Eight;
    assert!(deblock_vertical_edges(&mut samples, 32, 8, &edge, eight).is_ok());
    assert!(deblock_vertical_edges(&mut samples, 32, 7, &edge, eight).is_err());
    assert!(deblock_vertical_edges(&mut samples, 32, 25, &edge, eight).is_err());
    assert!(deblock_vertical_edges(&mut samples, 15, 8, &edge, eight).is_err());
    assert!(deblock_vertical_edges(&mut samples, 32, 32 * 17 + 8, &edge, eight).is_err());
    assert!(deblock_horizontal_edges(&mut samples, 32, 8 * 32, &edge, eight).is_ok());
    assert!(deblock_horizontal_edges(&mut samples, 32, 7 * 32, &edge, eight).is_err());
    assert!(deblock_horizontal_edges(&mut samples, 32, 8 * 32 + 29, &edge, eight).is_err());
    assert!(deblock_horizontal_edges(&mut samples, 32, 13 * 32, &edge, eight).is_err());
    assert!(deblock_horizontal_edges(&mut samples, 2, 32, &edge, eight).is_err());
    let mut ten = vec![0u16; 32 * 20];
    assert!(deblock_vertical_edges(&mut samples, 32, 8, &edge, BitDepth::Ten).is_err());
    assert!(deblock_vertical_edges(&mut ten, 32, 8, &edge, BitDepth::Ten).is_ok());
}

#[test]
fn edges_from_lanes_match_their_constructor() {
    dispatch!(Level::baseline(), simd => check_from_lanes(simd));
}

fn check_from_lanes<S: Simd>(simd: S) {
    let mut noise = Noise(0x5eed);
    for _ in 0..64 {
        let edges: [DeblockEdge; 8] = core::array::from_fn(|_| random_edge(&mut noise, 1023, 8));
        let low = edges.map(|edge| edge.q_thr() as u32 | (edge.side() as u32) << 16);
        let high = edges.map(|edge| {
            edge.max_width_pos() as u32
                | (edge.max_width_neg() as u32) << 8
                | u32::from(edge.prev_lossless()) << 16
                | u32::from(edge.curr_lossless()) << 17
        });
        let lanes = DeblockEdge::from_lanes(
            simd,
            u32x8::from_slice(simd, &low),
            u32x8::from_slice(simd, &high),
        );
        assert_eq!(lanes, edges);
    }
    let over = u32x8::splat(simd, 0xffff_ffff);
    let clamped = DeblockEdge::from_lanes(simd, over, over)[0];
    assert_eq!(
        (
            clamped.q_thr(),
            clamped.side(),
            clamped.max_width_pos(),
            clamped.max_width_neg()
        ),
        (700, 4095, 8, 8)
    );
}
