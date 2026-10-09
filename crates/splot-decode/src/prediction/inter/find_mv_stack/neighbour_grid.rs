// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Neighbour mode-info grid.
//!
//! The grid keeps one plane over a window of two superblock rows of one tile:
//! every § 7.12 probe reads the current superblock row or the row above it,
//! so a plane row is reused once its superblock row is two rows old. Each
//! cell names the leaf that covers it; the leaf table holds the syntax facts
//! that neighbour context derivation reads while symbols are decoded, and,
//! once § 7.12 has resolved the leaf, the motion payload that the reference
//! MV stack and the warp derivations read. A leaf's flags are visible as soon
//! as they are published and its motion only once it is resolved.

use core::num::NonZeroU32;
use core::ops::Range;

use super::{
    CWP_EQUAL, INTRABC_REF_FRAME, MotionMode, Mv, SWITCHABLE_FILTERS, TIP_REF_FRAME, warp_sub_mv_at,
};
use crate::prediction::{TileGridConstructionError, tile_grid_dimensions};
use crate::tile::SbRowWindow;

/// Syntax facts read by neighbour context derivation during symbol decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourFlags {
    bits: u8,
    pub(super) ref_frame0: i8,
    pub(super) ref_frame1: Option<i8>,
    pub(super) interp_filter: u8,
    pub(super) motion_mode: MotionMode,
    pub(super) precision: BlockPrecisionRecord,
}

impl NeighbourFlags {
    const IS_INTER: u8 = 1 << 0;
    const NEWMV_LIST0: u8 = 1 << 1;
    const NEWMV_LIST1: u8 = 1 << 2;
    const SKIP_MODE: u8 = 1 << 3;
    const SKIP: u8 = 1 << 4;
    const USE_AMVD: u8 = 1 << 5;
    const MASKED_COMPOUND: u8 = 1 << 6;
    const TIP_SIZE_16X16: u8 = 1 << 7;

    const fn flag(enabled: bool, mask: u8) -> u8 {
        if enabled { mask } else { 0 }
    }

    pub(super) const fn is_inter(self) -> bool {
        self.bits & Self::IS_INTER != 0
    }

    pub(super) const fn newmv_for_list0(self) -> bool {
        self.bits & Self::NEWMV_LIST0 != 0
    }

    pub(super) const fn newmv_for_list1(self) -> bool {
        self.bits & Self::NEWMV_LIST1 != 0
    }

    pub(super) const fn skip_mode(self) -> bool {
        self.bits & Self::SKIP_MODE != 0
    }

    pub(super) const fn skip(self) -> bool {
        self.bits & Self::SKIP != 0
    }

    pub(super) const fn use_amvd(self) -> bool {
        self.bits & Self::USE_AMVD != 0
    }

    pub(super) const fn masked_compound(self) -> bool {
        self.bits & Self::MASKED_COMPOUND != 0
    }

    pub(super) const fn tip_size_16x16(self) -> bool {
        self.bits & Self::TIP_SIZE_16X16 != 0
    }

    pub(super) const fn is_warp(self) -> bool {
        self.motion_mode.is_warp()
    }
}

pub(super) const EMPTY_NEIGHBOUR_FLAGS: NeighbourFlags = NeighbourFlags {
    bits: 0,
    ref_frame0: -1,
    ref_frame1: None,
    interp_filter: SWITCHABLE_FILTERS,
    motion_mode: MotionMode::Simple,
    precision: BlockPrecisionRecord {
        use_most_probable_precision: false,
        mv_precision: 0,
    },
};

/// Motion payload read by AV2 § 7.12 stack, bank and warp-sample derivation.
///
/// This is the read-side value only: the grid stores it once per leaf, see
/// [`LeafRecord`], and derives the per-cell sub-MVs when a cell is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourMotion {
    pub(super) mv: Mv,
    pub(super) mv1: Mv,
    pub(super) sub_mv: Mv,
    pub(super) sub_mv1: Mv,
    model: NeighbourMotionModel,
    pub(super) cwp_weight: i16,
    pub(super) base_r: u32,
    pub(super) base_c: u32,
    pub(super) bw4: u8,
    pub(super) bh4: u8,
}

impl NeighbourMotion {
    pub(super) const fn warp_params(self) -> Option<[i32; 6]> {
        match self.model {
            NeighbourMotionModel::Warp(params) => Some(params),
            NeighbourMotionModel::None | NeighbourMotionModel::Global(_) => None,
        }
    }

    pub(super) const fn is_global_mv(self, list: usize) -> bool {
        matches!(self.model, NeighbourMotionModel::Global(lists) if list < 2 && lists & (1 << list) != 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NeighbourMotionModel {
    None,
    Warp([i32; 6]),
    Global(u8),
}

/// One published leaf: its flags, and its motion once § 7.12 resolves it.
///
/// Publication writes one plane cell per mode-info position and reads perhaps
/// ten positions per leaf, so everything constant over the leaf lives here and
/// the plane cell is only the leaf's name.
#[derive(Clone, Copy)]
struct LeafRecord {
    flags: NeighbourFlags,
    mv: Mv,
    mv1: Mv,
    base_r: u32,
    base_c: u32,
    /// Index of the leaf's first warp model in the grid's model table.
    models: u32,
    cwp_weight: i16,
    bw4: u8,
    bh4: u8,
    global_mv_lists: u8,
    /// Which warp models the leaf has, in model-table order.
    model_bits: u8,
    resolved: bool,
}

impl LeafRecord {
    /// List-0 § 7.12.2.2 sub-MV splat model.
    const SPLAT0: u8 = 1 << 0;
    /// List-1 § 7.12.2.2 sub-MV splat model.
    const SPLAT1: u8 = 1 << 1;
    /// Neighbour-facing model, the same as the list-0 splat model.
    const STORED_IS_SPLAT0: u8 = 1 << 2;
    /// Neighbour-facing model held in its own table entry.
    const STORED_OWN: u8 = 1 << 3;
}

/// Footprint guard: a plane cell is four bytes and a leaf record forty-four.
const _: () = {
    assert!(size_of::<Option<NonZeroU32>>() == 4);
    assert!(size_of::<LeafRecord>() == 44);
    assert!(size_of::<NeighbourMotion>() == 72);
};

/// Both halves of one occupied grid position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourCell {
    pub(super) flags: NeighbourFlags,
    pub(super) motion: NeighbourMotion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockPrecisionRecord {
    pub(crate) use_most_probable_precision: bool,
    pub(crate) mv_precision: u8,
}

impl BlockPrecisionRecord {
    pub(crate) const fn most_probable(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: true,
            mv_precision,
        }
    }

    pub(crate) const fn explicit(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: false,
            mv_precision,
        }
    }
}

impl Default for BlockPrecisionRecord {
    fn default() -> Self {
        Self::most_probable(super::super::read_mv::MV_PRECISION_EIGHTH_PEL)
    }
}

/// Flag-plane inputs for one leaf, all of them syntax the entropy pass reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NeighbourFlagSyntax {
    pub(crate) is_inter: bool,
    pub(crate) ref_frame0: i8,
    pub(crate) ref_frame1: Option<i8>,
    pub(crate) newmv: [bool; 2],
    pub(crate) skip: bool,
    pub(crate) skip_mode: bool,
    pub(crate) use_amvd: bool,
    pub(crate) masked_compound: bool,
    pub(crate) tip_size_16x16: bool,
    pub(crate) interp_filter: u8,
    pub(crate) motion_mode: MotionMode,
    pub(crate) precision: BlockPrecisionRecord,
}

/// AV2 § 5.20.7.14 compound motion mode implied by the local-warp syntax.
pub(crate) const fn compound_motion_mode(local_warp: bool) -> MotionMode {
    if local_warp {
        MotionMode::LocalWarp
    } else {
        MotionMode::Simple
    }
}

/// § 5.20.7 flag record of a leaf that carries no motion of its own — intra
/// and intra block copy. Call sites fill in `is_inter`, `skip`,
/// `interp_filter` and `precision`; the rest of the record is fixed.
pub(crate) const NON_INTER_FLAG_SYNTAX: NeighbourFlagSyntax = NeighbourFlagSyntax {
    is_inter: false,
    ref_frame0: -1,
    ref_frame1: None,
    newmv: [false, false],
    skip: false,
    skip_mode: false,
    use_amvd: false,
    masked_compound: false,
    tip_size_16x16: false,
    interp_filter: SWITCHABLE_FILTERS,
    motion_mode: MotionMode::Simple,
    precision: BlockPrecisionRecord {
        use_most_probable_precision: false,
        mv_precision: 0,
    },
};

/// Motion-plane inputs for one leaf, all of them AV2 § 7.12 resolution output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NeighbourMotionValues {
    pub(crate) mv: [Mv; 2],
    pub(crate) cwp_weight: i16,
    /// Model the neighbour-facing derivations read (AVM `wm_params[0]`).
    pub(crate) stored_warp: Option<[i32; 6]>,
    /// Per-list GLOBALMV facts; the model is frame state, not leaf state.
    pub(crate) global_mv: [bool; 2],
    /// Per-list models driving the § 7.12.2.2 sub-MV splat.
    pub(crate) splat_warp: [Option<[i32; 6]>; 2],
}

/// § 7.12 motion record of a leaf that carries no motion of its own, which
/// the resolve pass publishes in the leaf's turn.
pub(crate) const ZERO_NEIGHBOUR_MOTION_VALUES: NeighbourMotionValues = NeighbourMotionValues {
    mv: [Mv::ZERO; 2],
    cwp_weight: CWP_EQUAL,
    stored_warp: None,
    global_mv: [false, false],
    splat_warp: [None, None],
};

/// Backing storage of the grid plane and its tables, recycled as one unit.
#[derive(Default)]
pub(super) struct GridPlanes {
    /// Index+1 of the leaf covering each cell, `None` where none is published.
    cells: Vec<Option<NonZeroU32>>,
    leaves: Vec<LeafRecord>,
    /// Warp models of the resolved leaves that have any.
    models: Vec<[i32; 6]>,
}

/// Sizes one tile's grid plane and empties its tables.
fn reset_grid_planes(
    planes: &mut GridPlanes,
    cells: usize,
    tile_cells: usize,
) -> Result<(), std::collections::TryReserveError> {
    // A grid the split path builds per frame starts empty, so its storage comes
    // from the spare set a retired grid left rather than up a growth ladder.
    take_spare_plane(&mut planes.cells, cells);
    take_spare_plane(&mut planes.leaves, tile_cells);
    take_spare_plane(&mut planes.models, tile_cells);
    planes.cells.clear();
    planes.leaves.clear();
    planes.models.clear();
    planes.cells.try_reserve_exact(cells)?;
    planes.cells.resize(cells, None);
    Ok(())
}

fn take_spare_plane<T: Send + 'static>(plane: &mut Vec<T>, cells: usize) {
    if plane.capacity() == 0 {
        *plane = crate::support::reusable_scratch::take_pooled_vec(cells);
    }
}

/// Returns a retired grid's plane and tables to the per-thread spare set.
///
/// The flag log is not among them: nothing takes a log back, so parking one
/// would hold a spare slot against the storage that is taken back.
impl Drop for NeighbourMvGrid {
    fn drop(&mut self) {
        use crate::support::reusable_scratch::recycle_pooled_vec;
        recycle_pooled_vec(core::mem::take(&mut self.planes.cells));
        recycle_pooled_vec(core::mem::take(&mut self.planes.leaves));
        recycle_pooled_vec(core::mem::take(&mut self.planes.models));
    }
}

/// One leaf's flag-plane publication, replayable onto a second grid.
///
/// A parse pass that hands its units to a resolve pass running later, or
/// elsewhere, logs what it published so the resolve pass can rebuild the same
/// flag plane on its own grid instead of sharing the parser's. The record is
/// exactly [`NeighbourMvGrid::record_flags`]'s arguments, so a replay is that
/// call again and cannot drift from it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NeighbourFlagRecord {
    r: u32,
    c: u32,
    n4w: u32,
    n4h: u32,
    syntax: NeighbourFlagSyntax,
}

#[derive(Default)]
pub(crate) struct NeighbourMvGrid {
    pub(super) origin_row: usize,
    pub(super) origin_col: usize,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    window: SbRowWindow,
    pub(super) planes: GridPlanes,
    /// Flag publications since the last [`NeighbourMvGrid::take_flag_log`],
    /// collected only while logging is on.
    flag_log: Vec<NeighbourFlagRecord>,
    logging: bool,
}

impl NeighbourMvGrid {
    #[cfg(test)]
    pub(crate) fn new_for_tile(
        mi_rows: core::ops::Range<usize>,
        mi_cols: core::ops::Range<usize>,
    ) -> Result<Self, TileGridConstructionError> {
        let mut grid = Self::default();
        grid.reset_for_tile(mi_rows, mi_cols, SbRowWindow::WHOLE_TILE_SB_H4)?;
        Ok(grid)
    }

    /// Lays this grid out for another tile, keeping the plane storage.
    ///
    /// The decoder holds one grid for the whole stream, so a steady-state tile
    /// reuses the planes the last one left instead of sizing new ones.
    pub(crate) fn reset_for_tile(
        &mut self,
        mi_rows: core::ops::Range<usize>,
        mi_cols: core::ops::Range<usize>,
        sb_h4: usize,
    ) -> Result<(), TileGridConstructionError> {
        let (rows, cols, tile_cells) = tile_grid_dimensions(&mi_rows, &mi_cols)?;
        self.origin_row = mi_rows.start;
        self.origin_col = mi_cols.start;
        self.mi_rows = rows;
        self.mi_cols = cols;
        self.window = SbRowWindow::new(rows, sb_h4);
        reset_grid_planes(
            &mut self.planes,
            self.window.plane_rows() * cols,
            tile_cells,
        )
        .map_err(|_| TileGridConstructionError::Allocation)?;
        self.flag_log.clear();
        self.logging = false;
        Ok(())
    }

    /// Starts logging flag publications for later replay onto another grid.
    pub(crate) const fn log_flags(&mut self) {
        self.logging = true;
    }

    /// Moves the flag publications made since the last call into `into`,
    /// handing `into`'s emptied storage back to the log.
    pub(crate) fn take_flag_log(&mut self, into: &mut Vec<NeighbourFlagRecord>) {
        into.clear();
        core::mem::swap(&mut self.flag_log, into);
    }

    /// Replays one unit's logged flag publications onto this grid.
    pub(crate) fn replay_flag_log(&mut self, records: &[NeighbourFlagRecord]) {
        for record in records {
            self.record_flags(
                record.r as usize,
                record.c as usize,
                record.n4w as usize,
                record.n4h as usize,
                record.syntax,
            );
        }
    }

    /// Publishes the flag plane for one leaf. The entropy pass calls this as
    /// soon as the leaf's syntax is parsed, before any § 7.12 resolution.
    pub(crate) fn record_flags(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        syntax: NeighbourFlagSyntax,
    ) {
        if self.logging {
            self.flag_log.push(NeighbourFlagRecord {
                r: r as u32,
                c: c as u32,
                n4w: n4w as u32,
                n4h: n4h as u32,
                syntax,
            });
        }
        let Some((rows, cols)) = self.footprint(r, c, n4w, n4h) else {
            return;
        };
        self.enter_sb_row(rows.start);
        let Some(slot) = u32::try_from(self.planes.leaves.len())
            .ok()
            .and_then(|leaf| NonZeroU32::new(leaf.wrapping_add(1)))
        else {
            return;
        };
        let flags = NeighbourFlags {
            bits: NeighbourFlags::flag(syntax.is_inter, NeighbourFlags::IS_INTER)
                | NeighbourFlags::flag(syntax.newmv[0], NeighbourFlags::NEWMV_LIST0)
                | NeighbourFlags::flag(syntax.newmv[1], NeighbourFlags::NEWMV_LIST1)
                | NeighbourFlags::flag(syntax.skip_mode, NeighbourFlags::SKIP_MODE)
                | NeighbourFlags::flag(syntax.skip, NeighbourFlags::SKIP)
                | NeighbourFlags::flag(syntax.use_amvd, NeighbourFlags::USE_AMVD)
                | NeighbourFlags::flag(syntax.masked_compound, NeighbourFlags::MASKED_COMPOUND)
                | NeighbourFlags::flag(syntax.tip_size_16x16, NeighbourFlags::TIP_SIZE_16X16),
            ref_frame0: syntax.ref_frame0,
            ref_frame1: syntax.ref_frame1,
            interp_filter: syntax.interp_filter.min(SWITCHABLE_FILTERS),
            motion_mode: syntax.motion_mode,
            precision: syntax.precision,
        };
        self.planes.leaves.push(LeafRecord {
            flags,
            mv: Mv::ZERO,
            mv1: Mv::ZERO,
            base_r: r as u32,
            base_c: c as u32,
            models: 0,
            cwp_weight: CWP_EQUAL,
            bw4: n4w as u8,
            bh4: n4h as u8,
            global_mv_lists: 0,
            model_bits: 0,
            resolved: false,
        });
        for rr in rows {
            let Some(span) = self.row_span(rr, &cols) else {
                continue;
            };
            if let Some(cells) = self.planes.cells.get_mut(span) {
                cells.fill(Some(slot));
            }
        }
    }

    /// Resolves the motion of one leaf whose flags this grid already holds,
    /// once § 7.12 resolution has produced its motion vectors and warp models.
    ///
    /// The leaf is found through its own first cell. A cell that names another
    /// leaf is a publication-order defect, and the leaf then stays unresolved,
    /// which every reader treats as unpublished.
    pub(crate) fn record_motion(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        values: NeighbourMotionValues,
    ) {
        let Some((rows, cols)) = self.footprint(r, c, n4w, n4h) else {
            return;
        };
        let Some(leaf) = self
            .row_span(rows.start, &cols)
            .and_then(|span| *self.planes.cells.get(span.start)?)
            .map(|slot| slot.get() as usize - 1)
        else {
            return;
        };
        let Some(record) = self.planes.leaves.get(leaf) else {
            return;
        };
        if (record.base_r, record.base_c, record.bw4, record.bh4)
            != (r as u32, c as u32, n4w as u8, n4h as u8)
        {
            return;
        }
        let Ok(models) = u32::try_from(self.planes.models.len()) else {
            return;
        };
        let mut model_bits = 0;
        let [splat0, splat1] = values.splat_warp;
        for (bit, model) in [(LeafRecord::SPLAT0, splat0), (LeafRecord::SPLAT1, splat1)] {
            if let Some(params) = model {
                self.planes.models.push(params);
                model_bits |= bit;
            }
        }
        match values.stored_warp {
            Some(params) if splat0 == Some(params) => model_bits |= LeafRecord::STORED_IS_SPLAT0,
            Some(params) => {
                self.planes.models.push(params);
                model_bits |= LeafRecord::STORED_OWN;
            }
            None => {}
        }
        let Some(record) = self.planes.leaves.get_mut(leaf) else {
            return;
        };
        record.mv = values.mv[0];
        record.mv1 = values.mv[1];
        record.models = models;
        record.model_bits = model_bits;
        record.cwp_weight = values.cwp_weight;
        record.global_mv_lists =
            u8::from(values.global_mv[0]) | (u8::from(values.global_mv[1]) << 1);
        record.resolved = true;
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_inter: bool,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        newmv: bool,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            is_inter,
            ref_frame0,
            ref_frame1,
            newmv,
            mv,
            skip,
            interp_filter,
            use_amvd,
            MotionMode::Simple,
            None,
            false,
            precision,
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_warp_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        ref_frame0: i8,
        newmv: bool,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        motion_mode: MotionMode,
        warp_params: [i32; 6],
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            true,
            ref_frame0,
            None,
            newmv,
            mv,
            skip,
            interp_filter,
            use_amvd,
            motion_mode,
            Some(warp_params),
            false,
            precision,
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn record_block_with_warp(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_inter: bool,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        newmv: bool,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        motion_mode: MotionMode,
        warp_params: Option<[i32; 6]>,
        tip_size_16x16: bool,
        precision: BlockPrecisionRecord,
    ) {
        self.record_flags(
            r,
            c,
            n4w,
            n4h,
            NeighbourFlagSyntax {
                is_inter,
                ref_frame0,
                ref_frame1,
                newmv: [newmv, false],
                skip,
                skip_mode: false,
                use_amvd,
                masked_compound: false,
                tip_size_16x16,
                interp_filter,
                motion_mode,
                precision,
            },
        );
        self.record_motion(
            r,
            c,
            n4w,
            n4h,
            NeighbourMotionValues {
                mv: [mv, Mv::ZERO],
                cwp_weight: CWP_EQUAL,
                stored_warp: warp_params.filter(|_| motion_mode.is_warp()),
                global_mv: [warp_params.is_some() && !motion_mode.is_warp(), false],
                splat_warp: [warp_params.filter(|_| motion_mode.is_warp()), None],
            },
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_tip_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        newmv: bool,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        tip_size_16x16: bool,
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            true,
            TIP_REF_FRAME,
            None,
            newmv,
            mv,
            skip,
            interp_filter,
            use_amvd,
            MotionMode::Simple,
            None,
            tip_size_16x16,
            precision,
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_compound_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        ref_frame0: i8,
        ref_frame1: i8,
        list0_is_newmv: bool,
        list1_is_newmv: bool,
        mv0: Mv,
        mv1: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        masked_compound: bool,
        cwp_weight: i16,
        skip_mode: bool,
        precision: BlockPrecisionRecord,
        warp_params: [Option<[i32; 6]>; 2],
    ) {
        self.record_flags(
            r,
            c,
            n4w,
            n4h,
            NeighbourFlagSyntax {
                is_inter: true,
                ref_frame0,
                ref_frame1: Some(ref_frame1),
                newmv: [list0_is_newmv, list1_is_newmv],
                skip,
                skip_mode,
                use_amvd,
                masked_compound,
                tip_size_16x16: false,
                interp_filter,
                motion_mode: compound_motion_mode(
                    warp_params[0].is_some() || warp_params[1].is_some(),
                ),
                precision,
            },
        );
        self.record_motion(
            r,
            c,
            n4w,
            n4h,
            NeighbourMotionValues {
                mv: [mv0, mv1],
                cwp_weight,
                // Neighbour-facing warp derivations read only the first model (AVM `wm_params[0]`).
                stored_warp: warp_params[0],
                global_mv: [false, false],
                splat_warp: warp_params,
            },
        );
    }

    /// Moves the window down to the superblock row holding tile row `row`.
    fn enter_sb_row(&mut self, row: usize) {
        let Some(slide) = self.window.enter(row.saturating_sub(self.origin_row)) else {
            return;
        };
        slide.apply(&mut self.planes.cells, self.mi_cols, None);
    }

    /// Plane row of tile row `row`, `None` outside the readable window.
    fn plane_row(&self, row: usize) -> Option<usize> {
        self.window.plane_row(row.checked_sub(self.origin_row)?)
    }

    /// Whether an access touched a row the window had already reused.
    pub(crate) fn window_violated(&self) -> bool {
        self.window.violated()
    }

    /// Plane row and column ranges covered by one leaf, `None` when the leaf
    /// lies entirely outside this tile's grid.
    fn footprint(
        &self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        let row_end = self.origin_row.saturating_add(self.mi_rows);
        let col_end = self.origin_col.saturating_add(self.mi_cols);
        let rows = r.max(self.origin_row)..r.saturating_add(n4h).min(row_end);
        let cols = c.max(self.origin_col)..c.saturating_add(n4w).min(col_end);
        (!rows.is_empty() && !cols.is_empty()).then_some((rows, cols))
    }

    /// Plane index range covering `cols` on grid row `rr`.
    fn row_span(&self, rr: usize, cols: &Range<usize>) -> Option<Range<usize>> {
        let row_base = self.plane_row(rr)?.checked_mul(self.mi_cols)?;
        let start = row_base.checked_add(cols.start.checked_sub(self.origin_col)?)?;
        let end = row_base.checked_add(cols.end.checked_sub(self.origin_col)?)?;
        Some(start..end)
    }

    fn index(&self, r: i32, c: i32) -> Option<usize> {
        if r < 0 || c < 0 {
            return None;
        }
        let r = self.plane_row(r as usize)?;
        let c = (c as usize).checked_sub(self.origin_col)?;
        if c >= self.mi_cols {
            return None;
        }
        r.checked_mul(self.mi_cols)?.checked_add(c)
    }

    /// § 5.20.9.1 is_inside: whether the mi position lies inside this tile.
    pub(super) fn is_inside(&self, r: usize, c: usize) -> bool {
        r >= self.origin_row
            && r < self.origin_row.saturating_add(self.mi_rows)
            && c >= self.origin_col
            && c < self.origin_col.saturating_add(self.mi_cols)
    }

    fn leaf_at(&self, r: i32, c: i32) -> Option<&LeafRecord> {
        let slot = (*self.planes.cells.get(self.index(r, c)?)?)?;
        self.planes.leaves.get(slot.get() as usize - 1)
    }

    /// Reads the flag half only, published or not resolved yet.
    pub(super) fn flags_at(&self, r: i32, c: i32) -> Option<NeighbourFlags> {
        self.leaf_at(r, c).map(|leaf| leaf.flags)
    }

    /// Reads both halves, and only where the leaf has been resolved: a leaf
    /// whose flags are already visible but whose § 7.12 resolution has not run
    /// is not a candidate. That is what keeps the decode-order candidates (the
    /// § 7.12 bottom-left probe above all) out of the stack once the flags run
    /// ahead of resolution.
    pub(super) fn get(&self, r: i32, c: i32) -> Option<NeighbourCell> {
        let leaf = self.leaf_at(r, c)?;
        if !leaf.resolved {
            return None;
        }
        let global = if leaf.global_mv_lists == 0 {
            NeighbourMotionModel::None
        } else {
            NeighbourMotionModel::Global(leaf.global_mv_lists)
        };
        let (model, sub_mv, sub_mv1) = if leaf.model_bits == 0 {
            (global, leaf.mv, leaf.mv1)
        } else {
            let mut models = self.planes.models.get(leaf.models as usize..);
            let mut take = |bit: u8| {
                let (first, rest) = models
                    .filter(|_| leaf.model_bits & bit != 0)?
                    .split_first()?;
                models = Some(rest);
                Some(*first)
            };
            let splat0 = take(LeafRecord::SPLAT0);
            let splat1 = take(LeafRecord::SPLAT1);
            let stored = if leaf.model_bits & LeafRecord::STORED_IS_SPLAT0 != 0 {
                splat0
            } else {
                take(LeafRecord::STORED_OWN)
            };
            let at = |params| {
                let base = (leaf.base_r as usize, leaf.base_c as usize);
                warp_sub_mv_at(params, base.0, base.1, r as usize, c as usize)
            };
            (
                stored.map_or(global, NeighbourMotionModel::Warp),
                splat0.map_or(leaf.mv, at),
                splat1.map_or(leaf.mv1, at),
            )
        };
        Some(NeighbourCell {
            flags: leaf.flags,
            motion: NeighbourMotion {
                mv: leaf.mv,
                mv1: leaf.mv1,
                sub_mv,
                sub_mv1,
                model,
                cwp_weight: leaf.cwp_weight,
                base_r: leaf.base_r,
                base_c: leaf.base_c,
                bw4: leaf.bw4,
                bh4: leaf.bh4,
            },
        })
    }

    pub(crate) fn intrabc_mv_at(&self, r: usize, c: usize) -> Option<Mv> {
        let cell = self.get(i32::try_from(r).ok()?, i32::try_from(c).ok()?)?;
        (cell.flags.ref_frame0 == INTRABC_REF_FRAME && cell.flags.ref_frame1.is_none())
            .then_some(cell.motion.mv)
    }

    pub(crate) fn is_non_tip_at(&self, r: i32, c: i32) -> bool {
        matches!(self.flags_at(r, c), Some(flags) if flags.ref_frame0 != TIP_REF_FRAME)
    }
}

#[cfg(test)]
#[path = "neighbour_grid_tests.rs"]
mod tests;
