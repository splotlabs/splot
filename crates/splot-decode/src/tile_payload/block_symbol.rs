// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal flat intra block-symbol trace frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::Error as CoreError;
use splot_core::symbol::{SymbolDecoder, SymbolDecoderSummary};

use super::DecodeTileWorkUnit;
use super::cdf::TileCdfSelector;
use super::cdf::block_context::YModeIndexContext;
use super::cdf::block_read::BlockSymbolTraceReadError;

const INTRA_Y_MODE_SET_REASON: &str = "intra_y_mode_set";
const INTRA_Y_MODE_INDEX_REASON: &str = "intra_y_mode_index";
const LUMA_OR_U_ALL_ZERO_TRANSFORM_REASON: &str = "luma_or_u_all_zero_transform";
const UV_MODE_INDEX_REASON: &str = "uv_mode_index";
const V_ALL_ZERO_TRANSFORM_REASON: &str = "v_all_zero_transform";

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

fn consume_trace(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<(), MinimalBlockSymbolTraceError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    for item in minimal_trace_items() {
        let decoded = cdfs
            .read_block_symbol_trace(item.selector, symbols)
            .map_err(|source| MinimalBlockSymbolTraceError::SymbolRead {
                reason: item.reason,
                source,
            })?
            .get();
        if decoded != item.expected {
            return Err(MinimalBlockSymbolTraceError::UnexpectedSymbol {
                reason: item.reason,
                expected: item.expected,
                actual: decoded,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct MinimalTraceItem {
    selector: TileCdfSelector,
    expected: u8,
    reason: &'static str,
}

const fn minimal_trace_items() -> [MinimalTraceItem; 5] {
    [
        MinimalTraceItem {
            selector: TileCdfSelector::YModeSet,
            expected: 0,
            reason: INTRA_Y_MODE_SET_REASON,
        },
        MinimalTraceItem {
            // AV2 § 8.3.2 y_mode_index ctx for the single-block tile-origin
            // case: both get_joint_mode neighbours are out of frame -> DC_PRED
            // -> ctx 0 (see `YModeIndexContext`).
            selector: TileCdfSelector::YModeIndex {
                ctx: YModeIndexContext::tile_origin_block().ctx(),
            },
            expected: 0,
            reason: INTRA_Y_MODE_INDEX_REASON,
        },
        MinimalTraceItem {
            selector: TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx: 2,
                plane_type: 0,
                tx_size: 0,
                ctx: 0,
            },
            expected: 0,
            reason: LUMA_OR_U_ALL_ZERO_TRANSFORM_REASON,
        },
        MinimalTraceItem {
            selector: TileCdfSelector::UvModeCflNotAllowed { ctx: 0 },
            expected: 6,
            reason: UV_MODE_INDEX_REASON,
        },
        MinimalTraceItem {
            selector: TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            },
            expected: 0,
            reason: V_ALL_ZERO_TRANSFORM_REASON,
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use core::ops::Range;

    use splot_core::span::{ByteOffset, ByteSpan};
    use splot_core::symbol::CdfUpdateMode;

    use super::super::cdf::{
        FrameCdfSubset, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy,
    };
    use super::super::partition_allowed::PartitionFeatureFlags;
    use super::super::partition_traversal::{
        TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
        TilePartitionLoopRestorationState, TilePartitionTraversalError,
        TilePartitionTraversalInput, plan_tile_partition_traversal_cursor,
    };
    use super::super::{SymbolInitBoundary, TileBruPath, TilePayloadSource};
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
