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

/// The V-plane counterpart of the coded-chroma oracle: the V block carries a single negative
/// DC coefficient (luma and U skipped). `splot decode` reconstructs flat luma 128, flat U 128,
/// and flat V 127. With the U and V oracles this proves coded residual on every plane.
#[test]
fn encoder_coded_chroma_v_ivf_decodes_to_a_flat_127_v_frame() {
    let ivf =
        splot_encode::emit_minimal_intra_coded_chroma_v_ivf().expect("emit the coded chroma V IVF");

    let input = temp_path("chroma-v-input", "ivf");
    let output = temp_path("chroma-v-output", "raw");
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
        "splot decode of the coded chroma V IVF failed"
    );
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // 8-bit 4:2:0 64x64: Y = [0..4096), U = [4096..5120), V = [5120..6144).
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

    let input = temp_path("all-input", "ivf");
    let output = temp_path("all-output", "raw");
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
        "splot decode of the all-planes IVF failed"
    );
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // Every plane carries coded residual -> flat 127 (128 minus the dequantized negative DC).
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

    let input = temp_path("two-coeff-input", "ivf");
    let output = temp_path("two-coeff-output", "raw");
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

    // exit_symbol() validates the eob=2 entropy stream; a malformed multi-coeff trace fails it.
    assert!(status.success(), "splot decode of the eob=2 IVF failed");
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");
    // The level-1 AC residual is sub-visible -> the frame reconstructs flat at 128.
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

    let input = temp_path("visible-ac-input", "ivf");
    let output = temp_path("visible-ac-output", "raw");
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
        "splot decode of the visible-AC IVF failed"
    );
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");

    // 8-bit 4:2:0 64x64: luma [0..4096), then U [4096..5120), V [5120..6144).
    let luma = &raw[..4096];
    // The level-4 AC is the lowest vertical frequency, so each row is constant across columns.
    for row in 0..64 {
        let r = &luma[row * 64..(row + 1) * 64];
        assert!(
            r.iter().all(|&s| s == r[0]),
            "luma row {row} is not constant across columns"
        );
    }
    // The vertical cosine: top 8 rows 129, middle 48 rows 128, bottom 8 rows 127.
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
    // It is genuinely non-flat (unlike every prior decodable frame).
    assert!(
        luma.iter().any(|&s| s != 128),
        "expected a visibly non-flat luma plane"
    );
    // Chroma is untouched (U and V skipped) -> flat 128.
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

    let input = temp_path("two-nonzero-input", "ivf");
    let output = temp_path("two-nonzero-output", "raw");
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
        "splot decode of the two-nonzero IVF failed"
    );
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");

    let luma = &raw[..4096];
    // The AC is the lowest vertical frequency, so each row is constant across columns.
    for row in 0..64 {
        let r = &luma[row * 64..(row + 1) * 64];
        assert!(
            r.iter().all(|&s| s == r[0]),
            "luma row {row} is not constant across columns"
        );
    }
    // Cosine + negative DC offset: top 50 rows 128, the bottom 14 rows 127 (deterministic).
    // Under the wrong (DC-first) sign order the bottom band would be 129 instead, so this
    // assertion fails fast on a sign-order regression.
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
    // Chroma is untouched (U and V skipped) -> flat 128.
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

    let input = temp_path("eob3-input", "ivf");
    let output = temp_path("eob3-output", "raw");
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

    assert!(status.success(), "splot decode of the eob=3 IVF failed");
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");

    let luma = &raw[..4096];
    // The AC is the lowest horizontal frequency, so each column is constant down its 64 rows.
    for col in 0..64 {
        let first = luma[col];
        assert!(
            (0..64).all(|row| luma[row * 64 + col] == first),
            "luma column {col} is not constant down its rows"
        );
    }
    // The horizontal cosine: left 8 columns 129, middle 48 columns 128, right 8 columns 127.
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
    // Chroma is untouched (U and V skipped) -> flat 128.
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

    let input = temp_path("twod-input", "ivf");
    let output = temp_path("twod-output", "raw");
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

    assert!(status.success(), "splot decode of the 2-D IVF failed");
    let raw = raw.expect("read the decoded raw output");
    assert_eq!(raw.len(), 6144, "unexpected decoded frame size");

    let luma = &raw[..4096];
    let px = |row: usize, col: usize| luma[row * 64 + col];
    // Genuinely 2-D: neither all rows nor all columns are constant.
    let any_row_varies = (0..64).any(|r| (0..64).any(|c| px(r, c) != px(r, 0)));
    let any_col_varies = (0..64).any(|c| (0..64).any(|r| px(r, c) != px(0, c)));
    assert!(
        any_row_varies && any_col_varies,
        "expected a genuinely 2-D (non-separable) luma"
    );
    // The 3x3 band grid (deterministic): a diagonal gradient. A wrong sign order would mirror it.
    let expected = [[128u8, 127, 127], [129, 128, 127], [129, 129, 128]];
    let rows = [4usize, 32, 60];
    let cols = [4usize, 32, 60];
    for (ri, &r) in rows.iter().enumerate() {
        for (ci, &c) in cols.iter().enumerate() {
            assert_eq!(px(r, c), expected[ri][ci], "2-D band ({r},{c})");
        }
    }
    // Chroma is untouched (U and V skipped) -> flat 128.
    assert!(
        raw[4096..].iter().all(|&s| s == 128),
        "expected flat 128 chroma"
    );
}
