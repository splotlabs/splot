// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-backed inter-frame and show-existing-frame builders (§ 5.18.2).
//!
//! [`build_inter_frame_header`] writes the canonical bits of the smallest
//! `OBU_REGULAR_TILE_GROUP` inter frame and parses them with the caller's § 7.23
//! reference state, so the parser derives the reference map (§ 7.7). The core
//! writer does not write inter headers, so the returned header keeps the exact
//! bits, and [`encode_inter_frame_obu`] copies them into the tile group.

use crate::bitio::BitReader;
use crate::headers::frame::encoder_stream::{
    MAX_REDUCED_TX_SET, obu_header, write_quant_and_filters,
};
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CoreSeqView, EncoderHeaderError, FrameFilterParams, FrameHeaderCore, FrameHeaderParseStatus,
    FrameReferenceStateView, KeyFrameTxParams, TxMode, init_core_from_prefix, parse_core_body,
    parse_frame_header_prefix,
};
use crate::headers::tile_group::{
    RecordedFrameHeaderBits, TileGroupFraming, TileGroupLayout, TileGroupStructure,
};
use crate::span::ByteOffset;
use crate::types::ObuType;
use crate::write::{
    BitWriter, WriteError, write_annexb_obu, write_tile_group_payload, write_tile_group_structure,
};

/// `PRIMARY_REF_NONE` (§ 3).
const PRIMARY_REF_NONE: u32 = 7;
/// `interpolation_filter` value `EIGHTTAP` (§ 6.17.5.1).
const EIGHTTAP: u32 = 0;

/// Frame-level choices of [`build_inter_frame_header`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterFrameParams {
    /// `base_q_idx`.
    pub base_q_idx: u32,
    /// The frame's `OrderHint`; the header codes its low `OrderHintBits` bits.
    pub order_hint: u32,
    /// `refresh_frame_flags`, one bit per reference slot.
    pub refresh_frame_flags: u32,
    /// `immediate_output_frame`; a hidden frame is shown later by a
    /// show-existing frame.
    pub show_frame: bool,
    /// Loop-filter choices, as for key frames.
    pub filters: FrameFilterParams,
    /// Transform mode and `reduced_tx_set`, as for key frames.
    pub tx: KeyFrameTxParams,
}

/// A parsed frame header and the exact bits it was parsed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedFrameHeader {
    /// The header the decoder derives.
    pub core: FrameHeaderCore,
    /// `NumFrameHeaderBits` bits of `frame_header_info()`.
    pub bits: RecordedFrameHeaderBits,
}

/// Builds an `OBU_REGULAR_TILE_GROUP` inter frame header for `seq` and the § 7.23
/// reference state `refs`.
///
/// The frame has the maximum frame size, `primary_ref_frame = PRIMARY_REF_NONE`
/// with `disable_cross_frame_cdf_init = 1`, the § 7.7 implicit reference map,
/// quarter-pel motion vectors, the frame-level `EIGHTTAP` filter,
/// `reference_select = 0`, `skip_mode_present = 0`, and the key-frame tail
/// (`TX_MODE_LARGEST`, `reduced_tx_set = 2`) with the filters of `params`.
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a zero or too-wide `base_q_idx`, a
/// single-picture view, `filters` the sequence cannot carry, bits that do not
/// parse to a complete inter header with `refs`, or `refs` without a valid slot.
pub fn build_inter_frame_header(
    seq: &CoreSeqView,
    refs: &FrameReferenceStateView<'_>,
    params: &InterFrameParams,
) -> Result<EncodedFrameHeader, EncoderHeaderError> {
    let (bytes, bit_len) = inter_frame_header_bits(seq, params)?;
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularTileGroup, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTileGroup, true);
    parse_core_body(&mut reader, &mut core, seq, None, refs)?;
    core.consumed_bits = reader.consumed_bits();
    let has_reference = core
        .inter
        .as_ref()
        .is_some_and(|inter| !inter.ref_frame_idx.is_empty());
    if core.status != FrameHeaderParseStatus::InterHeaderComplete
        || core.consumed_bits != bit_len
        || !has_reference
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let bits =
        RecordedFrameHeaderBits::record(&mut BitReader::new(&bytes, ByteOffset::new(0)), bit_len)?;
    Ok(EncodedFrameHeader { core, bits })
}

fn inter_frame_header_bits(
    seq: &CoreSeqView,
    params: &InterFrameParams,
) -> Result<(Vec<u8>, u64), EncoderHeaderError> {
    if seq.single_picture_header_flag {
        return Err(EncoderHeaderError::SinglePictureView);
    }
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_flag(true)?; // frame_is_inter
    w.write_flag(params.show_frame)?; // immediate_output_frame
    if !params.show_frame && !seq.monotonic_output_order_flag {
        w.write_flag(false)?; // implicit_output_frame
    }
    w.write_flag(false)?; // frame_size_override_flag
    let order_hint_mask = (1u32 << seq.order_hint_bits) - 1;
    w.write_bits(params.order_hint & order_hint_mask, seq.order_hint_bits)?;
    w.write_flag(true)?; // signal_primary_ref_frame
    w.write_flag(true)?; // disable_cross_frame_cdf_init
    w.write_bits(PRIMARY_REF_NONE, 3)?; // primary_ref_frame
    if seq.enable_short_refresh_frame_flags || seq.seq_force_screen_content_tools == 1 {
        return Err(unsupported(
            "short refresh flags or forced screen content tools",
        ));
    }
    w.write_bits(params.refresh_frame_flags, seq.num_ref_frames)?;
    if seq.seq_force_screen_content_tools == 2 {
        w.write_flag(false)?; // allow_screen_content_tools
    }
    w.write_flag(false)?; // allow_intrabc
    w.write_flag(true)?; // use_qtr_precision_mv
    w.write_flag(false)?; // is_filter_switchable
    w.write_bits(EIGHTTAP, 2)?; // interpolation_filter
    w.write_flag(false)?; // disable_cdf_update
    let tx = params.tx;
    if tx.tx_mode == TxMode::Only4x4 || tx.reduced_tx_set > MAX_REDUCED_TX_SET {
        return Err(EncoderHeaderError::InvalidTxParams {
            tx_mode: tx.tx_mode,
            reduced_tx_set: tx.reduced_tx_set,
        });
    }
    write_quant_and_filters(&mut w, seq, params.base_q_idx, &params.filters)?;
    w.write_flag(tx.tx_mode == TxMode::Select)?; // tx_mode_select
    w.write_flag(false)?; // reference_select
    w.write_flag(false)?; // skip_mode_present
    w.write_bits(u32::from(tx.reduced_tx_set), 2)?;
    let bit_len = w.bit_len();
    w.align_to_byte();
    Ok((w.into_bytes(), bit_len))
}

/// Serializes the `OBU_REGULAR_TILE_GROUP` OBU of `header` whose single tile
/// carries `tile_data`: `is_first_tile_group`, the recorded header bits, the
/// § 5.19 structure, and the tile.
///
/// # Errors
/// Returns [`EncoderHeaderError`] if the header has no tile info, `tile_data` is
/// empty, or the OBU does not serialize.
pub fn encode_inter_frame_obu(
    header: &EncodedFrameHeader,
    tile_data: &[u8],
) -> Result<Vec<u8>, EncoderHeaderError> {
    let tile_info = header
        .core
        .tile_info
        .as_ref()
        .ok_or(EncoderHeaderError::Incomplete)?;
    let layout = TileGroupLayout::new(
        tile_info.tile_cols,
        tile_info.tile_rows,
        tile_info.tile_cols_log2,
        tile_info.tile_rows_log2,
    );
    let mut payload = BitWriter::new();
    payload.write_flag(true)?; // is_first_tile_group
    for index in 0..header.bits.num_frame_header_bits() {
        let bit = header
            .bits
            .bit(index)
            .ok_or(EncoderHeaderError::Incomplete)?;
        payload.write_flag(bit)?;
    }
    write_tile_group_structure(
        &mut payload,
        &TileGroupStructure::single_tile_first_group(),
        layout,
    )?;
    write_tile_group_payload(
        &mut payload,
        &TileGroupFraming::single_tile(tile_data.len() as u64),
        &[tile_data],
        tile_info.tile_size_bytes.unwrap_or(1),
        false,
    )?;
    let mut w = BitWriter::new();
    write_annexb_obu(
        &mut w,
        &obu_header(ObuType::RegularTileGroup),
        &payload.into_bytes(),
    )?;
    Ok(w.into_bytes())
}

/// Serializes an `OBU_REGULAR_SEF` OBU that shows reference slot
/// `frame_to_show_map_idx` with `derive_sef_order_hint = 1` (§ 5.18.2).
///
/// # Errors
/// Returns [`EncoderHeaderError`] for a slot outside the sequence's reference
/// slots, a sequence without a known `film_grain_params_present`, or bits that
/// do not parse to a complete show-existing frame.
pub fn encode_show_existing_frame_obu(
    seq: &CoreSeqView,
    frame_to_show_map_idx: u32,
) -> Result<Vec<u8>, EncoderHeaderError> {
    if frame_to_show_map_idx >= seq.num_ref_frames {
        return Err(unsupported("frame_to_show_map_idx"));
    }
    let film_grain = seq
        .film_grain_params_present
        .ok_or(EncoderHeaderError::Incomplete)?;
    let mut w = BitWriter::new();
    w.write_uvlc(0)?; // cur_mfh_id
    w.write_uvlc(0)?; // seq_header_id_in_frame_header
    w.write_bits(frame_to_show_map_idx, ceil_log2(seq.num_ref_frames))?;
    w.write_flag(true)?; // derive_sef_order_hint
    if film_grain {
        w.write_flag(false)?; // apply_grain
    }
    w.write_trailing_bits(8 - w.bit_len() % 8)?;
    let payload = w.into_bytes();
    let mut reader = BitReader::new(&payload, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::RegularSef, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::RegularSef, true);
    parse_core_body(
        &mut reader,
        &mut core,
        seq,
        None,
        &FrameReferenceStateView::unknown(),
    )?;
    if core.status != FrameHeaderParseStatus::ShowExistingFrameComplete
        || core.sef_trailing_bits != Some(crate::headers::frame::SefTrailingBits::Valid)
    {
        return Err(EncoderHeaderError::Incomplete);
    }
    let mut obu = BitWriter::new();
    write_annexb_obu(&mut obu, &obu_header(ObuType::RegularSef), &payload)?;
    Ok(obu.into_bytes())
}

fn unsupported(what: &'static str) -> EncoderHeaderError {
    WriteError::NonCanonicalFrameHeader { what }.into()
}

#[cfg(test)]
#[path = "encoder_inter_stream_tests.rs"]
mod tests;
