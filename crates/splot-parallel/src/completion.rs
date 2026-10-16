// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A one-shot completion slot for pipeline hand-off.

use parking_lot::{Condvar, Mutex};
use std::sync::OnceLock;

/// A write-once slot a driver can wait on. Publishing wakes every live row
/// scheduler and every parked waiter.
#[derive(Debug)]
pub struct CompletionCell<V> {
    value: OnceLock<V>,
    wait: OnceLock<CompletionWait>,
}

#[derive(Debug)]
struct CompletionWait {
    parked: Mutex<usize>,
    cond: Condvar,
}

impl<V> CompletionCell<V> {
    /// Creates an empty cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            wait: OnceLock::new(),
        }
    }

    /// Borrows the value exclusively while no reader can access this cell.
    pub fn get_mut(&mut self) -> Option<&mut V> {
        self.value.get_mut()
    }

    /// Clears a retired publication while exclusive access excludes every reader.
    pub fn reset(&mut self) {
        self.value.take();
        if let Some(wait) = self.wait.get_mut() {
            *wait.parked.get_mut() = 0;
        }
    }

    /// Creates an already completed cell.
    #[must_use]
    pub fn completed(value: V) -> Self {
        Self {
            value: OnceLock::from(value),
            wait: OnceLock::new(),
        }
    }

    /// Publishes a value and wakes every observer.
    ///
    /// # Errors
    /// Returns the given value unchanged if the cell was already set.
    pub fn set(&self, value: V) -> Result<(), V> {
        self.value.set(value)?;
        crate::rows::wake_all();
        if let Some(wait) = self.wait.get() {
            let parked = wait.parked.lock();
            if *parked != 0 {
                wait.cond.notify_all();
            }
            drop(parked);
        }
        Ok(())
    }

    /// Returns the value without blocking.
    #[must_use]
    pub fn get(&self) -> Option<&V> {
        self.value.get()
    }

    /// Consumes the cell and returns the value if it was set.
    #[must_use]
    pub fn into_inner(self) -> Option<V> {
        self.value.into_inner()
    }

    /// Whether the value was published.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.value.get().is_some()
    }

    /// Parks the caller until the value is published.
    #[must_use]
    pub fn wait(&self) -> &V {
        if let Some(value) = self.value.get() {
            return value;
        }
        let wait = self.wait.get_or_init(|| CompletionWait {
            parked: Mutex::new(0),
            cond: Condvar::new(),
        });
        let mut parked = wait.parked.lock();
        while self.value.get().is_none() {
            *parked += 1;
            wait.cond.wait(&mut parked);
            *parked -= 1;
        }
        drop(parked);
        self.value
            .get()
            .unwrap_or_else(|| unreachable!("checked under the lock"))
    }
}

impl<V> Default for CompletionCell<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;

    use super::*;

    #[test]
    fn set_then_get_and_double_set_is_rejected() {
        let cell = CompletionCell::new();
        assert!(!cell.is_set());
        assert_eq!(cell.set(7), Ok(()));
        assert_eq!(cell.get(), Some(&7));
        assert_eq!(cell.set(8), Err(8));
        assert_eq!(*cell.wait(), 7);
    }

    #[test]
    fn wait_parks_until_another_thread_publishes() {
        let cell = Arc::new(CompletionCell::new());
        let publisher = Arc::clone(&cell);
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            publisher.set(3).unwrap();
        });
        assert_eq!(*cell.wait(), 3);
        thread.join().unwrap();
    }

    #[test]
    fn reset_clears_a_retired_value() {
        let mut cell = CompletionCell::completed(1);
        assert!(cell.is_set());
        cell.reset();
        assert!(!cell.is_set());
        assert_eq!(cell.get_mut(), None);
        assert_eq!(cell.into_inner(), None);
    }
}
