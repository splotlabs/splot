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
//!   * Luma: the full first-3-superblock DC region — the rectangle x[0,192) x
//!     y[0,128), 24576 samples — is bit-exact. Fixing the MI(4,0) `TX_16X64`
//!     keystone (the §7.13.2.12 IBP DC modifier plus the non-square `TX_16X64`
//!     residual) unblocked the whole DC chain that bordered it through the §7.13.2
//!     edge-coverage guard, widening the region 24x from the original 1024-sample
//!     `BLOCK_16X64` column. Every sample is the down-predicted flat `DC_PRED`
//!     value `64` except the origin 16x16 leaf (`68`, 256 samples) and the MI(4,0)
//!     IBP DC step (`65`, the top-left 3 columns x 16 rows == 48 samples). Whole
//!     region strictly bit-exact, no confident-wrong workspace samples.
//!   * Chroma: the frame-origin `DC_PRED` 32x32 U and V transforms (the §5.20.3.1
//!     SDP chroma tree at chroma `(0,0)`) — both flat `512`, the 10-bit no-neighbour
//!     DC fallback — are bit-exact (2048 chroma samples). The U/V origin reads only
//!     its own off-frame edges (chroma `DC_PRED`, not CfL).
//!   * Everything the primitive cannot prove bit-exact is DEFERRED: NON-DC luma
//!     (SMOOTH / directional); NON-DC chroma (the SMOOTH chroma leaf at chroma
//!     `(32,0)`); any IST / FSC leaf; a frame with a non-zero quantizer delta or
//!     matrix; and any block whose §7.13.2 prediction edges border a deferred
//!     (un-reconstructed) neighbour. The non-square (`TX_16X64`) `DC_PRED` residual
//!     and the §7.13.2.12 IBP DC modifier are now MODELLED and proven bit-exact at
//!     the MI(4,0) keystone, so they no longer wall the DC chain. The sink never
//!     claims a sample it has not proven bit-exact.
//!
//! Parse fidelity (verified against the AVM mode/uv_mode oracle, 2026-06-27 after
//! PR #510): every luma leaf splot resolves in the reachable region agrees with
//! AVM's `inspect --mode`, and the chroma origin resolves to `DC_PRED` matching
//! `inspect --uv_mode`. The reachable region is now bounded by the §7.13.3.18
//! IntrABC fail-closed stop, not by a reconstruction-primitive wall.
//!
//! The oracle YUV is 6 MB and is NOT committed; the committed assertions are the
//! region flat values (`68` / `65` / `64` luma, `512` chroma), their sample sums,
//! and FNV-1a-64 checksums. The PUBLIC decode stays fail-closed; these tests
//! exercise the bridge through a test-only sink driver, gated to the local mission
//! fixture (`SPLOT_AC0EJ3_IVF` or `$HOME/Documents/SplotLabs/ac0ej3.ivf`) and
//! `#[ignore]`d to match the existing `local_ac0ej3_*` probe convention.

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

/// The full first-3-superblock luma DC region the sink now reconstructs: the
/// rectangle x[0,192) (three 64-wide superblock columns) x y[0,128) (two
/// superblock rows), 24576 samples. Fixing the MI(4,0) `TX_16X64` keystone (the
/// §7.13.2.12 IBP DC modifier + the non-square `TX_16X64` residual) unblocks the
/// whole DC chain that bordered it through the §7.13.2 edge-coverage guard, so the
/// verified region widens 24x from the original 1024-sample column. Every sample is
/// the down-predicted flat `DC_PRED` value `64` except the origin 16x16 leaf
/// (`68`, 256 samples) and the MI(4,0) IBP DC step (`65`, the top-left 3 columns x
/// 16 rows == 48 samples). Derived offline from `ac0_prefiltered.yuv`.
const LUMA_REGION_WIDTH: usize = 192;
const LUMA_REGION_HEIGHT: usize = 128;
const LUMA_REGION_SAMPLE_COUNT: usize = LUMA_REGION_WIDTH * LUMA_REGION_HEIGHT;
/// Sum of the 192x128 region (`256 * 68 + 48 * 65 + 24272 * 64`).
const LUMA_REGION_SAMPLE_SUM: u64 = 1_573_936;
/// FNV-1a-64 over the 192x128 region (row-major, sample-major u16 LE).
const LUMA_REGION_FNV1A64: u64 = 0x31c1_4055_9bd3_8725;
/// The §7.13.2.12 IBP DC value at the MI(4,0) `TX_16X64` leaf's top-left 3 columns.
const MI40_IBP_STEP: u16 = 65;

/// The total reconstructed luma sample count after the §5.20.7.27 non-DCT inter
/// residual parse-advancement. The earlier widening reconstructed `32768` samples
/// (the 24576-sample first-3-superblock DC region x[0,192) x y[0,128), PLUS the
/// SB-column-3 `BLOCK_64X64 H_PRED` block `4096`, PLUS the 32x64 `DC_PRED` block
/// `2048`, PLUS the first §7.13.3.18 IntrABC `BLOCK_32X64` target `2048`). With the
/// non-DCT inter luma `transform_type` SETS (`TX_SET_INTER_1/2`, `TX_SET_DCT_IDTX*`,
/// alongside the long-side sets) now PARSED — the coefficient loop is
/// transform-type-agnostic and the unsupported inverse transform defers at the
/// sink — the entropy stream stays synced through the rest of frame-0's first four
/// superblock columns, so every downstream proven-subset leaf (DC / cardinal
/// H_PRED / V_PRED / zero-residual IntrABC copy) that those formerly-desynced
/// blocks bordered now reconstructs in decode order. The verified region grows to
/// `55296` bit-exact luma samples, bounded by x[0,256) x y[0,256) (the first four
/// SB columns over SB rows 0..3 plus the first two SB columns of SB row 3, minus
/// the deferred non-proven-subset blocks inside that box). Verified ZERO-mismatch,
/// per sample, against the AVM pre-filter reconstruction oracle (`ac0_prefiltered.yuv`).
const LUMA_RECON_SAMPLE_TOTAL: usize = 55296;

/// The SB-column-3 `BLOCK_64X64 H_PRED` block (x[192,256) x y[0,64)) — the
/// §7.13.3.18 IntrABC source. Mode `H_PRED` (pAngle 180, `AngleDeltaY == 0`), split
/// into four `TX_32X32` `DCT_DCT` transforms. Each row is the §7.13.2.8 step-5
/// horizontal copy of the real reconstructed left column (the x=191 DC region edge,
/// flat `64`), so the block is flat `64` except the top-right `TX_32X32` which
/// carries a `+4` DC residual to flat `68` (verified against the oracle). Derived
/// offline from `ac0_prefiltered.yuv`.
const HPRED_BLOCK_X: usize = 192;
const HPRED_BLOCK_Y: usize = 0;
const HPRED_BLOCK_SIDE: usize = 64;
const HPRED_BLOCK_SAMPLE_COUNT: usize = HPRED_BLOCK_SIDE * HPRED_BLOCK_SIDE;
/// Sum of the H_PRED block (`3072 * 64 + 1024 * 68`).
const HPRED_BLOCK_SAMPLE_SUM: u64 = 266_240;
/// FNV-1a-64 over the H_PRED block (row-major, sample-major u16 LE).
const HPRED_BLOCK_FNV1A64: u64 = 0x615c_4637_5763_6325;
/// The flat `H_PRED` copy value (the x=191 left-column DC edge), and the value of
/// the top-right `TX_32X32` after its `+4` DC residual.
const HPRED_FLAT: u16 = 64;
const HPRED_TOP_RIGHT_STEP: u16 = 68;

/// The first §7.13.3.18 IntrABC block's `BLOCK_32X64` luma TARGET (MI(16,56) →
/// x[224,256) x y[64,128)). The §5.20.5.4 block vector is integer (row `-512`
/// eighth-pel == `-64` samples, col `0`), and the block is a `skip` leaf (zero
/// residual), so §7.13.3.18 reduces to a plain copy of the displaced `CurrFrame`
/// SOURCE rectangle x[224,256) x y[0,64) — the right half of the already-reconstructed
/// H_PRED block. Source (hence target) is the top-right `TX_32X32` `68` over its top
/// 32 rows (y[64,96), copied from the H_PRED `68` at y[0,32)) and flat `64` below
/// (y[96,128)). The oracle confirms `target == source` over the full 32x64 block.
/// Derived offline from `ac0_prefiltered.yuv`.
const INTRABC_TARGET_X: usize = 224;
const INTRABC_TARGET_Y: usize = 64;
const INTRABC_TARGET_WIDTH: usize = 32;
const INTRABC_TARGET_HEIGHT: usize = 64;
const INTRABC_SAMPLE_COUNT: usize = INTRABC_TARGET_WIDTH * INTRABC_TARGET_HEIGHT;
/// The IntrABC source rectangle (x[224,256) x y[0,64)) the target copies from — the
/// right half of the reconstructed SB-column-3 H_PRED block.
const INTRABC_SOURCE_X: usize = 224;
const INTRABC_SOURCE_Y: usize = 0;
/// The `68` band height of the IntrABC target: its top 32 rows (the copied top-right
/// `TX_32X32` `68`), the rest flat `64`.
const INTRABC_TOP_BAND_HEIGHT: usize = 32;
/// Sum of the IntrABC target (`1024 * 68 + 1024 * 64`).
const INTRABC_TARGET_SAMPLE_SUM: u64 = 135_168;
/// FNV-1a-64 over the IntrABC target (row-major, sample-major u16 LE).
const INTRABC_TARGET_FNV1A64: u64 = 0xb70e_5832_e8aa_2325;

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

    // Coverage report: with the non-DCT inter residual parse-advancement keeping the
    // entropy stream synced, the verified luma region is now the `55296`-sample
    // bit-exact region bounded by x[0,256) x y[0,256) (the proven-subset leaves the
    // synced parse unblocked across the first four SB columns) — plus the two 32x32
    // chroma origin blocks (2048 chroma samples total across U and V).
    let (luma4x4, chroma4x4) = sink.reconstructed_counts();
    assert_eq!(
        luma4x4 * 16,
        LUMA_RECON_SAMPLE_TOTAL,
        "verified luma region is the 55296-sample bit-exact DC + cardinal + IntrABC region"
    );
    assert_eq!(
        chroma4x4 * 16,
        2 * CHROMA_ORIGIN_SAMPLE_COUNT,
        "verified chroma region is the U+V 32x32 origin blocks (2048 samples)"
    );
}

/// Bit-exact verification of the FULL first-3-superblock luma DC region (x[0,192) x
/// y[0,128), 24576 samples) against the AVM pre-filter oracle — a 24x widening of
/// the original 1024-sample `BLOCK_16X64` column.
///
/// This is unlocked by fixing the MI(4,0) `TX_16X64` keystone with two
/// reconstruction fixes: (1) the §7.15.4 outer inverse transform now drives the
/// NON-SQUARE residual path, and (2) the §7.13.2.12 IBP DC modifier (ac0ej3 has
/// `enable_ibp == 1`) blends the MI(4,0) left edge columns toward the reconstructed
/// `BLOCK_16X64` left neighbour, producing the oracle's `65` step in the top-left 3
/// columns. Every DC block downstream bordered that keystone through the §7.13.2
/// edge-coverage guard, so the whole first-3-SB luma DC chain now reconstructs in
/// one shot. The region is `68` (origin leaf, 256 samples), `65` (the MI(4,0) IBP
/// step, 48 samples), and `64` (the rest); per-sample, sum, and FNV-1a-64 all
/// match the oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_first_three_superblock_luma_reconstructs_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 first three superblock luma");

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    let mut mi40_step_samples = 0usize;
    for y in 0..LUMA_REGION_HEIGHT {
        for x in 0..LUMA_REGION_WIDTH {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            // Per-region oracle: the origin 16x16 leaf is `68`; the MI(4,0)
            // `TX_16X64` IBP-DC step (x[16,19), y[0,16)) is `65`; everything else is
            // the down-predicted flat `DC_PRED` value `64`.
            let in_mi40_step = (16..19).contains(&x) && y < BLOCK0_SIDE;
            let expected = if y < BLOCK0_SIDE && x < BLOCK0_SIDE {
                BLOCK0_FLAT_LUMA
            } else if in_mi40_step {
                MI40_IBP_STEP
            } else {
                LUMA_COLUMN_BELOW_ORIGIN
            };
            assert_eq!(
                sample, expected,
                "first-3-SB luma ({x},{y}) must be {expected}, got {sample}"
            );
            if in_mi40_step {
                mi40_step_samples += 1;
            }
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    // The MI(4,0) §7.13.2.12 IBP DC step is exactly the top-left 3 columns x 16 rows.
    assert_eq!(mi40_step_samples, 48, "MI(4,0) IBP DC step sample count");
    assert_eq!(
        count, LUMA_REGION_SAMPLE_COUNT,
        "first-3-SB luma sample count"
    );
    assert_eq!(
        sum, LUMA_REGION_SAMPLE_SUM,
        "first-3-SB luma sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        LUMA_REGION_FNV1A64,
        "first-3-SB luma FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Bit-exact verification of the SB-column-3 `BLOCK_64X64 H_PRED` block (x[192,256)
/// x y[0,64)) against the AVM pre-filter oracle — the §7.13.3.18 IntrABC SOURCE
/// block that the DC-only sink could not reconstruct.
///
/// The block is cardinal `H_PRED` (pAngle 180, `AngleDeltaY == 0`): each row is the
/// §7.13.2.8 step-5 horizontal copy `pred[i][j] = LeftCol[i]` of the real
/// reconstructed left column (no above, no corner, no IDIF, no `useIBP` — pAngle 180
/// is excluded by the §7.13.2.7 `pAngle < 90 || pAngle > 180` gate). Its left column
/// (x=191) is the right edge of the already-reconstructed first-3-superblock DC
/// region (flat `64`), so the four `TX_32X32` `DCT_DCT` transforms reconstruct flat
/// `64` except the top-right transform (x[224,256) x y[0,32)), which carries a `+4`
/// DC residual over its (flat-`64`) left neighbour and so is flat `68`. Per-sample,
/// sum, and FNV-1a-64 all match the oracle. This is the first bit-exact DIRECTIONAL
/// (non-DC) ac0ej3 block, and it unblocks the IntrABC brick (which copies from it).
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_sb_column3_hpred_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 SB-column-3 H_PRED block");

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in HPRED_BLOCK_Y..HPRED_BLOCK_Y + HPRED_BLOCK_SIDE {
        for x in HPRED_BLOCK_X..HPRED_BLOCK_X + HPRED_BLOCK_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            // The H_PRED copy of the flat-`64` left column is flat `64` everywhere
            // except the top-right `TX_32X32` (x[224,256) x y[0,32)) which is `68`.
            let in_top_right = (224..256).contains(&x) && y < 32;
            let expected = if in_top_right {
                HPRED_TOP_RIGHT_STEP
            } else {
                HPRED_FLAT
            };
            assert_eq!(
                sample, expected,
                "H_PRED block luma ({x},{y}) must be {expected}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(count, HPRED_BLOCK_SAMPLE_COUNT, "H_PRED block sample count");
    assert_eq!(
        sum, HPRED_BLOCK_SAMPLE_SUM,
        "H_PRED block sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        HPRED_BLOCK_FNV1A64,
        "H_PRED block FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );

    // The V_PRED block at SB column 4 (x[256,320) x y[0,64)) is at the frame TOP
    // (y == 0, no above neighbour): V_PRED reads ONLY the above row, which is
    // off-frame here, so the cardinal copy has no real neighbour and the sink
    // DEFERS it (stays at the unreconstructed fill value `0`).
    assert_eq!(
        sink.reconstructed_sample(PlaneId::Y, 256, 0).unwrap(),
        0,
        "the no-above V_PRED block at x[256,320) y=0 must stay deferred",
    );

    // The whole reconstructed luma region is now 55296 bit-exact samples (the
    // non-DCT inter residual parse-advancement kept the stream synced, unblocking
    // the rest of the first four SB columns' proven-subset leaves).
    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    assert_eq!(
        luma4x4 * 16,
        LUMA_RECON_SAMPLE_TOTAL,
        "the parse-advanced walk reconstructs 55296 bit-exact luma samples"
    );
}

/// Bit-exact verification of the FIRST §7.13.3.18 IntrABC block's `BLOCK_32X64`
/// luma TARGET (MI(16,56) → x[224,256) x y[64,128)) against the AVM pre-filter
/// oracle — the first break through the original mission IntrABC wall.
///
/// The §5.20.5.4 block vector is integer (row `-512` eighth-pel == `-64` samples,
/// col `0`) and the leaf is a `skip` block (zero residual), so §7.13.3.18 reduces to
/// a plain copy of the displaced `CurrFrame` SOURCE rectangle x[224,256) x y[0,64)
/// (the right half of the already-reconstructed SB-column-3 H_PRED block) into the
/// target. Two independent checks: (1) every target sample equals the SOURCE sample
/// directly above it by the integer DV (the copy is faithful), and (2) every target
/// sample matches the committed oracle constant (`68` over the top 32 rows copied
/// from the H_PRED top-right `TX_32X32`, `64` below), with the target's sum and
/// FNV-1a-64 matching the oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_first_intrabc_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");

    let sink = reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 first IntrABC block");

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for row in 0..INTRABC_TARGET_HEIGHT {
        for col in 0..INTRABC_TARGET_WIDTH {
            let tx = INTRABC_TARGET_X + col;
            let ty = INTRABC_TARGET_Y + row;
            let target_sample = sink.reconstructed_sample(PlaneId::Y, tx, ty).unwrap();

            // (1) Faithful integer copy: target == source (the §7.13.3.18 displaced
            // sample at the DV (-64 rows, 0 cols)).
            let sx = INTRABC_SOURCE_X + col;
            let sy = INTRABC_SOURCE_Y + row;
            let source_sample = sink.reconstructed_sample(PlaneId::Y, sx, sy).unwrap();
            assert_eq!(
                target_sample, source_sample,
                "IntrABC target ({tx},{ty}) must equal its DV source ({sx},{sy})"
            );

            // (2) Oracle constant: `68` over the top 32 rows (the copied H_PRED
            // top-right `TX_32X32`), `64` below.
            let expected = if row < INTRABC_TOP_BAND_HEIGHT {
                HPRED_TOP_RIGHT_STEP
            } else {
                HPRED_FLAT
            };
            assert_eq!(
                target_sample, expected,
                "IntrABC target ({tx},{ty}) must be {expected}, got {target_sample}"
            );
            fnv.update_u16(target_sample);
            sum += u64::from(target_sample);
            count += 1;
        }
    }

    assert_eq!(count, INTRABC_SAMPLE_COUNT, "IntrABC target sample count");
    assert_eq!(
        sum, INTRABC_TARGET_SAMPLE_SUM,
        "IntrABC target sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        INTRABC_TARGET_FNV1A64,
        "IntrABC target FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}
