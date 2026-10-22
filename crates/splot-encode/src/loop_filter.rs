// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-filter decisions of a frame: deblocking, CDEF and luma PC-Wiener
//! loop restoration. CCSO and GDF stay off.
//!
//! A tile codes `cdef_idx` and `use_pc_wiener` before the frame is reconstructed,
//! but the search needs the reconstruction. So a frame takes two passes:
//!
//! 1. The tile is coded with every filter off. [`FrameDecisions`] keeps each
//!    superblock decision and each transform decision.
//! 2. [`finish_frame`] searches the filter parameters on the pass-1
//!    reconstruction with the decoder's own filter chain. If any filter is on,
//!    it replays the kept decisions through a new tile writer whose header
//!    enables the filters, so the tile also codes the filter syntax. The
//!    unfiltered reconstruction does not change between the passes.

use std::collections::BTreeSet;

use splot_core::headers::frame::{
    CdefStrengthSet, CdefStrengthSets, FrameFilterParams, FrameHeaderCore, MAX_CDEF_STRENGTH_SETS,
};
use splot_core::headers::sequence::{SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    EncoderReferences, FilterRecorder, IntraTileSyntax, LrLayout, LrUnit, TransformBlock,
    apply_loop_filters, cdef_unit_grid,
};
use splot_recon::{BitDepth, DecodedFrame, PlaneId};

use crate::error::{Error, Result};
use crate::frame::SourceFrame;
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
/// `DfDeltaQ` values in the order the search tries them (`f(3)` offsets).
const DEBLOCK_LADDER: [i32; 8] = [0, 1, -1, 2, -2, 3, -4, -3];
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

/// The first pass of a frame.
pub(crate) struct FirstPass {
    pub(crate) core: FrameHeaderCore,
    pub(crate) tile: Vec<u8>,
    pub(crate) recon: DecodedFrame<u16>,
    pub(crate) recorder: FilterRecorder,
    pub(crate) decisions: FrameDecisions,
}

/// A coded frame: its header, its filters, its tile, and its filtered
/// reconstruction.
pub(crate) struct FilteredFrame {
    pub(crate) core: FrameHeaderCore,
    pub(crate) filters: FrameFilterParams,
    pub(crate) tile: Vec<u8>,
    pub(crate) recon: DecodedFrame<u16>,
}

/// Builds the frame header with the given loop filters.
pub(crate) type HeaderBuilder<'a> = &'a dyn Fn(&FrameFilterParams) -> Result<FrameHeaderCore>;

/// Picks the frame's loop filters and, when any is on, codes the second pass.
pub(crate) fn finish_frame(
    seq: &SequenceHeader,
    header: HeaderBuilder<'_>,
    preset: &PresetParams,
    (source, refs): (&SourceFrame, &EncoderReferences),
    pass: FirstPass,
) -> Result<FilteredFrame> {
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
        recorder: &pass.recorder,
        recon: &pass.recon,
        lambda: Lambda::from_qindex(qindex, pass.recon.bit_depth()).map_err(coding("lambda"))?,
    };
    let (params, syntax) = search.run(preset)?;
    if params == FrameFilterParams::default() {
        return Ok(FilteredFrame {
            core: pass.core,
            filters: params,
            tile: pass.tile,
            recon: pass.recon,
        });
    }
    let core = search.header(&params)?;
    let mut writer = KeyFrameTileWriter::new(seq, &core, refs).map_err(coding("filter pass"))?;
    writer.set_filter_syntax(syntax);
    let mut replay = Replay(pass.decisions.transforms.into_iter());
    for decision in &pass.decisions.superblocks {
        writer
            .write_superblock(decision, &mut replay)
            .map_err(coding("filter pass"))?;
    }
    if replay.0.next().is_some() {
        return Err(coding("filter pass")(TileWriterError::Shape(
            "replayed transform count",
        )));
    }
    let (tile, recon, (recorder, coded)) = writer
        .finish_with_filters()
        .map_err(coding("filter pass"))?;
    let lr_on: Vec<LrUnit> = coded.lr_on.into_iter().collect();
    let recon = apply_loop_filters(seq, &core, &recorder, &recon, &coded.cdef_indices, &lr_on)
        .map_err(coding("loop filters"))?;
    Ok(FilteredFrame {
        core,
        filters: params,
        tile,
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
}

impl Search<'_> {
    fn run(&self, preset: &PresetParams) -> Result<(FrameFilterParams, FilterSyntax)> {
        let filter = self.seq.filter.ok_or(Error::Unsupported {
            what: "a sequence header without filter config",
        })?;
        let mut params = self.deblocking(preset)?;
        let mut syntax = FilterSyntax::default();
        if filter.enable_cdef {
            let deblocked = if params.deblocking == [false; 4] {
                None
            } else {
                Some(self.filtered(&params, self.recon, &[])?)
            };
            let (cdef_damping, cdef_strengths, indices) =
                self.cdef(preset, deblocked.as_ref().unwrap_or(self.recon))?;
            params.cdef_damping = cdef_damping;
            params.cdef_strengths = cdef_strengths;
            syntax.cdef_indices = indices;
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
            let lr_on = self.restoration(&candidate, &syntax.cdef_indices)?;
            if !lr_on.is_empty() {
                params = candidate;
                syntax.lr_on = lr_on;
            }
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
        lr_on: &[LrUnit],
    ) -> Result<DecodedFrame<u16>> {
        let core = self.header(params)?;
        let (rows, cols) = self.cdef_grid();
        apply_loop_filters(
            self.seq,
            &core,
            self.recorder,
            frame,
            &vec![Some(0); rows * cols],
            lr_on,
        )
        .map_err(coding("loop filters"))
    }

    fn cdef_grid(&self) -> (usize, usize) {
        let luma = self.recon.coded_luma_size();
        cdef_unit_grid((luma.height().div_ceil(4), luma.width().div_ceil(4)))
    }

    /// Deblocking on every plane at the qindex-derived level, or with the
    /// per-plane best `DfDeltaQ` of up to `1 + 2 * loop_filter_search` levels.
    fn deblocking(&self, preset: &PresetParams) -> Result<FrameFilterParams> {
        let on = FrameFilterParams {
            deblocking: [true; 4],
            ..FrameFilterParams::default()
        };
        if preset.loop_filter_search == 0 {
            return Ok(on);
        }
        let levels = (1 + 2 * usize::from(preset.loop_filter_search)).min(DEBLOCK_LADDER.len());
        let off = self.plane_sse_all(self.recon);
        let mut best: [(u64, Option<i32>); 3] = core::array::from_fn(|plane| (off[plane], None));
        for &delta in &DEBLOCK_LADDER[..levels] {
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
        let qmax = if self.recon.bit_depth() == BitDepth::Eight {
            255
        } else {
            511
        };
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
        let chroma = (1..3)
            .map(|plane| self.sse(frame, plane, (col * 32, row * 32, 32, 32)))
            .sum();
        (luma, chroma)
    }

    /// The luma PC-Wiener units that lower the SSE of `params` (whose loop
    /// restoration is on) with CDEF indices `cdef_indices`.
    fn restoration(
        &self,
        params: &FrameFilterParams,
        cdef_indices: &[Option<usize>],
    ) -> Result<BTreeSet<LrUnit>> {
        let core = self.header(params)?;
        let layout = LrLayout::new(self.seq, &core).map_err(coding("restoration layout"))?;
        let units = layout.all_units().map_err(coding("restoration layout"))?;
        let run = |lr_on: &[LrUnit]| {
            apply_loop_filters(
                self.seq,
                &core,
                self.recorder,
                self.recon,
                cdef_indices,
                lr_on,
            )
            .map_err(coding("loop filters"))
        };
        let off = run(&[])?;
        let on = run(&units)?;
        let mut chosen = BTreeSet::new();
        for unit in units {
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
                chosen.insert(unit);
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
        (left, top, width, height): (usize, usize, usize, usize),
    ) -> u64 {
        let sizes = self.source.plane_sizes();
        let (Some(id), Some(size), Some(source)) = (
            [PlaneId::Y, PlaneId::U, PlaneId::V].get(plane),
            sizes.get(plane),
            self.source.planes().get(plane),
        ) else {
            return 0;
        };
        let Some(recon) = frame.plane(*id) else {
            return 0;
        };
        let right = left.saturating_add(width).min(size.width());
        let bottom = top.saturating_add(height).min(size.height());
        let stride = recon.stride_samples();
        let mut sum = 0u64;
        for row in top..bottom {
            let coded = recon
                .samples()
                .get(row * stride + left..row * stride + right);
            let original = source.get(row * size.width() + left..row * size.width() + right);
            for (&coded, &original) in coded.unwrap_or(&[]).iter().zip(original.unwrap_or(&[])) {
                sum += u64::from(coded.abs_diff(original)).pow(2);
            }
        }
        sum
    }
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

#[cfg(test)]
#[path = "loop_filter_tests.rs"]
mod tests;
