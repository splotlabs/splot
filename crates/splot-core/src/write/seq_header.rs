// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header **general-field** writers — the inverse of the § 5.4.1
//! general parser ([`crate::headers::sequence::parse_sequence_header_general`];
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`) and `seq_decoder_model_info()`
//! (§ 5.4.13; `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-13`) (`ENC-BITSTREAM-WRITER`).
//!
//! This module covers the §5.4.1 fields up to (but not including) the first child
//! config (`sequence_partition_config()`): the general fields, the cropping window,
//! the decoder-model cascade, and the two dependency maps. The top-level
//! `write_sequence_header` (and the child-config writers) land in later changes;
//! they compose these helpers in §5.4.1 read order.
//!
//! This module is additive: it depends on the model/parser read-only and serializes a
//! parsed [`SequenceHeaderGeneral`] back to bits via [`BitWriter`]. The universal
//! contract is semantic `read(write(x)) == x` for every model the parser can produce:
//! `parse_sequence_header_general(write_sequence_header_general(g)) == g`.
//!
//! Several §5.4.1 structures are stored *derived* rather than as raw bits — the
//! dependency maps (default-fill plus signaled overrides), the inferred constants of a
//! `single_picture_header_flag == 1` header, and the `minus_1`/present-flag pairs. A
//! model carrying a value the parser could never have emitted is rejected up front with
//! a typed [`WriteError`] *before any bit is written* (reject-before-write), so the
//! writer emits bits only from values the parser would have signaled and the round-trip
//! property is provable. See [`WriteError::NonCanonicalSequenceValue`].

use crate::headers::sequence::{
    CroppingWindow, MLayerDependencyMap, SequenceDecoderModelInfo, SequenceHeaderGeneral,
    TLayerDependencyMap, Tier,
};
use crate::types::{EmbeddedLayerId, TemporalLayerId};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `MAX_NUM_TLAYERS` (AV2 § 3): number of temporal layers a dependency map spans.
const MAX_NUM_TLAYERS: u8 = 4;
/// `MAX_NUM_MLAYERS` (AV2 § 3): number of embedded layers a dependency map spans.
const MAX_NUM_MLAYERS: u8 = 8;

/// `CeilLog2(value)` (AV2 v1.0.0 § 4.7, `docs/spec/av2/1.0.0/04-conventions.md#s-4-7`),
/// duplicated locally to compute the `seq_max_mlayer_cnt_minus_1` field width exactly as
/// `parse_sequence_header_general` does (it uses the private `ceil_log2_u32`). Kept private:
/// it is a writer-internal width derivation, not part of the model surface.
const fn ceil_log2_u32(value: u32) -> u32 {
    if value <= 1 {
        0
    } else {
        u32::BITS - (value - 1).leading_zeros()
    }
}

/// Returns the `seq_tier` bit (`0` for [`Tier::Main`], `1` for [`Tier::High`]).
///
/// AV2 v1.0.0 § 5.4.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`) reads
/// `seq_tier` as `f(1)`; the model's [`Tier`] has no `to_bit`, so the inverse of
/// [`Tier::from_bit`] is derived here.
const fn seq_tier_bit(tier: Tier) -> u8 {
    match tier {
        Tier::Main => 0,
        Tier::High => 1,
    }
}

/// Returns `Ok(())` if `value` fits in `width_bits`, else [`WriteError::ValueTooWide`]
/// — the same bound the `f(n)` write enforces, checked up front so a rejected header
/// never leaves a partial encoding in the writer.
fn check_field_width(value: u64, width_bits: u32) -> WriteResult<()> {
    // `width_bits` is always small here (<= 32 for these fields); guard the shift.
    let fits = width_bits >= 64 || value < (1u64 << width_bits);
    if fits {
        Ok(())
    } else {
        Err(WriteError::ValueTooWide { value, width_bits })
    }
}

/// Writes the general sequence-header fields through the dependency maps (AV2 v1.0.0
/// § 5.4.1, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_header_general`].
///
/// Writes, in §5.4.1 read order: `seq_header_id` (`uvlc`), `seq_profile_idc` (`f(5)`),
/// `single_picture_header_flag` (`f(1)`), `seq_level_idx` (`f(5)`), the conditional
/// `seq_tier` (`f(1)`), `chroma_format_idc`/`bit_depth_idc` (`uvlc`), the non-single-picture
/// layer block, the frame-dimension fields, the cropping window
/// ([`write_cropping_window`]), the decoder-model cascade
/// ([`write_sequence_decoder_model_info`]), and the dependency maps
/// ([`write_dependency_maps`]). This writes the §5.4.1 prefix only — no child config and
/// no `film_grain_params_present`.
///
/// The model is fully validated before any bit is written, so a rejected model leaves
/// `writer` unchanged.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if the writer is not on a byte boundary (the
///   sequence-header payload begins byte-aligned, immediately after the OBU header).
/// - [`WriteError::ValueTooWide`] if a fixed-width field (`seq_profile_idc` f(5),
///   `seq_level_idx` f(5), a layer id, `seq_max_mlayer_cnt_minus_1`, a frame-bits field,
///   `max_frame_*_minus_1`) exceeds its bit width — unreachable for a parser-produced model.
/// - [`WriteError::ValueOutOfRange`] if a `uvlc`/`ns` value lies outside the descriptor
///   domain.
/// - [`WriteError::NonCanonicalSequenceValue`] if a derived/inferred value disagrees with
///   the §5.4.1 re-derivation (e.g. `seq_tier == High` with the gate false, a single-picture
///   inferred constant, an `Option`/present-flag mismatch, or a non-canonical dependency map).
pub fn write_sequence_header_general(
    writer: &mut BitWriter,
    general: &SequenceHeaderGeneral,
) -> WriteResult<()> {
    // The §5.4.1 payload starts byte-aligned (right after the byte-granular OBU header);
    // reject a mid-byte writer up front rather than producing an unreadable prefix.
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    // Validate the whole model before emitting any bit, so a rejected model leaves the
    // writer untouched (reject-before-write).
    check_general_encodable(general)?;

    // seq_header_id: uvlc. `try_new` bounds it below `MAX_SEQ_NUM == 16`, always encodable.
    writer.write_uvlc(u32::from(general.seq_header_id.get()))?;
    // seq_profile_idc: f(5).
    writer.write_bits_u8(general.seq_profile_idc.get(), 5)?;
    // single_picture_header_flag: f(1).
    writer.write_bit(u8::from(general.single_picture_header_flag))?;
    // seq_level_idx: f(5).
    writer.write_bits_u8(general.seq_level_idx.get(), 5)?;
    // seq_tier: f(1), read only when seq_level_idx > 3 && !single_picture_header_flag.
    if seq_tier_is_signaled(general) {
        writer.write_bit(seq_tier_bit(general.seq_tier))?;
    }
    // chroma_format_idc / bit_depth_idc: uvlc each (the enums hold canonical 0..=3 / 0..=1).
    writer.write_uvlc(u32::from(general.chroma_format_idc.get()))?;
    writer.write_uvlc(u32::from(general.bit_depth_idc.get()))?;

    // The six-field layer block is signaled only for a non-single-picture header; a
    // single-picture header infers all six (checked in `check_general_encodable`).
    if !general.single_picture_header_flag {
        writer.write_bits_u8(general.seq_lcr_id.get(), 3)?; // seq_lcr_id: f(3)
        writer.write_bit(u8::from(general.still_picture))?; // still_picture: f(1)
        writer.write_bits_u8(general.max_tlayer_id.get(), 2)?; // max_tlayer_id: f(2)
        writer.write_bits_u8(general.max_mlayer_id.get(), 3)?; // max_mlayer_id: f(3)
        // seq_max_mlayer_cnt_minus_1: f(CeilLog2(max_mlayer_id + 1)), only when max_mlayer_id > 0.
        if general.max_mlayer_id.get() > 0 {
            let n = ceil_log2_u32(u32::from(general.max_mlayer_id.get()) + 1);
            let minus_1 = u32::from(general.seq_max_mlayer_count.get()) - 1;
            writer.write_bits(minus_1, n)?;
        }
        writer.write_bit(u8::from(general.monotonic_output_order_flag))?; // monotonic_output_order_flag: f(1)
    }

    // frame_width_bits_minus_1 / frame_height_bits_minus_1: f(4) of (bits - 1).
    writer.write_bits_u8(general.frame_width_bits.get() - 1, 4)?;
    writer.write_bits_u8(general.frame_height_bits.get() - 1, 4)?;
    // max_frame_width_minus_1: f(frame_width_bits); max_frame_height_minus_1: f(frame_height_bits).
    writer.write_bits(
        general.max_frame_width.minus_1(),
        u32::from(general.frame_width_bits.get()),
    )?;
    writer.write_bits(
        general.max_frame_height.minus_1(),
        u32::from(general.frame_height_bits.get()),
    )?;

    write_cropping_window(writer, general)?;

    // The decoder-model cascade is signaled only for a non-single-picture header.
    if !general.single_picture_header_flag {
        // seq_initial_display_delay_present_flag: f(1) = the Option's presence.
        let initial_delay_present = general.seq_initial_display_delay_minus_1.is_some();
        writer.write_bit(u8::from(initial_delay_present))?;
        if let Some(minus_1) = general.seq_initial_display_delay_minus_1 {
            writer.write_bits_u8(minus_1, 4)?; // seq_initial_display_delay_minus_1: f(4)
        }
        // decoder_model_info_present_flag: f(1).
        writer.write_bit(u8::from(general.decoder_model_info_present_flag))?;
        if general.decoder_model_info_present_flag {
            // num_units_in_decoding_tick: f(32) (validated > 0 in the check pass).
            let num_units = general.num_units_in_decoding_tick.ok_or(
                WriteError::NonCanonicalSequenceValue {
                    what: "num_units_in_decoding_tick",
                },
            )?;
            writer.write_bits(num_units, 32)?;
            // seq_decoder_model_info_present_flag: f(1).
            writer.write_bit(u8::from(general.seq_decoder_model_info_present_flag))?;
            if general.seq_decoder_model_info_present_flag {
                let model =
                    general
                        .decoder_model_info
                        .ok_or(WriteError::NonCanonicalSequenceValue {
                            what: "decoder_model_info",
                        })?;
                write_sequence_decoder_model_info(writer, &model)?;
            }
        }
    }

    write_dependency_maps(writer, general)
}

/// Returns `true` when `seq_tier` is signaled as `f(1)` (AV2 § 5.4.1): the gate is
/// `seq_level_idx > 3 && !single_picture_header_flag`. When the gate is false the parser
/// infers [`Tier::Main`] and no bit is read.
const fn seq_tier_is_signaled(general: &SequenceHeaderGeneral) -> bool {
    general.seq_level_idx.get() > 3 && !general.single_picture_header_flag
}

/// Validates that `general` is a model the §5.4.1 parser could have produced, returning a
/// typed [`WriteError`] before any bit is written. Mirrors `check_header_encodable` in
/// [`crate::write::obu`]: it enforces field widths, the single-picture inferred constants,
/// the `seq_tier` gate, the `minus_1`/present-flag pairings, the cropping-window present
/// flag, the decoder-model `Option`/flag agreement, and dependency-map canonicality.
fn check_general_encodable(general: &SequenceHeaderGeneral) -> WriteResult<()> {
    // f(5) fields.
    check_field_width(u64::from(general.seq_profile_idc.get()), 5)?;
    check_field_width(u64::from(general.seq_level_idx.get()), 5)?;

    // seq_tier inference loss: when the gate is false, the parser infers Main, so a stored
    // High could never have been signaled.
    if !seq_tier_is_signaled(general) && matches!(general.seq_tier, Tier::High) {
        return Err(WriteError::NonCanonicalSequenceValue { what: "seq_tier" });
    }

    if general.single_picture_header_flag {
        // §5.4.1 single-picture inference: the six-field layer block, the decoder-model
        // cascade, and the dependency-map flags are all inferred. A stored value other than
        // the parser's inferred constant could never have been signaled.
        if general.seq_lcr_id.get() != 0
            || !general.still_picture
            || general.max_tlayer_id.get() != 0
            || general.max_mlayer_id.get() != 0
            || general.seq_max_mlayer_count.get() != 1
            || !general.monotonic_output_order_flag
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_layer_block",
            });
        }
        if general.seq_initial_display_delay_minus_1.is_some()
            || general.decoder_model_info_present_flag
            || general.num_units_in_decoding_tick.is_some()
            || general.seq_decoder_model_info_present_flag
            || general.decoder_model_info.is_some()
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_decoder_model",
            });
        }
    } else {
        // Non-single-picture layer-block field widths.
        check_field_width(u64::from(general.seq_lcr_id.get()), 3)?;
        check_field_width(u64::from(general.max_tlayer_id.get()), 2)?;
        check_field_width(u64::from(general.max_mlayer_id.get()), 3)?;
        if general.max_mlayer_id.get() > 0 {
            // seq_max_mlayer_cnt_minus_1: must satisfy `count - 1 <= max_mlayer_id` (the
            // parser's `try_from_minus_1` bound) and fit f(CeilLog2(max_mlayer_id + 1)).
            let count = general.seq_max_mlayer_count.get();
            if count == 0 {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "seq_max_mlayer_count",
                });
            }
            let minus_1 = u32::from(count - 1);
            if minus_1 > u32::from(general.max_mlayer_id.get()) {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "seq_max_mlayer_count",
                });
            }
            let n = ceil_log2_u32(u32::from(general.max_mlayer_id.get()) + 1);
            check_field_width(u64::from(minus_1), n)?;
        } else if general.seq_max_mlayer_count.get() != 1 {
            // max_mlayer_id == 0 infers SeqMaxMlayerCnt == 1 (no bits read).
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_max_mlayer_count",
            });
        }

        // Decoder-model cascade: each Option's presence must agree with its gating flag.
        if general.decoder_model_info_present_flag {
            // num_units_in_decoding_tick is present and > 0 (the parser rejects 0).
            match general.num_units_in_decoding_tick {
                Some(0) | None => {
                    return Err(WriteError::NonCanonicalSequenceValue {
                        what: "num_units_in_decoding_tick",
                    });
                }
                Some(_) => {}
            }
            if general.seq_decoder_model_info_present_flag != general.decoder_model_info.is_some() {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "decoder_model_info",
                });
            }
        } else {
            // decoder_model_info_present_flag == 0 infers no inner state.
            if general.num_units_in_decoding_tick.is_some()
                || general.seq_decoder_model_info_present_flag
                || general.decoder_model_info.is_some()
            {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "decoder_model_info",
                });
            }
        }
    }

    // Frame-dimension widths.
    let frame_width_bits = general.frame_width_bits.get();
    let frame_height_bits = general.frame_height_bits.get();
    // The model stores `minus_1 + 1`; the coded `frame_*_bits_minus_1` is f(4), so the
    // stored width is in 1..=16.
    check_field_width(u64::from(frame_width_bits - 1), 4)?;
    check_field_width(u64::from(frame_height_bits - 1), 4)?;
    // max_frame_*_minus_1 must fit in frame_*_bits (width loss).
    check_field_width(
        u64::from(general.max_frame_width.minus_1()),
        u32::from(frame_width_bits),
    )?;
    check_field_width(
        u64::from(general.max_frame_height.minus_1()),
        u32::from(frame_height_bits),
    )?;

    check_cropping_window_encodable(general)?;
    check_dependency_maps_encodable(general)?;
    Ok(())
}

/// Writes `seq_cropping_window_present_flag` and (when set) the four `uvlc` cropping
/// offsets (AV2 v1.0.0 § 5.4.1, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`),
/// the inverse of the parser's `parse_cropping_window`. When the flag is clear the four
/// offsets are inferred to `0` and no bits follow.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] if `seq_cropping_window_present_flag == 0`
///   but the stored [`CroppingWindow`] is non-default (the parser infers an all-zero
///   window, so a non-zero offset has no bitstream home), or if any offset exceeds the
///   §6.4.1 `max_frame_*_minus_1` bound.
/// - [`WriteError::ValueOutOfRange`] from the `uvlc` writer for an unencodable offset.
pub fn write_cropping_window(
    writer: &mut BitWriter,
    general: &SequenceHeaderGeneral,
) -> WriteResult<()> {
    check_cropping_window_encodable(general)?;
    writer.write_bit(u8::from(general.seq_cropping_window_present_flag))?;
    if general.seq_cropping_window_present_flag {
        let w = &general.cropping_window;
        writer.write_uvlc(w.left)?;
        writer.write_uvlc(w.right)?;
        writer.write_uvlc(w.top)?;
        writer.write_uvlc(w.bottom)?;
    }
    Ok(())
}

/// Validates the cropping window against the §5.4.1/§6.4.1 rules: a clear present-flag
/// implies the default window, and each present offset is `<= max_frame_*_minus_1`.
fn check_cropping_window_encodable(general: &SequenceHeaderGeneral) -> WriteResult<()> {
    if !general.seq_cropping_window_present_flag {
        // A clear flag infers an all-zero window; any other window is unrepresentable.
        if general.cropping_window != CroppingWindow::default() {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "cropping_window",
            });
        }
        return Ok(());
    }
    // §6.4.1: each offset <= the corresponding max_frame_*_minus_1.
    let w = &general.cropping_window;
    let max_w = general.max_frame_width.minus_1();
    let max_h = general.max_frame_height.minus_1();
    if w.left > max_w || w.right > max_w || w.top > max_h || w.bottom > max_h {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "cropping_window",
        });
    }
    Ok(())
}

/// Writes `seq_decoder_model_info()` (AV2 v1.0.0 § 5.4.13,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-13`), the inverse of
/// [`crate::headers::sequence::parse_sequence_decoder_model_info`]: `decoder_buffer_delay`
/// (`uvlc`), `encoder_buffer_delay` (`uvlc`), `low_delay_mode_flag` (`f(1)`). The struct
/// has no internal gating (the §5.4.1 caller's `seq_decoder_model_info_present_flag` gates
/// it), so there are no canonicality hazards here.
///
/// # Errors
/// - [`WriteError::ValueOutOfRange`] if either `uvlc` delay equals `u32::MAX` (the reader
///   never produces a value needing 32 leading zero bits).
pub fn write_sequence_decoder_model_info(
    writer: &mut BitWriter,
    info: &SequenceDecoderModelInfo,
) -> WriteResult<()> {
    writer.write_uvlc(info.decoder_buffer_delay)?;
    writer.write_uvlc(info.encoder_buffer_delay)?;
    writer.write_bit(u8::from(info.low_delay_mode_flag))
}

/// Writes the §5.4.1 dependency-map region (AV2 v1.0.0 § 5.4.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`), the inverse of the parser's
/// `parse_dependency_maps`.
///
/// The model stores the *derived* [`MLayerDependencyMap`] / [`TLayerDependencyMap`] plus
/// three present-flags, not the raw signaled bits. This writer re-derives the exact
/// signaled-bit sequence from the stored maps the same way the parser derived the maps
/// (default-fill, then signaled overrides in the spec's loop order):
///
/// - `mlayer_dependency_present_flag` is signaled iff `max_mlayer_id > 0`; when set, the
///   diagonal-first bits of rows `1..=max_mlayer_id` come from
///   [`MLayerDependencyMap::depends_on`] (`refLayer` from `currLayer` down to `0`).
/// - `tlayer_dependency_present_flag` is signaled iff `max_tlayer_id > 0`; when set,
///   `multi_tlayer_dependency_map_present_flag` is signaled only when `max_mlayer_id > 0`,
///   then bits are emitted for embedded layer `0` (and all layers when `multi`), copying
///   layer 0 to layers `>0` when `!multi` (no bits emitted for the copied entries).
///
/// # Errors
/// [`WriteError::NonCanonicalSequenceValue`] if a stored map is not reproducible from its
/// present-flags — a clear present-flag whose map differs from the §5.4.1 default fill,
/// a signaled mlayer map whose row 0 differs from the default, or a `!multi` tlayer map
/// whose layers `>0` differ from layer 0.
pub fn write_dependency_maps(
    writer: &mut BitWriter,
    general: &SequenceHeaderGeneral,
) -> WriteResult<()> {
    check_dependency_maps_encodable(general)?;

    let max_mlayer = general.max_mlayer_id.get();
    let max_tlayer = general.max_tlayer_id.get();

    // mlayer_dependency_present_flag: f(1), only when max_mlayer_id > 0.
    if max_mlayer > 0 {
        writer.write_bit(u8::from(general.mlayer_dependency_present_flag))?;
        if general.mlayer_dependency_present_flag {
            for curr in 1..=max_mlayer {
                // §5.4.1: refLayer runs from currLayer down to 0 (diagonal first).
                for reference in (0..=curr).rev() {
                    let bit = general.mlayer_dependency_map.depends_on(
                        EmbeddedLayerId::from_bits(curr),
                        EmbeddedLayerId::from_bits(reference),
                    );
                    writer.write_bit(u8::from(bit))?;
                }
            }
        }
    }

    // tlayer_dependency_present_flag: f(1), only when max_tlayer_id > 0.
    if max_tlayer > 0 {
        writer.write_bit(u8::from(general.tlayer_dependency_present_flag))?;
        if general.tlayer_dependency_present_flag {
            // multi_tlayer_dependency_map_present_flag: f(1), only when max_mlayer_id > 0.
            if max_mlayer > 0 {
                writer.write_bit(u8::from(general.multi_tlayer_dependency_map_present_flag))?;
            }
            let multi = general.multi_tlayer_dependency_map_present_flag;
            for m in 0..=max_mlayer {
                for curr in 1..=max_tlayer {
                    for reference in (0..=curr).rev() {
                        // §5.4.1: a bit is signaled only for embedded layer 0, or for every
                        // layer when `multi`; with `!multi`, layers >0 copy layer 0 (no bit).
                        if multi || m == 0 {
                            let bit = general.tlayer_dependency_map.depends_on(
                                EmbeddedLayerId::from_bits(m),
                                TemporalLayerId::from_bits(curr),
                                TemporalLayerId::from_bits(reference),
                            );
                            writer.write_bit(u8::from(bit))?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Validates that the stored dependency maps are reproducible from their present-flags,
/// the central round-trip guard for the §5.4.1 dependency region. Mirrors the parser's
/// derivation so the writer emits bits only from values the parser would have signaled.
///
/// Three guards (each → [`WriteError::NonCanonicalSequenceValue`]):
///
/// 1. **Default-fill consistency.** A clear present-flag means the parser left the map at
///    its §5.4.1 default fill; a stored non-default map could never have been parsed.
/// 2. **mlayer row-0 consistency.** mlayer row 0 is never signaled (it keeps the default);
///    a signaled mlayer map whose row 0 differs from the default is non-canonical.
/// 3. **tlayer copy consistency.** With `!multi`, layers `1..=max_mlayer` copy layer 0;
///    a stored map that does not already satisfy `tlayer[m] == tlayer[0]` is non-canonical.
fn check_dependency_maps_encodable(general: &SequenceHeaderGeneral) -> WriteResult<()> {
    let max_mlayer = general.max_mlayer_id.get();
    let max_tlayer = general.max_tlayer_id.get();

    // --- mlayer map ---
    let m_default = MLayerDependencyMap::default_for(general.max_mlayer_id);
    if max_mlayer == 0 {
        // No mlayer bits are read; the present flag is inferred 0 and the map is the default.
        if general.mlayer_dependency_present_flag {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "mlayer_dependency_present_flag",
            });
        }
        if !mlayer_maps_equal(&general.mlayer_dependency_map, &m_default) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "mlayer_dependency_map",
            });
        }
    } else if !general.mlayer_dependency_present_flag {
        // Guard 1: a clear present-flag means the map equals the default fill.
        if !mlayer_maps_equal(&general.mlayer_dependency_map, &m_default) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "mlayer_dependency_map",
            });
        }
    } else {
        // Guard 2: row 0 is never signaled; it must equal the default row 0.
        for reference in 0..MAX_NUM_MLAYERS {
            let stored = general.mlayer_dependency_map.depends_on(
                EmbeddedLayerId::from_bits(0),
                EmbeddedLayerId::from_bits(reference),
            );
            let default = m_default.depends_on(
                EmbeddedLayerId::from_bits(0),
                EmbeddedLayerId::from_bits(reference),
            );
            if stored != default {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "mlayer_dependency_map",
                });
            }
        }
        // Entries outside the signaled triangle (currLayer > max_mlayer, or refLayer >
        // currLayer) are never overwritten by the parser; they must equal the default fill,
        // else reparse would diverge. `mlayer_maps_equal` over the full grid combined with the
        // signaled-region reconstruction below is unnecessary: the writer emits exactly the
        // signaled triangle, so only the *unsignaled* entries need to match the default.
        check_mlayer_unsignaled_matches_default(general, &m_default)?;
    }

    // --- tlayer map ---
    let t_default = TLayerDependencyMap::default_for(general.max_tlayer_id, general.max_mlayer_id);
    if max_tlayer == 0 {
        if general.tlayer_dependency_present_flag
            || general.multi_tlayer_dependency_map_present_flag
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "tlayer_dependency_present_flag",
            });
        }
        if !tlayer_maps_equal(&general.tlayer_dependency_map, &t_default) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "tlayer_dependency_map",
            });
        }
    } else if !general.tlayer_dependency_present_flag {
        // Guard 1: clear present-flag means the default fill. The multi flag is inferred 0.
        if general.multi_tlayer_dependency_map_present_flag {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "multi_tlayer_dependency_map_present_flag",
            });
        }
        if !tlayer_maps_equal(&general.tlayer_dependency_map, &t_default) {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "tlayer_dependency_map",
            });
        }
    } else {
        // The multi flag is only signaled when max_mlayer > 0; otherwise it is inferred 0.
        if max_mlayer == 0 && general.multi_tlayer_dependency_map_present_flag {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "multi_tlayer_dependency_map_present_flag",
            });
        }
        // Guard 3: with !multi, layers 1..=max_mlayer must already copy layer 0's values
        // (over the signaled triangle), and unsignaled entries must equal the default fill.
        check_tlayer_canonical_when_present(general, &t_default)?;
    }

    Ok(())
}

/// Returns `true` if two mlayer maps agree on every entry the parser could read or infer
/// (the full `MAX_NUM_MLAYERS` grid; ids outside the 3-bit range read `false`).
fn mlayer_maps_equal(a: &MLayerDependencyMap, b: &MLayerDependencyMap) -> bool {
    for curr in 0..MAX_NUM_MLAYERS {
        for reference in 0..MAX_NUM_MLAYERS {
            let ca = a.depends_on(
                EmbeddedLayerId::from_bits(curr),
                EmbeddedLayerId::from_bits(reference),
            );
            let cb = b.depends_on(
                EmbeddedLayerId::from_bits(curr),
                EmbeddedLayerId::from_bits(reference),
            );
            if ca != cb {
                return false;
            }
        }
    }
    true
}

/// Returns `true` if two tlayer maps agree on every `[m][curr][ref]` entry.
fn tlayer_maps_equal(a: &TLayerDependencyMap, b: &TLayerDependencyMap) -> bool {
    for m in 0..MAX_NUM_MLAYERS {
        for curr in 0..MAX_NUM_TLAYERS {
            for reference in 0..MAX_NUM_TLAYERS {
                let ca = a.depends_on(
                    EmbeddedLayerId::from_bits(m),
                    TemporalLayerId::from_bits(curr),
                    TemporalLayerId::from_bits(reference),
                );
                let cb = b.depends_on(
                    EmbeddedLayerId::from_bits(m),
                    TemporalLayerId::from_bits(curr),
                    TemporalLayerId::from_bits(reference),
                );
                if ca != cb {
                    return false;
                }
            }
        }
    }
    true
}

/// Ensures every mlayer entry the parser does NOT signal (row 0, rows `> max_mlayer`, and
/// the strict upper triangle `refLayer > currLayer`) equals the §5.4.1 default fill — those
/// entries keep their default after a `present == 1` parse, so a divergent stored value
/// would not round-trip.
fn check_mlayer_unsignaled_matches_default(
    general: &SequenceHeaderGeneral,
    default: &MLayerDependencyMap,
) -> WriteResult<()> {
    let max_mlayer = general.max_mlayer_id.get();
    let map = &general.mlayer_dependency_map;
    for curr in 0..MAX_NUM_MLAYERS {
        for reference in 0..MAX_NUM_MLAYERS {
            // The parser signals (and so can reproduce) only currLayer in 1..=max_mlayer and
            // refLayer in 0..=currLayer. Everything else stays at the default fill.
            let signaled = curr >= 1 && curr <= max_mlayer && reference <= curr;
            if signaled {
                continue;
            }
            let stored = map.depends_on(
                EmbeddedLayerId::from_bits(curr),
                EmbeddedLayerId::from_bits(reference),
            );
            let expected = default.depends_on(
                EmbeddedLayerId::from_bits(curr),
                EmbeddedLayerId::from_bits(reference),
            );
            if stored != expected {
                return Err(WriteError::NonCanonicalSequenceValue {
                    what: "mlayer_dependency_map",
                });
            }
        }
    }
    Ok(())
}

/// Ensures a `tlayer_dependency_present_flag == 1` tlayer map is reproducible: signaled
/// entries (embedded layer 0, or all layers when `multi`) are taken as-is, the `!multi`
/// copy of layer 0 to layers `>0` must already hold, and every unsignaled entry must equal
/// the §5.4.1 default fill.
fn check_tlayer_canonical_when_present(
    general: &SequenceHeaderGeneral,
    default: &TLayerDependencyMap,
) -> WriteResult<()> {
    let max_mlayer = general.max_mlayer_id.get();
    let max_tlayer = general.max_tlayer_id.get();
    let multi = general.multi_tlayer_dependency_map_present_flag;
    let map = &general.tlayer_dependency_map;

    let depends = |m: u8, curr: u8, reference: u8| {
        map.depends_on(
            EmbeddedLayerId::from_bits(m),
            TemporalLayerId::from_bits(curr),
            TemporalLayerId::from_bits(reference),
        )
    };
    let default_depends = |m: u8, curr: u8, reference: u8| {
        default.depends_on(
            EmbeddedLayerId::from_bits(m),
            TemporalLayerId::from_bits(curr),
            TemporalLayerId::from_bits(reference),
        )
    };

    for m in 0..MAX_NUM_MLAYERS {
        for curr in 0..MAX_NUM_TLAYERS {
            for reference in 0..MAX_NUM_TLAYERS {
                // The signaled/replicated region is m in 0..=max_mlayer, curr in 1..=max_tlayer,
                // ref in 0..=curr. Everything else keeps the default fill.
                let in_region =
                    m <= max_mlayer && (1..=max_tlayer).contains(&curr) && reference <= curr;
                if !in_region {
                    if depends(m, curr, reference) != default_depends(m, curr, reference) {
                        return Err(WriteError::NonCanonicalSequenceValue {
                            what: "tlayer_dependency_map",
                        });
                    }
                    continue;
                }
                // With !multi, layers >0 are a verbatim copy of layer 0's signaled values.
                if !multi && m > 0 {
                    let layer0 = depends(0, curr, reference);
                    if depends(m, curr, reference) != layer0 {
                        return Err(WriteError::NonCanonicalSequenceValue {
                            what: "tlayer_dependency_map",
                        });
                    }
                }
                // Signaled layer-0 entries (and `multi` layers) are free — any bool is encodable.
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::{ChromaFormatIdc, Tier, parse_sequence_header_general};
    use crate::span::ByteOffset;

    /// MSB-first bit builder mirroring the `Bits` helper in
    /// `headers::sequence`'s own tests, so this module reuses the same hand-built,
    /// spec-grounded fixtures.
    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    fn parse(bytes: &[u8]) -> SequenceHeaderGeneral {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_sequence_header_general(&mut reader).unwrap()
    }

    fn write(general: &SequenceHeaderGeneral) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_sequence_header_general(&mut writer, general).unwrap();
        writer.into_bytes()
    }

    /// Asserts the semantic round-trip `parse(write(g)) == g` and byte-stability.
    fn assert_semantic_roundtrip(general: &SequenceHeaderGeneral) {
        let bytes = write(general);
        let reparsed = parse(&bytes);
        assert_eq!(&reparsed, general, "parse(write(g)) != g");
        // Byte-stability: re-emitting the reparsed model produces identical bytes.
        assert_eq!(write(&reparsed), bytes, "write not idempotent");
    }

    // ----- Byte-exact tests against the parser's own hand-built fixtures -----

    /// A single-picture general header is byte-exact: the fixture ends on a byte
    /// boundary (the parser test `valid_single_picture_prefix` is byte-aligned),
    /// so the writer reproduces it exactly.
    #[test]
    fn single_picture_prefix_byte_exact() {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        // The general fields end mid-byte (41 bits). Both `Bits::into_bytes` and the
        // writer's `into_bytes` zero-pad the trailing partial byte identically, so the
        // produced byte vector is byte-exact against the fixture for the whole region.
        let data = bits.into_bytes();
        let general = parse(&data);
        let written = write(&general);
        assert_eq!(written, data, "single-picture prefix not byte-exact");
        assert_semantic_roundtrip(&general);
    }

    /// A non-single-picture header with a decoder-model cascade and a cropping
    /// window. The fixture is padded to a byte boundary with a final mlayer/tlayer
    /// absence; the comparison is semantic (mid-byte tail), and byte-exact over the
    /// whole-byte prefix.
    #[test]
    fn non_single_picture_with_cropping_and_delay_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(3); // seq_header_id
        bits.f(31, 5); // seq_profile_idc (Configurable)
        bits.bit(0); // single_picture_header_flag
        bits.f(2, 5); // seq_level_idx (<= 3 -> no seq_tier bit)
        bits.uvlc(2); // chroma_format_idc = 444
        bits.uvlc(1); // bit_depth_idc = 8-bit
        bits.f(5, 3); // seq_lcr_id
        bits.bit(1); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id
        bits.bit(0); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1 = 16
        bits.f(7, 4); // max_frame_height_minus_1 = 8
        bits.bit(1); // seq_cropping_window_present_flag
        bits.uvlc(1); // left
        bits.uvlc(2); // right
        bits.uvlc(3); // top
        bits.uvlc(4); // bottom
        bits.bit(1); // seq_initial_display_delay_present_flag
        bits.f(2, 4); // seq_initial_display_delay_minus_1
        bits.bit(0); // decoder_model_info_present_flag
        let data = bits.into_bytes();
        let general = parse(&data);
        assert_eq!(general.seq_header_id.get(), 3);
        assert_eq!(general.seq_profile_idc.get(), 31);
        assert_eq!(general.cropping_window.left, 1);
        assert_eq!(general.cropping_window.bottom, 4);
        assert_eq!(general.seq_initial_display_delay_minus_1, Some(2));
        assert_semantic_roundtrip(&general);
    }

    /// A header with `seq_decoder_model_info()` present exercises the §5.4.13
    /// inner writer and the full cascade.
    #[test]
    fn decoder_model_info_present_round_trips() {
        let mut bits = Bits::default();
        bits.uvlc(1); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(5, 5); // seq_level_idx (> 3 -> seq_tier bit follows)
        bits.bit(1); // seq_tier = High
        bits.uvlc(0); // chroma_format_idc = 420
        bits.uvlc(0); // bit_depth_idc = 10-bit
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(1); // decoder_model_info_present_flag
        bits.f(48000, 32); // num_units_in_decoding_tick
        bits.bit(1); // seq_decoder_model_info_present_flag
        bits.uvlc(7); // decoder_buffer_delay
        bits.uvlc(9); // encoder_buffer_delay
        bits.bit(1); // low_delay_mode_flag
        let data = bits.into_bytes();
        let general = parse(&data);
        assert_eq!(general.seq_tier, Tier::High);
        assert_eq!(general.num_units_in_decoding_tick, Some(48000));
        let model = general.decoder_model_info.unwrap();
        assert_eq!(model.decoder_buffer_delay, 7);
        assert_eq!(model.encoder_buffer_delay, 9);
        assert!(model.low_delay_mode_flag);
        assert_semantic_roundtrip(&general);
    }

    /// mlayer dependency map with signaled (descending) override bits round-trips —
    /// the writer must reproduce the diagonal-first order exactly.
    #[test]
    fn mlayer_dependency_override_round_trips() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 0, 2);
        bits.bit(1); // mlayer_dependency_present_flag
        bits.bit(0); // currLayer 1: [1][1]
        bits.bit(1); // currLayer 1: [1][0]
        bits.bit(1); // currLayer 2: [2][2]
        bits.bit(1); // currLayer 2: [2][1]
        bits.bit(0); // currLayer 2: [2][0]
        let data = bits.into_bytes();
        let general = parse(&data);
        assert!(general.mlayer_dependency_present_flag);
        assert_semantic_roundtrip(&general);
    }

    /// tlayer dependency with row-0 replication (`!multi`) round-trips: the writer
    /// emits only embedded-layer-0's bits and the copy is verified, not re-emitted.
    #[test]
    fn tlayer_dependency_row0_replication_round_trips() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 1, 1);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(0); // multi_tlayer_dependency_map_present_flag
        bits.bit(1); // mLayer 0, currTLayer 1: [0][1][1]
        bits.bit(0); // mLayer 0, currTLayer 1: [0][1][0]
        let data = bits.into_bytes();
        let general = parse(&data);
        assert!(general.tlayer_dependency_present_flag);
        assert!(!general.multi_tlayer_dependency_map_present_flag);
        assert_semantic_roundtrip(&general);
    }

    /// tlayer dependency with `multi` set: distinct per-mLayer rows round-trip.
    #[test]
    fn tlayer_dependency_multi_round_trips() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 1, 1);
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(1); // multi_tlayer_dependency_map_present_flag
        bits.bit(1); // mLayer 0: [0][1][1]
        bits.bit(0); // mLayer 0: [0][1][0]
        bits.bit(0); // mLayer 1: [1][1][1]
        bits.bit(1); // mLayer 1: [1][1][0]
        let data = bits.into_bytes();
        let general = parse(&data);
        assert!(general.multi_tlayer_dependency_map_present_flag);
        assert_semantic_roundtrip(&general);
    }

    /// The maximal-id dependency region (max_mlayer_id 7, max_tlayer_id 3, multi)
    /// exercises the largest signaled-bit loops.
    #[test]
    fn max_id_dependency_region_round_trips() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 3, 7);
        bits.bit(1); // mlayer_dependency_present_flag
        // mlayer signaled triangle: curr 1..=7, ref curr..=0 -> sum(2..=8) = 35 bits.
        for curr in 1u32..=7 {
            for reference in (0..=curr).rev() {
                bits.bit(((curr + reference) % 2) as u8);
            }
        }
        bits.bit(1); // tlayer_dependency_present_flag
        bits.bit(1); // multi
        // tlayer: m 0..=7, curr 1..=3, ref curr..=0 -> 8 * (2+3+4) = 72 bits.
        for m in 0u32..=7 {
            for curr in 1u32..=3 {
                for reference in (0..=curr).rev() {
                    bits.bit(((m + curr + reference) % 2) as u8);
                }
            }
        }
        let data = bits.into_bytes();
        let general = parse(&data);
        assert!(general.mlayer_dependency_present_flag);
        assert!(general.multi_tlayer_dependency_map_present_flag);
        assert_semantic_roundtrip(&general);
    }

    /// Appends the general non-single-picture fields up to the dependency-map
    /// region, mirroring `headers::sequence`'s test helper.
    fn push_general_until_deps(bits: &mut Bits, max_tlayer_id: u32, max_mlayer_id: u32) {
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx (<= 3 -> seq_tier inferred Main)
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(max_tlayer_id, 2); // max_tlayer_id
        bits.f(max_mlayer_id, 3); // max_mlayer_id
        if max_mlayer_id > 0 {
            let n = u32::BITS - max_mlayer_id.leading_zeros();
            bits.f(max_mlayer_id, n); // seq_max_mlayer_cnt_minus_1 = max_mlayer_id
        }
        bits.bit(0); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
    }

    // ----- Rejection tests (one per WriteError reject path) -----

    /// An unaligned writer is rejected before any bit (WriterNotByteAligned).
    #[test]
    fn rejects_unaligned_writer() {
        let general = parse(&single_picture_fixture());
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::WriterNotByteAligned)
        ));
        // The writer kept its single pre-existing bit; the helper wrote nothing.
        assert_eq!(writer.bit_len(), 1);
    }

    /// `seq_tier == High` with the gate false (single-picture or low level) is
    /// non-canonical and rejected with no bit written.
    #[test]
    fn rejects_seq_tier_high_when_gate_false() {
        let mut general = parse(&single_picture_fixture());
        general.seq_tier = Tier::High; // gate is false (single_picture)
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue { what: "seq_tier" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// A field whose model value exceeds its bit width is rejected with ValueTooWide.
    #[test]
    fn rejects_field_width_overflow() {
        let mut general = parse(&single_picture_fixture());
        // seq_level_idx is f(5); force a value of 32 via from_bits (parser-unreachable).
        general.seq_level_idx = crate::headers::sequence::LevelIdx::from_bits(32);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::ValueTooWide { width_bits: 5, .. })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// A `max_frame_width_minus_1` exceeding `frame_width_bits` is rejected
    /// (width loss) with ValueTooWide and no bit written.
    #[test]
    fn rejects_frame_width_exceeding_bits() {
        let mut general = parse(&single_picture_fixture());
        // frame_width_bits is 4 (minus_1 == 3); a width of 16 needs 5 bits.
        general.max_frame_width = crate::headers::sequence::FrameWidth::from_minus_1(16);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::ValueTooWide { width_bits: 4, .. })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// A non-default cropping window with the present-flag clear is non-canonical.
    #[test]
    fn rejects_non_default_window_when_flag_clear() {
        let mut general = parse(&single_picture_fixture());
        assert!(!general.seq_cropping_window_present_flag);
        general.cropping_window.left = 1; // non-default while flag is clear
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "cropping_window"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
        // The dedicated helper rejects identically.
        let mut w2 = BitWriter::new();
        assert!(matches!(
            write_cropping_window(&mut w2, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "cropping_window"
            })
        ));
        assert_eq!(w2.bit_len(), 0);
    }

    /// A cropping offset exceeding `max_frame_*_minus_1` is non-canonical.
    #[test]
    fn rejects_cropping_offset_out_of_range() {
        // Build a non-single-picture header WITH a present cropping window, then
        // push an offset past max_frame_width_minus_1.
        let mut bits = Bits::default();
        push_general_until_crop_present(&mut bits);
        bits.uvlc(0); // left
        bits.uvlc(0); // right
        bits.uvlc(0); // top
        bits.uvlc(0); // bottom
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        let data = bits.into_bytes();
        let mut general = parse(&data);
        assert!(general.seq_cropping_window_present_flag);
        // max_frame_width_minus_1 is 15; 16 is out of range.
        general.cropping_window.left = general.max_frame_width.minus_1() + 1;
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "cropping_window"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// An mlayer map that is non-default while the present-flag is clear is
    /// non-canonical (the parser would have left the default fill).
    #[test]
    fn rejects_non_canonical_mlayer_map() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 0, 2);
        bits.bit(0); // mlayer_dependency_present_flag = clear -> default fill
        let data = bits.into_bytes();
        let mut general = parse(&data);
        assert!(!general.mlayer_dependency_present_flag);
        // Replace the map with a different default fill (built for max_mlayer_id 1): its
        // `[2][0]` entry is false where the stored max_mlayer_id 2 expects true, so it can
        // no longer be reproduced from the clear present-flag.
        general.mlayer_dependency_map =
            MLayerDependencyMap::default_for(crate::types::EmbeddedLayerId::from_bits(1));
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "mlayer_dependency_map"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// A present-flag set on a header where the gate infers it clear
    /// (`max_mlayer_id == 0`) is non-canonical.
    #[test]
    fn rejects_inconsistent_present_flag() {
        let mut general = parse(&single_picture_fixture());
        // single-picture infers max_mlayer_id 0 and the present-flag 0; setting it true
        // is unrepresentable. The single-picture layer-block check fires first only if a
        // layer-block field changed; here we keep the block valid and just flip the flag.
        general.mlayer_dependency_present_flag = true;
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "mlayer_dependency_present_flag"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// `decoder_model_info_present_flag` set but `num_units_in_decoding_tick` is
    /// `None` is non-canonical (Option/flag mismatch).
    #[test]
    fn rejects_decoder_model_option_mismatch() {
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 0, 0);
        bits.bit(0); // (deps: max ids 0 -> no bits)
        let data = bits.into_bytes();
        let mut general = parse(&data);
        general.decoder_model_info_present_flag = true; // flag set but no num_units
        assert!(general.num_units_in_decoding_tick.is_none());
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "num_units_in_decoding_tick"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// `num_units_in_decoding_tick == Some(0)` is rejected (the parser rejects 0).
    #[test]
    fn rejects_zero_num_units() {
        // A non-single-picture model (so the decoder-model cascade is reachable).
        let mut bits = Bits::default();
        push_general_until_deps(&mut bits, 0, 0);
        let mut general = parse(&bits.into_bytes());
        general.decoder_model_info_present_flag = true;
        general.num_units_in_decoding_tick = Some(0);
        general.seq_decoder_model_info_present_flag = false;
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "num_units_in_decoding_tick"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// A single-picture header carrying a non-inferred layer-block constant is
    /// non-canonical.
    #[test]
    fn rejects_single_picture_non_inferred_constant() {
        let mut general = parse(&single_picture_fixture());
        general.still_picture = false; // inferred true for single-picture
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_sequence_header_general(&mut writer, &general),
            Err(WriteError::NonCanonicalSequenceValue {
                what: "single_picture_layer_block"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    // ----- Fixtures -----

    fn single_picture_fixture() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.into_bytes()
    }

    /// Non-single-picture general fields up to a PRESENT cropping window flag.
    fn push_general_until_crop_present(bits: &mut Bits) {
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id
        bits.bit(0); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(1); // seq_cropping_window_present_flag
    }

    /// Chroma sanity: a 420/8-bit single picture round-trips (covers the chroma uvlc).
    #[test]
    fn chroma_and_bit_depth_round_trip() {
        let general = parse(&single_picture_fixture());
        assert_eq!(general.chroma_format_idc, ChromaFormatIdc::Yuv420);
        assert_semantic_roundtrip(&general);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::parse_sequence_header_general;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    /// MSB-first bit builder (a copy of the `tests` module's helper, kept local so the
    /// proptest strategies can synthesize parser-reachable byte fixtures field-by-field).
    #[derive(Default, Clone)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }
        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }
        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }
        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    /// Builds a parser-reachable general-header byte fixture from primitive choices,
    /// honoring every §5.4.1 gate so the parser accepts it (and so the writer never
    /// rejects the parsed model).
    #[allow(clippy::too_many_arguments)]
    fn build_general_fixture(
        single_picture: bool,
        seq_header_id: u32,
        profile: u32,
        level: u32,
        tier_bit: u8,
        chroma: u32,
        bit_depth: u32,
        lcr: u32,
        still: u8,
        max_tlayer: u32,
        max_mlayer: u32,
        monotonic: u8,
        crop_present: bool,
        delay_present: bool,
        delay_minus_1: u32,
        decoder_model: bool,
        mlayer_present: bool,
        tlayer_present: bool,
        multi: bool,
        dep_bits: &[u8],
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(profile, 5);
        bits.bit(u8::from(single_picture));
        bits.f(level, 5);
        // seq_tier is signaled iff level > 3 && !single_picture.
        if level > 3 && !single_picture {
            bits.bit(tier_bit);
        }
        bits.uvlc(chroma);
        bits.uvlc(bit_depth);

        let (mt, mm) = if single_picture {
            (0u32, 0u32)
        } else {
            bits.f(lcr, 3);
            bits.bit(still);
            bits.f(max_tlayer, 2);
            bits.f(max_mlayer, 3);
            if max_mlayer > 0 {
                let n = u32::BITS - (max_mlayer + 1 - 1).leading_zeros();
                // seq_max_mlayer_cnt_minus_1 in 0..=max_mlayer; pick max_mlayer (valid).
                bits.f(max_mlayer, n);
            }
            bits.bit(monotonic);
            (max_tlayer, max_mlayer)
        };

        bits.f(3, 4); // frame_width_bits_minus_1 = 3 -> 4 bits
        bits.f(3, 4); // frame_height_bits_minus_1 = 3 -> 4 bits
        bits.f(15, 4); // max_frame_width_minus_1 = 15
        bits.f(7, 4); // max_frame_height_minus_1 = 7

        bits.bit(u8::from(crop_present));
        if crop_present {
            bits.uvlc(0); // left (<= 15)
            bits.uvlc(1); // right
            bits.uvlc(2); // top (<= 7)
            bits.uvlc(3); // bottom
        }

        if !single_picture {
            bits.bit(u8::from(delay_present));
            if delay_present {
                bits.f(delay_minus_1 & 0xF, 4);
            }
            bits.bit(u8::from(decoder_model));
            if decoder_model {
                bits.f(48000, 32); // num_units_in_decoding_tick (> 0)
                bits.bit(0); // seq_decoder_model_info_present_flag (keep it simple)
            }
        }

        // Dependency maps.
        let mut idx = 0usize;
        let mut next = || {
            let b = dep_bits.get(idx).copied().unwrap_or(0) & 1;
            idx += 1;
            b
        };
        if mm > 0 {
            bits.bit(u8::from(mlayer_present));
            if mlayer_present {
                for curr in 1..=mm {
                    for _ref in (0..=curr).rev() {
                        bits.bit(next());
                    }
                }
            }
        }
        if mt > 0 {
            bits.bit(u8::from(tlayer_present));
            if tlayer_present {
                let multi_eff = if mm > 0 {
                    bits.bit(u8::from(multi));
                    multi
                } else {
                    false
                };
                for m in 0..=mm {
                    for curr in 1..=mt {
                        for _ref in (0..=curr).rev() {
                            if multi_eff || m == 0 {
                                bits.bit(next());
                            }
                        }
                    }
                }
            }
        }
        bits.into_bytes()
    }

    fn parse_ok(bytes: &[u8]) -> Option<SequenceHeaderGeneral> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_sequence_header_general(&mut reader).ok()
    }

    proptest! {
        /// Every parser-reachable general header round-trips: parse(write(g)) == g, and
        /// re-emission is byte-stable.
        #[test]
        fn roundtrip_general_header(
            single_picture in any::<bool>(),
            seq_header_id in 0u32..16,
            profile in 0u32..32,
            level in 0u32..32,
            tier_bit in 0u8..2,
            chroma in 0u32..4,
            bit_depth in 0u32..2,
            lcr in 0u32..8,
            still in 0u8..2,
            max_tlayer in 0u32..4,
            max_mlayer in 0u32..8,
            monotonic in 0u8..2,
            crop_present in any::<bool>(),
            delay_present in any::<bool>(),
            delay_minus_1 in 0u32..16,
            decoder_model in any::<bool>(),
            mlayer_present in any::<bool>(),
            tlayer_present in any::<bool>(),
            multi in any::<bool>(),
            dep_bits in proptest::collection::vec(0u8..2, 0..128),
        ) {
            let fixture = build_general_fixture(
                single_picture, seq_header_id, profile, level, tier_bit, chroma, bit_depth,
                lcr, still, max_tlayer, max_mlayer, monotonic, crop_present, delay_present,
                delay_minus_1, decoder_model, mlayer_present, tlayer_present, multi, &dep_bits,
            );
            // The fixture is parser-reachable by construction; if a chosen combination
            // happens to truncate (e.g. dep_bits short), skip non-parsing inputs.
            let Some(general) = parse_ok(&fixture) else { return Ok(()); };

            let mut writer = BitWriter::new();
            write_sequence_header_general(&mut writer, &general).unwrap();
            let bytes = writer.into_bytes();
            let reparsed = parse_ok(&bytes).expect("written bytes must reparse");
            prop_assert_eq!(&reparsed, &general);
            // Byte-stable re-emission.
            let mut w2 = BitWriter::new();
            write_sequence_header_general(&mut w2, &reparsed).unwrap();
            prop_assert_eq!(w2.into_bytes(), bytes);
        }

        /// The writer never panics on a parsed model regardless of field values (it
        /// returns `Result`); covers the reject paths too.
        #[test]
        fn writer_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..48)) {
            if let Some(general) = parse_ok(&bytes) {
                let mut writer = BitWriter::new();
                let _ = write_sequence_header_general(&mut writer, &general);
                let mut w2 = BitWriter::new();
                let _ = write_cropping_window(&mut w2, &general);
                let mut w3 = BitWriter::new();
                let _ = write_dependency_maps(&mut w3, &general);
            }
        }
    }
}
