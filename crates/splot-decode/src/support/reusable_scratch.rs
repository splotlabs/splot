// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Re-entrancy-safe access to thread-local reusable scratch storage.

use std::cell::RefCell;
use std::thread::LocalKey;

pub(crate) type ErasedVecSlot = Option<Box<dyn std::any::Any + Send>>;

pub(crate) fn take_reusable_vec<T: Send + 'static, const N: usize>(
    cell: &RefCell<[ErasedVecSlot; N]>,
) -> Vec<T> {
    let mut samples = Vec::new();
    if let Some(cached) = cell.borrow_mut().iter_mut().flatten().find_map(|any| {
        any.downcast_mut::<Vec<T>>()
            .filter(|samples| samples.capacity() > 0)
    }) {
        std::mem::swap(&mut samples, cached);
    }
    samples
}

pub(crate) fn recycle_reusable_vec<T: Send + 'static, const N: usize>(
    cell: &RefCell<[ErasedVecSlot; N]>,
    samples: &mut Vec<T>,
) {
    let mut slots = cell.borrow_mut();
    if let Some(cached) = slots.iter_mut().flatten().find_map(|any| {
        any.downcast_mut::<Vec<T>>()
            .filter(|samples| samples.capacity() == 0)
    }) {
        std::mem::swap(samples, cached);
    } else if let Some(slot) = slots.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(Box::new(std::mem::take(samples)));
    }
}

pub(crate) fn with_reusable_scratch<T: Default, R>(
    scratch: &'static LocalKey<RefCell<T>>,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    scratch.with(|slot| {
        let Ok(mut value) = slot.try_borrow_mut() else {
            let mut fallback = T::default();
            return f(&mut fallback);
        };
        f(&mut value)
    })
}
