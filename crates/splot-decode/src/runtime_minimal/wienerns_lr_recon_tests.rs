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
//!   * Luma: the frame-origin `BLOCK_16X64` left column — the `DC_PRED` 16x16 leaf
//!     at the frame origin (all `68`) plus the down-predicted `DC_PRED` 16-wide
//!     transforms below it (rows 16..64, all `64`) — is bit-exact: the full 16x64
//!     (1024-sample) column matches the oracle (whole reconstructed region strictly
//!     bit-exact, no confident-wrong workspace samples).
//!   * Everything the primitive cannot prove bit-exact is DEFERRED: NON-DC luma
//!     (SMOOTH / directional); the first-SB chroma leaf; a non-`all_zero`
//!     NON-SQUARE `DC_PRED` leaf (the `TX_16X64` rectangular-residual inverse
//!     transform is not yet proven); any IST / FSC leaf; a frame with a non-zero
//!     quantizer delta or matrix; and any `DC_PRED` block whose §7.13.2 prediction
//!     edges border a deferred (un-reconstructed) neighbour. The sink never claims
//!     a sample it has not proven bit-exact.
//!
//! SMOOTH deferral (verified against the AVM mode/predicted oracle, 2026-06-27):
//! the §7.13.2.13 SMOOTH primitive (`predict_intra_smooth_rect_into`) and its
//! square recon wrappers exist and are bit-exact for synthetic SMOOTH fixtures,
//! but NO SMOOTH leaf in the ac0ej3 reachable region (the blocks decoded before the
//! walk's IntrABC fail-closed stop) can be admitted bit-exact: splot's parse
//! diverges from AVM immediately past this first `BLOCK_16X64` luma column, so every
//! leaf splot resolves as SMOOTH (luma `SMOOTH_PRED`/`SMOOTH_V_PRED` and chroma
//! `SMOOTH_PRED`, the first being the SB0 chroma leaf at chroma `(0,0)`) is a mode
//! that AVM resolves as `DC_PRED` / `H_PRED` / `UV_CFL_PRED` instead (0 of the 11
//! reachable splot-SMOOTH leaves agree with AVM's `inspect --mode`/`--uv_mode`).
//! AVM's prediction-only buffer for the SB0 chroma block is flat `512` (no-neighbour
//! `DC`); the SMOOTH primitive over the §7.13.2.1 no-neighbour fallback edges
//! (above `511`, left `513`) produces a `511..513` gradient instead, so admitting
//! SMOOTH here would write confidently-wrong samples. Per the verified-subset
//! discipline the sink DEFERS all SMOOTH until the upstream chroma/luma mode
//! resolution is reconciled with AVM; widening here requires a parser fix, not a
//! reconstruction change.
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

/// The full `BLOCK_16X64` left luma column the sink reconstructs (one 16-wide
/// transform column, `16` samples across and the full superblock height of `64`
/// down). The §5.20.5.3 `DC_PRED` origin leaf is the flat `68` block; the
/// down-predicted `DC_PRED` transforms below it are the flat oracle value `64`.
const LUMA_COLUMN_WIDTH: usize = 16;
const LUMA_COLUMN_HEIGHT: usize = 64;
/// Oracle value for the `DC_PRED` transforms below the origin leaf (rows 16..64),
/// derived offline from `ac0_prefiltered.yuv`.
const LUMA_COLUMN_BELOW_ORIGIN: u16 = 64;
const LUMA_COLUMN_SAMPLE_COUNT: usize = LUMA_COLUMN_WIDTH * LUMA_COLUMN_HEIGHT;
/// Sum of the full 16x64 column (`256 * 68 + 768 * 64`).
const LUMA_COLUMN_SAMPLE_SUM: u64 = 66_560;
/// FNV-1a-64 over the full 16x64 column (row-major, sample-major u16 LE), matching
/// the offline oracle checksum derivation.
const LUMA_COLUMN_FNV1A64: u64 = 0x893d_3114_b40a_7325;

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

/// Bit-exact verification of the FULL `BLOCK_16X64` left luma column against the
/// AVM pre-filter oracle. The sink reconstructs not only the frame-origin `DC_PRED`
/// 16x16 leaf (flat `68`) but the down-predicted `DC_PRED` 16-wide transforms below
/// it (rows 16..64, flat `64`), so the whole 16x64 (1024-sample) column is verified
/// — a 4x widening of the asserted bit-exact region beyond the origin 16x16 block.
/// Both the per-sample values and the column's sum + FNV-1a-64 checksum match the
/// oracle. (SMOOTH leaves past this column are DEFERRED, not widened here — see the
/// module doc comment: splot's parse diverges from AVM immediately past this column,
/// so every reachable splot-SMOOTH leaf disagrees with the AVM mode oracle.)
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_block_16x64_luma_column_reconstructs_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 BLOCK_16X64 luma column");

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in 0..LUMA_COLUMN_HEIGHT {
        for x in 0..LUMA_COLUMN_WIDTH {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            // The origin 16x16 leaf is `68`; everything below it is the
            // down-predicted `DC_PRED` flat oracle value `64`.
            let expected = if y < BLOCK0_SIDE {
                BLOCK0_FLAT_LUMA
            } else {
                LUMA_COLUMN_BELOW_ORIGIN
            };
            assert_eq!(
                sample, expected,
                "BLOCK_16X64 luma ({x},{y}) must be {expected}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(
        count, LUMA_COLUMN_SAMPLE_COUNT,
        "BLOCK_16X64 column sample count"
    );
    assert_eq!(
        sum, LUMA_COLUMN_SAMPLE_SUM,
        "BLOCK_16X64 column sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        LUMA_COLUMN_FNV1A64,
        "BLOCK_16X64 column FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}
