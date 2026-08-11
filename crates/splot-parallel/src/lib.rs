// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-parallel` — the approved concurrency primitives for splot codec runtime code.
//!
//! `splot` uses exactly one data-parallel engine (Rayon, via a *local* owned
//! [`WorkerPool`]). This crate depends on no other `splot-*` crate. It does not
//! use the global Rayon pool, `build_global`, unbounded channels, or any async
//! runtime.
//!
//! Downstream codec crates write data-parallel loops with the [`prelude`] (the
//! curated Rayon parallel-iterator traits) **inside** [`WorkerPool::install`], so
//! work runs on the configured pool rather than Rayon's global pool. See the
//! [`prelude`] docs and `docs/ARCHITECTURE.md` for the required pattern.
//!
//! Pipelined stages whose work depends on another stage's progress use
//! [`AdmissionScheduler`] rather than blocking: a pool task must never wait,
//! because work stealing can resume it only from below its own stack. Such a
//! task is submitted with the [`WatermarkCell`] / [`CompletionCell`] conditions
//! it needs and is spawned by whoever publishes the last of them.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.
pub mod admission;
pub mod completion;
pub mod error;
pub mod frame_delay;
pub mod pool;
pub mod prelude;
mod progress;
pub mod thread_count;
pub mod watermark;

pub use admission::{AdmissionScheduler, AdmissionWaiter, Admit, CompletionSource, Condition, Job};
pub use completion::CompletionCell;
pub use error::{FrameDelayParseError, ParallelError, ThreadCountParseError};
pub use frame_delay::FrameDelay;
pub use pool::{
    PoolProgressSnapshot, TaskScope, WorkerPool, assist_pool_or_park, current_pool_width,
    current_worker_index, on_multiworker_pool, on_worker_pool, pool_progress_snapshot,
    ready_task_scope,
};
pub use progress::PoolWaitMetrics;
pub use thread_count::ThreadCount;
pub use watermark::WatermarkCell;
