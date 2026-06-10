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
        // The end of the bitstream completes the final temporal unit, flushing the
        // deferred coded-video-sequence-scoped diagnostics (AV2 § 7.3.6; see
        // ValidatorContext::finish).
        context.finish(&mut report);
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
        bits.f(0, 1); // seq_max_mlayer_cnt_minus_1
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
        // The sequence header activates for xlayer 0 (OBU-order fallback) and its
        // seq_lcr_id resolves to the local LCR, whose lcr_mlayer_map[0][0] includes
        // embedded layer 1 without layer 0 against default MLayerDependencyMap[1][0].
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/mlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_tlayer_dependency_missing_is_flagged() {
        // The activated global LCR's lcr_tlayer_map[1][3][0] includes tlayer 1
        // without tlayer 0 against the default TLayerDependencyMap[0][1][0].
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_embedded(5, 3, 0b1, &[0b10]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(1, 0, 0, 3),
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
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
        // carry the LCR OBU's offset, which precedes the sequence header here.
        let td = temporal_delimiter_obu();
        let lcr = local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]);
        let seq_start = (td.len() + lcr.len()) as u64;
        let mut data = td;
        data.extend(lcr);
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
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
        // without temporal layer 0 against the default TLayerDependencyMap[0][1][0].
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b1, &[0b10]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            has_error(&report, "lcr/tlayer-dependency-missing"),
            "report was: {report}"
        );
    }

    #[test]
    fn lcr_global_mlayer_dependency_missing_is_flagged() {
        // Global LCR × mlayer map: lcr_mlayer_map[1][3] includes embedded layer 1
        // without embedded layer 0 against the default MLayerDependencyMap[1][0].
        let mut data = temporal_delimiter_obu();
        data.extend(global_lcr_obu_with_embedded(5, 3, 0b10, &[0b1]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(1, 0, 0, 3),
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
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
        // Under any Provided mode an externally-provided local LCR (not modeled)
        // could resolve seq_lcr_id ahead of the in-band record (§ 6.4.1), so the
        // agreement check is suppressed even when the set declares nothing —
        // mirroring the lcr/global-xlayer-map-missing-xlayer gate.
        let mut data = temporal_delimiter_obu();
        data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
        data.extend(annex_b_obu(
            0x04,
            &sequence_header_payload_with_lcr(0, 5, 1, 1),
        ));
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
        };
        let report = Validator::new(false).validate_bytes_with_options(&data, &options);
        assert!(
            !has_error(&report, "lcr/mlayer-dependency-missing")
                && !has_error(&report, "lcr/tlayer-dependency-missing"),
            "provided external HLS must suppress the LCR agreement check; report was: {report}"
        );
    }

    #[test]
    fn lcr_repeated_sequence_header_pairs_with_now_present_lcr() {
        // § 6.4.1 associates "this sequence header" with an LCR present prior to
        // it: the violating LCR arrives after the first header but before the
        // bit-identical repeat, so the repeat's association must be evaluated and
        // flagged exactly once.
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
}
