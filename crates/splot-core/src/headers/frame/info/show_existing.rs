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

use crate::error::{Error, Result, TrailingBitsErrorKind};
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::tail::{FrameTailInput, parse_film_grain_config_with_reader};
use crate::obu::parse_trailing_bits_with_reader;
use crate::syntax::{SyntaxObserver, SyntaxReader};
use crate::types::TemporalLayerId;

use super::{
    CoreSeqView, FRAME_HEADER_INFO_FEATURE, FrameHeaderCore, FrameHeaderParseStatus,
    FrameReferenceStateView, SefTrailingBits, get_disp_order_hint,
};

fn classify_sef_trailing_bits<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
) -> SefTrailingBits {
    let remaining = sr.remaining_bits();
    match parse_trailing_bits_with_reader(sr, remaining) {
        Ok(()) => SefTrailingBits::Valid,
        Err(Error::InvalidTrailingBits { kind, .. }) => match kind {
            TrailingBitsErrorKind::Empty => SefTrailingBits::Empty,
            TrailingBitsErrorKind::MissingOneBit => SefTrailingBits::MissingOneBit,
            TrailingBitsErrorKind::ZeroBitNotZero => SefTrailingBits::ZeroBitNotZero,
        },
        Err(_) => SefTrailingBits::MissingOneBit,
    }
}

pub(super) fn parse_show_existing_frame<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    reference_state: &FrameReferenceStateView<'_>,
    obu_tlayer_id: TemporalLayerId,
) -> Result<()> {
    let frame_to_show_map_idx =
        sr.read_bits("frame_to_show_map_idx", ceil_log2(seq.num_ref_frames))?;
    core.frame_to_show_map_idx = Some(frame_to_show_map_idx);
    let derive_sef_order_hint = sr.read_flag("derive_sef_order_hint")?;
    core.derive_sef_order_hint = Some(derive_sef_order_hint);
    if !derive_sef_order_hint {
        core.order_hint_lsb = Some(sr.read_bits("order_hint_lsb", seq.order_hint_bits)?);
        core.order_hint = get_disp_order_hint(
            core.obu_type,
            core.frame_type,
            core.restricted_prediction_switch,
            core.order_hint_lsb.unwrap_or(0),
            seq,
            reference_state,
            obu_tlayer_id,
        );
    } else if let Ok(index) = usize::try_from(frame_to_show_map_idx) {
        sr.absent("order_hint_lsb", "derive_sef_order_hint");
        core.order_hint = reference_state
            .ref_order_hint
            .and_then(|hints| hints.get(index))
            .copied();
    }
    core.refresh_frame_flags = Some(0);
    core.immediate_output_frame = Some(true);
    core.implicit_output_frame = Some(false);

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
    match parse_film_grain_config_with_reader(sr, input) {
        Ok(film_grain) => {
            core.sef_film_grain = Some(film_grain);
            core.sef_trailing_bits = Some(classify_sef_trailing_bits(sr));
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
