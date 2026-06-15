// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Data model for future `splot decode --output-format hash --json` reports.
//!
//! This module models only the success artifact shape. It does not decode
//! bitstreams, allocate frames, compute digests, write output paths, or invoke
//! external decoders.

/// Stable contract identifier for decode hash reports.
pub const DECODE_HASH_REPORT_CONTRACT_ID: &str = "splot.decode.hash_report";

/// Stable contract version for decode hash reports.
pub const DECODE_HASH_REPORT_CONTRACT_VERSION: u32 = 1;

/// Stable hash variant for AV2 § 7.21.2 raw intermediate output samples.
pub const DECODE_HASH_REPORT_RAW_INTERMEDIATE_OUTPUT_VARIANT: &str = "raw_intermediate_output";

/// Stable digest algorithm identifier for decoded-frame SHA-256 hashes.
pub const DECODE_HASH_REPORT_HASH_ALGORITHM_ID: &str = "splot-dfh-sha256-v1";

/// Stable canonical decoded-output sample byte-stream identifier.
pub const DECODE_HASH_REPORT_BYTE_STREAM_ID: &str = "av2-output-samples-v1";

/// Exact lowercase hexadecimal character count for `splot-dfh-sha256-v1`.
pub const DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN: usize = 64;

/// A `splot.decode.hash_report` v1 success artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeHashReport {
    /// Stable report contract identifier.
    pub contract_id: &'static str,
    /// Stable report contract version.
    pub contract_version: u32,
    /// Output variants requested for the report, even when no frames are emitted.
    pub selected_output_variants: Vec<DecodeOutputVariant>,
    /// Runtime or CLI thread policy selected for this decode run.
    pub selected_thread_policy: String,
    /// Output frames sorted by `output_index`.
    pub frames: Vec<DecodeHashFrame>,
}

impl DecodeHashReport {
    /// Builds a v1 report for the raw intermediate output variant.
    #[must_use]
    pub fn raw_intermediate_output(
        selected_thread_policy: impl Into<String>,
        frames: Vec<DecodeHashFrame>,
    ) -> Self {
        Self {
            contract_id: DECODE_HASH_REPORT_CONTRACT_ID,
            contract_version: DECODE_HASH_REPORT_CONTRACT_VERSION,
            selected_output_variants: vec![DecodeOutputVariant::RawIntermediateOutput],
            selected_thread_policy: selected_thread_policy.into(),
            frames,
        }
    }
}

/// Output variant identifiers carried by hash reports and hash entries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DecodeOutputVariant {
    /// AV2 § 7.21.2 output samples before film-grain synthesis.
    RawIntermediateOutput,
}

impl DecodeOutputVariant {
    /// Returns the stable string identifier for the variant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawIntermediateOutput => DECODE_HASH_REPORT_RAW_INTERMEDIATE_OUTPUT_VARIANT,
        }
    }
}

/// Decoded output pixel format identifiers used by hash report frame entries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DecodeHashPixelFormat {
    /// Monochrome output samples.
    Monochrome,
    /// YUV 4:2:0 output samples.
    Yuv420,
    /// YUV 4:2:2 output samples.
    Yuv422,
    /// YUV 4:4:4 output samples.
    Yuv444,
}

impl DecodeHashPixelFormat {
    /// Returns the stable string identifier for the pixel format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monochrome => "monochrome",
            Self::Yuv420 => "yuv420",
            Self::Yuv422 => "yuv422",
            Self::Yuv444 => "yuv444",
        }
    }
}

/// One output frame entry in a decode hash report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeHashFrame {
    /// Zero-based output-process order index.
    pub output_index: u64,
    /// Visible luma crop origin, in samples.
    pub visible_luma_left: u32,
    /// Visible luma crop origin, in samples.
    pub visible_luma_top: u32,
    /// Visible luma width, in samples.
    pub visible_luma_width: u32,
    /// Visible luma height, in samples.
    pub visible_luma_height: u32,
    /// Visible chroma crop origin, omitted for monochrome output.
    pub chroma_left: Option<u32>,
    /// Visible chroma crop origin, omitted for monochrome output.
    pub chroma_top: Option<u32>,
    /// Visible chroma width, omitted for monochrome output.
    pub chroma_width: Option<u32>,
    /// Visible chroma height, omitted for monochrome output.
    pub chroma_height: Option<u32>,
    /// Decoded sample bit depth.
    pub bit_depth: u8,
    /// Decoded output pixel format.
    pub pixel_format: DecodeHashPixelFormat,
    /// Hashes computed for this output frame.
    pub hashes: Vec<DecodeHashEntry>,
}

/// One decoded-frame hash entry for one output variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeHashEntry {
    /// Output variant hashed by this entry.
    pub variant: DecodeOutputVariant,
    /// Stable digest algorithm identifier.
    pub algorithm_id: &'static str,
    /// Stable decoded-output sample byte-stream identifier.
    pub byte_stream_id: &'static str,
    /// Lowercase hexadecimal digest text.
    pub digest_hex: String,
}

impl DecodeHashEntry {
    /// Builds a raw-intermediate-output `splot-dfh-sha256-v1` hash entry.
    #[must_use]
    pub fn raw_intermediate_output_sha256(digest_hex: impl Into<String>) -> Self {
        Self {
            variant: DecodeOutputVariant::RawIntermediateOutput,
            algorithm_id: DECODE_HASH_REPORT_HASH_ALGORITHM_ID,
            byte_stream_id: DECODE_HASH_REPORT_BYTE_STREAM_ID,
            digest_hex: digest_hex.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_intermediate_report_uses_stable_contract_fields() {
        let report = DecodeHashReport::raw_intermediate_output("threads=1", Vec::new());

        assert_eq!(report.contract_id, "splot.decode.hash_report");
        assert_eq!(report.contract_version, 1);
        assert_eq!(
            report.selected_output_variants,
            vec![DecodeOutputVariant::RawIntermediateOutput]
        );
        assert_eq!(report.selected_thread_policy, "threads=1");
        assert!(report.frames.is_empty());
    }

    #[test]
    fn stable_string_identifiers_match_output_contract() {
        assert_eq!(
            DecodeOutputVariant::RawIntermediateOutput.as_str(),
            "raw_intermediate_output"
        );
        assert_eq!(DecodeHashPixelFormat::Monochrome.as_str(), "monochrome");
        assert_eq!(DecodeHashPixelFormat::Yuv420.as_str(), "yuv420");
        assert_eq!(DecodeHashPixelFormat::Yuv422.as_str(), "yuv422");
        assert_eq!(DecodeHashPixelFormat::Yuv444.as_str(), "yuv444");
        assert_eq!(DECODE_HASH_REPORT_HASH_ALGORITHM_ID, "splot-dfh-sha256-v1");
        assert_eq!(DECODE_HASH_REPORT_BYTE_STREAM_ID, "av2-output-samples-v1");
        assert_eq!(DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN, 64);
    }

    #[test]
    fn raw_intermediate_hash_entry_uses_stable_identifiers() {
        let digest = "0".repeat(DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN);
        let entry = DecodeHashEntry::raw_intermediate_output_sha256(digest.clone());

        assert_eq!(entry.variant, DecodeOutputVariant::RawIntermediateOutput);
        assert_eq!(entry.algorithm_id, DECODE_HASH_REPORT_HASH_ALGORITHM_ID);
        assert_eq!(entry.byte_stream_id, DECODE_HASH_REPORT_BYTE_STREAM_ID);
        assert_eq!(entry.digest_hex, digest);
    }
}
