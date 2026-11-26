// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{PlaneId, PlaneRect};

use super::BlockEvidenceId;
use crate::bitstream::tile_payload::LumaCoeffBlock;
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

pub(crate) struct DecodedTransformUnit<'a> {
    pub(crate) plane: PlaneId,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: u8,
    pub(crate) coeffs: &'a LumaCoeffBlock,
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
}
