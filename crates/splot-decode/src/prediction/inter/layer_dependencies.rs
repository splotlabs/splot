// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal and embedded layer validation for coded inter, SEF, and TIP frames (§ 6.2.2 / § 6.17.2).

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;

use super::{ReferenceMetadata, malformed_frame_header};
use crate::Result;

pub(super) fn validate_obu_layer_ids(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    frame_index: Option<usize>,
) -> Result<()> {
    if envelope.header.embedded_layer_id > sequence.general.max_mlayer_id {
        return Err(malformed_frame_header(
            envelope.offset,
            frame_index,
            "6.2.2",
            format!(
                "OBU mlayer_id {} exceeds max_mlayer_id {}",
                envelope.header.embedded_layer_id.get(),
                sequence.general.max_mlayer_id.get()
            ),
        ));
    }
    if envelope.header.temporal_layer_id > sequence.general.max_tlayer_id {
        return Err(malformed_frame_header(
            envelope.offset,
            frame_index,
            "6.2.2",
            format!(
                "OBU temporal_layer_id {} exceeds max_tlayer_id {}",
                envelope.header.temporal_layer_id.get(),
                sequence.general.max_tlayer_id.get()
            ),
        ));
    }
    Ok(())
}

fn validate_slot_layer_dependencies(
    header: ObuHeader,
    slot_idx: usize,
    kind: &str,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    if let Some(slot_mlayer) = reference.slot_mlayer_id(slot_idx)
        && !sequence
            .general
            .mlayer_dependency_map
            .depends_on(header.embedded_layer_id, slot_mlayer)
    {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            "6.17.2",
            format!(
                "{kind} slot {slot_idx} has embedded layer {} disallowed by MLayerDependencyMap for embedded layer {}",
                slot_mlayer.get(),
                header.embedded_layer_id.get()
            ),
        ));
    }
    if let Some(slot_tlayer) = reference.slot_tlayer_id(slot_idx)
        && !sequence.general.tlayer_dependency_map.depends_on(
            header.embedded_layer_id,
            header.temporal_layer_id,
            slot_tlayer,
        )
    {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            "6.17.2",
            format!(
                "{kind} slot {slot_idx} has temporal layer {} disallowed by TLayerDependencyMap for temporal layer {}",
                slot_tlayer.get(),
                header.temporal_layer_id.get()
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_sef_layer_dependency(
    header: ObuHeader,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    let Some(slot) = core.frame_to_show_map_idx else {
        return Ok(());
    };
    let slot_idx = usize::try_from(slot).unwrap_or(usize::MAX);
    if reference.ref_valid.get(slot_idx) != Some(&true) {
        return Err(malformed_frame_header(
            offset,
            frame_index,
            "6.17.2",
            format!("show-existing-frame reference slot {slot} is not valid"),
        ));
    }
    validate_slot_layer_dependencies(
        header,
        slot_idx,
        "show-existing-frame",
        sequence,
        reference,
        offset,
        frame_index,
    )
}

pub(super) fn validate_inter_layer_dependencies(
    header: ObuHeader,
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    reference: &ReferenceMetadata,
    offset: ByteOffset,
    frame_index: Option<usize>,
) -> Result<()> {
    if core.frame_is_intra == Some(true) {
        return Ok(());
    }
    let Some(inter) = core.inter.as_ref() else {
        return Ok(());
    };
    let count = usize::try_from(inter.num_total_refs.unwrap_or(0)).unwrap_or(usize::MAX);
    for &slot in inter.ref_frame_idx.iter().take(count) {
        let slot_idx = slot as usize;
        if reference.ref_valid.get(slot_idx) != Some(&true) {
            continue;
        }
        validate_slot_layer_dependencies(
            header,
            slot_idx,
            "reference",
            sequence,
            reference,
            offset,
            frame_index,
        )?;
    }
    Ok(())
}
