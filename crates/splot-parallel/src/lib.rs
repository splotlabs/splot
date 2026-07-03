// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-parallel` — the approved concurrency primitives for splot codec runtime code.
//!
//! `splot` uses exactly one data-parallel engine (Rayon, via a *local* owned
//! [`WorkerPool`]) and exactly one coarse-pipeline queue primitive
//! (`crossbeam-channel`, bounded only, via [`bounded_queue`]). This crate
//! depends on no other `splot-*` crate. It does not use the global Rayon pool,
//! `build_global`, unbounded channels, or any async runtime.
//!
//! Downstream codec crates write data-parallel loops with the [`prelude`] (the
//! curated Rayon parallel-iterator traits) **inside** [`WorkerPool::install`], so
//! work runs on the configured pool rather than Rayon's global pool. See the
//! [`prelude`] docs and `docs/CONCURRENCY.md` for the required pattern.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.
pub mod error;
pub mod pool;
pub mod prelude;
pub mod queue;
pub mod thread_count;

pub use error::{ParallelError, ThreadCountParseError};
pub use pool::{WorkerPool, current_worker_count, current_worker_index, on_multiworker_pool};
pub use queue::{
    QueueCapacity, QueueReceiver, QueueSender, RecvError, SendError, TryRecvError, TrySendError,
    bounded_queue,
};
pub use thread_count::ThreadCount;
