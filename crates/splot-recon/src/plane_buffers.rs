// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-plane sample buffers a worker keeps between frames.
//!
//! A decode allocates and frees a plane buffer per frame per plane, which is
//! the bulk of its dynamic memory: the resident set swings by tens of megabytes
//! across a sequence while dav2d, which reuses one buffer per plane for the
//! whole stream, stays flat. Handing a retired plane's buffer to the next frame
//! of the same geometry removes that swing.

use core::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::ReconSample;

/// Spare buffers of one sample type.
type Spares<T> = Vec<Vec<T>>;

type Store = Mutex<HashMap<TypeId, Box<dyn Any + Send>>>;

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_spares<T: ReconSample, R>(act: impl FnOnce(&mut Spares<T>) -> R) -> R {
    let mut store = store().lock().unwrap_or_else(PoisonError::into_inner);
    let spares = store
        .entry(TypeId::of::<T>())
        .or_insert_with(|| Box::new(Spares::<T>::new()));
    match spares.downcast_mut::<Spares<T>>() {
        Some(spares) => act(spares),
        None => act(&mut Spares::<T>::new()),
    }
}

/// Buffers of one sample type kept in reserve.
///
/// A frame's planes and every unit's reconstruction rectangle come from here,
/// so the reserve has to cover all of them at once or the next frame allocates
/// again.
const MAX_SPARES_PER_TYPE: usize = 2048;

/// Takes a retired buffer able to hold `len` samples, or an empty one.
pub(crate) fn take<T: ReconSample>(len: usize) -> Vec<T> {
    with_spares::<T, _>(|spares| {
        // Best fit: handing a luma buffer to a chroma plane would keep the
        // larger allocation for the smaller plane, and every buffer would
        // ratchet up to the largest size in the frame.
        let index = spares
            .iter()
            .enumerate()
            .filter(|(_, buffer)| buffer.capacity() >= len)
            .min_by_key(|(_, buffer)| buffer.capacity())
            .map(|(index, _)| index)?;
        Some(spares.swap_remove(index))
    })
    .unwrap_or_default()
}

/// Retains a retired buffer for the next frame of this geometry.
pub(crate) fn recycle<T: ReconSample>(mut buffer: Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    with_spares::<T, _>(|spares| {
        if spares.len() < MAX_SPARES_PER_TYPE {
            spares.push(buffer);
        }
    });
}
