// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-plane sample buffers a worker keeps between frames.
//!
//! A decode allocates and frees a plane buffer per frame per plane, which is
//! the bulk of its dynamic memory: the resident set swings by tens of megabytes
//! across a sequence while dav2d, which reuses one buffer per plane for the
//! whole stream, stays flat. Handing a retired plane's buffer to the next frame
//! of the same geometry removes that swing.

use core::any::Any;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::ReconSample;

/// Plane buffers the decode retains in total, across every worker.
///
/// The cap is global on purpose: a per-worker cap multiplies by the pool width,
/// which at ten workers retained sixteen frames each and cost 270 MB. dav2d's
/// picture pool is likewise one shared free list for the whole context.
const MAX_RETAINED_PLANE_BUFFERS: usize = 8;

type Retained = Mutex<Vec<Box<dyn Any + Send>>>;

fn retained() -> &'static Retained {
    static PLANE_BUFFERS: OnceLock<Retained> = OnceLock::new();
    PLANE_BUFFERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Takes a retired buffer that already holds `len` samples, or an empty one.
pub(crate) fn take<T: ReconSample>(len: usize) -> Vec<T> {
    let mut retained = retained().lock().unwrap_or_else(PoisonError::into_inner);
    // The capacity must match, not merely suffice: handing a luma buffer to a
    // chroma plane would keep the larger allocation for the smaller plane, and
    // every buffer would ratchet up to the largest size in the frame. dav2d's
    // pool reallocates on any size change for the same reason.
    let Some(index) = retained.iter().position(|buffer| {
        buffer
            .downcast_ref::<Vec<T>>()
            .is_some_and(|buffer| buffer.capacity() == len)
    }) else {
        return Vec::new();
    };
    retained
        .swap_remove(index)
        .downcast::<Vec<T>>()
        .map_or_else(|_| Vec::new(), |buffer| *buffer)
}

/// Retains a retired plane's buffer for the next frame of this geometry.
pub(crate) fn recycle<T: ReconSample>(mut buffer: Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let mut retained = retained().lock().unwrap_or_else(PoisonError::into_inner);
    if retained.len() < MAX_RETAINED_PLANE_BUFFERS {
        retained.push(Box::new(buffer));
    }
}
