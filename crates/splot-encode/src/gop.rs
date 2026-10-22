// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference structure: the frame type, order hint and refreshed reference
//! slots of each frame, and the frame header and temporal unit they give.
//!
//! The stream is IPPP: every `keyint`-th frame is a closed-loop key frame,
//! and every other frame is a shown inter frame that predicts from the frame
//! before it and replaces it in slot 0 (§ 7.20 keeps only slot 0 valid after a
//! key frame, so each inter frame has exactly one reference).
//!
//! Encoder policy, fixed for now: the sequence sets
//! `monotonic_output_order_flag`, so a hidden frame is shown by a
//! show-existing frame; inter frames code `primary_ref_frame =
//! PRIMARY_REF_NONE` with `disable_cross_frame_cdf_init = 1`, so no CDFs
//! carry across frames; motion vectors are quarter-pel.

use core::num::NonZeroU32;

use splot_core::headers::frame::{
    CoreSeqView, FrameFilterParams, FrameHeaderCore, InterFrameParams, KeyFrameTxParams,
    build_inter_frame_header, build_key_frame_header, encode_inter_frame_obu,
    encode_key_frame_temporal_unit, encode_temporal_delimiter_obu,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_decode::encoder_syntax::EncoderReferences;

use crate::error::{Error, Result};
use crate::rate_control::FrameType;

/// `refresh_frame_flags` of an inter frame: slot 0, the only valid slot.
const INTER_REFRESH: u32 = 1;

/// The plan of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GopFrame {
    /// Zero-based coding-order index.
    pub(crate) frame_index: u64,
    /// Frame type.
    pub(crate) frame_type: FrameType,
    /// `OrderHint`: the distance from the last key frame.
    pub(crate) order_hint: u32,
    /// `immediate_output_frame`; a hidden frame waits for a show-existing frame.
    pub(crate) show_frame: bool,
}

/// Plans frame `frame_index` of an IPPP stream with key frames every `keyint` frames.
pub(crate) fn plan(frame_index: u64, keyint: NonZeroU32) -> GopFrame {
    let offset = frame_index % u64::from(keyint.get());
    GopFrame {
        frame_index,
        frame_type: if offset == 0 {
            FrameType::Key
        } else {
            FrameType::Inter
        },
        order_hint: offset as u32,
        show_frame: true,
    }
}

impl GopFrame {
    /// The frame header at `qindex` with `filters`, parsed with `refs`.
    pub(crate) fn header(
        &self,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        qindex: u32,
        filters: &FrameFilterParams,
        tx: KeyFrameTxParams,
    ) -> Result<FrameHeaderCore> {
        match self.frame_type {
            FrameType::Key => build_key_frame_header(view, qindex, self.order_hint, filters, tx),
            FrameType::Inter => build_inter_frame_header(
                view,
                &refs.header_view(),
                &self.inter_params(qindex, filters, tx),
            )
            .map(|header| header.core),
        }
        .map_err(header_error)
    }

    /// One temporal unit of the frame `core` built by [`Self::header`] with
    /// `(qindex, filters, tx)`; a key frame repeats the sequence header.
    pub(crate) fn temporal_unit(
        &self,
        seq: &SequenceHeader,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        (qindex, filters, tx): (u32, &FrameFilterParams, KeyFrameTxParams),
        core: &FrameHeaderCore,
        tile: &[u8],
    ) -> Result<Vec<u8>> {
        if self.frame_type == FrameType::Key {
            return encode_key_frame_temporal_unit(seq, core, true, tile).map_err(header_error);
        }
        let header = build_inter_frame_header(
            view,
            &refs.header_view(),
            &self.inter_params(qindex, filters, tx),
        )
        .map_err(header_error)?;
        if header.core != *core {
            return Err(Error::Unsupported {
                what: "an inter frame header that changed after coding",
            });
        }
        let mut unit = encode_temporal_delimiter_obu().map_err(header_error)?;
        unit.extend(encode_inter_frame_obu(&header, tile).map_err(header_error)?);
        Ok(unit)
    }

    fn inter_params(
        &self,
        qindex: u32,
        filters: &FrameFilterParams,
        tx: KeyFrameTxParams,
    ) -> InterFrameParams {
        InterFrameParams {
            base_q_idx: qindex,
            order_hint: self.order_hint,
            refresh_frame_flags: INTER_REFRESH,
            show_frame: self.show_frame,
            filters: *filters,
            tx,
        }
    }
}

fn header_error<E: std::error::Error + Send + Sync + 'static>(source: E) -> Error {
    Error::Coding {
        stage: "frame header",
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_ippp_with_key_frames_every_keyint() {
        let keyint = NonZeroU32::MIN.saturating_add(2);
        let frames: Vec<_> = (0..7)
            .map(|index| {
                let frame = plan(index, keyint);
                (frame.frame_type, frame.order_hint)
            })
            .collect();
        assert_eq!(
            frames,
            [
                (FrameType::Key, 0),
                (FrameType::Inter, 1),
                (FrameType::Inter, 2),
                (FrameType::Key, 0),
                (FrameType::Inter, 1),
                (FrameType::Inter, 2),
                (FrameType::Key, 0),
            ]
        );
        assert!((0..5).all(|index| plan(index, NonZeroU32::MIN).frame_type == FrameType::Key));
    }
}
