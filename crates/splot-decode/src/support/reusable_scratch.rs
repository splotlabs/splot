// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Re-entrancy-safe access to thread-local reusable scratch storage.

use std::cell::RefCell;
use std::thread::LocalKey;

pub(crate) type ErasedVecSlot = Option<Box<dyn std::any::Any + Send>>;

/// Retained per-thread frame- and tile-sized cell buffers, keyed by cell type.
///
/// Decoding rebuilds the same handful of area-sized grids for every tile and
/// every frame -- the frontier cursor's MI grids, the coalesced MI-size store,
/// the frame segment-id map, the temporal motion field. Each is one allocation
/// of tens to hundreds of kilobytes whose contents are overwritten anyway, so
/// they are taken from here and returned on drop.
const POOLED_VEC_SLOTS: usize = 32;

/// Buffers larger than this are let go rather than held for the process's life.
const MAX_POOLED_VEC_CELLS: usize = 1 << 24;

thread_local! {
    static POOLED_VECS: RefCell<[ErasedVecSlot; POOLED_VEC_SLOTS]> =
        const { RefCell::new([const { None }; POOLED_VEC_SLOTS]) };
}

/// Takes a spare buffer of `T` holding at least `cells` but not twice that, or
/// else the largest smaller spare, or else the smallest larger one.
///
/// A spare more than twice the request is taken last: grids of one cell type
/// come in several sizes, and a small grid that keeps a large buffer pins the
/// large one's pages for good.
pub(crate) fn take_pooled_vec<T: Send + 'static>(cells: usize) -> Vec<T> {
    POOLED_VECS
        .try_with(|slots| {
            slots
                .borrow_mut()
                .iter_mut()
                .filter_map(|slot| slot.as_mut()?.downcast_mut::<Vec<T>>())
                .filter(|spare| spare.capacity() > 0)
                .min_by_key(|spare| match spare.capacity() {
                    capacity if capacity >= cells && capacity / 2 <= cells => (0, capacity),
                    capacity if capacity < cells => (1, usize::MAX - capacity),
                    capacity => (2, capacity),
                })
                .map(core::mem::take)
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// Offers `cells` back to this thread's spare set.
pub(crate) fn recycle_pooled_vec<T: Send + 'static>(mut cells: Vec<T>) {
    if cells.capacity() == 0 || cells.capacity() > MAX_POOLED_VEC_CELLS {
        return;
    }
    cells.clear();
    let _ = POOLED_VECS.try_with(|slots| recycle_reusable_vec(slots, &mut cells));
}

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

#[cfg(test)]
mod tests {
    use super::{recycle_pooled_vec, take_pooled_vec};

    fn spare_capacity(spares: &[usize], cells: usize) -> usize {
        for &capacity in spares {
            recycle_pooled_vec(Vec::<u16>::with_capacity(capacity));
        }
        take_pooled_vec::<u16>(cells).capacity()
    }

    #[test]
    fn a_spare_more_than_twice_the_request_is_taken_last() {
        let pick = |spares: &'static [usize], cells| {
            std::thread::spawn(move || spare_capacity(spares, cells))
                .join()
                .unwrap_or(0)
        };
        assert_eq!(pick(&[1000, 100], 90), 100);
        assert_eq!(pick(&[1000, 50], 100), 50);
        assert_eq!(pick(&[1000], 100), 1000);
        assert_eq!(pick(&[1000, 100], 600), 1000);
    }
}
