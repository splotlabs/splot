// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `single_picture_header_flag = 1` sequence header (§ 5.4.1) and its
//! key frame (§ 5.18.2) with the forced filter enables (§ 5.18.7.9,
//! § 5.18.7.10, § 5.18.7.12).

use super::*;
use crate::headers::frame::FrameType;

fn still(width: u32, height: u32, tools: EncoderToolFlags) -> EncoderStreamParams {
    EncoderStreamParams {
        width,
        height,
        bit_depth: BitDepthIdc::Ten,
        chroma: ChromaFormatIdc::Yuv444,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            single_picture: true,
            ..tools
        },
    }
}

/// Inter tools that the reduced header does not code.
fn inter_tools() -> EncoderToolFlags {
    EncoderToolFlags {
        warp: true,
        tip: true,
        ref_frame_mvs: true,
        global_motion: true,
        inter_ddt: true,
        long_term_frame_id_bits: 3,
        avg_cdf: Some(false),
        tcq: true,
        bawp: true,
        ..EncoderToolFlags::default()
    }
}

#[test]
fn single_picture_header_parses_with_its_inferred_values() {
    let tools = EncoderToolFlags {
        cdef: true,
        gdf: true,
        ccso: true,
        loop_restoration: true,
        ..inter_tools()
    };
    let seq = build_encoder_sequence_header(&still(200, 136, tools)).unwrap();
    let general = &seq.general;
    assert!(general.single_picture_header_flag && general.still_picture);
    assert!(general.monotonic_output_order_flag && general.max_tlayer_id.get() == 0);
    let inter = seq.inter.unwrap();
    assert_eq!((inter.order_hint_bits, inter.num_ref_frames), (0, 2));
    assert_eq!(inter.long_term_frame_id_bits, 0);
    assert!(!inter.enable_ref_frame_mvs && inter.enable_bawp);
    assert_eq!(
        seq.screen_content.unwrap().seq_force_screen_content_tools,
        2
    );
    let tq = seq.transform_quant_entropy.unwrap();
    assert!(tq.enable_tcq && !tq.choose_tcq_per_frame);
    assert!(tq.enable_avg_cdf && tq.avg_cdf_type == 1);

    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(
        &view,
        120,
        0,
        &FrameFilterParams::default(),
        KeyFrameTxParams::default(),
        FrameTiles::default(),
    )
    .unwrap();
    assert_eq!(core.frame_type, Some(FrameType::Key));
    let cdef = core.cdef_params.unwrap();
    assert!(cdef.cdef_frame_enable && cdef.strengths.as_slice() == [CdefStrengthSet::default()]);
    let gdf = core.gdf_params.unwrap();
    assert!(gdf.gdf_frame_enable && gdf.gdf_per_block == Some(true));
    let ccso = core.ccso_params.unwrap();
    assert!(ccso.planes.iter().all(|plane| !plane.ccso_planes));
}

/// Temporal layers, implicit output, extended SDP and the high tier (level 8
/// codes `seq_tier` only without `single_picture_header_flag`) are refused,
/// and so is an off GDF in a frame inside one GDF block, which codes no
/// `gdf_per_block`, and `allow_tcq`.
#[test]
fn single_picture_refuses_forms_the_reduced_header_cannot_carry() {
    let refused = [
        EncoderToolFlags {
            max_tlayer_id: TemporalLayerId::from_bits(1),
            ..EncoderToolFlags::default()
        },
        EncoderToolFlags {
            implicit_output: true,
            ..EncoderToolFlags::default()
        },
        EncoderToolFlags {
            sdp: true,
            extended_sdp: true,
            ..EncoderToolFlags::default()
        },
        EncoderToolFlags {
            level: Some(LevelIdx::from_bits(8)),
            tier: Tier::High,
            ..EncoderToolFlags::default()
        },
    ];
    for tools in refused {
        assert!(
            build_encoder_sequence_header(&still(64, 64, tools)).is_err(),
            "{tools:?}"
        );
    }

    let gdf = EncoderToolFlags {
        gdf: true,
        ..EncoderToolFlags::default()
    };
    let view =
        CoreSeqView::from_sequence(&build_encoder_sequence_header(&still(128, 128, gdf)).unwrap())
            .unwrap();
    let key = |gdf| {
        let filters = FrameFilterParams {
            gdf,
            ..FrameFilterParams::default()
        };
        build_key_frame_header(
            &view,
            120,
            0,
            &filters,
            KeyFrameTxParams::default(),
            FrameTiles::default(),
        )
    };
    assert!(key(None).is_err());
    assert!(key(Some(FrameGdf::default())).is_ok());
    let tcq = KeyFrameTxParams {
        allow_tcq: true,
        ..KeyFrameTxParams::default()
    };
    let tcq_view = CoreSeqView::from_sequence(
        &build_encoder_sequence_header(&still(64, 64, inter_tools())).unwrap(),
    )
    .unwrap();
    assert!(
        build_key_frame_header(
            &tcq_view,
            120,
            0,
            &FrameFilterParams::default(),
            tcq,
            FrameTiles::default()
        )
        .is_err()
    );
}

#[test]
fn truncated_single_picture_header_does_not_parse() {
    let bytes = sequence_header_bits(&still(200, 136, inter_tools())).unwrap();
    for len in [1, bytes.len() / 2, bytes.len() - 1] {
        let parsed = parse_sequence_header(&mut BitReader::new(&bytes[..len], ByteOffset::new(0)));
        assert!(
            parsed.map_or(true, |header| !header.is_fully_parsed()),
            "{len}"
        );
    }
}
