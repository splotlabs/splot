// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ownership for direct reconstruction into the canonical workspace.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

#![allow(
    unsafe_code,
    reason = "direct reconstruction lends disjoint canonical rectangles without moving the frame allocation"
)]

use core::cell::UnsafeCell;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use splot_recon::{
    CurrentFrameLeaseProvenance, CurrentFrameRect, CurrentFrameWorkspace, PlaneRect, ReconSample,
    SuspendedCurrentFrameRect,
};

use crate::Result;

use super::invalid_inter_tile_scheduling_state;

struct OverlapState {
    live: Vec<bool>,
    next_commit: usize,
    committing: bool,
    taken: bool,
}

enum LeaseSurface<T: ReconSample> {
    Active(CurrentFrameRect<'static, T>),
    Suspended(SuspendedCurrentFrameRect<T>),
    Retired,
}

struct SurfaceSlot<T: ReconSample>(UnsafeCell<LeaseSurface<T>>);

/// Stable canonical storage whose disjoint prepass rectangles may outlive tasks.
pub(super) struct OverlappedReconWorkspace<T: ReconSample> {
    workspace: UnsafeCell<Option<CurrentFrameWorkspace<T>>>,
    provenance: UnsafeCell<Option<CurrentFrameLeaseProvenance<T>>>,
    surfaces: Vec<SurfaceSlot<T>>,
    access: RwLock<()>,
    state: Mutex<OverlapState>,
}

/// One disjoint canonical rectangle retained until its ordered commit.
pub(super) struct DirectReconRect<T: ReconSample> {
    owner: Arc<OverlappedReconWorkspace<T>>,
    index: usize,
    active: bool,
}

// SAFETY: every lease owns row slices disjoint from every other lease.
unsafe impl<T: ReconSample> Send for DirectReconRect<T> {}
// SAFETY: the access lock excludes whole-frame borrows while any surface is active.
unsafe impl<T: ReconSample> Send for OverlappedReconWorkspace<T> {}
// SAFETY: the access lock suspends every surface before whole-frame access;
// shared holders use distinct checked rectangle indices.
unsafe impl<T: ReconSample> Sync for OverlappedReconWorkspace<T> {}

impl<T: ReconSample> Drop for OverlappedReconWorkspace<T> {
    fn drop(&mut self) {
        for slot in &mut self.surfaces {
            *slot.0.get_mut() = LeaseSurface::Retired;
        }
        self.workspace.get_mut().take();
    }
}

impl<T: ReconSample> OverlappedReconWorkspace<T> {
    pub(super) fn new(
        mut workspace: CurrentFrameWorkspace<T>,
        rects: &[PlaneRect],
    ) -> Result<(Arc<Self>, Vec<DirectReconRect<T>>)> {
        let surfaces = workspace.rect_surfaces(rects)?;
        let surfaces = unsafe {
            // SAFETY: the owner below retains the workspace allocation until
            // every surface is retired. Moving `Vec` headers does not move their
            // sample allocations, and no operation resizes those allocations.
            core::mem::transmute::<Vec<CurrentFrameRect<'_, T>>, Vec<CurrentFrameRect<'static, T>>>(
                surfaces,
            )
        };
        let owner = Arc::new(Self {
            workspace: UnsafeCell::new(Some(workspace)),
            provenance: UnsafeCell::new(None),
            surfaces: surfaces
                .into_iter()
                .map(|surface| SurfaceSlot(UnsafeCell::new(LeaseSurface::Active(surface))))
                .collect(),
            access: RwLock::new(()),
            state: Mutex::new(OverlapState {
                live: vec![true; rects.len()],
                next_commit: 0,
                committing: false,
                taken: false,
            }),
        });
        let leases = (0..rects.len())
            .map(|index| DirectReconRect {
                owner: Arc::clone(&owner),
                index,
                active: true,
            })
            .collect();
        Ok((owner, leases))
    }

    pub(super) fn with_commit<R>(
        &self,
        count: usize,
        commit: impl FnOnce(&mut CurrentFrameWorkspace<T>) -> Result<R>,
    ) -> Result<R> {
        let _access = self.access.write().unwrap_or_else(PoisonError::into_inner);
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let end = state.next_commit.saturating_add(count);
            if state.taken
                || state.committing
                || end > state.live.len()
                || state.live[state.next_commit..end].iter().any(|&live| live)
            {
                return Err(invalid_inter_tile_scheduling_state());
            }
            state.committing = true;
        }
        self.suspend_live_surfaces();
        let result = unsafe {
            // SAFETY: the write guard excludes surface access and every live
            // surface is suspended, so no sample reference exists.
            match (&mut *self.workspace.get()).as_mut() {
                Some(workspace) => {
                    let result = commit(workspace);
                    *self.provenance.get() = Some(workspace.lease_provenance());
                    result
                }
                None => Err(invalid_inter_tile_scheduling_state()),
            }
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.committing = false;
        if result.is_ok() {
            state.next_commit = state.next_commit.saturating_add(count);
        }
        result
    }

    pub(super) fn take(&self) -> Result<CurrentFrameWorkspace<T>> {
        let _access = self.access.write().unwrap_or_else(PoisonError::into_inner);
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.taken
                || state.committing
                || state.next_commit != state.live.len()
                || state.live.iter().any(|&live| live)
            {
                return Err(invalid_inter_tile_scheduling_state());
            }
            state.taken = true;
        }
        unsafe {
            // SAFETY: every rectangle has been dropped and `taken` is terminal.
            (&mut *self.workspace.get())
                .take()
                .ok_or_else(invalid_inter_tile_scheduling_state)
        }
    }

    fn with_surface<R>(
        &self,
        index: usize,
        use_surface: impl FnOnce(&mut CurrentFrameRect<'static, T>) -> R,
    ) -> Option<R> {
        let _access = self.access.read().unwrap_or_else(PoisonError::into_inner);
        let slot = self.surfaces.get(index)?;
        let surface = unsafe {
            // SAFETY: each `DirectReconRect` owns one unique index, and the read
            // guard prevents commit from suspending its surface during use.
            &mut *slot.0.get()
        };
        if matches!(surface, LeaseSurface::Suspended(_)) {
            let LeaseSurface::Suspended(suspended) =
                core::mem::replace(surface, LeaseSurface::Retired)
            else {
                return None;
            };
            let provenance = unsafe {
                // SAFETY: the access guard excludes the writer that refreshes
                // this provenance after each whole-workspace borrow.
                (&*self.provenance.get()).as_ref()
            }?;
            let resumed = unsafe {
                // SAFETY: no whole-workspace borrow exists under the read guard;
                // the allocation remains owned and has not been resized.
                suspended.resume(provenance)
            };
            let Ok(resumed) = resumed else {
                return None;
            };
            *surface = LeaseSurface::Active(resumed);
        }
        match surface {
            LeaseSurface::Active(surface) => Some(use_surface(surface)),
            LeaseSurface::Suspended(_) | LeaseSurface::Retired => None,
        }
    }

    fn suspend_live_surfaces(&self) {
        for slot in &self.surfaces {
            let surface = unsafe {
                // SAFETY: the caller holds the exclusive access guard.
                &mut *slot.0.get()
            };
            if matches!(surface, LeaseSurface::Active(_)) {
                let LeaseSurface::Active(active) =
                    core::mem::replace(surface, LeaseSurface::Retired)
                else {
                    continue;
                };
                *surface = LeaseSurface::Suspended(active.suspend());
            }
        }
    }

    fn finish_writes(&self, index: usize) -> Result<()> {
        let _access = self.access.read().unwrap_or_else(PoisonError::into_inner);
        let slot = self
            .surfaces
            .get(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let surface = unsafe {
            // SAFETY: this lease uniquely owns `index`; the read guard excludes
            // whole-workspace access while its references are suspended.
            &mut *slot.0.get()
        };
        if matches!(surface, LeaseSurface::Active(_)) {
            let LeaseSurface::Active(active) = core::mem::replace(surface, LeaseSurface::Retired)
            else {
                return Err(invalid_inter_tile_scheduling_state());
            };
            *surface = LeaseSurface::Suspended(active.suspend());
        }
        matches!(surface, LeaseSurface::Suspended(_))
            .then_some(())
            .ok_or_else(invalid_inter_tile_scheduling_state)
    }

    fn retire(&self, index: usize) -> Result<()> {
        let _access = self.access.read().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let live = state
            .live
            .get_mut(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        if !*live {
            return Err(invalid_inter_tile_scheduling_state());
        }
        let slot = self
            .surfaces
            .get(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        let surface = unsafe {
            // SAFETY: this lease uniquely owns `index`; the read guard excludes
            // suspension while the slot is replaced.
            &mut *slot.0.get()
        };
        if !matches!(
            surface,
            LeaseSurface::Active(_) | LeaseSurface::Suspended(_)
        ) {
            return Err(invalid_inter_tile_scheduling_state());
        }
        *surface = LeaseSurface::Retired;
        *live = false;
        Ok(())
    }
}

impl<T: ReconSample> DirectReconRect<T> {
    pub(super) fn with_surface<R>(
        &mut self,
        use_surface: impl FnOnce(&mut CurrentFrameRect<'static, T>) -> R,
    ) -> Option<R> {
        self.active
            .then(|| self.owner.with_surface(self.index, use_surface))
            .flatten()
    }

    pub(super) fn retire(mut self) -> Result<()> {
        let result = self.owner.retire(self.index);
        if result.is_ok() {
            self.active = false;
        }
        result
    }

    pub(super) fn finish_writes(&mut self) -> Result<()> {
        if !self.active {
            return Err(invalid_inter_tile_scheduling_state());
        }
        self.owner.finish_writes(self.index)
    }
}

impl<T: ReconSample> Drop for DirectReconRect<T> {
    fn drop(&mut self) {
        if self.active && self.owner.retire(self.index).is_ok() {
            self.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use splot_recon::{
        BitDepth, CurrentFrameSurface, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
        PlaneSize,
    };

    use super::*;

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).expect("test rectangle")
    }

    #[test]
    fn commit_accesses_only_retired_region_while_future_lease_remains_live() {
        let size = PlaneSize::new(8, 8).expect("test size");
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size,
            rect(0, 0, 8, 8),
        )
        .expect("test frame info");
        let workspace = CurrentFrameWorkspace::<u8>::new(info, 0).expect("test workspace");
        let left = rect(0, 0, 4, 8);
        let right = rect(4, 0, 4, 8);
        let (owner, leases) =
            OverlappedReconWorkspace::new(workspace, &[left, right]).expect("direct leases");
        let mut leases = leases.into_iter();
        let mut left_lease = leases.next().expect("left lease");
        let mut right_lease = leases.next().expect("right lease");

        left_lease
            .with_surface(|surface| {
                CurrentFrameSurface::Rect(surface).write_rect(PlaneId::Y, left, &[3; 32], 4)
            })
            .expect("active left lease")
            .expect("left write");
        left_lease.retire().expect("retire left");
        owner
            .with_commit(1, |workspace| {
                workspace.write_rect(PlaneId::Y, left, &[5; 32], 4)?;
                Ok(())
            })
            .expect("left commit");
        right_lease
            .with_surface(|surface| {
                CurrentFrameSurface::Rect(surface).write_rect(PlaneId::Y, right, &[7; 32], 4)
            })
            .expect("active right lease")
            .expect("right write");
        right_lease.retire().expect("retire right");
        owner.with_commit(1, |_| Ok(())).expect("right commit");

        let workspace = owner.take().expect("finished workspace");
        for row in workspace
            .rect_rows(PlaneId::Y, rect(0, 0, 8, 8))
            .expect("rows")
        {
            assert_eq!(&row[..4], &[5; 4]);
            assert_eq!(&row[4..], &[7; 4]);
        }
    }
}
