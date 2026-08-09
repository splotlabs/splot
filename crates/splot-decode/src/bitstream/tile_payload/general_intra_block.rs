// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra block mode-info decode.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{ADJUSTED_TX_SIZE, MAX_TX_SIZE_RECT, SIZE_GROUP};
use splot_recon::{DpcmDirection, dpcm_direction};

use super::cdf::block_context::{
    IntraYMode, MODE_INDEX_COUNT, NON_DIRECTIONAL_MODES_COUNT, SupportedChromaMode,
    SupportedNonDcLumaMode, YModeEscapeResult, get_intra_uv_mode_set, reconstruct_minimal_y_mode,
    reconstruct_y_mode_first_set_directional_top_left, reconstruct_y_mode_offset_escape_top_left,
    reconstruct_y_mode_second_set_top_left, reconstruct_y_mode_with_neighbours,
    supported_chroma_mode, supported_chroma_mode_value, uv_mode_ctx,
};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, TileCdfSubset};
use super::intra_joint_modes::{
    LumaPalette, PALETTE_MAX_SIZE, TileFscModeState, TileIntraJointModeState, TileLumaPaletteState,
    TileUseDipState, TileUsesMrlsState,
};
use super::{DecodeTileWorkUnit, partition_size::BlockSize};

const CHROMA_MODE_COUNT: u8 = 8;
const UV_INTRA_MODES_CFL_NOT_ALLOWED: u8 = 13;
const UV_CFL_PRED_MODE: u8 = 13;
const UV_MODE_IDX_BITS: u32 = 3;
const Y_SECOND_MODE_BITS: u32 = 4;
const FIRST_MODE_COUNT: usize = 13;
const SECOND_MODE_COUNT: usize = 16;

const Y_MODE_SET_REASON: &str = "intra_y_mode_set";
const USE_DPCM_Y_REASON: &str = "intra_use_dpcm_y";
const DPCM_MODE_Y_REASON: &str = "intra_dpcm_mode_y";
const USE_DPCM_UV_REASON: &str = "intra_use_dpcm_uv";
const DPCM_MODE_UV_REASON: &str = "intra_dpcm_mode_uv";
// AV2 § 5.20.5.5 and § 5.20.5.6 syntax literals for parse-then-fail-closed DPCM.
const DPCM_VERTICAL_UV_MODE: u8 = 1;
const DPCM_HORIZONTAL_UV_MODE: u8 = 2;
const DPCM_VERTICAL_JOINT_MODE: u8 = 22;
const DPCM_HORIZONTAL_JOINT_MODE: u8 = 50;
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
const USE_DIP_REASON: &str = "intra_use_dip";
const DIP_TRANSPOSE_REASON: &str = "intra_dip_transpose";
const DIP_MODE_REASON: &str = "intra_dip_mode";
const PALETTE_Y_MODE_REASON: &str = "intra_palette_y_mode";
const PALETTE_Y_SIZE_REASON: &str = "intra_palette_y_size";
const PALETTE_CACHE_REASON: &str = "intra_palette_color_cache";
const PALETTE_COLOR_REASON: &str = "intra_palette_color";
const PALETTE_EXTRA_BITS_REASON: &str = "intra_palette_extra_bits";
const PALETTE_DELTA_REASON: &str = "intra_palette_delta";
const LOSSLESS_TX_SIZE_REASON: &str = "lossless_tx_size";
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
const BLOCK_4X4: usize = 0;
const TX_4X4: usize = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaToolConfig {
    enable_cfl_intra: bool,
    enable_mhccp: bool,
    chroma_subsampling_x: u32,
    chroma_subsampling_y: u32,
    enable_idtx_intra: bool,
    enable_mrls: bool,
    enable_dip: bool,
    allow_screen_content_tools: bool,
    lossless: bool,
}

impl GeneralIntraChromaToolConfig {
    #[must_use]
    pub(crate) const fn new(enable_cfl_intra: bool, enable_mhccp: bool) -> Self {
        Self {
            enable_cfl_intra,
            enable_mhccp,
            chroma_subsampling_x: 1,
            chroma_subsampling_y: 1,
            enable_idtx_intra: false,
            enable_mrls: false,
            enable_dip: false,
            allow_screen_content_tools: false,
            lossless: false,
        }
    }

    #[must_use]
    pub(crate) const fn with_chroma_subsampling(
        mut self,
        subsampling_x: u32,
        subsampling_y: u32,
    ) -> Self {
        self.chroma_subsampling_x = if subsampling_x > 1 { 1 } else { subsampling_x };
        self.chroma_subsampling_y = if subsampling_y > 1 { 1 } else { subsampling_y };
        self
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
    pub(crate) const fn with_enable_dip(mut self, enable_dip: bool) -> Self {
        self.enable_dip = enable_dip;
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
    pub(crate) const fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = lossless;
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
    pub(crate) use_dip: u8,
    pub(crate) dip_transpose: u8,
    pub(crate) dip_mode: u8,
    use_dpcm_y: u8,
    dpcm_mode_y: u8,
    use_dpcm_uv: u8,
    dpcm_mode_uv: u8,
    pub(crate) palette_y: Option<LumaPalette>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraChromaBlockMode {
    uv_mode: u8,
    coeff_uv_mode: u8,
    is_cfl: bool,
    cfl_params: Option<CflParams>,
    use_dpcm_uv: u8,
    dpcm_mode_uv: u8,
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
            use_dpcm_uv: 0,
            dpcm_mode_uv: 0,
        }
    }

    const fn dpcm(dpcm_mode_uv: u8) -> Self {
        let uv_mode = if dpcm_mode_uv == 0 {
            DPCM_VERTICAL_UV_MODE
        } else {
            DPCM_HORIZONTAL_UV_MODE
        };
        Self {
            uv_mode,
            coeff_uv_mode: uv_mode,
            is_cfl: false,
            cfl_params: None,
            use_dpcm_uv: 1,
            dpcm_mode_uv,
        }
    }

    const fn cfl(cfl_params: CflParams) -> Self {
        Self {
            uv_mode: UV_CFL_PRED_MODE,
            coeff_uv_mode: UV_CFL_PRED_MODE,
            is_cfl: true,
            cfl_params: Some(cfl_params),
            use_dpcm_uv: 0,
            dpcm_mode_uv: 0,
        }
    }

    #[cfg(test)]
    pub(crate) const fn cfl_for_test(cfl_params: CflParams) -> Self {
        Self::cfl(cfl_params)
    }

    #[cfg(test)]
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

    pub(crate) const fn chroma_dpcm_direction(self) -> Option<DpcmDirection> {
        dpcm_direction(self.use_dpcm_uv != 0, self.dpcm_mode_uv == 0)
    }

    pub(crate) fn supported_chroma_mode(self, y_mode: IntraYMode) -> Option<SupportedChromaMode> {
        if self.is_cfl {
            return None;
        }
        if self.use_dpcm_uv != 0 {
            return supported_chroma_mode_value(self.coeff_uv_mode);
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
    pub(crate) use_dip: u8,
    pub(crate) dip_transpose: u8,
    pub(crate) dip_mode: u8,
    pub(crate) use_dpcm_y: u8,
    pub(crate) dpcm_mode_y: u8,
}

impl GeneralIntraLumaBlockMode {
    #[must_use]
    pub(crate) const fn with_dip(mut self, use_dip: u8, dip_transpose: u8, dip_mode: u8) -> Self {
        self.use_dip = use_dip;
        self.dip_transpose = dip_transpose;
        self.dip_mode = dip_mode;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecodedLumaYMode {
    y_mode: IntraYMode,
    angle_delta_y: i8,
    intra_joint_mode: u8,
    use_dpcm_y: u8,
    dpcm_mode_y: u8,
}

impl DecodedLumaYMode {
    const fn from_y_mode(result: YModeEscapeResult) -> Self {
        Self {
            y_mode: result.y_mode,
            angle_delta_y: result.angle_delta_y,
            intra_joint_mode: result.intra_joint_mode,
            use_dpcm_y: 0,
            dpcm_mode_y: 0,
        }
    }

    const fn dpcm(dpcm_mode_y: u8) -> Self {
        if dpcm_mode_y == 0 {
            Self {
                y_mode: IntraYMode::dpcm_vertical(),
                angle_delta_y: 0,
                intra_joint_mode: DPCM_VERTICAL_JOINT_MODE,
                use_dpcm_y: 1,
                dpcm_mode_y,
            }
        } else {
            Self {
                y_mode: IntraYMode::dpcm_horizontal(),
                angle_delta_y: 0,
                intra_joint_mode: DPCM_HORIZONTAL_JOINT_MODE,
                use_dpcm_y: 1,
                dpcm_mode_y,
            }
        }
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
            use_dip: luma.use_dip,
            dip_transpose: luma.dip_transpose,
            dip_mode: luma.dip_mode,
            use_dpcm_y: luma.use_dpcm_y,
            dpcm_mode_y: luma.dpcm_mode_y,
            use_dpcm_uv: 0,
            dpcm_mode_uv: 0,
            palette_y: None,
        }
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
            use_dip: luma.use_dip,
            dip_transpose: luma.dip_transpose,
            dip_mode: luma.dip_mode,
            use_dpcm_y: luma.use_dpcm_y,
            dpcm_mode_y: luma.dpcm_mode_y,
            use_dpcm_uv: chroma.use_dpcm_uv,
            dpcm_mode_uv: chroma.dpcm_mode_uv,
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

    pub(crate) fn supported_chroma_mode(&self) -> Option<SupportedChromaMode> {
        if self.is_cfl {
            return None;
        }
        if self.use_dpcm_uv != 0 {
            return supported_chroma_mode_value(self.coeff_uv_mode);
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

    pub(crate) const fn uses_active_dip(&self) -> bool {
        self.use_dip != 0
    }

    pub(crate) const fn luma_dpcm_direction(&self) -> Option<DpcmDirection> {
        dpcm_direction(self.use_dpcm_y != 0, self.dpcm_mode_y == 0)
    }

    pub(crate) const fn chroma_dpcm_direction(&self) -> Option<DpcmDirection> {
        dpcm_direction(self.use_dpcm_uv != 0, self.dpcm_mode_uv == 0)
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
    #[error(
        "general intra mode-info modeIdx {mode_idx} with directional-neighbour ctx {ctx} requires §5.20.5.5 reorder support"
    )]
    UnsupportedDirectionalNeighbourReorder { ctx: usize, mode_idx: usize },
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
    block_size: BlockSize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraLumaBlockMode, GeneralIntraBlockModeError> {
    let mode_ctx = joint_modes.y_mode_index_ctx(block_r, block_c, block_n4w, block_n4h);
    let neighbour_joint_modes =
        joint_modes.neighbour_joint_modes(block_r, block_c, block_n4w, block_n4h);
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    let y_mode_result = decode_luma_y_mode(
        cdfs,
        symbols,
        chroma_tools.lossless,
        mode_ctx,
        neighbour_joint_modes,
        block_n4w,
        block_n4h,
    )?;

    let fsc_mode = if allow_fsc_intra(chroma_tools, block_n4w, block_n4h) {
        let bsize_group = fsc_bsize_group(block_size);
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
    if chroma_tools.enable_mrls
        && y_mode_result.use_dpcm_y == 0
        && y_mode_result.y_mode.is_directional()
    {
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
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: y_mode_result.use_dpcm_y,
        dpcm_mode_y: y_mode_result.dpcm_mode_y,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_block_modes_with_fsc_context(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    use_dip: &TileUseDipState,
    fsc_modes: &TileFscModeState,
    use_neighbor_fsc_context: bool,
    palette_state: &TileLumaPaletteState,
    is_cfl_ctx: usize,
    luma_block_size: BlockSize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
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
        luma_block_size,
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
        luma_block_size,
        chroma_n4w,
        chroma_n4h,
    )?;
    let palette_y = read_general_intra_palette_y_mode(
        work_unit,
        symbols,
        chroma_tools,
        palette_state,
        luma.y_mode,
        luma_block_size.index(),
        block_r,
        block_c,
        block_n4w,
        block_n4h,
        bit_depth_bits,
    )?;
    let (use_dip_value, dip_transpose, dip_mode) = read_general_intra_dip_mode_info(
        work_unit,
        symbols,
        chroma_tools,
        use_dip,
        luma.y_mode,
        palette_y,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
    )?;
    let luma = luma.with_dip(use_dip_value, dip_transpose, dip_mode);

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
        let palette_size_symbol = cdfs
            .read_block_symbol_trace(TileCdfSelector::PaletteYSize, symbols)
            .map_err(|source| GeneralIntraBlockModeError::SymbolRead {
                reason: PALETTE_Y_SIZE_REASON,
                source,
            })?;
        let palette_size = usize::from(palette_size_symbol.get()) + 2;
        let colors = read_palette_colors_y(
            symbols,
            palette_state,
            block_r,
            block_c,
            palette_size,
            bit_depth_bits,
        )?;
        return Ok(Some(LumaPalette::from_size_symbol(
            palette_size_symbol,
            colors,
        )));
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_general_intra_dip_mode_info(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    use_dip: &TileUseDipState,
    y_mode: IntraYMode,
    palette_y: Option<LumaPalette>,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<(u8, u8, u8), GeneralIntraBlockModeError> {
    if !dip_mode_info_allowed(chroma_tools, y_mode, palette_y, block_n4w, block_n4h) {
        return Ok((0, 0, 0));
    }

    let ctx = use_dip.use_dip_ctx(block_r, block_c, block_n4w, block_n4h);
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let use_dip_value = read_symbol(
        cdfs,
        symbols,
        TileCdfSelector::UseDip { ctx },
        USE_DIP_REASON,
    )?;
    if use_dip_value == 0 {
        return Ok((0, 0, 0));
    }

    let dip_transpose = read_literal_u8(symbols, 1, DIP_TRANSPOSE_REASON)?;
    let dip_mode = read_symbol(cdfs, symbols, TileCdfSelector::DipMode, DIP_MODE_REASON)?;
    Ok((use_dip_value, dip_transpose, dip_mode))
}

pub(crate) fn read_lossless_luma_tx_size(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    block_size: BlockSize,
    fsc_mode: bool,
    allow_select: bool,
) -> Result<usize, GeneralIntraBlockModeError> {
    read_lossless_tx_size(
        work_unit,
        symbols,
        block_size,
        fsc_mode,
        allow_select,
        false,
    )
}

pub(crate) fn read_lossless_tx_size(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    block_size: BlockSize,
    fsc_mode: bool,
    allow_select: bool,
    is_inter: bool,
) -> Result<usize, GeneralIntraBlockModeError> {
    if block_size.index() == BLOCK_4X4 || (!is_inter && !fsc_mode) || !allow_select {
        return Ok(TX_4X4);
    }
    let size_group = lossless_tx_size_group(block_size);
    let large = read_symbol(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        TileCdfSelector::LosslessTxSize {
            size_group,
            is_inter: usize::from(is_inter),
        },
        LOSSLESS_TX_SIZE_REASON,
    )?;
    if large != 0 {
        return Ok(lossless_max_tx_size(block_size));
    }
    Ok(TX_4X4)
}

fn read_palette_colors_y(
    symbols: &mut SymbolDecoder<'_>,
    palette_state: &TileLumaPaletteState,
    block_r: usize,
    block_c: usize,
    palette_size: usize,
    bit_depth_bits: u32,
) -> Result<[u16; PALETTE_MAX_SIZE], GeneralIntraBlockModeError> {
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
    Ok(colors)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_chroma_block_mode(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    mode_context: GeneralIntraChromaModeContext,
    y_mode: IntraYMode,
    block_size: BlockSize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraChromaBlockMode, GeneralIntraBlockModeError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    if chroma_tools.lossless {
        let use_dpcm_uv = read_symbol(
            cdfs,
            symbols,
            TileCdfSelector::UseDpcmUv,
            USE_DPCM_UV_REASON,
        )?;
        if use_dpcm_uv != 0 {
            let dpcm_mode_uv = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::DpcmModeUv,
                DPCM_MODE_UV_REASON,
            )?;
            return Ok(GeneralIntraChromaBlockMode::dpcm(dpcm_mode_uv));
        }
    }

    let cfl_allowed = mode_context.cfl_allowed_in_sdp
        && cfl_allowed_for_chroma_mode(chroma_tools, block_n4w, block_n4h);
    let mhccp_allowed = mode_context.cfl_allowed_in_sdp
        && mhccp_allowed_for_chroma_mode(chroma_tools, block_n4w, block_n4h);
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
            let cfl_params = read_cfl_alphas(
                work_unit,
                symbols,
                chroma_tools,
                block_size,
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

    let uv_mode = if uv_mode_base == CHROMA_MODE_COUNT - 1 {
        let idx = read_literal_u8(symbols, UV_MODE_IDX_BITS, UV_MODE_IDX_REASON)?;
        uv_mode_base.saturating_add(idx)
    } else {
        uv_mode_base
    };
    if uv_mode >= UV_INTRA_MODES_CFL_NOT_ALLOWED {
        return Err(GeneralIntraBlockModeError::InvalidUvMode { uv_mode });
    }

    let coeff_uv_mode = get_intra_uv_mode_set(y_mode, uv_mode)
        .ok_or(GeneralIntraBlockModeError::InvalidUvMode { uv_mode })?;

    Ok(GeneralIntraChromaBlockMode::no_cfl(uv_mode, coeff_uv_mode))
}

fn read_cfl_alphas(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    block_size: BlockSize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<CflParams, GeneralIntraBlockModeError> {
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let mhccp_allowed = mhccp_allowed_for_chroma_mode(chroma_tools, block_n4w, block_n4h);
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
        let size_group = cfl_mh_dir_size_group(block_size);
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
    lossless: bool,
    mode_ctx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Result<DecodedLumaYMode, GeneralIntraBlockModeError> {
    if lossless {
        let use_dpcm_y = read_symbol(cdfs, symbols, TileCdfSelector::UseDpcmY, USE_DPCM_Y_REASON)?;
        if use_dpcm_y != 0 {
            let dpcm_mode_y = read_symbol(
                cdfs,
                symbols,
                TileCdfSelector::DpcmModeY,
                DPCM_MODE_Y_REASON,
            )?;
            return Ok(DecodedLumaYMode::dpcm(dpcm_mode_y));
        }
    }

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
        )
        .map(DecodedLumaYMode::from_y_mode);
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
        )
        .map(DecodedLumaYMode::from_y_mode);
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
        )
        .map(DecodedLumaYMode::from_y_mode);
    }

    let y_mode = reconstruct_minimal_y_mode(y_mode_set, y_mode_index).ok_or(
        GeneralIntraBlockModeError::UnsupportedYMode {
            y_mode_set,
            mode_idx,
        },
    )?;
    Ok(DecodedLumaYMode::from_y_mode(YModeEscapeResult {
        y_mode,
        angle_delta_y: 0,
        intra_joint_mode: y_mode_index,
    }))
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

/// § 5.20.5.6 `cflAllowed`, whose non-lossless arm bounds the chroma plane
/// residual size at 64 samples per side.
fn cfl_allowed_for_non_lossless(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    let (width, height) = chroma_plane_dimensions(chroma_tools, block_n4w, block_n4h);
    chroma_tools.enable_cfl_intra && width <= 64 && height <= 64
}

/// § 5.20.5.6 `is_mhccp_allowed`, whose non-lossless arm tests the chroma
/// plane residual size: `(w > 4 || h > 4) && w <= 32 && h <= 32`.
fn mhccp_allowed_for_non_lossless(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    let (w, h) = chroma_plane_dimensions(chroma_tools, block_n4w, block_n4h);
    chroma_tools.enable_mhccp && (w > 4 || h > 4) && w <= 32 && h <= 32
}

/// `get_plane_residual_size( size, 1 )` in samples: the luma block subsampled
/// per plane, with § 5.20.4's four-sample floor.
fn chroma_plane_dimensions(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> (usize, usize) {
    let width = (block_n4w * 4) >> chroma_tools.chroma_subsampling_x;
    let height = (block_n4h * 4) >> chroma_tools.chroma_subsampling_y;
    (width.max(4), height.max(4))
}

fn cfl_allowed_for_chroma_mode(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    if chroma_tools.lossless {
        return chroma_tools.enable_cfl_intra
            && lossless_chroma_plane_is_4x4(chroma_tools, block_n4w, block_n4h);
    }
    cfl_allowed_for_non_lossless(chroma_tools, block_n4w, block_n4h)
}

fn mhccp_allowed_for_chroma_mode(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    if chroma_tools.lossless {
        return chroma_tools.enable_mhccp
            && lossless_chroma_plane_is_4x4(chroma_tools, block_n4w, block_n4h);
    }
    mhccp_allowed_for_non_lossless(chroma_tools, block_n4w, block_n4h)
}

fn lossless_chroma_plane_is_4x4(
    chroma_tools: GeneralIntraChromaToolConfig,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    if block_n4w == 0 || block_n4h == 0 {
        return false;
    }
    let chroma_n4w = (block_n4w >> chroma_tools.chroma_subsampling_x).max(1);
    let chroma_n4h = (block_n4h >> chroma_tools.chroma_subsampling_y).max(1);
    chroma_n4w == 1 && chroma_n4h == 1
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

fn dip_mode_info_allowed(
    chroma_tools: GeneralIntraChromaToolConfig,
    y_mode: IntraYMode,
    palette_y: Option<LumaPalette>,
    block_n4w: usize,
    block_n4h: usize,
) -> bool {
    chroma_tools.enable_dip
        && y_mode == IntraYMode::DC_PRED
        && palette_y.is_none()
        && block_n4w > 1
        && block_n4h > 1
        && block_n4w.saturating_mul(block_n4h) >= 8
}

fn fsc_bsize_group(block_size: BlockSize) -> usize {
    FSC_BSIZE_GROUPS[block_size.index()]
}

fn lossless_tx_size_group(block_size: BlockSize) -> usize {
    SIZE_GROUP[block_size.index()] as usize
}

fn lossless_max_tx_size(block_size: BlockSize) -> usize {
    let max_tx_size = MAX_TX_SIZE_RECT[block_size.index()] as usize;
    ADJUSTED_TX_SIZE[max_tx_size] as usize
}

fn cfl_mh_dir_size_group(block_size: BlockSize) -> usize {
    SIZE_GROUP[block_size.index()] as usize
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
    let value = cdfs
        .read_block_symbol_trace(selector, symbols)
        .map(splot_core::symbol::Symbol::get)
        .map_err(|source| GeneralIntraBlockModeError::SymbolRead { reason, source })?;
    Ok(value)
}

fn read_literal_u8(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> Result<u8, GeneralIntraBlockModeError> {
    read_literal_u32(symbols, bits, reason).map(|value| value as u8)
}

fn read_literal_u16(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> Result<u16, GeneralIntraBlockModeError> {
    read_literal_u32(symbols, bits, reason).map(|value| value as u16)
}

fn read_literal_u32(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> Result<u32, GeneralIntraBlockModeError> {
    let value = symbols
        .read_literal(bits)
        .map_err(|source| GeneralIntraBlockModeError::Literal { reason, source })?;
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
#[path = "general_intra_block_tests.rs"]
mod tests;
