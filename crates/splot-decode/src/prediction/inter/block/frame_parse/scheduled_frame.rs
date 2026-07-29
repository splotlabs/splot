// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Multi-row scheduled-frame exactness tests.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

#[cfg(test)]
mod tests {
    use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};
    use splot_parallel::ThreadCount;

    const MULTIROW_FIXTURE: &[u8] = include_bytes!(
        "../../../../../../../tests/conformance/vectors/valid/\
         syn-2frame-multirow-inter-64x256-10bit-q100.ivf"
    );
    const DEBLOCK_FIXTURE: &[u8] = include_bytes!(
        "../../../../../../../tests/conformance/vectors/valid/\
         syn-2frame-multirow-deblock-inter-64x256-10bit-q158.ivf"
    );
    const WIDE_THREE_FRAME_FIXTURE: &[u8] = include_bytes!(
        "../../../../../../../tests/conformance/vectors/valid/\
         syn-3frame-derived-smvp-96x64-q100.ivf"
    );
    #[test]
    fn per_row_admission_matches_serial_on_multirow_multistripe_inter()
    -> Result<(), Box<dyn std::error::Error>> {
        let serial = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
        let serial = serial.decode_hash_report_bytes(MULTIROW_FIXTURE, DecodeOptions::default())?;

        let parallel = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(8usize)))?;
        let parallel =
            parallel.decode_hash_report_bytes(MULTIROW_FIXTURE, DecodeOptions::default())?;

        assert_eq!(
            serial.frames[1].hashes[0].digest_hex,
            parallel.frames[1].hashes[0].digest_hex
        );
        assert_eq!(parallel.frames[1].visible_luma_height, 256);
        assert!(
            parallel.frames[1].visible_luma_height > 56,
            "fixture must retain multiple final-filter stripes"
        );
        Ok(())
    }

    #[test]
    fn scheduled_deblock_matches_serial_on_multirow_inter() -> Result<(), Box<dyn std::error::Error>>
    {
        let serial = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
        let serial = serial.decode_hash_report_bytes(DEBLOCK_FIXTURE, DecodeOptions::default())?;

        let parallel = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(8usize)))?;
        let parallel =
            parallel.decode_hash_report_bytes(DEBLOCK_FIXTURE, DecodeOptions::default())?;

        assert_eq!(
            serial.frames[1].hashes[0].digest_hex,
            parallel.frames[1].hashes[0].digest_hex
        );
        Ok(())
    }

    #[test]
    fn scheduled_wide_frame_matches_serial() -> Result<(), Box<dyn std::error::Error>> {
        let serial = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
        let serial =
            serial.decode_hash_report_bytes(WIDE_THREE_FRAME_FIXTURE, DecodeOptions::default())?;

        let parallel = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(8usize)))?;
        let parallel = parallel
            .decode_hash_report_bytes(WIDE_THREE_FRAME_FIXTURE, DecodeOptions::default())?;

        assert_eq!(serial.frames.len(), 3);
        assert_eq!(serial.frames.len(), parallel.frames.len());
        for (serial, parallel) in serial.frames.iter().zip(&parallel.frames) {
            assert_eq!(serial.hashes[0].digest_hex, parallel.hashes[0].digest_hex);
        }
        assert_eq!(parallel.frames[2].visible_luma_width, 96);
        Ok(())
    }
}
