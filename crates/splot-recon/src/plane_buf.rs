// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Plane sample storage whose first sample is aligned to [`PLANE_ALIGNMENT`]
//! bytes, so hosts can wrap decoded planes for SIMD without a copy.

#![allow(
    unsafe_code,
    reason = "views the aligned byte blocks as the sample slice they hold"
)]

use core::ptr::NonNull;
use std::collections::TryReserveError;
use std::marker::PhantomData;
use std::sync::Arc;

/// The byte alignment of every plane buffer's first sample and row stride.
pub const PLANE_ALIGNMENT: usize = 64;

/// The row stride, in samples, of a plane `width` samples wide: the width
/// rounded up to a whole number of [`PLANE_ALIGNMENT`]-byte blocks, or `None`
/// when that overflows.
#[must_use]
pub const fn plane_stride<T>(width: usize) -> Option<usize> {
    width.checked_next_multiple_of(PLANE_ALIGNMENT / size_of::<T>())
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct Block([u8; PLANE_ALIGNMENT]);

/// Owned plane sample storage aligned to [`PLANE_ALIGNMENT`] bytes. It
/// derefs to the sample slice and grows like a `Vec`.
pub struct PlaneBuf<T> {
    blocks: Vec<Block>,
    len: usize,
    samples: PhantomData<T>,
}

impl<T> PlaneBuf<T> {
    /// The number of samples held.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer holds no sample.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of samples the buffer holds without reallocating.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        match size_of::<T>() {
            0 => usize::MAX,
            size => self.blocks.capacity() * (PLANE_ALIGNMENT / size),
        }
    }
}

impl<T: Copy> PlaneBuf<T> {
    const SAMPLES_PER_BLOCK: usize = {
        assert!(size_of::<T>() > 0 && PLANE_ALIGNMENT.is_multiple_of(size_of::<T>()));
        assert!(align_of::<T>() <= PLANE_ALIGNMENT);
        PLANE_ALIGNMENT / size_of::<T>()
    };

    const fn blocks_for(samples: usize) -> usize {
        samples.div_ceil(Self::SAMPLES_PER_BLOCK)
    }

    /// The held samples.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        self
    }

    /// Reserves room for `additional` more samples.
    pub fn reserve_exact(&mut self, additional: usize) {
        let wanted = Self::blocks_for(self.len.saturating_add(additional));
        self.blocks
            .reserve_exact(wanted.saturating_sub(self.blocks.len()));
    }

    /// Reserves room for `additional` more samples, reporting allocation failure.
    ///
    /// # Errors
    /// Returns the allocator's [`TryReserveError`] when the room is unavailable.
    pub fn try_reserve_exact(&mut self, additional: usize) -> Result<(), TryReserveError> {
        let wanted = Self::blocks_for(self.len.saturating_add(additional));
        self.blocks
            .try_reserve_exact(wanted.saturating_sub(self.blocks.len()))
    }

    /// Empties the buffer; capacity is kept.
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// Shortens the buffer to `len` samples; capacity is kept.
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len);
    }

    /// Drops capacity beyond the held samples.
    pub fn shrink_to_fit(&mut self) {
        self.blocks.truncate(Self::blocks_for(self.len));
        self.blocks.shrink_to_fit();
    }

    /// Resizes to `len` samples, filling new ones with `value`.
    pub fn resize(&mut self, len: usize, value: T) {
        let old = self.len;
        self.grow_to(len);
        if len > old {
            self[old..].fill(value);
        }
    }

    /// Appends a copy of `samples`.
    pub fn append_samples(&mut self, samples: &[T]) {
        let old = self.len;
        self.grow_to(old + samples.len());
        self[old..].copy_from_slice(samples); // splot-copy-ok: callers outside the decoder hand in plain Vec storage
    }

    fn grow_to(&mut self, len: usize) {
        let blocks = Self::blocks_for(len);
        if blocks > self.blocks.len() {
            self.blocks.resize(blocks, Block([0; PLANE_ALIGNMENT]));
        }
        self.len = len;
    }
}

impl<T> Default for PlaneBuf<T> {
    fn default() -> Self {
        plane_buf()
    }
}

impl<T: Copy> core::ops::Deref for PlaneBuf<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        // `len` samples fit in the initialized 64-byte-aligned blocks (`grow_to`) and
        // SAFETY: each was written as a `T`, whose size divides the block and align <= 64.
        unsafe { core::slice::from_raw_parts(self.blocks.as_ptr().cast::<T>(), self.len) }
    }
}

impl<T: Copy> core::ops::DerefMut for PlaneBuf<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as in `deref`, through the unique borrow of the blocks.
        unsafe { core::slice::from_raw_parts_mut(self.blocks.as_mut_ptr().cast::<T>(), self.len) }
    }
}

impl<T: Copy + core::fmt::Debug> core::fmt::Debug for PlaneBuf<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Copy + PartialEq> PartialEq for PlaneBuf<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Copy + Eq> Eq for PlaneBuf<T> {}

/// An empty plane buffer; it allocates nothing.
#[must_use]
pub const fn plane_buf<T>() -> PlaneBuf<T> {
    PlaneBuf {
        blocks: Vec::new(),
        len: 0,
        samples: PhantomData,
    }
}

/// A plane buffer holding a copy of `samples`.
#[must_use]
pub fn plane_buf_from<T: Copy>(samples: &[T]) -> PlaneBuf<T> {
    let mut buffer = plane_buf();
    buffer.append_samples(samples);
    buffer
}

/// Full-width plane rows that another owner's allocation holds and lends
/// exclusively while this value lives; see [`Self::lend`].
pub struct LentRows<T: 'static> {
    samples: NonNull<T>,
    len: usize,
    _lender: Arc<dyn Send + Sync>,
}

unsafe impl<T: Send + 'static> Send for LentRows<T> {} // SAFETY: the rows are reached only through `&self` / `&mut self`, as through the `&'static mut [T]` this replaced.
unsafe impl<T: Sync + 'static> Sync for LentRows<T> {} // SAFETY: a shared value gives only shared access to the rows, as a `&'static mut [T]` does.

impl<T: Copy> LentRows<T> {
    #[must_use]
    /// Lends the `len` samples from `samples` for as long as the value lives.
    ///
    /// # Safety
    /// The samples must be initialized and inside one allocation that `lender`
    /// keeps alive. Safety: nothing else reads or writes them until the value drops.
    pub unsafe fn lend(samples: NonNull<T>, len: usize, lender: Arc<dyn Send + Sync>) -> Self {
        Self {
            samples,
            len,
            _lender: lender,
        }
    }
}

impl<T: Copy> core::ops::Deref for LentRows<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.samples.as_ptr(), self.len) } // SAFETY: `lend`'s caller keeps the `len` samples live and exclusive to this value, and `&self` shares them.
    }
}

impl<T: Copy> core::ops::DerefMut for LentRows<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.samples.as_ptr(), self.len) } // SAFETY: `lend`'s caller keeps the `len` samples live and exclusive to this value, and `&mut self` is unique.
    }
}

impl<T> core::fmt::Debug for LentRows<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "LentRows({})", self.len)
    }
}

/// A plane's samples: rows another owner lends, or its own buffer, which it
/// lends to itself, so every access reads one slice.
pub(crate) struct PlaneSamples<T: 'static> {
    rows: LentRows<T>,
    owned: PlaneBuf<T>,
}

impl<T> core::fmt::Debug for PlaneSamples<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "PlaneSamples({})", self.rows.len)
    }
}

/// The lender of rows whose buffer their value holds beside them.
fn held_beside() -> Arc<dyn Send + Sync> {
    static LENDER: std::sync::OnceLock<Arc<dyn Send + Sync>> = std::sync::OnceLock::new();
    Arc::clone(LENDER.get_or_init(|| Arc::new(())))
}

impl<T: Copy + Send + Sync + 'static> PlaneSamples<T> {
    /// The samples of `buf`, which the value owns until [`Self::take`].
    pub(crate) fn owned(mut owned: PlaneBuf<T>) -> Self {
        let (len, samples) = (owned.len(), NonNull::from(&mut owned[..]).cast::<T>());
        let rows = unsafe { LentRows::lend(samples, len, held_beside()) }; // SAFETY: `samples` are the `len` initialized samples of `owned`'s heap blocks, which stay in place while `owned` moves and outlive `rows` (declared after it; `take` drops `rows` first), and only `rows` reaches them.
        Self { rows, owned }
    }

    /// Rows another owner lends.
    pub(crate) fn lent(rows: LentRows<T>) -> Self {
        Self {
            rows,
            owned: plane_buf(),
        }
    }

    /// The address of the first sample of lent rows, `None` for an own buffer.
    pub(crate) fn lent_addr(&self) -> Option<usize> {
        (self.owned.capacity() == 0 && !self.rows.is_empty()).then(|| self.rows.as_ptr().addr())
    }

    /// The own buffer, same allocation, leaving no samples; an empty buffer
    /// for lent rows.
    pub(crate) fn take(&mut self) -> PlaneBuf<T> {
        let Self { rows, owned } = core::mem::replace(self, Self::owned(plane_buf()));
        drop(rows);
        owned
    }
}

impl<T: Copy> core::ops::Deref for PlaneSamples<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        &self.rows
    }
}

impl<T: Copy> core::ops::DerefMut for PlaneSamples<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_buffers_start_on_the_plane_alignment() {
        for len in [1usize, 63, 4096, 1 << 20] {
            let mut eight: PlaneBuf<u8> = plane_buf();
            eight.resize(len, 0);
            let mut ten: PlaneBuf<u16> = plane_buf();
            ten.resize(len, 0);
            ten.resize(len * 3, 0);
            assert_eq!(eight.as_ptr() as usize % PLANE_ALIGNMENT, 0);
            assert_eq!(ten.as_ptr() as usize % PLANE_ALIGNMENT, 0);
            ten.truncate(1);
            ten.shrink_to_fit();
            assert_eq!(ten.as_ptr() as usize % PLANE_ALIGNMENT, 0);
        }
        assert_eq!(plane_buf_from(&[1u8, 2, 3]).as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn owned_plane_samples_hand_back_the_same_buffer() {
        let mut buf: PlaneBuf<u16> = plane_buf();
        buf.resize(130, 3);
        let addr = buf.as_ptr().addr();
        let mut samples = PlaneSamples::owned(buf);
        samples[0] = 7;
        samples[129] = 9;
        assert_eq!(samples.lent_addr(), None);
        let back = samples.take();
        assert_eq!(back.as_ptr().addr(), addr);
        assert_eq!((back[0], back[1], back[129]), (7, 3, 9));
        assert!(samples.is_empty() && samples.take().is_empty());
        let mut again = PlaneSamples::owned(back);
        again[1] = 5;
        let back = again.take();
        assert_eq!((back.as_ptr().addr(), &back[..2]), (addr, &[7, 5][..]));
    }

    #[test]
    fn plane_samples_drop_in_every_order() {
        let filled = || {
            let mut buf: PlaneBuf<u8> = plane_buf();
            buf.resize(64, 1);
            PlaneSamples::owned(buf)
        };
        let mut samples = filled();
        samples[63] = 2;
        drop(samples);
        for plane_first in [true, false] {
            let mut samples = filled();
            samples[63] = 2;
            let back = samples.take();
            if plane_first {
                drop(samples);
                assert_eq!(back[63], 2);
                drop(back);
            } else {
                assert_eq!(back[63], 2);
                drop(back);
                assert!(samples.is_empty());
                drop(samples);
            }
        }
        drop(PlaneSamples::<u16>::owned(plane_buf()));
        let mut empty = PlaneSamples::<u16>::owned(plane_buf());
        assert!(empty.take().is_empty() && empty.lent_addr().is_none());
    }

    #[test]
    fn plane_buffers_resize_like_vec() {
        let mut ten: PlaneBuf<u16> = plane_buf();
        ten.resize(5, 7);
        ten.truncate(2);
        ten.resize(40, 9);
        ten.append_samples(&[1, 2]);
        let mut want = vec![7u16, 7];
        want.resize(40, 9);
        want.push(1);
        want.push(2);
        assert_eq!(ten.as_slice(), want.as_slice());
        assert!(ten.capacity() >= 42);
        assert!(ten.try_reserve_exact(1000).is_ok());
        assert!(ten.capacity() >= 1042);
    }
}
