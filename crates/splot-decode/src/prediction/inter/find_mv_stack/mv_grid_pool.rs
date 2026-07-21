// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Process-global retained pool for the tile-sized `NeighbourMvGrid` backing
//! allocation, mirroring `tile.rs`'s `RETAINED_RECON_ROW_BUFFERS` idiom.

use std::sync::Mutex;

use super::{NeighbourCell, NeighbourMvGrid};

/// Bounds how many tile-sized `NeighbourCell` grids are retained between tiles.
/// The grid is the widest per-MI tile buffer (one `Option<NeighbourCell>` per MI
/// cell), so its backing allocation is reused across tiles/frames rather than
/// reallocated per tile. Kept small to cap retained memory for large frames while
/// still covering concurrent tiles on the parallel path.
const MAX_RETAINED_NEIGHBOUR_MV_GRIDS: usize = 8;
/// Upper bound (in cells) on the capacity a grid may have and still be retained.
/// Comfortably covers a full ~1080p single-tile frame while ensuring one
/// oversized tile cannot pin its high-water allocation in the pool until process
/// exit — grids larger than this are dropped (freed) instead of recycled, so
/// retained memory follows typical tiles rather than the largest ever seen.
const MAX_RETAINED_NEIGHBOUR_MV_CELLS: usize = 1 << 17;
static RETAINED_NEIGHBOUR_MV_GRIDS: Mutex<Vec<Vec<Option<NeighbourCell>>>> = Mutex::new(Vec::new());

pub(super) fn take_neighbour_mv_cells(cells: usize) -> Vec<Option<NeighbourCell>> {
    let mut buffer = RETAINED_NEIGHBOUR_MV_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default();
    buffer.clear();
    buffer.resize(cells, None);
    buffer
}

fn recycle_neighbour_mv_cells(cells: Vec<Option<NeighbourCell>>) {
    if cells.capacity() == 0 || cells.capacity() > MAX_RETAINED_NEIGHBOUR_MV_CELLS {
        return;
    }
    let mut retained = RETAINED_NEIGHBOUR_MV_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if retained.len() < MAX_RETAINED_NEIGHBOUR_MV_GRIDS {
        retained.push(cells);
    }
}

impl Drop for NeighbourMvGrid {
    fn drop(&mut self) {
        recycle_neighbour_mv_cells(std::mem::take(&mut self.cells));
    }
}
