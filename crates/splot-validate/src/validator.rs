// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use splot_core::Error;
use splot_core::annexb::ObuEnvelope;
use splot_core::ivf::IvfError;
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

use crate::checks::{Check, default_checks, syntax_error_diagnostic};
use crate::context::ValidatorContext;
use crate::diagnostic::{Diagnostic, Severity, ValidationReport};
use crate::error_location::{error_bit_offset, error_offset};
use crate::options::ValidationOptions;

const IVF_DIAGNOSTIC_RULE_IDS: [&str; 5] = [
    "ivf/truncated-header",
    "ivf/invalid-signature",
    "ivf/invalid-header-length",
    "ivf/truncated-frame-header",
    "ivf/truncated-frame-payload",
];

/// Validates AV2 length-delimited bitstreams and produces a [`ValidationReport`].
#[derive(Debug, Clone, Copy)]
pub struct Validator {
    /// When `true`, [`Validator::is_acceptable`] treats a report with warnings
    /// (not just errors) as a conformance failure. The set of diagnostics produced
    /// by [`Validator::validate_bytes`] is unaffected.
    pub strict: bool,
}

impl Validator {
    /// Creates a validator.
    #[must_use]
    pub fn new(strict: bool) -> Self {
        Self { strict }
    }

    /// Returns `true` if `report` passes under this validator's strictness.
    ///
    /// A report always fails if it contains any [`Severity::Error`]; in
    /// [`Validator::strict`] mode it additionally fails if it contains any warning.
    /// This is the single source of truth for pass/fail (the CLI's exit status
    /// uses it).
    #[must_use]
    pub fn is_acceptable(&self, report: &ValidationReport) -> bool {
        report.is_conformant() && !(self.strict && report.warnings().next().is_some())
    }

    /// Validates `data` as a raw AV2 Annex B bitstream or an IVF-wrapped Annex B
    /// bitstream with the default
    /// [`ValidationOptions`] (no external HLS).
    ///
    /// A malformed bitstream is reported as one or more [`Severity::Error`]
    /// diagnostics, never as a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes(&self, data: &[u8]) -> ValidationReport {
        self.validate_bytes_with_options(data, &ValidationOptions::default())
    }

    /// Validates `data` as a raw AV2 Annex B bitstream or an IVF-wrapped Annex B
    /// bitstream using `options`.
    ///
    /// `options` supplies caller-provided external HLS availability (AV2 § 7.3.8);
    /// the default ([`Validator::validate_bytes`]) assumes none. A malformed
    /// bitstream is reported as one or more [`Severity::Error`] diagnostics, never as
    /// a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes_with_options(
        &self,
        data: &[u8],
        options: &ValidationOptions,
    ) -> ValidationReport {
        let mut report = ValidationReport::new();
        // Parse the whole stream, keeping OBUs parsed before any later structural
        // error so their conformance diagnostics are not lost.
        let parsed = parse_bitstream_partial(data);
        let checks = default_checks();
        let mut context = ValidatorContext::default();

        match parsed {
            ParsedBitstream::AnnexB(parsed) => {
                for obu in &parsed.obus {
                    context.observe_obu(obu, options, &mut report);
                    run_checks(&checks, obu, &mut report);
                }
                // The end of the bitstream completes the final temporal unit, flushing
                // the deferred coded-video-sequence-scoped diagnostics (AV2 § 7.3.6;
                // see ValidatorContext::finish).
                context.finish(options, &mut report);
                if let Some(error) = parsed.error {
                    report.push(parse_error_diagnostic(&error));
                }
            }
            ParsedBitstream::Ivf(parsed) => {
                for frame in &parsed.frames {
                    for obu in &frame.obus {
                        context.observe_obu(obu, options, &mut report);
                        run_checks(&checks, obu, &mut report);
                    }
                    if let Some(error) = &frame.error {
                        report.push(parse_error_diagnostic(error));
                    }
                }
                // The end of the IVF input completes the final temporal unit just like
                // the end of a raw Annex B bitstream.
                context.finish(options, &mut report);
                if let Some(error) = &parsed.error {
                    report.push(ivf_error_diagnostic(error));
                }
            }
        }

        report
    }
}

fn run_checks(checks: &[Box<dyn Check>], obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    for check in checks {
        check.run(obu, report);
    }
}

fn parse_error_diagnostic(error: &Error) -> Diagnostic {
    if let Some(diagnostic) = syntax_error_diagnostic(error) {
        return diagnostic;
    }

    let mut diagnostic =
        Diagnostic::new(Severity::Error, "bitstream/parse-error", error.to_string())
            .with_spec_section("Annex B");
    if let Some(offset) = error_offset(error) {
        diagnostic = diagnostic.with_byte_offset(offset);
    }
    if let Some(bit_offset) = error_bit_offset(error) {
        diagnostic = diagnostic.with_bit_offset(bit_offset);
    }
    diagnostic
}

fn ivf_error_diagnostic(error: &IvfError) -> Diagnostic {
    debug_assert!(IVF_DIAGNOSTIC_RULE_IDS.contains(&error.rule_id()));
    Diagnostic::new(Severity::Error, error.rule_id(), error.to_string())
        .with_spec_section("IVF")
        .with_byte_offset(error.offset())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        /// Appends an `rg(n)` code for `value` (matching `BitReader::read_rg`).
        fn rg(&mut self, value: u32, n: u32) {
            let q = value >> n;
            let remainder = value & ((1 << n) - 1);
            for _ in 0..q {
                self.bit(1);
            }
            self.bit(0);
            self.f(remainder, n);
        }

        fn align(&mut self) {
            while !self.bits.len().is_multiple_of(8) {
                self.bit(0);
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

    fn annex_b_obu(header: u8, payload: &[u8]) -> Vec<u8> {
        annex_b_obu_with_header(&[header], payload)
    }

    fn annex_b_obu_with_header(header: &[u8], payload: &[u8]) -> Vec<u8> {
        let size = payload.len() + header.len();
        assert!(u8::try_from(size).is_ok());
        let mut data = Vec::with_capacity(payload.len() + header.len() + 1);
        data.push(size as u8);
        data.extend_from_slice(header);
        data.extend_from_slice(payload);
        data
    }

    fn ivf_stream(payloads: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        let header =
            splot_core::ivf::IvfHeader::new(*b"AV02", 16, 16, 24, 1, payloads.len() as u32);
        assert!(splot_core::ivf::write_ivf_header(&mut data, &header).is_ok());
        for (pts, payload) in payloads.iter().enumerate() {
            assert!(splot_core::ivf::write_ivf_frame(&mut data, pts as u64, payload).is_ok());
        }
        data
    }

    fn layer_obu_header(obu_type: u8, tlayer: u8, mlayer: u8, xlayer: u8) -> [u8; 2] {
        [
            0x80 | (obu_type << 2) | (tlayer & 0b11),
            ((mlayer & 0b111) << 5) | (xlayer & 0b1_1111),
        ]
    }

    fn ceil_log2_u32(value: u32) -> u32 {
        if value <= 1 {
            0
        } else {
            u32::BITS - (value - 1).leading_zeros()
        }
    }

    fn sequence_header_payload(max_tlayer_id: u32, max_mlayer_id: u32) -> Vec<u8> {
        sequence_header_payload_with_id(0, max_tlayer_id, max_mlayer_id)
    }

    fn sequence_header_payload_with_id(
        seq_header_id: u32,
        max_tlayer_id: u32,
        max_mlayer_id: u32,
    ) -> Vec<u8> {
        sequence_header_payload_with_lcr(seq_header_id, 0, max_tlayer_id, max_mlayer_id)
    }

    fn sequence_header_payload_with_lcr(
        seq_header_id: u32,
        seq_lcr_id: u32,
        max_tlayer_id: u32,
        max_mlayer_id: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(seq_lcr_id, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(max_tlayer_id, 2);
        bits.f(max_mlayer_id, 3);
        if max_mlayer_id > 0 {
            // SeqMaxMlayerCnt = max_mlayer_id + 1 allows every declared embedded layer
            // 0..=max_mlayer_id in the coded video sequence (AV2 § 6.4.1), so a fixture
            // that also uses embedded layer max_mlayer_id stays conformant.
            bits.f(max_mlayer_id, ceil_log2_u32(max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
        }
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        if max_mlayer_id > 0 {
            bits.bit(0); // mlayer_dependency_present_flag
        }
        if max_tlayer_id > 0 {
            bits.bit(0); // tlayer_dependency_present_flag
        }
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// Appends the §5.4 child configs for a non-single-picture, 4:2:0 (non-monochrome)
    /// sequence header with every tool flag cleared, plus the §5.2.1 payload tail
    /// (`obu_extension_flag = 0` + `trailing_bits`). This makes the validator's
    /// state-test payloads complete sequence headers that pass the full syntax check.
    fn append_non_single_child_configs(bits: &mut Bits) {
        // sequence_partition_config (BLOCK_64X64, SDP off)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg
        bits.bit(0); // seq_seg_info_present_flag
        // sequence_intra_config
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (non-single-picture branch)
        bits.f(0, 4); // seq_enabled_motion_modes[INTERINTRA..MOTION_MODES]
        bits.bit(0); // enable_masked_compound
        bits.bit(0); // enable_ref_frame_mvs
        bits.f(0, 4); // order_hint_bits_minus_1
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // explicit_ref_frame_map
        bits.bit(0); // explicit_num_ref_frames
        bits.f(0, 3); // long_term_frame_id_bits
        bits.f(0, 2); // seq_max_drl_bits_minus_1 = ns(5) -> 0
        bits.bit(0); // allow_frame_max_drl_bits
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.f(0, 2); // num_same_ref_compound
        bits.bit(0); // enable_tip
        bits.bit(0); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(0); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(0); // enable_df_sub_pu
        bits.f(0, 2); // enable_opfl_refine
        bits.bit(0); // enable_refinemv
        bits.bit(0); // enable_bru
        bits.bit(0); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(0); // enable_flex_mvres
        bits.bit(0); // enable_global_motion
        bits.bit(0); // enable_short_refresh_frame_flags
        // sequence_scc_config (non-single-picture branch)
        bits.bit(1); // seq_choose_screen_content_tools -> SELECT
        bits.bit(1); // seq_choose_integer_mv -> SELECT
        // sequence_transform_quant_entropy_config
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.bit(0); // cdef_on_skip_txfm_always_on
        bits.bit(0); // cdef_on_skip_txfm_disabled -> Adaptive
        bits.f(0, 2); // df_par_bits_minus_2
        // sequence_tile_config
        bits.bit(0); // seq_tile_info_present_flag
        // film_grain_params_present
        bits.bit(0);
        // open_bitstream_unit tail (extensible OBU)
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
    }

    fn sequence_header_payload_with_decoder_model_info() -> Vec<u8> {
        sequence_header_payload_with_decoder_model_sum(0, 0, 0)
    }

    /// A complete, activatable sequence header (`seq_header_id`, `max_tlayer_id == 1`,
    /// `max_mlayer_id == 1`) carrying explicit `seq_decoder_model_info()` (§ 5.4.13)
    /// with the given `decoder_buffer_delay` / `encoder_buffer_delay`.
    fn sequence_header_payload_with_decoder_model_sum(
        seq_header_id: u32,
        decoder_delay: u32,
        encoder_delay: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(1, 2); // max_tlayer_id
        bits.f(1, 3); // max_mlayer_id
        bits.f(1, 1); // seq_max_mlayer_cnt_minus_1 -> SeqMaxMlayerCnt = 2 (layers 0, 1)
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(1); // decoder_model_info_present_flag
        bits.f(1, 32); // num_units_in_decoding_tick
        bits.bit(1); // seq_decoder_model_info_present_flag
        // seq_decoder_model_info() (§ 5.4.13)
        bits.uvlc(decoder_delay); // decoder_buffer_delay
        bits.uvlc(encoder_delay); // encoder_buffer_delay
        bits.bit(0); // low_delay_mode_flag
        // dependency maps: max_mlayer_id = 1 -> mlayer_dependency_present_flag,
        // max_tlayer_id = 1 -> tlayer_dependency_present_flag
        bits.bit(0); // mlayer_dependency_present_flag
        bits.bit(0); // tlayer_dependency_present_flag
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    fn stream_with_sequence_header(max_tlayer_id: u32, max_mlayer_id: u32) -> Vec<u8> {
        annex_b_obu(0x04, &sequence_header_payload(max_tlayer_id, max_mlayer_id))
    }

    fn sequence_header_obu_for_xlayer(
        xlayer: u8,
        max_tlayer_id: u32,
        max_mlayer_id: u32,
    ) -> Vec<u8> {
        let payload = sequence_header_payload(max_tlayer_id, max_mlayer_id);
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    fn temporal_delimiter_obu() -> Vec<u8> {
        annex_b_obu(0x08, &[])
    }

    #[test]
    fn conformant_temporal_delimiter() {
        let report = Validator::new(false).validate_bytes(&[0x01, 0x08]);
        assert!(report.is_conformant());
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn conformant_temporal_delimiter_in_ivf_is_accepted() {
        let report = Validator::new(false).validate_bytes(&ivf_stream(&[&[0x01, 0x08]]));
        assert!(report.is_conformant(), "report was: {report}");
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn malformed_ivf_frame_payload_is_a_diagnostic() {
        let mut data = ivf_stream(&[&[0x01, 0x08]]);
        data.extend_from_slice(&5u32.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&[0x01, 0x08]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(!report.is_conformant());
        let diagnostic_offset = report
            .errors()
            .find(|d| d.rule_id == "ivf/truncated-frame-payload")
            .map(|d| d.byte_offset);
        assert_eq!(
            diagnostic_offset,
            Some(Some(splot_core::span::ByteOffset::new(data.len() as u64)))
        );
    }

    #[test]
    fn annex_b_parse_error_inside_ivf_frame_is_a_bitstream_diagnostic() {
        let report = Validator::new(false).validate_bytes(&ivf_stream(&[&[0x05, 0x08]]));
        assert!(!report.is_conformant());
        let diagnostic_offset = report
            .errors()
            .find(|d| d.rule_id == "bitstream/parse-error")
            .map(|d| d.byte_offset);
        assert_eq!(
            diagnostic_offset,
            Some(Some(splot_core::span::ByteOffset::new(45)))
        );
    }

    #[test]
    fn temporal_delimiter_without_global_xlayer_is_flagged() {
        // size=2, header 0x88 0x05: TemporalDelimiter with extension, xlayer=5 (not global).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05]);
        assert!(!report.is_conformant());
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-required")
        );
    }

    #[test]
    fn parse_error_becomes_a_single_error_diagnostic() {
        let report = Validator::new(false).validate_bytes(&[0x00]);
        assert!(!report.is_conformant());
        assert_eq!(report.errors().count(), 1);
        assert!(report.diagnostics[0].byte_offset.is_some());
    }

    #[test]
    fn report_display_reports_status() {
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05]);
        assert!(report.to_string().contains("ERROR"));
    }

    #[test]
    fn diagnostics_from_prefix_survive_a_later_parse_error() {
        // OBU #0: TemporalDelimiter with extension, xlayer=5 (a §6.2.2 violation).
        // OBU #1: truncated (declares 5 bytes, only 1 present).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05, 0x05, 0x08]);
        assert!(!report.is_conformant());
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-required"),
            "expected the conformance error from the parseable prefix"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "expected the parse error for the truncated tail"
        );
    }

    #[test]
    fn reserved_obu_with_all_zero_payload_is_an_error() {
        // size=2: reserved header 0x00 (obu_type=0) + an all-zero payload byte.
        let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0x00]);
        assert!(!report.is_conformant());
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-reserved/all-zero-payload"),
            "an all-zero reserved OBU payload must be an error (AV2 § 5.3)"
        );
    }

    #[test]
    fn reserved_obu_with_nonzero_payload_is_conformant() {
        // size=2: reserved header 0x00 + non-zero payload. Reserved OBUs have no
        // defined payload syntax, so this is retained and ignored.
        let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0x40]);
        assert!(report.is_conformant());
    }

    #[test]
    fn reserved_obu_with_nonzero_trailing_bits_shape_is_conformant() {
        // size=2: reserved header 0x00 + non-zero payload that is not a valid
        // trailing_bits pattern. Reserved OBUs are ignored except for the
        // all-zero-payload guard.
        let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0xC0]);
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn temporal_delimiter_payload_trailing_bits_are_validated() {
        // size=2: temporal delimiter header 0x08 + valid full-byte trailing_bits.
        let valid = Validator::new(false).validate_bytes(&[0x02, 0x08, 0x80]);
        assert!(valid.is_conformant(), "report was: {valid}");

        // Same OBU with missing trailing_one_bit.
        let invalid = Validator::new(false).validate_bytes(&[0x02, 0x08, 0x00]);
        assert!(
            invalid
                .errors()
                .any(|d| d.rule_id == "trailing-bits/missing-one-bit"),
            "report was: {invalid}"
        );
    }

    #[test]
    fn sequence_header_payload_syntax_is_validated() {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(4); // invalid chroma_format_idc

        let data = annex_b_obu(0x04, &bits.into_bytes());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/chroma-format-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn sequence_header_payload_eof_is_reported() {
        let report = Validator::new(false).validate_bytes(&[0x01, 0x04]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "report was: {report}"
        );
    }

    #[test]
    fn global_xlayer_requires_base_layers_is_flagged() {
        // size=2, header 0xA0 0x3F: OBU_METADATA_SHORT, ext, mlayer=1, xlayer=31
        // (global). A global xlayer requires base mlayer/tlayer (§ 6.2.2).
        let report = Validator::new(false).validate_bytes(&[0x02, 0xA0, 0x3F]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-requires-base-layers"),
            "report was: {report}"
        );
    }

    #[test]
    fn global_xlayer_on_disallowed_type_is_flagged() {
        // size=2, header 0x84 0x1F: OBU_SEQUENCE_HEADER, ext, mlayer=0, xlayer=31.
        // The sequence header may not carry the global xlayer (§ 6.2.2).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x84, 0x1F]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-allowed-types"),
            "report was: {report}"
        );
    }

    #[test]
    fn base_layer_only_type_with_nonzero_layer_is_flagged() {
        // size=2, header 0x85 0x00: OBU_SEQUENCE_HEADER, ext, tlayer=1, mlayer=0,
        // xlayer=0. The sequence header must be base tlayer/mlayer (§ 6.2.2).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x85, 0x00]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/base-layer-only-types"),
            "report was: {report}"
        );
    }

    #[test]
    fn temporal_layer_zero_only_type_is_flagged() {
        // size=1, header 0x11: OBU_CLOSED_LOOP_KEY, no extension, tlayer=1. Closed/
        // open-loop key, switch, and RAS frames must use tlayer 0 (§ 6.2.2).
        let report = Validator::new(false).validate_bytes(&[0x01, 0x11]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/temporal-layer-zero-only-types"),
            "report was: {report}"
        );
    }

    #[test]
    fn reserved_obu_type_emits_info_and_stays_conformant() {
        // size=2, header 0x68 (reserved obu_type 26) + non-zero payload 0x80.
        // Reserved types are ignored by decoders: informational, not an error.
        let report = Validator::new(false).validate_bytes(&[0x02, 0x68, 0x80]);
        assert!(report.is_conformant(), "report was: {report}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "obu-header/reserved-obu-type"),
            "report was: {report}"
        );
    }

    #[test]
    fn active_sequence_header_allows_following_obu_within_layer_limits() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-state/")),
            "report was: {report}"
        );
    }

    #[test]
    fn sequence_header_with_decoder_model_info_tail_can_activate() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_info(),
        ));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-state/")),
            "report was: {report}"
        );
    }

    #[test]
    fn layer_obu_before_sequence_header_reports_missing_active_sequence() {
        let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn repeated_sequence_header_does_not_replace_active_limits_without_reference() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-state/")),
            "report was: {report}"
        );
    }

    #[test]
    fn local_prefix_hls_before_sequence_header_does_not_require_active_sequence() {
        for obu_type in [16, 17, 18] {
            let mut data = temporal_delimiter_obu();
            data.extend(annex_b_obu_with_header(
                &layer_obu_header(obu_type, 0, 0, 0),
                &[],
            ));
            data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
            data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

            let report = Validator::new(false).validate_bytes(&data);
            assert!(
                !report
                    .errors()
                    .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
                "obu_type={obu_type}, report was: {report}"
            );
        }
    }

    #[test]
    fn non_activating_sequence_header_does_not_suppress_missing_active_sequence_error() {
        // 0x05 = OBU_SEQUENCE_HEADER at tlayer=1, so it parses but cannot activate.
        let mut data = annex_b_obu(0x05, &sequence_header_payload(1, 0));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn temporal_unit_accepts_ascending_coded_xlayers() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 1), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("obu-order/")),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn global_hls_in_prefix_phase_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(16, 0, 0, 31),
            &[],
        ));
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("obu-order/")),
            "report was: {report}"
        );
    }

    #[test]
    fn temporal_unit_missing_delimiter_is_reported() {
        let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/temporal-unit-missing-delimiter"),
            "report was: {report}"
        );
    }

    #[test]
    fn global_hls_after_coded_layer_is_reported() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(16, 0, 0, 31),
            &[],
        ));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/global-hls-after-coded-layer"),
            "report was: {report}"
        );
    }

    #[test]
    fn coded_xlayers_must_ascend_within_temporal_unit() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 1), &[]));
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/xlayer-order-not-ascending"),
            "report was: {report}"
        );
    }

    #[test]
    fn non_global_padding_outside_coded_layer_is_reported() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(&layer_obu_header(25, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/padding-non-global-outside-coded-layer"),
            "report was: {report}"
        );
    }

    #[test]
    fn active_sequence_header_bounds_temporal_layer_id() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn active_sequence_header_bounds_embedded_layer_id() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 2, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn stateful_diagnostics_from_prefix_survive_a_later_parse_error() {
        let mut data = stream_with_sequence_header(0, 0);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 0, 0), &[]));
        data.extend_from_slice(&[0x05, 0x08]);

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "report was: {report}"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "report was: {report}"
        );
    }

    fn msdo_syntax_bits(num_streams_minus_2: u32) -> Bits {
        let mut bits = Bits::default();
        bits.f(num_streams_minus_2, 3); // num_streams_minus_2
        bits.f(0, 5); // multistream_profile_idc
        bits.f(0, 5); // multistream_level_idx
        bits.bit(0); // multistream_tier
        bits.bit(1); // multistream_even_allocation_flag
        for _ in 0..(num_streams_minus_2 + 2) {
            bits.f(0, 5); // sub_xlayer_id
            bits.f(0, 5); // sub_stream_max_profile
            bits.f(0, 5); // sub_stream_max_level
            bits.bit(0); // sub_stream_max_tier
        }
        bits.bit(0); // multistream_doh_constraint_flag
        bits
    }

    fn msdo_payload(num_streams_minus_2: u32) -> Vec<u8> {
        let mut bits = msdo_syntax_bits(num_streams_minus_2);
        bits.bit(1); // trailing_one_bit (valid trailing_bits)
        bits.into_bytes()
    }

    #[test]
    fn hls_duplicate_temporal_delimiter_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/duplicate-temporal-delimiter"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_repeated_identical_sequence_header_is_accepted() {
        let mut data = temporal_delimiter_obu();
        let payload = sequence_header_payload_with_id(0, 0, 0);
        data.extend(annex_b_obu(0x04, &payload));
        data.extend(annex_b_obu(0x04, &payload));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_repeated_non_identical_sequence_header_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_msdo_non_global_layer_id_is_flagged() {
        // OBU_MSDO (type 20) with an extension header at xlayer 5 (not global).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(20, 0, 0, 5),
            &msdo_payload(0),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/non-global-layer-id"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_msdo_too_many_streams_is_flagged() {
        // Global OBU_MSDO (0x50 infers GLOBAL_XLAYER_ID) with num_streams_minus_2 = 3.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x50, &msdo_payload(3)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/too-many-streams"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/non-global-layer-id"),
            "global MSDO must not be flagged for layer ids; report was: {report}"
        );
    }

    #[test]
    fn hls_msdo_malformed_trailing_bits_is_flagged() {
        // Valid MSDO syntax followed by a non-zero trailing bit after the
        // trailing_one_bit (AV2 § 5.2.1: MSDO is non-extensible -> trailing_bits).
        let mut bits = msdo_syntax_bits(0);
        bits.bit(1); // trailing_one_bit
        bits.bit(1); // trailing_zero_bit must be 0 -> violation
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x50, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "trailing-bits/zero-bit-not-zero"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_well_formed_msdo_has_no_trailing_bits_error() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x50, &msdo_payload(0)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("trailing-bits/")),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_repeated_sequence_header_across_temporal_units_without_clk_is_flagged() {
        // Temporal unit 1: seq header (id 0, params A), then an OBU_CLOSED_LOOP_KEY
        // for xlayer 0 (0x10 = type 4, no extension) with an empty payload — the raw
        // OBU header alone is the § 7.3.6 boundary event, so a new coded video
        // sequence starts at temporal unit 1 and the same-unit params-A header joins
        // it. Temporal unit 2 reuses seq_header_id 0 with different params B but
        // contains NO CLK, so it continues that SAME coded video sequence (AV2
        // § 7.3.6: "A new coded video sequence for an extended layer is defined to
        // start at each temporal unit that contains an OBU with obu_type equal to
        // OBU_CLOSED_LOOP_KEY in the coded extended layer unit corresponding to the
        // extended layer"), and the non-bit-identical repeat is a true violation
        // ("the contents must be bit-identical each time the activated sequence
        // header appears"). The former temporal-unit-reset approximation documented
        // this exact case as a false negative; the exact CVS model flags it via the
        // end-of-stream flush (a CLK later in temporal unit 2 could still have
        // started a new coded video sequence, so the comparison is deferred).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(annex_b_obu(0x10, &[]));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a non-identical repeat in the same coded video sequence (no CLK in \
             temporal unit 2) must be flagged at the end-of-stream flush; report \
             was: {report}"
        );
    }

    #[test]
    fn hls_cross_temporal_unit_repeat_is_flushed_at_next_temporal_delimiter() {
        // No CLK anywhere: per AV2 § 7.3.6 temporal units 1 and 2 belong to one
        // coded video sequence for xlayer 0, so the params-B repeat in temporal
        // unit 2 violates the bit-identity rule. The comparison is deferred while
        // temporal unit 2 is open and flushed by the temporal delimiter that starts
        // temporal unit 3 (not by the end-of-stream flush).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| d.rule_id == "hls/repeated-sequence-header-not-identical")
                .count(),
            1,
            "exactly the A-vs-B repeat must be flagged (the identical params-B \
             repeat in temporal unit 3 must not be); report was: {report}"
        );
    }

    #[test]
    fn hls_clk_for_other_xlayer_does_not_end_coded_video_sequence() {
        // AV2 § 7.3.6 defines coded video sequences per extended layer ("for an
        // extended layer ... in the coded extended layer unit corresponding to the
        // extended layer"): a CLK for xlayer 1 in temporal unit 2 must not reset
        // xlayer 0's CVS-scoped fingerprints, so xlayer 0's cross-temporal-unit
        // non-identical repeat stays flagged. A CLK for xlayer 0 itself drops the
        // deferred comparison (the repeat joins xlayer 0's new coded video
        // sequence).
        fn stream(clk_xlayer: u8) -> Vec<u8> {
            let mut data = temporal_delimiter_obu();
            data.extend(sequence_header_obu_for_xlayer(0, 1, 1)); // id 0, params A
            data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
            data.extend(temporal_delimiter_obu());
            data.extend(sequence_header_obu_for_xlayer(0, 0, 0)); // id 0, params B
            // OBU_CLOSED_LOOP_KEY (type 4) with an extension header at clk_xlayer;
            // the empty payload does not matter — § 7.3.6 is an OBU-header-level
            // boundary event.
            data.extend(annex_b_obu_with_header(
                &layer_obu_header(4, 0, 0, clk_xlayer),
                &[],
            ));
            data
        }

        let other_layer = Validator::new(false).validate_bytes(&stream(1));
        assert!(
            other_layer
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a CLK for xlayer 1 must not scope away xlayer 0's repeat; report was: \
             {other_layer}"
        );

        let same_layer = Validator::new(false).validate_bytes(&stream(0));
        assert!(
            !same_layer
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a CLK for xlayer 0 starts a new coded video sequence at temporal unit 2, \
             so the params-B header joins it; report was: {same_layer}"
        );
    }

    #[test]
    fn sequence_header_truncated_child_config_is_flagged() {
        // General fields parse, but the payload ends inside sequence_partition_config.
        // The full sequence-header check now reports this (the general-only check missed it).
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
        bits.bit(0); // seq_cropping_window_present_flag (no child config follows)
        let data = annex_b_obu(0x04, &bits.into_bytes());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "report was: {report}"
        );
    }

    /// Builds a single-picture `sequence_header_obu()` payload (16x8, BLOCK_64X64,
    /// level 0) with optional segment info and tile config, plus the §5.2.1 payload
    /// tail. Mirrors the splot-core still-picture parser field-for-field.
    fn single_picture_seq_header_payload(
        seg_present: bool,
        tile_present: bool,
        uniform: bool,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        // general (single picture, chroma 4:2:0, 16x8, level 0)
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx (single picture -> no seq_tier)
        bits.uvlc(0); // chroma_format_idc = 420
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1 -> 16
        bits.f(7, 4); // max_frame_height_minus_1 -> 8
        bits.bit(0); // seq_cropping_window_present_flag
        // sequence_partition_config
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock -> BLOCK_64X64
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(u8::from(seg_present)); // seq_seg_info_present_flag
        if seg_present {
            bits.bit(0); // seq_allow_seg_info_change
            for _ in 0..(8 * 3) {
                bits.bit(0); // seg_info(8): all features disabled
            }
        }
        // sequence_intra_config
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (single-picture branch)
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        // sequence_scc_config (single picture -> no signalled bits)
        // sequence_transform_quant_entropy_config
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (BLOCK_64X64, single picture)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
        // sequence_tile_config
        bits.bit(u8::from(tile_present)); // seq_tile_info_present_flag
        if tile_present {
            bits.bit(0); // allow_tile_info_change
            // tile_params(16, 8, BLOCK_64X64, ...): single tile, only uniform flag.
            bits.bit(u8::from(uniform)); // uniform_tile_spacing_flag
        }
        // film_grain_params_present
        bits.bit(0);
        // §5.2.1 tail (extensible OBU)
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        bits.into_bytes()
    }

    #[test]
    fn sequence_header_with_uniform_tile_config_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &single_picture_seq_header_payload(false, true, true),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("tile-params/")),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn sequence_header_with_nonuniform_tile_config_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &single_picture_seq_header_payload(false, true, false),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("tile-params/")),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn sequence_header_with_segment_info_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &single_picture_seq_header_payload(true, false, false),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn sequence_header_malformed_tail_after_segment_info_is_flagged() {
        // seg_info() now parses fully, so the §5.2.1 payload tail is validated. An extra
        // non-zero byte after the tail is a trailing-bits violation the previously
        // bounded parse missed.
        let mut payload = single_picture_seq_header_payload(true, false, false);
        payload.push(0xFF);
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &payload));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id.starts_with("trailing-bits/")),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_mfh_nonzero_obu_extension_flag_is_flagged() {
        // A fully parsed MFH (no seg_info) is extensible, so a set obu_extension_flag
        // after the syntax violates AV2 §6.2.1.
        let mut bits = Bits::default();
        bits.uvlc(0); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
        bits.bit(1); // obu_extension_flag = 1 -> §6.2.1 violation
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x0C, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/extension-flag-not-zero"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_mfh_out_of_range_ids_are_flagged() {
        // OBU_MULTI_FRAME_HEADER (type 3 -> 0x0C) with out-of-range ids.
        let mut bits = Bits::default();
        bits.uvlc(16); // mfh_seq_header_id (>= MAX_SEQ_NUM)
        bits.uvlc(16); // mfh_id_minus_1 -> mfhId = 17 (>= MAX_MFH_NUM)
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x0C, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "mfh/seq-header-id-out-of-range"),
            "report was: {report}"
        );
        assert!(
            report.errors().any(|d| d.rule_id == "mfh/id-out-of-range"),
            "report was: {report}"
        );
    }

    #[derive(Clone, Copy)]
    struct CiTiming {
        display_tick: u32,
        time_scale: u32,
        equal_picture_interval: bool,
        num_ticks_minus_1: u32,
    }

    /// Builds an `OBU_CONTENT_INTERPRETATION` (type 24) at obu_xlayer_id 0 /
    /// obu_mlayer_id `mlayer`, with all optional branches cleared except the
    /// requested timing, plus the §5.2.1 extensible payload tail.
    fn content_interpretation_obu(
        mlayer: u8,
        reserved_2bit: u32,
        timing: Option<CiTiming>,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(u8::from(timing.is_some())); // ci_timing_info_present_flag
        bits.f(reserved_2bit, 2); // ci_reserved_2bit
        if let Some(t) = timing {
            bits.f(t.display_tick, 32);
            bits.f(t.time_scale, 32);
            bits.bit(u8::from(t.equal_picture_interval));
            if t.equal_picture_interval {
                bits.uvlc(t.num_ticks_minus_1);
            }
        }
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, mlayer, 0), &bits.into_bytes())
    }

    /// Temporal delimiter + an activating sequence header for xlayer 0 that allows
    /// embedded layers 0 and 1, then two content-interpretation OBUs at embedded
    /// layers 0 and 1.
    fn stream_with_two_ci_layers(a: Option<CiTiming>, b: Option<CiTiming>) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, a));
        data.extend(content_interpretation_obu(1, 0, b));
        data
    }

    const BASE_TIMING: CiTiming = CiTiming {
        display_tick: 1000,
        time_scale: 30000,
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };

    #[test]
    fn ci_matching_timing_across_embedded_layers_is_accepted() {
        let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(BASE_TIMING));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-header/timing-")),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn ci_different_display_tick_across_embedded_layers_is_flagged() {
        let other = CiTiming {
            display_tick: 2000,
            ..BASE_TIMING
        };
        let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/timing-display-tick-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_different_time_scale_across_embedded_layers_is_flagged() {
        let other = CiTiming {
            time_scale: 60000,
            ..BASE_TIMING
        };
        let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/timing-time-scale-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_different_equal_picture_interval_across_embedded_layers_is_flagged() {
        let other = CiTiming {
            equal_picture_interval: false,
            ..BASE_TIMING
        };
        let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/timing-equal-picture-interval-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_different_num_ticks_across_embedded_layers_is_flagged() {
        let other = CiTiming {
            num_ticks_minus_1: 4,
            ..BASE_TIMING
        };
        let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/timing-num-ticks-mismatch"),
            "report was: {report}"
        );
    }

    /// Two temporal units: a CI with `BASE_TIMING` at embedded layer 0 in temporal
    /// unit 1, then a CI with a differing `time_scale` at embedded layer 1 in
    /// temporal unit 2 (same extended layer 0, no CLK).
    fn stream_with_timing_mismatch_across_temporal_units() -> Vec<u8> {
        let other = CiTiming {
            time_scale: 60000,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(temporal_delimiter_obu());
        data.extend(content_interpretation_obu(1, 0, Some(other)));
        data
    }

    #[test]
    fn ci_timing_mismatch_across_temporal_units_without_clk_is_flagged() {
        // The § 6.4.12 cross-embedded-layer timing comparison is tagged with the
        // BASELINE record's temporal unit: with no CLK, temporal unit 2 continues
        // xlayer 0's coded video sequence (AV2 § 7.3.6), so the deferred
        // comparison must be emitted by the end-of-stream flush.
        let data = stream_with_timing_mismatch_across_temporal_units();
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-header/timing-time-scale-mismatch"),
            "a cross-temporal-unit timing mismatch without a CLK stays in the same \
             coded video sequence and must be flagged; report was: {report}"
        );
    }

    #[test]
    fn ci_timing_mismatch_in_clk_temporal_unit_is_not_flagged() {
        // Same stream plus a CLK for xlayer 0 in temporal unit 2: per AV2 § 7.3.6
        // the new coded video sequence starts at the temporal unit, so the
        // embedded-layer-1 timing belongs to the NEW coded video sequence and the
        // deferred comparison against the old sequence's baseline is dropped (no
        // false positive at the exact CVS boundary).
        let mut data = stream_with_timing_mismatch_across_temporal_units();
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-header/timing-")),
            "a CLK in the differing CI's temporal unit starts a new coded video \
             sequence that the CI joins; report was: {report}"
        );
    }

    #[test]
    fn ci_repeated_for_same_embedded_layer_with_different_payload_is_flagged() {
        let other = CiTiming {
            time_scale: 24000,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(content_interpretation_obu(0, 0, Some(other)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeated_identical_for_same_embedded_layer_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeated_non_identical_across_temporal_units_without_clk_is_flagged() {
        // Temporal unit 2 has no CLK, so per AV2 § 7.3.6 it continues xlayer 0's
        // coded video sequence from temporal unit 1: a repeated CI OBU for the same
        // embedded layer with different information is a § 6.14 violation. The
        // comparison is deferred (a CLK later in temporal unit 2 could still have
        // started a new coded video sequence) and emitted by the end-of-stream
        // flush.
        let other = CiTiming {
            time_scale: 24000,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(temporal_delimiter_obu());
        data.extend(content_interpretation_obu(0, 0, Some(other)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "a cross-temporal-unit repeat without a CLK stays in the same coded \
             video sequence and must be flagged; report was: {report}"
        );
    }

    #[test]
    fn ci_repeated_non_identical_in_clk_temporal_unit_is_not_flagged() {
        // Same stream, but temporal unit 2 contains a CLK for xlayer 0 after the
        // repeated CI OBU: per AV2 § 7.3.6 the new coded video sequence starts at
        // the temporal unit, so the differing CI joins the NEW coded video sequence
        // and the deferred cross-temporal-unit comparison is dropped (no false
        // positive).
        let other = CiTiming {
            time_scale: 24000,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(temporal_delimiter_obu());
        data.extend(content_interpretation_obu(0, 0, Some(other)));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "a CLK in the repeat's temporal unit starts a new coded video sequence \
             that the repeat joins; report was: {report}"
        );
    }

    #[test]
    fn ci_reserved_bits_nonzero_is_warned() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0b10, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .any(|d| d.rule_id == "content-interpretation/reserved-bits-nonzero"),
            "report was: {report}"
        );
        // A reserved-bits anomaly is a warning, not a conformance error.
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn ci_repeat_differing_only_in_reserved_bits_is_not_flagged() {
        // AV2 § 6.14: ci_reserved_2bit is decoder-ignored, so two CI OBUs for the
        // same embedded layer that differ only in the reserved bits carry the same
        // information and must not be flagged as a non-identical repeat.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(content_interpretation_obu(0, 0b11, Some(BASE_TIMING)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    /// Content-interpretation OBU (xlayer 0 / mlayer 0) carrying a chroma sample
    /// position (interlace scan type, so top and bottom are coded independently).
    fn content_interpretation_chroma_obu(top: u32, bottom: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(2, 2); // ci_scan_type_idc = 2 (interlace) -> bottom coded
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(1); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        bits.uvlc(top);
        bits.uvlc(bottom);
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
    }

    /// Content-interpretation OBU (xlayer 0 / mlayer 0) carrying an aspect-ratio idc.
    fn content_interpretation_aspect_obu(idc: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(1); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        bits.f(idc, 8); // ci_aspect_ratio_idc (!= 255 -> no extended SAR)
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
    }

    #[test]
    fn ci_chroma_sample_position_out_of_range_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_chroma_obu(6, 0)); // top = 6 > 5
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/chroma-sample-position-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_chroma_sample_position_in_range_is_accepted() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_chroma_obu(5, 0)); // both <= 5
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/chroma-sample-position-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_aspect_ratio_idc_out_of_range_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_aspect_obu(17)); // 16 < 17 < 255
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/aspect-ratio-idc-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_aspect_ratio_idc_extended_marker_is_accepted() {
        // ci_aspect_ratio_idc == 255 is the extended-SAR marker, not out of range.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(0); // color description absent
        bits.bit(0); // chroma sample position absent
        bits.bit(1); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // timing absent
        bits.f(0, 2); // ci_reserved_2bit
        bits.f(255, 8); // ci_aspect_ratio_idc = 255 -> extended SAR
        bits.uvlc(16); // ci_sar_width
        bits.uvlc(9); // ci_sar_height
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(24, 0, 0, 0),
            &bits.into_bytes(),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/aspect-ratio-idc-out-of-range"),
            "report was: {report}"
        );
    }

    /// CI OBU (xlayer 0 / mlayer 0) carrying a color description with the given
    /// `ci_color_description_idc` (idc < 4, so the `rg(2)` prefix is a single zero
    /// bit). When idc == 0 an explicit BT.709 triple is coded.
    fn content_interpretation_color_obu(color_idc: u32) -> Vec<u8> {
        // This helper encodes rg(2) with a single terminating zero bit (q == 0), so
        // it is only correct for idc < 4. Use content_interpretation_color_custom_obu
        // for larger ids (it emits the full rg(2) unary prefix).
        assert!(
            color_idc < 4,
            "content_interpretation_color_obu only encodes idc < 4; use content_interpretation_color_custom_obu"
        );
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(1); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        bits.bit(0); // rg(2): q = 0 (terminating zero bit)
        bits.f(color_idc, 2); // rg(2): 2-bit remainder == idc for idc < 4
        if color_idc == 0 {
            bits.f(1, 8); // ci_color_primaries (BT.709)
            bits.f(1, 8); // ci_transfer_characteristics
            bits.f(1, 8); // ci_matrix_coefficients
        }
        bits.bit(0); // ci_full_range_flag
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
    }

    #[test]
    fn ci_repeat_differing_only_in_color_encoding_is_not_flagged() {
        // AV2 § 6.14: color descriptions can encode the same information in multiple
        // ways (a Table 6.13 preset idc vs. the equivalent explicit triple). The
        // repeated-CI check compares *derived* values, so an alias-equivalent
        // re-encoding is not flagged (it must never false-positive a conformant
        // stream): BT.709 as preset idc 1 and as explicit (1, 1, 1) carry the same
        // information.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_color_obu(1)); // BT.709 preset
        data.extend(content_interpretation_color_obu(0)); // explicit BT.709 triple
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    /// Multi-frame header OBU (type 3) at xlayer 0 referencing `seq_header_id`.
    fn multi_frame_header_obu(seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        annex_b_obu(0x0C, &bits.into_bytes())
    }

    /// Temporal delimiter + an activating sequence header with `seq_header_id` for
    /// xlayer 0, then a multi-frame header referencing `mfh_seq_header_id`.
    fn stream_with_mfh_reference(seq_header_id: u32, mfh_seq_header_id: u32) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_id(seq_header_id, 1, 1),
        ));
        data.extend(multi_frame_header_obu(mfh_seq_header_id));
        data
    }

    #[test]
    fn mfh_referencing_available_sequence_header_is_accepted() {
        let data = stream_with_mfh_reference(0, 0);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_referencing_missing_sequence_header_is_flagged() {
        // Only seq_header_id 0 is in-band; the MFH references 5. Default options do
        // not assume any external HLS, so this is unavailable.
        let data = stream_with_mfh_reference(0, 5);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_unavailable_under_default_options_emits_external_hls_disabled_advisory() {
        let data = stream_with_mfh_reference(0, 5);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .any(|d| d.rule_id == "hls/external-hls-disabled"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_reference_satisfied_by_external_hls_is_accepted() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let data = stream_with_mfh_reference(0, 5);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
        assert!(
            !report
                .warnings()
                .any(|d| d.rule_id == "hls/external-hls-disabled"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_reference_not_in_external_hls_set_is_flagged_without_advisory() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // External HLS is Provided but does not declare id 5, so the reference is
        // genuinely unavailable: the error fires, but the external-hls-disabled
        // advisory must be suppressed (external HLS is not disabled).
        let data = stream_with_mfh_reference(0, 5);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(99),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
        assert!(
            !report
                .warnings()
                .any(|d| d.rule_id == "hls/external-hls-disabled"),
            "advisory must be suppressed when external HLS is Provided; report was: {report}"
        );
    }

    #[test]
    fn external_hls_suppresses_no_active_sequence_header() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // A multi-frame header at xlayer 0 with no in-band sequence header at all.
        let mut data = temporal_delimiter_obu();
        data.extend(multi_frame_header_obu(5));

        // Default (external disabled): no in-band active sequence -> flagged.
        let default_report = Validator::new(false).validate_bytes(&data);
        assert!(
            default_report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {default_report}"
        );

        // External HLS provided: an external sequence header may be the active one,
        // so the missing-in-band-sequence error is suppressed (the validator must not
        // reject a conformant external-HLS stream), and the referenced id 5 resolves
        // externally.
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn external_hls_empty_set_does_not_suppress_no_active_sequence_header() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // An empty external set declares no sequence header that could be active, so
        // the missing-active-header error must NOT be suppressed (otherwise an empty
        // set would silently accept a malformed stream).
        let mut data = temporal_delimiter_obu();
        data.extend(multi_frame_header_obu(5));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn external_hls_suppresses_active_sequence_layer_limits() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // In-band active sequence (id 0) for xlayer 0 allows only embedded layer 0,
        // then a coded OBU at embedded layer 1.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 1, 0), &[]));

        // Default: the OBU exceeds the in-band active sequence's mlayer limit.
        let default_report = Validator::new(false).validate_bytes(&data);
        assert!(
            default_report
                .errors()
                .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
            "report was: {default_report}"
        );

        // External HLS declaring a sequence header: a different external sequence may
        // be active with limits this validator does not model, so the in-band
        // layer-limit check is suppressed (sound: never reject a conformant
        // external-HLS stream).
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_reference_to_malformed_tail_sequence_header_is_unavailable() {
        // A sequence header whose body parses but whose §5.2.1 payload tail is
        // malformed (an extra non-zero trailing byte) is not a valid available HLS
        // object, so it is not recorded — a later MFH referencing it is unavailable.
        let mut data = temporal_delimiter_obu();
        // A well-formed activating sequence header (id 0) so the MFH has an active
        // sequence for its xlayer.
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        // Sequence header id 7 with a malformed tail (trailing zero bit not zero).
        let mut malformed = sequence_header_payload_with_id(7, 0, 0);
        malformed.push(0xFF);
        data.extend(annex_b_obu(0x04, &malformed));
        // Multi-frame header referencing id 7.
        data.extend(multi_frame_header_obu(7));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn mfh_reference_to_malformed_layer_sequence_header_is_unavailable() {
        // A sequence header with a §6.2.2 layer violation (tlayer != 0, 0x05) is
        // malformed and is NOT a valid available HLS object, so an MFH referencing
        // only that copy of id 4 is unavailable (§7.3.8.6).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x05, &sequence_header_payload_with_id(4, 1, 1)));
        // An activating base-layer header (id 0) so the MFH has an active sequence for
        // its xlayer; it does not make id 4 available.
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(multi_frame_header_obu(4));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn external_hls_out_of_range_id_does_not_suppress_no_active_sequence_header() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // Declaring an out-of-range external id (16 >= MAX_SEQ_NUM) cannot make a
        // valid sequence header available, so it must not suppress the missing-active
        // error.
        let mut data = temporal_delimiter_obu();
        data.extend(multi_frame_header_obu(5));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(16),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_differing_in_color_preset_is_flagged() {
        // Genuinely different color information (BT.709 vs BT.2100 PQ) is a §6.14
        // violation and must be flagged even though both are preset encodings.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_color_obu(1)); // BT.709 SDR
        data.extend(content_interpretation_color_obu(2)); // BT.2100 PQ
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_differing_in_aspect_preset_is_flagged() {
        // Different aspect ratios (1:1 vs 12:11) are different information (§6.14).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
        data.extend(content_interpretation_aspect_obu(2)); // SAR 12:11
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    /// CI OBU (xlayer 0 / mlayer 0) carrying an explicit sample aspect ratio
    /// (`ci_aspect_ratio_idc == 255`).
    fn content_interpretation_extended_sar_obu(sar_width: u32, sar_height: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(1); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        bits.f(255, 8); // ci_aspect_ratio_idc = 255 -> extended SAR
        bits.uvlc(sar_width);
        bits.uvlc(sar_height);
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
    }

    #[test]
    fn ci_repeat_alias_equivalent_aspect_is_not_flagged() {
        // Aspect preset idc 1 derives to SAR 1:1, the same as the explicit 255-coded
        // SAR 1:1, so the alias-equivalent re-encoding must not be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_aspect_obu(1)); // preset SAR 1:1
        data.extend(content_interpretation_extended_sar_obu(1, 1)); // explicit SAR 1:1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_unreduced_explicit_sar_is_not_flagged() {
        // SAR 2:2 reduces to 1:1, the same ratio as the preset idc 1, so the
        // unreduced explicit re-encoding must not be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_aspect_obu(1)); // preset SAR 1:1
        data.extend(content_interpretation_extended_sar_obu(2, 2)); // explicit SAR 2:2 == 1:1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_present_color_difference_after_absent_baseline_is_flagged() {
        // An absent-color first CI must not hide a genuine difference between two
        // later PRESENT color descriptions (absent -> BT.709 -> BT.2100). The
        // baseline records the first present color so the present-vs-present
        // difference is detected.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(0, 0, None)); // color absent
        data.extend(content_interpretation_color_obu(1)); // BT.709
        data.extend(content_interpretation_color_obu(2)); // BT.2100 PQ
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_present_aspect_difference_after_absent_baseline_is_flagged() {
        // Same as above for aspect ratio: absent -> SAR 1:1 -> SAR 12:11.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(0, 0, None)); // aspect absent
        data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
        data.extend(content_interpretation_aspect_obu(2)); // SAR 12:11
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    /// CI OBU (xlayer 0 / mlayer 0) carrying a color description with an arbitrary
    /// `ci_color_description_idc` (properly `rg(2)`-encoded), the explicit triple when
    /// `idc == 0`, and the given full-range flag.
    fn content_interpretation_color_custom_obu(
        color_idc: u32,
        triple: Option<(u8, u8, u8)>,
        full_range: bool,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(1); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        // rg(2): (idc >> 2) one bits, a terminating zero, then the 2-bit remainder.
        for _ in 0..(color_idc >> 2) {
            bits.bit(1);
        }
        bits.bit(0);
        bits.f(color_idc & 0b11, 2);
        if color_idc == 0 {
            let (cp, tc, mc) = triple.unwrap_or((1, 1, 1));
            bits.f(u32::from(cp), 8);
            bits.f(u32::from(tc), 8);
            bits.f(u32::from(mc), 8);
        }
        bits.bit(u8::from(full_range)); // ci_full_range_flag
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
    }

    #[test]
    fn ci_repeat_reserved_color_vs_explicit_unspecified_is_not_flagged() {
        // A reserved color id (6) is decoder-ignored -> unspecified (2, 2, 2), the
        // same derived color as an explicit (2, 2, 2) with the same full-range flag,
        // so the repeat must not be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_color_custom_obu(6, None, false)); // reserved
        data.extend(content_interpretation_color_custom_obu(
            0,
            Some((2, 2, 2)),
            false,
        )); // explicit unspecified
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_present_color_vs_absent_default_is_flagged() {
        // An absent color description defaults to unspecified (2, 2, 2); a present
        // BT.709 carries different information, so the repeat is flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(0, 0, None)); // color absent
        data.extend(content_interpretation_color_obu(1)); // BT.709
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_present_aspect_vs_absent_default_is_flagged() {
        // An absent aspect ratio defaults to unspecified (0, 0); a present SAR 1:1
        // carries different information, so the repeat is flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(0, 0, None)); // aspect absent
        data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_repeat_both_absent_color_and_aspect_is_not_flagged() {
        // Two CIs that both omit color and aspect carry the same (unspecified)
        // information and must not be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(0, 0, None));
        data.extend(content_interpretation_obu(0, 0, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn ci_zero_display_tick_is_reported_under_timing_namespace() {
        // A timing-range violation carried by a content-interpretation OBU is
        // reported under the §6.4.12 timing namespace (sequence-header/timing-*).
        // §6.4.12 "Timing info semantics" is a subsection of §6.4 "Sequence header
        // OBU semantics", so the namespace follows the spec's section hierarchy and
        // is consistent with the cross-layer timing-mismatch diagnostics; the
        // diagnostic's spec_section (6.4.12) and byte offset locate it precisely.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(content_interpretation_obu(
            0,
            0,
            Some(CiTiming {
                display_tick: 0, // num_units_in_display_tick == 0 -> §6.4.12 violation
                time_scale: 30000,
                equal_picture_interval: false,
                num_ticks_minus_1: 0,
            }),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-header/timing-display-tick-zero"
                    && d.spec_section.as_deref() == Some("6.4.12")
            }),
            "report was: {report}"
        );
    }

    // --- Frame-header prefix activation / HLS reference checks ---

    /// A frame-bearing OBU (`header` byte) whose first tile group carries a frame
    /// header with `cur_mfh_id == 0` and the given `seq_header_id_in_frame_header`.
    fn frame_obu_direct_seq_ref(header: u8, seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
        bits.uvlc(0); // cur_mfh_id == 0 -> direct sequence-header reference
        bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
        annex_b_obu(header, &bits.into_bytes())
    }

    /// A frame-bearing OBU whose first tile group carries a frame header with
    /// `cur_mfh_id` greater than 0 (the sequence header resolves through the MFH).
    fn frame_obu_mfh_ref(header: u8, cur_mfh_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group
        bits.uvlc(cur_mfh_id); // cur_mfh_id > 0
        annex_b_obu(header, &bits.into_bytes())
    }

    /// Temporal delimiter + an activating sequence header (id `seq_id`) for xlayer 0.
    fn td_and_seq_header(seq_id: u32, max_tlayer_id: u32, max_mlayer_id: u32) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_id(seq_id, max_tlayer_id, max_mlayer_id),
        ));
        data
    }

    // 0x10 = OBU_CLOSED_LOOP_KEY (type 4), no extension, tlayer 0.
    const CLK_HEADER: u8 = 0x10;

    #[test]
    fn hls_frame_header_missing_sequence_header_is_flagged() {
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // references missing id 5
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_frame_header_sequence_header_available_inband_is_accepted() {
        let mut data = td_and_seq_header(3, 1, 1);
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // references available id 3
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_frame_header_sequence_header_available_external_is_accepted() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // No in-band sequence header with id 5; external HLS supplies it.
        let mut data = temporal_delimiter_obu();
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_frame_header_missing_mfh_is_flagged() {
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // references missing MFH id 2
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn hls_frame_header_mfh_available_is_accepted() {
        // TD, seq(0), MFH (mfh_seq_header_id 0, mfhId 1), CLK with cur_mfh_id 1.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(multi_frame_header_obu(0)); // mfh_seq_header_id 0 -> mfhId 1
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // resolves MFH 1 -> seq 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn frame_header_seq_header_id_out_of_range_is_not_double_reported() {
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 16)); // == MAX_SEQ_NUM -> out of range
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/seq-header-id-out-of-range"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "an out-of-range id must not also report unavailable; report was: {report}"
        );
    }

    #[test]
    fn frame_header_cur_mfh_id_out_of_range_is_not_double_reported() {
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 16)); // == MAX_MFH_NUM -> out of range
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/cur-mfh-id-out-of-range"),
            "report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
            "an out-of-range cur_mfh_id must not also report unavailable; report was: {report}"
        );
    }

    #[test]
    fn sequence_activation_uses_clk_referenced_sequence_header() {
        // Two available sequence headers with different layer limits: id 0 allows
        // tlayer up to 2, id 1 allows only tlayer 0. A CLK that references id 1
        // activates it for xlayer 0, so a following tlayer-1 OBU exceeds the limit.
        // Without frame-header activation, id 0 (the OBU-order fallback) would be
        // active and the tlayer-1 OBU would be accepted.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 2, 2)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // activate id 1
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "the CLK-referenced sequence header (id 1) must bound the tlayer; report was: {report}"
        );
    }

    #[test]
    fn sequence_fingerprint_preserved_for_in_cvs_repeat() {
        // A sequence header opens a CVS, a CLK references (activates) it, then a
        // non-identical repeat of the same id appears later in the SAME temporal
        // unit. Per AV2 § 7.3.6 the new coded video sequence starts at the temporal
        // unit, so the pre-CLK header joins it: its fingerprint survives the
        // activating CLK and the same-temporal-unit repeat is flagged eagerly.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // activates id 0
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "report was: {report}"
        );
    }

    #[test]
    fn sequence_reconfiguration_in_clk_temporal_unit_is_not_flagged() {
        // Temporal unit 2 reconfigures id 0 with different layer limits and contains
        // a CLK *after* the header: AV2 § 7.3.6 defines the new coded video sequence
        // to start at the temporal unit ("A new coded video sequence ... is defined
        // to start at each temporal unit that contains an OBU with obu_type equal to
        // OBU_CLOSED_LOOP_KEY ..."), so the pre-CLK params-B header joins the NEW
        // coded video sequence and is never in the same sequence as params A. The
        // deferred cross-temporal-unit comparison enqueued when params B was
        // observed is dropped when the CLK arrives later in the same temporal unit
        // (no false positive).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a reconfiguration in a CLK temporal unit must not be flagged; report was: {report}"
        );
    }

    #[test]
    fn first_picture_in_tu_is_tracked_per_extended_layer() {
        // AV2 § 6.17.2: "FirstPictureInTU is a variable that specifies if this is
        // the first frame unit in a coded extended layer unit in a temporal unit" —
        // i.e. per extended layer. A frame-bearing OBU in xlayer 0 must not clear
        // xlayer 1's FirstPictureInTU (so a CLK for xlayer 1 later in the same
        // temporal unit still derives startCVS, AV2 § 5.18.2), and the next global
        // temporal delimiter resets the per-temporal-unit state. The derivation is
        // not observable through diagnostics yet (startCVS gates no implemented
        // check), so this drives the context directly.
        use splot_core::annexb::parse_annex_b_obus;
        use splot_core::types::ExtendedLayerId;

        // TD; leading tile group (type 6) in xlayer 0; CLK (type 4) in xlayer 1; TD.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 1), &[]));
        data.extend(temporal_delimiter_obu());

        let obus = parse_annex_b_obus(&data).unwrap_or_default();
        assert_eq!(obus.len(), 4, "the test stream must parse into 4 OBUs");

        let options = ValidationOptions::default();
        let mut report = ValidationReport::new();
        let mut context = ValidatorContext::default();
        let x0 = ExtendedLayerId::from_bits(0);
        let x1 = ExtendedLayerId::from_bits(1);

        context.observe_obu(&obus[0], &options, &mut report); // temporal delimiter
        assert!(context.first_picture_in_tu(x0));
        assert!(context.first_picture_in_tu(x1));

        context.observe_obu(&obus[1], &options, &mut report); // frame in xlayer 0
        assert!(!context.first_picture_in_tu(x0));
        assert!(
            context.first_picture_in_tu(x1),
            "a frame in xlayer 0 must not clear xlayer 1's FirstPictureInTU"
        );

        context.observe_obu(&obus[2], &options, &mut report); // CLK in xlayer 1
        assert!(!context.first_picture_in_tu(x1));

        context.observe_obu(&obus[3], &options, &mut report); // next temporal unit
        assert!(context.first_picture_in_tu(x0));
        assert!(context.first_picture_in_tu(x1));
    }

    /// A frame-bearing OBU (with an extension header at the given layer ids) whose
    /// first tile group carries a frame header referencing `seq_header_id`.
    fn frame_obu_direct_seq_ref_layer(
        obu_type: u8,
        tlayer: u8,
        mlayer: u8,
        xlayer: u8,
        seq_header_id: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
        annex_b_obu_with_header(
            &layer_obu_header(obu_type, tlayer, mlayer, xlayer),
            &bits.into_bytes(),
        )
    }

    /// A multi-frame header OBU with in-range ids but a malformed §5.2.1 payload tail
    /// (`obu_extension_flag == 1`), so it is not a valid available HLS object.
    fn malformed_tail_mfh_obu(mfh_id_minus_1: u32, seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id); // mfh_seq_header_id
        bits.uvlc(mfh_id_minus_1); // mfh_id_minus_1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
        bits.bit(1); // obu_extension_flag = 1 -> §6.2.1 tail violation
        annex_b_obu(0x0C, &bits.into_bytes())
    }

    #[test]
    fn frame_header_activation_precedes_layer_limit_check() {
        // A CLK requires obu_tlayer_id == 0 but may carry a non-zero obu_mlayer_id
        // (AV2 §6.2.2). seq 0 allows only mlayer 0; seq 1 allows mlayer 1. A CLK at
        // obu_mlayer_id 1 that references seq 1 activates the permissive header BEFORE
        // its own layer-limit check, so it is not flagged. (Without activating first,
        // the stale seq-0 fallback would falsely flag mlayer-exceeds-max.)
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 1)));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 1)); // CLK, mlayer 1, ref seq 1

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
            "the CLK must activate seq 1 (allows mlayer 1) before its own limit check; \
             report was: {report}"
        );
    }

    #[test]
    fn frame_header_referencing_malformed_tail_mfh_is_unavailable() {
        // An MFH with in-range ids but a malformed §5.2.1 payload tail is not recorded
        // as available, so a frame referencing it via cur_mfh_id is unavailable rather
        // than resolved through the malformed HLS object.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(malformed_tail_mfh_obu(1, 0)); // mfhId 2, malformed tail
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // CLK cur_mfh_id 2
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn frame_header_missing_mfh_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // With external HLS provided, an out-of-band multi-frame header may satisfy the
        // cur_mfh_id reference. External MFHs are not modeled, so the validator neither
        // resolves the MFH nor emits a hard error — it must not reject the conformant
        // external-HLS stream.
        let mut data = temporal_delimiter_obu();
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // CLK cur_mfh_id 2, no in-band MFH
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
            "external HLS may supply the MFH; report was: {report}"
        );
    }

    #[test]
    fn frame_header_activation_applies_to_non_key_frames() {
        // AV2 §5.18.2 calls load_sequence_header() for every frame, before the
        // `if (keyFrame)` block — not just CLK/OLK key frames. seq 0 allows only
        // tlayer 0; seq 1 allows tlayer 1. A non-key OBU_REGULAR_TILE_GROUP at tlayer 1
        // that references seq 1 activates it, so it is checked against seq 1 (allows
        // tlayer 1) rather than the stale seq-0 fallback — no false positive.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 0)));
        // OBU_REGULAR_TILE_GROUP (type 7), tlayer 1, mlayer 0, xlayer 0, references seq 1.
        data.extend(frame_obu_direct_seq_ref_layer(7, 1, 0, 0, 1));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "a non-key frame must activate its referenced (permissive) seq header; \
             report was: {report}"
        );
    }

    // --- Frame-header core diagnostics (AV2 § 6.17.2 / § 6.17.4 / § 6.4.6) --------

    // OBU header bytes: obu_type << 2 (no extension, tlayer/mlayer 0). 0x4C = bridge
    // frame (type 19), 0x54 = RAS frame (type 21).
    const BRIDGE_HEADER: u8 = 0x4C;
    const RAS_HEADER: u8 = 0x54;

    /// Tunable knobs for [`frame_core_seq_payload`]; defaults from [`FrameCoreSeq::base`].
    #[derive(Clone, Copy)]
    struct FrameCoreSeq {
        seq_id: u32,
        frame_width_bits_minus_1: u32,
        frame_height_bits_minus_1: u32,
        max_frame_width_minus_1: u32,
        max_frame_height_minus_1: u32,
        order_hint_bits_minus_1: u32,
        num_ref_frames_minus_1: u32,
        long_term_frame_id_bits: u32,
        still_picture: bool,
        enable_short_refresh_frame_flags: bool,
    }

    impl FrameCoreSeq {
        /// seq 0; 8-bit frame dimensions, 16x16 maximum; OrderHintBits = 1,
        /// NumRefFrames = 8; no long-term ids; not still-picture; full refresh signaling.
        fn base() -> Self {
            Self {
                seq_id: 0,
                frame_width_bits_minus_1: 7,
                frame_height_bits_minus_1: 7,
                max_frame_width_minus_1: 15,
                max_frame_height_minus_1: 15,
                order_hint_bits_minus_1: 0,
                num_ref_frames_minus_1: 7,
                long_term_frame_id_bits: 0,
                still_picture: false,
                enable_short_refresh_frame_flags: false,
            }
        }
    }

    /// A fully-parseable §5.4 sequence header (xlayer 0, max_tlayer/mlayer 0,
    /// monotonic output) with a tunable inter config and frame dimensions, for
    /// exercising the frame-header core diagnostics.
    fn frame_core_seq_payload(o: FrameCoreSeq) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(o.seq_id);
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(u8::from(o.still_picture)); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id == 0
        bits.bit(1); // monotonic_output_order_flag
        bits.f(o.frame_width_bits_minus_1, 4);
        bits.f(o.frame_height_bits_minus_1, 4);
        // max_frame_*_minus_1 are read as f(frame_*_bits_minus_1 + 1).
        bits.f(o.max_frame_width_minus_1, o.frame_width_bits_minus_1 + 1);
        bits.f(o.max_frame_height_minus_1, o.frame_height_bits_minus_1 + 1);
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        // sequence_partition_config (BLOCK_64X64, SDP off)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg
        bits.bit(0); // seq_seg_info_present_flag
        // sequence_intra_config
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (non-single-picture branch)
        bits.f(0, 4); // seq_enabled_motion_modes
        bits.bit(0); // enable_masked_compound
        bits.bit(0); // enable_ref_frame_mvs
        bits.f(o.order_hint_bits_minus_1, 4); // order_hint_bits_minus_1
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder
        bits.bit(0); // explicit_ref_frame_map
        bits.bit(1); // explicit_num_ref_frames
        bits.f(o.num_ref_frames_minus_1, 4); // num_ref_frames_minus_1
        bits.f(o.long_term_frame_id_bits, 3); // long_term_frame_id_bits
        bits.f(0, 2); // seq_max_drl_bits_minus_1 (ns(5) -> 0)
        bits.bit(0); // allow_frame_max_drl_bits
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 (ns(3) -> 0)
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.f(0, 2); // num_same_ref_compound
        bits.bit(0); // enable_tip
        bits.bit(0); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(0); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(0); // enable_df_sub_pu
        bits.f(0, 2); // enable_opfl_refine
        bits.bit(0); // enable_refinemv
        bits.bit(0); // enable_bru
        bits.bit(0); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(0); // enable_flex_mvres
        bits.bit(0); // enable_global_motion
        bits.bit(u8::from(o.enable_short_refresh_frame_flags)); // enable_short_refresh_frame_flags
        // sequence_scc_config (SELECT both)
        bits.bit(1); // seq_choose_screen_content_tools
        bits.bit(1); // seq_choose_integer_mv
        // sequence_transform_quant_entropy_config
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.bit(0); // cdef_on_skip_txfm_always_on
        bits.bit(0); // cdef_on_skip_txfm_disabled
        bits.f(0, 2); // df_par_bits_minus_2
        // sequence_tile_config
        bits.bit(0); // seq_tile_info_present_flag
        bits.bit(0); // film_grain_params_present
        extensible_obu_tail(&mut bits);
        bits.into_bytes()
    }

    /// A temporal delimiter followed by a `frame_core_seq_payload` sequence header.
    fn td_and_frame_core_seq(o: FrameCoreSeq) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &frame_core_seq_payload(o)));
        data
    }

    /// Appends the § 5.18.2 intra structure cluster the core parser consumes after
    /// `disable_cdf_update` for a [`frame_core_seq_payload`] sequence (10-bit,
    /// 4:2:0, BLOCK_64X64, no sequence tile/segmentation info, every optional
    /// quantizer read disabled): a single-tile `tile_info()` (§ 5.18.7.2;
    /// `uniform_tile_spacing_flag` plus `col_increment_bits` zero increment bits —
    /// one for the 256-wide frame, none for the 16x16 default), `base_q_idx` f(9)
    /// (§ 5.18.6.1), `segmentation_enabled = 0` (§ 5.18.7.1), `using_qmatrix = 0`
    /// (§ 5.18.6.2), and `delta_q_present = 0` (§ 5.18.7.8). With a nonzero
    /// `base_q_idx` the § 5.18.2 lossless tail reads no further bits.
    fn intra_structure_tail(fb: &mut Bits, col_increment_bits: u32) {
        fb.bit(1); // uniform_tile_spacing_flag (tile_info)
        for _ in 0..col_increment_bits {
            fb.bit(0); // increment_tile_cols_log2 = 0
        }
        fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
        fb.bit(0); // segmentation_enabled
        fb.bit(0); // using_qmatrix
        fb.bit(0); // delta_q_present
    }

    #[test]
    fn validator_flags_ras_requires_long_term_frame_id_bits() {
        // The default sequence has long_term_frame_id_bits == 0, so a RAS frame
        // referencing it violates AV2 § 6.4.6.
        let mut data = td_and_seq_header(0, 0, 0);
        data.extend(frame_obu_direct_seq_ref(RAS_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/ras-requires-long-term-frame-id-bits"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_bridge_ref_index_out_of_range() {
        // NumRefFrames == 6 -> CeilLog2(6) == 3 bits, so bridge_frame_ref_idx can encode
        // 6 or 7, both >= NumRefFrames (AV2 § 6.17.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            num_ref_frames_minus_1: 5, // NumRefFrames == 6 (non-power-of-2)
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
        fb.f(6, 3); // bridge_frame_ref_idx == 6 (>= NumRefFrames 6)
        data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/bridge-ref-index-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_frame_size_exceeds_sequence_max() {
        // frame_width_bits == 8 (FrameWidth up to 256) but max_frame_width == 16; an
        // override frame size of 256 exceeds the sequence maximum (AV2 § 6.17.4.1).
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
        fb.bit(1); // frame_size_override_flag
        fb.f(0, 1); // order_hint f(OrderHintBits == 1)
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        fb.f(256 - 1, 8); // frame_width_minus_1 -> FrameWidth 256 (> max 16)
        fb.f(8 - 1, 8); // frame_height_minus_1 -> FrameHeight 8
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        // 256-wide frame: sbCols == 4, so tile_info() reads one column increment bit.
        intra_structure_tail(&mut fb, 1);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_frame_size_within_sequence_max() {
        // The same frame with FrameWidth 16 == max must not be flagged.
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // immediate_output_frame
        fb.bit(1); // frame_size_override_flag
        fb.f(0, 1); // order_hint
        fb.f(16 - 1, 8); // frame_width_minus_1 -> FrameWidth 16 (== max)
        fb.f(16 - 1, 8); // frame_height_minus_1 -> FrameHeight 16 (== max)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
            "report was: {report}"
        );
    }

    // OBU header bytes: obu_type << 2. 0x14 = OBU_OPEN_LOOP_KEY (5), 0x1c =
    // OBU_REGULAR_TILE_GROUP (7).
    const OLK_HEADER: u8 = 0x14;
    const RTG_HEADER: u8 = 0x1c;

    #[test]
    fn validator_flags_frame_to_refresh_out_of_range() {
        // Compact refresh with NumRefFrames == 6: frame_to_refresh == 6 (>= 6) yields
        // refresh_frame_flags == 1 << 6, a slot at/beyond NumRefFrames (AV2 § 6.17.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            num_ref_frames_minus_1: 5, // NumRefFrames == 6
            enable_short_refresh_frame_flags: true,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
        fb.bit(0); // immediate_output_frame
        fb.bit(0); // frame_size_override_flag (default dims)
        fb.f(0, 1); // order_hint
        fb.bit(1); // has_refresh_frame_flags
        fb.f(6, 3); // frame_to_refresh == 6 (CeilLog2(6) == 3) -> refresh = 1 << 6
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_reserved_ref_long_term_id() {
        // RAS with long_term_frame_id_bits == 4 and a ref_long_term_id of 15 == the
        // reserved (1 << 4) - 1 (AV2 § 6.17.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            long_term_frame_id_bits: 4,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // restricted_prediction_switch
        fb.f(1, 3); // num_key_ref_frames == 1
        fb.f(15, 4); // ref_long_term_id[0] == (1 << 4) - 1 (reserved)
        data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_zero_refresh_on_deferred_output() {
        // OLK forces immediate_output_frame == 0; refresh_frame_flags == 0 then violates
        // AV2 § 6.17.2 (a deferred-output frame must update a reference slot).
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        // OLK: long_term_id_plus_1 f(0) (no bits); immediate forced 0; implicit -> 0
        fb.bit(0); // frame_size_override_flag (default dims)
        fb.f(0, 1); // order_hint
        fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) == 0
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_still_picture_non_key_frame() {
        // still_picture == 1 requires a KEY_FRAME; an INTRA_ONLY frame violates
        // AV2 § 6.17.2.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            still_picture: true,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY (not KEY_FRAME)
        fb.bit(1); // immediate_output_frame == 1 (isolate the frame-type violation)
        fb.bit(0); // frame_size_override_flag (default dims)
        fb.f(0, 1); // order_hint
        fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero, not all-slots)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_intra_only_refresh_all_slots() {
        // INTRA_ONLY with NumRefFrames == 2 must not refresh every slot
        // (refresh_frame_flags != (1 << 2) - 1) (AV2 § 6.17.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            num_ref_frames_minus_1: 1, // NumRefFrames == 2
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
        fb.bit(1); // immediate_output_frame == 1
        fb.bit(0); // frame_size_override_flag (default dims)
        fb.f(0, 1); // order_hint
        fb.f(0b11, 2); // refresh_frame_flags f(NumRefFrames == 2) == all slots
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_in_range_frame_to_refresh() {
        // The same compact-refresh frame with frame_to_refresh == 5 (< NumRefFrames 6)
        // must not be flagged.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            num_ref_frames_minus_1: 5,
            enable_short_refresh_frame_flags: true,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
        fb.bit(0); // immediate_output_frame
        fb.bit(0); // frame_size_override_flag
        fb.f(0, 1); // order_hint
        fb.bit(1); // has_refresh_frame_flags
        fb.f(5, 3); // frame_to_refresh == 5 (< NumRefFrames 6)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_non_reserved_ref_long_term_id() {
        // ref_long_term_id == 14 != the reserved (1 << 4) - 1 == 15 must not be flagged.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            long_term_frame_id_bits: 4,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // restricted_prediction_switch
        fb.f(1, 3); // num_key_ref_frames == 1
        fb.f(14, 4); // ref_long_term_id[0] == 14 (not reserved)
        data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_nonzero_refresh_on_deferred_output() {
        // An OLK frame (immediate_output_frame == 0) with refresh_frame_flags != 0 is
        // conformant.
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_size_override_flag
        fb.f(0, 1); // order_hint
        fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_still_picture_key_frame() {
        // A still_picture sequence with a KEY_FRAME (CLK) and immediate_output_frame == 1
        // is conformant.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            still_picture: true,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(1); // immediate_output_frame == 1
        fb.bit(0); // frame_size_override_flag
        fb.f(0, 1); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_intra_only_partial_refresh() {
        // An INTRA_ONLY frame whose refresh_frame_flags is not all slots is conformant.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            num_ref_frames_minus_1: 1, // NumRefFrames == 2
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
        fb.bit(1); // immediate_output_frame == 1
        fb.bit(0); // frame_size_override_flag
        fb.f(0, 1); // order_hint
        fb.f(0b01, 2); // refresh_frame_flags == 1 (not all slots)
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        intra_structure_tail(&mut fb, 0);
        data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
            "report was: {report}"
        );
    }

    // --- Frame tile-info / QM-reference diagnostics (AV2 § 6.17.7.2 / § 6.17.6.2) ---

    /// Appends a CLK frame-header bit fixture from the activation prefix through
    /// `disable_cdf_update`, with `frame_size_override_flag == 1` and the given
    /// dimensions, leaving `fb` positioned at `tile_info()` (AV2 § 5.18.2). The
    /// dimension fields are `f(width_bits)` / `f(height_bits)`
    /// (`frame_*_bits_minus_1 + 1` from the sequence).
    fn clk_frame_until_tile_info(fb: &mut Bits, width: u32, height: u32, bits: (u32, u32)) {
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(0); // seq_header_id_in_frame_header
        fb.bit(0); // immediate_output_frame
        fb.bit(1); // frame_size_override_flag
        fb.f(0, 1); // order_hint f(OrderHintBits == 1)
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        fb.f(width - 1, bits.0); // frame_width_minus_1
        fb.f(height - 1, bits.1); // frame_height_minus_1
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
    }

    /// Appends the post-`tile_info()` § 5.18.2 structures with every optional read
    /// disabled: `base_q_idx` f(9) (§ 5.18.6.1), `segmentation_enabled = 0`
    /// (§ 5.18.7.1), `using_qmatrix = 0` (§ 5.18.6.2), `delta_q_present = 0`
    /// (§ 5.18.7.8); the lossless tail then reads no bits.
    fn quant_seg_tail(fb: &mut Bits) {
        fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
        fb.bit(0); // segmentation_enabled
        fb.bit(0); // using_qmatrix
        fb.bit(0); // delta_q_present
    }

    /// Encodes `ns(n)` value `0` (AV2 § 4.11.6): `w = FloorLog2(n) + 1`,
    /// `m = (1 << w) - n`; `0 < m` always holds, so the encoding is `f(0, w - 1)`.
    fn ns_zero(fb: &mut Bits, n: u32) {
        let w = 32 - n.leading_zeros();
        fb.f(0, w - 1);
    }

    #[test]
    fn validator_flags_frame_tile_cols_out_of_range() {
        // A 4160x16 frame (BLOCK_64X64, level 0 Main: maxTileWidthSb == 64) has
        // sbCols == 65; a non-uniform tile_params() coding 65 one-superblock columns
        // derives TileCols == 65 > MAX_TILE_COLS (AV2 § 6.17.7.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            frame_width_bits_minus_1: 12, // frame_width_minus_1 f(13)
            max_frame_width_minus_1: 4159,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        clk_frame_until_tile_info(&mut fb, 4160, 16, (13, 8));
        fb.bit(0); // uniform_tile_spacing_flag = 0 (tile_params, § 5.18.7.3)
        for start in 0..65u32 {
            // width_in_sbs_minus_1 ns(Min(sbCols - startSb, maxTileWidthSb)) == 0.
            ns_zero(&mut fb, (65 - start).min(64));
        }
        ns_zero(&mut fb, 1); // height_in_sbs_minus_1 (sbRows == 1, 0 bits)
        fb.f(0, 7); // context_update_tile_id f(TileRowsLog2 0 + TileColsLog2 7)
        fb.f(0, 2); // tile_size_bytes_minus_1
        quant_seg_tail(&mut fb);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "frame-header/tile-cols-out-of-range"
                    && d.spec_section.as_deref() == Some("6.17.7.2")
            }),
            "report was: {report}"
        );
        // context_update_tile_id 0 < 65 stays valid.
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/context-update-tile-id-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_flags_frame_tile_rows_out_of_range() {
        // The transposed layout: a 16x4160 frame has sbRows == 65; a non-uniform
        // tile_params() coding 65 one-superblock rows derives TileRows == 65 >
        // MAX_TILE_ROWS (AV2 § 6.17.7.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            frame_height_bits_minus_1: 12, // frame_height_minus_1 f(13)
            max_frame_height_minus_1: 4159,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        clk_frame_until_tile_info(&mut fb, 16, 4160, (8, 13));
        fb.bit(0); // uniform_tile_spacing_flag = 0
        ns_zero(&mut fb, 1); // width_in_sbs_minus_1 (sbCols == 1, 0 bits)
        for start in 0..65u32 {
            // height_in_sbs_minus_1 ns(Min(sbRows - startSb, maxTileHeightSb == 65)).
            ns_zero(&mut fb, 65 - start);
        }
        fb.f(0, 7); // context_update_tile_id f(TileRowsLog2 7 + TileColsLog2 0)
        fb.f(0, 2); // tile_size_bytes_minus_1
        quant_seg_tail(&mut fb);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "frame-header/tile-rows-out-of-range"
                    && d.spec_section.as_deref() == Some("6.17.7.2")
            }),
            "report was: {report}"
        );
    }

    /// Appends the uniform 3x1 tile_info() bits for a 160x16 frame (sbCols == 3:
    /// increments 1,1 reach maxLog2TileCols == 2, TileCols == 3, TileColsLog2 == 2),
    /// with the given `context_update_tile_id` f(2) (AV2 § 5.18.7.2 / § 5.18.7.3).
    fn uniform_3x1_tile_info(fb: &mut Bits, context_update_tile_id: u32) {
        fb.bit(1); // uniform_tile_spacing_flag
        fb.bit(1); // increment_tile_cols_log2 = 1
        fb.bit(1); // increment_tile_cols_log2 = 1 (reaches maxLog2TileCols)
        fb.f(context_update_tile_id, 2); // f(TileRowsLog2 0 + TileColsLog2 2)
        fb.f(0, 2); // tile_size_bytes_minus_1
    }

    #[test]
    fn validator_flags_frame_context_update_tile_id_out_of_range() {
        // A 160x16 frame splits into TileCols == 3 (not a power of two), so the
        // f(2) context_update_tile_id read can encode 3 >= TileCols * TileRows == 3
        // (AV2 § 6.17.7.2).
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            max_frame_width_minus_1: 159,
            ..FrameCoreSeq::base()
        });
        // The diagnostic is located at the frame OBU header, one Annex B leb128
        // size-prefix byte past the end of the preceding OBUs.
        let frame_obu_offset = data.len() as u64 + 1;
        let mut fb = Bits::default();
        clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
        uniform_3x1_tile_info(&mut fb, 3); // context_update_tile_id == 3 (>= 3 * 1)
        quant_seg_tail(&mut fb);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "frame-header/context-update-tile-id-out-of-range"
                    && d.spec_section.as_deref() == Some("6.17.7.2")
                    && d.byte_offset.map(|offset| offset.get()) == Some(frame_obu_offset)
            }),
            "expected the § 6.17.7.2 diagnostic at the frame OBU offset \
             {frame_obu_offset}; report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_conforming_frame_tile_layout() {
        // The same 3x1 layout with context_update_tile_id == 2 (< 3) is conformant:
        // none of the § 6.17.7.2 frame tile diagnostics fire.
        let mut data = td_and_frame_core_seq(FrameCoreSeq {
            max_frame_width_minus_1: 159,
            ..FrameCoreSeq::base()
        });
        let mut fb = Bits::default();
        clk_frame_until_tile_info(&mut fb, 160, 16, (8, 8));
        uniform_3x1_tile_info(&mut fb, 2); // context_update_tile_id == 2 (< 3 * 1)
        quant_seg_tail(&mut fb);
        data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        for rule in [
            "frame-header/tile-cols-out-of-range",
            "frame-header/tile-rows-out-of-range",
            "frame-header/context-update-tile-id-out-of-range",
        ] {
            assert!(
                !report.errors().any(|d| d.rule_id == rule),
                "{rule} must not fire on a conforming layout; report was: {report}"
            );
        }
    }

    /// Appends a 16x16 CLK frame whose `setup_qm_params()` (§ 5.18.6.2) references
    /// `qm_y[0] == level` with `qm_uv_same_as_y == 1` (so `qm_u[0]` / `qm_v[0]`
    /// reference the same slot for the 4:2:0 sequence).
    fn clk_frame_with_qm_reference(level: u32) -> Vec<u8> {
        let mut fb = Bits::default();
        clk_frame_until_tile_info(&mut fb, 16, 16, (8, 8));
        fb.bit(1); // uniform_tile_spacing_flag (sbCols == 1: no increments)
        fb.f(100, 9); // base_q_idx f(9) (§ 5.18.6.1)
        fb.bit(0); // segmentation_enabled (§ 5.18.7.1)
        fb.bit(1); // using_qmatrix (§ 5.18.6.2)
        fb.f(level, 4); // qm_y[0]
        fb.bit(1); // qm_uv_same_as_y (NumPlanes == 3)
        fb.bit(0); // delta_q_present (§ 5.18.7.8)
        annex_b_obu(CLK_HEADER, &fb.into_bytes())
    }

    /// A `quantizer_matrix_obu()` selecting a single default `level` with
    /// `qm_chroma_info_present_flag == 1` (`QmNumPlanes == 3`, AV2 § 5.13).
    fn qm_default_level_obu_chroma(level: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(1 << level, 15); // qm_bit_map: set `level`
        bits.bit(1); // qm_chroma_info_present_flag = 1 -> 3 planes
        bits.bit(1); // qm_is_default_flag for `level`
        bits.bit(1); // trailing_one_bit (QM is non-extensible)
        bits.align();
        annex_b_obu(QM_HEADER, &bits.into_bytes())
    }

    #[test]
    fn validator_flags_frame_qm_plane_count_mismatch() {
        // A QM OBU defines custom level 0 with QmNumPlanes == 1
        // (qm_chroma_info_present_flag == 0); the 4:2:0 sequence has NumPlanes == 3,
        // so a frame referencing level 0 violates AV2 § 6.17.6.2.
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        data.extend(qm_default_level_obu(0));
        data.extend(clk_frame_with_qm_reference(0));
        let report = Validator::new(false).validate_bytes(&data);
        let matches: Vec<_> = report
            .errors()
            .filter(|d| d.rule_id == "frame-header/qm-plane-count-mismatch")
            .collect();
        assert!(
            matches
                .iter()
                .any(|d| d.spec_section.as_deref() == Some("6.17.6.2")),
            "report was: {report}"
        );
        // qm_y/qm_u/qm_v all reference the same slot: one diagnostic, not three.
        assert_eq!(matches.len(), 1, "report was: {report}");
    }

    #[test]
    fn validator_is_silent_on_frame_qm_reference_without_qm_state() {
        // No quantizer matrix OBU defines level 0: the § 6.17.6.2 plane-count check
        // has no recorded QmNumPlanes to compare and must stay silent (the HLS
        // availability checks own the missing-state case).
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        data.extend(clk_frame_with_qm_reference(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
            "missing QM state must not produce a false positive; report was: {report}"
        );
    }

    #[test]
    fn validator_accepts_frame_qm_reference_with_matching_planes() {
        // A chroma QM OBU records QmNumPlanes == 3 for level 0, matching the 4:2:0
        // sequence's NumPlanes == 3: conformant per AV2 § 6.17.6.2.
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        data.extend(qm_default_level_obu_chroma(0));
        data.extend(clk_frame_with_qm_reference(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_is_silent_on_default_matrix_qm_reference() {
        // qm_y == 15 == NUM_CUSTOM_QMS selects the default matrix, not a custom
        // slot: the § 6.17.6.2 plane-count requirement does not apply, even with
        // mismatched recorded state for other levels.
        let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
        data.extend(qm_default_level_obu(0)); // 1-plane state for (unreferenced) level 0
        data.extend(clk_frame_with_qm_reference(15));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "frame-header/qm-plane-count-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn validator_preserves_existing_unavailable_sequence_header_check() {
        // The frame-header core wiring must not suppress the activation/HLS checks: a
        // frame referencing an unavailable sequence header still reports it.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // id 5 is unavailable
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
    }

    // --- HLS LCR / atlas availability (AV2 § 7.3.8.3 / § 7.3.8.4 / § 6.4.1) -------

    /// Appends the §5.2.1 extensible-OBU payload tail (`obu_extension_flag = 0` +
    /// `trailing_one_bit`); `into_bytes` zero-pads the remainder.
    fn extensible_obu_tail(bits: &mut Bits) {
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
    }

    /// A minimal global LCR OBU (`obu_xlayer_id == GLOBAL_XLAYER_ID == 31`).
    fn global_lcr_obu(global_id: u32, xlayer_map: u32, atlas_id: Option<u32>) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(xlayer_map, 31); // lcr_xlayer_map
        bits.bit(0); // lcr_aggregate_info_present_flag
        bits.bit(0); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(u8::from(atlas_id.is_some())); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(atlas_id.unwrap_or(0), 3); // lcr_global_atlas_id or reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A global LCR OBU whose `lcr_global_reserved_zero_5bits` is non-zero.
    fn global_lcr_obu_with_nonzero_reserved() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(1, 3); // lcr_global_config_record_id
        bits.f(0b1, 31); // lcr_xlayer_map
        bits.bit(0); // aggregate
        bits.bit(0); // ptl
        bits.bit(0); // payload
        bits.bit(0); // dependent
        bits.bit(0); // atlas present
        bits.f(0, 7); // purpose
        bits.bit(0); // doh
        bits.bit(0); // tile alignment
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0b1_0001, 5); // lcr_global_reserved_zero_5bits != 0
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A minimal local LCR OBU at `xlayer`.
    fn local_lcr_obu(
        xlayer: u8,
        global_id: u32,
        local_id: u32,
        local_atlas_id: Option<u32>,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_id
        bits.f(local_id, 3); // lcr_local_id
        bits.bit(0); // lcr_profile_tier_level_info_present_flag
        bits.bit(u8::from(local_atlas_id.is_some())); // lcr_local_atlas_id_present_flag
        bits.f(local_atlas_id.unwrap_or(0), 3); // lcr_local_atlas_id or reserved_zero_3bits
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        // lcr_xlayer_info(0, xId): all present flags clear, then byte_alignment().
        bits.bit(0); // lcr_rep_info_present_flag
        bits.bit(0); // lcr_xlayer_purpose_present_flag
        bits.bit(0); // lcr_xlayer_color_info_present_flag
        bits.bit(0); // lcr_embedded_layer_info_present_flag
        bits.align(); // byte_alignment()
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A minimal SINGLE-mode atlas segment OBU at `xlayer`.
    fn atlas_obu(xlayer: u8, atlas_segment_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(atlas_segment_id, 3); // atlas_segment_id
        bits.uvlc(2); // ats_atlas_segment_mode_idc = SINGLE_ATLAS
        bits.uvlc(0); // ats_nominal_width_minus_1
        bits.uvlc(0); // ats_nominal_height_minus_1
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
    }

    /// An atlas segment OBU whose `ats_atlas_segment_mode_idc` is out of range (5).
    fn atlas_obu_bad_mode(xlayer: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(5); // ats_atlas_segment_mode_idc = 5 -> out of range
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A MULTISTREAM_ATLAS OBU with a single segment, placed at `xlayer`.
    fn atlas_multistream_obu(xlayer: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(3); // ats_atlas_segment_mode_idc = MULTISTREAM_ATLAS
        bits.uvlc(0); // ats_msi_width
        bits.uvlc(0); // ats_msi_height
        bits.uvlc(0); // ats_msi_num_atlas_segments_minus_1 = 0 -> 1 segment
        bits.bit(0); // ats_msi_background_info_present_flag
        bits.f(0, 5); // ats_msi_input_stream_id
        bits.uvlc(0); // pos_x
        bits.uvlc(0); // pos_y
        bits.uvlc(0); // width
        bits.uvlc(0); // height
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A BASIC_ATLAS OBU at `xlayer` whose two segments share an `ats_input_stream_id`.
    fn atlas_basic_duplicate_stream_obu(xlayer: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(1); // ats_atlas_segment_mode_idc = BASIC_ATLAS
        bits.bit(1); // ats_stream_id_present
        bits.uvlc(0); // ats_width
        bits.uvlc(0); // ats_height
        bits.uvlc(1); // ats_num_atlas_segments_minus_1 = 1 -> 2 segments
        for _ in 0..2 {
            bits.f(5, 5); // ats_input_stream_id = 5 (duplicated)
            bits.uvlc(0); // pos_x
            bits.uvlc(0); // pos_y
            bits.uvlc(0); // width
            bits.uvlc(0); // height
        }
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A global MULTISTREAM_ATLAS OBU (xlayer 31) whose two segments share an
    /// `ats_msi_input_stream_id`.
    fn atlas_multistream_duplicate_stream_obu() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // atlas_segment_id
        bits.uvlc(3); // ats_atlas_segment_mode_idc = MULTISTREAM_ATLAS
        bits.uvlc(0); // ats_msi_width
        bits.uvlc(0); // ats_msi_height
        bits.uvlc(1); // ats_msi_num_atlas_segments_minus_1 = 1 -> 2 segments
        bits.bit(0); // ats_msi_background_info_present_flag
        for _ in 0..2 {
            bits.f(5, 5); // ats_msi_input_stream_id = 5 (duplicated)
            bits.uvlc(0); // pos_x
            bits.uvlc(0); // pos_y
            bits.uvlc(0); // width
            bits.uvlc(0); // height
        }
        bits.bit(0); // ats_signaled_atlas_segment_ids_flag
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(17, 0, 0, 31), &bits.into_bytes())
    }

    /// A global LCR OBU whose `lcr_dependent_xlayers_flag` is set (no payload).
    fn global_lcr_obu_with_dependent_flag() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(1, 3); // lcr_global_config_record_id
        bits.f(0b1, 31); // lcr_xlayer_map
        bits.bit(0); // aggregate
        bits.bit(0); // ptl
        bits.bit(0); // payload
        bits.bit(1); // lcr_dependent_xlayers_flag
        bits.bit(0); // atlas present
        bits.f(0, 7); // purpose
        bits.bit(0); // doh
        bits.bit(0); // tile alignment
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // reserved_zero_5bits
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A base-layer sequence header OBU at `xlayer` with `seq_lcr_id`.
    fn sequence_header_obu_with_lcr(xlayer: u8, seq_lcr_id: u32) -> Vec<u8> {
        let payload = sequence_header_payload_with_lcr(0, seq_lcr_id, 0, 0);
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }

    #[test]
    fn hls_seq_lcr_missing_record_is_flagged() {
        // seq_lcr_id = 5 but no LCR precedes the sequence header.
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_with_lcr(3, 5));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_seq_header_resolves_to_local_is_accepted() {
        // A local LCR in xlayer 3 with lcr_local_id = 5 satisfies seq_lcr_id = 5.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 0, 5, None));
        data.extend(sequence_header_obu_with_lcr(3, 5));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_seq_header_resolves_to_global_is_accepted() {
        // A global LCR id 5 whose xlayer_map includes xlayer 3 (bit 3 -> 0b1000 = 8).
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu(5, 0b1000, None));
        data.extend(sequence_header_obu_with_lcr(3, 5));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| {
                d.rule_id == "hls/unavailable-layer-configuration-record"
                    || d.rule_id == "lcr/global-xlayer-map-missing-xlayer"
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_global_xlayer_map_missing_xlayer_is_flagged() {
        // Global LCR id 5 whose xlayer_map (bit 0 only) does NOT include xlayer 3.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu(5, 0b1, None));
        data.extend(sequence_header_obu_with_lcr(3, 5));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/global-xlayer-map-missing-xlayer"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_local_missing_global_is_flagged() {
        // Local LCR references lcr_global_id = 2, but no global LCR is available.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 2, 1, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/global-lcr-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_local_missing_global_is_suppressed_under_external_hls() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // Under external HLS the global LCR could be supplied out-of-band (not
        // modeled), so the in-band-unavailable error must be suppressed.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 2, 1, None));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/global-lcr-unavailable"),
            "external HLS may supply the global LCR; report was: {report}"
        );
    }

    #[test]
    fn atlas_local_atlas_unavailable_is_flagged() {
        // Local LCR references lcr_local_atlas_id = 4, but no local atlas precedes it.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 0, 1, Some(4)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_local_atlas_unavailable_is_suppressed_under_external_hls() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // Under external HLS the local atlas could be supplied out-of-band (not
        // modeled), so the in-band-unavailable error must be suppressed.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 0, 1, Some(4)));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
            "external HLS may supply the local atlas; report was: {report}"
        );
    }

    #[test]
    fn atlas_local_atlas_available_is_accepted() {
        // A local atlas segment OBU (xlayer 3, id 4) precedes the referencing LCR.
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_obu(3, 4));
        data.extend(local_lcr_obu(3, 0, 1, Some(4)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_global_xlayer_map_missing_xlayer_is_suppressed_under_external_hls() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // Same shape as lcr_global_xlayer_map_missing_xlayer_is_flagged, but under
        // external HLS an unmodeled external local LCR could resolve seq_lcr_id ahead
        // of the in-band global, so the xlayer-map check must be suppressed.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu(5, 0b1, None));
        data.extend(sequence_header_obu_with_lcr(3, 5));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/global-xlayer-map-missing-xlayer"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_reserved_bits_nonzero_is_warned() {
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_nonzero_reserved());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .any(|d| d.rule_id == "lcr/reserved-bits-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_segment_mode_out_of_range_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_obu_bad_mode(31));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "atlas/segment-mode-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_global_id_zero_is_flagged() {
        // AV2 §6.8.2: lcr_global_config_record_id must be in 1..7.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu(0, 0b1, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/global-id-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_empty_xlayer_map_is_flagged() {
        // AV2 §6.8.2: lcr_xlayer_map must be in 1..(1 << 31) - 1.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu(1, 0, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/xlayer-map-empty"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_local_id_zero_is_flagged() {
        // AV2 §6.8.3: lcr_local_id must not be 0.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(3, 0, 0, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/local-id-zero"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_dependent_xlayers_flag_nonzero_is_warned() {
        // AV2 §6.8.2: lcr_dependent_xlayers_flag must be 0 (decoder-ignored -> warning).
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_dependent_flag());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .any(|d| d.rule_id == "lcr/dependent-xlayers-flag-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_multistream_outside_global_xlayer_is_flagged() {
        // AV2 §6.9: MULTISTREAM_ATLAS requires obu_xlayer_id == GLOBAL_XLAYER_ID.
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_multistream_obu(3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "atlas/multistream-requires-global-xlayer"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_multistream_in_global_xlayer_is_accepted() {
        // A multistream atlas at GLOBAL_XLAYER_ID is conformant.
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_multistream_obu(31));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "atlas/multistream-requires-global-xlayer"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_duplicate_input_stream_id_is_flagged() {
        // AV2 §6.9.6: ats_input_stream_id values of a basic atlas must be unique.
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_basic_duplicate_stream_obu(3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "atlas/duplicate-input-stream-id"),
            "report was: {report}"
        );
    }

    #[test]
    fn atlas_multistream_duplicate_input_stream_id_is_flagged() {
        // AV2 §6.9.4 gives ats_msi_input_stream_id the same (§6.9.6 unique) semantics.
        let mut data = temporal_delimiter_obu();
        data.extend(atlas_multistream_duplicate_stream_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "atlas/duplicate-input-stream-id"),
            "report was: {report}"
        );
    }

    // ----- Operating point set + buffer removal timing (ops-brt-hls-foundation) -----

    /// Wraps OPS payload bits with the extensible OBU tail (`obu_extension_flag = 0`
    /// then `trailing_bits`).
    fn finish_extensible(mut bits: Bits) -> Vec<u8> {
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        bits.align();
        bits.into_bytes()
    }

    /// Wraps non-extensible (BRT) payload bits with `trailing_bits` only.
    fn finish_non_extensible(mut bits: Bits) -> Vec<u8> {
        bits.bit(1); // trailing_one_bit
        bits.align();
        bits.into_bytes()
    }

    /// Appends one minimal global `operating_point_payload()`: a single included
    /// extended layer (layer 0), no optional fields, `ops_mlayer_info_idc == 0` so no
    /// PTL or mlayer info is coded. Writes a correct `ops_data_size`.
    fn append_minimal_global_payload(bits: &mut Bits) {
        let mut body = Bits::default();
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(0b1, 31); // ops_xlayer_map -> layer 0
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size (single-byte leb128)
        bits.bits.extend_from_slice(&body.bits);
    }

    /// A global OPS OBU defining or resetting `ops_id` with `ops_cnt` minimal
    /// operating points.
    fn global_ops_obu(reset: bool, ops_id: u32, ops_cnt: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(u8::from(reset)); // ops_reset_flag
        bits.f(ops_id, 4); // ops_id
        bits.f(ops_cnt, 3); // ops_cnt
        if ops_cnt > 0 {
            bits.f(0, 4); // ops_priority
            bits.f(0, 7); // ops_intent
            bits.bit(0); // ops_intent_present_flag
            bits.bit(0); // ops_ptl_present_flag
            bits.bit(0); // ops_color_info_present_flag
            bits.f(0, 2); // ops_mlayer_info_idc = 0
            for _ in 0..ops_cnt {
                append_minimal_global_payload(&mut bits);
            }
        }
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
    }

    /// A global OPS OBU (`ops_cnt == 1`, one included layer) with the given
    /// `ops_mlayer_info_idc`. Only used with idc values (0 or 3) that code no mlayer
    /// info for the layer.
    fn global_ops_idc_obu(idc: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // intent present
        bits.bit(0); // ptl present
        bits.bit(0); // color present
        bits.f(idc, 2); // ops_mlayer_info_idc
        append_minimal_global_payload(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
    }

    /// A global OPS OBU (`ops_cnt 1`, `idc 2`) with two included layers, where layer 1
    /// inherits its mlayer info from `(embedded_ops_id, embedded_op_index)`. With
    /// `embedded_ops_id == ops_id` this is a same-OPS reference; otherwise it resolves
    /// against another OPS in the active store.
    fn global_ops_inherited_obu(
        ops_id: u32,
        embedded_ops_id: u32,
        embedded_op_index: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // reset
        bits.f(ops_id, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // priority
        bits.f(0, 7); // intent
        bits.bit(0); // intent present
        bits.bit(0); // ptl present
        bits.bit(0); // color present
        bits.f(2, 2); // ops_mlayer_info_idc = 2
        let mut body = Bits::default();
        body.bit(0); // decoder model present
        body.bit(0); // initial display delay present
        body.f(0b11, 31); // ops_xlayer_map -> layers 0 and 1
        body.bit(1); // layer 0: ops_mlayer_explicit_info_flag = 1
        body.f(0, 8); // layer 0: ops_mlayer_map = 0
        body.bit(0); // layer 1: ops_mlayer_explicit_info_flag = 0 -> inherited
        body.f(embedded_ops_id, 4); // layer 1: ops_embedded_ops_id
        body.f(embedded_op_index, 3); // layer 1: ops_embedded_op_index
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
    }

    /// A local OPS OBU on `xlayer` with `ops_cnt` minimal payloads and the given
    /// `ops_reserved_2bits`. When `size_delta != 0`, the first payload's
    /// `ops_data_size` is offset by `size_delta` to force a size mismatch.
    fn local_ops_obu(
        xlayer: u8,
        reset: bool,
        ops_id: u32,
        ops_cnt: u32,
        reserved_2bits: u32,
        ptl_present: bool,
        size_delta: i32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(u8::from(reset));
        bits.f(ops_id, 4);
        bits.f(ops_cnt, 3);
        if ops_cnt > 0 {
            bits.f(0, 4); // priority
            bits.f(0, 7); // intent
            bits.bit(0); // intent present
            bits.bit(u8::from(ptl_present)); // ptl present
            bits.bit(0); // color present
            bits.f(reserved_2bits, 2); // ops_reserved_2bits
            for index in 0..ops_cnt {
                let mut body = Bits::default();
                if ptl_present {
                    // ops_seq_profile_tier_level_info() with nonzero reserved bits.
                    body.f(0, 5); // seq_profile_idc
                    body.f(0, 5); // level_idx
                    body.bit(0); // tier_flag
                    body.f(0, 3); // mlayer_count
                    body.f(0b11, 2); // ops_ptl_reserved_2bits (nonzero)
                }
                body.bit(0); // decoder model present
                body.bit(0); // initial display delay present
                body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
                body.align();
                let body_bytes = (body.bits.len() / 8) as i64;
                let declared = if index == 0 {
                    (body_bytes + i64::from(size_delta)).max(0) as u32
                } else {
                    body_bytes as u32
                };
                bits.f(declared, 8); // ops_data_size
                bits.bits.extend_from_slice(&body.bits);
            }
        }
        annex_b_obu_with_header(
            &layer_obu_header(18, 0, 0, xlayer),
            &finish_extensible(bits),
        )
    }

    /// An OPS-dependent BRT OBU on `xlayer` referencing `br_ops_id` with `br_ops_cnt`
    /// operating points (no per-op times).
    fn brt_dependent_obu(xlayer: u8, br_ops_id: u32, br_ops_cnt: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // br_ops_dependent_flag
        bits.f(br_ops_id, 4);
        bits.f(br_ops_cnt, 3);
        for _ in 0..br_ops_cnt {
            bits.bit(0); // br_decoder_model_present_op_flag = 0
        }
        annex_b_obu_with_header(
            &layer_obu_header(15, 0, 0, xlayer),
            &finish_non_extensible(bits),
        )
    }

    /// An extended-layer (non-OPS-dependent) BRT OBU on `xlayer`.
    fn brt_extended_layer_obu(xlayer: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // br_ops_dependent_flag = 0
        bits.rg(0, 4); // br_time
        annex_b_obu_with_header(
            &layer_obu_header(15, 0, 0, xlayer),
            &finish_non_extensible(bits),
        )
    }

    /// A local OPS OBU on `xlayer` (`ops_cnt == 1`) whose single operating point
    /// carries explicit `ops_decoder_model_info()` with the given decoder/encoder
    /// buffer delays (`§ 5.11.3`). `reset` sets `ops_reset_flag`.
    fn local_ops_obu_with_delays(
        xlayer: u8,
        reset: bool,
        ops_id: u32,
        decoder_delay: u32,
        encoder_delay: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(u8::from(reset)); // ops_reset_flag
        bits.f(ops_id, 4); // ops_id
        bits.f(1, 3); // ops_cnt
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_reserved_2bits
        let mut body = Bits::default();
        body.bit(1); // ops_decoder_model_info_for_this_op_present_flag
        body.uvlc(decoder_delay); // ops_decoder_buffer_delay
        body.uvlc(encoder_delay); // ops_encoder_buffer_delay
        body.bit(0); // ops_low_delay_mode_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(
            &layer_obu_header(18, 0, 0, xlayer),
            &finish_extensible(bits),
        )
    }

    /// A CLK frame OBU on `xlayer` whose first tile group's frame header references
    /// `seq_header_id` directly (`cur_mfh_id == 0`), confirming activation and starting
    /// a new coded video sequence for the layer (§ 7.3.6).
    fn clk_frame_for_xlayer(xlayer: u8, seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
        annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &bits.into_bytes())
    }

    fn decoder_model_warning_count(report: &ValidationReport, rule: &str) -> usize {
        report.warnings().filter(|d| d.rule_id == rule).count()
    }

    fn ops_error_count(report: &ValidationReport, rule: &str) -> usize {
        report.errors().filter(|d| d.rule_id == rule).count()
    }

    #[test]
    fn ops_local_reserved_bits_nonzero_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu(2, false, 0, 1, 0b10, false, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/local-reserved-bits-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_mlayer_info_idc_reserved_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_idc_obu(3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/mlayer-info-idc-reserved"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_ptl_reserved_bits_nonzero_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu(2, false, 0, 1, 0, true, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/ptl-reserved-bits-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_payload_size_mismatch_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/payload-size-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_inherited_op_index_out_of_range_is_flagged() {
        // Same-OPS reference (embedded_ops_id == ops_id 0): op_index 5 >= ops_cnt 1
        // and >= j (layer 1).
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_inherited_obu(0, 0, 5));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/inherited-op-index-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_cross_ops_inherited_op_index_out_of_range_is_flagged() {
        // OPS 0 inherits from a different, already-defined OPS 1 (ops_cnt 1) at
        // op_index 5, which is out of range — exercises the cross-OPS resolution
        // against the prior active OPS state.
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_obu(false, 1, 1)); // define OPS 1 (ops_cnt 1)
        data.extend(global_ops_inherited_obu(0, 1, 5)); // OPS 0 inherits OPS 1 op 5
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "ops/inherited-op-index-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_cross_ops_inherited_op_index_in_range_is_not_flagged() {
        // OPS 0 inherits from OPS 1 (ops_cnt 3) at op_index 2, which is in range, so
        // the cross-OPS bound check must not flag it.
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_obu(false, 1, 3)); // define OPS 1 (ops_cnt 3)
        data.extend(global_ops_inherited_obu(0, 1, 2)); // OPS 0 inherits OPS 1 op 2
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/inherited-op-index-out-of-range"),
            0,
            "an in-range cross-OPS inheritance must not be flagged: {report}"
        );
    }

    #[test]
    fn ops_reset_removes_active_ops() {
        // Define OPS 0 (cnt 2), reference it (resolves), reset it, reference again.
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_obu(false, 0, 2));
        data.extend(brt_dependent_obu(31, 0, 2)); // matches active ops_cnt 2
        data.extend(global_ops_obu(true, 0, 0)); // reset (cnt 0)
        data.extend(brt_dependent_obu(31, 0, 2)); // now unavailable
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "brt/unavailable-operating-point-set"),
            1,
            "expected exactly the post-reset BRT to be unavailable: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "brt/ops-count-mismatch"),
            0,
            "the pre-reset BRT must resolve and match: {report}"
        );
    }

    #[test]
    fn ops_update_changes_active_ops_count() {
        // Define OPS 0 (cnt 2), match a BRT, then update to cnt 3 and re-reference.
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_obu(false, 0, 2));
        data.extend(brt_dependent_obu(31, 0, 2)); // matches cnt 2
        data.extend(global_ops_obu(false, 0, 3)); // update -> cnt 3
        data.extend(brt_dependent_obu(31, 0, 2)); // now mismatches cnt 3
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "brt/ops-count-mismatch"),
            1,
            "only the post-update BRT should mismatch: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "brt/unavailable-operating-point-set"),
            0,
            "the OPS stays available across the update: {report}"
        );
    }

    #[test]
    fn brt_ops_count_mismatch_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(global_ops_obu(false, 0, 2));
        data.extend(brt_dependent_obu(31, 0, 3)); // br_ops_cnt 3 != ops_cnt 2
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "brt/ops-count-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn brt_missing_ops_is_flagged_when_external_hls_disabled() {
        let mut data = temporal_delimiter_obu();
        data.extend(brt_dependent_obu(31, 5, 1)); // no OPS defined
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
            "report was: {report}"
        );
    }

    #[test]
    fn brt_missing_ops_is_not_hard_error_when_external_ops_declared() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(brt_dependent_obu(31, 5, 1)); // no in-band OPS (31, 5)
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_operating_point_set(31, 5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert_eq!(
            ops_error_count(&report, "brt/unavailable-operating-point-set"),
            0,
            "a declared external OPS must suppress the hard missing-OPS error: {report}"
        );
    }

    #[test]
    fn brt_missing_ops_is_flagged_when_external_hls_lacks_the_ops() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(brt_dependent_obu(31, 5, 1)); // references OPS (31, 5)
        // External HLS is provided but only declares a sequence header and a
        // different OPS, so OPS (31, 5) is still unavailable and must be flagged.
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new()
                    .with_sequence_header_id(0)
                    .with_operating_point_set(31, 4),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "brt/unavailable-operating-point-set"),
            "an external HLS set that does not declare the referenced OPS must still flag it: \
             {report}"
        );
    }

    #[test]
    fn ops_malformed_payload_is_flagged() {
        // A global OPS header claims ops_cnt=1 but the payload ends immediately, so
        // the header fields cannot be read. The OPS syntax check must report it.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(18, 0, 0, 31),
            &[0x01],
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "a malformed OPS payload must be reported: {report}"
        );
    }

    #[test]
    fn brt_malformed_payload_is_flagged() {
        // br_ops_dependent_flag=1, br_ops_id=0, br_ops_cnt=1 fills the single payload
        // byte, so the per-op flag read runs past the input. The BRT syntax check
        // must report it.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(15, 0, 0, 31),
            &[0x81],
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "a malformed BRT payload must be reported: {report}"
        );
    }

    #[test]
    fn global_brt_before_coded_layers_is_accepted() {
        // A global BRT before any coded extended layer unit raises no ordering error.
        let mut data = temporal_delimiter_obu();
        data.extend(brt_extended_layer_obu(31)); // global BRT, extended-layer form
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // a coded-layer OBU
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "obu-order/global-hls-after-coded-layer"),
            0,
            "a global BRT before coded layers must be accepted: {report}"
        );
    }

    #[test]
    fn global_brt_after_coded_layer_is_not_flagged() {
        // § 7.3.7 does not list BRT among global prefix OBUs and § 7.3.3/§ 7.3.4 place
        // it in coded frame units, so a global BRT after a coded layer is left
        // unclassified rather than flagged (sound-over-complete; see
        // is_global_hls_prefix_obu).
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // coded-layer OBU
        data.extend(brt_extended_layer_obu(31)); // global BRT after the coded layer
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "obu-order/global-hls-after-coded-layer"),
            0,
            "a global BRT after a coded layer is not flagged in this phase: {report}"
        );
    }

    #[test]
    fn local_brt_follows_coded_layer_classification() {
        // A local BRT is a coded extended layer OBU (§ 7.3.3/§ 7.3.4): it starts the
        // coded-layer phase, so a later global OPS prefix is flagged out of order.
        let mut data = temporal_delimiter_obu();
        data.extend(brt_extended_layer_obu(2)); // local BRT -> coded extended layer unit
        data.extend(global_ops_obu(false, 0, 1)); // global OPS prefix after a coded layer
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/global-hls-after-coded-layer"),
            "a local BRT must start the coded-layer phase: {report}"
        );
    }

    // --- Decoder-model buffer-delay sum constancy (§6.4.13 / §6.10.5) ---

    #[test]
    fn decoder_model_intra_cvs_ops_sum_change_is_error() {
        // A CLK frame starts a coded video sequence for xlayer 0 (§ 7.3.6), then the
        // same (obu_xlayer_id, ops_id, op) is redefined WITHIN that CVS (same temporal
        // unit, no OPS reset), both explicit, differing sum (30 -> 40) -> error
        // (§ 6.10.5). The CLK makes the stream genuinely intra-CVS — the error tier is
        // gated on a started CVS, so this is the canonical "same coded video sequence"
        // scenario the spec delta describes.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "an intra-CVS OPS buffer-delay sum change must be a single error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_sum_change_before_first_clk_is_not_error() {
        // Two OPS redefinitions before any CLK: the OBUs lie in NO coded video sequence
        // (§ 7.3.6: a CVS starts at a CLK temporal unit), so the § 6.10.5 "video
        // sequence that includes one or more random access points" precondition is
        // unsatisfied and the error tier must not fire. The change spans no CVS or reset
        // boundary either, so the advisory stays silent too.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, no CVS yet
        data.extend(local_ops_obu_with_delays(2, false, 0, 25, 15)); // sum 40, still no CVS
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a pre-first-CLK OPS sum change is in no coded video sequence and must not \
             be an error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "a pre-first-CLK OPS sum change spans no boundary and must not warn: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_sum_change_with_late_clk_in_same_tu_is_not_error() {
        // Temporal-unit granularity (§ 7.3.6): the first OPS is in CVS 1 (TU1's CLK), the
        // second OPS sits in TU2 BEFORE TU2's own CLK. The CVS epoch is still 1 when the
        // second OPS is observed (the CLK comes later in TU2), but that CLK starts a NEW
        // coded video sequence for TU2, so the two OPS straddle a real CVS boundary and
        // the change is conforming under the per-CVS reading. The deferred error must be
        // dropped, never emitted — and the cross-CVS advisory fires in its place so the
        // genuinely cross-CVS change is not silently lost.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
        data.extend(temporal_delimiter_obu()); // TU2 begins
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, epoch still 1
        data.extend(clk_frame_for_xlayer(0, 0)); // late CLK -> TU2 is CVS 2
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a late same-TU CLK makes the change cross-CVS; the deferred error must be \
             dropped: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "the dropped deferred error must be replaced by the cross-CVS advisory, not \
             silently lost: {report}"
        );
    }

    #[test]
    fn decoder_model_intra_cvs_ops_same_sum_is_not_flagged() {
        // Identical sums (different split, 10+20 vs 20+10) must not fire either tier.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu_with_delays(2, false, 0, 20, 10)); // sum 30
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "an unchanged sum must not be flagged: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "an unchanged sum must not raise the advisory either: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_sum_change_across_cvs_is_not_error_but_warns() {
        // A genuine CVS boundary at a temporal-unit edge (§ 7.3.6): TU1 holds CVS 1's
        // OPS, TU2's CLK starts CVS 2 and its OPS redefines the same triple with a
        // different sum. The two OPS sit in different coded video sequences, so the
        // change is conforming under the per-CVS reading: no error, only the cross-CVS
        // advisory (§ 6.4.13 / § 6.10.5). Both OPS are placed AFTER their CVS's CLK so
        // neither shares a temporal unit across the boundary.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
        data.extend(temporal_delimiter_obu()); // TU2 begins
        data.extend(clk_frame_for_xlayer(0, 0)); // TU2: starts CVS 2 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, CVS 2
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a cross-CVS OPS sum change must not be an error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "a cross-CVS OPS sum change must raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_sum_change_across_reset_is_not_error_but_warns() {
        // An OPS reset between the two definitions (same CVS) re-baselines the
        // constraint: no error, but the reset-spanning change raises the advisory.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu_with_delays(2, true, 0, 25, 15)); // reset, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a reset-spanning OPS sum change must not be an error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "a reset-spanning OPS sum change must raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_redefinition_without_explicit_info_clears_baseline() {
        // The defining redefinition (ops_cnt > 0) omits ops_decoder_model_info() for the
        // op it covers. Per Annex E.1 (`annex-e-decoder-model.md` lines 25–27) the
        // previous parameters do not persist: the redefinition clears the stored
        // baseline for that triple rather than reusing it, so it neither compares against
        // the vanished value nor against the Annex E mode defaults. With nothing to
        // compare, no diagnostic of either tier is emitted.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, explicit
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "an absent-info redefinition clears the baseline and must not be compared: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "an absent-info redefinition must not raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_annex_e_defaults_are_never_compared() {
        // The default Annex E split (70000/20000, sum 90000) is a resource-availability
        // fallback, not a signalled value. A single explicit OPS whose sum equals that
        // default must not be compared against the default-bearing absent-info OPS.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
        data.extend(local_ops_obu_with_delays(2, false, 0, 70_000, 20_000)); // explicit 90000
        data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "absent-info OPS using the Annex E defaults must not be compared: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "absent-info OPS must not raise the advisory against an explicit value: {report}"
        );
    }

    #[test]
    fn decoder_model_seq_header_sum_change_across_cvs_warns() {
        // Two coded video sequences whose frame-confirmed activated sequence headers
        // carry explicit, differing seq_decoder_model_info() sums -> the § 6.4.13
        // advisory (warning). The seq-header tier has no error severity.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        )); // sum 0
        data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
        )); // sum 12
        data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "an activated seq-header sum change across a CLK must raise the advisory: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "the seq-header tier is advisory only, never an error: {report}"
        );
    }

    #[test]
    fn decoder_model_seq_header_same_id_reconfiguration_across_cvs_warns() {
        // A same-seq_header_id reconfiguration is the canonical conforming way to change
        // activated-header parameters across a CVS boundary (legal at the boundary,
        // § 7.3.6). CVS 1 activates seq_header_id 0 with sum 0; CVS 2 re-sends the SAME
        // id 0 with a differing sum (12) and a CLK re-confirming it. The id never
        // changes, so the activation event's id-change short-circuit would skip it — the
        // advisory must still fire because it is evaluated on every frame-confirmed
        // activation at the (post-CLK) new CVS epoch.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        )); // id 0, sum 0
        data.extend(clk_frame_for_xlayer(0, 0)); // confirm id 0 + start CVS 1
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 5, 7),
        )); // id 0 again, sum 12
        data.extend(clk_frame_for_xlayer(0, 0)); // re-confirm id 0 + start CVS 2
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "a same-id reconfiguration changing the sum across a CVS boundary must raise \
             the advisory: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "the seq-header tier is advisory only, never an error: {report}"
        );
    }

    #[test]
    fn decoder_model_seq_header_without_info_never_warns() {
        // Consecutive CVSs whose activated headers omit seq_decoder_model_info() never
        // fire the advisory.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "headers without decoder-model info must not raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_seq_header_fallback_guess_activation_never_warns() {
        // With several in-band sequence headers and NO frame to confirm activation,
        // the first-seen activation is a fallback guess that must not participate in
        // the cross-CVS advisory (agreement_activation_for returns None).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        ));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "an unconfirmed fallback-guess activation must not participate: {report}"
        );
    }

    #[test]
    fn decoder_model_external_hls_suppresses_both_ids() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // The exact intra-CVS error scenario, but with external HLS Provided: both the
        // error and the advisory must be suppressed.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        ));
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40 (intra-CVS)
        data.extend(clk_frame_for_xlayer(0, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
        ));
        data.extend(clk_frame_for_xlayer(0, 1));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new()
                    .with_sequence_header_id(0)
                    .with_sequence_header_id(1),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "external HLS must suppress the OPS error tier: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "external HLS must suppress both decoder-model advisories: {report}"
        );
    }

    #[test]
    fn decoder_model_external_hls_without_seq_headers_still_suppresses_seq_advisory() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // The seq-header advisory's only previous suppression was the
        // external_declares_sequence_header early return, which is false when the
        // Provided set declares NO sequence header (only an operating point set here).
        // The blanket `ExternalHlsMode::Provided` guard must still suppress the seq tier,
        // matching design decision 5: a same-id reconfiguration across a CVS that would
        // otherwise warn must stay silent.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        ));
        data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
        )); // differing sum
        data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_operating_point_set(31, 0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "Provided external HLS without declared sequence headers must still suppress \
             the seq-header advisory: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "the seq-header tier never emits an error: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_sum_change_across_targeted_reset_is_not_error_but_warns() {
        // A § 6.10.1 case-3 targeted reset (ops_reset_flag == 0, ops_cnt == 0) of OPS 0
        // between the two definitions re-baselines the constraint for that OPS alone,
        // exactly like a full reset: no error, but the reset-spanning sum change raises
        // the cross-CVS advisory. The CLK makes the stream genuinely intra-CVS so that
        // without the targeted-reset re-baselining the error tier WOULD fire.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu(0, false, 0, 0, 0, false, 0)); // targeted reset of OPS 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // redefine, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a targeted-reset-spanning OPS sum change must not be an error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "a targeted-reset-spanning OPS sum change must raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_targeted_reset_of_other_ops_still_errors() {
        // The intra-CVS error must still fire when the intervening targeted reset hits a
        // DIFFERENT OPS (here OPS 1): re-baselining is per-(obu_xlayer_id, opsID), so a
        // targeted reset of OPS 1 does not excuse a sum change of OPS 0 within the CVS.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
        data.extend(local_ops_obu(0, false, 1, 0, 0, false, 0)); // targeted reset of OPS 1
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "a targeted reset of a different OPS must not excuse OPS 0's intra-CVS sum \
             change: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_dm_less_redefinition_clears_baseline_no_diagnostic() {
        // FINDING C (Annex E.1, mirror `annex-e-decoder-model.md` lines 25–27): "If the
        // new Operating Point Set OBU does not signal decoder model parameters for a
        // given operating point, the previous set of decoder model parameters does not
        // persist." explicit-30, then a redefinition of the SAME (xlayer, ops_id) that
        // OMITS ops_decoder_model_info() for that op (so it does not persist), then
        // explicit-40: the dm-less redefinition clears the baseline, so explicit-40 has
        // nothing to compare against -> NEITHER the error nor the advisory fires. (All in
        // one CVS so that without clearing the error tier WOULD fire.)
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, explicit
        data.extend(local_ops_obu(0, false, 0, 1, 0, false, 0)); // redefine OPS 0, no dm info
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, explicit
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a dm-less redefinition clears the baseline; explicit-40 must not error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "a cleared baseline must not be compared, so no advisory either: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_unrelated_redefinition_still_errors_within_cvs() {
        // FINDING C control: an UNRELATED other-OPS OBU between the two explicit
        // definitions of OPS 0 must NOT clear OPS 0's baseline, so the intra-CVS error
        // still fires. OPS 1 is defined dm-less between the two OPS 0 definitions; the
        // clearing is keyed on (xlayer, ops_id), so OPS 1's redefinition leaves OPS 0
        // untouched and explicit-30 vs explicit-40 of OPS 0 is still a single error.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
        data.extend(local_ops_obu(0, false, 1, 1, 0, false, 0)); // unrelated OPS 1, no dm info
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "a dm-less redefinition of a DIFFERENT OPS must not clear OPS 0's baseline: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_seq_header_dm_less_activation_clears_baseline_no_warning() {
        // FINDING D (Annex E.1, mirror `annex-e-decoder-model.md` lines 24–25): "If the
        // new Sequence Header OBU does not signal decoder model parameters for an
        // extended layer, the previous set of decoder model parameters does not persist."
        // Three coded video sequences: CVS 1 activates an explicit-sum header, CVS 2
        // activates a header WITHOUT seq_decoder_model_info() (clearing the baseline),
        // CVS 3 activates an explicit header with a DIFFERENT sum. Because the dm-less
        // activation cleared the baseline, CVS 3 has no persistent previous parameter set
        // to compare against -> no cross-CVS advisory.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
        )); // id 0, sum 0
        data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1))); // id 1, no dm info
        data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2 (clears baseline)
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_decoder_model_sum(2, 5, 7),
        )); // id 2, sum 12
        data.extend(clk_frame_for_xlayer(0, 2)); // confirm + start CVS 3
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "a dm-less activation between the two explicit headers clears the baseline; \
             the later explicit sum must not raise the advisory: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "the seq-header tier never emits an error: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_cross_layer_local_reset_does_not_excuse_intra_cvs_error() {
        // FINDING B (§ 6.10.1 case 1, mirror `06-syntax-structures-semantics.md` lines
        // 2577–2578): a local reset resets "All OPS for the associated extended layer",
        // not all layers. xlayer 0 defines sum 30, xlayer 1 sends a LOCAL reset (which
        // resets only xlayer 1's OPS), then xlayer 0 redefines sum 40 within its own CVS.
        // No reset of xlayer 0 intervened, so the intra-CVS error must still fire — the
        // per-layer reset generation no longer lets xlayer 1's reset re-baseline xlayer 0.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
        data.extend(local_ops_obu(1, true, 0, 0, 0, false, 0)); // LOCAL reset of xlayer 1
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "a local reset of an unrelated extended layer must not excuse xlayer 0's \
             intra-CVS sum change: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_global_reset_re_baselines_other_layers() {
        // FINDING B control (§ 6.10.1 case 1, mirror lines 2577–2578): a GLOBAL reset
        // resets "all layers if global", so it DOES re-baseline xlayer 0. xlayer 0 sum
        // 30, then a global (GLOBAL_XLAYER_ID = 31) reset, then xlayer 0 sum 40 within the
        // CVS: the global reset re-baselines the constraint, so no error, only the
        // reset-spanning advisory.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
        data.extend(local_ops_obu(31, true, 0, 0, 0, false, 0)); // GLOBAL reset (all layers)
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a global reset re-baselines xlayer 0, so the change is not an error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            1,
            "a global-reset-spanning sum change must raise the advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_pre_clk_baseline_in_same_tu_migrates_to_new_cvs_error() {
        // FINDING A (§ 7.3.6, mirror `07-decoding-process.md` lines 604–606): "A new
        // coded video sequence for an extended layer is defined to start at each temporal
        // unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY ...". OPS
        // sum 30 is observed BEFORE the CLK, but the whole CLK temporal unit lies in the
        // NEW coded video sequence, so the baseline migrates to the new CVS epoch and the
        // post-CLK OPS sum 40 (same TU) is compared within ONE coded video sequence ->
        // the intra-CVS error fires (not merely the advisory).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
        data.extend(clk_frame_for_xlayer(0, 0)); // CLK -> whole TU is the new CVS
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, post-CLK, same TU
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "a pre-CLK baseline in the CLK's own TU migrates to the new CVS; the post-CLK \
             change is intra-CVS and must error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the migrated intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_both_definitions_pre_clk_in_same_tu_is_error() {
        // FINDING (round-3, § 7.3.6, mirror `07-decoding-process.md` lines 604–606): "A
        // new coded video sequence for an extended layer is defined to start at each
        // temporal unit that contains an OBU with obu_type equal to OBU_CLOSED_LOOP_KEY
        // ...". BOTH OPS definitions of the same (obu_xlayer_id, ops_id, op) occur BEFORE
        // the CLK in the SAME temporal unit, with no coded video sequence started yet for
        // the layer. The whole CLK temporal unit lies in the new coded video sequence, so
        // both observations are intra-CVS and the differing sum (30 -> 40) is the error
        // tier — the comparison is deferred PreCvs at the second OPS and emitted when the
        // CLK starts the layer's coded video sequence.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, pre-CLK
        data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU -> whole TU is the new CVS
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            1,
            "both pre-CLK OPS definitions in the CLK's own temporal unit are intra-CVS; \
             the differing sum must be a single error: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the deferred intra-CVS error must not also raise the cross-CVS advisory: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_both_definitions_pre_clk_no_clk_in_tu_stays_silent() {
        // The same [seq, OPS30, OPS40] pair as the round-3 case but with NO CLK in the
        // temporal unit (the temporal unit closes at the next temporal delimiter): the
        // observations are in no coded video sequence (§ 7.3.6), so the § 6.10.5
        // random-access-point precondition is unsatisfied and the deferred PreCvs error is
        // dropped silently — preserving the documented pre-first-CLK silence. The second
        // temporal delimiter completes the first temporal unit and triggers the silent
        // drop.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, no CVS yet
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, still no CVS
        data.extend(temporal_delimiter_obu()); // TU closes with no CLK for the layer
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            0,
            "a pre-first-CLK OPS sum change whose temporal unit closes with no CLK is in \
             no coded video sequence and must stay silent: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "the dropped pre-CVS comparison spans no boundary and must not warn: {report}"
        );
    }

    #[test]
    fn decoder_model_ops_multiple_pre_clk_changes_same_tu_error_per_change() {
        // Three pre-CLK definitions 30 -> 40 -> 50 of the same triple in the CLK's own
        // temporal unit. § 7.3.6 places all three in the new coded video sequence, so each
        // consecutive change is a distinct intra-CVS comparison: two PreCvs errors are
        // deferred (30 -> 40 at the second OPS, 40 -> 50 at the third) and both are emitted
        // when the CLK starts the layer's coded video sequence — exactly one diagnostic
        // per comparison, matching the eager mid-CVS path (one error per consecutive
        // change).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
        data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
        data.extend(local_ops_obu_with_delays(0, false, 0, 30, 20)); // sum 50
        data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
            2,
            "two consecutive intra-CVS sum changes must produce two errors, one per \
             comparison: {report}"
        );
        assert_eq!(
            decoder_model_warning_count(
                &report,
                "decoder-model/buffer-delay-sum-changed-across-cvs"
            ),
            0,
            "intra-CVS changes must not raise the cross-CVS advisory: {report}"
        );
    }

    // --- Quantizer matrix (§5.13 / §6.12) and film grain (§5.14 / §6.13) ---

    /// `OBU_QUANTIZATION_MATRIX` header byte: ext=0, type=22, tlayer=0.
    const QM_HEADER: u8 = 0x58;
    /// `OBU_FILM_GRAIN` header byte: ext=0, type=23, tlayer=0.
    const FG_HEADER: u8 = 0x5C;

    /// A complete, activating sequence header OBU for `obu_xlayer_id = 0` (so the
    /// coded-frame-unit QM/FG OBUs that follow have an active sequence header).
    fn active_sequence_header_obu() -> Vec<u8> {
        annex_b_obu(0x04, &sequence_header_payload(0, 0))
    }

    /// A `quantizer_matrix_obu()` with `qm_bit_map == 0` (the reset/default path).
    fn qm_reset_obu() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 15); // qm_bit_map = 0
        bits.bit(0); // qm_chroma_info_present_flag
        bits.bit(1); // trailing_one_bit (QM is non-extensible)
        bits.align();
        annex_b_obu(QM_HEADER, &bits.into_bytes())
    }

    /// A `quantizer_matrix_obu()` selecting a single `level` with its default matrix.
    fn qm_default_level_obu(level: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(1 << level, 15); // qm_bit_map: set `level`
        bits.bit(0); // qm_chroma_info_present_flag = 0 -> 1 plane
        bits.bit(1); // qm_is_default_flag for `level`
        bits.bit(1); // trailing_one_bit
        bits.align();
        annex_b_obu(QM_HEADER, &bits.into_bytes())
    }

    /// Appends the smallest non-monochrome `film_grain_model()` (no scaling points,
    /// `ar_coeff_lag == 0`).
    fn append_minimal_film_grain_model(bits: &mut Bits) {
        bits.bit(0); // chroma_scaling_from_luma
        bits.f(0, 4); // num_y_points
        bits.f(0, 4); // num_cb_points
        bits.f(0, 4); // num_cr_points
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0 -> no AR coeffs
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range = 0 -> mc_identity inferred 0
        bits.bit(0); // film_grain_block_size
    }

    /// A `film_grain_obu()` with the given `update_flags` and (non-monochrome)
    /// `chroma_idc`, with one minimal model per set update-flag bit.
    fn film_grain_obu_bytes(update_flags: u32, chroma_idc: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(update_flags, 8);
        bits.uvlc(chroma_idc);
        for _ in 0..update_flags.count_ones() {
            append_minimal_film_grain_model(&mut bits);
        }
        bits.bit(1); // trailing_one_bit (FG is non-extensible)
        bits.align();
        annex_b_obu(FG_HEADER, &bits.into_bytes())
    }

    fn has_error(report: &ValidationReport, rule: &str) -> bool {
        report.errors().any(|d| d.rule_id == rule)
    }

    #[test]
    fn qm_duplicate_reset_between_frames_is_flagged() {
        // Two reset (qm_bit_map == 0) QM OBUs between coded frames: only the first may
        // be a reset (§6.12).
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_reset_obu());
        data.extend(qm_reset_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "qm/duplicate-reset-between-frames"),
            "a second reset QM OBU must be flagged: {report}"
        );
    }

    #[test]
    fn qm_single_reset_between_frames_is_conformant() {
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_reset_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "qm/duplicate-reset-between-frames"),
            "a single reset QM OBU is conformant: {report}"
        );
    }

    #[test]
    fn qm_duplicate_level_between_frames_is_flagged() {
        // Two QM OBUs both specifying level 0 between coded frames (§6.12).
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_default_level_obu(0));
        data.extend(qm_default_level_obu(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "qm/duplicate-level-between-frames"),
            "specifying QM level 0 twice must be flagged: {report}"
        );
    }

    #[test]
    fn qm_distinct_levels_between_frames_is_conformant() {
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_default_level_obu(0));
        data.extend(qm_default_level_obu(1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "qm/duplicate-level-between-frames"),
            "two distinct QM levels are conformant: {report}"
        );
    }

    #[test]
    fn qm_duplicate_level_across_coded_frame_is_not_flagged() {
        // The same level on either side of a coded frame is in two different
        // "between coded frames" windows, so it is not a duplicate.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_default_level_obu(0));
        data.extend(annex_b_obu(0x10, &[0xe0])); // OBU_CLOSED_LOOP_KEY (frame-bearing)
        data.extend(qm_default_level_obu(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "qm/duplicate-level-between-frames"),
            "the same level across a coded frame must not be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_zero_update_flags_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_obu_bytes(0, 0)); // fgm_update_flags == 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/update-flags-zero"),
            "fgm_update_flags == 0 must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_chroma_idc_out_of_range_is_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_obu_bytes(0b0000_0001, 4)); // fgm_chroma_idc = 4 (> 3)
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/chroma-idc-out-of-range"),
            "fgm_chroma_idc > 3 must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_duplicate_slot_in_coded_frame_unit_is_flagged() {
        // Two film grain OBUs both updating slot 0 in the same coded frame unit (§6.13).
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_obu_bytes(0b0000_0001, 0));
        data.extend(film_grain_obu_bytes(0b0000_0001, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
            "updating slot 0 twice in one coded frame unit must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_distinct_slots_are_conformant() {
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0
        data.extend(film_grain_obu_bytes(0b0000_0010, 0)); // slot 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
            "distinct film grain slots are conformant: {report}"
        );
        assert!(!has_error(&report, "film-grain/update-flags-zero"));
        assert!(!has_error(&report, "film-grain/chroma-idc-out-of-range"));
    }

    #[test]
    fn qm_malformed_payload_is_flagged() {
        // A quantizer matrix payload too short for qm_bit_map f(15): the
        // QuantizerMatrixSyntax check must report the parse error.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(QM_HEADER, &[0xFF]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "bitstream/parse-error"),
            "a malformed quantizer matrix payload must be reported: {report}"
        );
    }

    #[test]
    fn film_grain_malformed_payload_is_flagged() {
        // fgm_update_flags sets slot 0, but the film_grain_model is truncated
        // (num_y_points = 5 with no point payload): the FilmGrainSyntax check must
        // report the parse error.
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
        bits.uvlc(2); // fgm_chroma_idc = 444 (non-monochrome)
        bits.bit(0); // chroma_scaling_from_luma
        bits.f(5, 4); // num_y_points = 5 -> point payload follows, but the input ends
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(FG_HEADER, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "bitstream/parse-error"),
            "a malformed film-grain payload must be reported: {report}"
        );
    }

    #[test]
    fn qm_quant_delta_out_of_range_is_flagged() {
        // AV2 §6.4.11: quant_delta must be in -128..=127. A user-defined QM whose first
        // delta is 128 (svlc encoded as uvlc(255)) must be flagged.
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // 1 plane
        bits.bit(0); // qm_is_default_flag = 0
        bits.bit(0); // qm_8x8_is_symmetric = 0
        bits.uvlc(255); // svlc(128) -> quant_delta = 128 (out of range)
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(annex_b_obu(QM_HEADER, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "qm/quant-delta-out-of-range"),
            "an out-of-range quant_delta must be flagged: {report}"
        );
    }

    /// Appends `num` scaling points (per-point value increment 1, scaling 0), so the
    /// points are strictly increasing.
    fn append_scaling_points(bits: &mut Bits, num: u32) {
        bits.f(num, 4);
        if num > 0 {
            bits.f(0, 3); // point_value_increment_bits_minus_1 = 0 -> bitsIncr = 1
            bits.f(0, 2); // point_scaling_bits_minus_5 = 0 -> bitsScal = 5
            for _ in 0..num {
                bits.f(1, 1); // value increment = 1
                bits.f(0, 5); // scaling = 0
            }
        }
    }

    /// Appends a non-monochrome `film_grain_model()` (chroma_scaling_from_luma = 0,
    /// ar_coeff_lag = 0) with the given scaling-point counts.
    fn append_film_grain_model_with_points(bits: &mut Bits, num_y: u32, num_cb: u32, num_cr: u32) {
        bits.bit(0); // chroma_scaling_from_luma = 0
        append_scaling_points(bits, num_y);
        append_scaling_points(bits, num_cb);
        append_scaling_points(bits, num_cr);
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0 -> numPosLuma = 0
        let num_pos_chroma = u32::from(num_y > 0); // numPosLuma(0) + (num_y>0 ? 1 : 0)
        if num_y > 0 {
            bits.f(0, 2); // bits_per_ar_coeff_y_minus_5 (numPosLuma = 0 coeffs)
        }
        if num_cb > 0 {
            bits.f(0, 2); // bits_per_ar_coeff_cb_minus_5 -> bitsCoef = 5
            for _ in 0..num_pos_chroma {
                bits.f(16, 5); // ar_coeffs_cb (16 - 16 = 0)
            }
        }
        if num_cr > 0 {
            bits.f(0, 2);
            for _ in 0..num_pos_chroma {
                bits.f(16, 5);
            }
        }
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        if num_cb > 0 {
            bits.f(0, 8); // cb_mult
            bits.f(0, 8); // cb_luma_mult
            bits.f(0, 9); // cb_offset
        }
        if num_cr > 0 {
            bits.f(0, 8); // cr_mult
            bits.f(0, 8); // cr_luma_mult
            bits.f(0, 9); // cr_offset
        }
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
    }

    fn film_grain_model_obu(chroma_idc: u32, num_y: u32, num_cb: u32, num_cr: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
        bits.uvlc(chroma_idc);
        append_film_grain_model_with_points(&mut bits, num_y, num_cb, num_cr);
        bits.bit(1); // trailing_one_bit
        bits.align();
        annex_b_obu(FG_HEADER, &bits.into_bytes())
    }

    #[test]
    fn film_grain_too_many_scaling_points_is_flagged() {
        // AV2 §6.17.10.2: num_y_points must be <= 14. A model with num_y_points = 15
        // must be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_model_obu(2, 15, 0, 0)); // 4:4:4, 15 luma points
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/scaling-points-out-of-range"),
            "num_y_points > 14 must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_420_unpaired_chroma_points_is_flagged() {
        // AV2 §6.17.10.2: in 4:2:0, cb and cr points must be both zero or both nonzero.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_model_obu(0, 1, 0, 1)); // 4:2:0, cb=0 but cr=1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/chroma-points-not-paired"),
            "unpaired 4:2:0 chroma points must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_paired_chroma_points_are_conformant() {
        // 4:2:0 with both cb and cr nonzero is conformant.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_model_obu(0, 1, 1, 1)); // 4:2:0, cb=1 and cr=1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "film-grain/chroma-points-not-paired"),
            "paired 4:2:0 chroma points are conformant: {report}"
        );
        assert!(!has_error(
            &report,
            "film-grain/scaling-points-out-of-range"
        ));
        assert!(!has_error(
            &report,
            "film-grain/scaling-point-not-increasing"
        ));
    }

    #[test]
    fn film_grain_non_increasing_scaling_point_is_flagged() {
        // AV2 §6.17.10.2: point_y_value[i] must be strictly greater than the previous.
        // Two luma points with increments [1, 0] -> values [1, 1] (not increasing).
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(2); // 4:4:4 (no chroma pairing constraint)
        bits.bit(0); // chroma_scaling_from_luma = 0
        bits.f(2, 4); // num_y_points = 2
        bits.f(0, 3); // point_value_increment_bits_minus_1 = 0 -> bitsIncr = 1
        bits.f(0, 2); // point_scaling_bits_minus_5 = 0 -> bitsScal = 5
        bits.f(1, 1); // point_y_value[0] increment = 1 -> value 1
        bits.f(0, 5); // point_y_scaling[0]
        bits.f(0, 1); // point_y_value[1] increment = 0 -> value stays 1 (not increasing)
        bits.f(0, 5); // point_y_scaling[1]
        bits.f(0, 4); // num_cb_points = 0
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0
        bits.f(0, 2); // bits_per_ar_coeff_y_minus_5 (numPosLuma = 0 coeffs; num_y > 0)
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
        bits.bit(1); // trailing_one_bit
        bits.align();
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(annex_b_obu(FG_HEADER, &bits.into_bytes()));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/scaling-point-not-increasing"),
            "a non-increasing scaling point must be flagged: {report}"
        );
    }

    #[test]
    fn qm_reset_then_level_definition_is_conformant() {
        // A reset (qm_bit_map == 0) followed by a level definition is the canonical
        // §5.13 sequence: the reset is the first QM OBU, and the subsequent level is
        // not a duplicate. This exercises the reset path (which also clears per-level
        // availability) without a false positive.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_reset_obu());
        data.extend(qm_default_level_obu(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(!has_error(&report, "qm/duplicate-reset-between-frames"));
        assert!(
            !has_error(&report, "qm/duplicate-level-between-frames"),
            "a level definition after a reset is not a duplicate: {report}"
        );
    }

    #[test]
    fn qm_duplicate_level_across_temporal_delimiter_is_flagged() {
        // AV2 §6.12: the duplicate-level window closes at a coded frame, NOT at a
        // temporal-unit boundary. The same level on either side of a bare temporal
        // delimiter (no intervening frame) is still a duplicate.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(qm_default_level_obu(0));
        data.extend(temporal_delimiter_obu()); // new temporal unit, but no coded frame
        data.extend(qm_default_level_obu(0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "qm/duplicate-level-between-frames"),
            "a level reused across a TD with no intervening frame must be flagged: {report}"
        );
    }

    #[test]
    fn film_grain_duplicate_slot_across_temporal_delimiter_is_flagged() {
        // AV2 §6.13: the duplicate-slot window closes at a coded frame, NOT at a
        // temporal-unit boundary.
        let mut data = temporal_delimiter_obu();
        data.extend(active_sequence_header_obu());
        data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0
        data.extend(temporal_delimiter_obu()); // new temporal unit, but no coded frame
        data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0 again
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
            "a slot reused across a TD with no intervening frame must be flagged: {report}"
        );
    }

    fn has_warning(report: &ValidationReport, rule: &str) -> bool {
        report.warnings().any(|d| d.rule_id == rule)
    }

    // --- padding OBU (AV2 § 5.16 / § 6.15) ---

    /// A global `OBU_PADDING` (xlayer 31) carrying `payload`, after a temporal delimiter.
    fn global_padding_stream(payload: &[u8]) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(25, 0, 0, 31),
            payload,
        ));
        data
    }

    #[test]
    fn padding_all_zero_payload_is_flagged() {
        let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0x00, 0x00]));
        assert!(
            has_error(&report, "padding/all-zero-payload"),
            "report was: {report}"
        );
    }

    #[test]
    fn padding_invalid_trailing_bits_is_flagged() {
        // 0x40 = 0b0100_0000: trailing_one_bit must be 1 but the first bit is 0.
        let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0x40]));
        assert!(
            has_error(&report, "padding/invalid-trailing-bits"),
            "report was: {report}"
        );
    }

    #[test]
    fn padding_valid_payload_is_accepted() {
        // One arbitrary padding byte then a trailing-bits byte.
        let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0xFF, 0x80]));
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("padding/")),
            "report was: {report}"
        );
    }

    #[test]
    fn padding_empty_payload_is_accepted() {
        let report = Validator::new(false).validate_bytes(&global_padding_stream(&[]));
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("padding/")),
            "report was: {report}"
        );
    }

    // --- metadata OBUs (AV2 § 5.17 / § 6.16) ---

    /// Builds a `metadata_short_obu()` payload: the 1-byte header, a 1-byte metadata
    /// type, the metadata unit bytes, and one OBU trailing byte.
    fn metadata_short_payload(first: u8, metadata_type: u8, unit: &[u8]) -> Vec<u8> {
        let mut payload = vec![first, metadata_type];
        payload.extend_from_slice(unit);
        payload.push(0x80);
        payload
    }

    /// A global `OBU_METADATA_SHORT` (xlayer 31) after a temporal delimiter.
    fn global_metadata_short_stream(payload: &[u8]) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            payload,
        ));
        data
    }

    /// A global `OBU_METADATA_GROUP` (xlayer 31) after a temporal delimiter.
    fn global_metadata_group_stream(payload: &[u8]) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 31),
            payload,
        ));
        data
    }

    #[test]
    fn metadata_short_layer_idc_out_of_range_is_flagged() {
        // first byte 0x38 = 0b0_011_1_000: layer_idc=3 (>= 3), cancel=1.
        let payload = [0x38, 0x04, 0x80];
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/short-layer-idc-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_short_payload_underflow_is_flagged() {
        // obuPayloadSize = 2, leb128 bytes = 1 -> 2 - 2 - 1 underflows.
        let payload = [0x00, 0x01];
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/unit-payload-underflow"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_group_unit_count_too_large_is_flagged() {
        // metadata_unit_cnt_minus_1 = 16383 (leb128 0xFF 0x7F).
        let payload = [0x00, 0xFF, 0x7F, 0x80];
        let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
        assert!(
            has_error(&report, "metadata/group-unit-count-too-large"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_group_header_underflow_is_flagged() {
        // Non-cancel unit, muh_header_size = 0: the payload_size leb byte alone makes
        // headerRemainingBytes negative.
        let payload = [0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x80];
        let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
        assert!(
            has_error(&report, "metadata/group-header-underflow"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_group_reserved_bits_nonzero_is_warned() {
        // type=0 (UnknownRaw), header_size=3, payload_size=0, reserved bits = 0b01.
        let payload = [0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x01, 0x80];
        let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
        assert!(
            has_warning(&report, "metadata/group-reserved-bits-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_group_xlayer_map_global_bit_set_is_flagged() {
        // Global group, layer_idc=LAYER_VALUES, muh_xlayer_map = 0x8000_0000 (bit 31 set).
        // header_size = payload_size leb (1) + fixed 2 + 4 (xlayer_map) = 7.
        let payload = [
            0x00, 0x00, // group header + cnt
            0x00, // metadata_type = 0
            0x0E, // muh_header_size = 7, cancel = 0
            0x00, // muh_payload_size = 0
            0x60, 0x00, // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
            0x80, 0x00, 0x00, 0x00, // muh_xlayer_map = bit 31 set
            0x80, // OBU trailing byte
        ];
        let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
        assert!(
            has_error(&report, "metadata/group-xlayer-map-global-bit-set"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_group_mlayer_map_below_obu_mlayer_is_flagged() {
        // Local group at mlayer 1, layer_idc=LAYER_VALUES -> one muh_mlayer_map byte with
        // bit 0 set (below obu_mlayer_id = 1). header_size = leb(1) + 2 + 1 = 4.
        let payload = [
            0x00, 0x00, // group header + cnt
            0x00, // metadata_type = 0
            0x08, // muh_header_size = 4, cancel = 0
            0x00, // muh_payload_size = 0
            0x60, 0x00, // layer_idc=LAYER_VALUES(3)
            0x01, // muh_mlayer_map = bit 0 set
            0x80, // OBU trailing byte
        ];
        // A non-global metadata OBU needs an active sequence header for its xlayer.
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(2, 1, 1));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 1, 2),
            &payload,
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/group-mlayer-map-below-obu-mlayer"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_temporal_point_info_in_group_is_flagged() {
        // A group unit with metadata_type = METADATA_TYPE_TEMPORAL_POINT_INFO (9).
        let payload = [0x00, 0x00, 0x09, 0x01, 0x80]; // one cancelled unit, type 9
        let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
        assert!(
            has_error(&report, "metadata/temporal-point-info-not-short"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_seconds_out_of_range_is_flagged() {
        let mut bits = Bits::default();
        bits.f(0, 5); // counting_type
        bits.bit(1); // full_timestamp_flag
        bits.bit(0); // discontinuity_flag
        bits.bit(0); // cnt_dropped_flag
        bits.f(0, 9); // n_frames
        bits.f(60, 6); // seconds_value = 60 (> 59)
        bits.f(0, 6); // minutes_value
        bits.f(0, 5); // hours_value
        bits.f(0, 5); // time_offset_length = 0
        bits.align();
        let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/timecode-seconds-out-of-range"),
            "report was: {report}"
        );
    }

    /// Builds a full-timestamp `metadata_timecode()` short OBU payload with the given
    /// seconds/minutes/hours values and no time offset.
    fn timecode_short_payload(seconds: u32, minutes: u32, hours: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 5); // counting_type
        bits.bit(1); // full_timestamp_flag
        bits.bit(0); // discontinuity_flag
        bits.bit(0); // cnt_dropped_flag
        bits.f(0, 9); // n_frames
        bits.f(seconds, 6); // seconds_value
        bits.f(minutes, 6); // minutes_value
        bits.f(hours, 5); // hours_value
        bits.f(0, 5); // time_offset_length = 0
        bits.align();
        metadata_short_payload(0x00, 4, &bits.into_bytes())
    }

    #[test]
    fn metadata_timecode_minutes_out_of_range_is_flagged() {
        let payload = timecode_short_payload(0, 60, 0); // minutes_value = 60 (> 59)
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/timecode-minutes-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_hours_out_of_range_is_flagged() {
        let payload = timecode_short_payload(0, 0, 24); // hours_value = 24 (> 23)
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/timecode-hours-out-of-range"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_in_range_is_accepted() {
        // Maximum valid full-timestamp values: 59 / 59 / 23.
        let payload = timecode_short_payload(59, 59, 23);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("metadata/timecode-")),
            "report was: {report}"
        );
    }

    /// Builds a `metadata_timecode()` short OBU payload with `full_timestamp_flag = 0`
    /// and per-field presence flags. `seconds`/`minutes`/`hours` are `Some(value)` when
    /// the field is signaled (its enclosing flag set), `None` when absent. `n_frames`
    /// is configurable. The hierarchical flags (§ 5.17.7) require seconds present for
    /// minutes, and minutes present for hours — the helper asserts that invariant so a
    /// test cannot encode an impossible bitstream.
    fn timecode_flagged_payload(
        n_frames: u32,
        seconds: Option<u32>,
        minutes: Option<u32>,
        hours: Option<u32>,
    ) -> Vec<u8> {
        metadata_short_payload(
            0x00,
            4,
            &timecode_unit_bits(n_frames, seconds, minutes, hours),
        )
    }

    /// The raw `metadata_timecode()` syntax bytes (no metadata-unit wrapper) for a
    /// `full_timestamp_flag = 0` set with per-field presence flags. See
    /// [`timecode_flagged_payload`] for the field semantics.
    fn timecode_unit_bits(
        n_frames: u32,
        seconds: Option<u32>,
        minutes: Option<u32>,
        hours: Option<u32>,
    ) -> Vec<u8> {
        assert!(
            !(minutes.is_some() && seconds.is_none()),
            "minutes_value requires seconds_value present (§ 5.17.7)"
        );
        assert!(
            !(hours.is_some() && minutes.is_none()),
            "hours_value requires minutes_value present (§ 5.17.7)"
        );
        let mut bits = Bits::default();
        bits.f(0, 5); // counting_type
        bits.bit(0); // full_timestamp_flag = 0 -> per-field flags
        bits.bit(0); // discontinuity_flag
        bits.bit(0); // cnt_dropped_flag
        bits.f(n_frames, 9); // n_frames
        bits.bit(u8::from(seconds.is_some())); // seconds_flag
        if let Some(s) = seconds {
            bits.f(s, 6); // seconds_value
            bits.bit(u8::from(minutes.is_some())); // minutes_flag
            if let Some(m) = minutes {
                bits.f(m, 6); // minutes_value
                bits.bit(u8::from(hours.is_some())); // hours_flag
                if let Some(h) = hours {
                    bits.f(h, 5); // hours_value
                }
            }
        }
        bits.f(0, 5); // time_offset_length = 0
        bits.align();
        bits.into_bytes()
    }

    #[test]
    fn metadata_timecode_inferred_seconds_without_previous_is_flagged() {
        // AV2 § 6.16.7: the first timecode in scope omits seconds_value
        // (full_timestamp_flag 0, seconds_flag 0), which is inferred from the previous
        // set in decoding order — but no previous set exists, so the inference has no
        // source and the field is required to have been present before.
        let payload = timecode_flagged_payload(0, None, None, None);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            report.errors().any(|d| {
                d.rule_id == "metadata/timecode-inferred-without-previous"
                    && d.message.contains("seconds_value")
            }),
            "report was: {report}"
        );
    }

    /// A global `OBU_METADATA_SHORT` (xlayer 31) carrying `payload`, with no temporal
    /// delimiter prefix (for chaining several into one stream).
    fn global_metadata_short_obu(payload: &[u8]) -> Vec<u8> {
        annex_b_obu_with_header(&layer_obu_header(8, 0, 0, 31), payload)
    }

    #[test]
    fn metadata_timecode_inference_after_present_value_passes() {
        // AV2 § 6.16.7: a full-timestamp first timecode carries seconds/minutes/hours,
        // so a following timecode that omits them all infers from that previous present
        // set in decoding order — no inference diagnostic.
        let mut data = temporal_delimiter_obu();
        data.extend(global_metadata_short_obu(&timecode_short_payload(
            12, 34, 5,
        )));
        data.extend(global_metadata_short_obu(&timecode_flagged_payload(
            0, None, None, None,
        )));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
            "an omitted field after a present previous value infers cleanly; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_full_timestamp_first_passes() {
        // A full-timestamp first timecode carries every field, so nothing is inferred.
        let payload = timecode_short_payload(0, 0, 0);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_inference_names_each_absent_field() {
        // First timecode carries seconds only (seconds_flag 1, minutes_flag 0): minutes
        // and hours are absent with no previous present value -> two diagnostics naming
        // minutes_value and hours_value; seconds_value is present, so it is silent.
        let payload = timecode_flagged_payload(0, Some(30), None, None);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        let inferred: Vec<&str> = report
            .errors()
            .filter(|d| d.rule_id == "metadata/timecode-inferred-without-previous")
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(inferred.len(), 2, "report was: {report}");
        assert!(
            inferred.iter().any(|m| m.contains("minutes_value"))
                && inferred.iter().any(|m| m.contains("hours_value"))
                && !inferred.iter().any(|m| m.contains("seconds_value")),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_inference_chain_resets_at_clk() {
        // A full-timestamp timecode in CVS 1 carries every field. A CLK at the next
        // temporal unit starts a new coded video sequence (§ 7.3.6), breaking the
        // decoding-order inference chain — so a following timecode that omits seconds
        // has no previous present value in the new sequence and is flagged.
        let mut data = global_metadata_short_stream(&timecode_short_payload(0, 0, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(0, None, None, None),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
            "the inference chain must reset across the CLK boundary; report was: {report}"
        );
    }

    /// `BASE_TIMING` is display_tick 1000, time_scale 30000, equal_picture_interval
    /// true, num_ticks_minus_1 1: TicksPerPicture = (1 + 1) * 1000 = 2000, so
    /// maxPicPerSecond = ceil(30000 / 2000) = 15. n_frames must be < 15.

    #[test]
    fn metadata_timecode_n_frames_exceeds_rate_is_flagged() {
        // n_frames 15 == maxPicPerSecond 15, which violates "n_frames shall be less
        // than maxPicPerSecond". The CI establishes timing at xlayer 0; the timecode is
        // a global suffix unit (xlayer 31) describing every layer.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_boundary_passes() {
        // n_frames 14 == maxPicPerSecond - 1: the strict "less than" bound is satisfied.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(14, Some(0), Some(0), Some(0)),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "n_frames == maxPicPerSecond - 1 must pass; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_without_timing_is_silent() {
        // No content interpretation establishes ci_timing_info_present_flag, so the
        // bound does not apply even for a large n_frames.
        let payload = timecode_flagged_payload(400, Some(0), Some(0), Some(0));
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "absent CI timing means no n_frames bound; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_ci_arrives_after_metadata_flagged() {
        // Re-evaluation path: the timecode precedes the content interpretation that
        // establishes its timing. A second identical CI must not re-report (the timing
        // is unchanged, so the recheck is skipped, mirroring the scan-type dedup).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
        ));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
            1,
            "the bound is reported once, not per repeated CI; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_unequal_interval_bound() {
        // equal_picture_interval 0: TicksPerPicture = num_units_in_display_tick = 1000,
        // so maxPicPerSecond = ceil(30000 / 1000) = 30. n_frames 30 violates, 29 passes.
        let unequal = CiTiming {
            equal_picture_interval: false,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(unequal)));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(30, Some(0), Some(0), Some(0)),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "report was: {report}"
        );

        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(unequal)));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(29, Some(0), Some(0), Some(0)),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "n_frames 29 == maxPicPerSecond - 1 must pass; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_omitted_after_omitted_is_flagged() {
        // Finding 1 (literal "present" reading): a full-timestamp first timecode carries
        // every field; a second set omits seconds (inferred cleanly from the present
        // first set); a THIRD set also omits seconds. Under the literal reading the
        // second set's seconds_value is INFERRED, not present, so it does not satisfy the
        // third set's "such a previous seconds_value shall have been present" requirement
        // — the third omission fires. (A chained-inference reading would stay silent.)
        let mut data = temporal_delimiter_obu();
        data.extend(global_metadata_short_obu(&timecode_short_payload(
            12, 34, 5,
        )));
        data.extend(global_metadata_short_obu(&timecode_flagged_payload(
            0, None, None, None,
        )));
        data.extend(global_metadata_short_obu(&timecode_flagged_payload(
            0, None, None, None,
        )));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "metadata/timecode-inferred-without-previous"
                    && d.message.contains("seconds_value")
            }),
            "an omitted field whose predecessor only INFERRED it (never coded it) fires \
             under the literal reading; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_omitted_then_clk_in_same_tu_seeds_from_new_cvs() {
        // Finding 2 (same-TU CLK attribution): a prior-CVS timecode carries seconds
        // (present). A new temporal unit holds, in decoding order, a timecode that omits
        // seconds and THEN a CLK. Per § 7.3.6 the whole temporal unit containing the CLK
        // joins the NEW coded video sequence, so the prior-CVS present seconds must not
        // seed the omitting set's inference — the diagnostic fires.
        let mut data = global_metadata_short_stream(&timecode_short_payload(0, 0, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(global_metadata_short_obu(&timecode_flagged_payload(
            0, None, None, None,
        )));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "metadata/timecode-inferred-without-previous"
                    && d.message.contains("seconds_value")
            }),
            "a same-TU CLK after the omitting timecode pulls it into the new CVS, so the \
             prior-CVS seed must not satisfy the inference; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_ci_after_metadata_anchors_at_metadata() {
        // Finding 3: the CI-after re-evaluation path anchors the diagnostic at the
        // offending timecode metadata OBU (which the message also names), not the later
        // content interpretation OBU.
        let mut data = temporal_delimiter_obu();
        // The timecode metadata OBU starts one Annex B leb128 size-prefix byte past the
        // preceding OBUs.
        let timecode_offset = data.len() as u64 + 1;
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
        ));
        let ci_offset = data.len() as u64 + 1;
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                    && d.byte_offset.map(|o| o.get()) == Some(timecode_offset)
            }),
            "the diagnostic must anchor at the timecode metadata OBU (byte \
             {timecode_offset}), not the CI OBU (byte {ci_offset}); report was: {report}"
        );
    }

    /// A global `OBU_METADATA_GROUP` (xlayer 31) carrying one non-cancel timecode unit
    /// (type 4) with `muh_layer_idc = LAYER_VALUES`, `muh_xlayer_map` selecting xlayer 0,
    /// and a single `muh_mlayer_map` byte targeting the embedded layers in `mlayer_map`.
    fn global_timecode_group_layer_values(mlayer_map: u8, unit: &[u8]) -> Vec<u8> {
        // muh_header_size = payload_size leb (1) + fixed 2 + muh_xlayer_map 4 + one
        // muh_mlayer_map byte (xlayer 0 selected) = 8. The timecode unit is a handful of
        // bytes, so its length is a single-byte leb128 muh_payload_size.
        assert!(unit.len() < 128, "timecode unit fits a 1-byte leb128");
        let payload_size = unit.len() as u8;
        let mut payload = vec![
            0x00, // is_suffix=0, necessity=0, application_id=0
            0x00, // metadata_unit_cnt_minus_1 = 0
            0x04, // metadata_type = 4 (METADATA_TYPE_TIMECODE)
            0x10, // muh_header_size = 8, cancel = 0
            payload_size,
            0x60,
            0x00, // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
            0x00,
            0x00,
            0x00,
            0x01,       // muh_xlayer_map = bit 0 (xlayer 0) set
            mlayer_map, // muh_mlayer_map for xlayer 0
        ];
        payload.extend_from_slice(unit);
        payload.push(0x80); // OBU trailing byte
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 31),
            &payload,
        ));
        data
    }

    #[test]
    fn metadata_timecode_n_frames_targeting_excludes_untargeted_layer_ci() {
        // Finding 4 (§ 6.16.3 layer targeting): a global LAYER_VALUES timecode targeting
        // embedded layer 1 only. Embedded layer 0 carries a low-rate CI timing whose
        // maxPicPerSecond the timecode's n_frames would exceed; embedded layer 1 carries a
        // CI whose timing makes the n_frames legal. The timecode must pair only with its
        // targeted layer (1), so no diagnostic — pairing with the untargeted layer 0 CI
        // (the pre-fix behavior) would wrongly fire.
        let low_rate = CiTiming {
            display_tick: 1000,
            time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
            equal_picture_interval: true,
            num_ticks_minus_1: 1,
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        // Embedded layer 0: low-rate CI (maxPicPerSecond 1); embedded layer 1: BASE_TIMING
        // (maxPicPerSecond 15). The timecode below targets layer 1 only with n_frames 2,
        // which is < 15 (legal for layer 1) but >= 1 (would violate layer 0).
        data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
        data.extend(content_interpretation_obu(1, 0, Some(BASE_TIMING)));
        let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
        // muh_mlayer_map bit 1 set -> targets embedded layer 1 only.
        data.extend(global_timecode_group_layer_values(0x02, &unit));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "a LAYER_VALUES timecode targeting layer 1 only must not pair with layer 0's \
             CI timing; report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_targeting_pairs_with_targeted_layer_ci() {
        // Finding 4 control: the same targeting still pairs with the TARGETED layer's CI.
        // Embedded layer 1 carries a low-rate CI (maxPicPerSecond 1); the layer-1-targeted
        // timecode's n_frames 2 exceeds it, so the diagnostic fires.
        let low_rate = CiTiming {
            display_tick: 1000,
            time_scale: 1000, // maxPicPerSecond = 1
            equal_picture_interval: true,
            num_ticks_minus_1: 1,
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(content_interpretation_obu(1, 0, Some(low_rate)));
        let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
        data.extend(global_timecode_group_layer_values(0x02, &unit));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "a LAYER_VALUES timecode targeting layer 1 must pair with layer 1's CI; \
             report was: {report}"
        );
    }

    #[test]
    fn metadata_timecode_n_frames_olk_reinit_drops_deferred_pairing() {
        // Finding 5 (§ 7.3.8.11 CI reinit): a prior-TU timecode carries n_frames 5. A
        // later temporal unit holds, in decoding order, a content interpretation OBU whose
        // low-rate timing (maxPicPerSecond 1) makes the prior-TU timecode violate the
        // n_frames bound — that pairing is *deferred* (the timecode sits in an earlier
        // temporal unit) — and then an OLK, a § 7.3.8.11 random access point that
        // reinitializes ci_timing_info_present_flag to 0. The OLK must drop the deferred
        // n_frames pairing (its pre-epoch timing no longer constrains the post-epoch
        // pictures), so no diagnostic survives. (Pre-fix the n_frames rule was not in the
        // OLK's drop set, so the deferred diagnostic would flush and fire.)
        let low_rate = CiTiming {
            display_tick: 1000,
            time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
            equal_picture_interval: true,
            num_ticks_minus_1: 1,
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        // TU0: timecode with n_frames 5 (no CI yet, so the bound is not decided here).
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
        ));
        data.extend(temporal_delimiter_obu()); // -> TU1
        // TU1: the CI establishes the violating timing -> the recheck DEFERS the n_frames
        // diagnostic (TU0 observation vs TU1 CI), then the OLK reinit drops it.
        data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
        data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> § 7.3.8.11 RAP
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
            "the OLK reinitializes ci_timing_info_present_flag to 0, so the deferred \
             pairing against the prior-TU CI must drop; report was: {report}"
        );
    }

    /// Builds a `metadata_decoded_frame_hash()` short OBU payload (type 5) with a single
    /// frame hash (per_plane 0) and the given reserved bit.
    fn frame_hash_payload(reserved: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 4); // hash_type = 0 (MD5)
        bits.bit(0); // per_plane = 0 -> single frame_hash
        bits.bit(0); // has_grain
        bits.bit(0); // is_monochrome
        bits.bit(reserved); // reserved
        for _ in 0..16 {
            bits.f(0, 8); // frame_hash bytes
        }
        bits.align();
        metadata_short_payload(0x00, 5, &bits.into_bytes())
    }

    #[test]
    fn metadata_decoded_frame_hash_reserved_nonzero_is_warned() {
        // AV2 § 6.16.13: "reserved shall be set to 0 and ignored by decoders" — a
        // non-zero reserved bit is a decoder-ignored producer anomaly (warning).
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_short_stream(&frame_hash_payload(1)));
        assert!(
            has_warning(&report, "metadata/decoded-frame-hash-reserved-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_decoded_frame_hash_reserved_zero_is_silent() {
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_short_stream(&frame_hash_payload(0)));
        assert!(
            !has_warning(&report, "metadata/decoded-frame-hash-reserved-nonzero"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_scan_type_pic_struct_reserved_is_flagged() {
        // mps_pic_struct_type = 13 (> 12): 0b01101_00_0 = 0x68.
        let payload = metadata_short_payload(0x00, 8, &[0x68]);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-reserved"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_valid_short_is_accepted() {
        // A cancelled short metadata OBU is well-formed and emits no metadata error.
        let payload = [0x08, 0x04, 0x80]; // cancel=1, type=4
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("metadata/")),
            "report was: {report}"
        );
    }

    // --- metadata reserved-value warnings (AV2 § 6.16.3) ---

    /// A metadata group payload with one non-cancel unit of reserved
    /// `metadata_type` 0 (no unit payload bytes) and the given `muh_layer_idc` /
    /// `muh_persistence_idc`. `muh_layer_idc` must not be `LAYER_VALUES` (3),
    /// which would add layer-map bytes.
    fn group_unit_payload(layer_idc: u8, persistence_idc: u8) -> Vec<u8> {
        assert_ne!(layer_idc, 3, "LAYER_VALUES would require layer-map bytes");
        vec![
            0x00, // is_suffix=0, necessity=0, application_id=0
            0x00, // metadata_unit_cnt_minus_1 = 0
            0x00, // metadata_type = 0 (Reserved -> UnknownRaw, no unit bytes)
            0x06, // muh_header_size = 3, cancel = 0
            0x00, // muh_payload_size = 0
            // muh_layer_idc f(3), muh_persistence_idc f(3), muh_priority hi 2 bits.
            (layer_idc << 5) | ((persistence_idc & 0x07) << 2),
            0x00, // muh_priority lo 6 bits + muh_reserved_zero_2bits
            0x80, // OBU trailing byte
        ]
    }

    #[test]
    fn metadata_persistence_reserved_idc_warns() {
        // AV2 § 6.16.3: muh_persistence_idc 4..=7 is "Reserved for AOMedia use"
        // with no "shall" attached -> a warning, never a conformance error.
        // Short form: first byte 0b0_000_0_100 (non-cancel, persistence 4).
        let short = metadata_short_payload(0x04, 1, &[0x12, 0x34, 0x56, 0x78]);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&short));
        assert!(
            has_warning(&report, "metadata/persistence-idc-reserved"),
            "report was: {report}"
        );
        assert!(
            report.is_conformant(),
            "a reserved value is a warning, not an error: {report}"
        );

        // Group form: one non-cancel unit with persistence 4.
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_group_stream(&group_unit_payload(0, 4)));
        assert!(
            has_warning(&report, "metadata/persistence-idc-reserved"),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn metadata_persistence_defined_idc_is_accepted() {
        // BASIC_PERSISTENCE (1) is a defined mode for both forms; a cancel unit's
        // muh_persistence_idc carries no persistence semantics (§ 5.17.2 reads it
        // before the early return), so a reserved value there does not warn.
        let short = metadata_short_payload(0x01, 1, &[0x12, 0x34, 0x56, 0x78]);
        let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&short));
        assert!(
            !has_warning(&report, "metadata/persistence-idc-reserved"),
            "report was: {report}"
        );

        let report = Validator::new(false)
            .validate_bytes(&global_metadata_group_stream(&group_unit_payload(0, 1)));
        assert!(
            !has_warning(&report, "metadata/persistence-idc-reserved"),
            "report was: {report}"
        );

        // Short cancel unit with persistence bits 4: 0b0_000_1_100.
        let cancelled = [0x0C, 0x01, 0x80];
        let report =
            Validator::new(false).validate_bytes(&global_metadata_short_stream(&cancelled));
        assert!(
            !has_warning(&report, "metadata/persistence-idc-reserved"),
            "a cancel unit must not warn about its persistence bits: {report}"
        );
    }

    #[test]
    fn metadata_group_layer_idc_reserved_warns() {
        // AV2 § 6.16.3: muh_layer_idc 4..=7 is "Reserved for AOMedia use" with no
        // "shall" attached -> a warning, never a conformance error (the short
        // form's muh_layer_idc < 3 rule, § 6.16.2, is a separate error).
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_group_stream(&group_unit_payload(4, 1)));
        assert!(
            has_warning(&report, "metadata/group-layer-idc-reserved"),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn metadata_group_layer_idc_defined_values_accepted() {
        // LAYER_CURRENT (2) is a defined mode -> no reserved-value warning.
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_group_stream(&group_unit_payload(2, 1)));
        assert!(
            !has_warning(&report, "metadata/group-layer-idc-reserved"),
            "report was: {report}"
        );
    }

    // --- metadata persistence / cancellation lifetime (AV2 § 6.16.3) and HDR
    // --- repeat content (AV2 § 6.16.5 / § 6.16.6) ---

    /// A short metadata OBU with an extension header at `obu_xlayer_id == xlayer`
    /// (tlayer / mlayer 0) carrying `metadata_short_payload(first, metadata_type,
    /// unit)`.
    fn metadata_short_obu_at(xlayer: u8, first: u8, metadata_type: u8, unit: &[u8]) -> Vec<u8> {
        annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, xlayer),
            &metadata_short_payload(first, metadata_type, unit),
        )
    }

    /// Parses `data` as Annex B and observes every OBU through a fresh
    /// [`ValidatorContext`], returning the context and the collected report. The
    /// § 6.16.3 lifetime-store semantics gate no validate_bytes diagnostic (they
    /// are decoder-applicability rules, not conformance requirements), so the
    /// store scenarios assert state through the context's query surface.
    fn context_after_observing(data: &[u8]) -> (ValidatorContext, ValidationReport) {
        use splot_core::annexb::parse_annex_b_obus;
        let obus = parse_annex_b_obus(data).unwrap_or_default();
        assert!(!obus.is_empty(), "the test stream must parse into OBUs");
        let options = ValidationOptions::default();
        let mut report = ValidationReport::new();
        let mut context = ValidatorContext::default();
        for obu in &obus {
            context.observe_obu(obu, &options, &mut report);
        }
        (context, report)
    }

    #[test]
    fn basic_persistence_cancel_clears_active() {
        // AV2 § 6.16.3 BASIC_PERSISTENCE: "Persistence until ... the cancel flag
        // (muh_cancel_flag) is encountered."
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x01, 1, &[0x12, 0x34, 0x56, 0x78])); // BASIC
        let (context, _) = context_after_observing(&data);
        assert_eq!(context.active_metadata_units(x0, 1).len(), 1);

        data.extend(metadata_short_obu_at(0, 0x08, 1, &[])); // cancel, type 1
        let (context, _) = context_after_observing(&data);
        assert!(
            context.active_metadata_units(x0, 1).is_empty(),
            "a cancel unit must clear the extended layer's BASIC record"
        );
    }

    #[test]
    fn global_persistence_ignores_cancel() {
        // AV2 § 6.16.3 GLOBAL_PERSISTENCE: "The cancel flag (muh_cancel_flag) does
        // not do anything to it."
        use crate::metadata_lifetime::PersistenceMode;
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x12, 0x34, 0x56, 0x78])); // GLOBAL
        data.extend(metadata_short_obu_at(0, 0x08, 1, &[])); // cancel, type 1
        let (context, _) = context_after_observing(&data);
        let units = context.active_metadata_units(x0, 1);
        assert_eq!(
            units.len(),
            1,
            "a GLOBAL_PERSISTENCE record must survive the cancel"
        );
        assert_eq!(units[0].persistence, PersistenceMode::Global);
    }

    #[test]
    fn global_persistence_overwrites_prior_global() {
        // AV2 § 6.16.3 GLOBAL_PERSISTENCE: "When this mode is signaled previously
        // signaled global metadata of this type are overwritten."
        use splot_core::headers::metadata::MetadataPayload;
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x12, 0x34, 0x56, 0x78]));
        // ObuEnvelope::offset is the OBU header's offset, just after the size byte.
        let second_obu_offset = data.len() as u64 + 1;
        data.extend(metadata_short_obu_at(0, 0x00, 1, &[0x99, 0x99, 0x56, 0x78]));
        let (context, _) = context_after_observing(&data);
        let units = context.active_metadata_units(x0, 1);
        assert_eq!(units.len(), 1, "the later GLOBAL record must overwrite");
        assert!(
            matches!(
                units[0].payload,
                MetadataPayload::HdrCll(cll) if cll.max_cll == 0x9999
            ),
            "the surviving record must carry the later content"
        );
        assert_eq!(
            units[0].offset.get(),
            second_obu_offset,
            "the surviving record must locate the later observation"
        );
    }

    #[test]
    fn global_xlayer_cancel_clears_all_layers() {
        // AV2 § 6.16.3: a cancel with obu_xlayer_id == GLOBAL_XLAYER_ID cancels the
        // type "for a set of extended layers"; cancel units carry no layer maps
        // (§ 5.17.3), so the store clears the type across all extended layers.
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);
        let x1 = ExtendedLayerId::from_bits(1);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x01, 1, &[0x12, 0x34, 0x56, 0x78]));
        data.extend(metadata_short_obu_at(1, 0x01, 1, &[0x12, 0x34, 0x56, 0x78]));
        data.extend(metadata_short_obu_at(31, 0x08, 1, &[])); // global cancel, type 1
        let (context, _) = context_after_observing(&data);
        assert!(context.active_metadata_units(x0, 1).is_empty());
        assert!(context.active_metadata_units(x1, 1).is_empty());
    }

    #[test]
    fn no_persistence_expires_at_coded_frame() {
        // AV2 § 6.16.3 NO_PERSISTENCE: "Used only for the current frame" — the
        // record lapses once a coded frame has been observed, while a BASIC record
        // survives it.
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x02, 1, &[0x12, 0x34, 0x56, 0x78])); // NO
        data.extend(metadata_short_obu_at(0, 0x01, 0, &[0xDE, 0xAD])); // BASIC, type 0
        let (context, _) = context_after_observing(&data);
        assert_eq!(context.active_metadata_units(x0, 1).len(), 1);
        assert_eq!(context.active_metadata_units(x0, 0).len(), 1);

        // A coded frame: a leading tile group (type 6) — the frame-bearing
        // classification is OBU-header-level, so the empty payload is irrelevant.
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
        let (context, _) = context_after_observing(&data);
        assert!(
            context.active_metadata_units(x0, 1).is_empty(),
            "a NO_PERSISTENCE record must expire at the coded frame"
        );
        assert_eq!(
            context.active_metadata_units(x0, 0).len(),
            1,
            "a BASIC record must survive the coded frame"
        );
    }

    #[test]
    fn cvs_restart_drops_active_metadata() {
        // AV2 § 7.3.6: a CLK starts a new coded video sequence AT the temporal
        // unit. The temporal-unit-1 record is dropped; the record observed earlier
        // in the CLK's own temporal unit joined the new coded video sequence and
        // survives. The two units use different muh_layer_idc values (1 and 2) so
        // the § 6.16.3 BASIC same-scope replacement cannot explain the drop.
        use splot_core::headers::metadata::MetadataPayload;
        use splot_core::types::ExtendedLayerId;
        let x0 = ExtendedLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78])); // layer_idc 1
        data.extend(temporal_delimiter_obu());
        data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78])); // layer_idc 2

        // Control: without a CLK the two records coexist (different layer scopes).
        let (context, _) = context_after_observing(&data);
        assert_eq!(context.active_metadata_units(x0, 1).len(), 2);

        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let (context, _) = context_after_observing(&data);
        let units = context.active_metadata_units(x0, 1);
        assert_eq!(
            units.len(),
            1,
            "the CLK must drop earlier-temporal-unit records only"
        );
        assert!(
            matches!(
                units[0].payload,
                MetadataPayload::HdrCll(cll) if cll.max_cll == 0x9999
            ),
            "the same-temporal-unit record joins the new coded video sequence"
        );
    }

    #[test]
    fn cancel_unknown_type_emits_nothing() {
        // The § 6.16.3 cancel text is purely indicative ("indicates that any
        // previously signaled metadata information ... is cancelled"); cancelling a
        // type that was never signaled is NOT a conformance finding.
        let report = Validator::new(false)
            .validate_bytes(&global_metadata_short_stream(&[0x08, 0x05, 0x80]));
        assert!(report.is_conformant(), "report was: {report}");
        assert_eq!(report.warnings().count(), 0, "report was: {report}");
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.rule_id.starts_with("metadata/")),
            "report was: {report}"
        );
    }

    /// Sequence-header payload (id 0, the given `max_tlayer_id`, max_mlayer_id 2)
    /// with a signaled § 5.4.1 mlayer dependency map: rows 1..=2 in descending
    /// refLayer order yield `[1][1]=1, [1][0]=0; [2][2]=1, [2][1]=0, [2][0]=1`.
    /// Row `[1][0]` differs from the lower-triangular default fill, proving the
    /// signaled map (not the default) is consulted. With `max_tlayer_id > 0` the
    /// `tlayer_dependency_present_flag` bit is cleared, so `TLayerDependencyMap`
    /// keeps the § 5.4.1 default fill (`refTLayer <= currTLayer`).
    fn sequence_header_payload_with_mlayer_deps(max_tlayer_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(max_tlayer_id, 2); // max_tlayer_id
        bits.f(2, 3); // max_mlayer_id = 2
        bits.f(0, 2); // seq_max_mlayer_cnt_minus_1 (CeilLog2(3) = 2 bits)
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(1); // mlayer_dependency_present_flag (max_mlayer_id > 0)
        // currLayer 1, refLayer 1..=0 descending: [1][1]=1, [1][0]=0.
        bits.bit(1);
        bits.bit(0);
        // currLayer 2, refLayer 2..=0 descending: [2][2]=1, [2][1]=0, [2][0]=1.
        bits.bit(1);
        bits.bit(0);
        bits.bit(1);
        // max_tlayer_id == 0 -> no tlayer_dependency_present_flag bit (§ 5.4.1).
        if max_tlayer_id > 0 {
            bits.bit(0); // tlayer_dependency_present_flag -> default fill
        }
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    #[test]
    fn propagation_respects_dependency_maps() {
        // AV2 § 6.16.3 propagation rules against the activated sequence header's
        // signaled § 5.4.1 dependency maps: upward multi-layer persistence needs
        // explicit layer persistence indication (muh_layer_idc == LAYER_VALUES and
        // muh_mlayer_map bits above obu_mlayer_id) AND MLayerDependencyMap[M][K]
        // == 1, combined with TLayerDependencyMap[M][C][T] == 1.
        use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
        let x0 = ExtendedLayerId::from_bits(0);
        let t0 = TemporalLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_mlayer_deps(0),
        ));
        // Group unit (HDR CLL, BASIC): muh_layer_idc = LAYER_VALUES with
        // muh_mlayer_map bits 1 and 2 set (above K = 0 -> explicit indication).
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 0),
            &[
                0x00,        // is_suffix=0, necessity=0, application_id=0
                0x00,        // metadata_unit_cnt_minus_1 = 0
                0x01,        // metadata_type = HdrCll
                0x08,        // muh_header_size = 4, cancel = 0
                0x04,        // muh_payload_size = 4
                0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
                0x00,        // priority lo + reserved bits
                0b0000_0110, // muh_mlayer_map: embedded layers 1 and 2
                0x12,
                0x34,
                0x56,
                0x78, // metadata_hdr_cll
                0x80, // OBU trailing byte
            ],
        ));
        // Group unit (reserved type 0, BASIC): muh_layer_idc = LAYER_CURRENT (2),
        // no explicit layer persistence indication.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 0),
            &[0x00, 0x00, 0x00, 0x06, 0x00, 0x44, 0x00, 0x80],
        ));
        let (context, _) = context_after_observing(&data);

        let values_units = context.active_metadata_units(x0, 1);
        assert_eq!(values_units.len(), 1);
        let record = &values_units[0];
        // Temporal persistence within K = 0: TLayerDependencyMap[0][0][0] is 1
        // (default fill; max_tlayer_id == 0 signals no tlayer map).
        assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
        // Multi-layer + combined persistence: MLayerDependencyMap[2][0] is
        // signaled 1 -> applies to embedded layer 2.
        assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
        // MLayerDependencyMap[1][0] is signaled 0 (the lower-triangular default
        // would say 1) -> does not apply to embedded layer 1.
        assert!(
            !context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(1), t0),
            "the SIGNALED map, not the default fill, must be consulted"
        );

        let current_units = context.active_metadata_units(x0, 0);
        assert_eq!(current_units.len(), 1);
        let record = &current_units[0];
        // LAYER_CURRENT has no explicit layer persistence indication (§ 6.16.3
        // NOTE), so the unit never persists upward even though
        // MLayerDependencyMap[2][0] is 1.
        assert!(!context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
        assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
    }

    #[test]
    fn propagation_temporal_axis_and_explicit_indication_boundary() {
        // AV2 § 6.16.3 temporal persistence: "Within embedded layer K, the
        // metadata persists to temporal layer C if TLayerDependencyMap[K][C][T]
        // is equal to 1." With max_tlayer_id = 1 and a cleared
        // tlayer_dependency_present_flag, the § 5.4.1 default fill is
        // refTLayer <= currTLayer, so a unit carried at temporal layer T = 1
        // applies to C = 1 ([K][1][1] = 1) and NOT to C = 0 ([K][0][1] = 0) —
        // proving applies_to consults the map as [K][target][source].
        use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
        let x0 = ExtendedLayerId::from_bits(0);
        let m0 = EmbeddedLayerId::from_bits(0);
        let t0 = TemporalLayerId::from_bits(0);
        let t1 = TemporalLayerId::from_bits(1);

        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_mlayer_deps(1),
        ));
        // Short unit (HDR CLL, BASIC) carried at obu_tlayer_id 1, mlayer 0.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 1, 0, 0),
            &metadata_short_payload(0x01, 1, &[0x12, 0x34, 0x56, 0x78]),
        ));
        // Group unit (reserved type 0, BASIC) at obu_tlayer_id 0, mlayer K = 0:
        // muh_layer_idc = LAYER_VALUES with muh_mlayer_map = 0b0000_0001 — only
        // the bit AT obu_mlayer_id, none above, so per the § 6.16.3 NOTE the unit
        // has NO explicit layer persistence indication.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 0),
            &[
                0x00,        // is_suffix=0, necessity=0, application_id=0
                0x00,        // metadata_unit_cnt_minus_1 = 0
                0x00,        // metadata_type = 0 (Reserved -> UnknownRaw, no unit bytes)
                0x08,        // muh_header_size = 4, cancel = 0
                0x00,        // muh_payload_size = 0
                0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
                0x00,        // priority lo + reserved bits
                0b0000_0001, // muh_mlayer_map: embedded layer 0 only
                0x80,        // OBU trailing byte
            ],
        ));
        let (context, _) = context_after_observing(&data);

        let cll_units = context.active_metadata_units(x0, 1);
        assert_eq!(cll_units.len(), 1);
        let record = &cll_units[0];
        // Temporal persistence to C = 1 within K = 0: [0][1][1] = 1 (default).
        assert!(context.metadata_applies_to(x0, record, m0, t1));
        // [0][0][1] = 0 (default: source T = 1 is not <= target C = 0). The
        // swapped argument order would read [0][1][0] = 1, so this assertion
        // pins the C/T wiring of applies_to.
        assert!(
            !context.metadata_applies_to(x0, record, m0, t0),
            "TLayerDependencyMap[K][C][T] must be consulted as [K][target][source]"
        );

        let values_units = context.active_metadata_units(x0, 0);
        assert_eq!(values_units.len(), 1);
        let record = &values_units[0];
        // muh_mlayer_map = 0b0000_0001 at K = 0 sets no bit ABOVE obu_mlayer_id,
        // so the unit has no explicit layer persistence indication (§ 6.16.3
        // NOTE) and never propagates upward, even though MLayerDependencyMap[2][0]
        // is signaled 1.
        assert!(!context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0));
        // It still applies at its own (K, T) source point.
        assert!(context.metadata_applies_to(x0, record, m0, t0));
    }

    #[test]
    fn propagation_requires_explicit_per_target_map_bit() {
        // AV2 § 6.16.3: "muh_mlayer_map contains a bitmask. The metadata unit is
        // intended for an embedded layer m if bit m of muh_mlayer_map is equal
        // to 1." and LAYER_VALUES means "The metadata applies to a set of
        // specific layer values, which are explicitly signaled." With the
        // § 5.4.1 default fill (mlayer_dependency_present_flag 0),
        // MLayerDependencyMap[1][0] and [2][0] are both 1, so the unit-level and
        // per-target readings of the multi-layer persistence bullet differ
        // exactly at embedded layer 1: bit 1 of muh_mlayer_map is clear, so the
        // metadata must NOT apply there despite the dependency, while the
        // explicitly targeted embedded layer 2 receives it.
        use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};
        let x0 = ExtendedLayerId::from_bits(0);
        let t0 = TemporalLayerId::from_bits(0);

        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 2)));
        // Group unit (HDR CLL, BASIC) at K = 0: muh_layer_idc = LAYER_VALUES
        // with muh_mlayer_map bits 0 and 2 set, bit 1 clear.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 0),
            &[
                0x00,        // is_suffix=0, necessity=0, application_id=0
                0x00,        // metadata_unit_cnt_minus_1 = 0
                0x01,        // metadata_type = HdrCll
                0x08,        // muh_header_size = 4, cancel = 0
                0x04,        // muh_payload_size = 4
                0x64,        // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
                0x00,        // priority lo + reserved bits
                0b0000_0101, // muh_mlayer_map: embedded layers 0 and 2, NOT 1
                0x12,
                0x34,
                0x56,
                0x78, // metadata_hdr_cll
                0x80, // OBU trailing byte
            ],
        ));
        let (context, _) = context_after_observing(&data);

        let units = context.active_metadata_units(x0, 1);
        assert_eq!(units.len(), 1);
        let record = &units[0];
        // Temporal persistence at the source point (K = 0, T = 0).
        assert!(context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(0), t0));
        // Bit 2 set + MLayerDependencyMap[2][0] == 1 -> applies to layer 2.
        assert!(
            context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(2), t0),
            "bit 2 of muh_mlayer_map is set and MLayerDependencyMap[2][0] is 1"
        );
        // Bit 1 clear -> layer 1 was never explicitly signaled, so the metadata
        // does not apply there even though MLayerDependencyMap[1][0] is 1.
        assert!(
            !context.metadata_applies_to(x0, record, EmbeddedLayerId::from_bits(1), t0),
            "bit 1 of muh_mlayer_map is clear, so embedded layer 1 is never \
             explicitly signaled (§ 6.16.3) despite MLayerDependencyMap[1][0] == 1"
        );
    }

    /// A 24-byte `metadata_hdr_mdcv()` unit (§ 5.17.6) with fixed chromaticities
    /// and the given `luminance_min`.
    fn hdr_mdcv_unit(luminance_min: u32) -> Vec<u8> {
        let mut unit = Vec::new();
        for v in [10u16, 20, 30, 40, 50, 60, 70, 80] {
            unit.extend_from_slice(&v.to_be_bytes());
        }
        unit.extend_from_slice(&1_000_000u32.to_be_bytes());
        unit.extend_from_slice(&luminance_min.to_be_bytes());
        unit
    }

    #[test]
    fn hdr_cll_repeat_same_content_accepted() {
        // AV2 § 6.16.5 allows redundant repeats: "Any additional metadata_hdr_cll
        // metadata units associated with an embedded layer in a coded video
        // sequence shall have the same content." Both units are global
        // LAYER_GLOBAL ("applies to all layers", § 6.16.3) with identical
        // content.
        let unit = [0x12, 0x34, 0x56, 0x78];
        let mut data = global_metadata_short_stream(&metadata_short_payload(0x11, 1, &unit));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x11, 1, &unit),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_repeat_content_differs_flagged() {
        // Two global LAYER_GLOBAL units share the all-layers association
        // (§ 6.16.3: "The metadata applies to all layers if obu_xlayer_id is
        // equal to GLOBAL_XLAYER_ID"), so the differing max_cll violates
        // § 6.16.5, emitted eagerly (same temporal unit). A third differing unit
        // with muh_layer_idc LAYER_UNSPECIFIED has no bitstream-derivable
        // association (§ 6.16.3: it "can potentially be indicated or determined
        // through external means") and is not compared against either.
        let mut data = global_metadata_short_stream(&metadata_short_payload(
            0x11,
            1,
            &[0x12, 0x34, 0x56, 0x78],
        ));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x11, 1, &[0x99, 0x99, 0x56, 0x78]),
        ));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x01, 1, &[0x00, 0x01, 0x56, 0x78]),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs")
                .count(),
            1,
            "exactly the intersecting differing repeat must be flagged: {report}"
        );
    }

    #[test]
    fn hdr_mdcv_repeat_content_differs_flagged() {
        // AV2 § 6.16.6 states the same-content rule identically for
        // metadata_hdr_mdcv; both units are global LAYER_GLOBAL.
        let mut data =
            global_metadata_short_stream(&metadata_short_payload(0x11, 2, &hdr_mdcv_unit(5)));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x11, 2, &hdr_mdcv_unit(9)),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/hdr-mdcv-repeat-content-differs"),
            "report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_after_cvs_restart_new_content_accepted() {
        // AV2 § 7.3.6: the CLK in temporal unit 2 starts a new coded video
        // sequence for xlayer 0, pruning the temporal-unit-1 baseline (a
        // LAYER_GLOBAL unit carried at xlayer 0 is associated with every
        // embedded layer of that extended layer, § 6.16.3); the different
        // content in the new coded video sequence is its own baseline, not a
        // § 6.16.5 violation.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_global_then_current_layer_differing_content_flagged() {
        // A global LAYER_GLOBAL unit "applies to all layers" (§ 6.16.3), so it
        // is associated with embedded layer (xlayer 0, mlayer 0); a later
        // LAYER_CURRENT unit for exactly that embedded layer with different
        // content violates § 6.16.5 ("Any additional metadata_hdr_cll metadata
        // units associated with an embedded layer in a coded video sequence
        // shall have the same content") even though the two units encode their
        // layer targeting differently.
        let mut data = global_metadata_short_stream(&metadata_short_payload(
            0x11,
            1,
            &[0x12, 0x34, 0x56, 0x78],
        ));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "a LAYER_CURRENT unit shares its embedded layer with the global \
             LAYER_GLOBAL baseline; report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_global_then_current_layer_same_content_accepted() {
        // The cross-mode twin with identical content: § 6.16.5 allows the
        // repeat.
        let unit = [0x12, 0x34, 0x56, 0x78];
        let mut data = global_metadata_short_stream(&metadata_short_payload(0x11, 1, &unit));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(metadata_short_obu_at(0, 0x21, 1, &unit));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_unspecified_layer_targeting_not_compared() {
        // LAYER_UNSPECIFIED (§ 6.16.3): "The current signaling does not specify
        // to what layers the metadata applies to. This information can
        // potentially be indicated or determined through external means." The
        // two units' real associations may be disjoint, so no § 6.16.5
        // comparison is derivable from the bitstream and differing content must
        // not be flagged (a documented false negative in the conservative
        // direction).
        let mut data = global_metadata_short_stream(&metadata_short_payload(
            0x01,
            1,
            &[0x12, 0x34, 0x56, 0x78],
        ));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x01, 1, &[0x99, 0x99, 0x56, 0x78]),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "LAYER_UNSPECIFIED units have no derivable association; report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_cross_mode_deferral_dropped_at_clk() {
        // The Universal baseline (global LAYER_GLOBAL, temporal unit 1)
        // intersects the LAYER_CURRENT unit for (xlayer 0, mlayer 0) in temporal
        // unit 2; the cross-temporal-unit comparison is deferred, tagged with
        // the intersection's concrete extended layer 0, and the CLK for xlayer 0
        // later in the same temporal unit starts a new coded video sequence
        // (§ 7.3.6), so the deferred § 6.16.5 finding is dropped.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(0x11, 1, &[0x12, 0x34, 0x56, 0x78]),
        ));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(temporal_delimiter_obu());
        data.extend(metadata_short_obu_at(0, 0x21, 1, &[0x99, 0x99, 0x56, 0x78]));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "the CLK puts the two units in different coded video sequences; \
             report was: {report}"
        );
    }

    /// A global (xlayer 31) group metadata OBU with one HDR CLL unit using
    /// `muh_layer_idc == LAYER_VALUES` explicit targeting: `muh_xlayer_map` is
    /// `xlayer_map` and one `muh_mlayer_map` byte (embedded layer 1) is emitted per
    /// selected extended layer (AV2 § 5.17.3).
    fn global_group_cll_obu(xlayer_map: u32, content: [u8; 4]) -> Vec<u8> {
        let map_count = xlayer_map.count_ones() as u8; // <= 32
        let mut payload = vec![
            0x00, // is_suffix=0, necessity=0, application_id=0
            0x00, // metadata_unit_cnt_minus_1 = 0
            0x01, // metadata_type = HdrCll
            // muh_header_size f(7): payload-size leb byte + 2 idc/priority bytes
            // + 4 muh_xlayer_map bytes + one muh_mlayer_map per selected layer.
            (1 + 2 + 4 + map_count) << 1, // cancel = 0
            0x04,                         // muh_payload_size = 4
            0x64,                         // layer_idc=3 (LAYER_VALUES), persistence=1 (BASIC)
            0x00,                         // priority lo + reserved bits
        ];
        payload.extend_from_slice(&xlayer_map.to_be_bytes()); // muh_xlayer_map
        // One muh_mlayer_map byte (embedded layer 1) per selected extended layer.
        payload.extend(std::iter::repeat_n(0b0000_0010u8, usize::from(map_count)));
        payload.extend_from_slice(&content); // metadata_hdr_cll()
        payload.push(0x80); // OBU trailing byte
        annex_b_obu_with_header(&layer_obu_header(9, 0, 0, 31), &payload)
    }

    #[test]
    fn hdr_cll_global_group_disjoint_layer_targeting_not_flagged() {
        // Two global group-form HDR CLL units with muh_layer_idc == LAYER_VALUES
        // explicitly target DISJOINT extended-layer sets via muh_xlayer_map
        // (§ 5.17.3). § 6.16.5 binds only units "associated with an embedded
        // layer in a coded video sequence"; the derived (xlayer, mlayer)
        // association sets share no embedded layer, so differing content must
        // NOT be flagged.

        // Single-layer disjoint targeting (xlayer 0 vs xlayer 1): the identical
        // single muh_mlayer_map byte would collide in a collapsed per-key
        // comparison.
        let mut data = temporal_delimiter_obu();
        data.extend(global_group_cll_obu(0b01, [0x12, 0x34, 0x56, 0x78]));
        data.extend(global_group_cll_obu(0b10, [0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "disjoint single-xlayer global targeting must not be compared; \
             report was: {report}"
        );

        // Multi-map disjoint targeting ({0,1} vs {2,3}).
        let mut data = temporal_delimiter_obu();
        data.extend(global_group_cll_obu(0b0011, [0x12, 0x34, 0x56, 0x78]));
        data.extend(global_group_cll_obu(0b1100, [0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "disjoint multi-xlayer global targeting must not be compared; \
             report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_global_group_overlapping_layer_targeting_flagged() {
        // Explicit LAYER_VALUES targeting {xlayer 0} vs {xlayer 0, xlayer 1}
        // (each selecting embedded layer 1, § 5.17.3) overlaps at the embedded
        // layer (0, 1), so differing content violates § 6.16.5; the finding
        // names the shared embedded layer.
        let mut data = temporal_delimiter_obu();
        data.extend(global_group_cll_obu(0b01, [0x12, 0x34, 0x56, 0x78]));
        data.extend(global_group_cll_obu(0b11, [0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        let finding = report
            .errors()
            .find(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs");
        assert!(
            finding.is_some_and(|finding| finding
                .message
                .contains("obu_xlayer_id 0 / obu_mlayer_id 1")),
            "overlapping global LAYER_VALUES targeting must be compared and the \
             finding must name a shared embedded layer; report was: {report}"
        );
        assert_eq!(
            report
                .errors()
                .filter(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs")
                .count(),
            1,
            "one differing baseline yields one finding; report was: {report}"
        );
    }

    #[test]
    fn hdr_cll_cross_tu_repeat_differs_flagged_at_flush() {
        // No CLK: temporal units 1 and 2 share xlayer 0's coded video sequence
        // (AV2 § 7.3.6), so the differing repeat violates § 6.16.5. The comparison
        // defers (a CLK could still arrive later in temporal unit 2) and the
        // end-of-stream flush emits it.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
        data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x12, 0x34, 0x56, 0x78]));
        data.extend(temporal_delimiter_obu());
        data.extend(metadata_short_obu_at(0, 0x11, 1, &[0x99, 0x99, 0x56, 0x78]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/hdr-cll-repeat-content-differs"),
            "a cross-temporal-unit repeat without a CLK stays in the same coded \
             video sequence and must be flagged; report was: {report}"
        );
    }

    #[test]
    fn metadata_truncated_observer_is_silent() {
        // A short metadata OBU whose metadataPayloadSize underflows (§ 5.17.2) and
        // a truncated group OBU: the stateful observer stays silent and leaves the
        // store unchanged — the stateless MetadataSyntax check owns the parse
        // error.
        use splot_core::annexb::parse_annex_b_obus;
        use splot_core::types::GLOBAL_XLAYER_ID;

        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &[0x00, 0x01],
        ));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(9, 0, 0, 31),
            &[0x00],
        ));
        let obus = parse_annex_b_obus(&data).unwrap_or_default();
        assert_eq!(obus.len(), 3, "the test stream must parse into 3 OBUs");

        let options = ValidationOptions::default();
        let mut report = ValidationReport::new();
        let mut context = ValidatorContext::default();
        for obu in &obus {
            context.observe_obu(obu, &options, &mut report);
        }
        assert!(
            report.diagnostics.is_empty(),
            "the observer must not report parse failures: {report}"
        );
        assert!(
            context
                .active_metadata_units(GLOBAL_XLAYER_ID, 1)
                .is_empty()
        );
    }

    // --- scan-type CVS consistency (AV2 § 6.16.10 Table 6.18) ---

    /// One `metadata_scan_type()` unit byte (AV2 § 5.17.10): `mps_pic_struct_type`
    /// `f(5)`, `mps_source_scan_type_idc` `f(2)` (0 here — no consistency rule
    /// binds it, § 6.16.10), `mps_duplicate_flag` `f(1)` (0).
    fn scan_type_unit(pic_struct: u8) -> [u8; 1] {
        [pic_struct << 3]
    }

    /// A global (xlayer 31) short metadata OBU carrying a scan-type unit (type 8);
    /// `first` selects prefix (`0x00`) or suffix (`0x80`) placement and carries
    /// `muh_layer_idc` / `muh_persistence_idc` 0.
    fn global_scan_type_obu(first: u8, pic_struct: u8) -> Vec<u8> {
        annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &metadata_short_payload(first, 8, &scan_type_unit(pic_struct)),
        )
    }

    /// An `OBU_CONTENT_INTERPRETATION` at the given layer ids carrying the given
    /// `ci_scan_type_idc` and optional timing (all other optional branches
    /// cleared), plus the § 5.2.1 extensible payload tail.
    fn content_interpretation_scan_obu_at(
        xlayer: u8,
        mlayer: u8,
        scan_type_idc: u32,
        timing: Option<CiTiming>,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(scan_type_idc, 2); // ci_scan_type_idc
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(u8::from(timing.is_some())); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        if let Some(t) = timing {
            bits.f(t.display_tick, 32);
            bits.f(t.time_scale, 32);
            bits.bit(u8::from(t.equal_picture_interval));
            if t.equal_picture_interval {
                bits.uvlc(t.num_ticks_minus_1);
            }
        }
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(24, 0, mlayer, xlayer), &bits.into_bytes())
    }

    /// [`content_interpretation_scan_obu_at`] at obu_xlayer_id 0 / obu_mlayer_id 0.
    fn content_interpretation_scan_obu(scan_type_idc: u32, timing: Option<CiTiming>) -> Vec<u8> {
        content_interpretation_scan_obu_at(0, 0, scan_type_idc, timing)
    }

    #[test]
    fn scan_type_group_mixing_in_cvs_flagged() {
        // AV2 § 6.16.10: "only one of the following conditions, for all pictures in
        // the current CVS, is true" — mps_pic_struct_type 0 (group {0, 7 or 8}) and
        // 3 (group {3, 4, 5 or 6}) in the same temporal unit mix two Table 6.18
        // groups within one coded video sequence (emitted eagerly: same temporal
        // unit means same coded video sequence, § 7.3.6).
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(global_scan_type_obu(0x00, 3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_group_mixing_across_tu_flagged() {
        // Temporal unit 2 has no CLK, so per AV2 § 7.3.6 it continues the coded
        // video sequence: the cross-temporal-unit group mix is deferred and emitted
        // by the end-of-stream flush.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(global_scan_type_obu(0x00, 3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            "a cross-temporal-unit group mix without a CLK stays in the same coded \
             video sequence and must be flagged; report was: {report}"
        );
    }

    #[test]
    fn scan_type_group_change_after_clk_accepted() {
        // Same stream, but temporal unit 2 contains a CLK: per AV2 § 7.3.6 the new
        // coded video sequence starts at the temporal unit, so mps_pic_struct_type
        // 3 belongs to the NEW coded video sequence and the deferred comparison
        // against the old sequence's group is dropped (no false positive).
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(global_scan_type_obu(0x00, 3));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            "a CLK in the temporal unit starts a new coded video sequence that the \
             group change joins; report was: {report}"
        );
    }

    #[test]
    fn scan_type_group_mixing_between_global_and_xlayer_scopes_flagged() {
        // Global scan-type metadata describes every layer's pictures, so a concrete
        // extended layer's group is checked against the global bucket's group.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(metadata_short_obu_at(0, 0x00, 8, &scan_type_unit(3)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_group_mixing_between_xlayer_and_global_scopes_flagged() {
        // Mirror of the pairing above: the concrete xlayer-0 scope establishes its
        // group baseline FIRST, and a later global-bucket unit of a different
        // Table 6.18 group must still be compared against that concrete scope
        // ("and vice versa") — the global unit is a suffix so it may follow the
        // coded-layer metadata OBU (§ 7.3.7).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(metadata_short_obu_at(0, 0x00, 8, &scan_type_unit(0)));
        data.extend(global_scan_type_obu(0x80, 3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            "a global unit must be compared against an existing concrete \
             extended-layer baseline; report was: {report}"
        );
    }

    #[test]
    fn scan_type_reserved_value_excluded_from_group_state() {
        // "Decoders shall ignore reserved values of mps_pic_struct_type"
        // (AV2 § 6.16.10): the reserved value 13 gets only its own stateless
        // diagnostic and never enters the group state, so the group baseline is 0
        // and exactly one group error fires for 0 vs 3.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 13));
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(global_scan_type_obu(0x00, 3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-pic-struct-reserved"),
            "report was: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
            1,
            "only 0 vs 3 may conflict (13 is excluded); report was: {report}"
        );
    }

    #[test]
    fn scan_type_ci_mismatch_flagged() {
        // Table 6.18: mps_pic_struct_type 3 requires "ci_scan_type_idc shall be
        // equal to 3", but the in-scope content interpretation establishes 1
        // (progressive). The scan-type metadata is a global suffix unit so it may
        // follow the coded-layer content interpretation OBU (§ 7.3.7).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(global_scan_type_obu(0x80, 3));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "report was: {report}"
        );

        // Accepted twin: mps_pic_struct_type 0 requires ci_scan_type_idc 1, which
        // matches the established value.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(global_scan_type_obu(0x80, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_ci_arrives_after_metadata_mismatch_flagged() {
        // Re-evaluation path: the scan-type metadata precedes the content
        // interpretation that decides its Table 6.18 restriction. A second
        // identical CI repeat must not re-report: its Table 6.18-decisive
        // content is unchanged, so the re-evaluation is skipped (§ 6.14 allows
        // exactly the identical repeat).
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 3));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(content_interpretation_scan_obu(1, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            1,
            "the mismatch is reported once, not per repeated CI; report was: {report}"
        );
    }

    #[test]
    fn scan_type_frame_doubling_requires_equal_picture_interval() {
        // Table 6.18 for mps_pic_struct_type 7 (frame doubling): "ci_scan_type_idc
        // shall be equal to 1 and equal_picture_interval shall be equal to 1".
        let unequal = CiTiming {
            equal_picture_interval: false,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, Some(unequal)));
        data.extend(global_scan_type_obu(0x80, 7));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(
                &report,
                "metadata/scan-type-equal-picture-interval-required"
            ),
            "report was: {report}"
        );

        // Accepted with equal_picture_interval == 1.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, Some(BASE_TIMING)));
        data.extend(global_scan_type_obu(0x80, 7));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(
                &report,
                "metadata/scan-type-equal-picture-interval-required"
            ),
            "report was: {report}"
        );

        // Silent when timing_info() is absent: the mirror attaches the restriction
        // to the signaled element and states no absent-timing rule (documented).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(global_scan_type_obu(0x80, 7));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(
                &report,
                "metadata/scan-type-equal-picture-interval-required"
            ),
            "absent timing_info must stay silent; report was: {report}"
        );
    }

    #[test]
    fn scan_type_without_ci_warns_unestablished_at_eos() {
        // Derived literal reading of Table 6.18 (AV2 § 6.16.10): every defined
        // mps_pic_struct_type restricts ci_scan_type_idc to 1, 2 or 3, while the
        // § 7.3.8.11 default — in effect when no content interpretation OBU is
        // present — is "ci_scan_type_idc = 0 (unspecified)", which satisfies no
        // row. Warning severity, never an error.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_warning(&report, "metadata/scan-type-ci-scan-type-unestablished"),
            "report was: {report}"
        );
        assert!(
            report.is_conformant(),
            "the unestablished case is a warning, not an error: {report}"
        );

        // Negative twin: an in-scope content interpretation established a non-zero
        // ci_scan_type_idc, so the coded video sequence flushes without a warning.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(global_scan_type_obu(0x80, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_warning(&report, "metadata/scan-type-ci-scan-type-unestablished"),
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_unestablished_warned_at_cvs_restart() {
        // A CLK ends the coded video sequence (AV2 § 7.3.6), retiring the global
        // bucket's scan-type observations: the unestablished-CI warning fires at
        // the restart, not only at the end of the stream, and exactly once (the
        // retired scope leaves nothing for the end-of-stream flush).
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .warnings()
                .filter(|d| d.rule_id == "metadata/scan-type-ci-scan-type-unestablished")
                .count(),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_ci_for_second_embedded_layer_rechecked() {
        // § 6.14 allows different embedded layers to carry different
        // ci_scan_type_idc ("No such constraint exists for content
        // interpretation OBUs in different embedded layers" beyond timing), so a
        // stream with only conforming CI OBUs can establish a matching value at
        // mlayer 0 and a mismatching one at mlayer 1: the later CI must still be
        // paired with the stored observation.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
        data.extend(content_interpretation_scan_obu_at(0, 1, 2, None)); // mismatch
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            1,
            "the mlayer-1 content interpretation must be paired with the stored \
             observation; report was: {report}"
        );
        assert!(
            !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
            "different embedded layers are distinct CI records (§ 6.14); \
             report was: {report}"
        );
    }

    #[test]
    fn scan_type_ci_mismatch_on_second_xlayer_rechecked() {
        // The global scan-type bucket pairs with every extended layer's CI
        // records, and § 6.14 leaves cross-extended-layer CI content
        // unconstrained (timing aside): a matching CI on xlayer 0 must not stop
        // the later mismatching CI on xlayer 1 from being paired.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
        data.extend(sequence_header_obu_for_xlayer(0, 0, 1));
        data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
        data.extend(sequence_header_obu_for_xlayer(1, 0, 1));
        data.extend(content_interpretation_scan_obu_at(1, 0, 2, None)); // mismatch
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "the xlayer-1 content interpretation must be paired with the global \
             observation; report was: {report}"
        );
    }

    #[test]
    fn scan_type_equal_picture_interval_rechecked_for_second_layer() {
        // mps_pic_struct_type 7 (Table 6.18: "ci_scan_type_idc shall be equal to
        // 1 and equal_picture_interval shall be equal to 1"): the first CI
        // (matching scan type, no timing) must not stop the later mlayer-1 CI —
        // whose timing_info() signals equal_picture_interval 0 — from being
        // paired with the stored observation.
        let unequal = CiTiming {
            equal_picture_interval: false,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 7));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu_at(0, 0, 1, None));
        data.extend(content_interpretation_scan_obu_at(0, 1, 1, Some(unequal)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(
                &report,
                "metadata/scan-type-equal-picture-interval-required"
            ),
            "report was: {report}"
        );
    }

    #[test]
    fn scan_type_contradicting_ci_repeat_rechecked_and_co_reported() {
        // A same-key CI repeat with different information is itself
        // non-conforming (§ 6.14, content-interpretation/repeated-ci-not-identical)
        // AND its changed ci_scan_type_idc violates the stored observation's
        // Table 6.18 restriction — distinct rules from distinct spec sections,
        // so both are reported.
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
        data.extend(content_interpretation_scan_obu_at(0, 0, 2, None)); // contradiction
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "content-interpretation/repeated-ci-not-identical"),
            "report was: {report}"
        );
        assert_eq!(
            ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            1,
            "the changed decisive content must be re-paired exactly once; \
             report was: {report}"
        );
    }

    // --- § 7.3.8.11 CI-parameter epoch at random access points (CLK / OLK) ---

    /// An `OBU_OPEN_LOOP_KEY` for xlayer 0 with an empty payload (the raw
    /// OBU-header event is all the § 7.3.8.11 epoch tracking consumes).
    fn open_loop_key_obu() -> Vec<u8> {
        annex_b_obu(0x14, &[])
    }

    #[test]
    fn scan_type_pre_olk_ci_not_paired_with_post_olk_metadata() {
        // § 7.3.8.11: the content interpretation parameters re-initialize to
        // defaults (ci_scan_type_idc = 0, unspecified) "at each temporal unit
        // containing an OBU in the extended layer with obu_type equal to
        // OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY". After the OLK with no
        // re-sent CI, the parameters the metadata's pictures see are the
        // defaults — never an error (the unestablished case is warning-only) —
        // so pairing the pre-OLK ci_scan_type_idc 2 against mps_pic_struct_type
        // 0 would be a false positive.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        data.extend(content_interpretation_scan_obu(2, None));
        data.extend(temporal_delimiter_obu());
        data.extend(open_loop_key_obu());
        data.extend(temporal_delimiter_obu());
        data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "the pre-OLK content interpretation no longer establishes the \
             parameters (§ 7.3.8.11); report was: {report}"
        );
    }

    #[test]
    fn scan_type_ci_resent_at_olk_pairs_with_post_olk_metadata() {
        // A CI OBU present in the random access point's own temporal unit
        // re-establishes the parameters (§ 7.3.8.11 step 2), so the Table 6.18
        // pairing fires for post-OLK metadata; the identical re-send is also not
        // a § 6.14 repeated-CI violation.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
        data.extend(content_interpretation_scan_obu(2, None));
        data.extend(temporal_delimiter_obu());
        data.extend(open_loop_key_obu());
        data.extend(content_interpretation_scan_obu(2, None)); // re-sent at the OLK
        data.extend(global_scan_type_obu(0x80, 0)); // suffix; requires idc 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "a CI re-sent at the OLK re-establishes ci_scan_type_idc 2 for the \
             new epoch; report was: {report}"
        );
        assert!(
            !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
            "the identical re-send is a legal § 6.14 repeat; report was: {report}"
        );
    }

    #[test]
    fn scan_type_pre_olk_metadata_not_paired_with_olk_tu_ci() {
        // The complementary direction: a pre-OLK picture's parameters belong to
        // the previous § 7.3.8.11 epoch, so a CI in the OLK's temporal unit must
        // not be paired with the earlier observation — in either
        // same-temporal-unit order (a CI before the OLK defers the pairing,
        // which the OLK then drops; a CI after the OLK is epoch-skipped).
        for ci_before_olk in [true, false] {
            let mut data = temporal_delimiter_obu();
            data.extend(global_scan_type_obu(0x00, 0)); // requires idc 1
            data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
            data.extend(temporal_delimiter_obu());
            if ci_before_olk {
                data.extend(content_interpretation_scan_obu(2, None));
                data.extend(open_loop_key_obu());
            } else {
                data.extend(open_loop_key_obu());
                data.extend(content_interpretation_scan_obu(2, None));
            }
            let report = Validator::new(false).validate_bytes(&data);
            assert!(
                !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
                "ci_before_olk={ci_before_olk}: the observation predates the \
                 OLK's § 7.3.8.11 epoch; report was: {report}"
            );
        }
    }

    #[test]
    fn repeated_ci_differs_across_olk_still_flagged() {
        // § 6.14 / § 7.3.8.10 scope the repeated-CI identity rule to the coded
        // video sequence ("all instances of a content interpretation OBU in an
        // embedded layer within a coded video sequence shall contain the same
        // information"), and an OLK does not start one during sequential
        // decoding (§ 7.4.4): the differing repeat across the OLK is still
        // flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_scan_obu(1, None));
        data.extend(temporal_delimiter_obu());
        data.extend(open_loop_key_obu());
        data.extend(content_interpretation_scan_obu(2, None));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "content-interpretation/repeated-ci-not-identical"),
            "the § 6.14 identity rule is CVS-scoped, not RAP-scoped; \
             report was: {report}"
        );
    }

    #[test]
    fn ci_timing_mismatch_across_olk_still_flagged() {
        // § 6.4.12 binds the timing values "within a coded video sequence ...
        // across all embedded layers"; the OLK is not a CVS boundary during
        // sequential decoding (§ 7.4.4), so the cross-embedded-layer mismatch
        // across it is still flagged.
        let other = CiTiming {
            time_scale: 60000,
            ..BASE_TIMING
        };
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
        data.extend(temporal_delimiter_obu());
        data.extend(open_loop_key_obu());
        data.extend(content_interpretation_obu(1, 0, Some(other)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "sequence-header/timing-time-scale-mismatch"),
            "the § 6.4.12 timing rule is CVS-scoped, not RAP-scoped; \
             report was: {report}"
        );
    }

    // --- metadata temporal-unit ordering (AV2 § 6.16.3 / § 7.3.7) ---

    #[test]
    fn metadata_prefix_global_after_coded_layer_is_flagged() {
        // Global prefix metadata (metadata_is_suffix == 0) after a coded extended layer
        // unit is a § 7.3.7 prefix-after-coded-layer violation.
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
        // first byte 0x08 = is_suffix 0, cancel 1.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &[0x08, 0x04, 0x80],
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "obu-order/global-hls-after-coded-layer"),
            "report was: {report}"
        );
    }

    #[test]
    fn metadata_suffix_global_after_coded_layer_is_not_treated_as_prefix() {
        // Global suffix metadata (metadata_is_suffix == 1) after a coded layer is NOT a
        // global prefix, so it must not be flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
        // first byte 0x88 = is_suffix 1, cancel 1.
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 31),
            &[0x88, 0x04, 0x80],
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "obu-order/global-hls-after-coded-layer"),
            "global suffix metadata must not be treated as a prefix; report was: {report}"
        );
    }

    #[test]
    fn metadata_non_global_order_uses_coded_xlayer_order() {
        // Non-global metadata participates in the coded extended layer ascending order:
        // after coded layers at xlayer 0 then 1, a metadata OBU at xlayer 0 is out of
        // order.
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        // A cancelled short metadata OBU at xlayer 0 (active sequence 0 is present).
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(8, 0, 0, 0),
            &[0x08, 0x04, 0x80],
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "obu-order/xlayer-order-not-ascending"),
            "report was: {report}"
        );
    }

    // ----- Layer-dependency-map agreement (layer-dependency-map-agreement) -----

    /// A base-layer (xlayer 0) sequence header payload with `max_tlayer_id == 1`,
    /// `max_mlayer_id == 1`, and a signaled mlayer dependency map that *clears*
    /// `MLayerDependencyMap[1][0]` (embedded layer 1 does not depend on layer 0),
    /// overriding the § 5.4.1 lower-triangular default fill.
    fn sequence_header_payload_mlayer_dep_cleared(seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(1, 2); // max_tlayer_id
        bits.f(1, 3); // max_mlayer_id
        bits.f(1, 1); // seq_max_mlayer_cnt_minus_1 -> SeqMaxMlayerCnt = 2 (layers 0, 1)
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(1); // mlayer_dependency_present_flag
        // § 5.4.1 signaled order: currLayer 1, refLayer descending 1 -> 0.
        bits.bit(1); // mlayer_dependency_map -> MLayerDependencyMap[1][1] = 1
        bits.bit(0); // mlayer_dependency_map -> MLayerDependencyMap[1][0] = 0
        bits.bit(0); // tlayer_dependency_present_flag
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A local OPS OBU at `xlayer` (`ops_cnt == 1`) whose single payload carries an
    /// explicit `ops_mlayer_info()` with the given maps. `tlayer_maps` holds one
    /// `ops_tlayer_map` per set bit of `mlayer_map`, in ascending set-bit order.
    fn local_ops_mlayer_obu(
        xlayer: u8,
        ops_id: u32,
        mlayer_map: u8,
        tlayer_maps: &[u8],
    ) -> Vec<u8> {
        assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(ops_id, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_reserved_2bits
        let mut body = Bits::default();
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(u32::from(mlayer_map), 8); // ops_mlayer_map
        for &tlayer_map in tlayer_maps {
            body.f(u32::from(tlayer_map), 4); // ops_tlayer_map
        }
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(
            &layer_obu_header(18, 0, 0, xlayer),
            &finish_extensible(bits),
        )
    }

    /// A global OPS OBU (`ops_cnt == 1`, `ops_mlayer_info_idc == 1`) whose single
    /// included extended layer `target_xlayer` carries an explicit
    /// `ops_mlayer_info()` with the given maps.
    fn global_ops_explicit_obu(
        ops_id: u32,
        target_xlayer: u8,
        mlayer_map: u8,
        tlayer_maps: &[u8],
    ) -> Vec<u8> {
        assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(ops_id, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(1, 2); // ops_mlayer_info_idc = 1 -> explicit mlayer info per layer
        let mut body = Bits::default();
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(1u32 << target_xlayer, 31); // ops_xlayer_map
        body.f(u32::from(mlayer_map), 8); // ops_mlayer_map
        for &tlayer_map in tlayer_maps {
            body.f(u32::from(tlayer_map), 4); // ops_tlayer_map
        }
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
    }

    /// Appends a § 5.8.8 `lcr_embedded_layer_info()` block: per set bit `j` of
    /// `mlayer_map` (ascending) a TEXTURE layer with the given `lcr_tlayer_map`, an
    /// all-zero `lcr_dependent_layer_map` (when `j > 0`), the same-resolution flag
    /// set, and the per-iteration `byte_alignment()`.
    fn append_lcr_embedded_layer_info(bits: &mut Bits, mlayer_map: u8, tlayer_maps: &[u8]) {
        assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
        bits.f(u32::from(mlayer_map), 8); // lcr_mlayer_map
        let mut next_tlayer = 0usize;
        for j in 0u8..8 {
            if mlayer_map & (1u8 << j) == 0 {
                continue;
            }
            bits.f(u32::from(tlayer_maps[next_tlayer]), 4); // lcr_tlayer_map
            next_tlayer += 1;
            bits.f(0, 8); // lcr_layer_type = TEXTURE_LAYER
            bits.f(0, 8); // lcr_view_type = VIEW_UNSPECIFIED
            if j > 0 {
                bits.f(0, u32::from(j)); // lcr_dependent_layer_map
            }
            bits.bit(1); // lcr_same_sh_max_resolution_flag
            bits.align(); // byte_alignment()
        }
    }

    /// A local LCR OBU at `xlayer` (`lcr_global_id == 0`) carrying embedded-layer
    /// info with the given maps.
    fn local_lcr_obu_with_embedded(
        xlayer: u8,
        local_id: u32,
        mlayer_map: u8,
        tlayer_maps: &[u8],
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(local_id, 3); // lcr_local_id
        bits.bit(0); // lcr_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_local_atlas_id_present_flag
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        // lcr_xlayer_info(0, xId)
        bits.bit(0); // lcr_rep_info_present_flag
        bits.bit(0); // lcr_xlayer_purpose_present_flag
        bits.bit(0); // lcr_xlayer_color_info_present_flag
        bits.bit(1); // lcr_embedded_layer_info_present_flag
        bits.align(); // byte_alignment()
        append_lcr_embedded_layer_info(&mut bits, mlayer_map, tlayer_maps);
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and
    /// whose single global payload carries embedded-layer info with the given maps.
    fn global_lcr_obu_with_embedded(
        global_id: u32,
        target_xlayer: u8,
        mlayer_map: u8,
        tlayer_maps: &[u8],
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(1u32 << target_xlayer, 31); // lcr_xlayer_map
        bits.bit(0); // lcr_aggregate_info_present_flag
        bits.bit(0); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(1); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(0); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        // One lcr_global_payload for target_xlayer: lcr_xlayer_info(1, xId).
        let mut body = Bits::default();
        body.bit(0); // lcr_rep_info_present_flag
        body.bit(0); // lcr_xlayer_purpose_present_flag
        body.bit(0); // lcr_xlayer_color_info_present_flag
        body.bit(1); // lcr_embedded_layer_info_present_flag
        body.align(); // byte_alignment()
        append_lcr_embedded_layer_info(&mut body, mlayer_map, tlayer_maps);
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // lcr_global_data_size (single-byte leb128)
        bits.bits.extend_from_slice(&body.bits);
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A multi-frame header OBU at `(tlayer, mlayer)` on xlayer 0 referencing
    /// `seq_header_id` (`mfh_id_minus_1 == 0`, so `mfhId == 1`).
    fn multi_frame_header_obu_with_layers(seq_header_id: u32, tlayer: u8, mlayer: u8) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id); // mfh_seq_header_id
        bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
        bits.bit(0); // mfh_frame_size_present_flag
        bits.bit(0); // mfh_deblocking_filter_update
        bits.bit(0); // mfh_seg_info_present_flag
        bits.bit(0); // obu_extension_flag = 0
        bits.bit(1); // trailing_one_bit
        annex_b_obu_with_header(&layer_obu_header(3, tlayer, mlayer, 0), &bits.into_bytes())
    }

    /// A CLK frame-bearing OBU at `(tlayer, mlayer)` on xlayer 0 whose frame header
    /// references the multi-frame header `cur_mfh_id`.
    fn frame_obu_mfh_ref_with_layers(tlayer: u8, mlayer: u8, cur_mfh_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(1); // is_first_tile_group
        bits.uvlc(cur_mfh_id); // cur_mfh_id > 0
        annex_b_obu_with_header(&layer_obu_header(4, tlayer, mlayer, 0), &bits.into_bytes())
    }

    /// A global OPS OBU (`ops_cnt == 1`, `ops_mlayer_info_idc == 2`) whose single
    /// included extended layer 0 *inherits* its mlayer info from
    /// `(embedded_ops_id, embedded_op_index)`. Cross-OPS inheritance keeps the
    /// layer-0 entry legal (same-OPS layer-0 inheritance is always out of range).
    fn global_ops_layer0_inherited_obu(
        ops_id: u32,
        embedded_ops_id: u32,
        embedded_op_index: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(ops_id, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(2, 2); // ops_mlayer_info_idc = 2 -> explicit-or-inherited per layer
        let mut body = Bits::default();
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(0b1, 31); // ops_xlayer_map -> layer 0 only
        body.bit(0); // layer 0: ops_mlayer_explicit_info_flag = 0 -> inherited
        body.f(embedded_ops_id, 4); // ops_embedded_ops_id
        body.f(embedded_op_index, 3); // ops_embedded_op_index
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
    }

    #[test]
    fn ops_mlayer_dependency_missing_is_flagged() {
        // Default § 5.4.1 maps with max_mlayer_id 1: MLayerDependencyMap[1][0] == 1,
        // but the OPS includes embedded layer 1 without layer 0.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "ops/mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_tlayer_dependency_missing_is_flagged() {
        // Default maps with max_tlayer_id 1: TLayerDependencyMap[0][1][0] == 1, but
        // the OPS tlayer map for embedded layer 0 includes tlayer 1 without tlayer 0.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b1, &[0b10]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "ops/tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_dependency_closed_maps_are_not_flagged() {
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b11, &[0b11, 0b11]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "ops/mlayer-dependency-missing")
                && !has_error(&report, "ops/tlayer-dependency-missing"),
            "dependency-closed maps must be silent; report was: {report}"
        );
    }

    #[test]
    fn ops_dependency_respects_signaled_mlayer_map() {
        // The activated header *clears* MLayerDependencyMap[1][0], so an OPS that
        // includes embedded layer 1 without layer 0 agrees with the signaled map.
        // This must consult the signaled § 5.4.1 override, not the default fill.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_mlayer_dep_cleared(0),
        ));
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "ops/mlayer-dependency-missing"),
            "a signaled non-dependency must not be flagged; report was: {report}"
        );
    }

    #[test]
    fn ops_before_sequence_header_is_checked_once_at_activation() {
        // The OPS precedes any sequence header; the later sequence header becomes
        // active for xlayer 0 (OBU-order fallback) and the stored OPS maps are then
        // checked exactly once.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1)));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn ops_without_activated_sequence_header_is_not_checked() {
        // No sequence header ever activates; the maps must not be fabricated.
        let mut data = temporal_delimiter_obu();
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "ops/mlayer-dependency-missing")
                && !has_error(&report, "ops/tlayer-dependency-missing"),
            "no activated sequence header means no agreement check; report was: {report}"
        );
    }

    #[test]
    fn ops_dependency_check_suppressed_under_external_sequence_headers() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // External HLS declares a sequence header, so an externally activated header
        // with unmodeled maps may govern: the agreement check must not fire.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "ops/mlayer-dependency-missing"),
            "external sequence headers must suppress the check; report was: {report}"
        );
    }

    #[test]
    fn ops_inherited_mlayer_info_is_not_dependency_checked() {
        // The source OPS 1's own explicit maps violate the activated (xlayer 0)
        // header and are flagged once at OPS 1's observation. OPS 0's layer-0 entry
        // — on the *activated* extended layer — inherits from OPS 1; § 6.10.7 binds
        // the maps "if present", so the inheriting entry adds no second finding
        // even though resolving the inheritance would reach violating maps.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(global_ops_explicit_obu(1, 0, 0b10, &[0b1])); // flagged once
        data.extend(global_ops_layer0_inherited_obu(0, 1, 0)); // inherits OPS 1 op 0
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            1,
            "only the source OPS's explicit maps may be flagged; report was: {report}"
        );
        assert!(
            !has_error(&report, "ops/tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_global_explicit_entry_mlayer_dependency_missing_is_flagged() {
        // A global OPS entry for extended layer 0 is checked against the sequence
        // header activated for that entry's extended layer.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(global_ops_explicit_obu(0, 0, 0b10, &[0b1]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "ops/mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn ops_dependency_not_duplicated_across_reactivation() {
        // The observation-side check fires once; a frame header re-activating the
        // same sequence header must not re-emit the finding.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_mlayer_dependency_missing_is_flagged() {
        // The sequence header activates for xlayer 0 and its seq_lcr_id resolves to the
        // local LCR, whose lcr_mlayer_map[0][0] includes embedded layer 1 without layer 0
        // against default MLayerDependencyMap[1][0]. A CLK frame referencing seq id 0
        // frame-confirms the activation (the § 6.8.9 check uses the strict
        // frame-confirmed gate, no sole-header fallback).
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_tlayer_dependency_missing_is_flagged() {
        // The activated global LCR's lcr_tlayer_map[1][3][0] includes tlayer 1
        // without tlayer 0 against the default TLayerDependencyMap[0][1][0]. A CLK frame
        // on xlayer 3 referencing seq id 0 frame-confirms the activation.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_embedded(5, 3, 0b1, &[0b10]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(1, 0, 0, 3),
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_dependency_closed_maps_are_not_flagged() {
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "dependency-closed maps must be silent; report was: {report}"
        );
    }

    #[test]
    fn lcr_without_seq_lcr_reference_is_not_checked() {
        // seq_lcr_id == 0: no LCR is associated, so no pairing exists to check.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "an unreferenced LCR must not be checked; report was: {report}"
        );
    }

    #[test]
    fn lcr_dependency_diagnostic_points_at_lcr_obu() {
        // The activating sequence header is not the violator: the diagnostic must
        // carry the LCR OBU's offset, which precedes the sequence header here. The CLK
        // frame appended after the header frame-confirms the activation without moving
        // the LCR or sequence-header offsets.
        let td = temporal_delimiter_obu();
        let lcr = local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]);
        let seq_start = (td.len() + lcr.len()) as u64;
        let mut data = td;
        data.extend(lcr);
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        let offsets: Vec<_> = report
            .errors()
            .filter(|d| d.rule_id == "lcr/mlayer-dependency-missing")
            .map(|d| d.byte_offset)
            .collect();
        assert!(
            matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
            "exactly one diagnostic pointing at the LCR OBU (before byte {seq_start}) was \
             expected; report was: {report}"
        );
    }

    #[test]
    fn lcr_dependency_not_duplicated_across_reactivation() {
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/mlayer-dependency-missing"),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_mlayer_dependency_missing_is_flagged() {
        // The frame (mlayer 0) references an MFH recorded at mlayer 1:
        // MLayerDependencyMap[0][1] == 0 under the default fill (§ 6.17.2).
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(multi_frame_header_obu_with_layers(0, 0, 1));
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "frame-header/mfh-mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_tlayer_dependency_missing_is_flagged() {
        // The frame (tlayer 0) references an MFH recorded at tlayer 1:
        // TLayerDependencyMap[0][0][1] == 0 under the default fill (§ 6.17.2).
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(multi_frame_header_obu_with_layers(0, 1, 0));
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_satisfied_layer_dependencies_are_not_flagged() {
        // A frame at (tlayer 1, mlayer 1) depends on the MFH's (tlayer 0, mlayer 0)
        // under the default lower-triangular fills.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(multi_frame_header_obu(0));
        data.extend(frame_obu_mfh_ref_with_layers(1, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
                && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
            "satisfied dependencies must be silent; report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_unavailable_is_not_layer_checked() {
        // No MFH resolves: the availability diagnostic owns the case and the
        // layer-dependency rules stay silent.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 2));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "hls/unavailable-multi-frame-header"),
            "report was: {report}"
        );
        assert!(
            !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
                && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
            "an unresolved MFH must not be layer-checked; report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_external_sequence_header_is_not_layer_checked() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // The MFH's sequence header resolves only externally; its maps are not
        // modeled, so the layer-dependency rules stay silent.
        let mut data = temporal_delimiter_obu();
        data.extend(multi_frame_header_obu_with_layers(5, 0, 1));
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
                && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
            "an externally resolved sequence header must not be layer-checked; report was: {report}"
        );
    }

    #[test]
    fn frame_mfh_unresolvable_sequence_header_is_not_layer_checked() {
        // The MFH's mfh_seq_header_id resolves to nothing under default options:
        // the availability diagnostics own the case and the layer-dependency rules
        // stay silent (no maps to check).
        let mut data = temporal_delimiter_obu();
        data.extend(multi_frame_header_obu_with_layers(5, 0, 1));
        data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "hls/unavailable-sequence-header"),
            "report was: {report}"
        );
        assert!(
            !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
                && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
            "an unresolvable sequence header must not be layer-checked; report was: {report}"
        );
    }

    #[test]
    fn ops_deferred_check_fires_on_frame_activation_change() {
        // The OPS is conformant under the initially active header 0 (whose signaled
        // map clears MLayerDependencyMap[1][0]) — silent at observation. A CLK then
        // re-activates xlayer 0 to header 1 (default maps), and the frame-driven
        // activation hook must evaluate the stored OPS maps against the new header
        // and emit exactly one finding.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_mlayer_dep_cleared(0),
        ));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            1,
            "the frame-driven re-activation must evaluate stored OPS maps; report was: {report}"
        );
    }

    #[test]
    fn ops_disagreement_reemitted_after_sequence_header_redefinition() {
        // The OPS disagrees with header 0 (flagged once). Re-sending header 0 with
        // changed agreement inputs (max_tlayer_id 1 -> 0 changes the default
        // TLayerDependencyMap) invalidates the id's dedup keys and re-fires the
        // checks: the still-disagreeing mlayer map is reported against the
        // redefined content too.
        let mut data = td_and_seq_header(0, 1, 1);
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            2,
            "a same-id redefinition must re-fire the agreement checks; report was: {report}"
        );
    }

    #[test]
    fn lcr_local_tlayer_dependency_missing_is_flagged() {
        // Local LCR × tlayer map: lcr_tlayer_map[0][0][0] includes temporal layer 1
        // without temporal layer 0 against the default TLayerDependencyMap[0][1][0]. The
        // CLK frame referencing seq id 0 frame-confirms the xlayer-0 activation.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b1, &[0b10]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_global_mlayer_dependency_missing_is_flagged() {
        // Global LCR × mlayer map: lcr_mlayer_map[1][3] includes embedded layer 1
        // without embedded layer 0 against the default MLayerDependencyMap[1][0]. A CLK
        // frame on xlayer 3 referencing seq id 0 frame-confirms the activation.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_embedded(5, 3, 0b10, &[0b1]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(1, 0, 0, 3),
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_local_record_takes_precedence_over_global() {
        // § 6.4.1 resolution order: with both a dependency-closed local LCR and a
        // violating global LCR carrying the same id, the local record is the
        // associated one, so no finding may be emitted.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
        data.extend(global_lcr_obu_with_embedded(5, 0, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "the local LCR resolves first (§ 6.4.1); report was: {report}"
        );
    }

    #[test]
    fn lcr_unresolved_nonzero_seq_lcr_id_is_not_dependency_checked() {
        // seq_lcr_id != 0 resolving to no in-band LCR: the § 7.3.8.3 availability
        // diagnostic owns the case; no dependency finding can exist without maps.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "hls/unavailable-layer-configuration-record"),
            "report was: {report}"
        );
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "an unresolved seq_lcr_id must not be dependency-checked; report was: {report}"
        );
    }

    #[test]
    fn lcr_after_sequence_header_is_not_paired() {
        // § 6.4.1 associates a sequence header only with an LCR "present prior to
        // this sequence header"; a later-arriving violating LCR must not be
        // retroactively paired with the earlier activation (the § 7.3.8.3
        // availability diagnostic already owns this stream).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "a later LCR must not pair with an earlier activation; report was: {report}"
        );
    }

    #[test]
    fn lcr_redefined_without_embedded_info_is_not_checked() {
        // A redefinition of local LCR 5 without embedded-layer info replaces the
        // stored maps wholesale; the activation must see the latest (map-less)
        // definition, not the superseded violating maps.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(local_lcr_obu(0, 0, 5, None)); // redefinition, no embedded info
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "stale maps from a superseded definition must not be checked; report was: {report}"
        );
    }

    #[test]
    fn lcr_dependency_check_suppressed_under_external_hls_provided() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // The § 6.8.9 closure pairs the in-band activated header against the LCR its
        // seq_lcr_id resolves to under § 6.4.1 (local-first). A Provided declaration is
        // PARTIAL (`ExternalHlsMode::Provided` — it cannot enumerate external LCRs), so an
        // unmodeled external *local* LCR with this seq_lcr_id could win the resolution
        // ahead of the in-band record; the in-band association may not be the activated
        // one, so the check is suppressed under ANY Provided mode (even an empty set) to
        // avoid a false positive — the same local-first-shadowing reasoning as the
        // lcr/global-xlayer-map-missing-xlayer gate. The stream WOULD fire under Disabled
        // (the trailing CLK frame frame-confirms the activation), confirming the
        // suppression is the only reason it is silent here.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        // Sanity: under Disabled this in-band violation fires.
        let baseline = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&baseline, "lcr/mlayer-dependency-missing"),
            "the in-band violation must fire under Disabled; report was: {baseline}"
        );
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "any Provided external HLS must suppress the association-dependent LCR \
             dependency check (an unmodeled external local LCR could shadow the in-band \
             association); report was: {report}"
        );
    }

    #[test]
    fn lcr_dependency_uses_strict_frame_confirmation() {
        // Finding-1 regression (codex 3393669703): the § 6.8.5 / § 6.8.8 / § 6.8.9 LCR
        // agreement checks must use the STRICT frame-confirmed gate — never the
        // sole-in-band-header OBU-order fallback — so they fire only against a frame-loaded
        // activation, matching the Annex A value-space precedent. A sole staged header with
        // NO frame is a guess (§ 7.3.6 permits staging), so the dependency check stays
        // silent until a frame confirms the activation.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        // Sole staged header, NO frame: strict gate keeps the check silent.
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "a sole staged header (no frame) must not fire the LCR dependency check via the \
             sole-header fallback; report was: {report}"
        );
        // Adding a frame that loads the staged header confirms the activation -> fires.
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/mlayer-dependency-missing"),
            "the frame-confirmed activation must fire the LCR dependency check; report was: \
             {report}"
        );
    }

    #[test]
    fn lcr_ptl_uses_strict_frame_confirmation() {
        // Finding-1 regression for § 6.8.5: a sole staged header (no frame) is silent;
        // the frame-confirmed activation fires.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8, // > lcr_max_level_idx 4
            tier: 0,
            max_mlayer_id: 0,
        }));
        assert!(
            !has_error(
                &Validator::new(false).validate_bytes(&data),
                "lcr/ptl-level-exceeds-max"
            ),
            "a sole staged header (no frame) must not fire the § 6.8.5 ceiling via the \
             sole-header fallback"
        );
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        assert!(
            has_error(
                &Validator::new(false).validate_bytes(&data),
                "lcr/ptl-level-exceeds-max"
            ),
            "the frame-confirmed activation must fire the § 6.8.5 ceiling"
        );
    }

    #[test]
    fn lcr_agreement_silent_when_external_header_could_be_the_activator() {
        // Finding-1 regression (codex 3393669703), the worst case the strict gate guards:
        // an external sequence header is DECLARED, and an in-band header is staged but NO
        // frame has loaded it. The OBU-order sole-header fallback would guess the staged
        // in-band header is active and fire the LCR checks against it — but the real
        // activated header could be the external one, so firing would be a false positive.
        // The checks must stay silent. (They also stay silent WITH a confirming frame here,
        // because any Provided mode suppresses the association-dependent LCR checks per the
        // partial-declaration policy — both paths are silent, neither fires against a guess.)
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(9),
            ),
        };
        // No frame: the strict gate alone keeps it silent (no activation to fire against).
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "with an external header declared and no frame, the LCR check must not fire \
             against a guessed in-band activation; report was: {report}"
        );
        // Even with a confirming frame the Provided gate suppresses the check.
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "under any Provided mode the association-dependent LCR check stays suppressed; \
             report was: {report}"
        );
    }

    #[test]
    fn lcr_repeated_sequence_header_pairs_with_now_present_lcr() {
        // § 6.4.1 associates "this sequence header" with an LCR present prior to
        // it: the violating LCR arrives after the first header but before the
        // bit-identical repeat, so the repeat's association must be evaluated and
        // flagged exactly once. The trailing CLK frame frame-confirms the activation.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/mlayer-dependency-missing"),
            1,
            "the repeated header must pair with the now-present LCR; report was: {report}"
        );
    }

    #[test]
    fn lcr_association_snapshotted_at_header_observation() {
        // § 6.4.1 associates the header with the LCR present prior to *that
        // header*: the dependency-closed LCR precedes the header, the violating
        // redefinition follows it, and the frame-driven activation must check the
        // header-observation snapshot (the closed maps), not the live store.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(1, 5, 1, 1), // id 1, seq_lcr_id 5
        ));
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1])); // redefinition
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // activates id 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "the post-header redefinition must not be paired with header 1; report was: {report}"
        );
    }

    // ----------------------------------------------------------------------------------
    // § 6.8.5 PTL ceilings and § 6.8.8 rep-info equality
    // (lcr-ptl-activated-sequence-agreement)
    // ----------------------------------------------------------------------------------

    /// Parameters for a § 6.8.5 PTL-bearing sequence header.
    #[derive(Clone, Copy)]
    struct SeqPtl {
        seq_header_id: u32,
        seq_lcr_id: u32,
        profile: u32,
        level: u32,
        /// `seq_tier` — only signalled (and so only != Main) when `level > 3`.
        tier: u32,
        /// `max_mlayer_id`; `SeqMaxMlayerCnt == max_mlayer_id + 1`.
        max_mlayer_id: u32,
    }

    /// A sequence header carrying the given § 6.8.5 PTL fields (`max_tlayer_id == 1`),
    /// otherwise identical to [`sequence_header_payload_with_lcr`]. `seq_tier` is only
    /// signalled in the bitstream when `seq_level_idx > 3` (§ 5.4.1).
    fn seq_header_ptl_payload(p: SeqPtl) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(p.seq_header_id);
        bits.f(p.profile, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(p.level, 5); // seq_level_idx
        if p.level > 3 {
            bits.bit(p.tier as u8); // seq_tier
        }
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(p.seq_lcr_id, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(1, 2); // max_tlayer_id
        bits.f(p.max_mlayer_id, 3); // max_mlayer_id
        if p.max_mlayer_id > 0 {
            bits.f(p.max_mlayer_id, ceil_log2_u32(p.max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
        }
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1 -> width 16
        bits.f(7, 4); // max_frame_height_minus_1 -> height 8
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        if p.max_mlayer_id > 0 {
            bits.bit(0); // mlayer_dependency_present_flag
        }
        bits.bit(0); // tlayer_dependency_present_flag (max_tlayer_id == 1 > 0)
        append_non_single_child_configs(&mut bits);
        annex_b_obu(0x04, &bits.into_bytes())
    }

    /// Parameters for a § 6.8.8 rep-info-bearing sequence header.
    #[derive(Clone, Copy)]
    struct SeqRep {
        seq_header_id: u32,
        seq_lcr_id: u32,
        /// `max_frame_width_minus_1` (`f(4)`), so the width is this + 1.
        width_minus_1: u32,
        /// `max_frame_height_minus_1` (`f(4)`).
        height_minus_1: u32,
        chroma_format_idc: u32,
        bit_depth_idc: u32,
        /// `seq_cropping_window_present_flag` and the four offsets when present.
        cropping: Option<(u32, u32, u32, u32)>,
    }

    /// The raw `sequence_header_obu()` payload bytes carrying the given § 6.8.8 rep-info
    /// fields (no embedded layers, `max_tlayer_id == 1`, `max_mlayer_id == 0`).
    fn seq_header_rep_payload_bytes(p: SeqRep) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(p.seq_header_id);
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(p.chroma_format_idc); // chroma_format_idc
        bits.uvlc(p.bit_depth_idc); // bit_depth_idc
        bits.f(p.seq_lcr_id, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(1, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id == 0
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1 (4-bit dims)
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(p.width_minus_1, 4); // max_frame_width_minus_1
        bits.f(p.height_minus_1, 4); // max_frame_height_minus_1
        match p.cropping {
            Some((left, right, top, bottom)) => {
                bits.bit(1); // seq_cropping_window_present_flag
                bits.uvlc(left); // seq_cropping_win_left_offset
                bits.uvlc(right);
                bits.uvlc(top);
                bits.uvlc(bottom);
            }
            None => bits.bit(0), // seq_cropping_window_present_flag
        }
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(0); // tlayer_dependency_present_flag (max_tlayer_id == 1)
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A sequence header carrying the given § 6.8.8 rep-info fields (no embedded layers,
    /// `max_tlayer_id == 1`, `max_mlayer_id == 0`), on extended layer 0.
    fn seq_header_rep_payload(p: SeqRep) -> Vec<u8> {
        annex_b_obu(0x04, &seq_header_rep_payload_bytes(p))
    }

    /// As [`seq_header_rep_payload`], but on the given `xlayer` (a § 6.2.2 base-layer
    /// sequence header — `tlayer == 0`, `mlayer == 0` — that can activate seq id `p` for
    /// that extended layer).
    fn seq_header_rep_obu_for_xlayer(xlayer: u8, p: SeqRep) -> Vec<u8> {
        let payload = seq_header_rep_payload_bytes(p);
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    /// A local LCR OBU at `xlayer` carrying `lcr_seq_profile_tier_level_info(xlayer)`
    /// with the given declared maxima (no rep info, no embedded info).
    fn local_lcr_obu_with_ptl(
        xlayer: u8,
        local_id: u32,
        max_profile: u32,
        max_level: u32,
        max_tier: u32,
        max_mlayer_count: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(local_id, 3); // lcr_local_id
        bits.bit(1); // lcr_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_local_atlas_id_present_flag
        // lcr_seq_profile_tier_level_info(xId)
        bits.f(max_profile, 5); // lcr_seq_profile_idc
        bits.f(max_level, 5); // lcr_max_level_idx
        bits.bit(max_tier as u8); // lcr_tier_flag
        bits.f(max_mlayer_count, 3); // lcr_max_mlayer_count
        bits.f(0, 2); // lsptli_reserved_2bits
        bits.f(0, 3); // reserved_zero_3bits (no atlas)
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        // lcr_xlayer_info(0, xId): all present flags clear.
        bits.bit(0); // lcr_rep_info_present_flag
        bits.bit(0); // lcr_xlayer_purpose_present_flag
        bits.bit(0); // lcr_xlayer_color_info_present_flag
        bits.bit(0); // lcr_embedded_layer_info_present_flag
        bits.align(); // byte_alignment()
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
    }

    /// Appends an `lcr_rep_info()` body with the given width/height, optional
    /// format info `(bit_depth, chroma)`, and optional cropping window
    /// `(left, right, top, bottom)`.
    fn append_lcr_rep_info(
        bits: &mut Bits,
        width: u32,
        height: u32,
        format: Option<(u32, u32)>,
        cropping: Option<(u32, u32, u32, u32)>,
    ) {
        bits.uvlc(width); // lcr_max_pic_width
        bits.uvlc(height); // lcr_max_pic_height
        bits.bit(u8::from(format.is_some())); // lcr_format_info_present_flag
        bits.bit(u8::from(cropping.is_some())); // lcr_cropping_window_present_flag
        if let Some((bit_depth, chroma)) = format {
            bits.uvlc(bit_depth); // lcr_bit_depth_idc
            bits.uvlc(chroma); // lcr_chroma_format_idc
        }
        if let Some((left, right, top, bottom)) = cropping {
            bits.uvlc(left); // lcr_cropping_win_left_offset
            bits.uvlc(right);
            bits.uvlc(top);
            bits.uvlc(bottom);
        }
    }

    /// A local LCR OBU at `xlayer` carrying `lcr_rep_info(0, xId)` (no PTL, no embedded
    /// info).
    fn local_lcr_obu_with_rep_info(
        xlayer: u8,
        local_id: u32,
        width: u32,
        height: u32,
        format: Option<(u32, u32)>,
        cropping: Option<(u32, u32, u32, u32)>,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(0, 3); // lcr_global_id
        bits.f(local_id, 3); // lcr_local_id
        bits.bit(0); // lcr_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_local_atlas_id_present_flag
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        // lcr_xlayer_info(0, xId): only rep info present.
        bits.bit(1); // lcr_rep_info_present_flag
        bits.bit(0); // lcr_xlayer_purpose_present_flag
        bits.bit(0); // lcr_xlayer_color_info_present_flag
        bits.bit(0); // lcr_embedded_layer_info_present_flag
        append_lcr_rep_info(&mut bits, width, height, format, cropping);
        bits.align(); // byte_alignment()
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
    }

    /// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and whose
    /// single global payload carries `lcr_rep_info(1, xId)` with the given fields (no
    /// PTL, no embedded info).
    fn global_lcr_obu_with_rep_info(
        global_id: u32,
        target_xlayer: u8,
        width: u32,
        height: u32,
        format: Option<(u32, u32)>,
        cropping: Option<(u32, u32, u32, u32)>,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(1u32 << target_xlayer, 31); // lcr_xlayer_map
        bits.bit(0); // lcr_aggregate_info_present_flag
        bits.bit(0); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(1); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(0); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        // One lcr_global_payload for target_xlayer: lcr_xlayer_info(1, xId).
        let mut body = Bits::default();
        body.bit(1); // lcr_rep_info_present_flag
        body.bit(0); // lcr_xlayer_purpose_present_flag
        body.bit(0); // lcr_xlayer_color_info_present_flag
        body.bit(0); // lcr_embedded_layer_info_present_flag
        append_lcr_rep_info(&mut body, width, height, format, cropping);
        body.align(); // byte_alignment()
        let body_bytes = (body.bits.len() / 8) as u32;
        debug_assert!(
            body_bytes < 128,
            "lcr_global_data_size must fit a single-byte leb128"
        );
        bits.f(body_bytes, 8); // lcr_global_data_size (single-byte leb128)
        bits.bits.extend_from_slice(&body.bits);
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and that
    /// carries `lcr_seq_profile_tier_level_info(target_xlayer)` with the given maxima
    /// (no payload).
    fn global_lcr_obu_with_ptl(
        global_id: u32,
        target_xlayer: u8,
        max_profile: u32,
        max_level: u32,
        max_tier: u32,
        max_mlayer_count: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(1u32 << target_xlayer, 31); // lcr_xlayer_map
        bits.bit(0); // lcr_aggregate_info_present_flag
        bits.bit(1); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(0); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(0); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        // One lcr_seq_profile_tier_level_info per set bit of lcr_xlayer_map.
        bits.f(max_profile, 5); // lcr_seq_profile_idc
        bits.f(max_level, 5); // lcr_max_level_idx
        bits.bit(max_tier as u8); // lcr_tier_flag
        bits.f(max_mlayer_count, 3); // lcr_max_mlayer_count
        bits.f(0, 2); // lsptli_reserved_2bits
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    // --- § 6.8.5 PTL ceilings ---------------------------------------------------------

    #[test]
    fn lcr_ptl_level_exceeds_max_is_flagged() {
        // Header seq_level_idx 8 > local LCR lcr_max_level_idx 4. The trailing CLK frame
        // frame-confirms the xlayer-0 activation (§ 6.8.5 uses the strict gate).
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/ptl-level-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_profile_exceeds_max_is_flagged() {
        // Header seq_profile_idc 3 > local LCR lcr_seq_profile_idc 1.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 1, 31, 0, 7));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 3,
            level: 0,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/ptl-profile-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_tier_exceeds_max_is_flagged() {
        // Header seq_tier 1 (High, level 5 > 3) > local LCR lcr_tier_flag 0.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 7));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 5,
            tier: 1,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/ptl-tier-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_mlayer_count_exceeds_max_is_flagged() {
        // Header SeqMaxMlayerCnt = max_mlayer_id 1 + 1 = 2 > lcr_max_mlayer_count 1.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 0,
            tier: 0,
            max_mlayer_id: 1,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/ptl-mlayer-count-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_equality_passes() {
        // Every header value equals its LCR-declared maximum: <= passes, no finding.
        let mut data = temporal_delimiter_obu();
        // lcr_max_mlayer_count 2 == SeqMaxMlayerCnt (max_mlayer_id 1 + 1).
        data.extend(local_lcr_obu_with_ptl(0, 5, 2, 5, 1, 2));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 2,
            level: 5,
            tier: 1,
            max_mlayer_id: 1,
        }));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/ptl-profile-exceeds-max")
                && !has_error(&report, "lcr/ptl-level-exceeds-max")
                && !has_error(&report, "lcr/ptl-tier-exceeds-max")
                && !has_error(&report, "lcr/ptl-mlayer-count-exceeds-max"),
            "equality must pass; report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_absent_info_compares_nothing() {
        // The associated local LCR carries no PTL info (present flag 0): § 6.8.5 gates
        // on "lcr_seq_profile_tier_level_info(i) present", so nothing is compared even
        // though the header level would exceed any plausible ceiling.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(0, 0, 5, None)); // no PTL, no rep info
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 4,
            level: 20,
            tier: 0,
            max_mlayer_id: 0,
        }));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/ptl-level-exceeds-max")
                && !has_error(&report, "lcr/ptl-profile-exceeds-max"),
            "absent PTL info must compare nothing; report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_unconfirmed_activation_is_silent_then_fires_on_frame() {
        // Two staged headers for xlayer 0: the OBU-order fallback is a guess (§ 7.3.6),
        // so nothing fires until a frame confirms one. The violating header (id 1) is
        // the one the frame loads.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 0, // no LCR -> not violating
            profile: 0,
            level: 0,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 1,
            seq_lcr_id: 5,
            profile: 0,
            level: 8, // > lcr_max_level_idx 4
            tier: 0,
            max_mlayer_id: 0,
        }));
        // Before any frame, the fallback is ambiguous: silent.
        let staged = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&staged, "lcr/ptl-level-exceeds-max"),
            "an unconfirmed activation must be silent; report was: {staged}"
        );
        // A frame confirming header 1 makes the violation decidable.
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
        let confirmed = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&confirmed, "lcr/ptl-level-exceeds-max"),
            "the frame-confirmed activation must fire; report was: {confirmed}"
        );
    }

    #[test]
    fn lcr_ptl_global_record_ceiling_is_checked() {
        // The association resolves a global LCR carrying PTL for xlayer 0; its
        // lcr_max_level_idx 4 < header seq_level_idx 8. The trailing CLK frame
        // frame-confirms the xlayer-0 activation.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_ptl(5, 0, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/ptl-level-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_not_duplicated_across_reactivation() {
        // The activation-driven re-check (frame re-references the same header) must not
        // duplicate the finding.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/ptl-level-exceeds-max"),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_redefinition_rechecks_affected_layer() {
        // First LCR 5 is conformant (ceiling 31); a non-identical redefinition lowers
        // lcr_max_level_idx to 4, and the bit-identical repeated header re-associates
        // to the new revision. The trailing CLK frame frame-confirms the activation
        // against the ceiling-4 revision and is flagged exactly once.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 7));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 7)); // redefinition: ceiling 4
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/ptl-level-exceeds-max"),
            1,
            "the redefinition's lowered ceiling must re-check exactly once; report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_diagnostic_points_at_lcr_obu() {
        // The diagnostic anchors at the LCR OBU (its declared maxima are the source),
        // which precedes the activating sequence header here. The CLK frame appended
        // after the header frame-confirms the activation without moving the offsets.
        let td = temporal_delimiter_obu();
        let lcr = local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1);
        let seq_start = (td.len() + lcr.len()) as u64;
        let mut data = td;
        data.extend(lcr);
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8,
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        let offsets: Vec<_> = report
            .errors()
            .filter(|d| d.rule_id == "lcr/ptl-level-exceeds-max")
            .map(|d| d.byte_offset)
            .collect();
        assert!(
            matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
            "the diagnostic must point at the LCR OBU (before byte {seq_start}); report: {report}"
        );
    }

    #[test]
    fn lcr_ptl_suppressed_under_external_hls_provided() {
        // The § 6.8.5 ceiling pairs the in-band activated header against the LCR its
        // seq_lcr_id resolves to under § 6.4.1. A Provided declaration is PARTIAL (it
        // cannot enumerate external LCRs), so an unmodeled external *local* LCR could win
        // the local-first resolution ahead of the in-band record; the check is suppressed
        // under any Provided mode (even an empty set) to avoid a false positive against an
        // association a real decoder may not use. The stream WOULD fire under Disabled (the
        // trailing CLK frame frame-confirms the activation).
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8, // > lcr_max_level_idx 4
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        assert!(
            has_error(
                &Validator::new(false).validate_bytes(&data),
                "lcr/ptl-level-exceeds-max"
            ),
            "the in-band ceiling violation must fire under Disabled"
        );
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/ptl-level-exceeds-max"),
            "an empty Provided set must suppress the association-dependent PTL ceiling \
             check; report was: {report}"
        );
    }

    #[test]
    fn lcr_ptl_suppressed_under_ops_only_external_hls_provided() {
        // An OPS-only Provided set (operating point sets declared but no sequence headers)
        // also suppresses the § 6.8.5 ceiling. This is the key cycle-2/cycle-3 point: the
        // suppression is NOT about declared sequence headers — ANY Provided mode may imply
        // unenumerated external LCRs (the set cannot express them), so an external local
        // LCR could still shadow the in-band § 6.4.1 association even when only OPS are
        // declared. The gate is `!Disabled`, not `declares_any_sequence_header`.
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
        data.extend(seq_header_ptl_payload(SeqPtl {
            seq_header_id: 0,
            seq_lcr_id: 5,
            profile: 0,
            level: 8, // > lcr_max_level_idx 4
            tier: 0,
            max_mlayer_id: 0,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_operating_point_set(0, 3),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/ptl-level-exceeds-max"),
            "an OPS-only Provided set must suppress the association-dependent PTL ceiling \
             check; report was: {report}"
        );
    }

    // --- § 6.8.8 rep-info equality ----------------------------------------------------

    #[test]
    fn lcr_rep_info_width_mismatch_is_flagged() {
        // lcr_max_pic_width 1920 != max_frame_width_minus_1 + 1 = 16. The trailing CLK
        // frame frame-confirms the xlayer-0 activation (§ 6.8.8 uses the strict gate).
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16
            height_minus_1: 7, // height 8
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/rep-info-mismatch"),
            "report was: {report}"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/rep-info-mismatch"
                    && d.message.contains("lcr_max_pic_width")),
            "the message must name the width field; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_height_bit_depth_chroma_mismatches_are_flagged() {
        // Height, bit depth, and chroma all disagree.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(
            0,
            5,
            16,
            999,          // wrong height
            Some((1, 2)), // wrong bit depth + chroma
            None,
        ));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 (agrees)
            height_minus_1: 7, // height 8
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_max_pic_height")),
            "height mismatch must be named; report was: {report}"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/rep-info-mismatch"
                    && d.message.contains("lcr_bit_depth_idc")),
            "bit-depth mismatch must be named; report was: {report}"
        );
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_chroma_format_idc")),
            "chroma mismatch must be named; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_cropping_present_flag_mismatch_is_flagged() {
        // LCR has a cropping window present; the header does not.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(
            0,
            5,
            16,
            8,
            None,
            Some((0, 0, 0, 0)),
        ));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None, // present flag 0
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_cropping_window_present_flag")),
            "the present-flag disagreement must be named; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_cropping_present_flag_mismatch_also_reports_offsets() {
        // The LCR carries a cropping window with a non-zero left offset; the header has
        // no window (present flag 0, offsets inferred to 0). Per the § 6.8.8 "shall match"
        // sentences, both the present-flag disagreement AND the offset disagreement fire:
        // the offset comparison runs against the header's inferred-0 values regardless of
        // the present-flag mismatch (see the rationale comment in
        // `context.rs` around the `seq_cropping_win_*` inference, lines 8046-8050).
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(
            0,
            5,
            16,
            8,
            None,
            Some((1, 0, 0, 0)), // left offset 1, window present
        ));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None, // present flag 0, offsets inferred 0
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_cropping_window_present_flag")),
            "the present-flag disagreement must fire; report was: {report}"
        );
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_cropping_win_left_offset")),
            "the left-offset disagreement must also fire (spec-correct over-reporting); \
             report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_cropping_offset_mismatch_is_flagged() {
        // Both present, but a top offset disagrees.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(
            0,
            5,
            16,
            8,
            None,
            Some((1, 2, 9, 4)), // top 9
        ));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: Some((1, 2, 3, 4)), // top 3
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_cropping_win_top_offset")),
            "the offset disagreement must be named; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_full_agreement_passes() {
        // Width/height/format/cropping all agree: no finding.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(
            0,
            5,
            16,
            8,
            Some((0, 0)),
            Some((1, 2, 3, 4)),
        ));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: Some((1, 2, 3, 4)),
        }));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/rep-info-mismatch"),
            "full agreement must be silent; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_absent_format_info_compares_nothing() {
        // The LCR rep info omits format info (present flag 0): the bit-depth / chroma
        // sentences gate on lcr_format_info_present_flag, so a header with any format is
        // not compared on those fields (width/height still agree here).
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 2, // would mismatch if compared
            bit_depth_idc: 1,     // would mismatch if compared
            cropping: None,
        }));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/rep-info-mismatch"),
            "absent format info must compare nothing; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_absent_rep_info_compares_nothing() {
        // The associated local LCR carries no rep info at all: nothing is compared.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu(0, 0, 5, None)); // no rep info
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "lcr/rep-info-mismatch"),
            "absent rep info must compare nothing; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_global_record_is_checked() {
        // A global LCR payload carrying rep info for xlayer 0 with a mismatched width.
        // The trailing CLK frame frame-confirms the xlayer-0 activation.
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_rep_info(5, 0, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "lcr/rep-info-mismatch"
                    && d.message.contains("lcr_max_pic_width")),
            "the global rep info must be checked; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_not_duplicated_across_reactivation() {
        // The activation-driven re-check (frame re-references the same header) must not
        // duplicate the finding.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 != lcr 1920
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/rep-info-mismatch"),
            1,
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_diagnostic_points_at_lcr_obu() {
        // The diagnostic anchors at the LCR OBU (its declared rep info is the source),
        // which precedes the activating sequence header here. The CLK frame appended
        // after the header frame-confirms the activation without moving the offsets.
        let td = temporal_delimiter_obu();
        let lcr = local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None);
        let seq_start = (td.len() + lcr.len()) as u64;
        let mut data = td;
        data.extend(lcr);
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 != lcr 1920
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        let offsets: Vec<_> = report
            .errors()
            .filter(|d| d.rule_id == "lcr/rep-info-mismatch")
            .map(|d| d.byte_offset)
            .collect();
        assert!(
            matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
            "the diagnostic must point at the LCR OBU (before byte {seq_start}); report: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_unconfirmed_activation_is_silent_then_fires_on_frame() {
        // The violating header (id 1) is staged behind a non-violating header (id 0);
        // only the frame-confirmed activation fires.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 0, // no LCR association -> not violating
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 1,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 != lcr 1920
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        let staged = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&staged, "lcr/rep-info-mismatch"),
            "an unconfirmed activation must be silent; report was: {staged}"
        );
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
        let confirmed = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&confirmed, "lcr/rep-info-mismatch"),
            "the frame-confirmed activation must fire; report was: {confirmed}"
        );
    }

    #[test]
    fn lcr_rep_info_redefinition_rechecks_affected_layer() {
        // A conformant LCR 5 is redefined with a mismatched width; the repeated header
        // re-associates to the new revision. The trailing CLK frame frame-confirms the
        // activation against the redefined (width-1920) revision, flagged exactly once.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None)); // agrees
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None)); // redefinition: width 1920
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15,
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "lcr/rep-info-mismatch"),
            1,
            "the redefinition must re-check exactly once; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_redefinition_of_only_dims_rechecks_all_layers_using_the_id() {
        // Regression for the § 6.8.8 LCR-agreement re-check widener. Header id 0 is active
        // for xlayer 0 AND xlayer 1, each associated with its own local LCR 5 whose rep
        // info matches the header (width 16). A later same-id redefinition (via the
        // xlayer-0 header OBU) changes ONLY max_frame_width to 8, which disagrees with
        // both LCRs. max_frame_width is not in the agreement-input set nor the Annex A
        // value-space fingerprint, so before the LCR-agreement fingerprint widener the
        // redefinition would only re-check the redefinition's own xlayer 0 and miss the
        // other layer (xlayer 1) the id is active for. The recheck must cover every
        // extended layer the id is active for, so the mismatch fires for BOTH (or at
        // minimum the non-activating xlayer 1).
        //
        // TU 1: both xlayers' local LCR 5 and their seq-0 headers (width 16, agree), then
        // frame-confirm xlayer 0 then xlayer 1 (ascending obu_xlayer_id, § 7.3.7;
        // frame_confirmed_xlayers is monotonic, so both stay confirmed).
        // OBUs are kept in ascending obu_xlayer_id order within each temporal unit
        // (§ 7.3.7): each xlayer's LCR and its seq-0 header are grouped, xlayer 0 then
        // xlayer 1; the frame confirmations live in their own temporal unit.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None)); // xlayer 0 LCR, width 16
        data.extend(seq_header_rep_obu_for_xlayer(
            0,
            SeqRep {
                seq_header_id: 0,
                seq_lcr_id: 5,
                width_minus_1: 15, // width 16 (agrees with xlayer 0 LCR)
                height_minus_1: 7,
                chroma_format_idc: 0,
                bit_depth_idc: 0,
                cropping: None,
            },
        ));
        data.extend(local_lcr_obu_with_rep_info(1, 5, 16, 8, None, None)); // xlayer 1 LCR, width 16
        data.extend(seq_header_rep_obu_for_xlayer(
            1,
            SeqRep {
                seq_header_id: 0,
                seq_lcr_id: 5,
                width_minus_1: 15, // width 16 (agrees with xlayer 1 LCR)
                height_minus_1: 7,
                chroma_format_idc: 0,
                bit_depth_idc: 0,
                cropping: None,
            },
        ));
        // TU 2: frame-confirm xlayer 0 then xlayer 1 (ascending; frame_confirmed_xlayers
        // is monotonic, so both stay confirmed afterward).
        data.extend(temporal_delimiter_obu());
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 ref seq 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 0)); // CLK xlayer 1 ref seq 0
        // TU 3: redefinition of seq 0 (via the xlayer-0 header OBU) changing ONLY
        // max_frame_width to 8 — disagreeing with both LCRs' width 16. seq 0 is still
        // active for BOTH xlayer 0 and xlayer 1, so the LCR-agreement fingerprint-change
        // recheck must cover both even though only xlayer 0 re-confirms here. (The 4-bit
        // frame-width field caps the value at 15+1=16, so the disagreement is encoded by
        // shrinking the width rather than enlarging it.)
        data.extend(temporal_delimiter_obu());
        data.extend(seq_header_rep_obu_for_xlayer(
            0,
            SeqRep {
                seq_header_id: 0,
                seq_lcr_id: 5,
                width_minus_1: 7, // width 8 != LCR width 16
                height_minus_1: 7,
                chroma_format_idc: 0,
                bit_depth_idc: 0,
                cropping: None,
            },
        ));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // re-activate seq 0 (xlayer 0)
        let report = Validator::new(false).validate_bytes(&data);
        let xlayer_1_mismatch = report.errors().any(|d| {
            d.rule_id == "lcr/rep-info-mismatch"
                && d.spec_section.as_deref() == Some("6.8.8")
                && d.message.contains("extended layer 1")
        });
        assert!(
            xlayer_1_mismatch,
            "a redefinition changing only max_frame_width must re-run the § 6.8.8 \
             agreement check for every extended layer the id is active for, including the \
             non-activating xlayer 1; report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_suppressed_under_external_hls_provided() {
        // § 6.8.8 pairs the in-band activated header against the LCR its seq_lcr_id
        // resolves to under § 6.4.1. A Provided declaration is PARTIAL (it cannot
        // enumerate external LCRs), so an unmodeled external *local* LCR could shadow the
        // in-band association; the check is suppressed under any Provided mode to avoid a
        // false positive. The stream WOULD fire under Disabled (trailing CLK frame
        // frame-confirms the activation).
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 != lcr_max_pic_width 1920
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        assert!(
            has_error(
                &Validator::new(false).validate_bytes(&data),
                "lcr/rep-info-mismatch"
            ),
            "the in-band rep-info mismatch must fire under Disabled"
        );
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/rep-info-mismatch"),
            "an empty Provided set must suppress the association-dependent rep-info check; \
             report was: {report}"
        );
    }

    #[test]
    fn lcr_rep_info_suppressed_under_ops_only_external_hls_provided() {
        // An OPS-only Provided set also suppresses the § 6.8.8 mismatch: ANY Provided mode
        // may imply unenumerated external LCRs, so the suppression is not about declared
        // sequence headers. The gate is `!Disabled`, not `declares_any_sequence_header`.
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
        data.extend(seq_header_rep_payload(SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 != lcr_max_pic_width 1920
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        }));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_operating_point_set(0, 3),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/rep-info-mismatch"),
            "an OPS-only Provided set must suppress the association-dependent rep-info \
             check; report was: {report}"
        );
    }

    #[test]
    fn ops_not_checked_against_ambiguous_fallback_header() {
        // Two in-band headers are available before any frame, so the OBU-order
        // fallback (id 0, default maps) is a guess; the frame then loads id 1,
        // whose signaled map clears MLayerDependencyMap[1][0]. The OPS must be
        // paired with the frame-confirmed header — flagging it against the
        // fallback would be an unretractable false positive.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_mlayer_dep_cleared(1),
        ));
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // loads id 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "ops/mlayer-dependency-missing"),
            "the OPS pairs with the frame-confirmed header, not the fallback; report was: {report}"
        );
    }

    #[test]
    fn ops_checked_when_frame_confirms_the_fallback_header() {
        // Same ambiguous-fallback stream, but the frame confirms the violating
        // header 0: the deferred check must fire exactly once at confirmation.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_mlayer_dep_cleared(1),
        ));
        data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // loads id 0
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            ops_error_count(&report, "ops/mlayer-dependency-missing"),
            1,
            "frame confirmation of the violating header must fire once; report was: {report}"
        );
    }

    // --- § 6.4.1 SWITCH / RAS dependency-map self-containment (3.1) --------------

    /// A sequence header (xlayer 0) with `max_mlayer_id == 1` whose default § 5.4.1
    /// dependency fill leaves `MLayerDependencyMap[1][0] == 1` (lower-triangular), so a
    /// SWITCH / RAS frame at obu_mlayer_id 1 depends on embedded layer 0.
    fn td_and_seq_header_mlayer_dependent() -> Vec<u8> {
        // sequence_header_payload_with_id(0, 0, 1): max_mlayer_id 1, no signaled map
        // (default fill), SeqMaxMlayerCnt 2.
        td_and_seq_header(0, 0, 1)
    }

    #[test]
    fn switch_frame_depending_on_another_embedded_layer_is_flagged() {
        // § 6.4.1: an OBU_SWITCH (type 10) at obu_mlayer_id 1 with
        // MLayerDependencyMap[1][0] != 0 is not self-contained.
        let mut data = td_and_seq_header_mlayer_dependent();
        // SWITCH, tlayer 0 (§ 6.2.2 temporal-layer-zero-only), mlayer 1, xlayer 0, ref seq 0.
        data.extend(frame_obu_direct_seq_ref_layer(10, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn ras_frame_depending_on_another_embedded_layer_is_flagged() {
        // § 6.4.1: an OBU_RAS_FRAME (type 21) at obu_mlayer_id 1 with
        // MLayerDependencyMap[1][0] != 0 is not self-contained.
        let mut data = td_and_seq_header_mlayer_dependent();
        // RAS frame, tlayer 0, mlayer 1, xlayer 0, ref seq 0.
        data.extend(frame_obu_direct_seq_ref_layer(21, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn self_contained_ras_frame_is_not_flagged() {
        // § 6.4.1: with a signaled map clearing MLayerDependencyMap[1][0], a RAS frame at
        // obu_mlayer_id 1 is self-contained and must not be flagged. (The separate
        // § 6.4.6 long_term_frame_id_bits check may still fire; it is a different rule.)
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_mlayer_dep_cleared(0),
        ));
        data.extend(frame_obu_direct_seq_ref_layer(21, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id
                    == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"),
            "a self-contained map must not flag the RAS frame; report was: {report}"
        );
    }

    #[test]
    fn switch_frame_at_base_embedded_layer_is_not_flagged() {
        // § 6.4.1: a SWITCH at obu_mlayer_id 0 has no other embedded layer it could
        // depend on (the rule ranges over m != obu_mlayer_id), so it is never flagged.
        let mut data = td_and_seq_header_mlayer_dependent();
        data.extend(frame_obu_direct_seq_ref_layer(10, 0, 0, 0, 0)); // SWITCH, mlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id
                    == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"),
            "report was: {report}"
        );
    }

    // --- § 6.4.1 distinct-obu_mlayer_id count vs SeqMaxMlayerCnt (3.2) -----------

    /// A sequence header (xlayer 0) with `max_mlayer_id == 1` and
    /// `seq_max_mlayer_cnt_minus_1 == 0` (`SeqMaxMlayerCnt == 1`): only one distinct
    /// embedded layer is allowed in the coded video sequence, even though embedded layer
    /// 1 is otherwise within `max_mlayer_id`.
    fn seq_header_payload_seqmaxcnt_one() -> Vec<u8> {
        seq_header_payload_seqmaxcnt_one_id(0)
    }

    /// As [`seq_header_payload_seqmaxcnt_one`] but with an explicit `seq_header_id`, so a
    /// fixture can place two distinct SeqMaxMlayerCnt-1 headers (e.g. an outgoing header
    /// and a different one a CLK re-references) in the same stream.
    fn seq_header_payload_seqmaxcnt_one_id(seq_header_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(1, 3); // max_mlayer_id = 1
        bits.f(0, 1); // seq_max_mlayer_cnt_minus_1 = 0 -> SeqMaxMlayerCnt = 1
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(0); // mlayer_dependency_present_flag (max_mlayer_id > 0)
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A sequence-header payload with `max_mlayer_id == 2` and `SeqMaxMlayerCnt == 2`
    /// (`seq_max_mlayer_cnt_minus_1 == 1`): the coded video sequence may use embedded
    /// layers up to 2 but at most two *distinct* `obu_mlayer_id` values (AV2 § 6.4.1).
    fn seq_header_payload_seqmaxcnt_two() -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(2, 3); // max_mlayer_id = 2
        bits.f(1, ceil_log2_u32(3)); // seq_max_mlayer_cnt_minus_1 = 1 -> SeqMaxMlayerCnt = 2
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        bits.bit(0); // mlayer_dependency_present_flag (max_mlayer_id > 0)
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    #[test]
    fn distinct_mlayer_count_exceeds_seqmax_is_flagged() {
        // § 6.4.1: SeqMaxMlayerCnt == 1, but the coded video sequence carries the
        // sequence header (embedded layer 0, forced by § 6.2.2) and a frame at embedded
        // layer 1 -> 2 distinct obu_mlayer_id values > SeqMaxMlayerCnt.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        // OBU_REGULAR_TILE_GROUP at mlayer 1, xlayer 0, references seq 0.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_within_seqmax_is_conforming() {
        // § 6.4.1: with SeqMaxMlayerCnt == 2 (sequence_header_payload(0, 1)), the same
        // two distinct embedded layers 0 and 1 are within budget — no diagnostic.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_resets_at_cvs_boundary() {
        // § 6.4.1 / § 7.3.6: the count is scoped to each coded video sequence. CVS 0
        // uses embedded layer 0 only; a CLK at embedded layer 1 starts CVS 1 for the
        // extended layer, where embedded layer 1 is the only distinct value. Each coded
        // video sequence carries one distinct obu_mlayer_id (<= SeqMaxMlayerCnt 1), so
        // the cumulative {0, 1} must NOT fire once the count resets at the boundary.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        // CVS 0: a frame at embedded layer 0 (references seq 0).
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        // Next temporal unit: a CLK at embedded layer 1 starts a new coded video sequence.
        data.extend(temporal_delimiter_obu());
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "the count must reset at the § 7.3.6 CVS boundary; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_before_first_clk_uses_active_header() {
        // Pre-first-CLK edge: with no CLK boundary yet, the implicit coded video
        // sequence still counts against the active (OBU-order fallback) header. A frame
        // at embedded layer 1 plus the embedded-layer-0 sequence header exceeds
        // SeqMaxMlayerCnt 1 even though no CLK has occurred.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // mlayer 1, no frame ref
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_emits_once_per_cvs() {
        // The check emits once per coded video sequence: two further frames at embedded
        // layer 1 after the first exceedance do not repeat the diagnostic.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
                .count(),
            1,
            "the § 6.4.1 distinct-mlayer check must emit once per CVS; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_pre_clk_obu_in_boundary_tu_is_not_flagged() {
        // § 7.3.6 / § 6.4.1: a coded video sequence starts AT the temporal unit that
        // contains the CLK, so an OBU of the same extended layer observed earlier in that
        // temporal unit already belongs to the NEW coded video sequence. Here the old CVS
        // (temporal unit 0) carries only embedded layer 0 (the sequence header) — within
        // SeqMaxMlayerCnt 1. Temporal unit 1 has a pre-CLK OBU at embedded layer 1 then a
        // CLK at embedded layer 1. Under FIX 4 (exact re-attribution), the new CVS is
        // re-seeded from the boundary temporal unit's seen set {1} (count 1 <= 1), so the
        // new CVS never exceeds; and the pre-CLK OBU's single-pass count into the *old*
        // CVS ({0, 1} = 2 > 1, first counted in temporal unit 0 so deferred) is dropped at
        // the boundary because the extended layer started a new CVS in temporal unit 1.
        // Both mechanisms leave nothing to emit. (This is also the FIX 4 "still-needed
        // pending-drop" coverage: the deferred exceedance whose set spanned a
        // pre-boundary temporal unit is dropped.)
        let mut data = temporal_delimiter_obu(); // temporal unit 0
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        // A frame at embedded layer 0 keeps the old CVS at {0} (count 1, conforming).
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        // Temporal unit 1: a pre-CLK OBU at embedded layer 1, then a CLK at embedded
        // layer 1 that begins a new coded video sequence for the extended layer.
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK, mlayer 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "a pre-CLK OBU in the CVS-starting temporal unit belongs to the new coded video \
             sequence; the new CVS {{1}} does not exceed and the deferred old-CVS exceedance \
             is dropped; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_pre_clk_header_reattributed_to_new_cvs_is_flagged() {
        // FIX 4 (Codex finding 1) positive case. § 7.3.6 (mirror
        // `07-decoding-process.md` lines 604-606): the new coded video sequence starts AT
        // the temporal unit containing the CLK, so the § 7.3.8.1 resent-at-RAP sequence
        // header observed BEFORE the CLK in that temporal unit (forced to obu_mlayer_id 0,
        // § 6.4.1 NOTE / § 6.2.2) belongs to the NEW coded video sequence and must count
        // toward SeqMaxMlayerCnt. A single temporal unit = [seq header @ mlayer 0,
        // CLK @ mlayer 1] with SeqMaxMlayerCnt 1 truly carries {0, 1} = 2 > 1 in the new
        // coded video sequence. The former whole-state drop at reset_cvs missed this; the
        // re-attribution must emit the exceedance exactly once.
        let mut data = temporal_delimiter_obu();
        // Resent-at-RAP sequence header (embedded layer 0, SeqMaxMlayerCnt 1).
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        // CLK at embedded layer 1 referencing seq 0 begins the new CVS at this temporal
        // unit; the pre-CLK header (mlayer 0) is re-attributed to it -> {0, 1} = 2 > 1.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| {
                    d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                        && d.spec_section.as_deref() == Some("6.4.1")
                })
                .count(),
            1,
            "the pre-CLK header is re-attributed to the new CVS; {{0, 1}} = 2 > 1 must fire \
             exactly once; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_reattribution_excludes_pre_boundary_tu_ids() {
        // FIX 4 no-false-positive case. The re-seeded new-CVS set must include ONLY ids
        // from the boundary temporal unit, never ids from earlier temporal units. With
        // SeqMaxMlayerCnt 2: temporal unit 0's old CVS carries {0, 2} (count 2 <= 2,
        // conforming); temporal unit 1 = [header @ mlayer 0, CLK @ mlayer 1] re-attributes
        // only the boundary temporal unit's ids -> new CVS {0, 1} = 2 <= 2. Neither CVS
        // exceeds, so no diagnostic. (If reset_cvs wrongly carried temporal unit 0's ids,
        // the new CVS would be {0, 1, 2} = 3 > 2 and falsely fire.)
        let mut data = temporal_delimiter_obu(); // temporal unit 0
        // SeqMaxMlayerCnt 2 (max_mlayer_id 2). sequence_header_payload(0, 2) sets
        // seq_max_mlayer_cnt_minus_1 = max_mlayer_id = 2 -> SeqMaxMlayerCnt 3; use an
        // explicit SeqMaxMlayerCnt-2 header instead.
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two()));
        // Old CVS ids {0 (header), 2}: a frame at embedded layer 2 (allowed, max_mlayer 2).
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 2, 0, 0));
        // Temporal unit 1: resent header (mlayer 0) then CLK @ mlayer 1 -> new CVS {0, 1}.
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two())); // resent header, mlayer 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK @ mlayer 1, ref seq 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "the new CVS must re-seed only boundary-temporal-unit ids ({{0, 1}} <= 2); \
             earlier-temporal-unit ids must not count; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_reattribution_reports_once_across_clk_in_boundary_tu() {
        // FIX 4 once-per-CVS guard across the boundary. In a single temporal unit (so the
        // set's first temporal unit is the boundary temporal unit and the exceedance emits
        // eagerly), pre-CLK ids {0, 1} already exceed SeqMaxMlayerCnt 1 (emitted once),
        // then a CLK @ mlayer 1 begins a new CVS re-seeded from {0, 1}. Because the old
        // state's first temporal unit equals the boundary temporal unit, the `reported`
        // flag carries into the re-seeded new-CVS state, so a further post-CLK OBU in the
        // same (now single) CVS does not re-report.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header, mlayer 0
        // Pre-CLK frame @ mlayer 1 -> {0, 1} = 2 > 1, first counted this temporal unit ->
        // eager emit (once).
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        // CLK @ mlayer 1 begins the new CVS, re-seeded from {0, 1} with the reported flag.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
        // A further post-CLK OBU @ mlayer 1 in the same new CVS must not re-report.
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
                .count(),
            1,
            "the exceedance visible both pre- and post-CLK in the boundary temporal unit \
             must report exactly once; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_reattribution_compares_against_clk_activated_header() {
        // PR #41 Codex false-positive regression. § 6.4.1 (mirror
        // `06-syntax-structures-semantics.md` lines 445-447): the distinct-obu_mlayer_id
        // count is scoped to "the coded video sequence associated with this sequence
        // header" — for the NEW coded video sequence a CLK begins that is the header the
        // CLK *activates*, not the outgoing one still active when the § 7.3.6 boundary
        // event fires. Outgoing header (id 0, SeqMaxMlayerCnt 1) is active and
        // frame-confirmed; the boundary temporal unit carries a re-sent header (mlayer 0)
        // and a pre-CLK OBU (mlayer 1) for the per-temporal-unit set {0, 1}, then a CLK
        // (mlayer 0) referencing a DIFFERENT header (id 1, SeqMaxMlayerCnt 2). The
        // re-seeded set {0, 1} = 2 conforms to the CLK-activated header's max 2, so
        // nothing must fire. Comparing 2 against the outgoing max 1 at reset time would
        // be a false positive.
        let mut data = temporal_delimiter_obu(); // temporal unit 0
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
        // Header id 1 with max_mlayer_id 1 -> SeqMaxMlayerCnt 2 (allows mlayer 0 and 1).
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 1)));
        // A frame at embedded layer 0 referencing seq 0 activates and frame-confirms it.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        // Temporal unit 1 (boundary): re-sent header (mlayer 0), a pre-CLK OBU (mlayer 1),
        // then a CLK (mlayer 0) referencing the DIFFERENT header id 1.
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "the re-seeded {{0, 1}} = 2 set must be compared against the CLK-activated header \
             (id 1, SeqMaxMlayerCnt 2), not the outgoing header (id 0, max 1); report was: \
             {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_reattribution_clk_activated_header_lower_max_is_flagged() {
        // Reverse-direction true positive that the reset-time check direction-masked.
        // Outgoing header (id 0, SeqMaxMlayerCnt 2) is active and frame-confirmed; the
        // boundary temporal unit carries a re-sent header (mlayer 0) and a pre-CLK OBU
        // (mlayer 1) for the per-temporal-unit set {0, 1}, then a CLK (mlayer 0)
        // referencing a DIFFERENT header (id 1, SeqMaxMlayerCnt 1). The re-seeded set
        // {0, 1} = 2 exceeds the CLK-activated header's max 1, so the § 6.4.1 exceedance
        // must fire exactly once, anchored at the CLK's extension byte. The old reset-time
        // check passed (2 <= outgoing max 2) and the activation-path retroactive check
        // catches it because the referenced id changes.
        let mut data = temporal_delimiter_obu(); // temporal unit 0
        // Header id 0 with max_mlayer_id 1 -> SeqMaxMlayerCnt 2.
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one_id(1))); // header id 1, max 1
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
        // Temporal unit 1 (boundary): re-sent header (mlayer 0), pre-CLK OBU (mlayer 1),
        // CLK (mlayer 0) referencing the DIFFERENT header id 1 (max 1).
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1))); // re-sent header id 0, mlayer 0
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1 (max 1)
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| {
                    d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                        && d.spec_section.as_deref() == Some("6.4.1")
                })
                .count(),
            1,
            "the re-seeded {{0, 1}} = 2 set exceeds the CLK-activated header's max 1 and must \
             fire exactly once; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_reattribution_same_header_exceedance_is_flagged() {
        // Same-header coverage the immediate reset-time check used to own: when the CLK
        // re-references the SAME already-frame-confirmed header, the re-seeded pre-CLK set
        // may already exceed that header's max in a way the eager count_distinct_mlayer
        // cannot re-surface (it never re-yields an already-seen id). Outgoing header id 0
        // (SeqMaxMlayerCnt 1) active and frame-confirmed; boundary temporal unit carries a
        // re-sent header (mlayer 0) and a pre-CLK OBU (mlayer 1) for the per-temporal-unit
        // set {0, 1} = 2 > 1, then a CLK (mlayer 1) referencing the SAME header id 0. The
        // CLK's own mlayer 1 is already in the re-seeded set, so the eager path yields
        // nothing; the post-activation retroactive check must still fire once.
        let mut data = temporal_delimiter_obu(); // temporal unit 0
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
        // Temporal unit 1 (boundary): re-sent header (mlayer 0), pre-CLK OBU (mlayer 1),
        // CLK (mlayer 1) referencing the SAME header id 0.
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK mlayer 1, ref seq 0
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| {
                    d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                        && d.spec_section.as_deref() == Some("6.4.1")
                })
                .count(),
            1,
            "the re-seeded {{0, 1}} = 2 set exceeds the re-referenced header's max 1 and must \
             fire exactly once even though the CLK's own mlayer is already in the set; report \
             was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // § 6.4.1: under caller-provided external HLS the active sequence header (and its
        // SeqMaxMlayerCnt) may be supplied out of band, so the in-band distinct-mlayer
        // count is unreliable and the check is suppressed even on the otherwise-firing
        // two-embedded-layer stream.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "external HLS must suppress the § 6.4.1 distinct-mlayer check; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_accumulated_before_header_activation_is_flagged() {
        // § 6.4.1: OBUs arriving before any active sequence header for their extended
        // layer accumulate a distinct-obu_mlayer_id count that the eager per-OBU check
        // cannot compare (no active header yet, and the activating header's own
        // already-seen obu_mlayer_id == 0 yields nothing new). Here two pre-header OBUs at
        // embedded layers 0 and 1 accumulate {0, 1} = 2 before the sequence header
        // activates with SeqMaxMlayerCnt 1; the activation-path retroactive check must
        // emit the exceedance, exactly once.
        let mut data = temporal_delimiter_obu();
        // Pre-header OBUs at embedded layers 0 and 1 (counted, no header active yet).
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        // The sequence header (embedded layer 0, forced by § 6.2.2) now activates with
        // SeqMaxMlayerCnt 1; its own obu_mlayer_id 0 was already counted above.
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| {
                    d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                        && d.spec_section.as_deref() == Some("6.4.1")
                })
                .count(),
            1,
            "a pre-header distinct-mlayer count must fire once on activation; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_accumulated_before_header_activation_within_seqmax_is_conforming() {
        // § 6.4.1: the same pre-header accumulation of embedded layers 0 and 1 ({0, 1} =
        // 2) is within budget when the activating header has SeqMaxMlayerCnt 2
        // (sequence_header_payload(0, 1)); the retroactive check must NOT fire.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_before_header_activation_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // § 6.4.1: caller-provided external HLS suppresses the retroactive activation-path
        // check exactly as it suppresses the eager per-OBU check — an out-of-band header
        // may carry a SeqMaxMlayerCnt this validator does not model.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "external HLS must suppress the retroactive distinct-mlayer check; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_before_frame_header_activation_is_flagged() {
        // § 6.4.1 / § 5.18.2: the retroactive distinct-mlayer comparison fires on the
        // frame-header load_sequence_header activation path, not only on an OBU-order
        // sequence-header activation. Extended layer 1 (which the xlayer-0 sequence
        // header does NOT activate by OBU order) accumulates {0, 1} = 2 distinct
        // obu_mlayer_id values before any header is active for it; a non-CLK frame
        // (OBU_REGULAR_TILE_GROUP) at xlayer 1 then references seq 0 (SeqMaxMlayerCnt 1)
        // and frame-confirms its activation. The activating frame's own obu_mlayer_id 0
        // is already in the set, so the eager count_distinct_mlayer yields nothing — only
        // the activation-path retroactive check can flag the exceedance.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        // Pre-header OBUs at xlayer 1, embedded layers 0 and 1 (no header active for
        // xlayer 1 yet, so they only accumulate the distinct count).
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
        // A non-CLK frame at xlayer 1, embedded layer 0, references seq 0: the § 5.18.2
        // activation that makes SeqMaxMlayerCnt available for xlayer 1.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert_eq!(
            report
                .errors()
                .filter(|d| {
                    d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                        && d.spec_section.as_deref() == Some("6.4.1")
                })
                .count(),
            1,
            "a pre-header count must fire once on frame-header activation; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_before_frame_header_activation_within_seqmax_is_conforming() {
        // § 6.4.1: the same pre-header accumulation at xlayer 1 ({0, 1} = 2) is within
        // budget when the frame-confirmed activating header has SeqMaxMlayerCnt 2
        // (sequence_header_payload(0, 1)); the frame-header-path retroactive check must
        // NOT fire.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
        );
    }

    #[test]
    fn distinct_mlayer_count_before_frame_header_activation_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // § 6.4.1: caller-provided external HLS suppresses the frame-header-path
        // retroactive check exactly as it suppresses the OBU-order-path and eager checks —
        // an out-of-band header may carry a SeqMaxMlayerCnt this validator does not model.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
        data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
            "external HLS must suppress the frame-header-path retroactive check; report was: {report}"
        );
    }

    // --- § 7.3.6 single active sequence header per extended layer per CVS (3.3) --

    #[test]
    fn second_activation_without_clk_is_flagged() {
        // § 7.3.6: a frame-confirmed activation of seq 0, then a non-CLK frame in the
        // same coded video sequence activating a different seq 1, violates the
        // single-active-sequence-header rule.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        // Two OBU_REGULAR_TILE_GROUP (type 7) frames, xlayer 0: confirm seq 0, then seq 1.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "hls/multiple-active-sequence-headers"
                    && d.spec_section.as_deref() == Some("7.3.6")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn reactivation_across_clk_is_conforming() {
        // § 7.3.6: a CLK starts a new coded video sequence, so re-activating a different
        // seq 1 across the CLK is permitted — the rule resets at each CLK.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
        // New temporal unit, then a CLK that starts a new CVS and activates seq 1.
        data.extend(temporal_delimiter_obu());
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK, ref seq 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
            "a CLK re-activation must not fire the § 7.3.6 check; report was: {report}"
        );
    }

    #[test]
    fn fallback_guess_then_frame_reference_is_not_flagged() {
        // § 7.3.6: when the prior activation was only the OBU-order fallback guess (not
        // frame-confirmed), the first frame referencing a different seq must not fire —
        // a guess a frame can contradict was never a real activation.
        let mut data = temporal_delimiter_obu();
        // OBU-order fallback activates seq 0 (first seen); seq 1 is also available.
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        // The first frame frame-confirms seq 1 (different from the fallback seq 0).
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
            "a fallback guess overridden by the first frame must not fire; report was: {report}"
        );
    }

    #[test]
    fn unreferenced_extra_sequence_header_is_conforming() {
        // § 7.3.6: additional sequence header OBUs with a different seq_header_id may be
        // present but unactivated; one never referenced by a frame must not fire.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        // The frame confirms seq 0.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        // An extra, unreferenced sequence header with a different id.
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
            "an unreferenced extra sequence header must not fire; report was: {report}"
        );
    }

    #[test]
    fn second_activation_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // § 7.3.6: under caller-provided external HLS the active sequence header may be
        // supplied out of band, so the in-band activation history is unreliable and the
        // check is suppressed even on the otherwise-firing two-activation stream.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
            "external HLS must suppress the § 7.3.6 check; report was: {report}"
        );
    }

    /// Builds the otherwise-firing two-activation stream of
    /// [`second_activation_without_clk_is_flagged`] (frame-confirm seq 0, then a non-CLK
    /// frame activating seq 1 in the same CVS for xlayer 0).
    fn two_activation_stream() -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // confirm seq 1
        data
    }

    #[test]
    fn second_activation_under_empty_external_hls_is_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // FIX 2 (Codex finding 3). `ExternalHlsSet::new()` declares an external channel
        // that declares NO sequence header, so it cannot supply an out-of-band active
        // sequence header (the validator emits hls/unavailable-sequence-header on that
        // premise elsewhere). The § 7.3.6 gate must therefore narrow to
        // declares_any_sequence_header() — an empty set must NOT suppress.
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report =
            Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "hls/multiple-active-sequence-headers"
                    && d.spec_section.as_deref() == Some("7.3.6")
            }),
            "an empty external set declares no sequence header and must not suppress; \
             report was: {report}"
        );
    }

    #[test]
    fn second_activation_under_sequence_free_external_hls_is_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // FIX 2: a non-empty external set that declares only an operating point set (no
        // sequence header) likewise cannot supply an out-of-band active sequence header,
        // so the § 7.3.6 check must still fire.
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_operating_point_set(0, 0),
            ),
        };
        let report =
            Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "hls/multiple-active-sequence-headers"
                    && d.spec_section.as_deref() == Some("7.3.6")
            }),
            "a sequence-header-free external set must not suppress; report was: {report}"
        );
    }

    #[test]
    fn second_activation_under_out_of_range_external_hls_id_is_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // FIX 2: an out-of-range declared id is ignored (options.rs), so the set declares
        // no usable sequence header and must not suppress the § 7.3.6 check (mirrors
        // external_hls_out_of_range_id_does_not_suppress_no_active_sequence_header).
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(16),
            ),
        };
        let report =
            Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "hls/multiple-active-sequence-headers"
                    && d.spec_section.as_deref() == Some("7.3.6")
            }),
            "an out-of-range external id is ignored and must not suppress; report was: {report}"
        );
    }

    // --- § 6.4.1 monotonic_output_order_flag agreement across a CMVS (3.4) -------

    /// A sequence-header payload (xlayer-neutral) with the given `seq_header_id`,
    /// `max_mlayer_id == 0`, and an explicit `monotonic_output_order_flag`.
    fn seq_header_payload_monotonic(seq_header_id: u32, monotonic: bool) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id = 0 (no seq_max_mlayer_cnt_minus_1 field)
        bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        // max_mlayer_id == 0 -> no mlayer_dependency_present_flag; max_tlayer_id == 0 ->
        // no tlayer_dependency_present_flag (§ 5.4.1).
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A sequence-header OBU for `xlayer` carrying [`seq_header_payload_monotonic`].
    fn seq_header_obu_monotonic(xlayer: u8, seq_header_id: u32, monotonic: bool) -> Vec<u8> {
        let payload = seq_header_payload_monotonic(seq_header_id, monotonic);
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    /// Builds a stream whose temporal unit 1 begins a § 7.3.2 CMVS (begin condition 1:
    /// a CLK temporal unit with an MSDO present and no CMVS yet active), with sequence
    /// headers for extended layers 0 and 1 whose `monotonic_output_order_flag` values
    /// are `monotonic_x0` and `monotonic_x1`. Temporal unit 2 (the CMVS is definitively
    /// `Inside` by then) *frame-confirms* both extended layers in turn — first xlayer 0,
    /// then xlayer 1 — so each layer is associated with its referenced sequence header
    /// per § 5.18.2 rather than the OBU-order fallback guess (§ 7.3.6 forbids treating
    /// an unreferenced extra header as activated). The cross-layer agreement check runs
    /// at each frame; the disagreement, if any, is emitted "when the second of the two
    /// headers is activated" (the xlayer-1 frame).
    fn cmvs_two_layer_stream(monotonic_x0: bool, monotonic_x1: bool) -> Vec<u8> {
        let mut data = temporal_delimiter_obu(); // starts temporal unit 1
        data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
        data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
        data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
        data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
        data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
        // Frame-confirm xlayer 0 (ref seq 0), then xlayer 1 (ref seq 1); the agreement
        // check runs at each, and the disagreement fires at the second activation.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
        data
    }

    #[test]
    fn monotonic_output_order_disagreement_inside_cmvs_is_flagged() {
        // § 6.4.1: inside an MSDO-begun CMVS, extended layers 0 (monotonic 1) and 1
        // (monotonic 0) disagree on monotonic_output_order_flag — flagged.
        let data = cmvs_two_layer_stream(true, false);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "report was: {report}"
        );
    }

    /// A single temporal unit that *opens* a § 7.3.2 CMVS (begin condition 1: a CLK
    /// temporal unit with an MSDO present and no CMVS yet active) and frame-confirms both
    /// extended layers WITHIN that same opening temporal unit. xlayer 0's CLK references
    /// seq 0 (`monotonic_x0`); xlayer 1's CLK references seq 1 (`monotonic_x1`). The CMVS
    /// membership is decidable at the CLK (§ 7.3.7: the at-most-one MSDO precedes every
    /// coded extended layer unit), so the cross-layer agreement check sees `Inside` when
    /// the second CLK activates — the begin direction of the boundary that the two-TU
    /// `cmvs_two_layer_stream` does not exercise.
    fn cmvs_two_layer_single_tu_stream(monotonic_x0: bool, monotonic_x1: bool) -> Vec<u8> {
        let mut data = temporal_delimiter_obu(); // single temporal unit
        data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
        data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
        data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
        // Both activations are CLK frame headers in this same opening temporal unit: the
        // first frame-confirms xlayer 0 (and, as the CLK, begins the CMVS), the second
        // frame-confirms xlayer 1 — the disagreement fires at the second activation.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
        data
    }

    #[test]
    fn monotonic_output_order_disagreement_in_cmvs_opening_tu_is_flagged() {
        // § 6.4.1 / § 7.3.2: two extended layers activating disagreeing
        // monotonic_output_order_flag values WITHIN the CMVS-opening temporal unit (MSDO +
        // CLKs + activations, a single temporal unit). § 7.3.7 makes the begin condition
        // decidable at the CLK, so the tracker reports `Inside` at the second activation
        // and the disagreement fires — the begin direction of the boundary (without it the
        // committed `Outside` of the previous temporal unit would stale-leak and the check
        // would miss this opening-temporal-unit disagreement).
        let data = cmvs_two_layer_single_tu_stream(true, false);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_agreement_in_cmvs_opening_tu_is_conforming() {
        // § 6.4.1: both extended layers agree (monotonic 1) within the CMVS-opening
        // temporal unit — no diagnostic. Guards the begin-direction adjustment against a
        // false positive on a conforming single-temporal-unit CMVS.
        let data = cmvs_two_layer_single_tu_stream(true, true);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "agreeing flags in the CMVS-opening temporal unit must not fire; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_agreement_inside_cmvs_is_conforming() {
        // § 6.4.1: inside the same CMVS, both extended layers agree (monotonic 1) — no
        // diagnostic.
        let data = cmvs_two_layer_stream(true, true);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "agreeing flags inside a CMVS must not fire; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_disagreement_outside_cmvs_is_not_flagged() {
        // § 6.4.1: the agreement requirement is scoped to a coded multistream video
        // sequence. With no MSDO and no global LCR, the CMVS tracker stays Outside, so
        // disagreeing monotonic_output_order_flag values across two extended layers do
        // not fire.
        let mut data = temporal_delimiter_obu();
        data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
        data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // activate xlayer 0
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // activate xlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "disagreement outside any CMVS must not fire; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_disagreement_in_unknown_cmvs_is_not_flagged() {
        // § 6.4.1 / § 7.3.2: a CLK temporal unit with a global LCR present but no MSDO
        // routes the CMVS tracker to Unknown (begin condition 3 needs an *activated*
        // global LCR, which is not modeled). The agreement check fires only in Inside,
        // so a disagreement while Unknown must not fire (conservative under-approximation).
        let mut data = temporal_delimiter_obu(); // temporal unit 1
        data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
        data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
        data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
        data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> Unknown (LCR present, no MSDO)
        data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Unknown
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // re-activate xlayer 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "disagreement while the CMVS tracker is Unknown must not fire; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_disagreement_in_cmvs_ending_tu_is_not_flagged() {
        // § 7.3.2 end condition 2 / § 7.3.7: a temporal unit that begins a new coded
        // video sequence (a CLK) but contains no MSDO and no activated global LCR ENDS
        // the active CMVS — that temporal unit is outside the CMVS. § 7.3.7 places the
        // optional MSDO before every coded extended layer unit, so MSDO absence is
        // already decidable at the CLK. A CLK in such a temporal unit that activates a
        // header disagreeing on monotonic_output_order_flag must therefore NOT fire,
        // even though the previous temporal unit left the tracker Inside.
        let mut data = temporal_delimiter_obu(); // temporal unit 1
        data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> begins the CMVS
        data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
        data.extend(annex_b_obu(0x04, &seq_header_payload_monotonic(1, false))); // seq 1 monotonic 0 (available)
        // CLK xlayer 0 referencing seq 0 frame-confirms xlayer 0 and begins the CMVS, so
        // xlayer 0 is a decidable association (isolating the end-of-CMVS state downgrade
        // from the fallback-guess gate).
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
        // Temporal unit 2: a CLK for xlayer 1 with NO MSDO ends the CMVS (end cond. 2);
        // it activates seq 1 (monotonic 0), disagreeing with xlayer 0's seq 0 (monotonic 1).
        data.extend(temporal_delimiter_obu());
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "an MSDO-less CLK temporal unit ends the CMVS; a disagreement activated there \
             is outside the CMVS and must not fire; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_unreferenced_extra_header_inside_cmvs_is_not_flagged() {
        // § 7.3.6: an additional sequence header with a different seq_header_id that no
        // frame references "is not activated and has no effect on the decoding process".
        // Inside a CMVS, xlayer 1 carries an extra never-referenced header (seq 2,
        // monotonic 0) before the header (seq 1, monotonic 1) its frame actually
        // references; xlayer 0 (seq 0, monotonic 1) agrees with the *referenced* xlayer-1
        // header, so the unreferenced disagreeing guess must not fire the check.
        let mut data = temporal_delimiter_obu(); // temporal unit 1
        data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
        data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
        // xlayer 1 sends the unreferenced extra header (seq 2, monotonic 0) FIRST, then
        // its referenced header (seq 1, monotonic 1). The OBU-order fallback for xlayer 1
        // is the never-activated seq 2.
        data.extend(seq_header_obu_monotonic(1, 2, false)); // xlayer 1 extra, unreferenced
        data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 referenced header
        data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
        data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm xlayer 0 (seq 0)
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // frame-confirm xlayer 1 (seq 1)
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "an unreferenced extra header with a differing flag must not fire (§ 7.3.6 \
             leaves it unactivated); report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_disagreement_under_external_hls_is_not_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // § 6.4.1: under caller-provided external HLS that declares a sequence header (the
        // in-use seq 0), an externally-activated sequence header has an unmodeled
        // monotonic_output_order_flag, so the cross-layer comparison is unreliable and
        // suppressed even on the otherwise-firing inside-CMVS disagreement stream. This is
        // the positive coverage for the narrowed (declares_any_sequence_header()) gate's
        // suppression branch (FIX 3 test 5).
        let data = cmvs_two_layer_stream(true, false);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "external HLS declaring a sequence header must suppress the § 6.4.1 monotonic \
             agreement check; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_disagreement_under_empty_external_hls_is_flagged() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // FIX 3 (Codex finding 4). `ExternalHlsSet::new()` declares an external channel
        // that declares NO sequence header, so it cannot supply an out-of-band active
        // sequence header. The § 6.4.1 monotonic gate must narrow to
        // declares_any_sequence_header() (as validate_active_sequence_limits and the
        // distinct-mlayer gate do), so an empty set must NOT suppress the inside-CMVS
        // disagreement. The stream fires mid-CMVS (TU2 has no CLK, so it stays Inside),
        // exercising the gate rather than the FIX 1 deferral drop.
        let data = cmvs_two_layer_stream(true, false);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "an empty external set declares no sequence header and must not suppress the \
             § 6.4.1 monotonic agreement check; report was: {report}"
        );
    }

    /// Builds the first two temporal units shared by the
    /// `monotonic_output_order_*_provisional_*` tests: TU1 opens a CMVS (MSDO + CLK,
    /// begin condition 1) and frame-confirms xlayer 0 to seq 0 (`monotonic 1`); xlayer 1
    /// carries seq 1 (`monotonic 1`). TU2 frame-confirms xlayer 1 to seq 1. Both layers
    /// agree on `monotonic_output_order_flag == 1` and the CMVS is committed `Inside`
    /// after TU2. The caller appends a TU3 whose shape exercises the provisional-Inside
    /// deferral.
    fn cmvs_provisional_inside_prefix() -> Vec<u8> {
        let mut data = temporal_delimiter_obu(); // temporal unit 1
        data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
        data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
        // CLK xlayer 0 referencing seq 0 begins the CMVS and frame-confirms xlayer 0
        // (kept before xlayer 1's header so coded extended layer units stay ascending).
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 seq 1 monotonic 1
        data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS committed Inside
        // Frame-confirm xlayer 1 (ref seq 1); both layers now agree (monotonic 1).
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
        data
    }

    #[test]
    fn monotonic_output_order_provisional_inside_clk_ending_tu_is_not_flagged() {
        // FIX 1 false-positive regression (Codex "Defer CMVS agreement until pre-CLK
        // headers are scoped"). § 7.3.2 end condition 2 (mirror
        // `07-decoding-process.md` lines 335-341): a temporal unit that begins a new
        // coded video sequence for an extended layer but contains no OBU_MSDO and no
        // activated global LCR ENDS the CMVS, so it sits OUTSIDE. When a same-id
        // reconfiguration of seq 0 (now monotonic 0) is observed at the *top* of such a
        // temporal unit, the CLK that ends the CMVS has not yet been observed, so the
        // committed `Inside` is provisional. The agreement check must defer its
        // header-time verdict and drop it once the CLK confirms the temporal unit ended
        // the CMVS — emitting at header time would be a false positive on a conformant
        // redefinition (§ 7.3.6 permits redefinition when a new CVS follows, mirror
        // `07-decoding-process.md` lines 608-611).
        let mut data = cmvs_provisional_inside_prefix();
        data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
        // Same-id reconfiguration of seq 0 with the disagreeing flag, observed BEFORE the
        // CLK that ends the CMVS for this temporal unit.
        data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
        // CLK xlayer 0 referencing seq 0: an MSDO-less CLK temporal unit ends the CMVS.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "a header redefinition at the top of a CMVS-ending CLK temporal unit is outside \
             the CMVS once the CLK is seen; the provisional header-time verdict must be \
             dropped; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_provisional_inside_mid_cmvs_redefinition_is_flagged() {
        // FIX 1 deferral-still-emits guard. Same TU3 shape as the false-positive case but
        // the temporal unit stays *inside* the CMVS (a non-CLK frame replaces the CLK), so
        // the deferred header-time verdict must be emitted at temporal-unit flush. A
        // mid-CMVS redefinition that disagrees on monotonic_output_order_flag is a genuine
        // § 6.4.1 violation.
        let mut data = cmvs_provisional_inside_prefix();
        data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO, no CLK)
        // Same-id reconfiguration of seq 0 with the disagreeing flag; no CLK follows, so
        // the temporal unit stays inside the CMVS.
        data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
        // A non-CLK frame for xlayer 0 keeps the CMVS Inside across this temporal unit.
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "a mid-CMVS redefinition disagreeing on monotonic_output_order_flag must be \
             emitted at temporal-unit flush; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_provisional_inside_flushes_at_end_of_bitstream() {
        // FIX 1 end-of-bitstream flush. The disagreeing redefinition is the last OBU: the
        // temporal unit never receives a CLK, so it stays inside the CMVS (§ 7.3.2 end
        // condition 3 closes the CMVS only at the end of the bitstream). The deferred
        // verdict must be emitted when `finish` flushes the final temporal unit.
        let mut data = cmvs_provisional_inside_prefix();
        data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
        // Same-id reconfiguration disagreeing on the flag, with no following frame at all.
        data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                    && d.spec_section.as_deref() == Some("6.4.1")
            }),
            "a disagreeing redefinition with no following CLK stays inside the CMVS and must \
             be emitted at the end-of-bitstream flush; report was: {report}"
        );
    }

    #[test]
    fn monotonic_output_order_provisional_inside_unknown_clk_is_not_flagged() {
        // FIX 1 Unknown guard. A CLK temporal unit with an activated-global-LCR candidate
        // present but no MSDO routes the CMVS tracker to `Unknown` (§ 7.3.2 end condition 2
        // needs "no activated global layer configuration record"; activation is not
        // modeled). The check fires only on `Inside`, so the deferred header-time verdict
        // for a redefinition at the top of such a temporal unit must be dropped.
        let mut data = cmvs_provisional_inside_prefix();
        data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
        data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
        // Same-id reconfiguration disagreeing on the flag, observed before the CLK.
        data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
        // CLK xlayer 0 with the global LCR present and no MSDO -> tracker goes Unknown.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
            "a CLK temporal unit with a global LCR and no MSDO routes the tracker to \
             Unknown; the provisional verdict must be dropped; report was: {report}"
        );
    }

    // ---------------------------------------------------------------------------
    // Annex A: profiles, levels, and tiers (AV2-A-PROFILES / AV2-A-LEVELS-TIERS).
    // ---------------------------------------------------------------------------

    /// Tunable knobs for [`annex_a_seq_payload`], a complete, frame-activatable §5.4
    /// sequence header (xlayer 0, `max_tlayer_id`/`max_mlayer_id` 0, monotonic output).
    #[derive(Clone, Copy)]
    struct AnnexASeq {
        seq_id: u32,
        profile_idc: u32,
        level_idx: u32,
        /// `seq_tier` bit; only signaled (and thus only meaningful) when
        /// `level_idx > 3`.
        high_tier: bool,
        chroma_format_idc: u32,
        bit_depth_idc: u32,
        max_frame_width_minus_1: u32,
        max_frame_height_minus_1: u32,
        frame_dim_bits_minus_1: u32,
    }

    impl AnnexASeq {
        /// Profile 0, level 0 (2.0), Main tier, 4:2:0, 10-bit, 16x16 maximum frame.
        fn base() -> Self {
            Self {
                seq_id: 0,
                profile_idc: 0,
                level_idx: 0,
                high_tier: false,
                chroma_format_idc: 0, // CHROMA_FORMAT_420
                bit_depth_idc: 0,     // 10-bit
                max_frame_width_minus_1: 15,
                max_frame_height_minus_1: 15,
                frame_dim_bits_minus_1: 7, // 8-bit frame dimensions
            }
        }
    }

    /// A complete §5.4 sequence header (non-single-picture, BLOCK_64X64, every tool
    /// flag cleared) with the profile/level/tier/chroma/bit-depth and frame dimensions
    /// from `o`, ready to be activated by a frame referencing `o.seq_id`. `seq_tier` is
    /// read only when `seq_level_idx > 3` (§5.4.1), matching the parser.
    fn annex_a_seq_payload(o: AnnexASeq) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(o.seq_id);
        bits.f(o.profile_idc, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(o.level_idx, 5); // seq_level_idx
        if o.level_idx > 3 {
            bits.bit(u8::from(o.high_tier)); // seq_tier
        }
        bits.uvlc(o.chroma_format_idc); // chroma_format_idc
        bits.uvlc(o.bit_depth_idc); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id
        bits.f(0, 3); // max_mlayer_id == 0
        bits.bit(1); // monotonic_output_order_flag
        bits.f(o.frame_dim_bits_minus_1, 4); // frame_width_bits_minus_1
        bits.f(o.frame_dim_bits_minus_1, 4); // frame_height_bits_minus_1
        bits.f(o.max_frame_width_minus_1, o.frame_dim_bits_minus_1 + 1);
        bits.f(o.max_frame_height_minus_1, o.frame_dim_bits_minus_1 + 1);
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        let monochrome = o.chroma_format_idc == 1; // CHROMA_FORMAT_400
        append_annex_a_child_configs(&mut bits, monochrome);
        bits.into_bytes()
    }

    /// A sequence-header OBU (xlayer 0) carrying [`annex_a_seq_payload`].
    fn annex_a_seq_obu(o: AnnexASeq) -> Vec<u8> {
        annex_b_obu(0x04, &annex_a_seq_payload(o))
    }

    /// Temporal delimiter + the [`annex_a_seq_payload`] sequence header for xlayer 0.
    fn td_and_annex_a_seq(o: AnnexASeq) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_a_seq_obu(o));
        data
    }

    /// [`td_and_annex_a_seq`] plus a minimal CLK frame (xlayer 0) that references
    /// `o.seq_id`, *frame-confirming* the header's activation (§ 5.18.2
    /// load_sequence_header) without driving the frame-core parse. The Annex A
    /// *value-space* checks fire only for a frame-confirmed activation (a staged
    /// OBU-order fallback is a guess that defers, § 7.3.6), so these checks need the
    /// confirming frame; the static *level-limit* checks instead use the fuller
    /// [`annex_a_frame_obu`], which both confirms and parses the frame core.
    fn td_seq_and_confirming_frame(o: AnnexASeq) -> Vec<u8> {
        let seq_id = o.seq_id;
        let mut data = td_and_annex_a_seq(o);
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, seq_id)); // CLK xlayer 0
        data
    }

    /// Appends the §5.4 child configs for a non-single-picture sequence header with
    /// every tool flag cleared, gating the chroma-only reads on `monochrome` exactly as
    /// the parser does, then the §5.2.1 payload tail.
    fn append_annex_a_child_configs(bits: &mut Bits, monochrome: bool) {
        // sequence_partition_config (BLOCK_64X64, SDP off)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock
        if !monochrome {
            bits.bit(0); // enable_sdp
        }
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg
        bits.bit(0); // seq_seg_info_present_flag
        // sequence_intra_config
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        if !monochrome {
            bits.f(0, 2); // cfl_ds_filter_index
        }
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (non-single-picture branch)
        bits.f(0, 4); // seq_enabled_motion_modes
        bits.bit(0); // enable_masked_compound
        bits.bit(0); // enable_ref_frame_mvs
        bits.f(0, 4); // order_hint_bits_minus_1
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder
        bits.bit(0); // explicit_ref_frame_map
        bits.bit(0); // explicit_num_ref_frames
        bits.f(0, 3); // long_term_frame_id_bits
        bits.f(0, 2); // seq_max_drl_bits_minus_1 (ns(5) -> 0)
        bits.bit(0); // allow_frame_max_drl_bits
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 (ns(3) -> 0)
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.f(0, 2); // num_same_ref_compound
        bits.bit(0); // enable_tip
        bits.bit(0); // enable_mv_traj
        bits.bit(0); // enable_bawp
        bits.bit(0); // enable_cwp
        bits.bit(0); // enable_imp_msk_bld
        bits.bit(0); // enable_df_sub_pu
        bits.f(0, 2); // enable_opfl_refine
        bits.bit(0); // enable_refinemv
        bits.bit(0); // enable_bru
        bits.bit(0); // enable_adaptive_mvd
        bits.bit(0); // enable_mvd_sign_derive
        bits.bit(0); // enable_flex_mvres
        bits.bit(0); // enable_global_motion
        bits.bit(0); // enable_short_refresh_frame_flags
        // sequence_scc_config (SELECT both)
        bits.bit(1); // seq_choose_screen_content_tools
        bits.bit(1); // seq_choose_integer_mv
        // sequence_transform_quant_entropy_config
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        if !monochrome {
            bits.bit(0); // enable_chroma_dctonly
        }
        bits.bit(0); // enable_inter_ddt
        bits.bit(0); // reduced_tx_part_set
        if !monochrome {
            bits.bit(0); // enable_cctx
        }
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // enable_avg_cdf
        if !monochrome {
            bits.bit(0); // separate_uv_delta_q
        }
        bits.bit(1); // equal_ac_dc_q
        if !monochrome {
            bits.f(0, 5); // base_uv_ac_delta_q
            bits.bit(0); // uv_ac_delta_q_enabled
        }
        // sequence_filter_config (BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.bit(0); // cdef_on_skip_txfm_always_on
        bits.bit(0); // cdef_on_skip_txfm_disabled
        bits.f(0, 2); // df_par_bits_minus_2
        // sequence_tile_config
        bits.bit(0); // seq_tile_info_present_flag
        bits.bit(0); // film_grain_params_present
        extensible_obu_tail(bits);
    }

    /// A CLK frame OBU (xlayer 0) that references `seq_id`, drives `frame_size()` to an
    /// override `FrameWidth` x `FrameHeight`, and reaches `tile_info()` — the parsed
    /// intra-frame path the Annex A level-limit checks consume.
    ///
    /// `frame_dim_bits` is the active sequence header's `frame_*_bits` (8 for the
    /// [`AnnexASeq`] defaults). For the single-tile uniform `tile_info()` (§ 5.18.7.2),
    /// `col_increment_bits` / `row_increment_bits` are the number of
    /// `increment_tile_cols_log2` / `increment_tile_rows_log2` stop bits the parser
    /// reads: a single `0` when the frame spans more than one superblock column (resp.
    /// row) of the BLOCK_64X64 grid and the level allows a wider single tile, else `0`.
    /// Use [`annex_a_single_tile_increments`] to compute these for a given level/frame.
    fn annex_a_frame_obu(
        seq_id: u32,
        width: u32,
        height: u32,
        frame_dim_bits: u32,
        col_increment_bits: u32,
        row_increment_bits: u32,
    ) -> Vec<u8> {
        let mut fb = Bits::default();
        fb.bit(1); // is_first_tile_group
        fb.uvlc(0); // cur_mfh_id == 0
        fb.uvlc(seq_id); // seq_header_id_in_frame_header
        fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
        fb.bit(1); // frame_size_override_flag
        fb.f(0, 1); // order_hint f(OrderHintBits == 1)
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        fb.f(width - 1, frame_dim_bits); // frame_width_minus_1
        fb.f(height - 1, frame_dim_bits); // frame_height_minus_1
        fb.bit(0); // allow_screen_content_tools
        fb.bit(0); // allow_intrabc
        fb.bit(0); // disable_cdf_update
        // tile_info() (§ 5.18.7.2 -> tile_params, § 5.18.7.3) for a single tile.
        fb.bit(1); // uniform_tile_spacing_flag
        for _ in 0..col_increment_bits {
            fb.bit(0); // increment_tile_cols_log2 stop bit
        }
        for _ in 0..row_increment_bits {
            fb.bit(0); // increment_tile_rows_log2 stop bit
        }
        fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
        fb.bit(0); // segmentation_enabled
        fb.bit(0); // using_qmatrix
        fb.bit(0); // delta_q_present
        annex_b_obu(CLK_HEADER, &fb.into_bytes())
    }

    /// Computes the single-tile uniform `tile_info()` `increment_tile_cols_log2` /
    /// `increment_tile_rows_log2` stop-bit counts for `(width, height)` at level 2.0
    /// (LevelIdx 0), Main tier, BLOCK_64X64 (the [`AnnexASeq`] base). Mirrors
    /// `parse_tile_layout` (§ 5.18.7.3): a single `0` stop bit when the dimension spans
    /// more than one superblock and a wider single tile is allowed, else none.
    fn annex_a_single_tile_increments(width: u32, height: u32) -> (u32, u32) {
        // BLOCK_64X64: sb4x4 = 16, sbShift = 4 (§ 9.3).
        let sb_cols = (2 * ((width + 7) >> 3) + 15) >> 4;
        let sb_rows = (2 * ((height + 7) >> 3) + 15) >> 4;
        // Level 2.0 Main tier: width_sf = 4, area_sf = 4 (tile.rs scaling tables).
        let max_tile_width_sb = (4 * 4096) >> (4 + 4); // == 64
        let max_tile_area_sb = (4u32 * 4096 * 2304) >> (2 * (4 + 2) + 2); // == 2304
        fn tile_log2(blk: u32, target: u32) -> u32 {
            let mut k = 0;
            while (blk << k) < target {
                k += 1;
            }
            k
        }
        let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
        let max_log2_tile_cols = tile_log2(1, sb_cols.min(64));
        let max_log2_tile_rows = tile_log2(1, sb_rows.min(64));
        let min_log2_tiles = min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows * sb_cols));
        // A single column tile: emit one stop bit iff the loop would run (min < max).
        let col_bits = u32::from(min_log2_tile_cols < max_log2_tile_cols);
        // After the single column tile, tile_cols_log2 == 0; the row loop starts at
        // (min_log2_tiles - 0) and reads a stop bit iff it is below max_log2_tile_rows.
        let min_log2_tile_rows = min_log2_tiles; // tile_cols_log2 == 0 for one column tile
        let row_bits = u32::from(min_log2_tile_rows < max_log2_tile_rows);
        (col_bits, row_bits)
    }

    // --- Profile / chroma / bit-depth value-space (Annex A.2 Table A.1) ---

    #[test]
    fn annex_a_flags_reserved_profile() {
        // seq_profile_idc 5 is in the reserved range 5-30 (Table A.1).
        let data = td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 5,
            ..AnnexASeq::base()
        });
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/profile-reserved" && d.spec_section.as_deref() == Some("A.2")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_profile_4_and_30_boundary() {
        // Profile 4 is the last defined profile (not reserved); profile 30 is the last
        // reserved value, profile 31 is Configurable (not reserved).
        let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 4,
            chroma_format_idc: 0, // 4:2:0 is allowed under profile 4
            ..AnnexASeq::base()
        }));
        assert!(
            !ok.errors().any(|d| d.rule_id == "annex-a/profile-reserved"),
            "profile 4 is defined, not reserved; report was: {ok}"
        );
        let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 30,
            ..AnnexASeq::base()
        }));
        assert!(
            bad.errors()
                .any(|d| d.rule_id == "annex-a/profile-reserved"),
            "profile 30 is reserved; report was: {bad}"
        );
    }

    #[test]
    fn annex_a_flags_chroma_format_mismatch_under_profile() {
        // Profile 0 allows only CHROMA_FORMAT_400 / CHROMA_FORMAT_420; chroma_format_idc
        // 3 (CHROMA_FORMAT_422) is outside its set (Table A.1).
        let data = td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 0,
            chroma_format_idc: 3, // CHROMA_FORMAT_422
            ..AnnexASeq::base()
        });
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_profile_3_allows_422_but_not_444() {
        // Profile 3 (Main_422) adds CHROMA_FORMAT_422 (idc 3) but not CHROMA_FORMAT_444
        // (idc 2) (Table A.1).
        let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 3,
            chroma_format_idc: 3, // CHROMA_FORMAT_422
            ..AnnexASeq::base()
        }));
        assert!(
            !ok.errors()
                .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
            "profile 3 allows 4:2:2; report was: {ok}"
        );
        let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 3,
            chroma_format_idc: 2, // CHROMA_FORMAT_444
            ..AnnexASeq::base()
        }));
        assert!(
            bad.errors()
                .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
            "profile 3 does not allow 4:4:4; report was: {bad}"
        );
    }

    #[test]
    fn annex_a_profile_4_allows_444_but_not_422() {
        // Profile 4 (Main_444) adds CHROMA_FORMAT_444 (idc 2) but not CHROMA_FORMAT_422
        // (idc 3) (Table A.1).
        let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 4,
            chroma_format_idc: 2, // CHROMA_FORMAT_444
            ..AnnexASeq::base()
        }));
        assert!(
            !ok.errors()
                .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
            "profile 4 allows 4:4:4; report was: {ok}"
        );
        let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 4,
            chroma_format_idc: 3, // CHROMA_FORMAT_422
            ..AnnexASeq::base()
        }));
        assert!(
            bad.errors()
                .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
            "profile 4 does not allow 4:2:2; report was: {bad}"
        );
    }

    #[test]
    fn annex_a_configurable_profile_is_unconstrained() {
        // Profile 31 (Configurable) leaves chroma/bit-depth unconstrained (Table A.1
        // dashes): a 4:2:2 sequence under it must not be flagged, and 31 is not reserved.
        let data = td_seq_and_confirming_frame(AnnexASeq {
            profile_idc: 31,
            chroma_format_idc: 3, // CHROMA_FORMAT_422
            ..AnnexASeq::base()
        });
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/profile-")),
            "the Configurable profile is unconstrained; report was: {report}"
        );
    }

    // --- Level / tier value-space (Annex A.4 Tables A.7 / A.9 NOTE) ---

    #[test]
    fn annex_a_flags_reserved_level() {
        // seq_level_idx 25 is in the reserved range 22-30 (Table A.7).
        let data = td_seq_and_confirming_frame(AnnexASeq {
            level_idx: 25,
            ..AnnexASeq::base()
        });
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_value_space_rechecked_on_same_id_redefinition_with_different_level() {
        // § 7.3.6 permits re-sending the activated seq_header_id with different content.
        // The Annex A value-space dedup key carries a fingerprint of the checked fields,
        // so a same-id redefinition whose seq_level_idx changes from a clean value to a
        // reserved one re-runs the check and flags it — rather than being suppressed by
        // the first (clean) activation's key.
        let mut data = temporal_delimiter_obu();
        // First activation: seq_header_id 0 at a defined level (0 == 2.0) — clean —
        // frame-confirmed by a CLK that references it.
        data.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 0,
            ..AnnexASeq::base()
        }));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm clean activation
        // Redefinition of the SAME seq_header_id 0 (still active for xlayer 0) at a
        // reserved level (25, in 22-30), re-confirmed by another CLK: re-activates and
        // must re-run the value-space check (the dedup key's fingerprint changed).
        data.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 25,
            ..AnnexASeq::base()
        }));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm redefinition
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
            }),
            "a same-id redefinition with a reserved seq_level_idx must re-run the Annex A \
             value-space check and flag the reserved level; report was: {report}"
        );
    }

    #[test]
    fn annex_a_value_space_deferred_until_a_frame_confirms_a_staged_header() {
        // § 7.3.6 allows staging multiple sequence headers before any frame activates
        // one. With two distinct staged headers and no frame, the OBU-order first-seen
        // fallback for xlayer 0 is a guess a later frame can contradict — so the Annex A
        // value-space check must NOT fire for the staged reserved-level header (a
        // value-space error against the guess could not be retracted). Once a frame
        // confirms the reserved-level header, the deferred check runs and flags it.
        let mut staged = temporal_delimiter_obu();
        // The reserved-level header (id 0, level 25) is seen FIRST, so the OBU-order
        // fallback makes it the active guess for xlayer 0 — but a second staged header
        // (id 1, clean) means the activation is NOT yet decidable (a later frame could
        // reference id 1 instead). The reserved-level value-space error must therefore be
        // deferred, not fired against the guess.
        staged.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 25, // reserved 22-30
            ..AnnexASeq::base()
        }));
        staged.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 1,
            level_idx: 0, // clean — defeats the sole-header decidability shortcut
            ..AnnexASeq::base()
        }));
        let staged_report = Validator::new(false).validate_bytes(&staged);
        assert!(
            !staged_report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/")),
            "two headers staged before any frame must not fire any Annex A value-space \
             diagnostic against the OBU-order fallback guess; report was: {staged_report}"
        );

        // Now a CLK frame on xlayer 0 references the reserved-level id 0, confirming its
        // activation: the deferred check runs and flags the reserved level.
        let mut confirmed = staged.clone();
        confirmed.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let confirmed_report = Validator::new(false).validate_bytes(&confirmed);
        assert!(
            confirmed_report.errors().any(|d| {
                d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
            }),
            "once a frame confirms the reserved-level header, the deferred Annex A check \
             must fire; report was: {confirmed_report}"
        );
    }

    #[test]
    fn annex_a_value_space_fires_for_in_band_header_under_external_hls() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // External HLS declares an unrelated header (id 5, < MAX_SEQ_NUM). The activating
        // frame still resolves to an IN-BAND header (id 0) whose seq_level_idx is reserved
        // — a locally decidable Annex A value-space fact that an external declaration
        // cannot shadow. The check must fire even under Provided mode (unlike the
        // agreement checks, which a Provided external header genuinely suppresses).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 25, // reserved 22-30
            ..AnnexASeq::base()
        }));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK ref in-band seq 0
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
            }),
            "an in-band reserved level activated by a frame must be flagged even when \
             external HLS declares an unrelated header; report was: {report}"
        );
    }

    #[test]
    fn annex_a_value_space_silent_for_external_only_activation() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // External HLS declares header id 5 and a frame references it via cur_mfh_id /
        // direct seq ref, but no in-band sequence header exists. The active header is
        // out-of-band content this validator does not model, so no Annex A value-space
        // fact is decidable — nothing must fire (unknown content never produces a
        // value-space diagnostic).
        let mut data = temporal_delimiter_obu();
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // ref external-only seq 5
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(5),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("annex-a/")),
            "an external-only activation with no in-band header must not produce any \
             Annex A value-space diagnostic; report was: {report}"
        );
    }

    #[test]
    fn annex_a_value_space_redefinition_rechecks_all_layers_using_the_id() {
        // Header id 0 is active for xlayer 0 (CLK frame) and xlayer 1 (a layer frame),
        // both referencing seq 0 at a clean level. A later same-id redefinition of seq 0
        // (still active for both layers) flips seq_level_idx to a reserved value: the
        // Annex A value-space recheck must cover EVERY extended layer the id is active
        // for, not only the redefinition-activating layer. The diagnostic is anchored at
        // the (shared) defining sequence-header OBU and deduped per
        // (xlayer, seq_header_id, cvs_epoch, fingerprint), so it fires once per affected
        // layer key.
        // TU 1: clean activation of seq 0, frame-confirmed for xlayer 0 then xlayer 1
        // (ascending obu_xlayer_id order, § 7.3.7). `frame_confirmed_xlayers` is monotonic,
        // so both stay confirmed afterward.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 0,
            ..AnnexASeq::base()
        }));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 ref seq 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 0)); // CLK xlayer 1 ref seq 0
        // TU 2: same-id redefinition of seq 0 flipping the level to a reserved value (25),
        // re-confirmed by an xlayer-0 CLK. seq 0 is still active for BOTH xlayer 0 and
        // xlayer 1, so the value-space-fingerprint-change recheck must cover both even
        // though only xlayer 0 re-confirms here.
        data.extend(temporal_delimiter_obu());
        data.extend(annex_a_seq_obu(AnnexASeq {
            seq_id: 0,
            level_idx: 25,
            ..AnnexASeq::base()
        }));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // re-activate seq 0 (xlayer 0)
        let report = Validator::new(false).validate_bytes(&data);
        let reserved_level_count = report
            .errors()
            .filter(|d| {
                d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
            })
            .count();
        assert!(
            reserved_level_count >= 2,
            "a redefinition flipping the level to reserved must re-run the Annex A check \
             for every extended layer (0 and 1) the id is active for, firing once per \
             affected layer key; got {reserved_level_count} reserved-level diagnostics. \
             report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_level_21_and_31() {
        // LevelIdx 21 (8.3) is the last defined level; 31 is Maximum parameters (valid);
        // 22 is the first reserved value.
        for level_idx in [21u32, 31] {
            let report =
                Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
                    level_idx,
                    ..AnnexASeq::base()
                }));
            assert!(
                !report
                    .errors()
                    .any(|d| d.rule_id == "annex-a/level-reserved"),
                "level {level_idx} is valid; report was: {report}"
            );
        }
        let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
            level_idx: 22,
            ..AnnexASeq::base()
        }));
        assert!(
            bad.errors().any(|d| d.rule_id == "annex-a/level-reserved"),
            "level 22 is reserved; report was: {bad}"
        );
    }

    #[test]
    fn annex_a_high_tier_below_level_4_0_is_unreachable_in_syntax() {
        // Spec-honesty boundary: `annex-a/high-tier-below-4-0` (the Table A.9 NOTE,
        // mirror lines 436-437) is a warning, but the § 5.4.1 parser only reads seq_tier
        // when seq_level_idx > 3 (i.e. LevelIdx 4 == level 4.0 and above) — for any lower
        // level it infers Tier::Main. So a *signaled* High tier below 4.0 cannot occur in
        // a parseable stream, and the warning never fires. This test pins that: even with
        // the High-tier knob set at level 0, the parser infers Main and no warning is
        // emitted. The check is kept as a documented, defensive guard.
        let report = Validator::new(false).validate_bytes(&td_and_annex_a_seq(AnnexASeq {
            level_idx: 0,
            high_tier: true, // not signaled at level 0; the parser infers Main tier
            ..AnnexASeq::base()
        }));
        assert!(
            report
                .warnings()
                .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
            "seq_tier is not signaled below level_idx 4, so High tier below 4.0 is \
             unreachable; report was: {report}"
        );
        assert!(
            report.is_conformant(),
            "a level-0 Main-tier stream is conformant; report was: {report}"
        );
    }

    #[test]
    fn annex_a_high_tier_at_level_4_0_is_accepted() {
        // seq_tier High at LevelIdx 4 (level 4.0) is allowed (Table A.9 NOTE: 4.0 and
        // above), so no high-tier warning.
        let report = Validator::new(false).validate_bytes(&td_and_annex_a_seq(AnnexASeq {
            level_idx: 4,
            high_tier: true,
            ..AnnexASeq::base()
        }));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/high-tier-below-4-0"),
            "High tier at level 4.0 is allowed; report was: {report}"
        );
        assert!(
            report
                .warnings()
                .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
            "no high-tier warning at level 4.0; report was: {report}"
        );
    }

    // --- Static level limits on the parsed intra frame path (Annex A.4) ---

    /// A level-2.0 sequence header with 10-bit frame dimensions (max 1024), plus a
    /// frame of the given size that reaches `tile_info()` with the right single-tile
    /// increment bits, all wrapped with a temporal delimiter prefix.
    fn level_2_0_stream(width: u32, height: u32) -> Vec<u8> {
        let seq = AnnexASeq {
            level_idx: 0,
            frame_dim_bits_minus_1: 9, // 10-bit frame dims (max 1024)
            max_frame_width_minus_1: 1023,
            max_frame_height_minus_1: 1023,
            ..AnnexASeq::base()
        };
        let mut data = td_and_annex_a_seq(seq);
        let (col, row) = annex_a_single_tile_increments(width, height);
        data.extend(annex_a_frame_obu(0, width, height, 10, col, row));
        data
    }

    #[test]
    fn annex_a_frame_width_exceeds_max_h_size() {
        // Level 2.0 (LevelIdx 0) MaxHSize is 640. FrameWidth 641 (> 640) with a short
        // height stays under MaxPicSize 147456, isolating the MaxHSize limit (fail-past).
        let report = Validator::new(false).validate_bytes(&level_2_0_stream(641, 16));
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/frame-size-exceeds-level" && d.message.contains("MaxHSize")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_frame_at_max_h_size_passes() {
        // FrameWidth exactly 640 == MaxHSize passes (boundary, pass-at-limit).
        let report = Validator::new(false).validate_bytes(&level_2_0_stream(640, 16));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/frame-size-exceeds-level"),
            "FrameWidth 640 == MaxHSize 640 must pass; report was: {report}"
        );
    }

    #[test]
    fn annex_a_frame_pic_size_exceeds_level() {
        // Level 2.0 MaxPicSize is 147456. FrameWidth 640 x FrameHeight 640 = 409600 >
        // 147456 (both dimensions are within MaxHSize/MaxVSize 640, isolating the
        // pic-size limit).
        let report = Validator::new(false).validate_bytes(&level_2_0_stream(640, 640));
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/frame-size-exceeds-level" && d.message.contains("MaxPicSize")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_frame_below_minimum_dimension() {
        // FrameWidth < 16 violates the Annex A.4 minimum-dimension rule. An 8-wide frame
        // has sbCols == 1 -> no increment bit.
        let report = Validator::new(false).validate_bytes(&level_2_0_stream(8, 16));
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/frame-size-below-minimum"
                    && d.spec_section.as_deref() == Some("A.4")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_frame_at_minimum_dimension_passes() {
        // FrameWidth == FrameHeight == 16 is exactly the minimum (boundary).
        let report = Validator::new(false).validate_bytes(&level_2_0_stream(16, 16));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/frame-size-below-minimum"),
            "16x16 is the minimum and must pass; report was: {report}"
        );
    }

    #[test]
    fn annex_a_level_31_disables_level_limits() {
        // seq_level_idx 31 (Maximum parameters): no level-based constraints, so a huge
        // frame that would blow past every level-2.0 limit must not be flagged.
        let seq = AnnexASeq {
            level_idx: 31,
            frame_dim_bits_minus_1: 11, // 12-bit dims (max 4096)
            max_frame_width_minus_1: 4095,
            max_frame_height_minus_1: 4095,
            ..AnnexASeq::base()
        };
        let mut data = td_and_annex_a_seq(seq);
        // Level 31 (NO_LEVEL) tile layout: max_tile_width_sb == sbCols, so a single tile
        // reads one column and one row stop bit for a 4000x4000 (63x63 superblock) frame.
        data.extend(annex_a_frame_obu(0, 4000, 4000, 12, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/frame-size-")),
            "level 31 disables all level-limit checks; report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/tile-count-exceeds-level"),
            "level 31 disables the tile-count check too; report was: {report}"
        );
    }

    #[test]
    fn annex_a_reserved_level_disables_level_limits() {
        // A reserved seq_level_idx (22-30) is not in Tables A.8/A.9, so the level-limit
        // checks are disabled (the reserved-level value-space error still fires).
        let seq = AnnexASeq {
            level_idx: 22,
            frame_dim_bits_minus_1: 9,
            max_frame_width_minus_1: 1023,
            max_frame_height_minus_1: 1023,
            ..AnnexASeq::base()
        };
        let mut data = td_and_annex_a_seq(seq);
        // A reserved seq_level_idx has no defined tile scaling, so the frame's tile_info()
        // parse stops as Unimplemented and the frame-core checks are skipped — the
        // level-limit checks never run regardless of the (here unreached) tile bits.
        data.extend(annex_a_frame_obu(0, 640, 640, 10, 1, 1)); // would exceed level 2.0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/frame-size-exceeds-level"),
            "a reserved level has no level limits; report was: {report}"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "annex-a/level-reserved"),
            "the reserved-level value-space error still fires; report was: {report}"
        );
    }

    // --- ops_level_idx reserved (Annex A.4 Table A.7) ---

    #[test]
    fn annex_a_flags_reserved_ops_level_idx() {
        // A global OPS carrying ops_level_idx 25 (reserved 22-30) for one extended layer.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_level(25));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/level-reserved"
                    && d.message.contains("ops_level_idx")
                    && d.spec_section.as_deref() == Some("A.4")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_valid_ops_level_idx() {
        // ops_level_idx 4 (level 4.0) is a defined level — no level-reserved error.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_level(4));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/level-reserved"),
            "ops_level_idx 4 is a defined level; report was: {report}"
        );
    }

    #[test]
    fn annex_a_flags_high_tier_below_4_0_in_ops() {
        // The reachable high-tier-below-4.0 arm (mirror lines 443-451 + the Table A.9
        // NOTE): the OPS PTL signals ops_tier_flag unconditionally (§ 5.11.2), so a High
        // tier (ops_tier_flag == 1) with ops_level_idx 3 (level 3.1, below 4.0) is a real
        // case the seq-header arm cannot reach. Exactly one advisory warning fires.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_level_tier(3, true));
        let report = Validator::new(false).validate_bytes(&data);
        let high_tier: Vec<_> = report
            .warnings()
            .filter(|d| d.rule_id == "annex-a/high-tier-below-4-0")
            .collect();
        assert_eq!(
            high_tier.len(),
            1,
            "exactly one high-tier-below-4.0 warning; report was: {report}"
        );
        let warning = high_tier[0];
        assert_eq!(warning.spec_section.as_deref(), Some("A.4"));
        assert!(
            warning.message.contains("ops_tier_flag")
                && warning.message.contains("ops_level_idx 3"),
            "message names ops_tier_flag/ops_level_idx; report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_high_tier_at_4_0_in_ops() {
        // ops_tier_flag == 1 at ops_level_idx 4 (level 4.0) is allowed (Table A.9 NOTE:
        // 4.0 and above) — no high-tier-below-4.0 diagnostic.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_level_tier(4, true));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
            "High tier at level 4.0 is allowed; report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_main_tier_below_4_0_in_ops() {
        // ops_tier_flag == 0 (Main) at ops_level_idx 3 (below 4.0) is fine — the NOTE
        // only restricts the High tier — so no high-tier-below-4.0 diagnostic.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_level_tier(3, false));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .warnings()
                .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
            "Main tier below 4.0 is fine; report was: {report}"
        );
    }

    /// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
    /// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
    /// `ops_level_idx == level_idx` with `ops_tier_flag == 0` (Main).
    fn ops_obu_with_level(level_idx: u32) -> Vec<u8> {
        ops_obu_with_level_tier(level_idx, false)
    }

    /// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
    /// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
    /// `ops_level_idx == level_idx` and `ops_tier_flag == high_tier`, with
    /// `ops_seq_profile_idc == 0`.
    fn ops_obu_with_level_tier(level_idx: u32, high_tier: bool) -> Vec<u8> {
        ops_obu_with_profile_level_tier(0, level_idx, high_tier)
    }

    /// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
    /// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
    /// `ops_seq_profile_idc == profile_idc`, `ops_level_idx == level_idx`, and
    /// `ops_tier_flag == high_tier`. Modeled on `local_ops_obu` (OBU type 18); the
    /// per-op `ops_data_size` is the byte-aligned body length. Unlike the sequence
    /// header, the OPS PTL carries `ops_tier_flag` unconditionally, so High tier can be
    /// signaled at any level here.
    fn ops_obu_with_profile_level_tier(
        profile_idc: u32,
        level_idx: u32,
        high_tier: bool,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt == 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(1); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_reserved_2bits (local OPS)
        // operating_point_payload(0):
        let mut body = Bits::default();
        // ops_seq_profile_tier_level_info() (§ 5.11.2).
        body.f(profile_idc, 5); // ops_seq_profile_idc
        body.f(level_idx, 5); // ops_level_idx
        body.bit(u8::from(high_tier)); // ops_tier_flag
        body.f(0, 3); // ops_mlayer_count
        body.f(0, 2); // ops_ptl_reserved_2bits
        body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
        body.bit(0); // ops_initial_display_delay_present_flag
        body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
        body.align();
        let body_bytes = (body.bits.len() / 8) as u32;
        bits.f(body_bytes, 8); // ops_data_size (leb128, single byte for len < 128)
        bits.bits.extend_from_slice(&body.bits);
        annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 0), &finish_extensible(bits))
    }

    #[test]
    fn annex_a_flags_reserved_ops_seq_profile_idc() {
        // A local OPS carrying ops_seq_profile_idc 7 (reserved 5-30) for one extended
        // layer must be flagged as a reserved profile (§ 6.10.4 maps the OPS-derived
        // profile id onto Annex A.2 Table A.1).
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_profile_level_tier(7, 4, false));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/profile-reserved"
                    && d.message.contains("ops_seq_profile_idc")
                    && d.spec_section.as_deref() == Some("A.2")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn annex_a_accepts_valid_ops_seq_profile_idc() {
        // ops_seq_profile_idc 0 is a defined profile — no profile-reserved error.
        let mut data = temporal_delimiter_obu();
        data.extend(ops_obu_with_profile_level_tier(0, 4, false));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/profile-reserved"),
            "ops_seq_profile_idc 0 is a defined profile; report was: {report}"
        );
    }

    // --- § 6.6 MSDO sub-stream constraints / § 7.3.8.2 identity (AV2-5.6-MSDO) ----

    /// One sub-stream entry for [`msdo_obu_configured`]: `(sub_xlayer_id,
    /// sub_stream_max_profile, sub_stream_max_level, sub_stream_max_tier)`.
    type SubStreamEntry = (u32, u32, u32, u32);

    /// A global OBU_MSDO with the given `multistream_profile_idc`,
    /// `multistream_doh_constraint_flag`, and per-substream entries (`num_streams_minus_2
    /// = entries.len() - 2`). `multistream_level_idx` / `multistream_tier` are 0 and
    /// allocation is even.
    fn msdo_obu_configured(
        multistream_profile_idc: u32,
        doh_constraint_flag: bool,
        entries: &[SubStreamEntry],
    ) -> Vec<u8> {
        assert!(entries.len() >= 2, "an MSDO has at least 2 sub-streams");
        let num_streams_minus_2 = (entries.len() - 2) as u32;
        let mut bits = Bits::default();
        bits.f(num_streams_minus_2, 3); // num_streams_minus_2
        bits.f(multistream_profile_idc, 5); // multistream_profile_idc
        bits.f(0, 5); // multistream_level_idx
        bits.bit(0); // multistream_tier
        bits.bit(1); // multistream_even_allocation_flag
        for &(sub_xlayer_id, max_profile, max_level, max_tier) in entries {
            bits.f(sub_xlayer_id, 5); // sub_xlayer_id
            bits.f(max_profile, 5); // sub_stream_max_profile
            bits.f(max_level, 5); // sub_stream_max_level
            bits.f(max_tier, 1); // sub_stream_max_tier
        }
        bits.bit(u8::from(doh_constraint_flag)); // multistream_doh_constraint_flag
        bits.bit(1); // trailing_one_bit (valid trailing_bits)
        annex_b_obu(0x50, &bits.into_bytes())
    }

    /// A sequence-header payload with explicit `seq_profile_idc`, `seq_level_idx`,
    /// `seq_tier`, and `monotonic_output_order_flag`, `max_*layer_id == 0`. `seq_tier` is
    /// only signaled when `seq_level_idx > 3` (§ 5.4.1); the caller must pick a level
    /// above 3 to exercise a High tier. The payload is a complete, activatable header.
    fn seq_header_payload_ptl(
        seq_header_id: u32,
        profile_idc: u32,
        level_idx: u32,
        tier_high: bool,
        monotonic: bool,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(profile_idc, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(level_idx, 5); // seq_level_idx
        if level_idx > 3 {
            bits.bit(u8::from(tier_high)); // seq_tier (signaled only for level > 3)
        }
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id = 0
        bits.f(0, 3); // max_mlayer_id = 0
        bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A sequence-header OBU on `xlayer` carrying [`seq_header_payload_ptl`].
    fn seq_header_obu_ptl(
        xlayer: u8,
        seq_header_id: u32,
        profile_idc: u32,
        level_idx: u32,
        tier_high: bool,
        monotonic: bool,
    ) -> Vec<u8> {
        let payload =
            seq_header_payload_ptl(seq_header_id, profile_idc, level_idx, tier_high, monotonic);
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    // -- Task 2.1: msdo/profile-below-substream-max (locally decidable) -----------

    #[test]
    fn msdo_profile_below_substream_max_is_flagged() {
        // § 6.6: multistream_profile_idc (1) < sub_stream_max_profile[1] (3) — flagged.
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(1, true, &[(0, 0, 0, 0), (1, 3, 0, 0)]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/profile-below-substream-max"
                    && d.spec_section.as_deref() == Some("6.6")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn msdo_profile_equal_to_substream_max_is_conforming() {
        // § 6.6 boundary: multistream_profile_idc (3) == sub_stream_max_profile (3) — ok.
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(3, true, &[(0, 3, 0, 0), (1, 2, 0, 0)]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/profile-below-substream-max"),
            "equality must pass; report was: {report}"
        );
    }

    // -- Task 2.2: annex-a/profile-reserved for multistream_profile_idc ----------

    #[test]
    fn msdo_reserved_multistream_profile_is_flagged() {
        // § 6.6: multistream_profile_idc 7 is reserved (5..=30) — annex-a/profile-reserved.
        let mut data = temporal_delimiter_obu();
        // sub_stream_max_profile must be <= multistream_profile_idc to isolate the
        // reserved-profile finding from the floor check.
        data.extend(msdo_obu_configured(7, true, &[(0, 7, 0, 0), (1, 7, 0, 0)]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/profile-reserved"
                    && d.message.contains("multistream_profile_idc")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn msdo_valid_multistream_profile_is_not_reserved() {
        // multistream_profile_idc 4 is a defined profile — no profile-reserved error.
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(4, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/profile-reserved"),
            "multistream_profile_idc 4 is defined; report was: {report}"
        );
    }

    // -- Task 3.2: sub-stream PTL-ceiling agreement, both arrival orders ---------

    /// Builds a single-temporal-unit two-layer multistream stream that opens a CMVS
    /// (MSDO + CLK), with sequence headers carrying the given PTL, then frame-confirms
    /// each extended layer via a CLK frame referencing its header. `msdo_first` controls
    /// the arrival order: when true the MSDO precedes the headers/activations
    /// (MSDO-then-activation); when false it follows the activations
    /// (activation-then-MSDO). The MSDO declares sub_xlayer_id 0 and 1 with the given
    /// ceilings and a satisfied DOH flag.
    fn substream_ptl_stream(
        msdo_first: bool,
        seq0: (u32, u32, bool),
        seq1: (u32, u32, bool),
        ceil0: (u32, u32, u32),
        ceil1: (u32, u32, u32),
    ) -> Vec<u8> {
        let msdo = msdo_obu_configured(
            31, // Configurable profile: a high floor so the profile-below check never fires
            true,
            &[
                (0, ceil0.0, ceil0.1, ceil0.2),
                (1, ceil1.0, ceil1.1, ceil1.2),
            ],
        );
        let headers_and_frames = {
            let mut d = Vec::new();
            d.extend(seq_header_obu_ptl(0, 0, seq0.0, seq0.1, seq0.2, true));
            d.extend(seq_header_obu_ptl(1, 1, seq1.0, seq1.1, seq1.2, true));
            // CLK frame headers confirm activation and (the first) opens the CMVS.
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
            d
        };
        let mut data = temporal_delimiter_obu();
        if msdo_first {
            data.extend(msdo);
            data.extend(headers_and_frames);
        } else {
            data.extend(headers_and_frames);
            data.extend(msdo);
        }
        data
    }

    #[test]
    fn substream_level_exceeds_max_is_flagged_msdo_first() {
        // Spec scenario: MSDO sub_stream_max_level[1] = 4 for sub_xlayer_id 1; a
        // frame-confirmed header with seq_level_idx = 8 activates on extended layer 1.
        // MSDO arrives before the activations.
        let data = substream_ptl_stream(
            true,
            (0, 4, false), // xlayer 0 header: level 4
            (0, 8, false), // xlayer 1 header: level 8 -> exceeds ceiling 4
            (0, 21, 0),    // ceiling for xlayer 0
            (0, 4, 0),     // ceiling for xlayer 1: max_level 4
        );
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/substream-level-exceeds-max"
                    && d.spec_section.as_deref() == Some("6.6")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn substream_level_exceeds_max_is_flagged_activation_first() {
        // Same violation, MSDO arriving AFTER both activations (activation-then-MSDO).
        let data = substream_ptl_stream(false, (0, 4, false), (0, 8, false), (0, 21, 0), (0, 4, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/substream-level-exceeds-max"),
            "the violation must fire when the MSDO follows the activation; report was: {report}"
        );
    }

    #[test]
    fn substream_level_equal_to_max_is_conforming() {
        // § 6.6 boundary: seq_level_idx (4) == sub_stream_max_level (4) — no diagnostic.
        let data = substream_ptl_stream(true, (0, 4, false), (0, 4, false), (0, 4, 0), (0, 4, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("msdo/substream-")),
            "equality must pass; report was: {report}"
        );
    }

    #[test]
    fn substream_profile_exceeds_max_is_flagged() {
        // § 6.6: seq_profile_idc (4) on xlayer 1 exceeds sub_stream_max_profile (2).
        let data = substream_ptl_stream(true, (0, 0, false), (4, 0, false), (4, 21, 0), (2, 21, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/substream-profile-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn substream_tier_exceeds_max_is_flagged() {
        // § 6.6: seq_tier High (1) on xlayer 1 (level 8 > 3 so tier is signaled) exceeds
        // sub_stream_max_tier (0).
        let data = substream_ptl_stream(
            true,
            (0, 8, false),
            (0, 8, true), // xlayer 1: High tier
            (0, 21, 1),
            (0, 21, 0), // ceiling tier 0
        );
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/substream-tier-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn substream_max_not_flagged_for_unconfirmed_activation() {
        // Frame-confirmed gating: an OBU-order fallback header that no frame references
        // must NOT be checked against the MSDO ceiling (§ 7.3.6 staged-but-unactivated).
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(31, true, &[(0, 4, 4, 0), (1, 4, 4, 0)]));
        // Two staged headers on xlayer 1, neither frame-confirmed; the second has a level
        // above the ceiling. With two in-band candidates and no frame, neither is the
        // decidable sole-candidate activation.
        data.extend(seq_header_obu_ptl(1, 0, 0, 4, false, true));
        data.extend(seq_header_obu_ptl(1, 1, 0, 8, false, true));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("msdo/substream-")),
            "an unconfirmed staged header must not be checked; report was: {report}"
        );
    }

    #[test]
    fn substream_max_suppressed_under_external_hls() {
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        // The substream-max agreement is suppressed when external HLS declares a
        // sequence header (the activated header may be out-of-band with unmodeled PTL).
        let data = substream_ptl_stream(true, (0, 4, false), (0, 8, false), (0, 21, 0), (0, 4, 0));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new()
                    .with_sequence_header_id(0)
                    .with_sequence_header_id(1),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("msdo/substream-")),
            "external HLS suppresses the substream-max agreement; report was: {report}"
        );
    }

    // -- Task 4: msdo/doh-constraint-required, both arrival orders ---------------

    /// A single-temporal-unit two-layer multistream stream opening a CMVS, with the
    /// extended-layer-0 header carrying `monotonic_x0` and extended-layer-1 header
    /// carrying `monotonic_x1`, an MSDO with the given `doh_constraint_flag`, and
    /// frame-confirmed activations. `msdo_first` selects the arrival order.
    fn doh_stream(
        msdo_first: bool,
        doh_flag: bool,
        monotonic_x0: bool,
        monotonic_x1: bool,
    ) -> Vec<u8> {
        let msdo = msdo_obu_configured(31, doh_flag, &[(0, 21, 21, 0), (1, 21, 21, 0)]);
        let headers_and_frames = {
            let mut d = Vec::new();
            d.extend(seq_header_obu_ptl(0, 0, 0, 0, false, monotonic_x0));
            d.extend(seq_header_obu_ptl(1, 1, 0, 0, false, monotonic_x1));
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
            d
        };
        let mut data = temporal_delimiter_obu();
        if msdo_first {
            data.extend(msdo);
            data.extend(headers_and_frames);
        } else {
            data.extend(headers_and_frames);
            data.extend(msdo);
        }
        data
    }

    #[test]
    fn doh_constraint_required_is_flagged_msdo_first() {
        // § 6.6: a CMVS-inside activated header with monotonic_output_order_flag == 0
        // while multistream_doh_constraint_flag == 0 — flagged. MSDO arrives first.
        let data = doh_stream(true, false, true, false);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/doh-constraint-required"
                    && d.spec_section.as_deref() == Some("6.6")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn doh_constraint_required_is_flagged_activation_first() {
        // Same violation, MSDO arriving after the activations (activation-then-MSDO).
        let data = doh_stream(false, false, true, false);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "the DOH requirement must fire when the MSDO follows the activation; report was: {report}"
        );
    }

    #[test]
    fn doh_constraint_satisfied_by_flag_is_conforming() {
        // multistream_doh_constraint_flag == 1 satisfies the requirement even with a
        // non-monotonic activated header.
        let data = doh_stream(true, true, true, false);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "doh_constraint_flag == 1 satisfies the requirement; report was: {report}"
        );
    }

    #[test]
    fn doh_constraint_not_flagged_when_all_monotonic() {
        // Every activated header is monotonic (flag == 1), so the requirement is vacuous
        // even with multistream_doh_constraint_flag == 0.
        let data = doh_stream(true, false, true, true);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "all-monotonic headers do not trigger the DOH requirement; report was: {report}"
        );
    }

    #[test]
    fn doh_constraint_not_flagged_outside_cmvs() {
        // With no MSDO opening a CMVS the tracker stays Outside, so a non-monotonic
        // header with no DOH context does not fire. (Here there is no MSDO at all.)
        let mut data = temporal_delimiter_obu();
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, false)); // non-monotonic
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "no CMVS means no DOH requirement; report was: {report}"
        );
    }

    // -- Codex PR #47 follow-ups: deferred DOH evaluation + duplicate ceilings ---

    #[test]
    fn doh_constraint_not_flagged_when_clk_ends_cmvs_for_the_activating_tu() {
        // Codex finding 3392940061. The DOH check must defer until the temporal unit's
        // CMVS membership is final. Scenario: TU1 opens a CMVS (MSDO with
        // multistream_doh_constraint_flag == 0, a monotonic-1 header, a CLK), so it is
        // Inside; TU2 redefines the active header to monotonic_output_order_flag == 0 and
        // activates it via a non-CLK frame BEFORE a later MSDO-less CLK ends the CMVS
        // (§ 7.3.2 end condition 2, mirror `07-decoding-process.md` lines 335-341). The
        // monotonic-0 header therefore sits OUTSIDE the CMVS, so § 6.6 does not apply to
        // it and no `msdo/doh-constraint-required` may fire. The pre-fix eager check,
        // gated on the still-`Inside` committed state at activation time (the ending CLK
        // is observed only later in TU2), fired a false positive.
        let mut data = temporal_delimiter_obu(); // temporal unit 1
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 21, 21, 0), (1, 21, 21, 0)],
        ));
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // seq 0 monotonic 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
        data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
        // Redefinition: a new header (seq 1) for xlayer 0 with monotonic 0, activated by a
        // non-CLK frame so on_sequence_activation re-runs (the eager path the finding
        // describes). The ending CLK has not yet been observed when this activates.
        data.extend(seq_header_obu_ptl(0, 1, 0, 0, false, false)); // seq 1 monotonic 0
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // non-CLK frame activates seq 1
        // A later MSDO-less CLK ends the CMVS for temporal unit 2 (end condition 2), so
        // the monotonic-0 header above is outside the CMVS.
        data.extend(annex_b_obu(0x10, &[])); // bare CLK on xlayer 0, no MSDO
        data.extend(temporal_delimiter_obu()); // close temporal unit 2 via a boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "a monotonic-0 header in a temporal unit whose MSDO-less CLK ends the CMVS is \
             outside the CMVS; § 6.6 must not fire; report was: {report}"
        );
    }

    #[test]
    fn doh_constraint_flagged_when_same_id_clk_opens_cmvs_after_the_activation() {
        // Codex finding 3392940072. A header frame-confirmed BEFORE any CMVS, then a
        // temporal unit with an MSDO (multistream_doh_constraint_flag == 0) followed by a
        // same-id CLK that opens the CMVS. The same-id CLK re-references the already-active
        // header, so on_sequence_activation is skipped (the seq id is unchanged and the
        // layer was already frame-confirmed) — the pre-fix eager check, which only ran on
        // a (re)activation, never saw the CMVS transition to Inside and missed the
        // violation. The header has monotonic_output_order_flag == 0, so § 6.6 requires
        // multistream_doh_constraint_flag == 1; the deferred evaluation at temporal-unit
        // completion re-examines all frame-confirmed activations and fires.
        let mut data = temporal_delimiter_obu(); // temporal unit 1 (no CMVS yet)
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, false)); // seq 0 monotonic 0
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // non-CLK frame confirms seq 0
        data.extend(temporal_delimiter_obu()); // temporal unit 2
        // The MSDO (doh flag 0) precedes the coded extended layer unit (§ 7.3.7), and the
        // same-id CLK frame re-references seq 0 and opens the CMVS at that CLK.
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 21, 21, 0), (1, 21, 21, 0)],
        ));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/doh-constraint-required"
                    && d.spec_section.as_deref() == Some("6.6")
            }),
            "a same-id CLK that opens a CMVS over a monotonic-0 frame-confirmed header must \
             fire § 6.6 at temporal-unit resolution; report was: {report}"
        );
    }

    #[test]
    fn substream_max_duplicate_sub_xlayer_id_keeps_the_most_restrictive_ceiling() {
        // Codex finding 3392940071. § 6.6 imposes the sub_stream_max_* ceiling "for each
        // sequence header activated by the i-th independent sub-stream" — for EACH i. With
        // a duplicate sub_xlayer_id (the spec declares no uniqueness requirement), an
        // activated header must satisfy BOTH declared ceilings, so the effective per-layer
        // ceiling is the per-dimension minimum. Here sub_xlayer_id 1 is declared twice with
        // sub_stream_max_level 8 and 4; a header at level 6 on extended layer 1 exceeds the
        // tighter ceiling 4 and must be flagged. A pre-fix last-wins insert would keep
        // whichever entry came last and miss the violation when the 8-ceiling won.
        for (first, second) in [
            ((1, 21, 8, 0), (1, 21, 4, 0)),
            ((1, 21, 4, 0), (1, 21, 8, 0)),
        ] {
            let mut data = temporal_delimiter_obu();
            data.extend(msdo_obu_configured(
                31,
                true,
                &[(0, 21, 21, 0), first, second],
            ));
            // Interleave each layer's header with its CLK frame in ascending xlayer order
            // (§ 7.3.7 coded-extended-layer-unit ordering): xlayer 0 header + frame, then
            // xlayer 1 header + frame.
            data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // xlayer 0 header
            data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm xlayer 0
            data.extend(seq_header_obu_ptl(1, 1, 0, 6, false, true)); // xlayer 1: level 6
            data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // confirm xlayer 1
            let report = Validator::new(false).validate_bytes(&data);
            assert!(
                report.errors().any(|d| {
                    d.rule_id == "msdo/substream-level-exceeds-max"
                        && d.spec_section.as_deref() == Some("6.6")
                }),
                "a duplicate sub_xlayer_id must enforce the most restrictive (level 4) \
                 ceiling regardless of declaration order ({first:?} then {second:?}); \
                 report was: {report}"
            );
        }
    }

    // -- Task 5: § 7.3.8.2 non-RAP MSDO identity --------------------------------

    /// A temporal-unit-delimited stream: each entry is `(make_rap, msdo)` where
    /// `make_rap` adds a CLK (§ 7.4.1 random access point) to that temporal unit and
    /// `msdo` is the MSDO payload bytes for that temporal unit (already a full OBU).
    fn msdo_identity_stream(units: &[(bool, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (make_rap, msdo) in units {
            data.extend(temporal_delimiter_obu());
            data.extend(msdo.clone());
            if *make_rap {
                data.extend(annex_b_obu(0x10, &[])); // CLK on xlayer 0
            }
        }
        data
    }

    #[test]
    fn non_rap_changed_msdo_is_flagged() {
        // § 7.3.8.2: a non-RAP temporal unit carrying a changed OBU_MSDO — flagged.
        // A trailing temporal delimiter ends the offending TU, so the finding is emitted
        // from the TD-driven `complete_temporal_unit` path (distinct from the
        // end-of-stream flush exercised by the sibling test below).
        let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let mut data = msdo_identity_stream(&[
            (true, msdo_a),  // RAP TU establishes the reference
            (false, msdo_b), // non-RAP TU with a changed MSDO -> flagged
        ]);
        data.extend(temporal_delimiter_obu()); // end the offending TU via a TD boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/non-rap-not-identical"
                    && d.spec_section.as_deref() == Some("7.3.8.2")
            }),
            "report was: {report}"
        );
    }

    #[test]
    fn non_rap_identical_msdo_is_conforming() {
        // § 7.3.8.2: a non-RAP temporal unit carrying an identical OBU_MSDO — no error.
        let msdo = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = msdo_identity_stream(&[(true, msdo.clone()), (false, msdo)]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
            "an identical MSDO must pass; report was: {report}"
        );
    }

    #[test]
    fn rap_changed_msdo_is_conforming() {
        // § 7.3.8.2: a RAP temporal unit (contains a CLK) carrying a changed OBU_MSDO is
        // exempt — no identity error, and the reference updates.
        let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = msdo_identity_stream(&[(true, msdo_a), (true, msdo_b)]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
            "a changed MSDO at a random access point is exempt; report was: {report}"
        );
    }

    #[test]
    fn non_rap_changed_msdo_at_end_of_stream_is_flagged() {
        // The final temporal unit has no trailing temporal delimiter; the end-of-stream
        // flush still resolves its buffered MSDO against the previous one.
        let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        // First TU is a RAP establishing the reference; second TU is the final,
        // non-RAP TU with a changed MSDO and no trailing delimiter.
        let data = msdo_identity_stream(&[(true, msdo_a), (false, msdo_b)]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
            "the end-of-stream flush must resolve the final TU's MSDO; report was: {report}"
        );
    }

    // == msdo-global-lcr-agreement: § 6.8.2 / § 7.3.2 / Annex A Table A.4 =============

    /// `lcr_aggregate_info()` fields for the configurable global-LCR builder.
    #[derive(Clone, Copy)]
    struct AggInfo {
        config_idc: u32,
        aggregate_level_idx: u32,
        max_tier_flag: u8,
        max_interop: u32,
    }

    /// One `lcr_seq_profile_tier_level_info(i)` entry for the global-LCR builder, in the
    /// xlayer-ascending order the parser reads them.
    #[derive(Clone, Copy)]
    struct GlobalPtl {
        seq_profile_idc: u32,
        max_level_idx: u32,
        tier_flag: u8,
        max_mlayer_count: u32,
    }

    /// A global LCR OBU with the § 6.8.2 agreement fields configurable: the xlayer map, an
    /// optional `lcr_aggregate_info()`, an optional per-xlayer `lcr_seq_profile_tier_level_info`
    /// list (ascending xlayer order, one per set bit of `xlayer_map`), and the
    /// `lcr_doh_constraint_flag`. No global payload.
    fn global_lcr_obu_agreement(
        global_id: u32,
        xlayer_map: u32,
        agg: Option<AggInfo>,
        ptls: Option<&[GlobalPtl]>,
        doh_constraint_flag: bool,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.f(global_id, 3); // lcr_global_config_record_id
        bits.f(xlayer_map, 31); // lcr_xlayer_map
        bits.bit(u8::from(agg.is_some())); // lcr_aggregate_info_present_flag
        bits.bit(u8::from(ptls.is_some())); // lcr_seq_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_global_payload_present_flag
        bits.bit(0); // lcr_dependent_xlayers_flag
        bits.bit(0); // lcr_global_atlas_id_present_flag
        bits.f(0, 7); // lcr_global_purpose_id
        bits.bit(u8::from(doh_constraint_flag)); // lcr_doh_constraint_flag
        bits.bit(0); // lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // lcr_global_reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        if let Some(agg) = agg {
            bits.f(agg.config_idc, 6); // lcr_config_idc
            bits.f(agg.aggregate_level_idx, 5); // lcr_aggregate_level_idx
            bits.bit(agg.max_tier_flag); // lcr_max_tier_flag
            bits.f(agg.max_interop, 4); // lcr_max_interop
        }
        if let Some(ptls) = ptls {
            for ptl in ptls {
                bits.f(ptl.seq_profile_idc, 5); // lcr_seq_profile_idc[i]
                bits.f(ptl.max_level_idx, 5); // lcr_max_level_idx[i]
                bits.bit(ptl.tier_flag); // lcr_tier_flag[i]
                bits.f(ptl.max_mlayer_count, 3); // lcr_max_mlayer_count[i]
                bits.f(0, 2); // lsptli_reserved_2bits
            }
        }
        extensible_obu_tail(&mut bits);
        annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
    }

    /// A sequence-header payload that references `seq_lcr_id` (so an activated frame for
    /// this layer associates the header with that LCR), with explicit PTL and
    /// `monotonic_output_order_flag`, `max_*layer_id == 0`.
    fn seq_header_payload_lcr_ref(
        seq_header_id: u32,
        profile_idc: u32,
        level_idx: u32,
        tier_high: bool,
        monotonic: bool,
        seq_lcr_id: u32,
        max_mlayer_id: u32,
    ) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(seq_header_id);
        bits.f(profile_idc, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(level_idx, 5); // seq_level_idx
        if level_idx > 3 {
            bits.bit(u8::from(tier_high)); // seq_tier (signaled only for level > 3)
        }
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(seq_lcr_id, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(0, 2); // max_tlayer_id = 0
        bits.f(max_mlayer_id, 3); // max_mlayer_id
        if max_mlayer_id > 0 {
            // SeqMaxMlayerCnt = max_mlayer_id + 1 allows embedded layers 0..=max_mlayer_id.
            bits.f(max_mlayer_id, ceil_log2_u32(max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
        }
        bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        if max_mlayer_id > 0 {
            bits.bit(0); // mlayer_dependency_present_flag
        }
        append_non_single_child_configs(&mut bits);
        bits.into_bytes()
    }

    /// A sequence-header OBU on `xlayer` carrying [`seq_header_payload_lcr_ref`].
    fn seq_header_obu_lcr_ref(
        xlayer: u8,
        seq_header_id: u32,
        profile_idc: u32,
        monotonic: bool,
        seq_lcr_id: u32,
    ) -> Vec<u8> {
        let payload = seq_header_payload_lcr_ref(
            seq_header_id,
            profile_idc,
            0,
            false,
            monotonic,
            seq_lcr_id,
            0,
        );
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    /// A two-extended-layer CMVS stream opening a CMVS in a single temporal unit (begin
    /// condition 1: a CLK temporal unit with an MSDO present), with a global LCR present and
    /// activated by both layers' headers via `seq_lcr_id == global_id`. `msdo_first` selects
    /// the arrival order of the MSDO relative to the headers/global-LCR. Both layers are
    /// frame-confirmed by CLK frames in the opening temporal unit.
    #[allow(clippy::too_many_arguments)]
    fn lcr_msdo_stream(
        msdo_first: bool,
        global_id: u32,
        global_xlayer_map: u32,
        agg: Option<AggInfo>,
        ptls: Option<&[GlobalPtl]>,
        global_doh: bool,
        msdo: Vec<u8>,
    ) -> Vec<u8> {
        let global = global_lcr_obu_agreement(global_id, global_xlayer_map, agg, ptls, global_doh);
        let headers_and_frames = {
            // Global HLS (the global LCR) first, then per-layer coded extended layer units in
            // ascending obu_xlayer_id order (§ 7.3.7): seq0 + CLK0, then seq1 + CLK1.
            let mut d = Vec::new();
            d.extend(global.clone());
            d.extend(seq_header_obu_lcr_ref(0, 0, 0, true, global_id));
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
            d.extend(seq_header_obu_lcr_ref(1, 1, 0, true, global_id));
            d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
            d
        };
        let mut data = temporal_delimiter_obu();
        if msdo_first {
            data.extend(msdo);
            data.extend(headers_and_frames);
        } else {
            data.extend(headers_and_frames);
            data.extend(msdo);
        }
        data
    }

    #[test]
    fn lcr_msdo_stream_count_mismatch_is_flagged_both_orders() {
        // § 6.8.2 constraint 1: num_streams_minus_2 + 2 (2) != LcrMaxNumXLayerCount (3,
        // from a 3-bit xlayer_map 0b111). Flagged in both arrival orders.
        for msdo_first in [true, false] {
            let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
            let data = lcr_msdo_stream(msdo_first, 1, 0b111, None, None, false, msdo);
            let report = Validator::new(false).validate_bytes(&data);
            assert!(
                report.errors().any(|d| {
                    d.rule_id == "lcr/msdo-stream-count-mismatch"
                        && d.spec_section.as_deref() == Some("6.8.2")
                }),
                "stream-count mismatch must fire (msdo_first={msdo_first}); report was: {report}"
            );
        }
    }

    #[test]
    fn lcr_msdo_stream_count_match_is_conforming() {
        // § 6.8.2 constraint 1 boundary: num_streams (2) == LcrMaxNumXLayerCount (2, map
        // 0b11). No stream-count mismatch.
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-stream-count-mismatch"),
            "matching stream count must not fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_sub_xlayer_not_in_lcr_is_flagged() {
        // § 6.8.2 constraint 2: an MSDO sub_xlayer_id (2) not in LcrXLayerID[] (the map
        // 0b11 sets bits 0 and 1 only). LcrMaxNumXLayerCount is 2 == num_streams, so only
        // the membership constraint fires.
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (2, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "a sub_xlayer_id outside LcrXLayerID[] must fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_sub_xlayer_in_lcr_is_conforming() {
        // § 6.8.2 constraint 2 boundary: every sub_xlayer_id (0, 1) is in LcrXLayerID[]
        // (map 0b11). No membership mismatch.
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"),
            "in-set sub_xlayer_ids must not fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_aggregate_level_and_tier_mismatch_is_flagged() {
        // § 6.8.2 constraint 3: multistream_level_idx (the msdo builder hardcodes 0) !=
        // lcr_aggregate_level_idx (5), and multistream_tier (0) != lcr_max_tier_flag (1).
        // multistream_profile_idc 0 -> config 0 allows it and IOP 0 == max_interop 0, so
        // only the level and tier arms fire.
        let agg = AggInfo {
            config_idc: 0,
            aggregate_level_idx: 5,
            max_tier_flag: 1,
            max_interop: 0,
        };
        let msdo = msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-aggregate-mismatch"
                    && d.spec_section.as_deref() == Some("6.8.2")
                    && d.message.contains("multistream_level_idx")
            }),
            "an aggregate level mismatch must fire; report was: {report}"
        );
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-aggregate-mismatch" && d.message.contains("multistream_tier")
            }),
            "an aggregate tier mismatch must fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_aggregate_interop_and_config_mismatch_is_flagged() {
        // § 6.8.2 constraint 3: multistream_profile_idc 4 (IOP 1, and config 0 C_Main_420_10
        // does NOT allow profile 4 per Table A.6) vs lcr_config_idc 0 and lcr_max_interop 0.
        // So both the Table A.6 config consistency and the Table A.1 interop equality fire.
        let agg = AggInfo {
            config_idc: 0,
            aggregate_level_idx: 0,
            max_tier_flag: 0,
            max_interop: 0,
        };
        // multistream_profile_idc 4 needs a level > 3 in the headers to be conformant for
        // High tier, but profile alone is fine; msdo level is 0 here (matches agg level 0).
        let msdo = msdo_obu_configured(4, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-aggregate-mismatch" && d.message.contains("lcr_config_idc")
            }),
            "a Table A.6 config-idc inconsistency must fire; report was: {report}"
        );
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-aggregate-mismatch"
                    && d.message.contains("interoperability point")
            }),
            "a Table A.1 interop inequality must fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_aggregate_agreement_is_conforming() {
        // § 6.8.2 constraint 3 boundary: every aggregate field agrees. multistream_profile_idc
        // 0 (IOP 0), level 0, tier 0; config 0 allows profile 0, max_interop 0, level 0, tier 0.
        let agg = AggInfo {
            config_idc: 0,
            aggregate_level_idx: 0,
            max_tier_flag: 0,
            max_interop: 0,
        };
        let msdo = msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-aggregate-mismatch"),
            "fully-agreeing aggregate info must not fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_substream_ptl_mismatch_is_flagged_both_orders() {
        // § 6.8.2 constraint 4: sub_stream_max_level[1] (4) != lcr_max_level_idx for
        // sub_xlayer_id 1 (7). Exact-equality semantics. Both arrival orders.
        let ptls = [
            GlobalPtl {
                seq_profile_idc: 0,
                max_level_idx: 0,
                tier_flag: 0,
                max_mlayer_count: 0,
            },
            GlobalPtl {
                seq_profile_idc: 0,
                max_level_idx: 7,
                tier_flag: 0,
                max_mlayer_count: 0,
            },
        ];
        for msdo_first in [true, false] {
            let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 4, 0)]);
            let data = lcr_msdo_stream(msdo_first, 1, 0b11, None, Some(&ptls), false, msdo);
            let report = Validator::new(false).validate_bytes(&data);
            assert!(
                report.errors().any(|d| {
                    d.rule_id == "lcr/msdo-substream-ptl-mismatch"
                        && d.spec_section.as_deref() == Some("6.8.2")
                }),
                "a per-substream PTL mismatch must fire (msdo_first={msdo_first}); report: {report}"
            );
        }
    }

    #[test]
    fn lcr_msdo_substream_ptl_agreement_is_conforming() {
        // § 6.8.2 constraint 4 boundary: exact equality on every dimension for each i.
        let ptls = [
            GlobalPtl {
                seq_profile_idc: 0,
                max_level_idx: 0,
                tier_flag: 0,
                max_mlayer_count: 0,
            },
            GlobalPtl {
                seq_profile_idc: 0,
                max_level_idx: 4,
                tier_flag: 0,
                max_mlayer_count: 0,
            },
        ];
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 4, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, Some(&ptls), false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-substream-ptl-mismatch"),
            "exact-matching per-substream PTL must not fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_doh_flag_mismatch_is_flagged() {
        // § 6.8.2 constraint 5: multistream_doh_constraint_flag (1) != lcr_doh_constraint_flag
        // (0). All headers monotonic so the DOH *requirement* does not also fire.
        let msdo = msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-doh-flag-mismatch"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "a DOH-flag mismatch must fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_doh_flag_agreement_is_conforming() {
        // § 6.8.2 constraint 5 boundary: both flags 1.
        let msdo = msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b11, None, None, true, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-doh-flag-mismatch"),
            "agreeing DOH flags must not fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_agreement_inert_for_unactivated_global_lcr() {
        // § 6.8.2: an observed-but-never-activated global LCR triggers no agreement
        // diagnostic. Here the headers use seq_lcr_id == 0 (no association), so the chain
        // never resolves the global LCR as activated even though a stream-count and DOH-flag
        // disagreement would otherwise fire.
        let global = global_lcr_obu_agreement(1, 0b111, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(global);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // seq_lcr_id == 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 0));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
            "an unactivated global LCR triggers no § 6.8.2 agreement diagnostic; report: {report}"
        );
    }

    #[test]
    fn lcr_doh_constraint_required_is_flagged() {
        // § 6.8.2 DOH requirement (lines 1619-1621): an activated header has
        // monotonic_output_order_flag == 0 while the activated global LCR's
        // lcr_doh_constraint_flag == 0. The MSDO's flag matches the global's (both 0) so the
        // §6.8.2 flag-mismatch does not also fire.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // monotonic 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // monotonic 0 -> requires DOH
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/doh-constraint-required"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "the LCR DOH-constraint requirement must fire; report was: {report}"
        );
    }

    #[test]
    fn lcr_doh_constraint_satisfied_by_flag_is_conforming() {
        // § 6.8.2 DOH requirement boundary: lcr_doh_constraint_flag == 1 satisfies it even
        // with a non-monotonic activated header. The MSDO flag is 1 too (so no flag mismatch).
        let global = global_lcr_obu_agreement(1, 0b11, None, None, true);
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(global);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // monotonic 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/doh-constraint-required"),
            "lcr_doh_constraint_flag == 1 satisfies the requirement; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_agreement_suppressed_under_external_hls() {
        // External HLS declaring a sequence header makes the activation chain unreliable, so
        // the § 6.8.2 agreement is suppressed even with a stream-count disagreement.
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(true, 1, 0b111, None, None, false, msdo);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
            "external HLS suppresses the § 6.8.2 agreement; report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_agreement_inert_when_global_lcr_not_present_in_this_cmvs() {
        // Codex finding 1 (3393129738): a global LCR activated in an earlier CVS must not
        // leak into a *later* CMVS's § 6.8.2 evaluation when that CMVS contains no global-LCR
        // OBU. TU1/CVS1 opens a CMVS with a conforming MSDO (num_streams 2 == LcrMaxNumXLayer
        // Count 2) and global LCR id 1 activated by both layers' headers. TU2/CVS2 opens a NEW
        // CMVS (a changed MSDO: profile differs from TU1's) whose headers still reference
        // seq_lcr_id 1 (so the association chain resolves the *still-available* global LCR),
        // but no global-LCR OBU is re-sent. The TU2 MSDO declares num_streams 3, which would
        // disagree with the leaked record's LcrMaxNumXLayerCount 2 — yet § 6.8.2 must NOT fire,
        // because the global LCR is not present in TU2's CMVS. Pre-fix `activated_global_lcr`
        // resolves the leaked record via the live `global_lcr_records` map and the
        // stream-count mismatch fires falsely.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
        let mut data = temporal_delimiter_obu(); // TU1/CVS1: opens CMVS #1
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(global);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // activates global LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // activates global LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
        data.extend(temporal_delimiter_obu()); // TU2/CVS2: a changed MSDO opens CMVS #2
        // num_streams_minus_2 + 2 == 3, profile 31 (differs from TU1's 0 → § 7.3.2 begin
        // condition 2 starts a new CMVS); three sub-streams. NO global LCR re-sent this CMVS.
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0), (2, 0, 0, 0)],
        ));
        // Headers redefined at the CVS boundary still reference seq_lcr_id 1 (the leaked
        // record is still available in-band), and are re-activated by the CLK frames.
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
        data.extend(temporal_delimiter_obu()); // close TU2
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
            "a global LCR absent from this CMVS must not be evaluated against its MSDO; \
             report was: {report}"
        );
    }

    #[test]
    fn lcr_msdo_agreement_uses_association_time_global_lcr_snapshot() {
        // Codex finding 2 (3393129741): the § 6.8.2 record is resolved from the
        // *association-time* snapshot, not a live lookup — a same-id global-LCR redefinition
        // after a header associated with the earlier revision must not retarget the agreement
        // at the later revision. Both revisions of global LCR id 1 have LcrMaxNumXLayerCount 2
        // (map 0b11) so the stream count matches; they differ only in lcr_doh_constraint_flag.
        // Revision A (doh 1) is observed before the headers, so the headers associate+activate
        // rev A. Revision B (doh 0) is re-sent after the headers (a redefinition). The MSDO's
        // multistream_doh_constraint_flag is 1, which AGREES with rev A but DISAGREES with rev
        // B. The agreement must compare against rev A (no mismatch). Pre-fix the live lookup
        // sees rev B and `lcr/msdo-doh-flag-mismatch` fires falsely.
        let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1
        let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0 (redefine)
        let mut data = temporal_delimiter_obu();
        // MSDO multistream_doh_constraint_flag == 1 (agrees with rev A).
        data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(global_a); // rev A first
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // associates rev A
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // associates rev A
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        data.extend(global_b); // rev B redefines id 1 AFTER the headers associated rev A
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/msdo-doh-flag-mismatch"),
            "the agreement must use the association-time rev A (doh 1), which agrees with the \
             MSDO; report was: {report}"
        );

        // Inverse: the MSDO agrees with rev B but disagrees with rev A. The diagnostic must
        // fire naming rev A's value (doh 1), because rev A is the associated record.
        let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1 (associated)
        let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0
        let mut data = temporal_delimiter_obu();
        // MSDO multistream_doh_constraint_flag == 0 (agrees with rev B, disagrees with rev A).
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(global_a);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        data.extend(global_b);
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-doh-flag-mismatch"
                    // rev A's lcr_doh_constraint_flag is 1; the message names the associated
                    // record's value.
                    && d.message.contains("lcr_doh_constraint_flag (1)")
            }),
            "the agreement must fire against the association-time rev A (doh 1); report was: \
             {report}"
        );
    }

    #[test]
    fn lcr_doh_constraint_required_fires_without_msdo() {
        // Codex finding 3 (3393129743): the LCR DOH requirement is LCR-only — it must fire in
        // a global-LCR-only CMVS (no OBU_MSDO) when a confirmed activated header has
        // monotonic_output_order_flag == 0 and the activated global LCR's
        // lcr_doh_constraint_flag == 0. § 7.3.2 begin condition 3 (a CLK TU activating a global
        // LCR with no MSDO) opens such a CMVS. Pre-fix the resolver early-returns on the
        // missing MSDO, so the LCR-only requirement never fires.
        let global = global_lcr_obu_agreement(1, 0b1, None, None, false); // doh 0, single xlayer
        let mut data = temporal_delimiter_obu();
        data.extend(global); // global LCR present, no MSDO
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, false, 1)); // monotonic 0, activates LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 (begin cond 3)
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/doh-constraint-required"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "the LCR DOH requirement is LCR-only and must fire without an MSDO; report was: \
             {report}"
        );
    }

    #[test]
    fn lcr_doh_constraint_required_scoped_to_current_cmvs() {
        // Codex finding 4 (3393129745): the DOH loop must consider only sequence headers
        // activated within the CURRENT CMVS, not every frame-confirmed xlayer ever seen.
        // TU1/CVS1: xlayer 1 activates a header with monotonic_output_order_flag == 0 (frame-
        // confirmed), in a standalone CVS (no MSDO, no LCR → CMVS stays Outside) that ends
        // before the CMVS of interest. TU2/CVS2: opens a definitively-Inside CMVS on xlayer 0
        // (a CLK + MSDO begins it) whose own header is monotonic == 1, with an activated global
        // LCR whose lcr_doh_constraint_flag == 0. The non-monotonic xlayer-1 header belongs to
        // the earlier, ended CVS — it is NOT activated within TU2's CMVS, so no diagnostic may
        // fire. Pre-fix the loop iterates the whole-history frame_confirmed_xlayers set and
        // flags the leaked xlayer-1 header against TU2's global LCR. (The MSDO's
        // multistream_doh_constraint_flag is 1, so no § 6.6 check fires; the global LCR's count
        // and doh-flag match the MSDO so no § 6.8.2 agreement disagreement fires either.)
        let mut data = temporal_delimiter_obu(); // TU1/CVS1: a non-monotonic xlayer-1 header
        data.extend(seq_header_obu_lcr_ref(1, 5, 0, false, 0)); // xlayer 1, monotonic 0, no LCR
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 5)); // CLK xlayer 1 → confirms seq 5
        data.extend(temporal_delimiter_obu()); // TU2/CVS2: a fresh Inside CMVS on xlayer 0
        // A 2-xlayer global LCR (doh 0) and a matching 2-substream MSDO (doh 0): the count
        // matches (2 == 2) and the doh flags match, so neither the § 6.8.2 agreement nor the
        // § 6.6 MSDO DOH check fires — only the LCR DOH requirement is exercised, and it must
        // not fire because TU2's own header is monotonic == 1.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, 2 xlayers
        data.extend(global);
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // doh 0
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0, monotonic 1, LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 (begin cond 1)
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "lcr/doh-constraint-required"),
            "a non-monotonic header from an earlier, ended CVS is outside this CMVS and must \
             not trigger the LCR DOH requirement; report was: {report}"
        );
    }

    #[test]
    fn msdo_doh_constraint_required_scoped_to_current_cmvs() {
        // Codex finding 4 applied to the § 6.6 `msdo/doh-constraint-required` check
        // (resolve_deferred_doh_constraint): it also iterated the whole-history
        // frame_confirmed_xlayers set. TU1/CVS1: xlayer 1 activates a non-monotonic header in a
        // standalone CVS (no MSDO → CMVS stays Outside) that ends. TU2/CVS2: opens a
        // definitively-Inside CMVS on xlayer 0 (a CLK + MSDO) whose own header is monotonic ==
        // 1, with multistream_doh_constraint_flag == 0. The leaked xlayer-1 header is outside
        // TU2's CMVS, so no `msdo/doh-constraint-required` may fire.
        let mut data = temporal_delimiter_obu(); // TU1/CVS1: a non-monotonic xlayer-1 header
        data.extend(seq_header_obu_lcr_ref(1, 5, 0, false, 0)); // xlayer 1, monotonic 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 5)); // CLK xlayer 1 → confirms seq 5
        data.extend(temporal_delimiter_obu()); // TU2/CVS2: a fresh CMVS on xlayer 0
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // doh 0
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // xlayer 0, monotonic 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "msdo/doh-constraint-required"),
            "a non-monotonic header from an earlier, ended CVS is outside this CMVS and must \
             not trigger the § 6.6 MSDO DOH requirement; report was: {report}"
        );
    }

    // -- § 7.3.2 cmvs/boundary-set-mismatch -------------------------------------

    #[test]
    fn cmvs_boundary_set_mismatch_is_flagged() {
        // § 7.3.2 boundary-set identity: a CMVS opens (TU1: MSDO + global LCR activated by
        // the header + CLK), then TU2 begins a new coded video sequence (a CLK) with NO
        // OBU_MSDO but WITH the activated global LCR. Under the MSDO-alone rules TU2 ends the
        // CMVS (end condition 2); under the MSDO+global-LCR rules it does not — the boundary
        // sets diverge, so cmvs/boundary-set-mismatch fires. The global LCR's
        // lcr_doh_constraint_flag matches the MSDO's (both 0), the xlayer_map count matches
        // num_streams (2), and aggregate/PTL info is absent, so no § 6.8.2 disagreement is
        // raised — only the boundary mismatch.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
        let mut data = temporal_delimiter_obu(); // temporal unit 1: opens the CMVS
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global.clone());
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
        data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
        // The global LCR is re-sent and re-activated by a same-id CLK; no MSDO this TU.
        data.extend(global);
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
        data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "cmvs/boundary-set-mismatch"
                    && d.spec_section.as_deref() == Some("7.3.2")
            }),
            "the boundary-set divergence must fire; report was: {report}"
        );
    }

    #[test]
    fn cmvs_boundary_set_no_mismatch_when_clk_carries_msdo() {
        // § 7.3.2: when the CLK-bearing TU2 also carries an OBU_MSDO, end condition 2 does
        // not apply under EITHER rule set (it begins a new CMVS instead), so the boundary
        // sets agree — no mismatch.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global.clone());
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(temporal_delimiter_obu()); // TU2: carries an MSDO
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global);
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
            "a CLK TU carrying an MSDO does not diverge the boundary sets; report was: {report}"
        );
    }

    #[test]
    fn cmvs_boundary_set_silent_for_unactivated_global_lcr() {
        // § 7.3.2: when the global LCR in the CLK-bearing TU is only PRESENT but never
        // activated (the CMVS tracker routes that to Unknown), the divergence is undecidable
        // and must stay silent (lesson 12). Here TU2's CLK references seq_lcr_id 0, so no
        // global LCR is activated.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // no LCR association
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(temporal_delimiter_obu()); // TU2: global LCR present but unactivated
        data.extend(global);
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK, still ref seq 0 (lcr 0)
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
            "an unactivated global LCR keeps the boundary check silent; report was: {report}"
        );
    }

    #[test]
    fn cmvs_boundary_set_silent_when_activated_global_lcr_only_earlier_not_in_boundary_tu() {
        // Codex finding (3393274375): cmvs/boundary-set-mismatch over-fired. § 7.3.2 end
        // condition 2's divergence requires the BOUNDARY temporal unit itself to "have an
        // activated global layer configuration record" — a property of that temporal unit, not
        // of the whole CMVS window. Pre-fix the resolution found ANY activated global LCR
        // anywhere in the window, so a CMVS that activated a global LCR EARLIER over-fired at a
        // later CLK boundary TU that activated none of its own.
        //
        // TU1 opens the CMVS: MSDO (substreams 0,1), global LCR id 1 (map 0b11, doh 0) activated
        // by xlayer 0's header (seq_lcr_id 1, monotonic 1), CLK xlayer 0. xlayer 0's activated
        // global LCR remains chain-resolvable. TU2 is the boundary: it carries a global LCR OBU
        // (present → a boundary divergence CANDIDATE) and a CLK on xlayer 1 referencing a header
        // with seq_lcr_id 0 (NO LCR activation in TU2). xlayer 0 is NOT re-activated in TU2, so
        // the only global LCR activation lies in TU1, not the boundary TU. Both rule sets end
        // the CMVS at TU2 → no divergence → cmvs/boundary-set-mismatch must stay silent.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, LcrXLayerID {0,1}
        let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, activates global LCR
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global.clone());
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0 activates global LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
        data.extend(temporal_delimiter_obu()); // TU2: boundary — global LCR present but unactivated here
        data.extend(global); // global LCR present (divergence candidate), re-sent
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 0)); // xlayer 1 header, seq_lcr_id 0 (no LCR)
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, no LCR activation
        data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
            "the boundary TU activates no global LCR of its own (only an earlier TU did), so \
             both boundary rule sets end the CMVS here and there is no mismatch; report was: \
             {report}"
        );
    }

    #[test]
    fn lcr_only_cmvs_window_survives_to_later_frame_confirmed_activation() {
        // Codex finding (3393274378): an LCR-only CMVS opened via § 7.3.2 begin condition 3
        // (a CLK temporal unit that activates a global LCR with NO OBU_MSDO) is routed to
        // CmvsState::Unknown. A LATER temporal unit with no CLK fires no § 7.3.2 end condition
        // (end conditions 1/2 both require a CLK that "begins a new coded video sequence"), so
        // the CMVS window must be KEPT — pre-fix the window action returned Close, clearing the
        // window, and a later frame-confirmed non-monotonic activation in that LCR-only CMVS
        // was skipped by the deferred § 6.8.2 LCR-DOH check.
        //
        // TU1 opens the LCR-only CMVS: global LCR id 1 (lcr_doh_constraint_flag == 0) activated
        // by xlayer 0's header (seq_lcr_id 1, monotonic 1 → no DOH violation yet), CLK xlayer 0,
        // no MSDO. TU2 is a continuation (no CLK): xlayer 1's header (seq_lcr_id 1, monotonic 0)
        // is frame-confirmed by a regular tile group. With the window kept, xlayer 1's activation
        // lies in the CMVS, so lcr/doh-constraint-required must fire.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, xlayers 0,1
        let mut data = temporal_delimiter_obu(); // TU1: opens the LCR-only CMVS (begin cond 3)
        data.extend(global);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0, monotonic 1, activates LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, no MSDO
        data.extend(temporal_delimiter_obu()); // TU2: continuation (no CLK) — window must be kept
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // xlayer 1, monotonic 0, refs LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // regular tile group confirms xlayer 1
        data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/doh-constraint-required"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "the LCR-only CMVS window must survive a non-CLK temporal unit so a later \
             non-monotonic activation triggers the § 6.8.2 LCR-DOH requirement; report was: \
             {report}"
        );
    }

    #[test]
    fn lcr_msdo_agreement_flags_earlier_nonconforming_msdo_overwritten_by_later() {
        // Codex finding (3393274380): § 6.8.2 requires the MSDO↔global-LCR agreement to hold
        // for EVERY OBU_MSDO present in the CMVS, but the live `msdo_substream_max` is
        // last-wins. A non-conforming MSDO-A at the first RAP TU, then a conforming MSDO-B at a
        // later RAP TU of the SAME CMVS, must both be evaluated. Pre-fix the deferred resolution
        // read only the live (last-wins) MSDO record, so when MSDO-A's TU activates NO global
        // LCR (the agreement does not resolve there) and the global LCR is only activated LATER
        // in MSDO-B's TU, MSDO-B has already overwritten the live record — MSDO-A escapes.
        //
        // TU1 opens the CMVS (begin condition 1: CLK + MSDO-A) and activates NO global LCR:
        // xlayer 0's header references seq_lcr_id 0 (no LCR). So `activated_global_lcr()` is None
        // at TU1's boundary and MSDO-A is not evaluated yet. TU2 stays in the SAME CMVS (MSDO-B
        // shares every § 7.3.2 condition-2 key field with MSDO-A — only the RAP-permitted
        // sub_xlayer_id[i] differs — so it does not begin a new CMVS), introduces the global LCR
        // (map 0b11 → LcrXLayerID {0,1}), and activates it via xlayer 1 (seq_lcr_id 1, CLK). At
        // TU2's boundary the global LCR is activated and the live record is MSDO-B (conforming),
        // so pre-fix nothing fires. MSDO-A names sub_xlayer_id 2 (∉ {0,1}); accumulating every
        // in-window MSDO catches it. MSDO-B sits at TU2's CLK (a RAP), so § 7.3.8.2's non-RAP
        // identity rule does not fire on the sub_xlayer_id difference.
        let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // LcrXLayerID {0,1}
        let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, NO global LCR activated
        // MSDO-A: sub_xlayer_ids [0, 2] — sub_xlayer_id 2 ∉ {0,1} → disagrees with the LCR.
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (2, 0, 0, 0)],
        ));
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // xlayer 0, seq_lcr_id 0 (no LCR)
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
        data.extend(temporal_delimiter_obu()); // TU2: same CMVS, introduces+activates the global LCR
        // MSDO-B: same key fields, sub_xlayer_ids [0, 1] — all ∈ {0,1} → agrees. Only the
        // RAP-permitted sub_xlayer_id differs from MSDO-A, so no new CMVS begins.
        data.extend(msdo_obu_configured(
            31,
            false,
            &[(0, 0, 0, 0), (1, 0, 0, 0)],
        ));
        data.extend(global); // global LCR observed before the header that references it
        data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // xlayer 1, seq_lcr_id 1 → global LCR 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1 -> activates LCR 1
        data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"
                    && d.spec_section.as_deref() == Some("6.8.2")
                    // sub_xlayer_id 2 is carried ONLY by MSDO-A, so naming it proves the earlier
                    // non-conforming MSDO-A was evaluated, not just the later conforming MSDO-B.
                    && d.message.contains("sub_xlayer_id 2")
            }),
            "every MSDO in the CMVS must be evaluated, so the earlier non-conforming MSDO-A \
             (sub_xlayer_id 2 ∉ LcrXLayerID[]) must fire even though the later conforming MSDO-B \
             overwrote the live MSDO record; report was: {report}"
        );
    }

    // -- Annex A Table A.4 IOP presence re-land ---------------------------------

    #[test]
    fn annex_a_iop0_two_xlayers_without_msdo_is_flagged() {
        // Table A.4 row "0 Y": a profile-0 (IOP 0) coded video sequence with two distinct
        // non-global obu_xlayer_id values and no OBU_MSDO requires an OBU_MSDO. PR #46
        // scenario: multi-xlayer stream without MSDO. Both layers are frame-confirmed.
        let mut data = temporal_delimiter_obu();
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // xlayer 0, profile 0 (IOP 0)
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true)); // xlayer 1, profile 0
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/msdo-required-for-iop"
                    && d.spec_section.as_deref() == Some("A.2")
            }),
            "a two-xlayer IOP0 CVS without an MSDO must be flagged; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop0_single_xlayer_without_msdo_is_conforming() {
        // Table A.4 row "0 N": a single-extended-layer IOP0 CVS prohibits an MSDO and does
        // not require one — no diagnostic with one xlayer and no MSDO.
        let mut data = temporal_delimiter_obu();
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/msdo-")
                    || d.rule_id == "annex-a/lcr-required-for-iop"),
            "a single-xlayer IOP0 CVS needs no MSDO/LCR; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop0_two_xlayers_with_msdo_is_conforming() {
        // Table A.4 row "0 Y": with the required OBU_MSDO present, no diagnostic. The MSDO's
        // multistream_profile_idc 0 sets the IOP to 0.
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/msdo-")),
            "the MSDO satisfies the IOP0 multi-xlayer requirement; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_window_seeds_pre_clk_msdo_to_new_cvs() {
        // PR #46 scenario: pre-CLK MSDO belongs to the new sequence. TU1 is a single-xlayer
        // IOP0 CVS (no MSDO, conforming). TU2 carries an OBU_MSDO BEFORE its CLK; § 7.3.6
        // attributes that MSDO to the NEW coded video sequence (TU2), not TU1. So TU1's
        // window has no MSDO (and one xlayer — conforming), and the prohibited-MSDO rule does
        // not fire against TU1. The window machinery must not have leaked TU2's pre-CLK MSDO
        // into TU1's evaluation.
        let mut data = temporal_delimiter_obu(); // TU1: single-xlayer IOP0, no MSDO
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(temporal_delimiter_obu()); // TU2: MSDO precedes the CLK
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(seq_header_obu_ptl(0, 1, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 starts the new CVS
        data.extend(seq_header_obu_ptl(1, 2, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        // TU1's window (one xlayer, no MSDO) is conforming; TU2's window (two xlayers, MSDO
        // present) is conforming. No prohibited-MSDO false positive against TU1.
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-prohibited-for-iop"),
            "the pre-CLK MSDO belongs to the new CVS, not TU1; report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
            "TU2's two-xlayer CVS has its required MSDO; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop2_requires_global_lcr_unactivated_does_not_satisfy() {
        // PR #46 scenario: an unactivated global LCR does not satisfy the arm. Table A.4 row
        // "2 N Y" (IOP2, one xlayer, two embedded layers): MSDO prohibited; a local or
        // activated global LCR required. A global LCR is present but NEVER activated
        // (seq_lcr_id == 0), so the requirement still fails.
        let global = global_lcr_obu_agreement(1, 0b1, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(global);
        // Single xlayer 0, profile 2 (IOP 2), with two embedded layers (max_mlayer_id 1),
        // referencing no LCR (seq_lcr_id 0), so the global LCR is never activated.
        let payload = seq_header_payload_lcr_ref(0, 2, 0, false, true, 0, 1);
        data.extend(annex_b_obu(0x04, &payload));
        // A CLK frame at obu_mlayer_id 0 confirms the activation, and a second frame at
        // obu_mlayer_id 1 makes a second embedded layer present.
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "annex-a/lcr-required-for-iop"
                    && d.spec_section.as_deref() == Some("A.2")
            }),
            "an unactivated global LCR does not satisfy the IOP2 LCR requirement; report: {report}"
        );
    }

    #[test]
    fn annex_a_iop2_requires_global_lcr_activated_satisfies() {
        // Table A.4 row "2 N Y" boundary: the same IOP2 one-xlayer two-embedded-layer CVS,
        // but the header references the global LCR (seq_lcr_id == 1) so it is ACTIVATED — the
        // activated global LCR satisfies the requirement.
        let global = global_lcr_obu_agreement(1, 0b1, None, None, false);
        let mut data = temporal_delimiter_obu();
        data.extend(global);
        let payload = seq_header_payload_lcr_ref(0, 2, 0, false, true, 1, 1); // seq_lcr_id 1
        data.extend(annex_b_obu(0x04, &payload));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/lcr-required-for-iop"),
            "an activated global LCR satisfies the IOP2 LCR requirement; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_window_suppressed_under_external_hls() {
        // The Table A.4 presence checks need in-band HLS completeness, so they are suppressed
        // under any Provided external HLS — even the otherwise-flagged two-xlayer IOP0 CVS
        // without an MSDO.
        use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
        let mut data = temporal_delimiter_obu();
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(
                ExternalHlsSet::new().with_sequence_header_id(0),
            ),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("annex-a/") && d.rule_id.ends_with("-for-iop")),
            "external HLS suppresses the Table A.4 presence checks; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_window_silent_for_reserved_profile() {
        // A reserved seq_profile_idc (5) has no table-determined interoperability point, so
        // the Table A.4 row is not determinable and the presence check stays silent (the
        // reserved profile itself is flagged by annex-a/profile-reserved).
        let mut data = temporal_delimiter_obu();
        let payload = seq_header_payload_lcr_ref(0, 5, 0, false, true, 0, 0);
        data.extend(annex_b_obu(0x04, &payload));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        let payload1 = seq_header_payload_lcr_ref(1, 5, 0, false, true, 0, 0);
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(1, 0, 0, 1),
            &payload1,
        ));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.ends_with("-for-iop")),
            "a reserved profile leaves the Table A.4 row undeterminable; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_same_id_reactivation_seeds_new_window() {
        // PR #46 scenario: same-id reactivation. A profile-0 (IOP0) header is frame-confirmed
        // in TU1; TU2 has a SECOND xlayer's header plus a CLK that re-references the SAME
        // already-active header for xlayer 0 (no id change, so on_sequence_activation is
        // skipped). The new CVS's IOP must be seeded from the active confirmed header so the
        // two-xlayer IOP0 CVS without an MSDO is still flagged.
        let mut data = temporal_delimiter_obu(); // TU1: confirm xlayer 0 seq 0
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(temporal_delimiter_obu()); // TU2: a second xlayer + a same-id CLK
        data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true)); // xlayer 1 seq 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // same-id CLK xlayer 0, seq 0
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
            "a same-id CLK reactivation must seed the new IOP0 window so the two-xlayer CVS \
             without an MSDO is flagged; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_window_late_tu_second_xlayer_counts() {
        // PR #46 scenario: late-TU second xlayer. A second extended layer appears in a LATER
        // temporal unit of the SAME coded video sequence (no intervening CLK opens a new
        // CVS), so the window's distinct-xlayer count reaches 2 and the IOP0 multi-xlayer
        // MSDO requirement fires for the whole-CVS window.
        let mut data = temporal_delimiter_obu(); // TU1: opens the CVS with xlayer 0
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        data.extend(temporal_delimiter_obu()); // TU2: a second xlayer joins (no CLK new-CVS)
        data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // non-CLK frame xlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
            "a second xlayer in a later TU of the same CVS must reach E > 1; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_declared_count_precedence_over_observed() {
        // PR #46 scenario: declared-count precedence (Table A.3 definition order). An MSDO
        // declares num_streams_minus_2 + 2 = 2 (E > 1) even though only ONE distinct
        // non-global obu_xlayer_id (0) is actually coded. The declared count takes precedence
        // (mirror lines 148-149), so E > 1 and the IOP0 multi-xlayer requirement is satisfied
        // by the present MSDO — and crucially the prohibited-MSDO rule (which needs E == 1)
        // does NOT fire against the single observed xlayer.
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // only xlayer 0 is coded
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-prohibited-for-iop"),
            "the MSDO's declared count (2) takes precedence over the single observed xlayer, \
             so the MSDO is not prohibited; report was: {report}"
        );
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
            "the present MSDO satisfies the multi-xlayer requirement; report was: {report}"
        );
    }

    #[test]
    fn annex_a_iop_window_uses_association_time_global_lcr_snapshot() {
        // claude-review nit 4 (3393139837): the Table A.4 IOP window's activated-LCR
        // accounting must read `LcrMaxNumXLayerCount` from the *association-time* snapshot
        // (`LcrAssociation.global_record`), exactly like the § 6.8.2 agreement path, NOT a
        // live `global_lcr_records` lookup. A same-id global-LCR redefinition mid-CVS with a
        // different `lcr_xlayer_map` otherwise retargets the window's extended-layer count to
        // the later revision.
        //
        // Global LCR id 1 rev A has lcr_xlayer_map 0b1 -> LcrMaxNumXLayerCount 1 (E == 1).
        // Rev B redefines id 1 with lcr_xlayer_map 0b11 -> LcrMaxNumXLayerCount 2 (E > 1).
        // The header (profile 0 -> IOP0) associates rev A in TU1 and is frame-confirmed; that
        // window correctly counts E == 1. TU2 redefines id 1 to rev B, then a same-id CLK
        // re-references the still-active header, re-firing the IOP activation note that seeds
        // TU2's (new-CVS) window. The snapshot path keeps the activated count at rev A's 1
        // (E == 1, IOP0 -> MSDO neither required nor present -> no error). Pre-fix, the live
        // lookup at re-activation time sees rev B's count 2 (E > 1), so the IOP0 multi-xlayer
        // `annex-a/msdo-required-for-iop` fires falsely against a CVS with no MSDO.
        let global_a = global_lcr_obu_agreement(1, 0b1, None, None, false); // count 1
        let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // count 2 (redefine)
        let mut data = temporal_delimiter_obu(); // TU1: rev A present, header associates rev A
        data.extend(global_a);
        data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // profile 0, seq_lcr_id 1
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK confirms xlayer 0
        data.extend(temporal_delimiter_obu()); // TU2: rev B redefines id 1, then same-id CLK
        data.extend(global_b); // redefine id 1 AFTER the header associated rev A
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // same-id CLK re-activates
        data.extend(temporal_delimiter_obu());
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
            "the IOP window must use the association-time rev A (LcrMaxNumXLayerCount 1, E == 1) \
             so the single-xlayer IOP0 CVS requires no MSDO; report was: {report}"
        );
    }
}
