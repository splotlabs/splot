// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Neighbour mode-info grid.
//!
//! The grid keeps two planes over one tile-sized mode-info lattice. The flag
//! plane holds the syntax facts that neighbour context derivation reads while
//! symbols are decoded; the motion plane holds the AV2 § 7.12 motion payload
//! that the reference MV stack and the warp derivations read. Each plane
//! carries its own occupancy — flags for context derivation, a named leaf for
//! motion — so the two may be published at different times, and context
//! derivation never touches motion memory.

use core::ops::Range;

use super::mv_grid_pool::take_neighbour_mv_planes;
use super::{
    CWP_EQUAL, INTRABC_REF_FRAME, MotionMode, Mv, SWITCHABLE_FILTERS, TIP_REF_FRAME, warp_sub_mv_at,
};
use crate::prediction::{TileGridConstructionError, tile_grid_dimensions};

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
/// This is the read-side value only. The plane stores it split in two, because
/// every field but the sub-MVs is constant over a leaf: see [`MotionCell`].
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

/// The motion a leaf publishes into every cell it covers alike.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LeafMotion {
    mv: Mv,
    mv1: Mv,
    model: NeighbourMotionModel,
    cwp_weight: i16,
    base_r: u32,
    base_c: u32,
    bw4: u8,
    bh4: u8,
}

/// One motion-plane cell: the leaf that published it and the § 7.12.2.2 sub-MVs,
/// which are the only motion that varies from cell to cell inside one leaf.
///
/// Publication writes one cell per mode-info position and reads perhaps ten
/// positions per leaf, so the plane is a write surface: keeping the per-leaf
/// constants out of it is what bounds the bytes a frame's publication touches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MotionCell {
    /// Index into the grid's leaf table; out of range on an unpublished cell.
    leaf: u32,
    sub_mv: Mv,
    sub_mv1: Mv,
}

/// Names no leaf, so [`NeighbourMvGrid::get`] reads the cell as unpublished.
const UNPUBLISHED_LEAF: u32 = u32::MAX;

const EMPTY_MOTION_CELL: MotionCell = MotionCell {
    leaf: UNPUBLISHED_LEAF,
    sub_mv: Mv::ZERO,
    sub_mv1: Mv::ZERO,
};

/// Plane footprint guard: neighbour context derivation reads eight bytes per
/// grid cell and § 7.12 resolution writes twenty, not the full record.
const _: () = {
    assert!(size_of::<Option<NeighbourFlags>>() == 8);
    assert!(size_of::<MotionCell>() == 20);
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

/// Backing storage of both grid planes, recycled as one unit.
#[derive(Default)]
pub(super) struct GridPlanes {
    pub(super) flags: Vec<Option<NeighbourFlags>>,
    pub(super) motion: Vec<MotionCell>,
    pub(super) leaves: Vec<LeafMotion>,
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

pub(crate) struct NeighbourMvGrid {
    pub(super) origin_row: usize,
    pub(super) origin_col: usize,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    pub(super) planes: GridPlanes,
    /// Flag publications since the last [`NeighbourMvGrid::take_flag_log`],
    /// collected only while logging is on.
    flag_log: Vec<NeighbourFlagRecord>,
    logging: bool,
}

impl NeighbourMvGrid {
    pub(crate) fn new_for_tile(
        mi_rows: core::ops::Range<usize>,
        mi_cols: core::ops::Range<usize>,
    ) -> Result<Self, TileGridConstructionError> {
        let (rows, cols, cells) = tile_grid_dimensions(&mi_rows, &mi_cols)?;
        Ok(Self {
            origin_row: mi_rows.start,
            origin_col: mi_cols.start,
            mi_rows: rows,
            mi_cols: cols,
            planes: take_neighbour_mv_planes(cells)
                .map_err(|_| TileGridConstructionError::Allocation)?,
            flag_log: Vec::new(),
            logging: false,
        })
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
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::ModeRecord);
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
        self.publish_flags(flags, rows, cols);
    }

    /// Publishes the motion plane for one leaf, once § 7.12 resolution has
    /// produced its motion vectors and warp models.
    pub(crate) fn record_motion(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        values: NeighbourMotionValues,
    ) {
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::ModeRecord);
        let Some((rows, cols)) = self.footprint(r, c, n4w, n4h) else {
            return;
        };
        self.motion_plane();
        let leaf = u32::try_from(self.planes.leaves.len()).unwrap_or(UNPUBLISHED_LEAF);
        if leaf == UNPUBLISHED_LEAF {
            return;
        }
        let global_mv_lists = u8::from(values.global_mv[0]) | (u8::from(values.global_mv[1]) << 1);
        let model = values.stored_warp.map_or_else(
            || {
                if global_mv_lists == 0 {
                    NeighbourMotionModel::None
                } else {
                    NeighbourMotionModel::Global(global_mv_lists)
                }
            },
            NeighbourMotionModel::Warp,
        );
        self.planes.leaves.push(LeafMotion {
            mv: values.mv[0],
            mv1: values.mv[1],
            model,
            cwp_weight: values.cwp_weight,
            base_r: r as u32,
            base_c: c as u32,
            bw4: n4w as u8,
            bh4: n4h as u8,
        });
        let cell = MotionCell {
            leaf,
            sub_mv: values.mv[0],
            sub_mv1: values.mv[1],
        };
        self.publish_motion(cell, (r, c), rows, cols, values.splat_warp);
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
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::ModeRecord);
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
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::ModeRecord);
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
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::ModeRecord);
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

    /// Sizes the motion plane on the first publication.
    ///
    /// The split path parses on one grid and resolves on another, so the parse
    /// grid's motion plane is never published and never read. Sizing it lazily
    /// keeps the fill on the grid that uses it while leaving the pooled
    /// allocation intact for the next grid that does. An unsized plane reads as
    /// unpublished everywhere, which is what [`NeighbourMvGrid::get`] owes a
    /// plane no leaf has published into.
    fn motion_plane(&mut self) {
        if self.planes.motion.is_empty() {
            let cells = self.mi_rows.saturating_mul(self.mi_cols);
            self.planes.motion.resize(cells, EMPTY_MOTION_CELL);
        }
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

    fn publish_flags(&mut self, flags: NeighbourFlags, rows: Range<usize>, cols: Range<usize>) {
        for rr in rows {
            let Some(span) = self.row_span(rr, &cols) else {
                continue;
            };
            if let Some(slots) = self.planes.flags.get_mut(span) {
                slots.fill(Some(flags));
            }
        }
    }

    fn publish_motion(
        &mut self,
        cell: MotionCell,
        base: (usize, usize),
        rows: Range<usize>,
        cols: Range<usize>,
        warp_params: [Option<[i32; 6]>; 2],
    ) {
        for rr in rows {
            let Some(span) = self.row_span(rr, &cols) else {
                continue;
            };
            let Some(slots) = self.planes.motion.get_mut(span) else {
                continue;
            };
            if warp_params[0].is_none() && warp_params[1].is_none() {
                slots.fill(cell);
                continue;
            }
            for (slot, cc) in slots.iter_mut().zip(cols.clone()) {
                *slot = cell;
                if let Some(params) = warp_params[0] {
                    slot.sub_mv = warp_sub_mv_at(params, base.0, base.1, rr, cc);
                }
                if let Some(params) = warp_params[1] {
                    slot.sub_mv1 = warp_sub_mv_at(params, base.0, base.1, rr, cc);
                }
            }
        }
    }

    /// Plane index range covering `cols` on grid row `rr`.
    fn row_span(&self, rr: usize, cols: &Range<usize>) -> Option<Range<usize>> {
        let row_base = rr.checked_sub(self.origin_row)?.checked_mul(self.mi_cols)?;
        let start = row_base.checked_add(cols.start.checked_sub(self.origin_col)?)?;
        let end = row_base.checked_add(cols.end.checked_sub(self.origin_col)?)?;
        Some(start..end)
    }

    fn index(&self, r: i32, c: i32) -> Option<usize> {
        if r < 0 || c < 0 {
            return None;
        }
        let r = (r as usize).checked_sub(self.origin_row)?;
        let c = (c as usize).checked_sub(self.origin_col)?;
        if r >= self.mi_rows || c >= self.mi_cols {
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

    /// Reads the flag half only; motion memory is not touched.
    pub(super) fn flags_at(&self, r: i32, c: i32) -> Option<NeighbourFlags> {
        *self.planes.flags.get(self.index(r, c)?)?
    }

    /// Reads both halves, and only where the motion half has been published:
    /// a cell names no leaf exactly while no leaf has resolved it, so a leaf
    /// whose flags are already visible but whose § 7.12 resolution has not run
    /// is not a candidate. That is what keeps the decode-order candidates (the
    /// § 7.12 bottom-left probe above all) out of the stack once the flag plane
    /// runs ahead of resolution.
    pub(super) fn get(&self, r: i32, c: i32) -> Option<NeighbourCell> {
        let index = self.index(r, c)?;
        let flags = (*self.planes.flags.get(index)?)?;
        let cell = *self.planes.motion.get(index)?;
        let leaf = self.planes.leaves.get(cell.leaf as usize)?;
        Some(NeighbourCell {
            flags,
            motion: NeighbourMotion {
                mv: leaf.mv,
                mv1: leaf.mv1,
                sub_mv: cell.sub_mv,
                sub_mv1: cell.sub_mv1,
                model: leaf.model,
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
