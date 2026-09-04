// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One coefficient buffer per reconstruction row.
//!
//! A transform block's quantised coefficients outlive their parse: the row's
//! commands are replayed by a later pass, on another thread. Owning them per
//! block meant a `Vec` per transform, the decode's largest source of
//! allocations.
//!
//! The row already crosses that gap, so its blocks share one buffer and keep a
//! range into it. The buffer returns to a pool when the row's last block is
//! dropped, so a row costs no allocation once the pool is warm -- without that
//! the per-row buffer fragments the small zone worse than the blocks did.

use core::cell::RefCell;
use core::ops::Range;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};

/// A row's coefficients, shared by every block parsed into it.
pub(crate) struct RowCoeffs(OnceLock<Vec<i32>>);

impl core::fmt::Debug for RowCoeffs {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RowCoeffs")
            .field("len", &self.0.get().map_or(0, Vec::len))
            .finish()
    }
}

impl PartialEq for RowCoeffs {
    fn eq(&self, other: &Self) -> bool {
        self.0.get() == other.0.get()
    }
}

impl Eq for RowCoeffs {}

impl Drop for RowCoeffs {
    fn drop(&mut self) {
        let Some(mut coeffs) = self.0.take() else {
            return;
        };
        crate::support::buffer_pool::recycle(&mut coeffs);
    }
}

/// The handle a block holds until its row is sealed.
pub(crate) type CoeffBatch = Arc<RowCoeffs>;

/// Handles kept for the next row to claim.
///
/// Sized for the rows the pipeline keeps in flight: a block holds its row's
/// handle until the commit spine replays it, so a smaller reserve is empty
/// every time a row opens.
const MAX_SPARE_HANDLES: usize = 128;

fn spare_handles() -> &'static Mutex<Vec<CoeffBatch>> {
    static SPARE: OnceLock<Mutex<Vec<CoeffBatch>>> = OnceLock::new();
    SPARE.get_or_init(|| Mutex::new(Vec::new()))
}

/// A handle for a new row, reusing one whose blocks have all been replayed.
///
/// The reserve is searched rather than popped: a handle a row still in flight
/// is holding must be left where it is, not taken out to be tested and lost.
fn new_handle() -> CoeffBatch {
    let mut spare = spare_handles().lock();
    let reusable = spare
        .iter()
        .position(|handle| Arc::strong_count(handle) == 1);
    if let Some(index) = reusable {
        let mut handle = spare.swap_remove(index);
        drop(spare);
        if let Some(row) = Arc::get_mut(&mut handle)
            && let Some(mut coeffs) = row.0.take()
        {
            crate::support::buffer_pool::recycle(&mut coeffs);
        }
        return handle;
    }
    drop(spare);
    Arc::new(RowCoeffs(OnceLock::new()))
}

/// Offers a spent handle back, for the next row.
fn retire_handle(handle: CoeffBatch) {
    let mut spare = spare_handles().lock();
    if spare.len() < MAX_SPARE_HANDLES {
        spare.push(handle);
    }
}

thread_local! {
    /// Coefficients parsed so far for the row open on this worker.
    static PARSE_ARENA: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// The handle those blocks hold, filled by [`seal`].
    static PARSE_BATCH: RefCell<Option<CoeffBatch>> = const { RefCell::new(None) };
}

/// The handle for the row being parsed, which [`seal`] later fills.
pub(crate) fn batch() -> CoeffBatch {
    PARSE_BATCH.with(|batch| {
        batch.try_borrow_mut().map_or_else(
            |_| new_handle(),
            |mut batch| Arc::clone(batch.get_or_insert_with(new_handle)),
        )
    })
}

/// Appends one block's coefficients and returns their range in the row.
pub(crate) fn append(coeffs: &[i32]) -> Range<u32> {
    PARSE_ARENA.with(|arena| {
        arena.try_borrow_mut().map_or(0..0, |mut arena| {
            let start = u32::try_from(arena.len()).unwrap_or(u32::MAX);
            if arena.try_reserve(coeffs.len()).is_err() {
                return 0..0;
            }
            arena.extend_from_slice(coeffs);
            let end = u32::try_from(arena.len()).unwrap_or(u32::MAX);
            start..end
        })
    })
}

/// Publishes the row to the blocks holding it and opens the next one.
///
/// Every block parsed since the last seal reads this buffer, so a seal must
/// come between a block's parse and its reconstruction.
pub(crate) fn seal() {
    let coeffs = PARSE_ARENA.with(|arena| {
        arena.try_borrow_mut().map_or_else(
            |_| Vec::new(),
            |mut arena| {
                let mut row = crate::support::buffer_pool::take(arena.len());
                if row
                    .try_reserve(arena.len().saturating_sub(row.capacity()))
                    .is_ok()
                {
                    row.extend_from_slice(&arena);
                }
                arena.clear();
                row
            },
        )
    });
    PARSE_BATCH.with(|batch| {
        if let Ok(mut batch) = batch.try_borrow_mut()
            && let Some(open) = batch.take()
        {
            let _ = open.0.set(coeffs);
            retire_handle(open);
        }
    });
}

/// Discards anything a failed parse left behind.
pub(crate) fn reset() {
    PARSE_ARENA.with(|arena| {
        if let Ok(mut arena) = arena.try_borrow_mut() {
            arena.clear();
        }
    });
    seal();
}

/// Reads one block's coefficients out of its row.
pub(crate) fn coeffs_of<'a>(batch: &'a CoeffBatch, range: &Range<u32>) -> &'a [i32] {
    let start = range.start as usize;
    let end = range.end as usize;
    batch
        .0
        .get()
        .and_then(|coeffs| coeffs.get(start..end))
        .unwrap_or(&[])
}

/// Seals `coeffs` into a batch covering all of them, for tests.
#[cfg(test)]
pub(crate) fn sealed(coeffs: Vec<i32>) -> (CoeffBatch, Range<u32>) {
    let len = u32::try_from(coeffs.len()).unwrap_or(u32::MAX);
    let batch = Arc::new(RowCoeffs(OnceLock::new()));
    let _ = batch.0.set(coeffs);
    (batch, 0..len)
}
