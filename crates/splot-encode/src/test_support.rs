// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decoder oracles shared by the encoder tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

use splot_decode::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};
use splot_parallel::ThreadCount;
use splot_recon::BitDepth;

/// Planes as the decoder's raw output: `u8` samples at 8 bits, little-endian
/// `u16` samples otherwise.
pub(crate) fn raw_bytes<'a>(
    rows: impl IntoIterator<Item = &'a [u16]>,
    bit_depth: BitDepth,
) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        for &sample in row {
            match bit_depth {
                BitDepth::Eight => out.push(sample as u8),
                BitDepth::Ten => out.extend_from_slice(&sample.to_le_bytes()),
            }
        }
    }
    out
}

/// The raw splot-decode output of an Annex B stream.
pub(crate) fn decode(annexb: &[u8]) -> Vec<u8> {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1))).unwrap();
    let mut out = Vec::new();
    ctx.decode_raw_bytes(annexb, DecodeOptions::default(), &mut out)
        .unwrap();
    out
}

/// Asserts that `decoded` equals `expected` and names the first mismatch.
pub(crate) fn assert_same_samples(decoded: &[u8], expected: &[u8], case: &str) {
    assert_eq!(decoded.len(), expected.len(), "{case}: output length");
    if let Some(at) = decoded.iter().zip(expected).position(|(a, b)| a != b) {
        panic!("{case}: first sample mismatch at byte {at}");
    }
}

/// The AVM decoder named by `SPLOT_AVM_DECODER`, if set.
pub(crate) fn avm_decoder() -> Option<PathBuf> {
    let path = std::env::var_os("SPLOT_AVM_DECODER").map(PathBuf::from);
    if path.is_none() {
        eprintln!("skip: SPLOT_AVM_DECODER is not set");
    }
    path
}

/// Decodes the Annex B temporal units `units` of a `width x height` stream
/// with AVM and returns its raw output.
pub(crate) fn avm_decode(
    avmdec: &PathBuf,
    units: &[Vec<u8>],
    (width, height): (u32, u32),
    bit_depth: BitDepth,
    name: &str,
) -> Vec<u8> {
    let work = std::env::temp_dir().join(format!("splot-encode-avm-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let mut ivf = Vec::new();
    splot_core::ivf::write_ivf_header(
        &mut ivf,
        &splot_core::ivf::IvfHeader::new(
            *b"AV02",
            width as u16,
            height as u16,
            30,
            1,
            units.len() as u32,
        ),
    )
    .unwrap();
    for (pts, unit) in units.iter().enumerate() {
        splot_core::ivf::write_ivf_frame(&mut ivf, pts as u64, unit).unwrap();
    }
    let input = work.join(format!("{name}.ivf"));
    let output = work.join(format!("{name}.yuv"));
    std::fs::write(&input, &ivf).unwrap();
    let status = Command::new(avmdec)
        .arg(&input)
        .arg("--rawvideo")
        .arg(format!("--output-bit-depth={}", bit_depth.bits()))
        .arg("-o")
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "avmdec failed on {name}");
    std::fs::read(&output).unwrap()
}
