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
    core.refresh_frame_flags = Some(0);
    core.immediate_output_frame = Some(true);

    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    let input = FrameTailInput {
        coded_lossless: false,
        film_grain_params_present,
        single_picture_header_flag: false,
        immediate_output_frame: true,
        implicit_output_frame: false,
    };
    match parse_film_grain_config(reader, &input) {
        Ok(film_grain) => {
            core.sef_film_grain = Some(film_grain);
            core.sef_trailing_bits = Some(classify_sef_trailing_bits(reader));
            core.status = FrameHeaderParseStatus::ShowExistingFrameComplete;
            Ok(())
        }
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideShowExistingFrame;
            Ok(())
        }
        Err(error) => Err(error),
    }
}
