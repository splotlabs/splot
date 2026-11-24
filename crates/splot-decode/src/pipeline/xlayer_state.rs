// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-extended-layer decode state and global high-level syntax coordination.

use std::collections::BTreeMap;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::layer_config_record::LayerConfigurationRecord;
use splot_core::headers::metadata::{MetadataType, MetadataUnit};
use splot_core::headers::operating_point_set::OperatingPointSet;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType};

use super::frame_lifecycle::FilmGrainSlots;
use super::output_effects::{
    MetadataAction, MetadataPersistence, OutputEffectState, parse_metadata_obu, parse_ops_obu,
    parse_validated_lcr,
};
use super::output_schedule::OutputScheduler;
use super::unsupported_feature_at;
use crate::bitstream::stream_plan::DecodeSourceIssue;
use crate::error::{DecodeError, Result};
use crate::reference::buffer::RuntimeReferenceBuffer;

#[derive(Default)]
pub(super) struct InBandLongTermPrelude {
    pub(super) frames: Vec<InBandLongTermFrame>,
}

pub(super) struct InBandLongTermFrame {
    pub(super) obu_type: ObuType,
    pub(super) long_term_id: u32,
    pub(super) hidden: bool,
    pub(super) frame_index: usize,
}

impl InBandLongTermPrelude {
    pub(super) fn begin_frame(&mut self, first_picture_in_tu: bool) {
        if first_picture_in_tu {
            self.frames.clear();
        }
    }

    pub(super) fn validate_required(
        &self,
        core: &FrameHeaderCore,
        reference: &RuntimeReferenceBuffer,
        offset: ByteOffset,
    ) -> Result<()> {
        self.validate_required_with(&core.ref_long_term_ids, offset, |id, frame_index| {
            reference.retains_hidden_long_term_reference(id, frame_index)
        })
    }

    pub(super) fn validate_required_with(
        &self,
        required_ids: &[u32],
        offset: ByteOffset,
        mut is_retained: impl FnMut(u32, usize) -> bool,
    ) -> Result<()> {
        if required_ids.is_empty() {
            return Ok(());
        }
        for &id in required_ids {
            if !self.frames.iter().any(|frame| frame.long_term_id == id) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_missing",
                    offset,
                    "each ref_long_term_id must name a preceding in-band key frame in the random-access temporal unit",
                    "7.3.9.1",
                ));
            }
            if self
                .frames
                .iter()
                .any(|frame| frame.long_term_id == id && !frame.hidden)
            {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_visible",
                    offset,
                    "in-band long-term reference key frames must disable immediate and implicit output",
                    "7.3.9.1",
                ));
            }
            if !self.frames.iter().any(|frame| {
                frame.long_term_id == id && frame.hidden && is_retained(id, frame.frame_index)
            }) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_slot_unavailable",
                    offset,
                    "an in-band long-term reference must remain in the same valid reference slot used by sequential decoding",
                    "7.3.9.1",
                ));
            }
        }
        let mut saw_olk = false;
        for frame in self
            .frames
            .iter()
            .filter(|frame| required_ids.contains(&frame.long_term_id))
        {
            if frame.obu_type == ObuType::OpenLoopKey {
                saw_olk = true;
            } else if frame.obu_type == ObuType::ClosedLoopKey && saw_olk {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_order",
                    offset,
                    "in-band long-term CLK references must precede all in-band long-term OLK references",
                    "7.3.9.1",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn note_frame(&mut self, core: &FrameHeaderCore, frame_index: usize) {
        if !matches!(core.obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey) {
            return;
        }
        let Some(long_term_id) = core.long_term_id.and_then(|id| u32::try_from(id).ok()) else {
            return;
        };
        self.frames.push(InBandLongTermFrame {
            obu_type: core.obu_type,
            long_term_id,
            hidden: core.immediate_output_frame == Some(false)
                && core.implicit_output_frame == Some(false),
            frame_index,
        });
    }
}

#[derive(Clone)]
struct GlobalMetadataItem {
    persistence: MetadataPersistence,
    layer_idc: u8,
    source_mlayer: EmbeddedLayerId,
    xlayer_map: Option<u32>,
    mlayer_maps: Vec<u8>,
    unit: MetadataUnit,
    tu_index: u64,
}

impl GlobalMetadataItem {
    fn same_scope(&self, other: &Self) -> bool {
        self.layer_idc == other.layer_idc
            && self.source_mlayer == other.source_mlayer
            && self.xlayer_map == other.xlayer_map
            && self.mlayer_maps == other.mlayer_maps
    }

    fn to_action(&self, persistence_idc: u8) -> MetadataAction {
        MetadataAction::Store {
            layer_idc: self.layer_idc,
            persistence_idc,
            source_mlayer: self.source_mlayer,
            xlayer_map: self.xlayer_map,
            mlayer_maps: self.mlayer_maps.clone(),
            unit: self.unit.clone(),
        }
    }
}

#[derive(Default)]
struct GlobalMetadataEntry {
    item: Option<(u8, GlobalMetadataItem)>,
}

#[derive(Default)]
pub(super) struct GlobalHlsState {
    tu_index: u64,
    global_lcr_xlayers: [Option<u32>; 8],
    global_ops_counts: BTreeMap<u8, u8>,
    global_metadata: Vec<GlobalMetadataEntry>,
}

impl GlobalHlsState {
    fn begin_temporal_unit(&mut self) {
        self.tu_index = self.tu_index.saturating_add(1);
        self.global_metadata.retain(|entry| {
            entry
                .item
                .as_ref()
                .is_some_and(|(_, item)| item.persistence != MetadataPersistence::No)
        });
    }

    fn record_ops(&mut self, ops: &OperatingPointSet) {
        if ops.reset_flag {
            self.global_ops_counts.clear();
        }
        if ops.ops_cnt == 0 {
            self.global_ops_counts.remove(&ops.ops_id);
        } else {
            self.global_ops_counts.insert(ops.ops_id, ops.ops_cnt);
        }
    }

    fn cancel_metadata(&mut self, metadata_type: MetadataType) {
        self.global_metadata.retain(|entry| {
            entry.item.as_ref().is_some_and(|(_, item)| {
                item.unit.metadata_type != metadata_type
                    || item.persistence == MetadataPersistence::Global
            })
        });
    }

    fn record_metadata_action(&mut self, action: &MetadataAction) {
        match action {
            MetadataAction::Cancel(metadata_type) => self.cancel_metadata(*metadata_type),
            MetadataAction::Store {
                layer_idc,
                persistence_idc,
                source_mlayer,
                xlayer_map,
                mlayer_maps,
                unit,
            } => {
                let candidate = GlobalMetadataItem {
                    persistence: MetadataPersistence::from_idc(*persistence_idc),
                    layer_idc: *layer_idc,
                    source_mlayer: *source_mlayer,
                    xlayer_map: *xlayer_map,
                    mlayer_maps: mlayer_maps.clone(),
                    unit: unit.clone(),
                    tu_index: self.tu_index,
                };
                match candidate.persistence {
                    MetadataPersistence::Global => {
                        self.global_metadata.retain(|entry| {
                            entry.item.as_ref().is_some_and(|(_, item)| {
                                item.unit.metadata_type != candidate.unit.metadata_type
                                    || item.persistence != MetadataPersistence::Global
                            })
                        });
                    }
                    MetadataPersistence::Basic | MetadataPersistence::Enhanced => {
                        self.global_metadata.retain(|entry| {
                            entry.item.as_ref().is_some_and(|(_, item)| {
                                item.unit.metadata_type != candidate.unit.metadata_type
                                    || item.persistence == MetadataPersistence::Global
                                    || !item.same_scope(&candidate)
                            })
                        });
                    }
                    MetadataPersistence::No | MetadataPersistence::Reserved => {}
                }
                self.global_metadata.push(GlobalMetadataEntry {
                    item: Some((*persistence_idc, candidate)),
                });
            }
        }
    }

    fn sync_effects(&self, effects: &mut OutputEffectState) {
        effects.set_tu_index(self.tu_index);
        for (id, slot) in self.global_lcr_xlayers.iter().enumerate() {
            if let Some(xlayer_map) = *slot {
                effects.apply_global_lcr(id as u8, xlayer_map);
            }
        }
        for (&ops_id, &ops_cnt) in &self.global_ops_counts {
            effects.apply_ops_entry(GLOBAL_XLAYER_ID, ops_id, ops_cnt);
        }
        for entry in &self.global_metadata {
            if let Some((persistence_idc, item)) = &entry.item
                && (matches!(
                    item.persistence,
                    MetadataPersistence::Global
                        | MetadataPersistence::Basic
                        | MetadataPersistence::Enhanced
                ) || item.tu_index == self.tu_index)
            {
                effects.apply_metadata_action(GLOBAL_XLAYER_ID, item.to_action(*persistence_idc));
            }
        }
    }

    fn global_lcr_has_xlayer(&self, lcr_id: u8, xlayer: ExtendedLayerId) -> bool {
        self.global_lcr_xlayers
            .get(usize::from(lcr_id))
            .copied()
            .flatten()
            .is_some_and(|map| (map & (1u32 << u32::from(xlayer.get()))) != 0)
    }
}

pub(super) struct XLayerState {
    pub(super) xlayer_id: ExtendedLayerId,
    pub(super) sequence: SequenceHeader,
    pub(super) shared_sequence: Option<Arc<SequenceHeader>>,
    pub(super) reference: RuntimeReferenceBuffer,
    pub(super) scheduler: OutputScheduler,
    pub(super) output_effect_state: OutputEffectState,
    pub(super) film_grain_slots: FilmGrainSlots,
    pub(super) in_band_long_term_prelude: InBandLongTermPrelude,
    seen_frame_in_tu: bool,
    pub(super) decoding_initial_tu: bool,
    pub(super) discard_leading_frames: bool,
}

impl XLayerState {
    #[expect(clippy::too_many_arguments)]
    pub(super) fn new(
        xlayer_id: ExtendedLayerId,
        sequence: SequenceHeader,
        reference: RuntimeReferenceBuffer,
        scheduler: OutputScheduler,
        output_effect_state: OutputEffectState,
        film_grain_slots: FilmGrainSlots,
        in_band_long_term_prelude: InBandLongTermPrelude,
        discard_leading_frames: bool,
    ) -> Self {
        Self {
            xlayer_id,
            sequence,
            shared_sequence: None,
            reference,
            scheduler,
            output_effect_state,
            film_grain_slots,
            in_band_long_term_prelude,
            seen_frame_in_tu: true,
            decoding_initial_tu: true,
            discard_leading_frames,
        }
    }

    fn on_temporal_delimiter(&mut self, clear_discard: bool) {
        self.seen_frame_in_tu = false;
        self.decoding_initial_tu = false;
        if clear_discard {
            self.discard_leading_frames = false;
        }
        self.in_band_long_term_prelude.begin_frame(true);
        self.output_effect_state.begin_temporal_unit();
    }

    pub(super) fn begin_coded_picture(&mut self) -> bool {
        let first_picture_in_tu = !self.seen_frame_in_tu;
        self.seen_frame_in_tu = true;
        if first_picture_in_tu {
            self.decoding_initial_tu = false;
            self.discard_leading_frames = false;
        }
        self.in_band_long_term_prelude
            .begin_frame(first_picture_in_tu);
        first_picture_in_tu
    }

    pub(super) fn begin_discarded_leading_frame(&mut self) {
        self.seen_frame_in_tu = true;
    }
}

pub(super) struct XLayerTable {
    first: Option<XLayerState>,
    rest: Vec<XLayerState>,
    global: GlobalHlsState,
}

impl XLayerTable {
    pub(super) fn new() -> Self {
        Self {
            first: None,
            rest: Vec::new(),
            global: GlobalHlsState::default(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &XLayerState> {
        self.first.iter().chain(self.rest.iter())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut XLayerState> {
        self.first.iter_mut().chain(self.rest.iter_mut())
    }

    pub(super) fn len(&self) -> usize {
        usize::from(self.first.is_some()) + self.rest.len()
    }

    pub(super) fn contains(&self, xlayer: ExtendedLayerId) -> bool {
        self.get(xlayer).is_some()
    }

    pub(super) fn get(&self, xlayer: ExtendedLayerId) -> Option<&XLayerState> {
        if let Some(first) = &self.first
            && first.xlayer_id == xlayer
        {
            return Some(first);
        }
        self.rest.iter().find(|layer| layer.xlayer_id == xlayer)
    }

    pub(super) fn get_mut(&mut self, xlayer: ExtendedLayerId) -> Option<&mut XLayerState> {
        if let Some(first) = &mut self.first
            && first.xlayer_id == xlayer
        {
            return Some(first);
        }
        self.rest.iter_mut().find(|layer| layer.xlayer_id == xlayer)
    }

    pub(super) fn insert(&mut self, mut state: XLayerState) {
        let Some(first) = &mut self.first else {
            self.first = Some(state);
            return;
        };
        if state.xlayer_id == first.xlayer_id {
            *first = state;
            return;
        }
        if state.xlayer_id.get() < first.xlayer_id.get() {
            core::mem::swap(first, &mut state);
        }
        match self
            .rest
            .binary_search_by_key(&state.xlayer_id.get(), |layer| layer.xlayer_id.get())
        {
            Ok(idx) => self.rest[idx] = state,
            Err(idx) => self.rest.insert(idx, state),
        }
    }

    pub(super) fn newly_and_total(&self, xlayer: ExtendedLayerId) -> (&[usize], usize) {
        let total = self.total_emitted_count();
        let newly = self
            .get(xlayer)
            .map_or(&[][..], |layer| layer.scheduler.newly());
        (newly, total)
    }

    pub(super) fn create_effect_state(&self, xlayer: ExtendedLayerId) -> OutputEffectState {
        let mut effects = OutputEffectState::new_for_xlayer(xlayer);
        self.global.sync_effects(&mut effects);
        effects
    }

    pub(super) fn is_discarding_leading_frames(&self, xlayer: ExtendedLayerId) -> bool {
        self.get(xlayer)
            .is_some_and(|layer| layer.discard_leading_frames)
    }

    pub(super) fn observe_global_obus(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        skipped_tu: bool,
    ) -> Result<()> {
        for envelope in obus {
            if !envelope.header.extended_layer_id.is_global() {
                continue;
            }
            match envelope.header.obu_type {
                ObuType::TemporalDelimiter => {
                    self.global.begin_temporal_unit();
                    for layer in self.iter_mut() {
                        layer.on_temporal_delimiter(skipped_tu);
                    }
                }
                ObuType::LayerConfigurationRecord => {
                    if let LayerConfigurationRecord::Global(info) = parse_validated_lcr(*envelope)?
                    {
                        if let Some(slot) = self
                            .global
                            .global_lcr_xlayers
                            .get_mut(usize::from(info.global_config_record_id))
                        {
                            *slot = Some(info.xlayer_map);
                        }
                        for layer in self.iter_mut() {
                            layer
                                .output_effect_state
                                .apply_global_lcr(info.global_config_record_id, info.xlayer_map);
                        }
                    }
                }
                ObuType::OperatingPointSet => {
                    let ops = parse_ops_obu(*envelope)?;
                    self.global.record_ops(&ops);
                    for layer in self.iter_mut() {
                        layer.output_effect_state.apply_ops_record(&ops);
                    }
                }
                ObuType::MetadataShort | ObuType::MetadataGroup => {
                    parse_metadata_obu(*envelope, false, |action| {
                        self.global.record_metadata_action(&action);
                        for layer in self.iter_mut() {
                            layer
                                .output_effect_state
                                .apply_metadata_action(GLOBAL_XLAYER_ID, action.clone());
                        }
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn observe_other_non_global_prefix(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        skip_xlayer: ExtendedLayerId,
    ) -> Result<()> {
        for envelope in obus {
            let xlayer = envelope.header.extended_layer_id;
            if xlayer.is_global() || xlayer == skip_xlayer {
                continue;
            }
            if let Some(layer) = self.get_mut(xlayer) {
                layer
                    .output_effect_state
                    .observe_prefix_obu(*envelope, &layer.sequence)?;
                if envelope.header.obu_type == ObuType::FilmGrain {
                    layer
                        .film_grain_slots
                        .update_from_obu(*envelope, &layer.sequence)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn apply_non_global_prefix(
        obus: &[ObuEnvelope<'_>],
        target_xlayer: ExtendedLayerId,
        sequence: &SequenceHeader,
        effects: &mut OutputEffectState,
        fg_slots: &mut FilmGrainSlots,
    ) -> Result<()> {
        for envelope in obus {
            if envelope.header.extended_layer_id != target_xlayer {
                continue;
            }
            effects.observe_prefix_obu(*envelope, sequence)?;
            if envelope.header.obu_type == ObuType::FilmGrain {
                fg_slots.update_from_obu(*envelope, sequence)?;
            }
        }
        Ok(())
    }

    pub(super) fn observe_skipped(&mut self, obus: &[ObuEnvelope<'_>]) -> Result<()> {
        self.observe_global_obus(obus, true)?;
        self.observe_other_non_global_prefix(obus, GLOBAL_XLAYER_ID)
    }

    pub(super) fn observe_suffix_for_xlayer(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        target_xlayer: ExtendedLayerId,
    ) -> Result<()> {
        for envelope in obus {
            let xlayer = envelope.header.extended_layer_id;
            if xlayer.is_global() {
                match envelope.header.obu_type {
                    ObuType::Padding => {}
                    ObuType::MetadataShort | ObuType::MetadataGroup => {
                        parse_metadata_obu(*envelope, true, |action| {
                            self.global.record_metadata_action(&action);
                            for layer in self.iter_mut() {
                                layer
                                    .output_effect_state
                                    .apply_metadata_action(GLOBAL_XLAYER_ID, action.clone());
                            }
                        })?;
                    }
                    _ => {
                        if let Some(layer) = self.get_mut(target_xlayer) {
                            layer.output_effect_state.observe_suffix_obu(*envelope)?;
                        }
                    }
                }
            } else if xlayer == target_xlayer
                && let Some(layer) = self.get_mut(target_xlayer)
            {
                layer.output_effect_state.observe_suffix_obu(*envelope)?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_xlayer_lcr(
        &self,
        xlayer: ExtendedLayerId,
        sequence: &SequenceHeader,
        effects: &OutputEffectState,
        offset: ByteOffset,
        initializing_new_layer: bool,
    ) -> Result<()> {
        let will_have_multiple = if initializing_new_layer && !self.contains(xlayer) {
            self.first.is_some()
        } else {
            !self.rest.is_empty()
        };
        let lcr_id = sequence.general.seq_lcr_id.get();
        if lcr_id != 0 {
            let in_global = self.global.global_lcr_has_xlayer(lcr_id, xlayer);
            let has_global_record = self
                .global
                .global_lcr_xlayers
                .get(usize::from(lcr_id))
                .copied()
                .flatten()
                .is_some();
            let in_local = !will_have_multiple && effects.has_local_lcr_id(lcr_id);
            let deferred_annex_a = !will_have_multiple
                && !has_global_record
                && !in_local
                && sequence.general.max_mlayer_id.get() > 0;
            if !in_global && !in_local && !deferred_annex_a {
                return Err(DecodeError::MalformedSource {
                    issue: DecodeSourceIssue::sequence_header_conformance(
                        offset,
                        "6.4.1",
                        format!(
                            "extended layer {} references global LCR {} which is missing or omits bit {}",
                            xlayer.get(),
                            lcr_id,
                            xlayer.get()
                        ),
                    ),
                });
            }
        } else if will_have_multiple {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    offset,
                    "6.4.1",
                    format!(
                        "extended layer {} requires seq_lcr_id != 0 in a multi-xlayer stream",
                        xlayer.get()
                    ),
                ),
            });
        }
        if will_have_multiple && initializing_new_layer {
            for existing in self.iter() {
                let existing_lcr_id = existing.sequence.general.seq_lcr_id.get();
                if existing_lcr_id == 0
                    || !self
                        .global
                        .global_lcr_has_xlayer(existing_lcr_id, existing.xlayer_id)
                {
                    return Err(DecodeError::MalformedSource {
                        issue: DecodeSourceIssue::sequence_header_conformance(
                            offset,
                            "6.4.1",
                            format!(
                                "active extended layer {} must be declared in an active global LCR before another extended layer activates",
                                existing.xlayer_id.get()
                            ),
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn retains(&self, frame_index: usize) -> bool {
        self.iter().any(|layer| {
            layer.reference.retains(frame_index) || layer.scheduler.retains(frame_index)
        })
    }

    pub(super) fn forget(&mut self, frame_index: usize) {
        for layer in self.iter_mut() {
            layer.scheduler.forget(frame_index);
        }
    }

    pub(super) fn total_emitted_count(&self) -> usize {
        self.iter().map(|layer| layer.scheduler.emitted_count).sum()
    }

    pub(super) fn flush_layer(&mut self, layer_pos: usize) -> Vec<usize> {
        if layer_pos == 0 {
            self.first
                .as_mut()
                .map_or_else(Vec::new, |layer| layer.scheduler.flush_all())
        } else {
            self.rest[layer_pos - 1].scheduler.flush_all()
        }
    }
}
