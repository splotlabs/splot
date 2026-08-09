// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-crate end-to-end oracle: `splot-encode` emits a decodable minimal intra skip
//! frame, and `splot decode` reconstructs it to a flat frame. `splot-cli` keeps
//! `splot-encode` as a dev-only dependency so these integration tests can prove the
//! encoder's supported output against the decoder without a production CLI edge.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_path(stem: &str, extension: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splot-enc-dec-roundtrip-{stem}-{}-{nanos}-{id}.{extension}",
        std::process::id()
    ))
}

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

/// The encoder's first end-to-end decodable output: `splot-encode` emits a 64x64 all-intra
/// `OBU_CLOSED_LOOP_KEY` DC skip frame, and `splot decode --output-format raw` reconstructs
/// it. The block is skipped (all-zero residual), so every luma and chroma sample is the
/// § 7.13.2 DC prediction of a no-neighbour block — `128` for 8-bit — giving a flat frame.
#[test]
fn encoder_skip_ivf_decodes_to_a_flat_128_frame() {
    let ivf = splot_encode::emit_minimal_intra_skip_ivf().expect("emit the minimal skip IVF");
    let raw = decode_raw_64x64(&ivf, "skip");
    assert!(
        raw.iter().all(|&sample| sample == 128),
        "expected a flat 128 frame from the skip block",
    );
}

/// The encoder's first decodable output carrying a **coded** coefficient: `splot-encode`
/// emits a 64x64 all-intra frame whose luma block has a single negative DC coefficient
/// (U and V skipped), and `splot decode` reconstructs it. The dequantized residual is added
/// to the `128` predictor, so the luma plane is flat `127` while the skipped chroma stays
/// flat `128` — proving the encoder emits real residual the decoder reconstructs, not just
/// prediction.
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

/// The encoder's first decodable output carrying a coded **chroma** coefficient: the U block
/// has a single negative DC coefficient (luma and V skipped). `splot decode` reconstructs a
/// flat luma plane of 128 (skipped), a flat U plane of 127 (the dequantized chroma residual),
/// and a flat V plane of 128 (skipped) — proving the encoder emits chroma residual the decoder
/// reconstructs, isolated from luma.
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

/// The V-plane counterpart of the coded-chroma oracle: the V block carries a single negative
/// DC coefficient (luma and U skipped). `splot decode` reconstructs flat luma 128, flat U 128,
/// and flat V 127. With the U and V oracles this proves coded residual on every plane.
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

/// The encoder's first frame with all three planes coded at once: luma, U, and V each carry a
/// single negative coded DC coefficient (mirroring the q80 fixture's structure). `splot decode`
/// reconstructs every plane below 128. With the per-plane oracles this proves the encoder can
/// code all planes simultaneously, including the V `txb_skip` `EobU != 0` interaction.
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

/// The encoder's first multi-coefficient (`eob > 1`) frame: the luma block carries a single
/// nonzero AC coefficient (eob=2, U and V skipped). `splot decode` validates the eob=2 entropy
/// stream (the §8.2.4 exit_symbol check passes only if the AC `coeff_base` symbols are
/// consistent). The level-1 AC reconstructs to a sub-visible residual, so the frame is flat
/// 128; a visibly-non-flat AC (larger magnitude, needing the per-level DC `coeff_base` context)
/// is a follow-up. This proves the encoder emits a decodable eob>1 block, distinct from skip.
#[test]
fn encoder_two_coeff_ivf_decodes_successfully() {
    let ivf = splot_encode::emit_minimal_intra_two_coeff_ivf().expect("emit the eob=2 IVF");
    let raw = decode_raw_64x64(&ivf, "two-coeff");
    assert!(
        raw.iter().all(|&s| s == 128),
        "expected the sub-visible level-1 AC to reconstruct flat at 128",
    );
}

/// The encoder's first frame where a coefficient **visibly shapes the reconstruction**. The
/// luma block carries a single nonzero level-4 AC coefficient at scan index 1 (eob=2, U and V
/// skipped). Unlike the sub-visible level-1 AC (which reconstructs flat 128), the level-4 AC
/// dequantizes to a residual that survives rounding, producing a vertical low-frequency cosine:
/// each row is constant across columns, the top 8 rows are 129, the middle 48 are 128, and the
/// bottom 8 are 127. `splot decode` reconstructs it bit-exactly through the entropy + inverse
/// transform path.
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

/// The encoder's first block with **two nonzero coefficients**: a positive level-4 AC at scan
/// index 1 and a **negative** level-1 DC at scan index 0 (eob=2, U and V skipped). This exercises
/// the DC `coeff_base` nonzero path and the AV2 §5.20.7.27 reverse-scan sign pass (`c = eob-1 ..
/// 0`): the AC `sign_bit` bypass (c=1) then the DC `dc_sign` CDF (c=0). The negative DC makes the
/// reconstruction sign-order-sensitive — the wrong order would brighten rather than darken the
/// lower band — so this oracle genuinely proves the ordering. `splot decode` reconstructs the
/// cosine superimposed on the negative DC offset: each luma row constant, the top 50 rows 128 and
/// the bottom 14 rows 127.
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

/// The encoder's first frame with **eob > 2**: the luma block has eob=3 with a single nonzero
/// level-4 AC at scan index 2 (raster 1, the horizontal frequency-1 position), U and V skipped.
/// This exercises the `eob_extra` CDF symbol (`eob_pt_1024 == 2`, `eob_extra == 0` -> eob 3) — the
/// gateway to arbitrary-length blocks. `splot decode` reconstructs a horizontal low-frequency
/// cosine (the transpose of the visible-AC vertical cosine): each column is constant, the left 8
/// columns are 129, the middle 48 are 128, and the right 8 are 127.
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

/// The encoder's first **2-D reconstruction**: eob=3 with two nonzero level-4 ACs — a positive
/// horizontal AC at scan index 2 and a negative vertical AC at scan index 1 (U and V skipped).
/// This is the first block whose decode varies in BOTH dimensions (neither rows nor columns are
/// constant) — the horizontal and vertical low-frequency cosines superimposed with opposite
/// signs, giving a diagonal gradient. The asymmetric AC signs also prove the reverse-scan
/// two-`sign_bit` order: a wrong order would mirror the diagonal. `splot decode` reconstructs the
/// 3x3 band grid (rows/cols sampled at 4/32/60): [[128,127,127],[129,128,127],[129,129,128]].
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

/// Milestone A keystone: the encoder produces a real packet through the **public Context API**,
/// and that packet decodes back to the input. An all-128 64x64 frame has zero residual against
/// the §7.13.2 no-neighbour DC predictor, so `Context::send_frame` + `receive_packet` emit the
/// skip frame, and `splot decode` reconstructs it bit-exactly: decode(encode(input)) == input.
/// This proves the lifecycle wiring (not just the bare emitter), and the honesty invariant —
/// the output decodes to the input, never a canned frame.
#[test]
fn context_encodes_all_128_frame_to_a_packet_that_decodes_to_all_128() {
    assert_all_128_roundtrips_at_qp(splot_encode::DEFAULT_QP);
}

/// The `EncoderConfig::qp` field is the single fixed-quantizer source: it threads into the
/// frame-header `base_q_idx`, so a non-default `qp` produces a different (still decodable) stream.
/// A skip frame's all-zero residual reconstructs flat-128 independent of `base_q_idx`, so this
/// proves the qp threading decodes bit-exactly at a non-default quantizer (40, in the supported
/// q-context-0 range) — not just at the default 80.
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
