// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Re-entrancy-safe access to thread-local reusable scratch storage.

use std::cell::RefCell;
use std::thread::LocalKey;

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
