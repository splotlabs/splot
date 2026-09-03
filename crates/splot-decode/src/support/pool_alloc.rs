// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(
    unsafe_code,
    reason = "reviewed size-class free lists behind the global allocator"
)]
#![allow(
    clippy::cast_ptr_alignment,
    reason = "every pooled block is allocated at POOL_ALIGN, which exceeds a pointer's"
)]

//! Size-class free lists in front of the system allocator.
//!
//! A steady-state dav2d frame reaches the system allocator not at all: its
//! buffers were allocated once for the context and are reused for the whole
//! stream. splot reaches it hundreds of times a frame, spread over so many
//! short-lived lists that pooling them one owner at a time stops paying long
//! before the traffic is gone.
//!
//! This is the same idea applied underneath instead of above: a block the
//! decode releases is kept on a free list for its size class and handed to the
//! next request of that class, so the steady state stops calling the system
//! allocator without every owner having to be rewritten. It changes where the
//! memory comes from, not how long a buffer lives -- the owner-side pools in
//! [`crate::support::buffer_pool`] are what keep buffers alive across frames,
//! and this only removes the traffic they cannot reach.
//!
//! Blocks above [`MAX_CLASS_BYTES`], or wanting an alignment the system
//! allocator does not already promise, go straight through.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::ptr;
use std::alloc::System;

/// Smallest pooled block, and the alignment every pooled block satisfies.
const MIN_CLASS_SHIFT: u32 = 4;
/// Largest pooled block. Above this a request goes to the system allocator,
/// where a handful of large buffers cost far less than holding them here.
const MAX_CLASS_SHIFT: u32 = 23;
/// Largest pooled block in bytes.
const MAX_CLASS_BYTES: usize = 1 << MAX_CLASS_SHIFT;
/// Number of size classes.
const CLASS_COUNT: usize = (MAX_CLASS_SHIFT - MIN_CLASS_SHIFT + 1) as usize;
/// Alignment a pooled block guarantees, matching the system allocator's.
const POOL_ALIGN: usize = 1 << MIN_CLASS_SHIFT;
/// Bytes a worker keeps cached per size class.
///
/// A byte budget rather than a block count, so the small classes -- where the
/// traffic is -- keep plenty while a class of megabyte blocks keeps one or two.
/// Bounded either way, so a thread that frees far more than it allocates cannot
/// pin memory indefinitely.
const MAX_CACHED_BYTES_PER_CLASS: usize = 1 << 22;

/// Blocks a worker keeps for one class, at least a couple however large.
const fn max_cached(class: usize) -> usize {
    let by_bytes = MAX_CACHED_BYTES_PER_CLASS / class_bytes(class);
    if by_bytes < 2 { 2 } else { by_bytes }
}

thread_local! {
    /// Head of this worker's free list for each class, threaded through the
    /// first word of every block on it.
    ///
    /// `const`-initialised cells with no destructor: a thread-local that
    /// allocated or ran teardown code would re-enter the allocator.
    static FREE_LISTS: [Cell<*mut u8>; CLASS_COUNT] =
        const { [const { Cell::new(ptr::null_mut()) }; CLASS_COUNT] };
    /// How many blocks each of those lists holds.
    static FREE_COUNTS: [Cell<usize>; CLASS_COUNT] =
        const { [const { Cell::new(0) }; CLASS_COUNT] };
}

/// The size class serving `layout`, or `None` when it must go through.
fn class_of(layout: Layout) -> Option<usize> {
    if layout.align() > POOL_ALIGN || layout.size() > MAX_CLASS_BYTES {
        return None;
    }
    let size = layout.size().max(POOL_ALIGN);
    let shift = usize::BITS - (size - 1).leading_zeros();
    let shift = shift.max(MIN_CLASS_SHIFT);
    Some((shift - MIN_CLASS_SHIFT) as usize)
}

/// The block size one class hands out.
const fn class_bytes(class: usize) -> usize {
    1 << (MIN_CLASS_SHIFT as usize + class)
}

/// The layout a pooled block of `class` was allocated with.
fn class_layout(class: usize) -> Option<Layout> {
    Layout::from_size_align(class_bytes(class), POOL_ALIGN).ok()
}

/// Blocks one class parks for any worker to claim.
///
/// A worker's own list only sees blocks it freed itself, and the decode parses
/// on one worker and reconstructs on another, so a list can starve while
/// another worker's is at its cap. This is the shared tier those spill into.
const CENTRAL_CAPACITY: usize = 512;

/// One class's shared blocks, behind a spin lock.
///
/// A spin lock rather than a mutex: a mutex allocates its platform lock the
/// first time it is taken, and that allocation would re-enter this allocator
/// while the lock is held.
struct Central {
    locked: core::sync::atomic::AtomicBool,
    blocks: core::cell::UnsafeCell<[usize; CENTRAL_CAPACITY]>,
    len: core::cell::UnsafeCell<usize>,
}

/// `locked` guards every access to `blocks` and `len`, and a parked address is
/// owned by no thread.
// SAFETY: as documented directly above.
unsafe impl Sync for Central {}

impl Central {
    const fn new() -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            blocks: core::cell::UnsafeCell::new([0; CENTRAL_CAPACITY]),
            len: core::cell::UnsafeCell::new(0),
        }
    }

    /// Runs `act` holding this class's lock.
    fn with<R>(&self, act: impl FnOnce(&mut [usize; CENTRAL_CAPACITY], &mut usize) -> R) -> R {
        use core::sync::atomic::Ordering;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held, so no other thread is inside `act`.
        let blocks = unsafe { &mut *self.blocks.get() };
        // SAFETY: as above.
        let len = unsafe { &mut *self.len.get() };
        let result = act(blocks, len);
        self.locked.store(false, Ordering::Release);
        result
    }

    /// Claims a parked block, if this class has one.
    fn pop(&self) -> *mut u8 {
        self.with(|blocks, len| {
            if *len == 0 {
                return ptr::null_mut();
            }
            *len -= 1;
            ptr::with_exposed_provenance_mut(blocks[*len])
        })
    }

    /// Parks a block for any worker, reporting whether it was taken.
    fn push(&self, block: *mut u8) -> bool {
        self.with(|blocks, len| {
            if *len == CENTRAL_CAPACITY {
                return false;
            }
            blocks[*len] = block.expose_provenance();
            *len += 1;
            true
        })
    }
}

/// Shared blocks per class.
static CENTRAL: [Central; CLASS_COUNT] = [const { Central::new() }; CLASS_COUNT];

/// Size-class free lists in front of the system allocator.
pub struct PoolAlloc;

/// Every pointer handed out is one the system allocator returned for this
/// class's layout, or one off a free list, which only ever holds blocks
/// allocated with that same layout and not currently lent out.
// SAFETY: as documented directly above.
unsafe impl GlobalAlloc for PoolAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(class) = class_of(layout) else {
            // SAFETY: the caller's contract for a non-zero layout is unchanged.
            return unsafe { System.alloc(layout) };
        };
        let pooled = FREE_LISTS
            .try_with(|lists| {
                let head = lists[class].get();
                if head.is_null() {
                    return ptr::null_mut();
                }
                // SAFETY: a listed block holds its successor in its first word.
                let next = unsafe { ptr::read(head.cast::<*mut u8>()) };
                lists[class].set(next);
                let _ = FREE_COUNTS.try_with(|counts| {
                    counts[class].set(counts[class].get().saturating_sub(1));
                });
                head
            })
            .unwrap_or(ptr::null_mut());
        if !pooled.is_null() {
            return pooled;
        }
        let shared = CENTRAL[class].pop();
        if !shared.is_null() {
            return shared;
        }
        let Some(block) = class_layout(class) else {
            // SAFETY: as above.
            return unsafe { System.alloc(layout) };
        };
        // SAFETY: `block` is a valid non-zero layout for this class.
        unsafe { System.alloc(block) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let Some(class) = class_of(layout) else {
            // SAFETY: `ptr` came from `System` for this same layout.
            unsafe { System.dealloc(ptr, layout) };
            return;
        };
        let cached = FREE_COUNTS
            .try_with(|counts| {
                if counts[class].get() >= max_cached(class) {
                    return false;
                }
                FREE_LISTS
                    .try_with(|lists| {
                        // SAFETY: the block is ours while it is not lent out.
                        unsafe { ptr::write(ptr.cast::<*mut u8>(), lists[class].get()) };
                        lists[class].set(ptr);
                        counts[class].set(counts[class].get().saturating_add(1));
                        true
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if cached || CENTRAL[class].push(ptr) {
            return;
        }
        let Some(block) = class_layout(class) else {
            // SAFETY: as above.
            unsafe { System.dealloc(ptr, layout) };
            return;
        };
        // SAFETY: a pooled block was allocated with exactly this layout.
        unsafe { System.dealloc(ptr, block) };
    }
}
