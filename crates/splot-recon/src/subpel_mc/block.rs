// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.3.18 blocks as unclipped source windows, the sinks their final
//! pass writes, and the dispatch to the vector kernels.
//!
//! An unscaled block reads one rectangular window of the reference. When
//! every read of that window is inside the clip bounds and the storage, the
//! kernels read the plane in place; otherwise the window is first copied
//! with each coordinate clamped, which is the `Clip3` read of the spec, so
//! one unclipped kernel serves every block. Scaled blocks take the general
//! per-sample convolution.

use super::{
    FILTER_BITS, INTER_ROUND0, INTER_ROUND1_COMPOUND, INTER_ROUND1_NON_COMPOUND, NUM_FILTER_TYPES,
    NUM_PHASES, NUM_TAPS, ReferencePlaneView, SCALE_SUBPEL_BITS, SUBPEL_FILTERS, SUBPEL_MASK,
    SubpelPredictParams, plane_bounded, scaled_position, with_subpel_intermediate,
};
use crate::format::ReconSample;
use crate::math::round2_i32;

/// Where the final pass of a block lands. Every value it stores is the
/// `Round2(.., InterRound1)` output of the convolution.
pub(super) enum Sink<'a> {
    /// `Clip1` of a single-reference prediction into `u8` storage.
    Put { output: &'a mut [u8], stride: usize },
    /// `Clip1` to `max` of a single-reference prediction into `u16` storage.
    PutWide {
        output: &'a mut [u16],
        stride: usize,
        max: u16,
    },
    /// The `InterRound1 == 7` compound predictor.
    Prep {
        output: &'a mut [i32],
        stride: usize,
    },
    /// The § 7.13.3.15 COMPOUND_AVERAGE of the contiguous `w * h` first
    /// predictor `pred0` and this block's compound predictor, into `u8`.
    Average {
        pred0: &'a [i32],
        forward: i32,
        output: &'a mut [u8],
        stride: usize,
    },
    /// [`Self::Average`] clipped to `max` into `u16` storage.
    AverageWide {
        pred0: &'a [i32],
        forward: i32,
        output: &'a mut [u16],
        stride: usize,
        max: u16,
    },
}

/// The COMPOUND_AVERAGE `Round2(f * p0 + (16 - f) * p1, 4 + InterPostRound)`.
#[allow(clippy::inline_always, reason = "measured compound hot path")]
#[inline(always)]
pub(super) fn blend(pred0: i32, pred1: i32, forward: i32, max: i32) -> i32 {
    let post_round = 2 * FILTER_BITS - (INTER_ROUND0 + INTER_ROUND1_COMPOUND);
    round2_i32(forward * pred0 + (16 - forward) * pred1, 4 + post_round).clamp(0, max)
}

impl Sink<'_> {
    /// Whether the sink takes a single-reference prediction.
    pub(super) const fn is_put(&self) -> bool {
        matches!(self, Self::Put { .. } | Self::PutWide { .. })
    }

    /// `InterRound1` of the values the sink takes.
    pub(super) const fn inter_round1(&self) -> u32 {
        if self.is_put() {
            INTER_ROUND1_NON_COMPOUND
        } else {
            INTER_ROUND1_COMPOUND
        }
    }

    /// Whether a `w x h` block fits the sink's storage.
    fn fits(&self, w: usize, h: usize) -> bool {
        let end = |stride: usize| (h - 1) * stride + w;
        match self {
            Self::Put { output, stride } => output.len() >= end(*stride),
            Self::PutWide { output, stride, .. } => output.len() >= end(*stride),
            Self::Prep { output, stride } => output.len() >= end(*stride),
            Self::Average {
                pred0,
                output,
                stride,
                ..
            } => output.len() >= end(*stride) && pred0.len() >= w * h,
            Self::AverageWide {
                pred0,
                output,
                stride,
                ..
            } => output.len() >= end(*stride) && pred0.len() >= w * h,
        }
    }

    /// Stores the value of output `(row, col)` of a `w`-wide block.
    pub(super) fn store(&mut self, w: usize, row: usize, col: usize, value: i32) {
        match self {
            Self::Put { output, stride } => {
                if let Some(slot) = output.get_mut(row * *stride + col) {
                    *slot = value.clamp(0, i32::from(u8::MAX)) as u8;
                }
            }
            Self::PutWide {
                output,
                stride,
                max,
            } => {
                if let Some(slot) = output.get_mut(row * *stride + col) {
                    *slot = value.clamp(0, i32::from(*max)) as u16;
                }
            }
            Self::Prep { output, stride } => {
                if let Some(slot) = output.get_mut(row * *stride + col) {
                    *slot = value;
                }
            }
            Self::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                if let (Some(slot), Some(&first)) = (
                    output.get_mut(row * *stride + col),
                    pred0.get(row * w + col),
                ) {
                    *slot = blend(first, value, *forward, i32::from(u8::MAX)) as u8;
                }
            }
            Self::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                if let (Some(slot), Some(&first)) = (
                    output.get_mut(row * *stride + col),
                    pred0.get(row * w + col),
                ) {
                    *slot = blend(first, value, *forward, i32::from(*max)) as u16;
                }
            }
        }
    }
}

/// One pass's active taps from tap `start`, as `count` pairs: halved byte
/// pairs for `pmaddubsw`, halved and full `i16` pairs for `pmaddwd`.
#[derive(Clone, Copy)]
pub(super) struct Taps {
    pub(super) start: usize,
    pub(super) count: usize,
    pub(super) bytes: [i16; NUM_TAPS / 2],
    pub(super) words: [i32; NUM_TAPS / 2],
    pub(super) full: [i32; NUM_TAPS / 2],
}

/// The first tap and the even tap count, 2, 4, 6 or 8, of a filter row's
/// nonzero span, so a pass has a whole number of tap pairs.
const fn even_tap_span(row: &[i32; NUM_TAPS]) -> (usize, usize) {
    let mut start = 0;
    while start < NUM_TAPS && row[start] == 0 {
        start += 1;
    }
    let mut end = NUM_TAPS;
    while end > start && row[end - 1] == 0 {
        end -= 1;
    }
    let count = match end - start {
        0..=2 => 2,
        3..=4 => 4,
        5..=6 => 6,
        _ => NUM_TAPS,
    };
    let start = if start < NUM_TAPS - count {
        start
    } else {
        NUM_TAPS - count
    };
    (start, count)
}

/// The [`Taps`] of every nonzero `Subpel_Filters` phase.
pub(super) static TAPS: [[Option<Taps>; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut table = [[None; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 1;
        while phase < NUM_PHASES {
            let row = &SUBPEL_FILTERS[filter][phase];
            let (start, taps) = even_tap_span(row);
            let (mut bytes, mut words, mut full) = ([0; 4], [0; 4], [0; 4]);
            let mut pair = 0;
            while pair < taps / 2 {
                let (a, b) = (row[start + 2 * pair], row[start + 2 * pair + 1]);
                bytes[pair] = i16::from_le_bytes([(a / 2) as u8, (b / 2) as u8]);
                words[pair] = ((a / 2) & 0xffff) | ((b / 2) << 16);
                full[pair] = (a & 0xffff) | (b << 16);
                pair += 1;
            }
            table[filter][phase] = Some(Taps {
                start,
                count: taps / 2,
                bytes,
                words,
                full,
            });
            phase += 1;
        }
        filter += 1;
    }
    table
};

impl Taps {
    pub(super) const fn span(&self) -> usize {
        2 * self.count
    }
}

/// A block with unclipped columns: column `c` of window row `r` is
/// `source[start(r) + c]`, readable for `c < reach(w)`. Window row `r` reads
/// source row `top + r` clamped to `rows`, the § 7.13.3.18 row `Clip3`.
pub(super) struct Block<'a, T> {
    pub(super) source: &'a [T],
    pub(super) stride: usize,
    pub(super) origin: usize,
    pub(super) top: isize,
    pub(super) rows: [isize; 2],
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) horizontal: Option<&'static Taps>,
    pub(super) vertical: Option<&'static Taps>,
    /// For a column-clamped window of at most 16 columns read from `origin`:
    /// the source sample of each window column, the `Clip3` read.
    pub(super) remap: Option<[u8; 16]>,
}

impl<'a, T> Block<'a, T> {
    /// This block read from `source`, the same storage as other samples.
    pub(super) fn with_source<U>(&self, source: &'a [U]) -> Block<'a, U> {
        Block {
            source,
            stride: self.stride,
            origin: self.origin,
            top: self.top,
            rows: self.rows,
            w: self.w,
            h: self.h,
            horizontal: self.horizontal,
            vertical: self.vertical,
            remap: self.remap,
        }
    }

    /// The source index of column 0 of window row `row`.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn start(&self, row: usize) -> usize {
        self.origin
            + (self.top + row as isize).clamp(self.rows[0], self.rows[1]) as usize * self.stride
    }
}

/// Window columns every kernel may read past the window origin of a row.
pub(super) const fn reach(w: usize) -> usize {
    w.next_multiple_of(16) + 16
}

/// The intermediate storage the kernels stage `rows` window rows of a
/// `w`-wide block in, none without a vertical pass.
pub(super) const fn stage_len(rows: usize, w: usize, vertical: Option<&Taps>) -> usize {
    match vertical {
        Some(_) => {
            let groups = rows.next_multiple_of(4) * 16;
            let strips = rows * w.div_ceil(8) * 8 + 64;
            if groups > strips { groups } else { strips }
        }
        None => 0,
    }
}

/// The samples the vector kernels load: eight-bit and 16-bit storage.
pub(super) trait Sample: ReconSample + super::portable::Lanes {
    /// Runs `f` on retained storage of at least `len` samples.
    fn with_storage<R>(len: usize, f: impl FnOnce(&mut [Self]) -> R) -> R;
}

std::thread_local! {
    static WINDOW_U8: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
    static WINDOW_U16: std::cell::Cell<Vec<u16>> = const { std::cell::Cell::new(Vec::new()) };
}

macro_rules! retained {
    ($ty:ty, $slot:ident) => {
        impl Sample for $ty {
            fn with_storage<R>(len: usize, f: impl FnOnce(&mut [Self]) -> R) -> R {
                $slot.with(|slot| {
                    let mut storage = slot.take();
                    if storage.len() < len {
                        storage.resize(len, 0);
                    }
                    let result = f(&mut storage[..len]);
                    slot.set(storage);
                    result
                })
            }
        }
    };
}

retained!(u8, WINDOW_U8);
retained!(u16, WINDOW_U16);

/// Predicts a validated block whose sink fits it.
pub(super) fn predict<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    mut sink: Sink<'_>,
    scratch: Option<&mut [i16]>,
) {
    let params = &plane_bounded(reference, params);
    if !sink.fits(params.w, params.h) {
        return;
    }
    let unscaled =
        params.step_x == 1 << SCALE_SUBPEL_BITS && params.step_y == 1 << SCALE_SUBPEL_BITS;
    if unscaled && let Some(view) = reference.as_u8() {
        return unscaled_block(&view, params, sink, scratch);
    }
    if unscaled && let Some(view) = reference.as_u16() {
        return unscaled_block(&view, params, sink, scratch);
    }
    with_subpel_intermediate(
        super::validate_subpel_params(params).unwrap_or(0) * params.w,
        |stage| {
            general(reference, params, &mut sink, stage);
        },
    );
}

/// The window of an unscaled block: its taps and the first source column
/// and row each pass reads, with their counts.
struct Layout {
    horizontal: Option<&'static Taps>,
    vertical: Option<&'static Taps>,
    column: i32,
    columns: usize,
    row: i32,
    rows: usize,
    last_row: usize,
}

impl Layout {
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn new(params: &SubpelPredictParams) -> Self {
        let taps = |size: usize, start: i32| {
            TAPS[params.interp.pass_index(size as u32) as usize]
                [((start >> 6) & SUBPEL_MASK) as usize]
                .as_ref()
        };
        let (horizontal, vertical) = (
            taps(params.w, params.start_x),
            taps(params.h, params.start_y),
        );
        let window = |start: i32, size: usize, taps: Option<&Taps>| {
            let base = start >> SCALE_SUBPEL_BITS;
            match taps {
                Some(taps) => (base - 3 + taps.start as i32, size + taps.span() - 1),
                None => (base, size),
            }
        };
        let (column, columns) = window(params.start_x, params.w, horizontal);
        let (row, rows) = window(params.start_y, params.h, vertical);
        Self {
            horizontal,
            vertical,
            column,
            columns,
            row,
            rows,
            last_row: (row + rows as i32 - 1)
                .min(params.last_y)
                .max(params.first_y) as usize,
        }
    }

    /// The block read from the plane in place: directly when every column
    /// is inside the clip bounds, else for a horizontally filtered block of
    /// at most 8 columns through a column remap. `None` when the window must be
    /// emulated.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn in_place<'a, T: ReconSample>(
        &self,
        view: &ReferencePlaneView<'a, T>,
        params: &SubpelPredictParams,
    ) -> Option<Block<'a, T>> {
        let block = |origin: usize, remap: Option<[u8; 16]>| Block {
            source: view.samples,
            stride: view.stride,
            origin,
            top: self.row as isize,
            rows: [params.first_y as isize, params.last_y as isize],
            w: params.w,
            h: params.h,
            horizontal: self.horizontal,
            vertical: self.vertical,
            remap,
        };
        let fits = |column: i32, reach: usize| {
            column >= 0
                && self.last_row * view.stride + column as usize + reach <= view.samples.len()
        };
        if self.column >= params.first_x
            && self.column + self.columns as i32 - 1 <= params.last_x
            && fits(self.column, reach(params.w))
        {
            return Some(block(self.column as usize, None));
        }
        let base = self.column.clamp(params.first_x, params.last_x);
        (params.w <= 8 && self.horizontal.is_some() && fits(base, 16)).then(|| {
            block(
                base as usize,
                Some(core::array::from_fn(|lane| {
                    ((self.column + lane as i32).clamp(params.first_x, params.last_x) - base) as u8
                })),
            )
        })
    }
}

/// Predicts both references of a compound cell of at most 64 samples and
/// 16 columns and stores their COMPOUND_AVERAGE clipped to `max`, or returns
/// `false` without writing when a window must be emulated or the cell is
/// outside the vector subset.
pub(super) fn small_compound<'a, T: ReconSample, O: ReconSample>(
    references: [&ReferencePlaneView<'a, T>; 2],
    params: [&SubpelPredictParams; 2],
    forward: i32,
    scratch: &mut [i16],
    output: &mut [O],
    stride: usize,
) -> bool {
    let (w, h) = (params[0].w, params[0].h);
    if w > 16 || w * h > 64 || output.len() < (h - 1) * stride + w {
        return false;
    }
    let block = |reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams| {
        let params = plane_bounded(reference, params);
        let layout = Layout::new(&params);
        let len = stage_len(layout.rows, params.w, layout.vertical);
        Some((layout.in_place(reference, &params)?, len))
    };
    let (Some((first, first_len)), Some((second, second_len))) = (
        block(references[0], params[0]),
        block(references[1], params[1]),
    ) else {
        return false;
    };
    let mut cell = [0; CELL_STAGE];
    let stage = match scratch.get_mut(..first_len.max(second_len)) {
        Some(stage) => stage,
        None => &mut cell,
    };
    let as_u8 = |block: &Block<'a, T>| Some(block.with_source(T::u8_slice(block.source)?));
    let as_u16 = |block: &Block<'a, T>| Some(block.with_source(T::u16_slice(block.source)?));
    if let (Some(first), Some(second), Some(output)) =
        (as_u8(&first), as_u8(&second), O::u8_slice_mut(output))
    {
        return super::eight_bit::compound([&first, &second], forward, stage, output, stride);
    }
    if let (Some(first), Some(second), Some(output)) =
        (as_u16(&first), as_u16(&second), O::u16_slice_mut(output))
    {
        let max = params[0].bit_depth.max_sample();
        return super::sixteen_bit::compound(
            [&first, &second],
            forward,
            max,
            stage,
            output,
            stride,
        );
    }
    false
}

/// The stage of the tallest compound cell, 4x16.
const CELL_STAGE: usize = 24 * 16;

fn unscaled_block<T: Sample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    mut sink: Sink<'_>,
    scratch: Option<&mut [i16]>,
) {
    let (w, h) = (params.w, params.h);
    let layout = Layout::new(params);
    let Layout {
        horizontal,
        vertical,
        column,
        columns,
        row,
        rows,
        ..
    } = layout;
    let stage_len = stage_len(rows, w, vertical);
    let mut run = |block: &Block<'_, T>, stage: &mut [i16]| {
        if super::eight_bit::predict(block, &mut sink, stage)
            || super::sixteen_bit::predict(block, &mut sink, stage)
        {
            return true;
        }
        if block.remap.is_some() {
            return false;
        }
        super::portable::predict(block, &mut sink, stage);
        true
    };
    let mut scratch = scratch;
    let mut run_staged = |block: &Block<'_, T>| match scratch.as_deref_mut() {
        Some(scratch) if scratch.len() >= stage_len => run(block, &mut scratch[..stage_len]),
        _ if stage_len == 0 => run(block, &mut []),
        _ => with_subpel_intermediate(stage_len, |stage| run(block, stage)),
    };
    if let Some(block) = layout.in_place(view, params)
        && run_staged(&block)
    {
        return;
    }
    let stride = reach(w);
    let inside = (params.first_x - column).clamp(0, columns as i32) as usize
        ..(params.last_x - column + 1).clamp(0, columns as i32) as usize;
    T::with_storage(rows * stride, |storage| {
        for (r, line) in storage.chunks_exact_mut(stride).enumerate() {
            let y = (row + r as i32).clamp(params.first_y, params.last_y) as usize;
            let source = view.row(y);
            let edge = |x: i32| source[(x as usize).min(source.len() - 1)];
            let (left, right) = (edge(params.first_x), edge(params.last_x));
            let (start, end) = (inside.start.min(inside.end), inside.end);
            let from = y * view.stride + (column + start as i32).max(0) as usize;
            for at in (start..end).step_by(16) {
                let span = view
                    .samples
                    .get(from + at - start..)
                    .and_then(<[T]>::first_chunk::<16>);
                match (
                    line.get_mut(at..).and_then(<[T]>::first_chunk_mut::<16>),
                    span,
                ) {
                    (Some(slot), Some(span)) => *slot = *span,
                    _ => {
                        for (offset, slot) in line[at..end.min(at + 16)].iter_mut().enumerate() {
                            *slot = source[(column + (at + offset) as i32) as usize];
                        }
                    }
                }
            }
            line[..start].fill(left);
            line[end..columns].fill(right);
        }
        run_staged(&Block {
            source: storage,
            stride,
            origin: 0,
            top: 0,
            rows: [0, rows as isize - 1],
            w,
            h,
            horizontal,
            vertical,
            remap: None,
        });
    });
}

/// The § 7.13.3.18 two-pass convolution sample by sample, for scaled
/// blocks: `intermediate` holds `intermediateHeight * w` values.
fn general<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    sink: &mut Sink<'_>,
    intermediate: &mut [i16],
) {
    let SubpelPredictParams {
        interp,
        w,
        h,
        start_x,
        start_y,
        step_x,
        step_y,
        first_x,
        first_y,
        last_x,
        last_y,
        bit_depth: _,
    } = *params;
    let h_filter = &SUBPEL_FILTERS[interp.pass_index(w as u32) as usize];
    let v_filter = &SUBPEL_FILTERS[interp.pass_index(h as u32) as usize];
    let rows = intermediate.len() / w.max(1);
    for (r, row_out) in intermediate.chunks_exact_mut(w).enumerate().take(rows) {
        let ref_row = ((start_y >> SCALE_SUBPEL_BITS) + r as i32 - 3).clamp(first_y, last_y);
        for (c, out) in row_out.iter_mut().enumerate() {
            let p = scaled_position(start_x, step_x, c);
            let taps = &h_filter[((p >> 6) & SUBPEL_MASK) as usize];
            let mut sum = 0;
            for (t, &tap) in taps.iter().enumerate() {
                let ref_col = ((p >> SCALE_SUBPEL_BITS) + t as i32 - 3).clamp(first_x, last_x);
                sum += tap * reference.sample(ref_row as usize, ref_col as usize);
            }
            *out = round2_i32(sum, INTER_ROUND0) as i16;
        }
    }
    let inter_round1 = sink.inter_round1();
    for r in 0..h {
        let p = scaled_position(start_y & 1023, step_y, r);
        let taps = &v_filter[((p >> 6) & SUBPEL_MASK) as usize];
        let base = (p >> SCALE_SUBPEL_BITS) as usize;
        for c in 0..w {
            let mut sum = 0;
            for (t, &tap) in taps.iter().enumerate() {
                let value = intermediate.get((base + t) * w + c).copied().unwrap_or(0);
                sum += tap * i32::from(value);
            }
            sink.store(w, r, c, round2_i32(sum, inter_round1));
        }
    }
}
