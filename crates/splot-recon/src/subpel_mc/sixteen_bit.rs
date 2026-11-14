// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled § 7.13.3.18 interpolation of 16-bit storage in word lanes: the
//! shapes each kernel takes, in one order for every target.
//!
//! Each tap weighs a sample into `i32`, so every sum is exact; a horizontal
//! value fits `i16` for 8- and 10-bit samples. The kernels are the [`neon`]
//! ones on AArch64 and the portable ones elsewhere; full-sample blocks and
//! blocks no kernel takes decline to the portable kernels.

use super::ReferencePlaneView;
use super::block::{Block, Sink, Window, is_cell};
use super::cell::{Cell, Plane, Written};
use crate::format::ReconSample;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
use generic as kernel;
#[cfg(target_arch = "x86_64")]
mod avx2;
#[cfg(target_arch = "x86_64")]
use avx2 as kernel;
#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "aarch64")]
use neon as kernel;
#[cfg(test)]
mod tests;

/// Whether the word-lane kernels cover `block`: 4 columns in pairs of rows,
/// or strips of 8 columns, the last overlapping the one before it.
fn fits<T>(block: &Block<'_, T>) -> bool {
    (block.w == 4 && block.h.is_multiple_of(2)) || block.w >= 8
}

/// Predicts a filtered 16-bit block and returns `true`, or returns `false`
/// without writing for a full-sample block, a block the kernels do not
/// cover or an eight-bit compound sink.
pub(super) fn predict<T: ReconSample>(block: &Block<'_, T>, sink: &mut Sink<'_>) -> bool {
    let filtered = block.horizontal.is_some() || block.vertical.is_some();
    match T::u16_slice(block.source) {
        Some(source) if filtered && fits(block) && !sink.is_narrow() => {
            kernel::predict(&block.with_source(source), sink);
            true
        }
        _ => false,
    }
}

/// Predicts both blocks of a 4x4 or 8x8 [`is_cell`] compound pair in
/// registers and stores their COMPOUND_AVERAGE, clipped to `clip.1`, into
/// `output`, or returns `false` without writing for any other pair.
pub(super) fn compound(
    blocks: [&Block<'_, u16>; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    let (w, h) = (blocks[0].w, blocks[0].h);
    let cell = |block: &Block<'_, u16>| {
        block.w == w && block.h == h && is_cell(w, h, block.horizontal, block.vertical)
    };
    cell(blocks[0]) && cell(blocks[1]) && {
        kernel::compound_cell(blocks, clip, output, stride);
        true
    }
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the 4x4 or 8x8
/// [`is_cell`] cell of `windows` within `bounds` read from `views`, in place
/// or by clamped rows and columns, or returns `false` without writing when
/// the cell kernel does not cover it.
pub(super) fn compound_windows(
    views: [&ReferencePlaneView<'_, u16>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    kernel::compound_windows(views, windows, bounds, clip, output, stride)
}

/// Runs the cells of a program of `size`-square cells that the program
/// kernel covers into `planes`, and returns their bits.
pub(super) fn program(
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    shape: (usize, usize),
    cells: &[Cell],
    clip: (i32, u16),
) -> Written {
    kernel::program(planes, shape, cells, clip)
}

/// The kernels of this module on targets without [`neon`], and those
/// [`avx2`] declines to: the portable kernels on a stage.
#[cfg(not(target_arch = "aarch64"))]
mod generic {
    use super::super::ReferencePlaneView;
    use super::super::block::{self, Block, Sink, Window};
    use super::super::cell::{self, Cell, Plane, Written};

    pub(super) fn predict(block: &Block<'_, u16>, sink: &mut Sink<'_>) {
        let len = block::stage_needed(block);
        block::with_stage(len, None, |stage| {
            super::super::portable::predict(block, sink, stage);
        });
    }

    pub(super) fn compound_cell(
        blocks: [&Block<'_, u16>; 2],
        (forward, max): (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) {
        let len = blocks.map(block::stage_needed);
        block::with_stage(len[0].max(len[1]), None, |stage| {
            super::super::portable::compound_u16(blocks, forward, max, stage, output, stride);
        });
    }

    pub(super) fn compound_windows(
        views: [&ReferencePlaneView<'_, u16>; 2],
        windows: &[Window; 2],
        bounds: [[i32; 4]; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) -> bool {
        block::copied(views, windows, bounds, clip, output, stride)
    }

    pub(super) fn program(
        planes: &mut [Option<Plane<'_, '_, u16>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        cell::cell_by_cell(planes, shape, cells, clip, [[0; 4]; 2])
    }
}
