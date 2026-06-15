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
//! More payload writers will build on this module as the writer surface grows; see
//! `docs/spec-coverage-writer.md` (once landed) for the per-structure coverage matrix.

pub mod bit_writer;
pub mod error;
pub mod obu;
pub mod segment;
pub mod seq_config;
pub mod seq_header;
pub mod seq_tile;

pub use bit_writer::BitWriter;
pub use error::{WriteError, WriteResult};
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
