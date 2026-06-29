// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[derive(Default)]
pub(in crate::validator::tests) struct Bits {
    pub(in crate::validator::tests) bits: Vec<u8>,
}

impl Bits {
    pub(in crate::validator::tests) fn bit(&mut self, bit: u8) {
        self.bits.push(bit & 1);
    }

    pub(in crate::validator::tests) fn f(&mut self, value: u32, width: u32) {
        for shift in (0..width).rev() {
            self.bit(((value >> shift) & 1) as u8);
        }
    }

    pub(in crate::validator::tests) fn uvlc(&mut self, value: u32) {
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
    pub(in crate::validator::tests) fn rg(&mut self, value: u32, n: u32) {
        let q = value >> n;
        let remainder = value & ((1 << n) - 1);
        for _ in 0..q {
            self.bit(1);
        }
        self.bit(0);
        self.f(remainder, n);
    }

    pub(in crate::validator::tests) fn align(&mut self) {
        while !self.bits.len().is_multiple_of(8) {
            self.bit(0);
        }
    }

    /// Number of bits accumulated so far (for byte-exact test truncation).
    pub(in crate::validator::tests) fn bit_len(&self) -> usize {
        self.bits.len()
    }

    /// The raw per-bit values accumulated so far (one `u8` per bit), consuming the
    /// builder. Used to replay a bit sequence verbatim (e.g. a `frame_header_copy()`).
    pub(in crate::validator::tests) fn drain_bits(self) -> Vec<u8> {
        self.bits
    }

    pub(in crate::validator::tests) fn into_bytes(self) -> Vec<u8> {
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

pub(in crate::validator::tests) fn annex_b_obu(header: u8, payload: &[u8]) -> Vec<u8> {
    annex_b_obu_with_header(&[header], payload)
}

pub(in crate::validator::tests) fn annex_b_obu_with_header(
    header: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    let size = payload.len() + header.len();
    assert!(u8::try_from(size).is_ok());
    let mut data = Vec::with_capacity(payload.len() + header.len() + 1);
    data.push(size as u8);
    data.extend_from_slice(header);
    data.extend_from_slice(payload);
    data
}

pub(in crate::validator::tests) fn ivf_stream(payloads: &[&[u8]]) -> Vec<u8> {
    let mut data = Vec::new();
    let header = splot_core::ivf::IvfHeader::new(*b"AV02", 16, 16, 24, 1, payloads.len() as u32);
    assert!(splot_core::ivf::write_ivf_header(&mut data, &header).is_ok());
    for (pts, payload) in payloads.iter().enumerate() {
        assert!(splot_core::ivf::write_ivf_frame(&mut data, pts as u64, payload).is_ok());
    }
    data
}

pub(in crate::validator::tests) fn layer_obu_header(
    obu_type: u8,
    tlayer: u8,
    mlayer: u8,
    xlayer: u8,
) -> [u8; 2] {
    [
        0x80 | (obu_type << 2) | (tlayer & 0b11),
        ((mlayer & 0b111) << 5) | (xlayer & 0b1_1111),
    ]
}

pub(in crate::validator::tests) fn ceil_log2_u32(value: u32) -> u32 {
    if value <= 1 {
        0
    } else {
        u32::BITS - (value - 1).leading_zeros()
    }
}

pub(in crate::validator::tests) fn sequence_header_payload(
    max_tlayer_id: u32,
    max_mlayer_id: u32,
) -> Vec<u8> {
    sequence_header_payload_with_id(0, max_tlayer_id, max_mlayer_id)
}

pub(in crate::validator::tests) fn sequence_header_payload_with_id(
    seq_header_id: u32,
    max_tlayer_id: u32,
    max_mlayer_id: u32,
) -> Vec<u8> {
    sequence_header_payload_with_lcr(seq_header_id, 0, max_tlayer_id, max_mlayer_id)
}

pub(in crate::validator::tests) fn sequence_header_payload_with_lcr(
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
pub(in crate::validator::tests) fn append_non_single_child_configs(bits: &mut Bits) {
    bits.bit(0); // use_256x256_superblock
    bits.bit(0); // use_128x128_superblock
    bits.bit(0); // enable_sdp
    bits.bit(0); // enable_ext_partitions
    bits.bit(0); // reduce_pb_aspect_ratio
    bits.bit(0); // enable_ext_seg
    bits.bit(0); // seq_seg_info_present_flag
    bits.bit(0); // enable_dip
    bits.bit(0); // enable_intra_edge_filter
    bits.bit(0); // enable_mrls
    bits.bit(0); // enable_cfl_intra
    bits.f(0, 2); // cfl_ds_filter_index
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
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
    bits.bit(1); // seq_choose_screen_content_tools -> SELECT
    bits.bit(1); // seq_choose_integer_mv -> SELECT
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
    bits.bit(0); // disable_loopfilters_across_tiles
    bits.bit(0); // enable_cdef
    bits.bit(0); // enable_gdf
    bits.bit(0); // enable_restoration
    bits.bit(0); // enable_ccso
    bits.bit(0); // cdef_on_skip_txfm_always_on
    bits.bit(0); // cdef_on_skip_txfm_disabled -> Adaptive
    bits.f(0, 2); // df_par_bits_minus_2
    bits.bit(0); // seq_tile_info_present_flag
    bits.bit(0);
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
}

pub(in crate::validator::tests) fn sequence_header_payload_with_decoder_model_info() -> Vec<u8> {
    sequence_header_payload_with_decoder_model_sum(0, 0, 0)
}

/// A complete, activatable sequence header (`seq_header_id`, `max_tlayer_id == 1`,
/// `max_mlayer_id == 1`) carrying explicit `seq_decoder_model_info()` (§ 5.4.13)
/// with the given `decoder_buffer_delay` / `encoder_buffer_delay`.
pub(in crate::validator::tests) fn sequence_header_payload_with_decoder_model_sum(
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
    bits.uvlc(decoder_delay); // decoder_buffer_delay
    bits.uvlc(encoder_delay); // encoder_buffer_delay
    bits.bit(0); // low_delay_mode_flag
    bits.bit(0); // mlayer_dependency_present_flag
    bits.bit(0); // tlayer_dependency_present_flag
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

pub(in crate::validator::tests) fn stream_with_sequence_header(
    max_tlayer_id: u32,
    max_mlayer_id: u32,
) -> Vec<u8> {
    annex_b_obu(0x04, &sequence_header_payload(max_tlayer_id, max_mlayer_id))
}

pub(in crate::validator::tests) fn sequence_header_obu_for_xlayer(
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

pub(in crate::validator::tests) fn temporal_delimiter_obu() -> Vec<u8> {
    annex_b_obu(0x08, &[])
}

/// Whether `report` is conformant once the expected `celu/missing-output-frame-unit`
/// finding is set aside. A minimal HLS-only fixture (a sequence header / CI at a concrete
/// `obu_xlayer_id` with no frame-bearing OBU) is a *header-only* coded extended layer unit:
/// § 7.3.6 line 536 ("at least one coded output frame unit shall be present") applies to
/// every CELU, so such a fixture legitimately fires `celu/missing-output-frame-unit`. These
/// fixtures exercise an orthogonal concern (HLS parsing / timing / levels), so the helper
/// confirms the stream is otherwise conformant without weakening that concern's assertion.
pub(in crate::validator::tests) fn conformant_apart_from_header_only_celu(
    report: &ValidationReport,
) -> bool {
    report
        .errors()
        .all(|d| d.rule_id == "celu/missing-output-frame-unit")
}
