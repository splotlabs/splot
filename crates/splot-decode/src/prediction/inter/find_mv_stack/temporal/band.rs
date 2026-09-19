// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One full-width source superblock row of an AV2 § 7.9 motion field.

use std::ops::Range;
use std::sync::Arc;

use super::{
    MotionFieldLayout, TemporalMotionBlock, TemporalMotionCell, TemporalMotionField,
    TemporalMotionFieldMetadata, TemporalMotionRows, resolve_block_refs, resolve_temporal_refs,
    visit_temporal_block_cells,
};

/// One full-width source superblock row of temporal motion.
///
/// A band the frame is still filling owns its cells. A band sliced off a field
/// that is already published borrows that field's cells instead, so publishing
/// a settled field costs one shared handle per band rather than a copy of it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionBand {
    pub(super) layout: MotionFieldLayout,
    pub(super) metadata: TemporalMotionFieldMetadata,
    pub(super) row_base8: usize,
    pub(super) cells: BandCells,
}

#[derive(Clone)]
pub(super) enum BandCells {
    Owned(Vec<TemporalMotionCell>),
    Published(Arc<Vec<TemporalMotionCell>>),
    Shared(Arc<TemporalMotionField>, Range<usize>),
}

impl BandCells {
    pub(super) fn cells(&self) -> &[TemporalMotionCell] {
        match self {
            Self::Owned(cells) => cells,
            Self::Published(cells) => cells,
            Self::Shared(field, range) => field
                .contiguous_cells()
                .get(range.clone())
                .unwrap_or_default(),
        }
    }

    fn owned_mut(&mut self) -> Option<&mut Vec<TemporalMotionCell>> {
        match self {
            Self::Owned(cells) => Some(cells),
            Self::Published(cells) => Arc::get_mut(cells),
            Self::Shared(_, _) => None,
        }
    }
}

impl core::fmt::Debug for BandCells {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.cells().fmt(formatter)
    }
}

impl PartialEq for BandCells {
    fn eq(&self, other: &Self) -> bool {
        self.cells() == other.cells()
    }
}

impl Eq for BandCells {}

impl TemporalMotionBand {
    pub(crate) fn is_field_view(&self) -> bool {
        matches!(self.cells, BandCells::Shared(_, _))
    }

    pub(crate) fn is_exclusive(&mut self) -> bool {
        self.cells.owned_mut().is_some()
    }

    pub(crate) fn reset(
        &mut self,
        layout: MotionFieldLayout,
        metadata: &TemporalMotionFieldMetadata,
        index: usize,
    ) -> crate::Result<()> {
        let cells = self
            .cells
            .owned_mut()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let count = layout
            .width8()
            .checked_mul(layout.rows8(index).len())
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        cells.clear();
        cells.try_reserve_exact(count).map_err(|_| {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: splot_recon::PlaneId::Y,
                context: "inter reference motion band",
            }
        })?;
        cells.resize(count, TemporalMotionCell::default());
        self.layout = layout;
        self.metadata = metadata.clone();
        self.row_base8 = layout.rows8(index).start;
        Ok(())
    }

    pub(crate) fn vacant(
        layout: MotionFieldLayout,
        metadata: &TemporalMotionFieldMetadata,
    ) -> Self {
        Self {
            layout,
            metadata: metadata.clone(),
            row_base8: 0,
            cells: BandCells::Owned(Vec::new()),
        }
    }

    /// Publishes owned cells once; complete-field views share the same storage.
    pub(crate) fn into_shared(mut self) -> Self {
        if let BandCells::Owned(cells) = &mut self.cells {
            self.cells = BandCells::Published(Arc::new(core::mem::take(cells)));
        }
        self
    }

    pub(crate) fn row_end8(&self) -> usize {
        self.row_base8
            .saturating_add(self.cells.cells().len().div_ceil(self.layout.width8.max(1)))
            .min(self.layout.height8)
    }

    #[allow(
        clippy::inline_always,
        reason = "TMVP projection reads one row at a time"
    )]
    #[inline(always)]
    pub(super) fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]> {
        let row = y8.checked_sub(self.row_base8)?;
        let start = row.checked_mul(self.layout.width8)?;
        let end = start.checked_add(self.layout.width8)?;
        self.cells.cells().get(start..end)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(crate) fn record_blocks(&mut self, blocks: &[TemporalMotionBlock]) -> crate::Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let row_base8 = self.row_base8;
        let row_end8 = self.row_end8();
        let width8 = self.layout.width8;
        let cells = self
            .cells
            .owned_mut()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        for &block in blocks {
            let resolved =
                resolve_block_refs(block.ref_order_hints, &self.metadata.ref_order_hints);
            visit_temporal_block_cells(block, width8, row_end8, |y8, x8, cell, hints| {
                let Some(row) = y8.checked_sub(row_base8) else {
                    return;
                };
                let Some(index) = row
                    .checked_mul(width8)
                    .and_then(|base| base.checked_add(x8))
                else {
                    return;
                };
                if let Some(target) = cells.get_mut(index) {
                    *target = resolve_temporal_refs(cell, hints, &resolved);
                }
            });
        }
        Ok(())
    }
}

impl TemporalMotionRows for TemporalMotionBand {
    fn dimensions8(&self) -> (usize, usize) {
        (self.layout.width8(), self.layout.height8())
    }

    fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]> {
        TemporalMotionBand::row(self, y8)
    }
}
