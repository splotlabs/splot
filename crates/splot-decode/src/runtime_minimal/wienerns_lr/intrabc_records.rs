// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded IntrABC syntax handoff for the ac0ej3 selectable transform-record frontier.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
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
    IntrabcRefMvBank, IntrabcStackGeometry, build_intrabc_ref_mv_stack,
};
use super::recon::WienerNsLrReconSink;
use super::{intra_capped_seq_sb_size, wienerns_lr_selectable_transform_record_error_reason};

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;
/// AVM `INTRABC_DELAY_PIXELS`; only the default-only test fallback reads it.
#[cfg(test)]
const INTRABC_DELAY_PIXELS: i32 = 256;

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
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4,
            values: vec![None; values_len],
            bank: IntrabcRefMvBank::new(sb_size4),
            enable_refmvbank,
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
        if row_end > self.mi_rows || col_end > self.mi_cols {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_bounds",
            ));
        }
        for r in row..row_end {
            for c in col..col_end {
                let index = self.index(r, c, tile_offset)?;
                self.values[index] = Some(facts);
            }
        }
        // AV2 § 7.12.2 `av2_read_mode_info` POST-block bank maintenance: feed the
        // block (IBC or not) into the ref-MV bank in decode order. The SB-row reset
        // already ran at block entry in [`Self::prepare_for_block`].
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

    /// Returns whether the AV2 § 7.12.2 spatial SMVP scan could contribute an
    /// IntrABC candidate to the current block's ref-MV stack.
    ///
    /// For `is_intrabc == 1`, § 7.12.2.6 Scan point / § 7.12.2.5 Scan col only add
    /// a candidate when the scanned neighbour is itself an IntrABC block
    /// (`add_ref_mv_candidate` requires `is_intrabc == is_intrabc_block(candidate)`,
    /// `mvref_common.c:834`). This decoder does not yet model the spatial-scan
    /// ordering/availability faithfully, so it scans a SUPERSET of the positions
    /// AV2 reads — every left-column, above-row, top-right, top-left and
    /// `scan_col(-3)` MI position adjacent to the block — and reports `true` if ANY
    /// holds an IntrABC neighbour. Over-reporting only forces a (safe) defer; it
    /// never produces a wrong stack. At an SB-row boundary (§ 7.12.2 `isSbBorder`)
    /// the § 7.12.2 step-14 above-left probe is `deltaCol = -1 - isSbBorder == -2`,
    /// then § 7.12.2.6 Scan point aligns it `deltaCol -= MiCol & 1`, so the leftmost
    /// above-row column extends to `MiCol - 2 - (MiCol & 1)`.
    fn spatial_scan_has_intrabc_neighbour(&self, geometry: IntrabcBlockGeometry) -> bool {
        let row = geometry.block.row;
        let col = geometry.block.col;
        let n4w = geometry.n4w;
        let n4h = geometry.n4h;
        // Left column (and the deeper scan_col deltaCol = -3): every MI row the
        // block spans, plus the bottom-left and top-left corners.
        let left_cols: [Option<usize>; 2] = [col.checked_sub(1), col.checked_sub(3)];
        let above_row = row.checked_sub(1);
        // Left/scan-col span: rows [row - 1 .. row + n4h] (top-left corner down to
        // bottom-left), bounded to the grid.
        for left_col in left_cols.into_iter().flatten() {
            let top = row.saturating_sub(1);
            let bottom = row.saturating_add(n4h); // inclusive of the bottom-left row
            for r in top..=bottom {
                if self.position_is_intrabc(r, left_col) {
                    return true;
                }
            }
        }
        // Above row: columns [leftmost .. col + n4w] (the step-14 / top-left corner
        // across to the top-right), where leftmost covers the SB-border probe.
        if let Some(above) = above_row {
            let is_sb_border = self.sb_size4 != 0 && row.is_multiple_of(self.sb_size4);
            let extra_left = if is_sb_border { 2 + (col & 1) } else { 1 };
            let leftmost = col.saturating_sub(extra_left);
            let rightmost = col.saturating_add(n4w); // inclusive of the top-right col
            for c in leftmost..=rightmost {
                if self.position_is_intrabc(above, c) {
                    return true;
                }
            }
        }
        false
    }

    /// Whether MI `(row, col)` holds a recorded IntrABC block (and is in range).
    fn position_is_intrabc(&self, row: usize, col: usize) -> bool {
        if row >= self.mi_rows || col >= self.mi_cols {
            return false;
        }
        self.values
            .get(row * self.mi_cols + col)
            .copied()
            .flatten()
            .is_some_and(|facts| facts.use_intrabc)
    }

    /// The tile ref-MV bank (the § 7.12.2 IntrABC bank state as of the last
    /// recorded block).
    fn bank(&self) -> &IntrabcRefMvBank {
        &self.bank
    }

    /// Builds the real AV2 § 7.12.2 IntrABC ref-MV stack (§ 7.12.2.21 ref-MV-bank
    /// fill + § 7.12.2.20 default search) the decoded DRL index selects into. The
    /// assign path and sink copy read `stack[ref_mv_idx]` from this list (NOT the
    /// bounded default-only fallback), so a bank-reordered block reconstructs with
    /// its correct BV. DEFERS when the § 7.12.2 spatial SMVP scan could contribute an
    /// unmodelled IntrABC candidate; over-reject is safe (skip-IBC entropy is
    /// BV-value-independent, so the walk still advances).
    fn intrabc_real_ref_mv_stack(
        &self,
        sequence: &SequenceHeader,
        geometry: IntrabcBlockGeometry,
        max_bvp_drl_bits_minus_1: u32,
        tile_offset: ByteOffset,
    ) -> Result<Vec<Mv>> {
        if self.spatial_scan_has_intrabc_neighbour(geometry) {
            return Err(wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack",
            ));
        }
        let stack_geometry = IntrabcStackGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            sb_samples: superblock_samples(sequence, tile_offset)?,
            frame_w: i32::try_from(self.mi_cols.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
            frame_h: i32::try_from(self.mi_rows.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
            max_bvp_drl_bits_minus_1,
        };
        Ok(build_intrabc_ref_mv_stack(
            self.bank(),
            stack_geometry,
            self.enable_refmvbank,
        ))
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
    // §7.12.2 Find MV stack: the §7.12.2.21 ref-MV-bank fill can REORDER the stack
    // relative to the bounded default-only fallback (e.g. ac0ej3 MI(0,232) selects
    // the bank candidate (0,-256) at DRL 0), so the assign path and sink copy below
    // read the selected BV from this REAL stack to reconstruct it bit-exact.
    let real_stack = state.intrabc_real_ref_mv_stack(
        sequence,
        geometry,
        syntax.max_bvp_drl_bits_minus_1,
        tile_offset,
    )?;
    let pred_mv = real_stack.get(syntax.ref_mv_idx).copied().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_mv_idx_out_of_range",
        )
    })?;
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
    // leaf the sink's `reconstruct_intrabc_block` copied only the prediction; the
    // residual add is gated/deferred inside the sink's transform-record path until the
    // displaced-copy + inverse-transform residual reconstruction is proven bit-exact.
    Ok(info)
}

/// Test-only path for default-only-stack IntrABC fixtures: select the §7.12.2
/// predicted BV from the bounded default fill (the live [`read_intrabc_info`]
/// instead selects from the REAL ref-MV stack) and finish the record.
#[cfg(test)]
fn read_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let syntax = read_intrabc_info_syntax(cdfs, symbols, sequence, core, tile_offset)?;
    let stack = intrabc_ref_stack_with_limit(
        sequence,
        geometry,
        syntax.max_bvp_drl_bits_minus_1,
        tile_offset,
    )?;
    let pred_mv = stack.get(syntax.ref_mv_idx).copied().ok_or_else(|| {
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

fn finish_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    syntax: IntrabcInfoSyntax,
    pred_mv: Mv,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    // `pred_mv` is `real_stack[ref_mv_idx]` from the caller's §7.12.2 Find MV stack:
    // NEARMV (mode 1) copies it verbatim, NEWMV (mode 0) adds the §5.20.7.27 MVD.
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
    let target = PlaneRect::new(target_x, target_y, width, height).map_err(|_| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
    if !target.is_within(domain.storage) || !rect_is_within_rect(target, domain.tile_bounds) {
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds",
        ));
    }
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
    Ok(local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        block_w: geometry.n4w * MI_SIZE,
        block_h: geometry.n4h * MI_SIZE,
        dv_row: info.block_mv.row,
        dv_col: info.block_mv.col,
        sb_size,
    }))
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

fn tile_interval_for_block(
    starts: &[u32],
    block_start: usize,
    block_len: usize,
    bounds_reason: &'static str,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    let block_end = block_start.checked_add(block_len).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_geometry",
        )
    })?;
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
        if block_start >= start && block_end <= end {
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

/// The bounded default-only §7.12.2.20 IntrABC fallback stack (four default block
/// vectors, no ref-MV-bank fill). Test-only: the live path builds the REAL §7.12.2
/// stack via [`TileIntrabcPreludeState::intrabc_real_ref_mv_stack`].
#[cfg(test)]
fn intrabc_ref_stack_with_limit(
    sequence: &SequenceHeader,
    geometry: IntrabcBlockGeometry,
    max_bvp_drl_bits_minus_1: u32,
    tile_offset: ByteOffset,
) -> Result<Vec<Mv>> {
    let max_count = usize::try_from(max_bvp_drl_bits_minus_1)
        .ok()
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_candidate_count",
            )
        })?;
    let block_width = geometry
        .n4w
        .checked_mul(MI_SIZE)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_width",
            )
        })?;
    let block_height = geometry
        .n4h
        .checked_mul(MI_SIZE)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_block_height",
            )
        })?;
    let sb_width = superblock_samples(sequence, tile_offset)?;
    let sb_height = sb_width;
    let mut candidates = Vec::new();
    // Bounded local handoff assumes §5.20.5.4 `find_mv_stack(0)` found no
    // spatial/bank candidates; current prediction support must replace this
    // before `block_mv` or `ref_mv_idx` become output-affecting.
    add_to_ref_bv(&mut candidates, max_count, 0, -sb_height, tile_offset)?;
    add_to_ref_bv(
        &mut candidates,
        max_count,
        -(sb_width + INTRABC_DELAY_PIXELS),
        0,
        tile_offset,
    )?;
    add_to_ref_bv(&mut candidates, max_count, 0, -block_height, tile_offset)?;
    add_to_ref_bv(&mut candidates, max_count, -block_width, 0, tile_offset)?;
    Ok(candidates)
}

#[cfg(test)]
fn add_to_ref_bv(
    candidates: &mut Vec<Mv>,
    max_count: usize,
    dx: i32,
    dy: i32,
    tile_offset: ByteOffset,
) -> Result<()> {
    if candidates.len() >= max_count {
        return Ok(());
    }
    let row = dy.checked_mul(8).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_bv_row_overflow",
        )
    })?;
    let col = dx.checked_mul(8).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_bv_col_overflow",
        )
    })?;
    candidates.push(Mv { row, col });
    Ok(())
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
mod tests {
    use splot_core::headers::frame::{
        FrameSize, IntrabcParams, TxMode, build_minimal_intra_clk_core,
        build_minimal_intra_sequence_header,
    };
    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
    use splot_core::symbol_encoder::SymbolEncoder;

    use crate::error::DecodeError;
    use crate::tile_payload::{FrameCdfSubset, MvCdfSelector};

    use super::*;

    const BLOCK_16X16: usize = 6;

    fn no_off() -> ByteOffset {
        ByteOffset::new(0)
    }

    /// A `skip` IntrABC neighbour prelude with an integer block vector, for
    /// populating the neighbour grid / ref-MV bank in tests.
    fn ac0ej3_skip_neighbour() -> IntrabcBlockPrelude {
        ac0ej3_intrabc_neighbour(Mv { row: -512, col: 0 })
    }

    fn ac0ej3_intrabc_neighbour(bv: Mv) -> IntrabcBlockPrelude {
        IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: true,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: bv.into(),
            }),
        }
    }

    fn selectable_fixture() -> (SequenceHeader, FrameHeaderCore) {
        let mut sequence = build_minimal_intra_sequence_header().unwrap();
        let (mut core, _) = build_minimal_intra_clk_core().unwrap();
        sequence
            .inter
            .as_mut()
            .unwrap()
            .seq_max_bvp_drl_bits_minus_1 = 0;
        sequence.inter.as_mut().unwrap().enable_bawp = false;
        core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Select;
        core.allow_intrabc = Some(true);
        core.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(false),
            allow_local_intrabc: None,
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        core.force_integer_mv = Some(false);
        (sequence, core)
    }

    fn selectable_large_frame_fixture() -> (SequenceHeader, FrameHeaderCore) {
        let (sequence, mut core) = selectable_fixture();
        core.frame_size = Some(FrameSize::new(128, 128));
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 32];
        tile_info.mi_row_starts = vec![0, 32];
        (sequence, core)
    }

    fn unsupported_reason(error: DecodeError) -> &'static str {
        match error {
            DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
            other => panic!("unexpected decode error: {other:?}"),
        }
    }

    fn encode_steps(steps: &[(Option<TileCdfSelector>, u32)]) -> Vec<u8> {
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        for &(selector, value) in steps {
            if let Some(selector) = selector {
                cdfs.with_row_mut(selector, |row| {
                    encoder.write_symbol(row, Symbol::new(u8::try_from(value).unwrap()))
                })
                .unwrap()
                .unwrap();
            } else {
                encoder.write_literal(value, 1).unwrap();
            }
        }
        encoder.finish().unwrap().into_bytes()
    }

    fn decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    fn state() -> TileIntrabcPreludeState {
        let (sequence, _) = selectable_fixture();
        TileIntrabcPreludeState::new(64, 64, &sequence, ByteOffset::new(0)).unwrap()
    }

    /// Runs the live `read_intrabc_use_and_skip` → `read_intrabc_info` sequence over
    /// `steps` on the large-frame fixture, returning the decoded `use_skip`, the
    /// `read_intrabc_info` result (`Ok(info)` for a `skip` leaf that advances, `Err`
    /// for a fail-closed leaf), and the symbol count consumed. `skip_flag` is the
    /// leaf's §5.20.5.3 `skip` carried into `read_intrabc_info`.
    fn run_intrabc_prelude(
        steps: &[(Option<TileCdfSelector>, u32)],
        skip_flag: bool,
    ) -> (IntrabcUseSkip, Result<IntrabcInfo>, u64) {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(steps);
        let mut symbols = decoder(&payload);
        let state = state();
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let info = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &state,
            &sequence,
            &core,
            geometry,
            skip_flag,
            None,
            ByteOffset::new(20),
        );
        (use_skip, info, symbols.symbol_count())
    }

    #[test]
    fn active_intrabc_nearmv_skip_reads_use_skip_mode_and_drl_then_advances() {
        // A `skip` IntrABC leaf reads its mode-info in order, then the walk advances:
        // `read_intrabc_info` returns `Ok` (no residual symbols follow a skip leaf), so
        // the partition/superblock walk continues to the next block.
        let (use_skip, info, symbol_count) = run_intrabc_prelude(
            &[
                (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
                (Some(TileCdfSelector::Skip { ctx: 0 }), 1),
                (Some(TileCdfSelector::IntrabcMode), 1),
                (None, 0),
            ],
            true,
        );

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: true,
            }
        );
        assert_eq!(info.unwrap().intrabc_mode, 1);
        assert_eq!(symbol_count, 4);
    }

    #[test]
    fn active_intrabc_newmv_nonskip_reads_block_vector_and_returns_info_for_residual() {
        let (use_skip, info, symbol_count) = run_intrabc_prelude(
            &[
                (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
                (Some(TileCdfSelector::Skip { ctx: 0 }), 0),
                (Some(TileCdfSelector::IntrabcMode), 0),
                (None, 0),
                (Some(TileCdfSelector::IntrabcPrecision), 1),
                (
                    Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                        mv_ctx: 1,
                    })),
                    0,
                ),
                (
                    Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                        precision: usize::from(MV_PRECISION_QUARTER_PEL),
                        shell_set: 0,
                        mv_ctx: 1,
                    })),
                    0,
                ),
                (
                    Some(TileCdfSelector::ReadMv(
                        MvCdfSelector::ShellOffsetLowClass {
                            mv_ctx: 1,
                            shell_class: 0,
                        },
                    )),
                    0,
                ),
            ],
            false,
        );

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: false,
            }
        );
        // A NON-`skip` IntrABC leaf reads its full §5.20.5.4 block-vector syntax and
        // returns the parsed mode-info: its §5.20.6.1 inter tx-partition + §5.20.7.29
        // inter transform-type + §5.20.7.27 coefficient residual are decoded by the
        // is_inter-aware tx-record + coefficient machinery the partition walk drives
        // AFTER this prelude returns, so the prelude itself must not stop the walk.
        let info = info.expect("non-skip IntrABC prelude returns parsed mode-info");
        assert_eq!(info.intrabc_mode, 0);
        assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
        assert_eq!(symbol_count, 8);
    }

    #[test]
    fn active_intrabc_ref_stack_requires_proven_empty_candidate_stack() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 2 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 2 }), 0),
            (Some(TileCdfSelector::IntrabcMode), 1),
            (None, 0),
        ]);
        let mut symbols = decoder(&payload);
        let mut state = state();
        let prior_intrabc = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: false,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
                block_mv: IntrabcBlockVector { row: -512, col: 0 },
            }),
        };
        state
            .record_block(19, 0, 4, 1, prior_intrabc, ByteOffset::new(0))
            .unwrap();
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let error = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &state,
            &sequence,
            &core,
            geometry,
            false,
            None,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: true,
                skip_flag: false,
            }
        );
        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack"
        );
        assert_eq!(symbols.symbol_count(), 5);
    }

    // ac0ej3 frame-0 MI(0,112): MI(16,56)'s bank BV (-512,0) is REJECTED by the
    // §7.12.2.21 frame-boundary test (ref_y = -64 <= -block_height), so the REAL
    // stack is default-only and the live path's DRL-idx-3 entry equals the fallback
    // tail (0,-256). Sibling module checks exact AVM values; this proves the
    // prelude-state wiring (no spatial neighbour -> admit).
    #[test]
    fn intrabc_real_stack_selects_ac0ej3_mi_0_112_default_only_bv() {
        let (mut sequence, _core) = selectable_large_frame_fixture();
        sequence
            .inter
            .as_mut()
            .unwrap()
            .seq_max_bvp_drl_bits_minus_1 = 2;
        // 256 MI cols keep MI(0,112)'s candidates in bounds; 128 MI rows span height.
        let mut state = TileIntrabcPreludeState::new(128, 256, &sequence, no_off()).unwrap();
        state
            .record_block(16, 56, 8, 16, ac0ej3_skip_neighbour(), no_off())
            .unwrap();
        // MI(0,112): 32x64 (n4w 8, n4h 16), no spatial IntrABC neighbour, DRL idx 3.
        let geometry =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(0, 112, BLOCK_16X16, false), 8, 16);

        let stack = state
            .intrabc_real_ref_mv_stack(&sequence, geometry, 2, ByteOffset::new(110))
            .unwrap();
        assert_eq!(stack.get(3).copied(), Some(Mv { row: 0, col: -256 }));
    }

    // ac0ej3 frame-0 MI(0,232) (the reordered-stack block #517 left at the frontier):
    // the §7.12.2.21 LIFO fill admits MI(0,112)'s bank BV (0,-256) and REJECTS
    // MI(16,56)'s (-512,0), so the REAL stack puts (0,-256) at the HEAD (DRL index 0)
    // ahead of the defaults -- the live path selects this bank candidate, NOT the
    // default-fill head. This proves the bank reorder reaches the assign path.
    #[test]
    fn intrabc_real_stack_selects_ac0ej3_mi_0_232_reordered_bank_bv() {
        let (mut sequence, _core) = selectable_large_frame_fixture();
        sequence
            .inter
            .as_mut()
            .unwrap()
            .seq_max_bvp_drl_bits_minus_1 = 2;
        let mut state = TileIntrabcPreludeState::new(128, 256, &sequence, no_off()).unwrap();
        // MI(16,56) then MI(0,112) both record into the bank.
        state
            .record_block(16, 56, 8, 16, ac0ej3_skip_neighbour(), no_off())
            .unwrap();
        let mi_0_112 = ac0ej3_intrabc_neighbour(Mv { row: 0, col: -256 });
        state
            .record_block(0, 112, 8, 16, mi_0_112, no_off())
            .unwrap();
        // MI(0,232): 32x64, no spatial IntrABC neighbour (far from prior blocks).
        let geometry =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(0, 232, BLOCK_16X16, false), 8, 16);

        let stack = state
            .intrabc_real_ref_mv_stack(&sequence, geometry, 2, ByteOffset::new(110))
            .unwrap();
        // DRL index 0 selects the bank-reordered candidate, not the default-fill head;
        // the bank BV (0,-256) is NOT one of this geometry's four §7.12.2.20 defaults.
        assert_eq!(stack.first().copied(), Some(Mv { row: 0, col: -256 }));
        let default_only = intrabc_ref_stack_with_limit(&sequence, geometry, 2, no_off()).unwrap();
        assert_ne!(
            default_only.first().copied(),
            Some(Mv { row: 0, col: -256 })
        );
    }

    // Finding 1: the § 7.12.2 step-14 SB-border above-left probe is at
    // `deltaCol = -1 - isSbBorder == -2`, aligned by § 7.12.2.6 to
    // `MiCol - 2 - (MiCol & 1)`. For an SB-border block with EVEN MiCol that probe
    // reads (row-1, MiCol-2); an IntrABC neighbour existing ONLY there must DEFER.
    // The control (an interior block whose above scan starts at MiCol-1) must NOT
    // see it, so the fix is targeted, not over-broad. The fixture's seq SB is
    // 64x64, so sb_size4 == 16 and MiRow 16 is an SB-row boundary, MiRow 20 is not.
    #[test]
    fn spatial_scan_detects_sb_border_col_minus_two_neighbour() {
        let (sequence, _core) = selectable_large_frame_fixture();
        let neighbour = ac0ej3_skip_neighbour();
        // SB-border block MI(16,56) (even MiCol): the (15,54)==(row-1,MiCol-2) probe.
        let mut sb_border = TileIntrabcPreludeState::new(64, 64, &sequence, no_off()).unwrap();
        sb_border
            .record_block(15, 54, 1, 1, neighbour, no_off())
            .unwrap();
        let at_border =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(16, 56, BLOCK_16X16, false), 8, 16);
        assert!(sb_border.spatial_scan_has_intrabc_neighbour(at_border));

        // Control: interior block MI(20,56) (row 20 % 16 != 0) does NOT probe MiCol-2.
        let mut interior = TileIntrabcPreludeState::new(64, 64, &sequence, no_off()).unwrap();
        interior
            .record_block(19, 54, 1, 1, neighbour, no_off())
            .unwrap();
        let at_interior =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 56, BLOCK_16X16, false), 8, 16);
        assert!(!interior.spatial_scan_has_intrabc_neighbour(at_interior));
    }

    #[test]
    fn intrabc_newmv_geometry_derives_integer_luma_copy_rectangles() {
        let (sequence, mut core) = selectable_large_frame_fixture();
        core.force_integer_mv = Some(true);
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_ONE_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                0,
            ),
        ]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 16);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_nearmv_geometry_derives_integer_luma_copy_rectangles() {
        let (sequence, core) = selectable_large_frame_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[(Some(TileCdfSelector::IntrabcMode), 1), (None, 0)]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(20, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();
        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 16);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_geometry_derives_bilinear_fractional_luma_prediction_region() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 0,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -132, col: 0 },
        };

        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(32, 32, 16, 16).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(32, 15, 16, 17).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 32);
        assert_eq!(prediction.scaling.start_y >> 10, 15);
        assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
        assert_ne!((prediction.scaling.start_y >> 6) & 15, 0);
    }

    #[test]
    fn intrabc_geometry_uses_mi_domain_for_partial_edge_frame() {
        let (_, mut core) = selectable_fixture();
        core.frame_size = Some(FrameSize::new(10, 10));
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 4];
        tile_info.mi_row_starts = vec![0, 4];
        let block = IntrabcBlockContext::new(2, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 2);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -64, col: 0 },
        };

        let prediction =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap();

        assert_eq!(prediction.target, PlaneRect::new(0, 8, 16, 8).unwrap());
        assert_eq!(prediction.source, PlaneRect::new(0, 0, 16, 8).unwrap());
        assert_eq!(prediction.scaling.start_x >> 10, 0);
        assert_eq!(prediction.scaling.start_y >> 10, 0);
    }

    #[test]
    fn intrabc_geometry_rejects_source_outside_current_tile() {
        let (_, mut core) = selectable_large_frame_fixture();
        let tile_info = core.tile_info.as_mut().unwrap();
        tile_info.mi_col_starts = vec![0, 4, 8];
        tile_info.mi_row_starts = vec![0, 8];
        let block = IntrabcBlockContext::new(4, 4, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: -128 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_self_referential_source() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_out_of_frame_source() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -512, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_out_of_frame_target() {
        let (_, core) = selectable_large_frame_fixture();
        let block = IntrabcBlockContext::new(32, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
        );
    }

    #[test]
    fn intrabc_geometry_rejects_missing_frame_size() {
        let (_, mut core) = selectable_large_frame_fixture();
        core.frame_size = None;
        let block = IntrabcBlockContext::new(8, 8, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: 0, col: 0 },
        };

        let error =
            derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
                .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size"
        );
    }

    #[test]
    fn intrabc_newmv_one_pel_record_shifts_shell_delta() {
        let (sequence, mut core) = selectable_fixture();
        core.force_integer_mv = Some(true);
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_ONE_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                1,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::ColMvIndex {
                    mv_ctx: 1,
                    ctx: 0,
                })),
                0,
            ),
            (None, 0),
        ]);
        let mut symbols = decoder(&payload);
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let info = read_intrabc_info_record(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();

        assert_eq!(
            info,
            IntrabcInfo {
                intrabc_mode: 0,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_ONE_PEL,
                block_mv: IntrabcBlockVector { row: -504, col: 0 },
            }
        );
        assert_eq!(symbols.symbol_count(), 6);
    }

    #[test]
    fn intrabc_newmv_read_errors_use_intrabc_frontier_diagnostic() {
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = [];
        let mut symbols = decoder(&payload);

        // Force the shared read_mv helper to fail at the IntrABC caller boundary:
        // intrabc_mode 0 == NEWMV reads an MVD, and an out-of-spec precision (0) makes
        // the §6.10.27 MvReadConfig validation reject it. Public IntrABC mode-info
        // only passes spec-valid precisions, so this only exercises the error mapping.
        let error = assign_intrabc_mv(
            &mut cdfs,
            &mut symbols,
            0,
            0,
            Mv { row: 0, col: 0 },
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv"
        );
    }

    #[test]
    fn non_intrabc_path_reads_only_use_intrabc_symbol() {
        let (_, core) = selectable_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[(Some(TileCdfSelector::Intrabc { ctx: 0 }), 0)]);
        let mut symbols = decoder(&payload);
        let state = state();
        let block = IntrabcBlockContext::new(0, 0, 2, false);
        let geometry = IntrabcBlockGeometry::new(block, 4, 4);

        let use_skip = read_intrabc_use_and_skip(
            &mut cdfs,
            &mut symbols,
            &state,
            &core,
            geometry,
            ByteOffset::new(20),
        )
        .unwrap();

        assert_eq!(
            use_skip,
            IntrabcUseSkip {
                use_intrabc: false,
                skip_flag: false,
            }
        );
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn contexts_use_intrabc_npos_and_skip_nposbuf_boundaries() {
        let mut state = state();
        let ordinary = IntrabcBlockPrelude {
            use_intrabc: false,
            is_inter: false,
            skip_flag: false,
            intrabc: None,
        };
        let intrabc_skip = ac0ej3_skip_neighbour();
        state
            .record_block(15, 4, 4, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(15, 8, 4, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(16, 3, 1, 4, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(16, 4, 4, 4, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(),
            2
        );
        assert_eq!(state.skip_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(), 2);
    }

    #[test]
    fn contexts_stop_after_first_two_valid_neighbour_candidates() {
        let mut state = state();
        let ordinary = IntrabcBlockPrelude {
            use_intrabc: false,
            is_inter: false,
            skip_flag: false,
            intrabc: None,
        };
        let intrabc_skip = ac0ej3_skip_neighbour();
        state
            .record_block(23, 7, 1, 1, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(19, 11, 1, 1, ordinary, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(20, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();
        state
            .record_block(19, 8, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(),
            0
        );
        assert_eq!(state.skip_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(), 0);
    }

    #[test]
    fn contexts_preserve_duplicate_neighbour_slots_before_cap() {
        let mut state = state();
        let intrabc_skip = ac0ej3_skip_neighbour();
        state
            .record_block(0, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
            .unwrap();

        assert_eq!(
            state.intrabc_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(),
            2
        );
        assert_eq!(state.skip_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(), 2);
    }

    #[test]
    fn intrabc_ref_stack_caps_256_sequence_superblocks_to_intra_sb_size() {
        let (mut sequence, _) = selectable_fixture();
        let partition = sequence.partition.as_mut().unwrap();
        partition.use_256x256_superblock = true;
        partition.use_128x128_superblock = false;
        let geometry =
            IntrabcBlockGeometry::new(IntrabcBlockContext::new(0, 0, BLOCK_16X16, false), 4, 4);

        let candidates =
            intrabc_ref_stack_with_limit(&sequence, geometry, 2, ByteOffset::new(0)).unwrap();

        assert_eq!(
            candidates,
            vec![
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -128, col: 0 },
                Mv { row: 0, col: -128 },
            ]
        );
    }

    // Codex finding 2: the §6.19.7.12 local-range geometry. ac0ej3's first IntrABC
    // block (128x128 SB, MI(16,56), block 32x64, DV (-512, 0)) is PROVEN valid — its
    // source x[224,256) y[0,64) sits in the SAME superblock as the active block and the
    // same superblock column. Verified against AVM `av2_is_dv_in_local_range`.
    #[test]
    fn local_intrabc_range_admits_ac0ej3_first_block() {
        assert!(local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 16,
            mi_col: 56,
            block_w: 32,
            block_h: 64,
            dv_row: -512,
            dv_col: 0,
            sb_size: 128,
        }));
    }

    // A DV whose source lies in the UNCODED bottom-right region of the active block's
    // top-left corner is rejected (the §6.19.7.12 first local-range guard).
    #[test]
    fn local_intrabc_range_rejects_uncoded_bottom_right_source() {
        // dv (+8, +8) eighth-pel == +1 sample down/right: source overlaps the uncoded
        // region (`(dvCol>>3)+bw > 0 && (dvRow>>3)+bh > 0`).
        assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 16,
            mi_col: 56,
            block_w: 32,
            block_h: 64,
            dv_row: 8,
            dv_col: 8,
            sb_size: 128,
        }));
    }

    // A DV whose source is in a DIFFERENT (previous) superblock is rejected by the
    // same-superblock narrowing.
    #[test]
    fn local_intrabc_range_rejects_source_beyond_left_buffer_window() {
        // Block at SB column 2 (mi_col 64 == 256px), source displaced 3 SBs left
        // (dv_col == -3 * 128 * 8 == -3072): the source SB column is 3 SBs left of the
        // active SB, so it is not in the same superblock.
        assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 64,
            mi_col: 64,
            block_w: 32,
            block_h: 32,
            dv_row: 0,
            dv_col: -3072,
            sb_size: 128,
        }));
    }

    // Codex re-review finding 1: a source in the PREVIOUS 128x128 superblock — which
    // `av2_is_dv_in_local_range`'s left-buffer window would admit but §6.19.7.12
    // `check_valid_local_ibc` can reject on a 64x64 IBC-buffer collocation collision — is
    // DEFERRED by the same-superblock narrowing. Codex's example: MI(0,68) (active px
    // 272 == SB col 2), DV (0, -128px): source px[144,175] sits in SB col 1, a previous
    // superblock, so it must defer (never copy a not-actually-valid MV).
    #[test]
    fn local_intrabc_range_rejects_previous_superblock_buffer_collision_source() {
        assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
            mi_row: 0,
            mi_col: 68,
            block_w: 32,
            block_h: 32,
            dv_row: 0,
            dv_col: -128 * 8,
            sb_size: 128,
        }));
    }

    // The full `intrabc_dv_proven_valid` gate DEFERS when `allow_local_intrabc` is
    // explicitly `Some(false)`, even for an otherwise-valid same-SB integer DV.
    #[test]
    fn proven_valid_defers_when_local_intrabc_disabled() {
        let (sequence, mut core) = selectable_large_frame_fixture();
        core.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(true),
            allow_local_intrabc: Some(false),
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        // A same-SB source within SB col 0 (active px 0, source displaced fully inside).
        let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 0, 2, false), 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -512, col: 0 },
        };
        assert!(
            !intrabc_dv_proven_valid(&sequence, &core, geometry, info, ByteOffset::new(20))
                .unwrap()
        );
    }

    // Codex re-review finding 2: §5.18.3.4 inference. A frame with `allow_global_intrabc
    // == 0` infers `allow_local_intrabc = 1` (the parser stores the inferred value as
    // `None`), so an inferred-local frame with a valid same-SB integer DV is ADMITTED.
    #[test]
    fn proven_valid_admits_inferred_local_intrabc_frame() {
        let (sequence, mut core) = selectable_large_frame_fixture();
        // allow_global_intrabc == 0 -> allow_local_intrabc inferred 1 (stored `None`).
        core.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(false),
            allow_local_intrabc: None,
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        // A same-SB source for any SB size >= 64: block 16x16 at MI(4,4) (active px
        // (16,16), SB (0,0)), DV (-128 eighth == -16px row, 0) -> source px[16,31] y[0,15],
        // directly above the block in the same superblock (col 0, row 0). The DV clears
        // the uncoded-bottom-right guard ((dvRow>>3)+bh == 0, not > 0).
        let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(4, 4, 2, false), 4, 4);
        let info = IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -128, col: 0 },
        };
        assert!(
            intrabc_dv_proven_valid(&sequence, &core, geometry, info, ByteOffset::new(20)).unwrap()
        );
    }
}
