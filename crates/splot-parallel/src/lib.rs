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
//! The decoder runs its frames on the [`RowScheduler`]: dedicated worker
//! threads that claim rows of in-flight frames oldest first, reading readiness
//! from the frames' own counters and the [`WatermarkCell`] /
//! [`CompletionCell`] values their references publish.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.
pub mod completion;
pub mod error;
pub mod frame_delay;
pub mod pool;
pub mod prelude;
pub mod rows;
pub mod thread_count;
pub mod watermark;

pub use completion::CompletionCell;
pub use error::{FrameDelayParseError, ParallelError, ThreadCountParseError};
pub use frame_delay::FrameDelay;
pub use pool::WorkerPool;
pub use rows::{
    Claim, FrameWork, ParallelStage, RowProgress, RowScheduler, RowWake, Stage, run_on_thread,
    spawn_thread,
};
pub use thread_count::ThreadCount;
pub use watermark::WatermarkCell;
