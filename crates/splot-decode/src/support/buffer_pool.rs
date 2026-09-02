// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-scale buffers the decode reuses instead of reallocating per frame.
//!
//! Several per-frame structures hold one cell per mode-info unit or per
//! trajectory cell, so rebuilding them each frame moves megabytes through the
//! allocator and fragments its small zone. dav2d allocates the equivalent
//! arrays once for the decoder context and reuses them for the whole sequence.
//!
//! The cap is global rather than per worker on purpose: a per-worker cache
//! multiplies by the pool width, which at ten workers cost tens of megabytes.

use core::any::Any;
use std::sync::{Mutex, OnceLock, PoisonError};

/// Buffers retained in total, across every worker.
const MAX_RETAINED_BUFFERS: usize = 12;

type Retained = Mutex<Vec<Box<dyn Any + Send>>>;

fn retained() -> &'static Retained {
    static BUFFERS: OnceLock<Retained> = OnceLock::new();
    BUFFERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Takes a retired buffer that already holds `cells` items, or an empty one.
///
/// Each caller has its own element type, so a buffer is only ever reused for
/// the same kind of data and can grow at most to the stream's largest frame --
/// the size dav2d allocates up front anyway.
pub(crate) fn take<T: Send + 'static>(cells: usize) -> Vec<T> {
    let mut retained = retained().lock().unwrap_or_else(PoisonError::into_inner);
    let Some(index) = retained.iter().position(|buffer| {
        buffer
            .downcast_ref::<Vec<T>>()
            .is_some_and(|buffer| buffer.capacity() >= cells)
    }) else {
        return Vec::new();
    };
    retained
        .swap_remove(index)
        .downcast::<Vec<T>>()
        .map_or_else(|_| Vec::new(), |buffer| *buffer)
}

/// Retains a retired buffer for the next frame that wants this shape.
pub(crate) fn recycle<T: Send + 'static>(buffer: &mut Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let mut retained = retained().lock().unwrap_or_else(PoisonError::into_inner);
    if retained.len() < MAX_RETAINED_BUFFERS {
        retained.push(Box::new(core::mem::take(buffer)));
    }
}
