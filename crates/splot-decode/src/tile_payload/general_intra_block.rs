// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra block mode-info decode for the AVM-oracle general intra path.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-BLOCK-MODES`.
//!
//! This decodes the AV2 § 5.20.5.3 `intra_frame_mode_info()` mode symbols for a
//! single minimal-tool intra key-frame block — `read_intra_y_mode()` then
//! `read_intra_uv_mode()` — in spec order, without the frozen minimal-tier
//! trace's hardcoded value assertions. For the supported minimal-tool subset,
//! most tool branches read no symbols, so the core mode symbols are
//! `y_mode_set`, `y_mode_index`, and `uv_mode` (with the
//! `uv_mode == CHROMA_MODE_COUNT - 1` escape literal). When the sequence allows
//! FSC or MRL syntax, this module also consumes inactive `fsc_mode == 0` and
//! retains decoded MRL metadata for callers that can stay syntax-only before
//! sample prediction.
//!
//! Scope: it decodes and consumes the mode symbols and reconstructs the typed
//! luma `YMode`; the typed `UVMode` reconstruction (`get_intra_uv_mode_set`),
//! the residual / transform-block syntax, coefficient decode, dequantization,
//! inverse transform, reconstruction, and output remain future increments.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::SIZE_GROUP;

use super::DecodeTileWorkUnit;
use super::cdf::block_context::{
    IntraYMode, MODE_INDEX_COUNT, NON_DIRECTIONAL_MODES_COUNT, SupportedChromaMode,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode, YModeEscapeResult,
    reconstruct_minimal_y_mode, reconstruct_y_mode_first_set_directional_top_left,
    reconstruct_y_mode_offset_escape_top_left, reconstruct_y_mode_second_set_top_left,
    reconstruct_y_mode_with_neighbours, supported_chroma_mode, uv_mode_ctx,
};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};
use super::intra_joint_modes::{TileFscModeState, TileIntraJointModeState, TileUsesMrlsState};

/// AV2 § 3 `CHROMA_MODE_COUNT`: the number of values for the `uv_mode` symbol
/// (`03-symbols.md`); `uv_mode == CHROMA_MODE_COUNT - 1` triggers the
/// `uv_mode_idx` `L(3)` escape (§ 5.20.5.3 `read_intra_uv_mode`).
const CHROMA_MODE_COUNT: u8 = 8;

/// AV2 § 3 `UV_INTRA_MODES_CFL_NOT_ALLOWED` (`03-symbols.md`): the number of
/// chroma intra modes when CfL is not allowed; the decoded `uv_mode` (after the
/// escape) must index this list (`0..UV_INTRA_MODES_CFL_NOT_ALLOWED`).
const UV_INTRA_MODES_CFL_NOT_ALLOWED: u8 = 13;
/// AV2 §6.19.7.4 `UVMode`: `UV_CFL_PRED`.
const UV_CFL_PRED_MODE: u8 = 13;

/// AV2 § 5.20.5.3 `uv_mode_idx` literal width (`L(3)`).
const UV_MODE_IDX_BITS: u32 = 3;
/// AV2 § 5.20.5.5 `y_second_mode` literal width (`L(4)`).
const Y_SECOND_MODE_BITS: u32 = 4;
/// AV2 § 3 `FIRST_MODE_COUNT`: first second-mode `modeIdx` value.
const FIRST_MODE_COUNT: usize = 13;
/// AV2 § 3 `SECOND_MODE_COUNT`: values per non-zero `y_mode_set`.
const SECOND_MODE_COUNT: usize = 16;

const Y_MODE_SET_REASON: &str = "intra_y_mode_set";
const Y_MODE_INDEX_REASON: &str = "intra_y_mode_index";
const Y_MODE_OFFSET_REASON: &str = "intra_y_mode_offset";
const Y_SECOND_MODE_REASON: &str = "intra_y_second_mode";
const UV_MODE_REASON: &str = "intra_uv_mode";
const UV_MODE_IDX_REASON: &str = "intra_uv_mode_idx";
const IS_CFL_REASON: &str = "intra_is_cfl";
const CFL_INDEX_REASON: &str = "intra_cfl_index";
const CFL_SIGN_REASON: &str = "intra_cfl_alpha_signs";
const CFL_ALPHA_U_REASON: &str = "intra_cfl_alpha_u";
const CFL_ALPHA_V_REASON: &str = "intra_cfl_alpha_v";
const CFL_MHCCP_REASON: &str = "intra_cfl_mhccp";
const CFL_MH_DIR_REASON: &str = "intra_cfl_mh_dir";
const FSC_MODE_REASON: &str = "intra_fsc_mode";
const MRL_INDEX_REASON: &str = "intra_mrl_index";
const MRL_SEC_INDEX_REASON: &str = "intra_mrl_sec_index";
const FSC_MAX_SAMPLES: usize = 32;
const FSC_BSIZE_GROUPS: [usize; 29] = [
    0, 1, 1, 2, 3, 3, 4, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 3, 3, 4, 4, 6, 6, 4, 4, 6, 6,
];
const CFL_EXPLICIT: u8 = 0;
const CFL_MULTI: u8 = 2;
const CFL_SIGN_ZERO: u8 = 0;

/// Sequence tool flags that affect §5.20.5.5/§5.20.5.6 intra mode syntax.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaToolConfig {
    enable_cfl_intra: bool,
    enable_mhccp: bool,
    enable_idtx_intra: bool,
    enable_mrls: bool,
}

impl GeneralIntraChromaToolConfig {
    /// Creates a config from parsed sequence chroma-tool flags.
    #[must_use]
    pub(crate) const fn new(enable_cfl_intra: bool, enable_mhccp: bool) -> Self {
        Self {
            enable_cfl_intra,
            enable_mhccp,
            enable_idtx_intra: false,
            enable_mrls: false,
        }
    }

    /// Returns a copy with the parsed `enable_idtx_intra` transform flag.
    #[must_use]
    pub(crate) const fn with_enable_idtx_intra(mut self, enable_idtx_intra: bool) -> Self {
        self.enable_idtx_intra = enable_idtx_intra;
        self
    }

    /// Returns a copy with the parsed `enable_mrls` intra flag.
    #[must_use]
    pub(crate) const fn with_enable_mrls(mut self, enable_mrls: bool) -> Self {
        self.enable_mrls = enable_mrls;
        self
    }

    /// No sequence chroma tools are enabled.
    #[must_use]
    pub(crate) const fn disabled() -> Self {
        Self::new(false, false)
    }
}

/// Tree/context facts that affect AV2 §5.20.5.6 `read_intra_uv_mode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaModeContext {
    cfl_allowed_in_sdp: bool,
}

impl GeneralIntraChromaModeContext {
    /// Context for non-SDP or non-chroma-part callers.
    #[must_use]
    pub(crate) const fn shared_or_non_sdp() -> Self {
        Self {
            cfl_allowed_in_sdp: true,
        }
    }

    /// Context for an SDP `CHROMA_PART` leaf with retained §5.20.3.1 state.
    #[must_use]
    pub(crate) const fn sdp_chroma_part(cfl_allowed_in_sdp: bool) -> Self {
        Self { cfl_allowed_in_sdp }
    }
}

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
    /// The `UVMode` value handed to coefficient transform-type derivation. For
    /// active CfL this is `UV_CFL_PRED`; for existing no-CfL paths it preserves
    /// the prior decoded-index behavior until those paths are widened separately.
    coeff_uv_mode: u8,
    /// True when §5.20.5.6 selected `UV_CFL_PRED`.
    is_cfl: bool,
    /// The AV2 § 5.20.5.3 `IntraJointMode` (`= modeDelta`, the reorder index)
    /// stored into `IntraJointModes` for this block, which feeds the § 8.3.2
    /// `y_mode_index` neighbour context of later blocks. A directional mode has
    /// `intra_joint_mode >= NON_DIRECTIONAL_MODES_COUNT`.
    pub(crate) intra_joint_mode: u8,
    /// Decoded AV2 §5.20.5.5 `mrl_index`; zero when MRL syntax is disabled or
    /// not present for this luma mode.
    pub(crate) mrl_index: u8,
    /// Decoded AV2 §5.20.5.5 `mrl_sec_index` when `mrl_index > 0`.
    pub(crate) mrl_sec_index: Option<u8>,
    /// Decoded AV2 §5.20.5.3 `fsc_mode`.
    pub(crate) fsc_mode: u8,
    /// Derived AV2 §5.20.5.3 `UsesMrls` value stored for later neighbours.
    pub(crate) uses_mrls: u8,
}

/// Decoded chroma-side mode-info facts for one intra block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaBlockMode {
    uv_mode: u8,
    coeff_uv_mode: u8,
    is_cfl: bool,
}

impl GeneralIntraChromaBlockMode {
    const fn no_cfl(uv_mode: u8) -> Self {
        Self {
            uv_mode,
            coeff_uv_mode: uv_mode,
            is_cfl: false,
        }
    }

    const fn cfl() -> Self {
        Self {
            uv_mode: UV_CFL_PRED_MODE,
            coeff_uv_mode: UV_CFL_PRED_MODE,
            is_cfl: true,
        }
    }

    /// The legacy decoded `uv_mode` value used by existing no-CfL prediction
    /// helpers.
    pub(crate) const fn uv_mode(self) -> u8 {
        self.uv_mode
    }

    /// The `UVMode` value used by coefficient transform-type derivation.
    pub(crate) const fn coeff_uv_mode(self) -> usize {
        self.coeff_uv_mode as usize
    }

    /// True when this mode selected `UV_CFL_PRED`.
    pub(crate) const fn is_cfl(self) -> bool {
        self.is_cfl
    }
}

/// The decoded luma-side mode-info facts for one intra block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraLumaBlockMode {
    /// The reconstructed typed luma intra mode (§ 5.20.5.3 `read_intra_y_mode`).
    pub(crate) y_mode: IntraYMode,
    /// The reconstructed `AngleDeltaY`.
    pub(crate) angle_delta_y: i8,
    /// The AV2 § 5.20.5.3 `IntraJointMode` stored for neighbour contexts.
    pub(crate) intra_joint_mode: u8,
    /// Decoded AV2 §5.20.5.5 `mrl_index`; zero when no MRL syntax is active.
    pub(crate) mrl_index: u8,
    /// Decoded AV2 §5.20.5.5 `mrl_sec_index` when `mrl_index > 0`.
    pub(crate) mrl_sec_index: Option<u8>,
    /// Decoded AV2 §5.20.5.3 `fsc_mode`.
    pub(crate) fsc_mode: u8,
    /// Derived AV2 §5.20.5.3 `UsesMrls` value stored for later neighbours.
    pub(crate) uses_mrls: u8,
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
    /// because only `AngleDeltaY == 0` (the cardinal pAngles 90/180 and the
    /// middle pAngles 135/157) is verified.
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
        if self.is_cfl {
            return None;
        }
        supported_chroma_mode(self.y_mode, self.uv_mode)
    }

    /// The `UVMode` value used by coefficient transform-type derivation.
    pub(crate) const fn coeff_uv_mode(&self) -> usize {
        self.coeff_uv_mode as usize
    }

    /// True when decoded sample prediction would need AV2 §7.13.2 MRL support.
    pub(crate) const fn uses_active_mrl(&self) -> bool {
        self.uses_mrls != 0
    }

    /// True when decoded sample prediction would need FSC/IDTX coefficient support.
    pub(crate) const fn uses_active_fsc(&self) -> bool {
        self.fsc_mode != 0
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
    /// The decoded luma mode syntax resolved to a §5.20.5.5 `modeIdx` outside
    /// the supported `YMode` reconstruction subset.
    #[error(
        "general intra mode-info cannot reconstruct YMode for y_mode_set {y_mode_set}, modeIdx {mode_idx}"
    )]
    UnsupportedYMode {
        /// Decoded `y_mode_set` value.
        y_mode_set: u8,
        /// Resolved §5.20.5.5 `modeIdx` value.
        mode_idx: usize,
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
    /// The block-size index used to select `Fsc_Bsize_Groups[MiSize]` is outside
    /// the mirrored table.
    #[error("general intra mode-info block-size index {block_size_index} has no FSC group")]
    InvalidFscBlockSizeIndex {
        /// `MiSize` block-size index.
        block_size_index: usize,
    },
    /// The block-size index used to select `Size_Group[MiSize]` for
    /// `cfl_mh_dir` is outside the mirrored table.
    #[error(
        "general intra mode-info block-size index {block_size_index} has no CfL MH direction size group"
    )]
    InvalidCflMhDirBlockSizeIndex {
        /// `MiSize` block-size index.
        block_size_index: usize,
    },
    /// The block selected the MHCCP-enabled chroma-from-luma path.
    #[error("general intra mode-info selected unsupported MHCCP chroma prediction")]
    UnsupportedMhccpMode,
    /// The block hit a directional luma `modeIdx` while it has a directional
    /// joint-mode neighbour (`ctx != 0`). `get_intra_y_mode_set` preselects the
    /// neighbours' (and, for `Block_Width * Block_Height > 64`, their ±1..4
    /// expanded) modes ahead of the `Default_Mode_List_Y` scan — a reorder the
    /// current top-left-equivalent helpers do not model. The luma syntax element
    /// that produced `modeIdx` has already been consumed when this is returned.
    #[error(
        "general intra mode-info modeIdx {mode_idx} with a directional neighbour (ctx {ctx}) needs the unmodelled §5.20.5.5 directional-neighbour reorder"
    )]
    UnsupportedDirectionalNeighbourReorder {
        /// The computed § 8.3.2 `y_mode_index` context (`1` or `2`).
        ctx: usize,
        /// The resolved §5.20.5.5 `modeIdx` whose reorder needs neighbour state.
        mode_idx: usize,
    },
}

/// Decodes the AV2 § 5.20.5.3 luma-side mode-info symbols for one general
/// intra luma/shared block.
///
/// `joint_modes` is the tile's per-MI `IntraJointModes` grid (§ 5.20.5.3); the
/// sibling `uses_mrls` grid supplies the MRL contexts. The block's `MiSize`
/// index (`block_size_index`), MI position (`block_r`, `block_c`), and MI
/// width/height (`block_n4w`, `block_n4h`, `Num_4x4_Blocks_Wide/High[MiSize]`)
/// select the FSC group and left/above neighbours for the § 8.3.2 contexts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_luma_block_mode(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraLumaBlockMode, GeneralIntraBlockModeError> {
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
    let neighbour_joint_modes =
        joint_modes.neighbour_joint_modes(block_r, block_c, block_n4w, block_n4h);

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // read_intra_y_mode(): y_mode_set (§ 8.3.2 `TileYModeSetCdf`, no context).
    let y_mode_set = read_symbol(cdfs, symbols, TileCdfSelector::YModeSet, Y_MODE_SET_REASON)?;

    // Reconstruct the typed luma `YMode`, `AngleDeltaY`, and the stored
    // `IntraJointMode` (`modeDelta`) (§ 5.20.5.5 `read_intra_y_mode`,
    // `get_intra_y_mode_set`, `Reordered_Y_Mode`).
    //
    // `y_mode_set == 0` reads `y_mode_index`; non-zero `y_mode_set` reads the
    // 4-bit `y_second_mode` literal instead. The supported directional branches
    // only cover `mode_ctx == 0`, where §5.20.5.5 `get_intra_y_mode_set` does not
    // preselect any directional neighbour modes before the `Default_Mode_List_Y`
    // scan. The syntax element for an unsupported neighbour-reorder branch is
    // consumed before returning the fail-closed diagnostic.
    let (y_mode, angle_delta_y, intra_joint_mode) = if y_mode_set == 0 {
        // y_mode_index (§ 8.3.2 `TileYModeIndexCdf[ctx]`, ctx from `get_joint_mode`).
        let y_mode_index = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::YModeIndex { ctx: mode_ctx },
            Y_MODE_INDEX_REASON,
        )?;

        if y_mode_index == MODE_INDEX_COUNT - 1 {
            let y_mode_offset = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::YModeOffset { ctx: mode_ctx },
                Y_MODE_OFFSET_REASON,
            )?;
            let mode_idx = usize::from(MODE_INDEX_COUNT - 1) + usize::from(y_mode_offset);
            let escape = reconstruct_y_mode_result(
                y_mode_set,
                mode_idx,
                mode_ctx,
                neighbour_joint_modes,
                block_n4w,
                block_n4h,
                reconstruct_y_mode_offset_escape_top_left(y_mode_offset),
            )?;
            (escape.y_mode, escape.angle_delta_y, escape.intra_joint_mode)
        } else if usize::from(y_mode_index) >= NON_DIRECTIONAL_MODES_COUNT {
            let mode_idx = usize::from(y_mode_index);
            let result = reconstruct_y_mode_result(
                y_mode_set,
                mode_idx,
                mode_ctx,
                neighbour_joint_modes,
                block_n4w,
                block_n4h,
                reconstruct_y_mode_first_set_directional_top_left(y_mode_index),
            )?;
            (result.y_mode, result.angle_delta_y, result.intra_joint_mode)
        } else {
            let mode_idx = usize::from(y_mode_index);
            let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
                GeneralIntraBlockModeError::UnsupportedYMode {
                    y_mode_set,
                    mode_idx,
                },
            )?;
            (y_mode, 0, y_mode_index)
        }
    } else {
        let y_second_mode = symbols.read_literal(Y_SECOND_MODE_BITS).map_err(|source| {
            GeneralIntraBlockModeError::Literal {
                reason: Y_SECOND_MODE_REASON,
                source,
            }
        })? as u8;
        let mode_idx = FIRST_MODE_COUNT
            .saturating_add(
                usize::from(y_mode_set.saturating_sub(1)).saturating_mul(SECOND_MODE_COUNT),
            )
            .saturating_add(usize::from(y_second_mode));
        let result = reconstruct_y_mode_result(
            y_mode_set,
            mode_idx,
            mode_ctx,
            neighbour_joint_modes,
            block_n4w,
            block_n4h,
            reconstruct_y_mode_second_set_top_left(y_mode_set, y_second_mode),
        )?;
        (result.y_mode, result.angle_delta_y, result.intra_joint_mode)
    };

    let fsc_mode = if allow_fsc_intra(chroma_tools, block_n4w, block_n4h) {
        let bsize_group = fsc_bsize_group(block_size_index)
            .ok_or(GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { block_size_index })?;
        read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::FscMode {
                ctx: fsc_modes.fsc_mode_ctx(block_r, block_c, block_n4w, block_n4h),
                bsize_group,
            },
            FSC_MODE_REASON,
        )?
    } else {
        0
    };

    let mut mrl_index = 0;
    let mut mrl_sec_index = None;
    let mut uses_mrls_value = 0;
    if chroma_tools.enable_mrls && y_mode.is_directional() {
        // AV2 § 8.3.2 derives the MRL contexts from neighbouring `UsesMrls`.
        mrl_index = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::MrlIndex {
                ctx: uses_mrls.mrl_index_ctx(block_r, block_c, block_n4w, block_n4h),
            },
            MRL_INDEX_REASON,
        )?;
        if mrl_index > 0 {
            let secondary = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::MrlSecIndex {
                    ctx: uses_mrls.mrl_sec_index_ctx(block_r, block_c, block_n4w, block_n4h),
                },
                MRL_SEC_INDEX_REASON,
            )?;
            mrl_sec_index = Some(secondary);
            uses_mrls_value = if secondary == 0 { 1 } else { 2 };
        }
    }

    Ok(GeneralIntraLumaBlockMode {
        y_mode,
        angle_delta_y,
        intra_joint_mode,
        mrl_index,
        mrl_sec_index,
        fsc_mode,
        uses_mrls: uses_mrls_value,
    })
}

/// Decodes the AV2 § 5.20.5.3/§5.20.5.6 mode-info symbols for one shared
/// luma+chroma intra block.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_block_modes(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    let luma = decode_general_intra_luma_block_mode(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
    )?;
    let uv_mode = decode_general_intra_chroma_block_mode(
        work_unit,
        symbols,
        chroma_tools,
        GeneralIntraChromaModeContext::shared_or_non_sdp(),
        luma.y_mode,
        block_size_index,
        block_n4w,
        block_n4h,
    )?;

    Ok(GeneralIntraBlockModes {
        y_mode: luma.y_mode,
        angle_delta_y: luma.angle_delta_y,
        uv_mode: uv_mode.uv_mode(),
        coeff_uv_mode: uv_mode.coeff_uv_mode,
        is_cfl: uv_mode.is_cfl(),
        intra_joint_mode: luma.intra_joint_mode,
        mrl_index: luma.mrl_index,
        mrl_sec_index: luma.mrl_sec_index,
        fsc_mode: luma.fsc_mode,
        uses_mrls: luma.uses_mrls,
    })
}

/// Decodes the AV2 §5.20.5.6 chroma mode-info symbols for one chroma-capable
/// intra block, using the already-decoded luma `YMode`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_chroma_block_mode(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    mode_context: GeneralIntraChromaModeContext,
    y_mode: IntraYMode,
    block_size_index: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraChromaBlockMode, GeneralIntraBlockModeError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // read_intra_uv_mode(): when CfL is sequence-enabled and the supported
    // non-lossless intra block is no larger than the current LR subset, or when
    // MHCCP is allowed for the block, §5.20.5.6 reads `is_cfl` before `uv_mode`.
    // The current runtime stops if that symbol is true; because every previously
    // admitted block has `is_cfl == 0`, the `UVCfls` neighbour context for this
    // boundary stays 0.
    let cfl_allowed = mode_context.cfl_allowed_in_sdp
        && cfl_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    let mhccp_allowed = mode_context.cfl_allowed_in_sdp
        && mhccp_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    if cfl_allowed || mhccp_allowed {
        let is_cfl = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IsCfl { ctx: 0 },
            IS_CFL_REASON,
        )?;
        if is_cfl != 0 {
            if mhccp_allowed && !cfl_allowed {
                return Err(GeneralIntraBlockModeError::UnsupportedMhccpMode);
            }
            read_cfl_alphas(
                work_unit,
                symbols,
                chroma_tools,
                block_size_index,
                block_n4w,
                block_n4h,
            )?;
            return Ok(GeneralIntraChromaBlockMode::cfl());
        }
    }

    // uv_mode (§ 8.3.2 `TileUVModeCflNotAllowedCdf[ctx]`,
    // `ctx = is_directional_mode(YMode)`).
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

    Ok(GeneralIntraChromaBlockMode::no_cfl(uv_mode))
}

fn read_cfl_alphas(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    block_size_index: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<(), GeneralIntraBlockModeError> {
    let mhccp_allowed = mhccp_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    let cfl_mhccp = if !chroma_tools.enable_cfl_intra {
        1
    } else if mhccp_allowed {
        read_symbol(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            TileCdfSelector::CflMhccp,
            CFL_MHCCP_REASON,
        )?
    } else {
        0
    };

    let cfl_index = if cfl_mhccp != 0 {
        CFL_MULTI
    } else {
        read_symbol(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            TileCdfSelector::CflIndex,
            CFL_INDEX_REASON,
        )?
    };

    if cfl_index == CFL_MULTI {
        let size_group = cfl_mh_dir_size_group(block_size_index)?;
        let _ = read_symbol(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            TileCdfSelector::CflMhDir { size_group },
            CFL_MH_DIR_REASON,
        )?;
    }

    if cfl_index != CFL_EXPLICIT {
        return Ok(());
    }

    let cfl_alpha_signs = read_symbol(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        TileCdfSelector::CflSign,
        CFL_SIGN_REASON,
    )?;
    let sign_u = (cfl_alpha_signs + 1) / 3;
    let sign_v = (cfl_alpha_signs + 1) % 3;
    if sign_u != CFL_SIGN_ZERO {
        let ctx = cfl_alpha_u_ctx(sign_u, sign_v);
        let _ = read_symbol(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            TileCdfSelector::CflAlpha { ctx },
            CFL_ALPHA_U_REASON,
        )?;
    }
    if sign_v != CFL_SIGN_ZERO {
        let ctx = cfl_alpha_v_ctx(sign_u, sign_v);
        let _ = read_symbol(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            TileCdfSelector::CflAlpha { ctx },
            CFL_ALPHA_V_REASON,
        )?;
    }
    Ok(())
}

fn reconstruct_y_mode_result(
    y_mode_set: u8,
    mode_idx: usize,
    mode_ctx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
    top_left_result: Option<YModeEscapeResult>,
) -> Result<YModeEscapeResult, GeneralIntraBlockModeError> {
    if mode_ctx == 0 {
        return top_left_result.ok_or(GeneralIntraBlockModeError::UnsupportedYMode {
            y_mode_set,
            mode_idx,
        });
    }

    reconstruct_y_mode_with_neighbours(mode_idx, neighbour_joint_modes, block_n4w, block_n4h).ok_or(
        GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder {
            ctx: mode_ctx,
            mode_idx,
        },
    )
}

fn cfl_allowed_for_non_lossless_420(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    chroma_tools.enable_cfl_intra && block_n4w <= 16 && block_n4h <= 16
}

fn mhccp_allowed_for_non_lossless_420(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    chroma_tools.enable_mhccp
        && (block_n4w > 2 || block_n4h > 2)
        && block_n4w <= 16
        && block_n4h <= 16
}

fn allow_fsc_intra(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    chroma_tools.enable_idtx_intra
        && block_n4w.saturating_mul(4) <= FSC_MAX_SAMPLES
        && block_n4h.saturating_mul(4) <= FSC_MAX_SAMPLES
}

fn fsc_bsize_group(block_size_index: usize) -> Option<usize> {
    FSC_BSIZE_GROUPS.get(block_size_index).copied()
}

fn cfl_mh_dir_size_group(block_size_index: usize) -> Result<usize, GeneralIntraBlockModeError> {
    SIZE_GROUP
        .get(block_size_index)
        .and_then(|value| usize::try_from(*value).ok())
        .ok_or(GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex { block_size_index })
}

const fn cfl_alpha_u_ctx(sign_u: u8, sign_v: u8) -> usize {
    ((sign_u - 1) * 3 + sign_v) as usize
}

const fn cfl_alpha_v_ctx(sign_u: u8, sign_v: u8) -> usize {
    ((sign_v - 1) * 3 + sign_u) as usize
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
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
    use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

    use super::super::cdf::{
        FrameCdfSubset, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy,
    };
    use super::super::partition_allowed::PartitionFeatureFlags;
    use super::super::partition_traversal::{
        TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
        TilePartitionLoopRestorationState, TilePartitionTraversalInput,
        plan_tile_partition_traversal_cursor,
    };
    use super::super::{
        SymbolInitBoundary, TileBruPath, TileCoeffFrameFacts, TileCoeffFrameFactsInput,
        TilePayloadSource,
    };
    use super::*;
    use crate::{DecodeLayerSelection, DecodeLimits, DecodeObuSourceKind};

    const BLOCK_16X16: usize = 6;
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
            coeff_frame_facts: TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
                enable_fsc: false,
                enable_idtx_intra: false,
                enable_intra_ist: false,
                enable_inter_ist: false,
                enable_chroma_dctonly: false,
                enable_cctx: false,
                reduced_tx_set: 0,
                lossless_array: [false; MAX_SEGMENTS],
                allow_tcq: false,
                allow_parity_hiding: false,
                base_q_idx: 0,
            }),
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

    fn symbol_decoder<'payload>(payload: &'payload [u8]) -> SymbolDecoder<'payload> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap()
    }

    fn encode_symbol_sequence(sequence: &[(TileCdfSelector, u8)]) -> Vec<u8> {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::with_config(
            SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        );
        for &(selector, value) in sequence {
            tile.with_row_mut(selector, |row| {
                encoder.write_symbol(row, Symbol::new(value))
            })
            .unwrap()
            .unwrap();
        }
        encoder.finish().unwrap().into_bytes()
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

    fn empty_uses_mrls() -> TileUsesMrlsState {
        TileUsesMrlsState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
    }

    fn empty_fsc_modes() -> TileFscModeState {
        TileFscModeState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
    }

    #[test]
    fn decodes_dc_luma_mode_and_a_chroma_mode_in_spec_order() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_frontier(&mut work_unit);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        // Top-left block (0, 0): out-of-frame neighbours -> ctx 0.
        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled(),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_64X64,
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
        let uses_mrls = empty_uses_mrls();
        joint_modes.record_block(0, 0, SB_N4, SB_N4, SMOOTH_V_JOINT_MODE);

        // The right superblock at (0, 16) reads the non-directional left neighbour.
        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled(),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_64X64,
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
        let uses_mrls = empty_uses_mrls();
        joint_modes.record_block(0, 0, SB_N4, SB_N4, D135_JOINT_MODE);

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled(),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_64X64,
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

    #[test]
    fn directional_luma_mrl_zero_is_consumed_when_mrls_are_enabled() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
            (TileCdfSelector::MrlIndex { ctx: 0 }, 0),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let luma = decode_general_intra_luma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_64X64,
            0,
            0,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert!(luma.y_mode.is_directional());
        assert_eq!(luma.mrl_index, 0);
        assert_eq!(luma.mrl_sec_index, None);
        assert_eq!(luma.uses_mrls, 0);
        assert_eq!(symbols.symbol_count(), 3);
        assert_eq!(symbols.finish().unwrap().symbol_count, 3);
    }

    #[test]
    fn active_mrl_metadata_is_retained_after_mrl_sec_index_is_consumed() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
            (TileCdfSelector::MrlIndex { ctx: 0 }, 1),
            (TileCdfSelector::MrlSecIndex { ctx: 0 }, 0),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let luma = decode_general_intra_luma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_64X64,
            0,
            0,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert_eq!(luma.mrl_index, 1);
        assert_eq!(luma.mrl_sec_index, Some(0));
        assert_eq!(luma.uses_mrls, 1);
        assert_eq!(symbols.symbol_count(), 4);
    }

    #[test]
    fn active_fsc_mode_metadata_is_retained() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
            (
                TileCdfSelector::FscMode {
                    ctx: 0,
                    bsize_group: fsc_bsize_group(BLOCK_16X16).unwrap(),
                },
                1,
            ),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let luma = decode_general_intra_luma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_enable_idtx_intra(true),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            BLOCK_16X16,
            0,
            0,
            4,
            4,
        )
        .unwrap();

        assert_eq!(luma.y_mode, IntraYMode::DC_PRED);
        assert_eq!(luma.fsc_mode, 1);
        assert_eq!(symbols.symbol_count(), 3);
    }

    #[test]
    fn mrl_symbols_use_retained_neighbour_contexts() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
            (TileCdfSelector::MrlIndex { ctx: 2 }, 1),
            (TileCdfSelector::MrlSecIndex { ctx: 1 }, 1),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = TileIntraJointModeState::new(2 * SB_N4, 2 * SB_N4).unwrap();
        let mut uses_mrls = TileUsesMrlsState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap();
        uses_mrls.record_block(7, 11, 1, 1, 2);
        uses_mrls.record_block(11, 7, 1, 1, 1);

        let luma = decode_general_intra_luma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
            &joint_modes,
            &uses_mrls,
            &TileFscModeState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap(),
            BLOCK_16X16,
            8,
            8,
            4,
            4,
        )
        .unwrap();

        assert_eq!(luma.mrl_index, 1);
        assert_eq!(luma.mrl_sec_index, Some(1));
        assert_eq!(luma.uses_mrls, 2);
        assert_eq!(symbols.symbol_count(), 4);
    }

    #[test]
    fn cfl_allowed_follows_current_non_lossless_420_bounds() {
        let cfl = GeneralIntraChromaToolConfig::new(true, false);
        assert!(cfl_allowed_for_non_lossless_420(cfl, 16, 16));
        assert!(!cfl_allowed_for_non_lossless_420(cfl, 17, 16));
        assert!(!cfl_allowed_for_non_lossless_420(
            GeneralIntraChromaToolConfig::disabled(),
            16,
            16
        ));
    }

    #[test]
    fn mhccp_allowed_follows_current_non_lossless_420_bounds() {
        let mhccp = GeneralIntraChromaToolConfig::new(false, true);
        assert!(mhccp_allowed_for_non_lossless_420(mhccp, 4, 4));
        assert!(mhccp_allowed_for_non_lossless_420(mhccp, 16, 16));
        // 4x4 chroma residual in 4:2:0 corresponds to 8x8 luma (`n4 == 2`), so
        // `(w > 4 || h > 4)` is false.
        assert!(!mhccp_allowed_for_non_lossless_420(mhccp, 2, 2));
        // More than 32x32 chroma residual in 4:2:0 corresponds to luma `n4 > 16`.
        assert!(!mhccp_allowed_for_non_lossless_420(mhccp, 17, 16));
        assert!(!mhccp_allowed_for_non_lossless_420(
            GeneralIntraChromaToolConfig::disabled(),
            4,
            4
        ));
    }

    #[test]
    fn active_cfl_chroma_mode_returns_typed_uv_cfl_pred() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::IsCfl { ctx: 0 }, 1),
            (TileCdfSelector::CflIndex, 1),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);

        let mode = decode_general_intra_chroma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::new(true, false),
            GeneralIntraChromaModeContext::shared_or_non_sdp(),
            IntraYMode::DC_PRED,
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert!(mode.is_cfl());
        assert_eq!(mode.uv_mode(), UV_CFL_PRED_MODE);
        assert_eq!(mode.coeff_uv_mode(), usize::from(UV_CFL_PRED_MODE));
        assert_eq!(symbols.symbol_count(), 2);
        assert_eq!(symbols.finish().unwrap().symbol_count, 2);
    }

    #[test]
    fn sdp_chroma_part_cfl_disallowed_reads_uv_mode_without_is_cfl() {
        let payload =
            encode_symbol_sequence(&[(TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 0)]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);

        let mode = decode_general_intra_chroma_block_mode(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::new(true, true),
            GeneralIntraChromaModeContext::sdp_chroma_part(false),
            IntraYMode::DC_PRED,
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert!(!mode.is_cfl());
        assert_eq!(mode.uv_mode(), 0);
        assert_eq!(symbols.symbol_count(), 1);
        assert_eq!(symbols.finish().unwrap().symbol_count, 1);
    }

    #[test]
    fn read_cfl_alphas_consumes_explicit_sign_and_alpha_contexts() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::CflIndex, CFL_EXPLICIT),
            (TileCdfSelector::CflSign, 7),
            (TileCdfSelector::CflAlpha { ctx: 5 }, 3),
            (TileCdfSelector::CflAlpha { ctx: 5 }, 4),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);

        read_cfl_alphas(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::new(true, false),
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert_eq!(symbols.symbol_count(), 4);
        assert_eq!(symbols.finish().unwrap().symbol_count, 4);
    }

    #[test]
    fn read_cfl_alphas_empty_payload_fails_exit_symbol_validation() {
        let payload: [u8; 0] = [];
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);

        read_cfl_alphas(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::new(true, false),
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert!(symbols.finish().is_err());
    }

    #[test]
    fn cfl_alpha_contexts_match_spec_tables() {
        let u_contexts: Vec<_> = (0..=7)
            .filter_map(|alpha_signs| {
                let sign_u = (alpha_signs + 1) / 3;
                let sign_v = (alpha_signs + 1) % 3;
                (sign_u != CFL_SIGN_ZERO).then(|| (alpha_signs, cfl_alpha_u_ctx(sign_u, sign_v)))
            })
            .collect();
        let v_contexts: Vec<_> = (0..=7)
            .filter_map(|alpha_signs| {
                let sign_u = (alpha_signs + 1) / 3;
                let sign_v = (alpha_signs + 1) % 3;
                (sign_v != CFL_SIGN_ZERO).then(|| (alpha_signs, cfl_alpha_v_ctx(sign_u, sign_v)))
            })
            .collect();

        assert_eq!(
            u_contexts,
            vec![(2, 0), (3, 1), (4, 2), (5, 3), (6, 4), (7, 5)]
        );
        assert_eq!(
            v_contexts,
            vec![(0, 0), (1, 3), (3, 1), (4, 4), (6, 2), (7, 5)]
        );
    }

    #[test]
    fn cfl_mh_dir_size_group_uses_generated_size_group() {
        assert_eq!(cfl_mh_dir_size_group(BLOCK_64X64).unwrap(), 3);

        let invalid = splot_core::tables::conversion::SIZE_GROUP.len();
        let err = cfl_mh_dir_size_group(invalid).unwrap_err();
        assert!(matches!(
            err,
            GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex {
                block_size_index
            } if block_size_index == invalid
        ));
    }
}
