// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, PlaneId, PlaneRect, coefficient_scan_slice, dequantize_block, tx_class,
};

use super::{BlockEvidenceId, BlockEvidenceStatus};
use crate::bitstream::tile_payload::{
    CoeffBlock, CoefficientCoding, FrameQmSegmentScope, LumaCoeffBlock, dequant_block_params,
};
use crate::encoder_syntax::TxType;
use crate::tile::block_context::TxShape;

/// A transform unit's identity within a captured coded frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BlockTransformId {
    /// The coding block that parsed this transform unit.
    pub block: BlockEvidenceId,
    /// The plane of the transform unit.
    pub plane: PlaneId,
    /// The full transform rectangle in samples of this plane.
    pub coded: PlaneRect,
}

impl BlockTransformId {
    /// Identifies a transform by its coding block, plane and native rectangle.
    #[must_use]
    pub const fn new(block: BlockEvidenceId, plane: PlaneId, coded: PlaneRect) -> Self {
        Self {
            block,
            plane,
            coded,
        }
    }
}

/// Geometry observed in the residual parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockTransformEvidence {
    /// The owning block, plane and native rectangle.
    pub id: BlockTransformId,
    /// The full transform rectangle projected to coded luma coordinates.
    pub luma_coded: PlaneRect,
    /// Transform details from an exact selection.
    pub details: Option<BlockTransformDetails>,
}

/// Transform facts recorded for a selected unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockTransformDetails {
    /// The parser found a nonzero end-of-block position for this plane.
    pub has_coefficients: bool,
    /// The AV2 `PlaneTxType` when the plane codes coefficients.
    pub plane_tx_type: Option<TxType>,
    /// The unit belongs to a lossless segment.
    pub lossless: bool,
}

/// The coefficient domain before inverse transforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockCoefficientDomain {
    /// Coefficients for the native plane in the transform identity.
    Plane,
    /// A CCTX component stored in the U or V transform slot.
    CrossComponent {
        /// The pair's nonzero AV2 `cctx_type`, signaled in the U slot.
        mode: u8,
    },
}

/// The matrix selected by AV2 § 7.14.4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockCoefficientMatrix {
    /// The effective matrix level for this plane and transform geometry.
    pub level: usize,
    /// The decoder used the user-defined matrix at this level.
    pub user_defined: bool,
}

/// The resolved inputs to coefficient dequantization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockCoefficientQuantization {
    /// The effective block index before plane-specific adjustments.
    pub qindex: u32,
    /// The DC quantizer from AV2 § 7.14.2, in eighths before matrix weighting.
    pub dc_quant: u32,
    /// The AC quantizer from AV2 § 7.14.2, in eighths before matrix weighting.
    pub ac_quant: u32,
    /// The effective AV2 § 7.14.4 divisor.
    pub denominator: u32,
    /// The bit depth that determines the dequantized clipping range.
    pub bit_depth: BitDepth,
    /// The active matrix, indexed by row and column in the coefficient grid.
    pub matrix: Option<BlockCoefficientMatrix>,
}

/// The secondary transform syntax for an intra luma transform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockSecondaryTransform {
    /// The nonzero signaled AV2 `sec_tx_type`.
    pub sec_tx_type: u8,
    /// The signaled `most_probable_stx_set`, when present.
    pub most_probable_stx_set: Option<u8>,
}

/// Exact signed coefficient grids from one selected transform.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockTransformCoefficients {
    /// The transform identity within this report's frame.
    pub id: BlockTransformId,
    /// The transform width in coefficients, limited to 32.
    pub width: usize,
    /// The transform height in coefficients, limited to 32.
    pub height: usize,
    /// The decoded AV2 `Quant` integers, including implicit zeros.
    /// Index as `y * width + x`, with DC at index zero.
    pub quantized: Vec<i32>,
    /// The AV2 § 7.14.4 `Dequant` integers before CCTX or secondary transforms.
    /// Index as `y * width + x`. The values have unit scale.
    pub dequantized: Vec<i32>,
    /// AV2 scan indices mapped to raster indices. An all-zero unit has no scan.
    pub scan: Option<&'static [u16]>,
    /// The decoded extent. Ordinary coding uses a prefix of the scan.
    /// Forward skip coding uses a suffix of this length.
    pub eob: usize,
    /// The component domain of both grids.
    pub domain: BlockCoefficientDomain,
    /// The resolved dequantization inputs, when the unit codes coefficients.
    pub quantization: Option<BlockCoefficientQuantization>,
    /// The coefficient parser used trellis-coded quantization.
    pub use_tcq: bool,
    /// The active secondary transform syntax, when present.
    pub intra_secondary: Option<BlockSecondaryTransform>,
    /// The coefficient parser used forward skip coding.
    pub forward_scan: bool,
    /// The parser derived the DC magnitude parity from the other levels.
    pub dc_parity_hidden: bool,
}

pub(crate) struct TransformCoefficientContext<'a> {
    pub(crate) arena: &'a [i32],
    pub(crate) qindex: u32,
    pub(crate) segment_id: u8,
    pub(crate) bit_depth: BitDepth,
}

pub(crate) struct DecodedTransformUnit<'a> {
    pub(crate) plane: PlaneId,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: u8,
    pub(crate) coeffs: &'a LumaCoeffBlock,
    pub(crate) cctx_u: Option<&'a LumaCoeffBlock>,
}

impl DecodedTransformUnit<'_> {
    pub(crate) fn to_evidence(
        &self,
        block: BlockEvidenceId,
        subsampling: (bool, bool),
    ) -> Option<BlockTransformEvidence> {
        let shape = TxShape::from_tx_size(usize::from(self.tx_size))?;
        let width = 1 << shape.width_log2();
        let height = 1 << shape.height_log2();
        let coded = PlaneRect::new(self.x, self.y, width, height).ok()?;
        let luma_coded = if self.plane == PlaneId::Y {
            coded
        } else {
            let (sx, sy) = (usize::from(subsampling.0), usize::from(subsampling.1));
            PlaneRect::new(self.x << sx, self.y << sy, width << sx, height << sy).ok()?
        };
        Some(BlockTransformEvidence {
            id: BlockTransformId::new(block, self.plane, coded),
            luma_coded,
            details: None,
        })
    }

    pub(crate) fn details(&self) -> BlockTransformDetails {
        let has_coefficients = self.coeffs.eob() != 0;
        BlockTransformDetails {
            has_coefficients,
            plane_tx_type: if has_coefficients {
                TxType::from_ordinal(self.coeffs.plane_tx_type())
            } else {
                None
            },
            lossless: self.coeffs.lossless,
        }
    }

    pub(super) fn capture_coefficients(
        &self,
        id: BlockTransformId,
        context: &TransformCoefficientContext<'_>,
    ) -> Result<BlockTransformCoefficients, BlockEvidenceStatus> {
        let width = id.coded.width().min(32);
        let height = id.coded.height().min(32);
        let len = width * height;
        let mut quantized = zero_grid(len)?;
        let mut dequantized = zero_grid(len)?;
        let eob = self.coeffs.eob();
        let cctx = self
            .cctx_u
            .and_then(|u| u.cctx_type.filter(|&mode| mode != 0).map(|mode| (u, mode)));
        let domain = cctx.map_or(BlockCoefficientDomain::Plane, |(_, mode)| {
            BlockCoefficientDomain::CrossComponent { mode }
        });
        let mut scan = None;
        let mut quantization = None;
        if eob != 0 {
            let block = CoeffBlock::new(self.coeffs, context.arena)
                .map_err(|_| BlockEvidenceStatus::Unavailable)?;
            let scan_map = coefficient_scan_slice(width, height, tx_class(block.plane_tx_type()))
                .map_err(|_| BlockEvidenceStatus::Unavailable)?;
            if block.scan_order {
                let positions = scan_map
                    .get(..block.quant.len())
                    .ok_or(BlockEvidenceStatus::Unavailable)?;
                for (&value, &position) in block.quant.iter().zip(positions) {
                    quantized[usize::from(position)] = value;
                }
            } else {
                quantized
                    .get_mut(..block.quant.len())
                    .ok_or(BlockEvidenceStatus::Unavailable)?
                    .copy_from_slice(block.quant);
            }
            scan = Some(scan_map);
            let _segment_scope = FrameQmSegmentScope::install(usize::from(context.segment_id));
            let params = dequant_block_params(
                context.qindex,
                id.plane,
                (id.coded.width().ilog2(), id.coded.height().ilog2()),
                cctx.map_or_else(|| block.plane_tx_type(), |(u, _)| u.plane_tx_type()),
                block.use_tcq,
                context.bit_depth,
            );
            dequantize_block(&params, &quantized, &mut dequantized)
                .map_err(|_| BlockEvidenceStatus::Unavailable)?;
            quantization = Some(BlockCoefficientQuantization {
                qindex: context.qindex,
                dc_quant: params.dc_quant,
                ac_quant: params.ac_quant,
                denominator: params.dq_denom,
                bit_depth: params.bit_depth,
                matrix: params.qm.map(|qm| BlockCoefficientMatrix {
                    level: qm.seg_level,
                    user_defined: qm.user.is_some(),
                }),
            });
        }
        Ok(BlockTransformCoefficients {
            id,
            width,
            height,
            quantized,
            dequantized,
            scan,
            eob,
            domain,
            quantization,
            use_tcq: self.coeffs.use_tcq,
            intra_secondary: self
                .coeffs
                .intra_ist
                .filter(|ist| ist.sec_tx_type != 0)
                .map(|ist| BlockSecondaryTransform {
                    sec_tx_type: ist.sec_tx_type,
                    most_probable_stx_set: ist.most_probable_stx_set,
                }),
            forward_scan: self.coeffs.coding == CoefficientCoding::ForwardSkip,
            dc_parity_hidden: self.coeffs.coding == CoefficientCoding::HiddenDcParity,
        })
    }
}

fn zero_grid(len: usize) -> Result<Vec<i32>, BlockEvidenceStatus> {
    let mut grid = Vec::new();
    grid.try_reserve_exact(len)
        .map_err(|_| BlockEvidenceStatus::AllocationFailed)?;
    grid.resize(len, 0);
    Ok(grid)
}
