// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{BlockDecodeContext, DecodeBlockFrontier, EntryState, LeafSlot};
use crate::block_evidence::{BlockEvidenceTile, BlockPrediction};

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
        command.record_transforms(context.arena, evidence, frontier);
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
            command.placed.block.residual.as_ref()
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
            command.residual.as_ref()
        }
        _ => None,
    };
    if evidence.wants_transforms()
        && let Some(residual) = residual
    {
        residual.record_transforms(context.sink.residual_blocks, evidence, frontier);
    }
}
