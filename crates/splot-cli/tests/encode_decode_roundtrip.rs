// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-crate end-to-end oracle: `splot-encode` emits a decodable minimal intra skip
//! frame, and `splot decode` reconstructs it to a flat frame. `splot-cli` is the only crate
//! that depends on both `splot-encode` and `splot-decode`, so this is where the encoder's
//! first decodable output is proven against the decoder.

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

/// The encoder's first end-to-end decodable output: `splot-encode` emits a 64x64 all-intra
/// `OBU_CLOSED_LOOP_KEY` DC skip frame, and `splot decode --output-format raw` reconstructs
/// it. The block is skipped (all-zero residual), so every luma and chroma sample is the
/// § 7.13.2 DC prediction of a no-neighbour block — `128` for 8-bit — giving a flat frame.
#[test]
fn encoder_skip_ivf_decodes_to_a_flat_128_frame() {
    let ivf = splot_encode::emit_minimal_intra_skip_ivf().expect("emit the minimal skip IVF");

    let input = temp_path("input", "ivf");
    let output = temp_path("output", "raw");
    std::fs::write(&input, &ivf).expect("write the emitted IVF");

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

    // Capture results, then clean the temp files up front so a failing assertion below
    // never leaks them.
    let raw = std::fs::read(&output);
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);

    assert!(status.success(), "splot decode of the emitted IVF failed");
    let raw = raw.expect("read the decoded raw output");
    // 8-bit 4:2:0 64x64: Y (64*64) + U (32*32) + V (32*32) = 6144 bytes.
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // The skip block reconstructs to the flat no-neighbour DC predictor.
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

    let input = temp_path("coded-input", "ivf");
    let output = temp_path("coded-output", "raw");
    std::fs::write(&input, &ivf).expect("write the emitted IVF");

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

    assert!(status.success(), "splot decode of the coded IVF failed");
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // Luma (first 4096 bytes) carries the coded residual; chroma (next 2048) is skipped.
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

    let input = temp_path("chroma-input", "ivf");
    let output = temp_path("chroma-output", "raw");
    std::fs::write(&input, &ivf).expect("write the emitted IVF");

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

    assert!(
        status.success(),
        "splot decode of the coded chroma IVF failed"
    );
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // 8-bit 4:2:0 64x64: Y = [0..4096), U = [4096..5120), V = [5120..6144).
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
