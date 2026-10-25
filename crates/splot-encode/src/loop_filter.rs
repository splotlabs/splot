// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-filter decisions of a frame: deblocking, CDEF, CCSO, PC-Wiener and
//! Wiener-NS loop restoration, and GDF.
//!
//! A tile codes `use_gdf`, `cdef_idx`, `ccso_blk` and `use_pc_wiener` before the frame is
//! reconstructed, but the search needs the reconstruction. So a frame takes
//! two passes:
//!
//! 1. Each tile is coded with every filter off. [`FrameDecisions`] keeps each
//!    superblock decision and each transform decision of a tile.
//! 2. [`finish_frame`] merges the tiles and searches the filter parameters on
//!    the pass-1 reconstruction with the decoder's own filter chain. If any
//!    filter is on, it replays the kept decisions of each tile through a new
//!    tile writer whose header enables the filters, so the tile also codes the
//!    filter syntax. The unfiltered reconstruction does not change between
//!    the passes.

use std::collections::BTreeMap;

use splot_core::headers::frame::{
    CdefStrengthSet, CdefStrengthSets, FrameFilterParams, FrameHeaderCore, MAX_CDEF_STRENGTH_SETS,
};
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    EncoderReferences, FilterRecorder, FrameStart, GdfFlags, IntraTileSyntax, LrLayout, LrUnit,
    LrUnitType, TileEnd, TransformBlock, apply_loop_filters, cdef_unit_grid, merge_tiles,
};
use splot_parallel::WorkerPool;
use splot_recon::{DecodedFrame, PlaneId};

use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::intra_frame::map_tiles;
use crate::kernels::kernels;
use crate::preset::PresetParams;
use crate::rd::{Bits, Lambda, RdCost, log2_q8};
use crate::tile_writer::filters::FilterSyntax;
use crate::tile_writer::{
    KeyFrameTileWriter, ResidualCoder, SuperblockDecision, TileWriterError, TransformDecision,
    TxType,
};

/// `(pri, sec)` CDEF strengths in the order the search tries them.
const CDEF_LADDER: [(u8, u8); 16] = [
    (0, 0),
    (2, 0),
    (4, 1),
    (6, 2),
    (1, 0),
    (8, 2),
    (3, 1),
    (10, 4),
    (2, 1),
    (5, 2),
    (12, 4),
    (4, 0),
    (6, 1),
    (15, 4),
    (8, 4),
    (1, 1),
];
/// `DfDeltaQ` values in the order the search tries them, `0` and then by
/// |offset|.
const DEBLOCK_LADDER: [i32; 8] = [0, 1, -1, 2, -2, 3, -3, -4];
/// Approximate frame-header bits of one CDEF strength set.
const CDEF_SET_HEADER_BITS: u32 = 12;

/// The coding decisions of one tile, kept for the second pass.
#[derive(Default)]
pub(crate) struct FrameDecisions {
    superblocks: Vec<SuperblockDecision>,
    transforms: Vec<TransformDecision>,
}

impl FrameDecisions {
    /// Writes `decision` through `writer` and keeps it with the transform
    /// decisions `coder` makes.
    pub(crate) fn write_superblock(
        &mut self,
        writer: &mut KeyFrameTileWriter,
        decision: SuperblockDecision,
        coder: &mut impl ResidualCoder,
    ) -> core::result::Result<(), TileWriterError> {
        writer.write_superblock(
            &decision,
            &mut Recording {
                coder,
                transforms: &mut self.transforms,
            },
        )?;
        self.superblocks.push(decision);
        Ok(())
    }

    /// Writes the kept decisions again through `writer`.
    fn replay(self, writer: &mut KeyFrameTileWriter) -> core::result::Result<(), TileWriterError> {
        let mut replay = Replay(self.transforms.into_iter());
        for decision in &self.superblocks {
            writer.write_superblock(decision, &mut replay)?;
        }
        if replay.0.next().is_some() {
            return Err(TileWriterError::Shape("replayed transform count"));
        }
        Ok(())
    }
}

/// Forwards every transform to `coder` and keeps it for the second pass.
struct Recording<'a, C> {
    coder: &'a mut C,
    transforms: &'a mut Vec<TransformDecision>,
}

impl<C: ResidualCoder> ResidualCoder for Recording<'_, C> {
    fn code_luma(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        tx_types: &[TxType],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let decision = self.coder.code_luma(syntax, tx, prediction, tx_types)?;
        self.transforms.push(decision.clone());
        Ok(decision)
    }

    fn code(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let decision = self.coder.code(syntax, tx, prediction)?;
        self.transforms.push(decision.clone());
        Ok(decision)
    }
}

/// Replays the kept transform decisions in coding order.
struct Replay(std::vec::IntoIter<TransformDecision>);

impl ResidualCoder for Replay {
    fn code(
        &mut self,
        _syntax: &mut IntraTileSyntax,
        _tx: &TransformBlock,
        _prediction: &[u16],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        self.0
            .next()
            .ok_or(TileWriterError::Shape("replayed transform count"))
    }
}

/// One tile coded by its own writer.
pub(crate) struct CodedTile {
    bytes: Vec<u8>,
    recon: DecodedFrame<u16>,
    cdfs: TileEnd,
    recorder: FilterRecorder,
    syntax: FilterSyntax,
    decisions: FrameDecisions,
}

impl CodedTile {
    /// The finished `writer` and the `decisions` it wrote.
    pub(crate) fn finish(
        writer: KeyFrameTileWriter,
        decisions: FrameDecisions,
    ) -> core::result::Result<Self, TileWriterError> {
        let (bytes, recon, cdfs, (recorder, syntax)) = writer.finish_with_filters()?;
        Ok(Self {
            bytes,
            recon,
            cdfs,
            recorder,
            syntax,
            decisions,
        })
    }
}

/// The tiles of a frame put together: the bytes and final CDFs of each tile,
/// the merged reconstruction, filter records and coded filter choices, and the
/// decisions of each tile.
struct MergedTiles {
    bytes: Vec<Vec<u8>>,
    cdfs: Vec<TileEnd>,
    recon: DecodedFrame<u16>,
    recorder: FilterRecorder,
    syntax: FilterSyntax,
    decisions: Vec<FrameDecisions>,
}

impl MergedTiles {
    /// `tiles` of the frame `core`, in tile order.
    fn new(core: &FrameHeaderCore, tiles: Vec<CodedTile>) -> Result<Self> {
        let mut bytes = Vec::with_capacity(tiles.len());
        let mut cdfs = Vec::with_capacity(tiles.len());
        let mut recons = Vec::with_capacity(tiles.len());
        let mut recorder: Option<FilterRecorder> = None;
        let mut syntax = FilterSyntax::default();
        let mut decisions = Vec::with_capacity(tiles.len());
        for tile in tiles {
            bytes.push(tile.bytes);
            cdfs.push(tile.cdfs);
            recons.push(tile.recon);
            match recorder.as_mut() {
                Some(recorder) => recorder.append(tile.recorder),
                None => recorder = Some(tile.recorder),
            }
            if syntax.cdef_indices.is_empty() {
                syntax.cdef_indices = tile.syntax.cdef_indices;
            } else {
                for (index, coded) in syntax.cdef_indices.iter_mut().zip(tile.syntax.cdef_indices) {
                    *index = index.or(coded);
                }
            }
            syntax.lr_on.extend(tile.syntax.lr_on);
            if syntax.ccso_blocks.is_empty() {
                syntax.ccso_blocks = tile.syntax.ccso_blocks;
            }
            if syntax.gdf_blocks.is_empty() {
                syntax.gdf_blocks = tile.syntax.gdf_blocks;
            }
            decisions.push(tile.decisions);
        }
        let recon = match <[_; 1]>::try_from(recons) {
            Ok([recon]) => recon,
            Err(recons) => merge_tiles(core, &recons).map_err(coding("tile merge"))?,
        };
        Ok(Self {
            bytes,
            cdfs,
            recon,
            recorder: recorder.ok_or(Error::Unsupported {
                what: "a frame without tiles",
            })?,
            syntax,
            decisions,
        })
    }
}

/// The first pass of a frame: its header and its tiles in tile order.
pub(crate) struct FirstPass {
    pub(crate) core: FrameHeaderCore,
    /// The state the frame starts from, which the filter pass starts from too.
    pub(crate) start: FrameStart,
    pub(crate) tiles: Vec<CodedTile>,
}

/// A coded frame: its header, its filters, its tiles and their final CDFs,
/// and its filtered reconstruction.
pub(crate) struct FilteredFrame {
    pub(crate) core: FrameHeaderCore,
    pub(crate) filters: FrameFilterParams,
    pub(crate) tiles: Vec<Vec<u8>>,
    pub(crate) cdfs: Vec<TileEnd>,
    pub(crate) recon: DecodedFrame<u16>,
}

/// Builds the frame header with the given loop filters.
pub(crate) type HeaderBuilder<'a> = &'a dyn Fn(&FrameFilterParams) -> Result<FrameHeaderCore>;

/// Picks the frame's loop filters and, when any is on, codes the second pass
/// of every tile, on `pool` when there is one.
pub(crate) fn finish_frame(
    seq: &SequenceHeader,
    header: HeaderBuilder<'_>,
    preset: &PresetParams,
    (source, refs, pool): (&SourceFrame, &EncoderReferences, Option<&WorkerPool>),
    pass: FirstPass,
) -> Result<FilteredFrame> {
    let MergedTiles {
        bytes,
        cdfs,
        recon,
        recorder,
        decisions,
        ..
    } = MergedTiles::new(&pass.core, pass.tiles)?;
    let qindex = pass
        .core
        .quantization_params
        .ok_or(Error::Unsupported {
            what: "a frame without quantization params",
        })?
        .base_q_idx;
    let search = Search {
        seq,
        header,
        qindex,
        source,
        recorder: &recorder,
        recon: &recon,
        lambda: Lambda::from_qindex(qindex, recon.bit_depth()).map_err(coding("lambda"))?,
        start: &pass.start,
        refs,
        pool,
    };
    let (params, syntax) = search.run(preset)?;
    if params == FrameFilterParams::default() {
        return Ok(FilteredFrame {
            core: pass.core,
            filters: params,
            tiles: bytes,
            cdfs,
            recon,
        });
    }
    let core = search.header(&params)?;
    let writers = decisions
        .into_iter()
        .enumerate()
        .map(|(tile, decisions)| {
            let mut writer = KeyFrameTileWriter::with_start(seq, &core, refs, &pass.start, tile)?;
            writer.set_filter_syntax(syntax.clone());
            Ok((writer, decisions))
        })
        .collect::<core::result::Result<Vec<_>, TileWriterError>>()
        .map_err(coding("filter pass"))?;
    let tiles = map_tiles(pool, writers, |(mut writer, decisions)| {
        decisions
            .replay(&mut writer)
            .and_then(|()| CodedTile::finish(writer, FrameDecisions::default()))
            .map_err(coding("filter pass"))
    })?;
    let MergedTiles {
        bytes,
        cdfs,
        recon,
        recorder,
        syntax: coded,
        ..
    } = MergedTiles::new(&core, tiles)?;
    let lr_on: Vec<(LrUnit, LrUnitType)> = coded.lr_on.into_iter().collect();
    let recon = apply_loop_filters(
        seq,
        &core,
        &recorder,
        &recon,
        (
            &coded.cdef_indices,
            &coded.ccso_blocks,
            Some(GdfFlags {
                blocks: &coded.gdf_blocks,
                refs,
            }),
        ),
        &lr_on,
    )
    .map_err(coding("loop filters"))?;
    Ok(FilteredFrame {
        core,
        filters: params,
        tiles: bytes,
        cdfs,
        recon,
    })
}

fn coding<E: std::error::Error + Send + Sync + 'static>(
    stage: &'static str,
) -> impl FnOnce(E) -> Error {
    move |source| Error::Coding {
        stage,
        source: Box::new(source),
    }
}

struct Search<'a> {
    seq: &'a SequenceHeader,
    header: HeaderBuilder<'a>,
    qindex: u32,
    source: &'a SourceFrame,
    recorder: &'a FilterRecorder,
    recon: &'a DecodedFrame<u16>,
    lambda: Lambda,
    start: &'a FrameStart,
    refs: &'a EncoderReferences,
    pool: Option<&'a WorkerPool>,
}

impl Search<'_> {
    fn run(&self, preset: &PresetParams) -> Result<(FrameFilterParams, FilterSyntax)> {
        let filter = self.seq.filter.ok_or(Error::Unsupported {
            what: "a sequence header without filter config",
        })?;
        let mut params = self.deblocking(preset)?;
        let mut syntax = FilterSyntax::default();
        let ccso = filter.enable_ccso && preset.ccso_candidates > 0;
        let deblocked = if params.deblocking == [false; 4] || !(filter.enable_cdef || ccso) {
            None
        } else {
            Some(self.filtered(&params, self.recon, &[])?)
        };
        let deblocked = deblocked.as_ref().unwrap_or(self.recon);
        if filter.enable_cdef {
            let (cdef_damping, cdef_strengths, indices) = self.cdef(preset, deblocked)?;
            params.cdef_damping = cdef_damping;
            params.cdef_strengths = cdef_strengths;
            syntax.cdef_indices = indices;
        }
        if ccso {
            (params.ccso, syntax.ccso_blocks) = self.ccso(&params, &syntax, deblocked, preset)?;
        }
        if filter.enable_restoration {
            let sb128 = self
                .seq
                .partition
                .is_some_and(|partition| partition.seq_sb_size() != SuperblockSize::Block64x64);
            let candidate = FrameFilterParams {
                lr_luma_unit_size: Some(if sb128 { 128 } else { 64 }),
                ..params
            };
            let (chosen, lr_on) = if preset.wiener_ns > 0 {
                let pre_cdef = (filter.enable_cdef || ccso).then_some(deblocked);
                self.restoration_rd(&candidate, &syntax, (preset, pre_cdef))?
            } else {
                (candidate, self.restoration(&candidate, &syntax)?)
            };
            if !lr_on.is_empty() {
                params = chosen;
                syntax.lr_on = lr_on;
            }
        }
        if filter.enable_gdf
            && preset.gdf_candidates > 0
            && let Some((gdf, blocks)) =
                self.gdf(&params, &syntax, usize::from(preset.gdf_candidates))?
        {
            params.gdf = Some(gdf);
            syntax.gdf_blocks = blocks;
        }
        Ok((params, syntax))
    }

    fn header(&self, params: &FrameFilterParams) -> Result<FrameHeaderCore> {
        (self.header)(params)
    }

    /// `frame` filtered with `params`, every `cdef_idx` `0`, and `lr_on` units on.
    fn filtered(
        &self,
        params: &FrameFilterParams,
        frame: &DecodedFrame<u16>,
        lr_on: &[(LrUnit, LrUnitType)],
    ) -> Result<DecodedFrame<u16>> {
        let core = self.header(params)?;
        let (rows, cols) = self.cdef_grid();
        apply_loop_filters(
            self.seq,
            &core,
            self.recorder,
            frame,
            (&vec![Some(0); rows * cols], &[], None),
            lr_on,
        )
        .map_err(coding("loop filters"))
    }

    fn cdef_grid(&self) -> (usize, usize) {
        let luma = self.recon.coded_luma_size();
        cdef_unit_grid((luma.height().div_ceil(4), luma.width().div_ceil(4)))
    }

    /// Deblocking on every plane at the qindex-derived level, or with the
    /// per-plane `DfDeltaQ` of least SSE among the first 3, 5 or 8 offsets
    /// of [`DEBLOCK_LADDER`] at depth 1, 2 or 3 that the sequence codes; on
    /// equal SSE the earlier offset, so `0` and then the smallest |offset|.
    fn deblocking(&self, preset: &PresetParams) -> Result<FrameFilterParams> {
        let on = FrameFilterParams {
            deblocking: [true; 4],
            ..FrameFilterParams::default()
        };
        if preset.loop_filter_search == 0 {
            return Ok(on);
        }
        let bits = self
            .seq
            .filter
            .map_or(3, |f| u32::from(f.df_par_bits_minus_2) + 2);
        let range = -(1 << (bits - 1))..=(1 << (bits - 1)) - 1;
        let levels = [0, 3, 5, 8][usize::from(preset.loop_filter_search).min(3)];
        let off = self.plane_sse_all(self.recon);
        let mut best: [(u64, Option<i32>); 3] = core::array::from_fn(|plane| (off[plane], None));
        for &delta in DEBLOCK_LADDER[..levels]
            .iter()
            .filter(|&&d| range.contains(&d))
        {
            let params = FrameFilterParams {
                deblocking_delta_q: [delta; 4],
                ..on
            };
            let sse = self.plane_sse_all(&self.filtered(&params, self.recon, &[])?);
            for (best, sse) in best.iter_mut().zip(sse) {
                if sse < best.0 {
                    *best = (sse, Some(delta));
                }
            }
        }
        let [luma, u, v] = best.map(|(_, delta)| delta);
        let chroma = |delta: Option<i32>| luma.and(delta);
        let planes = [luma, luma, chroma(u), chroma(v)];
        Ok(FrameFilterParams {
            deblocking: planes.map(|delta| delta.is_some()),
            deblocking_delta_q: planes.map(|delta| delta.unwrap_or(0)),
            ..FrameFilterParams::default()
        })
    }

    /// The CDEF damping, strength sets and per-unit indices that minimize SSE
    /// plus the approximate index rate on the deblocked frame.
    fn cdef(
        &self,
        preset: &PresetParams,
        deblocked: &DecodedFrame<u16>,
    ) -> Result<(u8, CdefStrengthSets, Vec<Option<usize>>)> {
        let candidates = &CDEF_LADDER[..[8, 12, 16][usize::from(preset.loop_filter_search).min(2)]];
        let qmax = u32::from(crate::rate_control::max_qindex(self.recon.bit_depth()));
        let damping = 3 + (3 * self.qindex / qmax).min(3) as u8;
        let (rows, cols) = self.cdef_grid();
        let units = rows * cols;
        // sse[candidate][unit] as (luma, chroma).
        let mut sse = Vec::with_capacity(candidates.len());
        for &(pri, sec) in candidates {
            let set = CdefStrengthSet {
                y_pri_strength: pri,
                y_sec_strength: sec,
                uv_pri_strength: pri,
                uv_sec_strength: sec,
            };
            let params = FrameFilterParams {
                cdef_damping: damping,
                cdef_strengths: cdef_sets(&[set])?,
                ..FrameFilterParams::default()
            };
            let frame = self.filtered(&params, deblocked, &[])?;
            sse.push(
                (0..units)
                    .map(|unit| self.cdef_unit_sse(&frame, unit / cols, unit % cols))
                    .collect::<Vec<_>>(),
            );
        }
        let pairs: Vec<(usize, usize)> = (0..candidates.len())
            .flat_map(|y| (0..candidates.len()).map(move |uv| (y, uv)))
            .collect();
        let cost = |unit: usize, (y, uv): (usize, usize)| sse[y][unit].0 + sse[uv][unit].1;
        let total = |sets: &[(usize, usize)]| {
            let distortion: u64 = (0..units)
                .map(|unit| sets.iter().map(|&set| cost(unit, set)).min().unwrap_or(0))
                .sum();
            RdCost {
                distortion,
                rate: cdef_rate(sets.len(), units),
            }
            .cost(self.lambda)
        };
        let mut sets: Vec<(usize, usize)> = Vec::new();
        let mut best_total = u128::MAX;
        while sets.len() < MAX_CDEF_STRENGTH_SETS {
            let mut trial = sets.clone();
            trial.push((0, 0));
            let pick = pairs
                .iter()
                .filter(|pair| !sets.contains(pair))
                .map(|&pair| {
                    *trial.last_mut().unwrap_or(&mut (0, 0)) = pair;
                    (total(&trial), pair)
                })
                .min_by_key(|&(value, _)| value);
            match pick {
                Some((value, pair)) if value < best_total => {
                    best_total = value;
                    sets.push(pair);
                }
                _ => break,
            }
        }
        if sets.iter().all(|&(y, uv)| y == 0 && uv == 0) {
            return Ok((0, CdefStrengthSets::empty(), Vec::new()));
        }
        let indices = (0..units)
            .map(|unit| {
                let best = sets
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &set)| cost(unit, set));
                Some(best.map_or(0, |(index, _)| index))
            })
            .collect();
        let strengths: Vec<CdefStrengthSet> = sets
            .iter()
            .map(|&(y, uv)| CdefStrengthSet {
                y_pri_strength: candidates[y].0,
                y_sec_strength: candidates[y].1,
                uv_pri_strength: candidates[uv].0,
                uv_sec_strength: candidates[uv].1,
            })
            .collect();
        Ok((damping, cdef_sets(&strengths)?, indices))
    }

    /// `(luma, chroma)` SSE of one 64x64 CDEF unit.
    fn cdef_unit_sse(&self, frame: &DecodedFrame<u16>, row: usize, col: usize) -> (u64, u64) {
        let luma = self.sse(frame, 0, (col * 64, row * 64, 64, 64));
        let format = frame.info().pixel_format();
        let (w, h) = (64 >> format.subsampling_x(), 64 >> format.subsampling_y());
        let chroma = (1..3)
            .map(|plane| self.sse(frame, plane, (col * w, row * h, w, h)))
            .sum();
        (luma, chroma)
    }

    /// The CCSO choice on top of the CDEF choice of `params` and `syntax`.
    fn ccso(
        &self,
        params: &FrameFilterParams,
        syntax: &FilterSyntax,
        deblocked: &DecodedFrame<u16>,
        preset: &PresetParams,
    ) -> Result<ccso::CcsoChoice> {
        let cdef = FrameFilterParams {
            cdef_damping: params.cdef_damping,
            cdef_strengths: params.cdef_strengths,
            ..FrameFilterParams::default()
        };
        let filtered = if cdef.cdef_strengths.is_empty() {
            None
        } else {
            let core = self.header(&cdef)?;
            Some(
                apply_loop_filters(
                    self.seq,
                    &core,
                    self.recorder,
                    deblocked,
                    (&syntax.cdef_indices, &[], None),
                    &[],
                )
                .map_err(coding("loop filters"))?,
            )
        };
        let core = self.header(&FrameFilterParams {
            ccso: [ccso::placeholder_plane(); 3],
            ..*params
        })?;
        ccso::search(
            &ccso::CcsoInput {
                seq: self.seq,
                core: &core,
                start: self.start,
                deblocked,
                filtered: filtered.as_ref().unwrap_or(deblocked),
                source: self.source,
                lambda: self.lambda,
                pool: self.pool,
            },
            usize::from(preset.ccso_candidates),
        )
    }

    /// The luma PC-Wiener units that lower the SSE of `params` (whose loop
    /// restoration is on) with the CDEF and CCSO choices of `syntax`.
    fn restoration(
        &self,
        params: &FrameFilterParams,
        syntax: &FilterSyntax,
    ) -> Result<BTreeMap<LrUnit, LrUnitType>> {
        let core = self.header(params)?;
        let layouts = LrLayout::tiles(self.seq, &core).map_err(coding("restoration layout"))?;
        let mut tile_units = Vec::new();
        for layout in &layouts {
            for unit in layout.all_units().map_err(coding("restoration layout"))? {
                tile_units.push((layout, unit));
            }
        }
        let units: Vec<_> = tile_units
            .iter()
            .map(|&(_, unit)| (unit, LrUnitType::PcWiener))
            .collect();
        let run = |lr_on: &[(LrUnit, LrUnitType)]| {
            apply_loop_filters(
                self.seq,
                &core,
                self.recorder,
                self.recon,
                (&syntax.cdef_indices, &syntax.ccso_blocks, None),
                lr_on,
            )
            .map_err(coding("loop filters"))
        };
        let off = run(&[])?;
        let on = run(&units)?;
        let mut chosen = BTreeMap::new();
        for (layout, unit) in tile_units {
            let (mut sse_on, mut sse_off) = (0, 0);
            for rect in layout
                .unit_rects(unit)
                .map_err(coding("restoration unit"))?
            {
                let rect = (rect.x(), rect.y(), rect.width(), rect.height());
                sse_on += self.sse(&on, unit.plane, rect);
                sse_off += self.sse(&off, unit.plane, rect);
            }
            if sse_on < sse_off {
                chosen.insert(unit, LrUnitType::PcWiener);
            }
        }
        Ok(chosen)
    }

    /// Per-plane SSE of `frame` against the source.
    fn plane_sse_all(&self, frame: &DecodedFrame<u16>) -> [u64; 3] {
        core::array::from_fn(|plane| self.sse(frame, plane, (0, 0, usize::MAX, usize::MAX)))
    }

    /// SSE of plane `plane` over the visible part of `(left, top, width, height)`.
    fn sse(
        &self,
        frame: &DecodedFrame<u16>,
        plane: usize,
        rect: (usize, usize, usize, usize),
    ) -> u64 {
        plane_sse(self.source, frame, plane, rect)
    }
}

/// SSE of plane `plane` of `frame` against `source` over the visible part of
/// `(left, top, width, height)`.
fn plane_sse(
    source: &SourceFrame,
    frame: &DecodedFrame<u16>,
    plane: usize,
    (left, top, width, height): (usize, usize, usize, usize),
) -> u64 {
    let sizes = source.plane_sizes();
    let (Some(id), Some(size), Some(source)) = (
        [PlaneId::Y, PlaneId::U, PlaneId::V].get(plane),
        sizes.get(plane),
        source.planes().get(plane),
    ) else {
        return 0;
    };
    let Some(recon) = frame.plane(*id) else {
        return 0;
    };
    let right = left.saturating_add(width).min(size.width());
    let bottom = top.saturating_add(height).min(size.height());
    let stride = recon.stride_samples();
    let kernels = kernels();
    (top..bottom)
        .map(|row| {
            let coded = recon
                .samples()
                .get(row * stride + left..row * stride + right);
            let original = source.get(row * size.width() + left..row * size.width() + right);
            kernels.sse_row(coded.unwrap_or(&[]), original.unwrap_or(&[]))
        })
        .sum()
}

fn cdef_sets(sets: &[CdefStrengthSet]) -> Result<CdefStrengthSets> {
    CdefStrengthSets::from_slice(sets).ok_or(Error::Unsupported {
        what: "more CDEF strength sets than a frame header codes",
    })
}

/// Approximate bits of `sets` strength sets: header bits plus the `cdef_idx`
/// symbols of every unit.
fn cdef_rate(sets: usize, units: usize) -> Bits {
    let per_unit = match sets {
        0 | 1 => 0,
        n => Bits::whole(1).0 + u64::from(log2_q8(n as u32 - 1)),
    };
    Bits(Bits::whole(CDEF_SET_HEADER_BITS).0 * sets as u64 + per_unit * units as u64)
}

#[path = "loop_filter_ccso.rs"]
mod ccso;
#[path = "loop_filter_gdf.rs"]
mod gdf;
#[path = "loop_filter_wienerns.rs"]
mod wienerns;

#[cfg(test)]
#[path = "loop_filter_tests.rs"]
mod tests;
