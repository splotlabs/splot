// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(
    unsafe_code,
    reason = "one frame's temporal motion cells, lent to bands as disjoint ranges"
)]

//! Single-allocation storage for one frame's § 7.9 temporal motion cells.

use super::TemporalMotionCell;

/// One frame's § 7.9 temporal motion cells, in a single allocation.
///
/// Bands publish progressively as their superblock rows land, so splitting the
/// frame into one `Vec` per band would cost the allocator one block per band
/// for as long as the frame is a reference. dav2d keeps a single refcounted
/// `f->mvs` block per frame and tracks band progress separately; this is that
/// block.
///
/// Each band owns a disjoint half-open range of it. A band's rows are written
/// only through its own `TemporalMotionBand`, which lives behind that band's
/// mutex until the band publishes and is read-only afterwards, so no two bands
/// ever address the same cell.
pub(super) struct SharedTemporalCells {
    cells: core::cell::UnsafeCell<Vec<TemporalMotionCell>>,
}

/// Safety: the cells are only ever reached through disjoint band ranges.
unsafe impl Send for SharedTemporalCells {}
/// Safety: bands address disjoint ranges, so shared access never aliases.
unsafe impl Sync for SharedTemporalCells {}

impl SharedTemporalCells {
    pub(super) fn new(cells: Vec<TemporalMotionCell>) -> Self {
        Self {
            cells: core::cell::UnsafeCell::new(cells),
        }
    }

    /// Borrows one band's range for reading.
    #[allow(
        clippy::inline_always,
        reason = "TMVP projection reads one row at a time"
    )]
    #[inline(always)]
    pub(super) fn range(&self, start: usize, len: usize) -> Option<&[TemporalMotionCell]> {
        // SAFETY: every band has a disjoint range in the shared cell block.
        let cells: &[TemporalMotionCell] = unsafe { &*self.cells.get() };
        cells.get(start..start.checked_add(len)?)
    }
}

/// One band's own range of a frame's cells.
///
/// Holds the frame's block alive and caches the range's address, so a row read
/// is one dereference — the same shape `DeblockedPlaneStorage` uses for frame
/// samples, and what keeps the § 7.9 projection as cheap as it was when each
/// band owned a `Vec`.
#[derive(Clone)]
pub(super) struct BandCells {
    #[allow(dead_code, reason = "keeps the block `data` points into alive")]
    owner: std::sync::Arc<SharedTemporalCells>,
    data: core::ptr::NonNull<[TemporalMotionCell]>,
}

/// Safety: the owning block is `Send`, and a band's range is disjoint from
/// every other band's.
unsafe impl Send for BandCells {}
/// Safety: the owning block is `Sync`, and a band's range is disjoint from
/// every other band's.
unsafe impl Sync for BandCells {}

impl BandCells {
    pub(super) fn new(
        owner: std::sync::Arc<SharedTemporalCells>,
        start: usize,
        len: usize,
    ) -> Option<Self> {
        let data = core::ptr::NonNull::from(owner.range(start, len)?);
        Some(Self { owner, data })
    }

    pub(super) const fn len(&self) -> usize {
        self.data.len()
    }

    #[allow(
        clippy::inline_always,
        reason = "TMVP projection reads one row at a time"
    )]
    #[inline(always)]
    pub(super) fn as_slice(&self) -> &[TemporalMotionCell] {
        // SAFETY: `owner` keeps this band's disjoint range alive.
        unsafe { self.data.as_ref() }
    }

    /// Lends the band's range for writing, once per record run.
    pub(super) fn as_mut_slice(&mut self) -> &mut [TemporalMotionCell] {
        // SAFETY: `&mut self` uniquely borrows this unpublished band.
        unsafe { self.data.as_mut() }
    }
}
