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
use crate::filters::wienerns_lr::intrabc_ref_mv_stack::{
    SpatialIntrabcProbes, SpatialScanGeometry, capture_spatial_intrabc_probes,
};
use crate::prediction::{TileGridConstructionError, tile_grid_dimensions};
use crate::tile::SbRowWindow;

/// Syntax facts read by neighbour context derivation during symbol decode,
/// one byte per field: the flag bits, `ref_frame0`, `ref_frame1`
/// ([`Self::NO_REF`] when absent), `interp_filter`, `motion_mode`, the two
/// halves of the [`BlockPrecisionRecord`], and whether a leaf was published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourFlags(u64);

impl NeighbourFlags {
    const IS_INTER: u8 = 1 << 0;
    const NEWMV_LIST0: u8 = 1 << 1;
    const NEWMV_LIST1: u8 = 1 << 2;
    const SKIP_MODE: u8 = 1 << 3;
    const SKIP: u8 = 1 << 4;
    const USE_AMVD: u8 = 1 << 5;
    const MASKED_COMPOUND: u8 = 1 << 6;
    const TIP_SIZE_16X16: u8 = 1 << 7;
    const NO_REF: u8 = i8::MIN as u8;

    const fn flag(enabled: bool, mask: u8) -> u8 {
        if enabled { mask } else { 0 }
    }

    #[inline]
    fn from_syntax(syntax: NeighbourFlagSyntax) -> Self {
        Self(u64::from_le_bytes([
            Self::flag(syntax.is_inter, Self::IS_INTER)
                | Self::flag(syntax.newmv[0], Self::NEWMV_LIST0)
                | Self::flag(syntax.newmv[1], Self::NEWMV_LIST1)
                | Self::flag(syntax.skip_mode, Self::SKIP_MODE)
                | Self::flag(syntax.skip, Self::SKIP)
                | Self::flag(syntax.use_amvd, Self::USE_AMVD)
                | Self::flag(syntax.masked_compound, Self::MASKED_COMPOUND)
                | Self::flag(syntax.tip_size_16x16, Self::TIP_SIZE_16X16),
            syntax.ref_frame0 as u8,
            syntax
                .ref_frame1
                .map_or(Self::NO_REF, |ref_frame| ref_frame as u8),
            syntax.interp_filter.min(SWITCHABLE_FILTERS),
            syntax.motion_mode as u8,
            syntax.precision.use_most_probable_precision as u8,
            syntax.precision.mv_precision,
            1,
        ]))
    }

    const fn byte(self, index: u32) -> u8 {
        (self.0 >> (8 * index)) as u8
    }

    const fn bit(self, mask: u8) -> bool {
        self.byte(0) & mask != 0
    }

    pub(super) const fn is_inter(self) -> bool {
        self.bit(Self::IS_INTER)
    }

    pub(super) const fn newmv_for_list0(self) -> bool {
        self.bit(Self::NEWMV_LIST0)
    }

    pub(super) const fn newmv_for_list1(self) -> bool {
        self.bit(Self::NEWMV_LIST1)
    }

    pub(super) const fn skip_mode(self) -> bool {
        self.bit(Self::SKIP_MODE)
    }

    pub(super) const fn skip(self) -> bool {
        self.bit(Self::SKIP)
    }

    pub(super) const fn use_amvd(self) -> bool {
        self.bit(Self::USE_AMVD)
    }

    pub(super) const fn masked_compound(self) -> bool {
        self.bit(Self::MASKED_COMPOUND)
    }

    pub(super) const fn tip_size_16x16(self) -> bool {
        self.bit(Self::TIP_SIZE_16X16)
    }

    pub(super) const fn ref_frame0(self) -> i8 {
        self.byte(1) as i8
    }

    pub(super) const fn ref_frame1(self) -> Option<i8> {
        match self.byte(2) {
            Self::NO_REF => None,
            ref_frame => Some(ref_frame as i8),
        }
    }

    pub(super) const fn interp_filter(self) -> u8 {
        self.byte(3)
    }

    pub(super) const fn is_warp(self) -> bool {
        self.byte(4) >= MotionMode::LocalWarp as u8
    }

    pub(super) const fn is_local_warp(self) -> bool {
        self.byte(4) == MotionMode::LocalWarp as u8
    }

    pub(super) const fn use_most_probable_precision(self) -> bool {
        self.byte(5) != 0
    }

    pub(super) const fn mv_precision(self) -> u8 {
        self.byte(6)
    }

    pub(super) const fn is_present(self) -> bool {
        self.byte(7) != 0
    }
}

/// The flags of a position where no leaf is published: a non-inter leaf.
pub(super) const ABSENT_NEIGHBOUR_FLAGS: NeighbourFlags = NeighbourFlags(u64::from_le_bytes([
    0,
    -1i8 as u8,
    NeighbourFlags::NO_REF,
    SWITCHABLE_FILTERS,
    0,
    0,
    0,
    0,
]));

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

/// Footprint guard: a plane cell is four bytes and a leaf record forty-eight.
const _: () = {
    assert!(size_of::<Option<NonZeroU32>>() == 4);
    assert!(size_of::<LeafRecord>() == 48);
    assert!(size_of::<NeighbourMotion>() == 72);
};

/// Both halves of one occupied grid position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighbourCell {
    pub(super) flags: NeighbourFlags,
    pub(super) motion: NeighbourMotion,
}

/// A resolved leaf seen from one probed position, [`NeighbourMvGrid::probe`].
///
/// The warp-derived parts (sub-MVs of a splat model, the stored model) are
/// only computed when a reader asks for them.
#[derive(Clone, Copy)]
pub(super) struct ProbedLeaf<'g> {
    leaf: &'g LeafRecord,
    models: &'g [[i32; 6]],
    r: i32,
    c: i32,
}

impl ProbedLeaf<'_> {
    pub(super) const fn flags(&self) -> NeighbourFlags {
        self.leaf.flags
    }

    pub(super) const fn cwp_weight(&self) -> i16 {
        self.leaf.cwp_weight
    }

    fn model(&self, offset: usize) -> Option<[i32; 6]> {
        self.models.get(self.leaf.models as usize + offset).copied()
    }

    fn splat(&self, list: usize) -> Option<[i32; 6]> {
        let bits = self.leaf.model_bits;
        match list {
            0 if bits & LeafRecord::SPLAT0 != 0 => self.model(0),
            1 if bits & LeafRecord::SPLAT1 != 0 => {
                self.model(usize::from(bits & LeafRecord::SPLAT0 != 0))
            }
            _ => None,
        }
    }

    /// The neighbour-facing warp model, `NeighbourMotion::warp_params`.
    pub(super) fn stored_warp(&self) -> Option<[i32; 6]> {
        let bits = self.leaf.model_bits;
        if bits & LeafRecord::STORED_IS_SPLAT0 != 0 {
            self.splat(0)
        } else if bits & LeafRecord::STORED_OWN != 0 {
            self.model(
                usize::from(bits & LeafRecord::SPLAT0 != 0)
                    + usize::from(bits & LeafRecord::SPLAT1 != 0),
            )
        } else {
            None
        }
    }

    /// The list's § 7.12.2.2 sub-MV at the probed position.
    #[allow(clippy::inline_always, reason = "measured: every stack probe")]
    #[inline(always)]
    pub(super) fn sub_mv(&self, list: usize) -> Mv {
        if self.leaf.model_bits & (LeafRecord::SPLAT0 << list) == 0 {
            return if list == 0 {
                self.leaf.mv
            } else {
                self.leaf.mv1
            };
        }
        self.splat_sub_mv(list)
    }

    #[inline(never)]
    fn splat_sub_mv(&self, list: usize) -> Mv {
        let mv = if list == 0 {
            self.leaf.mv
        } else {
            self.leaf.mv1
        };
        self.splat(list).map_or(mv, |params| {
            warp_sub_mv_at(
                params,
                self.leaf.base_r as usize,
                self.leaf.base_c as usize,
                self.r as usize,
                self.c as usize,
            )
        })
    }

    #[inline]
    pub(super) fn sub_mvs(&self) -> [Mv; 2] {
        [self.sub_mv(0), self.sub_mv(1)]
    }

    /// Whether the list is a GLOBALMV list of a leaf without a stored model.
    #[inline]
    pub(super) fn is_global_mv(&self, list: usize) -> bool {
        self.leaf.global_mv_lists & (1 << list) != 0
            && (self.leaf.model_bits & (LeafRecord::STORED_IS_SPLAT0 | LeafRecord::STORED_OWN) == 0
                || self.stored_warp().is_none())
    }
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

/// The [`NeighbourMvGrid::record_flags`] result of a leaf the grid keeps no
/// record of.
pub(crate) const NO_LEAF: u32 = u32::MAX;

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

/// The cells of a grid rectangle and the leaf records and models they name,
/// renumbered from 1 and 0.
#[derive(Default)]
pub(crate) struct GridEdge {
    rows: Range<usize>,
    cols: Range<usize>,
    cells: Vec<Option<NonZeroU32>>,
    leaves: Vec<LeafRecord>,
    models: Vec<[i32; 6]>,
}

/// The flags at `at` of an edge line, absent unless `inside`.
fn edge(line: &[NeighbourFlags], inside: bool, at: usize) -> NeighbourFlags {
    line.get(at)
        .copied()
        .filter(|_| inside)
        .unwrap_or(ABSENT_NEIGHBOUR_FLAGS)
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
impl Drop for NeighbourMvGrid {
    fn drop(&mut self) {
        use crate::support::reusable_scratch::recycle_pooled_vec;
        recycle_pooled_vec(core::mem::take(&mut self.planes.cells));
        recycle_pooled_vec(core::mem::take(&mut self.planes.leaves));
        recycle_pooled_vec(core::mem::take(&mut self.planes.models));
    }
}

/// One published leaf: `(r, c, n4w, n4h)`, the references of an inter leaf,
/// and its resolved motion vectors.
pub(crate) type PublishedLeaf = (
    (usize, usize, usize, usize),
    Option<(i8, Option<i8>)>,
    [Mv; 2],
    [Option<[i32; 6]>; 2],
    [bool; 2],
);

/// A [`NeighbourMvGrid::checkpoint`].
pub(crate) struct GridCheckpoint {
    leaves: usize,
    models: usize,
    /// Leaf and model records appended after `leaves` and `models`, which
    /// a rollback puts back: a later rollback to an older checkpoint drops them.
    appended: (Vec<LeafRecord>, Vec<[i32; 6]>),
    footprint: Option<(Range<usize>, Range<usize>)>,
    cells: Vec<Option<NonZeroU32>>,
}

#[derive(Default)]
pub(crate) struct NeighbourMvGrid {
    pub(super) origin_row: usize,
    pub(super) origin_col: usize,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    window: SbRowWindow,
    /// [`SbRowWindow::readable_rows`] of `window`, in tile rows.
    readable: (usize, usize, usize),
    /// First grid row past the window's superblock row, zero before the first.
    sb_row_end: usize,
    pub(super) planes: GridPlanes,
    /// Whether only the edge lines are kept: a resolve pass rebuilds the
    /// plane on its own grid from the parsed leaves.
    edges_only: bool,
    /// With [`Self::track_edges`], the last leaf on each tile column and row
    /// (see `TileMiSizeState`), and `above` as the superblock row began.
    above: Vec<NeighbourFlags>,
    sb_above: Vec<NeighbourFlags>,
    left: Vec<NeighbourFlags>,
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
        self.readable = self.window.readable_rows();
        self.sb_row_end = 0;
        reset_grid_planes(
            &mut self.planes,
            self.window.plane_rows() * cols,
            tile_cells,
        )
        .map_err(|_| TileGridConstructionError::Allocation)?;
        self.edges_only = false;
        self.above.clear();
        Ok(())
    }

    /// Publishes only the edge lines from now on.
    pub(crate) const fn keep_edges_only(&mut self) {
        self.edges_only = true;
    }

    /// Keeps the edge lines that the parse's probes read.
    pub(crate) fn track_edges(&mut self) -> Result<(), TileGridConstructionError> {
        for (line, len) in [
            (&mut self.above, self.mi_cols),
            (&mut self.sb_above, self.mi_cols),
            (&mut self.left, self.mi_rows),
        ] {
            line.clear();
            line.try_reserve_exact(len)
                .map_err(|_| TileGridConstructionError::Allocation)?;
            line.resize(len, ABSENT_NEIGHBOUR_FLAGS);
        }
        Ok(())
    }

    /// The flags at the four § 7.12.2.6 immediate probes of a block: the
    /// left-bottom, above-right, left-top and above-left.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-block neighbour contexts"
    )]
    #[inline(always)]
    pub(super) fn immediate_flags(
        &self,
        r: usize,
        c: usize,
        bw4: usize,
        bh4: usize,
    ) -> [NeighbourFlags; 4] {
        if self.above.is_empty() {
            return self.immediate_plane_flags(r, c, bw4, bh4);
        }
        let (row, col) = (
            r.wrapping_sub(self.origin_row),
            c.wrapping_sub(self.origin_col),
        );
        [
            edge(&self.left, col != 0, row + bh4 - 1),
            edge(&self.above, row != 0, col + bw4 - 1),
            edge(&self.left, col != 0, row),
            edge(&self.above, row != 0, col),
        ]
    }

    /// [`Self::immediate_flags`] of a grid without edge lines (the encoder's).
    #[cold]
    #[inline(never)]
    fn immediate_plane_flags(
        &self,
        r: usize,
        c: usize,
        bw4: usize,
        bh4: usize,
    ) -> [NeighbourFlags; 4] {
        let (up, left) = (r.wrapping_sub(1), c.wrapping_sub(1));
        self.flags_at_each([(r + bh4 - 1, left), (up, c + bw4 - 1), (r, left), (up, c)])
    }

    /// The flags on row `up` above a block at column `c` on its superblock
    /// row's top, at columns `cols`.
    pub(super) fn sb_border_flags(
        &self,
        c: usize,
        up: usize,
        cols: [usize; 2],
    ) -> [NeighbourFlags; 2] {
        if self.above.is_empty() {
            return self.flags_at_each(cols.map(|col| (up, col)));
        }
        let inside = up.wrapping_sub(self.origin_row) < self.mi_rows;
        cols.map(|col| {
            let line = if col < c { &self.sb_above } else { &self.above };
            edge(line, inside, col.wrapping_sub(self.origin_col))
        })
    }

    /// Publishes the flag plane for one leaf. The entropy pass calls this as
    /// soon as the leaf's syntax is parsed, before any § 7.12 resolution.
    /// Returns the leaf's record index for [`Self::record_leaf_motion`], or
    /// [`NO_LEAF`] when this grid keeps no record of it.
    #[inline]
    pub(crate) fn record_flags(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        syntax: NeighbourFlagSyntax,
    ) -> u32 {
        self.publish_flags(r, c, n4w, n4h, NeighbourFlags::from_syntax(syntax))
    }

    fn publish_flags(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        flags: NeighbourFlags,
    ) -> u32 {
        let Some((rows, cols)) = self.footprint(r, c, n4w, n4h) else {
            return NO_LEAF;
        };
        self.enter_sb_row(rows.start);
        let (row, col) = (rows.start - self.origin_row, cols.start - self.origin_col);
        let (row_end, col_end) = (rows.end - self.origin_row, cols.end - self.origin_col);
        if (rows.start, cols.start) == (r, c) && !self.above.is_empty() {
            for (line, run) in [
                (&mut self.above, col..col_end),
                (&mut self.left, row..row_end),
            ] {
                if let Some(run) = line.get_mut(run) {
                    crate::support::fill_mi_run(run, flags);
                }
            }
            if self.edges_only {
                return NO_LEAF;
            }
        }
        let Some(slot) = u32::try_from(self.planes.leaves.len())
            .ok()
            .and_then(|leaf| NonZeroU32::new(leaf.wrapping_add(1)))
        else {
            return NO_LEAF;
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
        let (lo, hi, bias) = self.readable;
        let (width, stride) = (cols.len(), self.mi_cols);
        let first = row
            .wrapping_add(bias)
            .wrapping_mul(stride)
            .wrapping_add(col);
        if row >= lo
            && row_end <= hi
            && let Some(region) = self
                .planes
                .cells
                .get_mut(first..first + (rows.len() - 1) * stride + width)
        {
            for run in region.chunks_mut(stride) {
                if let Some(run) = run.get_mut(..width) {
                    crate::support::fill_mi_run(run, Some(slot));
                }
            }
            return slot.get() - 1;
        }
        for rr in rows {
            if let Some(span) = self.row_span(rr, &cols)
                && let Some(cells) = self.planes.cells.get_mut(span)
            {
                crate::support::fill_mi_run(cells, Some(slot));
            }
        }
        slot.get() - 1
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
        if let Some(slot) = self
            .index_at(rows.start, cols.start)
            .and_then(|index| *self.planes.cells.get(index)?)
        {
            self.record_leaf_motion(slot.get() - 1, r, c, n4w, n4h, values);
        }
    }

    /// [`Self::record_motion`] for the leaf whose [`Self::record_flags`]
    /// returned `leaf`; a record that is not that leaf's stays unresolved.
    pub(crate) fn record_leaf_motion(
        &mut self,
        leaf: u32,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        values: NeighbourMotionValues,
    ) {
        let leaf = leaf as usize;
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
        let warp = warp_params.filter(|_| motion_mode >= MotionMode::LocalWarp);
        self.record_motion(
            r,
            c,
            n4w,
            n4h,
            NeighbourMotionValues {
                mv: [mv, Mv::ZERO],
                cwp_weight: CWP_EQUAL,
                stored_warp: warp,
                global_mv: [warp_params.is_some() && warp.is_none(), false],
                splat_warp: [warp, None],
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

    /// The leaf and model counts and the cells of the leaf footprint
    /// `(r, c, n4w, n4h)`, for [`Self::rollback`]. Publications after it must
    /// stay inside the footprint and the current superblock row. With `since`,
    /// an older live checkpoint, it also keeps the records published after
    /// `since`, so it stays valid after a rollback to `since`.
    pub(crate) fn checkpoint(
        &self,
        (r, c, n4w, n4h): (usize, usize, usize, usize),
        since: Option<&GridCheckpoint>,
    ) -> GridCheckpoint {
        let mut cells = Vec::new();
        let footprint = self.footprint(r, c, n4w, n4h);
        if let Some((rows, cols)) = &footprint {
            for row in rows.clone() {
                if let Some(saved) = self
                    .row_span(row, cols)
                    .and_then(|span| self.planes.cells.get(span))
                {
                    cells.extend_from_slice(saved);
                }
            }
        }
        let (leaves, models) = since.map_or(
            (self.planes.leaves.len(), self.planes.models.len()),
            |since| (since.leaves, since.models),
        );
        GridCheckpoint {
            leaves,
            models,
            appended: (
                self.planes
                    .leaves
                    .get(leaves..)
                    .unwrap_or_default()
                    .to_vec(),
                self.planes
                    .models
                    .get(models..)
                    .unwrap_or_default()
                    .to_vec(),
            ),
            footprint,
            cells,
        }
    }

    /// Puts a [`Self::checkpoint`] back.
    pub(crate) fn rollback(&mut self, checkpoint: &GridCheckpoint) {
        self.planes.leaves.truncate(checkpoint.leaves);
        self.planes.models.truncate(checkpoint.models);
        self.planes.leaves.extend_from_slice(&checkpoint.appended.0);
        self.planes.models.extend_from_slice(&checkpoint.appended.1);
        let Some((rows, cols)) = &checkpoint.footprint else {
            return;
        };
        let mut saved = checkpoint.cells.chunks(cols.len());
        for row in rows.clone() {
            if let Some(span) = self.row_span(row, cols)
                && let (Some(cells), Some(saved)) = (self.planes.cells.get_mut(span), saved.next())
            {
                cells.copy_from_slice(saved);
            }
        }
    }

    /// The cells of tile rows `rows` x columns `cols` with the leaves and
    /// models they name, for [`Self::import_edge`] on another grid of the
    /// same tile; a row outside the window reads as unpublished.
    pub(crate) fn edge(&self, rows: Range<usize>, cols: Range<usize>) -> GridEdge {
        let end = self.origin_col + self.mi_cols;
        let cols = cols.start.clamp(self.origin_col, end)..cols.end.clamp(self.origin_col, end);
        let mut edge = GridEdge {
            cols: cols.clone(),
            ..GridEdge::default()
        };
        let mut names: Vec<(u32, NonZeroU32)> = Vec::new();
        for row in rows.clone() {
            let cells = self
                .row_span(row, &cols)
                .and_then(|span| self.planes.cells.get(span));
            for index in 0..cols.len() {
                let cell = cells.and_then(|cells| cells.get(index).copied().flatten());
                edge.cells.push(cell.and_then(|slot| {
                    if let Some(&(_, name)) = names.iter().find(|(old, _)| *old == slot.get()) {
                        return Some(name);
                    }
                    let mut leaf = *self.planes.leaves.get(slot.get() as usize - 1)?;
                    let count = (leaf.model_bits
                        & (LeafRecord::SPLAT0 | LeafRecord::SPLAT1 | LeafRecord::STORED_OWN))
                        .count_ones() as usize;
                    let models = self.planes.models.get(leaf.models as usize..)?;
                    leaf.models = u32::try_from(edge.models.len()).ok()?;
                    edge.models.extend_from_slice(models.get(..count)?);
                    edge.leaves.push(leaf);
                    let name = NonZeroU32::new(u32::try_from(edge.leaves.len()).ok()?)?;
                    names.push((slot.get(), name));
                    Some(name)
                }));
            }
        }
        edge.rows = rows;
        edge
    }

    /// Publishes an [`Self::edge`] of another grid over the same cells.
    pub(crate) fn import_edge(&mut self, edge: &GridEdge) {
        let (Ok(leaves), Ok(models)) = (
            u32::try_from(self.planes.leaves.len()),
            u32::try_from(self.planes.models.len()),
        ) else {
            return;
        };
        self.planes
            .leaves
            .extend(edge.leaves.iter().map(|leaf| LeafRecord {
                models: leaf.models + models,
                ..*leaf
            }));
        self.planes.models.extend_from_slice(&edge.models);
        let mut saved = edge.cells.chunks(edge.cols.len().max(1));
        for row in edge.rows.clone() {
            if let (Some(span), Some(saved)) = (self.row_span(row, &edge.cols), saved.next())
                && let Some(cells) = self.planes.cells.get_mut(span)
            {
                for (cell, name) in cells.iter_mut().zip(saved) {
                    *cell = name.and_then(|name| name.checked_add(leaves));
                }
            }
        }
    }

    /// The published leaves in publication order.
    pub(crate) fn published_leaves(&self) -> impl Iterator<Item = PublishedLeaf> + '_ {
        self.planes.leaves.iter().map(|leaf| {
            (
                (
                    leaf.base_r as usize,
                    leaf.base_c as usize,
                    usize::from(leaf.bw4),
                    usize::from(leaf.bh4),
                ),
                leaf.flags
                    .is_inter()
                    .then_some((leaf.flags.ref_frame0(), leaf.flags.ref_frame1())),
                [leaf.mv, leaf.mv1],
                self.splat_models(leaf),
                [0, 1].map(|list| leaf.global_mv_lists & (1 << list) != 0),
            )
        })
    }

    /// The per-list splat warp models of `leaf`.
    fn splat_models(&self, leaf: &LeafRecord) -> [Option<[i32; 6]>; 2] {
        let models = self
            .planes
            .models
            .get(leaf.models as usize..)
            .unwrap_or_default();
        let splat0 = leaf.model_bits & LeafRecord::SPLAT0 != 0;
        let splat1 = leaf.model_bits & LeafRecord::SPLAT1 != 0;
        [
            models.first().copied().filter(|_| splat0),
            models.get(usize::from(splat0)).copied().filter(|_| splat1),
        ]
    }

    /// Moves the window down to the superblock row holding tile row `row`.
    #[inline]
    pub(crate) fn enter_sb_row(&mut self, row: usize) {
        if row >= self.sb_row_end {
            self.slide_to_sb_row(row);
        }
    }

    #[inline(never)]
    fn slide_to_sb_row(&mut self, row: usize) {
        let tile_row = row.saturating_sub(self.origin_row);
        let slide = self.window.enter(tile_row);
        let sb_h4 = self.window.sb_h4();
        self.sb_row_end = (tile_row - tile_row % sb_h4)
            .saturating_add(sb_h4)
            .saturating_add(self.origin_row);
        self.readable = self.window.readable_rows();
        if let Some(slide) = slide {
            slide.apply(&mut self.planes.cells, self.mi_cols, None);
        }
        self.sb_above.clone_from(&self.above);
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
        self.plane_span(self.plane_row(rr)?, cols)
    }

    /// Plane index range covering `cols` on plane row `plane_row`.
    fn plane_span(&self, plane_row: usize, cols: &Range<usize>) -> Option<Range<usize>> {
        let row_base = plane_row.checked_mul(self.mi_cols)?;
        let start = row_base.checked_add(cols.start.checked_sub(self.origin_col)?)?;
        let end = row_base.checked_add(cols.end.checked_sub(self.origin_col)?)?;
        Some(start..end)
    }

    #[allow(clippy::inline_always, reason = "measured: every neighbour probe")]
    #[inline(always)]
    fn index(&self, r: i32, c: i32) -> Option<usize> {
        self.index_at(r as isize as usize, c as isize as usize)
    }

    /// [`Self::index`] of a position whose negative coordinates wrapped.
    #[allow(clippy::inline_always, reason = "measured: every neighbour probe")]
    #[inline(always)]
    fn index_at(&self, r: usize, c: usize) -> Option<usize> {
        let (lo, hi, bias) = self.readable;
        let row = r.wrapping_sub(self.origin_row);
        let col = c.wrapping_sub(self.origin_col);
        if col >= self.mi_cols {
            return None;
        }
        if row.wrapping_sub(lo) >= hi.wrapping_sub(lo) {
            return self.index_outside_readable_rows(r, c);
        }
        Some(row.wrapping_add(bias).wrapping_mul(self.mi_cols) + col)
    }

    /// [`Self::index`] of a row outside the readable rows, which reads as
    /// unpublished and flags a row the window has reused.
    #[cold]
    #[inline(never)]
    fn index_outside_readable_rows(&self, r: usize, c: usize) -> Option<usize> {
        let r = self.plane_row(r)?;
        let c = c.checked_sub(self.origin_col)?;
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

    /// [`Self::flags_at`] of each position, negative coordinates wrapped, with
    /// the grid layout read once for all of them.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-block neighbour contexts"
    )]
    #[inline(always)]
    pub(super) fn flags_at_each<const N: usize>(
        &self,
        positions: [(usize, usize); N],
    ) -> [NeighbourFlags; N] {
        let (cells, leaves) = (self.planes.cells.as_slice(), self.planes.leaves.as_slice());
        positions.map(|(r, c)| {
            self.index_at(r, c)
                .and_then(|index| *cells.get(index)?)
                .and_then(|slot| leaves.get(slot.get() as usize - 1))
                .map_or(ABSENT_NEIGHBOUR_FLAGS, |leaf| leaf.flags)
        })
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
    #[allow(
        clippy::inline_always,
        reason = "measured: probes read a few cell fields"
    )]
    #[inline(always)]
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
            self.warp_motion(leaf, global, r, c)
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

    /// The resolved leaf covering `(r, c)`, `None` like [`Self::get`].
    #[allow(clippy::inline_always, reason = "measured: every stack probe")]
    #[inline(always)]
    pub(super) fn probe(&self, r: i32, c: i32) -> Option<ProbedLeaf<'_>> {
        let leaf = self.leaf_at(r, c)?;
        leaf.resolved.then_some(ProbedLeaf {
            leaf,
            models: &self.planes.models,
            r,
            c,
        })
    }

    /// Warp-model half of [`Self::get`], out of line so the inlined
    /// translation-only path stays small.
    #[inline(never)]
    fn warp_motion(
        &self,
        leaf: &LeafRecord,
        global: NeighbourMotionModel,
        r: i32,
        c: i32,
    ) -> (NeighbourMotionModel, Mv, Mv) {
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
    }

    /// `motion.base_c` of [`Self::get`]`(r, c)` without building the cell.
    #[inline]
    pub(super) fn base_c_at(&self, r: i32, c: i32) -> Option<u32> {
        self.leaf_at(r, c)
            .filter(|leaf| leaf.resolved)
            .map(|leaf| leaf.base_c)
    }

    pub(crate) fn intrabc_mv_at(&self, r: usize, c: usize) -> Option<Mv> {
        let cell = self.get(i32::try_from(r).ok()?, i32::try_from(c).ok()?)?;
        (cell.flags.ref_frame0() == INTRABC_REF_FRAME && cell.flags.ref_frame1().is_none())
            .then_some(cell.motion.mv)
    }

    /// The § 7.12.2.6 spatial IntraBC probes of a block, over the leaves
    /// resolved so far: those decoded before it.
    pub(crate) fn capture_spatial_intrabc_probes(
        &self,
        block: &super::MvBlockContext,
    ) -> SpatialIntrabcProbes {
        let geometry = SpatialScanGeometry {
            mi_row: block.mi_row,
            mi_col: block.mi_col,
            n4w: block.bw4,
            n4h: block.bh4,
            mi_rows: block.mi_rows,
            mi_cols: block.mi_cols,
            sb_size4: block.sb_h4,
        };
        let base_c = |r: usize, c: usize| {
            self.base_c_at(i32::try_from(r).ok()?, i32::try_from(c).ok()?)
                .map(|col| col as usize)
        };
        capture_spatial_intrabc_probes(geometry, |r, c| base_c(r, c).is_some(), base_c)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn warp_model_at(&self, r: usize, c: usize) -> Option<[i32; 6]> {
        self.get(i32::try_from(r).ok()?, i32::try_from(c).ok()?)?
            .motion
            .warp_params()
    }

    pub(crate) fn is_non_tip_at(&self, r: i32, c: i32) -> bool {
        matches!(self.flags_at(r, c), Some(flags) if flags.ref_frame0() != TIP_REF_FRAME)
    }
}

#[cfg(test)]
#[path = "neighbour_grid_tests.rs"]
mod tests;
