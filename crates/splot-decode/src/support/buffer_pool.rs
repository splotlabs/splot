// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Buffers the decode keeps for the life of the process, not the frame.
//!
//! dav2d allocates a context's arrays once and reuses them for the whole
//! stream, so a steady-state frame costs it no allocation at all. splot builds
//! the equivalent structures per frame, per tile and per unit, which is the
//! bulk of its dynamic memory.
//!
//! This is that context store. One list of spare buffers per element type is
//! created the first time that type asks, and every later take and give only
//! moves a `Vec` in and out of it -- the previous type-erased store boxed each
//! buffer as it came back, so recycling cost an allocation of its own.

use core::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Spare buffers of one element type.
type Spares<T> = Vec<Vec<T>>;

type Store = Mutex<HashMap<TypeId, Box<dyn Any + Send>>>;

fn store() -> &'static Store {
    static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Runs `act` against the spare list for `T`, creating it on first use.
fn with_spares<T: Send + 'static, R>(act: impl FnOnce(&mut Spares<T>) -> R) -> R {
    let mut store = store().lock().unwrap_or_else(PoisonError::into_inner);
    let spares = store
        .entry(TypeId::of::<T>())
        .or_insert_with(|| Box::new(Spares::<T>::new()));
    match spares.downcast_mut::<Spares<T>>() {
        Some(spares) => act(spares),
        None => act(&mut Spares::<T>::new()),
    }
}

/// Buffers of one element type kept in reserve.
///
/// A frame's structures are rebuilt every frame, so the reserve has to cover
/// every one the pipeline holds at once or the next frame allocates again. The
/// bound is per element type, and a type that never grows that deep simply
/// never fills it.
const MAX_SPARES_PER_TYPE: usize = 2048;

/// Takes a spare buffer able to hold `cells` items, or an empty one.
///
/// Each element type has its own reserve, so a buffer is only ever reused for
/// the same kind of data and grows at most to the stream's largest frame.
pub(crate) fn take<T: Send + 'static>(cells: usize) -> Vec<T> {
    with_spares::<T, _>(|spares| {
        // Best fit, not first fit: handing a frame-sized buffer to a row-sized
        // request would keep the larger allocation for the smaller job, and
        // every buffer would ratchet up to the largest the stream ever needs.
        let index = spares
            .iter()
            .enumerate()
            .filter(|(_, buffer)| buffer.capacity() >= cells)
            .min_by_key(|(_, buffer)| buffer.capacity())
            .map(|(index, _)| index)?;
        Some(spares.swap_remove(index))
    })
    .unwrap_or_default()
}

/// Retains a retired buffer for the next frame that wants this shape.
pub(crate) fn recycle<T: Send + 'static>(buffer: &mut Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let buffer = core::mem::take(buffer);
    with_spares::<T, _>(|spares| {
        if spares.len() < MAX_SPARES_PER_TYPE {
            spares.push(buffer);
        }
    });
}
