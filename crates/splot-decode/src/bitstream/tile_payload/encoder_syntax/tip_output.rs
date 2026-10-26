// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `TIP_FRAME_AS_OUTPUT` frames (§ 5.18.2, § 7.11.3): the decoder's TIP
//! output reconstruction and reference update over the encoder references.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;
use splot_core::types::EmbeddedLayerId;
use splot_recon::{DecodedFrame, FramePlaneSamples};

use super::super::intra::sequence_bit_depth;
use super::super::{EncoderSyntaxError, Result, rule};
use super::{EncoderReferences, settled_state};
use crate::pipeline::frame_ref_update_from_core;
use crate::prediction::inter::{
    FrameDecodeGeometry, InterDecodeScratch, reconstruct_tip_output, resolve_tip_output_core,
};

impl EncoderReferences {
    /// The TIP output frame header `core` with the quantizer the decoder
    /// infers, and the decoder's output picture of the frame.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] when `core` is not a complete TIP
    /// output frame header or the references cannot project it.
    pub fn tip_output(
        &self,
        core: &FrameHeaderCore,
    ) -> Result<(FrameHeaderCore, DecodedFrame<u16>)> {
        let mut core = core.clone();
        resolve_tip_output_core(
            &mut core,
            &self.sequence,
            &self.state,
            ByteOffset::new(0),
            None,
        )
        .map_err(rule("TIP output header"))?;
        let geometry = FrameDecodeGeometry::new(
            &core,
            &self.sequence,
            sequence_bit_depth(&self.sequence),
            false,
        )
        .map_err(rule("TIP output geometry"))?;
        let (frame, _) = reconstruct_tip_output(
            &mut InterDecodeScratch::default(),
            FramePlaneSamples::default(),
            &core,
            &self.sequence,
            &self.state,
            geometry,
            ByteOffset::new(0),
        )
        .map_err(rule("TIP output"))?;
        Ok((core, frame))
    }

    /// Records the TIP output frame `core` of [`Self::tip_output`], which
    /// refreshes no slot, as the decoder's § 7.20 reference update does.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a frame that refreshes a slot.
    pub fn update_tip_output(&mut self, core: &FrameHeaderCore) -> Result<()> {
        if core.refresh_frame_flags != Some(0) {
            return Err(EncoderSyntaxError::Unsupported(
                "a TIP output frame that refreshes a slot",
            ));
        }
        self.buffer.prepare_for_frame(core.obu_type, true);
        let update =
            frame_ref_update_from_core(core, ByteOffset::new(0), EmbeddedLayerId::from_bits(0))
                .map_err(rule("reference update"))?;
        self.buffer.update(self.frames.len(), &update, false);
        self.buffer
            .note_frame(core.obu_type, true, &update, &core.ref_long_term_ids);
        self.state = settled_state(&self.buffer, &self.frames)?;
        Ok(())
    }
}
