// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.13.3.18 blocks as unclipped source windows, the sinks their final
//! pass writes, and the dispatch to the vector kernels.
//!
//! An unscaled block reads one rectangular window of the reference. When
//! every read of that window is inside the clip bounds and the storage, the
//! kernels read the plane in place; otherwise the window is first copied
//! with each coordinate clamped, which is the `Clip3` read of the spec, so
//! one unclipped kernel serves every block, single-reference or compound,
//! of either sample depth. Scaled blocks take the general per-sample
//! convolution.

use super::{
    FILTER_BITS, INTER_ROUND0, INTER_ROUND1_COMPOUND, INTER_ROUND1_NON_COMPOUND,
    InterpolationFilter, NUM_FILTER_TYPES, NUM_PHASES, NUM_TAPS, ReferencePlaneView,
    SCALE_SUBPEL_BITS, SUBPEL_FILTERS, SUBPEL_MASK, SubpelPredictParams, plane_bounded,
    scaled_position, with_compound_pred0, with_subpel_intermediate,
};
use crate::format::ReconSample;
use crate::math::round2_i32;
use fearless_simd::{Simd, SimdBase, i8x16, prelude::*, u8x16, u16x8};

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
    /// [`Self::Prep`] of an eight-bit block, whose compound predictors
    /// fit `i16` (see [`super::eight_bit`]).
    PrepNarrow {
        output: &'a mut [i16],
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
    /// [`Self::Average`] of an eight-bit block and its `i16` first
    /// predictor.
    AverageNarrow {
        pred0: &'a [i16],
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

    /// Whether the sink takes `i16` eight-bit compound predictors.
    pub(super) const fn is_narrow(&self) -> bool {
        matches!(self, Self::PrepNarrow { .. } | Self::AverageNarrow { .. })
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
            Self::PrepNarrow { output, stride } => output.len() >= end(*stride),
            Self::AverageNarrow {
                pred0,
                output,
                stride,
                ..
            } => output.len() >= end(*stride) && pred0.len() >= w * h,
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
            Self::PrepNarrow { output, stride } => {
                if let Some(slot) = output.get_mut(row * *stride + col) {
                    *slot = value as i16;
                }
            }
            Self::AverageNarrow {
                pred0,
                forward,
                output,
                stride,
            } => {
                if let (Some(slot), Some(&first)) = (
                    output.get_mut(row * *stride + col),
                    pred0.get(row * w + col),
                ) {
                    *slot = blend(i32::from(first), value, *forward, i32::from(u8::MAX)) as u8;
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

/// One pass's active taps from tap `start`, as `count` pairs: the halved
/// taps as `i8` bytes of one little-endian word, halved and full `i16`
/// pairs.
#[derive(Clone, Copy)]
pub(super) struct Taps {
    pub(super) start: usize,
    pub(super) count: usize,
    #[cfg_attr(
        not(any(target_arch = "aarch64", target_arch = "x86_64")),
        allow(dead_code, reason = "read by the NEON and AVX2 kernels")
    )]
    pub(super) dot: u64,
    #[cfg_attr(
        not(any(target_arch = "aarch64", target_arch = "x86_64")),
        allow(dead_code, reason = "read by the NEON and AVX2 kernels")
    )]
    pub(super) words: [i32; NUM_TAPS / 2],
    pub(super) full: [i32; NUM_TAPS / 2],
}

/// The first tap and the even tap count, 2, 4, 6 or 8, of a filter row's
/// nonzero span, so a pass has a whole number of tap pairs.
pub(super) const fn even_tap_span(row: &[i32; NUM_TAPS]) -> (usize, usize) {
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
            let (mut words, mut full, mut dot) = ([0; 4], [0; 4], 0);
            let mut pair = 0;
            while pair < taps / 2 {
                let (a, b) = (row[start + 2 * pair], row[start + 2 * pair + 1]);
                let bytes = u16::from_le_bytes([(a / 2) as u8, (b / 2) as u8]);
                dot |= (bytes as u64) << (16 * pair);
                words[pair] = ((a / 2) & 0xffff) | ((b / 2) << 16);
                full[pair] = (a & 0xffff) | (b << 16);
                pair += 1;
            }
            table[filter][phase] = Some(Taps {
                start,
                count: taps / 2,
                dot,
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

/// Per filter type and phase, the [`TAPS`] of a pass with the offset of its
/// first window column from the sample position and its extra columns.
static PASSES: [[(Option<&Taps>, i32, u32); NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut table = [[(None, 0, 0); NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 0;
        while phase < NUM_PHASES {
            if let Some(taps) = &TAPS[filter][phase] {
                table[filter][phase] = (Some(taps), taps.start as i32 - 3, taps.span() as u32 - 1);
            }
            phase += 1;
        }
        filter += 1;
    }
    table
};

/// A block read in place: window row `r` starts at `source[origin + r *
/// stride]` and has [`reach`] readable samples.
pub(super) struct Block<'a, T> {
    pub(super) source: &'a [T],
    pub(super) stride: usize,
    pub(super) origin: usize,
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) horizontal: Option<&'static Taps>,
    pub(super) vertical: Option<&'static Taps>,
}

impl<'a, T> Block<'a, T> {
    /// This block read from `source`, the same storage as other samples.
    pub(super) fn with_source<U>(&self, source: &'a [U]) -> Block<'a, U> {
        Block {
            source,
            stride: self.stride,
            origin: self.origin,
            w: self.w,
            h: self.h,
            horizontal: self.horizontal,
            vertical: self.vertical,
        }
    }

    /// The source index of column 0 of window row `row`.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn start(&self, row: usize) -> usize {
        self.origin + row * self.stride
    }
}

/// Window columns every kernel may read past the window origin of a row.
pub(super) const fn reach(w: usize) -> usize {
    w.next_multiple_of(16) + 16
}

/// The intermediate storage the portable kernels stage `rows` window rows
/// of a `w`-wide block in, none without a vertical pass.
pub(super) const fn stage_len(rows: usize, w: usize, vertical: Option<&Taps>) -> usize {
    match vertical {
        Some(_) if w <= 4 => rows.div_ceil(2) * 8,
        Some(_) => rows * w.next_multiple_of(8),
        None => 0,
    }
}

/// [`stage_len`] of the window rows of `block`.
pub(super) fn stage_needed<T>(block: &Block<'_, T>) -> usize {
    let rows = block.h + block.vertical.map_or(0, |taps| taps.span() - 1);
    stage_len(rows, block.w, block.vertical)
}

/// The samples the vector kernels load: eight-bit and 16-bit storage.
pub(super) trait Sample: ReconSample + super::portable::Lanes {
    /// Runs `f` on retained storage `slot`, 0 or 1, of at least `len`
    /// samples.
    fn with_storage<R>(slot: usize, len: usize, f: impl FnOnce(&mut [Self]) -> R) -> R;
    /// The 16 bytes of the samples from `at`, `None` past them, or with
    /// `PADDED` the samples left zero-padded.
    fn bytes<S: Simd, const PADDED: bool>(simd: S, samples: &[Self], at: usize)
    -> Option<u8x16<S>>;
    /// Stores 16 bytes into the samples from `at`.
    fn put_bytes<S: Simd>(samples: &mut [Self], at: usize, bytes: u8x16<S>);
}

std::thread_local! {
    static WINDOW_U8: [std::cell::Cell<Vec<u8>>; 2] =
        const { [std::cell::Cell::new(Vec::new()), std::cell::Cell::new(Vec::new())] };
    static WINDOW_U16: [std::cell::Cell<Vec<u16>>; 2] =
        const { [std::cell::Cell::new(Vec::new()), std::cell::Cell::new(Vec::new())] };
}

macro_rules! retained {
    ($ty:ty, $slot:ident, $lanes:ident, $n:literal) => {
        impl Sample for $ty {
            fn with_storage<R>(slot: usize, len: usize, f: impl FnOnce(&mut [Self]) -> R) -> R {
                $slot.with(|slots| {
                    let slot = &slots[slot & 1];
                    let mut storage = slot.take();
                    if storage.len() < len {
                        storage.resize(len, 0);
                    }
                    let result = f(&mut storage[..len]);
                    slot.set(storage);
                    result
                })
            }

            #[allow(clippy::inline_always, reason = "border window copy")]
            #[inline(always)]
            fn bytes<S: Simd, const PADDED: bool>(
                simd: S,
                samples: &[Self],
                at: usize,
            ) -> Option<u8x16<S>> {
                let tail = samples.get(at..)?;
                if let Some(lanes) = tail.first_chunk::<$n>() {
                    return Some($lanes::load_array_ref(simd, lanes).bitcast());
                }
                let mut lanes = [0; $n];
                // splot-copy-ok: a zero-padded load of the last bytes of a narrow plane
                lanes.get_mut(..tail.len())?.copy_from_slice(tail);
                PADDED.then(|| $lanes::load_array(simd, lanes).bitcast())
            }

            #[allow(clippy::inline_always, reason = "border window copy")]
            #[inline(always)]
            fn put_bytes<S: Simd>(samples: &mut [Self], at: usize, bytes: u8x16<S>) {
                if let Some(slot) = samples
                    .get_mut(at..)
                    .and_then(<[Self]>::first_chunk_mut::<$n>)
                {
                    bytes.bitcast::<$lanes<S>>().store_array(slot);
                }
            }
        }
    };
}

retained!(u8, WINDOW_U8, u8x16, 16);
retained!(u16, WINDOW_U16, u16x8, 8);
/// The window of an unscaled `w x h` block: the taps of each pass and the
/// first source column and row it reads, with their counts.
#[derive(Clone, Copy)]
pub(super) struct Window {
    pub(super) horizontal: Option<&'static Taps>,
    pub(super) vertical: Option<&'static Taps>,
    pub(super) column: i32,
    pub(super) row: i32,
    columns: usize,
    rows: usize,
    pub(super) w: usize,
    pub(super) h: usize,
}

/// The `[stride, width, readable rows, storage length]` of a plane.
pub(super) type Geometry = [usize; 4];

impl Window {
    /// The window of a block of `interp` from 1/1024-sample `[x, y]`.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn new(interp: InterpolationFilter, w: usize, h: usize, [x, y]: [i32; 2]) -> Self {
        let filters = [interp.pass_index(w as u32), interp.pass_index(h as u32)];
        Self::of_filters(filters.map(usize::from), w, h, [x, y])
    }

    /// [`Self::new`] of the `Subpel_Filters` rows of each pass.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn of_filters(filters: [usize; 2], w: usize, h: usize, [x, y]: [i32; 2]) -> Self {
        let pass = |filter: usize, size: usize, start: i32| {
            let (taps, offset, extent) =
                PASSES[filter % NUM_FILTER_TYPES][((start >> 6) & SUBPEL_MASK) as usize];
            (
                taps,
                (start >> SCALE_SUBPEL_BITS) + offset,
                size + extent as usize,
            )
        };
        let (horizontal, column, columns) = pass(filters[0], w, x);
        let (vertical, row, rows) = pass(filters[1], h, y);
        Self {
            horizontal,
            vertical,
            column,
            row,
            columns,
            rows,
            w,
            h,
        }
    }

    /// The source index of the window origin in a plane of `geometry` when
    /// the window lies inside `[firstX, firstY, lastX, lastY]` clamped to the
    /// plane and its last row has [`reach`] samples of storage.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn origin(
        &self,
        [stride, width, rows, len]: Geometry,
        [first_x, first_y, last_x, last_y]: [i32; 4],
    ) -> Option<usize> {
        let inside = self.column >= first_x.max(0)
            && self.column + self.columns as i32 <= last_x.min(width as i32 - 1) + 1
            && self.row >= first_y.max(0)
            && self.row + self.rows as i32 <= last_y.min(rows as i32 - 1) + 1;
        if !inside {
            return None;
        }
        let origin = self.row as usize * stride + self.column as usize;
        let end = origin + self.rows.checked_sub(1)? * stride + reach(self.w);
        (end <= len).then_some(origin)
    }

    /// The block of this window read in place from `source`.
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    pub(super) fn block<'a, T>(
        &self,
        source: &'a [T],
        stride: usize,
        origin: usize,
    ) -> Block<'a, T> {
        Block {
            source,
            stride,
            origin,
            w: self.w,
            h: self.h,
            horizontal: self.horizontal,
            vertical: self.vertical,
        }
    }

    /// The block over a copy of the window in `storage`, each sample the
    /// § 7.13.3.18 `Clip3` read of `view` within `bounds` clamped to the
    /// plane, or `None` when that clip range is empty.
    fn emulate<'s, T: Sample>(
        &self,
        view: &ReferencePlaneView<'_, T>,
        bounds: [i32; 4],
        storage: &'s mut [T],
    ) -> Option<Block<'s, T>> {
        let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
        let (first_x, last_x) = (bound(bounds[0], view.width), bound(bounds[2], view.width));
        let (first_y, last_y) = (
            bound(bounds[1], view.readable_rows),
            bound(bounds[3], view.readable_rows),
        );
        if first_x > last_x || first_y > last_y {
            return None;
        }
        let clip = [first_x, first_y, last_x, last_y];
        let stride = reach(self.w);
        if view.width * size_of::<T>() < 16 {
            baseline_simd!(simd => self.emulate_chunks::<_, _, true>(simd, view, clip, stride, storage))?;
        } else {
            baseline_simd!(simd => self.emulate_chunks::<_, _, false>(simd, view, clip, stride, storage))?;
        }
        Some(self.block(storage, stride, 0))
    }

    /// The copy of [`Self::emulate`] in chunks of 16 bytes: each chunk of
    /// window columns gathers every row's clipped samples from one load of
    /// that row, zero-padded past a plane narrower than 16 bytes, through
    /// one byte index vector.
    #[allow(clippy::inline_always, reason = "border window copy")]
    #[inline(always)]
    fn emulate_chunks<S: Simd, T: Sample, const PADDED: bool>(
        &self,
        simd: S,
        view: &ReferencePlaneView<'_, T>,
        [first_x, first_y, last_x, last_y]: [i32; 4],
        stride: usize,
        storage: &mut [T],
    ) -> Option<()> {
        let size = size_of::<T>();
        let per = 16 / size;
        let lanes = u8x16::load_array(simd, core::array::from_fn(|lane| lane as u8));
        let within: i8x16<S> = (lanes >> (size as u32 - 1)).bitcast();
        let parity = lanes & u8x16::splat(simd, size as u8 - 1);
        let last = (size * view.width).saturating_sub(16);
        let plan = |chunk: usize| {
            let x = self.column + (chunk * per) as i32;
            let first = x.clamp(first_x, last_x);
            let offset = (size * first as usize).min(last);
            let shift = i8x16::splat(simd, (x - first).clamp(-16, 16) as i8);
            let high = i8x16::splat(simd, (last_x - first).clamp(0, 16) as i8);
            let column =
                simd.min_i8x16(simd.max_i8x16(within + shift, i8x16::splat(simd, 0)), high);
            let column: u8x16<S> = column.bitcast();
            let column = if size == 2 { column + column } else { column };
            let indices =
                column + u8x16::splat(simd, (size * first as usize - offset) as u8) + parity;
            (offset / size, chunk * per, indices)
        };
        let rows = [first_y, last_y];
        match self.columns.div_ceil(per) {
            1 => self.copy_rows::<S, T, 1, PADDED>(simd, view, rows, stride, storage, [plan(0)]),
            2 => {
                let plans = [plan(0), plan(1)];
                self.copy_rows::<S, T, 2, PADDED>(simd, view, rows, stride, storage, plans)
            }
            chunks => (0..chunks).try_for_each(|chunk| {
                let plans = [plan(chunk)];
                self.copy_rows::<S, T, 1, PADDED>(simd, view, rows, stride, storage, plans)
            }),
        }
    }

    /// The `C` chunks `(source column, window column, byte indices)` of
    /// [`Self::emulate_chunks`] of every window row: the rows above
    /// `first_y` and below `last_y` store the gather of that row.
    #[allow(clippy::inline_always, reason = "border window copy")]
    #[inline(always)]
    fn copy_rows<S: Simd, T: Sample, const C: usize, const PADDED: bool>(
        &self,
        simd: S,
        view: &ReferencePlaneView<'_, T>,
        [first_y, last_y]: [i32; 2],
        stride: usize,
        storage: &mut [T],
        plans: [(usize, usize, u8x16<S>); C],
    ) -> Option<()> {
        let gather = |y: i32| -> Option<[u8x16<S>; C]> {
            let row = y as usize * view.stride;
            let mut values = [u8x16::splat(simd, 0); C];
            for (value, &(offset, _, indices)) in values.iter_mut().zip(&plans) {
                let bytes = T::bytes::<S, PADDED>(simd, view.samples, row + offset)?;
                *value = simd.swizzle_dyn_u8x16(bytes, indices);
            }
            Some(values)
        };
        let put = |line: &mut [T], values: [u8x16<S>; C]| {
            for (&(_, at, _), value) in plans.iter().zip(values) {
                T::put_bytes(line, at, value);
            }
        };
        let mut lines = storage.chunks_exact_mut(stride).take(self.rows);
        let above = (first_y - self.row).clamp(0, self.rows as i32);
        let inside = (last_y + 1 - self.row).clamp(above, self.rows as i32) - above;
        let edge = gather(first_y)?;
        for line in lines.by_ref().take(above as usize) {
            put(line, edge);
        }
        for (y, line) in (self.row + above..).zip(lines.by_ref().take(inside as usize)) {
            put(line, gather(y)?);
        }
        let edge = gather(last_y)?;
        for line in lines {
            put(line, edge);
        }
        Some(())
    }
}
/// Runs `f` on the block of `window` read in place from `view`, else on a
/// copy in retained storage `slot`; `None` without running it when the clip
/// range of `bounds` is empty.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
pub(super) fn with_block<T: Sample, R>(
    window: &Window,
    view: &ReferencePlaneView<'_, T>,
    bounds: [i32; 4],
    slot: usize,
    f: impl FnOnce(&Block<'_, T>) -> R,
) -> Option<R> {
    if let Some(origin) = window.origin(view.geometry(), bounds) {
        return Some(f(&window.block(view.samples, view.stride, origin)));
    }
    emulated(*window, view, bounds, slot, f)
}

/// [`with_block`] of a window that is not read in place.
#[cold]
#[inline(never)]
fn emulated<T: Sample, R>(
    window: Window,
    view: &ReferencePlaneView<'_, T>,
    bounds: [i32; 4],
    slot: usize,
    f: impl FnOnce(&Block<'_, T>) -> R,
) -> Option<R> {
    let len = window.rows * reach(window.w);
    T::with_storage(slot, len, |storage| {
        Some(f(&window.emulate(view, bounds, storage)?))
    })
}

/// Predicts a validated block whose sink fits it.
pub(super) fn predict<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    mut sink: Sink<'_>,
    scratch: Option<&mut [i16]>,
) {
    if !sink.fits(params.w, params.h) {
        return;
    }
    let unscaled =
        params.step_x == 1 << SCALE_SUBPEL_BITS && params.step_y == 1 << SCALE_SUBPEL_BITS;
    let window = Window::new(
        params.interp,
        params.w,
        params.h,
        [params.start_x, params.start_y],
    );
    let bounds = params.bounds();
    if unscaled && let Some(view) = reference.as_u8() {
        with_block(&window, &view, bounds, 0, |block| {
            kernels(block, &mut sink, scratch);
        });
        return;
    }
    if unscaled && let Some(view) = reference.as_u16() {
        with_block(&window, &view, bounds, 0, |block| {
            kernels(block, &mut sink, scratch);
        });
        return;
    }
    let params = &plane_bounded(reference, params);
    with_subpel_intermediate(
        super::validate_subpel_params(params).unwrap_or(0) * params.w,
        |stage| {
            general(reference, params, &mut sink, stage);
        },
    );
}

/// Runs `f` on a stage of `len` values: `scratch` when it holds them, else
/// the stack or retained storage.
pub(super) fn with_stage<R>(
    len: usize,
    scratch: Option<&mut [i16]>,
    f: impl FnOnce(&mut [i16]) -> R,
) -> R {
    match scratch.and_then(|scratch| scratch.get_mut(..len)) {
        Some(stage) => f(stage),
        None if len == 0 => f(&mut []),
        None if len <= PUT_STAGE => f(&mut [0; PUT_STAGE][..len]),
        None => with_subpel_intermediate(len, f),
    }
}

/// The stage of an eight-tap put of up to 16x16 samples.
const PUT_STAGE: usize = 23 * 16 + 64;

/// Runs the kernels of `block`: the eight-bit or 16-bit kernels of its
/// shape and sink, else the portable kernels on a stage.
pub(super) fn kernels<T: Sample>(
    block: &Block<'_, T>,
    sink: &mut Sink<'_>,
    scratch: Option<&mut [i16]>,
) {
    if super::eight_bit::predict(block, sink) || super::sixteen_bit::predict(block, sink) {
        return;
    }
    with_stage(stage_needed(block), scratch, |stage| {
        super::portable::predict(block, sink, stage);
    });
}

/// Whether a `w x h` block whose passes have `horizontal` and `vertical`
/// taps is a register cell: 8x8, or 4x4 with at most two tap pairs in each
/// pass.
#[allow(clippy::inline_always, reason = "measured compound hot path")]
#[inline(always)]
pub(super) fn is_cell(
    w: usize,
    h: usize,
    horizontal: Option<&Taps>,
    vertical: Option<&Taps>,
) -> bool {
    let small = |taps: Option<&Taps>| taps.is_none_or(|taps| taps.count <= 2);
    w == h && (w == 8 || (w == 4 && small(horizontal) && small(vertical)))
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the blocks of
/// `windows` read from `references` within their `bounds` into `output`;
/// `false` without writing when a clip range is empty or the output samples
/// are neither 8- nor 16-bit.
#[allow(clippy::inline_always, reason = "measured compound hot path")]
#[inline(always)]
pub(super) fn compound<T: Sample, O: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    windows: [Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    let place = |window: &Window, view: &ReferencePlaneView<'_, T>, bounds: [i32; 4]| {
        window.origin(view.geometry(), bounds)
    };
    if let (Some(first), Some(second)) = (
        place(&windows[0], references[0], bounds[0]),
        place(&windows[1], references[1], bounds[1]),
    ) {
        let first = windows[0].block(references[0].samples, references[0].stride, first);
        let second = windows[1].block(references[1].samples, references[1].stride, second);
        return compound_blocks([&first, &second], clip, output, stride);
    }
    compound_emulated(references, windows, bounds, clip, output, stride)
}

/// [`compound`] of a pair with a window that is not read in place.
#[cold]
#[inline(never)]
fn compound_emulated<T: Sample, O: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    windows: [Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    let cell = windows.iter().all(|window| {
        window.w == windows[0].w && is_cell(window.w, window.h, window.horizontal, window.vertical)
    });
    if cell && clamped_cell(references, &windows, bounds, clip, &mut *output, stride) {
        return true;
    }
    copied(references, &windows, bounds, clip, output, stride)
}

/// [`compound_emulated`] of each window copied with its reads clamped.
pub(super) fn copied<T: Sample, O: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    with_block(&windows[0], references[0], bounds[0], 0, |first| {
        with_block(&windows[1], references[1], bounds[1], 1, |second| {
            compound_blocks([first, second], clip, output, stride)
        })
    })
    .flatten()
    .unwrap_or(false)
}

/// The register-cell kernel of a 4x4 or 8x8 compound pair of [`is_cell`]
/// windows read by clamped rows and columns, `false` without writing for a
/// pair it does not cover.
fn clamped_cell<T: Sample, O: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    if let ([Some(first), Some(second)], Some(output)) = (
        references.map(ReferencePlaneView::as_u8),
        O::u8_slice_mut(&mut *output),
    ) {
        let views = [&first, &second];
        return super::eight_bit::compound_windows(views, windows, bounds, clip.0, output, stride);
    }
    if let ([Some(first), Some(second)], Some(output)) = (
        references.map(ReferencePlaneView::as_u16),
        O::u16_slice_mut(&mut *output),
    ) {
        let views = [&first, &second];
        return super::sixteen_bit::compound_windows(views, windows, bounds, clip, output, stride);
    }
    false
}

/// [`compound`] of two blocks read in place.
#[allow(clippy::inline_always, reason = "measured compound hot path")]
#[inline(always)]
fn compound_blocks<T: Sample, O: ReconSample>(
    blocks: [&Block<'_, T>; 2],
    (forward, max): (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    let (w, h) = (blocks[0].w, blocks[0].h);
    let still = blocks
        .iter()
        .all(|block| block.horizontal.is_none() && block.vertical.is_none());
    if still && w == h && (w == 4 || w == 8) && (0..=16).contains(&forward) {
        let origins = blocks.map(|block| block.origin);
        if let (Some(first), Some(second), Some(output)) = (
            T::u8_slice(blocks[0].source),
            T::u8_slice(blocks[1].source),
            O::u8_slice_mut(&mut *output),
        ) {
            let sources = [(first, blocks[0].stride), (second, blocks[1].stride)];
            super::cell::fullpel_u8(sources, origins, w, forward, output, stride);
            return true;
        }
        if let (Some(first), Some(second), Some(output)) = (
            T::u16_slice(blocks[0].source),
            T::u16_slice(blocks[1].source),
            O::u16_slice_mut(&mut *output),
        ) {
            let sources = [(first, blocks[0].stride), (second, blocks[1].stride)];
            super::cell::fullpel_u16(sources, origins, w, (forward, max), output, stride);
            return true;
        }
    }
    if let (Some(first), Some(second), Some(output)) = (
        T::u8_slice(blocks[0].source),
        T::u8_slice(blocks[1].source),
        O::u8_slice_mut(&mut *output),
    ) {
        let blocks = [blocks[0].with_source(first), blocks[1].with_source(second)];
        super::eight_bit::compound([&blocks[0], &blocks[1]], forward, output, stride);
        return true;
    }
    if let (Some(first), Some(second), Some(output)) = (
        T::u16_slice(blocks[0].source),
        T::u16_slice(blocks[1].source),
        O::u16_slice_mut(&mut *output),
    ) && (still || w * h <= 64)
    {
        let blocks = [blocks[0].with_source(first), blocks[1].with_source(second)];
        let blocks = [&blocks[0], &blocks[1]];
        if still {
            super::portable::compound_u16(blocks, forward, max, &mut [], output, stride);
            return true;
        }
        if super::sixteen_bit::compound(blocks, (forward, max), output, stride) {
            return true;
        }
    }
    two_pass(blocks, (forward, max), output, stride)
}

/// [`compound_blocks`] through the first predictor staged for the second:
/// `false` without writing for output samples neither 8- nor 16-bit.
pub(super) fn two_pass<T: Sample, O: ReconSample>(
    blocks: [&Block<'_, T>; 2],
    (forward, max): (i32, u16),
    output: &mut [O],
    stride: usize,
) -> bool {
    let (w, h) = (blocks[0].w, blocks[0].h);
    with_compound_pred0(w * h, |pred0| {
        kernels(
            blocks[0],
            &mut Sink::Prep {
                output: pred0,
                stride: w,
            },
            None,
        );
        let pred0 = &*pred0;
        let mut sink = if let Some(output) = O::u8_slice_mut(&mut *output) {
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            }
        } else if let Some(output) = O::u16_slice_mut(output) {
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            }
        } else {
            return false;
        };
        kernels(blocks[1], &mut sink, None);
        true
    })
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
