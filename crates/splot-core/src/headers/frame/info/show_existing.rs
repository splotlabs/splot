// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.18.2 show-existing-frame (SEF) sub-path.
//!
//! A self-contained island of the frame-header core parser:
//! [`parse_show_existing_frame`] reads the SEF fields (`frame_to_show_map_idx`, the
//! order hint, the terminal `film_grain_config()`) and [`classify_sef_trailing_bits`]
//! resolves the SEF OBU's `trailing_bits()` boundary (§ 5.2.1 / § 5.2.3). Both operate
//! on the shared [`FrameHeaderCore`](super::FrameHeaderCore) /
//! [`CoreSeqView`](super::CoreSeqView).

use crate::bitio::BitReader;
use crate::error::{Error, Result, TrailingBitsErrorKind};
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::tail::{FrameTailInput, parse_film_grain_config};
use crate::obu::parse_trailing_bits;

use super::{
    CoreSeqView, FRAME_HEADER_INFO_FEATURE, FrameHeaderCore, FrameHeaderParseStatus,
    SefTrailingBits,
};

/// Validates `trailing_bits( remainingPayloadBits )` over the rest of `reader`'s payload
/// for a show-existing-frame OBU (AV2 § 5.2.1 / § 5.2.3), classifying the outcome without
/// failing the parse so the already-parsed SEF facts survive.
fn classify_sef_trailing_bits(reader: &mut BitReader<'_>) -> SefTrailingBits {
    match parse_trailing_bits(reader, reader.remaining_bits()) {
        Ok(()) => SefTrailingBits::Valid,
        Err(Error::InvalidTrailingBits { kind, .. }) => match kind {
            TrailingBitsErrorKind::Empty => SefTrailingBits::Empty,
            TrailingBitsErrorKind::MissingOneBit => SefTrailingBits::MissingOneBit,
            TrailingBitsErrorKind::ZeroBitNotZero => SefTrailingBits::ZeroBitNotZero,
        },
        // `parse_trailing_bits` reads exactly `remaining_bits()` bits, so it cannot run
        // past the payload; an EOF here is unreachable, but treat it conservatively as a
        // missing marker rather than panicking.
        Err(_) => SefTrailingBits::MissingOneBit,
    }
}

/// Parses the show-existing-frame sub-path (AV2 § 5.18.2), stopping before
/// `film_grain_config()`.
pub(super) fn parse_show_existing_frame(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
) -> Result<()> {
    core.frame_to_show_map_idx = Some(reader.read_f(ceil_log2(seq.num_ref_frames))?);
    let derive_sef_order_hint = reader.read_flag()?;
    if !derive_sef_order_hint {
        core.order_hint_lsb = Some(reader.read_f(seq.order_hint_bits)?);
    }
    // AV2 § 5.18.2 (mirror :4180-4184): refresh_frame_flags = 0; immediate_output_frame = 1.
    // FrameType comes from the referenced slot (reference state), so it is left unknown.
    core.refresh_frame_flags = Some(0);
    core.immediate_output_frame = Some(true);

    // AV2 § 5.18.2 (mirror :4186): the SEF path calls film_grain_config() (§ 5.18.10.1),
    // then return()s (mirror :4196) — the frame header is complete. SEF only occurs when
    // single_picture_header_flag == 0 (the else arm of mirror :4131), so the
    // film_grain_config() single-picture inference is dead; with immediate_output_frame = 1
    // the (!immediate && !implicit) output gate is false, so apply_grain is f(1) when grain
    // is present. The save_grain_params() call at mirror :4190 reads no bits.
    //
    // film_grain_config() consumes film_grain_params_present (§ 5.4.1, the apply_grain
    // gate). If the active sequence header was a bounded stop that never read that flag, it
    // is genuinely unknown — the SEF facts above (frame_to_show_map_idx, order hint, output
    // flags) are preserved, but the parser cannot decide apply_grain without guessing, so it
    // stops honestly here rather than inventing the flag.
    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    let input = FrameTailInput {
        // SEF reads no read_tx_mode(): coded_lossless is irrelevant here but supplied for
        // the shared input shape (film_grain_config does not consult it).
        coded_lossless: false,
        film_grain_params_present,
        // SEF never runs under single_picture_header_flag (see above).
        single_picture_header_flag: false,
        immediate_output_frame: true,
        implicit_output_frame: false,
    };
    match parse_film_grain_config(reader, &input) {
        Ok(film_grain) => {
            core.sef_film_grain = Some(film_grain);
            // AV2 § 5.2.1 (:124-152) / § 5.2.3: a SEF OBU is not an is_tile_group() type,
            // so usedArith == 0 and the rest of the payload is exactly
            // trailing_bits( remainingPayloadBits ) — the SEF arm of § 5.18.2 (mirror :4145)
            // return()s right after film_grain_config() (:4186), and there is no tile data.
            // Classify that boundary so the validator can surface a non-conformant tail
            // (including the grain_seed-eats-the-marker case) as a § 6.2.1 / § 5.2.3
            // diagnostic, without failing the parse — the parsed SEF facts survive.
            core.sef_trailing_bits = Some(classify_sef_trailing_bits(reader));
            core.status = FrameHeaderParseStatus::ShowExistingFrameComplete;
            Ok(())
        }
        // A payload EOF inside the SEF film_grain_config() keeps the already-parsed SEF
        // facts (frame_to_show_map_idx, order hint, output flags) and reports the
        // truncation through the status rather than failing the whole parse. The SEF tail
        // IS film_grain_config() (a fully-modeled region), so this is a decidable
        // truncation (StoppedInsideShowExistingFrame), distinct from the ordinary bounded
        // CoreFieldsOnly stop — the validator surfaces it as a truncated-frame-header error.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideShowExistingFrame;
            Ok(())
        }
        Err(error) => Err(error),
    }
}
