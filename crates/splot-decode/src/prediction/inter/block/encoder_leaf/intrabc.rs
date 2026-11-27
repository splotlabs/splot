// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra block copy leaves of an intra frame (§ 5.20.5.4, § 7.11.4): the
//! tile state, the reference block vectors, the copy prediction and the
//! publication, as the decoder's intra-frame walk parses and resolves them.

use crate::filters::wienerns_lr::intrabc_records::{
    IntrabcBlockGeometry, IntrabcInfo, IntrabcRefSelection, frame_allows_intrabc,
    intrabc_use_coded, max_bvp_drl_bits_minus_1,
};
use crate::filters::wienerns_lr::intrabc_ref_mv_stack::{
    IntrabcStackGeometry, SpatialIntrabcProbes, SpatialScanGeometry,
    capture_spatial_intrabc_probes, select_intrabc_ref_mv_from_candidates,
};
use crate::prediction::inter::find_mv_stack::NeighbourMotionValues;

use super::*;

/// The § 8.3.2 contexts of one IntraBC leaf: `use_intrabc`, `skip_txfm` and
/// `morph_pred`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcContexts {
    pub(crate) use_intrabc: usize,
    pub(crate) skip: usize,
    pub(crate) morph_pred: usize,
}

impl EncoderInterTile {
    /// The state of the tile over luma 4x4 `rows` and `cols` of the intra
    /// frame `core` with `allow_intrabc`; `Err` names a missing fact.
    pub(crate) fn new_intrabc(
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        reference: InterReferenceState<u16>,
        (rows, cols): (core::ops::Range<usize>, core::ops::Range<usize>),
        (qindex, bit_depth): (u32, BitDepth),
    ) -> core::result::Result<Self, &'static str> {
        let seq_inter = sequence
            .inter
            .as_ref()
            .ok_or("incomplete sequence header")?;
        let size = core.frame_size.ok_or("incomplete intra frame header")?;
        let (mi_rows, mi_cols) = (
            (size.height as usize).div_ceil(8) * 2,
            (size.width as usize).div_ceil(8) * 2,
        );
        let sb_size = sequence.partition.as_ref().map_or(
            SuperblockSize::Block64x64,
            SequencePartitionConfig::seq_sb_size,
        );
        let sb_h4 = frame_superblock_h4(sb_size, true);
        let tile_start = (rows.start, cols.start);
        let mut grid = NeighbourMvGrid::default();
        grid.reset_for_tile(
            rows.clone(),
            cols.clone(),
            sb_h4,
            crate::prediction::inter::find_mv_stack::NeighbourGridMode::Motion,
        )
        .map_err(|_| "neighbour grid size")?;
        let mut intrabc = TileIntrabcPreludeState::default();
        intrabc
            .reset_for_tile((mi_rows, mi_cols), rows, cols, sequence, true, true)
            .map_err(|_| "intra block copy state")?;
        Ok(Self {
            ref_facts: super::super::compound_path::FrameRefFacts::new(core, &reference, &[], 0),
            grid,
            warp_bank: WarpParamBank::new(),
            ref_mv_bank: seq_inter.enable_refmvbank.then(RefMvBank::new),
            temporal: None,
            temporal_first: false,
            ref_frame_idx: Vec::new(),
            order_hints: Vec::new(),
            current_order_hint: core.display_order_hint().unwrap_or(0),
            reference,
            core: core.clone(),
            sequence: sequence.clone(),
            precision: frame_mv_precision(core).map_err(|_| "MV precision")?,
            max_drl_bits_minus_1: 0,
            drl_reorder: sequence_drl_reorder(sequence),
            compound_tools: (false, false),
            tile_start,
            blend_tools: (false, false),
            interp_switchable: false,
            num_same_ref_compound: 0,
            compound_pairs: Vec::new(),
            sb_h4,
            mi_rows,
            mi_cols,
            chroma: sequence.general.chroma_format_idc,
            qindex,
            bit_depth,
            interintra_scratch: super::super::interintra::InterIntraScratch::default(),
            tip_ref_pair: None,
            tip_records: Vec::new(),
            tip_trial_records: Vec::new(),
            tip_scratch: super::super::tip::TipReconstructScratch::default(),
            refined: mc::RefinedUnits::default(),
            intrabc: Some(intrabc),
        })
    }

    /// The motion that names the IntraBC reference of a leaf coding
    /// `RefMvIdx = drl`, for the inter leaf paths an IntraBC leaf shares.
    pub(crate) const fn intrabc_motion(&self, drl: usize) -> EncoderMotion {
        EncoderMotion::Single(EncoderSingleMotion {
            ref_frame: INTRABC_REF_FRAME,
            mode: SINGLE_MODE_NEWMV,
            ref_mv_idx: drl,
            mvd: MotionVector::ZERO,
            interp: ReconInterpolationFilter::Bilinear,
            precision: Some(self.precision),
            use_amvd: false,
            tip: false,
            interintra: None,
            bawp: None,
        })
    }

    /// The frame header of the tile.
    pub(crate) const fn frame_header(&self) -> &FrameHeaderCore {
        &self.core
    }

    /// The sequence header of the tile.
    pub(crate) const fn sequence_header(&self) -> &SequenceHeader {
        &self.sequence
    }

    /// The base column of the coded block that covers the 4x4 cell at `row`,
    /// `col`; `None` for an uncoded cell. An inter frame reads it from the
    /// neighbour grid, as the decoder does.
    pub(crate) fn intrabc_coded_base_col(&self, row: usize, col: usize) -> Option<usize> {
        match &self.intrabc {
            Some(state) => state.block_base_col_at(row, col),
            None => self.grid_coded_base_col(row, col),
        }
    }

    /// [`Self::intrabc_coded_base_col`] of an inter frame, out of line: the
    /// local-area scans of the intra-frame path inline the other arm.
    #[inline(never)]
    fn grid_coded_base_col(&self, row: usize, col: usize) -> Option<usize> {
        self.grid
            .base_c_at(i32::try_from(row).ok()?, i32::try_from(col).ok()?)
            .map(|col| col as usize)
    }

    /// The § 7.12.2.6 spatial IntraBC probes of `block` in an inter frame,
    /// from the neighbour grid as the decoder's resolve pass takes them.
    #[inline(never)]
    fn grid_intrabc_probes(&self, block: &MvBlockContext) -> SpatialIntrabcProbes {
        let geometry = SpatialScanGeometry {
            mi_row: block.mi_row,
            mi_col: block.mi_col,
            n4w: block.bw4,
            n4h: block.bh4,
            mi_rows: block.mi_rows,
            mi_cols: block.mi_cols,
            sb_size4: block.sb_h4,
        };
        let base_col = |row, col| self.grid_coded_base_col(row, col);
        capture_spatial_intrabc_probes(geometry, |row, col| base_col(row, col).is_some(), base_col)
    }

    /// The `use_intrabc` and `skip_txfm` contexts of `frontier` in an inter
    /// frame, from the neighbour context as the decoder reads them.
    #[inline(never)]
    fn grid_intrabc_contexts(&self, frontier: &DecodeBlockFrontier) -> Result<IntrabcContexts> {
        let block = self.block(frontier, (SINGLE_REF_FRAME0, None))?;
        let neighbour = block_neighbour_ctx(&self.grid, &block);
        Ok(IntrabcContexts {
            use_intrabc: neighbour.intrabc_ctx(),
            skip: neighbour.skip_ctx(),
            morph_pred: 0,
        })
    }

    /// `(allow_global_intrabc, allow_local_intrabc)` of the frame with the
    /// values that § 5.18.3.4 implies: an inter frame has the local area
    /// only, an intra frame without the global area has the local area.
    pub(crate) fn intrabc_areas(&self) -> (bool, bool) {
        if self.intrabc.is_none() {
            return (false, true);
        }
        let params = self.core.intrabc;
        let global = params.is_some_and(|p| p.allow_global_intrabc == Some(true));
        let local = !global || params.is_some_and(|p| p.allow_local_intrabc == Some(true));
        (global, local)
    }

    /// Whether the frame codes `allow_intrabc` with the local area.
    pub(crate) fn intrabc_local_area(&self) -> bool {
        frame_allows_intrabc(&self.core) && self.intrabc_areas().1
    }

    /// `force_integer_mv` of the frame, also where only the inter path
    /// records it.
    pub(crate) fn force_integer_mv(&self) -> bool {
        effective_force_integer_mv(&self.core)
    }

    /// Whether the tile is the IntraBC state of an intra frame.
    pub(crate) const fn is_intra_frame(&self) -> bool {
        self.intrabc.is_some()
    }

    /// The superblock height the reference MV bank resets with: an intra
    /// frame caps it at 32 4x4 units.
    pub(super) fn bank_sb_h4(&self) -> usize {
        super::super::resolve::effective_intrabc_sb_h4(self.sb_h4, self.intrabc.is_some())
    }

    /// Records the IntraBC facts of the leaf at `frontier`.
    pub(super) fn record_intrabc_prelude(
        &mut self,
        frontier: &DecodeBlockFrontier,
        prelude: IntrabcBlockPrelude,
    ) -> Result<()> {
        let (n4w, n4h) = block_size4(frontier)?;
        match &mut self.intrabc {
            Some(state) => state.record_block(frontier.r, frontier.c, n4w, n4h, prelude),
            None => Ok(()),
        }
    }

    /// The IntraBC contexts of `frontier`, or `None` when it codes no
    /// `use_intrabc`.
    pub(crate) fn intrabc_contexts(
        &self,
        frontier: &DecodeBlockFrontier,
    ) -> Result<Option<IntrabcContexts>> {
        let (n4w, n4h) = block_size4(frontier)?;
        if !intrabc_use_coded(&self.core, frontier, n4w, n4h) {
            return Ok(None);
        }
        let Some(state) = &self.intrabc else {
            return self.grid_intrabc_contexts(frontier).map(Some);
        };
        let (r, c) = (frontier.r, frontier.c);
        Ok(Some(IntrabcContexts {
            use_intrabc: state.intrabc_ctx(r, c, n4w, n4h)?,
            skip: state.skip_ctx(r, c, n4w, n4h)?,
            morph_pred: state.morph_pred_ctx(r, c, n4w, n4h)?,
        }))
    }

    /// `max_bvp_drl_bits_minus_1` of the frame.
    pub(crate) fn max_bvp_drl_bits_minus_1(&self) -> Result<u32> {
        max_bvp_drl_bits_minus_1(&self.sequence, &self.core)
    }

    /// The reference block vector of each `RefMvIdx` of the leaf at
    /// `frontier`, as the decoder's § 7.12 IntraBC stack resolves them.
    pub(crate) fn intrabc_ref_mvs(
        &mut self,
        frontier: &DecodeBlockFrontier,
    ) -> Result<Vec<MotionVector>> {
        let (n4w, n4h) = block_size4(frontier)?;
        let max = self.max_bvp_drl_bits_minus_1()?;
        let block = self.block(frontier, (INTRABC_REF_FRAME, None))?;
        self.reset_banks(&block);
        let geometry = IntrabcBlockGeometry::from_frontier(frontier, n4w, n4h);
        let grid = &self.grid;
        let probes = match &self.intrabc {
            Some(state) => state.capture_spatial_intrabc_probes(geometry),
            None => self.grid_intrabc_probes(&block),
        };
        let spatial = probes.resolve(|row, col| grid.intrabc_mv_at(row, col));
        let bank = self.ref_mv_bank.as_ref().map(RefMvBank::intrabc_candidates);
        let stack = IntrabcStackGeometry {
            mi_row: frontier.r,
            mi_col: frontier.c,
            n4w,
            n4h,
            sb_samples: i32::try_from(self.bank_sb_h4() * 4).unwrap_or(i32::MAX),
            frame_w: i32::try_from(self.mi_cols * 4).unwrap_or(i32::MAX),
            frame_h: i32::try_from(self.mi_rows * 4).unwrap_or(i32::MAX),
        };
        (0..=max as usize + 1)
            .map(|index| {
                let selection = IntrabcRefSelection::new(max, index).ok_or(INTRA_ONLY)?;
                Ok(select_intrabc_ref_mv_from_candidates(
                    bank.as_deref().unwrap_or(&[]),
                    stack,
                    &spatial,
                    self.ref_mv_bank.is_some(),
                    self.drl_reorder,
                    selection,
                ))
            })
            .collect()
    }

    /// Predicts the leaf at `frontier` by intra block copy with the block
    /// vector `mv` from the reconstruction in `workspace` (§ 7.11.4), with
    /// the § 7.13.3.26 morphological prediction of its luma when `morph`.
    pub(crate) fn predict_intrabc<B: splot_core::simd::SimdBackend>(
        &mut self,
        backend: B,
        frontier: &DecodeBlockFrontier,
        mv: MotionVector,
        morph: bool,
        workspace: &mut CurrentFrameWorkspace<u16>,
    ) -> Result<()> {
        let (n4w, n4h) = block_size4(frontier)?;
        let info = IntrabcInfo {
            morph_pred: morph,
            block_mv: mv,
        };
        let avail = super::super::tile_neighbour_availability(
            frontier.r,
            frontier.c,
            self.tile_start.0,
            self.tile_start.1,
        );
        let subsampling = self.chroma.subsampling();
        let prediction = super::super::intrabc::IntrabcReconPrediction::derive(
            &self.core,
            frontier,
            avail,
            n4w,
            n4h,
            info,
            (u32::from(subsampling.0), u32::from(subsampling.1)),
            ByteOffset::new(0),
        )?;
        super::super::intrabc::IntrabcReconCommand::new(
            prediction,
            None,
            0,
            self.qindex,
            false,
            self.bit_depth,
        )
        .reconstruct(backend, &[], &[], workspace)
    }

    /// Publishes a coded IntraBC leaf with block vector `mv`, `morph_pred =
    /// morph` and `skip_txfm = skip`: its flags, its IntraBC facts, then its
    /// motion.
    pub(crate) fn publish_intrabc(
        &mut self,
        frontier: &DecodeBlockFrontier,
        mv: MotionVector,
        (morph, skip): (bool, bool),
    ) -> Result<()> {
        let block = self.block(frontier, (INTRABC_REF_FRAME, None))?;
        self.grid.record_flags(
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            NeighbourFlagSyntax {
                is_inter: true,
                ref_frame0: INTRABC_REF_FRAME,
                skip,
                interp_filter: interp_filter_no_neighbour_ctx(false) as u8,
                precision: BlockPrecisionRecord::explicit(self.precision),
                ..NON_INTER_FLAG_SYNTAX
            },
        );
        let prelude = IntrabcBlockPrelude::from_use_skip(IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: skip,
        })
        .with_morph_pred(morph);
        self.record_intrabc_prelude(frontier, prelude)?;
        self.reset_banks(&block);
        self.grid.record_motion(
            block.mi_row,
            block.mi_col,
            block.bw4,
            block.bh4,
            NeighbourMotionValues {
                mv: [mv, MotionVector::ZERO],
                cwp_weight: mc::CWP_EQUAL,
                stored_warp: None,
                global_mv: [false, false],
                splat_warp: [None, None],
            },
        );
        let sb_h4 = self.bank_sb_h4();
        if let Some(bank) = &mut self.ref_mv_bank {
            bank.update_for_block(
                INTRABC_REF_FRAME,
                None,
                mv,
                None,
                mc::CWP_EQUAL,
                block.mi_row,
                block.mi_col,
                block.bw4,
                block.bh4,
                sb_h4,
            );
        }
        Ok(())
    }

    /// The IntraBC facts and the block vector of each 4x4 unit, for the
    /// debug rollback check.
    #[cfg(debug_assertions)]
    pub(crate) fn intrabc_debug_state(
        &self,
    ) -> Option<(TileIntrabcPreludeState, Vec<Option<MotionVector>>)> {
        let state = self.intrabc.as_ref()?;
        let units = (0..self.mi_rows).flat_map(|r| (0..self.mi_cols).map(move |c| (r, c)));
        let mvs = units.map(|(r, c)| self.grid.intrabc_mv_at(r, c)).collect();
        Some((state.clone(), mvs))
    }
}

const INTRA_ONLY: crate::DecodeHeaderStateError =
    crate::DecodeHeaderStateError::InvalidInterBlockModeState;
