// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use splot_core::Error;
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus_partial};

use crate::checks::{Check, default_checks, syntax_error_diagnostic};
use crate::context::ValidatorContext;
use crate::diagnostic::{Diagnostic, Severity, ValidationReport};
use crate::error_location::{error_bit_offset, error_offset};
use crate::options::ValidationOptions;

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

    /// Validates `data` as an AV2 Annex B bitstream with the default
    /// [`ValidationOptions`] (no external HLS).
    ///
    /// A malformed bitstream is reported as one or more [`Severity::Error`]
    /// diagnostics, never as a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes(&self, data: &[u8]) -> ValidationReport {
        self.validate_bytes_with_options(data, &ValidationOptions::default())
    }

    /// Validates `data` as an AV2 Annex B bitstream using `options`.
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
        let parsed = parse_annex_b_obus_partial(data);
        let checks = default_checks();
        let mut context = ValidatorContext::default();
        for obu in &parsed.obus {
            context.observe_obu(obu, options, &mut report);
            run_checks(&checks, obu, &mut report);
        }
        if let Some(error) = parsed.error {
            report.push(parse_error_diagnostic(&error));
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
            bits.f(0, ceil_log2_u32(max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
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
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(1, 2); // max_tlayer_id
        bits.f(1, 3); // max_mlayer_id
        bits.f(0, 1); // seq_max_mlayer_cnt_minus_1
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
        bits.uvlc(0); // decoder_buffer_delay
        bits.uvlc(0); // encoder_buffer_delay
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
    fn hls_repeated_sequence_header_after_clk_starts_new_coded_video_sequence() {
        // Temporal unit 1: seq header (id 0, params A), then an OBU_CLOSED_LOOP_KEY for
        // xlayer 0 (0x10 = type 4, no extension, xlayer 0) with an empty payload (its
        // prefix parse fails, so it does not activate). Temporal unit 2 reuses
        // seq_header_id 0 with different params B — a legal reconfiguration in a new
        // CVS that must NOT be flagged as a non-identical repeat (AV2 § 7.3.8). The
        // second temporal delimiter clears the CVS-scoped fingerprint (the reset is at
        // the temporal-unit boundary, not the CLK), so params A and B are not compared.
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(annex_b_obu(0x10, &[]));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a new temporal unit clears the CVS fingerprint; report was: {report}"
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
        // non-identical repeat of the same id appears later in the SAME temporal unit.
        // The opening header's fingerprint survives the activating CLK, so the repeat
        // is still flagged (the previous CLK-level reset would have missed it).
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
    fn sequence_reconfiguration_across_temporal_unit_is_not_flagged() {
        // A new temporal unit (and CVS) legally reconfigures id 0 with different
        // layer limits. The temporal-unit reset clears the previous fingerprint, so
        // this is NOT flagged as a non-identical repeat (no false positive).
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
        data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
        data.extend(temporal_delimiter_obu());
        data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
            "a reconfiguration in a new temporal unit must not be flagged; report was: {report}"
        );
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
}
