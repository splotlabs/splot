// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Eight-bit unscaled § 7.13.3.18 interpolation in byte lanes: the shapes
//! each kernel takes, in one order for every target.
//!
//! Every `Subpel_Filters` tap is even, so halved taps fit `i8` and one
//! multiply weighs a pair of source bytes. Exactness of the halving:
//! `Round2(2s, 3) == Round2(s, 2)`, `Round2(2s, 11) == Round2(s, 10)` and
//! `Round2(2s, 7) == Round2(s, 6)`; a zero phase skips its pass, where
//! `Round2(Round2(x, 3) << 7, 11) == Round2(Round2(x, 3), 4)` and
//! `Round2(16x, 11) == Round2(x, 7)` keep the two-pass value. Halved sums of
//! one row lie in `-7140..=23460`, so `i16` lanes are exact.
//!
//! Eight-bit compound predictors fit `i16` ([`Sink::PrepNarrow`]): with
//! the § 7.13.3.16 `InterRound0 = 3` and compound `InterRound1 = 7` and
//! the § 7.13.3.18 filter rows (taps sum to 128, positive taps at most 184,
//! negative at least -56), the horizontal pass spans `-1785..=5865` and
//! the vertical pass `Round2(-656880, 7)..=Round2(1179120, 7)`, that is
//! `-5132..=9212`; a zero phase stores `sample << 4 <= 4080`. The sum of two
//! predictors, the equal-weight average, stays in `i16` too.
//!
//! The kernels are the [`neon`] ones on AArch64, the [`avx2`] ones on
//! x86_64 and the portable ones elsewhere; blocks no kernel takes decline
//! to the portable kernels.

use super::ReferencePlaneView;
use super::block::{self, Block, Sink, Taps, Window, is_cell};
use super::cell::{Cell, Plane, Written};
use crate::format::ReconSample;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
use generic as kernel;
#[cfg(target_arch = "x86_64")]
pub(super) mod avx2;
#[cfg(target_arch = "x86_64")]
use avx2 as kernel;
#[cfg(target_arch = "aarch64")]
pub(super) mod neon;
#[cfg(target_arch = "aarch64")]
use neon as kernel;

/// Whether the byte-lane kernels cover `block`: strips of 8 columns, the
/// last overlapping the one before it, or 4 columns in pairs of rows with at
/// most two horizontal tap pairs.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
pub(super) fn fits<T>(block: &Block<'_, T>) -> bool {
    block.w >= 8 || (block.w == 4 && block.h.is_multiple_of(2) && pairs(block.horizontal) <= 2)
}

/// The tap pairs of a pass, zero for a skipped pass.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
pub(super) fn pairs(taps: Option<&Taps>) -> usize {
    taps.map_or(0, |taps| taps.count)
}

/// Predicts an eight-bit block in byte lanes and returns `true`, or returns
/// `false` without writing for a block the kernels do not cover or a sink
/// wider than eight bits.
pub(super) fn predict<T: ReconSample>(block: &Block<'_, T>, sink: &mut Sink<'_>) -> bool {
    let admitted = match sink {
        Sink::PutWide { max, .. } => *max == u16::from(u8::MAX),
        Sink::AverageWide { .. } => false,
        _ => true,
    };
    match T::u8_slice(block.source) {
        Some(source) if admitted && fits(block) => {
            kernel::predict(&block.with_source(source), sink);
            true
        }
        _ => false,
    }
}

/// Stores the `Clip1` single-reference prediction of `block` into `output`
/// rows `stride` apart.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
pub(super) fn put(block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
    if block.horizontal.is_none() && block.vertical.is_none() && copy(block, output, stride) {
        return;
    }
    if fits(block) {
        kernel::put(block, output, stride);
        return;
    }
    block::kernels(block, &mut Sink::Put { output, stride }, None);
}

/// [`put`] of two blocks of one window, the U and V planes of a block, each
/// into its `(output, stride)`: a filtered pair in one call of the pair
/// kernel.
#[inline]
pub(super) fn put_pair(blocks: [&Block<'_, u8>; 2], outputs: [(&mut [u8], usize); 2]) {
    let [(first, first_stride), (second, second_stride)] = outputs;
    let filtered = blocks[0].horizontal.is_some() || blocks[0].vertical.is_some();
    if filtered && fits(blocks[0]) {
        kernel::put_pair(blocks, [(first, first_stride), (second, second_stride)]);
        return;
    }
    put(blocks[0], first, first_stride);
    put(blocks[1], second, second_stride);
}

/// Stores a zero-phase block (no filter pass): its prediction is its
/// samples. `false` without writing when its rows leave the storage of
/// `block` or `output`.
#[inline(never)]
fn copy(block: &Block<'_, u8>, output: &mut [u8], stride: usize) -> bool {
    let (w, h) = (block.w, block.h);
    let fits = |len: usize, stride: usize| {
        (h - 1)
            .checked_mul(stride)
            .and_then(|end| end.checked_add(w))
            .is_some_and(|end| end <= len)
    };
    let Some(source) = block.source.get(block.origin..).filter(|source| {
        stride >= w && fits(source.len(), block.stride) && fits(output.len(), stride)
    }) else {
        return false;
    };
    macro_rules! copy_lines {
        ($n:literal) => {
            for row in 0..h {
                let (from, to) = (row * block.stride, row * stride);
                if let (Some(source), Some(target)) = (
                    source.get(from..).and_then(<[u8]>::first_chunk::<$n>),
                    output.get_mut(to..).and_then(<[u8]>::first_chunk_mut::<$n>),
                ) {
                    *target = *source;
                }
            }
        };
    }
    match w {
        4 => copy_lines!(4),
        8 => copy_lines!(8),
        16 => copy_lines!(16),
        32 => copy_lines!(32),
        64 => copy_lines!(64),
        128 => copy_lines!(128),
        _ => {
            let lines = source
                .chunks(block.stride)
                .zip(output.chunks_mut(stride))
                .take(h);
            for (source, target) in lines {
                if let (Some(source), Some(target)) = (source.get(..w), target.get_mut(..w)) {
                    // splot-copy-ok: a zero-phase prediction is its reference samples
                    target.copy_from_slice(source);
                }
            }
        }
    }
    true
}

/// Predicts both blocks of a compound pair of one shape and stores their
/// COMPOUND_AVERAGE into `output`: a 4x4 or 8x8 register cell, two
/// full-sample blocks averaged directly, else the first predictor staged
/// for the second.
pub(super) fn compound(
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    match path(blocks) {
        Path::Cell => kernel::compound_cell([blocks], forward, [(output, stride)]),
        Path::Staged => staged(blocks, forward, output, stride),
        Path::Fullpel => kernel::compound_fullpel(blocks, forward, output, stride),
        Path::Strips => kernel::compound_strips([blocks], forward, [(output, stride)]),
    }
}

/// The kernel of a compound pair of one shape.
enum Path {
    /// A 4x4 or 8x8 register cell.
    Cell,
    /// A block the byte-lane kernels do not cover.
    Staged,
    /// Two full-sample blocks.
    Fullpel,
    /// The strip kernels, the first predictor staged for the second.
    Strips,
}

/// The [`Path`] of the compound pair `blocks`.
#[allow(clippy::inline_always, reason = "measured compound hot path")]
#[inline(always)]
fn path(blocks: [&Block<'_, u8>; 2]) -> Path {
    let (w, h) = (blocks[0].w, blocks[0].h);
    let cell = |block: &Block<'_, u8>| is_cell(w, h, block.horizontal, block.vertical);
    let still = |block: &Block<'_, u8>| block.horizontal.is_none() && block.vertical.is_none();
    if cell(blocks[0]) && cell(blocks[1]) {
        Path::Cell
    } else if !fits(blocks[0]) || !fits(blocks[1]) {
        Path::Staged
    } else if still(blocks[0]) && still(blocks[1]) {
        Path::Fullpel
    } else {
        Path::Strips
    }
}

/// [`compound`] of the U and V pairs of one block, which share their
/// windows, each into its `(output, stride)`: filtered pairs in one call of
/// the pair kernel.
#[inline]
pub(super) fn compound_pair(
    pairs: [[&Block<'_, u8>; 2]; 2],
    forward: i32,
    outputs: [(&mut [u8], usize); 2],
) {
    match path(pairs[0]) {
        Path::Cell => kernel::compound_cell(pairs, forward, outputs),
        Path::Strips => kernel::compound_strips(pairs, forward, outputs),
        Path::Staged | Path::Fullpel => {
            for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
                compound(blocks, forward, output, stride);
            }
        }
    }
}

/// [`compound`] through the `i16` first predictor staged for the second.
#[inline(never)]
fn staged(blocks: [&Block<'_, u8>; 2], forward: i32, output: &mut [u8], stride: usize) {
    let w = blocks[0].w;
    super::with_compound_pred0_narrow(w * blocks[0].h, |pred0| {
        let sink = &mut Sink::PrepNarrow {
            output: pred0,
            stride: w,
        };
        block::kernels(blocks[0], sink, None);
        let sink = &mut Sink::AverageNarrow {
            pred0,
            forward,
            output,
            stride,
        };
        block::kernels(blocks[1], sink, None);
    });
}

/// Stores the COMPOUND_AVERAGE of the 4x4 or 8x8 [`is_cell`] cell of
/// `windows` within `bounds`, read in place or by clamped rows and columns
/// of `views`, or returns `false` without writing when the cell kernel
/// does not cover it.
pub(super) fn compound_windows(
    views: [&ReferencePlaneView<'_, u8>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    kernel::compound_windows(views, windows, bounds, forward, output, stride)
}

/// Runs the cells of a program of `size`-square cells that the program
/// kernel covers into `planes`, and returns their bits.
pub(super) fn program(
    planes: &mut [Option<Plane<'_, '_, u8>>; 2],
    (filter, size): (usize, usize),
    cells: &[Cell],
    clip: (i32, u16),
) -> Written {
    kernel::program(planes, (filter, size), cells, clip)
}

/// The kernels of this module on targets without [`neon`], and those
/// [`avx2`] declines to: the portable kernels on a stage, each compound
/// pair through its staged first predictor.
#[cfg(not(target_arch = "aarch64"))]
#[cfg_attr(
    target_arch = "x86_64",
    allow(dead_code, reason = "the AVX2 kernels decline to part of these")
)]
mod generic {
    use super::super::ReferencePlaneView;
    use super::super::block::{self, Block, Sink, Window};
    use super::super::cell::{self, Cell, Plane, Written};

    pub(super) fn predict(block: &Block<'_, u8>, sink: &mut Sink<'_>) {
        let len = block::stage_needed(block);
        block::with_stage(len, None, |stage| {
            super::super::portable::predict(block, sink, stage);
        });
    }

    pub(super) fn put(block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
        predict(block, &mut Sink::Put { output, stride });
    }

    pub(super) fn put_pair(blocks: [&Block<'_, u8>; 2], outputs: [(&mut [u8], usize); 2]) {
        for (block, (output, stride)) in blocks.into_iter().zip(outputs) {
            put(block, output, stride);
        }
    }

    pub(super) fn compound_cell<const P: usize>(
        pairs: [[&Block<'_, u8>; 2]; P],
        forward: i32,
        outputs: [(&mut [u8], usize); P],
    ) {
        compound_strips(pairs, forward, outputs);
    }

    pub(super) fn compound_fullpel(
        blocks: [&Block<'_, u8>; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) {
        super::staged(blocks, forward, output, stride);
    }

    pub(super) fn compound_strips<const P: usize>(
        pairs: [[&Block<'_, u8>; 2]; P],
        forward: i32,
        outputs: [(&mut [u8], usize); P],
    ) {
        for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
            super::staged(blocks, forward, output, stride);
        }
    }

    pub(super) fn compound_windows(
        views: [&ReferencePlaneView<'_, u8>; 2],
        windows: &[Window; 2],
        bounds: [[i32; 4]; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        let clip = (forward, u16::from(u8::MAX));
        block::copied(views, windows, bounds, clip, output, stride)
    }

    pub(super) fn program(
        planes: &mut [Option<Plane<'_, '_, u8>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        cell::cell_by_cell(planes, shape, cells, clip, [[0; 4]; 2])
    }
}
