// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Key-frame coding: the RD partition and mode search of each superblock,
//! residual coding, and the temporal unit of one intra frame.

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams,
    build_encoder_sequence_header, build_key_frame_header, encode_key_frame_temporal_unit,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    IntraTileSyntax, TransformBlock, TxType, recon_params, tx_log2,
};
use splot_recon::{BitDepth, DecodedFrame, PlaneId, PlaneSize};

use crate::config::Config;
use crate::encoder::{CodedFrame, FrameCoder};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::kernels::rdoq::{RdoqBlock, rdoq};
use crate::kernels::{BlockDims, ForwardTransform2d, Quantizer, kernels};
use crate::loop_filter::{FilteredFrame, FirstPass, FrameDecisions, finish_key_frame};
use crate::mode_decision::TrialCoder;
use crate::preset::PresetParams;
use crate::rate_control::{FramePlanInfo, FrameType};
use crate::rd::coeff_rate::{CdfCoeffRates, RateSink, block_rate};
use crate::rd::partition::search_partition;
use crate::rd::{Lambda, RdCost};
use crate::tile_writer::coeffs::eob;
use crate::tile_writer::{
    KeyFrameTileWriter, ResidualCoder, SuperblockDecision, TileWriterError, TransformDecision,
};
use crate::tx_choice;

/// Quantizer deadzone in Q8 (`128` rounds to nearest).
const DEADZONE_ROUNDING_Q8: u32 = 96;

/// Codes every frame as a closed-loop key frame.
pub(crate) struct IntraFrameCoder {
    sequence: SequenceHeader,
    view: CoreSeqView,
    bit_depth: BitDepth,
    preset: &'static PresetParams,
}

impl IntraFrameCoder {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let preset = config.preset.params();
        let sequence = build_encoder_sequence_header(&EncoderStreamParams {
            width: config.width,
            height: config.height,
            bit_depth: match config.bit_depth {
                BitDepth::Eight => BitDepthIdc::Eight,
                BitDepth::Ten => BitDepthIdc::Ten,
            },
            chroma: ChromaFormatIdc::Yuv420,
            sb_size: if preset.max_block_log2 >= 7 {
                SuperblockSize::Block128x128
            } else {
                SuperblockSize::Block64x64
            },
            tools: EncoderToolFlags {
                cdef: true,
                loop_restoration: true,
                cfl: true,
                ibp: true,
                intra_edge_filter: true,
                ..EncoderToolFlags::default()
            },
        })
        .map_err(coding("sequence header"))?;
        let view = CoreSeqView::from_sequence(&sequence).ok_or(Error::Unsupported {
            what: "an incomplete sequence header",
        })?;
        Ok(Self {
            sequence,
            view,
            bit_depth: config.bit_depth,
            preset,
        })
    }
}

impl FrameCoder for IntraFrameCoder {
    fn encode_frame(
        &mut self,
        source: &SourceFrame,
        _plan: &FramePlanInfo,
        qp: u8,
    ) -> Result<CodedFrame> {
        let qindex = u32::from(qp);
        let core = build_key_frame_header(
            &self.view,
            qindex,
            0,
            &FrameFilterParams::default(),
            tx_choice::key_frame_tx_params(self.preset),
        )
        .map_err(coding("frame header"))?;
        let mut writer =
            KeyFrameTileWriter::new(&self.sequence, &core).map_err(coding("tile setup"))?;
        let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
        let sb = writer.syntax().superblock_size4() * 4;
        let padded = PaddedSource::new(source, mi_cols * 4 + sb, mi_rows * 4 + sb);
        let lambda = Lambda::from_qindex(qindex, self.bit_depth).map_err(coding("lambda"))?;
        let mut coder = SourceResidualCoder {
            source: &padded,
            qindex,
            bit_depth: self.bit_depth,
            preset: self.preset,
            lambda,
            candidate: false,
        };
        let mut decisions = FrameDecisions::default();
        let preset = self.preset;
        while writer.next_superblock_root().is_some() {
            let tree = writer
                .search_superblock(|syntax, root| {
                    let mut trial = TrialCoder::new(syntax, &mut coder, preset, lambda);
                    Ok(search_partition(&mut trial, root, preset, lambda)?.0)
                })
                .map_err(coding("superblock search"))?;
            decisions
                .write_superblock(&mut writer, SuperblockDecision { tree }, &mut coder)
                .map_err(coding("superblock"))?;
        }
        let (tile, recon, (recorder, _)) = writer
            .finish_with_filters()
            .map_err(coding("tile finish"))?;
        let pass = FirstPass {
            core,
            tile,
            recon,
            recorder,
            decisions,
        };
        let FilteredFrame { core, tile, recon } =
            finish_key_frame(&self.sequence, &self.view, self.preset, source, pass)?;
        let temporal_unit = encode_key_frame_temporal_unit(&self.sequence, &core, true, &tile)
            .map_err(coding("temporal unit"))?;
        Ok(CodedFrame {
            temporal_unit,
            recon: visible_planes(&recon)?,
            frame_type: FrameType::Key,
        })
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

/// Source planes extended by edge replication to cover every coded block.
pub(crate) struct PaddedSource {
    planes: [Vec<u16>; 3],
    strides: [usize; 3],
    visible: [PlaneSize; 3],
}

impl PaddedSource {
    fn new(source: &SourceFrame, width: usize, height: usize) -> Self {
        let sizes = source.plane_sizes();
        let planes = core::array::from_fn(|plane| {
            let size = sizes[plane];
            let (w, h) = if plane == 0 {
                (width, height)
            } else {
                (width.div_ceil(2), height.div_ceil(2))
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
        let strides = [width, width.div_ceil(2), width.div_ceil(2)];
        Self {
            planes,
            strides,
            visible: sizes,
        }
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
        let size = self.visible[tx.plane];
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

/// Transforms and quantizes the source residual of each transform block, by
/// RDOQ when the preset asks for it, else by a deadzone quantizer, and picks
/// each luma transform type by rate and distortion.
pub(crate) struct SourceResidualCoder<'a> {
    pub(crate) source: &'a PaddedSource,
    qindex: u32,
    bit_depth: BitDepth,
    preset: &'static PresetParams,
    lambda: Lambda,
    /// Quantizes with the deadzone even when the preset asks for RDOQ: a
    /// candidate trial only ranks choices; RDOQ runs in a leaf's final trial
    /// and in the stream.
    pub(crate) candidate: bool,
}

impl ResidualCoder for SourceResidualCoder<'_> {
    /// The offered type of the preset's leading `tx_types` with the lowest
    /// `D + lambda * R`, with the exact coefficient and type rate.
    fn code_luma(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        tx_types: &[TxType],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let count = tx_choice::tx_type_candidates(self.preset).min(tx_types.len());
        let mut best: Option<(u128, TransformDecision)> = None;
        for &tx_type in &tx_types[..count] {
            let tx = TransformBlock { tx_type, ..*tx };
            let decision = self.code(syntax, &tx, prediction)?;
            if count == 1 {
                return Ok(decision);
            }
            if let Some(cost) = self.luma_cost(syntax, &tx, prediction, &decision.levels)?
                && best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost)
            {
                best = Some((cost, decision));
            }
        }
        best.map(|(_, decision)| decision)
            .ok_or(TileWriterError::Shape("no luma transform type"))
    }

    fn code(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let (source, stride) = self.source.at(tx);
        let residual: Vec<i32> = source
            .chunks(stride)
            .zip(prediction.chunks(w))
            .take(h)
            .flat_map(|(src, pred)| {
                src.iter()
                    .zip(pred)
                    .map(|(&s, &p)| i32::from(s) - i32::from(p))
            })
            .collect();
        let (params, transform) = recon_params(
            self.qindex,
            plane_id(tx.plane)?,
            (log2_w, log2_h),
            tx.tx_type,
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
        let len = forward.coeff_width() * forward.coeff_height();
        let mut coeffs = vec![0; len];
        kernels()
            .forward_transform_2d(&forward, &residual, &mut coeffs)
            .map_err(kernel)?;
        let mut levels = vec![0; len];
        let mut dequant = vec![0; len];
        // shortcut: RDOQ walks the 2D-class scan only; IDTX and 1D types take the deadzone.
        if self.preset.rdoq && !self.candidate && (tx.tx_type as u8) < TxType::Idtx as u8 {
            let block = RdoqBlock {
                dc_q2: params.dc_quant,
                ac_q2: params.ac_quant,
                dq_denom: params.dq_denom,
                bit_depth: self.bit_depth,
                log2_width: forward.coeff_width().ilog2(),
                log2_height: forward.coeff_height().ilog2(),
                plane: tx.plane,
            };
            let lambda_q8 = self.lambda.coefficient_q8(params.dq_denom);
            let mut rates = CdfCoeffRates { syntax, tx };
            rdoq(
                kernels(),
                &block,
                &coeffs,
                &mut rates,
                lambda_q8,
                &mut levels,
                &mut dequant,
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
                .quantize(&quantizer, &coeffs, &mut levels, &mut dequant)
                .map_err(kernel)?;
        }
        Ok(TransformDecision {
            tx_type: tx.tx_type,
            levels,
        })
    }
}

impl SourceResidualCoder<'_> {
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
        {
            return Ok(None);
        }
        let recon = tx_choice::reconstruct(prediction, tx, levels, (self.qindex, self.bit_depth))?;
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
fn visible_planes(recon: &DecodedFrame<u16>) -> Result<[Vec<u16>; 3]> {
    let view = recon.as_frame_ref();
    let mut planes: [Vec<u16>; 3] = Default::default();
    for (out, id) in planes.iter_mut().zip([PlaneId::Y, PlaneId::U, PlaneId::V]) {
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
