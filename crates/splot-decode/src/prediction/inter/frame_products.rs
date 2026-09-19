// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Publication handles and slot-owned entropy-product writers.

use std::sync::Arc;

use splot_parallel::{CompletionCell, Condition};

use crate::bitstream::tile_payload::{FrameCdfSubset, FrameSegmentIdMap};
use crate::filters::ccso::CcsoUnitGrid;

#[derive(Clone, Debug)]
pub(crate) struct EntropyProductHandle<T>(Arc<CompletionCell<Option<Arc<T>>>>);

impl<T> EntropyProductHandle<T> {
    #[cfg(test)]
    pub(crate) fn settled(product: Arc<T>) -> Self {
        Self(Arc::new(CompletionCell::completed(Some(product))))
    }

    pub(crate) fn pending() -> Self {
        Self(Arc::new(CompletionCell::new()))
    }

    pub(crate) fn fail(&self) {
        let _ = self.0.set(None);
    }

    pub(crate) fn product(&self) -> Option<&Arc<T>> {
        self.0.get().and_then(Option::as_ref)
    }

    pub(crate) fn condition(&self) -> Condition<'_> {
        Condition::completion(self.0.as_ref())
    }

    fn can_reuse(&self) -> bool {
        Arc::strong_count(&self.0) == 1
            && self
                .0
                .get()
                .and_then(Option::as_ref)
                .is_none_or(|value| Arc::strong_count(value) == 1)
    }

    #[cfg(test)]
    fn identity(&self) -> *const CompletionCell<Option<Arc<T>>> {
        Arc::as_ptr(&self.0)
    }
}

pub(crate) type FrameCdfHandle = EntropyProductHandle<FrameCdfSubset>;
pub(crate) type SegmentIdMapHandle = EntropyProductHandle<FrameSegmentIdMap>;

#[derive(Debug)]
struct PublishedCcso {
    visible: Option<Arc<CcsoUnitGrid>>,
    spare: Option<Arc<CcsoUnitGrid>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CcsoGridHandle(Arc<CompletionCell<Option<PublishedCcso>>>);

impl CcsoGridHandle {
    #[cfg(test)]
    pub(crate) fn settled(product: Option<Arc<CcsoUnitGrid>>) -> Self {
        let spare = product.is_none().then(|| Arc::new(CcsoUnitGrid::spare()));
        Self(Arc::new(CompletionCell::completed(Some(PublishedCcso {
            visible: product,
            spare,
        }))))
    }

    fn pending() -> Self {
        Self(Arc::new(CompletionCell::new()))
    }

    pub(crate) fn fail(&self) {
        let _ = self.0.set(None);
    }

    pub(crate) fn product(&self) -> Option<&Option<Arc<CcsoUnitGrid>>> {
        self.0
            .get()
            .and_then(Option::as_ref)
            .map(|value| &value.visible)
    }

    pub(crate) fn condition(&self) -> Condition<'_> {
        Condition::completion(self.0.as_ref())
    }

    fn can_reuse(&self) -> bool {
        Arc::strong_count(&self.0) == 1
            && self.0.get().and_then(Option::as_ref).is_none_or(|value| {
                value
                    .spare
                    .as_ref()
                    .or(value.visible.as_ref())
                    .is_none_or(|owned| Arc::strong_count(owned) == 1)
            })
    }
}

#[derive(Clone)]
pub(crate) struct FrameProducts {
    pub(crate) frame_cdfs: FrameCdfHandle,
    pub(crate) ccso_grid: CcsoGridHandle,
    pub(crate) segment_ids: SegmentIdMapHandle,
}

pub(crate) struct FrameProductWriters {
    handles: FrameProducts,
    frame_cdfs: Option<Arc<FrameCdfSubset>>,
    ccso_grid: Option<Arc<CcsoUnitGrid>>,
    segment_ids: Option<Arc<FrameSegmentIdMap>>,
    settled: bool,
}

impl FrameProducts {
    pub(crate) fn can_reuse(&self) -> bool {
        self.frame_cdfs.can_reuse() && self.ccso_grid.can_reuse() && self.segment_ids.can_reuse()
    }

    pub(crate) fn claim(&mut self) -> Option<FrameProductWriters> {
        if !self.can_reuse() {
            return None;
        }
        let (Some(cdf_cell), Some(ccso_cell), Some(segment_cell)) = (
            Arc::get_mut(&mut self.frame_cdfs.0),
            Arc::get_mut(&mut self.ccso_grid.0),
            Arc::get_mut(&mut self.segment_ids.0),
        ) else {
            return None;
        };
        let frame_cdfs = cdf_cell.get_mut().and_then(Option::take);
        let ccso_grid = ccso_cell
            .get_mut()
            .and_then(Option::take)
            .and_then(|state| state.spare.or(state.visible));
        let segment_ids = segment_cell.get_mut().and_then(Option::take);
        cdf_cell.reset();
        ccso_cell.reset();
        segment_cell.reset();
        Some(FrameProductWriters {
            handles: self.clone(),
            frame_cdfs,
            ccso_grid,
            segment_ids,
            settled: false,
        })
    }

    pub(crate) fn into_parts(self) -> (FrameCdfHandle, CcsoGridHandle, SegmentIdMapHandle) {
        (self.frame_cdfs, self.ccso_grid, self.segment_ids)
    }
}

impl FrameProductWriters {
    #[cfg(test)]
    pub(crate) fn fresh() -> Option<Self> {
        FrameProducts::default().claim()
    }
    pub(crate) fn handles(&self) -> FrameProducts {
        self.handles.clone()
    }

    pub(crate) fn frame_cdfs(&mut self) -> crate::Result<&mut FrameCdfSubset> {
        let output = self
            .frame_cdfs
            .get_or_insert_with(|| Arc::new(FrameCdfSubset::from_defaults()));
        Arc::get_mut(output)
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn segment_ids(
        &mut self,
        mi_rows: usize,
        mi_cols: usize,
    ) -> crate::Result<&mut FrameSegmentIdMap> {
        if self.segment_ids.is_none() {
            self.segment_ids = Some(Arc::new(super::block::frame_segment_id_map(
                mi_rows, mi_cols,
            )?));
        }
        let output = Arc::get_mut(
            self.segment_ids
                .as_mut()
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?,
        )
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        output
            .reset(mi_rows, mi_cols)
            .map_err(|error| super::block::segment_map_error(&error))?;
        Ok(output)
    }

    pub(crate) fn inherit_segment_ids(
        &mut self,
        previous: &FrameSegmentIdMap,
    ) -> crate::Result<()> {
        if self.segment_ids.is_none() {
            let (mi_rows, mi_cols) = previous.dimensions();
            self.segment_ids = Some(Arc::new(super::block::frame_segment_id_map(
                mi_rows, mi_cols,
            )?));
        }
        let output = Arc::get_mut(
            self.segment_ids
                .as_mut()
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?,
        )
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        output
            .copy_from(previous)
            .map_err(|error| super::block::segment_map_error(&error))?;
        Ok(())
    }

    pub(crate) fn ccso_grid(&mut self) -> &mut Arc<CcsoUnitGrid> {
        self.ccso_grid
            .get_or_insert_with(|| Arc::new(CcsoUnitGrid::spare()))
    }

    pub(crate) fn take_ccso_blocks(&mut self, active: bool) -> crate::Result<[Vec<u8>; 3]> {
        if !active && self.ccso_grid.is_none() {
            return Ok(std::array::from_fn(|_| Vec::new()));
        }
        let grid = self.ccso_grid();
        Arc::get_mut(grid)
            .map(CcsoUnitGrid::take_blocks)
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn finish_ccso(
        &mut self,
        state: crate::filters::wienerns_lr::tx_records::ccso::CcsoState,
    ) -> crate::Result<Option<Arc<CcsoUnitGrid>>> {
        if !state.active && self.ccso_grid.is_none() {
            return Ok(None);
        }
        let output = self
            .ccso_grid
            .take()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let (output, active) = state.finish_into(output)?;
        self.ccso_grid = Some(Arc::clone(&output));
        Ok(active.then_some(output))
    }

    pub(crate) fn cdf_output(&self) -> crate::Result<Arc<FrameCdfSubset>> {
        self.frame_cdfs
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn finish_segment_ids(&self) -> crate::Result<Arc<FrameSegmentIdMap>> {
        self.segment_ids
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
    }

    pub(crate) fn settle(
        mut self,
        frame_cdfs: Arc<FrameCdfSubset>,
        ccso_grid: Option<Arc<CcsoUnitGrid>>,
        segment_ids: Arc<FrameSegmentIdMap>,
    ) -> FrameProducts {
        self.frame_cdfs.take();
        let owned_ccso = self.ccso_grid.take();
        let ccso_spare = owned_ccso.filter(|owned| {
            ccso_grid
                .as_ref()
                .is_none_or(|visible| !Arc::ptr_eq(owned, visible))
        });
        self.segment_ids.take();
        let _ = self.handles.frame_cdfs.0.set(Some(frame_cdfs));
        let _ = self.handles.ccso_grid.0.set(Some(PublishedCcso {
            visible: ccso_grid,
            spare: ccso_spare,
        }));
        let _ = self.handles.segment_ids.0.set(Some(segment_ids));
        self.settled = true;
        self.handles.clone()
    }
}

impl Drop for FrameProductWriters {
    fn drop(&mut self) {
        if !self.settled {
            self.handles.frame_cdfs.fail();
            self.handles.ccso_grid.fail();
            self.handles.segment_ids.fail();
        }
    }
}

impl Default for FrameProducts {
    fn default() -> Self {
        Self {
            frame_cdfs: FrameCdfHandle::pending(),
            ccso_grid: CcsoGridHandle::pending(),
            segment_ids: SegmentIdMapHandle::pending(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn failed_claim_keeps_completed_products_unchanged() {
        let mut slots = FrameProducts::default();
        let writers = slots.claim().unwrap();
        let cdfs = Arc::new(FrameCdfSubset::from_defaults());
        let segments = Arc::new(FrameSegmentIdMap::new(1, 1).unwrap());
        let ccso = Arc::new(
            CcsoUnitGrid::new(
                false,
                0,
                [false; 3],
                std::array::from_fn(|_| Vec::new()),
                0,
                0,
            )
            .unwrap(),
        );
        let handles = writers.settle(
            Arc::clone(&cdfs),
            Some(Arc::clone(&ccso)),
            Arc::clone(&segments),
        );
        let identities = (
            handles.frame_cdfs.identity(),
            Arc::as_ptr(&cdfs),
            Arc::as_ptr(&ccso),
            Arc::as_ptr(&segments),
        );
        drop(handles);
        let readers = (Arc::clone(&cdfs), Arc::clone(&ccso), Arc::clone(&segments));

        assert!(slots.claim().is_none());
        assert_eq!(slots.frame_cdfs.identity(), identities.0);
        assert!(Arc::ptr_eq(slots.frame_cdfs.product().unwrap(), &cdfs));
        assert!(Arc::ptr_eq(
            slots.ccso_grid.product().unwrap().as_ref().unwrap(),
            &ccso,
        ));
        assert!(Arc::ptr_eq(slots.segment_ids.product().unwrap(), &segments));

        drop(readers);
        drop(cdfs);
        drop(ccso);
        drop(segments);
        let mut reused = slots.claim().unwrap();
        assert_eq!(
            std::ptr::from_ref(reused.frame_cdfs().unwrap()),
            identities.1
        );
        assert_eq!(Arc::as_ptr(reused.ccso_grid()), identities.2);
        assert_eq!(
            std::ptr::from_ref(reused.segment_ids(1, 1).unwrap()),
            identities.3,
        );
    }

    #[test]
    fn inherited_segment_values_use_current_slot_backing() {
        let mut previous_slots = FrameProducts::default();
        let previous_writers = previous_slots.claim().unwrap();
        let mut previous = Arc::new(FrameSegmentIdMap::new(2, 3).unwrap());
        Arc::get_mut(&mut previous).unwrap().fill(7);
        let previous_handles = previous_writers.settle(
            Arc::new(FrameCdfSubset::from_defaults()),
            None,
            Arc::clone(&previous),
        );

        let mut current_slots = FrameProducts::default();
        let current_writers = current_slots.claim().unwrap();
        let warm_map = Arc::new(FrameSegmentIdMap::new(1, 1).unwrap());
        let warm_pointer = Arc::as_ptr(&warm_map);
        let current_handles = current_writers.settle(
            Arc::new(FrameCdfSubset::from_defaults()),
            None,
            Arc::clone(&warm_map),
        );
        drop(current_handles);
        drop(warm_map);
        let mut current_writers = current_slots.claim().unwrap();
        current_writers.inherit_segment_ids(&previous).unwrap();
        let current = current_writers.finish_segment_ids().unwrap();
        assert_eq!(current.as_ref(), previous.as_ref());
        assert_eq!(current.block_min(0, 0, 3, 2), 7);
        assert!(!Arc::ptr_eq(&current, &previous));
        assert_eq!(Arc::as_ptr(&current), warm_pointer);
        let current_pointer = Arc::as_ptr(&current);
        let current_handles = current_writers.settle(
            Arc::new(FrameCdfSubset::from_defaults()),
            None,
            Arc::clone(&current),
        );

        drop(previous_handles);
        drop(previous);
        assert!(previous_slots.claim().is_some());

        drop(current_handles);
        drop(current);
        let mut reused = current_slots.claim().unwrap();
        let reused_map = std::ptr::from_ref::<FrameSegmentIdMap>(reused.segment_ids(2, 3).unwrap());
        assert_eq!(reused_map, current_pointer);
    }

    #[test]
    fn ccso_backing_survives_enabled_disabled_enabled() {
        use crate::filters::wienerns_lr::tx_records::ccso::CcsoState;

        let mut slots = FrameProducts::default();
        let mut enabled = slots.claim().unwrap();
        let blocks = enabled.take_ccso_blocks(true).unwrap();
        let state =
            CcsoState::active(4, [true, false, false], [false; 3], (1, 2, 2), blocks).unwrap();
        let first = enabled.finish_ccso(state).unwrap().unwrap();
        let arc_pointer = Arc::as_ptr(&first);
        let vec_pointer = first.plane_blocks(0).unwrap().as_ptr();
        let handles = enabled.settle(
            Arc::new(FrameCdfSubset::from_defaults()),
            Some(Arc::clone(&first)),
            Arc::new(FrameSegmentIdMap::new(1, 1).unwrap()),
        );
        drop(handles);
        drop(first);

        let mut disabled = slots.claim().unwrap();
        let blocks = disabled.take_ccso_blocks(false).unwrap();
        assert_eq!(blocks[0].as_ptr(), vec_pointer);
        assert!(
            disabled
                .finish_ccso(CcsoState::inactive_with(blocks))
                .unwrap()
                .is_none()
        );
        let handles = disabled.settle(
            Arc::new(FrameCdfSubset::from_defaults()),
            None,
            Arc::new(FrameSegmentIdMap::new(1, 1).unwrap()),
        );
        drop(handles);

        let mut reenabled = slots.claim().unwrap();
        let blocks = reenabled.take_ccso_blocks(true).unwrap();
        assert_eq!(blocks[0].as_ptr(), vec_pointer);
        let state =
            CcsoState::active(4, [true, false, false], [false; 3], (1, 1, 1), blocks).unwrap();
        let grid = reenabled.finish_ccso(state).unwrap().unwrap();
        assert_eq!(Arc::as_ptr(&grid), arc_pointer);
        assert_eq!(grid.plane_blocks(0).unwrap().as_ptr(), vec_pointer);
    }
}
