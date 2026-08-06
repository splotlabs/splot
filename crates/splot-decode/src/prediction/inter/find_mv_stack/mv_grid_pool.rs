// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Process-global retained pool for the tile-sized `NeighbourMvGrid` backing
//! allocation, mirroring `tile.rs`'s `RETAINED_RECON_ROW_BUFFERS` idiom.

use std::sync::Mutex;

use super::NeighbourMvGrid;
use super::neighbour_grid::GridPlanes;

/// Bounds how many tile-sized neighbour grids are retained between tiles. The
/// grid is the widest per-MI tile buffer (one flag slot plus one motion slot per
/// MI cell), so its backing allocation is reused across tiles/frames rather than
/// reallocated per tile. Kept small to cap retained memory for large frames while
/// still covering concurrent tiles on the parallel path.
const MAX_RETAINED_NEIGHBOUR_MV_GRIDS: usize = 8;
/// Upper bound (in cells) on the capacity a grid may have and still be retained.
/// Comfortably covers a full ~1080p single-tile frame while ensuring one
/// oversized tile cannot pin its high-water allocation in the pool until process
/// exit — grids larger than this are dropped (freed) instead of recycled, so
/// retained memory follows typical tiles rather than the largest ever seen.
const MAX_RETAINED_NEIGHBOUR_MV_CELLS: usize = 1 << 17;
static RETAINED_NEIGHBOUR_MV_GRIDS: Mutex<Vec<GridPlanes>> = Mutex::new(Vec::new());
/// Recycled planes for one tile-sized grid, with the flag plane sized and the
/// motion plane left empty on its retained allocation: a grid that only ever
/// publishes flags — the split path's parse pass — never pays the motion fill,
/// and the first `record_motion` sizes it. See `NeighbourMvGrid::motion_plane`.
pub(super) fn take_neighbour_mv_planes(cells: usize) -> GridPlanes {
    let mut planes = RETAINED_NEIGHBOUR_MV_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default();
    planes.flags.clear();
    planes.flags.resize(cells, None);
    planes.motion.clear();
    planes.leaves.clear();
    planes
}

fn recycle_neighbour_mv_planes(planes: GridPlanes) {
    let capacity = planes.flags.capacity().max(planes.motion.capacity());
    if capacity == 0 || capacity > MAX_RETAINED_NEIGHBOUR_MV_CELLS {
        return;
    }
    let mut retained = RETAINED_NEIGHBOUR_MV_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if retained.len() < MAX_RETAINED_NEIGHBOUR_MV_GRIDS {
        retained.push(planes);
    }
}

impl Drop for NeighbourMvGrid {
    fn drop(&mut self) {
        recycle_neighbour_mv_planes(std::mem::take(&mut self.planes));
    }
}
