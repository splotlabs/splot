// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame coding: the RD partition and mode search of each superblock, with
//! intra and single-reference inter leaves, residual coding, and the temporal
//! unit of a key or inter frame.

use std::sync::Arc;

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, FrameHeaderCore,
    GrainScaling, InterpolationFilter, SeqTiles, TipGlobalMv, UniformTiles,
    build_encoder_sequence_header, encode_film_grain_obu, encoder_film_grain_model,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    CCTX_ROTATIONS, EncoderReferences, IntraTileSyntax, Ist, LeafBlock, Palette, ParityHiding,
    Partition, PartitionNode, TransformBlock, TxType, recon_params, tile_count, tx_log2,
};
use splot_parallel::WorkerPool;
use splot_parallel::prelude::*;
use splot_recon::{
    BitDepth, DecodedFrame, DequantBlockParams, PixelFormat, PlaneId, PlaneSize, QmFrameLevels,
    secondary_transform_layout,
};

use crate::cfl_filter::cfl_ds_filter;
use crate::config::Config;
use crate::encoder::{CodedFrame, FrameCoder};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::gop::{self, GopFrame};
use crate::grain;
use crate::intrabc::BvSearch;
use crate::kernels::rdoq::{CoeffRates, RdoqBlock, hide_parity, parity_moves, rdoq};
use crate::kernels::tcq::{TcqScratch, tcq};
use crate::kernels::{BlockDims, ForwardTransform2d, Quantizer, kernels};
use crate::loop_filter::{CodedTile, FilteredFrame, FirstPass, FrameDecisions, finish_frame};
use crate::mode_decision::two_pass_partition;
use crate::motion_search::MotionSearch;
use crate::palette::PaletteSearch;
use crate::preset::{CoeffLevel, FrameKind, PresetContext, PresetParams, ResolutionClass};
use crate::rate_control::FrameType;
use crate::rd::coeff_rate::{CdfCoeffRates, RateSink, RowCaches, block_rate};
use crate::rd::{Bits, Lambda, RdCost};
use crate::screen::is_screen_content;
use crate::tile_writer::coeffs::eob;
use crate::tile_writer::{
    KeyFrameTileWriter, PartitionTree, ResidualCoder, SuperblockDecision, TileWriterError,
    TransformDecision,
};
use crate::tpl;
use crate::tx_choice;
use crate::wavefront;

/// The TCQ trellis multiplier in percent of the RDOQ one: measured best near
/// 60 on park_joy and paris (100 loses 0.7-1.1% BD-rate), while RDOQ itself
/// loses at 60.
const TCQ_LAMBDA_PERCENT: u64 = 60;
/// Quantizer deadzone in Q8 (`128` rounds to nearest).
const DEADZONE_ROUNDING_Q8: u32 = 96;

/// Codes key frames and inter frames.
pub(crate) struct IntraFrameCoder {
    params: EncoderStreamParams,
    sequence: SequenceHeader,
    view: CoreSeqView,
    bit_depth: BitDepth,
    chroma: PixelFormat,
    preset: PresetParams,
    /// The preset row a screen-content sequence switches to; `None` keeps
    /// [`Self::preset`].
    screen: Option<PresetParams>,
    references: EncoderReferences,
    /// The content interpretation OBU of every key frame; empty for none.
    content_obu: Vec<u8>,
    tiles: UniformTiles,
    /// Codes the frames of a wave and the tiles of a frame; `None` with one thread.
    pool: Option<WorkerPool>,
    /// The global motion models every inter frame codes, set by tests.
    #[cfg(test)]
    global_motion: [Option<[i32; 6]>; splot_core::headers::frame::REFS_PER_FRAME],
    /// The global motion models of the next frames to code.
    global_models: crate::global_motion::GlobalModels,
    /// The lookahead's open-loop motion of the frames of the current window.
    open_loop: crate::lookahead::OpenLoopMotion,
    /// The block importances of the next frames to code.
    importance: tpl::BlockImportance,
    /// Show-existing frames output so far, which seed their film grain.
    shown_existing: u64,
    /// The last committed key frame coded a film grain model, which the
    /// output frames after it apply.
    grain_active: bool,
    /// Key frames committed so far; their long-term ids cycle over it.
    key_frames: u32,
    /// Which temporal layers code warped motion, from the committed frames.
    warp_gate: crate::warp::WarpGate,
    /// The intra superblocks of the frame in each reference slot.
    slot_intra: [Option<Arc<IntraCells>>; 16],
    /// Stops the frames of a cancelled encoder.
    pub(crate) cancel: crate::cancel::CancelToken,
    /// Counts the searched superblocks of the encoder's frames.
    pub(crate) progress: crate::progress::Progress,
}

/// One flag per 8x8 luma cell of a frame, in raster order: whether an
/// intra leaf covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntraCells {
    /// Cells per row.
    cols: usize,
    /// Cell rows.
    rows: usize,
    /// The flags.
    flags: Vec<bool>,
    /// Frames since the chain of primary references last searched intra in
    /// every leaf: `0` for a frame that skipped no intra search.
    age: u8,
}

impl IntraCells {
    /// The cells of a `width x height` luma frame that the luma rectangles
    /// `intra` cover, `age` frames after a full intra search.
    fn new(
        (width, height): (usize, usize),
        intra: impl IntoIterator<Item = (usize, usize, usize, usize)>,
        age: u8,
    ) -> Self {
        let (cols, rows) = (width.div_ceil(8), height.div_ceil(8));
        let mut cells = Self {
            cols,
            rows,
            flags: vec![false; cols * rows],
            age,
        };
        for rect in intra {
            let (columns, rows) = cells.span(rect, 0);
            for row in rows {
                for col in columns.clone() {
                    cells.flags[row * cols + col] = true;
                }
            }
        }
        cells
    }

    /// The cell columns and rows of the luma rectangle `(x, y, w, h)` grown
    /// by `margin` cells, inside the frame.
    fn span(
        &self,
        (x, y, w, h): (usize, usize, usize, usize),
        margin: usize,
    ) -> (core::ops::Range<usize>, core::ops::Range<usize>) {
        (
            (x / 8).saturating_sub(margin)..((x + w).div_ceil(8) + margin).min(self.cols),
            (y / 8).saturating_sub(margin)..((y + h).div_ceil(8) + margin).min(self.rows),
        )
    }

    /// Whether an intra leaf covers a cell of the luma rectangle `rect`
    /// grown by `margin` cells.
    pub(crate) fn near(&self, rect: (usize, usize, usize, usize), margin: u8) -> bool {
        let (cols, rows) = self.span(rect, usize::from(margin));
        rows.into_iter()
            .any(|row| cols.clone().any(|col| self.flags[row * self.cols + col]))
    }
}

/// Appends to `out` the luma rectangle of each intra leaf of `tree` at
/// `node` (the luma tree of an SDP node).
fn intra_leaves(
    syntax: &IntraTileSyntax,
    node: PartitionNode,
    tree: &PartitionTree,
    out: &mut Vec<(usize, usize, usize, usize)>,
) -> core::result::Result<(), TileWriterError> {
    if !syntax.in_frame(node) {
        return Ok(());
    }
    match (tree, syntax.sdp_roots(node)) {
        (PartitionTree::Sdp { luma, .. }, Some((root, _))) => intra_leaves(syntax, root, luma, out),
        (PartitionTree::Sdp { .. }, None) => {
            Err(TileWriterError::Shape("SDP tree at a shared node"))
        }
        (PartitionTree::Split { kind, children }, _) => {
            for (child, tree) in syntax.children(node, *kind)?.into_iter().zip(children) {
                intra_leaves(syntax, child, tree, out)?;
            }
            Ok(())
        }
        (PartitionTree::Leaf(choice), _) if choice.inter.is_none() => {
            let (w, h) = node.dimensions()?;
            out.push((node.col4() * 4, node.row4() * 4, w, h));
            Ok(())
        }
        (PartitionTree::Region { .. }, _) => {
            let (w, h) = node.dimensions()?;
            out.push((node.col4() * 4, node.row4() * 4, w, h));
            Ok(())
        }
        (PartitionTree::Leaf(_), _) => Ok(()),
    }
}

impl IntraFrameCoder {
    /// A coder of `config`. Preset overrides apply to the preset row and to
    /// its screen-content row.
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let preset = config.override_preset(config.preset.params())?;
        let mut coder = Self::with_preset(config, &preset)?;
        coder.screen = (preset.screen_detect)
            .then(|| config.override_preset(config.preset.screen_params()))
            .transpose()?;
        Ok(coder)
    }

    /// A coder of `config` with the search limits of `preset`; only the
    /// preset's own row switches to its screen-content row.
    fn with_preset(config: &Config, preset: &PresetParams) -> Result<Self> {
        let params = EncoderStreamParams {
            width: config.width,
            height: config.height,
            bit_depth: match config.bit_depth {
                BitDepth::Eight => BitDepthIdc::Eight,
                BitDepth::Ten => BitDepthIdc::Ten,
            },
            chroma: ChromaFormatIdc::try_new(u32::from(config.chroma.chroma_format_idc())).ok_or(
                Error::UnsupportedChroma {
                    chroma: config.chroma,
                },
            )?,
            sb_size: match preset.max_block_log2 {
                8.. if config.height >= u32::from(preset.sb256_min_height) => {
                    SuperblockSize::Block256x256
                }
                7.. => SuperblockSize::Block128x128,
                _ => SuperblockSize::Block64x64,
            },
            tools: EncoderToolFlags {
                cdef: true,
                loop_restoration: true,
                ccso: preset.ccso_candidates > 0,
                gdf: preset.gdf_candidates > 0,
                cfl: true,
                ibp: true,
                intra_edge_filter: true,
                mrls: preset.mrl_trials > 0,
                intra_ist: preset.ist_sets > 0,
                inter_ddt: preset.inter_ddt,
                inter_ist: preset.inter_ist,
                tip: preset.tip > 0 && preset.mv_prediction,
                tip_output: preset.tip > 0 && preset.mv_prediction && preset.tip_output > 0,
                tip_explicit_qp: preset.tip > 0
                    && preset.mv_prediction
                    && preset.tip_output > 0
                    && preset.df_sub_pu
                    && preset.tip_explicit_qp,
                tip_hole_fill: preset.tip > 0 && preset.mv_prediction && preset.tip_hole_fill,
                opfl_refine: preset.opfl,
                refinemv: preset.refinemv,
                tip_refinemv: preset.tip > 0 && preset.mv_prediction && preset.tip_refinemv,
                cctx: preset.cctx,
                mhccp: preset.mhccp,
                fsc: preset.fsc,
                idtx_intra: preset.idtx_intra && !preset.fsc,
                screen_content: preset.palette || preset.integer_mv,
                dip: preset.dip,
                sdp: preset.sdp,
                extended_sdp: preset.sdp && preset.extended_sdp > 0,
                ext_partitions: preset.ext_partitions > 0,
                uneven_4way_partitions: preset.ext_partitions > 1,
                avg_cdf: preset.cdf_carry.then_some(preset.cdf_avg_across_tiles),
                ref_frame_mvs: preset.mv_prediction,
                refmvbank: preset.mv_prediction,
                drl_reorder: preset.mv_prediction.then_some(!preset.drl_reorder_always),
                mv_traj: preset.mv_prediction && preset.mv_traj,
                max_drl_bits_minus_1: u32::from(preset.drl_bits.saturating_sub(1)),
                mvd_tools: preset.mvd_tools,
                warp: preset.warp > 0,
                global_motion: preset.global_motion > 0,
                masked_compound: preset.compound_types >= 1,
                cwp: preset.compound_types >= 2,
                implicit_mask: preset.implicit_mask,
                df_sub_pu: preset.df_sub_pu,
                df_par_bits_extra: preset.deblock_offset_bits.saturating_sub(3),
                interintra: preset.interintra,
                bawp: preset.bawp,
                parity_hiding: preset.parity_hiding,
                long_term_frame_id_bits: preset.long_term_id_bits,
                num_ref_frames: (preset.num_ref_frames > 0).then_some(preset.num_ref_frames),
                explicit_ref_frame_map: preset.explicit_refs > 0,
                seq_tiles: (preset.seq_tile_info > 0).then_some(SeqTiles {
                    cols_log2: config.tiles.cols_log2,
                    rows_log2: config.tiles.rows_log2,
                    allow_tile_info_change: preset.seq_tile_info > 1,
                }),
                short_refresh: preset.short_refresh,
                tcq: preset.tcq.values().contains(&true),
                film_grain: config.film_grain,
                ..EncoderToolFlags::default()
            },
        };
        let (sequence, view, references) = sequence_state(&params)?;
        Ok(Self {
            params,
            references,
            content_obu: config.content.obu(config.frame_rate)?,
            sequence,
            view,
            bit_depth: config.bit_depth,
            chroma: config.chroma,
            preset: *preset,
            screen: (preset.screen_detect && core::ptr::eq(preset, config.preset.params()))
                .then(|| *config.preset.screen_params()),
            tiles: UniformTiles {
                cols_log2: config.tiles.cols_log2,
                rows_log2: config.tiles.rows_log2,
                middle_cdf_update_tile: preset.cdf_update_tile == 1,
                reuse_tile_info: preset.seq_tile_info < 3,
                ..UniformTiles::default()
            },
            pool: (config.threads.resolve().get() > 1)
                .then(|| WorkerPool::new(config.threads))
                .transpose()
                .map_err(coding("worker pool"))?,
            #[cfg(test)]
            global_motion: [None; splot_core::headers::frame::REFS_PER_FRAME],
            importance: tpl::BlockImportance::default(),
            global_models: crate::global_motion::GlobalModels::new(),
            open_loop: crate::lookahead::OpenLoopMotion::new(),
            shown_existing: 0,
            grain_active: false,
            key_frames: 0,
            warp_gate: crate::warp::WarpGate::default(),
            slot_intra: Default::default(),
            cancel: crate::cancel::CancelToken::default(),
            progress: crate::progress::Progress::default(),
        })
    }
}

impl FrameCoder for IntraFrameCoder {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        map_tiles(self.pool.as_ref(), wave.to_vec(), |(source, plan, qp)| {
            self.code_frame(source, &plan, qp, &self.superblock_quant(&plan, source))
        })
    }

    fn set_block_importance(&mut self, importance: tpl::BlockImportance) {
        self.importance = importance;
    }

    fn wants_global_motion(&self) -> bool {
        self.preset.global_motion > 0
    }

    fn set_global_motion(&mut self, models: crate::global_motion::GlobalModels) {
        self.global_models = models;
    }

    fn set_open_loop_motion(&mut self, motion: crate::lookahead::OpenLoopMotion) {
        self.open_loop = motion;
    }

    fn pool(&self) -> Option<WorkerPool> {
        self.pool.clone()
    }

    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()> {
        if let Some(used) = frame.warp {
            self.warp_gate.record(used);
        }
        if let Some(core) = frame.tip_output.take() {
            return self
                .references
                .update_tip_output(&core)
                .map_err(coding("TIP output update"));
        }
        let (core, recon, initial, cdfs, ccso_blocks) =
            frame.reference.take().ok_or(Error::Unsupported {
                what: "a frame committed twice",
            })?;
        let refreshed = core.refresh_frame_flags.unwrap_or(u32::MAX);
        for (slot, intra) in self.slot_intra.iter_mut().enumerate() {
            if refreshed >> slot & 1 == 1 {
                intra.clone_from(&frame.intra_cells);
            }
        }
        if frame.frame_type == FrameType::Key {
            self.key_frames = self.key_frames.wrapping_add(1);
            self.grain_active = core
                .intra_tail
                .is_some_and(|tail| tail.film_grain.apply_grain);
        }
        self.references
            .update(&core, recon, &initial, &cdfs, &ccso_blocks)
            .map_err(coding("reference update"))
    }

    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
        let seed = self.grain_active.then(|| {
            self.shown_existing += 1;
            gop::shown_grain_seed(self.shown_existing)
        });
        let unit = gop::show_existing_unit(&self.view, slot, seed)?;
        self.references
            .show_existing(slot)
            .map_err(coding("show-existing frame"))?;
        Ok(unit)
    }

    fn start_sequence(&mut self, first: &SourceFrame) -> Result<()> {
        let mut tools = self.params.tools;
        if let Some(screen) = self.screen.filter(|_| is_screen_content(first)) {
            self.preset = screen;
            tools.fsc = screen.fsc;
            tools.screen_content = screen.palette || screen.integer_mv;
        }
        if self.preset.cfl_filter {
            tools.cfl_ds_filter = cfl_ds_filter(first);
        }
        self.set_tools(tools)
    }
}

/// The sequence header of `params`, its view and empty references.
fn sequence_state(
    params: &EncoderStreamParams,
) -> Result<(SequenceHeader, CoreSeqView, EncoderReferences)> {
    let sequence = build_encoder_sequence_header(params).map_err(coding("sequence header"))?;
    let view = CoreSeqView::from_sequence(&sequence).ok_or(Error::Unsupported {
        what: "an incomplete sequence header",
    })?;
    let references = EncoderReferences::new(&sequence).map_err(coding("reference buffer"))?;
    Ok((sequence, view, references))
}

impl IntraFrameCoder {
    /// Rebuilds the sequence with `tools`; call it before the first frame.
    fn set_tools(&mut self, tools: EncoderToolFlags) -> Result<()> {
        if tools != self.params.tools {
            self.params.tools = tools;
            (self.sequence, self.view, self.references) = sequence_state(&self.params)?;
        }
        Ok(())
    }

    /// [`Self::set_tools`] with `cfl_ds_filter_index` `index`.
    #[cfg(test)]
    fn set_cfl_filter(&mut self, index: u8) -> Result<()> {
        self.set_tools(EncoderToolFlags {
            cfl_ds_filter: index,
            ..self.params.tools
        })
    }

    /// The `(qindex offset, lambda log2 Q8)` of each superblock of `plan`
    /// from its block importances, plus [`tpl::ssim_lambda_log2`] of `source`
    /// at `ssim_aq`; empty without either or when every offset
    /// snaps to 0 at the `delta_q_res` step, which codes no `delta_q_present`
    /// and keeps the frame lambda.
    fn superblock_quant(&self, plan: &GopFrame, source: &SourceFrame) -> Vec<(i16, i16)> {
        let costs = self.importance.frames.get(&plan.display_index);
        if costs.is_none() && self.preset.ssim_aq == 0 {
            return Vec::new();
        }
        let sb = match self.view.frame_sb_size(plan.frame_type == FrameType::Key) {
            SuperblockSize::Block64x64 => 64,
            SuperblockSize::Block128x128 => 128,
            SuperblockSize::Block256x256 => 256,
        };
        let (width, height) = (self.params.width as usize, self.params.height as usize);
        let grid = (width.div_ceil(sb), height.div_ceil(sb), sb);
        let mut quant = costs.map_or_else(
            || vec![(0, 0); grid.0 * grid.1],
            |costs| {
                tpl::superblock_quant(
                    costs,
                    self.importance.grid,
                    grid,
                    (self.importance.strength, self.importance.centred),
                )
            },
        );
        if self.preset.ssim_aq > 0 {
            let size = source.plane_sizes()[0];
            let ssim = tpl::ssim_lambda_log2(
                &source.planes()[0],
                (size.width(), size.height()),
                self.bit_depth,
                grid,
                self.preset.ssim_aq,
            );
            for (sb_quant, lambda_log2) in quant.iter_mut().zip(ssim) {
                *sb_quant = tpl::quant_of(i32::from(sb_quant.1) + lambda_log2);
            }
        }
        let half_step = (1 << self.preset.delta_q_res) >> 1;
        if quant.iter().all(|&(offset, _)| offset.abs() <= half_step) {
            return Vec::new();
        }
        quant
    }

    /// The `tip_global_wtd_index` whose TIP prediction of `source` has the
    /// least luma SSE over every other in-frame 64x64 block (a checkerboard)
    /// of the frame `core`, a `TIP_FRAME_AS_REF` frame; ties keep the lower
    /// index.
    fn tip_weight(&self, source: &SourceFrame, core: &FrameHeaderCore) -> Result<u8> {
        const PROBE4: usize = 16;
        let initial = self
            .references
            .frame_start(core)
            .map_err(coding("initial CDFs"))?;
        let mut totals = [0u64; 8];
        let mut padded = None;
        for tile in 0..tile_count(core) {
            let mut writer = KeyFrameTileWriter::with_start(
                &self.sequence,
                core,
                &self.references,
                &initial,
                tile,
            )
            .map_err(coding("tile setup"))?;
            let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
            let sb4 = writer.syntax().superblock_size4();
            let padded = padded.get_or_insert_with(|| {
                PaddedSource::new(
                    source,
                    self.chroma,
                    mi_cols * 4 + sb4 * 4,
                    mi_rows * 4 + sb4 * 4,
                )
            });
            writer
                .probe_superblocks(|syntax, root| {
                    let nodes = if sb4 > PROBE4 {
                        syntax.children(root, Partition::Split)?
                    } else {
                        vec![root]
                    };
                    for node in nodes {
                        let (row4, col4) = (node.row4(), node.col4());
                        let full = row4 + PROBE4 <= mi_rows && col4 + PROBE4 <= mi_cols;
                        if !full || (row4 / PROBE4 + col4 / PROBE4) % 2 == 1 {
                            continue;
                        }
                        let leaf = syntax.leaf(node)?;
                        if !syntax.tip_allowed(&leaf)? {
                            continue;
                        }
                        for (weight, total) in (0u8..).zip(totals.iter_mut()) {
                            for (tx, prediction) in syntax.tip_prediction(&leaf, weight)? {
                                *total += padded.sse(&tx, &prediction)?;
                            }
                        }
                    }
                    Ok(())
                })
                .map_err(coding("TIP weight"))?;
        }
        Ok((0u8..)
            .zip(totals)
            .min_by_key(|&(_, total)| total)
            .map_or(0, |(weight, _)| weight))
    }

    /// The `grain_seed` of `plan` and, for a key frame whose `source` has
    /// noise, the `OBU_FILM_GRAIN` of its model; nothing without film grain.
    fn film_grain(&self, source: &SourceFrame, plan: &GopFrame) -> Result<(Option<u16>, Vec<u8>)> {
        if !self.params.tools.film_grain {
            return Ok((None, Vec::new()));
        }
        let (grain_on, obu) = if plan.frame_type == FrameType::Key {
            let scalings = grain::estimate(source, self.chroma)?;
            let [y, cb, cr] = &scalings;
            if scalings.iter().all(GrainScaling::is_empty) {
                (false, Vec::new())
            } else {
                let model = encoder_film_grain_model(self.params.chroma, [y, cb, cr])
                    .map_err(coding("film grain"))?;
                let obu = encode_film_grain_obu(self.params.chroma, &model)
                    .map_err(coding("film grain"))?;
                (true, obu)
            }
        } else {
            (self.grain_active, Vec::new())
        };
        let seed = (grain_on && plan.show_frame).then(|| gop::coded_grain_seed(plan.frame_index));
        Ok((seed, obu))
    }

    /// Codes `source` as `plan` at quantizer `qp` against the committed
    /// references; non-empty `sb_quant`, one `(qindex offset, lambda log2
    /// Q8)` per superblock in frame raster order, moves the qindex of each
    /// superblock through `delta_q` and scales its lambda.
    /// The context of the [`crate::preset::Levels`] fields for `plan` coded
    /// at `qindex`.
    pub(crate) const fn preset_context(&self, plan: &GopFrame, qindex: u32) -> PresetContext {
        PresetContext {
            resolution: ResolutionClass::of(self.params.width, self.params.height),
            frame: FrameKind::of(plan.frame_type, plan.layer, plan.refresh_frame_flags),
            coeff: CoeffLevel::of(qindex),
        }
    }

    fn code_frame(
        &self,
        source: &SourceFrame,
        plan: &GopFrame,
        qp: u16,
        sb_quant: &[(i16, i16)],
    ) -> Result<CodedFrame> {
        self.cancel.check()?;
        let plan = &GopFrame {
            primary_slot: plan.primary_slot.filter(|_| self.preset.cdf_carry),
            ..*plan
        };
        let qindex = u32::from(qp);
        let context = self.preset_context(plan, qindex);
        let mut tools = gop::frame_tools((&self.preset, context), self.tiles, qindex);
        tools.0.allow_tcq = self.preset.tcq.at(context);
        let ids = (1u32 << self.preset.long_term_id_bits) - 1;
        if plan.frame_type == FrameType::Key && ids > 0 {
            tools.0.long_term_id_plus_1 = 1 + self.key_frames % ids;
        }
        tools.0.disable_cdf_update = self.preset.disable_cdf_update;
        tools.0.disable_cross_frame_cdf_init = self.preset.primary_default_cdfs;
        tools.0.choose_primary_ref_frame = self.preset.primary_ref_choose;
        tools.0.qm_level = crate::tx_choice::qm_level(&self.preset, qindex, self.bit_depth);
        #[cfg(test)]
        {
            tools.2.7 = self.global_motion;
        }
        if plan.frame_type == FrameType::Inter && !self.global_models.is_empty() {
            let probe = plan.header(
                &self.view,
                &self.references,
                qindex,
                &FrameFilterParams::default(),
                tools,
            )?;
            let hints = probe
                .inter
                .as_ref()
                .map_or(&[][..], |inter| &inter.order_hints[..]);
            for (model, pair) in tools.2.7.iter_mut().zip(reference_pairs(plan, hints)) {
                *model = pair.and_then(|pair| self.global_models.get(&pair)).copied();
            }
        }
        tools.2.6 &= self.warp_gate.allows(plan.layer);
        tools.0.delta_q_res = (!sb_quant.is_empty()).then_some(self.preset.delta_q_res);
        let (grain_seed, grain_obu) = self.film_grain(source, plan)?;
        let key_obus = if plan.frame_type == FrameType::Key {
            [self.content_obu.as_slice(), &grain_obu].concat()
        } else {
            grain_obu
        };
        tools.0.grain_seed = grain_seed;
        if tools.2.3.is_some() || tools.2.5 != 0 {
            let probe = plan.header(
                &self.view,
                &self.references,
                qindex,
                &FrameFilterParams::default(),
                tools,
            )?;
            if !self.references.tip_pair_available(&probe) {
                (tools.2.3, tools.2.5) = (None, 0);
            } else if tools.2.3.is_some() && self.preset.tip >= 2 && !self.params.tools.tip_refinemv
            {
                tools.2.3 = Some(self.tip_weight(source, &probe)?);
            }
        }
        let header = |filters: &FrameFilterParams| {
            plan.header(&self.view, &self.references, qindex, filters, tools)
        };
        let core = header(&FrameFilterParams::default())?;
        let initial = self
            .references
            .frame_start(&core)
            .map_err(coding("initial CDFs"))?;
        let writers = (0..tile_count(&core))
            .map(|tile| {
                KeyFrameTileWriter::with_start(
                    &self.sequence,
                    &core,
                    &self.references,
                    &initial,
                    tile,
                )
            })
            .collect::<core::result::Result<Vec<_>, _>>()
            .map_err(coding("tile setup"))?;
        let first = writers.first().ok_or(Error::Unsupported {
            what: "a frame without tiles",
        })?;
        let (mi_rows, mi_cols) = first.syntax().mi_dimensions();
        let sb = first.syntax().superblock_size4() * 4;
        let padded = PaddedSource::new(source, self.chroma, mi_cols * 4 + sb, mi_rows * 4 + sb);
        let lambda = Lambda::from_qindex(qindex, self.bit_depth).map_err(coding("lambda"))?;
        let open_loop = match &core.inter {
            Some(inter) if !self.open_loop.is_empty() => reference_pairs(plan, &inter.order_hints)
                .map(|pair| pair.and_then(|pair| self.open_loop.get(&pair).cloned()))
                .collect(),
            _ => Vec::new(),
        };
        let motion = MotionSearch::new(
            first.syntax(),
            &padded,
            (&self.preset, context),
            lambda,
            open_loop.clone(),
        )
        .map_err(coding("motion search"))?;
        let bv_search = (core.allow_intrabc == Some(true)).then(|| {
            let (luma, stride, width, height) = padded.luma();
            BvSearch::new(luma, stride, (width, height))
        });
        let (padded, motion, bv_search) = (&padded, motion.as_ref(), bv_search.as_ref());
        let (bit_depth, preset) = (self.bit_depth, &self.preset);
        let sb_cols = mi_cols.div_ceil(sb / 4);
        let superblocks = sb_cols * mi_rows.div_ceil(sb / 4);
        if !sb_quant.is_empty() && sb_quant.len() != superblocks {
            return Err(Error::Unsupported {
                what: "superblock offsets that do not cover the superblock grid",
            });
        }
        let publishes = self.preset.ref_intra_margin != u8::MAX;
        let frame_size = (mi_cols * 4, mi_rows * 4);
        let reference_intra = plan
            .primary_slot
            .filter(|_| plan.frame_type == FrameType::Inter && publishes)
            .and_then(|slot| self.slot_intra.get(slot as usize)?.clone())
            .filter(|cells| {
                (cells.cols, cells.rows) == (frame_size.0.div_ceil(8), frame_size.1.div_ceil(8))
                    && cells.age < self.preset.ref_intra_refresh
            });
        #[cfg(test)]
        if reference_intra.as_ref().is_some_and(|cells| cells.age > 0) {
            crate::mode_decision::CHAINED_REFERENCES
                .set(crate::mode_decision::CHAINED_REFERENCES.get() + 1);
        }
        let intra = std::sync::Mutex::new(Vec::new());
        let wavefront = self.preset.wavefront && bv_search.is_none() && !self.preset.palette;
        let search = |writer: &mut KeyFrameTileWriter, coder: &mut SourceResidualCoder| {
            self.cancel.check()?;
            let root = writer.next_superblock_root().ok_or(Error::Unsupported {
                what: "a search past its tile",
            })?;
            let index = root.row4() / (sb / 4) * sb_cols + root.col4() / (sb / 4);
            let (lambda, target) = match sb_quant.get(index) {
                Some(&(offset, lambda_log2)) => {
                    let mut target = qindex.saturating_add_signed(i32::from(offset));
                    if wavefront {
                        let step = 1 << self.preset.delta_q_res;
                        let max_q = splot_recon::max_quantizer_index(bit_depth);
                        target =
                            target.clamp(1 + (qindex - 1) % step, max_q - (max_q - qindex) % step);
                    }
                    let sb_qindex = writer
                        .set_superblock_qindex(target)
                        .map_err(coding("delta-q"))?;
                    let applied = 2 * (sb_qindex as i32 - qindex as i32);
                    let clip_slack = (1 << self.preset.delta_q_res) + 1;
                    let per_octave = tpl::QINDEX_PER_IMPORTANCE_OCTAVE as i32;
                    let lambda_log2 = i32::from(lambda_log2).clamp(
                        (applied - clip_slack) * 128 / per_octave,
                        (applied + clip_slack) * 128 / per_octave,
                    );
                    let lambda = lambda.scaled(lambda_log2);
                    coder.set_quantizer(sb_qindex, lambda);
                    (lambda, Some(sb_qindex))
                }
                None => (lambda, None),
            };
            let tree = writer
                .search_superblock(|syntax, root| {
                    two_pass_partition(
                        syntax,
                        coder,
                        (preset, lambda),
                        (motion, &open_loop),
                        (root, reference_intra.as_deref()),
                    )
                })
                .map_err(coding("superblock search"))?;
            if publishes {
                let mut leaves = Vec::new();
                intra_leaves(writer.syntax(), root, &tree, &mut leaves)
                    .map_err(coding("intra cells"))?;
                intra
                    .lock()
                    .map_err(|_| Error::Unsupported {
                        what: "intra cells of a search that panicked",
                    })?
                    .extend(leaves);
            }
            self.progress.superblock(superblocks);
            Ok((SuperblockDecision { tree }, target))
        };
        let new_coder = || {
            let mut coder = SourceResidualCoder::new(padded, qindex, bit_depth, preset, lambda);
            coder.intrabc = bv_search;
            coder.context = context;
            coder.cancel = Some(&self.cancel);
            coder
        };
        let writers = writers.into_iter().enumerate().collect();
        let tiles = map_tiles(self.pool.as_ref(), writers, |(tile, mut writer)| {
            if wavefront && writer.superblock_grid().0 > 1 {
                let row_writer = || {
                    KeyFrameTileWriter::with_start(
                        &self.sequence,
                        &core,
                        &self.references,
                        &initial,
                        tile,
                    )
                    .map_err(coding("tile setup"))
                };
                return wavefront::code_tile(
                    self.pool.as_ref(),
                    writer,
                    row_writer,
                    new_coder,
                    search,
                );
            }
            let mut coder = new_coder();
            let mut decisions = FrameDecisions::default();
            while writer.next_superblock_root().is_some() {
                let decided = search(&mut writer, &mut coder)?;
                decisions
                    .write_superblock(&mut writer, decided, &mut coder)
                    .map_err(coding("superblock"))?;
            }
            CodedTile::finish(writer, decisions).map_err(coding("tile finish"))
        })?;
        let intra = intra.into_inner().map_err(|_| Error::Unsupported {
            what: "intra cells of a search that panicked",
        })?;
        let intra_cells = publishes.then(|| {
            let age = reference_intra
                .as_ref()
                .map_or(0, |cells| cells.age.saturating_add(1));
            Arc::new(IntraCells::new(frame_size, intra, age))
        });
        #[cfg(test)]
        let global_leaves = tiles
            .iter()
            .map(|tile| tile.count_leaves(&tests::is_global))
            .sum();
        let warp_leaves = tiles
            .iter()
            .map(CodedTile::warp_leaves)
            .fold((0, 0), |(w, i), (dw, di)| (w + dw, i + di));
        let FilteredFrame {
            core,
            filters,
            tiles,
            cdfs,
            recon,
            ccso_blocks,
        } = finish_frame(
            &self.sequence,
            &header,
            &self.preset,
            (source, &self.references, self.pool.as_ref()),
            &self.cancel,
            FirstPass {
                core,
                start: initial.clone(),
                tiles,
            },
        )?;
        let tiles: Vec<&[u8]> = tiles.iter().map(Vec::as_slice).collect();
        let temporal_unit = plan.temporal_unit(
            &self.sequence,
            &self.view,
            &self.references,
            (qindex, &filters, tools),
            &core,
            (&tiles, &key_obus),
        )?;
        let coded = CodedFrame {
            temporal_unit,
            recon: visible_planes(&recon)?,
            frame_type: plan.frame_type,
            reference: Some((core, recon, initial, cdfs, ccso_blocks)),
            tip_output: None,
            intra_cells,
            warp: (plan.frame_type == crate::rate_control::FrameType::Inter).then_some(
                crate::warp::WarpUse {
                    layer: plan.layer,
                    enabled: tools.2.6,
                    leaves: warp_leaves,
                },
            ),
            #[cfg(test)]
            global_leaves,
        };
        match tools.2.3 {
            Some(weight)
                if self.params.tools.tip_output
                    && plan.refresh_frame_flags == 0
                    && plan.show_frame =>
            {
                Ok(self
                    .tip_output_frame(source, plan, (qindex, weight, grain_seed), &coded)
                    .ok()
                    .flatten()
                    .unwrap_or(coded))
            }
            _ => Ok(coded),
        }
    }

    /// The frame `plan`, coded normally at `qindex` with TIP weight `weight`
    /// and film grain seed `grain_seed` as `coded`, as a `TIP_FRAME_AS_OUTPUT`
    /// frame with the same seed when its RD cost times 100
    /// is at most `PresetParams::tip_output` times that of `coded`; both
    /// distortions are the squared error of every plane. The TIP frame takes
    /// the candidate of least RD cost that `PresetParams::tip_output_search`
    /// tries. The caller keeps `coded` when this fails.
    fn tip_output_frame(
        &self,
        source: &SourceFrame,
        plan: &GopFrame,
        (qindex, weight, grain_seed): (u32, u8, Option<u16>),
        coded: &CodedFrame,
    ) -> Result<Option<CodedFrame>> {
        let ref_frame_idx = coded
            .reference
            .as_ref()
            .and_then(|(core, ..)| core.inter.as_ref())
            .map(|inter| &inter.ref_frame_idx[..])
            .ok_or(Error::Unsupported {
                what: "a TIP frame without references",
            })?;
        let lambda = Lambda::from_qindex(qindex, self.bit_depth).map_err(coding("lambda"))?;
        let cost = |planes: &[Vec<u16>; 3], bytes: usize| {
            let distortion = planes
                .iter()
                .zip(source.planes())
                .flat_map(|(recon, source)| recon.iter().zip(source))
                .map(|(&a, &b)| u64::from(a.abs_diff(b)).pow(2))
                .sum();
            let rate = Bits((bytes as u64 * 8) << Bits::FRAC_BITS);
            RdCost { distortion, rate }.cost(lambda)
        };
        let trial = |(mv, filter, deblocking): (TipGlobalMv, Option<InterpolationFilter>, bool)| {
            let (core, temporal_unit) = plan.tip_output_unit(
                &self.view,
                &self.references,
                ref_frame_idx,
                (weight, grain_seed),
                (qindex, mv, filter, deblocking),
            )?;
            let (core, frame) = self
                .references
                .tip_output(&core)
                .map_err(coding("TIP output"))?;
            let recon = visible_planes(&frame)?;
            let tip = cost(&recon, temporal_unit.len());
            Ok::<_, Error>((tip, (mv, filter, deblocking), (core, temporal_unit, recon)))
        };
        let zero = TipGlobalMv::default();
        #[cfg(test)]
        if let Some(choice) = TIP_OUTPUT_FORCED.get() {
            let (tip, _, (core, temporal_unit, recon)) = trial(choice)?;
            let normal = cost(&coded.recon, coded.temporal_unit.len());
            return Ok(
                (tip * 100 <= normal * u128::from(self.preset.tip_output)).then(|| CodedFrame {
                    temporal_unit,
                    recon,
                    frame_type: plan.frame_type,
                    reference: None,
                    tip_output: Some(core),
                    warp: None,
                    intra_cells: None,
                    global_leaves: 0,
                }),
            );
        }
        let mut best = trial((zero, None, false))?;
        let keep = |best: &mut (u128, _, _), candidate: (u128, _, _)| {
            if candidate.0 < best.0 {
                *best = candidate;
            }
        };
        if self.preset.tip_output_search >= 1 {
            for filter in [
                InterpolationFilter::Eighttap,
                InterpolationFilter::EighttapSmooth,
            ] {
                keep(&mut best, trial((zero, Some(filter), false))?);
            }
        }
        if self.preset.tip_output_search >= 2 {
            let filter = best.1.1;
            let mut axis = |mv: fn(i32) -> TipGlobalMv| -> Result<i32> {
                let mut pick = (best.0, 0);
                for step in [-2, -1, 1, 2] {
                    let candidate = trial((mv(step), filter, false))?;
                    if candidate.0 < pick.0 {
                        pick = (candidate.0, step);
                    }
                    keep(&mut best, candidate);
                }
                Ok(pick.1)
            };
            let row = axis(|step| TipGlobalMv::new(step, 0))?;
            let col = axis(|step| TipGlobalMv::new(0, step))?;
            if row != 0 && col != 0 {
                keep(
                    &mut best,
                    trial((TipGlobalMv::new(row, col), filter, false))?,
                );
            }
            if self.params.tools.df_sub_pu {
                let (mv, filter, _) = best.1;
                keep(&mut best, trial((mv, filter, true))?);
            }
        }
        let (tip, _, (core, temporal_unit, recon)) = best;
        let normal = cost(&coded.recon, coded.temporal_unit.len());
        Ok(
            (tip * 100 <= normal * u128::from(self.preset.tip_output)).then(|| CodedFrame {
                temporal_unit,
                recon,
                frame_type: plan.frame_type,
                reference: None,
                tip_output: Some(core),
                warp: None,
                intra_cells: None,
                #[cfg(test)]
                global_leaves: 0,
            }),
        )
    }
}

#[cfg(test)]
thread_local! {
    /// The `(TipGlobalMv, TipInterpFilter, TIP deblocking)` every TIP output
    /// frame on this thread codes instead of the searched one.
    pub(crate) static TIP_OUTPUT_FORCED: core::cell::Cell<
        Option<(TipGlobalMv, Option<InterpolationFilter>, bool)>,
    > = const { core::cell::Cell::new(None) };
}

/// The `(frame, reference)` display indices of each list reference of
/// `plan` from the frame's `order_hints`, which count from the last key
/// frame; `None` for an unused reference.
pub(crate) fn reference_pairs<'a>(
    plan: &GopFrame,
    order_hints: &'a [u32],
) -> impl Iterator<Item = Option<(u64, u64)>> + 'a {
    let (display, key) = (
        plan.display_index,
        plan.display_index - u64::from(plan.order_hint),
    );
    order_hints
        .iter()
        .map(move |&hint| (hint != u32::MAX).then(|| (display, key + u64::from(hint))))
}

/// `f` of each item, in order: on `pool` when there is one, else in turn.
/// The error is the first in item order, whatever the thread count.
pub(crate) fn map_tiles<T: Send, R: Send>(
    pool: Option<&WorkerPool>,
    items: Vec<T>,
    f: impl Fn(T) -> Result<R> + Send + Sync,
) -> Result<Vec<R>> {
    match pool {
        Some(pool) => pool
            .install(|| items.into_par_iter().map(f).collect::<Vec<_>>())
            .into_iter()
            .collect(),
        None => items.into_iter().map(f).collect(),
    }
}

pub(crate) fn coding<E: std::error::Error + Send + Sync + 'static>(
    stage: &'static str,
) -> impl FnOnce(E) -> Error {
    move |source| Error::Coding {
        stage,
        source: Box::new(source),
    }
}

/// Source planes extended by edge replication to cover every coded block;
/// the chroma planes of a monochrome source stay empty.
pub(crate) struct PaddedSource {
    planes: [Vec<u16>; 3],
    strides: [usize; 3],
    visible: Vec<PlaneSize>,
}

impl PaddedSource {
    fn new(source: &SourceFrame, format: PixelFormat, width: usize, height: usize) -> Self {
        let sizes = source.plane_sizes();
        let (sub_x, sub_y) = (
            usize::from(format.subsampling_x()),
            usize::from(format.subsampling_y()),
        );
        let strides = [
            width,
            width.div_ceil(1 << sub_x),
            width.div_ceil(1 << sub_x),
        ];
        let planes = core::array::from_fn(|plane| {
            let Some(&size) = sizes.get(plane) else {
                return Vec::new();
            };
            let (w, h) = if plane == 0 {
                (width, height)
            } else {
                (strides[plane], height.div_ceil(1 << sub_y))
            };
            let src = &source.planes()[plane];
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                let row = &src[y.min(size.height() - 1) * size.width()..][..size.width()];
                out.extend_from_slice(row);
                out.resize(out.len() + w - size.width(), row[size.width() - 1]);
            }
            out
        });
        Self {
            planes,
            strides,
            visible: sizes.to_vec(),
        }
    }

    /// The luma plane, its stride, and its padded `(width, height)`.
    pub(crate) fn luma(&self) -> (&[u16], usize, usize, usize) {
        let stride = self.strides[0];
        (
            &self.planes[0],
            stride,
            stride,
            self.planes[0].len() / stride,
        )
    }

    /// The source samples at the top-left of `tx` and their stride.
    fn at(&self, tx: &TransformBlock) -> (&[u16], usize) {
        let stride = self.strides[tx.plane];
        (&self.planes[tx.plane][tx.y * stride + tx.x..], stride)
    }

    /// SATD between the source and the row-major `prediction` of `tx`.
    pub(crate) fn satd(
        &self,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> core::result::Result<u64, TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (source, stride) = self.at(tx);
        Ok(kernels().satd(
            source,
            stride,
            prediction,
            1 << log2_w,
            BlockDims::new(1 << log2_w, 1 << log2_h).map_err(kernel)?,
        ))
    }

    /// Squared error between the source and the row-major `recon` of `tx`
    /// over the visible samples.
    pub(crate) fn sse(
        &self,
        tx: &TransformBlock,
        recon: &[u16],
    ) -> core::result::Result<u64, TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let size = self
            .visible
            .get(tx.plane)
            .ok_or(TileWriterError::Unsupported("plane index"))?;
        let visible_w = w.min(size.width().saturating_sub(tx.x));
        let visible_h = h.min(size.height().saturating_sub(tx.y));
        let (source, stride) = self.at(tx);
        if visible_w == w && visible_h == h {
            return Ok(kernels().sse(
                source,
                stride,
                recon,
                w,
                BlockDims::new(w, h).map_err(kernel)?,
            ));
        }
        Ok(source
            .chunks(stride)
            .zip(recon.chunks(w))
            .take(visible_h)
            .flat_map(|(src, rec)| src[..visible_w].iter().zip(&rec[..visible_w]))
            .map(|(&s, &r)| u64::from(s.abs_diff(r)).pow(2))
            .sum())
    }

    /// The AC-energy loss of the row-major `recon` of the fully visible block
    /// `tx` in squared-error units at `psy_rd` percent: the AC energy is the
    /// SATD against zero less the SAD (the Hadamard DC terms). White noise of
    /// deviation `a` over `n` samples has about `6.4 * n * a` on 8x8 tiles
    /// and half that on the 4x4 tiles of a block with a side under 8, which
    /// the loss doubles, so 100% charges a flat recon of noise its own
    /// squared error at every block shape. `0` for a block that crosses the
    /// frame edge.
    pub(crate) fn ac_loss(
        &self,
        tx: &TransformBlock,
        recon: &[u16],
        psy_rd: u8,
    ) -> core::result::Result<u64, TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let size = self
            .visible
            .get(tx.plane)
            .ok_or(TileWriterError::Unsupported("plane index"))?;
        if tx.x + w > size.width() || tx.y + h > size.height() {
            return Ok(0);
        }
        let (source, stride) = self.at(tx);
        let dims = BlockDims::new(w, h).map_err(kernel)?;
        Ok(ac_energy_loss((source, stride), recon, dims, psy_rd))
    }
}

/// [`PaddedSource::ac_loss`] of the strided `source` block and the row-major
/// `recon` of `dims` (sides at most 64).
pub(crate) fn ac_energy_loss(
    (source, stride): (&[u16], usize),
    recon: &[u16],
    dims: BlockDims,
    psy_rd: u8,
) -> u64 {
    const ZERO: [u16; 64 * 64] = [0; 64 * 64];
    let w = dims.width();
    let ac = |block: &[u16], stride: usize| {
        let satd = kernels().satd(block, stride, &ZERO, w, dims);
        satd.saturating_sub(u64::from(kernels().sad(block, stride, &ZERO, w, dims)))
    };
    let tile_gain = if w.min(dims.height()) < 8 { 2 } else { 1 };
    let loss = ac(source, stride).abs_diff(ac(recon, w)) * tile_gain;
    u64::from(psy_rd) * loss * loss / (41 * (w * dims.height()) as u64 * 100)
}

/// Whether every in-frame leaf of `partition` of `node` (`node` itself for
/// `NONE`) may code an intra or an inter block.
pub(crate) fn codable_children(
    syntax: &IntraTileSyntax,
    node: PartitionNode,
    partition: Partition,
) -> core::result::Result<bool, TileWriterError> {
    let leaves = if partition == Partition::None {
        vec![node]
    } else {
        syntax.children(node, partition)?
    };
    for child in leaves {
        if syntax.in_frame(child) {
            let leaf = syntax.leaf(child)?;
            if !syntax.intra_allowed(&leaf) && !syntax.inter_allowed(&leaf) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Transforms and quantizes the source residual of each transform block, by
/// RDOQ when the preset asks for it, else by a deadzone quantizer, and picks
/// each luma transform type by rate and distortion.
pub(crate) struct SourceResidualCoder<'a> {
    pub(crate) source: &'a PaddedSource,
    qindex: u32,
    pub(crate) bit_depth: BitDepth,
    preset: &'a PresetParams,
    lambda: Lambda,
    /// A deadzone candidate trial, used when `full_trials` is 0 or to rank
    /// (MDS1): quantize with the deadzone and code `DCT_DCT`.
    pub(crate) candidate: bool,
    /// A mode trial of the search: a block that hides the other parity moves
    /// its scan index 0 level instead of pricing every move.
    pub(crate) trial: bool,
    /// Price TCQ blocks as plain levels ([`IntraTileSyntax::set_scalar_trial`]).
    pub(crate) scalar: bool,
    /// Residual, coefficient, and dequantized buffers reused across blocks.
    scratch: [Vec<i32>; 3],
    /// The U and V coefficients of a CCTX trial.
    pair: [Vec<i32>; 2],
    /// The row costs and the trellis buffers reused across blocks.
    rates: (RowCaches, TcqScratch),
    /// The reconstruction buffers of the transform-type and CCTX costs.
    recon: tx_choice::ReconScratch,
    pub(crate) palette: PaletteSearch,
    /// The block hashes of the frame source, in a frame with IntraBC.
    pub(crate) intrabc: Option<&'a BvSearch>,
    /// The context of the frame's [`crate::preset::Levels`] fields.
    pub(crate) context: PresetContext,
    /// Stops the block search of a cancelled encoder.
    pub(crate) cancel: Option<&'a crate::cancel::CancelToken>,
}

impl<'a> SourceResidualCoder<'a> {
    /// Quantizes the following blocks at `qindex` and prices them with `lambda`.
    pub(crate) const fn set_quantizer(&mut self, qindex: u32, lambda: Lambda) {
        self.qindex = qindex;
        self.lambda = lambda;
    }

    pub(crate) fn new(
        source: &'a PaddedSource,
        qindex: u32,
        bit_depth: BitDepth,
        preset: &'a PresetParams,
        lambda: Lambda,
    ) -> Self {
        Self {
            source,
            qindex,
            bit_depth,
            preset,
            lambda,
            candidate: false,
            trial: false,
            scalar: false,
            scratch: Default::default(),
            pair: Default::default(),
            rates: Default::default(),
            recon: tx_choice::ReconScratch::default(),
            palette: PaletteSearch::default(),
            intrabc: None,
            context: PresetContext::default(),
            cancel: None,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// The model rank of each exact type-search winner (tests only).
    pub(crate) static MODEL_RANKS: core::cell::RefCell<Vec<(usize, u128, u128)>> =
        const { core::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// Chroma pairs outside mode trials that code a CCTX rotation other than
    /// the one that leaves V the least energy (tests only).
    pub(crate) static CCTX_LATER_ROTATIONS: core::cell::Cell<usize> =
        const { core::cell::Cell::new(0) };
    /// Luma blocks outside mode trials whose RD-chosen IST kernel is
    /// not the one that keeps the most energy (tests only).
    pub(crate) static IST_RD_OTHER: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

impl ResidualCoder for SourceResidualCoder<'_> {
    fn palette_map(
        &mut self,
        leaf: &LeafBlock,
        palette: &Palette,
    ) -> core::result::Result<&[u8], TileWriterError> {
        self.palette.map(self.source, leaf, palette)
    }

    /// The offered type of the preset's leading `tx_types` with the lowest
    /// `D + lambda * R`, with the exact coefficient and type rate.
    fn code_luma(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        tx_types: &[TxType],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let count = if self.candidate {
            1
        } else {
            tx_choice::tx_type_candidates(self.preset).min(tx_types.len())
        };
        let inter = syntax.leaf_is_inter();
        let ist_sets = match (self.candidate, inter) {
            (true, _) => 0,
            (false, true) => u8::from(self.preset.inter_ist && !self.trial),
            (false, false) => self.preset.ist_sets,
        };
        let (ist_probe, ist_rd) = if inter {
            (4, self.preset.inter_ist_rd)
        } else {
            (2, self.preset.intra_ist_rd)
        };
        let mut best: Option<(u128, TransformDecision)> = None;
        let keep = match self.preset.tx_type_model_keep {
            0 => count,
            keep => usize::from(keep).min(count),
        };
        let order = if keep < count {
            self.model_order(syntax, tx, prediction, &tx_types[..count], keep)?
        } else {
            tx_types[..count].to_vec()
        };
        let scalar_types = (keep > 1 || ist_sets > 0) && !self.preset.tcq_type_trellis;
        let scalar = self.scalar;
        self.scalar |= scalar_types;
        #[cfg(test)]
        let mut exact = Vec::new();
        for (tried, &tx_type) in order.iter().enumerate() {
            if tried >= keep && best.is_some() {
                break;
            }
            let tx = TransformBlock { tx_type, ..*tx };
            let decision = self.code(syntax, &tx, prediction)?;
            let ist = ist_sets > 0
                && eob(&tx, &decision.levels)? > 1
                && syntax.ist_coded(&tx, ist_probe)?;
            if count == 1 && !ist {
                self.scalar = scalar;
                if scalar_types {
                    return self.code(syntax, &tx, prediction);
                }
                return Ok(decision);
            }
            let mut trials = vec![decision];
            if ist && ist_rd {
                for ist in Self::ist_kernels(syntax, &tx, ist_sets)? {
                    trials.push(self.code(syntax, &TransformBlock { ist, ..tx }, prediction)?);
                }
            } else if ist {
                let ist = self.ist_choice(syntax, &tx, prediction, ist_sets)?;
                if ist != Ist::OFF {
                    let tx = TransformBlock { ist, ..tx };
                    trials.push(self.code(syntax, &tx, prediction)?);
                }
            }
            for decision in trials {
                let coded = TransformBlock {
                    ist: decision.ist,
                    ..tx
                };
                let cost = self.luma_cost(syntax, &coded, prediction, &decision.levels)?;
                #[cfg(test)]
                exact.extend(cost.map(|cost| (tx_type, cost)));
                if let Some(cost) = cost
                    && best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost)
                {
                    best = Some((cost, decision));
                }
            }
        }
        self.scalar = scalar;
        let (_, decision) = best.ok_or(TileWriterError::Shape("no luma transform type"))?;
        #[cfg(test)]
        if !self.trial && ist_rd && decision.ist != Ist::OFF {
            let plain = TransformBlock {
                tx_type: decision.tx_type,
                ..*tx
            };
            if self.ist_choice(syntax, &plain, prediction, ist_sets)? != decision.ist {
                IST_RD_OTHER.set(IST_RD_OTHER.get() + 1);
            }
        }
        #[cfg(test)]
        if keep == count && count > 1 {
            let ranked = self.model_ranking(syntax, tx, prediction, &tx_types[..count])?;
            let rank = ranked
                .iter()
                .position(|&(_, index)| tx_types[index] == decision.tx_type);
            let best_of = |top: usize| {
                exact
                    .iter()
                    .filter(|(t, _)| ranked[..top].iter().any(|&(_, i)| tx_types[i] == *t))
                    .map(|&(_, cost)| cost)
                    .min()
            };
            if let (Some(all), Some(two)) = (best_of(count), best_of(2)) {
                MODEL_RANKS.with_borrow_mut(|ranks| ranks.push((rank.unwrap_or(count), all, two)));
            }
        }
        if scalar_types {
            return self.code(
                syntax,
                &TransformBlock {
                    tx_type: decision.tx_type,
                    ist: decision.ist,
                    ..*tx
                },
                prediction,
            );
        }
        Ok(decision)
    }

    fn code(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let mut scratch = core::mem::take(&mut self.scratch);
        let [residual, coeffs, dequant] = &mut scratch;
        syntax.set_scalar_trial(self.candidate || self.trial || self.scalar);
        let use_tcq = syntax.uses_tcq(tx);
        let ddt = syntax.uses_ddt();
        let qm = syntax.quant_matrices();
        let (params, forward) =
            self.forward(tx, (use_tcq, ddt, qm), prediction, residual, coeffs)?;
        if let Some(secondary) = syntax.secondary_transform(tx)? {
            kernels()
                .forward_ist(&secondary_transform_layout(&secondary)?, coeffs)
                .map_err(kernel)?;
        }
        let levels = self.quantize(syntax, tx, (&params, &forward), coeffs, dequant);
        self.scratch = scratch;
        Ok(TransformDecision {
            tx_type: tx.tx_type,
            ist: tx.ist,
            cctx: 0,
            levels: levels?,
        })
    }

    /// Both blocks coded plainly, and in a leaf's final trial with the
    /// rotation that leaves V the least energy when it leaves less than either
    /// plain plane; the pair with the lower `D + lambda * R` wins.
    fn code_chroma_pair(
        &mut self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        predictions: [&[u16]; 2],
    ) -> core::result::Result<[TransformDecision; 2], TileWriterError> {
        let plain = [
            self.code(syntax, u, predictions[0])?,
            self.code(syntax, v, predictions[1])?,
        ];
        if self.candidate || !self.preset.cctx || u.tx_type != v.tx_type || !syntax.cctx_coded(u, 2)
        {
            return Ok(plain);
        }
        let mut scratch = core::mem::take(&mut self.scratch);
        let mut pair = core::mem::take(&mut self.pair);
        let rotated = self.rotated_pairs(syntax, [u, v], predictions, &mut scratch, &mut pair);
        (self.scratch, self.pair) = (scratch, pair);
        let mut best: Option<(u128, [TransformDecision; 2])> = None;
        let rotated = rotated?;
        #[cfg(test)]
        let least_energy = rotated.first().map(|[_, _, cctx]| cctx[0] as u8);
        for [u_levels, v_levels, cctx] in rotated {
            let coded = TransformBlock {
                cctx: cctx[0] as u8,
                ..*u
            };
            if !syntax.cctx_coded(&coded, eob(&coded, &u_levels)?) {
                continue;
            }
            if best.is_none() {
                let plain_cost = self.pair_cost(
                    syntax,
                    [u, v],
                    predictions,
                    [&plain[0].levels, &plain[1].levels],
                    None,
                )?;
                best = Some((plain_cost, plain.clone()));
            }
            let cost = self.pair_cost(
                syntax,
                [&coded, v],
                predictions,
                [&u_levels, &v_levels],
                Some(()),
            )?;
            if best
                .as_ref()
                .is_some_and(|(best_cost, _)| cost < *best_cost)
            {
                best = Some((
                    cost,
                    [
                        TransformDecision {
                            cctx: coded.cctx,
                            levels: u_levels,
                            ..plain[0].clone()
                        },
                        TransformDecision {
                            levels: v_levels,
                            ..plain[1].clone()
                        },
                    ],
                ));
            }
        }
        #[cfg(test)]
        if !self.trial
            && best
                .as_ref()
                .is_some_and(|(_, [u, _])| u.cctx != 0 && Some(u.cctx) != least_energy)
        {
            CCTX_LATER_ROTATIONS.set(CCTX_LATER_ROTATIONS.get() + 1);
        }
        Ok(best.map_or(plain, |(_, pair)| pair))
    }
}

impl SourceResidualCoder<'_> {
    /// The levels of `u` and `v` rotated by each of the preset's
    /// `cctx_candidates` CCTX rotations that leave V the least energy, with
    /// their `cctx_type` (as `[u, v, [cctx]]`), least energy first. One
    /// candidate is tried only when it leaves less than the smaller plain
    /// plane.
    fn rotated_pairs(
        &mut self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        [u_prediction, v_prediction]: [&[u16]; 2],
        [residual, plain_u, dequant]: &mut [Vec<i32>; 3],
        [u_coeffs, v_coeffs]: &mut [Vec<i32>; 2],
    ) -> core::result::Result<Vec<[Vec<i32>; 3]>, TileWriterError> {
        let ddt = (false, syntax.uses_ddt(), syntax.quant_matrices());
        let (u_params, u_forward) = self.forward(u, ddt, u_prediction, residual, u_coeffs)?;
        let (v_params, v_forward) = self.forward(v, ddt, v_prediction, residual, v_coeffs)?;
        let [uu, vv, uv] = kernels().cctx_stats(u_coeffs, v_coeffs);
        let energy = |[c, s]: [i32; 2]| {
            let (c, s) = (i128::from(c), i128::from(s));
            s * s * i128::from(uu) + c * c * i128::from(vv) - 2 * c * s * i128::from(uv)
        };
        let mut ranked: Vec<(i128, usize)> = CCTX_ROTATIONS
            .iter()
            .map(|&rotation| energy(rotation))
            .enumerate()
            .map(|(index, e)| (e, index))
            .collect();
        ranked.sort_unstable();
        let keep = usize::from(self.preset.cctx_candidates.max(1));
        if keep == 1
            && ranked
                .first()
                .is_none_or(|&(e, _)| e >= i128::from(uu.min(vv)) << 16)
        {
            return Ok(Vec::new());
        }
        let plain_v = (keep > 1).then(|| {
            plain_u.clone_from(u_coeffs);
            v_coeffs.clone()
        });
        let mut out = Vec::with_capacity(keep);
        for (tried, &(_, index)) in ranked.iter().take(keep).enumerate() {
            if let Some(plain_v) = plain_v.as_ref().filter(|_| tried > 0) {
                u_coeffs.clone_from(plain_u);
                v_coeffs.clone_from(plain_v);
            }
            kernels().cctx_rotate(u_coeffs, v_coeffs, CCTX_ROTATIONS[index]);
            let cctx =
                i32::try_from(index + 1).map_err(|_| TileWriterError::Unsupported("CCTX type"))?;
            let coded = TransformBlock {
                cctx: cctx as u8,
                ..*u
            };
            let u_levels =
                self.quantize(syntax, &coded, (&u_params, &u_forward), u_coeffs, dequant)?;
            let v_levels = self.quantize(syntax, v, (&v_params, &v_forward), v_coeffs, dequant)?;
            out.push([u_levels, v_levels, vec![cctx]]);
        }
        Ok(out)
    }

    /// `D + lambda * R` of chroma blocks `u` and `v` coded as `levels`, with
    /// the decoder's cross-chroma reconstruction when `cctx` is `Some`.
    fn pair_cost(
        &mut self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        predictions: [&[u16]; 2],
        levels: [&[i32]; 2],
        cctx: Option<()>,
    ) -> core::result::Result<u128, TileWriterError> {
        let distortion = if let Some(()) = cctx {
            let recon = syntax.cctx_reconstruction([u, v], predictions, levels)?;
            self.source.sse(u, &recon[0])? + self.source.sse(v, &recon[1])?
        } else {
            let mut distortion = 0;
            for (index, tx) in [u, v].into_iter().enumerate() {
                let quant = (self.qindex, self.bit_depth);
                let (prediction, levels) = (predictions[index], levels[index]);
                tx_choice::reconstruct(syntax, prediction, tx, levels, quant, &mut self.recon)?;
                distortion += self.source.sse(tx, &self.recon.recon)?;
            }
            distortion
        };
        let u_eob = eob(u, levels[0])?;
        let mut sink = RateSink::frozen();
        let rate = block_rate(syntax, &mut sink, u, levels[0], false)?;
        let rate = rate + block_rate(syntax, &mut sink, v, levels[1], u_eob != 0)?;
        let cost = RdCost { distortion, rate };
        Ok(cost.cost(self.lambda))
    }

    /// The levels of the forward coefficients `coeffs` of `tx`: TCQ, RDOQ or
    /// the deadzone quantizer as the preset and the trial ask, with parity
    /// hiding.
    fn quantize(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        (params, forward): (&DequantBlockParams, &ForwardTransform2d),
        coeffs: &[i32],
        dequant: &mut Vec<i32>,
    ) -> core::result::Result<Vec<i32>, TileWriterError> {
        let len = coeffs.len();
        dequant.resize(len, 0);
        let use_tcq = syntax.uses_tcq(tx);
        let mut levels = vec![0; len];
        let (block, lambda_q8) = self.rdoq_block(tx, params, forward)?;
        let parity = syntax.parity_hiding(tx);
        let (rows, trellis) = &mut self.rates;
        let fsc = syntax.uses_fsc(tx);
        let mut rates = CdfCoeffRates::new(syntax, tx, rows);
        let every_type = self.preset.rdoq_every_type && !self.trial;
        let rdoq_type = every_type || (tx.tx_type as u8) < TxType::Idtx as u8;
        if use_tcq {
            let lambda_q8 = lambda_q8 * TCQ_LAMBDA_PERCENT / 100;
            tcq(
                kernels(),
                &block,
                coeffs,
                (&mut rates, trellis),
                lambda_q8,
                &mut levels,
            )
            .map_err(kernel)?;
        } else if self.preset.rdoq && !self.candidate && rdoq_type && !fsc {
            rdoq(
                kernels(),
                &block,
                coeffs,
                &mut rates,
                lambda_q8,
                &mut levels,
                dequant,
            )
            .map_err(kernel)?;
        } else {
            let quantizer = block.quantizer(DEADZONE_ROUNDING_Q8).map_err(kernel)?;
            kernels()
                .quantize(&quantizer, coeffs, &mut levels, dequant)
                .map_err(kernel)?;
        }
        let rates = (!self.candidate && !self.trial).then_some(&mut rates);
        hide(
            parity,
            (&block, coeffs, lambda_q8),
            rates,
            (&mut levels, dequant),
        )?;
        Ok(levels)
    }

    /// The RDOQ view of `tx` and its coefficient lambda.
    fn rdoq_block(
        &self,
        tx: &TransformBlock,
        params: &DequantBlockParams,
        forward: &ForwardTransform2d,
    ) -> core::result::Result<(RdoqBlock, u64), TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let block = RdoqBlock {
            dc_q2: params.dc_quant,
            ac_q2: params.ac_quant,
            dq_denom: params.dq_denom,
            bit_depth: self.bit_depth,
            log2_width: forward.coeff_width().ilog2(),
            log2_height: forward.coeff_height().ilog2(),
            plane: tx.plane,
            tx_type: tx.tx_type,
            qm: match &params.qm {
                Some(qm) => Some(
                    qm.built_in_weights(params.tx_width * params.tx_height)
                        .ok_or(TileWriterError::Unsupported("user quantizer matrices"))?,
                ),
                None => None,
            },
        };
        let lambda_q8 = self
            .lambda
            .coefficient_q8(Quantizer::dq_denom_without_tcq(log2_w, log2_h));
        Ok((block, lambda_q8))
    }

    /// The source residual of `tx` over `prediction` and its primary forward
    /// transform into `coeffs`, with the block's dequantization parameters.
    fn forward(
        &self,
        tx: &TransformBlock,
        (use_tcq, use_ddt, qm): (bool, bool, Option<QmFrameLevels>),
        prediction: &[u16],
        residual: &mut Vec<i32>,
        coeffs: &mut Vec<i32>,
    ) -> core::result::Result<(DequantBlockParams, ForwardTransform2d), TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let (source, stride) = self.source.at(tx);
        let dims = BlockDims::new(w, h).map_err(kernel)?;
        residual.resize(w * h, 0);
        kernels().residual(source, stride, prediction, w, dims, residual);
        let (params, transform) = recon_params(
            self.qindex,
            plane_id(tx.plane)?,
            (log2_w, log2_h),
            (tx.tx_type, use_tcq, use_ddt),
            (self.bit_depth, qm),
        )?;
        let forward = ForwardTransform2d::new(
            log2_w,
            log2_h,
            transform.row_type,
            transform.col_type,
            self.bit_depth,
        )
        .map_err(kernel)?;
        coeffs.resize(forward.coeff_width() * forward.coeff_height(), 0);
        kernels()
            .forward_transform_2d(&forward, residual, coeffs)
            .map_err(kernel)?;
        Ok((params, forward))
    }

    /// The offered luma `types` of `tx`: the `keep` with the lowest
    /// [`Self::model_cost`] in symbol order, then the others in symbol order.
    fn model_order(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        types: &[TxType],
        keep: usize,
    ) -> core::result::Result<Vec<TxType>, TileWriterError> {
        let mut ranked = self.model_ranking(syntax, tx, prediction, types)?;
        let (best, rest) = ranked.split_at_mut(keep);
        best.sort_unstable_by_key(|&(_, index)| index);
        rest.sort_unstable_by_key(|&(_, index)| index);
        Ok(ranked.iter().map(|&(_, index)| types[index]).collect())
    }

    /// `(model cost, index)` of each of `types`, lowest cost first (ties in
    /// symbol order).
    fn model_ranking(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        types: &[TxType],
    ) -> core::result::Result<Vec<(u128, usize)>, TileWriterError> {
        let mut scratch = core::mem::take(&mut self.scratch);
        let mut levels = Vec::new();
        syntax.set_scalar_trial(true);
        let mut ranked = Vec::with_capacity(types.len());
        let mut found = Ok(());
        for (index, &tx_type) in types.iter().enumerate() {
            let tx = TransformBlock { tx_type, ..*tx };
            match self.model_cost(syntax, &tx, prediction, &mut scratch, &mut levels) {
                Ok(cost) => ranked.push((cost.unwrap_or(u128::MAX), index)),
                Err(error) => {
                    found = Err(error);
                    break;
                }
            }
        }
        syntax.set_scalar_trial(self.candidate || self.trial || self.scalar);
        self.scratch = scratch;
        found?;
        ranked.sort_unstable();
        Ok(ranked)
    }

    /// The model cost of luma block `tx` in RDOQ units: the coefficient-domain
    /// squared error of its plain deadzone levels plus lambda times their
    /// frozen rate; `None` when the decoder would infer another type for those
    /// levels or cannot code them. The caller sets the scalar trial.
    fn model_cost(
        &self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        [residual, coeffs, dequant]: &mut [Vec<i32>; 3],
        levels: &mut Vec<i32>,
    ) -> core::result::Result<Option<u128>, TileWriterError> {
        let (params, forward) = self.forward(
            tx,
            (false, syntax.uses_ddt(), syntax.quant_matrices()),
            prediction,
            residual,
            coeffs,
        )?;
        let (block, lambda_q8) = self.rdoq_block(tx, &params, &forward)?;
        let quantizer = block.quantizer(DEADZONE_ROUNDING_Q8).map_err(kernel)?;
        levels.resize(coeffs.len(), 0);
        dequant.resize(coeffs.len(), 0);
        let (_, error) = kernels()
            .quantize_error(&quantizer, coeffs, levels, dequant)
            .map_err(kernel)?;
        let moved = hide(
            syntax.parity_hiding(tx),
            (&block, coeffs, lambda_q8),
            None::<&mut CdfCoeffRates>,
            (levels, dequant),
        )?;
        let eob = eob(tx, levels)?;
        if eob > 0
            && syntax
                .inferred_luma_tx_type(tx, eob)?
                .is_some_and(|t| t != tx.tx_type)
        {
            return Ok(None);
        }
        let rate = block_rate(syntax, &mut RateSink::frozen(), tx, levels, false)?;
        if rate == Bits::IMPOSSIBLE {
            return Ok(None);
        }
        let error = u128::try_from((i128::from(error) << 16) + moved).unwrap_or(0);
        Ok(Some(error + u128::from(lambda_q8) * u128::from(rate.0)))
    }

    /// Every secondary transform of the first `sets` sets that `tx` can code.
    fn ist_kernels(
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        sets: u8,
    ) -> core::result::Result<Vec<Ist>, TileWriterError> {
        let mut out = Vec::new();
        for set in 0..sets.min(syntax.ist_sets(tx)?) {
            for sec_tx_type in 1..=3 {
                let ist = Ist { sec_tx_type, set };
                if syntax
                    .secondary_transform(&TransformBlock { ist, ..*tx })?
                    .is_some()
                {
                    out.push(ist);
                }
            }
        }
        Ok(out)
    }

    /// The secondary transform of the first `sets` sets whose outputs keep the
    /// most energy of the primary coefficients of `tx`.
    fn ist_choice(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        sets: u8,
    ) -> core::result::Result<Ist, TileWriterError> {
        let sets = sets.min(syntax.ist_sets(tx)?);
        let mut scratch = core::mem::take(&mut self.scratch);
        let [residual, coeffs, work] = &mut scratch;
        let found = self.forward(
            tx,
            (
                syntax.uses_tcq(tx),
                syntax.uses_ddt(),
                syntax.quant_matrices(),
            ),
            prediction,
            residual,
            coeffs,
        );
        let mut best = (0, Ist::OFF);
        if found.is_ok() {
            for set in 0..sets {
                for sec_tx_type in 1..=3 {
                    let ist = Ist { sec_tx_type, set };
                    let Some(params) =
                        syntax.secondary_transform(&TransformBlock { ist, ..*tx })?
                    else {
                        continue;
                    };
                    let layout = secondary_transform_layout(&params)?;
                    work.clone_from(coeffs);
                    kernels().forward_ist(&layout, work).map_err(kernel)?;
                    let energy: u64 = layout.inputs[..layout.n]
                        .iter()
                        .map(|&pos| u64::from(work[usize::from(pos)].unsigned_abs()).pow(2))
                        .sum();
                    if energy > best.0 {
                        best = (energy, ist);
                    }
                }
            }
        }
        self.scratch = scratch;
        found?;
        Ok(best.1)
    }

    /// `D + lambda * R` of luma block `tx` coded as `levels`, from its exact
    /// reconstruction and the frozen CDF rows; `None` when the decoder would
    /// infer another type for the levels.
    fn luma_cost(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        levels: &[i32],
    ) -> core::result::Result<Option<u128>, TileWriterError> {
        let eob = eob(tx, levels)?;
        if eob > 0
            && syntax
                .inferred_luma_tx_type(tx, eob)?
                .is_some_and(|t| t != tx.tx_type)
            || tx.ist != Ist::OFF && !syntax.ist_coded(tx, eob)?
        {
            return Ok(None);
        }
        tx_choice::reconstruct(
            syntax,
            prediction,
            tx,
            levels,
            (self.qindex, self.bit_depth),
            &mut self.recon,
        )?;
        let cost = RdCost {
            distortion: self.source.sse(tx, &self.recon.recon)?,
            rate: block_rate(syntax, &mut RateSink::frozen(), tx, levels, false)?,
        };
        Ok(Some(cost.cost(self.lambda)))
    }
}

/// Moves one level of `levels` (and its `dequant`) so that the block keeps
/// the § 5.20.7.27 hidden parity `parity`; `rates` ranks the moves, else only
/// the scan index 0 level moves. Returns the `J` change of the move.
fn hide<R: CoeffRates>(
    parity: Option<ParityHiding>,
    (block, coeffs, lambda_q8): (&RdoqBlock, &[i32], u64),
    rates: Option<&mut R>,
    (levels, dequant): (&mut [i32], &mut [i32]),
) -> core::result::Result<i128, TileWriterError> {
    let Some(parity) = parity else {
        return Ok(0);
    };
    let holds = |levels: &[i32]| parity.holds(levels).map_err(TileWriterError::from);
    if holds(levels)? {
        return Ok(0);
    }
    let moves = parity_moves(block, coeffs, rates, lambda_q8, levels).map_err(kernel)?;
    hide_parity(holds, moves, (levels, dequant))
}

fn plane_id(plane: usize) -> core::result::Result<PlaneId, TileWriterError> {
    [PlaneId::Y, PlaneId::U, PlaneId::V]
        .get(plane)
        .copied()
        .ok_or(TileWriterError::Unsupported("plane index"))
}

fn kernel(error: crate::kernels::KernelError) -> TileWriterError {
    TileWriterError::Kernel(error)
}

/// The visible samples of each plane, packed like [`SourceFrame`] planes.
pub(crate) fn visible_planes(recon: &DecodedFrame<u16>) -> Result<[Vec<u16>; 3]> {
    let view = recon.as_frame_ref();
    let mut planes: [Vec<u16>; 3] = Default::default();
    let ids = [PlaneId::Y, PlaneId::U, PlaneId::V];
    let count = if recon.info().pixel_format().is_monochrome() {
        1
    } else {
        3
    };
    for (out, id) in planes.iter_mut().zip(ids).take(count) {
        let plane = view.plane(id).ok_or(Error::ReconSizeMismatch)?;
        for row in plane.visible_rows() {
            out.extend_from_slice(row);
        }
    }
    Ok(planes)
}

#[cfg(test)]
#[path = "intra_frame_tests.rs"]
mod tests;
