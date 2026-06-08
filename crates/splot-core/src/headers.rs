// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Placeholder sequence- and frame-header types.
//!
//! Only fields citable from AV2 v1.0.0 are modeled; the full syntax is not yet
//! implemented. Do not add fields that are not backed by the spec — leave a spec
//! TODO that names the implementation-matrix feature id instead (see AGENTS.md).

pub mod atlas_segment;
pub mod buffer_removal_timing;
pub mod content_interpretation;
pub mod film_grain;
pub mod frame;
pub mod layer_config_record;
pub mod metadata;
pub mod operating_point_set;
pub mod padding;
pub mod quantizer_matrix;
pub mod sequence;
pub mod tile_group;

pub use atlas_segment::{AtlasSegment, parse_atlas_segment};
pub use buffer_removal_timing::{
    BufferRemovalOpTiming, BufferRemovalTiming, parse_buffer_removal_timing,
};
pub use content_interpretation::{ContentInterpretation, parse_content_interpretation};
pub use film_grain::{
    FilmGrainModel, FilmGrainObu, FilmGrainScalingPoint, FilmGrainSlotUpdate, parse_film_grain,
};
pub use frame::{FrameHeaderPrefix, FrameHeaderPrefixStatus, parse_frame_header_prefix};
pub use layer_config_record::{LayerConfigurationRecord, parse_layer_config_record};
pub use metadata::{
    BandUnits, BandingComponent, BandingHintsDetail, MetadataBandingHints,
    MetadataDecodedFrameHash, MetadataGroupObu, MetadataGroupUnit, MetadataHdrCll, MetadataHdrMdcv,
    MetadataIccProfile, MetadataItutT35, MetadataPayload, MetadataScanType, MetadataShortObu,
    MetadataTemporalPointInfo, MetadataTimecode, MetadataType, MetadataUnit, MetadataUnknownRaw,
    MetadataUserDataUnregistered, VaryingBandUnits, parse_metadata_group, parse_metadata_short,
};
pub use operating_point_set::{
    OperatingPointPayload, OperatingPointSet, OpsAggregateInfo, OpsColorInfo, OpsDecoderModelInfo,
    OpsMlayerInfo, OpsMlayerSource, OpsSeqProfileTierLevelInfo, OpsXlayerEntry,
    parse_operating_point_set,
};
pub use padding::{PaddingObu, parse_padding_obu};
pub use quantizer_matrix::{
    FundamentalQmTransform, QuantizerMatrixLevel, QuantizerMatrixObu, UserDefinedQmPlane,
    UserDefinedQmTransform, parse_quantizer_matrix,
};
pub use sequence::{SequenceHeader, SequenceHeaderGeneral};
pub use tile_group::{TileGroupHeaderPrefix, parse_tile_group_prefix};

/// Full AV2 frame header (`frame_header()`). Only a prefix parser exists today; the
/// complete § 5.18 syntax (frame size, segmentation, filtering, quantization,
/// prediction/reference, film grain, and `frame_header_copy()`) is not yet modeled.
/// For the activation/reference fields, see [`FrameHeaderPrefix`].
// TODO(spec: AV2-5.18-FRAME-HEADER): model the full frame header syntax (AV2 v1.0.0 § 5.18).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeader {}
