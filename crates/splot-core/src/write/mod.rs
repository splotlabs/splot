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
//! inverse of the § 5.2.2 parser), and [`seq_header`] writes the § 5.4.1 sequence-header
//! general fields (the inverse of `parse_sequence_header_general`); more payload writers
//! will build on this module as the writer surface grows; see
//! `docs/spec-coverage-writer.md` (once landed) for the per-structure coverage matrix.

pub mod bit_writer;
pub mod error;
pub mod obu;
pub mod seq_header;

pub use bit_writer::BitWriter;
pub use error::{WriteError, WriteResult};
pub use obu::{write_annexb_obu, write_obu_header, write_obu_header_extension};
pub use seq_header::{
    write_cropping_window, write_dependency_maps, write_sequence_decoder_model_info,
    write_sequence_header_general,
};
