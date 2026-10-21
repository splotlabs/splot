// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra modes, prediction, and reconstruction of the encoder facade.
//!
//! A leaf is planned with the decoder's own residual plan; each transform block
//! then predicts and reconstructs through the decoder's per-unit kernel
//! dispatch, so the encoder never assembles edges or prediction itself.

use splot_core::headers::frame::{FrameHeaderCore, TxMode};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader};
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
    CFL_EXPLICIT, CHROMA_MODE_COUNT, CflParams, FIRST_MODE_COUNT, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode, GeneralIntraChromaToolConfig, GeneralIntraLumaBlockMode,
    SECOND_MODE_COUNT, UV_MODE_IDX_BITS, Y_SECOND_MODE_BITS, cfl_allowed_for_chroma_mode,
    cfl_alpha_u_ctx, cfl_alpha_v_ctx,
};
use super::super::general_intra_residual::{LumaTransformTypeContext, intra_chroma_plane_tx_type};
use super::super::partition_traversal::{
    DecodedLeafPublication, GeneralIntraLeafMode, GeneralIntraTreeWalkError,
    is_cfl_context_for_chroma_ref, publish_intra_leaf_state,
};
use super::{
    EncoderSyntaxError, IntraMode, IntraTileSyntax, LeafBlock, Result, TransformBlock, TxPartition,
    coefficient_scan, rule, tx_log2,
};
use crate::pipeline::general_intra::{
    GeneralIntraReconScratch, chroma_edge_smoothness, general_intra_block_ctx,
    general_intra_chroma_tools, inherited_chroma_angle_delta, luma_transform_type_context,
    record_chroma_smooth, rect_chroma_plan, rect_luma_plan, sequence_cfl_ds_filter_index,
};
use crate::prediction::intra_edge::{IntraEdgeCtx, TileSmoothGrid};
use crate::residual::pipeline::GeneralIntraResidualPlan;
use crate::tile::SbRowWindow;
use crate::tile::block_context::BlockCtx;

/// `cfl_index` value `CFL_DERIVED_ALPHA` (§ 6.19.7.25 Table 6.26).
const CFL_DERIVED_ALPHA: usize = 1;
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
}

impl IntraModes {
    /// `DC_PRED` luma and chroma.
    pub const DC: Self = Self {
        y_mode: IntraMode::Dc,
        angle_delta: 0,
        uv_mode: ChromaMode::Intra(IntraMode::Dc),
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
    chroma: GeneralIntraChromaToolConfig,
    cfl_ds_filter_index: u8,
    enable_ibp: bool,
    enable_intra_edge_filter: bool,
    enable_chroma_dctonly: bool,
    bit_depth: BitDepth,
    qindex: u32,
    pub(super) tx_mode: Option<TxMode>,
    pub(super) reduced_tx_set: usize,
    pub(super) reduced_tx_part_set: bool,
}

impl IntraTools {
    /// Rejects every tool whose mode syntax the facade does not write.
    pub(super) fn new(seq: &SequenceHeader, core: &FrameHeaderCore, qindex: u32) -> Result<Self> {
        let (Some(intra), Some(tq)) = (seq.intra, seq.transform_quant_entropy) else {
            return Err(EncoderSyntaxError::Unsupported(
                "incomplete sequence header",
            ));
        };
        if intra.enable_mrls
            || intra.enable_dip
            || intra.enable_mhccp
            || tq.enable_idtx_intra
            || core.allow_screen_content_tools == Some(true)
        {
            return Err(EncoderSyntaxError::Unsupported(
                "MRL, DIP, MHCCP, FSC, or palette",
            ));
        }
        Ok(Self {
            chroma: general_intra_chroma_tools(seq, core),
            cfl_ds_filter_index: sequence_cfl_ds_filter_index(seq),
            enable_ibp: intra.enable_ibp,
            enable_intra_edge_filter: intra.enable_intra_edge_filter,
            enable_chroma_dctonly: tq.enable_chroma_dctonly,
            bit_depth: match seq.general.bit_depth_idc {
                BitDepthIdc::Eight => BitDepth::Eight,
                BitDepthIdc::Ten => BitDepth::Ten,
            },
            qindex,
            tx_mode: core.intra_tail.as_ref().map(|tail| tail.tx_mode),
            reduced_tx_set: core
                .intra_tail
                .as_ref()
                .map_or(0, |tail| usize::from(tail.reduced_tx_set)),
            reduced_tx_part_set: tq.reduced_tx_part_set,
        })
    }
}

/// One leaf planned for prediction and reconstruction.
pub(super) struct PlannedLeaf {
    plan: GeneralIntraResidualPlan,
    units: Vec<TransformBlock>,
    intra_edge: IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
}

/// The reconstructed frame and the state its prediction reads.
pub(super) struct Recon {
    workspace: CurrentFrameWorkspace<u16>,
    scratch: GeneralIntraReconScratch<u16>,
    pub(super) block_decoded: TileBlockDecodedState,
    y_smooth: TileSmoothGrid,
    chroma_smooth: TileSmoothGrid,
    leaf: Option<PlannedLeaf>,
}

/// The edge state and the saved samples of one 4:2:0 luma rectangle.
pub(super) struct ReconSnapshot {
    block_decoded: TileBlockDecodedState,
    y_smooth: TileSmoothGrid,
    chroma_smooth: TileSmoothGrid,
    samples: Vec<(PlaneId, PlaneRect, Vec<u16>)>,
}

impl Recon {
    /// Saves the edge state and the samples of the luma rectangle
    /// `(x, y, width, height)`, clipped to storage.
    pub(super) fn snapshot(
        &self,
        (x, y, w, h): (usize, usize, usize, usize),
    ) -> Result<ReconSnapshot> {
        let mut samples = Vec::with_capacity(3);
        for (plane, shift) in [(PlaneId::Y, 0), (PlaneId::U, 1), (PlaneId::V, 1)] {
            let size = self
                .workspace
                .plane(plane)
                .map_err(rule("snapshot plane"))?
                .storage_size();
            let (px, py) = (x >> shift, y >> shift);
            let pw = (x + w)
                .div_ceil(1 << shift)
                .min(size.width())
                .saturating_sub(px);
            let ph = (y + h)
                .div_ceil(1 << shift)
                .min(size.height())
                .saturating_sub(py);
            if pw == 0 || ph == 0 {
                continue;
            }
            let rect = PlaneRect::new(px, py, pw, ph).map_err(rule("snapshot rect"))?;
            let mut saved = Vec::with_capacity(pw * ph);
            for row in self
                .workspace
                .rect_rows(plane, rect)
                .map_err(rule("snapshot rows"))?
            {
                saved.extend_from_slice(row);
            }
            samples.push((plane, rect, saved));
        }
        Ok(ReconSnapshot {
            block_decoded: self.block_decoded.clone(),
            y_smooth: self.y_smooth.clone(),
            chroma_smooth: self.chroma_smooth.clone(),
            samples,
        })
    }

    /// Puts a [`Self::snapshot`] back.
    pub(super) fn restore(&mut self, snapshot: &ReconSnapshot) -> Result<()> {
        self.block_decoded.clone_from(&snapshot.block_decoded);
        self.y_smooth.clone_from(&snapshot.y_smooth);
        self.chroma_smooth.clone_from(&snapshot.chroma_smooth);
        self.leaf = None;
        for (plane, rect, samples) in &snapshot.samples {
            self.workspace
                .write_rect(*plane, *rect, samples, rect.width())
                .map_err(rule("restore rows"))?;
        }
        Ok(())
    }

    /// A 4:2:0 frame of the decoder's geometry: `FrameWidth x FrameHeight`
    /// samples over `MiCols * 4 x MiRows * 4` storage.
    pub(super) fn new(
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
        tools: &IntraTools,
        (mi_rows, mi_cols): (usize, usize),
        sb_size4: usize,
    ) -> Result<Self> {
        if seq.general.chroma_format_idc != ChromaFormatIdc::Yuv420 {
            return Err(EncoderSyntaxError::Unsupported(
                "chroma format other than 4:2:0",
            ));
        }
        let size = core
            .frame_size
            .ok_or(EncoderSyntaxError::Unsupported("missing frame size"))?;
        let (width, height) = (size.width as usize, size.height as usize);
        let workspace = PlaneSize::new(width, height)
            .and_then(|size| {
                DecodedFrameInfo::new(
                    OutputIndex::new(0),
                    tools.bit_depth,
                    PixelFormat::Yuv420,
                    size,
                    PlaneRect::new(0, 0, width, height)?,
                )
            })
            .and_then(|info| info.with_storage_luma_size(PlaneSize::new(mi_cols * 4, mi_rows * 4)?))
            .and_then(|info| CurrentFrameWorkspace::new(info, 0))
            .map_err(rule("frame workspace"))?;
        let mut block_decoded = TileBlockDecodedState::default();
        block_decoded
            .reset(3, 1, 1, sb_size4, mi_cols, mi_rows)
            .map_err(rule("BlockDecoded state"))?;
        let grid = |rows: usize, cols: usize| {
            let mut grid = TileSmoothGrid::default();
            grid.reset_for_tile(0..rows, 0..cols, SbRowWindow::WHOLE_TILE_SB_H4)
                .map(|()| grid)
                .map_err(|_| EncoderSyntaxError::Unsupported("smooth grid size"))
        };
        Ok(Self {
            workspace,
            scratch: GeneralIntraReconScratch::default(),
            block_decoded,
            y_smooth: grid(mi_rows, mi_cols)?,
            chroma_smooth: grid(mi_rows.div_ceil(2), mi_cols.div_ceil(2))?,
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
            ChromaMode::CflDerived => {
                self.put(symbols, TileCdfSelector::CflIndex, CFL_DERIVED_ALPHA)?;
            }
            ChromaMode::Cfl { alpha_u, alpha_v } => {
                self.put(
                    symbols,
                    TileCdfSelector::CflIndex,
                    usize::from(CFL_EXPLICIT),
                )?;
                self.write_cfl_alphas(symbols, alpha_u, alpha_v)?;
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
        let (block_ctx, _) = self.block_ctx(leaf);
        let (_, joint) = self.y_mode_index(leaf, modes)?;
        let block_modes = block_modes(leaf, modes, joint)?;
        let luma = rect_luma_plan(&block_modes, block_ctx, false, self.sb_size4)
            .map_err(rule("luma plan"))?;
        let chroma = rect_chroma_plan(&block_modes, self.tools.cfl_ds_filter_index, self.sb_size4);
        let mut plan = GeneralIntraResidualPlan::rect(block_ctx, luma, chroma, false, None, false)
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
                let tx_type = if plane == 0 {
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
        let zeros = vec![0; 1 << (log2_w.min(5) + log2_h.min(5))];
        self.reconstruct_with(index, &zeros, 0, tx.tx_type)?;
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
        let tx_type = self.unit(index)?.tx_type;
        self.reconstruct_as(index, tx_type, levels)
    }

    /// [`Self::reconstruct`] with transform type `tx_type`, which must be the type
    /// the decoder reads or infers for these levels.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a level count that does not match the
    /// block, or without a planned block `index`.
    pub fn reconstruct_as(&mut self, index: usize, tx_type: TxType, levels: &[i32]) -> Result<()> {
        let tx = self.unit(index)?;
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (width, height) = (1usize << log2_w.min(5), 1usize << log2_h.min(5));
        if levels.len() != width * height {
            return Err(EncoderSyntaxError::Unsupported(
                "level count of a transform block",
            ));
        }
        let scan = coefficient_scan(&TransformBlock { tx_type, ..tx })?;
        let eob = scan
            .iter()
            .rposition(|&pos| levels[usize::from(pos)] != 0)
            .map_or(0, |last| last + 1);
        self.reconstruct_with(index, levels, eob, tx_type)?;
        if let Some(leaf) = &self.recon.leaf
            && tx.plane == 0
        {
            leaf.plan
                .publish_unit(index, &mut self.recon.block_decoded)
                .map_err(rule("BlockDecoded update"))?;
        }
        Ok(())
    }

    /// Publishes a coded leaf into the neighbour, smoothness, and `BlockDecoded` state.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid geometry or mode.
    pub fn publish_intra_leaf(&mut self, leaf: &LeafBlock, modes: IntraModes) -> Result<()> {
        let (_, joint) = self.y_mode_index(leaf, modes)?;
        let block_modes = block_modes(leaf, modes, joint)?;
        let mut mode = GeneralIntraLeafMode::luma(
            joint,
            modes.y_mode,
            modes.angle_delta,
            0,
            MrlSelection::Disabled,
        );
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
        Ok(())
    }

    /// Saves the coding context and the reconstruction of the luma rectangle
    /// `(x, y, width, height)`; a trial that writes only inside it can roll back
    /// with [`Self::restore`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a rectangle outside the frame.
    pub fn snapshot(&self, luma: (usize, usize, usize, usize)) -> Result<super::SyntaxSnapshot> {
        Ok(super::SyntaxSnapshot {
            cdfs: self.cdfs.clone(),
            coeff: self.coeff.clone(),
            mi_sizes: self.mi_sizes.clone(),
            joint_modes: self.joint_modes.clone(),
            uses_mrls: self.uses_mrls.clone(),
            use_dip: self.use_dip.clone(),
            fsc_modes: self.fsc_modes.clone(),
            palette_y: self.palette_y.clone(),
            uv_cfls: self.uv_cfls.clone(),
            y_modes: self.y_modes.clone(),
            recon: self.recon.snapshot(luma)?,
        })
    }

    /// Puts a [`Self::snapshot`] back.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] if the saved samples no longer fit.
    pub fn restore(&mut self, snapshot: &super::SyntaxSnapshot) -> Result<()> {
        self.cdfs.clone_from(&snapshot.cdfs);
        self.coeff.clone_from(&snapshot.coeff);
        self.mi_sizes.clone_from(&snapshot.mi_sizes);
        self.joint_modes.clone_from(&snapshot.joint_modes);
        self.uses_mrls.clone_from(&snapshot.uses_mrls);
        self.use_dip.clone_from(&snapshot.use_dip);
        self.fsc_modes.clone_from(&snapshot.fsc_modes);
        self.palette_y.clone_from(&snapshot.palette_y);
        self.uv_cfls.clone_from(&snapshot.uv_cfls);
        self.y_modes.clone_from(&snapshot.y_modes);
        self.recon.restore(&snapshot.recon)
    }

    /// The reconstructed frame.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] if the frame does not freeze.
    pub fn finish(self) -> Result<DecodedFrame<u16>> {
        self.recon.workspace.freeze().map_err(rule("frame"))
    }

    fn reconstruct_with(
        &mut self,
        index: usize,
        quant: &[i32],
        eob: usize,
        tx_type: TxType,
    ) -> Result<()> {
        let Recon {
            workspace,
            scratch,
            block_decoded,
            leaf,
            ..
        } = &mut self.recon;
        let leaf = leaf.as_ref().ok_or(NO_LEAF)?;
        leaf.plan
            .reconstruct_unit(
                index,
                quant,
                eob,
                tx_type as usize,
                scratch,
                workspace,
                block_decoded,
                self.tools.qindex,
                leaf.intra_edge,
                leaf.luma_context,
            )
            .map_err(rule("reconstruction"))
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
        if alpha_u.abs() > MAX_CFL_ALPHA || alpha_v.abs() > MAX_CFL_ALPHA {
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

    /// The § 5.20.5.5 mode index that codes the luma modes of `modes` in `leaf`,
    /// found by running the decoder's list derivation over every index.
    fn y_mode_index(&self, leaf: &LeafBlock, modes: IntraModes) -> Result<(usize, IntraJointMode)> {
        let (n4w, n4h) = leaf_size4(leaf)?;
        let (r, c) = (leaf.frontier.r, leaf.frontier.c);
        let ctx = self.y_mode_ctx(leaf)?;
        let neighbours = self.joint_modes.neighbour_joint_modes(r, c, n4w, n4h);
        (0..)
            .map_while(|value| ModeIndex::try_new(value).ok())
            .find_map(|index| {
                let decoded = if ctx == 0 {
                    reconstruct_y_mode_top_left(index)
                } else {
                    reconstruct_y_mode_with_neighbours(index, neighbours, n4w, n4h)
                }
                .ok()?;
                (decoded.y_mode == modes.y_mode && decoded.angle_delta_y == modes.angle_delta)
                    .then_some((index.value(), decoded.intra_joint_mode))
            })
            .ok_or(EncoderSyntaxError::Unsupported("luma mode and angle delta"))
    }

    fn block_ctx(&self, leaf: &LeafBlock) -> (BlockCtx, Option<(usize, usize)>) {
        general_intra_block_ctx(
            &leaf.frontier,
            self.chroma,
            (0, 0..self.frame.mi_cols()),
            self.mi_dimensions(),
            self.tools.bit_depth,
        )
    }

    fn put(
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

const NO_LEAF: EncoderSyntaxError = EncoderSyntaxError::Unsupported("unplanned transform block");

/// The decoder's block modes of `modes` in `leaf`, without MRL, FSC, DIP, or palette.
fn block_modes(
    leaf: &LeafBlock,
    modes: IntraModes,
    intra_joint_mode: IntraJointMode,
) -> Result<GeneralIntraBlockModes> {
    let luma = GeneralIntraLumaBlockMode {
        y_mode: modes.y_mode,
        angle_delta_y: modes.angle_delta,
        intra_joint_mode,
        mrl: MrlSelection::Disabled,
        fsc_mode: 0,
        use_dip: 0,
        dip_transpose: 0,
        dip_mode: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };
    if !leaf.has_chroma() {
        return Ok(GeneralIntraBlockModes::luma_only(luma));
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
    };
    Ok(GeneralIntraBlockModes::from_luma_chroma_palette(
        luma, chroma, None,
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
