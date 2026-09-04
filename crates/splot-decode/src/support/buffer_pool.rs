// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode buffer allocation helpers.

/// Allocates an empty buffer able to hold `cells` items.
pub(crate) fn take<T>(cells: usize) -> Vec<T> {
    let mut buffer = Vec::new();
    let _ = buffer.try_reserve_exact(cells);
    buffer
}

/// Clears a buffer before its owner drops it.
pub(crate) fn recycle<T>(buffer: &mut Vec<T>) {
    buffer.clear();
}
