// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded producer/consumer queues for coarse pipeline stages only.
//!
//! These wrap `crossbeam-channel` bounded channels. They are for coarse
//! pipeline-stage boundaries, never per-pixel/per-block/per-row hot loops.
//! The unbounded channel is intentionally not exposed.
use core::num::NonZeroUsize;

use crossbeam_channel::{Receiver, Sender, bounded};

/// Re-exported crossbeam error types so downstream crates do not name
/// `crossbeam-channel` directly.
pub use crossbeam_channel::{RecvError, SendError, TryRecvError, TrySendError};

/// A non-zero bounded-queue capacity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct QueueCapacity(NonZeroUsize);

impl QueueCapacity {
    /// Builds a capacity from a non-zero value.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self(capacity)
    }

    /// The capacity value.
    #[must_use]
    pub fn get(self) -> NonZeroUsize {
        self.0
    }

    /// A coarse pipeline capacity of `per_worker` slots per worker, using
    /// saturating arithmetic (result is always non-zero).
    #[must_use]
    pub fn per_worker(workers: NonZeroUsize, per_worker: NonZeroUsize) -> Self {
        let slots = workers.get().saturating_mul(per_worker.get());
        Self(NonZeroUsize::new(slots).unwrap_or(NonZeroUsize::MIN))
    }
}

/// The sending half of a bounded splot queue.
#[derive(Clone, Debug)]
pub struct QueueSender<T>(Sender<T>);

/// The receiving half of a bounded splot queue.
#[derive(Clone, Debug)]
pub struct QueueReceiver<T>(Receiver<T>);

impl<T> QueueSender<T> {
    /// Sends a value, blocking if the queue is full. See [`SendError`].
    ///
    /// # Errors
    /// Returns [`SendError`] if all receivers have been dropped.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.send(value)
    }

    /// Tries to send without blocking. See [`TrySendError`].
    ///
    /// # Errors
    /// Returns [`TrySendError`] if the queue is full or disconnected.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.0.try_send(value)
    }

    /// The number of queued messages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the queue is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> QueueReceiver<T> {
    /// Receives a value, blocking until one is available. See [`RecvError`].
    ///
    /// # Errors
    /// Returns [`RecvError`] if the queue is empty and all senders are dropped.
    pub fn recv(&self) -> Result<T, RecvError> {
        self.0.recv()
    }

    /// Tries to receive without blocking. See [`TryRecvError`].
    ///
    /// # Errors
    /// Returns [`TryRecvError`] if the queue is empty or disconnected.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.0.try_recv()
    }

    /// The number of queued messages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the queue is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Creates a bounded queue with the given capacity. The unbounded variant is
/// intentionally not provided.
#[must_use]
pub fn bounded_queue<T>(capacity: QueueCapacity) -> (QueueSender<T>, QueueReceiver<T>) {
    let (sender, receiver) = bounded(capacity.get().get());
    (QueueSender(sender), QueueReceiver(receiver))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn per_worker_multiplies() {
        assert_eq!(QueueCapacity::per_worker(nz(4), nz(2)).get().get(), 8);
    }

    #[test]
    fn per_worker_saturates_without_panic() {
        let huge = QueueCapacity::per_worker(nz(usize::MAX), nz(usize::MAX));
        assert!(huge.get().get() >= 1);
    }

    #[test]
    fn round_trips_a_value() {
        let (tx, rx) = bounded_queue::<u32>(QueueCapacity::new(nz(2)));
        tx.send(7).unwrap();
        assert_eq!(rx.recv().unwrap(), 7);
    }

    #[test]
    fn try_send_fills_to_capacity_then_reports_full() {
        let (tx, _rx) = bounded_queue::<u32>(QueueCapacity::new(nz(2)));
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        assert_eq!(tx.try_send(3).unwrap_err(), TrySendError::Full(3));
    }

    #[test]
    fn len_and_is_empty_track_queue() {
        let (tx, rx) = bounded_queue::<u32>(QueueCapacity::new(nz(4)));
        assert!(tx.is_empty());
        assert_eq!(tx.len(), 0);

        tx.send(10).unwrap();
        tx.send(20).unwrap();
        assert_eq!(tx.len(), 2);
        assert_eq!(rx.len(), 2);
        assert!(!rx.is_empty());

        rx.recv().unwrap();
        assert_eq!(rx.len(), 1);
        assert!(!rx.is_empty());

        rx.recv().unwrap();
        assert!(rx.is_empty());
        assert_eq!(rx.len(), 0);
    }
}
