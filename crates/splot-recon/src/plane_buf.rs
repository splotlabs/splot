// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Plane sample storage whose first sample is aligned to [`PLANE_ALIGNMENT`]
//! bytes, so hosts can wrap decoded planes for SIMD without a copy.

#![allow(
    unsafe_code,
    reason = "views the aligned byte blocks as the sample slice they hold"
)]

use std::collections::TryReserveError;
use std::marker::PhantomData;

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
        assert!(size_of::<T>() > 0 && PLANE_ALIGNMENT % size_of::<T>() == 0);
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
    pub fn extend_from_slice(&mut self, samples: &[T]) {
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
        // SAFETY: `len` samples fit in the initialized, 64-byte-aligned blocks
        // (`grow_to`), every one was written as a `T` before `len` covered it,
        // and `T`'s size divides the block and its alignment is at most 64.
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
    buffer.extend_from_slice(samples);
    buffer
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
    fn plane_buffers_resize_like_vec() {
        let mut ten: PlaneBuf<u16> = plane_buf();
        ten.resize(5, 7);
        ten.truncate(2);
        ten.resize(40, 9);
        ten.extend_from_slice(&[1, 2]);
        let mut want = vec![7u16, 7];
        want.resize(40, 9);
        want.extend_from_slice(&[1, 2]);
        assert_eq!(ten.as_slice(), want.as_slice());
        assert!(ten.capacity() >= 42);
        ten.try_reserve_exact(1000)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(ten.capacity() >= 1042);
    }
}
