// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Atlas availability and picture associations (AV2 §§ 6.8–6.9 and 7.3.8.4).

use std::sync::Arc;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope};
use splot_core::bitio::BitReader;
use splot_core::headers::atlas_segment::{AtlasModeInfo, AtlasSegment, parse_scoped_atlas_segment};
use splot_core::headers::layer_config_record::{
    LcrGlobalInfo, LcrLocalInfo, LcrXlayerAtlasRef, LcrXlayerInfo,
};
use splot_core::headers::sequence::SequenceHeaderId;
use splot_core::obu::finish_obu_payload;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId};

use crate::{DecodeError, DecodeSourceIssue, Result};

/// An immutable atlas description and its position in the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasLayout {
    /// Offset of the atlas OBU.
    pub offset: ByteOffset,
    /// Whether the caller supplied this OBU in an external source.
    pub external: bool,
    /// Parsed layout, including segment labels and mode-specific geometry.
    pub atlas: AtlasSegment,
}

/// The association between one decoded picture and one scoped atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasAssociation {
    /// Extended-layer ID of the atlas. ID 31 identifies a global atlas.
    pub scope: ExtendedLayerId,
    /// Atlas ID in this scope.
    pub atlas_id: u8,
    /// Offset of the referring LCR, when present.
    pub lcr_offset: Option<ByteOffset>,
    /// Segment label for this picture, when specified.
    pub segment_id: Option<u8>,
    segment_index: Option<usize>,
    /// LCR rendering priority, when specified.
    pub priority_order: Option<u8>,
    /// LCR rendering method, when specified.
    pub rendering_method: Option<u8>,
    /// Layout at the picture's source position. A referenced global atlas can be absent.
    pub layout: Option<Arc<AtlasLayout>>,
}

impl AtlasAssociation {
    /// Returns the segment index in the attached layout, when available.
    #[must_use]
    pub const fn segment_index(&self) -> Option<usize> {
        self.segment_index
    }
}

/// Caller-supplied atlas OBUs for a new decode or a random access point.
/// Each extended layer, including the global scope, has eight atlas IDs.
#[derive(Clone, Debug, Default)]
pub struct ExternalAtlases {
    pub(super) scopes: Vec<Option<ScopedAtlases>>,
}

impl ExternalAtlases {
    /// Supplies one Annex B length-delimited atlas OBU. The offset locates its length prefix in the external source.
    /// An OBU replaces the previous atlas with the same scope and ID.
    ///
    /// # Errors
    /// Returns a malformed-source error for an invalid scope, payload, offset range, or OBU count.
    /// A failed insert preserves the previous atlas.
    pub fn insert(&mut self, obu: &[u8], base_offset: ByteOffset) -> Result<()> {
        if base_offset.get().checked_add(obu.len() as u64).is_none() {
            return Err(invalid(
                base_offset,
                "B",
                "external atlas source offset overflows".into(),
            ));
        }
        let mut cursor = AnnexBObuCursor::new(obu, base_offset);
        let envelope = cursor
            .next_obu()
            .map_err(|error| parse_error(&error))?
            .ok_or_else(|| invalid(base_offset, "B", "expected one external atlas OBU".into()))?;
        if envelope.header.obu_type != splot_core::types::ObuType::AtlasSegment
            || cursor.has_remaining()
        {
            return Err(invalid(
                envelope.offset,
                "5.9",
                "expected exactly one external atlas OBU".into(),
            ));
        }
        let layout = AtlasLayout::parse(envelope, true)?;
        scope_mut(&mut self.scopes, envelope.header.extended_layer_id).store(layout);
        Ok(())
    }
}

pub(super) fn scope_mut<T: Default>(scopes: &mut Vec<Option<T>>, id: ExtendedLayerId) -> &mut T {
    let index = usize::from(id.get());
    if scopes.len() <= index {
        scopes.resize_with(index + 1, || None);
    }
    scopes[index].get_or_insert_with(T::default)
}

#[derive(Clone, Debug, Default)]
pub(super) struct ScopedAtlases {
    layouts: Option<Box<[Option<Arc<AtlasLayout>>; 8]>>,
}

impl ScopedAtlases {
    pub(super) fn observe(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        self.store(AtlasLayout::parse(envelope, false)?);
        Ok(())
    }

    fn store(&mut self, layout: AtlasLayout) {
        let id = usize::from(layout.atlas.atlas_segment_id);
        self.layouts.get_or_insert_with(Default::default)[id] = Some(Arc::new(layout));
    }

    fn get(&self, id: u8) -> Option<&Arc<AtlasLayout>> {
        self.layouts.as_ref()?[usize::from(id)].as_ref()
    }

    pub(super) fn msdo_associations(
        &self,
        xlayer: ExtendedLayerId,
        output: &mut Vec<AtlasAssociation>,
    ) {
        let Some(layouts) = &self.layouts else { return };
        for layout in layouts.iter().flatten() {
            if let Some((index, id)) = layout.stream_segment(xlayer) {
                output.push(AtlasAssociation {
                    scope: splot_core::types::GLOBAL_XLAYER_ID,
                    atlas_id: layout.atlas.atlas_segment_id,
                    lcr_offset: None,
                    segment_id: Some(id),
                    segment_index: Some(index),
                    priority_order: None,
                    rendering_method: None,
                    layout: Some(layout.clone()),
                });
            }
        }
    }
}

impl AtlasLayout {
    fn parse(envelope: ObuEnvelope<'_>, external: bool) -> Result<Self> {
        if envelope.header.temporal_layer_id.get() != 0
            || envelope.header.embedded_layer_id.get() != 0
        {
            return Err(invalid(
                envelope.offset,
                "6.2.2",
                "atlas must use temporal and embedded layer zero".into(),
            ));
        }
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let atlas = parse_scoped_atlas_segment(&mut reader, envelope.header.extended_layer_id)
            .map_err(|error| parse_error(&error))?;
        finish_obu_payload(
            &mut reader,
            envelope.payload,
            envelope.header.obu_type.is_extensible_obu(),
        )
        .map_err(|error| parse_error(&error))?;
        Ok(Self {
            offset: envelope.offset,
            external,
            atlas,
        })
    }

    fn stream_segment(&self, xlayer: ExtendedLayerId) -> Option<(usize, u8)> {
        let index = match &self.atlas.mode_info {
            AtlasModeInfo::Multistream(info) | AtlasModeInfo::MultistreamAlpha(info) => info
                .segments
                .iter()
                .position(|segment| segment.input_stream_id == xlayer.get()),
            AtlasModeInfo::Basic(info) => info
                .segments
                .iter()
                .position(|segment| segment.input_stream_id == Some(xlayer.get())),
            _ => None,
        }?;
        self.atlas
            .label
            .segment_ids
            .get(index)
            .map(|&id| (index, id))
    }
}

type LocalRecord = (ByteOffset, Arc<LcrLocalInfo>);

pub(super) type GlobalRecord = (u64, ByteOffset, Arc<LcrGlobalInfo>);

/// The § 6.4.1 association uses the records available before the sequence header.
#[derive(Default, Clone)]
pub(super) struct SequenceLcr {
    pub(super) local_parent: Option<u8>,
    local: Option<LocalRecord>,
    pub(super) global: Option<GlobalRecord>,
}

impl SequenceLcr {
    pub(super) fn direct_global_for(&self, xlayer: ExtendedLayerId) -> bool {
        self.local_parent.is_none()
            && self
                .global
                .as_ref()
                .is_some_and(|(_, _, global)| global.xlayer_map & (1 << xlayer.get()) != 0)
    }
}

#[derive(Default)]
pub(super) struct LayerAtlases {
    pub(super) layouts: ScopedAtlases,
    pub(super) local_ids: u8,
    records: Option<Box<[Option<LocalRecord>; 8]>>,
    sequences: Vec<SequenceLcr>,
    pub(super) active: SequenceLcr,
}

impl From<ScopedAtlases> for LayerAtlases {
    fn from(layouts: ScopedAtlases) -> Self {
        Self {
            layouts,
            ..Self::default()
        }
    }
}

impl LayerAtlases {
    pub(super) fn has_local(&self, id: u8) -> bool {
        (1..=7).contains(&id) && self.local_ids & (1 << id) != 0
    }

    pub(super) fn observe_local_lcr(
        &mut self,
        offset: ByteOffset,
        info: LcrLocalInfo,
    ) -> Result<()> {
        if let Some(id) = info.local_atlas_id
            && self.layouts.get(id).is_none()
        {
            return Err(invalid(
                offset,
                "7.3.8.4",
                format!(
                    "local LCR references unavailable atlas {id} in extended layer {}",
                    info.xlayer_id
                ),
            ));
        }
        if info.local_id != 0 {
            self.local_ids |= 1 << info.local_id;
        }
        let id = usize::from(info.local_id);
        if info.local_atlas_id.is_some() || info.global_id != 0 {
            self.records.get_or_insert_with(Default::default)[id] = Some((offset, Arc::new(info)));
        } else if let Some(records) = &mut self.records {
            records[id] = None;
        }
        Ok(())
    }

    pub(super) fn bind(
        &mut self,
        sequence: SequenceHeaderId,
        id: u8,
        globals: &[Option<GlobalRecord>; 8],
    ) {
        let local = self
            .records
            .as_ref()
            .and_then(|records| records[usize::from(id)].clone());
        let local_parent = self
            .has_local(id)
            .then(|| local.as_ref().map_or(0, |(_, record)| record.global_id));
        let global = globals[usize::from(local_parent.unwrap_or(id))].clone();
        let index = usize::from(sequence.get());
        if self.sequences.len() <= index {
            if local_parent.is_none() && global.is_none() {
                return;
            }
            self.sequences.resize_with(index + 1, SequenceLcr::default);
        }
        self.sequences[index] = SequenceLcr {
            local_parent,
            local,
            global,
        };
    }

    pub(super) fn binding(&self, sequence: SequenceHeaderId) -> Option<&SequenceLcr> {
        self.sequences.get(usize::from(sequence.get()))
    }

    pub(super) fn activate(&mut self, sequence: SequenceHeaderId) {
        self.active = self.binding(sequence).cloned().unwrap_or_default();
    }

    pub(super) fn snapshot(
        &self,
        globals: &ScopedAtlases,
        msdo_stream: bool,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        output: &mut Vec<AtlasAssociation>,
    ) -> Result<()> {
        if let Some((_, offset, record)) = &self.active.global
            && record.xlayer_map & (1u32 << xlayer.get()) != 0
            && (record.global_atlas_id.is_some() || globals.get(0).is_some())
        {
            let info = record.payloads.iter().find(|p| p.xlayer_id == xlayer.get());
            output.push(globals.association(
                // § 6.8.2 infers a global atlas ID of zero.
                record.global_atlas_id.unwrap_or(0),
                *offset,
                splot_core::types::GLOBAL_XLAYER_ID,
                xlayer,
                mlayer,
                info.map(|p| &p.xlayer_info),
            )?);
        } else if msdo_stream {
            globals.msdo_associations(xlayer, output);
        }
        if let Some((offset, record)) = &self.active.local
            && let Some(id) = record.local_atlas_id
        {
            output.push(self.layouts.association(
                id,
                *offset,
                xlayer,
                xlayer,
                mlayer,
                Some(&record.xlayer_info),
            )?);
        }
        Ok(())
    }
}

impl ScopedAtlases {
    fn association(
        &self,
        atlas_id: u8,
        lcr_offset: ByteOffset,
        scope: ExtendedLayerId,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        xlayer_info: Option<&LcrXlayerInfo>,
    ) -> Result<AtlasAssociation> {
        let layout = self.get(atlas_id);
        let reference = xlayer_info.and_then(|info| {
            info.xlayer_atlas.or_else(|| {
                let layer = info
                    .embedded_layer_info
                    .as_ref()?
                    .layers
                    .iter()
                    .find(|entry| entry.mlayer_index == mlayer.get())?;
                Some(LcrXlayerAtlasRef {
                    atlas_segment_id: layer.atlas_segment_id?,
                    priority_order: layer.priority_order?,
                    rendering_method: layer.rendering_method?,
                })
            })
        });
        let (segment_id, segment_index) = match (reference, layout) {
            (Some(reference), Some(layout)) => {
                let id = reference.atlas_segment_id;
                // § 5.9.1 writes the inverse label map in ascending index order.
                let index = layout
                    .atlas
                    .label
                    .segment_ids
                    .iter()
                    .rposition(|label| *label == id)
                    .ok_or_else(|| {
                        invalid(
                            if layout.external {
                                lcr_offset
                            } else {
                                lcr_offset.max(layout.offset)
                            },
                            "6.9.2",
                            format!("atlas {atlas_id} has no segment with ID {id}"),
                        )
                    })?;
                (Some(id), Some(index))
            }
            (Some(reference), None) => (Some(reference.atlas_segment_id), None),
            (None, layout) => layout
                .and_then(|layout| layout.stream_segment(xlayer))
                .map_or((None, None), |(index, id)| (Some(id), Some(index))),
        };
        Ok(AtlasAssociation {
            scope,
            atlas_id,
            lcr_offset: Some(lcr_offset),
            segment_id,
            segment_index,
            priority_order: reference.map(|r| r.priority_order),
            rendering_method: reference.map(|r| r.rendering_method),
            layout: layout.cloned(),
        })
    }
}

fn parse_error(error: &splot_core::Error) -> DecodeError {
    DecodeError::MalformedSource {
        issue: crate::stream_plan::issue_from_core_error(
            crate::DecodeSourceIssueKind::ObuPayloadParseError,
            None,
            error,
        ),
    }
}

fn invalid(offset: ByteOffset, section: &'static str, message: String) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::obu_payload_parse(offset, section, message),
    }
}
