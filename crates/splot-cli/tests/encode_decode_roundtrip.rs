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
    assert!(status.success(), "splot decode of the emitted IVF failed");

    let raw = std::fs::read(&output).expect("read the decoded raw output");
    // 8-bit 4:2:0 64x64: Y (64*64) + U (32*32) + V (32*32) = 6144 bytes.
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // The skip block reconstructs to the flat no-neighbour DC predictor.
    assert!(
        raw.iter().all(|&sample| sample == 128),
        "expected a flat 128 frame from the skip block",
    );

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
}
