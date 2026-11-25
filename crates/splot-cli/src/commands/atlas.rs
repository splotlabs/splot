// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Atlas metadata in inspect and decode reports.

use serde::Serialize;
use splot_core::headers::atlas_segment::AtlasSegment;
use splot_decode::{AtlasAssociation, DecodeHashFrame};

#[derive(Serialize)]
pub(super) struct AtlasSegmentView {
    atlas_id: u8,
    mode: u32,
    explicit_segment_ids: bool,
    segment_ids: Vec<u8>,
}

impl AtlasSegmentView {
    pub(super) fn new(atlas: &AtlasSegment) -> Self {
        Self {
            atlas_id: atlas.atlas_segment_id,
            mode: atlas.mode.idc(),
            explicit_segment_ids: atlas.label.signaled_atlas_segment_ids,
            segment_ids: atlas.label.segment_ids.clone(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct PictureAtlasView<'a> {
    extended_layer_id: u8,
    embedded_layer_id: u8,
    associations: Vec<AssociationView<'a>>,
}

impl<'a> PictureAtlasView<'a> {
    pub(super) fn of(frame: &'a DecodeHashFrame) -> Option<Self> {
        (!frame.atlas_associations.is_empty()).then(|| Self {
            extended_layer_id: frame.extended_layer_id.get(),
            embedded_layer_id: frame.embedded_layer_id.get(),
            associations: frame
                .atlas_associations
                .iter()
                .map(AssociationView::new)
                .collect(),
        })
    }
}

#[derive(Serialize)]
struct AssociationView<'a> {
    scope_xlayer_id: u8,
    atlas_id: u8,
    lcr_offset: Option<u64>,
    segment_id: Option<u8>,
    segment_index: Option<usize>,
    priority_order: Option<u8>,
    rendering_method: Option<u8>,
    layout: Option<LayoutView<'a>>,
}

#[derive(Serialize)]
struct LayoutView<'a> {
    byte_offset: u64,
    external: bool,
    mode: u32,
    explicit_segment_ids: bool,
    segment_ids: &'a [u8],
}

impl<'a> AssociationView<'a> {
    fn new(association: &'a AtlasAssociation) -> Self {
        Self {
            scope_xlayer_id: association.scope.get(),
            atlas_id: association.atlas_id,
            lcr_offset: association.lcr_offset.map(|offset| offset.get()),
            segment_id: association.segment_id,
            segment_index: association.segment_index(),
            priority_order: association.priority_order,
            rendering_method: association.rendering_method,
            layout: association.layout.as_ref().map(|layout| LayoutView {
                byte_offset: layout.offset.get(),
                external: layout.external,
                mode: layout.atlas.mode.idc(),
                explicit_segment_ids: layout.atlas.label.signaled_atlas_segment_ids,
                segment_ids: &layout.atlas.label.segment_ids,
            }),
        }
    }
}
