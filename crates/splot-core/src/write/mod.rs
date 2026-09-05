// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 bitstream **writer** primitives — the inverse of the [`crate::bitio`]
//! reader (`ENC-BITSTREAM-WRITER`).
//!
//! [`BitWriter`] emits AV2 descriptors; the domain modules compose them into
//! headers and payloads. [`write_complete_obu`] adds framing, and [`roundtrip_obu`]
//! verifies semantic parser/writer round trips.
//!
//! Film-grain widths and quantizer-matrix compression are canonicalized; their
//! emitted bytes need not match the original representation.

pub mod atlas_segment;
pub mod bit_writer;
pub mod buffer_removal_timing;
pub mod content_interpretation;
pub mod dispatch;
pub mod error;
pub mod film_grain;
pub mod frame_config;
pub mod frame_filters;
pub mod frame_header;
pub mod frame_header_core;
pub mod frame_quant;
pub mod frame_restoration;
pub mod frame_segmentation;
pub mod frame_tail;
pub mod frame_tiling;
pub mod layer_config_record;
pub mod metadata;
pub mod msdo;
pub mod multi_frame_header;
pub mod obu;
pub mod operating_point_set;
pub mod quantizer_matrix;
pub mod roundtrip;
pub mod segment;
pub mod seq_config;
pub mod seq_header;
pub mod seq_tile;
pub mod tile_group;

pub use atlas_segment::write_atlas_segment;
pub use bit_writer::BitWriter;
pub use buffer_removal_timing::write_buffer_removal_timing;
pub use content_interpretation::write_content_interpretation;
pub use dispatch::{write_complete_obu, write_obu_payload};
pub use error::{WriteError, WriteResult};
pub use film_grain::write_film_grain;
pub use frame_config::{write_frame_size, write_intrabc_params, write_screen_content_params};
pub use frame_filters::{write_cdef_params, write_deblocking_filter_params, write_gdf_params};
pub use frame_header::write_frame_header_prefix;
pub use frame_header_core::write_frame_header_core;
pub use frame_quant::{
    write_delta_q_params, write_lossless_info, write_quantization_params, write_read_delta_q,
    write_setup_qm_params,
};
pub use frame_restoration::{write_ccso_params, write_lr_params};
pub use frame_segmentation::write_segmentation_params;
pub use frame_tail::{write_film_grain_config, write_intra_tail, write_tx_mode};
pub use frame_tiling::write_tile_info;
pub use layer_config_record::write_layer_config_record;
pub use metadata::{
    write_metadata_group_obu, write_metadata_group_obu_flat, write_metadata_payload,
    write_metadata_short_obu, write_metadata_unit,
};
pub use msdo::write_msdo;
pub use multi_frame_header::write_multi_frame_header;
pub use obu::{write_annexb_obu, write_obu_header, write_obu_header_extension};
pub use operating_point_set::write_operating_point_set;
pub use quantizer_matrix::write_quantizer_matrix;
pub use roundtrip::{RoundtripOutcome, recover_roundtrip_passthrough, roundtrip_obu};
pub use segment::write_seg_info;
pub use seq_config::{
    write_sequence_inter_config, write_sequence_intra_config, write_sequence_partition_config,
    write_sequence_scc_config, write_sequence_segment_config,
    write_sequence_transform_quant_entropy_config,
};
pub use seq_header::{
    write_cropping_window, write_dependency_maps, write_sequence_decoder_model_info,
    write_sequence_header_general,
};
pub use seq_tile::{
    write_sequence_filter_config, write_sequence_header, write_sequence_tile_config,
};
pub use tile_group::{
    write_tile_group_continuation_obu, write_tile_group_obu, write_tile_group_payload,
    write_tile_group_structure,
};
