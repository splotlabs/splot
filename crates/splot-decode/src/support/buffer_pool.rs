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
use parking_lot::Mutex;
use std::collections::HashMap;

/// Spare buffers of one element type.
type Spares<T> = Vec<Vec<T>>;

type Store = Mutex<HashMap<TypeId, Box<dyn Any + Send>>>;

fn store() -> &'static Store {
    static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Runs `act` against the spare list for `T`, creating it on first use.
fn with_spares<T: Send + 'static, R>(act: impl FnOnce(&mut Spares<T>) -> R) -> R {
    let mut store = store().lock();
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
const MAX_SPARES_PER_TYPE: usize = 512;

/// Takes a spare buffer able to hold `cells` items, or a fresh one.
///
/// Each element type has its own reserve, so a buffer is only ever reused for
/// the same kind of data and grows at most to the stream's largest frame.
/// When no spare fits, the whole request is reserved in one step: handing back
/// an empty list leaves the caller to grow it a doubling at a time, spending
/// an allocation on every rung.
pub(crate) fn take<T: Send + 'static>(cells: usize) -> Vec<T> {
    let Some(buffer) = take_fitting::<T>(cells) else {
        let mut fresh = Vec::new();
        fresh.try_reserve_exact(cells).ok();
        return fresh;
    };
    buffer
}

/// The smallest spare that fits, so a frame-sized buffer is never spent on a
/// row-sized request -- which would keep the larger allocation for the smaller
/// job and ratchet every buffer up to the largest the stream needs.
fn take_fitting<T: Send + 'static>(cells: usize) -> Option<Vec<T>> {
    with_spares::<T, _>(|spares| {
        let index = spares
            .iter()
            .enumerate()
            .filter(|(_, buffer)| buffer.capacity() >= cells)
            .min_by_key(|(_, buffer)| buffer.capacity())
            .map(|(index, _)| index)?;
        Some(spares.swap_remove(index))
    })
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

/// A buffer that returns itself to the store when it leaves scope.
///
/// For the short-lived lists a caller builds, reads once and drops, where
/// threading a reusable buffer through the callers would say less than it
/// costs.
pub(crate) struct Retained<T: Send + 'static>(Vec<T>);

impl<T: Send + 'static> Retained<T> {
    pub(crate) fn take() -> Self {
        Self(take(0))
    }
}

impl<T: Send + 'static> core::ops::Deref for Retained<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Send + 'static> core::ops::DerefMut for Retained<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Send + 'static> Drop for Retained<T> {
    fn drop(&mut self) {
        recycle(&mut self.0);
    }
}
