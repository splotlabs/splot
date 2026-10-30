// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame coding: the RD partition and mode search of each superblock, with
//! intra and single-reference inter leaves, residual coding, and the temporal
//! unit of a key or inter frame.

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, FrameHeaderCore,
    GrainScaling, UniformTiles, build_encoder_sequence_header, encode_film_grain_obu,
    encoder_film_grain_model,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    CCTX_ROTATIONS, EncoderReferences, IntraTileSyntax, Ist, LeafBlock, Palette, Partition,
    PartitionNode, TransformBlock, TxType, recon_params, tile_count, tx_log2,
};
use splot_parallel::WorkerPool;
use splot_parallel::prelude::*;
use splot_recon::{
    BitDepth, DecodedFrame, DequantBlockParams, PixelFormat, PlaneId, PlaneSize,
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
use crate::kernels::rdoq::{RdoqBlock, hide_parity, parity_moves, rdoq};
use crate::kernels::tcq::{TcqScratch, tcq};
use crate::kernels::{BlockDims, ForwardTransform2d, Quantizer, kernels};
use crate::loop_filter::{CodedTile, FilteredFrame, FirstPass, FrameDecisions, finish_frame};
use crate::mode_decision::TrialCoder;
use crate::motion_search::MotionSearch;
use crate::palette::PaletteSearch;
use crate::preset::{PresetParams, TcqFrames};
use crate::rate_control::FrameType;
use crate::rd::coeff_rate::{CdfCoeffRates, RateSink, RowCaches, block_rate};
use crate::rd::partition::search_partition;
use crate::rd::{Bits, Lambda, RdCost};
use crate::screen::is_screen_content;
use crate::tile_writer::coeffs::eob;
use crate::tile_writer::{
    KeyFrameTileWriter, ResidualCoder, SuperblockDecision, TileWriterError, TransformDecision,
};
use crate::tpl;
use crate::tx_choice;

/// The TCQ trellis multiplier in percent of the RDOQ one: measured best near
/// 60 on park_joy and paris (100 loses 0.7-1.1% BD-rate), while RDOQ itself
/// loses at 60.
const TCQ_LAMBDA_PERCENT: u64 = 60;
/// Quantizer deadzone in Q8 (`128` rounds to nearest).
const DEADZONE_ROUNDING_Q8: u32 = 96;
/// `avg_cdf_type` of a stream that carries CDFs: `true` averages the saved
/// CDFs across tiles, `false` blends the primary reference's CDFs with a
/// second reference's.
const AVG_CDF_ACROSS_TILES: bool = false;
/// `constrain_drl_reorder` of a stream that reorders the DRL: `true`
/// reorders under `DRL_REORDER_CONSTRAINT`, `false` always.
const DRL_REORDER_CONSTRAINT: bool = true;

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
    tiles: UniformTiles,
    /// Codes the frames of a wave and the tiles of a frame; `None` with one thread.
    pool: Option<WorkerPool>,
    /// The global motion models every inter frame codes, set by tests.
    #[cfg(test)]
    global_motion: [Option<[i32; 6]>; 7],
    /// The global motion models of the next frames to code.
    global_models: crate::global_motion::GlobalModels,
    /// The block importances of the next frames to code.
    importance: tpl::BlockImportance,
    /// Show-existing frames output so far, which seed their film grain.
    shown_existing: u64,
    /// The last committed key frame coded a film grain model, which the
    /// output frames after it apply.
    grain_active: bool,
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
                opfl_refine: preset.opfl,
                refinemv: preset.refinemv,
                tip_refinemv: preset.tip > 0 && preset.mv_prediction && preset.tip_refinemv,
                cctx: preset.cctx,
                mhccp: preset.mhccp,
                fsc: preset.fsc,
                screen_content: preset.palette || preset.integer_mv,
                dip: preset.dip,
                sdp: preset.sdp,
                ext_partitions: preset.ext_partitions > 0,
                uneven_4way_partitions: preset.ext_partitions > 1,
                avg_cdf: preset.cdf_carry.then_some(AVG_CDF_ACROSS_TILES),
                ref_frame_mvs: preset.mv_prediction,
                refmvbank: preset.mv_prediction,
                drl_reorder: preset.mv_prediction.then_some(DRL_REORDER_CONSTRAINT),
                max_drl_bits_minus_1: u32::from(preset.drl_bits.saturating_sub(1)),
                mvd_tools: preset.mvd_tools,
                warp: preset.warp > 0,
                global_motion: preset.global_motion > 0,
                masked_compound: preset.compound_types >= 2,
                cwp: preset.compound_types >= 1,
                implicit_mask: preset.implicit_mask,
                parity_hiding: preset.parity_hiding,
                tcq: preset.tcq != TcqFrames::Off,
                film_grain: config.film_grain,
                ..EncoderToolFlags::default()
            },
        };
        let (sequence, view, references) = sequence_state(&params)?;
        Ok(Self {
            params,
            references,
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
                ..UniformTiles::default()
            },
            pool: (config.threads.resolve().get() > 1)
                .then(|| WorkerPool::new(config.threads))
                .transpose()
                .map_err(coding("worker pool"))?,
            #[cfg(test)]
            global_motion: [None; 7],
            importance: tpl::BlockImportance::default(),
            global_models: crate::global_motion::GlobalModels::new(),
            shown_existing: 0,
            grain_active: false,
        })
    }
}

impl FrameCoder for IntraFrameCoder {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        map_tiles(self.pool.as_ref(), wave.to_vec(), |(source, plan, qp)| {
            self.code_frame(source, &plan, qp, &self.superblock_quant(&plan))
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

    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()> {
        if let Some(core) = frame.tip_output.take() {
            return self
                .references
                .update_tip_output(&core)
                .map_err(coding("TIP output update"));
        }
        let (core, recon, initial, cdfs) = frame.reference.take().ok_or(Error::Unsupported {
            what: "a frame committed twice",
        })?;
        if frame.frame_type == FrameType::Key {
            self.grain_active = core
                .intra_tail
                .is_some_and(|tail| tail.film_grain.apply_grain);
        }
        self.references
            .update(&core, recon, &initial, &cdfs)
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
    /// from its block importances; empty without them or when every offset
    /// snaps to 0 at the `delta_q_res` step, which codes no `delta_q_present`
    /// and keeps the frame lambda.
    fn superblock_quant(&self, plan: &GopFrame) -> Vec<(i16, i16)> {
        let Some(costs) = self.importance.frames.get(&plan.display_index) else {
            return Vec::new();
        };
        let sb = match self.params.sb_size {
            SuperblockSize::Block64x64 => 64,
            SuperblockSize::Block128x128 => 128,
            SuperblockSize::Block256x256 => 256,
        };
        let (width, height) = (self.params.width as usize, self.params.height as usize);
        let quant = tpl::superblock_quant(
            costs,
            self.importance.grid,
            (width.div_ceil(sb), height.div_ceil(sb), sb),
            self.importance.strength,
        );
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
    fn code_frame(
        &self,
        source: &SourceFrame,
        plan: &GopFrame,
        qp: u16,
        sb_quant: &[(i16, i16)],
    ) -> Result<CodedFrame> {
        let plan = &GopFrame {
            primary_slot: plan.primary_slot.filter(|_| self.preset.cdf_carry),
            ..*plan
        };
        let qindex = u32::from(qp);
        let mut tools = gop::frame_tools(&self.preset, self.tiles, qindex);
        tools.0.allow_tcq = self.preset.tcq.allows(plan.layer);
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
            let key_display = plan.display_index - u64::from(plan.order_hint);
            let hints = probe
                .inter
                .as_ref()
                .map_or(&[][..], |inter| &inter.order_hints[..]);
            for (model, &hint) in tools.2.7.iter_mut().zip(hints) {
                *model = (hint != u32::MAX)
                    .then(|| {
                        let reference = key_display + u64::from(hint);
                        self.global_models.get(&(plan.display_index, reference))
                    })
                    .flatten()
                    .copied();
            }
        }
        tools.0.delta_q_res = (!sb_quant.is_empty()).then_some(self.preset.delta_q_res);
        let (grain_seed, grain_obu) = self.film_grain(source, plan)?;
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
        let motion = MotionSearch::new(first.syntax(), &padded, &self.preset, lambda)
            .map_err(coding("motion search"))?;
        let bv_search = (core.allow_intrabc == Some(true)).then(|| {
            let (luma, stride, width, height) = padded.luma();
            BvSearch::new(luma, stride, (width, height))
        });
        let (padded, motion, bv_search) = (&padded, motion.as_ref(), bv_search.as_ref());
        let (bit_depth, preset) = (self.bit_depth, &self.preset);
        let sb_cols = mi_cols.div_ceil(sb / 4);
        if !sb_quant.is_empty() && sb_quant.len() != sb_cols * mi_rows.div_ceil(sb / 4) {
            return Err(Error::Unsupported {
                what: "superblock offsets that do not cover the superblock grid",
            });
        }
        let tiles = map_tiles(self.pool.as_ref(), writers, |mut writer| {
            let mut coder = SourceResidualCoder::new(padded, qindex, bit_depth, preset, lambda);
            coder.intrabc = bv_search;
            let mut decisions = FrameDecisions::default();
            while let Some(root) = writer.next_superblock_root() {
                let index = root.row4() / (sb / 4) * sb_cols + root.col4() / (sb / 4);
                let lambda = match sb_quant.get(index) {
                    Some(&(offset, lambda_log2)) => {
                        let target = qindex.saturating_add_signed(i32::from(offset));
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
                        (lambda, Some(target))
                    }
                    None => (lambda, None),
                };
                let (lambda, target) = lambda;
                let tree = writer
                    .search_superblock(|syntax, root| {
                        let mut trial = TrialCoder::new(syntax, &mut coder, preset, lambda, motion);
                        Ok(search_partition(&mut trial, root, preset, lambda)?.0)
                    })
                    .map_err(coding("superblock search"))?;
                decisions
                    .write_superblock(
                        &mut writer,
                        (SuperblockDecision { tree }, target),
                        &mut coder,
                    )
                    .map_err(coding("superblock"))?;
            }
            CodedTile::finish(writer, decisions).map_err(coding("tile finish"))
        })?;
        let global_leaves = tiles
            .iter()
            .map(|tile| tile.count_leaves(&crate::global_motion::is_global))
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
        } = finish_frame(
            &self.sequence,
            &header,
            &self.preset,
            (source, &self.references, self.pool.as_ref()),
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
            (&tiles, &grain_obu),
        )?;
        let coded = CodedFrame {
            temporal_unit,
            recon: visible_planes(&recon)?,
            frame_type: plan.frame_type,
            reference: Some((core, recon, initial, cdfs)),
            tip_output: None,
            warp_leaves,
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
    /// distortions are the squared error of every plane. The caller keeps
    /// `coded` when this fails.
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
        let (core, temporal_unit) = plan.tip_output_unit(
            &self.view,
            &self.references,
            ref_frame_idx,
            (weight, grain_seed),
        )?;
        let (core, frame) = self
            .references
            .tip_output(&core)
            .map_err(coding("TIP output"))?;
        let recon = visible_planes(&frame)?;
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
        let tip = cost(&recon, temporal_unit.len());
        let normal = cost(&coded.recon, coded.temporal_unit.len());
        Ok(
            (tip * 100 <= normal * u128::from(self.preset.tip_output)).then(|| CodedFrame {
                temporal_unit,
                recon,
                frame_type: plan.frame_type,
                reference: None,
                tip_output: Some(core),
                warp_leaves: coded.warp_leaves,
                global_leaves: coded.global_leaves,
            }),
        )
    }
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

fn coding<E: std::error::Error + Send + Sync + 'static>(
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
    bit_depth: BitDepth,
    preset: &'a PresetParams,
    lambda: Lambda,
    /// A candidate trial without `full_trials`: quantize with the deadzone and
    /// code `DCT_DCT`.
    pub(crate) candidate: bool,
    /// A mode trial of the search: a block that hides the other parity moves
    /// its scan index 0 level instead of pricing every move.
    pub(crate) trial: bool,
    /// Price TCQ blocks as plain levels ([`IntraTileSyntax::set_scalar_trial`]).
    pub(crate) scalar: bool,
    /// When `Some`, a luma block that fills its leaf stores the quadrant
    /// energy of its prediction residual here.
    pub(crate) quadrants: Option<[u64; 4]>,
    /// Residual, coefficient, and dequantized buffers reused across blocks.
    scratch: [Vec<i32>; 3],
    /// The U and V coefficients of a CCTX trial.
    pair: [Vec<i32>; 2],
    /// The row costs and the trellis buffers reused across blocks.
    rates: (RowCaches, TcqScratch),
    pub(crate) palette: PaletteSearch,
    /// The block hashes of the frame source, in a frame with IntraBC.
    pub(crate) intrabc: Option<&'a BvSearch>,
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
            quadrants: None,
            scratch: Default::default(),
            pair: Default::default(),
            rates: Default::default(),
            palette: PaletteSearch::default(),
            intrabc: None,
        }
    }
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
        if let (Some(quadrants), true) = (&mut self.quadrants, tx.fills_block) {
            let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
            let rect = (tx.x, tx.y, 1 << log2_w, 1 << log2_h);
            *quadrants = crate::mode_decision::quadrant_sse(self.source, rect, prediction)?;
        }
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
        let ist_probe = if inter { 4 } else { 2 };
        let mut best: Option<(u128, TransformDecision)> = None;
        let scalar_types = (count > 1 || ist_sets > 0) && !self.preset.tcq_type_trellis;
        let scalar = self.scalar;
        self.scalar |= scalar_types;
        for &tx_type in &tx_types[..count] {
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
            if ist {
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
                if let Some(cost) = self.luma_cost(syntax, &coded, prediction, &decision.levels)?
                    && best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost)
                {
                    best = Some((cost, decision));
                }
            }
        }
        self.scalar = scalar;
        let (_, decision) = best.ok_or(TileWriterError::Shape("no luma transform type"))?;
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
        let (params, forward) = self.forward(tx, (use_tcq, ddt), prediction, residual, coeffs)?;
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
        let rotated = self.rotated_pair(syntax, [u, v], predictions, &mut scratch, &mut pair);
        (self.scratch, self.pair) = (scratch, pair);
        let Some([u_levels, v_levels, cctx]) = rotated? else {
            return Ok(plain);
        };
        let coded = TransformBlock {
            cctx: cctx[0] as u8,
            ..*u
        };
        if !syntax.cctx_coded(&coded, eob(&coded, &u_levels)?) {
            return Ok(plain);
        }
        let plain_cost = self.pair_cost(
            syntax,
            [u, v],
            predictions,
            [&plain[0].levels, &plain[1].levels],
            None,
        )?;
        let cost = self.pair_cost(
            syntax,
            [&coded, v],
            predictions,
            [&u_levels, &v_levels],
            Some(()),
        )?;
        if cost >= plain_cost {
            return Ok(plain);
        }
        Ok([
            TransformDecision {
                cctx: coded.cctx,
                levels: u_levels,
                ..plain[0].clone()
            },
            TransformDecision {
                levels: v_levels,
                ..plain[1].clone()
            },
        ])
    }
}

impl SourceResidualCoder<'_> {
    /// The levels of `u` and `v` rotated by the CCTX that leaves V the least
    /// energy, and that `cctx_type` (as `[u, v, [cctx]]`); `None` when no
    /// rotation leaves less than the smaller plain plane.
    fn rotated_pair(
        &mut self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        [u_prediction, v_prediction]: [&[u16]; 2],
        [residual, _, dequant]: &mut [Vec<i32>; 3],
        [u_coeffs, v_coeffs]: &mut [Vec<i32>; 2],
    ) -> core::result::Result<Option<[Vec<i32>; 3]>, TileWriterError> {
        let ddt = (false, syntax.uses_ddt());
        let (u_params, u_forward) = self.forward(u, ddt, u_prediction, residual, u_coeffs)?;
        let (v_params, v_forward) = self.forward(v, ddt, v_prediction, residual, v_coeffs)?;
        let [uu, vv, uv] = kernels().cctx_stats(u_coeffs, v_coeffs);
        let energy = |[c, s]: [i32; 2]| {
            let (c, s) = (i128::from(c), i128::from(s));
            s * s * i128::from(uu) + c * c * i128::from(vv) - 2 * c * s * i128::from(uv)
        };
        let Some((index, best)) = CCTX_ROTATIONS
            .iter()
            .map(|&rotation| energy(rotation))
            .enumerate()
            .min_by_key(|&(_, e)| e)
        else {
            return Ok(None);
        };
        if best >= i128::from(uu.min(vv)) << 16 {
            return Ok(None);
        }
        kernels().cctx_rotate(u_coeffs, v_coeffs, CCTX_ROTATIONS[index]);
        let cctx =
            i32::try_from(index + 1).map_err(|_| TileWriterError::Unsupported("CCTX type"))?;
        let coded = TransformBlock {
            cctx: cctx as u8,
            ..*u
        };
        let u_levels = self.quantize(syntax, &coded, (&u_params, &u_forward), u_coeffs, dequant)?;
        let v_levels = self.quantize(syntax, v, (&v_params, &v_forward), v_coeffs, dequant)?;
        Ok(Some([u_levels, v_levels, vec![cctx]]))
    }

    /// `D + lambda * R` of chroma blocks `u` and `v` coded as `levels`, with
    /// the decoder's cross-chroma reconstruction when `cctx` is `Some`.
    fn pair_cost(
        &self,
        syntax: &mut IntraTileSyntax,
        [u, v]: [&TransformBlock; 2],
        predictions: [&[u16]; 2],
        levels: [&[i32]; 2],
        cctx: Option<()>,
    ) -> core::result::Result<u128, TileWriterError> {
        let recon = match cctx {
            Some(()) => syntax.cctx_reconstruction([u, v], predictions, levels)?,
            None => [
                tx_choice::reconstruct(
                    syntax,
                    predictions[0],
                    u,
                    levels[0],
                    (self.qindex, self.bit_depth),
                )?,
                tx_choice::reconstruct(
                    syntax,
                    predictions[1],
                    v,
                    levels[1],
                    (self.qindex, self.bit_depth),
                )?,
            ],
        };
        let u_eob = eob(u, levels[0])?;
        let mut sink = RateSink::frozen();
        let rate = block_rate(syntax, &mut sink, u, levels[0], false)?;
        let rate = rate + block_rate(syntax, &mut sink, v, levels[1], u_eob != 0)?;
        let cost = RdCost {
            distortion: self.source.sse(u, &recon[0])? + self.source.sse(v, &recon[1])?,
            rate,
        };
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
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let len = coeffs.len();
        dequant.resize(len, 0);
        let use_tcq = syntax.uses_tcq(tx);
        let mut levels = vec![0; len];
        let block = RdoqBlock {
            dc_q2: params.dc_quant,
            ac_q2: params.ac_quant,
            dq_denom: params.dq_denom,
            bit_depth: self.bit_depth,
            log2_width: forward.coeff_width().ilog2(),
            log2_height: forward.coeff_height().ilog2(),
            plane: tx.plane,
            tx_type: tx.tx_type,
        };
        let lambda_q8 = self
            .lambda
            .coefficient_q8(Quantizer::dq_denom_without_tcq(log2_w, log2_h));
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
            let quantizer = Quantizer::new(
                params.dc_quant,
                params.ac_quant,
                params.dq_denom,
                self.bit_depth,
                DEADZONE_ROUNDING_Q8,
            )
            .map_err(kernel)?;
            kernels()
                .quantize(&quantizer, coeffs, &mut levels, dequant)
                .map_err(kernel)?;
        }
        if let Some(parity) = parity {
            let holds = |levels: &[i32]| parity.holds(levels).map_err(TileWriterError::from);
            if !holds(&levels)? {
                let rates = (!self.candidate && !self.trial).then_some(&mut rates);
                let moves =
                    parity_moves(&block, coeffs, rates, lambda_q8, &levels).map_err(kernel)?;
                hide_parity(holds, moves, (&mut levels, dequant))?;
            }
        }
        Ok(levels)
    }

    /// The source residual of `tx` over `prediction` and its primary forward
    /// transform into `coeffs`, with the block's dequantization parameters.
    fn forward(
        &self,
        tx: &TransformBlock,
        (use_tcq, use_ddt): (bool, bool),
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
            self.bit_depth,
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
            (syntax.uses_tcq(tx), syntax.uses_ddt()),
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
        &self,
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
        let recon = tx_choice::reconstruct(
            syntax,
            prediction,
            tx,
            levels,
            (self.qindex, self.bit_depth),
        )?;
        let cost = RdCost {
            distortion: self.source.sse(tx, &recon)?,
            rate: block_rate(syntax, &mut RateSink::frozen(), tx, levels, false)?,
        };
        Ok(Some(cost.cost(self.lambda)))
    }
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
