// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Curated parallel-iteration prelude.
//!
//! Re-exports the Rayon parallel-iterator traits that downstream codec crates
//! need to write data-parallel loops — **without** taking a direct `rayon`
//! dependency (which the concurrency policy forbids outside `splot-parallel`).
//! This is the *only* sanctioned way for `splot-encode`, `splot-decode`, and
//! future codec crates to reach Rayon's parallel iterators.
//!
//! # Required usage
//!
//! Parallel iterators MUST be driven inside [`crate::WorkerPool::install`] so the
//! work runs on the context's configured pool (sized by `--threads`) instead of
//! Rayon's implicit **global** pool. A parallel iterator written at the top level
//! — outside `install` — silently runs on the global pool and will not scale with
//! the configured worker count; `cargo xtask check-concurrency-policy` flags this.
//!
//! For deterministic output regardless of thread count, prefer the *indexed*
//! iterators ([`IndexedParallelIterator`]) and collect into an ordered container
//! (results land in stable index order), rather than reducing in completion order
//! or mutating shared state.
//!
//! ```
//! use splot_parallel::prelude::*;
//! use splot_parallel::{ThreadCount, WorkerPool};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = WorkerPool::new(ThreadCount::Auto)?;
//! // Indexed parallel map, collected in stable index order → deterministic.
//! let doubled: Vec<u64> = pool.install(|| (0..8u64).into_par_iter().map(|x| x * 2).collect());
//! assert_eq!(doubled, (0..8u64).map(|x| x * 2).collect::<Vec<_>>());
//! # Ok(()) }
//! ```

pub use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator,
    IntoParallelRefMutIterator, ParallelIterator,
};
pub use rayon::slice::{ParallelSlice, ParallelSliceMut};
