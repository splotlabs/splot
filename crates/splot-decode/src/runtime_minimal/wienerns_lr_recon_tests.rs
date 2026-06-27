// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Region-verification tests for the ac0ej3 general-intra reconstruction bridge.
//!
//! Drives the real ac0ej3 mission stream through the `TX_MODE_SELECT` selectable
//! transform-record walk with a reconstruction sink attached, and verifies the
//! reconstructed NON-IntrABC DC region BIT-EXACT against the AVM pre-filter
//! reconstruction oracle (`ac0_prefiltered.yuv`,
//! md5 `f7959cb85a41dcf0e6ebf9179835da03`).
//!
//! With PR #497's per-block CCSO read the selectable walk now parses ac0ej3's
//! first superblock bit-exact vs AVM, so the bridge reconstructs the verified DC
//! subset to the spec-correct samples. The first 16x16 luma block (the §5.20.5.3
//! `DC_PRED` leaf at the frame origin) reconstructs BIT-EXACT (all `68`); the
//! committed constants below are the small oracle assertion derived offline from
//! `ac0_prefiltered.yuv`.
//!
//! Reconstructed-and-verified region for frame-0 (gated to the proven DC subset):
//!   * Luma: the frame-origin `DC_PRED` 16x16 leaf is bit-exact, and the bridge
//!     reconstructs every general-intra `DC_PRED` luma transform it reaches before
//!     the §7.13.3.18 IntrABC fail-closed rejection.
//!   * The first superblock's NON-DC luma (SMOOTH / directional) and its chroma
//!     (the first-SB chroma leaf is `SMOOTH`, not `DC`) are OUTSIDE the verified DC
//!     subset and are deliberately left UNRECONSTRUCTED — the sink never claims a
//!     sample it has not proven bit-exact.
//!
//! The oracle YUV is 6 MB and is NOT committed; the committed assertion is the
//! frame-origin 16x16 block's flat value (`68`), its sample sum, and an FNV-1a-64
//! checksum of the block. The PUBLIC decode stays fail-closed; these tests exercise
//! the bridge through a test-only sink driver, gated to the local mission fixture
//! (`SPLOT_AC0EJ3_IVF` or `$HOME/Documents/SplotLabs/ac0ej3.ivf`) and `#[ignore]`d
//! to match the existing `local_ac0ej3_*` probe convention.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use splot_parallel::ThreadCount;
use splot_recon::PlaneId;

use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};

use super::reconstruct_ac0ej3_intra_region_from_plan;

/// Frame-origin `DC_PRED` luma leaf side (a 16x16 §5.20.6 `TxSize` transform).
const BLOCK0_SIDE: usize = 16;

/// Committed oracle assertion for the frame-origin 16x16 `DC_PRED` luma block,
/// derived offline from the AVM pre-filter reconstruction `ac0_prefiltered.yuv`
/// (its first 16x16 luma samples; sample-major u16 little-endian). The block is a
/// flat `DC_PRED` leaf, so every sample is `68`.
const BLOCK0_FLAT_LUMA: u16 = 68;
const BLOCK0_SAMPLE_COUNT: usize = BLOCK0_SIDE * BLOCK0_SIDE;
const BLOCK0_SAMPLE_SUM: u64 = 17_408;
const BLOCK0_FNV1A64: u64 = 0x68b9_9236_1d60_fb25;

fn local_ac0ej3_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SPLOT_AC0EJ3_IVF") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join("Documents/SplotLabs/ac0ej3.ivf"))
}

fn context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context")
}

fn require_fixture() -> PathBuf {
    let Some(path) = local_ac0ej3_path() else {
        panic!("set SPLOT_AC0EJ3_IVF or HOME for the ignored local ac0ej3 reconstruction test");
    };
    if !path.is_file() {
        panic!("local ac0ej3 fixture not found at {}", path.display());
    }
    path
}

/// FNV-1a-64 over a u16 sample stream (little-endian bytes), matching the offline
/// oracle checksum derivation.
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn update_u16(&mut self, value: u16) {
        for byte in value.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

/// Infrastructure check: the reconstruction bridge threads a sink through the
/// selectable transform-record walk and reconstructs the verified `DC_PRED` luma
/// region into a current-frame workspace in decode order, while the public decode
/// path stays fail-closed. This proves the bridge wiring (sink threading +
/// primitive reuse) over the live ac0ej3 parse.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_reconstruction_bridge_populates_a_workspace_region() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 selectable intra region");

    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    // The bridge reconstructs every general-intra `DC_PRED` luma transform it
    // reaches before the walk's IntrABC fail-closed rejection. This pins that the
    // sink threading and primitive reuse are wired over the live parse.
    assert!(
        luma4x4 > 0,
        "the reconstruction bridge must populate luma samples"
    );
    // Every reconstructed sample is in the 10-bit range (no overflow / garbage
    // type errors from the primitive reuse): scan the frame-origin DC block.
    for y in 0..BLOCK0_SIDE {
        for x in 0..BLOCK0_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            assert!(sample < 1024, "10-bit luma sample out of range: {sample}");
        }
    }
}

/// Bit-exact verification against the AVM pre-filter reconstruction oracle for the
/// frame-origin `DC_PRED` luma block. With the now-AVM-faithful first-superblock
/// parse (PR #497 CCSO read), the bridge reconstructs this block bit-exact: every
/// sample is the committed flat value `68`, and the block's sum and FNV-1a-64
/// checksum match the oracle. This is the first BIT-EXACT ac0ej3 reconstruction
/// milestone, verified against the AVM pre-filter oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_frame_origin_dc_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 frame-origin DC luma block");

    // Every frame-origin block sample is the committed flat oracle value.
    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in 0..BLOCK0_SIDE {
        for x in 0..BLOCK0_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            assert_eq!(
                sample, BLOCK0_FLAT_LUMA,
                "frame-origin luma ({x},{y}) must be {BLOCK0_FLAT_LUMA}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(
        count, BLOCK0_SAMPLE_COUNT,
        "frame-origin block sample count"
    );
    assert_eq!(
        sum, BLOCK0_SAMPLE_SUM,
        "frame-origin block sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        BLOCK0_FNV1A64,
        "frame-origin block FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}
