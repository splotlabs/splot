// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Plane sample storage whose first sample is aligned to [`PLANE_ALIGNMENT`]
//! bytes, so hosts can wrap decoded planes for SIMD without a copy.

#![allow(
    unsafe_code,
    reason = "an Allocator impl is unsafe; it forwards to Global with a stricter alignment"
)]

use core::ptr::NonNull;
use std::alloc::{AllocError, Allocator, Global, Layout};

/// The byte alignment of every plane buffer's first sample and row stride.
pub const PLANE_ALIGNMENT: usize = 64;

/// The global allocator, with every allocation aligned to [`PLANE_ALIGNMENT`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaneAlloc;

/// Owned plane sample storage aligned to [`PLANE_ALIGNMENT`] bytes. It
/// derefs to the underlying `Vec`, so it grows, shrinks and slices like one.
#[derive(Debug, Eq, PartialEq)]
pub struct PlaneBuf<T>(Vec<T, PlaneAlloc>);

impl<T> PlaneBuf<T> {
    /// The number of samples held.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the buffer holds no sample.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> Default for PlaneBuf<T> {
    fn default() -> Self {
        plane_buf()
    }
}

impl<T> core::ops::Deref for PlaneBuf<T> {
    type Target = Vec<T, PlaneAlloc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> core::ops::DerefMut for PlaneBuf<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// An empty plane buffer; it allocates nothing.
#[must_use]
pub const fn plane_buf<T>() -> PlaneBuf<T> {
    PlaneBuf(Vec::new_in(PlaneAlloc))
}

/// A plane buffer holding a copy of `samples`.
#[must_use]
pub fn plane_buf_from<T: Copy>(samples: &[T]) -> PlaneBuf<T> {
    let mut buffer = Vec::with_capacity_in(samples.len(), PlaneAlloc);
    buffer.extend_from_slice(samples); // splot-copy-ok: callers outside the decoder hand in plain Vec storage
    PlaneBuf(buffer)
}

fn aligned(layout: Layout) -> Result<Layout, AllocError> {
    layout.align_to(PLANE_ALIGNMENT).map_err(|_| AllocError)
}

unsafe impl Allocator for PlaneAlloc {
    fn allocate(
        &self, // SAFETY: every method forwards to Global with the alignment raised alike.
        layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        Global.allocate(aligned(layout)?)
    }

    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        Global.allocate_zeroed(aligned(layout)?)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if let Ok(layout) = aligned(layout) {
            unsafe { Global.deallocate(ptr, layout) } // SAFETY: `ptr` came from this allocator with this aligned layout.
        }
    }

    unsafe fn grow(
        &self, // SAFETY: the caller upholds the Allocator::grow contract for `ptr`.
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let (old, new) = (aligned(old_layout)?, aligned(new_layout)?);
        unsafe { Global.grow(ptr, old, new) } // SAFETY: same pointer and layouts as the caller's contract, both aligned alike.
    }

    unsafe fn grow_zeroed(
        &self, // SAFETY: the caller upholds the Allocator::grow_zeroed contract for `ptr`.
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let (old, new) = (aligned(old_layout)?, aligned(new_layout)?);
        unsafe { Global.grow_zeroed(ptr, old, new) } // SAFETY: same pointer and layouts as the caller's contract, both aligned alike.
    }

    unsafe fn shrink(
        &self, // SAFETY: the caller upholds the Allocator::shrink contract for `ptr`.
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let (old, new) = (aligned(old_layout)?, aligned(new_layout)?);
        unsafe { Global.shrink(ptr, old, new) } // SAFETY: same pointer and layouts as the caller's contract, both aligned alike.
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
}
