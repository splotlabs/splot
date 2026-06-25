// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded IntrABC syntax handoff for the ac0ej3 selectable transform-record frontier.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use crate::error::Result;
use crate::tile_payload::{DecodeBlockFrontier, TileCdfSelector, TileCdfSubset};

use super::wienerns_lr_selectable_transform_record_error_reason;

const BLOCK_64X64: usize = 12;
const MI_SIZE: usize = 4;
const INTRABC_CONTEXT_MAX: usize = 2;
const SKIP_CONTEXT_MAX: usize = 2;
const MV_PRECISION_ONE_PEL: u8 = 0;
const MV_PRECISION_QUARTER_PEL: u8 = 1;

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
}

/// Tile-local neighbour state for IntrABC and skip contexts used by §8.3.2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TileIntrabcPreludeState {
    mi_rows: usize,
    mi_cols: usize,
    sb_size4: usize,
    values: Vec<Option<IntrabcBlockFacts>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntrabcBlockFacts {
    use_intrabc: bool,
    skip_flag: bool,
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
        Ok(Self {
            mi_rows,
            mi_cols,
            sb_size4: intra_sb_size4(sequence, tile_offset)?,
            values: vec![None; values_len],
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
        let facts = IntrabcBlockFacts {
            use_intrabc: prelude.use_intrabc,
            skip_flag: prelude.skip_flag,
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
        Ok(())
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
            if !positions.contains(&candidate) {
                positions.push(candidate);
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

pub(super) fn read_intrabc_info(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
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

    let mv_precision = if force_integer_mv {
        MV_PRECISION_ONE_PEL
    } else {
        MV_PRECISION_QUARTER_PEL
    };
    if intrabc_mode == 0 {
        if !force_integer_mv {
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
        }
        return Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv",
        ));
    }
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
        intrabc_mode: u8::try_from(intrabc_mode).unwrap_or(1),
        ref_mv_idx,
        mv_precision,
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
    let partition = sequence.partition.as_ref().ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_missing_partition_config",
        )
    })?;
    Ok(match partition.seq_sb_size() {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 | SuperblockSize::Block256x256 => 32,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use splot_core::headers::frame::{
        IntrabcParams, TxMode, build_minimal_intra_clk_core, build_minimal_intra_sequence_header,
    };
    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
    use splot_core::symbol_encoder::SymbolEncoder;

    use crate::error::DecodeError;
    use crate::tile_payload::FrameCdfSubset;

    use super::*;

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

    #[test]
    fn active_intrabc_nearmv_reads_use_skip_mode_and_drl_in_order() {
        let (sequence, core) = selectable_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 1),
            (Some(TileCdfSelector::IntrabcMode), 1),
            (None, 0),
        ]);
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
        let info = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            ByteOffset::new(20),
        )
        .unwrap();

        assert_eq!(
            IntrabcBlockPrelude::from_use_skip(use_skip, Some(info)),
            IntrabcBlockPrelude {
                use_intrabc: true,
                is_inter: true,
                skip_flag: true,
                intrabc: Some(IntrabcInfo {
                    intrabc_mode: 1,
                    ref_mv_idx: 0,
                    mv_precision: MV_PRECISION_QUARTER_PEL,
                }),
            }
        );
        assert_eq!(symbols.symbol_count(), 4);
    }

    #[test]
    fn active_intrabc_newmv_reads_precision_then_fails_closed() {
        let (sequence, core) = selectable_fixture();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let payload = encode_steps(&[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 0),
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (Some(TileCdfSelector::IntrabcPrecision), 1),
        ]);
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
                use_intrabc: true,
                skip_flag: false,
            }
        );
        let error = read_intrabc_info(
            &mut cdfs,
            &mut symbols,
            &sequence,
            &core,
            ByteOffset::new(20),
        )
        .unwrap_err();

        assert_eq!(
            unsupported_reason(error),
            "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv"
        );
        assert_eq!(symbols.symbol_count(), 5);
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
        let intrabc_skip = IntrabcBlockPrelude {
            use_intrabc: true,
            is_inter: true,
            skip_flag: true,
            intrabc: Some(IntrabcInfo {
                intrabc_mode: 1,
                ref_mv_idx: 0,
                mv_precision: MV_PRECISION_QUARTER_PEL,
            }),
        };
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
}
