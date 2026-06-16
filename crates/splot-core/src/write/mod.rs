// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 bitstream **writer** primitives — the inverse of the [`crate::bitio`]
//! reader (`ENC-BITSTREAM-WRITER`).
//!
//! This module is additive to the parser: it depends on the reader/model
//! read-only and serializes values back into AV2 descriptors. The foundational
//! [`BitWriter`] inverts every [`crate::bitio::BitReader`] primitive MSB-first, so
//! for every value it accepts the round-trip property `read(write(x)) == x` holds.
//!
//! On top of the primitives, [`obu`] writes OBU headers and Annex B framing (the
//! inverse of the § 5.2.2 parser); [`seq_header`] writes the § 5.4.1 sequence-header
//! general fields (the inverse of `parse_sequence_header_general`); [`seq_config`] writes
//! the § 5.4.3 – § 5.4.8 child-config cascade (partition, segment, intra, inter, scc,
//! transform/quant/entropy); [`segment`] writes the shared `seg_info()` body (§ 5.4.9);
//! and [`seq_tile`] writes the § 5.4.10 filter config and the § 5.4.2 tile config
//! (including the table-derived § 5.18.7.3 `tile_params()`), plus the composing
//! [`seq_tile::write_sequence_header`] that emits the whole § 5.4.1 payload in read order.
//! [`frame_header`] begins the frame-header writer with the § 5.18.2 activation prefix (the
//! inverse of `parse_frame_header_prefix`); [`frame_quant`] writes the § 5.18.6 / § 5.18.7.8 /
//! § 5.18.2 quantization cluster (`quantization_params()`, `setup_qm_params()`,
//! `read_delta_q()`, `delta_q_params()`, and the per-segment lossless/QM tail); and
//! [`frame_segmentation`] writes the § 5.18.7.1 `segmentation_params()` intra path
//! (reusing the shared § 5.4.9 `seg_info()` body writer); and [`frame_filters`] writes the
//! § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter cluster
//! (`deblocking_filter_params()`, `gdf_params()`, and `cdef_params()`); and
//! [`frame_restoration`] writes the § 5.18.7.11 / § 5.18.7.12 loop-restoration and CCSO
//! cluster (`lr_params()` on the `frame_filters_on == false` surface, and `ccso_params()`); and
//! [`frame_tail`] writes the § 5.18.2 intra tail (`read_tx_mode()`, the no-bit intra inferences,
//! `reduced_tx_set`, and `film_grain_config()`); and [`frame_header_core`] composes all of the
//! above into [`frame_header_core::write_frame_header_core`], the inverse of
//! `parse_frame_header_core` on the `IntraHeaderComplete` path (it writes the frame-type-dependent
//! control-region glue directly and delegates every sub-structure to the writers above).
//! [`metadata`] writes the § 5.17 metadata OBUs (the inverses of the § 5.17.1 – § 5.17.13 parsers):
//! [`metadata::write_metadata_short_obu`] / [`metadata::write_metadata_group_obu`] compose the two
//! OBU forms, [`metadata::write_metadata_unit`] writes a bounded `metadata_unit()` with its § 6.16.1
//! padding, and [`metadata::write_metadata_payload`] dispatches the 11 typed child payloads
//! (length-summarized blob bytes are supplied as a separate `passthrough` slice).
//! More payload writers will build on this module as the writer surface grows; see
//! `docs/spec-coverage-writer.md` (once landed) for the per-structure coverage matrix.

pub mod bit_writer;
pub mod error;
pub mod frame_config;
pub mod frame_filters;
pub mod frame_header;
pub mod frame_header_core;
pub mod frame_quant;
pub mod frame_restoration;
pub mod frame_segmentation;
pub mod frame_tail;
pub mod frame_tiling;
pub mod metadata;
pub mod obu;
pub mod segment;
pub mod seq_config;
pub mod seq_header;
pub mod seq_tile;

pub use bit_writer::BitWriter;
pub use error::{WriteError, WriteResult};
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
pub use metadata::{
    write_metadata_group_obu, write_metadata_payload, write_metadata_short_obu, write_metadata_unit,
};
pub use obu::{write_annexb_obu, write_obu_header, write_obu_header_extension};
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
