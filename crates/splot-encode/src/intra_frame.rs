// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Key-frame coding: partition choice, SATD intra mode choice, residual
//! coding, and the temporal unit of one intra frame.

use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams,
    build_encoder_sequence_header, build_key_frame_header, encode_key_frame_temporal_unit,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_decode::encoder_syntax::{
    IntraTileSyntax, LeafBlock, MAX_CFL_ALPHA, PartitionNode, TransformBlock, dct_recon_params,
    recon_params, tx_log2,
};
use splot_recon::{BitDepth, DecodedFrame, PlaneId};

use crate::config::Config;
use crate::encoder::{CodedFrame, FrameCoder};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::kernels::{BlockDims, ForwardTransform2d, Quantizer, kernels};
use crate::loop_filter::{FilteredFrame, FirstPass, FrameDecisions, finish_key_frame};
use crate::preset::PresetParams;
use crate::rate_control::{FramePlanInfo, FrameType};
use crate::tile_writer::{
    ChromaMode, INTRA_MODES, IntraModes, KeyFrameTileWriter, PARTITIONS, Partition, PartitionTree,
    ResidualCoder, SuperblockDecision, TileWriterError, TransformDecision,
};

/// Quantizer deadzone in Q8 (`128` rounds to nearest).
const DEADZONE_ROUNDING_Q8: u32 = 96;
/// The largest `|AngleDeltaY|` (§ 5.20.5.5).
const MAX_ANGLE_DELTA: i8 = 3;

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
        let core = build_key_frame_header(&self.view, qindex, 0, &FrameFilterParams::default())
            .map_err(coding("frame header"))?;
        let mut writer =
            KeyFrameTileWriter::new(&self.sequence, &core).map_err(coding("tile setup"))?;
        let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
        let sb = writer.syntax().superblock_size4() * 4;
        let padded = PaddedSource::new(source, mi_cols * 4 + sb, mi_rows * 4 + sb);
        let split_variance = split_variance(qindex, self.bit_depth)?;
        let mut coder = SourceResidualCoder {
            source: &padded,
            qindex,
            bit_depth: self.bit_depth,
        };
        let mut decisions = FrameDecisions::default();
        while let Some(root) = writer.next_superblock_root() {
            let tree = choose_tree(writer.syntax(), root, &padded, self.preset, split_variance)
                .map_err(coding("partition choice"))?;
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
struct PaddedSource {
    planes: [Vec<u16>; 3],
    strides: [usize; 3],
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
        Self { planes, strides }
    }

    /// Variance times area of the luma block at `(x, y)` of `w x h` samples.
    fn luma_variance_sum(&self, x: usize, y: usize, w: usize, h: usize) -> u64 {
        let stride = self.strides[0];
        let (mut sum, mut sum_sq) = (0u64, 0u64);
        for row in self.planes[0][y * stride + x..].chunks(stride).take(h) {
            for &sample in &row[..w] {
                sum += u64::from(sample);
                sum_sq += u64::from(sample) * u64::from(sample);
            }
        }
        sum_sq - sum * sum / (w * h) as u64
    }
}

/// The per-sample variance above which a block splits: a quarter of the luma
/// AC step in sample units, squared (tuned for BD-rate on CIF content).
fn split_variance(qindex: u32, bit_depth: BitDepth) -> Result<u64> {
    let (params, _) =
        dct_recon_params(qindex, PlaneId::Y, 3, 3, bit_depth).map_err(coding("quantizer step"))?;
    let step = u64::from(params.ac_quant) / 64;
    Ok((step * step / 16).max(1))
}

/// Splits a block while it is above the preset's largest size, or while its
/// source variance exceeds `split_variance` and it is above the smallest size.
fn choose_tree(
    syntax: &IntraTileSyntax,
    node: PartitionNode,
    source: &PaddedSource,
    preset: &PresetParams,
    split_variance: u64,
) -> core::result::Result<PartitionTree, TileWriterError> {
    if !syntax.in_frame(node) {
        return Ok(PartitionTree::Leaf(IntraModes::DC));
    }
    let options = syntax.partition_options(node)?;
    let allowed: Vec<Partition> = PARTITIONS
        .into_iter()
        .filter(|&p| options.allows(p))
        .collect();
    let partition = match options.implied().filter(|&p| options.allows(p)) {
        Some(implied) => implied,
        None if allowed.len() == 1 => allowed[0],
        None => {
            let (w, h) = node.dimensions()?;
            let log2 = w.max(h).ilog2();
            let detailed = || {
                let (x, y) = (node.col4() * 4, node.row4() * 4);
                source.luma_variance_sum(x, y, w, h) > split_variance * (w * h) as u64
            };
            let split = log2 > u32::from(preset.max_block_log2)
                || (log2 > u32::from(preset.min_block_log2) && detailed());
            if !split && options.allows(Partition::None) {
                Partition::None
            } else {
                flattest_split(syntax, node, source, &allowed)?
            }
        }
    };
    if partition == Partition::None {
        return Ok(PartitionTree::Leaf(IntraModes::DC));
    }
    let children = syntax
        .children(node, partition)?
        .into_iter()
        .map(|child| choose_tree(syntax, child, source, preset, split_variance))
        .collect::<core::result::Result<_, _>>()?;
    Ok(PartitionTree::Split {
        kind: partition,
        children,
    })
}

/// The allowed quad or binary split with the smallest largest child side, then
/// the lowest total child luma variance. Below the root AV2 often allows only
/// binary splits; preferring short sides avoids 64-point transforms, which drop
/// the upper half of the frequencies.
fn flattest_split(
    syntax: &IntraTileSyntax,
    node: PartitionNode,
    source: &PaddedSource,
    allowed: &[Partition],
) -> core::result::Result<Partition, TileWriterError> {
    let mut best: Option<((usize, u64), Partition)> = None;
    for &partition in allowed {
        if !matches!(
            partition,
            Partition::Split | Partition::Horz | Partition::Vert
        ) {
            continue;
        }
        let mut cost = (0, 0);
        for child in syntax.children(node, partition)? {
            if syntax.in_frame(child) {
                let (w, h) = child.dimensions()?;
                cost.0 = cost.0.max(w.max(h));
                cost.1 += source.luma_variance_sum(child.col4() * 4, child.row4() * 4, w, h);
            }
        }
        if best.is_none_or(|(best_cost, _)| cost < best_cost) {
            best = Some((cost, partition));
        }
    }
    match best {
        Some((_, partition)) => Ok(partition),
        None => allowed
            .first()
            .copied()
            .ok_or(TileWriterError::Shape("no allowed partition")),
    }
}

/// Transforms and quantizes the source residual of each transform block.
struct SourceResidualCoder<'a> {
    source: &'a PaddedSource,
    qindex: u32,
    bit_depth: BitDepth,
}

impl ResidualCoder for SourceResidualCoder<'_> {
    fn code(
        &mut self,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> core::result::Result<TransformDecision, TileWriterError> {
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let stride = self.source.strides[tx.plane];
        let source = &self.source.planes[tx.plane][tx.y * stride + tx.x..];
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
        let quantizer = Quantizer::new(
            params.dc_quant,
            params.ac_quant,
            params.dq_denom,
            self.bit_depth,
            DEADZONE_ROUNDING_Q8,
        )
        .map_err(kernel)?;
        let mut levels = vec![0; len];
        let mut dequant = vec![0; len];
        kernels()
            .quantize(&quantizer, &coeffs, &mut levels, &mut dequant)
            .map_err(kernel)?;
        Ok(TransformDecision {
            tx_type: tx.tx_type,
            levels,
        })
    }

    /// The luma modes with the lowest prediction SATD.
    fn luma_modes(
        &mut self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        planned: IntraModes,
    ) -> core::result::Result<IntraModes, TileWriterError> {
        let mut best = (u64::MAX, planned);
        for y_mode in INTRA_MODES {
            let deltas = if y_mode.is_directional() {
                -MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA
            } else {
                0..=0
            };
            for angle_delta in deltas {
                let modes = IntraModes {
                    y_mode,
                    angle_delta,
                    ..planned
                };
                let cost = self.prediction_satd(syntax, leaf, modes, false)?[0];
                if cost < best.0 {
                    best = (cost, modes);
                }
            }
        }
        Ok(best.1)
    }

    /// The chroma mode with the lowest prediction SATD; explicit CFL alphas are
    /// chosen per plane.
    fn chroma_mode(
        &mut self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> core::result::Result<ChromaMode, TileWriterError> {
        let mut candidates: Vec<ChromaMode> = INTRA_MODES.map(ChromaMode::Intra).to_vec();
        let mut alpha_u = (u64::MAX, 0);
        let mut alpha_v = (u64::MAX, 0);
        if syntax.cfl_allowed(leaf) {
            candidates.push(ChromaMode::CflDerived);
            for alpha in (-MAX_CFL_ALPHA..=MAX_CFL_ALPHA).filter(|&alpha| alpha != 0) {
                let uv_mode = ChromaMode::Cfl {
                    alpha_u: alpha,
                    alpha_v: alpha,
                };
                let cost =
                    self.prediction_satd(syntax, leaf, IntraModes { uv_mode, ..modes }, true)?;
                alpha_u = alpha_u.min((cost[1], alpha));
                alpha_v = alpha_v.min((cost[2], alpha));
            }
        }
        let mut best = (
            alpha_u.0.saturating_add(alpha_v.0),
            ChromaMode::Cfl {
                alpha_u: alpha_u.1,
                alpha_v: alpha_v.1,
            },
        );
        for uv_mode in candidates {
            let cost = self.prediction_satd(syntax, leaf, IntraModes { uv_mode, ..modes }, true)?;
            if cost[1] + cost[2] < best.0 {
                best = (cost[1] + cost[2], uv_mode);
            }
        }
        Ok(best.1)
    }
}

impl SourceResidualCoder<'_> {
    /// The per-plane SATD between the source and the prediction of the luma or
    /// the chroma blocks of `leaf` under `modes`.
    fn prediction_satd(
        &self,
        syntax: &mut IntraTileSyntax,
        leaf: &LeafBlock,
        modes: IntraModes,
        chroma: bool,
    ) -> core::result::Result<[u64; 3], TileWriterError> {
        let mut cost = [0; 3];
        for (index, tx) in syntax.plan_leaf(leaf, modes)?.iter().enumerate() {
            if (tx.plane > 0) != chroma {
                continue;
            }
            let prediction = syntax.predict(index)?;
            let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
            let stride = self.source.strides[tx.plane];
            cost[tx.plane] += kernels().satd(
                &self.source.planes[tx.plane][tx.y * stride + tx.x..],
                stride,
                &prediction,
                1 << log2_w,
                BlockDims::new(1 << log2_w, 1 << log2_h).map_err(kernel)?,
            );
        }
        Ok(cost)
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
