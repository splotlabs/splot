// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{BlockDecodeContext, DecodeBlockFrontier, EntryState, LeafSlot};
use crate::block_evidence::{BlockEvidenceTile, BlockPrediction, TransformCoefficientContext};

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
