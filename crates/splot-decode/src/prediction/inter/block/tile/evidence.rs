// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::PlaneId;

use super::{BlockDecodeContext, DecodeBlockFrontier, EntryState, LeafSlot};
use crate::block_evidence::{
    BlockEvidenceTile, BlockPrediction, DecodedTransformUnit, TransformCoefficientContext,
};
use crate::prediction::inter::{InterResidual, InterResidualBlock};

#[cold]
pub(super) fn observe_leaf(
    evidence: BlockEvidenceTile<'_>,
    context: &BlockDecodeContext<'_>,
    frontier: &DecodeBlockFrontier,
) {
    let Some(entry) = context.entries.last() else {
        return;
    };
    if let EntryState::Intra(command) = &entry.state
        && evidence.wants_transforms()
    {
        command.record_transforms(
            context.arena,
            evidence,
            frontier,
            &TransformCoefficientContext {
                arena: context.sink.residual_arena,
                qindex: command.qindex(),
                segment_id: command.segment_id(),
                bit_depth: context.sink.params.bit_depth,
            },
        );
    }
    let residual = match entry.slot(context.sink.slots) {
        Some(LeafSlot::PendingInter(command)) => {
            if evidence.wants_blocks() {
                evidence.record_block(
                    frontier,
                    context.evidence_parent,
                    command.segment_id,
                    command.qindex,
                    command.prediction(context.sink.core, &context.sink.params.ref_facts),
                );
            }
            command
                .placed
                .block
                .residual
                .as_ref()
                .map(|residual| (residual, command.qindex, command.segment_id))
        }
        Some(LeafSlot::PendingIntrabc(command)) => {
            if evidence.wants_blocks() {
                evidence.record_block(
                    frontier,
                    context.evidence_parent,
                    command.segment_id,
                    command.qindex,
                    BlockPrediction::IntraBlockCopy,
                );
            }
            command
                .residual
                .as_ref()
                .map(|residual| (residual, command.qindex, command.segment_id))
        }
        _ => None,
    };
    if evidence.wants_transforms()
        && let Some((residual, qindex, segment_id)) = residual
    {
        residual.record_transforms(
            context.sink.residual_blocks,
            evidence,
            frontier,
            &TransformCoefficientContext {
                arena: context.sink.residual_arena,
                qindex,
                segment_id,
                bit_depth: context.sink.params.bit_depth,
            },
        );
    }
}

impl InterResidual {
    fn record_transforms(
        &self,
        arena: &[InterResidualBlock],
        evidence: BlockEvidenceTile<'_>,
        frontier: &DecodeBlockFrontier,
        context: &TransformCoefficientContext<'_>,
    ) {
        let blocks = &arena[self.block_range.start as usize..self.block_range.end as usize];
        for (index, unit) in blocks.iter().enumerate() {
            let cctx_u = if unit.cctx_pair_delta < 0 {
                index
                    .checked_add_signed(isize::from(unit.cctx_pair_delta))
                    .and_then(|u| blocks.get(u))
                    .map(|u| &u.coeffs)
            } else {
                (unit.plane == PlaneId::U).then_some(&unit.coeffs)
            };
            evidence.record_transform(
                frontier,
                &DecodedTransformUnit {
                    plane: unit.plane,
                    x: usize::from(unit.x),
                    y: usize::from(unit.y),
                    tx_size: unit.tx_size,
                    coeffs: &unit.coeffs,
                    cctx_u,
                },
                context,
            );
        }
    }
}
