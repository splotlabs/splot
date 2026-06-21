// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal flat intra block-symbol trace frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` and
//! `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` and
//! `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF`.

use splot_core::Error as CoreError;
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::DecodeTileWorkUnit;
use super::cdf::block_context::{YModeIndexContext, reconstruct_minimal_y_mode, uv_mode_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};
use super::coeff_loop::ordinary_pass::geometry::CoeffOrdinaryTxSizeGeometryConfig;
use super::coeff_loop::use_fsc_branch::{
    CoeffUseFscBranchError, CoeffUseFscFrameFactsInput, apply_coeff_use_fsc_branch_from_frame_facts,
};
use super::coeff_loop::{
    CoeffLoopContextError, LumaAllZeroContextInput, VAllZeroContextInput, luma_all_zero_context,
    v_all_zero_context,
};
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};

const INTRA_Y_MODE_SET_REASON: &str = "intra_y_mode_set";
const INTRA_Y_MODE_INDEX_REASON: &str = "intra_y_mode_index";
const LUMA_OR_U_ALL_ZERO_TRANSFORM_REASON: &str = "luma_or_u_all_zero_transform";
const UV_MODE_INDEX_REASON: &str = "uv_mode_index";
const V_ALL_ZERO_TRANSFORM_REASON: &str = "v_all_zero_transform";
const MINIMAL_LUMA_TX_W4: usize = 16;
const MINIMAL_LUMA_TX_H4: usize = 16;
const MINIMAL_CHROMA_TX_W4: usize = 4;
const MINIMAL_CHROMA_TX_H4: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TracedAllZeroCoeffGeometry {
    plane: usize,
    start_x: usize,
    start_y: usize,
    w4: usize,
    h4: usize,
}

/// Summary returned after the traced block symbols and `exit_symbol()` pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MinimalBlockSymbolTrace {
    summary: SymbolDecoderSummary,
}

impl MinimalBlockSymbolTrace {
    /// Successful AV2 § 8.2.4 `exit_symbol()` summary.
    #[must_use]
    pub(crate) const fn summary(self) -> SymbolDecoderSummary {
        self.summary
    }
}

/// Error returned by the minimal block-symbol trace frontier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MinimalBlockSymbolTraceError {
    /// A traced block-symbol CDF read failed.
    #[error("minimal block-symbol trace read failed for {reason}: {source}")]
    SymbolRead {
        /// Stable trace reason.
        reason: &'static str,
        /// Source CDF selection or symbol-decoder error.
        source: BlockSymbolTraceReadError,
    },
    /// The traced symbol decoded to a value outside the supported minimal tier.
    #[error("minimal block-symbol trace expected {expected} for {reason}, got {actual}")]
    UnexpectedSymbol {
        /// Stable trace reason.
        reason: &'static str,
        /// Expected traced symbol value.
        expected: u8,
        /// Actual decoded symbol value.
        actual: u8,
    },
    /// The decoded `y_mode_set` / `y_mode_index` fell outside the supported
    /// minimal `YMode` reconstruction subset (unreachable for the asserted
    /// flat-intra trace, which decodes `y_mode_set == 0` and `y_mode_index == 0`).
    #[error(
        "minimal block-symbol trace cannot reconstruct YMode for y_mode_set {y_mode_set}, y_mode_index {y_mode_index}"
    )]
    UnsupportedYMode {
        /// Decoded `y_mode_set` value.
        y_mode_set: u8,
        /// Decoded `y_mode_index` value.
        y_mode_index: u8,
    },
    /// The tile work-unit MI range could not seed coefficient context state.
    #[error(
        "minimal block-symbol trace has invalid coefficient context {axis} range {start}..{end}"
    )]
    InvalidCoeffContextRange {
        /// Range axis.
        axis: &'static str,
        /// Range start.
        start: u32,
        /// Range end.
        end: u32,
    },
    /// The tile work-unit MI range does not fit this platform's `usize`.
    #[error(
        "minimal block-symbol trace coefficient context {axis} length {value} does not fit usize"
    )]
    CoeffContextDimensionOverflow {
        /// Range axis.
        axis: &'static str,
        /// Length value.
        value: u32,
    },
    /// Coefficient context state allocation or validation failed.
    #[error("minimal block-symbol trace coefficient context state failed: {source}")]
    CoeffContextState {
        /// Source coefficient context state error.
        source: TileCoeffStateError,
    },
    /// Coefficient-loop context handoff failed.
    #[error("minimal block-symbol trace coefficient-loop context failed: {source}")]
    CoeffLoopContext {
        /// Source coefficient-loop context error.
        source: CoeffLoopContextError,
    },
    /// Traced 4x4 transform geometry overflowed while converting to pixels.
    #[error(
        "minimal block-symbol trace coefficient transform {axis} {blocks_4x4} 4x4 blocks overflows pixel units"
    )]
    CoeffTxGeometryDimensionOverflow {
        /// Dimension axis.
        axis: &'static str,
        /// Dimension in 4x4 blocks.
        blocks_4x4: usize,
    },
    /// Traced transform geometry has no generated AV2 transform-size entry.
    #[error(
        "minimal block-symbol trace unsupported coefficient transform geometry {w4}x{h4} 4x4 blocks ({width}x{height} pixels)"
    )]
    UnsupportedCoeffTxGeometry {
        /// Width in 4x4 blocks.
        w4: usize,
        /// Height in 4x4 blocks.
        h4: usize,
        /// Width in pixels.
        width: usize,
        /// Height in pixels.
        height: usize,
    },
    /// A generated AV2 transform-size table entry could not be represented.
    #[error(
        "minimal block-symbol trace invalid {table}[{tx_size}] transform-size table value {value}"
    )]
    InvalidCoeffTxTableValue {
        /// Table name.
        table: &'static str,
        /// Transform-size index.
        tx_size: usize,
        /// Raw generated table value.
        value: i32,
    },
    /// Coefficient-loop frame-entry handoff failed.
    #[error("minimal block-symbol trace coefficient frame-entry handoff failed: {source}")]
    CoeffFrameEntry {
        /// Source coefficient-loop frame-entry error.
        source: CoeffUseFscBranchError,
    },
    /// `exit_symbol()` rejected the tile payload suffix.
    #[error("minimal block-symbol trace exit_symbol failed: {source}")]
    ExitSymbol {
        /// Source symbol-decoder error.
        source: CoreError,
    },
}

/// Consumes the traced flat intra block-symbol sequence after the partition frontier.
pub(crate) fn consume_minimal_block_symbol_trace<'payload>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
    mut symbols: SymbolDecoder<'payload>,
) -> Result<MinimalBlockSymbolTrace, MinimalBlockSymbolTraceError> {
    let before = work_unit.cdf().tile_cdfs().clone();
    let result = consume_trace(work_unit, &mut symbols).and_then(|()| {
        symbols
            .exit_symbol()
            .map(|summary| MinimalBlockSymbolTrace { summary })
            .map_err(|source| MinimalBlockSymbolTraceError::ExitSymbol { source })
    });
    match result {
        Ok(trace) => Ok(trace),
        Err(error) => {
            *work_unit.cdf_mut().tile_cdfs_mut() = before;
            Err(error)
        }
    }
}

// The traced symbols are decoded sequentially (not from a static table) so the
// § 8.3.2 context of a later symbol can be derived from earlier decodes — the
// `uv_mode` context depends on the reconstructed luma `YMode`.
fn consume_trace(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<(), MinimalBlockSymbolTraceError> {
    let mut coeff_context = minimal_tile_coeff_context(work_unit)?;
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // y_mode_set (§ 8.3.2 `TileYModeSetCdf`, no context).
    let y_mode_set = decode_block_symbol(
        cdfs,
        symbols,
        TileCdfSelector::YModeSet,
        0,
        INTRA_Y_MODE_SET_REASON,
    )?;

    // y_mode_index (§ 8.3.2 context from `get_joint_mode`; both neighbours are
    // out of frame at the tile origin, so the context is 0).
    let y_mode_index = decode_block_symbol(
        cdfs,
        symbols,
        TileCdfSelector::YModeIndex {
            ctx: YModeIndexContext::tile_origin_block().ctx(),
        },
        0,
        INTRA_Y_MODE_INDEX_REASON,
    )?;

    // Reconstruct the luma `YMode` from the decoded set/index (§ 5); the asserted
    // trace values (set 0, index 0) resolve to `DC_PRED`.
    let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
        MinimalBlockSymbolTraceError::UnsupportedYMode {
            y_mode_set,
            y_mode_index,
        },
    )?;

    // luma all-zero transform (txb_skip), § 8.3.2. The coefficient context state
    // is freshly zeroed for this first transform block, so its above/left
    // reductions are 0 and the context reduces to the transform-fills-block
    // branch -> 0 (this is the CDF *context* index, not the decoded value).
    //
    // The decoded `all_zero` symbol is asserted to 1: AV2 § 5.20.7.27 / AVM
    // `decodetxb.c` (`read_coeffs_txb`) read `all_zero = read_symbol(txb_skip_cdf)`
    // and take the no-coefficient skip branch when `all_zero == 1`. An all-zero
    // (skipped) transform block therefore carries `txb_skip == 1`.
    //
    // TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): the `tx_fills_block`
    // geometry comes from the § 5.20 transform-block syntax (not yet modelled);
    // it is asserted here to the value the conformant fixture forces.
    let luma_txb_skip_ctx = luma_all_zero_context(
        &coeff_context,
        LumaAllZeroContextInput {
            x4: 0,
            y4: 0,
            w4: MINIMAL_LUMA_TX_W4,
            h4: MINIMAL_LUMA_TX_H4,
            tx_fills_block: true,
            fsc_active: false,
        },
    )
    .map_err(|source| MinimalBlockSymbolTraceError::CoeffLoopContext { source })?;
    decode_block_symbol(
        cdfs,
        symbols,
        TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx: 2,
            plane_type: 0,
            tx_size: 0,
            ctx: luma_txb_skip_ctx,
        },
        1,
        LUMA_OR_U_ALL_ZERO_TRANSFORM_REASON,
    )?;
    apply_all_zero_coeff_frame_entry_from_traced_geometry(
        &mut coeff_context,
        cdfs,
        symbols,
        TracedAllZeroCoeffGeometry {
            plane: 0,
            start_x: 0,
            start_y: 0,
            w4: MINIMAL_LUMA_TX_W4,
            h4: MINIMAL_LUMA_TX_H4,
        },
    )?;

    // uv_mode (§ 8.3.2 context = `is_directional_mode(YMode)`; DC_PRED -> 0).
    decode_block_symbol(
        cdfs,
        symbols,
        TileCdfSelector::UvModeCflNotAllowed {
            ctx: uv_mode_ctx(y_mode),
        },
        6,
        UV_MODE_INDEX_REASON,
    )?;

    // V all-zero transform (v_txb_skip), § 8.3.2. The coefficient context state
    // is freshly zeroed for this first transform block, and the U plane was
    // decoded all-zero just above so EobU == 0; the context reduces to the
    // chroma-block-larger-than-transform contribution -> 3 (CDF context index,
    // not the decoded value).
    //
    // The decoded `all_zero` symbol is asserted to 1 for the same reason as the
    // luma read above: per AV2 § 5.20.7.27 / AVM `decodetxb.c` an all-zero
    // (skipped) V transform block carries `v_txb_skip == 1`.
    //
    // TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): the
    // `chroma_block_larger_than_tx` geometry comes from the § 5.20
    // transform-block syntax (not yet modelled); it is asserted here to the
    // value the conformant fixture forces.
    let v_txb_skip_context = v_all_zero_context(
        &coeff_context,
        VAllZeroContextInput {
            x4: 0,
            y4: 0,
            w4: MINIMAL_CHROMA_TX_W4,
            h4: MINIMAL_CHROMA_TX_H4,
            chroma_block_larger_than_tx: true,
            eob_u_nonzero: false,
        },
    )
    .map_err(|source| MinimalBlockSymbolTraceError::CoeffLoopContext { source })?;
    decode_block_symbol(
        cdfs,
        symbols,
        TileCdfSelector::VTxbSkip {
            coeff_cdf_q_ctx: 1,
            ctx: v_txb_skip_context,
        },
        1,
        V_ALL_ZERO_TRANSFORM_REASON,
    )?;
    apply_all_zero_coeff_frame_entry_from_traced_geometry(
        &mut coeff_context,
        cdfs,
        symbols,
        TracedAllZeroCoeffGeometry {
            plane: 2,
            start_x: 0,
            start_y: 0,
            w4: MINIMAL_CHROMA_TX_W4,
            h4: MINIMAL_CHROMA_TX_H4,
        },
    )?;

    Ok(())
}

fn apply_all_zero_coeff_frame_entry_from_traced_geometry(
    coeff_context: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: TracedAllZeroCoeffGeometry,
) -> Result<(), MinimalBlockSymbolTraceError> {
    let geometry = traced_all_zero_coeff_geometry(input)?;
    apply_all_zero_coeff_frame_entry(coeff_context, cdfs, symbols, geometry)
}

// AV2 § 5.20.7.27 passes `txSz` into `coeffs()`, while AV2 § 9.2 maps each
// generated transform-size enum to `Tx_Width` / `Tx_Height`.
fn traced_all_zero_coeff_geometry(
    input: TracedAllZeroCoeffGeometry,
) -> Result<CoeffOrdinaryTxSizeGeometryConfig, MinimalBlockSymbolTraceError> {
    Ok(CoeffOrdinaryTxSizeGeometryConfig {
        plane: input.plane,
        start_x: input.start_x,
        start_y: input.start_y,
        tx_size: tx_size_from_w4_h4(input.w4, input.h4)?,
    })
}

fn tx_size_from_w4_h4(w4: usize, h4: usize) -> Result<usize, MinimalBlockSymbolTraceError> {
    let width = w4.checked_mul(4).ok_or(
        MinimalBlockSymbolTraceError::CoeffTxGeometryDimensionOverflow {
            axis: "width",
            blocks_4x4: w4,
        },
    )?;
    let height = h4.checked_mul(4).ok_or(
        MinimalBlockSymbolTraceError::CoeffTxGeometryDimensionOverflow {
            axis: "height",
            blocks_4x4: h4,
        },
    )?;
    for (tx_size, (&tx_width, &tx_height)) in TX_WIDTH.iter().zip(TX_HEIGHT.iter()).enumerate() {
        let tx_width = tx_size_table_entry_usize("Tx_Width", tx_size, tx_width)?;
        let tx_height = tx_size_table_entry_usize("Tx_Height", tx_size, tx_height)?;
        if tx_width == width && tx_height == height {
            return Ok(tx_size);
        }
    }
    Err(MinimalBlockSymbolTraceError::UnsupportedCoeffTxGeometry {
        w4,
        h4,
        width,
        height,
    })
}

fn tx_size_table_entry_usize(
    table: &'static str,
    tx_size: usize,
    value: i32,
) -> Result<usize, MinimalBlockSymbolTraceError> {
    usize::try_from(value).map_err(|_| MinimalBlockSymbolTraceError::InvalidCoeffTxTableValue {
        table,
        tx_size,
        value,
    })
}

fn apply_all_zero_coeff_frame_entry(
    coeff_context: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
) -> Result<(), MinimalBlockSymbolTraceError> {
    apply_coeff_use_fsc_branch_from_frame_facts(
        coeff_context,
        cdfs,
        symbols,
        CoeffUseFscFrameFactsInput::AllZero(geometry),
    )
    .map(|_| ())
    .map_err(|source| MinimalBlockSymbolTraceError::CoeffFrameEntry { source })
}

fn minimal_tile_coeff_context(
    work_unit: &DecodeTileWorkUnit<'_>,
) -> Result<TileCoeffContextState, MinimalBlockSymbolTraceError> {
    let mi_rows = range_len_usize("rows", work_unit.mi_row_range())?;
    let mi_cols = range_len_usize("columns", work_unit.mi_col_range())?;
    TileCoeffContextState::new(mi_rows, mi_cols)
        .map_err(|source| MinimalBlockSymbolTraceError::CoeffContextState { source })
}

fn range_len_usize(
    axis: &'static str,
    range: core::ops::Range<u32>,
) -> Result<usize, MinimalBlockSymbolTraceError> {
    let length = range.end.checked_sub(range.start).ok_or(
        MinimalBlockSymbolTraceError::InvalidCoeffContextRange {
            axis,
            start: range.start,
            end: range.end,
        },
    )?;
    usize::try_from(length).map_err(|_| {
        MinimalBlockSymbolTraceError::CoeffContextDimensionOverflow {
            axis,
            value: length,
        }
    })
}

/// Reads one traced block symbol, mapping CDF/symbol failures and an unexpected
/// decoded value to typed errors, and returns the decoded value on success.
fn decode_block_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    expected: u8,
    reason: &'static str,
) -> Result<u8, MinimalBlockSymbolTraceError> {
    let decoded = cdfs
        .read_block_symbol_trace(selector, symbols)
        .map_err(|source| MinimalBlockSymbolTraceError::SymbolRead { reason, source })?
        .get();
    if decoded != expected {
        return Err(MinimalBlockSymbolTraceError::UnexpectedSymbol {
            reason,
            expected,
            actual: decoded,
        });
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use core::ops::Range;

    use splot_core::segment::MAX_SEGMENTS;
    use splot_core::span::{ByteOffset, ByteSpan};
    use splot_core::symbol::CdfUpdateMode;

    use super::super::cdf::{
        FrameCdfSubset, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy,
    };
    use super::super::coeff_loop::AllZeroCoeffBlockInput;
    use super::super::coeff_loop::ordinary_pass::{
        CoeffOrdinaryBranchInput, apply_coeff_ordinary_branch,
    };
    use super::super::partition_allowed::PartitionFeatureFlags;
    use super::super::partition_traversal::{
        TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
        TilePartitionLoopRestorationState, TilePartitionTraversalError,
        TilePartitionTraversalInput, plan_tile_partition_traversal_cursor,
    };
    use super::super::{SymbolInitBoundary, TileBruPath, TileCoeffFrameFacts, TilePayloadSource};
    use super::*;
    use crate::{DecodeLayerSelection, DecodeLimits, DecodeObuSourceKind};

    const BLOCK_64X64: usize = 12;
    const BLOCK_256X256: usize = 18;
    const PAYLOAD: [u8; 2] = [0x12, 0xFB];

    fn make_work_unit<'payload>(
        payload: &'payload [u8],
        update_mode: CdfUpdateMode,
    ) -> DecodeTileWorkUnit<'payload> {
        DecodeTileWorkUnit {
            source: TilePayloadSource::new(
                DecodeObuSourceKind::AnnexB,
                None,
                0,
                ByteOffset::new(0),
            ),
            selected_layer: DecodeLayerSelection::base(),
            tile_num: 0,
            tile_row: 0,
            tile_col: 0,
            mi_row_range: Range { start: 0, end: 64 },
            mi_col_range: Range { start: 0, end: 64 },
            tile_bytes: payload,
            tile_byte_span: ByteSpan::new(ByteOffset::new(128), payload.len() as u64),
            tile_size: payload.len() as u64,
            current_q_index_at_entry: 0,
            coeff_frame_facts: TileCoeffFrameFacts::new(
                false,
                false,
                0,
                [false; MAX_SEGMENTS],
                false,
                false,
                0,
            ),
            bru_path: TileBruPath::NotUsed,
            symbol: SymbolInitBoundary {
                consumed_bits: payload.len().saturating_mul(8).min(15) as u64,
                symbol_max_bits: payload.len() as i64 * 8 - 15,
                cdf_update_mode: update_mode,
            },
            cdf: TileCdfWorkUnitBoundary::new(
                update_mode,
                tile_cdf_save_policy(TileCdfPolicyInput::single_tile_default(), 0).unwrap(),
                FrameCdfSubset::from_defaults(),
            ),
        }
    }

    fn symbols_at_block_frontier<'payload>(
        work_unit: &mut DecodeTileWorkUnit<'payload>,
    ) -> Result<SymbolDecoder<'payload>, TilePartitionTraversalError> {
        let rows = vec![vec![BLOCK_256X256; 16]; 16];
        let mi0_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let mi1_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let left = vec![BLOCK_256X256; 16];
        let above = vec![BLOCK_256X256; 16];
        let context = TilePartitionContextState::new(
            [&mi0_rows, &mi1_rows],
            [&left, &left],
            [&above, &above],
        );
        let frame = TilePartitionFrameFacts::new(
            16,
            16,
            BLOCK_64X64,
            3,
            true,
            true,
            true,
            true,
            false,
            TilePartitionLoopRestorationState::NoSyntax,
            PartitionFeatureFlags::new(true, true),
            4,
            true,
            TilePartitionBruState::Active,
        )
        .unwrap();
        let cursor = plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
            work_unit,
            frame,
            context,
            DecodeLimits::DEFAULT,
        ))?;
        let (plan, symbols) = cursor.into_parts();
        assert_eq!(plan.symbol_count_after(), 1);
        assert_eq!(plan.frontier().b_size.index(), BLOCK_64X64);
        Ok(symbols)
    }

    fn assert_frame_entry_matches_direct_ordinary(
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        block: AllZeroCoeffBlockInput,
    ) {
        let mut direct_work_unit = make_work_unit(&PAYLOAD, CdfUpdateMode::Disabled);
        let mut wrapper_work_unit = make_work_unit(&PAYLOAD, CdfUpdateMode::Disabled);
        let mut direct_symbols = symbols_at_block_frontier(&mut direct_work_unit).unwrap();
        let mut wrapper_symbols = symbols_at_block_frontier(&mut wrapper_work_unit).unwrap();
        let mut direct_context = minimal_tile_coeff_context(&direct_work_unit).unwrap();
        let mut wrapper_context = minimal_tile_coeff_context(&wrapper_work_unit).unwrap();

        apply_coeff_ordinary_branch(
            &mut direct_context,
            direct_work_unit.cdf_mut().tile_cdfs_mut(),
            &mut direct_symbols,
            CoeffOrdinaryBranchInput::AllZero(block),
        )
        .unwrap();
        apply_all_zero_coeff_frame_entry(
            &mut wrapper_context,
            wrapper_work_unit.cdf_mut().tile_cdfs_mut(),
            &mut wrapper_symbols,
            geometry,
        )
        .unwrap();

        assert_eq!(wrapper_context, direct_context);
        assert_eq!(
            wrapper_work_unit.cdf().tile_cdfs(),
            direct_work_unit.cdf().tile_cdfs()
        );
        assert_eq!(
            wrapper_symbols.consumed_bits(),
            direct_symbols.consumed_bits()
        );
        assert_eq!(
            wrapper_symbols.symbol_count(),
            direct_symbols.symbol_count()
        );
    }

    #[test]
    fn all_zero_coefficient_frame_entry_matches_direct_ordinary_branch() {
        assert_frame_entry_matches_direct_ordinary(
            traced_all_zero_coeff_geometry(TracedAllZeroCoeffGeometry {
                plane: 0,
                start_x: 0,
                start_y: 0,
                w4: MINIMAL_LUMA_TX_W4,
                h4: MINIMAL_LUMA_TX_H4,
            })
            .unwrap(),
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: MINIMAL_LUMA_TX_W4,
                h4: MINIMAL_LUMA_TX_H4,
            },
        );
        assert_frame_entry_matches_direct_ordinary(
            traced_all_zero_coeff_geometry(TracedAllZeroCoeffGeometry {
                plane: 2,
                start_x: 0,
                start_y: 0,
                w4: MINIMAL_CHROMA_TX_W4,
                h4: MINIMAL_CHROMA_TX_H4,
            })
            .unwrap(),
            AllZeroCoeffBlockInput {
                plane: 2,
                x4: 0,
                y4: 0,
                w4: MINIMAL_CHROMA_TX_W4,
                h4: MINIMAL_CHROMA_TX_H4,
            },
        );
    }

    #[test]
    fn all_zero_tx_size_geometry_resolves_generated_tables() {
        let luma = traced_all_zero_coeff_geometry(TracedAllZeroCoeffGeometry {
            plane: 0,
            start_x: 0,
            start_y: 0,
            w4: MINIMAL_LUMA_TX_W4,
            h4: MINIMAL_LUMA_TX_H4,
        })
        .unwrap();
        assert_eq!(luma.plane, 0);
        assert_eq!(usize::try_from(TX_WIDTH[luma.tx_size]).unwrap(), 64);
        assert_eq!(usize::try_from(TX_HEIGHT[luma.tx_size]).unwrap(), 64);

        let v = traced_all_zero_coeff_geometry(TracedAllZeroCoeffGeometry {
            plane: 2,
            start_x: 0,
            start_y: 0,
            w4: MINIMAL_CHROMA_TX_W4,
            h4: MINIMAL_CHROMA_TX_H4,
        })
        .unwrap();
        assert_eq!(v.plane, 2);
        assert_eq!(usize::try_from(TX_WIDTH[v.tx_size]).unwrap(), 16);
        assert_eq!(usize::try_from(TX_HEIGHT[v.tx_size]).unwrap(), 16);
    }

    #[test]
    fn unsupported_all_zero_tx_size_geometry_consumes_no_state() {
        let mut work_unit = make_work_unit(&PAYLOAD, CdfUpdateMode::Disabled);
        let mut symbols = symbols_at_block_frontier(&mut work_unit).unwrap();
        let mut coeff_context = minimal_tile_coeff_context(&work_unit).unwrap();
        let context_before = coeff_context.clone();
        let tile_before = work_unit.cdf().tile_cdfs().clone();
        let saved_before = work_unit.cdf().saved_cdfs().clone();
        let frame_before = work_unit.cdf().frame_cdfs().clone();
        let consumed_bits_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();

        let err = apply_all_zero_coeff_frame_entry_from_traced_geometry(
            &mut coeff_context,
            work_unit.cdf_mut().tile_cdfs_mut(),
            &mut symbols,
            TracedAllZeroCoeffGeometry {
                plane: 0,
                start_x: 0,
                start_y: 0,
                w4: 3,
                h4: 3,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            MinimalBlockSymbolTraceError::UnsupportedCoeffTxGeometry {
                w4: 3,
                h4: 3,
                width: 12,
                height: 12
            }
        ));
        assert_eq!(coeff_context, context_before);
        assert_eq!(work_unit.cdf().tile_cdfs(), &tile_before);
        assert_eq!(work_unit.cdf().saved_cdfs(), &saved_before);
        assert_eq!(work_unit.cdf().frame_cdfs(), &frame_before);
        assert_eq!(symbols.consumed_bits(), consumed_bits_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }

    #[test]
    fn traced_symbol_mismatch_fails_closed_and_rolls_back_cdfs() {
        let mut saw_mismatch = false;
        for second_byte in u8::MIN..=u8::MAX {
            if second_byte == PAYLOAD[1] {
                continue;
            }
            let payload = [PAYLOAD[0], second_byte];
            let mut work_unit = make_work_unit(&payload, CdfUpdateMode::Disabled);
            let Ok(symbols) = symbols_at_block_frontier(&mut work_unit) else {
                continue;
            };
            let before = work_unit.cdf().tile_cdfs().clone();
            let saved_before = work_unit.cdf().saved_cdfs().clone();
            let frame_before = work_unit.cdf().frame_cdfs().clone();

            let err = consume_minimal_block_symbol_trace(&mut work_unit, symbols).unwrap_err();

            if matches!(err, MinimalBlockSymbolTraceError::UnexpectedSymbol { .. }) {
                assert_eq!(work_unit.cdf().tile_cdfs(), &before);
                assert_eq!(work_unit.cdf().saved_cdfs(), &saved_before);
                assert_eq!(work_unit.cdf().frame_cdfs(), &frame_before);
                saw_mismatch = true;
                break;
            }
        }
        assert!(saw_mismatch);
    }

    #[test]
    fn invalid_cdf_row_reports_parse_failure_and_preserves_rows() {
        let mut work_unit = make_work_unit(&PAYLOAD, CdfUpdateMode::Disabled);
        let symbols = symbols_at_block_frontier(&mut work_unit).unwrap();
        work_unit
            .cdf_mut()
            .tile_cdfs_mut()
            .with_row_mut(TileCdfSelector::YModeSet, |row| row[0] = 0)
            .unwrap();
        let before = work_unit.cdf().tile_cdfs().clone();
        let saved_before = work_unit.cdf().saved_cdfs().clone();
        let frame_before = work_unit.cdf().frame_cdfs().clone();

        let err = consume_minimal_block_symbol_trace(&mut work_unit, symbols).unwrap_err();

        assert!(matches!(
            err,
            MinimalBlockSymbolTraceError::SymbolRead {
                reason: INTRA_Y_MODE_SET_REASON,
                ..
            }
        ));
        assert_eq!(work_unit.cdf().tile_cdfs(), &before);
        assert_eq!(work_unit.cdf().saved_cdfs(), &saved_before);
        assert_eq!(work_unit.cdf().frame_cdfs(), &frame_before);
    }
}
