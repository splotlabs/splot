// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra block mode-info decode for the AVM-oracle general intra path.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-BLOCK-MODES`.
//!
//! This decodes the AV2 § 5.20.5.3 `intra_frame_mode_info()` mode symbols for a
//! single minimal-tool intra key-frame block — `read_intra_y_mode()` then
//! `read_intra_uv_mode()` — in spec order, without the frozen minimal-tier
//! trace's hardcoded value assertions. For the supported minimal-tool subset
//! (no intra block copy, segmentation, GDF, CDEF, CCSO, delta-Q, lossless DPCM,
//! palette, DIP, FSC, MRL, CfL, or MHCCP), those tool branches read no symbols,
//! so the only mode symbols are `y_mode_set`, `y_mode_index`, and `uv_mode`
//! (with the `uv_mode == CHROMA_MODE_COUNT - 1` escape literal).
//!
//! Scope: it decodes and consumes the mode symbols and reconstructs the typed
//! luma `YMode`; the typed `UVMode` reconstruction (`get_intra_uv_mode_set`),
//! the residual / transform-block syntax, coefficient decode, dequantization,
//! inverse transform, reconstruction, and output remain future increments.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::DecodeTileWorkUnit;
use super::cdf::block_context::{
    IntraYMode, YModeIndexContext, reconstruct_minimal_y_mode, uv_mode_ctx,
};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};

/// AV2 § 3 `CHROMA_MODE_COUNT`: the number of values for the `uv_mode` symbol
/// (`03-symbols.md`); `uv_mode == CHROMA_MODE_COUNT - 1` triggers the
/// `uv_mode_idx` `L(3)` escape (§ 5.20.5.3 `read_intra_uv_mode`).
const CHROMA_MODE_COUNT: u8 = 8;

/// AV2 § 3 `UV_INTRA_MODES_CFL_NOT_ALLOWED` (`03-symbols.md`): the number of
/// chroma intra modes when CfL is not allowed; the decoded `uv_mode` (after the
/// escape) must index this list (`0..UV_INTRA_MODES_CFL_NOT_ALLOWED`).
const UV_INTRA_MODES_CFL_NOT_ALLOWED: u8 = 13;

/// AV2 § 5.20.5.3 `uv_mode_idx` literal width (`L(3)`).
const UV_MODE_IDX_BITS: u32 = 3;

const Y_MODE_SET_REASON: &str = "intra_y_mode_set";
const Y_MODE_INDEX_REASON: &str = "intra_y_mode_index";
const UV_MODE_REASON: &str = "intra_uv_mode";
const UV_MODE_IDX_REASON: &str = "intra_uv_mode_idx";

/// The decoded mode-info facts for one general intra block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraBlockModes {
    /// The reconstructed typed luma intra mode (§ 5.20.5.3 `read_intra_y_mode`).
    pub(crate) y_mode: IntraYMode,
    /// The decoded `uv_mode` value (after the `CHROMA_MODE_COUNT - 1` escape),
    /// the index into the chroma mode list; typed `UVMode` reconstruction is a
    /// future increment.
    pub(crate) uv_mode: u8,
}

/// Error returned while decoding general intra block mode info.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraBlockModeError {
    /// A mode-info CDF symbol read failed.
    #[error("general intra mode-info symbol read failed for {reason}: {source}")]
    SymbolRead {
        /// Stable symbol reason.
        reason: &'static str,
        /// Source CDF selection or symbol-decoder error.
        source: BlockSymbolTraceReadError,
    },
    /// A mode-info escape literal read failed.
    #[error("general intra mode-info literal read failed for {reason}: {source}")]
    Literal {
        /// Stable literal reason.
        reason: &'static str,
        /// Source symbol-decoder error.
        source: CoreError,
    },
    /// The decoded `y_mode_set` / `y_mode_index` fell outside the supported
    /// minimal-tool luma `YMode` reconstruction subset (non-zero `y_mode_set`,
    /// the directional reorder path, or the `MODE_INDEX_COUNT - 1`
    /// `y_mode_offset` escape — all deferred to a future increment).
    #[error(
        "general intra mode-info cannot reconstruct YMode for y_mode_set {y_mode_set}, y_mode_index {y_mode_index}"
    )]
    UnsupportedYMode {
        /// Decoded `y_mode_set` value.
        y_mode_set: u8,
        /// Decoded `y_mode_index` value.
        y_mode_index: u8,
    },
    /// The decoded `uv_mode` (after the `uv_mode_idx` escape) indexed past the
    /// CfL-not-allowed chroma mode list (`>= UV_INTRA_MODES_CFL_NOT_ALLOWED`),
    /// so `get_intra_uv_mode_set` has no entry for it (malformed or unsupported
    /// chroma mode syntax).
    #[error("general intra mode-info decoded out-of-range uv_mode {uv_mode}")]
    InvalidUvMode {
        /// Decoded `uv_mode` value.
        uv_mode: u8,
    },
}

/// Decodes the AV2 § 5.20.5.3 mode-info symbols for the single minimal-tool
/// intra block, returning the reconstructed luma `YMode` and the decoded
/// `uv_mode`.
pub(crate) fn decode_general_intra_block_modes(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // read_intra_y_mode(): y_mode_set (§ 8.3.2 `TileYModeSetCdf`, no context).
    let y_mode_set = read_symbol(cdfs, symbols, TileCdfSelector::YModeSet, Y_MODE_SET_REASON)?;

    // y_mode_index (§ 8.3.2 context from `get_joint_mode`; both neighbours are
    // out of frame at the single-block tile origin, so the context is 0).
    let y_mode_index = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::YModeIndex {
            ctx: YModeIndexContext::tile_origin_block().ctx(),
        },
        Y_MODE_INDEX_REASON,
    )?;

    // Reconstruct the typed luma `YMode` (§ 5.20.5.3 `read_intra_y_mode`,
    // `get_intra_y_mode_set`, `Reordered_Y_Mode`); the minimal-tool subset
    // decodes a non-directional mode.
    let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
        GeneralIntraBlockModeError::UnsupportedYMode {
            y_mode_set,
            y_mode_index,
        },
    )?;

    // read_intra_uv_mode(): uv_mode (§ 8.3.2 `TileUVModeCflNotAllowedCdf[ctx]`,
    // `ctx = is_directional_mode(YMode)`). CfL and MHCCP are disabled, so the
    // `is_cfl` symbol is not read.
    let uv_mode_base = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::UvModeCflNotAllowed {
            ctx: uv_mode_ctx(y_mode),
        },
        UV_MODE_REASON,
    )?;

    // The `uv_mode == CHROMA_MODE_COUNT - 1` escape adds an `L(3)` `uv_mode_idx`
    // (§ 5.20.5.3 `read_intra_uv_mode`).
    let uv_mode = if uv_mode_base == CHROMA_MODE_COUNT - 1 {
        let uv_mode_idx = symbols.read_literal(UV_MODE_IDX_BITS).map_err(|source| {
            GeneralIntraBlockModeError::Literal {
                reason: UV_MODE_IDX_REASON,
                source,
            }
        })?;
        uv_mode_base.saturating_add(uv_mode_idx as u8)
    } else {
        uv_mode_base
    };

    // The decoded `uv_mode` must index the CfL-not-allowed chroma mode list; the
    // `uv_mode_idx` escape can otherwise produce 13 or 14, which
    // `get_intra_uv_mode_set` cannot map.
    if uv_mode >= UV_INTRA_MODES_CFL_NOT_ALLOWED {
        return Err(GeneralIntraBlockModeError::InvalidUvMode { uv_mode });
    }

    Ok(GeneralIntraBlockModes { y_mode, uv_mode })
}

/// Reads one mode-info `S()` symbol, mapping a CDF/symbol failure to a typed
/// error and returning the decoded value.
fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    reason: &'static str,
) -> Result<u8, GeneralIntraBlockModeError> {
    cdfs.read_block_symbol_trace(selector, symbols)
        .map(|symbol| symbol.get())
        .map_err(|source| GeneralIntraBlockModeError::SymbolRead { reason, source })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use core::ops::Range;

    use splot_core::segment::MAX_SEGMENTS;
    use splot_core::span::{ByteOffset, ByteSpan};
    use splot_core::symbol::CdfUpdateMode;

    use super::super::cdf::{
        FrameCdfSubset, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy,
    };
    use super::super::partition_allowed::PartitionFeatureFlags;
    use super::super::partition_traversal::{
        TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
        TilePartitionLoopRestorationState, TilePartitionTraversalInput,
        plan_tile_partition_traversal_cursor,
    };
    use super::super::{SymbolInitBoundary, TileBruPath, TileCoeffFrameFacts, TilePayloadSource};
    use super::*;
    use crate::{DecodeLayerSelection, DecodeLimits, DecodeObuSourceKind};

    const BLOCK_64X64: usize = 12;
    const BLOCK_256X256: usize = 18;
    // The same hand-crafted minimal tile payload the frozen block-symbol trace
    // tests use: its first two block symbols decode `y_mode_set == 0` and
    // `y_mode_index == 0` (DC_PRED), proving spec-order mode decode on the
    // general path.
    const PAYLOAD: [u8; 2] = [0x12, 0xFB];

    fn make_work_unit<'payload>(payload: &'payload [u8]) -> DecodeTileWorkUnit<'payload> {
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
                cdf_update_mode: CdfUpdateMode::Disabled,
            },
            cdf: TileCdfWorkUnitBoundary::new(
                CdfUpdateMode::Disabled,
                tile_cdf_save_policy(TileCdfPolicyInput::single_tile_default(), 0).unwrap(),
                FrameCdfSubset::from_defaults(),
            ),
        }
    }

    fn symbols_at_block_frontier<'payload>(
        work_unit: &mut DecodeTileWorkUnit<'payload>,
    ) -> SymbolDecoder<'payload> {
        let rows = vec![vec![BLOCK_256X256; 16]; 16];
        let mi0_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let mi1_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let edge = vec![BLOCK_256X256; 16];
        let context =
            TilePartitionContextState::new([&mi0_rows, &mi1_rows], [&edge, &edge], [&edge, &edge]);
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
        ))
        .unwrap();
        let (_plan, symbols) = cursor.into_parts();
        symbols
    }

    #[test]
    fn decodes_dc_luma_mode_and_a_chroma_mode_in_spec_order() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_frontier(&mut work_unit);

        let modes = decode_general_intra_block_modes(&mut work_unit, &mut symbols).unwrap();

        // y_mode_set == 0, y_mode_index == 0 -> DC_PRED (the same first two
        // symbols the frozen trace decodes; the general path reads them without
        // asserting and reconstructs the typed mode).
        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
        // The decoded uv_mode is a valid chroma-mode-list index for the
        // CfL-not-allowed set (after any escape extension); out-of-range values
        // are rejected before constructing GeneralIntraBlockModes.
        assert!(
            modes.uv_mode < UV_INTRA_MODES_CFL_NOT_ALLOWED,
            "uv_mode {} out of range",
            modes.uv_mode
        );
    }
}
