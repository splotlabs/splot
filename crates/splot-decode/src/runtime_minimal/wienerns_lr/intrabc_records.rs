// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded IntrABC syntax handoff for the ac0ej3 selectable transform-record frontier.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{DrlReorder, SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;
use splot_recon::{PlaneRect, PlaneSize};

use crate::error::Result;
use crate::runtime_minimal::inter::mv_scaling::{PlaneScaling, derive_plane_scaling};
use crate::tile_payload::{DecodeBlockFrontier, TileCdfSelector, TileCdfSubset};

use super::super::inter::{
    Mv,
    read_mv::{
        MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL, MvReadConfig, mv_clamp_to_integer,
        read_newmv_block_mvd_with_config,
    },
};
use super::intrabc_ref_mv_stack::{
    DrlReorderMode, IntrabcRefMvBank, IntrabcStackAdmission, IntrabcStackGeometry,
    SpatialIntrabcScan, SpatialScanGeometry, intrabc_ref_stack_admission, spatial_intrabc_scan,
};
use super::recon::WienerNsLrReconSink;
use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;

/// Result of the §5.20.5.3 `use_intrabc` / `read_skip` prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcUseSkip {
    pub(super) use_intrabc: bool,
    pub(super) skip_flag: bool,
}

/// Luma/shared mode-info facts retained by the transform-record handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockPrelude {
    pub(super) use_intrabc: bool,
    pub(super) is_inter: bool,
    pub(super) skip_flag: bool,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "retained IntrABC mode-info facts are consumed by tests until prediction uses them"
        )
    )]
    pub(super) intrabc: Option<IntrabcInfo>,
}

impl IntrabcBlockPrelude {
    pub(super) const fn from_use_skip(
        use_skip: IntrabcUseSkip,
        intrabc: Option<IntrabcInfo>,
    ) -> Self {
        Self {
            use_intrabc: use_skip.use_intrabc,
            is_inter: use_skip.use_intrabc,
            skip_flag: use_skip.skip_flag,
            intrabc,
        }
    }
}

/// Minimal block facts needed by the local IntrABC syntax handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockContext {
    row: usize,
    col: usize,
    b_size: usize,
    is_chroma_part: bool,
}

impl IntrabcBlockContext {
    pub(super) fn from_frontier(frontier: &DecodeBlockFrontier) -> Self {
        Self {
            row: frontier.r,
            col: frontier.c,
            b_size: frontier.b_size.index(),
            is_chroma_part: frontier.is_chroma_part(),
        }
    }

    #[cfg(test)]
    const fn new(row: usize, col: usize, b_size: usize, is_chroma_part: bool) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
        }
    }
}

/// Current block geometry for §5.20.5.3 IntrABC context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockGeometry {
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
}

impl IntrabcBlockGeometry {
    pub(super) fn from_frontier(frontier: &DecodeBlockFrontier, n4w: usize, n4h: usize) -> Self {
        Self {
            block: IntrabcBlockContext::from_frontier(frontier),
            n4w,
            n4h,
        }
    }

    #[cfg(test)]
    const fn new(block: IntrabcBlockContext, n4w: usize, n4h: usize) -> Self {
        Self { block, n4w, n4h }
    }
}

/// Bounded §5.20.5.4 facts that can be parsed without current-frame prediction.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained IntrABC mode-info facts are consumed by tests until prediction uses them"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcInfo {
    pub(super) intrabc_mode: u8,
    pub(super) ref_mv_idx: usize,
    pub(super) mv_precision: u8,
    pub(super) block_mv: IntrabcBlockVector,
}

/// Retained IntrABC block vector in eighth-pel units.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "retained IntrABC block-vector facts are consumed by tests until prediction uses them"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcBlockVector {
    pub(super) row: i32,
    pub(super) col: i32,
}

/// Checked luma current-frame copy geometry derived from an IntrABC block vector.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "live ac0ej3 path stops at the missing CurrFrame frontier until samples are populated"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcPredictionGeometry {
    pub(super) scaling: PlaneScaling,
    pub(super) source: PlaneRect,
    pub(super) target: PlaneRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcInfoSyntax {
    intrabc_mode: usize,
    ref_mv_idx: usize,
    mv_precision: u8,
    max_bvp_drl_bits_minus_1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcLumaPredictionDomain {
    storage: PlaneSize,
    tile_bounds: PlaneRect,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
}

impl From<Mv> for IntrabcBlockVector {
    fn from(value: Mv) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

impl From<IntrabcBlockVector> for Mv {
    fn from(value: IntrabcBlockVector) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

/// Tile-local neighbour state for IntrABC and skip contexts used by §8.3.2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TileIntrabcPreludeState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    values: Vec<Option<IntrabcBlockFacts>>,
    /// AV2 § 7.12.2 IntrABC reference-MV bank (the intra list), maintained in
    /// decode order so a later IntrABC block sees the bank state AVM holds.
    bank: IntrabcRefMvBank,
    /// AV2 sequence-header `enable_refmvbank` (§ 5.5.2 / `SequenceInterConfig`).
    /// When `0`, AV2 runs neither the § 7.12.2.21 ref-MV-bank fill nor the
    /// block-end bank update, so the stack is spatial-scan + default-fill only.
    enable_refmvbank: bool,
    /// AV2 § 5.4.6 `DrlReorder` mode (`SequenceInterConfig::drl_reorder`), gating the
    /// § 7.12.2.19 nearest-prefix max-weight sort. ac0ej3 is `DRL_REORDER_ALWAYS`.
    drl_reorder: DrlReorderMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockFacts {
    use_intrabc: bool,
    skip_flag: bool,
    /// The recorded block vector (eighth-pel) for an IntrABC leaf, used by the
    /// § 7.12.2 spatial scan; `None` for a non-IntrABC block.
    block_mv: Option<IntrabcBlockVector>,
}

impl TileIntrabcPreludeState {
    pub(super) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sequence: &SequenceHeader,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let values_len = mi_rows.checked_mul(mi_cols).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_grid_overflow",
            )
        })?;
        let sb_size4 = intra_sb_size4(sequence, tile_offset)?;
        let enable_refmvbank = sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank);
        // Map the § 5.4.6 `DrlReorder` mode (inferred `DRL_REORDER_DISABLED` when the
        // sequence has no inter config) into the decode-internal `DrlReorderMode`.
        let drl_reorder = match sequence.inter.as_ref().map(|inter| inter.drl_reorder) {
            Some(DrlReorder::Always) => DrlReorderMode::Always,
            Some(DrlReorder::Constraint) => DrlReorderMode::Constraint,
            Some(DrlReorder::Disabled) | None => DrlReorderMode::Disabled,
        };
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4,
            values: vec![None; values_len],
            bank: IntrabcRefMvBank::new(sb_size4),
            enable_refmvbank,
            drl_reorder,
        })
    }

    pub(super) fn record_block(
        &mut self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        prelude: IntrabcBlockPrelude,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let block_mv = prelude.intrabc.map(|info| info.block_mv);
        let facts = IntrabcBlockFacts {
            use_intrabc: prelude.use_intrabc,
            skip_flag: prelude.skip_flag,
            block_mv,
        };
        let row_end = row.checked_add(n4h).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_row_overflow",
            )
        })?;
        let col_end = col.checked_add(n4w).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_col_overflow",
            )
        })?;
        // §5.20.6.1 fills the mode-info grid over the block's NOMINAL MI footprint
        // (`row..row_end`, `col..col_end`) with NO MiRows/MiCols clamp; a leaf
        // straddling the frame edge (a partial bottom/right SB row) keeps its nominal
        // size. The out-of-frame MI cells are dropped DOWNSTREAM by the §5.20.3.2
        // `block_coded(r,c) { return r < MiRows && c < MiCols }`
        // (05-syntax-structures.md:9621). Model that frame-edge drop here: skip cells
        // past the frame extent in the fill so a partial-SB MI row at the frame edge
        // records only its in-frame cells (leaving out-of-frame cells `None`) instead
        // of erroring `..._intrabc_block_bounds`. The `checked_add` guards above stay:
        // they catch genuine usize overflow, not frame edges.
        for r in row..row_end {
            if r >= self.mi_rows {
                break;
            }
            for c in col..col_end {
                if c >= self.mi_cols {
                    break;
                }
                let index = self.index(r, c, tile_offset)?;
                self.values[index] = Some(facts);
            }
        }
        // AV2 § 7.12.2 `av2_read_mode_info` POST-block bank maintenance: feed the
        // block (IBC or not) into the ref-MV bank in decode order. The SB-row reset
        // already ran at block entry in [`Self::prepare_for_block`]. The bank footprint
        // uses the block's NOMINAL `n4w`/`n4h`, NOT the frame-clamped extent:
        // `decide_rmb_unit_update_count` (`mvref_common.c:4589`) derives the rmb-unit
        // count from `bsize = mbmi->sb_type` (the nominal block size), so clamping it
        // would desync the §7.12.2 remain_hits budget. The per-MI grid IS clipped by
        // §5.20.3.2 `block_coded`; the bank is NOT — this contrast is load-bearing.
        if self.enable_refmvbank {
            self.bank.update_after_block(
                row,
                col,
                n4w,
                n4h,
                prelude.use_intrabc,
                block_mv.map(Mv::from),
            );
        }
        Ok(())
    }

    /// AV2 § 7.12.2 `av2_reset_refmv_bank` per-superblock-row reset, run at block
    /// ENTRY (before the § 7.12.2.21 fill reads the bank for admission) so the first
    /// block of a new superblock row reads a freshly-zeroed bank, mirroring the
    /// `av2_zero(xd->ref_mv_bank)` at `decodeframe.c:4639` which runs before the
    /// row's blocks decode. A no-op when `enable_refmvbank == 0`.
    pub(super) fn prepare_for_block(&mut self, row: usize, col: usize) {
        if self.enable_refmvbank {
            self.bank.enter_block_superblock(row, col);
        }
    }

    pub(super) fn intrabc_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let mut ctx = 0usize;
        for (r, c) in self.neighbor_positions(row, col, n4w, n4h, true, tile_offset)? {
            if self
                .value(r, c, tile_offset)?
                .is_some_and(|facts| facts.use_intrabc)
            {
                ctx += 1;
            }
        }
        Ok(ctx.min(INTRABC_CONTEXT_MAX))
    }

    fn skip_ctx(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        tile_offset: ByteOffset,
    ) -> Result<usize> {
        let mut ctx = 0usize;
        for (r, c) in self.neighbor_positions(row, col, n4w, n4h, false, tile_offset)? {
            if self
                .value(r, c, tile_offset)?
                .is_some_and(|facts| facts.skip_flag)
            {
                ctx += 1;
            }
        }
        Ok(ctx.min(SKIP_CONTEXT_MAX))
    }

    fn neighbor_positions(
        &self,
        row: usize,
        col: usize,
        n4w: usize,
        n4h: usize,
        same_sb_row: bool,
        tile_offset: ByteOffset,
    ) -> Result<Vec<(usize, usize)>> {
        if n4w == 0 || n4h == 0 {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_empty_block",
            ));
        }
        let bottom_row = row.checked_add(n4h - 1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_neighbor_row_overflow",
            )
        })?;
        let right_col = col.checked_add(n4w - 1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_neighbor_col_overflow",
            )
        })?;
        let candidates = [
            col.checked_sub(1).map(|left_col| (bottom_row, left_col)),
            row.checked_sub(1).map(|above_row| (above_row, right_col)),
            col.checked_sub(1).map(|left_col| (row, left_col)),
            row.checked_sub(1).map(|above_row| (above_row, col)),
        ];
        let mut positions = Vec::new();
        for candidate in candidates.into_iter().flatten() {
            let (r, c) = candidate;
            if r >= self.mi_rows || c >= self.mi_cols {
                continue;
            }
            if same_sb_row && r / self.sb_size4 != row / self.sb_size4 {
                continue;
            }
            positions.push(candidate);
            if positions.len() == 2 {
                break;
            }
        }
        Ok(positions)
    }

    fn value(
        &self,
        row: usize,
        col: usize,
        tile_offset: ByteOffset,
    ) -> Result<Option<IntrabcBlockFacts>> {
        let index = self.index(row, col, tile_offset)?;
        Ok(self.values[index])
    }

    fn index(&self, row: usize, col: usize, tile_offset: ByteOffset) -> Result<usize> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_bounds",
            ));
        }
        row.checked_mul(self.mi_cols)
            .and_then(|start| start.checked_add(col))
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_intrabc_index_overflow",
                )
            })
    }

    /// Runs the AV2 § 7.12.2 spatial SMVP scan for the IntrABC block `geometry`,
    /// returning the ordered, deduped spatial neighbour block vectors plus a defer
    /// flag (see [`spatial_intrabc_scan`]). The scan reads each MI position's
    /// recorded IntrABC block vector from this tile's mode-info grid.
    fn spatial_intrabc_scan(&self, geometry: IntrabcBlockGeometry) -> SpatialIntrabcScan {
        let scan_geometry = SpatialScanGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            mi_rows: self.mi_rows,
            mi_cols: self.mi_cols,
            sb_size4: self.sb_size4,
        };
        spatial_intrabc_scan(
            scan_geometry,
            |row, col| self.block_vector_at(row, col),
            |row, col| self.is_mi_coded(row, col),
        )
    }

    /// The recorded block vector of the IntrABC block at MI `(row, col)`, or `None`
    /// when the position is out of range, not coded, or not an IntrABC block.
    fn block_vector_at(&self, row: usize, col: usize) -> Option<Mv> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return None;
        }
        let facts = self
            .values
            .get(row * self.mi_cols + col)
            .copied()
            .flatten()?;
        if !facts.use_intrabc {
            return None;
        }
        facts.block_mv.map(Mv::from)
    }

    /// AV2 § 7.12.2.6 `is_mi_coded` (`blockd.c:34` `av2_mark_block_as_coded`): whether
    /// MI `(row, col)` has been CODED earlier in decode order. The `values` grid is
    /// filled per-MI for EVERY recorded block (IBC or not) at record time, AFTER its
    /// ref-MV stack is built, so a `Some` entry here means the MI is coded and the
    /// current block (not yet recorded) is excluded — matching AVM, which marks the
    /// block coded only after building its stack. Used by the § 7.12.2.6
    /// `has_top_right` per-4x4 availability gate.
    fn is_mi_coded(&self, row: usize, col: usize) -> bool {
        if row >= self.mi_rows || col >= self.mi_cols {
            return false;
        }
        self.values
            .get(row * self.mi_cols + col)
            .copied()
            .flatten()
            .is_some()
    }

    /// The tile ref-MV bank (the § 7.12.2 IntrABC bank state as of the last
    /// recorded block).
    fn bank(&self) -> &IntrabcRefMvBank {
        &self.bank
    }
}

pub(super) fn read_intrabc_use_and_skip(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcUseSkip> {
    let block = geometry.block;
    let n4w = geometry.n4w;
    let n4h = geometry.n4h;
    if !intrabc_use_is_coded(core, block, n4w, n4h) {
        return Ok(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: false,
        });
    }
    let intrabc_ctx = state.intrabc_ctx(block.row, block.col, n4w, n4h, tile_offset)?;
    let use_intrabc = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::Intrabc { ctx: intrabc_ctx },
        tile_offset,
    )? != 0;
    if !use_intrabc {
        return Ok(IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: false,
        });
    }
    let skip_ctx = state.skip_ctx(block.row, block.col, n4w, n4h, tile_offset)?;
    let skip_flag = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::Skip { ctx: skip_ctx },
        tile_offset,
    )? != 0;
    Ok(IntrabcUseSkip {
        use_intrabc: true,
        skip_flag,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read_intrabc_info(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    skip_flag: bool,
    sink: Option<&mut WienerNsLrReconSink<u16>>,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    let pred_mv =
        ensure_intrabc_ref_stack_supported(state, sequence, geometry, syntax, tile_offset)?;
    let info =
        finish_intrabc_info_record(cdfs, symbols, sequence, core, syntax, pred_mv, tile_offset)?;
    let prediction = derive_intrabc_luma_prediction_geometry(core, geometry, info, tile_offset)?;
    // §7.13.3.18 IntrABC luma prediction: with the block-vector geometry bounds-checked
    // above, an attached reconstruction sink copies the displaced predictor rectangle
    // from the partially-built `CurrFrame` (gated to the proven integer-vector skip
    // subset inside the sink). The §6.19.7.12 `is_mv_valid` conformance predicate must
    // ALSO hold before the copy: the geometry derivation proves the tile-edge clause,
    // and `intrabc_dv_proven_valid` proves the global-intrabc wavefront clause (the
    // local-IBC-buffer clause needs runtime buffer state splot does not track, so it is
    // conservatively deferred). An invalid (or not-provably-valid) DV defers the copy —
    // never marks an out-of-buffer reference bit-exact. The sink retains the
    // reconstructed IntrABC target for the region test; the PUBLIC decode threads no
    // sink, so it never copies a sample and still emits no frame.
    if let Some(sink) = sink
        && intrabc_dv_proven_valid(sequence, core, geometry, info, tile_offset)?
    {
        sink.reconstruct_intrabc_block(
            prediction.source,
            prediction.target,
            skip_flag,
            tile_offset,
        )?;
    }
    // §5.20.4 decode_block continuation. A `skip` IntrABC leaf carries NO residual,
    // and §5.20.6.1 read_block_tx_size for a skipped inter block (is_inter == 1 for
    // IntrABC, §5.20.5.3) assigns Max_Tx_Size_Rect with NO partition symbols (the
    // walk's `allow_select == !skip || !is_inter == false` max-rect branch). A NON-skip
    // IntrABC leaf instead reads its §5.20.6.1 inter tx-partition + §5.20.7.29 inter
    // transform-type + §5.20.7.27 coefficient residual via the SAME is_inter-aware
    // tx-record + coefficient machinery the partition walk drives after this prelude
    // returns. Returning the parsed mode-info for BOTH cases lets that machinery
    // advance the partition/superblock walk AVM-faithfully to the next leaf — the
    // entropy state after the block is exactly where AVM leaves it. For a non-skip
    // leaf the sink's `reconstruct_intrabc_block` wrote the displaced copy as the
    // §7.13.2 PREDICTION and recorded the target as pending; each §5.20.7.27 residual
    // transform leaf then adds its decoded residual onto that predictor inside the
    // sink's transform-record path (gated to the proven integer-DV / no-real-IST
    // subset), so the IntrABC reconstruction is prediction + residual, bit-exact.
    Ok(info)
}

#[cfg(test)]
fn read_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    use super::intrabc_ref_mv_stack::build_intrabc_ref_mv_stack;
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    // These geometry tests have no neighbours: the §7.12.2 stack is bank-empty +
    // spatial-empty, i.e. the §7.12.2.20 default fill, and `pred_mv` is its
    // `RefStackMv[ref_mv_idx]` (the same entry the live admission path selects).
    let stack_geometry = IntrabcStackGeometry {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        n4w: geometry.n4w,
        n4h: geometry.n4h,
        sb_samples: superblock_samples(sequence, tile_offset)?,
        frame_w: i32::MAX,
        frame_h: i32::MAX,
        max_bvp_drl_bits_minus_1: syntax.max_bvp_drl_bits_minus_1,
    };
    let stack = build_intrabc_ref_mv_stack(&IntrabcRefMvBank::new(0), stack_geometry, false, &[]);
    let pred_mv = *stack.get(syntax.ref_mv_idx).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx_out_of_range",
        )
    })?;
    finish_intrabc_info_record(cdfs, symbols, sequence, core, syntax, pred_mv, tile_offset)
}

fn read_intrabc_info_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfoSyntax> {
    let force_integer_mv = core.force_integer_mv.ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_mv_precision",
        )
    })?;
    let max_bvp_drl_bits_minus_1 = max_bvp_drl_bits_minus_1(sequence, core, tile_offset)?;
    let m = usize::try_from(max_bvp_drl_bits_minus_1)
        .ok()
        .and_then(|bits| bits.checked_add(1))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_drl_count",
            )
        })?;
    let intrabc_mode = read_symbol(cdfs, symbols, TileCdfSelector::IntrabcMode, tile_offset)?;
    if intrabc_mode > 1 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mode",
        ));
    }

    let mut ref_mv_idx = 0usize;
    for idx in 0..m {
        let drl_mode = symbols.read_literal(1).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_drl_read",
            )
        })?;
        if drl_mode == 0 {
            ref_mv_idx = idx;
            break;
        }
        ref_mv_idx = idx.checked_add(1).ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx",
            )
        })?;
    }

    let mut mv_precision = if force_integer_mv {
        MV_PRECISION_ONE_PEL
    } else {
        MV_PRECISION_QUARTER_PEL
    };
    if intrabc_mode == 0 && !force_integer_mv {
        let precision = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IntrabcPrecision,
            tile_offset,
        )?;
        if precision > 1 {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_precision",
            ));
        }
        mv_precision = if precision != 0 {
            MV_PRECISION_QUARTER_PEL
        } else {
            MV_PRECISION_ONE_PEL
        };
    }
    Ok(IntrabcInfoSyntax {
        intrabc_mode,
        ref_mv_idx,
        mv_precision,
        max_bvp_drl_bits_minus_1,
    })
}

/// AV2 § 7.12.2 IntrABC ref-MV stack admission gate.
///
/// Builds the real § 7.12.2 stack (the § 7.12.2 spatial SMVP scan + the
/// § 7.12.2.21 ref-MV-bank fill + the § 7.12.2.20 default block vectors) and, on
/// admit, returns the predictor block vector `RefStackMv[ref_mv_idx]` the decoded
/// DRL index selects, so [`assign_intrabc_mv`] uses the AVM-faithful candidate.
/// DEFERS when the § 7.12.2 spatial scan reaches a position this decoder does not
/// model faithfully, or when the DRL index lands outside the built stack.
fn ensure_intrabc_ref_stack_supported(
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    syntax: IntrabcInfoSyntax,
    tile_offset: ByteOffset,
) -> Result<Mv> {
    let stack_geometry = IntrabcStackGeometry {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        n4w: geometry.n4w,
        n4h: geometry.n4h,
        sb_samples: superblock_samples(sequence, tile_offset)?,
        frame_w: i32::try_from(state.mi_cols.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        frame_h: i32::try_from(state.mi_rows.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        max_bvp_drl_bits_minus_1: syntax.max_bvp_drl_bits_minus_1,
    };
    let spatial = state.spatial_intrabc_scan(geometry);
    match intrabc_ref_stack_admission(
        state.bank(),
        stack_geometry,
        &spatial,
        state.enable_refmvbank,
        state.drl_reorder,
        syntax.ref_mv_idx,
    ) {
        IntrabcStackAdmission::Admit { selected } => Ok(selected),
        IntrabcStackAdmission::Defer => Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    syntax: IntrabcInfoSyntax,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    // `pred_mv` is `RefStackMv[ref_mv_idx]` from the real §7.12.2 stack the live
    // path built (spatial scan + ref-MV bank + default fill); the assign path adds
    // the §5.20.5.4 MV delta to it (NEWMV) or uses it directly (NEARMV).
    let block_mv = assign_intrabc_mv(
        cdfs,
        symbols,
        syntax.intrabc_mode,
        syntax.mv_precision,
        pred_mv,
        tile_offset,
    )?;
    if core.frame_is_intra == Some(true)
        && core.allow_screen_content_tools == Some(true)
        && sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_bawp)
    {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_morph_pred",
        ));
    }
    Ok(IntrabcInfo {
        intrabc_mode: u8::try_from(syntax.intrabc_mode).unwrap_or(1),
        ref_mv_idx: syntax.ref_mv_idx,
        mv_precision: syntax.mv_precision,
        block_mv,
    })
}

fn assign_intrabc_mv(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    intrabc_mode: usize,
    mv_precision: u8,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcBlockVector> {
    let block_mv = if intrabc_mode == 0 {
        let diff = read_newmv_block_mvd_with_config(
            cdfs,
            symbols,
            tile_offset,
            MvReadConfig::intrabc(mv_precision),
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv",
            )
        })?;
        Mv {
            row: mv_clamp_to_integer(pred_mv.row + diff.row),
            col: mv_clamp_to_integer(pred_mv.col + diff.col),
        }
    } else {
        pred_mv
    };
    Ok(block_mv.into())
}

pub(super) fn derive_intrabc_luma_prediction_geometry(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<IntrabcPredictionGeometry> {
    let domain = intrabc_luma_prediction_domain(core, geometry, tile_offset)?;
    let width = geometry.n4w.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let height = geometry.n4h.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let target_x = geometry.block.col.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let target_y = geometry.block.row.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    // Clamp the nominal target rect to the visible region (storage ∩ tile_bounds),
    // modelling AVM §5.20.3.2 `block_coded(r,c) { r < MiRows && c < MiCols }`
    // (05-syntax-structures.md:9621): a bottom/right-edge block carries its full
    // nominal MI extent for §7.12.2 context/parse purposes but reconstructs and
    // stores only its in-frame samples. A genuinely off-frame block (top-left MI
    // outside the visible region) is still rejected — never an overhang.
    let target = intrabc_clamped_target(target_x, target_y, width, height, &domain, tile_offset)?;
    let source = intrabc_luma_source_envelope(target, info.block_mv, tile_offset)?;
    if !source.is_within(domain.storage) || !rect_is_within_rect(source, domain.tile_bounds) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    if rects_overlap(source, target) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity",
        ));
    }
    let scaling = derive_plane_scaling(
        i64::try_from(target_x).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::try_from(target_y).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::from(info.block_mv.row),
        i64::from(info.block_mv.col),
        0,
        0,
        domain.ref_mi_cols,
        domain.ref_mi_rows,
        i64::try_from(width).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        i64::try_from(height).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
    );
    Ok(IntrabcPredictionGeometry {
        scaling,
        source,
        target,
    })
}

/// Clamps a nominal IntrABC luma target rect to the visible region
/// (`domain.storage` ∩ `domain.tile_bounds`), modelling AVM §5.20.3.2 `block_coded`.
///
/// AVM's §5.20.3.2 `block_coded(r,c) { r < MiRows && c < MiCols }` carries a block's
/// full NOMINAL extent for parse/context, but reconstructs and stores only its
/// in-frame samples. A bottom/right-edge block whose nominal footprint overhangs the
/// cropped frame (the §6.19.7.12 `intrabc_target_bounds` frontier) therefore has an
/// EFFECTIVE in-frame target shrunk to `min(nominal_right, storage_right, tile_right)`
/// (symmetric for the bottom edge). This only ever SHRINKS the rect to the visible
/// intersection — it never widens what is copied.
///
/// The block's TOP-LEFT must itself be inside both the storage and the tile bounds
/// (`target_x >= tile_x`, `target_y >= tile_y`, and within storage): a genuinely
/// off-frame block (a top-left MI past the visible region) is rejected with the same
/// `intrabc_target_bounds` reason, since it has no visible samples to reconstruct.
fn intrabc_clamped_target(
    target_x: usize,
    target_y: usize,
    width: usize,
    height: usize,
    domain: &IntrabcLumaPredictionDomain,
    tile_offset: ByteOffset,
) -> Result<PlaneRect> {
    let tile_x = domain.tile_bounds.x();
    let tile_y = domain.tile_bounds.y();
    let tile_right = tile_x
        .checked_add(domain.tile_bounds.width())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_bottom = tile_y
        .checked_add(domain.tile_bounds.height())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    // The top-left corner must be in-frame: at/after the tile origin, before the
    // visible right/bottom edge, and within storage. A block whose top-left MI is
    // itself off-frame is genuinely out of the visible region (not an overhang).
    let nominal_right = target_x.checked_add(width).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let nominal_bottom = target_y.checked_add(height).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let visible_right = nominal_right.min(domain.storage.width()).min(tile_right);
    let visible_bottom = nominal_bottom.min(domain.storage.height()).min(tile_bottom);
    if target_x < tile_x
        || target_y < tile_y
        || target_x >= domain.storage.width()
        || target_y >= domain.storage.height()
        || target_x >= visible_right
        || target_y >= visible_bottom
    {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        ));
    }
    // `visible_right > target_x` and `visible_bottom > target_y` are guaranteed by the
    // guard above, so the effective extents are positive (no zero-dimension error).
    let eff_width = visible_right - target_x;
    let eff_height = visible_bottom - target_y;
    PlaneRect::new(target_x, target_y, eff_width, eff_height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
}

fn intrabc_luma_prediction_domain(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcLumaPredictionDomain> {
    // Keep the missing-frame-size diagnostic distinct, but derive the luma
    // storage domain from the padded MI sentinels used by §7.13.3.18.
    let _frame_size = core.frame_size.ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size",
        )
    })?;
    let tile_info = core.tile_info.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let mi_cols = tile_info.mi_col_starts.last().copied().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let mi_rows = tile_info.mi_row_starts.last().copied().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    if mi_cols == 0 || mi_rows == 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        ));
    }
    let width = usize::try_from(mi_cols)
        .ok()
        .and_then(|cols| cols.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let height = usize::try_from(mi_rows)
        .ok()
        .and_then(|rows| rows.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let storage = PlaneSize::new(width, height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let (tile_col_start, tile_col_end) = tile_interval_for_block(
        &tile_info.mi_col_starts,
        geometry.block.col,
        geometry.n4w,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let (tile_row_start, tile_row_end) = tile_interval_for_block(
        &tile_info.mi_row_starts,
        geometry.block.row,
        geometry.n4h,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let tile_x = tile_col_start.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let tile_y = tile_row_start.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let tile_width = tile_col_end
        .checked_sub(tile_col_start)
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_height = tile_row_end
        .checked_sub(tile_row_start)
        .and_then(|value| value.checked_mul(MI_SIZE))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let tile_bounds = PlaneRect::new(tile_x, tile_y, tile_width, tile_height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    Ok(IntrabcLumaPredictionDomain {
        storage,
        tile_bounds,
        ref_mi_cols: i64::from(mi_cols),
        ref_mi_rows: i64::from(mi_rows),
    })
}

/// Resolves AV2 §5.18.3.4 `allow_local_intrabc` for an IntrABC-enabled frame, honoring
/// the inference rules.
///
/// Per §5.18.3.4 `intrabc_params()`, when `allow_intrabc == 1`: for an intra frame,
/// `allow_local_intrabc` is read only when `allow_global_intrabc == 1` and is otherwise
/// INFERRED `1` (the `else { allow_local_intrabc = 1 }` branch); for an inter frame,
/// `allow_global_intrabc = 0` and `allow_local_intrabc = 1` (both inferred). The parser
/// stores an inferred value as `None`, so `Some(false)` is the only value that disables
/// the local branch — `Some(true)` and `None` (inferred `1`) both enable it. A frame
/// without `allow_intrabc` has no IntrABC blocks, so this is only reached for one.
fn resolve_allow_local_intrabc(core: &FrameHeaderCore) -> bool {
    core.intrabc
        .as_ref()
        .is_some_and(|params| params.allow_intrabc && params.allow_local_intrabc != Some(false))
}

/// Resolves AV2 §5.18.3.4 `allow_global_intrabc` for an IntrABC-enabled frame.
///
/// Per §5.18.3.4 `intrabc_params()`, `allow_global_intrabc` is READ (an `f(1)` flag) only
/// for an intra frame with `allow_intrabc == 1`; for an inter frame it is INFERRED `0`.
/// Unlike `allow_local_intrabc`, it is never inferred to `1`, so the parser stores the
/// active value as `Some(true)`/`Some(false)` and an inferred `0` as `None`. The §7.13.3.18
/// global wavefront branch is therefore only enabled when the flag is explicitly
/// `Some(true)` — `None` (inferred `0` on a non-intra frame) and `Some(false)` both disable
/// it. The matching `frame_is_intra` clause of `av2_is_dv_valid` is checked by the caller.
fn resolve_allow_global_intrabc(core: &FrameHeaderCore) -> bool {
    core.intrabc
        .as_ref()
        .is_some_and(|params| params.allow_intrabc && params.allow_global_intrabc == Some(true))
}

/// AV2 §6.19.7.12 `is_mv_valid` for the bounded ac0ej3 IntrABC subset, proven via the
/// `allow_local_intrabc` local-IBC-buffer branch NARROWED to same-superblock sources.
///
/// §6.19.7.12 first rejects a block vector whose displaced source leaves the current
/// tile (already enforced before this is called by the source/target tile-bounds
/// checks in [`derive_intrabc_luma_prediction_geometry`]). It then takes the
/// `allow_local_intrabc` branch (`av2_is_dv_in_local_range`), whose constraints split
/// into a DETERMINISTIC geometry part and a RUNTIME `IBCCoded` / `IBCBufferValid`
/// collocation part (`check_valid_local_ibc`) that depends on per-sample IBC-buffer
/// state splot does not track.
///
/// This predicate admits ONLY a same-superblock source ([`local_intrabc_range_valid`]):
/// a source in a PREVIOUS superblock can survive `av2_is_dv_in_local_range`'s left-buffer
/// window yet be rejected by §6.19.7.12 `check_valid_local_ibc` on a 64x64 IBC-buffer
/// collocation collision, so narrowing to the current superblock keeps the gate
/// fail-closed against that runtime case. The remaining runtime requirement (the source
/// is coded/valid) is subsumed by the caller's STRONGER guarantee
/// ([`super::recon::WienerNsLrReconSink::reconstruct_intrabc_block`]): the entire source
/// rectangle is already RECONSTRUCTED by this sink in decode order. It returns `false`
/// (DEFER — over-rejecting is safe) for everything else: a non-integer block vector,
/// `allow_local_intrabc != 1` (resolving the §5.18.3.4 inference), or a source outside
/// the active superblock. ac0ej3's first IntrABC block (128x128 SB, integer DV, source
/// in the SAME superblock as the active block) is proven valid by exactly this branch —
/// verified against AVM `av2_is_dv_valid` / `av2_is_dv_in_local_range`
/// (`av2/common/mvref_common.h`).
fn intrabc_dv_proven_valid(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<bool> {
    // A non-integer block vector is deferred upstream (the source/target shapes differ
    // in the sink); the conformance predicate only proves the integer-copy subset.
    if info.block_mv.row & 7 != 0 || info.block_mv.col & 7 != 0 {
        return Ok(false);
    }
    if !resolve_allow_local_intrabc(core) {
        return Ok(false);
    }
    // §6.19.7.12 superblock size (samples). The same-superblock admission below never
    // uses the `numLeftActiveSB` left-buffer window, so the §6.19.7.12 64x64-tier BRU
    // reduction (the only local-range term needing runtime SB-active state) cannot
    // affect it — no BRU gate is needed for the same-SB subset.
    let sb_samples = superblock_samples(sequence, tile_offset)?;
    let sb_size = usize::try_from(sb_samples).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let block_w = geometry.n4w * MI_SIZE;
    let block_h = geometry.n4h * MI_SIZE;
    // `av2_is_dv_valid` (`av2/common/mvref_common.h`) tries the `allow_local_intrabc`
    // local-IBC-buffer branch FIRST and returns valid on a hit (mvref_common.h:927-951),
    // then — for an intra-only frame with `allow_global_intrabc` — falls through to the
    // GLOBAL wavefront branch (mvref_common.h:956-993). Mirror that order: try the same-SB
    // local subset first, then the §7.13.3.18 global wavefront branch gated on an
    // intra-only frame AND an explicitly-read `allow_global_intrabc` (resolved by
    // [`resolve_allow_global_intrabc`]). Both clauses are deterministic geometry checks
    // that only ADMIT a source whose entire rectangle is already coded in decode order; an
    // admitted copy is per-sample bit-exact vs the AVM pre-filter oracle. Over-rejecting is
    // always safe (a deferred copy never claims a wrong sample), so a non-{64,128,256} SB
    // size or missing tile info fails the global clause closed.
    let local_valid = local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w,
        block_h,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size,
    });
    if local_valid {
        return Ok(true);
    }
    // The matching `frame_is_intra` clause of `av2_is_dv_valid` (mvref_common.h:954) plus
    // the explicit `allow_global_intrabc`. `None` (inferred 0 on a non-intra frame) and
    // `Some(false)` both disable the global branch — only an explicitly-read `Some(true)`
    // on an intra-only frame enables it.
    if core.frame_is_intra != Some(true) || !resolve_allow_global_intrabc(core) {
        return Ok(false);
    }
    // The global wavefront branch needs the TRUE §5.18.7.6 superblock size (it must NOT
    // collapse the 256 tier to 128 the way `superblock_samples` does — the `gradient` /
    // `mib_size_log2` terms depend on the real size) and the block's tile 64x64-column
    // extent (`total_sb64_per_row`).
    let global_sb_size = global_superblock_samples(sequence, tile_offset)?;
    let total_sb64_per_row = intrabc_tile_total_sb64_per_row(core, geometry, tile_offset)?;
    Ok(global_intrabc_range_valid(IntrabcGlobalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w,
        block_h,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size: global_sb_size,
        total_sb64_per_row,
    }))
}

/// The TILE `total_sb64_per_row` of `av2_is_dv_valid` (`av2/common/mvref_common.h:984`):
/// `(((mi_col_end - mi_col_start - 1) >> mi_size_high_log2[BLOCK_64X64]) + 1)`, the count
/// of 64x64 columns spanning the block's tile. Derived from the block's tile interval
/// (single-tile ac0ej3 spans the whole frame) so a multi-tile frame uses the correct
/// per-tile extent.
fn intrabc_tile_total_sb64_per_row(
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<i64> {
    let tile_info = core.tile_info.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let (tile_col_start, tile_col_end) = tile_interval_for_block(
        &tile_info.mi_col_starts,
        geometry.block.col,
        geometry.n4w,
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        tile_offset,
    )?;
    let tile_mi_cols =
        i64::try_from(tile_col_end.saturating_sub(tile_col_start)).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    if tile_mi_cols <= 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        ));
    }
    // mi_size_high_log2[BLOCK_64X64] == 4 (a 64x64 SB is 16 MI tall/wide).
    Ok(((tile_mi_cols - 1) >> 4) + 1)
}

/// True §5.18.7.6 superblock sample size (64/128/256) for the global wavefront branch.
///
/// Distinct from [`superblock_samples`], which caps the 256x256 tier to 128 (the
/// same-SB local subset never needs the true 256 size); the global branch's
/// `mib_size_log2`, `gradient`, and `sb_64_residual` terms all depend on the REAL SB
/// size, so it must not collapse 256 to 128.
fn global_superblock_samples(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 => 128,
        SuperblockSize::Block256x256 => 256,
    })
}

/// Inputs to the deterministic §6.19.7.12 local-intrabc range check (sample / MI-unit
/// terms; the block vector is in eighth-pel units).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcLocalRangeInputs {
    mi_row: usize,
    mi_col: usize,
    block_w: usize,
    block_h: usize,
    dv_row: i32,
    dv_col: i32,
    sb_size: usize,
}

/// The DETERMINISTIC geometry constraints of AV2 §6.19.7.12 `av2_is_dv_in_local_range`
/// (`av2/common/mvref_common.h`), for an integer block vector, NARROWED to the
/// same-superblock subset so the runtime `check_valid_local_ibc` 64x64 IBC-buffer
/// collocation cannot reject a source this gate admits.
///
/// `av2_is_dv_in_local_range` admits a source in the current SB OR the left `numLeftSB`
/// SBs, but a source in a PREVIOUS 128x128 SB whose 64x64 `ibc_buffer_index` collides
/// with the current buffer position is then rejected by §6.19.7.12 `check_valid_local_ibc`
/// (`if (bufIdx == ibc_buffer_index(IBCBufferCurRow, IBCBufferCurCol)) { ... if
/// (IBCCoded[colo]) return 0 }`) — runtime buffer state this gate does not track. To stay
/// fail-closed, this predicate admits ONLY a source whose superblock equals the active
/// block's superblock: the uncoded-bottom-right exclusion, the same-superblock-row
/// constraint, AND the source rectangle fully within the active block's superblock (a
/// stricter superset of the `valid_SB` window that excludes every previous-SB
/// buffer-collision case). A reconstructed same-SB source cannot collide with the
/// current 64x64 buffer slot in a way `check_valid_local_ibc` rejects, so the caller's
/// "source fully reconstructed" proof safely subsumes the remaining runtime part.
/// ac0ej3's first IntrABC block (MI(16,56), DV(-64,0), source x[224,256) y[0,64)) is a
/// same-SB source (act SB col 1, source SB col 1) and stays admitted.
fn local_intrabc_range_valid(inputs: IntrabcLocalRangeInputs) -> bool {
    let bw = inputs.block_w as i64;
    let bh = inputs.block_h as i64;
    let dv_row = i64::from(inputs.dv_row);
    let dv_col = i64::from(inputs.dv_col);
    // sb_size_log2 from the superblock sample size (64 -> 6, 128 -> 7, 256 -> 8).
    let sb_size_log2 = match inputs.sb_size {
        64 => 6,
        128 => 7,
        256 => 8,
        _ => return false,
    };

    let src_top_edge = (inputs.mi_row as i64 * MI_SIZE as i64) * 8 + dv_row;
    let src_left_edge = (inputs.mi_col as i64 * MI_SIZE as i64) * 8 + dv_col;
    let src_bottom_edge = (inputs.mi_row as i64 * MI_SIZE as i64 + bh) * 8 + dv_row;
    let src_right_edge = (inputs.mi_col as i64 * MI_SIZE as i64 + bw) * 8 + dv_col;
    // Integer DV: no interp border, and the `-1` rounding on the bottom/right edges.
    let src_top_y = src_top_edge >> 3;
    let src_left_x = src_left_edge >> 3;
    let src_bottom_y = (src_bottom_edge >> 3) - 1;
    let src_right_x = (src_right_edge >> 3) - 1;
    let act_left_x = inputs.mi_col as i64 * MI_SIZE as i64;
    let act_top_y = inputs.mi_row as i64 * MI_SIZE as i64;

    // Reference block cannot be in the uncoded bottom-right region of the current
    // block's top-left corner (integer DV: no interp borders).
    if ((dv_col >> 3) + bw) > 0 && ((dv_row >> 3) + bh) > 0 {
        return false;
    }
    // The whole source rectangle must lie in the SAME superblock as the active block
    // (row AND column). This is stricter than `av2_is_dv_in_local_range`'s
    // current-or-left-`numLeftSB`-SBs window: it excludes every previous-SB source,
    // which is exactly the set §6.19.7.12 `check_valid_local_ibc` may then reject on a
    // 64x64 IBC-buffer collocation collision (runtime state this gate cannot prove).
    let act_sb_col = act_left_x >> sb_size_log2;
    let act_sb_row = act_top_y >> sb_size_log2;
    (src_left_x >> sb_size_log2) == act_sb_col
        && (src_right_x >> sb_size_log2) == act_sb_col
        && (src_top_y >> sb_size_log2) == act_sb_row
        && (src_bottom_y >> sb_size_log2) == act_sb_row
}

/// Inputs to the deterministic §7.13.3.18 global-intrabc wavefront range check (sample /
/// MI-unit terms; the block vector is in eighth-pel units, `sb_size` the TRUE 64/128/256
/// superblock sample size, `total_sb64_per_row` the block's tile 64x64-column extent).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcGlobalRangeInputs {
    mi_row: usize,
    mi_col: usize,
    block_w: usize,
    block_h: usize,
    dv_row: i32,
    dv_col: i32,
    sb_size: usize,
    total_sb64_per_row: i64,
}

/// AVM `INTRABC_DELAY_PIXELS / 64` (`av2/common/mvref_common.h:610`): the wavefront SB64
/// delay of the global IntrABC reference window.
const INTRABC_DELAY_SB64: i64 = 4;
/// `mi_size_wide_log2[BLOCK_64X64]` / `mi_size_high_log2[BLOCK_64X64]` == 4 (a 64x64 SB is
/// 16 MI wide/tall, `common_data.h`).
const LOG2_MI_PER_64: i64 = 4;
/// `MI_SIZE_LOG2` == 2 (`enums.h:162`).
const MI_SIZE_LOG2: i64 = 2;

/// The GLOBAL wavefront branch of AV2 §7.13.3.18 / AVM `av2_is_dv_valid`
/// (`av2/common/mvref_common.h:954-993`), for an INTEGER block vector on an INTRA-ONLY
/// frame with `allow_global_intrabc`, modelled in i64.
///
/// This is the path AVM takes after the `allow_local_intrabc` local branch misses
/// (mvref_common.h:951) and the frame is intra-only with global IntrABC enabled. It admits
/// a source in the already-coded top-left wavefront region of the frame: the source's
/// bottom-right 64x64 raster cell must lead the active block's by more than the
/// `INTRABC_DELAY_SB64` delay, and the source-vs-active SB64 column gap must respect the
/// per-SB-row wavefront `gradient`. The block vector is integer here (a fractional DV is
/// deferred upstream), so the `IBC_*_INTERP_BORDER` terms are all zero and the bottom/right
/// edges keep the `-1` integer rounding of [`local_intrabc_range_valid`] (mvref_common.h's
/// `(src_*_edge >> 3) - 1`).
///
/// `sb_64_residual` is the ONLY term needing the §5.20 superblock root partition
/// (`SB_HORZ_OR_QUAD` of a 128x128 SB at a bottom-left 64x64 position relaxes the horizon
/// by one SB64). splot does not retain `sb_root_partition_info`, so this models it
/// FAIL-CLOSED: `sb_64_residual == 0` (the strictest horizon) UNLESS the bottom-left-128
/// position is geometrically possible (`sb_size == 128` and the active block sits at the
/// bottom-left 64x64 quadrant of its 128x128 SB), in which case BOTH the `residual == 0`
/// and the relaxed `residual == -1` horizons must admit the source — never admit a DV any
/// partition value would reject. Whether the active block's partition is actually
/// `SB_HORZ_OR_QUAD` only ever WIDENS AVM's admission, so requiring the stricter horizon
/// can over-reject (safe: a deferred copy never writes a wrong sample) but never
/// over-admits.
fn global_intrabc_range_valid(inputs: IntrabcGlobalRangeInputs) -> bool {
    let bw = inputs.block_w as i64;
    let bh = inputs.block_h as i64;
    let dv_row = i64::from(inputs.dv_row);
    let dv_col = i64::from(inputs.dv_col);
    let mi_row = inputs.mi_row as i64;
    let mi_col = inputs.mi_col as i64;
    let mi = i64::from(MI_SIZE as u32);
    // `mib_size_log2` is the MI-tier log2 of the SB (64 -> 4, 128 -> 5, 256 -> 6); the
    // matching `sb_size` sample size is `(1 << mib_size_log2) * MI_SIZE`.
    let mib_size_log2: i64 = match inputs.sb_size {
        64 => 4,
        128 => 5,
        256 => 6,
        _ => return false,
    };
    let sb_size = i64::from(inputs.sb_size as u32);

    // Integer DV: no interp border, and the `-1` integer rounding on the bottom/right
    // source edges, exactly as `av2_is_dv_valid` derives them (mvref_common.h:956-993).
    let src_bottom_edge = (mi_row * mi + bh) * 8 + dv_row;
    let src_right_edge = (mi_col * mi + bw) * 8 + dv_col;

    let active_sb_row = mi_row >> mib_size_log2;
    let active_sb64_col = mi_col >> LOG2_MI_PER_64;
    let src_sb_row = ((src_bottom_edge >> 3) - 1) >> (mib_size_log2 + MI_SIZE_LOG2);
    let src_sb64_col = ((src_right_edge >> 3) - 1) >> (LOG2_MI_PER_64 + MI_SIZE_LOG2);
    // `active_sb64_row` is the 64x64-tier row of the active block (sample-major).
    let active_sb64_row = (mi_row * mi) >> (LOG2_MI_PER_64 + MI_SIZE_LOG2);

    // The `wavefront` raster index mixes the SB row stride (`total_sb64_per_row`, 64x64
    // columns) with the SB128/256-tier `active_sb_row` exactly as AVM does.
    let active_sb64 = active_sb_row * inputs.total_sb64_per_row + active_sb64_col;
    let src_sb64 = src_sb_row * inputs.total_sb64_per_row + src_sb64_col;

    let gradient = 1 + INTRABC_DELAY_SB64 + i64::from(sb_size > 64) + 2 * i64::from(sb_size > 128);
    let wf_offset = gradient * (active_sb_row - src_sb_row);

    // `sb_64_residual` fail-closed: 0 unless the bottom-left-128 position is possible.
    let is_bottom_left = sb_size == 128 && (active_sb64_col & 1) == 0 && (active_sb64_row & 1) == 1;
    // The set of residual values any §5.20 root partition could legally produce here.
    // For the bottom-left-128 case AVM's `SB_HORZ_OR_QUAD` partition yields `-1` and
    // every other partition yields `0`; without partition state both must admit.
    let residuals: &[i64] = if is_bottom_left { &[0, -1] } else { &[0] };
    residuals.iter().all(|&sb_64_residual| {
        if src_sb64 >= active_sb64 - INTRABC_DELAY_SB64 - sb_64_residual {
            return false;
        }
        if src_sb_row > active_sb_row
            || src_sb64_col >= active_sb64_col - INTRABC_DELAY_SB64 - sb_64_residual + wf_offset
        {
            return false;
        }
        true
    })
}

fn tile_interval_for_block(
    starts: &[u32],
    block_start: usize,
    block_len: usize,
    bounds_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    // Overflow guard on the nominal MI extent (kept verbatim): a block whose nominal
    // MI span overflows `usize` is malformed geometry. The end itself is no longer a
    // containment bound — a bottom/right-edge block's nominal extent may overhang the
    // visible tile and is clamped to the visible region downstream
    // ([`intrabc_clamped_target`]), per AVM §5.20.3.2 `block_coded`.
    let block_end = block_start.checked_add(block_len).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let _ = block_end;
    for window in starts.windows(2) {
        let start = usize::try_from(window[0]).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
        let end = usize::try_from(window[1]).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
        // The block belongs to the interval whose MI range contains its TOP-LEFT MI
        // (§5.20.3.2 `block_coded` admits a block by its top-left corner; the nominal
        // extent may overhang the tile/frame bottom-right edge). The visible-region
        // clamp happens downstream — this interval supplies the tile bounds.
        if block_start >= start && block_start < end {
            return Ok((start, end));
        }
    }
    Err(wienerns_lr_selectable_transform_record_error_reason(
        tile_offset,
        bounds_reason,
    ))
}

fn rect_is_within_rect(rect: PlaneRect, bounds: PlaneRect) -> bool {
    let Some(rect_right) = rect.x().checked_add(rect.width()) else {
        return false;
    };
    let Some(rect_bottom) = rect.y().checked_add(rect.height()) else {
        return false;
    };
    let Some(bounds_right) = bounds.x().checked_add(bounds.width()) else {
        return false;
    };
    let Some(bounds_bottom) = bounds.y().checked_add(bounds.height()) else {
        return false;
    };
    rect.x() >= bounds.x()
        && rect.y() >= bounds.y()
        && rect_right <= bounds_right
        && rect_bottom <= bounds_bottom
}

fn rects_overlap(first: PlaneRect, second: PlaneRect) -> bool {
    let Some(first_right) = first.x().checked_add(first.width()) else {
        return true;
    };
    let Some(first_bottom) = first.y().checked_add(first.height()) else {
        return true;
    };
    let Some(second_right) = second.x().checked_add(second.width()) else {
        return true;
    };
    let Some(second_bottom) = second.y().checked_add(second.height()) else {
        return true;
    };
    first.x() < second_right
        && first_right > second.x()
        && first.y() < second_bottom
        && first_bottom > second.y()
}

fn intrabc_luma_source_envelope(
    target: PlaneRect,
    block_mv: IntrabcBlockVector,
    tile_offset: ByteOffset,
) -> Result<PlaneRect> {
    // IntrABC forces BILINEAR (§5.20.5.4). In §7.13.3.18 its non-zero taps are
    // the integer sample and the next sample on the fractional side, so no
    // top/left EIGHTTAP halo is part of this effective luma footprint.
    let bottom_border = usize::from(block_mv.row & 7 != 0);
    let right_border = usize::from(block_mv.col & 7 != 0);
    let delta_row = block_mv.row >> 3;
    let delta_col = block_mv.col >> 3;
    let source_x = i64::try_from(target.x())
        .ok()
        .and_then(|value| value.checked_add(i64::from(delta_col)))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    let source_y = i64::try_from(target.y())
        .ok()
        .and_then(|value| value.checked_add(i64::from(delta_row)))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?;
    if source_x < 0 || source_y < 0 {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds",
        ));
    }
    let source_width = target.width().checked_add(right_border).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    let source_height = target.height().checked_add(bottom_border).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    PlaneRect::new(
        usize::try_from(source_x).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        usize::try_from(source_y).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
            )
        })?,
        source_width,
        source_height,
    )
    .map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })
}

fn superblock_samples(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<i32> {
    let partition = sequence.partition.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config",
        )
    })?;
    Ok(match partition.seq_sb_size() {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 128,
    })
}

fn intrabc_use_is_coded(
    core: &FrameHeaderCore,
    block: IntrabcBlockContext,
    n4w: usize,
    n4h: usize,
) -> bool {
    core.allow_intrabc == Some(true)
        && !block.is_chroma_part
        && n4w <= 64 / MI_SIZE
        && n4h <= 64 / MI_SIZE
        && block.b_size != BLOCK_64X64
}

fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    tile_offset: ByteOffset,
) -> Result<usize> {
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(|symbol| usize::from(symbol.get()))
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_symbol_read",
            )
        })
}

fn max_bvp_drl_bits_minus_1(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<u32> {
    if let Some(max) = core
        .intrabc
        .as_ref()
        .and_then(|params| params.max_bvp_drl_bits_minus_1)
    {
        return Ok(max);
    }
    let inter = sequence.inter.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_missing_inter_config",
        )
    })?;
    Ok(inter.seq_max_bvp_drl_bits_minus_1)
}

fn intra_sb_size4(sequence: &SequenceHeader, tile_offset: ByteOffset) -> Result<usize> {
    Ok(match intra_capped_seq_sb_size(sequence, tile_offset)? {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 32,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "intrabc_records_tests.rs"]
mod tests;
