// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The rows the vector kernels of both sample sizes walk: each block's
//! window rows and output rows validated once, then handed out as whole
//! rows, so no load or store of a row checks its index.

#![allow(clippy::inline_always, reason = "measured strip-kernel row helpers")]

use super::block::Block;
use core::num::NonZeroU32;
use core::slice::{ChunksExact, ChunksExactMut};

/// The stride and the count of the first `n - 1` rows of [`rows`] in narrow
/// types, `None` for a zero `stride` or one of 2^32 or more, or `n` zero or
/// above 2^16: their product, the length of those rows, divides by the
/// stride exactly, so no row iterator divides.
#[inline(always)]
pub(super) fn span(stride: usize, n: usize) -> Option<(NonZeroU32, u16)> {
    let stride = NonZeroU32::new(u32::try_from(stride).ok()?)?;
    Some((stride, u16::try_from(n.wrapping_sub(1)).ok()?))
}

/// Rows `0..n` of a 2-D array in `data` from `at`, of the [`span`] of their
/// stride and `n`, `len` wide: the first `n - 1` as whole strides, then the
/// last, `len` long. `None` when they leave `data`.
#[inline(always)]
pub(super) fn rows<T>(
    data: &[T],
    at: usize,
    (stride, n): (NonZeroU32, u16),
    len: usize,
) -> Option<(ChunksExact<'_, T>, &[T])> {
    let (stride, span) = (
        stride.get() as usize,
        usize::from(n) * stride.get() as usize,
    );
    let (body, last) = data.get(at..)?.get(..span + len)?.split_at_checked(span)?;
    Some((body.chunks_exact(stride), last))
}

/// [`rows`] of mutable `data`, `stride` apart.
#[inline(always)]
pub(super) fn rows_mut<T>(
    data: &mut [T],
    at: usize,
    stride: usize,
    len: usize,
    n: usize,
) -> Option<(ChunksExactMut<'_, T>, &mut [T])> {
    let (stride, n) = span(stride, n)?;
    let (stride, span) = (
        stride.get() as usize,
        usize::from(n) * stride.get() as usize,
    );
    let (body, last) = data
        .get_mut(at..)?
        .get_mut(..span + len)?
        .split_at_mut_checked(span)?;
    (stride >= len).then(|| (body.chunks_exact_mut(stride), last))
}

/// Lanes `0..4` of `values` into row `a`, lanes `4..8` into row `b`.
#[inline(always)]
pub(super) fn store_pair<T: Copy, const N: usize>(a: &mut [T], b: &mut [T], lanes: [T; N]) {
    for (row, lanes) in [a, b].into_iter().zip(lanes.as_chunks::<4>().0) {
        if let Some(slot) = row.first_chunk_mut::<4>() {
            *slot = *lanes;
        }
    }
}

/// Where a kernel stores: the output rows of a strip, as `Clip1` samples
/// or as compound predictors.
pub(super) trait Out {
    /// Whether the outputs are single-reference predictions.
    const PUT: bool;
    /// Whether the outputs average a first predictor.
    const AVERAGE: bool = false;
    /// One output row of a strip.
    type Row<'r>
    where
        Self: 'r;
    /// Output rows of a strip.
    type Rows<'r>: Iterator<Item = Self::Row<'r>>
    where
        Self: 'r;
    /// The COMPOUND_AVERAGE weight of the first predictor.
    fn forward(&self) -> i32 {
        8
    }
    /// Rows `0..n` of the strip `len` wide from column `col`, as [`rows`].
    fn rows(&mut self, col: usize, len: usize, n: usize)
    -> Option<(Self::Rows<'_>, Self::Row<'_>)>;
}

/// The `Clip1` single-reference prediction into `T` storage.
pub(super) struct Put<'a, T> {
    pub(super) output: &'a mut [T],
    pub(super) stride: usize,
}

impl<T> Out for Put<'_, T> {
    const PUT: bool = true;
    type Row<'r>
        = &'r mut [T]
    where
        Self: 'r;
    type Rows<'r>
        = ChunksExactMut<'r, T>
    where
        Self: 'r;
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        rows_mut(self.output, col, self.stride, len, n)
    }
}

/// The compound predictor into `P` storage.
pub(super) struct Prep<'a, P> {
    pub(super) output: &'a mut [P],
    pub(super) stride: usize,
}

impl<P> Out for Prep<'_, P> {
    const PUT: bool = false;
    type Row<'r>
        = &'r mut [P]
    where
        Self: 'r;
    type Rows<'r>
        = ChunksExactMut<'r, P>
    where
        Self: 'r;
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        rows_mut(self.output, col, self.stride, len, n)
    }
}

/// The § 7.13.3.15 COMPOUND_AVERAGE of the contiguous `w`-wide first
/// predictor `pred0` and the block's compound predictor into `T` storage.
pub(super) struct Average<'a, P, T> {
    pub(super) pred0: &'a [P],
    pub(super) w: usize,
    pub(super) forward: i32,
    pub(super) output: &'a mut [T],
    pub(super) stride: usize,
}

impl<P, T> Out for Average<'_, P, T> {
    const PUT: bool = false;
    const AVERAGE: bool = true;
    type Row<'r>
        = (&'r [P], &'r mut [T])
    where
        Self: 'r;
    type Rows<'r>
        = core::iter::Zip<ChunksExact<'r, P>, ChunksExactMut<'r, T>>
    where
        Self: 'r;
    #[inline(always)]
    fn forward(&self) -> i32 {
        self.forward
    }
    #[inline(always)]
    fn rows(
        &mut self,
        col: usize,
        len: usize,
        n: usize,
    ) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        let (first, first_last) = rows(self.pred0, col, span(self.w, n)?, len)?;
        let (output, last) = rows_mut(self.output, col, self.stride, len, n)?;
        Some((first.zip(output), (first_last, last)))
    }
}

/// The strips of `n` columns of `block` whose vertical pass has `V` tap
/// pairs, the last overlapping the one before it: each window row of a
/// strip, `reach` long, read once by `line`, the last `2 * V - 1` kept in a
/// register ring unrolled by its period, and `vertical` of the ring, its
/// phase and the next row handed to `emit` with its output row.
#[inline(always)]
pub(super) fn strips<T, X: Copy, Y, O: Out, const V: usize>(
    block: &Block<'_, T>,
    out: &mut O,
    (n, reach): (usize, usize),
    zero: X,
    line: impl Fn(&[T]) -> Option<X>,
    vertical: impl Fn(usize, &[X; 8], X) -> Y,
    mut emit: impl FnMut(O::Row<'_>, Y),
) {
    let period = 2 * V;
    let Some((stride, rows)) = span(block.stride, block.h) else {
        return;
    };
    let stride = stride.get() as usize;
    let (head, body) = ((period.max(1) - 1) * stride, usize::from(rows) * stride);
    if stride < reach {
        return;
    }
    'strips: for col in (0..block.w).step_by(n).map(|col| col.min(block.w - n)) {
        let Some((head, source)) = block
            .source
            .get(block.origin + col..)
            .and_then(|strip| strip.get(..head + body + reach)?.split_at_checked(head))
        else {
            return;
        };
        let Some((source, last)) = source.split_at_checked(body) else {
            return;
        };
        let source = source.chunks_exact(stride);
        let Some((targets, target)) = out.rows(col, n, block.h) else {
            return;
        };
        if V == 0 {
            for (row, target) in source.zip(targets) {
                let Some(values) = line(row) else { return };
                emit(target, vertical(0, &[zero; 8], values));
            }
            if let Some(values) = line(last) {
                emit(target, vertical(0, &[zero; 8], values));
            }
            continue;
        }
        let mut head = head.chunks_exact(stride);
        let mut ring = [zero; 8];
        unroll!(k < period - 1, [0 1 2 3 4 5 6] => {
            let Some(values) = head.next().and_then(&line) else { return };
            ring[k] = values;
        });
        let (mut rows, mut final_row) = (source.zip(targets), Some((last, target)));
        loop {
            unroll!(phase < period, [0 1 2 3 4 5 6 7] => {
                let Some((row, next_target)) = rows.next().or_else(|| final_row.take()) else {
                    continue 'strips;
                };
                let Some(next) = line(row) else { return };
                emit(next_target, vertical(phase, &ring, next));
                ring[(phase + period - 1) % period] = next;
            });
        }
    }
}

/// A 4-column `block` whose vertical pass has `V` tap pairs, two rows per
/// vector: `line` of each pair of window rows, `reach` long, `stride`
/// apart within one item of a double-stride row iterator, the row past the
/// window repeating the last, the last `V` pairs kept in a register
/// ring, and `vertical` of the ring and the next pair handed to `emit` with
/// its two output rows.
#[inline(always)]
pub(super) fn pairs<T, X: Copy, Y, O: Out, const V: usize>(
    block: &Block<'_, T>,
    out: &mut O,
    reach: usize,
    zero: X,
    line: impl Fn(&[T], &[T]) -> Option<X>,
    vertical: impl Fn(&[X; 4], X) -> Y,
    mut emit: impl FnMut(O::Row<'_>, O::Row<'_>, Y),
) {
    let Some(span) = span(2 * block.stride, block.h / 2 + V) else {
        return;
    };
    let stride = block.stride;
    let tail = if V == 0 { stride + reach } else { reach };
    let Some((mut source, last)) = rows(block.source, block.origin, span, tail) else {
        return;
    };
    if stride < reach {
        return;
    }
    let last = (
        last,
        if V == 0 {
            last.get(stride..)
        } else {
            Some(last)
        },
    );
    let Some((mut targets, target)) = out.rows(0, 4, block.h) else {
        return;
    };
    let mut pair = || match source.next() {
        Some(rows) => line(rows, rows.get(stride..)?),
        None => line(last.0, last.1?),
    };
    let (mut ring, mut target) = ([zero; 4], Some(target));
    unroll!(k < V, [0 1 2 3] => {
        let Some(values) = pair() else { return };
        ring[k] = values;
    });
    while let (Some(a), Some(b)) = (targets.next(), targets.next().or_else(|| target.take())) {
        let Some(next) = pair() else { return };
        emit(a, b, vertical(&ring, next));
        ring = core::array::from_fn(|k| if k + 1 < V { ring[k + 1] } else { next });
    }
}
