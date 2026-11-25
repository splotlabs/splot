// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Read-only access to the atlas snapshot of a decoded picture.

use core::ffi::c_int;

use splot_core::headers::atlas_segment::{AtlasModeInfo, AtlasSegment};
use splot_core::span::ByteOffset;
use splot_decode::{AtlasAssociation, ExternalAtlases, Picture};

use crate::{
    SPLOT_EDECODE, SPLOT_EINVAL, SPLOT_OK, SplotDecoder, SplotSettings, borrowed_slice,
    open_decoder,
};

/// One caller-supplied Annex B atlas OBU, including its length prefix.
#[repr(C)]
pub struct SplotAtlasInput {
    /// Bytes of exactly one framed atlas OBU.
    pub data: *const u8,
    /// Length of `data` in bytes.
    pub size: usize,
    /// Offset of the length prefix in the caller's source.
    pub source_offset: u64,
}

/// Open a decoder with external atlas metadata for random access. The
/// decoder owns the parsed layouts. It retains no caller pointers.
/// The final input for each scope and atlas ID takes precedence.
/// A flush restores these layouts.
///
/// Returns `SPLOT_EINVAL` for invalid pointers, array lengths or settings.
/// Returns `SPLOT_EDECODE` for invalid atlas data or a decoder initialization failure.
/// On error, a valid `out` receives a null pointer.
///
/// # Safety
/// Supply writable, aligned storage for a null decoder pointer in `out`. Supply initialized, readable
/// settings. Each nonempty input array must occupy one live allocation.
/// Supply `count` aligned, initialized descriptors and `size` readable bytes
/// for each descriptor. Keep all input storage immutable during the call.
/// Output storage must not overlap any input. A zero count accepts null `inputs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn splot_open_with_atlases(
    out: Option<&mut *mut SplotDecoder>,
    settings: Option<&SplotSettings>,
    inputs: *const SplotAtlasInput,
    count: usize,
) -> c_int {
    let Some(out) = out else { return SPLOT_EINVAL };
    *out = core::ptr::null_mut();
    let Some(settings) = settings else {
        return SPLOT_EINVAL;
    };
    // SAFETY: The caller supplies an immutable descriptor array for this call.
    let Some(inputs) = (unsafe { borrowed_slice(inputs, count) }) else {
        return SPLOT_EINVAL;
    };
    let mut atlases = ExternalAtlases::default();
    for input in inputs {
        // SAFETY: The caller keeps each descriptor's byte array readable for this call.
        let Some(data) = (unsafe { borrowed_slice(input.data, input.size) }) else {
            return SPLOT_EINVAL;
        };
        if atlases
            .insert(data, ByteOffset::new(input.source_offset))
            .is_err()
        {
            return SPLOT_EDECODE;
        }
    }
    open_decoder(out, settings, atlases)
}

/// Atlas metadata owned by one picture. Pointers into it remain valid until
/// the caller releases that picture with `splot_picture_unref`.
pub struct SplotPictureAtlas {
    pub(super) picture: Picture,
}

/// An optional eight-bit atlas field. Read `value` only when `present` is true.
#[repr(C)]
#[derive(Default, Debug)]
pub struct SplotAtlasByte {
    /// True when the association or layout specifies this field.
    pub present: bool,
    /// The field value.
    pub value: u8,
}

impl From<Option<u8>> for SplotAtlasByte {
    fn from(value: Option<u8>) -> Self {
        Self {
            present: value.is_some(),
            value: value.unwrap_or(0),
        }
    }
}

/// One atlas association for the picture. Array pointers borrow the picture.
#[repr(C)]
#[derive(Default, Debug)]
pub struct SplotAtlasInfo {
    /// The extended layer ID of the atlas. The global scope has ID 31.
    pub scope_xlayer_id: u8,
    /// The referenced atlas ID.
    pub atlas_id: u8,
    /// True when an LCR supplies this association.
    pub has_lcr_offset: bool,
    /// Source byte offset of the LCR when `has_lcr_offset` is true.
    pub lcr_offset: u64,
    /// The segment associated with this picture.
    pub segment_id: SplotAtlasByte,
    /// The segment index when the available layout resolves the segment ID.
    pub segment_index: SplotAtlasByte,
    /// The LCR priority order for this association.
    pub priority_order: SplotAtlasByte,
    /// The LCR rendering method for this association.
    pub rendering_method: SplotAtlasByte,
    /// False when the bitstream omits an optional global layout.
    pub has_layout: bool,
    /// True when the layout comes from caller-supplied metadata.
    pub external: bool,
    /// Source byte offset of the atlas OBU when `has_layout` is true.
    pub atlas_offset: u64,
    /// Atlas mode from AV2 Table 6.11. Valid when `has_layout` is true.
    pub mode: u8,
    /// True when the atlas signals explicit segment IDs.
    pub explicit_segment_ids: bool,
    /// Segment IDs in atlas index order. The array has `segment_count` entries.
    pub segment_ids: *const u8,
    /// Number of segments in the layout.
    pub segment_count: usize,
    /// Canvas width for modes 1, 3 and 4. Nominal width for mode 2.
    pub width: u64,
    /// Canvas height for modes 1, 3 and 4. Nominal height for mode 2.
    pub height: u64,
    /// True when the layout specifies a background color.
    pub has_background: bool,
    /// Background red, green and blue, in order.
    pub background_rgb: [u8; 3],
    /// True when mode 4 signals alpha segments.
    pub alpha_segments_present: bool,
    /// Number of region columns for mode 0.
    pub region_columns: u32,
    /// Number of region rows for mode 0.
    pub region_rows: u32,
    /// True when all mode 0 regions have the same dimensions.
    pub uniform_spacing: bool,
    /// Region widths minus one. Uniform spacing has one entry, otherwise one per column.
    pub column_widths_minus_1: *const u32,
    /// Region heights minus one. Uniform spacing has one entry, otherwise one per row.
    pub row_heights_minus_1: *const u32,
    /// True when each segment maps to one region in raster order.
    pub single_region_per_segment: bool,
}

/// Geometry and source of one indexed segment.
#[repr(C)]
#[derive(Default, Debug)]
pub struct SplotAtlasSegment {
    /// The extended layer ID of the input stream, if present.
    pub input_stream_id: SplotAtlasByte,
    /// Horizontal position in the canvas for modes 1, 3 and 4.
    pub x: u32,
    /// Vertical position in the canvas for modes 1, 3 and 4.
    pub y: u32,
    /// Segment width for modes 1, 3 and 4.
    pub width: u32,
    /// Segment height for modes 1, 3 and 4.
    pub height: u32,
    /// True for an alpha segment in mode 4.
    pub alpha: bool,
    /// Mode 0 first column for an explicit region mapping.
    pub top_left_region_column: u32,
    /// Mode 0 first row for an explicit region mapping.
    pub top_left_region_row: u32,
    /// Mode 0 last column relative to the first column.
    pub bottom_right_region_column_off: u32,
    /// Mode 0 last row relative to the first row.
    pub bottom_right_region_row_off: u32,
}

/// Read association `index` from a picture's atlas snapshot.
/// Supply zero-initialized output storage. Returns `SPLOT_EINVAL` for null
/// pointers or an index outside the association array.
#[unsafe(no_mangle)]
pub extern "C" fn splot_atlas_info(
    snapshot: Option<&SplotPictureAtlas>,
    index: usize,
    out: Option<&mut SplotAtlasInfo>,
) -> c_int {
    let Some(out) = out else { return SPLOT_EINVAL };
    *out = SplotAtlasInfo::default();
    let Some(association) = snapshot.and_then(|s| s.picture.atlas_associations().get(index)) else {
        return SPLOT_EINVAL;
    };
    *out = SplotAtlasInfo::from(association);
    SPLOT_OK
}

impl From<&AtlasAssociation> for SplotAtlasInfo {
    fn from(association: &AtlasAssociation) -> Self {
        let mut out = Self {
            scope_xlayer_id: association.scope.get(),
            atlas_id: association.atlas_id,
            has_lcr_offset: association.lcr_offset.is_some(),
            lcr_offset: association.lcr_offset.map_or(0, ByteOffset::get),
            segment_id: association.segment_id.into(),
            segment_index: association.segment_index().map(|index| index as u8).into(),
            priority_order: association.priority_order.into(),
            rendering_method: association.rendering_method.into(),
            ..Self::default()
        };
        let Some(layout) = &association.layout else {
            return out;
        };
        let atlas = &layout.atlas;
        out.has_layout = true;
        out.external = layout.external;
        out.atlas_offset = layout.offset.get();
        out.mode = atlas.mode.idc() as u8;
        out.explicit_segment_ids = atlas.label.signaled_atlas_segment_ids;
        out.segment_ids = atlas.label.segment_ids.as_ptr();
        out.segment_count = atlas.label.segment_ids.len();
        match &atlas.mode_info {
            AtlasModeInfo::Single(single) => {
                out.width = u64::from(single.nominal_width_minus_1) + 1;
                out.height = u64::from(single.nominal_height_minus_1) + 1;
            }
            AtlasModeInfo::Basic(basic) => {
                out.width = u64::from(basic.width);
                out.height = u64::from(basic.height);
            }
            AtlasModeInfo::Multistream(multi) | AtlasModeInfo::MultistreamAlpha(multi) => {
                out.width = u64::from(multi.width);
                out.height = u64::from(multi.height);
                out.alpha_segments_present = multi.alpha_segments_present == Some(true);
                if let Some((r, g, b)) = multi.background {
                    out.has_background = true;
                    out.background_rgb = [r, g, b];
                }
            }
            AtlasModeInfo::Enhanced(enhanced) => {
                let region = &enhanced.region;
                out.region_columns = region.num_region_columns_minus_1 + 1;
                out.region_rows = region.num_region_rows_minus_1 + 1;
                out.uniform_spacing = region.uniform_spacing;
                out.single_region_per_segment = enhanced.mapping.single_region_per_atlas_segment;
                out.column_widths_minus_1 = region.region_width_minus_1.as_ref().map_or_else(
                    || region.column_widths_minus_1.as_ptr(),
                    core::ptr::from_ref,
                );
                out.row_heights_minus_1 = region
                    .region_height_minus_1
                    .as_ref()
                    .map_or_else(|| region.row_heights_minus_1.as_ptr(), core::ptr::from_ref);
            }
            _ => {}
        }
        out
    }
}

/// Read one segment of association `index` from the picture's atlas snapshot.
/// Supply zero-initialized output storage. Returns `SPLOT_EINVAL` for null
/// pointers, an unavailable layout, or an index outside either array.
#[unsafe(no_mangle)]
pub extern "C" fn splot_atlas_segment(
    snapshot: Option<&SplotPictureAtlas>,
    index: usize,
    segment_index: usize,
    out: Option<&mut SplotAtlasSegment>,
) -> c_int {
    let Some(out) = out else { return SPLOT_EINVAL };
    *out = SplotAtlasSegment::default();
    let Some(layout) = snapshot
        .and_then(|s| s.picture.atlas_associations().get(index))
        .and_then(|association| association.layout.as_deref())
    else {
        return SPLOT_EINVAL;
    };
    if let Some(segment) = segment_geometry(&layout.atlas, segment_index) {
        *out = segment;
        SPLOT_OK
    } else {
        SPLOT_EINVAL
    }
}

fn segment_geometry(atlas: &AtlasSegment, index: usize) -> Option<SplotAtlasSegment> {
    atlas.label.segment_ids.get(index)?;
    let mut out = SplotAtlasSegment::default();
    match &atlas.mode_info {
        AtlasModeInfo::Basic(basic) => {
            let segment = basic.segments.get(index)?;
            out.input_stream_id = segment.input_stream_id.into();
            out.x = segment.top_left_pos_x;
            out.y = segment.top_left_pos_y;
            out.width = segment.width;
            out.height = segment.height;
        }
        AtlasModeInfo::Multistream(multi) | AtlasModeInfo::MultistreamAlpha(multi) => {
            let segment = multi.segments.get(index)?;
            out.input_stream_id = Some(segment.input_stream_id).into();
            out.x = segment.top_left_pos_x;
            out.y = segment.top_left_pos_y;
            out.width = segment.width;
            out.height = segment.height;
            out.alpha = segment.alpha_segment_flag;
        }
        AtlasModeInfo::Enhanced(enhanced) if !enhanced.mapping.single_region_per_atlas_segment => {
            let region = enhanced.mapping.segments.get(index)?;
            out.top_left_region_column = region.top_left_region_column;
            out.top_left_region_row = region.top_left_region_row;
            out.bottom_right_region_column_off = region.bottom_right_region_column_off;
            out.bottom_right_region_row_off = region.bottom_right_region_row_off;
        }
        AtlasModeInfo::Single(_) | AtlasModeInfo::Enhanced(_) => {}
        _ => return None,
    }
    Some(out)
}
