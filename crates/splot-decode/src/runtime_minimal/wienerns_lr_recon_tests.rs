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
//!   * Chroma: the frame-origin `DC_PRED` 32x32 U and V transforms (the §5.20.3.1
//!     SDP chroma tree at chroma `(0,0)`) — both flat `512`, the 10-bit no-neighbour
//!     DC fallback — are bit-exact (2048 chroma samples). The U/V origin reads only
//!     its own off-frame edges (chroma `DC_PRED`, not CfL), so it reconstructs
//!     independently of the deferred luma DC chain to its right.
//!   * Everything the primitive cannot prove bit-exact is DEFERRED: NON-DC luma
//!     (SMOOTH / directional, e.g. the H_PRED band at luma mi-col 48); NON-DC chroma
//!     (the SMOOTH chroma leaf at chroma `(32,0)`); a non-`all_zero` NON-SQUARE
//!     `DC_PRED` leaf (the frame-origin `TX_16X64` keystone — its rectangular-residual
//!     inverse transform reconstructs a flat block but the oracle has a `[64,65]`
//!     spatial AC residual, so it is the verified wall for the whole first-3-SB luma
//!     DC region); any IST / FSC leaf; a frame with a non-zero quantizer delta or
//!     matrix; and any block whose §7.13.2 prediction edges border a deferred
//!     (un-reconstructed) neighbour. The sink never claims a sample it has not proven
//!     bit-exact.
//!
//! Parse fidelity (verified against the AVM mode/uv_mode oracle, 2026-06-27 after
//! PR #510): every luma leaf splot resolves in the reachable region now agrees with
//! AVM's `inspect --mode` (the frame-origin DC blocks AND the mi-col-48 H_PRED band
//! all match), and the chroma origin resolves to `DC_PRED` matching `inspect
//! --uv_mode`. The earlier SMOOTH-vs-AVM disagreement was a parser desync fixed by
//! PR #508/#510; the reconstruction wall is now the non-square `TX_16X64` AC residual
//! (a reconstruction-primitive limitation), not a parse divergence. The luma H_PRED
//! band and the rest of the DC chain stay DEFERRED only because every block past the
//! origin column descends, through the §7.13.2 edge-coverage guard, from the
//! still-deferred non-square keystone — not because their modes are wrong.
//!
//! The oracle YUV is 6 MB and is NOT committed; the committed assertions are the
//! frame-origin luma column flat values (`68` / `64`), the chroma origin flat value
//! (`512`), their sample sums, and FNV-1a-64 checksums. The PUBLIC decode stays
//! fail-closed; these tests exercise
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

/// The frame-origin chroma `DC_PRED` transform side (a 32x32 §5.20.6 `TxSize` in
/// the 4:2:0 chroma plane, the chroma leaf covering the §5.20.3.1 SDP chroma tree
/// at the frame origin). Both U and V resolve to chroma `DC_PRED` with no neighbour
/// (the §7.13.2.1 no-neighbour fallback) and an `all_zero` residual, so each plane
/// is the flat 10-bit DC fallback `1 << (10 - 1)` == `512`.
const CHROMA_ORIGIN_SIDE: usize = 32;
/// Flat oracle value for the frame-origin chroma `DC_PRED` block (10-bit
/// no-neighbour DC fallback), derived offline from `ac0_prefiltered.yuv` (its first
/// 32x32 U and V samples are both uniformly `512`).
const CHROMA_ORIGIN_FLAT: u16 = 512;
const CHROMA_ORIGIN_SAMPLE_COUNT: usize = CHROMA_ORIGIN_SIDE * CHROMA_ORIGIN_SIDE;
/// Sum of one 32x32 chroma origin plane (`1024 * 512`).
const CHROMA_ORIGIN_SAMPLE_SUM: u64 = 524_288;
/// FNV-1a-64 over one 32x32 chroma origin plane (row-major, sample-major u16 LE),
/// matching the offline oracle checksum derivation (identical for U and V).
const CHROMA_ORIGIN_FNV1A64: u64 = 0xa53e_893c_24f1_e325;

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
    assert!(
        path.is_file(),
        "local ac0ej3 fixture not found at {}",
        path.display()
    );
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

/// Bit-exact verification of the frame-origin chroma `DC_PRED` blocks (U and V)
/// against the AVM pre-filter oracle. With the AVM-faithful first-superblock parse
/// (PR #510's §8.3.2 `is_cfl` neighbour-context fix), the §5.20.3.1 SDP chroma tree
/// at the frame origin resolves to chroma `DC_PRED` with an `all_zero` residual
/// (verified against `inspect --uv_mode`), so the sink reconstructs the 32x32 U and
/// V origin transforms to the flat 10-bit no-neighbour DC fallback `512`. Both
/// planes' per-sample values, sums, and FNV-1a-64 checksums match the oracle —
/// 2048 chroma samples (1024 U + 1024 V) added to the asserted bit-exact region
/// beyond the 1024-sample luma column. The next chroma leaf (chroma `(32,0)`,
/// resolved `SMOOTH_PRED`) is DEFERRED, so it stays at the unreconstructed fill
/// value `0`; this asserts that deferral too, proving the sink never claims a chroma
/// sample it has not proven bit-exact. The U origin block reconstructs independently
/// of the deferred luma DC chain to its right (chroma `DC_PRED` reads only its own
/// off-frame edges, never the luma plane), so the wall at the non-square `TX_16X64`
/// luma keystone does not contaminate it.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_frame_origin_chroma_dc_blocks_reconstruct_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 frame-origin chroma DC blocks");

    // Each of the U and V origin planes is a flat 32x32 `512` block matching the
    // oracle, with an identical sum and FNV-1a-64 checksum.
    for plane in [PlaneId::U, PlaneId::V] {
        let mut fnv = Fnv1a64::new();
        let mut sum: u64 = 0;
        let mut count = 0usize;
        for y in 0..CHROMA_ORIGIN_SIDE {
            for x in 0..CHROMA_ORIGIN_SIDE {
                let sample = sink.reconstructed_sample(plane, x, y).unwrap();
                assert_eq!(
                    sample, CHROMA_ORIGIN_FLAT,
                    "chroma {plane:?} origin ({x},{y}) must be {CHROMA_ORIGIN_FLAT}, got {sample}"
                );
                fnv.update_u16(sample);
                sum += u64::from(sample);
                count += 1;
            }
        }
        assert_eq!(
            count, CHROMA_ORIGIN_SAMPLE_COUNT,
            "chroma {plane:?} origin block sample count"
        );
        assert_eq!(
            sum, CHROMA_ORIGIN_SAMPLE_SUM,
            "chroma {plane:?} origin block sample sum must match the pre-filter oracle"
        );
        assert_eq!(
            fnv.finish(),
            CHROMA_ORIGIN_FNV1A64,
            "chroma {plane:?} origin FNV-1a-64 must match the pre-filter oracle (bit-exact)"
        );
    }

    // The second chroma leaf (chroma `(32,0)`, resolved `SMOOTH_PRED`) is DEFERRED:
    // its samples stay at the unreconstructed fill value `0`, never a SMOOTH
    // prediction the sink has not proven bit-exact against AVM.
    assert_eq!(
        sink.reconstructed_sample(PlaneId::U, CHROMA_ORIGIN_SIDE, 0)
            .unwrap(),
        0,
        "the deferred SMOOTH chroma leaf at chroma (32,0) must stay unreconstructed"
    );

    // Coverage report: the verified region is the 1024-sample luma column plus the
    // two 32x32 chroma origin blocks (2048 chroma 4x4 units total across U and V).
    let (luma4x4, chroma4x4) = sink.reconstructed_counts();
    assert_eq!(
        luma4x4 * 16,
        LUMA_COLUMN_SAMPLE_COUNT,
        "verified luma region is the 1024-sample BLOCK_16X64 column"
    );
    assert_eq!(
        chroma4x4 * 16,
        2 * CHROMA_ORIGIN_SAMPLE_COUNT,
        "verified chroma region is the U+V 32x32 origin blocks (2048 samples)"
    );
}
