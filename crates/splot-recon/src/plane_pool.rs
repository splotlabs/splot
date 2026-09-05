// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Process-wide spare storage for frame-sized workspace plane buffers.
//!
//! A decode holds several workspaces per frame at once -- the reconstruction
//! target, the sealed copy the deblock frontier reads, and the buffer the
//! filter stages publish into -- and each retires through a different owner. A
//! per-owner hand-off slot only covers a frame-pipelining depth of one, since
//! at depth `D` up to `D` workspaces retire between two takes. Retiring here
//! instead is depth-independent: a workspace takes storage that fits and
//! returns it on drop, which is what dav2d's picture pool does for the same
//! lifetime.
//!
//! Buffers this size are fresh pages the kernel zeroes on first touch, so the
//! saving is real work rather than allocator traffic alone.

use std::sync::Mutex;

/// Spare buffers held per sample depth.
///
/// One frame's worth of planes is three buffers, and the decoder keeps at most
/// a few frames' workspaces in flight per depth, so this bounds the retained
/// storage at a handful of frames while still covering the deepest pipeline.
const MAX_SPARE_BUFFERS: usize = 24;

/// Buffers below this are tile- or test-sized rather than frame-sized, and
/// pooling them would only crowd out the ones worth keeping.
const MIN_POOLED_SAMPLES: usize = 1 << 14;

static EIGHT_BIT_SPARES: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
static TEN_BIT_SPARES: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());

/// Takes a spare buffer holding at least `samples`, or an empty one.
fn take<T>(spares: &Mutex<Vec<Vec<T>>>, samples: usize) -> Vec<T> {
    let Ok(mut spares) = spares.lock() else {
        return Vec::new();
    };
    let Some(index) = spares.iter().position(|spare| spare.capacity() >= samples) else {
        return Vec::new();
    };
    spares.swap_remove(index)
}

/// Offers `buffer` back, keeping it only while there is room.
fn recycle<T>(spares: &Mutex<Vec<Vec<T>>>, mut buffer: Vec<T>) {
    if buffer.capacity() < MIN_POOLED_SAMPLES {
        return;
    }
    buffer.clear();
    if let Ok(mut spares) = spares.lock()
        && spares.len() < MAX_SPARE_BUFFERS
    {
        spares.push(buffer);
    }
}

/// Releases every spare buffer both pools hold.
///
/// A decode's workspaces are the only frame-sized storage that retires here, so
/// the pool is drained when a decode context is dropped rather than held for
/// the life of a process that may never decode again.
pub fn release_plane_spares() {
    if let Ok(mut spares) = EIGHT_BIT_SPARES.lock() {
        spares.clear();
        spares.shrink_to_fit();
    }
    if let Ok(mut spares) = TEN_BIT_SPARES.lock() {
        spares.clear();
        spares.shrink_to_fit();
    }
}

/// The sample-depth-specific halves of the pool.
pub trait PooledPlaneSamples: Sized {
    /// Takes a spare buffer holding at least `samples`, or an empty one.
    fn take_plane_buffer(samples: usize) -> Vec<Self>;
    /// Offers a retired plane buffer back to the pool.
    fn recycle_plane_buffer(buffer: Vec<Self>);
}

impl PooledPlaneSamples for u8 {
    fn take_plane_buffer(samples: usize) -> Vec<Self> {
        take(&EIGHT_BIT_SPARES, samples)
    }

    fn recycle_plane_buffer(buffer: Vec<Self>) {
        recycle(&EIGHT_BIT_SPARES, buffer);
    }
}

impl PooledPlaneSamples for u16 {
    fn take_plane_buffer(samples: usize) -> Vec<Self> {
        take(&TEN_BIT_SPARES, samples)
    }

    fn recycle_plane_buffer(buffer: Vec<Self>) {
        recycle(&TEN_BIT_SPARES, buffer);
    }
}
