// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::span::ByteOffset;

use crate::test_bits::Bits;

fn inter_seq() -> InterSeqView {
    InterSeqView {
        num_ref_frames: 8,
        enable_short_refresh_frame_flags: false,
        explicit_ref_frame_map: true,
        enable_ref_frame_mvs: true,
        enable_bru: false,
        enable_tip: false,
        enable_tip_output: false,
        enable_tip_hole_fill: false,
        enable_refinemv: false,
        enable_tip_refinemv: false,
        seq_max_drl_bits_minus_1: 0,
        allow_frame_max_drl_bits: false,
        seq_frame_motion_modes_present_flag: false,
        seq_enabled_motion_modes: [false; MOTION_MODES],
        enable_opfl_refine: 0,
        max_mlayer_id: 0,
        mlayer_dependency_map: MLayerDependencyMap::default_for(EmbeddedLayerId::from_bits(0)),
        tlayer_dependency_map: TLayerDependencyMap::default_for(
            TemporalLayerId::BASE,
            EmbeddedLayerId::from_bits(0),
        ),
        seq_force_screen_content_tools: 0,
        seq_force_integer_mv: 0,
        allow_frame_max_bvp_drl_bits: false,
        frame_width_bits: 12,
        frame_height_bits: 12,
        sb_size: SuperblockSize::Block128x128,
    }
}

fn inter_ctx() -> InterFrameContext {
    InterFrameContext {
        obu_type: ObuType::RegularTileGroup,
        obu_tlayer_id: TemporalLayerId::BASE,
        frame_type: FrameType::Inter,
        is_bridge: false,
        bridge_frame_ref_idx: None,
        default_frame_size: Some(FrameSize::new(4096, 2304)),
        order_hint: Some(0),
        ras_long_term_ref_mask: None,
    }
}

#[test]
fn frame_max_drl_bits_round_trips_the_reference_quniform_code_for_all_defaults() {
    for seq_minus_1 in 0u32..=4 {
        for frame_minus_1 in 0u32..=4 {
            let mut bits = Bits::default();
            bits.bit(1); // signal_primary_ref_frame
            bits.bit(0); // disable_cross_frame_cdf_init
            bits.f(2, 3); // primary_ref_frame
            bits.f(0, 8); // refresh_frame_flags
            bits.bit(1); // frame_explicit_ref_frame_map
            bits.f(1, 3); // num_total_refs
            bits.f(0, 3); // ref_frame_idx[0]
            bits.bit(0); // use_ref_frame_mvs
            bits.bit(0); // allow_intrabc
            if frame_minus_1 == seq_minus_1 {
                bits.bit(0);
            } else {
                bits.bit(1);
                bits.f(frame_minus_1 - u32::from(frame_minus_1 > seq_minus_1), 2);
            }
            bits.bit(1); // use_qtr_precision_mv
            bits.bit(1); // is_filter_switchable sentinel
            bits.bit(0); // disable_cdf_update
            let expected_consumed = bits.bit_len() as u64;
            let data = bits.into_bytes();
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let mut seq = inter_seq();
            seq.allow_frame_max_drl_bits = true;
            seq.seq_max_drl_bits_minus_1 = seq_minus_1;
            let mut control = InterControl::default();
            parse_inter_control_into(
                &mut reader,
                &seq,
                &inter_ctx(),
                &FrameReferenceStateView::unknown(),
                false,
                &mut control,
            )
            .unwrap();
            assert_eq!(control.max_drl_bits_minus_1, Some(frame_minus_1));
            assert_eq!(reader.consumed_bits(), expected_consumed);
            assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
        }
    }
}

#[test]
fn mv_precision_av2_codes_match_table_6_19() {
    for (precision, code) in [
        (MvPrecision::OnePel, 3),
        (MvPrecision::HalfPel, 4),
        (MvPrecision::QuarterPel, 5),
        (MvPrecision::EighthPel, 6),
    ] {
        assert_eq!(precision.av2_code(), code);
    }
}

#[test]
fn inter_mv_precision_paths_preserve_following_bit_alignment() {
    let cases: [(bool, &[u8], MvPrecision, u8); 4] = [
        (true, &[], MvPrecision::OnePel, 3),
        (false, &[0, 0], MvPrecision::HalfPel, 4),
        (false, &[1], MvPrecision::QuarterPel, 5),
        (false, &[0, 1], MvPrecision::EighthPel, 6),
    ];
    for (force_integer_mv, precision_bits, expected, code) in cases {
        let mut bits = Bits::default();
        bits.bit(1); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(2, 3); // primary_ref_frame
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs
        bits.f(0, 3); // ref_frame_idx[0]
        bits.bit(0); // use_ref_frame_mvs
        bits.bit(0); // allow_intrabc
        for &bit in precision_bits {
            bits.bit(bit);
        }
        bits.bit(1); // is_filter_switchable sentinel immediately after precision
        bits.bit(0); // disable_cdf_update
        let expected_consumed = bits.bit_len() as u64;
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        if force_integer_mv {
            seq.seq_force_screen_content_tools = 1;
            seq.seq_force_integer_mv = 1;
        }
        let mut control = InterControl::default();
        parse_inter_control_into(
            &mut reader,
            &seq,
            &inter_ctx(),
            &FrameReferenceStateView::unknown(),
            false,
            &mut control,
        )
        .unwrap();

        assert_eq!(control.mv_precision, Some(expected));
        assert_eq!(expected.av2_code(), code);
        assert_eq!(control.force_integer_mv, Some(force_integer_mv));
        assert_eq!(
            control.interpolation_filter,
            Some(InterpolationFilter::Switchable),
            "the sentinel after {expected:?} must remain synchronized"
        );
        assert_eq!(reader.consumed_bits(), expected_consumed);
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }
}

#[test]
fn inter_explicit_map_parses_through_to_shared_tail() {
    let mut bits = Bits::default();
    bits.bit(1); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
    bits.f(2, 3); // primary_ref_frame
    bits.f(0b1010_1010, 8); // refresh_frame_flags f(NumRefFrames)
    bits.bit(1); // frame_explicit_ref_frame_map (seq explicit_ref_frame_map)
    bits.f(2, 3); // num_total_refs
    bits.f(3, 3); // ref_frame_idx[0]
    bits.f(5, 3); // ref_frame_idx[1]
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0 (one bit)
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(0); // is_filter_switchable = 0
    bits.f(2, 2); // interpolation_filter = 2 (EIGHTTAP_SHARP)
    bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

    assert_eq!(control.signal_primary_ref_frame, Some(true));
    assert_eq!(control.primary_ref_frame, Some(2));
    assert_eq!(control.refresh_frame_flags, Some(0b1010_1010));
    assert_eq!(control.explicit_ref_frame_map, Some(true));
    assert_eq!(control.num_total_refs, Some(2));
    assert_eq!(control.ref_frame_idx, vec![3, 5]);
    assert_eq!(control.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(control.use_ref_frame_mvs, Some(false));
    assert_eq!(control.tip_frame_mode, Some(TipFrameMode::Disabled));
    assert_eq!(control.allow_tip_hole_fill, Some(false));
    assert_eq!(control.tip_global_wtd_index, Some(0));
    assert_eq!(control.allow_screen_content_tools, Some(false));
    assert_eq!(control.allow_intrabc, Some(false));
    assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
    assert_eq!(
        control.interpolation_filter,
        Some(InterpolationFilter::EighttapSharp)
    );
    assert_eq!(
        control.frame_enabled_motion_modes,
        Some([false; MOTION_MODES])
    );
    assert_eq!(control.disable_cdf_update, Some(false));
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

#[test]
fn bridge_overwrite_uses_short_refresh_flag_syntax() {
    let mut bits = Bits::default();
    bits.bit(1); // bridge_frame_overwrite_flag
    bits.bit(1); // has_refresh_frame_flags
    bits.f(5, 3); // frame_to_refresh
    bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
    bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_short_refresh_frame_flags = true;
    let ctx = InterFrameContext {
        obu_type: ObuType::BridgeFrame,
        obu_tlayer_id: TemporalLayerId::BASE,
        frame_type: FrameType::Inter,
        is_bridge: true,
        bridge_frame_ref_idx: Some(2),
        default_frame_size: None,
        order_hint: Some(0),
        ras_long_term_ref_mask: None,
    };
    let mut valid = [false; NUM_REF_FRAMES];
    valid[2] = true;
    let order_hints = [0; NUM_REF_FRAMES];
    let mut widths = [0; NUM_REF_FRAMES];
    let mut heights = [0; NUM_REF_FRAMES];
    widths[2] = 1280;
    heights[2] = 720;
    let state = FrameReferenceStateView::from_slots(&valid, &order_hints, &widths, &heights);

    let mut control = InterControl::default();

    parse_inter_control_into(&mut reader, &seq, &ctx, &state, false, &mut control).unwrap();

    assert_eq!(control.refresh_frame_flags, Some(1 << 5));
    assert_eq!(control.frame_size, Some(FrameSize::new(1280, 720)));
    assert_eq!(control.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert_eq!(reader.consumed_bits(), 29);
}

/// AV2 § 5.18.2 (mirror :5039-5043): on the ordinary inter / switch path (TipFrameMode
/// disabled, `!bru_inactive`, `!IsBridge`), the control region reads `disable_cdf_update`
/// f(1) — the `else` arm of `if ( TipFrameMode == TIP_FRAME_AS_OUTPUT || bru_inactive ||
/// IsBridge )` — immediately before the shared-tail `tile_info()` (mirror :5183). On the
/// `ReachedSharedTail` stop the parser must consume that bit and record it, so the shared
/// tail starts at the exact bit position. Asserts the consumed-bit count and the recorded
/// flag for both `disable_cdf_update` values. (Pre-fix the bit was not consumed: the
/// consumed count was one short and `disable_cdf_update` stayed `None`, so the shared tail
/// would have started one bit early.)
fn reached_shared_tail_consumes_disable_cdf_update(disable_cdf_update_bit: u8) {
    const BITS_BEFORE_DISABLE_CDF_UPDATE: u64 = 30;
    let mut bits = Bits::default();
    bits.bit(1); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
    bits.f(2, 3); // primary_ref_frame
    bits.f(0b1010_1010, 8); // refresh_frame_flags f(NumRefFrames)
    bits.bit(1); // frame_explicit_ref_frame_map (seq explicit_ref_frame_map)
    bits.f(2, 3); // num_total_refs
    bits.f(3, 3); // ref_frame_idx[0]
    bits.f(5, 3); // ref_frame_idx[1]
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(0); // is_filter_switchable = 0
    bits.f(2, 2); // interpolation_filter = 2
    bits.bit(disable_cdf_update_bit); // disable_cdf_update f(1) (mirror :5041)
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    assert_eq!(
        control.disable_cdf_update,
        Some(disable_cdf_update_bit != 0)
    );
    assert_eq!(reader.consumed_bits(), BITS_BEFORE_DISABLE_CDF_UPDATE + 1);
}

#[test]
fn reached_shared_tail_consumes_disable_cdf_update_zero() {
    reached_shared_tail_consumes_disable_cdf_update(0);
}

#[test]
fn reached_shared_tail_consumes_disable_cdf_update_one() {
    reached_shared_tail_consumes_disable_cdf_update(1);
}

/// Builds the explicit-map inter prefix up to (and not including) the
/// `frame_opfl_refine_type()` reads, with `num_total_refs == 1` so the TIP gate / tmvp
/// reads stay absent. The caller appends the `opfl_refine_type` / `opfl_refine_all` bits.
fn opfl_refine_prefix(bits: &mut Bits) {
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
    bits.f(0, 8); // refresh_frame_flags f(NumRefFrames)
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(0, 3); // ref_frame_idx[0]
    bits.bit(0); // use_ref_frame_mvs = 0 (num_total_refs == 1 -> no tmvp)
}

/// Appends the inter tail after `frame_opfl_refine_type()`: screen_content / intrabc /
/// MV-precision / interpolation-filter, converging into the shared tail.
fn opfl_refine_tail(bits: &mut Bits) {
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1 (no interpolation_filter f(2))
    bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
}

#[test]
fn opfl_refine_auto_preserves_following_bit_alignment() {
    for (switchable, expected) in [(1, REFINE_SWITCHABLE), (0, REFINE_NONE)] {
        let mut bits = Bits::default();
        opfl_refine_prefix(&mut bits);
        bits.bit(switchable);
        if switchable == 0 {
            bits.bit(0); // opfl_refine_all
        }
        opfl_refine_tail(&mut bits);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_opfl_refine = REFINE_AUTO;
        let ctx = inter_ctx();
        let rs = FrameReferenceStateView::unknown();
        let mut control = InterControl::default();
        parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
        assert_eq!(control.tip_frame_mode, Some(TipFrameMode::Disabled));
        assert_eq!(control.opfl_refine_type, Some(expected));
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(
            control.interpolation_filter,
            Some(InterpolationFilter::Switchable)
        );
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
    }
}

#[test]
fn inter_implicit_map_stops_unmodeled() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags f(8)
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.explicit_ref_frame_map, Some(false));
    assert_eq!(control.num_total_refs, None);
    assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
}

/// AV2 § 7.7 — the implicit reference map (`!explicitRefFrameMap`) is modeled when the
/// reference state proves exactly one valid slot: `get_ref_frames()` derives
/// `NumTotalRefs == 1`, `ref_frame_idx == [theSlot]` (no bits), and the control region
/// parses through to the shared tail. (The fixture-bytes end-to-end proof lives in
/// info.rs::frame_header_core_inter_implicit_map_reaches_shared_tail_on_fixture.)
#[test]
fn inter_implicit_map_one_valid_slot_reaches_shared_tail() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags f(8)
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update f(1), just before the shared tail
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let mut ctx = inter_ctx();
    ctx.default_frame_size = Some(FrameSize::new(64, 64));
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[0] = true;
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_w[0] = 64;
    ref_h[0] = 64;
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.explicit_ref_frame_map, Some(false));
    assert_eq!(control.num_total_refs, Some(1));
    assert_eq!(control.ref_frame_idx, vec![0]);
    assert_eq!(control.use_ref_frame_mvs, Some(false));
    assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
    assert_eq!(control.disable_cdf_update, Some(false));
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

/// AV2 § 7.7 — the implicit map with no valid slot derives `NumTotalRefs == 0` and reaches
/// the shared tail (`ref_frame_idx` empty); the gate admits the zero-reference case too.
#[test]
fn inter_implicit_map_no_valid_slot_reaches_shared_tail_with_zero_refs() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags f(8)
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update f(1)
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let ctx = inter_ctx();
    let ref_valid = [false; NUM_REF_FRAMES];
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let ref_w = [0u32; NUM_REF_FRAMES];
    let ref_h = [0u32; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.num_total_refs, Some(0));
    assert!(control.ref_frame_idx.is_empty());
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

/// AV2 § 7.7 — the implicit map with two valid slots stays an UnmodeledDerivation
/// stop (the § 7.7 ranking needs the unmodeled RefBaseQIdx / RefCounter / layer scoring).
#[test]
fn inter_implicit_map_two_valid_slots_stops_unmodeled() {
    for (second_slot, order_hint) in [(2, 0), (1, 10)] {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.explicit_ref_frame_map = false;
        let mut ctx = inter_ctx();
        ctx.order_hint = Some(order_hint);
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[0] = true;
        ref_valid[second_slot] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let ref_w = [64u32; NUM_REF_FRAMES];
        let ref_h = [64u32; NUM_REF_FRAMES];
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let mut control = InterControl::default();
        parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
        assert_eq!(control.num_total_refs, None);
        assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
    }
}

/// AV2 § 7.7 — with `RefBaseQIdx` modeled, the implicit map over two valid slots is
/// derived exactly (the multi-reference brick `DECODE-INTER-MULTIREF-RUNTIME`). This
/// is the parser-side proof that `from_slots_with_base_q_idx` lifts the at-most-one
/// gate: with order_hint 10, slot 0 (oh 8, q 40) and slot 1 (oh 5, q 40), both past
/// references at equal dims, § 7.7 ranks slot 0 (score -248) before slot 1 (-220), so
/// `NumTotalRefs == 2` and `ref_frame_idx == [0, 1]`, mirroring the
/// `get_ref_frames::two_distinct_refs_rank_by_score` table-level worked example.
#[test]
fn inter_implicit_map_two_valid_slots_with_base_q_idx_ranks_exact() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
    bits.f(0, 8); // refresh_frame_flags f(NumRefFrames)
    bits.bit(0); // use_ref_frame_mvs = 0 (NumTotalRefs > 1, but mvs off -> no tmvp bits)
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(0); // is_filter_switchable = 0
    bits.f(2, 2); // interpolation_filter = 2
    bits.bit(0); // disable_cdf_update f(1)
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let mut ctx = inter_ctx();
    ctx.order_hint = Some(10);
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[1] = true;
    let mut ref_oh = [0u32; NUM_REF_FRAMES];
    ref_oh[0] = 8;
    ref_oh[1] = 5;
    let ref_w = [4096u32; NUM_REF_FRAMES];
    let ref_h = [2304u32; NUM_REF_FRAMES];
    let mut ref_q = [0u32; NUM_REF_FRAMES];
    ref_q[0] = 40;
    ref_q[1] = 40;
    let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
        &ref_valid, &ref_oh, &ref_w, &ref_h, &ref_q,
    );
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.num_total_refs, Some(2));
    assert_eq!(control.ref_frame_idx, vec![0, 1]);
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

/// AV2 § 7.7 — incomplete ranking inputs over two valid slots stay an
/// `UnmodeledDerivation` stop even though `RefBaseQIdx` is present: a `RefOrderHint`
/// slice shorter than the active reference-slot count cannot cover both valid slots, so
/// `derive_implicit_ref_map` must not silently default the missing slot's order hint to
/// zero and derive `ref_frame_idx` from fabricated state. This is the parser-side proof
/// of the `DECODE-INTER-MULTIREF-RUNTIME` completeness gate.
#[test]
fn inter_implicit_map_two_valid_slots_incomplete_ranking_inputs_stop_unmodeled() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags f(8)
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let mut ctx = inter_ctx();
    ctx.order_hint = Some(10);
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[1] = true;
    let ref_oh_short = [8u32];
    let ref_w = [4096u32; NUM_REF_FRAMES];
    let ref_h = [2304u32; NUM_REF_FRAMES];
    let mut ref_q = [0u32; NUM_REF_FRAMES];
    ref_q[0] = 40;
    ref_q[1] = 40;
    let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
        &ref_valid,
        &ref_oh_short,
        &ref_w,
        &ref_h,
        &ref_q,
    );
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.num_total_refs, None);
    assert_eq!(control.stop, Some(InterStop::UnmodeledDerivation));
}

#[test]
fn inter_implicit_map_allowed_frames_filters_disallowed_valid_slot() {
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let ctx = inter_ctx();
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[3] = true;
    let ref_oh = [8u32; NUM_REF_FRAMES];
    let ref_w = [4096u32; NUM_REF_FRAMES];
    let ref_h = [2304u32; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h)
        .with_allowed_frames(Some(1 << 3));
    let map = derive_implicit_ref_map(&seq, &ctx, &rs, false, None).unwrap();
    assert_eq!(map.num_total_refs, 1);
    assert_eq!(map.ref_frame_idx.as_slice(), &[3]);
}

/// § 7.7: every restricted slot (`RESTRICTED_ORDER_HINT` in the model, `RESTRICTED_OH` in the spec)
/// is left out of the ranking and appended after the ranked references, so two
/// restricted slots with equal saved state both reach the list.
#[test]
fn inter_implicit_map_appends_each_restricted_slot() {
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let mut ctx = inter_ctx();
    ctx.order_hint = Some(4);
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[..3].fill(true);
    let mut ref_oh = [0u32; NUM_REF_FRAMES];
    ref_oh[0] = 3;
    ref_oh[1] = u32::MAX;
    ref_oh[2] = u32::MAX;
    let ref_w = [192u32; NUM_REF_FRAMES];
    let ref_h = [192u32; NUM_REF_FRAMES];
    let ref_q = [140u32; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
        &ref_valid, &ref_oh, &ref_w, &ref_h, &ref_q,
    );
    let map =
        derive_implicit_ref_map(&seq, &ctx, &rs, true, Some(FrameSize::new(192, 192))).unwrap();
    assert_eq!(map.num_total_refs, 3);
    assert_eq!(map.ref_frame_idx.as_slice(), &[0, 1, 2]);
}

#[test]
fn inter_implicit_map_intersects_ras_and_allowed_frames_masks() {
    let mut seq = inter_seq();
    seq.explicit_ref_frame_map = false;
    let mut ctx = inter_ctx();
    ctx.ras_long_term_ref_mask = Some((1 << 1) | (1 << 2));
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[1] = true;
    ref_valid[2] = true;
    ref_valid[4] = true;
    let ref_oh = [8u32; NUM_REF_FRAMES];
    let ref_w = [4096u32; NUM_REF_FRAMES];
    let ref_h = [2304u32; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h)
        .with_allowed_frames(Some((1 << 2) | (1 << 4)));
    let map = derive_implicit_ref_map(&seq, &ctx, &rs, false, None).unwrap();
    assert_eq!(map.num_total_refs, 1);
    assert_eq!(map.ref_frame_idx.as_slice(), &[2]);
}

#[test]
fn inter_ref_idx_out_of_buffer_range_is_flagged() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 20); // refresh_frame_flags f(NumRefFrames == 20)
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(17, 5); // ref_frame_idx[0] = 17 (>= NUM_REF_FRAMES 16)
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.num_ref_frames = 20;
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert!(control.has_invalid_ref_frame_idx);
    assert_eq!(control.ref_frame_idx, vec![17]);
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

/// AV2 § 6.17.2 (mirror :4605-4606). With a non-power-of-two NumRefFrames the
/// read width `CeilLog2(NumRefFrames)` over-covers the active slot range, so an encoded
/// `ref_frame_idx[i]` can exceed `NumRefFrames - 1` while still fitting the buffer bound
/// of 16. RefValid is defined only over `0..NumRefFrames-1`, so such an index is an
/// unconditional violation. NumRefFrames == 6 -> 3-bit ref_frame_idx (values 0..7); an
/// idx of 7 is >= 6 and must be flagged even though it is < NUM_REF_FRAMES (16).
fn parse_ref_idx_with_num_ref_frames_6(idx: u32) -> InterControl {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 6); // refresh_frame_flags f(NumRefFrames == 6)
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(idx, 3); // ref_frame_idx[0] f(CeilLog2(6) == 3)
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.num_ref_frames = 6;
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    control
}

#[test]
fn inter_ref_idx_beyond_active_num_ref_frames_is_flagged() {
    let control = parse_ref_idx_with_num_ref_frames_6(7);
    assert!(
        control.has_invalid_ref_frame_idx,
        "ref_frame_idx == 7 with NumRefFrames == 6 names a slot outside the active \
         buffer (RefValid undefined there); §6.17.2 requires it be flagged"
    );
    assert_eq!(control.ref_frame_idx, vec![7]);
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

#[test]
fn inter_ref_idx_within_active_num_ref_frames_is_silent() {
    let control = parse_ref_idx_with_num_ref_frames_6(5);
    assert!(
        !control.has_invalid_ref_frame_idx,
        "ref_frame_idx == 5 with NumRefFrames == 6 is in the active range, so the \
         active-bound flag must not fire"
    );
    assert_eq!(control.ref_frame_idx, vec![5]);
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

#[test]
fn switch_frame_infers_primary_ref_none_and_reads_explicit_map() {
    let mut bits = Bits::default();
    bits.f(0xFF, 8);
    bits.f(1, 3); // num_total_refs = 1
    bits.f(4, 3); // ref_frame_idx[0]
    bits.f(1920 - 1, 12);
    bits.f(1080 - 1, 12);
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let mut ctx = inter_ctx();
    ctx.frame_type = FrameType::Switch;
    ctx.obu_type = ObuType::Switch;
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, true, &mut control).unwrap();
    assert_eq!(control.primary_ref_frame, Some(PRIMARY_REF_NONE));
    assert_eq!(control.explicit_ref_frame_map, Some(true));
    assert_eq!(control.num_total_refs, Some(1));
    assert_eq!(control.frame_size, Some(FrameSize::new(1920, 1080)));
    assert_eq!(control.use_ref_frame_mvs, Some(false));
    assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
}

#[test]
fn frame_size_with_refs_copies_valid_ref_dims() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(2, 3); // ref_frame_idx[0] = 2
    bits.bit(1); // found_ref = 1 (frame_size_with_refs)
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let ctx = inter_ctx();
    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[2] = true;
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_w[2] = 1280;
    ref_h[2] = 720;
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, true, &mut control).unwrap();
    assert_eq!(control.frame_size, Some(FrameSize::new(1280, 720)));
}

#[test]
fn frame_size_with_refs_continues_when_hit_slot_unknown() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(2, 3); // ref_frame_idx[0] = 2 (Unknown slot under the unknown() view)
    bits.bit(1); // found_ref = 1 -> hits the unmodeled slot 2 (size unknown)
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    bits.bit(0); // disable_cdf_update f(1), just before the shared tail
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, true, &mut control).unwrap();
    assert_eq!(
        control.frame_size, None,
        "the hit slot's dims are unmodeled, so the size is genuinely unknown"
    );
    assert_eq!(
        control.use_ref_frame_mvs,
        Some(false),
        "the presence-known field after the size hit is reached, not dropped (F4)"
    );
    assert_eq!(control.allow_intrabc, Some(false));
    assert_eq!(control.mv_precision, Some(MvPrecision::HalfPel));
    assert_eq!(control.disable_cdf_update, Some(false));
    assert_eq!(
        control.stop,
        Some(InterStop::ReachedSharedTail),
        "with the bit position exact, the control region parses through to the shared tail"
    );
}

#[test]
fn bru_triple_reads_and_inactive_returns() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(2, 3); // num_total_refs = 2
    bits.f(0, 3); // ref_frame_idx[0]
    bits.f(1, 3); // ref_frame_idx[1]
    bits.bit(1); // use_bru
    bits.f(1, 1); // bru_ref f(CeilLog2(2)=1)
    bits.bit(1); // bru_inactive = 1
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_bru = true;
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
    assert_eq!(control.use_bru, Some(true));
    assert_eq!(control.bru_ref, Some(1));
    assert_eq!(control.bru_inactive, Some(true));
    assert_eq!(control.use_ref_frame_mvs, Some(false));
    assert_eq!(control.stop, Some(InterStop::BruInactiveOrBridgeReturn));
}

#[test]
fn tip_gate_stops_poisoned_on_tip_frame_mode() {
    for tip_output_obu in [false, true] {
        let mut bits = Bits::default();
        let mut fields = vec![(0, 1)]; // signal_primary_ref_frame
        if !tip_output_obu {
            fields.push((0, 1)); // disable_cross_frame_cdf_init (uncoded for TIP OBUs)
        }
        fields.extend([
            (0, 8), // refresh_frame_flags
            (1, 1), // frame_explicit_ref_frame_map
            (2, 3), // num_total_refs = 2
            (0, 3), // ref_frame_idx[0]
            (1, 3), // ref_frame_idx[1]
            (1, 1), // use_ref_frame_mvs = 1
            (0, 1), // tmvp_sample_step_minus_1 (num_total_refs>1, sb 128 != 64x64)
        ]);
        if !tip_output_obu {
            fields.push((1, 1)); // tip_frame_mode = 1 (uncoded for TIP OBUs: AS_OUTPUT)
        }
        for (value, width) in fields {
            bits.f(value, width);
        }
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_tip = true;
        seq.enable_tip_output = tip_output_obu;
        let mut ctx = inter_ctx();
        if tip_output_obu {
            ctx.obu_type = ObuType::RegularTip;
        }
        let rs = FrameReferenceStateView::unknown();
        let mut control = InterControl::default();
        parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();
        assert_eq!(
            (
                control.use_ref_frame_mvs,
                control.tmvp_sample_step_minus_1,
                control.tip_frame_mode,
                control.stop,
            ),
            (
                Some(true),
                Some(false),
                Some(if tip_output_obu {
                    TipFrameMode::AsOutput
                } else {
                    TipFrameMode::AsRef
                }),
                Some(InterStop::PoisonedReferenceState),
            ),
            "tip_output_obu={tip_output_obu}"
        );
        assert_eq!(control.allow_tip_hole_fill, Some(false));
        assert_eq!(control.tip_global_wtd_index, None);
    }
}

#[test]
fn tip_output_derives_opfl_refine_without_reads() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame; TIP OBUs do not code disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(2, 3); // num_total_refs = 2
    bits.f(0, 3); // ref_frame_idx[0]
    bits.f(1, 3); // ref_frame_idx[1]
    bits.bit(1); // use_ref_frame_mvs
    bits.bit(0); // tmvp_sample_step_minus_1

    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_tip = true;
    seq.enable_tip_output = true;
    seq.enable_tip_refinemv = true;
    seq.enable_opfl_refine = REFINE_AUTO;
    let mut ctx = inter_ctx();
    ctx.obu_type = ObuType::RegularTip;
    let rs = FrameReferenceStateView::unknown();

    let mut control = InterControl::default();

    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

    assert_eq!(control.tip_frame_mode, Some(TipFrameMode::AsOutput));
    assert_eq!(control.allow_tip_hole_fill, Some(false));
    assert_eq!(control.tip_global_mv, None);
    assert_eq!(control.tip_interpolation_filter, None);
    assert_eq!(control.opfl_refine_type, Some(REFINE_ALL));
    assert_eq!(control.stop, Some(InterStop::PoisonedReferenceState));
    assert_eq!(reader.consumed_bits(), 21);
}

#[test]
fn tip_output_retains_signed_global_mv_and_regular_filter() {
    let mut bits = Bits::default();
    bits.f(0, 9); // signal_primary_ref_frame and refresh_frame_flags
    bits.f(0b101_0000_0011_0101, 15); // explicit refs, TMVP, and TIP weight
    bits.bit(0); // tip_mv_zero
    bits.f(3, 4); // tip_mv_row
    bits.f(4, 4); // tip_mv_col
    bits.bit(1); // tip_mv_row_sign
    bits.bit(0); // tip_mv_col_sign
    bits.bit(0); // tip_sharp
    bits.bit(1); // tip_regular

    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_tip = true;
    seq.enable_tip_output = true;
    let mut ctx = inter_ctx();
    ctx.obu_type = ObuType::RegularTip;
    let ref_valid = [true, true, false, false, false, false, false, false];
    let ref_oh = [1, 2, 0, 0, 0, 0, 0, 0];
    let ref_w = [4096; 8];
    let ref_h = [2304; 8];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

    let mut control = InterControl::default();

    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

    assert_eq!(control.tip_frame_mode, Some(TipFrameMode::AsOutput));
    assert_eq!(control.tip_global_wtd_index, Some(5));
    assert_eq!(control.tip_global_mv, Some(TipGlobalMv { row: -3, col: 4 }));
    assert_eq!(
        control.tip_interpolation_filter,
        Some(InterpolationFilter::Eighttap)
    );
    assert_eq!(control.stop, Some(InterStop::TipAsOutputReturn));
    assert_eq!(reader.consumed_bits(), 37);
}

#[test]
fn tip_output_eof_before_prediction_controls_is_error() {
    let mut bits = Bits::default();
    bits.f(0, 9); // signal_primary_ref_frame and refresh_frame_flags
    bits.f(0b101_0000_0011_0101, 15); // explicit refs, TMVP, and TIP weight
    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_tip = true;
    seq.enable_tip_output = true;
    let mut ctx = inter_ctx();
    ctx.obu_type = ObuType::RegularTip;
    let ref_valid = [true, true, false, false, false, false, false, false];
    let ref_oh = [1, 2, 0, 0, 0, 0, 0, 0];
    let ref_w = [4096; 8];
    let ref_h = [2304; 8];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let mut control = InterControl::default();

    assert!(matches!(
        parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control),
        Err(crate::error::Error::UnexpectedEof { .. })
    ));
}

#[test]
fn non_output_tip_obu_consumes_coded_tip_mode() {
    let mut bits = Bits::default();
    bits.bit(0); // signal_primary_ref_frame; TIP OBUs do not code disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(2, 3); // num_total_refs = 2
    bits.f(0, 3); // ref_frame_idx[0]
    bits.f(1, 3); // ref_frame_idx[1]
    bits.bit(1); // use_ref_frame_mvs
    bits.bit(0); // tmvp_sample_step_minus_1
    bits.bit(1); // tip_frame_mode = TIP_FRAME_AS_REF

    let data = bits.into_bytes();
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let mut seq = inter_seq();
    seq.enable_tip = true;
    seq.enable_tip_output = false;
    let mut ctx = inter_ctx();
    ctx.obu_type = ObuType::RegularTip;
    let rs = FrameReferenceStateView::unknown();

    let mut control = InterControl::default();

    parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

    assert_eq!(control.tip_frame_mode, Some(TipFrameMode::AsRef));
    assert_eq!(control.allow_tip_hole_fill, Some(false));
    assert_eq!(control.stop, Some(InterStop::PoisonedReferenceState));
    assert_eq!(reader.consumed_bits(), 22);
}

#[test]
fn tip_as_ref_retains_coded_and_inferred_weights() {
    for equal_weight in [false, true] {
        let mut bits = Bits::default();
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(2, 3); // num_total_refs = 2
        bits.f(0, 3); // ref_frame_idx[0]
        bits.f(1, 3); // ref_frame_idx[1]
        bits.bit(1); // use_ref_frame_mvs
        bits.bit(0); // tmvp_sample_step_minus_1
        bits.bit(1); // tip_frame_mode = TIP_FRAME_AS_REF
        bits.bit(1); // allow_tip_hole_fill
        if !equal_weight {
            bits.f(5, 3); // tip_global_wtd_index
        }
        opfl_refine_tail(&mut bits);

        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut seq = inter_seq();
        seq.enable_tip = true;
        seq.enable_tip_hole_fill = true;
        seq.enable_tip_refinemv = equal_weight;
        seq.enable_opfl_refine = if equal_weight { REFINE_ALL as u8 } else { 0 };
        let mut ctx = inter_ctx();
        ctx.order_hint = Some(if equal_weight { 5 } else { 0 });
        let ref_valid = [true, true, false, false, false, false, false, false];
        let ref_oh = if equal_weight {
            [1, 9, 0, 0, 0, 0, 0, 0]
        } else {
            [1, 2, 0, 0, 0, 0, 0, 0]
        };
        let ref_w = [4096; 8];
        let ref_h = [2304; 8];
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

        let mut control = InterControl::default();

        parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).unwrap();

        assert_eq!(control.tip_frame_mode, Some(TipFrameMode::AsRef));
        assert_eq!(control.allow_tip_hole_fill, Some(true));
        assert_eq!(
            control.tip_global_wtd_index,
            Some(if equal_weight { 0 } else { 5 })
        );
        assert_eq!(control.tip_global_mv, Some(TipGlobalMv::default()));
        assert_eq!(
            control.tip_interpolation_filter,
            Some(InterpolationFilter::EighttapSharp)
        );
        assert_eq!(control.allow_intrabc, Some(false));
        assert_eq!(control.stop, Some(InterStop::ReachedSharedTail));
        assert_eq!(reader.consumed_bits(), if equal_weight { 29 } else { 32 });
    }
}

#[test]
fn tip_ref_counts_skip_restricted_order_hints() {
    let ref_valid = [true, true, true, false, false, false, false, false];
    let ref_oh = [u32::MAX, 1, 9, 0, 0, 0, 0, 0];
    let ref_w = [4096; 8];
    let ref_h = [2304; 8];
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

    assert_eq!(tip_ref_counts(&rs, &[0, 1, 2], 5), Some((1, 1)));
}

#[test]
fn inter_eof_before_primary_ref_is_error() {
    let data: [u8; 0] = [];
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let seq = inter_seq();
    let ctx = inter_ctx();
    let rs = FrameReferenceStateView::unknown();
    let mut control = InterControl::default();
    assert!(parse_inter_control_into(&mut reader, &seq, &ctx, &rs, false, &mut control).is_err());
}

fn fixture_sequence_and_inter_obu(
    data: &[u8],
) -> (
    crate::headers::sequence::SequenceHeader,
    crate::annexb::ObuEnvelope<'_>,
) {
    use crate::headers::sequence::SequenceHeader;
    use crate::obu::{ParsedObu, PayloadStatus};
    use crate::stream::{ParsedBitstream, parse_bitstream_partial};

    let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(data) else {
        panic!("fixture is an IVF container");
    };
    let mut seq_header: Option<SequenceHeader> = None;
    let mut inter_obu: Option<crate::annexb::ObuEnvelope<'_>> = None;
    for frame in &ivf.frames {
        for obu in &frame.obus {
            match obu.header.obu_type {
                ObuType::SequenceHeader => {
                    if let Ok(PayloadStatus::Parsed(ParsedObu::SequenceHeader(sh))) =
                        obu.payload_status()
                    {
                        seq_header = Some(*sh);
                    }
                }
                ObuType::RegularTileGroup if inter_obu.is_none() => {
                    inter_obu = Some(*obu);
                }
                _ => {}
            }
        }
    }
    (
        seq_header.expect("fixture has a sequence header"),
        inter_obu.expect("fixture has a regular tile group (inter) frame"),
    )
}

/// AV2 § 7.7 + § 5.18.2 — drive the real `syn-key-inter-64x64` fixture's inter frame
/// through `parse_frame_header_core` with the post-key reference state, and prove the
/// implicit reference map (`get_ref_frames()`) advances the parser past
/// `InterStop::UnmodeledDerivation` to the shared tail.
///
/// The fixture is a `OBU_CLOSED_LOOP_KEY` key (`refresh_frame_flags == 255`,
/// `base_q_idx == 70`, 64x64) + an `OBU_REGULAR_TILE_GROUP` inter frame
/// (`order_hint_lsb == 1`, `explicitRefFrameMap == 0`, `primary_ref_frame ==
/// PRIMARY_REF_CHOOSE`). After the key frame, the § 7.23 `first` rule (mirror :14132,
/// `(KEY) ? first : 1`) leaves only slot 0 `RefValid`, with `RefOrderHint[0] == 0` and
/// `RefFrameWidth/Height[0] == 64` — exactly the at-most-one-valid-reference case the
/// modeled `get_ref_frames()` resolves. The bit-level proof is that the parser consumes
/// the same inter-header field sequence the explicit map would (frame size / BRU /
/// MV-precision / interpolation filter / motion modes / disable_cdf_update) and reaches
/// `InterStop::ReachedSharedTail` — possible only if the implicit map yielded
/// `NumTotalRefs == 1`, `ref_frame_idx == [0]` and the parse continued from the right bit.
#[test]
fn frame_header_core_inter_implicit_map_reaches_shared_tail_on_fixture() {
    use crate::headers::frame::info::{FrameHeaderParseInput, parse_frame_header_core};

    let data =
        include_bytes!("../../../../../tests/conformance/vectors/valid/syn-key-inter-64x64.ivf");
    let (seq_header, inter_obu) = fixture_sequence_and_inter_obu(data);

    let mut ref_valid = [false; NUM_REF_FRAMES];
    let mut ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_oh[0] = 0;
    ref_w[0] = 64;
    ref_h[0] = 64;
    let reference_state = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

    let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
    let input = FrameHeaderParseInput {
        obu_type: inter_obu.header.obu_type,
        obu_tlayer_id: inter_obu.header.temporal_layer_id,
        first_picture_in_tu: false,
        active_sequence: Some(&seq_header),
        mfh_record: None,
        reference_state,
    };
    let core = parse_frame_header_core(&mut reader, &input).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Inter));
    let inter = core
        .inter
        .as_ref()
        .expect("the inter control region was parsed");
    assert_eq!(
        inter.explicit_ref_frame_map,
        Some(false),
        "the fixture uses the IMPLICIT reference map"
    );
    assert_eq!(inter.num_total_refs, Some(1));
    assert_eq!(inter.ref_frame_idx, vec![0]);
    assert_eq!(
        inter.stop,
        Some(InterStop::ReachedSharedTail),
        "with get_ref_frames() modeled, the implicit-map inter frame reaches the shared tail"
    );
    assert!(!core.status.is_truncated_in_modeled_region());
}

/// AV2 § 7.7 — the implicit map STAYS an `UnmodeledDerivation` stop when the
/// modeled reference state proves two valid slots (the § 7.7 ranking then needs the
/// unmodeled `RefBaseQIdx` / `RefCounter` / layer scoring inputs). Same fixture inter
/// bytes, but a reference state with two valid slots: the gate refuses to guess.
#[test]
fn frame_header_core_inter_implicit_map_two_valid_slots_stops_unmodeled() {
    use crate::headers::frame::info::{FrameHeaderParseInput, parse_frame_header_core};

    let data =
        include_bytes!("../../../../../tests/conformance/vectors/valid/syn-key-inter-64x64.ivf");
    let (seq_header, inter_obu) = fixture_sequence_and_inter_obu(data);

    // Two valid slots: the § 7.7 ranking depends on the unmodeled scoring inputs.
    let mut ref_valid = [false; NUM_REF_FRAMES];
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[1] = true;
    ref_w[0] = 64;
    ref_h[0] = 64;
    ref_w[1] = 64;
    ref_h[1] = 64;
    let reference_state = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

    let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
    let input = FrameHeaderParseInput {
        obu_type: inter_obu.header.obu_type,
        obu_tlayer_id: inter_obu.header.temporal_layer_id,
        first_picture_in_tu: false,
        active_sequence: Some(&seq_header),
        mfh_record: None,
        reference_state,
    };
    let core = parse_frame_header_core(&mut reader, &input).unwrap();
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.explicit_ref_frame_map, Some(false));
    assert_eq!(
        inter.stop,
        Some(InterStop::UnmodeledDerivation),
        "two valid slots need the unmodeled § 7.7 scoring inputs — no guessing"
    );
}

/// AV2 § 5.18.2 — drive the real `syn-2frame-inter-64x64` fixture's inter frame through
/// `parse_frame_header_core` with the post-key reference state, and prove the inter
/// frame header parses end to end through its § 5.18.2 shared tail and inter-specific
/// arms, reaching the terminal `FrameHeaderParseStatus::InterHeaderComplete`.
///
/// The fixture (`--enable-global-motion=1 --qp=80 --sb-size=64`) is a `OBU_CLOSED_LOOP_KEY`
/// key + an `OBU_REGULAR_TILE_GROUP` inter frame: a single 64x64 zero-MV skip block,
/// `explicitRefFrameMap == 0`, `NumTotalRefs == 1`, `TipFrameMode == TIP_FRAME_DISABLED`,
/// `!IsBridge`, `!bru_inactive`, broad tools off, exactly the minimal subset the shared
/// tail models. The asserted values are derived from the § 5.18.2 spec + the fixture
/// sequence config (the hand-decoded 56-bit inter payload: 27-bit control region + 21-bit
/// shared tail + 8-bit § 5.19 tile-group tail) and confirmed against the bit-exact
/// avmdec/dav2d decode the fixture records. The bit-level proof is that the parse consumes
/// exactly the shared-tail field sequence and reaches `InterHeaderComplete` — possible only
/// if every field width / presence (tile_info / quant / segmentation / deblocking's inter
/// allow_df_sub_pu arm / the inter coding-mode arms / global_motion / film_grain) was read
/// at the right bit.
#[test]
fn frame_header_core_inter_fixture_reaches_inter_header_complete() {
    use crate::headers::frame::info::{
        FrameHeaderParseInput, FrameHeaderParseStatus, parse_frame_header_core,
    };
    use crate::headers::frame::tail::TxMode;

    let data =
        include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");
    let (seq_header, inter_obu) = fixture_sequence_and_inter_obu(data);

    let mut ref_valid = [false; NUM_REF_FRAMES];
    let mut ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_oh[0] = 0;
    ref_w[0] = 64;
    ref_h[0] = 64;
    let reference_state = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);

    let mut reader = BitReader::new(inter_obu.payload, inter_obu.payload_offset());
    let input = FrameHeaderParseInput {
        obu_type: inter_obu.header.obu_type,
        obu_tlayer_id: inter_obu.header.temporal_layer_id,
        first_picture_in_tu: false,
        active_sequence: Some(&seq_header),
        mfh_record: None,
        reference_state,
    };
    let core = parse_frame_header_core(&mut reader, &input).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::InterHeaderComplete,
        "the minimal-subset inter frame parses its whole § 5.18.2 shared tail"
    );
    assert!(
        !core.status.is_truncated_in_modeled_region(),
        "a complete header is not a truncation"
    );
    assert_eq!(core.frame_type, Some(FrameType::Inter));

    let inter = core.inter.as_ref().expect("inter control region parsed");
    assert_eq!(inter.stop, Some(InterStop::ReachedSharedTail));
    assert_eq!(inter.explicit_ref_frame_map, Some(false));
    assert_eq!(inter.num_total_refs, Some(1));
    assert_eq!(inter.ref_frame_idx, vec![0]);

    let tile_info = core.tile_info.as_ref().expect("tile_info parsed");
    assert_eq!(
        tile_info.tile_cols, 1,
        "single 64x64 superblock -> one tile"
    );
    assert_eq!(tile_info.tile_rows, 1);
    let quant = core
        .quantization_params
        .as_ref()
        .expect("quantization_params parsed");
    assert_eq!(
        quant.base_q_idx, 119,
        "base_q_idx f(8) for the qp80 inter frame"
    );
    let seg = core
        .segmentation_params
        .as_ref()
        .expect("segmentation parsed");
    assert!(
        !seg.segmentation_enabled,
        "the minimal fixture disables segmentation"
    );
    let deblocking = core
        .deblocking_filter_params
        .as_ref()
        .expect("deblocking parsed");
    assert_eq!(
        deblocking.apply_deblocking_filter, [false; 4],
        "deblocking is off for the flat copy frame"
    );

    let tail = core.inter_tail.as_ref().expect("inter tail parsed");
    assert_eq!(tail.tx_mode, TxMode::Largest, "tx_mode_select == 0");
    assert!(!tail.reference_select, "single-reference frame");
    assert!(!tail.skip_mode_present);
    assert!(!tail.allow_bawp, "enable_bawp off");
    assert!(!tail.allow_warpmv_mode, "no DELTAWARP motion mode enabled");
    assert_eq!(tail.reduced_tx_set, 0);
    assert!(
        !tail.global_motion.use_global_motion,
        "global motion enabled in the sequence but unused by this frame"
    );
    assert!(!tail.film_grain.apply_grain, "no film grain applied");
}
