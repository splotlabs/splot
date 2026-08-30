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
//! [`tile_group`] writes the § 5.19 `tile_group_obu()` structure
//! ([`tile_group::write_tile_group_structure`], the inverse of `parse_tile_group_structure`): the
//! optional `tile_start_and_end_present_flag` / `tg_start` / `tg_end` tile-range fields and the
//! closing `byte_alignment()` (the parse-context `outcome` / `header_bytes` / `payload_size`
//! artifacts belong to the composing OBU writer, not this slice); and the § 5.20.1
//! `tile_group_payload()` per-tile framing ([`tile_group::write_tile_group_payload`], the inverse of
//! `parse_tile_group_framing` on the intra path): each non-last tile's `tile_size_minus_1`
//! `le(TileSizeBytes)` size field plus the coded-tile bytes (a per-tile passthrough), with the last
//! tile's size field elided; and the composing first-tile-group `tile_group_obu()` writer
//! ([`tile_group::write_tile_group_obu`], the inverse of the `parse_tile_group_prefix`,
//! `frame_header()`, `parse_tile_group_structure`, `parse_tile_group_framing` sequence for
//! `is_first_tile_group == 1`): it sequences the `is_first_tile_group = 1` flag, the embedded
//! `frame_header()` (via `write_frame_header_core`), the § 5.19 structure, and the § 5.20.1 payload
//! framing into one OBU payload, drafting into a scratch writer and committing only on full success
//! (it owns no OBU header / size / trailing bits).
//! [`dispatch`] composes these per-structure writers into the unified complete-OBU writer (the
//! inverse of `dispatch_obu_payload` + `finish_obu_payload`):
//! [`dispatch::write_obu_payload`] emits a [`crate::obu::ParsedObu`]'s typed body plus the § 5.2.1 /
//! § 6.2.1 OBU tail (`obu_extension_flag = 0` + `trailing_bits()` for an extensible non-empty body;
//! nothing for the temporal delimiter; padding owns its own tail), and
//! [`dispatch::write_complete_obu`] prepends the § 5.2.2 header. All fourteen OBU payload types now
//! have a body writer, and the dispatch is exhaustive.
//! [`layer_config_record`] writes the § 5.8 `layer_config_record_obu()`
//! ([`layer_config_record::write_layer_config_record`], the inverse of
//! [`crate::headers::layer_config_record::parse_layer_config_record`]): it branches on the
//! `Global` / `Local` variant, inverts the § 5.8.1 – § 5.8.9 nest (with its `byte_alignment()`
//! sites and the length-bounded `lcr_global_payload()` filler), and ignores the header-derived
//! parse-context ids that have no bit representation in the body.
//! [`quantizer_matrix`] writes the § 5.13 / § 5.4.11 `quantizer_matrix_obu()`
//! ([`quantizer_matrix::write_quantizer_matrix`], the inverse of
//! [`crate::headers::quantizer_matrix::parse_quantizer_matrix`]): because the model stores only the
//! decoded coefficients (not the wire deltas or the symmetric / transpose / copy / coefficient-repeat
//! compressions), the writer canonicalizes to the long form (every skip flag `0`, one `svlc()` delta
//! per cell in 2D diagonal scan order), so the semantic round-trip holds while byte-exactness is not
//! guaranteed — the last OBU-type body writer, completing the dispatch.
//! [`film_grain`] writes the § 5.14 / § 5.18.10.2 `film_grain_obu()`
//! ([`film_grain::write_film_grain`], the inverse of
//! [`crate::headers::film_grain::parse_film_grain`]): because the model is lossy versus the wire
//! (it stores cumulative scaling-point values and de-biased AR coefficients, not the wire
//! bit-widths), the writer re-derives a minimal in-range width per array (like leb128-minimal), so
//! the semantic round-trip holds while byte-exactness is not guaranteed.
//! [`roundtrip::roundtrip_obu`] then closes the loop: it `parse → write → reparse`-checks the
//! dispatch (recovering the opaque `passthrough` for padding and the metadata blobs) so the writer
//! is verified as the parser's inverse, in-tree and under the `roundtrip_obu_bytes` fuzz target.
//! Run `cargo xtask writer-coverage` for the generated per-feature writer coverage matrix (the
//! `write` maturity of every writable AV2 syntax feature), drift-guarded by
//! `cargo xtask check-feature-status` when a render is committed locally.

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
