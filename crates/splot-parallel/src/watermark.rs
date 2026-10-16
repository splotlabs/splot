// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A monotonic progress watermark.

use std::sync::atomic::{AtomicUsize, Ordering};

/// A monotonic progress watermark readers poll. Raising it wakes every live
/// row scheduler, whose frames re-read the value when they claim rows.
///
/// A failed producer publishes [`WatermarkCell::FAILED`] so every dependent is
/// admitted and can observe the producer's failure state.
#[derive(Debug, Default)]
pub struct WatermarkCell {
    value: AtomicUsize,
}

impl WatermarkCell {
    /// The terminal value published by a failed producer.
    pub const FAILED: usize = usize::MAX;

    /// Creates a watermark at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: AtomicUsize::new(0),
        }
    }

    /// Starts a new publication cycle after all readers have released the cell.
    pub fn reset(&mut self) {
        *self.value.get_mut() = 0;
    }

    /// Returns the highest published value without blocking.
    #[must_use]
    pub fn current(&self) -> usize {
        self.value.load(Ordering::Acquire)
    }

    /// Raises the watermark and wakes the row schedulers.
    pub fn publish(&self, value: usize) -> usize {
        if self.value.fetch_max(value, Ordering::AcqRel) < value {
            crate::rows::wake_all();
        }
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_is_monotonic_and_reset_clears_a_failure() {
        let mut cell = WatermarkCell::new();
        assert_eq!(cell.publish(3), 3);
        assert_eq!(cell.publish(2), 3);
        assert_eq!(cell.publish(WatermarkCell::FAILED), WatermarkCell::FAILED);
        cell.reset();
        assert_eq!(cell.current(), 0);
    }
}
