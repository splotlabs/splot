// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Atlas availability and picture associations (AV2 §§ 6.8–6.9 and 7.3.8.4).

use std::collections::BTreeMap;
use std::sync::Arc;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope};
use splot_core::bitio::BitReader;
use splot_core::headers::atlas_segment::{AtlasModeInfo, AtlasSegment, parse_scoped_atlas_segment};
use splot_core::headers::layer_config_record::{LcrLocalInfo, LcrXlayerAtlasRef, LcrXlayerInfo};
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
    pub(super) scopes: BTreeMap<ExtendedLayerId, ScopedAtlases>,
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
            .map_err(parse_error)?
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
        if envelope.header.temporal_layer_id.get() != 0
            || envelope.header.embedded_layer_id.get() != 0
        {
            return Err(invalid(
                envelope.offset,
                "6.2.2",
                "external atlas must use temporal and embedded layer zero".into(),
            ));
        }
        let layout = AtlasLayout::parse(envelope, true)?;
        self.scopes
            .entry(envelope.header.extended_layer_id)
            .or_default()
            .store(layout);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct ScopedAtlases {
    layouts: BTreeMap<u8, Arc<AtlasLayout>>,
}

impl ScopedAtlases {
    pub(super) fn observe(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        self.store(AtlasLayout::parse(envelope, false)?);
        Ok(())
    }

    fn store(&mut self, layout: AtlasLayout) {
        self.layouts
            .insert(layout.atlas.atlas_segment_id, Arc::new(layout));
    }

    pub(super) fn msdo_associations(
        &self,
        xlayer: ExtendedLayerId,
        output: &mut Vec<AtlasAssociation>,
    ) {
        for (&atlas_id, layout) in &self.layouts {
            if let Some((index, id)) = layout.stream_segment(xlayer) {
                output.push(AtlasAssociation {
                    scope: splot_core::types::GLOBAL_XLAYER_ID,
                    atlas_id,
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
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let atlas = parse_scoped_atlas_segment(&mut reader, envelope.header.extended_layer_id)
            .map_err(parse_error)?;
        finish_obu_payload(
            &mut reader,
            envelope.payload,
            envelope.header.obu_type.is_extensible_obu(),
        )
        .map_err(parse_error)?;
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

#[derive(Default)]
pub(super) struct LayerAtlases {
    pub(super) layouts: ScopedAtlases,
    pub(super) records: BTreeMap<u8, (ByteOffset, LcrLocalInfo)>,
    pub(super) active_global: Option<Box<AtlasReference>>,
    pub(super) active_local: Option<Box<AtlasReference>>,
}

impl LayerAtlases {
    pub(super) fn observe_local_lcr(
        &mut self,
        offset: ByteOffset,
        info: LcrLocalInfo,
    ) -> Result<()> {
        if let Some(id) = info.local_atlas_id
            && !self.layouts.layouts.contains_key(&id)
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
        self.records.insert(info.local_id, (offset, info));
        Ok(())
    }
}

pub(super) struct AtlasReference {
    pub(super) id: u8,
    pub(super) offset: ByteOffset,
    pub(super) explicit: bool,
    pub(super) xlayer_info: Option<LcrXlayerInfo>,
}

impl AtlasReference {
    pub(super) fn association(
        &self,
        layouts: &ScopedAtlases,
        scope: ExtendedLayerId,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
    ) -> Result<Option<AtlasAssociation>> {
        let layout = layouts.layouts.get(&self.id);
        if !self.explicit && layout.is_none() {
            return Ok(None);
        }
        let reference = self.xlayer_info.as_ref().and_then(|info| {
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
                                self.offset
                            } else {
                                self.offset.max(layout.offset)
                            },
                            "6.9.2",
                            format!("atlas {} has no segment with ID {id}", self.id),
                        )
                    })?;
                (Some(id), Some(index))
            }
            (Some(reference), None) => (Some(reference.atlas_segment_id), None),
            (None, layout) => layout
                .and_then(|layout| layout.stream_segment(xlayer))
                .map_or((None, None), |(index, id)| (Some(id), Some(index))),
        };
        Ok(Some(AtlasAssociation {
            scope,
            atlas_id: self.id,
            lcr_offset: Some(self.offset),
            segment_id,
            segment_index,
            priority_order: reference.map(|r| r.priority_order),
            rendering_method: reference.map(|r| r.rendering_method),
            layout: layout.cloned(),
        }))
    }
}

fn parse_error(error: splot_core::Error) -> DecodeError {
    DecodeError::MalformedSource {
        issue: crate::stream_plan::issue_from_core_error(
            crate::DecodeSourceIssueKind::ObuPayloadParseError,
            None,
            &error,
        ),
    }
}

fn invalid(offset: ByteOffset, section: &'static str, message: String) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::obu_payload_parse(offset, section, message),
    }
}
