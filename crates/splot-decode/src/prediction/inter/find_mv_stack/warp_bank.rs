// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.12.2 warp-parameter bank and the WRL warp-parameter stack.

use super::{
    DEFAULT_WARP_PARAMS, MAX_WARP_REF_CANDIDATES, MvBlockContext, NeighbourCell, NeighbourMvGrid,
    bank_ring_update, seed_walk_from_row_above,
};

const WARP_PARAM_BANK_SIZE: usize = 4;
const MAX_WARP_SB_HITS: u32 = 64;
const WARP_BANK_REFS: usize = 7;

pub(crate) struct WarpParamBank {
    entries: [[[i32; 6]; WARP_PARAM_BANK_SIZE]; WARP_BANK_REFS],
    sizes: [usize; WARP_BANK_REFS],
    starts: [usize; WARP_BANK_REFS],
    sb_hits: u32,
    current_sb: Option<(usize, usize)>,
}

impl WarpParamBank {
    pub(crate) fn new() -> Self {
        Self {
            entries: [[DEFAULT_WARP_PARAMS; WARP_PARAM_BANK_SIZE]; WARP_BANK_REFS],
            sizes: [0; WARP_BANK_REFS],
            starts: [0; WARP_BANK_REFS],
            sb_hits: 0,
            current_sb: None,
        }
    }

    pub(crate) fn reset_for_leaf(
        &mut self,
        grid: &NeighbourMvGrid,
        mi_row: usize,
        mi_col: usize,
        sb_size4: usize,
    ) {
        let sb = (mi_row / sb_size4.max(1), mi_col / sb_size4.max(1));
        if self.current_sb == Some(sb) {
            return;
        }
        let new_sb_row = self.current_sb.is_none_or(|(row, _)| row != sb.0);
        self.current_sb = Some(sb);
        self.sb_hits = 0;
        if new_sb_row {
            self.sizes = [0; WARP_BANK_REFS];
            self.starts = [0; WARP_BANK_REFS];
        }
        seed_walk_from_row_above(grid, sb.0 * sb_size4, sb.1 * sb_size4, sb_size4, |cell| {
            if cell.is_warp()
                && let Some(params) = cell.warp_params
            {
                self.update(cell.ref_frame0, params);
            }
        });
    }

    pub(crate) fn update(&mut self, ref_frame0: i8, params: [i32; 6]) {
        if self.sb_hits >= MAX_WARP_SB_HITS {
            return;
        }
        self.sb_hits += 1;
        let Some(ref_idx) = usize::try_from(ref_frame0)
            .ok()
            .filter(|&idx| idx < WARP_BANK_REFS)
        else {
            return;
        };
        bank_ring_update(
            &mut self.entries[ref_idx],
            &mut self.sizes[ref_idx],
            &mut self.starts[ref_idx],
            params,
            |candidate| candidate[2..6] == params[2..6],
        );
    }

    pub(super) fn fill(&self, ref_frame0: i8, warp: &mut WarpParamStack) {
        let Some(ref_idx) = usize::try_from(ref_frame0)
            .ok()
            .filter(|&idx| idx < WARP_BANK_REFS)
        else {
            return;
        };
        let count = self.sizes[ref_idx];
        let start = self.starts[ref_idx];
        for i in (0..count).rev() {
            let idx = (start + i) % WARP_PARAM_BANK_SIZE;
            warp.insert(self.entries[ref_idx][idx]);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WarpParamStack {
    pub(super) slots: [[i32; 6]; MAX_WARP_REF_CANDIDATES],
    pub(super) num_found: usize,
}

impl WarpParamStack {
    pub(super) fn new() -> Self {
        Self {
            slots: [DEFAULT_WARP_PARAMS; MAX_WARP_REF_CANDIDATES],
            num_found: 0,
        }
    }

    pub(super) fn insert(&mut self, params: [i32; 6]) {
        if self.num_found < MAX_WARP_REF_CANDIDATES {
            self.slots[self.num_found] = params;
            self.num_found += 1;
        }
    }

    pub(super) fn add_scan_point(&mut self, cell: NeighbourCell, block: &MvBlockContext) {
        if cell.is_inter
            && cell.is_warp()
            && cell.ref_frame0 == block.ref_frame0
            && let Some(params) = cell.warp_params
        {
            self.insert(params);
        }
    }
}
