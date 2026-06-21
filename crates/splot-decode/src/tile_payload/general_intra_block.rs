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
    IntraYMode, MODE_INDEX_COUNT, SupportedChromaMode, SupportedDirectionalLumaMode,
    SupportedNonDcLumaMode, reconstruct_minimal_y_mode, reconstruct_y_mode_offset_escape_top_left,
    supported_chroma_mode, uv_mode_ctx,
};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};
use super::intra_joint_modes::TileIntraJointModeState;

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
const Y_MODE_OFFSET_REASON: &str = "intra_y_mode_offset";
const UV_MODE_REASON: &str = "intra_uv_mode";
const UV_MODE_IDX_REASON: &str = "intra_uv_mode_idx";

/// The decoded mode-info facts for one general intra block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraBlockModes {
    /// The reconstructed typed luma intra mode (§ 5.20.5.3 `read_intra_y_mode`).
    pub(crate) y_mode: IntraYMode,
    /// The reconstructed `AngleDeltaY` (§ 5.20.5.3), `0` for non-directional
    /// modes and for the supported directional subset.
    pub(crate) angle_delta_y: i8,
    /// The decoded `uv_mode` value (after the `CHROMA_MODE_COUNT - 1` escape),
    /// the index into the chroma mode list; typed `UVMode` reconstruction is a
    /// future increment.
    pub(crate) uv_mode: u8,
    /// The AV2 § 5.20.5.3 `IntraJointMode` (`= modeDelta`, the reorder index)
    /// stored into `IntraJointModes` for this block, which feeds the § 8.3.2
    /// `y_mode_index` neighbour context of later blocks. A directional mode has
    /// `intra_joint_mode >= NON_DIRECTIONAL_MODES_COUNT`.
    pub(crate) intra_joint_mode: u8,
}

impl GeneralIntraBlockModes {
    /// True when the luma plane uses `DC_PRED`.
    pub(crate) fn luma_is_dc(&self) -> bool {
        self.y_mode == IntraYMode::DC_PRED
    }

    /// The supported non-DC luma predictor for this block, or `None` for DC and
    /// the not-yet-supported non-DC luma modes (see [`IntraYMode::supported_nondc`]).
    pub(crate) fn supported_nondc_luma(&self) -> Option<SupportedNonDcLumaMode> {
        self.y_mode.supported_nondc()
    }

    /// The supported directional-angle luma predictor for this block, or `None`
    /// for non-directional modes and the not-yet-supported directional modes /
    /// non-zero angle deltas (see [`IntraYMode::supported_directional`]). A
    /// directional mode with a non-zero `AngleDeltaY` is reported as unsupported
    /// because only `AngleDeltaY == 0` (pAngle 135) is verified.
    pub(crate) fn supported_directional_luma(&self) -> Option<SupportedDirectionalLumaMode> {
        if self.angle_delta_y != 0 {
            return None;
        }
        self.y_mode.supported_directional()
    }

    /// The supported chroma predictor for this block (DC or SMOOTH), resolving
    /// the decoded `uv_mode` index through § 5.20.5.3 `get_intra_uv_mode_set`
    /// (handling both the non-directional and directional luma branches), or
    /// `None` for an unsupported chroma mode (see [`supported_chroma_mode`]).
    pub(crate) fn supported_chroma_mode(&self) -> Option<SupportedChromaMode> {
        supported_chroma_mode(self.y_mode, self.uv_mode)
    }
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
    /// The block hit the AV2 § 5.20.5.3 `y_mode_offset` escape
    /// (`y_mode_set == 0`, `y_mode_index == MODE_INDEX_COUNT - 1`) while it has a
    /// directional joint-mode neighbour (`ctx != 0`). The escape's `modeIdx` is
    /// `>= NON_DIRECTIONAL_MODES_COUNT`, so `get_intra_y_mode_set` enters its
    /// directional selection loop, which preselects the neighbours' (and, for
    /// `Block_Width * Block_Height > 64`, their ±1..4 expanded) modes ahead of the
    /// `Default_Mode_List_Y` scan — a reorder
    /// [`reconstruct_y_mode_offset_escape_top_left`] does not model. The resolved
    /// mode would be directional (needing the deferred § 7.13.2.8 luma IDIF), so it
    /// is deferred to a future increment. The `y_mode_offset` symbol has already
    /// been consumed when this is returned.
    #[error(
        "general intra mode-info y_mode_offset escape with a directional neighbour (ctx {ctx}, y_mode_offset {y_mode_offset}) needs the unmodelled §5.20.5.3 directional-neighbour reorder"
    )]
    UnsupportedDirectionalNeighbourReorder {
        /// The computed § 8.3.2 `y_mode_index` context (`1` or `2`).
        ctx: usize,
        /// The decoded `y_mode_offset` whose escape `modeIdx` needs the reorder.
        y_mode_offset: u8,
    },
}

/// Decodes the AV2 § 5.20.5.3 mode-info symbols for one general intra block,
/// returning the reconstructed luma `YMode`, the decoded `uv_mode`, and the
/// stored `IntraJointMode` (`modeDelta`).
///
/// `joint_modes` is the tile's per-MI `IntraJointModes` grid (§ 5.20.5.3); the
/// block's MI position (`block_r`, `block_c`) and MI width/height (`block_n4w`,
/// `block_n4h`, `Num_4x4_Blocks_Wide/High[MiSize]`) select the left/above
/// neighbours for the § 8.3.2 `y_mode_index` context.
pub(crate) fn decode_general_intra_block_modes(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    joint_modes: &TileIntraJointModeState,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    // AV2 § 8.3.2 `y_mode_index` / `y_mode_offset` CDF context, derived from the
    // already-decoded left/above neighbours' stored `IntraJointMode` (§ 5.20.5.3
    // `get_joint_mode`). `ctx` is `0`, `1`, or `2` — the number of directional
    // (`>= NON_DIRECTIONAL_MODES_COUNT`) left/above neighbours — and indexes the
    // `TileYModeIndexCdf[ctx]` / `TileYModeOffsetCdf[ctx]` banks. The full `0..=2`
    // range is now used directly (the `ctx != 0` selection is verified bit-exact
    // against the AVM/dav2d oracle: a block whose left neighbour is the D135
    // directional superblock decodes its non-directional luma mode with the
    // `ctx == 1` CDF row, `syn-dirneigh-intra-128x64-q80`).
    let mode_ctx = joint_modes.y_mode_index_ctx(block_r, block_c, block_n4w, block_n4h);

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // read_intra_y_mode(): y_mode_set (§ 8.3.2 `TileYModeSetCdf`, no context).
    let y_mode_set = read_symbol(cdfs, symbols, TileCdfSelector::YModeSet, Y_MODE_SET_REASON)?;

    // y_mode_index (§ 8.3.2 `TileYModeIndexCdf[ctx]`, ctx from `get_joint_mode`).
    let y_mode_index = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::YModeIndex { ctx: mode_ctx },
        Y_MODE_INDEX_REASON,
    )?;

    // Reconstruct the typed luma `YMode`, `AngleDeltaY`, and the stored
    // `IntraJointMode` (`modeDelta`) (§ 5.20.5.3 `read_intra_y_mode`,
    // `get_intra_y_mode_set`, `Reordered_Y_Mode`).
    //
    // For `y_mode_set == 0`, `y_mode_index == MODE_INDEX_COUNT - 1` is the
    // `y_mode_offset` escape (§ 5.20.5.3): read `y_mode_offset` (§ 8.3.2
    // `TileYModeOffsetCdf[ctx]`, sharing the `y_mode_index` context). The escape's
    // `modeIdx == (MODE_INDEX_COUNT - 1) + y_mode_offset` is always
    // `>= NON_DIRECTIONAL_MODES_COUNT`, so `get_intra_y_mode_set(modeIdx)` enters
    // its directional selection loop (§ 5.20.5.3 lines 11120-11176). When there is
    // a directional joint-mode neighbour (`mode_ctx != 0`) that loop preselects the
    // neighbours' modes (and, for `Block_Width * Block_Height > 64`, their ±1..4
    // expansion) BEFORE the `Default_Mode_List_Y` scan, reordering the candidate
    // list; [`reconstruct_y_mode_offset_escape_top_left`] only models the
    // no-directional-neighbour case (both `get_joint_mode` out of frame /
    // non-directional, `count == 0`). The directional-neighbour escape resolves to
    // a directional `YMode`, which needs the deferred § 7.13.2.8 luma IDIF anyway,
    // so it is rejected here (the symbol has been consumed; the caller surfaces it
    // as an unsupported feature). Otherwise the non-directional `Reordered_Y_Mode`
    // prefix maps the index directly and `IntraJointMode == modeDelta == y_mode_index`
    // (`modeIdx < NON_DIRECTIONAL_MODES_COUNT` passes through `get_intra_y_mode_set`
    // unchanged regardless of neighbours, § 5.20.5.3 lines 11036 + 11117-11118).
    let (y_mode, angle_delta_y, intra_joint_mode) =
        if y_mode_set == 0 && y_mode_index == MODE_INDEX_COUNT - 1 {
            let y_mode_offset = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::YModeOffset { ctx: mode_ctx },
                Y_MODE_OFFSET_REASON,
            )?;
            if mode_ctx != 0 {
                return Err(
                    GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder {
                        ctx: mode_ctx,
                        y_mode_offset,
                    },
                );
            }
            let escape = reconstruct_y_mode_offset_escape_top_left(y_mode_offset).ok_or(
                GeneralIntraBlockModeError::UnsupportedYMode {
                    y_mode_set,
                    y_mode_index,
                },
            )?;
            (escape.y_mode, escape.angle_delta_y, escape.intra_joint_mode)
        } else {
            let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
                GeneralIntraBlockModeError::UnsupportedYMode {
                    y_mode_set,
                    y_mode_index,
                },
            )?;
            (y_mode, 0, y_mode_index)
        };

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

    if std::env::var("SPLOT_DBG_MODE").is_ok() {
        eprintln!(
            "DBG block r={block_r} c={block_c} ctx={mode_ctx} y_mode_set={y_mode_set} y_mode_index={y_mode_index} y_mode={y_mode:?} angle={angle_delta_y} joint={intra_joint_mode} uv_mode={uv_mode}"
        );
    }
    Ok(GeneralIntraBlockModes {
        y_mode,
        angle_delta_y,
        uv_mode,
        intra_joint_mode,
    })
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

    // A 64x64 superblock is 16x16 MI units (Num_4x4_Blocks_Wide/High).
    const SB_N4: usize = 16;
    // A representative directional IntraJointMode (>= NON_DIRECTIONAL_MODES_COUNT):
    // the merged D135 modeDelta 36 (§ 5.20.5.3).
    const D135_JOINT_MODE: u8 = 36;
    // A representative non-directional IntraJointMode (< NON_DIRECTIONAL_MODES_COUNT):
    // SMOOTH_V modeDelta 2.
    const SMOOTH_V_JOINT_MODE: u8 = 2;

    fn empty_joint_modes() -> TileIntraJointModeState {
        TileIntraJointModeState::new(SB_N4, 2 * SB_N4).unwrap()
    }

    #[test]
    fn decodes_dc_luma_mode_and_a_chroma_mode_in_spec_order() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_frontier(&mut work_unit);
        let joint_modes = empty_joint_modes();

        // Top-left block (0, 0): out-of-frame neighbours -> ctx 0.
        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            &joint_modes,
            0,
            0,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        // y_mode_set == 0, y_mode_index == 0 -> DC_PRED (the same first two
        // symbols the frozen trace decodes; the general path reads them without
        // asserting and reconstructs the typed mode).
        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
        // DC_PRED is non-directional: IntraJointMode == modeDelta == y_mode_index == 0.
        assert_eq!(modes.intra_joint_mode, 0);
        // The decoded uv_mode is a valid chroma-mode-list index for the
        // CfL-not-allowed set (after any escape extension); out-of-range values
        // are rejected before constructing GeneralIntraBlockModes.
        assert!(
            modes.uv_mode < UV_INTRA_MODES_CFL_NOT_ALLOWED,
            "uv_mode {} out of range",
            modes.uv_mode
        );
    }

    #[test]
    fn non_directional_left_neighbour_keeps_ctx_zero_and_decodes() {
        // The verified mbvg case: a left neighbour storing a non-directional
        // IntraJointMode (SMOOTH_V, modeDelta 2 < 5) keeps the § 8.3.2 context 0,
        // so the right block decodes exactly as the top-left does (same CDF row).
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_frontier(&mut work_unit);
        let mut joint_modes = empty_joint_modes();
        joint_modes.record_block(0, 0, SB_N4, SB_N4, SMOOTH_V_JOINT_MODE);

        // The right superblock at (0, 16) reads the non-directional left neighbour.
        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            &joint_modes,
            0,
            SB_N4,
            SB_N4,
            SB_N4,
        )
        .unwrap();
        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
    }

    #[test]
    fn directional_neighbour_ctx_reads_with_the_real_context() {
        // A left neighbour storing a directional IntraJointMode (D135, modeDelta
        // 36 >= 5) makes the § 8.3.2 `y_mode_index` context 1. The decode no longer
        // rejects ctx != 0: it reads `y_mode_set` / `y_mode_index` from the real
        // `TileYModeIndexCdf[1]` row (verified bit-exact by the
        // `syn-dirneigh-intra-128x64-q80` oracle fixture), consuming symbols.
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_frontier(&mut work_unit);
        let symbol_count_before = symbols.symbol_count();
        let mut joint_modes = empty_joint_modes();
        joint_modes.record_block(0, 0, SB_N4, SB_N4, D135_JOINT_MODE);

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            &joint_modes,
            0,
            SB_N4,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        // Symbols were consumed (the ctx != 0 read is no longer short-circuited).
        assert!(symbols.symbol_count() > symbol_count_before);
        // The reconstructed mode is a valid luma intra mode and (for this trace) a
        // non-directional one — the verified neighbour-reading subset.
        assert!(!modes.y_mode.is_directional());
    }
}
