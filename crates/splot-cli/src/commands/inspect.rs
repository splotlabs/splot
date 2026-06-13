// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot inspect` — print OBUs and headers from a bitstream.
//!
//! ## Payload status vs. stateful frame-header views
//!
//! The per-OBU `payload_status` field comes from the **stateless** dispatcher
//! ([`splot_core::obu::dispatch_obu_payload`]). For the 11 frame-carrying OBU types
//! (the tile-group family and the SEF / TIP / bridge family) that dispatcher parses only
//! the state-free § 5.18.2 / § 5.19 activation prefix and reports
//! `prefix_parsed_awaiting_state` (the remainder needs the activated sequence header
//! state it does not hold). The inspector's own `frame_header_prefix`,
//! `frame_header_core`, `frame_header_copy`, and `tile_group_structure` views are the
//! **richer surface**: they thread the running sequence-header and multi-frame-header
//! state across OBUs and so resolve the deeper § 5.18 / § 5.19 syntax. The two surfaces
//! are consistent by construction — the stateful views never contradict the stateless
//! prefix, they extend it.

use std::path::PathBuf;
use std::process::ExitCode;

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use clap::Args;
use serde::Serialize;
use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::buffer_removal_timing::{
    BufferRemovalTiming, parse_buffer_removal_timing,
};
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::film_grain::{FilmGrainObu, parse_film_grain};
use splot_core::headers::frame::{
    CcsoParams, CcsoPlaneParams, CdefParams, CdefStrengthSet, DeblockingFilterParams, DeltaQParams,
    FilmGrainConfig, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameHeaderPrefix, FrameHeaderTail, FrameReferenceStateView, GdfParams,
    InterControl, LosslessInfo, LrParams, LrPartialParams, LrPlaneParams, QuantizationParams,
    SefTrailingBits, SegmentationParams, SetupQmParams, TileInfo, parse_frame_header_core,
    parse_frame_header_prefix,
};
use splot_core::headers::metadata::{MetadataUnit, parse_metadata_group, parse_metadata_short};
use splot_core::headers::operating_point_set::{OperatingPointSet, parse_operating_point_set};
use splot_core::headers::padding::parse_padding_obu;
use splot_core::headers::quantizer_matrix::{QuantizerMatrixObu, parse_quantizer_matrix};
use splot_core::headers::sequence::{SequenceHeader, SequenceHeaderId, parse_sequence_header};
use splot_core::headers::tile_group::{
    TileGroupLayout, parse_tile_group_framing, parse_tile_group_prefix, parse_tile_group_structure,
};
use splot_core::hls::{MfhId, MultiFrameHeaderRecord, parse_multi_frame_header};
use splot_core::ivf::{IvfFrame, IvfHeader};
use splot_core::obu::{ObuHeader, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

use crate::commands::read_input;

/// Arguments for `splot inspect`.
#[derive(Args, Debug)]
pub struct InspectArgs {
    /// Path to a raw AV2 Annex B bitstream or IVF-wrapped Annex B stream.
    pub input: PathBuf,
    /// Emit the inspection as JSON.
    #[arg(long)]
    pub json: bool,
    /// Print only OBU headers (omit payload sizes in text output).
    #[arg(long)]
    pub headers: bool,
}

/// A serializable summary of one OBU for `--json` output.
#[derive(Serialize)]
struct InspectRecord {
    index: usize,
    byte_offset: u64,
    size: u32,
    payload_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    ivf_header: Option<InspectIvfHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ivf_frame: Option<InspectIvfFrame>,
    payload_status: InspectPayloadStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_header: Option<SequenceHeaderView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_interpretation: Option<ContentInterpretationView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operating_point_set: Option<OperatingPointSetView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_removal_timing: Option<BufferRemovalTimingView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantizer_matrix: Option<QuantizerMatrixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    film_grain: Option<FilmGrainView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    padding: Option<PaddingView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_short: Option<MetadataShortView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_group: Option<MetadataGroupView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_header_prefix: Option<FrameHeaderPrefixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_header_core: Option<FrameHeaderCoreView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_header_copy: Option<FrameHeaderCopyView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_group_structure: Option<TileGroupStructureView>,
    header: ObuHeader,
}

impl InspectRecord {
    fn new(
        index: usize,
        obu: &ObuEnvelope<'_>,
        sequences: &BTreeMap<u8, SequenceHeader>,
        multi_frame_headers: &BTreeMap<u32, MultiFrameHeaderRecord>,
        ivf_header: Option<IvfHeader>,
        ivf_frame: Option<IvfFrame<'_>>,
    ) -> Self {
        Self {
            index,
            byte_offset: obu.offset.get(),
            size: obu.size,
            payload_len: obu.payload.len(),
            ivf_header: ivf_header.map(InspectIvfHeader::new),
            ivf_frame: ivf_frame.map(InspectIvfFrame::new),
            payload_status: InspectPayloadStatus::new(obu),
            sequence_header: sequence_header_view(obu),
            content_interpretation: content_interpretation_view(obu),
            operating_point_set: operating_point_set_view(obu),
            buffer_removal_timing: buffer_removal_timing_view(obu),
            quantizer_matrix: quantizer_matrix_view(obu),
            film_grain: film_grain_view(obu),
            padding: padding_view(obu),
            metadata_short: metadata_short_view(obu),
            metadata_group: metadata_group_view(obu),
            frame_header_prefix: frame_header_prefix_view(obu),
            frame_header_core: frame_header_core_view(obu, sequences, multi_frame_headers),
            frame_header_copy: frame_header_copy_view(obu),
            tile_group_structure: tile_group_structure_view(obu, sequences, multi_frame_headers),
            header: obu.header,
        }
    }
}

/// IVF header metadata attached to OBU records from an IVF input.
#[derive(Serialize)]
struct InspectIvfHeader {
    fourcc: String,
    width: u16,
    height: u16,
    timebase_denominator: u32,
    timebase_numerator: u32,
    declared_frame_count: u32,
}

impl InspectIvfHeader {
    fn new(header: IvfHeader) -> Self {
        Self {
            fourcc: fourcc_string(header.fourcc),
            width: header.width,
            height: header.height,
            timebase_denominator: header.timebase_denominator,
            timebase_numerator: header.timebase_numerator,
            declared_frame_count: header.frame_count,
        }
    }
}

/// IVF frame metadata attached to OBU records from an IVF input.
#[derive(Serialize)]
struct InspectIvfFrame {
    index: usize,
    header_offset: u64,
    payload_offset: u64,
    size: u32,
    pts: u64,
}

impl InspectIvfFrame {
    fn new(frame: IvfFrame<'_>) -> Self {
        Self {
            index: frame.index,
            header_offset: frame.header_offset.get(),
            payload_offset: frame.payload_offset.get(),
            size: frame.size,
            pts: frame.pts,
        }
    }
}

fn fourcc_string(fourcc: [u8; 4]) -> String {
    fourcc
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

/// Re-parses a `padding_obu()` so `--json` can expose its padding and trailing lengths.
fn padding_view(obu: &ObuEnvelope<'_>) -> Option<PaddingView> {
    if obu.header.obu_type != ObuType::Padding {
        return None;
    }
    parse_padding_obu(obu.payload, obu.payload_offset())
        .ok()
        .map(|padding| PaddingView {
            padding_len: padding.padding_len,
            trailing_len: padding.trailing_len,
        })
}

/// A compact, machine-readable view of a parsed `padding_obu()`.
#[derive(Serialize)]
struct PaddingView {
    padding_len: usize,
    trailing_len: usize,
}

/// A per-unit metadata summary (`metadata_type` and declared payload size); never dumps
/// the raw payload bytes.
#[derive(Serialize)]
struct MetadataUnitView {
    metadata_type: u32,
    metadata_type_name: &'static str,
    payload_size: usize,
}

impl MetadataUnitView {
    fn new(unit: &MetadataUnit) -> Self {
        Self {
            metadata_type: unit.metadata_type.value(),
            metadata_type_name: unit.metadata_type.spec_name(),
            payload_size: unit.payload_size,
        }
    }
}

/// Re-parses a `metadata_short_obu()` so `--json` can expose its header fields and the
/// metadata unit summary.
fn metadata_short_view(obu: &ObuEnvelope<'_>) -> Option<MetadataShortView> {
    if obu.header.obu_type != ObuType::MetadataShort {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_metadata_short(&mut reader, obu.payload.len())
        .ok()
        .map(|metadata| MetadataShortView {
            is_suffix: metadata.metadata_is_suffix,
            layer_idc: metadata.muh_layer_idc,
            cancel: metadata.muh_cancel_flag,
            persistence_idc: metadata.muh_persistence_idc,
            metadata_type: metadata.metadata_type.value(),
            metadata_type_name: metadata.metadata_type.spec_name(),
            unit: metadata.unit.as_ref().map(MetadataUnitView::new),
        })
}

/// A compact, machine-readable view of a parsed `metadata_short_obu()`.
#[derive(Serialize)]
struct MetadataShortView {
    is_suffix: bool,
    layer_idc: u8,
    cancel: bool,
    persistence_idc: u8,
    metadata_type: u32,
    metadata_type_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<MetadataUnitView>,
}

/// Re-parses a `metadata_group_obu()` so `--json` can expose its header fields and a
/// per-unit summary.
fn metadata_group_view(obu: &ObuEnvelope<'_>) -> Option<MetadataGroupView> {
    if obu.header.obu_type != ObuType::MetadataGroup {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_metadata_group(&mut reader, obu.header.extended_layer_id)
        .ok()
        .map(|group| MetadataGroupView {
            is_suffix: group.metadata_is_suffix,
            necessity_idc: group.metadata_necessity_idc,
            application_id: group.metadata_application_id,
            unit_count: group.units.len(),
            units: group
                .units
                .iter()
                .map(|unit| MetadataGroupUnitView {
                    metadata_type: unit.metadata_type.value(),
                    metadata_type_name: unit.metadata_type.spec_name(),
                    cancel: unit.muh_cancel_flag,
                    payload_size: unit.muh_payload_size,
                    layer_idc: unit.muh_layer_idc,
                })
                .collect(),
        })
}

/// A compact, machine-readable view of a parsed `metadata_group_obu()`.
#[derive(Serialize)]
struct MetadataGroupView {
    is_suffix: bool,
    necessity_idc: u8,
    application_id: u8,
    unit_count: usize,
    units: Vec<MetadataGroupUnitView>,
}

/// A compact per-unit summary for `MetadataGroupView`.
#[derive(Serialize)]
struct MetadataGroupUnitView {
    metadata_type: u32,
    metadata_type_name: &'static str,
    cancel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer_idc: Option<u8>,
}

/// Re-parses an `operating_point_set_obu()` so `--json` can expose its key fields.
fn operating_point_set_view(obu: &ObuEnvelope<'_>) -> Option<OperatingPointSetView> {
    if obu.header.obu_type != ObuType::OperatingPointSet {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_operating_point_set(&mut reader, obu.header.extended_layer_id)
        .ok()
        .map(|ops| OperatingPointSetView::new(&ops))
}

/// A compact, machine-readable view of a parsed `operating_point_set_obu()`.
#[derive(Serialize)]
struct OperatingPointSetView {
    xlayer_id: u8,
    is_global: bool,
    reset_flag: bool,
    ops_id: u8,
    ops_cnt: u8,
    payload_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    mlayer_info_idc: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_reserved_2bits: Option<u8>,
}

impl OperatingPointSetView {
    fn new(ops: &OperatingPointSet) -> Self {
        Self {
            xlayer_id: ops.xlayer_id.get(),
            is_global: ops.is_global(),
            reset_flag: ops.reset_flag,
            ops_id: ops.ops_id,
            ops_cnt: ops.ops_cnt,
            payload_count: ops.payloads.len(),
            mlayer_info_idc: ops.mlayer_info_idc,
            local_reserved_2bits: ops.local_reserved_2bits,
        }
    }
}

/// Re-parses a `buffer_removal_timing_obu()` so `--json` can expose its key fields.
fn buffer_removal_timing_view(obu: &ObuEnvelope<'_>) -> Option<BufferRemovalTimingView> {
    if obu.header.obu_type != ObuType::BufferRemovalTiming {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_buffer_removal_timing(&mut reader)
        .ok()
        .map(|brt| BufferRemovalTimingView::new(&brt))
}

/// A compact, machine-readable view of a parsed `buffer_removal_timing_obu()`.
#[derive(Serialize)]
struct BufferRemovalTimingView {
    ops_dependent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_ops_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_ops_cnt: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    op_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    br_time: Option<u32>,
}

impl BufferRemovalTimingView {
    fn new(brt: &BufferRemovalTiming) -> Self {
        let ops_reference = brt.ops_reference();
        Self {
            ops_dependent: brt.is_ops_dependent(),
            br_ops_id: ops_reference.map(|(id, _)| id),
            br_ops_cnt: ops_reference.map(|(_, cnt)| cnt),
            op_count: ops_reference.map(|_| brt.op_timings().len()),
            br_time: brt.extended_layer_time(),
        }
    }
}

/// Re-parses a `quantizer_matrix_obu()` so `--json` can expose its key fields. Large
/// matrices are summarized as shape labels only, never dumped by default.
fn quantizer_matrix_view(obu: &ObuEnvelope<'_>) -> Option<QuantizerMatrixView> {
    if obu.header.obu_type != ObuType::QuantizationMatrix {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_quantizer_matrix(&mut reader)
        .ok()
        .map(|qm| QuantizerMatrixView::new(&qm))
}

/// A compact, machine-readable view of a parsed `quantizer_matrix_obu()`. Coefficient
/// matrices are summarized by shape, not dumped.
#[derive(Serialize)]
struct QuantizerMatrixView {
    qm_bit_map: u16,
    num_planes: u8,
    is_reset: bool,
    levels: Vec<QuantizerMatrixLevelView>,
}

impl QuantizerMatrixView {
    fn new(qm: &QuantizerMatrixObu) -> Self {
        let levels = qm
            .levels
            .iter()
            .map(|level| QuantizerMatrixLevelView {
                level: level.level,
                is_default: level.is_default,
                matrix_shapes: level
                    .matrices
                    .as_ref()
                    .map(|transforms| {
                        transforms
                            .iter()
                            .map(|transform| transform.transform.shape_label())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Self {
            qm_bit_map: qm.qm_bit_map,
            num_planes: qm.num_planes,
            is_reset: qm.is_reset(),
            levels,
        }
    }
}

/// A compact per-level summary for `QuantizerMatrixView`.
#[derive(Serialize)]
struct QuantizerMatrixLevelView {
    level: u8,
    is_default: bool,
    /// Fundamental transform shapes present for a user-defined level (empty for a
    /// default level).
    matrix_shapes: Vec<&'static str>,
}

/// Re-parses a `film_grain_obu()` so `--json` can expose its key fields. Scaling
/// points and AR-coefficient arrays are summarized by count, not dumped.
fn film_grain_view(obu: &ObuEnvelope<'_>) -> Option<FilmGrainView> {
    if obu.header.obu_type != ObuType::FilmGrain {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_film_grain(&mut reader)
        .ok()
        .map(|fg| FilmGrainView::new(&fg))
}

/// A compact, machine-readable view of a parsed `film_grain_obu()`.
#[derive(Serialize)]
struct FilmGrainView {
    fgm_update_flags: u8,
    fgm_chroma_idc: u32,
    monochrome: bool,
    updated_slots: Vec<u8>,
    models: Vec<FilmGrainModelView>,
}

impl FilmGrainView {
    fn new(fg: &FilmGrainObu) -> Self {
        let models = fg
            .models
            .iter()
            .map(|update| FilmGrainModelView {
                slot: update.slot,
                chroma_scaling_from_luma: update.model.chroma_scaling_from_luma,
                num_y_points: update.model.num_y_points,
                num_cb_points: update.model.num_cb_points,
                num_cr_points: update.model.num_cr_points,
                ar_coeff_lag: update.model.ar_coeff_lag,
                overlap_flag: update.model.overlap_flag,
                clip_to_restricted_range: update.model.clip_to_restricted_range,
                film_grain_block_size: update.model.film_grain_block_size,
            })
            .collect();
        Self {
            fgm_update_flags: fg.update_flags,
            fgm_chroma_idc: fg.chroma_idc,
            monochrome: fg.monochrome,
            updated_slots: fg.models.iter().map(|update| update.slot).collect(),
            models,
        }
    }
}

/// A compact per-slot model summary for `FilmGrainView`.
#[derive(Serialize)]
struct FilmGrainModelView {
    slot: u8,
    chroma_scaling_from_luma: bool,
    num_y_points: u8,
    num_cb_points: u8,
    num_cr_points: u8,
    ar_coeff_lag: u8,
    overlap_flag: bool,
    clip_to_restricted_range: bool,
    film_grain_block_size: bool,
}

/// Re-parses a frame-bearing OBU's prefix so `--json` can expose the
/// activation/reference fields. This is **prefix-only** data, never a complete frame
/// header. The inspector does not model temporal-unit state, so `FirstPictureInTU` is
/// withheld (`None`) and `startCVS` is not surfaced.
fn frame_header_prefix_view(obu: &ObuEnvelope<'_>) -> Option<FrameHeaderPrefixView> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let prefix = if obu_type.is_tile_group() {
        parse_tile_group_prefix(&mut reader, obu_type, None)
            .ok()
            .and_then(|tile_group| tile_group.frame_header)?
    } else if obu_type.is_sef() || obu_type.is_tip_frame() || obu_type == ObuType::BridgeFrame {
        parse_frame_header_prefix(&mut reader, obu_type, None).ok()?
    } else {
        return None;
    };
    Some(FrameHeaderPrefixView::new(&prefix))
}

/// Surfaces the `frame_header_copy()` region of a non-first tile group (AV2 § 5.18.1).
///
/// A non-first tile group (`is_first_tile_group == 0`) with `frame_header_present_flag ==
/// 1` carries `frame_header_copy()` — a bit copy of the first tile group's frame header
/// (§ 5.18.1 mirror :3960-3981). The inspector is stateless per OBU, so it surfaces the
/// *presence* and start position of the copy region; the § 6.17.1 bit-identity comparison
/// against the first header (`frame-header/copy-bits-mismatch` /
/// `frame-header/copy-bits-truncated`) is a stateful validator check, not surfaced here.
fn frame_header_copy_view(obu: &ObuEnvelope<'_>) -> Option<FrameHeaderCopyView> {
    if !obu.header.obu_type.is_tile_group() {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let is_first_tile_group = reader.read_bit().ok()? != 0;
    if is_first_tile_group {
        // A first tile group carries frame_header( 1 ), surfaced via frame_header_core; it
        // has no copy region.
        return None;
    }
    let frame_header_present_flag = reader.read_bit().ok()? != 0;
    if !frame_header_present_flag {
        // frame_header_present_flag == 0: no frame_header_copy() in this tile group.
        return None;
    }
    Some(FrameHeaderCopyView {
        payload_kind: "frame_header_copy",
        // The copy region begins at the reader's current position, AFTER the two prefix
        // bits (is_first_tile_group + frame_header_present_flag). Those bits are still
        // within the first payload byte, so `byte_offset()` alone points at the byte
        // CONTAINING the prefix bits — a byte-only field would let a consumer mistake the
        // two prefix bits for copy bits. Pair it with the MSB-first bit position within
        // that byte (== 2 here) so the copy region's first bit is locatable exactly.
        copy_region_start_byte: reader.byte_offset().get(),
        copy_region_start_bit: reader.bit_offset().get(),
        // The comparison needs the coded frame's first header (cross-OBU state the
        // stateless inspector does not hold); the validator performs it.
        compared: false,
    })
}

/// A non-first tile group's `frame_header_copy()` presence view for `--json`.
///
/// The copy region's start is byte+bit precise: `copy_region_start_byte` is the
/// absolute byte offset of the byte containing the region's first bit, and
/// `copy_region_start_bit` is that bit's MSB-first position (`0..=7`) within the byte.
/// The region begins after the two `tile_group_obu()` prefix bits, so a byte-only
/// position would be ambiguous about whether the prefix bits are copy bits.
#[derive(Serialize)]
struct FrameHeaderCopyView {
    payload_kind: &'static str,
    copy_region_start_byte: u64,
    copy_region_start_bit: u8,
    compared: bool,
}

/// The § 5.19 `tile_group_obu()` structure after `frame_header()` for `--json` (AV2
/// § 5.19). Surfaced only for the FIRST tile group of an intra-complete coded frame
/// (`status` records whether the structure parsed fully or truncated); `header_bytes` /
/// `payload_size` record the `headerBytes` / unparsed § 5.20 payload boundary.
#[derive(Serialize)]
struct TileGroupStructureView {
    payload_kind: &'static str,
    num_tiles: u32,
    tile_start_and_end_present_flag: bool,
    tg_start: u32,
    tg_end: u32,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    header_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_size: Option<u64>,
    /// The § 5.20.1 per-tile framing (offset/size per tile), present only when the structure
    /// is complete and the framing was decidable (AV2 § 5.20.1). Empty/absent otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tile_framing: Vec<TileFramingView>,
    /// The provable § 5.20.1 framing defect label, when the framing found one.
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_framing_defect: Option<&'static str>,
}

/// One tile's § 5.20.1 byte framing for `--json`: its `tile_size_minus_1` length-field offset
/// (absent for the last/bridge tiles) and its `tileSize`-byte coded-tile region, all as byte
/// offsets relative to the `tile_group_payload()` region start (AV2 § 5.20.1).
#[derive(Serialize)]
struct TileFramingView {
    tile_num: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_field_offset: Option<u64>,
    tile_data_offset: u64,
    tile_size: u64,
}

/// A prefix-only view of a parsed `frame_header_info()` for `--json`. The
/// `payload_kind` / `prefix_status` labels make explicit that this is not a complete
/// frame header (AV2 § 5.18 is only prefix-parsed).
#[derive(Serialize)]
struct FrameHeaderPrefixView {
    payload_kind: &'static str,
    prefix_status: &'static str,
    cur_mfh_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq_header_id_in_frame_header: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referenced_sequence_header_id: Option<u8>,
    is_key_frame: bool,
    is_bridge: bool,
    is_regular: bool,
}

impl FrameHeaderPrefixView {
    fn new(prefix: &FrameHeaderPrefix) -> Self {
        Self {
            payload_kind: "frame_header_prefix",
            prefix_status: prefix.status.label(),
            cur_mfh_id: prefix.cur_mfh_id.get(),
            seq_header_id_in_frame_header: prefix.seq_header_id_in_frame_header,
            referenced_sequence_header_id: prefix.referenced_sequence_header_id.map(|id| id.get()),
            is_key_frame: prefix.is_key_frame,
            is_bridge: prefix.is_bridge,
            is_regular: prefix.is_regular,
        }
    }
}

/// Resolves the sequence header a frame references from the inspector's running map of
/// seen sequence headers: a `cur_mfh_id == 0` frame references one directly
/// (`referenced_sequence_header_id`); a `cur_mfh_id > 0` frame references it through the
/// resolved multi-frame header's `mfh_seq_header_id` (AV2 § 5.18.2 `load_sequence_header`).
fn resolve_inspect_sequence<'a>(
    obu: &ObuEnvelope<'_>,
    sequences: &'a BTreeMap<u8, SequenceHeader>,
    mfh_record: Option<&MultiFrameHeaderRecord>,
) -> Option<&'a SequenceHeader> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let prefix = if obu_type.is_tile_group() {
        parse_tile_group_prefix(&mut reader, obu_type, None)
            .ok()?
            .frame_header?
    } else {
        parse_frame_header_prefix(&mut reader, obu_type, None).ok()?
    };
    let seq_id = if prefix.cur_mfh_id.is_zero() {
        prefix.referenced_sequence_header_id?
    } else {
        // cur_mfh_id > 0: the referenced sequence header is the resolved MFH's
        // mfh_seq_header_id (§ 7.3.8.7), available only when the MFH was seen in-band.
        mfh_record?.mfh_seq_header_id
    };
    sequences.get(&seq_id.get())
}

/// Runs the frame-header **core** parser against the active sequence header (when one
/// is resolvable) and exposes its parse status and known core fields. Falls back to
/// the activation-only result when the sequence is unavailable. For a `cur_mfh_id > 0`
/// frame, the in-band multi-frame header resolving that reference (when seen) is passed
/// in so the § 5.18.4.1 default dimensions and § 5.18.7.1 segmentation arm are
/// surfaced; an unresolved reference leaves the parse at its unsupported stop.
fn frame_header_core_view(
    obu: &ObuEnvelope<'_>,
    sequences: &BTreeMap<u8, SequenceHeader>,
    multi_frame_headers: &BTreeMap<u32, MultiFrameHeaderRecord>,
) -> Option<FrameHeaderCoreView> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    if obu_type.is_tile_group() {
        // Only the first tile group carries a parseable frame_header(1) (AV2 § 5.19).
        if reader.read_bit().ok()? == 0 {
            return None;
        }
    } else if !(obu_type.is_sef() || obu_type.is_tip_frame() || obu_type == ObuType::BridgeFrame) {
        return None;
    }
    // For tile-group OBUs, `reader` is now past the frame_header_present_flag bit and
    // is what parse_frame_header_core consumes; resolve_inspect_sequence deliberately
    // uses its own fresh reader to re-parse the small activation prefix (not a
    // reader-position bug).
    // Resolve the frame's `cur_mfh_id` (> 0) against the in-band multi-frame-header
    // store via a fresh activation-prefix parse (cur_mfh_id precedes any
    // sequence-dependent field, so it is reliable without sequence state).
    let mfh_record = resolve_inspect_mfh(obu, multi_frame_headers);
    let active_sequence = resolve_inspect_sequence(obu, sequences, mfh_record);
    let input = FrameHeaderParseInput {
        obu_type,
        first_picture_in_tu: false,
        active_sequence,
        mfh_record,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let core = parse_frame_header_core(&mut reader, &input).ok()?;
    Some(FrameHeaderCoreView::new(&core))
}

/// Surfaces the § 5.19 `tile_group_obu()` structure after `frame_header()` for the FIRST
/// tile group of an intra-complete coded frame (AV2 § 5.19). Decidable only when the
/// frame header reaches [`FrameHeaderParseStatus::IntraHeaderComplete`] on the intra path
/// (so `use_bru`/`bru_inactive` derive to 0 and the BRU arms are dead) with a parsed
/// `tile_info()`; otherwise `None` (the BRU-undecidable honest stop).
///
/// A CONTINUATION tile group (`is_first_tile_group == 0`) is deliberately NOT surfaced
/// here: its § 5.19 structure derives from the first tile group of the SAME coded frame
/// in the SAME layer triple, and that pairing is the validator's state — the
/// `(xlayer, mlayer, tlayer)` key plus the segmenter's `FrameBoundary` decisions
/// (including the Ambiguous poison). A most-recent-first heuristic would mis-pair
/// interleaved-layer streams and fabricate a false framing view, so the per-OBU
/// inspector omits the view instead (the same scoping as [`frame_header_copy_view`],
/// which surfaces presence only). The continuation's framing IS validated — the
/// `tile-payload/*` diagnostics run on continuations via the validator's recorded
/// first-header layout. A stateful inspect surface for continuations is a named
/// residual of `AV2-5.20-TILE-GROUP-PAYLOAD`.
fn tile_group_structure_view(
    obu: &ObuEnvelope<'_>,
    sequences: &BTreeMap<u8, SequenceHeader>,
    multi_frame_headers: &BTreeMap<u32, MultiFrameHeaderRecord>,
) -> Option<TileGroupStructureView> {
    let obu_type = obu.header.obu_type;
    if !obu_type.is_tile_group() {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    // Only the first tile group carries a parseable frame_header(1) (AV2 § 5.19).
    if reader.read_bit().ok()? == 0 {
        return None;
    }
    let mfh_record = resolve_inspect_mfh(obu, multi_frame_headers);
    let active_sequence = resolve_inspect_sequence(obu, sequences, mfh_record);
    let input = FrameHeaderParseInput {
        obu_type,
        first_picture_in_tu: false,
        active_sequence,
        mfh_record,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let core = parse_frame_header_core(&mut reader, &input).ok()?;
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete
        || core.frame_is_intra != Some(true)
    {
        return None;
    }
    let tile_info = core.tile_info.as_ref()?;
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    // `reader` is positioned past frame_header(); parse the structure from the same reader.
    let structure =
        parse_tile_group_structure(&mut reader, layout, obu.payload.len() as u64).ok()?;

    // §5.20.1: surface the per-tile framing over the tile_group_payload() region when the
    // structure completed and the range is self-consistent. IsBridge == 0 on this
    // intra-complete tile-group path; TileSizeBytes comes from tile_info() (None == single
    // tile, framed as the lone last tile).
    let (mut tile_framing, mut tile_framing_defect) = (Vec::new(), None);
    if let (Some(header_bytes), Some(payload_size)) =
        (structure.header_bytes, structure.payload_size)
        && structure.tg_end >= structure.tg_start
    {
        let num_tiles_in_group = u64::from(structure.tg_end - structure.tg_start) + 1;
        let tsb = match tile_info.tile_size_bytes {
            Some(tsb) if (1..=4).contains(&tsb) => Some(tsb),
            None if num_tiles_in_group == 1 => Some(1),
            _ => None,
        };
        if let Some(tsb) = tsb {
            let start = usize::try_from(header_bytes).unwrap_or(usize::MAX);
            let end =
                usize::try_from(header_bytes.saturating_add(payload_size)).unwrap_or(usize::MAX);
            if let Some(region) = obu.payload.get(start..end.min(obu.payload.len())) {
                let framing = parse_tile_group_framing(
                    region,
                    structure.tg_start,
                    structure.tg_end,
                    tsb,
                    false,
                );
                tile_framing = framing
                    .tiles
                    .iter()
                    .map(|t| TileFramingView {
                        tile_num: t.tile_num,
                        size_field_offset: t.size_field_offset,
                        tile_data_offset: t.tile_data_offset,
                        tile_size: t.tile_size,
                    })
                    .collect();
                tile_framing_defect = framing.defect.map(|d| d.label());
            }
        }
    }

    Some(TileGroupStructureView {
        payload_kind: "tile_group_structure",
        num_tiles: layout.num_tiles,
        tile_start_and_end_present_flag: structure.tile_start_and_end_present_flag,
        tg_start: structure.tg_start,
        tg_end: structure.tg_end,
        status: structure.outcome.label(),
        header_bytes: structure.header_bytes,
        payload_size: structure.payload_size,
        tile_framing,
        tile_framing_defect,
    })
}

/// Resolves a frame's `cur_mfh_id` (> 0, in range) to the in-band multi-frame-header
/// record that defines it, if one has been seen. `None` for a `cur_mfh_id == 0` direct
/// reference or an unresolved id.
fn resolve_inspect_mfh<'a>(
    obu: &ObuEnvelope<'_>,
    multi_frame_headers: &'a BTreeMap<u32, MultiFrameHeaderRecord>,
) -> Option<&'a MultiFrameHeaderRecord> {
    let obu_type = obu.header.obu_type;
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    let prefix = if obu_type.is_tile_group() {
        parse_tile_group_prefix(&mut reader, obu_type, None)
            .ok()?
            .frame_header?
    } else {
        parse_frame_header_prefix(&mut reader, obu_type, None).ok()?
    };
    let cur_mfh_id = prefix.cur_mfh_id;
    if cur_mfh_id.is_zero() || !cur_mfh_id.in_range() {
        return None;
    }
    multi_frame_headers.get(&cur_mfh_id.get())
}

/// A frame-header core summary for `--json`. `status` makes explicit how far the core
/// parser reached (AV2 § 5.18.2); only known fields are serialized.
#[derive(Serialize)]
struct FrameHeaderCoreView {
    payload_kind: &'static str,
    status: &'static str,
    cur_mfh_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq_header_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_existing_frame: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_is_intra: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    immediate_output_frame: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    implicit_output_frame: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order_hint_lsb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_frame_flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_size: Option<FrameSizeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bridge_frame_ref_idx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_to_show_map_idx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_layout: Option<TileLayoutView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantization: Option<QuantizationParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segmentation: Option<SegmentationParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qm_params: Option<SetupQmParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta_q: Option<DeltaQParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lossless: Option<LosslessInfoView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deblocking: Option<DeblockingFilterParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gdf: Option<GdfParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cdef: Option<CdefParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lr: Option<LrParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lr_partial: Option<LrPartialParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso: Option<CcsoParamsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intra_tail: Option<FrameHeaderTailView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sef_film_grain: Option<FilmGrainConfigView>,
    /// The §5.2.1 / §5.2.3 show-existing-frame `trailing_bits()` classification (stable
    /// label), present only on a completed SEF header. A value other than `"valid"` means
    /// the SEF payload tail is non-conformant (surfaced as a diagnostic by the validator).
    #[serde(skip_serializing_if = "Option::is_none")]
    sef_trailing_bits: Option<&'static str>,
    /// The parsed §5.18.2 non-intra control region (inter / switch / TIP path), present
    /// only when the frame is non-intra. Surfaces the primary-reference signaling, the
    /// explicit reference map, the reference-grounded frame size, the BRU triple, the MV
    /// precision / interpolation filter / motion modes, and the inter stop class.
    #[serde(skip_serializing_if = "Option::is_none")]
    inter: Option<InterControlView>,
    consumed_bits: u64,
}

/// The parsed §5.18.2 non-intra control region for `--json` (AV2 § 5.18.2).
#[derive(Serialize)]
struct InterControlView {
    /// The inter stop class (stable label).
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal_primary_ref_frame: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disable_cross_frame_cdf_init: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_ref_frame: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bridge_frame_overwrite_flag: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    explicit_ref_frame_map: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_total_refs: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ref_frame_idx: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_size: Option<FrameSizeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_bru: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bru_ref: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bru_inactive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_ref_frame_mvs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tmvp_sample_step_minus_1: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tip_frame_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_drl_bits_minus_1: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mv_precision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interpolation_filter: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_enabled_motion_modes: Option<Vec<bool>>,
    /// `disable_cdf_update` (AV2 § 5.18.2, mirror :5041), read on the ordinary inter /
    /// switch path immediately before the shared tail (`InterStop::ReachedSharedTail`).
    #[serde(skip_serializing_if = "Option::is_none")]
    disable_cdf_update: Option<bool>,
    has_invalid_ref_frame_idx: bool,
}

impl InterControlView {
    fn new(inter: &InterControl) -> Self {
        Self {
            stop: inter.stop.map(|stop| stop.label()),
            signal_primary_ref_frame: inter.signal_primary_ref_frame,
            disable_cross_frame_cdf_init: inter.disable_cross_frame_cdf_init,
            primary_ref_frame: inter.primary_ref_frame,
            bridge_frame_overwrite_flag: inter.bridge_frame_overwrite_flag,
            explicit_ref_frame_map: inter.explicit_ref_frame_map,
            num_total_refs: inter.num_total_refs,
            ref_frame_idx: inter.ref_frame_idx.clone(),
            frame_size: inter.frame_size.map(|size| FrameSizeView {
                width: size.width,
                height: size.height,
            }),
            use_bru: inter.use_bru,
            bru_ref: inter.bru_ref,
            bru_inactive: inter.bru_inactive,
            use_ref_frame_mvs: inter.use_ref_frame_mvs,
            tmvp_sample_step_minus_1: inter.tmvp_sample_step_minus_1,
            tip_frame_mode: inter.tip_frame_mode.map(|mode| mode.label()),
            max_drl_bits_minus_1: inter.max_drl_bits_minus_1,
            mv_precision: inter.mv_precision.map(|precision| precision.label()),
            interpolation_filter: inter.interpolation_filter.map(|filter| filter.label()),
            frame_enabled_motion_modes: inter
                .frame_enabled_motion_modes
                .map(|modes| modes.to_vec()),
            disable_cdf_update: inter.disable_cdf_update,
            has_invalid_ref_frame_idx: inter.has_invalid_ref_frame_idx,
        }
    }
}

/// A frame's parsed luma dimensions for `--json`.
#[derive(Serialize)]
struct FrameSizeView {
    width: u32,
    height: u32,
}

/// A parsed frame `tile_info()` layout for `--json` (AV2 § 5.18.7.2).
#[derive(Serialize)]
struct TileLayoutView {
    reuse_tile_info: bool,
    tile_cols: u32,
    tile_rows: u32,
    tile_cols_log2: u8,
    tile_rows_log2: u8,
    context_update_tile_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_size_bytes: Option<u32>,
}

impl TileLayoutView {
    fn new(tile_info: &TileInfo) -> Self {
        Self {
            reuse_tile_info: tile_info.reuse_tile_info,
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            tile_cols_log2: tile_info.tile_cols_log2,
            tile_rows_log2: tile_info.tile_rows_log2,
            context_update_tile_id: tile_info.context_update_tile_id,
            tile_size_bytes: tile_info.tile_size_bytes,
        }
    }
}

/// Parsed `quantization_params()` for `--json` (AV2 § 5.18.6.1).
#[derive(Serialize)]
struct QuantizationParamsView {
    base_q_idx: u32,
    delta_q_y_dc: i32,
    delta_q_u_dc: i32,
    delta_q_u_ac: i32,
    delta_q_v_dc: i32,
    delta_q_v_ac: i32,
    diff_uv_delta: bool,
}

impl QuantizationParamsView {
    fn new(params: &QuantizationParams) -> Self {
        Self {
            base_q_idx: params.base_q_idx,
            delta_q_y_dc: params.delta_q_y_dc,
            delta_q_u_dc: params.delta_q_u_dc,
            delta_q_u_ac: params.delta_q_u_ac,
            delta_q_v_dc: params.delta_q_v_dc,
            delta_q_v_ac: params.delta_q_v_ac,
            diff_uv_delta: params.diff_uv_delta,
        }
    }
}

/// One enabled `FeatureEnabled[i][j]` / `FeatureData[i][j]` entry for `--json`
/// (AV2 § 5.18.7.1). Disabled features are omitted to keep the summary compact.
#[derive(Serialize)]
struct SegmentFeatureView {
    segment_id: u8,
    feature: u8,
    data: i32,
}

/// Parsed `segmentation_params()` for `--json` (AV2 § 5.18.7.1).
#[derive(Serialize)]
struct SegmentationParamsView {
    segmentation_enabled: bool,
    reuse_seg_info: bool,
    segmentation_update_map: bool,
    segmentation_temporal_update: bool,
    seg_id_pre_skip: bool,
    last_active_seg_id: u8,
    enabled_features: Vec<SegmentFeatureView>,
}

impl SegmentationParamsView {
    fn new(params: &SegmentationParams) -> Self {
        let enabled_features = params
            .features
            .iter()
            .enumerate()
            .flat_map(|(segment_id, levels)| {
                levels.iter().enumerate().filter_map(move |(feature, f)| {
                    f.enabled.then_some(SegmentFeatureView {
                        segment_id: segment_id as u8,
                        feature: feature as u8,
                        data: f.data,
                    })
                })
            })
            .collect();
        Self {
            segmentation_enabled: params.segmentation_enabled,
            reuse_seg_info: params.reuse_seg_info,
            segmentation_update_map: params.segmentation_update_map,
            segmentation_temporal_update: params.segmentation_temporal_update,
            seg_id_pre_skip: params.seg_id_pre_skip,
            last_active_seg_id: params.last_active_seg_id,
            enabled_features,
        }
    }
}

/// One `(qm_y[i], qm_u[i], qm_v[i])` level set for `--json` (AV2 § 5.18.6.2).
#[derive(Serialize)]
struct QmSetLevelsView {
    qm_y: u8,
    qm_u: u8,
    qm_v: u8,
}

/// Parsed `setup_qm_params()` for `--json` (AV2 § 5.18.6.2). `levels` carries only
/// the `pic_qm_num_minus_1 + 1` parsed sets, and only when `using_qmatrix`.
#[derive(Serialize)]
struct SetupQmParamsView {
    using_qmatrix: bool,
    pic_qm_num_minus_1: u8,
    levels: Vec<QmSetLevelsView>,
}

impl SetupQmParamsView {
    fn new(params: &SetupQmParams) -> Self {
        let qm_num = if params.using_qmatrix {
            usize::from(params.pic_qm_num_minus_1) + 1
        } else {
            0
        };
        Self {
            using_qmatrix: params.using_qmatrix,
            pic_qm_num_minus_1: params.pic_qm_num_minus_1,
            levels: params
                .levels
                .iter()
                .take(qm_num)
                .map(|set| QmSetLevelsView {
                    qm_y: set.qm_y,
                    qm_u: set.qm_u,
                    qm_v: set.qm_v,
                })
                .collect(),
        }
    }
}

/// Parsed `delta_q_params()` for `--json` (AV2 § 5.18.7.8).
#[derive(Serialize)]
struct DeltaQParamsView {
    delta_q_present: bool,
    delta_q_res: u8,
}

impl DeltaQParamsView {
    fn new(params: &DeltaQParams) -> Self {
        Self {
            delta_q_present: params.delta_q_present,
            delta_q_res: params.delta_q_res,
        }
    }
}

/// The § 5.18.2 per-segment lossless derivation and `allow_tcq` /
/// `allow_parity_hiding` summary for `--json`.
#[derive(Serialize)]
struct LosslessInfoView {
    coded_lossless: bool,
    has_lossless_segment: bool,
    allow_tcq: bool,
    allow_parity_hiding: bool,
}

impl LosslessInfoView {
    fn new(info: &LosslessInfo) -> Self {
        Self {
            coded_lossless: info.coded_lossless,
            has_lossless_segment: info.has_lossless_segment,
            allow_tcq: info.allow_tcq,
            allow_parity_hiding: info.allow_parity_hiding,
        }
    }
}

/// Parsed `deblocking_filter_params()` for `--json` (AV2 § 5.18.5.2).
#[derive(Serialize)]
struct DeblockingFilterParamsView {
    apply_deblocking_filter: [bool; 4],
    df_delta_q_present: [bool; 4],
    df_delta_q: [i32; 4],
}

impl DeblockingFilterParamsView {
    fn new(params: &DeblockingFilterParams) -> Self {
        Self {
            apply_deblocking_filter: params.apply_deblocking_filter,
            df_delta_q_present: params.df_delta_q_present,
            df_delta_q: params.df_delta_q,
        }
    }
}

/// Parsed `gdf_params()` for `--json` (AV2 § 5.18.7.9). The per-frame fields are
/// omitted when GDF is not frame-enabled.
#[derive(Serialize)]
struct GdfParamsView {
    gdf_frame_enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    gdf_per_block: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gdf_pic_qc_idx: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gdf_pic_scale_idx: Option<u8>,
}

impl GdfParamsView {
    fn new(params: &GdfParams) -> Self {
        Self {
            gdf_frame_enable: params.gdf_frame_enable,
            gdf_per_block: params.gdf_per_block,
            gdf_pic_qc_idx: params.gdf_pic_qc_idx,
            gdf_pic_scale_idx: params.gdf_pic_scale_idx,
        }
    }
}

/// One CDEF strength set for `--json` (AV2 § 5.18.7.10).
#[derive(Serialize)]
struct CdefStrengthSetView {
    y_pri_strength: u8,
    y_sec_strength: u8,
    uv_pri_strength: u8,
    uv_sec_strength: u8,
}

impl CdefStrengthSetView {
    fn new(set: &CdefStrengthSet) -> Self {
        Self {
            y_pri_strength: set.y_pri_strength,
            y_sec_strength: set.y_sec_strength,
            uv_pri_strength: set.uv_pri_strength,
            uv_sec_strength: set.uv_sec_strength,
        }
    }
}

/// Parsed `cdef_params()` for `--json` (AV2 § 5.18.7.10). The per-frame fields are
/// omitted when CDEF is not frame-enabled.
#[derive(Serialize)]
struct CdefParamsView {
    cdef_frame_enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cdef_damping: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cdef_strengths: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cdef_on_skip_txfm_frame_enable: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    strengths: Vec<CdefStrengthSetView>,
}

impl CdefParamsView {
    fn new(params: &CdefParams) -> Self {
        Self {
            cdef_frame_enable: params.cdef_frame_enable,
            cdef_damping: params.cdef_damping,
            cdef_strengths: params.cdef_strengths,
            cdef_on_skip_txfm_frame_enable: params.cdef_on_skip_txfm_frame_enable,
            strengths: params
                .strengths
                .iter()
                .map(CdefStrengthSetView::new)
                .collect(),
        }
    }
}

/// One plane's parsed `lr_params()` state for `--json` (AV2 § 5.18.7.11).
#[derive(Serialize)]
struct LrPlaneParamsView {
    restoration_type: &'static str,
    frame_filters_on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_filter_classes: Option<u8>,
}

impl LrPlaneParamsView {
    fn new(plane: &LrPlaneParams) -> Self {
        Self {
            restoration_type: plane.restoration_type.label(),
            frame_filters_on: plane.frame_filters_on,
            num_filter_classes: plane.num_filter_classes,
        }
    }
}

/// Parsed `lr_params()` for `--json` (AV2 § 5.18.7.11). The per-plane entries are empty
/// when loop restoration is disabled (the early return).
#[derive(Serialize)]
struct LrParamsView {
    uses_lr: bool,
    loop_restoration_size: [u32; 3],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    planes: Vec<LrPlaneParamsView>,
}

impl LrParamsView {
    fn new(params: &LrParams) -> Self {
        Self {
            uses_lr: params.uses_lr,
            loop_restoration_size: params.loop_restoration_size,
            planes: params.planes.iter().map(LrPlaneParamsView::new).collect(),
        }
    }
}

/// The partial `lr_params()` facts for `--json` when the parse stopped before the unmodeled
/// frame-level Wiener bank decode (AV2 § 5.18.7.11, the core
/// `StoppedBeforeWienerNsFilter` status). This is surfaced under the distinct `lr_partial`
/// key (never `lr`) so a stopped parse is never reported as a complete one; `stopped_before`
/// records where the parse halted.
#[derive(Serialize)]
struct LrPartialParamsView {
    stopped_before: &'static str,
    uses_lr: bool,
    loop_restoration_size: [u32; 3],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    planes: Vec<LrPlaneParamsView>,
}

impl LrPartialParamsView {
    fn new(partial: &LrPartialParams) -> Self {
        Self {
            stopped_before: "read_wienerns_filter",
            uses_lr: partial.uses_lr,
            loop_restoration_size: partial.loop_restoration_size,
            planes: partial.planes.iter().map(LrPlaneParamsView::new).collect(),
        }
    }
}

/// One plane's parsed `ccso_params()` state for `--json` (AV2 § 5.18.7.12).
#[derive(Serialize)]
struct CcsoPlaneParamsView {
    ccso_planes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_bo_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_scale_idx: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_quant_idx: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_ext_filter: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_edge_clf: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_max_band_log2: Option<u8>,
}

impl CcsoPlaneParamsView {
    fn new(plane: &CcsoPlaneParams) -> Self {
        Self {
            ccso_planes: plane.ccso_planes,
            ccso_bo_only: plane.ccso_bo_only,
            ccso_scale_idx: plane.ccso_scale_idx,
            ccso_quant_idx: plane.ccso_quant_idx,
            ccso_ext_filter: plane.ccso_ext_filter,
            ccso_edge_clf: plane.ccso_edge_clf,
            ccso_max_band_log2: plane.ccso_max_band_log2,
        }
    }
}

/// Parsed `ccso_params()` for `--json` (AV2 § 5.18.7.12). `ccso_frame_flag` is omitted
/// when CCSO is disabled (the early return leaves all planes off).
#[derive(Serialize)]
struct CcsoParamsView {
    #[serde(skip_serializing_if = "Option::is_none")]
    ccso_frame_flag: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    planes: Vec<CcsoPlaneParamsView>,
}

impl CcsoParamsView {
    fn new(params: &CcsoParams) -> Self {
        Self {
            ccso_frame_flag: params.ccso_frame_flag,
            planes: params.planes.iter().map(CcsoPlaneParamsView::new).collect(),
        }
    }
}

/// Parsed `film_grain_config()` for `--json` (AV2 § 5.18.10.1). `fgm_id` / `grain_seed`
/// are omitted when `apply_grain` is `0`.
#[derive(Serialize)]
struct FilmGrainConfigView {
    apply_grain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fgm_id: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grain_seed: Option<u16>,
}

impl FilmGrainConfigView {
    fn new(config: &FilmGrainConfig) -> Self {
        Self {
            apply_grain: config.apply_grain,
            fgm_id: config.fgm_id,
            grain_seed: config.grain_seed,
        }
    }
}

/// Parsed § 5.18.2 intra tail for `--json` (`read_tx_mode()` § 5.18.8.1,
/// `frame_reference_mode()` § 5.18.8.3, `skip_mode_params()` § 5.18.8.2, the inferred
/// `allow_bawp` / `allow_warpmv_mode`, `reduced_tx_set`, `global_motion_params()`
/// § 5.18.9.1 intra arm, and `film_grain_config()` § 5.18.10.1).
#[derive(Serialize)]
struct FrameHeaderTailView {
    tx_mode: &'static str,
    reference_select: bool,
    skip_mode_present: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    reduced_tx_set: u8,
    use_global_motion: bool,
    film_grain: FilmGrainConfigView,
}

impl FrameHeaderTailView {
    fn new(tail: &FrameHeaderTail) -> Self {
        Self {
            tx_mode: tail.tx_mode.label(),
            reference_select: tail.reference_select,
            skip_mode_present: tail.skip_mode_present,
            allow_bawp: tail.allow_bawp,
            allow_warpmv_mode: tail.allow_warpmv_mode,
            reduced_tx_set: tail.reduced_tx_set,
            use_global_motion: tail.use_global_motion,
            film_grain: FilmGrainConfigView::new(&tail.film_grain),
        }
    }
}

impl FrameHeaderCoreView {
    fn new(core: &FrameHeaderCore) -> Self {
        Self {
            payload_kind: "frame_header_core",
            status: core.status.label(),
            cur_mfh_id: core.cur_mfh_id.get(),
            seq_header_id: core.seq_header_id_in_frame_header,
            show_existing_frame: core.show_existing_frame,
            frame_type: core.frame_type.map(|frame_type| frame_type.label()),
            frame_is_intra: core.frame_is_intra,
            immediate_output_frame: core.immediate_output_frame,
            implicit_output_frame: core.implicit_output_frame,
            order_hint_lsb: core.order_hint_lsb,
            refresh_frame_flags: core.refresh_frame_flags,
            frame_size: core.frame_size.map(|size| FrameSizeView {
                width: size.width,
                height: size.height,
            }),
            bridge_frame_ref_idx: core.bridge_frame_ref_idx,
            frame_to_show_map_idx: core.frame_to_show_map_idx,
            tile_layout: core.tile_info.as_ref().map(TileLayoutView::new),
            quantization: core
                .quantization_params
                .as_ref()
                .map(QuantizationParamsView::new),
            segmentation: core
                .segmentation_params
                .as_ref()
                .map(SegmentationParamsView::new),
            qm_params: core.setup_qm_params.as_ref().map(SetupQmParamsView::new),
            delta_q: core.delta_q_params.as_ref().map(DeltaQParamsView::new),
            lossless: core.lossless_info.as_ref().map(LosslessInfoView::new),
            deblocking: core
                .deblocking_filter_params
                .as_ref()
                .map(DeblockingFilterParamsView::new),
            gdf: core.gdf_params.as_ref().map(GdfParamsView::new),
            cdef: core.cdef_params.as_ref().map(CdefParamsView::new),
            lr: core.lr_params.as_ref().map(LrParamsView::new),
            lr_partial: core
                .lr_params_partial
                .as_ref()
                .map(LrPartialParamsView::new),
            ccso: core.ccso_params.as_ref().map(CcsoParamsView::new),
            intra_tail: core.intra_tail.as_ref().map(FrameHeaderTailView::new),
            sef_film_grain: core.sef_film_grain.as_ref().map(FilmGrainConfigView::new),
            sef_trailing_bits: core.sef_trailing_bits.map(SefTrailingBits::label),
            inter: core.inter.as_ref().map(InterControlView::new),
            consumed_bits: core.consumed_bits,
        }
    }
}

/// Re-parses a content-interpretation OBU so `--json` can expose its parsed flags
/// and timing status.
fn content_interpretation_view(obu: &ObuEnvelope<'_>) -> Option<ContentInterpretationView> {
    if obu.header.obu_type != ObuType::ContentInterpretation {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_content_interpretation(&mut reader)
        .ok()
        .map(|ci| ContentInterpretationView::new(&ci))
}

/// A compact, machine-readable view of a parsed `content_interpretation_obu()`.
#[derive(Serialize)]
struct ContentInterpretationView {
    scan_type_idc: u8,
    color_description_present: bool,
    chroma_sample_position_present: bool,
    aspect_ratio_info_present: bool,
    timing_info_present: bool,
    reserved_2bit: u8,
}

impl ContentInterpretationView {
    fn new(ci: &ContentInterpretation) -> Self {
        Self {
            scan_type_idc: ci.scan_type_idc.get(),
            color_description_present: ci.color_description.is_some(),
            chroma_sample_position_present: ci.chroma_sample_position.is_some(),
            aspect_ratio_info_present: ci.aspect_ratio.is_some(),
            timing_info_present: ci.timing_info.is_some(),
            reserved_2bit: ci.reserved_2bit,
        }
    }
}

/// Re-parses a sequence-header OBU so `--json` can expose which `§5.4` child
/// sections were parsed and which (if any) are bounded as unimplemented.
fn sequence_header_view(obu: &ObuEnvelope<'_>) -> Option<SequenceHeaderView> {
    if obu.header.obu_type != ObuType::SequenceHeader {
        return None;
    }
    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    parse_sequence_header(&mut reader)
        .ok()
        .map(|header| SequenceHeaderView::new(&header))
}

/// A compact, machine-readable view of a parsed `sequence_header_obu()`.
#[derive(Serialize)]
struct SequenceHeaderView {
    seq_header_id: u8,
    seq_profile_idc: u8,
    single_picture_header_flag: bool,
    chroma_format_idc: u8,
    bit_depth: u8,
    max_tlayer_id: u8,
    max_mlayer_id: u8,
    fully_parsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unimplemented_at: Option<&'static str>,
    children: SequenceChildrenView,
}

impl SequenceHeaderView {
    fn new(header: &SequenceHeader) -> Self {
        let general = &header.general;
        Self {
            seq_header_id: general.seq_header_id.get(),
            seq_profile_idc: general.seq_profile_idc.get(),
            single_picture_header_flag: general.single_picture_header_flag,
            chroma_format_idc: general.chroma_format_idc.get(),
            bit_depth: general.bit_depth_idc.bit_depth(),
            max_tlayer_id: general.max_tlayer_id.get(),
            max_mlayer_id: general.max_mlayer_id.get(),
            fully_parsed: header.is_fully_parsed(),
            unimplemented_at: header.unimplemented_at,
            children: SequenceChildrenView {
                partition: header.partition.is_some(),
                segment: header.segment.is_some(),
                intra: header.intra.is_some(),
                inter: header.inter.is_some(),
                screen_content: header.screen_content.is_some(),
                transform_quant_entropy: header.transform_quant_entropy.is_some(),
                filter: header.filter.is_some(),
                tile: header.tile.is_some(),
                film_grain_params_present: header.film_grain_params_present.is_some(),
            },
        }
    }
}

/// Which `§5.4` child sections of a sequence header were parsed.
#[derive(Serialize)]
struct SequenceChildrenView {
    partition: bool,
    segment: bool,
    intra: bool,
    inter: bool,
    screen_content: bool,
    transform_quant_entropy: bool,
    filter: bool,
    tile: bool,
    film_grain_params_present: bool,
}

/// A serializable summary of how much OBU payload syntax is currently parsed.
///
/// For the 11 frame-carrying OBU types the stateless dispatcher reaches only the
/// `prefix_parsed_awaiting_state` status (its § 5.18.2 / § 5.19 activation prefix; the
/// rest needs the activated sequence header). The richer, state-aware surface for those
/// OBUs is the per-record `frame_header_prefix` / `frame_header_core` /
/// `frame_header_copy` / `tile_group_structure` views, which thread the running
/// sequence-header and multi-frame-header state.
#[derive(Serialize)]
struct InspectPayloadStatus {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    syntax: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feature: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocked_on: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl InspectPayloadStatus {
    fn new(obu: &ObuEnvelope<'_>) -> Self {
        match obu.payload_status() {
            Ok(PayloadStatus::Parsed(parsed)) => Self {
                status: "parsed",
                syntax: Some(parsed.syntax_name()),
                feature: Some(parsed.feature_id()),
                blocked_on: None,
                error: None,
            },
            Ok(PayloadStatus::Opaque(_)) => Self {
                status: "opaque",
                syntax: None,
                feature: None,
                blocked_on: None,
                error: None,
            },
            // The dispatcher parsed the frame-carrying OBU's state-free prefix; the
            // state-dependent remainder is surfaced by the stateful frame-header views.
            Ok(PayloadStatus::PrefixParsed {
                prefix,
                blocked_on,
                feature,
            }) => Self {
                status: "prefix_parsed_awaiting_state",
                syntax: Some(prefix.label()),
                feature: Some(feature),
                blocked_on: Some(blocked_on),
                error: None,
            },
            Ok(PayloadStatus::Unimplemented { feature, .. }) => Self {
                status: "unimplemented",
                syntax: None,
                feature: Some(feature),
                blocked_on: None,
                error: None,
            },
            Err(error) => Self {
                status: "invalid",
                syntax: None,
                feature: Some("AV2-5.2.1-OBU-DISPATCH"),
                blocked_on: None,
                error: Some(error.to_string()),
            },
        }
    }
}

/// Runs `splot inspect`.
///
/// Exit codes: `0` on success, `1` if a structural parse error is hit (the OBUs
/// parsed before it are still printed), and `2` (via an `Err`) for I/O failures.
///
/// # Errors
/// Returns an error if the input file cannot be read or the output cannot be
/// serialized.
pub fn run(args: &InspectArgs) -> Result<ExitCode> {
    let data = read_input(&args.input)?;
    // Use the partial parser so the OBUs before a malformed tail are still shown.
    let parsed = parse_bitstream_partial(&data);

    if args.json {
        let records = inspect_records(&parsed);
        let json =
            serde_json::to_string_pretty(&records).context("failed to serialize inspection")?;
        println!("{json}");
    } else {
        print_human(&parsed, args.headers);
    }

    if print_parse_errors(&parsed) {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::from(0))
    }
}

fn inspect_records(parsed: &ParsedBitstream<'_>) -> Vec<InspectRecord> {
    // Track sequence headers in OBU order so a later frame header's core parse can
    // resolve the sequence state it references (AV2 § 5.18.2 load_sequence_header).
    let mut sequences: BTreeMap<u8, SequenceHeader> = BTreeMap::new();
    // Track in-band multi-frame headers (keyed by mfhId) so a later frame header's
    // `cur_mfh_id > 0` core parse can resolve its § 5.7 state (AV2 § 5.18.2).
    let mut multi_frame_headers: BTreeMap<u32, MultiFrameHeaderRecord> = BTreeMap::new();
    let mut records = Vec::new();

    match parsed {
        ParsedBitstream::AnnexB(parsed) => {
            records.reserve(parsed.obus.len());
            for obu in &parsed.obus {
                push_inspect_record(
                    &mut records,
                    obu,
                    &mut sequences,
                    &mut multi_frame_headers,
                    None,
                    None,
                );
            }
        }
        ParsedBitstream::Ivf(parsed) => {
            let ivf_header = parsed.header;
            let capacity = parsed.frames.iter().map(|frame| frame.obus.len()).sum();
            records.reserve(capacity);
            for frame in &parsed.frames {
                for obu in &frame.obus {
                    push_inspect_record(
                        &mut records,
                        obu,
                        &mut sequences,
                        &mut multi_frame_headers,
                        ivf_header,
                        Some(frame.frame),
                    );
                }
            }
        }
    }

    records
}

fn push_inspect_record(
    records: &mut Vec<InspectRecord>,
    obu: &ObuEnvelope<'_>,
    sequences: &mut BTreeMap<u8, SequenceHeader>,
    multi_frame_headers: &mut BTreeMap<u32, MultiFrameHeaderRecord>,
    ivf_header: Option<IvfHeader>,
    ivf_frame: Option<IvfFrame<'_>>,
) {
    let index = records.len();
    records.push(InspectRecord::new(
        index,
        obu,
        sequences,
        multi_frame_headers,
        ivf_header,
        ivf_frame,
    ));
    match obu.header.obu_type {
        ObuType::SequenceHeader => {
            let mut reader = BitReader::new(obu.payload, obu.payload_offset());
            if let Ok(sequence) = parse_sequence_header(&mut reader) {
                sequences.insert(sequence.general.seq_header_id.get(), sequence);
            }
        }
        ObuType::MultiFrameHeader => {
            // Record the parsed § 5.7 state (frame size, segmentation arm, deblocking
            // update) keyed by mfhId, mirroring the validator's availability record, so
            // a later `cur_mfh_id` reference resolves the same view.
            let mut reader = BitReader::new(obu.payload, obu.payload_offset());
            if let Ok(mfh) = parse_multi_frame_header(&mut reader)
                && mfh.mfh_id_in_range()
                && mfh.seq_header_id_in_range()
                && let Some(seq_id) = SequenceHeaderId::try_new(mfh.mfh_seq_header_id)
                && let Ok(mfh_id_value) = u32::try_from(mfh.mfh_id())
            {
                multi_frame_headers.insert(
                    mfh_id_value,
                    MultiFrameHeaderRecord {
                        mfh_id: MfhId::from_raw(mfh_id_value),
                        mfh_seq_header_id: seq_id,
                        mfh_tlayer_id: obu.header.temporal_layer_id,
                        mfh_mlayer_id: obu.header.embedded_layer_id,
                        mfh_frame_size: mfh.mfh_frame_size,
                        mfh_seg_info_present_flag: mfh.mfh_seg_info_present_flag,
                        mfh_ext_seg_flag: mfh.mfh_ext_seg_flag,
                        mfh_allow_seg_info_change: mfh.mfh_allow_seg_info_change,
                        mfh_segment_info: mfh.segment_info,
                        mfh_deblocking_filter_update: mfh.mfh_deblocking_filter_update,
                        mfh_apply_deblocking_filter: mfh.mfh_apply_deblocking_filter,
                        offset: obu.offset,
                    },
                );
            }
        }
        _ => {}
    }
}

fn print_human(parsed: &ParsedBitstream<'_>, headers_only: bool) {
    if let ParsedBitstream::Ivf(parsed) = parsed
        && let Some(header) = parsed.header
    {
        println!(
            "IVF fourcc={} {}x{} timebase={}/{} declared_frames={}",
            fourcc_string(header.fourcc),
            header.width,
            header.height,
            header.timebase_numerator,
            header.timebase_denominator,
            header.frame_count
        );
    }

    let obus = collect_obus(parsed);
    println!("{} OBU(s)", obus.len());
    for (index, obu) in obus.iter().enumerate() {
        let header = &obu.header;
        println!(
            "OBU #{index}  @byte {}  size={}  type={}({})  ext={}  tlayer={} mlayer={} xlayer={}",
            obu.offset,
            obu.size,
            header.obu_type.spec_name(),
            header.obu_type.raw(),
            header.has_header_extension,
            header.temporal_layer_id.get(),
            header.embedded_layer_id.get(),
            header.extended_layer_id.get(),
        );
        if !headers_only {
            println!("        payload: {} byte(s)", obu.payload.len());
        }
    }
}

fn collect_obus<'data>(parsed: &ParsedBitstream<'data>) -> Vec<ObuEnvelope<'data>> {
    match parsed {
        ParsedBitstream::AnnexB(parsed) => parsed.obus.clone(),
        ParsedBitstream::Ivf(parsed) => parsed
            .frames
            .iter()
            .flat_map(|frame| frame.obus.iter().copied())
            .collect(),
    }
}

fn print_parse_errors(parsed: &ParsedBitstream<'_>) -> bool {
    let mut failed = false;
    match parsed {
        ParsedBitstream::AnnexB(parsed) => {
            if let Some(error) = &parsed.error {
                eprintln!("error: failed to parse remainder of bitstream: {error}");
                failed = true;
            }
        }
        ParsedBitstream::Ivf(parsed) => {
            for frame in &parsed.frames {
                if let Some(error) = &frame.error {
                    eprintln!(
                        "error: failed to parse IVF frame {} payload: {error}",
                        frame.frame.index
                    );
                    failed = true;
                }
            }
            if let Some(error) = &parsed.error {
                eprintln!("error: failed to parse IVF container: {error}");
                failed = true;
            }
        }
    }
    failed
}
