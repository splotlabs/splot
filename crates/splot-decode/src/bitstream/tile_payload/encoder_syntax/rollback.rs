// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Region-local snapshots of the encoder facade.
//!
//! A trial on a partition node writes neighbour state only over the node's
//! luma 4x4 rectangle, widened up and left to the origin that the chroma of a
//! has-chroma leaf at an odd 4x4 position in a subsampled axis covers: block publication
//! fills the leaf footprint and its chroma reference block, and the
//! coefficient and partition contexts fill the above and left lines along
//! those blocks. A snapshot saves those cells, and marks a journal that keeps
//! the first contents of each CDF row a trial reaches, so neither cost grows
//! with the frame. The `BlockDecoded` state and the warp bank are
//! superblock-sized and saved whole. A snapshot enters the node's superblock
//! row in every windowed grid first, so no trial moves a window.

use core::num::NonZeroU32;
use core::ops::Range;

use splot_recon::{PlaneId, PlaneRect};

use super::super::block_decoded_state::TileBlockDecodedState;
use super::super::cdf::block_context::{IntraJointMode, MrlSelection};
use super::super::cdf::coeff_context::CoeffBaseSelection;
use super::super::cdf::{CoeffBaseRow, CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::super::intra_joint_modes::{LumaPalette, TileIntraYModeFacts};
use super::super::tile_frontier::chroma_subsampling;
use super::intra::Recon;
use super::{EncoderSyntaxError, IntraTileSyntax, PartitionNode, Result, rule};

/// A CDF row of the tile, by the accessor that reaches it.
#[derive(Clone, Copy, Debug)]
pub(super) enum CdfRow {
    Tile(TileCdfSelector),
    Coeff(CoeffCdfSelector),
    Base(CoeffBaseSelection, usize, usize),
    Br(usize, bool, usize),
}

impl CdfRow {
    fn get(self, cdfs: &mut TileCdfSubset) -> Result<&mut [u16]> {
        match self {
            Self::Tile(selector) => cdfs.row_mut(selector).map_err(rule("CDF row")),
            Self::Coeff(selector) => cdfs
                .coeff_rows_mut()
                .row_mut(selector)
                .map_err(rule("coeff_base_eob row")),
            Self::Base(selection, tx_size, quantizer) => Ok(
                match cdfs
                    .coeff_rows_mut()
                    .base_row(selection, tx_size, quantizer)
                    .map_err(rule("coeff_base row"))?
                {
                    CoeffBaseRow::Base(row) => row.as_mut_slice(),
                    CoeffBaseRow::Lf(row) => row.as_mut_slice(),
                },
            ),
            Self::Br(plane, is_lf, ctx) => cdfs
                .coeff_rows_mut()
                .br_row(plane, is_lf, ctx)
                .map(<[u16; _]>::as_mut_slice)
                .map_err(rule("coeff_br row")),
        }
    }
}

/// The contents each CDF row had when a trial first reached it after the
/// latest mark; a rollback copies back only those rows.
///
/// A row is logged once per epoch, and every mark and rollback starts one, so
/// the journal past any live mark holds each row reached since then with its
/// contents at the mark as the oldest entry.
pub(super) struct CdfJournal {
    entries: Vec<(CdfRow, usize)>,
    saved: Vec<u16>,
    stamps: Vec<u32>,
    epoch: u32,
    generation: u64,
    #[cfg(debug_assertions)]
    snapshots: u32,
}

impl CdfJournal {
    /// A journal for the rows of `cdfs`.
    pub(super) fn new(cdfs: &TileCdfSubset) -> Self {
        Self {
            entries: Vec::new(),
            saved: Vec::new(),
            stamps: vec![0; cdfs.rows_span().1],
            epoch: 0,
            generation: 0,
            #[cfg(debug_assertions)]
            snapshots: 0,
        }
    }

    /// The row of `key`, logged first.
    fn row<'a>(&mut self, cdfs: &'a mut TileCdfSubset, key: CdfRow) -> Result<&'a mut [u16]> {
        let (base, _) = cdfs.rows_span();
        let row = key.get(cdfs)?;
        self.log(base, key, row);
        Ok(row)
    }

    /// Logs `row`, the row of `key` in the table at `base`, unless this epoch has.
    #[inline]
    pub(super) fn log(&mut self, base: usize, key: CdfRow, row: &[u16]) {
        let slot = row.as_ptr().addr().wrapping_sub(base) / size_of::<u16>();
        if let Some(stamp) = self.stamps.get_mut(slot) {
            if *stamp == self.epoch {
                return;
            }
            *stamp = self.epoch;
        }
        self.entries.push((key, self.saved.len()));
        self.saved.extend_from_slice(row);
    }

    /// Forgets every mark.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.saved.clear();
        self.generation += 1;
        self.next_epoch();
    }

    fn next_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamps.fill(0);
            self.epoch = 1;
        }
    }

    pub(super) fn mark(&mut self) -> usize {
        self.next_epoch();
        self.entries.len()
    }

    pub(super) fn rollback(&mut self, cdfs: &mut TileCdfSubset, mark: usize) -> Result<()> {
        while self.entries.len() > mark {
            let Some((key, start)) = self.entries.pop() else {
                break;
            };
            let row = key.get(cdfs)?;
            row.copy_from_slice(self.saved.get(start..start + row.len()).ok_or(STALE)?);
            self.saved.truncate(start);
        }
        self.next_epoch();
        Ok(())
    }

    /// Each row reached since `mark` once, with its current contents.
    fn rows_since(
        &mut self,
        cdfs: &mut TileCdfSubset,
        mark: usize,
    ) -> Result<(Vec<CdfRow>, Vec<u16>)> {
        self.next_epoch();
        let (base, _) = cdfs.rows_span();
        let (mut keys, mut rows) = (Vec::new(), Vec::new());
        for index in mark..self.entries.len() {
            let key = self.entries[index].0;
            let row = key.get(cdfs)?;
            let slot = row.as_ptr().addr().wrapping_sub(base) / size_of::<u16>();
            if let Some(stamp) = self.stamps.get_mut(slot) {
                if *stamp == self.epoch {
                    continue;
                }
                *stamp = self.epoch;
            }
            keys.push(key);
            rows.extend_from_slice(row);
        }
        self.next_epoch();
        Ok((keys, rows))
    }

    /// Writes rows from [`Self::rows_since`] back, logging each.
    fn replay(&mut self, cdfs: &mut TileCdfSubset, keys: &[CdfRow], rows: &[u16]) -> Result<()> {
        let mut rows = rows;
        for &key in keys {
            let row = self.row(cdfs, key)?;
            let (saved, rest) = rows.split_at_checked(row.len()).ok_or(STALE)?;
            row.copy_from_slice(saved);
            rows = rest;
        }
        Ok(())
    }

    /// Whether this snapshot also checks its rollback against a whole copy.
    #[cfg(debug_assertions)]
    fn sample(&mut self) -> bool {
        self.snapshots = self.snapshots.wrapping_add(1);
        self.snapshots.wrapping_mul(0x9E37_79B9) >> 30 == 0
    }
}

const STALE: EncoderSyntaxError =
    EncoderSyntaxError::Unsupported("a snapshot the facade can no longer restore");

/// The neighbour cells of a node rectangle, in visiting order.
#[derive(Default)]
struct Cells {
    bytes: Vec<u8>,
    flags: Vec<bool>,
    joint_modes: Vec<IntraJointMode>,
    uses_mrls: Vec<MrlSelection>,
    palettes: Vec<Option<NonZeroU32>>,
    y_modes: Vec<Option<TileIntraYModeFacts>>,
}

/// One direction of a cell copy: into a [`Cells`] list or back out of one.
enum Pass<'a, T> {
    Save(&'a mut Vec<T>),
    Restore(&'a [T], bool),
}

impl<'a, T: Copy> Pass<'a, T> {
    fn new(save: Option<&'a mut Vec<T>>, saved: &'a [T]) -> Self {
        save.map_or(Self::Restore(saved, true), Self::Save)
    }

    fn visit(&mut self, cells: &mut [T]) {
        match self {
            Self::Save(store) => store.extend_from_slice(cells),
            Self::Restore(saved, ok) => match saved.split_at_checked(cells.len()) {
                Some((head, tail)) => {
                    cells.copy_from_slice(head);
                    *saved = tail;
                }
                None => *ok = false,
            },
        }
    }

    fn done(&self) -> bool {
        match self {
            Self::Save(_) => true,
            Self::Restore(saved, ok) => *ok && saved.is_empty(),
        }
    }
}

/// A saved [`IntraTileSyntax`] context and reconstruction of one node, so a
/// search can code a trial there and roll it back without copying the frame.
pub struct SyntaxSnapshot {
    generation: u64,
    mark: usize,
    forward: (Vec<CdfRow>, Vec<u16>),
    rows: Range<usize>,
    cols: Range<usize>,
    cells: Cells,
    palettes: Vec<LumaPalette>,
    block_decoded: TileBlockDecodedState,
    sdp: super::SdpPartitionState,
    samples: Vec<(PlaneId, PlaneRect, Vec<u16>)>,
    inter: super::inter::InterSnapshot,
    #[cfg(debug_assertions)]
    check: Option<Box<WholeState>>,
}

/// Every piece of state a snapshot saves only in part.
#[cfg(debug_assertions)]
#[derive(PartialEq)]
struct WholeState {
    cdfs: TileCdfSubset,
    coeff: super::TileCoeffContextState,
    mi_sizes: super::TileMiSizeState,
    joint_modes: super::TileIntraJointModeState,
    uses_mrls: super::TileUsesMrlsState,
    use_dip: super::TileUseDipState,
    fsc_modes: super::TileFscModeState,
    palette_y: super::TileLumaPaletteState,
    uv_cfls: super::TileUvCflState,
    y_modes: super::TileIntraYModeState,
    y_smooth: crate::prediction::intra_edge::TileSmoothGrid,
    chroma_smooth: crate::prediction::intra_edge::TileSmoothGrid,
    ref_mv_bank: Option<crate::prediction::inter::RefMvBank>,
    tip_records: Vec<crate::prediction::inter::TemporalMotionBlock>,
    intrabc: Option<(
        crate::filters::wienerns_lr::intrabc_records::TileIntrabcPreludeState,
        Vec<Option<crate::prediction::inter::Mv>>,
    )>,
}

impl IntraTileSyntax {
    /// Saves the coding context and the reconstruction of `node`; a trial
    /// coded at it can roll back with [`Self::restore`] until an older
    /// snapshot is restored or the next superblock begins.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a node outside the frame.
    pub fn snapshot(&mut self, node: PartitionNode) -> Result<SyntaxSnapshot> {
        let mark = self.journal.mark();
        self.save(node, mark, (Vec::new(), Vec::new()), None)
    }

    /// [`Self::snapshot`] of the state a trial reached from `start`, a live
    /// snapshot of the same node: it restores after `start` is restored.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] when `start` is no longer live.
    pub fn snapshot_since(
        &mut self,
        start: &SyntaxSnapshot,
        node: PartitionNode,
    ) -> Result<SyntaxSnapshot> {
        self.check_live(start)?;
        let forward = self.journal.rows_since(&mut self.cdfs, start.mark)?;
        self.save(node, start.mark, forward, Some(&start.inter))
    }

    /// Puts a [`Self::snapshot`] back.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] when the snapshot is no longer live.
    pub fn restore(&mut self, snapshot: &SyntaxSnapshot) -> Result<()> {
        self.check_live(snapshot)?;
        self.journal.rollback(&mut self.cdfs, snapshot.mark)?;
        let (keys, rows) = &snapshot.forward;
        self.journal.replay(&mut self.cdfs, keys, rows)?;
        if !self.visit_cells(&snapshot.rows, &snapshot.cols, None, &snapshot.cells) {
            return Err(STALE);
        }
        self.palette_y.palettes_mut().clone_from(&snapshot.palettes);
        self.recon.block_decoded.clone_from(&snapshot.block_decoded);
        self.sdp = snapshot.sdp;
        self.recon.restore_samples(&snapshot.samples)?;
        self.inter_restore(&snapshot.inter);
        #[cfg(debug_assertions)]
        if snapshot
            .check
            .as_ref()
            .is_some_and(|whole| **whole != self.whole_state())
        {
            return Err(EncoderSyntaxError::Unsupported(
                "a rollback that differs from the saved state",
            ));
        }
        Ok(())
    }

    fn check_live(&self, snapshot: &SyntaxSnapshot) -> Result<()> {
        if snapshot.generation == self.journal.generation
            && snapshot.mark <= self.journal.entries.len()
        {
            Ok(())
        } else {
            Err(STALE)
        }
    }

    /// The neighbour cells cover the node and its chroma reference block,
    /// which every chroma block a leaf under the node codes lies in, from
    /// the 8x8 origin. The samples cover the node alone.
    fn save(
        &mut self,
        node: PartitionNode,
        mark: usize,
        forward: (Vec<CdfRow>, Vec<u16>),
        since: Option<&super::inter::InterSnapshot>,
    ) -> Result<SyntaxSnapshot> {
        let (w, h) = node.dimensions()?;
        let (x, y) = (node.col4() * 4, node.row4() * 4);
        let chroma = node.0.chroma_ref_geometry();
        let size = chroma.size();
        let (n4w, n4h) = (
            size.num_4x4_wide().map_err(rule("block size"))?,
            size.num_4x4_high().map_err(rule("block size"))?,
        );
        let (sub_x, sub_y) = self.subsampling();
        let rows =
            node.row4().min(chroma.row()) & !sub_y..(node.row4() + h / 4).max(chroma.row() + n4h);
        let cols =
            node.col4().min(chroma.col()) & !sub_x..(node.col4() + w / 4).max(chroma.col() + n4w);
        self.enter_rows(rows.start);
        let mut cells = Cells::default();
        self.visit_cells(&rows, &cols, Some(&mut cells), &Cells::default());
        Ok(SyntaxSnapshot {
            generation: self.journal.generation,
            mark,
            forward,
            rows,
            cols,
            cells,
            palettes: self.palette_y.palettes_mut().clone(),
            block_decoded: self.recon.block_decoded.clone(),
            sdp: self.sdp,
            samples: self.recon.save_samples((x, y, w, h), (sub_x, sub_y))?,
            inter: self.inter_snapshot((x, y, w, h), since),
            #[cfg(debug_assertions)]
            check: self.journal.sample().then(|| Box::new(self.whole_state())),
        })
    }

    /// `(subsampling_x, subsampling_y)` of the sequence.
    fn subsampling(&self) -> (usize, usize) {
        let (x, y) = chroma_subsampling(self.chroma);
        (usize::from(x), usize::from(y))
    }

    fn enter_rows(&mut self, row: usize) {
        self.joint_modes.grid_mut().enter_row(row);
        self.uses_mrls.grid_mut().enter_row(row);
        self.use_dip.grid_mut().enter_row(row);
        self.fsc_modes.grid_mut().enter_row(row);
        self.palette_y.grid_mut().enter_row(row);
        self.uv_cfls.grid_mut().enter_row(row);
        self.y_modes.grid_mut().enter_row(row);
        self.recon.y_smooth.grid_mut().enter_row(row);
        let chroma_row = row >> self.subsampling().1;
        self.recon.chroma_smooth.grid_mut().enter_row(chroma_row);
    }

    /// Copies the cells of luma rows `rows` x columns `cols` into `save`, or
    /// back from `saved` without it; `false` when `saved` does not fit.
    fn visit_cells(
        &mut self,
        rows: &Range<usize>,
        cols: &Range<usize>,
        save: Option<&mut Cells>,
        saved: &Cells,
    ) -> bool {
        let (sub_x, sub_y) = self.subsampling();
        let (chroma_rows, chroma_cols) = (
            rows.start >> sub_y..rows.end.div_ceil(1 << sub_y),
            cols.start >> sub_x..cols.end.div_ceil(1 << sub_x),
        );
        let (bytes, flags, joint, mrls, palettes, y_modes) = match save {
            Some(c) => (
                Some(&mut c.bytes),
                Some(&mut c.flags),
                Some(&mut c.joint_modes),
                Some(&mut c.uses_mrls),
                Some(&mut c.palettes),
                Some(&mut c.y_modes),
            ),
            None => (None, None, None, None, None, None),
        };
        let mut bytes = Pass::new(bytes, &saved.bytes);
        self.mi_sizes
            .visit_region(rows.clone(), cols, |c| bytes.visit(c));
        self.coeff
            .visit_region(rows, cols, (sub_x, sub_y), |c| bytes.visit(c));
        self.use_dip
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| bytes.visit(c));
        self.fsc_modes
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| bytes.visit(c));
        self.uv_cfls
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| bytes.visit(c));
        let mut flags = Pass::new(flags, &saved.flags);
        self.recon
            .y_smooth
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| flags.visit(c));
        self.recon
            .chroma_smooth
            .grid_mut()
            .visit_cells(chroma_rows, &chroma_cols, |c| flags.visit(c));
        let mut joint = Pass::new(joint, &saved.joint_modes);
        self.joint_modes
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| joint.visit(c));
        let mut mrls = Pass::new(mrls, &saved.uses_mrls);
        self.uses_mrls
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| mrls.visit(c));
        let mut palettes = Pass::new(palettes, &saved.palettes);
        self.palette_y
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| palettes.visit(c));
        let mut y_modes = Pass::new(y_modes, &saved.y_modes);
        self.y_modes
            .grid_mut()
            .visit_cells(rows.clone(), cols, |c| y_modes.visit(c));
        bytes.done()
            && flags.done()
            && joint.done()
            && mrls.done()
            && palettes.done()
            && y_modes.done()
    }

    #[cfg(debug_assertions)]
    fn whole_state(&self) -> WholeState {
        WholeState {
            cdfs: self.cdfs.clone(),
            coeff: self.coeff.clone(),
            mi_sizes: self.mi_sizes.clone(),
            joint_modes: self.joint_modes.clone(),
            uses_mrls: self.uses_mrls.clone(),
            use_dip: self.use_dip.clone(),
            fsc_modes: self.fsc_modes.clone(),
            palette_y: self.palette_y.clone(),
            uv_cfls: self.uv_cfls.clone(),
            y_modes: self.y_modes.clone(),
            y_smooth: self.recon.y_smooth.clone(),
            chroma_smooth: self.recon.chroma_smooth.clone(),
            ref_mv_bank: self
                .inter
                .as_ref()
                .and_then(|inter| inter.tile.ref_mv_bank().cloned()),
            tip_records: self
                .inter
                .as_ref()
                .map(|inter| inter.tile.tip_records().to_vec())
                .unwrap_or_default(),
            intrabc: self
                .inter
                .as_ref()
                .and_then(|inter| inter.tile.intrabc_debug_state()),
        }
    }
}

impl Recon {
    /// The samples of the luma rectangle `(x, y, width, height)`, clipped to
    /// storage. In a subsampled axis the chroma rectangle starts at the 8-sample
    /// luma origin, which a has-chroma leaf at an odd 4x4 position codes.
    fn save_samples(
        &self,
        (x, y, w, h): (usize, usize, usize, usize),
        (sub_x, sub_y): (usize, usize),
    ) -> Result<Vec<(PlaneId, PlaneRect, Vec<u16>)>> {
        let mut samples = Vec::with_capacity(3);
        let planes = if self.workspace.info().pixel_format().is_monochrome() {
            1
        } else {
            3
        };
        for (plane, (sx, sy)) in [
            (PlaneId::Y, (0, 0)),
            (PlaneId::U, (sub_x, sub_y)),
            (PlaneId::V, (sub_x, sub_y)),
        ]
        .into_iter()
        .take(planes)
        {
            let size = self
                .workspace
                .plane(plane)
                .map_err(rule("snapshot plane"))?
                .storage_size();
            let (px, py) = ((x & !(4 * sx)) >> sx, (y & !(4 * sy)) >> sy);
            let pw = (x + w)
                .div_ceil(1 << sx)
                .min(size.width())
                .saturating_sub(px);
            let ph = (y + h)
                .div_ceil(1 << sy)
                .min(size.height())
                .saturating_sub(py);
            if pw == 0 || ph == 0 {
                continue;
            }
            let rect = PlaneRect::new(px, py, pw, ph).map_err(rule("snapshot rect"))?;
            let mut saved = Vec::with_capacity(pw * ph);
            for row in self
                .workspace
                .rect_rows(plane, rect)
                .map_err(rule("snapshot rows"))?
            {
                saved.extend_from_slice(row);
            }
            samples.push((plane, rect, saved));
        }
        Ok(samples)
    }

    /// Puts [`Self::save_samples`] back and drops the planned leaf.
    fn restore_samples(&mut self, samples: &[(PlaneId, PlaneRect, Vec<u16>)]) -> Result<()> {
        self.leaf = None;
        for (plane, rect, samples) in samples {
            self.workspace
                .write_rect(*plane, *rect, samples, rect.width())
                .map_err(rule("restore rows"))?;
        }
        Ok(())
    }
}
