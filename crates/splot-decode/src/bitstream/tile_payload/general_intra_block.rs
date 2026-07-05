// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra block mode-info decode.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::SIZE_GROUP;

use super::DecodeTileWorkUnit;
use super::cdf::block_context::{
    IntraYMode, MODE_INDEX_COUNT, NON_DIRECTIONAL_MODES_COUNT, SupportedChromaMode,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode, YModeEscapeResult,
    reconstruct_minimal_y_mode, reconstruct_y_mode_first_set_directional_top_left,
    reconstruct_y_mode_offset_escape_top_left, reconstruct_y_mode_second_set_top_left,
    reconstruct_y_mode_with_neighbours, resolved_chroma_uv_mode, supported_chroma_mode,
    uv_mode_ctx,
};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};
use super::intra_joint_modes::{
    LumaPalette, PALETTE_MAX_SIZE, TileFscModeState, TileIntraJointModeState, TileLumaPaletteState,
    TileUsesMrlsState,
};

const CHROMA_MODE_COUNT: u8 = 8;
const UV_INTRA_MODES_CFL_NOT_ALLOWED: u8 = 13;
const UV_CFL_PRED_MODE: u8 = 13;
const UV_MODE_IDX_BITS: u32 = 3;
const Y_SECOND_MODE_BITS: u32 = 4;
const FIRST_MODE_COUNT: usize = 13;
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
const PALETTE_Y_MODE_REASON: &str = "intra_palette_y_mode";
const PALETTE_Y_SIZE_REASON: &str = "intra_palette_y_size";
const PALETTE_CACHE_REASON: &str = "intra_palette_color_cache";
const PALETTE_COLOR_REASON: &str = "intra_palette_color";
const PALETTE_EXTRA_BITS_REASON: &str = "intra_palette_extra_bits";
const PALETTE_DELTA_REASON: &str = "intra_palette_delta";
const FSC_MAX_SAMPLES: usize = 32;
const PALETTE_MIN_BLOCK_SIZE_INDEX: usize = 3;
const PALETTE_MAX_SAMPLES: usize = 64;
const INTER_FSC_MODE_CONTEXT: usize = 3;
const FSC_BSIZE_GROUPS: [usize; 29] = [
    0, 1, 1, 2, 3, 3, 4, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 3, 3, 4, 4, 6, 6, 4, 4, 6, 6,
];
const CFL_EXPLICIT: u8 = 0;
const CFL_MULTI: u8 = 2;
const CFL_SIGN_ZERO: u8 = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaToolConfig {
    enable_cfl_intra: bool,
    enable_mhccp: bool,
    enable_idtx_intra: bool,
    enable_mrls: bool,
    allow_screen_content_tools: bool,
}

impl GeneralIntraChromaToolConfig {
    #[must_use]
    pub(crate) const fn new(enable_cfl_intra: bool, enable_mhccp: bool) -> Self {
        Self {
            enable_cfl_intra,
            enable_mhccp,
            enable_idtx_intra: false,
            enable_mrls: false,
            allow_screen_content_tools: false,
        }
    }

    #[must_use]
    pub(crate) const fn with_enable_idtx_intra(mut self, enable_idtx_intra: bool) -> Self {
        self.enable_idtx_intra = enable_idtx_intra;
        self
    }

    #[must_use]
    pub(crate) const fn with_enable_mrls(mut self, enable_mrls: bool) -> Self {
        self.enable_mrls = enable_mrls;
        self
    }

    #[must_use]
    pub(crate) const fn with_allow_screen_content_tools(
        mut self,
        allow_screen_content_tools: bool,
    ) -> Self {
        self.allow_screen_content_tools = allow_screen_content_tools;
        self
    }

    #[must_use]
    pub(crate) const fn disabled() -> Self {
        Self::new(false, false)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaModeContext {
    cfl_allowed_in_sdp: bool,
    is_cfl_ctx: usize,
}

impl GeneralIntraChromaModeContext {
    #[must_use]
    pub(crate) const fn shared_or_non_sdp(is_cfl_ctx: usize) -> Self {
        Self {
            cfl_allowed_in_sdp: true,
            is_cfl_ctx,
        }
    }

    #[must_use]
    pub(crate) const fn sdp_chroma_part(cfl_allowed_in_sdp: bool, is_cfl_ctx: usize) -> Self {
        Self {
            cfl_allowed_in_sdp,
            is_cfl_ctx,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraBlockModes {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
    pub(crate) uv_mode: u8,
    coeff_uv_mode: u8,
    is_cfl: bool,
    cfl_params: Option<CflParams>,
    pub(crate) intra_joint_mode: u8,
    pub(crate) mrl_index: u8,
    pub(crate) mrl_sec_index: Option<u8>,
    pub(crate) fsc_mode: u8,
    pub(crate) uses_mrls: u8,
    pub(crate) palette_y: Option<LumaPalette>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaBlockMode {
    uv_mode: u8,
    coeff_uv_mode: u8,
    is_cfl: bool,
    cfl_params: Option<CflParams>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CflIndex {
    Explicit,
    DerivedAlpha,
    Multi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CflParams {
    pub(crate) index: CflIndex,
    pub(crate) alpha_u: i8,
    pub(crate) alpha_v: i8,
    pub(crate) mh_dir: Option<u8>,
}

impl GeneralIntraChromaBlockMode {
    const fn no_cfl(uv_mode: u8, coeff_uv_mode: u8) -> Self {
        Self {
            uv_mode,
            coeff_uv_mode,
            is_cfl: false,
            cfl_params: None,
        }
    }

    const fn cfl(cfl_params: CflParams) -> Self {
        Self {
            uv_mode: UV_CFL_PRED_MODE,
            coeff_uv_mode: UV_CFL_PRED_MODE,
            is_cfl: true,
            cfl_params: Some(cfl_params),
        }
    }

    pub(crate) const fn uv_mode(self) -> u8 {
        self.uv_mode
    }

    pub(crate) const fn coeff_uv_mode(self) -> usize {
        self.coeff_uv_mode as usize
    }

    pub(crate) const fn is_cfl(self) -> bool {
        self.is_cfl
    }

    pub(crate) const fn cfl_params(self) -> Option<CflParams> {
        self.cfl_params
    }

    pub(crate) fn supported_chroma_mode(self, y_mode: IntraYMode) -> Option<SupportedChromaMode> {
        if self.is_cfl {
            return None;
        }
        supported_chroma_mode(y_mode, self.uv_mode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraLumaBlockMode {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
    pub(crate) intra_joint_mode: u8,
    pub(crate) mrl_index: u8,
    pub(crate) mrl_sec_index: Option<u8>,
    pub(crate) fsc_mode: u8,
    pub(crate) uses_mrls: u8,
}

impl GeneralIntraLumaBlockMode {
    #[allow(dead_code)]
    pub(crate) fn supported_directional_luma(self) -> Option<SupportedDirectionalLumaMode> {
        supported_directional_luma(self.y_mode, self.angle_delta_y)
    }
}

impl GeneralIntraBlockModes {
    pub(crate) const fn luma_only(luma: GeneralIntraLumaBlockMode) -> Self {
        Self {
            y_mode: luma.y_mode,
            angle_delta_y: luma.angle_delta_y,
            uv_mode: 0,
            coeff_uv_mode: 0,
            is_cfl: false,
            cfl_params: None,
            intra_joint_mode: luma.intra_joint_mode,
            mrl_index: luma.mrl_index,
            mrl_sec_index: luma.mrl_sec_index,
            fsc_mode: luma.fsc_mode,
            uses_mrls: luma.uses_mrls,
            palette_y: None,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn from_luma_chroma(
        luma: GeneralIntraLumaBlockMode,
        chroma: GeneralIntraChromaBlockMode,
    ) -> Self {
        Self::from_luma_chroma_palette(luma, chroma, None)
    }

    pub(crate) const fn from_luma_chroma_palette(
        luma: GeneralIntraLumaBlockMode,
        chroma: GeneralIntraChromaBlockMode,
        palette_y: Option<LumaPalette>,
    ) -> Self {
        Self {
            y_mode: luma.y_mode,
            angle_delta_y: luma.angle_delta_y,
            uv_mode: chroma.uv_mode,
            coeff_uv_mode: chroma.coeff_uv_mode,
            is_cfl: chroma.is_cfl,
            cfl_params: chroma.cfl_params,
            intra_joint_mode: luma.intra_joint_mode,
            mrl_index: luma.mrl_index,
            mrl_sec_index: luma.mrl_sec_index,
            fsc_mode: luma.fsc_mode,
            uses_mrls: luma.uses_mrls,
            palette_y,
        }
    }

    pub(crate) const fn with_palette_y(mut self, palette_y: Option<LumaPalette>) -> Self {
        self.palette_y = palette_y;
        self
    }

    pub(crate) fn luma_is_dc(&self) -> bool {
        self.y_mode == IntraYMode::DC_PRED
    }

    pub(crate) const fn is_cfl(&self) -> bool {
        self.is_cfl
    }

    pub(crate) const fn cfl_params(&self) -> Option<CflParams> {
        self.cfl_params
    }

    pub(crate) fn supported_nondc_luma(&self) -> Option<SupportedNonDcLumaMode> {
        self.y_mode.supported_nondc()
    }

    #[allow(dead_code)]
    pub(crate) fn supported_directional_luma(&self) -> Option<SupportedDirectionalLumaMode> {
        supported_directional_luma(self.y_mode, self.angle_delta_y)
    }

    pub(crate) fn supported_chroma_mode(&self) -> Option<SupportedChromaMode> {
        if self.is_cfl {
            return None;
        }
        supported_chroma_mode(self.y_mode, self.uv_mode)
    }

    pub(crate) const fn coeff_uv_mode(&self) -> usize {
        self.coeff_uv_mode as usize
    }

    pub(crate) const fn uses_active_mrl(&self) -> bool {
        self.uses_mrls != 0
    }

    pub(crate) const fn uses_active_fsc(&self) -> bool {
        self.fsc_mode != 0
    }

    pub(crate) const fn palette_y(&self) -> Option<LumaPalette> {
        self.palette_y
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraBlockModeError {
    #[error("general intra mode-info symbol read failed for {reason}: {source}")]
    SymbolRead {
        reason: &'static str,
        source: BlockSymbolTraceReadError,
    },
    #[error("general intra mode-info literal read failed for {reason}: {source}")]
    Literal {
        reason: &'static str,
        source: CoreError,
    },
    #[error(
        "general intra mode-info cannot reconstruct YMode for y_mode_set {y_mode_set}, modeIdx {mode_idx}"
    )]
    UnsupportedYMode { y_mode_set: u8, mode_idx: usize },
    #[error("general intra mode-info decoded out-of-range uv_mode {uv_mode}")]
    InvalidUvMode { uv_mode: u8 },
    #[error("general intra mode-info block-size index {block_size_index} has no FSC group")]
    InvalidFscBlockSizeIndex { block_size_index: usize },
    #[error(
        "general intra mode-info block-size index {block_size_index} has no CfL MH direction size group"
    )]
    InvalidCflMhDirBlockSizeIndex { block_size_index: usize },
    #[error("general intra mode-info selected unsupported MHCCP chroma prediction")]
    UnsupportedMhccpMode,
    #[error(
        "general intra mode-info modeIdx {mode_idx} with directional-neighbour ctx {ctx} requires §5.20.5.5 reorder support"
    )]
    UnsupportedDirectionalNeighbourReorder { ctx: usize, mode_idx: usize },
    #[error("general intra mode-info decoded invalid luma palette size {palette_size}")]
    InvalidPaletteYSize { palette_size: usize },
}

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
    decode_general_intra_luma_block_mode_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        true,
        block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_luma_block_mode_with_fsc_context(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    use_neighbor_fsc_context: bool,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraLumaBlockMode, GeneralIntraBlockModeError> {
    let mode_ctx = joint_modes.y_mode_index_ctx(block_r, block_c, block_n4w, block_n4h);
    let neighbour_joint_modes =
        joint_modes.neighbour_joint_modes(block_r, block_c, block_n4w, block_n4h);
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS") {
        eprintln!(
            "intra luma block start=({block_r},{block_c}) n4=({block_n4w}x{block_n4h}) bsize={block_size_index} mode_ctx={mode_ctx} neighbours={neighbour_joint_modes:?} checkpoint={:?}",
            symbols.checkpoint(),
        );
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    let y_mode_result = decode_luma_y_mode(
        cdfs,
        symbols,
        mode_ctx,
        neighbour_joint_modes,
        block_n4w,
        block_n4h,
    )?;

    let fsc_mode = if allow_fsc_intra(chroma_tools, block_n4w, block_n4h) {
        let bsize_group = fsc_bsize_group(block_size_index)
            .ok_or(GeneralIntraBlockModeError::InvalidFscBlockSizeIndex { block_size_index })?;
        let ctx = if use_neighbor_fsc_context {
            fsc_modes.fsc_mode_ctx(block_r, block_c, block_n4w, block_n4h)
        } else {
            INTER_FSC_MODE_CONTEXT
        };
        read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::FscMode { ctx, bsize_group },
            FSC_MODE_REASON,
        )?
    } else {
        0
    };

    let mut mrl_index = 0;
    let mut mrl_sec_index = None;
    let mut uses_mrls_value = 0;
    if chroma_tools.enable_mrls && y_mode_result.y_mode.is_directional() {
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
        y_mode: y_mode_result.y_mode,
        angle_delta_y: y_mode_result.angle_delta_y,
        intra_joint_mode: y_mode_result.intra_joint_mode,
        mrl_index,
        mrl_sec_index,
        fsc_mode,
        uses_mrls: uses_mrls_value,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_block_modes(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    palette_state: &TileLumaPaletteState,
    is_cfl_ctx: usize,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
    bit_depth_bits: u32,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    decode_general_intra_block_modes_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        true,
        palette_state,
        is_cfl_ctx,
        block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
        block_size_index,
        block_n4w,
        block_n4h,
        bit_depth_bits,
    )
}

#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_block_modes_with_chroma_size(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    palette_state: &TileLumaPaletteState,
    is_cfl_ctx: usize,
    luma_block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
    chroma_block_size_index: usize,
    chroma_n4w: usize,
    chroma_n4h: usize,
    bit_depth_bits: u32,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    decode_general_intra_block_modes_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        true,
        palette_state,
        is_cfl_ctx,
        luma_block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
        chroma_block_size_index,
        chroma_n4w,
        chroma_n4h,
        bit_depth_bits,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_block_modes_with_fsc_context(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    use_neighbor_fsc_context: bool,
    palette_state: &TileLumaPaletteState,
    is_cfl_ctx: usize,
    luma_block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
    chroma_block_size_index: usize,
    chroma_n4w: usize,
    chroma_n4h: usize,
    bit_depth_bits: u32,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    let luma = decode_general_intra_luma_block_mode_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        use_neighbor_fsc_context,
        luma_block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
    )?;
    let uv_mode = decode_general_intra_chroma_block_mode(
        work_unit,
        symbols,
        chroma_tools,
        GeneralIntraChromaModeContext::shared_or_non_sdp(is_cfl_ctx),
        luma.y_mode,
        chroma_block_size_index,
        chroma_n4w,
        chroma_n4h,
    )?;
    let palette_y = read_general_intra_palette_y_mode(
        work_unit,
        symbols,
        chroma_tools,
        palette_state,
        luma.y_mode,
        luma_block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
        bit_depth_bits,
    )?;

    Ok(GeneralIntraBlockModes::from_luma_chroma_palette(
        luma, uv_mode, palette_y,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_general_intra_palette_y_mode(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    palette_state: &TileLumaPaletteState,
    y_mode: IntraYMode,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
    bit_depth_bits: u32,
) -> Result<Option<LumaPalette>, GeneralIntraBlockModeError> {
    if !palette_y_mode_allowed(chroma_tools, y_mode, block_size_index, block_n4w, block_n4h) {
        return Ok(None);
    }

    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let has_palette_y = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::PaletteYMode,
        PALETTE_Y_MODE_REASON,
    )?;
    if has_palette_y != 0 {
        let palette_size = usize::from(read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::PaletteYSize,
            PALETTE_Y_SIZE_REASON,
        )?) + 2;
        let colors = read_palette_colors_y(
            symbols,
            palette_state,
            block_r,
            block_c,
            palette_size,
            bit_depth_bits,
        )?;
        return LumaPalette::new(palette_size as u8, colors)
            .ok_or(GeneralIntraBlockModeError::InvalidPaletteYSize { palette_size })
            .map(Some);
    }
    Ok(None)
}

fn read_palette_colors_y(
    symbols: &mut SymbolDecoder<'_>,
    palette_state: &TileLumaPaletteState,
    block_r: usize,
    block_c: usize,
    palette_size: usize,
    bit_depth_bits: u32,
) -> Result<[u16; PALETTE_MAX_SIZE], GeneralIntraBlockModeError> {
    if !(2..=PALETTE_MAX_SIZE).contains(&palette_size) {
        return Err(GeneralIntraBlockModeError::InvalidPaletteYSize { palette_size });
    }
    let (cache, cache_len) = palette_state.palette_cache(block_r, block_c);
    let mut colors = [0u16; PALETTE_MAX_SIZE];
    let mut idx = 0usize;
    for &cached in cache.iter().take(cache_len) {
        if idx >= palette_size {
            break;
        }
        if read_literal_u8(symbols, 1, PALETTE_CACHE_REASON)? != 0 {
            colors[idx] = cached;
            idx += 1;
        }
    }
    if idx < palette_size {
        colors[idx] = read_literal_u16(symbols, bit_depth_bits, PALETTE_COLOR_REASON)?;
        idx += 1;
        if idx < palette_size {
            let min_bits = bit_depth_bits.saturating_sub(3);
            let mut bits =
                min_bits + u32::from(read_literal_u8(symbols, 2, PALETTE_EXTRA_BITS_REASON)?);
            let max_sample = (1u32 << bit_depth_bits) - 1;
            let mut range = max_sample.saturating_sub(u32::from(colors[idx - 1]));
            while idx < palette_size {
                let delta = read_literal_u16(symbols, bits, PALETTE_DELTA_REASON)? + 1;
                let value = u32::from(colors[idx - 1])
                    .saturating_add(u32::from(delta))
                    .min(max_sample);
                colors[idx] = value as u16;
                range = range.saturating_sub(value.saturating_sub(u32::from(colors[idx - 1])));
                bits = bits.min(ceil_log2_u32(range));
                idx += 1;
            }
        }
    }
    colors[..palette_size].sort_unstable();
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS") {
        eprintln!(
            "intra palette y block=({block_r},{block_c}) size={palette_size} colors={:?}",
            &colors[..palette_size]
        );
    }
    Ok(colors)
}

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

    let cfl_allowed = mode_context.cfl_allowed_in_sdp
        && cfl_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    let mhccp_allowed = mode_context.cfl_allowed_in_sdp
        && mhccp_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS") {
        eprintln!(
            "intra chroma block y_mode={y_mode:?} n4=({block_n4w}x{block_n4h}) bsize={block_size_index} is_cfl_ctx={} cfl_allowed={cfl_allowed} mhccp_allowed={mhccp_allowed} checkpoint={:?}",
            mode_context.is_cfl_ctx,
            symbols.checkpoint(),
        );
    }
    if cfl_allowed || mhccp_allowed {
        let is_cfl = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IsCfl {
                ctx: mode_context.is_cfl_ctx,
            },
            IS_CFL_REASON,
        )?;
        if is_cfl != 0 {
            if mhccp_allowed && !cfl_allowed {
                return Err(GeneralIntraBlockModeError::UnsupportedMhccpMode);
            }
            let cfl_params = read_cfl_alphas(
                work_unit,
                symbols,
                chroma_tools,
                block_size_index,
                block_n4w,
                block_n4h,
            )?;
            return Ok(GeneralIntraChromaBlockMode::cfl(cfl_params));
        }
    }

    let uv_mode_base = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::UvModeCflNotAllowed {
            ctx: uv_mode_ctx(y_mode),
        },
        UV_MODE_REASON,
    )?;

    let mut uv_mode_idx = None;
    let uv_mode = if uv_mode_base == CHROMA_MODE_COUNT - 1 {
        let idx = read_literal_u8(symbols, UV_MODE_IDX_BITS, UV_MODE_IDX_REASON)?;
        uv_mode_idx = Some(idx);
        uv_mode_base.saturating_add(idx)
    } else {
        uv_mode_base
    };
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_CHROMA_MODE") {
        eprintln!(
            "general intra chroma mode y_mode={y_mode:?} block_n4=({block_n4w}x{block_n4h}) uv_mode_base={uv_mode_base} uv_mode_idx={uv_mode_idx:?} uv_mode={uv_mode}"
        );
    }

    if uv_mode >= UV_INTRA_MODES_CFL_NOT_ALLOWED {
        return Err(GeneralIntraBlockModeError::InvalidUvMode { uv_mode });
    }

    let coeff_uv_mode = resolved_chroma_uv_mode(y_mode, uv_mode)
        .ok_or(GeneralIntraBlockModeError::InvalidUvMode { uv_mode })?;

    Ok(GeneralIntraChromaBlockMode::no_cfl(uv_mode, coeff_uv_mode))
}

fn read_cfl_alphas(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    block_size_index: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<CflParams, GeneralIntraBlockModeError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let mhccp_allowed = mhccp_allowed_for_non_lossless_420(chroma_tools, block_n4w, block_n4h);
    let cfl_mhccp = if !chroma_tools.enable_cfl_intra {
        1
    } else if mhccp_allowed {
        read_symbol(cdfs, symbols, TileCdfSelector::CflMhccp, CFL_MHCCP_REASON)?
    } else {
        0
    };

    let cfl_index = if cfl_mhccp != 0 {
        CFL_MULTI
    } else {
        read_symbol(cdfs, symbols, TileCdfSelector::CflIndex, CFL_INDEX_REASON)?
    };

    if cfl_index == CFL_MULTI {
        let size_group = cfl_mh_dir_size_group(block_size_index)?;
        let mh_dir = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::CflMhDir { size_group },
            CFL_MH_DIR_REASON,
        )?;
        return Ok(CflParams {
            index: CflIndex::Multi,
            alpha_u: 0,
            alpha_v: 0,
            mh_dir: Some(mh_dir),
        });
    }

    if cfl_index != CFL_EXPLICIT {
        return Ok(CflParams {
            index: CflIndex::DerivedAlpha,
            alpha_u: 0,
            alpha_v: 0,
            mh_dir: None,
        });
    }

    let cfl_alpha_signs = read_symbol(cdfs, symbols, TileCdfSelector::CflSign, CFL_SIGN_REASON)?;
    let sign_u = (cfl_alpha_signs + 1) / 3;
    let sign_v = (cfl_alpha_signs + 1) % 3;
    let alpha_u = if sign_u != CFL_SIGN_ZERO {
        let ctx = cfl_alpha_u_ctx(sign_u, sign_v);
        signed_cfl_alpha(
            sign_u,
            read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::CflAlpha { ctx },
                CFL_ALPHA_U_REASON,
            )?,
        )
    } else {
        0
    };
    let alpha_v = if sign_v != CFL_SIGN_ZERO {
        let ctx = cfl_alpha_v_ctx(sign_u, sign_v);
        signed_cfl_alpha(
            sign_v,
            read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::CflAlpha { ctx },
                CFL_ALPHA_V_REASON,
            )?,
        )
    } else {
        0
    };
    Ok(CflParams {
        index: CflIndex::Explicit,
        alpha_u,
        alpha_v,
        mh_dir: None,
    })
}

fn signed_cfl_alpha(sign: u8, alpha_minus_one: u8) -> i8 {
    let magnitude = alpha_minus_one.saturating_add(1) as i8;
    if sign == 1 { -magnitude } else { magnitude }
}

fn decode_luma_y_mode(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mode_ctx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Result<YModeEscapeResult, GeneralIntraBlockModeError> {
    let y_mode_set = read_symbol(cdfs, symbols, TileCdfSelector::YModeSet, Y_MODE_SET_REASON)?;
    if y_mode_set != 0 {
        let y_second_mode = read_literal_u8(symbols, Y_SECOND_MODE_BITS, Y_SECOND_MODE_REASON)?;
        let mode_idx = FIRST_MODE_COUNT
            .saturating_add(
                usize::from(y_mode_set.saturating_sub(1)).saturating_mul(SECOND_MODE_COUNT),
            )
            .saturating_add(usize::from(y_second_mode));
        return reconstruct_y_mode_result(
            y_mode_set,
            mode_idx,
            mode_ctx,
            neighbour_joint_modes,
            block_n4w,
            block_n4h,
            reconstruct_y_mode_second_set_top_left(y_mode_set, y_second_mode),
        );
    }

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
        return reconstruct_y_mode_result(
            y_mode_set,
            mode_idx,
            mode_ctx,
            neighbour_joint_modes,
            block_n4w,
            block_n4h,
            reconstruct_y_mode_offset_escape_top_left(y_mode_offset),
        );
    }

    let mode_idx = usize::from(y_mode_index);
    if mode_idx >= NON_DIRECTIONAL_MODES_COUNT {
        return reconstruct_y_mode_result(
            y_mode_set,
            mode_idx,
            mode_ctx,
            neighbour_joint_modes,
            block_n4w,
            block_n4h,
            reconstruct_y_mode_first_set_directional_top_left(y_mode_index),
        );
    }

    let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
        GeneralIntraBlockModeError::UnsupportedYMode {
            y_mode_set,
            mode_idx,
        },
    )?;
    Ok(YModeEscapeResult {
        y_mode,
        angle_delta_y: 0,
        intra_joint_mode: y_mode_index,
    })
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

#[allow(dead_code)]
fn supported_directional_luma(
    y_mode: IntraYMode,
    angle_delta_y: i8,
) -> Option<SupportedDirectionalLumaMode> {
    if angle_delta_y != 0 {
        return None;
    }
    y_mode.supported_directional()
}

fn cfl_allowed_for_non_lossless_420(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    // AV2 §5.20.5.6: 4:2:0 chroma blocks are half luma size.
    chroma_tools.enable_cfl_intra && block_n4w <= 32 && block_n4h <= 32
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

fn palette_y_mode_allowed(
    chroma_tools: GeneralIntraChromaToolConfig,
    y_mode: IntraYMode,
    block_size_index: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    chroma_tools.allow_screen_content_tools
        && y_mode == IntraYMode::DC_PRED
        && block_size_index >= PALETTE_MIN_BLOCK_SIZE_INDEX
        && block_n4w.saturating_mul(4) <= PALETTE_MAX_SAMPLES
        && block_n4h.saturating_mul(4) <= PALETTE_MAX_SAMPLES
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

fn read_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
    reason: &'static str,
) -> Result<u8, GeneralIntraBlockModeError> {
    let trace = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS");
    let before = trace.then(|| symbols.checkpoint());
    let row_before = if trace {
        cdfs.row(selector).ok().map(<[i32]>::to_vec)
    } else {
        None
    };
    let value = cdfs
        .read_block_symbol_trace(selector, symbols)
        .map(splot_core::symbol::Symbol::get)
        .map_err(|source| GeneralIntraBlockModeError::SymbolRead { reason, source })?;
    if trace {
        eprintln!(
            "intra symbol reason={reason} selector={selector:?} value={value} before={:?} after={:?} row_before={row_before:?}",
            before,
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

fn read_literal_u8(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> Result<u8, GeneralIntraBlockModeError> {
    let trace = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS");
    let trace_raw = crate::trace_flags::trace_flag!("SPLOT_TRACE_RAW_LITERALS");
    let before = trace.then(|| symbols.checkpoint());
    let raw_before = trace_raw.then(|| symbols.checkpoint());
    let value = symbols
        .read_literal(bits)
        .map(|value| value as u8)
        .map_err(|source| GeneralIntraBlockModeError::Literal { reason, source })?;
    if trace {
        eprintln!(
            "intra literal reason={reason} bits={bits} value={value} before={before:?} after={:?}",
            symbols.checkpoint(),
        );
    }
    if let Some(raw_before) = raw_before {
        eprintln!(
            "raw_literal kind=intra_mode reason={reason} width={bits} value={value} checkpoint_before={raw_before:?} checkpoint_after={:?}",
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

fn read_literal_u16(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> Result<u16, GeneralIntraBlockModeError> {
    let trace = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRA_MODE_SYMBOLS");
    let trace_raw = crate::trace_flags::trace_flag!("SPLOT_TRACE_RAW_LITERALS");
    let before = trace.then(|| symbols.checkpoint());
    let raw_before = trace_raw.then(|| symbols.checkpoint());
    let value = symbols
        .read_literal(bits)
        .map(|value| value as u16)
        .map_err(|source| GeneralIntraBlockModeError::Literal { reason, source })?;
    if trace {
        eprintln!(
            "intra literal reason={reason} bits={bits} value={value} before={before:?} after={:?}",
            symbols.checkpoint(),
        );
    }
    if let Some(raw_before) = raw_before {
        eprintln!(
            "raw_literal kind=intra_mode reason={reason} width={bits} value={value} checkpoint_before={raw_before:?} checkpoint_after={:?}",
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

const fn ceil_log2_u32(value: u32) -> u32 {
    if value < 2 {
        0
    } else {
        u32::BITS - (value - 1).leading_zeros()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
    use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

    use super::super::cdf::FrameCdfSubset;
    use super::super::encode_symbol_sequence;
    use super::super::partition_allowed::PartitionFeatureFlags;
    use super::super::partition_traversal::tests::make_work_unit as make_test_work_unit;
    use super::super::partition_traversal::{
        TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
        TilePartitionLoopRestorationState, TilePartitionTraversalInput,
        plan_tile_partition_traversal_cursor,
    };
    use super::*;
    use crate::DecodeLimits;

    const BLOCK_16X16: usize = 6;
    const BLOCK_64X64: usize = 12;
    const BLOCK_256X256: usize = 18;
    const CLEAR_PARTITION_CONTEXT: usize = 0;
    const PAYLOAD: [u8; 2] = [0x12, 0xFB];

    fn make_work_unit(payload: &[u8]) -> DecodeTileWorkUnit<'_> {
        make_test_work_unit(payload, CdfUpdateMode::Disabled)
    }

    fn symbols_at_block_start<'payload>(
        work_unit: &mut DecodeTileWorkUnit<'payload>,
    ) -> SymbolDecoder<'payload> {
        let rows: Vec<Vec<usize>> = (0..16).map(|_| vec![BLOCK_256X256; 16]).collect();
        let mi0_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let mi1_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
        let edge = [CLEAR_PARTITION_CONTEXT; 16];
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

    fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap()
    }

    const SB_N4: usize = 16;
    const D135_JOINT_MODE: u8 = 36;
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

    fn empty_palette_state() -> TileLumaPaletteState {
        TileLumaPaletteState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
    }

    #[test]
    fn cfl_allowed_420_uses_chroma_plane_64_sample_limit() {
        let tools = GeneralIntraChromaToolConfig::new(true, false);

        assert!(cfl_allowed_for_non_lossless_420(tools, 16, 16));
        assert!(cfl_allowed_for_non_lossless_420(tools, 32, 16));
        assert!(cfl_allowed_for_non_lossless_420(tools, 32, 32));
        assert!(!cfl_allowed_for_non_lossless_420(tools, 33, 32));
        assert!(!cfl_allowed_for_non_lossless_420(tools, 32, 33));
        assert!(!cfl_allowed_for_non_lossless_420(tools, 64, 32));
        assert!(!cfl_allowed_for_non_lossless_420(tools, 32, 64));
        assert!(!cfl_allowed_for_non_lossless_420(
            GeneralIntraChromaToolConfig::new(false, false),
            16,
            16,
        ));
    }

    #[test]
    fn decodes_dc_luma_mode_and_a_chroma_mode_in_spec_order() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_start(&mut work_unit);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled(),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            &empty_palette_state(),
            0,
            BLOCK_64X64,
            0,
            0,
            SB_N4,
            SB_N4,
            8,
        )
        .unwrap();

        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
        assert_eq!(modes.intra_joint_mode, 0);
        assert!(
            modes.uv_mode < UV_INTRA_MODES_CFL_NOT_ALLOWED,
            "uv_mode {} out of range",
            modes.uv_mode
        );
    }

    #[test]
    fn non_directional_left_neighbour_keeps_ctx_zero_and_decodes() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_start(&mut work_unit);
        let mut joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();
        joint_modes.record_block(0, 0, SB_N4, SB_N4, SMOOTH_V_JOINT_MODE);

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled(),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            &empty_palette_state(),
            0,
            BLOCK_64X64,
            0,
            SB_N4,
            SB_N4,
            SB_N4,
            8,
        )
        .unwrap();
        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
    }

    #[test]
    fn directional_neighbour_ctx_reads_with_the_real_context() {
        let mut work_unit = make_work_unit(&PAYLOAD);
        let mut symbols = symbols_at_block_start(&mut work_unit);
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
            &empty_palette_state(),
            0,
            BLOCK_64X64,
            0,
            SB_N4,
            SB_N4,
            SB_N4,
            8,
        )
        .unwrap();

        assert!(symbols.symbol_count() > symbol_count_before);
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
    fn mixed_region_fsc_mode_uses_inter_context() {
        let bsize_group = fsc_bsize_group(BLOCK_16X16).unwrap();
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
            (
                TileCdfSelector::FscMode {
                    ctx: INTER_FSC_MODE_CONTEXT,
                    bsize_group,
                },
                1,
            ),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();
        let mut fsc_modes = TileFscModeState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap();
        fsc_modes.record_block(7, 11, 1, 1, 1);
        fsc_modes.record_block(11, 7, 1, 1, 1);

        let luma = decode_general_intra_luma_block_mode_with_fsc_context(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_enable_idtx_intra(true),
            &joint_modes,
            &uses_mrls,
            &fsc_modes,
            false,
            BLOCK_16X16,
            8,
            8,
            4,
            4,
        )
        .unwrap();

        assert_eq!(luma.fsc_mode, 1);
        assert_eq!(symbols.symbol_count(), 3);
    }

    #[test]
    fn inactive_palette_y_mode_is_consumed_after_chroma_mode() {
        let payload = encode_symbol_sequence(&[
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
            (TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 1),
            (TileCdfSelector::PaletteYMode, 0),
        ]);
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_allow_screen_content_tools(true),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            &empty_palette_state(),
            0,
            BLOCK_16X16,
            0,
            0,
            4,
            4,
            8,
        )
        .unwrap();

        assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
        assert_eq!(modes.uv_mode, 1);
        assert_eq!(symbols.symbol_count(), 4);
        assert_eq!(symbols.finish().unwrap().symbol_count, 4);
    }

    #[test]
    fn active_palette_y_mode_reads_size_and_literal_colors() {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::with_config(
            SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        );
        for (selector, value) in [
            (TileCdfSelector::YModeSet, 0),
            (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
            (TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 1),
            (TileCdfSelector::PaletteYMode, 1),
            (TileCdfSelector::PaletteYSize, 0),
        ] {
            tile.with_row_mut(selector, |row| {
                encoder.write_symbol(row, Symbol::new(value))
            })
            .unwrap()
            .unwrap();
        }
        encoder.write_literal(10, 8).unwrap();
        encoder.write_literal(0, 2).unwrap();
        encoder.write_literal(3, 5).unwrap();
        let payload = encoder.finish().unwrap().into_bytes();
        let mut work_unit = make_work_unit(&payload);
        let mut symbols = symbol_decoder(&payload);
        let joint_modes = empty_joint_modes();
        let uses_mrls = empty_uses_mrls();

        let modes = decode_general_intra_block_modes(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::disabled().with_allow_screen_content_tools(true),
            &joint_modes,
            &uses_mrls,
            &empty_fsc_modes(),
            &empty_palette_state(),
            0,
            BLOCK_16X16,
            0,
            0,
            4,
            4,
            8,
        )
        .unwrap();

        let palette = modes.palette_y().expect("active palette");
        assert_eq!(palette.size(), 2);
        assert_eq!(&palette.colors()[..2], &[10, 14]);
        assert_eq!(symbols.symbol_count(), 20);
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
    fn mhccp_allowed_follows_current_non_lossless_420_bounds() {
        let mhccp = GeneralIntraChromaToolConfig::new(false, true);
        assert!(mhccp_allowed_for_non_lossless_420(mhccp, 4, 4));
        assert!(mhccp_allowed_for_non_lossless_420(mhccp, 16, 16));
        assert!(!mhccp_allowed_for_non_lossless_420(mhccp, 2, 2));
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
            GeneralIntraChromaModeContext::shared_or_non_sdp(0),
            IntraYMode::DC_PRED,
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert!(mode.is_cfl());
        assert_eq!(mode.uv_mode(), UV_CFL_PRED_MODE);
        assert_eq!(mode.coeff_uv_mode(), usize::from(UV_CFL_PRED_MODE));
        assert_eq!(
            mode.cfl_params(),
            Some(CflParams {
                index: CflIndex::DerivedAlpha,
                alpha_u: 0,
                alpha_v: 0,
                mh_dir: None
            })
        );
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
            GeneralIntraChromaModeContext::sdp_chroma_part(false, 0),
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

        let params = read_cfl_alphas(
            &mut work_unit,
            &mut symbols,
            GeneralIntraChromaToolConfig::new(true, false),
            BLOCK_64X64,
            SB_N4,
            SB_N4,
        )
        .unwrap();

        assert_eq!(
            params,
            CflParams {
                index: CflIndex::Explicit,
                alpha_u: 4,
                alpha_v: 5,
                mh_dir: None
            }
        );
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
