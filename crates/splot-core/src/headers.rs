// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed AV2 sequence-, frame-, metadata-, and auxiliary-header models and parsers.
//!
//! Models and parser coverage follow AV2 v1.0.0. Do not add fields that are not
//! backed by the spec; leave a spec TODO that names the implementation-matrix
//! feature id instead (see AGENTS.md).

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

pub use atlas_segment::{AtlasSegment, parse_atlas_segment, parse_atlas_segment_observed};
pub use buffer_removal_timing::{
    BufferRemovalOpTiming, BufferRemovalTiming, parse_buffer_removal_timing,
    parse_buffer_removal_timing_observed,
};
pub use content_interpretation::{
    ContentInterpretation, parse_content_interpretation, parse_content_interpretation_observed,
};
pub use film_grain::{
    FilmGrainModel, FilmGrainObu, FilmGrainScalingPoint, FilmGrainSlotUpdate, parse_film_grain,
    parse_film_grain_observed,
};
pub use frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseStatus, FrameHeaderPrefix,
    FrameReferenceStateView, FrameSize, FrameType, parse_frame_header_core,
    parse_frame_header_core_observed, parse_frame_header_prefix,
    parse_frame_header_prefix_observed,
};
pub use layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record, parse_layer_config_record_observed,
};
pub use metadata::{
    BandUnits, BandingComponent, BandingHintsDetail, MetadataBandingHints,
    MetadataDecodedFrameHash, MetadataGroupObu, MetadataGroupUnit, MetadataHdrCll, MetadataHdrMdcv,
    MetadataIccProfile, MetadataItutT35, MetadataPayload, MetadataScanType, MetadataShortObu,
    MetadataTemporalPointInfo, MetadataTimecode, MetadataType, MetadataUnit, MetadataUnknownRaw,
    MetadataUserDataUnregistered, VaryingBandUnits, parse_metadata_group,
    parse_metadata_group_observed, parse_metadata_short, parse_metadata_short_observed,
};
pub use operating_point_set::{
    OperatingPointPayload, OperatingPointSet, OpsAggregateInfo, OpsColorInfo, OpsDecoderModelInfo,
    OpsMlayerInfo, OpsMlayerSource, OpsSeqProfileTierLevelInfo, OpsXlayerEntry,
    parse_operating_point_set, parse_operating_point_set_observed,
};
pub use padding::{PaddingObu, parse_padding_obu, parse_padding_obu_observed};
pub use quantizer_matrix::{
    FundamentalQmTransform, QuantizerMatrixLevel, QuantizerMatrixObu, UserDefinedQmPlane,
    UserDefinedQmTransform, parse_quantizer_matrix, parse_quantizer_matrix_observed,
};
pub use sequence::{
    SequenceHeader, SequenceHeaderGeneral, parse_sequence_decoder_model_info_observed,
    parse_sequence_header_general_observed, parse_sequence_header_observed,
    parse_timing_info_observed,
};
pub use tile_group::{
    TileGroupHeaderPrefix, parse_tile_group_prefix, parse_tile_group_prefix_observed,
    parse_tile_group_structure_observed, parse_tile_group_structure_with_bru_observed,
};
