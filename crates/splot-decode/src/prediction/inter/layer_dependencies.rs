// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal and embedded layer validation for coded inter, SEF, and TIP frames (§ 6.2.2 / § 6.17.2).

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::{FrameHeaderCore, FrameReferenceStateView};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, TemporalLayerId};

use super::{ReferenceMetadata, malformed_frame_header};
use crate::Result;

impl ReferenceMetadata {
    pub(crate) fn slot_tlayer_id(&self, slot: usize) -> Option<TemporalLayerId> {
        self.ref_tlayer_id
            .get(slot)
            .copied()
            .map(TemporalLayerId::from_bits)
    }

    pub(crate) fn slot_mlayer_id(&self, slot: usize) -> Option<EmbeddedLayerId> {
        self.ref_mlayer_id
            .get(slot)
            .copied()
            .map(EmbeddedLayerId::from_bits)
    }

    pub(crate) fn header_view(&self) -> FrameReferenceStateView<'_> {
        let view = FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.ref_valid,
            &self.ref_order_hint,
            &self.ref_frame_width,
            &self.ref_frame_height,
            &self.ref_base_q_idx,
        )
        .with_quantizer_delta_state(&self.ref_chroma_ac_deltas)
        .with_primary_reference_state(&self.ref_counter, &self.ref_is_inter)
        .with_long_term_id_state(&self.ref_long_term_id)
        .with_global_motion_state(
            &self.ref_num_total_refs,
            &self.saved_global_motion_order_hints,
            &self.saved_global_motion_params,
        )
        .with_single_layer_order_hint_state(
            &self.ref_order_hint_lsbs,
            &self.ref_implicit_output_frame,
            &self.ref_immediate_output_frame,
        )
        .with_lr_frame_filter_class_counts(&self.lr_frame_filter_class_counts)
        .with_lr_frame_filter_taps(self.lr_frame_filter_taps.as_deref().unwrap_or_default())
        .with_allowed_frames(self.allowed_frames);
        let view = if self.ref_tlayer_id.is_empty() {
            view
        } else {
            view.with_tlayer_id_state(&self.ref_tlayer_id)
        };
        if self.ref_mlayer_id.is_empty() {
            view
        } else {
            view.with_mlayer_id_state(&self.ref_mlayer_id)
        }
    }
}

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
