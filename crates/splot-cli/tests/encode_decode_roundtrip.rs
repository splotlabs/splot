// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-crate end-to-end oracle: `splot-encode` emits a decodable minimal intra skip
//! frame, and `splot decode` reconstructs it to a flat frame. `splot-cli` keeps
//! `splot-encode` as a dev-only dependency so these integration tests can prove the
//! encoder's supported output against the decoder without a production CLI edge.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::temp_path;

use std::process::Command;

#[track_caller]
fn decode_raw_64x64(ivf: &[u8], case: &str) -> Vec<u8> {
    let input = temp_path(case, "ivf");
    let output = temp_path(case, "raw");
    std::fs::write(&input, ivf).expect("write the test IVF");

    let status = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args([
            "decode",
            input.to_str().expect("utf-8 input path"),
            "--output",
            output.to_str().expect("utf-8 output path"),
            "--output-format",
            "raw",
        ])
        .status()
        .expect("run the splot binary");

    let raw = std::fs::read(&output);
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);

    assert!(status.success(), "splot decode failed for {case}");
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    raw
}

/// The minimal DC skip frame reconstructs to the AV2 § 7.13.2 no-neighbour
/// prediction: 128 on every 8-bit plane.
#[test]
fn encoder_skip_ivf_decodes_to_a_flat_128_frame() {
    let ivf = splot_encode::emit_minimal_intra_skip_ivf().expect("emit the minimal skip IVF");
    let raw = decode_raw_64x64(&ivf, "skip");
    assert!(
        raw.iter().all(|&sample| sample == 128),
        "expected a flat 128 frame from the skip block",
    );
}

/// A negative luma DC coefficient reconstructs luma to 127; skipped chroma stays 128.
#[test]
fn encoder_coded_dc_ivf_decodes_to_a_flat_127_luma_frame() {
    let ivf = splot_encode::emit_minimal_intra_coded_dc_ivf().expect("emit the coded DC IVF");
    let raw = decode_raw_64x64(&ivf, "coded-dc");
    let (luma, chroma) = raw.split_at(4096);
    assert!(
        luma.iter().all(|&sample| sample == 127),
        "expected a flat 127 luma plane from the coded DC",
    );
    assert!(
        chroma.iter().all(|&sample| sample == 128),
        "expected flat 128 chroma from the skipped chroma planes",
    );
}

/// A negative U DC coefficient reconstructs U to 127; skipped luma and V stay 128.
#[test]
fn encoder_coded_chroma_ivf_decodes_to_a_flat_127_u_frame() {
    let ivf =
        splot_encode::emit_minimal_intra_coded_chroma_ivf().expect("emit the coded chroma IVF");
    let raw = decode_raw_64x64(&ivf, "coded-chroma-u");
    assert!(
        raw[..4096].iter().all(|&s| s == 128),
        "expected a flat 128 luma plane (skipped)",
    );
    assert!(
        raw[4096..5120].iter().all(|&s| s == 127),
        "expected a flat 127 U plane from the coded chroma DC",
    );
    assert!(
        raw[5120..6144].iter().all(|&s| s == 128),
        "expected a flat 128 V plane (skipped)",
    );
}

/// A negative V DC coefficient reconstructs V to 127; skipped luma and U stay 128.
#[test]
fn encoder_coded_chroma_v_ivf_decodes_to_a_flat_127_v_frame() {
    let ivf =
        splot_encode::emit_minimal_intra_coded_chroma_v_ivf().expect("emit the coded chroma V IVF");
    let raw = decode_raw_64x64(&ivf, "coded-chroma-v");
    assert!(
        raw[..4096].iter().all(|&s| s == 128),
        "expected a flat 128 luma plane (skipped)",
    );
    assert!(
        raw[4096..5120].iter().all(|&s| s == 128),
        "expected a flat 128 U plane (skipped)",
    );
    assert!(
        raw[5120..6144].iter().all(|&s| s == 127),
        "expected a flat 127 V plane from the coded chroma DC",
    );
}

/// All three coded DC planes reconstruct to 127, exercising V txb_skip with EobU != 0.
#[test]
fn encoder_all_planes_coded_ivf_decodes_to_a_flat_127_frame() {
    let ivf =
        splot_encode::emit_minimal_intra_all_planes_coded_ivf().expect("emit the all-planes IVF");
    let raw = decode_raw_64x64(&ivf, "all-planes");
    assert!(
        raw.iter().all(|&s| s == 127),
        "expected every plane flat at 127 from the all-planes coded block",
    );
}

/// An eob=2 level-1 AC is sub-visible and reconstructs to 128. Successful decode
/// checks its coeff_base stream through the AV2 § 8.2.4 exit symbol.
#[test]
fn encoder_two_coeff_ivf_decodes_successfully() {
    let ivf = splot_encode::emit_minimal_intra_two_coeff_ivf().expect("emit the eob=2 IVF");
    let raw = decode_raw_64x64(&ivf, "two-coeff");
    assert!(
        raw.iter().all(|&s| s == 128),
        "expected the sub-visible level-1 AC to reconstruct flat at 128",
    );
}

/// A level-4 AC at scan index 1 survives rounding: luma rows form bands of
/// 129 (top 8), 128 (middle 48), and 127 (bottom 8); chroma stays 128.
#[test]
fn encoder_visible_ac_ivf_decodes_to_a_vertical_cosine_luma() {
    let ivf = splot_encode::emit_minimal_intra_visible_ac_ivf().expect("emit the visible-AC IVF");
    let raw = decode_raw_64x64(&ivf, "visible-ac");

    let luma = &raw[..4096];
    for row in 0..64 {
        let r = &luma[row * 64..(row + 1) * 64];
        assert!(
            r.iter().all(|&s| s == r[0]),
            "luma row {row} is not constant across columns"
        );
    }
    let row_value = |row: usize| luma[row * 64];
    for row in 0..8 {
        assert_eq!(row_value(row), 129, "top band row {row}");
    }
    for row in 8..56 {
        assert_eq!(row_value(row), 128, "middle band row {row}");
    }
    for row in 56..64 {
        assert_eq!(row_value(row), 127, "bottom band row {row}");
    }
    assert!(
        luma.iter().any(|&s| s != 128),
        "expected a visibly non-flat luma plane"
    );
    assert!(
        raw[4096..].iter().all(|&s| s == 128),
        "expected flat 128 chroma"
    );
}

/// A level-4 AC and negative level-1 DC exercise the AV2 § 5.20.7.27 reverse
/// sign pass: AC bypass then DC CDF. The sign-sensitive result has 50 luma
/// rows at 128 and 14 at 127; chroma stays 128.
#[test]
fn encoder_two_nonzero_ivf_decodes_to_a_cosine_plus_dc_offset() {
    let ivf = splot_encode::emit_minimal_intra_two_nonzero_ivf().expect("emit the two-nonzero IVF");
    let raw = decode_raw_64x64(&ivf, "two-nonzero");

    let luma = &raw[..4096];
    for row in 0..64 {
        let r = &luma[row * 64..(row + 1) * 64];
        assert!(
            r.iter().all(|&s| s == r[0]),
            "luma row {row} is not constant across columns"
        );
    }
    let row_value = |row: usize| luma[row * 64];
    for row in 0..50 {
        assert_eq!(row_value(row), 128, "top band row {row}");
    }
    for row in 50..64 {
        assert_eq!(row_value(row), 127, "bottom band row {row}");
    }
    assert!(
        luma.iter().any(|&s| s != 128),
        "expected a non-flat luma plane"
    );
    assert!(
        raw[4096..].iter().all(|&s| s == 128),
        "expected flat 128 chroma"
    );
}

/// A level-4 AC at scan index 2 exercises eob_extra (eob_pt_1024=2, extra=0).
/// Luma columns form bands of 129 (left 8), 128 (middle 48), and 127 (right 8).
#[test]
fn encoder_eob3_ivf_decodes_to_a_horizontal_cosine_luma() {
    let ivf = splot_encode::emit_minimal_intra_eob3_ivf().expect("emit the eob=3 IVF");
    let raw = decode_raw_64x64(&ivf, "eob3");

    let luma = &raw[..4096];
    for col in 0..64 {
        let first = luma[col];
        assert!(
            (0..64).all(|row| luma[row * 64 + col] == first),
            "luma column {col} is not constant down its rows"
        );
    }
    let col_value = |col: usize| luma[col];
    for col in 0..8 {
        assert_eq!(col_value(col), 129, "left band column {col}");
    }
    for col in 8..56 {
        assert_eq!(col_value(col), 128, "middle band column {col}");
    }
    for col in 56..64 {
        assert_eq!(col_value(col), 127, "right band column {col}");
    }
    assert!(
        luma.iter().any(|&s| s != 128),
        "expected a non-flat luma plane"
    );
    assert!(
        raw[4096..].iter().all(|&s| s == 128),
        "expected flat 128 chroma"
    );
}

/// Opposite-sign level-4 horizontal and vertical ACs test reverse sign-bit order.
/// A reversed order mirrors the expected diagonal gradient; chroma stays 128.
#[test]
fn encoder_2d_ivf_decodes_to_a_diagonal_gradient_luma() {
    let ivf = splot_encode::emit_minimal_intra_2d_ivf().expect("emit the 2-D IVF");
    let raw = decode_raw_64x64(&ivf, "2d");

    let luma = &raw[..4096];
    let px = |row: usize, col: usize| luma[row * 64 + col];
    let any_row_varies = (0..64).any(|r| (0..64).any(|c| px(r, c) != px(r, 0)));
    let any_col_varies = (0..64).any(|c| (0..64).any(|r| px(r, c) != px(0, c)));
    assert!(
        any_row_varies && any_col_varies,
        "expected a genuinely 2-D (non-separable) luma"
    );
    let expected = [[128u8, 127, 127], [129, 128, 127], [129, 129, 128]];
    let rows = [4usize, 32, 60];
    let cols = [4usize, 32, 60];
    for (ri, &r) in rows.iter().enumerate() {
        for (ci, &c) in cols.iter().enumerate() {
            assert_eq!(px(r, c), expected[ri][ci], "2-D band ({r},{c})");
        }
    }
    assert!(
        raw[4096..].iter().all(|&s| s == 128),
        "expected flat 128 chroma"
    );
}

/// The public Context API emits a packet whose decoded samples equal the input.
#[test]
fn context_encodes_all_128_frame_to_a_packet_that_decodes_to_all_128() {
    assert_all_128_roundtrips_at_qp(splot_encode::DEFAULT_QP);
}

/// A non-default qp=40 reaches base_q_idx while zero residual still decodes to 128.
#[test]
fn context_encodes_all_128_frame_at_non_default_qp_that_decodes_to_all_128() {
    assert_all_128_roundtrips_at_qp(40);
}

/// Encodes an all-128 64x64 frame through the public Context at the given fixed `qp`, muxes the
/// packet into an IVF, decodes it with the `splot` binary, and asserts the reconstruction is the
/// all-128 input. Shared by the default-qp and non-default-qp oracles.
fn assert_all_128_roundtrips_at_qp(qp: u8) {
    use splot_encode::{
        Context, EncoderConfig, EncoderRuntimeConfig, FlushStatus, Frame, FrameId, FrameInfo,
        FramePlaneInput, FramePlanesInput, PlaneRect, PlaneSize, ReceivePacketStatus,
        SendFrameStatus,
    };

    let y = vec![128_u8; 64 * 64];
    let u = vec![128_u8; 32 * 32];
    let v = vec![128_u8; 32 * 32];
    let info = FrameInfo::yuv420_8bit(FrameId::new(0), PlaneSize::new(64, 64).unwrap());
    let frame = Frame::from_planes(
        info,
        FramePlanesInput::yuv(
            FramePlaneInput::new(&y, 64, PlaneRect::new(0, 0, 64, 64).unwrap()),
            FramePlaneInput::new(&u, 32, PlaneRect::new(0, 0, 32, 32).unwrap()),
            FramePlaneInput::new(&v, 32, PlaneRect::new(0, 0, 32, 32).unwrap()),
        ),
    )
    .expect("build the all-128 input frame");

    let mut config = EncoderConfig::new(64, 64);
    config.qp = qp;
    let mut ctx =
        Context::new(config, EncoderRuntimeConfig::default()).expect("build the encoder context");
    assert!(matches!(
        ctx.send_frame(&frame).expect("send_frame"),
        SendFrameStatus::Accepted { .. }
    ));
    assert!(matches!(
        ctx.flush().expect("flush"),
        FlushStatus::Draining { .. }
    ));
    let packet_data = match ctx.receive_packet().expect("receive_packet") {
        ReceivePacketStatus::Packet(packet) => Ok(packet.data),
        status => Err(status),
    }
    .expect("expected a packet from the public Context");
    assert!(!packet_data.is_empty(), "the packet must carry coded bytes");

    let mut ivf = Vec::new();
    splot_core::ivf::write_ivf_header(
        &mut ivf,
        &splot_core::ivf::IvfHeader::new(*b"AV02", 64, 64, 30, 1, 1),
    )
    .expect("write the IVF header");
    splot_core::ivf::write_ivf_frame(&mut ivf, 0, &packet_data).expect("mux the packet into IVF");

    let raw = decode_raw_64x64(&ivf, "context");
    assert!(
        raw.iter().all(|&s| s == 128),
        "the decoded frame must equal the all-128 encoder input",
    );
}
