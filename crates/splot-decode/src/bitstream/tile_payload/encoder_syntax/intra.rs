// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra modes, prediction, and reconstruction of the encoder facade.
//!
//! A leaf is planned with the decoder's own residual plan; each transform block
//! then predicts and reconstructs through the decoder's per-unit kernel
//! dispatch, so the encoder never assembles edges or prediction itself.

use core::ops::Range;

use splot_core::headers::frame::{FrameHeaderCore, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, SequenceHeader};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, OutputIndex, PixelFormat,
    PlaneId, PlaneRect, PlaneSize,
};

use super::super::block_decoded_state::TileBlockDecodedState;
use super::super::cdf::TileCdfSelector;
use super::super::cdf::block_context::{
    IntraJointMode, MODE_INDEX_COUNT, ModeIndex, MrlSelection, get_intra_uv_mode_set,
    reconstruct_y_mode_top_left, reconstruct_y_mode_with_neighbours, supported_chroma_mode,
    uv_mode_ctx,
};
use super::super::general_intra_block::{
    CFL_DERIVED_ALPHA, CFL_EXPLICIT, CHROMA_MODE_COUNT, CflMultiDirection, CflParams,
    FIRST_MODE_COUNT, GeneralIntraBlockModes, GeneralIntraChromaBlockMode,
    GeneralIntraChromaToolConfig, GeneralIntraLumaBlockMode, SECOND_MODE_COUNT, UV_MODE_IDX_BITS,
    Y_SECOND_MODE_BITS, cfl_allowed_for_chroma_mode, cfl_alpha_u_ctx, cfl_alpha_v_ctx,
    cfl_mh_dir_size_group, mhccp_allowed_for_chroma_mode,
};
use super::super::general_intra_block::{
    INTER_FSC_MODE_CONTEXT, allow_fsc_intra, dip_mode_info_allowed, fsc_bsize_group,
};
use super::super::general_intra_residual::{LumaTransformTypeContext, intra_chroma_plane_tx_type};
use super::super::intra_joint_modes::LumaPalette;
use super::super::partition_traversal::{
    DecodedLeafPublication, GeneralIntraLeafMode, GeneralIntraTreeWalkError,
    is_cfl_context_for_chroma_ref, publish_intra_leaf_state,
};
use super::super::tile_frontier::chroma_subsampling;
use super::palette::{Palette, PlannedPalette, unit_map};
use super::{
    EncoderSyntaxError, IntraMode, IntraTileSyntax, Ist, LeafBlock, Result, TileEnd,
    TransformBlock, TxPartition, coefficient_scan, rule, tx_log2,
};
use crate::filters::wienerns_lr::intrabc_records::frame_allows_intrabc;
use crate::pipeline::general_intra::{
    GeneralIntraReconScratch, chroma_edge_smoothness, general_intra_block_ctx,
    general_intra_chroma_tools, inherited_chroma_angle_delta, luma_transform_type_context,
    record_chroma_smooth, rect_chroma_plan, rect_luma_plan, sequence_cfl_ds_filter_index,
};
use crate::prediction::inter::chroma_smooth_tile_ranges;
use crate::prediction::intra_edge::{IntraEdgeCtx, TileSmoothGrid};
use crate::residual::pipeline::GeneralIntraResidualPlan;
use crate::tile::block_context::BlockCtx;

/// `CFL_ALPHABET_SIZE` (§ 3): the largest `|CflAlphaU|` and `|CflAlphaV|`.
pub const MAX_CFL_ALPHA: i8 = 8;

/// The chroma prediction of a leaf (§ 5.20.5.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChromaMode {
    /// A `UVMode` other than `UV_CFL_PRED`; `AngleDeltaUV` follows the luma mode.
    Intra(IntraMode),
    /// `UV_CFL_PRED` with explicit alphas, each in `-MAX_CFL_ALPHA..=MAX_CFL_ALPHA`
    /// and not both zero.
    Cfl {
        /// `CflAlphaU`.
        alpha_u: i8,
        /// `CflAlphaV`.
        alpha_v: i8,
    },
    /// `UV_CFL_PRED` with `CFL_DERIVED_ALPHA`.
    CflDerived,
    /// `UV_CFL_PRED` with `CFL_MULTI` (MHCCP) and `cfl_mh_dir`, `0..3`.
    Mhccp {
        /// `cfl_mh_dir`.
        direction: u8,
    },
}

/// The prediction modes of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraModes {
    /// `YMode`.
    pub y_mode: IntraMode,
    /// `AngleDeltaY`, `-3..=3`; zero for a non-directional mode.
    pub angle_delta: i8,
    /// The chroma prediction; a leaf without chroma ignores it.
    pub uv_mode: ChromaMode,
    /// The luma reference line of a directional `y_mode`.
    pub mrl: Mrl,
    /// `fsc_mode`: the luma blocks code IDTX, with FSC levels when the
    /// sequence enables FSC.
    pub fsc: bool,
    /// `use_dip` with its mode: a `DC_PRED` luma prediction by DIP.
    pub dip: Option<Dip>,
    /// `has_palette_y` with its palette: a `DC_PRED` luma prediction from
    /// the color map [`IntraTileSyntax::set_palette_map`] sets.
    pub palette: Option<Palette>,
}

/// `dip_mode` and `dip_transpose` of a DIP luma prediction (§ 5.20.7.9).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Dip {
    /// `dip_mode`, below [`IntraTileSyntax::dip_modes`].
    pub mode: u8,
    /// `dip_transpose`.
    pub transpose: bool,
}

impl IntraModes {
    /// `DC_PRED` luma and chroma.
    pub const DC: Self = Self {
        y_mode: IntraMode::Dc,
        angle_delta: 0,
        uv_mode: ChromaMode::Intra(IntraMode::Dc),
        mrl: Mrl::OFF,
        fsc: false,
        dip: None,
        palette: None,
    };
}

/// `mrl_index` and `mrl_sec_index` (§ 5.20.5.5): the reference line a
/// directional luma prediction reads, and whether it blends a second angle.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Mrl {
    /// `mrl_index`, `0..=3`; `0` reads the adjacent line and ignores `secondary`.
    pub index: u8,
    /// `mrl_sec_index`.
    pub secondary: bool,
}

impl Mrl {
    /// The adjacent line.
    pub const OFF: Self = Self {
        index: 0,
        secondary: false,
    };
}

/// A § 3 transform type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TxType {
    /// `DCT_DCT`.
    DctDct,
    /// `ADST_DCT`.
    AdstDct,
    /// `DCT_ADST`.
    DctAdst,
    /// `ADST_ADST`.
    AdstAdst,
    /// `FLIPADST_DCT`.
    FlipadstDct,
    /// `DCT_FLIPADST`.
    DctFlipadst,
    /// `FLIPADST_FLIPADST`.
    FlipadstFlipadst,
    /// `ADST_FLIPADST`.
    AdstFlipadst,
    /// `FLIPADST_ADST`.
    FlipadstAdst,
    /// `IDTX`.
    Idtx,
    /// `V_DCT`.
    VDct,
    /// `H_DCT`.
    HDct,
    /// `V_ADST`.
    VAdst,
    /// `H_ADST`.
    HAdst,
    /// `V_FLIPADST`.
    VFlipadst,
    /// `H_FLIPADST`.
    HFlipadst,
}

impl TxType {
    const ALL: [Self; 16] = [
        Self::DctDct,
        Self::AdstDct,
        Self::DctAdst,
        Self::AdstAdst,
        Self::FlipadstDct,
        Self::DctFlipadst,
        Self::FlipadstFlipadst,
        Self::AdstFlipadst,
        Self::FlipadstAdst,
        Self::Idtx,
        Self::VDct,
        Self::HDct,
        Self::VAdst,
        Self::HAdst,
        Self::VFlipadst,
        Self::HFlipadst,
    ];

    pub(super) fn from_value(value: usize) -> Result<Self> {
        Self::ALL
            .get(value)
            .copied()
            .ok_or(EncoderSyntaxError::Unsupported("transform type"))
    }
}

/// The sequence and frame facts intra prediction reads.
pub(super) struct IntraTools {
    pub(super) chroma: GeneralIntraChromaToolConfig,
    cfl_ds_filter_index: u8,
    enable_ibp: bool,
    enable_intra_edge_filter: bool,
    enable_mrls: bool,
    /// `enable_intra_ist`.
    pub(super) enable_intra_ist: bool,
    /// `enable_inter_ddt`.
    pub(super) enable_inter_ddt: bool,
    /// `enable_inter_ist`.
    pub(super) enable_inter_ist: bool,
    /// `enable_fsc`.
    pub(super) enable_fsc: bool,
    /// `enable_idtx_intra`: leaves may code `fsc_mode`.
    fsc_mode: bool,
    /// `enable_cctx`.
    pub(super) enable_cctx: bool,
    pub(super) enable_chroma_dctonly: bool,
    pub(super) bit_depth: BitDepth,
    pub(super) qindex: u32,
    pub(super) tx_mode: Option<TxMode>,
    pub(super) reduced_tx_set: usize,
    pub(super) reduced_tx_part_set: bool,
    /// `allow_parity_hiding`.
    pub(super) parity_hiding: bool,
    /// `allow_tcq`.
    pub(super) tcq: bool,
    /// TCQ blocks code plain levels for a search trial.
    pub(super) scalar_trial: bool,
}

impl IntraTools {
    /// Rejects every tool whose syntax or reconstruction the facade does not model.
    pub(super) fn new(seq: &SequenceHeader, core: &FrameHeaderCore, qindex: u32) -> Result<Self> {
        let tail = match (&core.intra_tail, &core.inter_tail) {
            (Some(tail), _) => Some((tail.tx_mode, tail.reduced_tx_set)),
            (None, Some(tail)) => Some((tail.tx_mode, tail.reduced_tx_set)),
            (None, None) => None,
        };
        let (Some(intra), Some(tq), Some((tx_mode, reduced_tx_set)), true) = (
            seq.intra,
            seq.transform_quant_entropy,
            tail,
            seq.partition.is_some(),
        ) else {
            return Err(EncoderSyntaxError::Unsupported("incomplete header"));
        };
        let lossless = core.lossless_info.as_ref();
        let refused = [
            (
                frame_allows_intrabc(core)
                    && !(core.frame_is_intra == Some(true)
                        && core.intrabc.is_some_and(|params| {
                            params.allow_global_intrabc == Some(true)
                                && params.allow_local_intrabc == Some(false)
                        })),
                "intra block copy other than global-only in an intra frame",
            ),
            (
                qindex == 0
                    || lossless
                        .is_some_and(|info| info.coded_lossless || info.has_lossless_segment),
                "lossless",
            ),
            (
                core.segmentation_params
                    .as_ref()
                    .is_some_and(|seg| seg.segmentation_enabled)
                    || core
                        .setup_qm_params
                        .as_ref()
                        .is_some_and(|qm| qm.using_qmatrix),
                "segmentation or quantizer matrices",
            ),
            (tx_mode == TxMode::Only4x4, "TX_MODE_ONLY_4X4"),
        ];
        if let Some(&(_, what)) = refused.iter().find(|&&(on, _)| on) {
            return Err(EncoderSyntaxError::Unsupported(what));
        }
        Ok(Self {
            chroma: general_intra_chroma_tools(seq, core),
            cfl_ds_filter_index: sequence_cfl_ds_filter_index(seq),
            enable_ibp: intra.enable_ibp,
            enable_intra_edge_filter: intra.enable_intra_edge_filter,
            enable_mrls: intra.enable_mrls,
            enable_intra_ist: tq.enable_intra_ist,
            enable_inter_ddt: tq.enable_inter_ddt,
            enable_inter_ist: tq.enable_inter_ist,
            enable_fsc: tq.enable_fsc,
            fsc_mode: tq.enable_idtx_intra,
            enable_cctx: tq.enable_cctx,
            enable_chroma_dctonly: tq.enable_chroma_dctonly,
            bit_depth: sequence_bit_depth(seq),
            qindex,
            tx_mode: Some(tx_mode),
            reduced_tx_set: usize::from(reduced_tx_set),
            reduced_tx_part_set: tq.reduced_tx_part_set,
            parity_hiding: lossless.is_some_and(|info| info.allow_parity_hiding),
            tcq: lossless.is_some_and(|info| info.allow_tcq),
            scalar_trial: false,
        })
    }
}

/// The luma modes of every § 5.20.5.5 mode index under one neighbour context:
/// `(top-left, neighbour joint modes, n4w, n4h)`.
#[derive(Default)]
pub(super) struct YModeTable {
    key: Option<(bool, [IntraJointMode; 2], usize, usize)>,
    entries: Vec<(IntraMode, i8, usize, IntraJointMode)>,
}

/// One leaf planned for prediction and reconstruction.
pub(super) struct PlannedLeaf {
    plan: GeneralIntraResidualPlan,
    palette: Option<PlannedPalette>,
    units: Vec<TransformBlock>,
    intra_edge: IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
}

/// The reconstructed frame and the state its prediction reads.
pub(super) struct Recon {
    pub(super) workspace: CurrentFrameWorkspace<u16>,
    scratch: GeneralIntraReconScratch<u16>,
    pub(super) block_decoded: TileBlockDecodedState,
    pub(super) y_smooth: TileSmoothGrid,
    pub(super) chroma_smooth: TileSmoothGrid,
    pub(super) leaf: Option<PlannedLeaf>,
}

impl Recon {
    /// A frame of the decoder's geometry: `FrameWidth x FrameHeight`
    /// samples over `MiCols * 4 x MiRows * 4` storage, with the prediction
    /// state of the tile over luma 4x4 `rows` and `cols`.
    pub(super) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        tools: &IntraTools,
        (rows, cols): (Range<usize>, Range<usize>),
        sb_size4: usize,
    ) -> Result<Self> {
        let chroma = seq.general.chroma_format_idc;
        if chroma.is_monochrome() {
            return Err(EncoderSyntaxError::Unsupported("monochrome"));
        }
        let (sub_x, sub_y) = chroma_subsampling(chroma);
        let size = core
            .frame_size
            .ok_or(EncoderSyntaxError::Unsupported("missing frame size"))?;
        let (width, height) = (size.width as usize, size.height as usize);
        let (mi_rows, mi_cols) = (height.div_ceil(8) * 2, width.div_ceil(8) * 2);
        let workspace = PlaneSize::new(width, height)
            .and_then(|size| {
                DecodedFrameInfo::new(
                    OutputIndex::new(0),
                    tools.bit_depth,
                    PixelFormat::from_av2_chroma_format_idc(chroma.get())?,
                    size,
                    PlaneRect::new(0, 0, width, height)?,
                )
            })
            .and_then(|info| info.with_storage_luma_size(PlaneSize::new(mi_cols * 4, mi_rows * 4)?))
            .and_then(|info| CurrentFrameWorkspace::new(info, 0))
            .map_err(rule("frame workspace"))?;
        let mut block_decoded = TileBlockDecodedState::default();
        block_decoded
            .reset(3, sub_x.into(), sub_y.into(), sb_size4, cols.end, rows.end)
            .map_err(rule("BlockDecoded state"))?;
        let grid = |(rows, cols): (Range<usize>, Range<usize>), sb_h4: usize| {
            let mut grid = TileSmoothGrid::default();
            grid.reset_for_tile(rows, cols, sb_h4)
                .map(|()| grid)
                .map_err(|_| EncoderSyntaxError::Unsupported("smooth grid size"))
        };
        Ok(Self {
            workspace,
            scratch: GeneralIntraReconScratch::default(),
            block_decoded,
            y_smooth: grid((rows.clone(), cols.clone()), sb_size4)?,
            chroma_smooth: grid(
                chroma_smooth_tile_ranges(rows, cols, chroma),
                sb_size4 >> usize::from(sub_y),
            )?,
            leaf: None,
        })
    }
}

impl IntraTileSyntax {
    /// Whether `leaf` may code `UV_CFL_PRED` (§ 5.20.5.6 `cflAllowed`).
    #[must_use]
    pub fn cfl_allowed(&self, leaf: &LeafBlock) -> bool {
        let (_, chroma) = self.block_ctx(leaf);
        leaf.has_chroma()
            && chroma
                .is_some_and(|(n4w, n4h)| cfl_allowed_for_chroma_mode(self.tools.chroma, n4w, n4h))
    }

    /// Whether `leaf` may code `fsc_mode` (§ 5.20.5.5 `allow_fsc_intra`).
    #[must_use]
    pub fn fsc_allowed(&self, leaf: &LeafBlock) -> bool {
        self.tools.fsc_mode
            && leaf_size4(leaf).is_ok_and(|(n4w, n4h)| allow_fsc_intra(self.tools.chroma, n4w, n4h))
    }

    /// Whether the planned intra leaf codes `fsc_mode`.
    pub(super) const fn planned_fsc(&self) -> bool {
        self.fsc_leaf
    }

    /// Whether `leaf` may code MHCCP (§ 5.20.5.6 `is_mhccp_allowed`).
    #[must_use]
    pub fn mhccp_allowed(&self, leaf: &LeafBlock) -> bool {
        let (_, chroma) = self.block_ctx(leaf);
        self.cfl_allowed(leaf)
            && chroma.is_some_and(|(n4w, n4h)| {
                mhccp_allowed_for_chroma_mode(self.tools.chroma, n4w, n4h)
            })
    }

    /// Writes the § 5.20.5.5 luma and § 5.20.5.6 chroma mode syntax of `leaf`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a mode the leaf cannot
    /// code, and [`EncoderSyntaxError::Rule`] for a symbol write failure.
    pub fn write_intra_modes(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        self.write_y_uv_modes(leaf, modes, symbols)?;
        self.write_palette_mode_info(leaf, modes, symbols)?;
        self.write_dip(leaf, modes, symbols)
    }

    /// Whether `leaf` codes `use_dip` under the luma mode of `modes` (§ 5.20.7.9).
    #[must_use]
    pub fn dip_allowed(&self, leaf: &LeafBlock, modes: IntraModes) -> bool {
        let palette = modes
            .palette
            .and_then(|palette| self.luma_palette(palette).ok());
        leaf_size4(leaf).is_ok_and(|(n4w, n4h)| {
            dip_mode_info_allowed(self.tools.chroma, modes.y_mode, palette, n4w, n4h)
        })
    }

    /// The number of `dip_mode` values.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a missing CDF row.
    pub fn dip_modes(&mut self) -> Result<u8> {
        let arity = self
            .tile_row(TileCdfSelector::DipMode)?
            .len()
            .saturating_sub(1);
        u8::try_from(arity).map_err(|_| EncoderSyntaxError::Unsupported("DIP modes"))
    }

    fn write_dip(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        if !self.dip_allowed(leaf, modes) {
            return match modes.dip {
                Some(_) => Err(EncoderSyntaxError::Unsupported("DIP in this block")),
                None => Ok(()),
            };
        }
        let (n4w, n4h) = leaf_size4(leaf)?;
        let ctx = self
            .use_dip
            .use_dip_ctx(leaf.frontier.r, leaf.frontier.c, n4w, n4h);
        self.put(
            symbols,
            TileCdfSelector::UseDip { ctx },
            usize::from(modes.dip.is_some()),
        )?;
        let Some(dip) = modes.dip else {
            return Ok(());
        };
        if dip.mode >= self.dip_modes()? {
            return Err(EncoderSyntaxError::Unsupported("DIP mode"));
        }
        literal(symbols, usize::from(dip.transpose), 1)?;
        self.put(symbols, TileCdfSelector::DipMode, dip.mode.into())
    }

    fn write_y_uv_modes(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let (mode_idx, _) = self.y_mode_index(leaf, modes)?;
        let ctx = self.y_mode_ctx(leaf)?;
        if mode_idx < FIRST_MODE_COUNT {
            let last = usize::from(MODE_INDEX_COUNT - 1);
            self.put(symbols, TileCdfSelector::YModeSet, 0)?;
            self.put(
                symbols,
                TileCdfSelector::YModeIndex { ctx },
                mode_idx.min(last),
            )?;
            if mode_idx >= last {
                self.put(
                    symbols,
                    TileCdfSelector::YModeOffset { ctx },
                    mode_idx - last,
                )?;
            }
        } else {
            let second = mode_idx - FIRST_MODE_COUNT;
            self.put(
                symbols,
                TileCdfSelector::YModeSet,
                1 + second / SECOND_MODE_COUNT,
            )?;
            literal(symbols, second % SECOND_MODE_COUNT, Y_SECOND_MODE_BITS)?;
        }
        if self.fsc_allowed(leaf) {
            let (n4w, n4h) = leaf_size4(leaf)?;
            let ctx = if !self.inter_frame() || !leaf.frontier.is_mixed_region() {
                self.fsc_modes
                    .fsc_mode_ctx(leaf.frontier.r, leaf.frontier.c, n4w, n4h)
            } else {
                INTER_FSC_MODE_CONTEXT
            };
            let bsize_group = fsc_bsize_group(leaf.frontier.b_size);
            let selector = TileCdfSelector::FscMode { ctx, bsize_group };
            self.put(symbols, selector, modes.fsc.into())?;
        } else if modes.fsc {
            return Err(EncoderSyntaxError::Unsupported("FSC in this block"));
        }
        self.mrl_selection(modes)?;
        if self.tools.enable_mrls && modes.y_mode.is_directional() {
            let (n4w, n4h) = leaf_size4(leaf)?;
            let (r, c) = (leaf.frontier.r, leaf.frontier.c);
            let ctx = self.uses_mrls.mrl_index_ctx(r, c, n4w, n4h);
            self.put(
                symbols,
                TileCdfSelector::MrlIndex { ctx },
                modes.mrl.index.into(),
            )?;
            if modes.mrl.index > 0 {
                let ctx = self.uses_mrls.mrl_sec_index_ctx(r, c, n4w, n4h);
                let secondary = modes.mrl.secondary.into();
                self.put(symbols, TileCdfSelector::MrlSecIndex { ctx }, secondary)?;
            }
        }
        if !leaf.has_chroma() {
            return Ok(());
        }
        let is_cfl = !matches!(modes.uv_mode, ChromaMode::Intra(_));
        if self.cfl_allowed(leaf) {
            let ctx = is_cfl_context_for_chroma_ref(
                &self.uv_cfls,
                self.bounds,
                leaf.frontier.chroma_ref_geometry(),
            )
            .get();
            self.put(symbols, TileCdfSelector::IsCfl { ctx }, usize::from(is_cfl))?;
        } else if is_cfl {
            return Err(EncoderSyntaxError::Unsupported("CFL in this block"));
        }
        match modes.uv_mode {
            ChromaMode::Intra(uv_mode) => {
                let symbol = usize::from(uv_mode_symbol(modes.y_mode, uv_mode)?);
                let last = usize::from(CHROMA_MODE_COUNT - 1);
                let ctx = uv_mode_ctx(modes.y_mode);
                self.put(
                    symbols,
                    TileCdfSelector::UvModeCflNotAllowed { ctx },
                    symbol.min(last),
                )?;
                if symbol >= last {
                    literal(symbols, symbol - last, UV_MODE_IDX_BITS)?;
                }
            }
            ChromaMode::CflDerived | ChromaMode::Cfl { .. } | ChromaMode::Mhccp { .. } => {
                self.write_cfl_index(leaf, modes.uv_mode, symbols)?;
            }
        }
        Ok(())
    }

    /// Plans `leaf` under `modes` and returns its transform blocks in § 5.20.7.23
    /// order; [`Self::predict`] and [`Self::reconstruct`] take their indices.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a mode the leaf cannot code.
    pub fn plan_leaf(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<Vec<TransformBlock>> {
        self.plan_leaf_partitioned(leaf, modes, TxPartition::None)
    }

    /// [`Self::plan_leaf`] with the luma transform split by `partition`, one of
    /// [`Self::luma_tx_partitions`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a mode or a partition the leaf cannot code.
    pub fn plan_leaf_partitioned(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        partition: TxPartition,
    ) -> Result<Vec<TransformBlock>> {
        self.clear_inter_plan();
        self.plan_cctx(leaf);
        self.fsc_leaf = modes.fsc;
        let (block_ctx, _) = self.block_ctx(leaf);
        let (_, joint) = self.y_mode_index(leaf, modes)?;
        let palette = self.plan_palette(leaf, modes)?;
        let mrl = self.mrl_selection(modes)?;
        let block_modes = block_modes(leaf, modes, joint, mrl, palette.map(|p| p.palette))?;
        let luma = rect_luma_plan(&block_modes, block_ctx, self.tools.tcq, self.sb_size4)
            .map_err(rule("luma plan"))?;
        let chroma = rect_chroma_plan(&block_modes, self.tools.cfl_ds_filter_index, self.sb_size4);
        if modes.fsc && !self.fsc_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported("FSC in this block"));
        }
        let mut plan =
            GeneralIntraResidualPlan::rect(block_ctx, luma, chroma, modes.fsc, None, false)
                .map_err(rule("residual plan"))?;
        if let Some(tx_size) = self.partitioned_tx_size(leaf, partition)? {
            plan.partition_luma(tx_size, partition)
                .map_err(rule("transform partition"))?;
        }
        let uv_mode = block_modes.coeff_uv_mode();
        let angle_delta_uv = inherited_chroma_angle_delta(uv_mode, modes.y_mode, modes.angle_delta);
        let units = plan
            .transform_blocks()
            .map(|(plane, x, y, tx_size, fills_block)| {
                let tx_type = if plane == 0 && modes.fsc {
                    TxType::Idtx
                } else if plane == 0 {
                    TxType::DctDct
                } else {
                    TxType::from_value(
                        intra_chroma_plane_tx_type(
                            plane,
                            tx_size,
                            uv_mode,
                            i32::from(angle_delta_uv),
                            self.tools.enable_chroma_dctonly,
                        )
                        .map_err(rule("chroma transform type"))?,
                    )?
                };
                Ok(TransformBlock {
                    tx_type,
                    plane,
                    x,
                    y,
                    tx_size,
                    fills_block,
                    ist: Ist::OFF,
                    cctx: 0,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let (above_smooth, left_smooth) = self
            .recon
            .y_smooth
            .block_smoothness(leaf.frontier.c, leaf.frontier.r);
        let (chroma_above_smooth, chroma_left_smooth) =
            chroma_edge_smoothness(Some(&self.recon.chroma_smooth), block_ctx);
        self.recon.leaf = Some(PlannedLeaf {
            plan,
            palette,
            units: units.clone(),
            intra_edge: IntraEdgeCtx {
                enable_ibp: self.tools.enable_ibp,
                enable_intra_edge_filter: self.tools.enable_intra_edge_filter,
                above_smooth,
                left_smooth,
                chroma_above_smooth,
                chroma_left_smooth,
            },
            luma_context: luma_transform_type_context(&block_modes),
        });
        Ok(units)
    }

    /// Predicts planned transform block `index` into the frame and returns the
    /// row-major prediction; samples past the frame storage repeat its edge.
    ///
    /// The block is not marked decoded, so predicting candidate modes leaves the
    /// availability of later blocks unchanged.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned block `index`.
    pub fn predict(&mut self, index: usize) -> Result<Vec<u16>> {
        let tx = self.unit(index)?;
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let zeros = &ZERO_LEVELS[..1 << (log2_w.min(5) + log2_h.min(5))];
        self.reconstruct_with(index, zeros, 0, &tx)?;
        self.reconstruction(&tx)
    }

    /// The row-major samples the frame holds over `tx`; samples past the frame
    /// storage repeat its edge.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a block outside the planes.
    pub fn reconstruction(&self, tx: &TransformBlock) -> Result<Vec<u16>> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (width, height) = (1usize << log2_w, 1usize << log2_h);
        let plane = plane_id(tx.plane)?;
        let storage = self
            .recon
            .workspace
            .plane(plane)
            .map_err(rule("sample readback"))?
            .storage_size();
        let visible_w = width.min(storage.width().saturating_sub(tx.x));
        let visible_h = height.min(storage.height().saturating_sub(tx.y));
        if visible_w == 0 || visible_h == 0 {
            return Ok(vec![0; width * height]);
        }
        let rect =
            PlaneRect::new(tx.x, tx.y, visible_w, visible_h).map_err(rule("sample readback"))?;
        let mut out = Vec::with_capacity(width * height);
        for row in self
            .recon
            .workspace
            .rect_rows(plane, rect)
            .map_err(rule("sample readback"))?
        {
            out.extend_from_slice(row);
            out.resize(out.len() + width - visible_w, row[visible_w - 1]);
        }
        while out.len() < width * height {
            let start = out.len() - width;
            out.extend_from_within(start..start + width);
        }
        Ok(out)
    }

    /// Reconstructs planned transform block `index` from its row-major adjusted
    /// `levels` (at most 32x32) and marks a luma block decoded.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a level count that does not match the
    /// block, or without a planned block `index`.
    pub fn reconstruct(&mut self, index: usize, levels: &[i32]) -> Result<()> {
        let tx = self.unit(index)?;
        self.reconstruct_as(index, &tx, levels)
    }

    /// [`Self::reconstruct`] with the transform type and secondary transform
    /// of `coded`, planned block `index` as the decoder reads or infers them
    /// for these levels.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a level count that does not match the
    /// block, a `coded` block elsewhere, or without a planned block `index`.
    pub fn reconstruct_as(
        &mut self,
        index: usize,
        coded: &TransformBlock,
        levels: &[i32],
    ) -> Result<()> {
        let tx = self.unit(index)?;
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (width, height) = (1usize << log2_w.min(5), 1usize << log2_h.min(5));
        if levels.len() != width * height {
            return Err(EncoderSyntaxError::Unsupported(
                "level count of a transform block",
            ));
        }
        let coded = TransformBlock {
            tx_type: coded.tx_type,
            ist: coded.ist,
            ..tx
        };
        let eob = coefficient_scan(&coded)?
            .iter()
            .rposition(|&pos| levels[usize::from(pos)] != 0)
            .map_or(0, |last| last + 1);
        let quant = self.decoder_quant(&coded, levels)?;
        self.reconstruct_with(index, &quant, eob, &coded)?;
        if let Some(leaf) = &self.recon.leaf
            && tx.plane == 0
        {
            leaf.plan
                .publish_unit(index, &mut self.recon.block_decoded)
                .map_err(rule("BlockDecoded update"))?;
        }
        Ok(())
    }

    /// Reconstructs planned chroma blocks `u_index` and `v_index` of one
    /// position from their row-major adjusted `levels` with the `cctx` of
    /// `u_coded` (nonzero) and the transform type of both.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for blocks the decoder cannot pair or
    /// without planned blocks.
    pub fn reconstruct_cctx_pair(
        &mut self,
        [u_index, v_index]: [usize; 2],
        u_coded: &TransformBlock,
        [u_levels, v_levels]: [&[i32]; 2],
    ) -> Result<()> {
        let u = TransformBlock {
            tx_type: u_coded.tx_type,
            cctx: u_coded.cctx,
            ..self.unit(u_index)?
        };
        let v = TransformBlock {
            tx_type: u_coded.tx_type,
            ..self.unit(v_index)?
        };
        let (u_eob, v_eob) = (
            super::cctx::block_eob(&u, u_levels)?,
            super::cctx::block_eob(&v, v_levels)?,
        );
        let Recon {
            workspace,
            scratch,
            block_decoded,
            leaf,
            ..
        } = &mut self.recon;
        let leaf = leaf.as_ref().ok_or(NO_LEAF)?;
        leaf.plan
            .reconstruct_cctx_pair(
                [(u_index, u_levels, u_eob), (v_index, v_levels, v_eob)],
                (u.tx_type as usize, usize::from(u.cctx)),
                scratch,
                workspace,
                block_decoded,
                self.tools.qindex,
                leaf.intra_edge,
                leaf.luma_context,
            )
            .map_err(rule("CCTX reconstruction"))
    }

    /// Publishes a coded leaf into the neighbour, smoothness, and `BlockDecoded` state.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid geometry or mode.
    pub fn publish_intra_leaf(&mut self, leaf: &LeafBlock, modes: IntraModes) -> Result<()> {
        let (_, joint) = self.y_mode_index(leaf, modes)?;
        let mrl = self.mrl_selection(modes)?;
        let palette = modes
            .palette
            .map(|palette| self.luma_palette(palette))
            .transpose()?;
        let block_modes = block_modes(leaf, modes, joint, mrl, palette)?;
        let fsc = u8::from(modes.fsc);
        let mut mode = GeneralIntraLeafMode::luma(joint, modes.y_mode, modes.angle_delta, fsc, mrl)
            .with_use_dip(modes.dip.is_some().into())
            .with_palette_y(palette);
        if leaf.frontier.has_chroma {
            mode = mode.with_uv_cfl(block_modes.is_cfl());
        }
        publish_intra_leaf_state::<core::convert::Infallible>(
            self.frame,
            leaf.call,
            &leaf.frontier,
            mode,
            leaf.call.b_size,
            &mut self.joint_modes,
            &mut self.fsc_modes,
            &mut self.use_dip,
            &mut self.uses_mrls,
            &mut self.palette_y,
            &mut self.uv_cfls,
            &mut self.y_modes,
            &mut self.mi_sizes,
        )
        .map_err(|error| match error {
            GeneralIntraTreeWalkError::Traversal(source) => rule("leaf publication")(source),
            GeneralIntraTreeWalkError::MiSize(source) => rule("leaf publication")(source),
            GeneralIntraTreeWalkError::Leaf(never) => match never {},
        })?;
        let (n4w, n4h) = leaf_size4(leaf)?;
        self.recon.y_smooth.record(
            leaf.frontier.r,
            leaf.frontier.c,
            n4w,
            n4h,
            mode.y_mode_is_smooth(),
        );
        if leaf.frontier.has_chroma {
            let (block_ctx, _) = self.block_ctx(leaf);
            record_chroma_smooth(
                Some(&mut self.recon.chroma_smooth),
                block_ctx,
                block_modes.supported_chroma_mode(),
            );
        }
        DecodedLeafPublication::new(leaf.call, leaf.call.b_size, self.sb_size4 - 1)
            .publish_block_decoded(&mut self.recon.block_decoded)
            .map_err(rule("BlockDecoded update"))?;
        self.recon.leaf = None;
        self.publish_inter_neighbours(leaf)
    }

    /// The reconstructed frame and the tile's final CDFs and motion records.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] if the frame does not freeze.
    pub fn finish(self) -> Result<(DecodedFrame<u16>, TileEnd)> {
        let motion = self
            .inter
            .as_ref()
            .map(|inter| inter.tile.temporal_records())
            .unwrap_or_default();
        let frame = self.recon.workspace.freeze().map_err(rule("frame"))?;
        Ok((
            frame,
            TileEnd {
                cdfs: self.cdfs,
                motion,
            },
        ))
    }

    fn reconstruct_with(
        &mut self,
        index: usize,
        quant: &[i32],
        eob: usize,
        coded: &TransformBlock,
    ) -> Result<()> {
        let use_tcq = self.uses_tcq(coded);
        let (tx_type, ist) = (coded.tx_type, coded.ist.syntax(false));
        let (log2_w, log2_h) = tx_log2(coded.tx_size)?;
        let Recon {
            workspace,
            scratch,
            block_decoded,
            leaf,
            ..
        } = &mut self.recon;
        let leaf = leaf.as_ref().ok_or(NO_LEAF)?;
        let palette_map = match coded.plane {
            0 => unit_map(
                (&self.palette_map, &mut self.palette_unit),
                leaf.palette.as_ref(),
                (coded.x, coded.y, 1 << log2_w, 1 << log2_h),
            )?,
            _ => None,
        };
        leaf.plan
            .reconstruct_unit(
                index,
                quant,
                eob,
                (tx_type as usize, use_tcq, ist),
                palette_map,
                scratch,
                workspace,
                block_decoded,
                self.tools.qindex,
                leaf.intra_edge,
                leaf.luma_context,
            )
            .map_err(rule("reconstruction"))
    }

    /// The palette of the planned leaf.
    pub(super) fn planned_palette(&self) -> Result<Option<PlannedPalette>> {
        self.recon
            .leaf
            .as_ref()
            .map(|leaf| leaf.palette)
            .ok_or(NO_LEAF)
    }

    /// The decoder's palette of `palette`.
    fn luma_palette(&self, palette: Palette) -> Result<LumaPalette> {
        palette.luma(u32::from(self.tools.bit_depth.bits()))
    }

    /// The luma transform-type context of the planned leaf.
    pub(super) fn planned_luma_context(&self) -> Result<LumaTransformTypeContext> {
        self.recon
            .leaf
            .as_ref()
            .map(|leaf| leaf.luma_context)
            .ok_or(NO_LEAF)
    }

    fn unit(&self, index: usize) -> Result<TransformBlock> {
        self.recon
            .leaf
            .as_ref()
            .and_then(|leaf| leaf.units.get(index))
            .copied()
            .ok_or(NO_LEAF)
    }

    /// `cfl_mhccp`, `cfl_index`, `cfl_mh_dir` and the alphas of a CFL `uv_mode`
    /// (§ 5.20.7.32).
    fn write_cfl_index(
        &mut self,
        leaf: &LeafBlock,
        uv_mode: ChromaMode,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let multi = matches!(uv_mode, ChromaMode::Mhccp { .. });
        if self.mhccp_allowed(leaf) {
            self.put(symbols, TileCdfSelector::CflMhccp, usize::from(multi))?;
        } else if multi {
            return Err(EncoderSyntaxError::Unsupported("MHCCP in this block"));
        }
        match uv_mode {
            ChromaMode::Mhccp { direction } => {
                CflMultiDirection::from_symbol(direction)
                    .ok_or(EncoderSyntaxError::Unsupported("MHCCP direction"))?;
                let size_group = cfl_mh_dir_size_group(leaf.frontier.b_size);
                self.put(
                    symbols,
                    TileCdfSelector::CflMhDir { size_group },
                    direction.into(),
                )
            }
            ChromaMode::Cfl { alpha_u, alpha_v } => {
                self.put(
                    symbols,
                    TileCdfSelector::CflIndex,
                    usize::from(CFL_EXPLICIT),
                )?;
                self.write_cfl_alphas(symbols, alpha_u, alpha_v)
            }
            _ => self.put(
                symbols,
                TileCdfSelector::CflIndex,
                usize::from(CFL_DERIVED_ALPHA),
            ),
        }
    }

    fn write_cfl_alphas(
        &mut self,
        symbols: &mut impl SymbolSink,
        alpha_u: i8,
        alpha_v: i8,
    ) -> Result<()> {
        let sign = |alpha: i8| -> u8 {
            match alpha.signum() {
                0 => 0,
                -1 => 1,
                _ => 2,
            }
        };
        let (sign_u, sign_v) = (sign(alpha_u), sign(alpha_v));
        let signs = (sign_u * 3 + sign_v)
            .checked_sub(1)
            .ok_or(EncoderSyntaxError::Unsupported("CFL with two zero alphas"))?;
        let max = MAX_CFL_ALPHA.unsigned_abs();
        if alpha_u.unsigned_abs() > max || alpha_v.unsigned_abs() > max {
            return Err(EncoderSyntaxError::Unsupported("CFL alpha magnitude"));
        }
        self.put(symbols, TileCdfSelector::CflSign, usize::from(signs))?;
        if sign_u != 0 {
            let ctx = cfl_alpha_u_ctx(sign_u, sign_v);
            let value = usize::from(alpha_u.unsigned_abs() - 1);
            self.put(symbols, TileCdfSelector::CflAlpha { ctx }, value)?;
        }
        if sign_v != 0 {
            let ctx = cfl_alpha_v_ctx(sign_u, sign_v);
            let value = usize::from(alpha_v.unsigned_abs() - 1);
            self.put(symbols, TileCdfSelector::CflAlpha { ctx }, value)?;
        }
        Ok(())
    }

    /// The decoder's `MrlIndex` selection of `modes`.
    fn mrl_selection(&self, modes: IntraModes) -> Result<MrlSelection> {
        if modes.mrl.index == 0 {
            return Ok(MrlSelection::Disabled);
        }
        if !self.tools.enable_mrls || !modes.y_mode.is_directional() {
            return Err(EncoderSyntaxError::Unsupported("MRL in this block"));
        }
        MrlSelection::from_symbols(modes.mrl.index, Some(modes.mrl.secondary.into()))
            .map_err(|_| EncoderSyntaxError::Unsupported("MRL index"))
    }

    /// The § 5.20.5.5 mode index that codes the luma modes of `modes` in `leaf`,
    /// found by running the decoder's list derivation over every index once
    /// per neighbour context.
    fn y_mode_index(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<(usize, IntraJointMode)> {
        let (n4w, n4h) = leaf_size4(leaf)?;
        let (r, c) = (leaf.frontier.r, leaf.frontier.c);
        let top_left = self.y_mode_ctx(leaf)? == 0;
        let neighbours = self.joint_modes.neighbour_joint_modes(r, c, n4w, n4h);
        let key = (top_left, neighbours, n4w, n4h);
        let table = &mut self.y_mode_table;
        if table.key != Some(key) {
            table.entries.clear();
            for index in (0..).map_while(|value| ModeIndex::try_new(value).ok()) {
                let decoded = if top_left {
                    reconstruct_y_mode_top_left(index)
                } else {
                    reconstruct_y_mode_with_neighbours(index, neighbours, n4w, n4h)
                };
                if let Ok(decoded) = decoded {
                    table.entries.push((
                        decoded.y_mode,
                        decoded.angle_delta_y,
                        index.value(),
                        decoded.intra_joint_mode,
                    ));
                }
            }
            table.key = Some(key);
        }
        table
            .entries
            .iter()
            .find(|&&(y_mode, delta, ..)| y_mode == modes.y_mode && delta == modes.angle_delta)
            .map(|&(.., index, joint)| (index, joint))
            .ok_or(EncoderSyntaxError::Unsupported("luma mode and angle delta"))
    }

    pub(super) fn block_ctx(&self, leaf: &LeafBlock) -> (BlockCtx, Option<(usize, usize)>) {
        general_intra_block_ctx(
            &leaf.frontier,
            self.chroma,
            (self.tile.0.start, self.tile.1.clone()),
            self.mi_dimensions(),
            self.tools.bit_depth,
        )
    }

    pub(super) fn put(
        &mut self,
        symbols: &mut impl SymbolSink,
        selector: TileCdfSelector,
        value: usize,
    ) -> Result<()> {
        let value =
            u8::try_from(value).map_err(|_| EncoderSyntaxError::Unsupported("symbol value"))?;
        let row = self.tile_row(selector)?;
        symbols
            .symbol(row, Symbol::new(value))
            .map_err(rule("symbol write"))
    }
}

/// The sample bit depth of `seq`.
pub(super) const fn sequence_bit_depth(seq: &SequenceHeader) -> BitDepth {
    match seq.general.bit_depth_idc {
        BitDepthIdc::Eight => BitDepth::Eight,
        BitDepthIdc::Ten => BitDepth::Ten,
    }
}

/// The levels of a predicted block: every adjusted (at most 32x32) level zero.
static ZERO_LEVELS: [i32; 1024] = [0; 1024];

const NO_LEAF: EncoderSyntaxError = EncoderSyntaxError::Unsupported("unplanned transform block");

/// The decoder's block modes of `modes` in `leaf` with luma `palette`.
fn block_modes(
    leaf: &LeafBlock,
    modes: IntraModes,
    intra_joint_mode: IntraJointMode,
    mrl: MrlSelection,
    palette: Option<LumaPalette>,
) -> Result<GeneralIntraBlockModes> {
    let luma = GeneralIntraLumaBlockMode {
        y_mode: modes.y_mode,
        angle_delta_y: modes.angle_delta,
        intra_joint_mode,
        mrl,
        fsc_mode: modes.fsc.into(),
        use_dip: modes.dip.is_some().into(),
        dip_transpose: modes.dip.is_some_and(|dip| dip.transpose).into(),
        dip_mode: modes.dip.map_or(0, |dip| dip.mode),
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };
    if !leaf.has_chroma() {
        return Ok(GeneralIntraBlockModes::luma_only(luma).with_palette_y(palette));
    }
    let chroma = match modes.uv_mode {
        ChromaMode::Intra(uv_mode) => {
            let symbol = uv_mode_symbol(modes.y_mode, uv_mode)?;
            GeneralIntraChromaBlockMode::Prediction {
                mode: supported_chroma_mode(modes.y_mode, symbol)
                    .ok_or(EncoderSyntaxError::Unsupported("chroma mode"))?,
                coeff_uv_mode: get_intra_uv_mode_set(modes.y_mode, symbol)
                    .ok_or(EncoderSyntaxError::Unsupported("chroma mode"))?,
                dpcm: None,
            }
        }
        ChromaMode::Cfl { alpha_u, alpha_v } => {
            GeneralIntraChromaBlockMode::Cfl(CflParams::Explicit { alpha_u, alpha_v })
        }
        ChromaMode::CflDerived => GeneralIntraChromaBlockMode::Cfl(CflParams::DerivedAlpha),
        ChromaMode::Mhccp { direction } => GeneralIntraChromaBlockMode::Cfl(CflParams::Multi {
            direction: CflMultiDirection::from_symbol(direction)
                .ok_or(EncoderSyntaxError::Unsupported("MHCCP direction"))?,
        }),
    };
    Ok(GeneralIntraBlockModes::from_luma_chroma_palette(
        luma, chroma, palette,
    ))
}

/// The `uv_mode` symbol whose § 5.20.5.6 `get_intra_uv_mode_set` value is `uv_mode`.
fn uv_mode_symbol(y_mode: IntraMode, uv_mode: IntraMode) -> Result<u8> {
    (0u8..)
        .map_while(|symbol| get_intra_uv_mode_set(y_mode, symbol).map(|mode| (symbol, mode)))
        .find_map(|(symbol, mode)| (mode == uv_mode as u8).then_some(symbol))
        .ok_or(EncoderSyntaxError::Unsupported("chroma mode"))
}

fn leaf_size4(leaf: &LeafBlock) -> Result<(usize, usize)> {
    let size = leaf.frontier.b_size;
    Ok((
        size.num_4x4_wide().map_err(rule("block size"))?,
        size.num_4x4_high().map_err(rule("block size"))?,
    ))
}

fn literal(symbols: &mut impl SymbolSink, value: usize, bits: u32) -> Result<()> {
    symbols
        .literal(value as u32, bits)
        .map_err(rule("symbol write"))
}

fn plane_id(plane: usize) -> Result<PlaneId> {
    [PlaneId::Y, PlaneId::U, PlaneId::V]
        .get(plane)
        .copied()
        .ok_or(EncoderSyntaxError::Unsupported("plane index"))
}
