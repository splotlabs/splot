// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Committed negative-mutator harness (CONF-AVM-INVALID-STREAMS).
//!
//! For each row in [`MUTATIONS`], the harness reads a committed *valid* seed
//! vector from `tests/conformance/vectors/valid/`, applies a documented,
//! deterministic byte/field mutation **in memory** (the committed file is never
//! written), validates the mutated bytes with the same entry point the CLI
//! `validate` command uses (`splot_validate::Validator::validate_bytes`), and
//! asserts the row's expected error `rule_id` is present in the report.
//!
//! Unlike the cargo-fuzz targets (random bytes, no-panic only), these are
//! *targeted* malformations with a *named expected diagnostic*: a regression that
//! stops emitting the expected diagnostic fails CI. The mutations target stable,
//! decidable diagnostics at the container / OBU-header / LEB128-framing layers,
//! which are robust under byte surgery.
//!
//! Each row asserts that its expected `rule_id` is **present** among the report's
//! errors (not set-equality): a single low-level mutation can legitimately
//! cascade into downstream higher-level errors (e.g. dropping a coded frame also
//! trips a temporal-unit check), and pinning the primary diagnostic by presence
//! keeps the row robust to those benign secondary errors.
//!
//! Anti-vacuity / causation: every seed is first asserted to validate **clean**
//! (zero errors), so the diagnostic is provably caused by the mutation and not by
//! a pre-broken seed. The expected ids are existing **registered** diagnostics
//! (see `docs/VALIDATOR-DIAGNOSTICS.md`); this harness adds no new diagnostics.
//!
//! No AVM, no network: the harness only reads already-committed seed bytes and
//! runs the in-process validator, so it gates under `cargo test` (hence under
//! `cargo xtask ci`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use splot_validate::Validator;

/// A documented, deterministic in-memory mutation of a committed seed's bytes.
enum Mutation {
    /// Overwrite the byte at `offset` — which MUST currently equal `from` — with
    /// `to`. Pinning the expected original byte (`from`) makes the mutation
    /// self-validating: if a regenerated seed shifts the byte layout, the row
    /// fails with a clear "seed layout drifted" message instead of silently
    /// mutating the wrong byte (or panicking on an out-of-range index).
    SetByte { offset: usize, from: u8, to: u8 },
}

impl Mutation {
    /// Applies the mutation to a clone of `seed`, returning the mutated bytes.
    fn apply(&self, seed: &[u8], what: &str) -> Vec<u8> {
        let mut bytes = seed.to_vec();
        match *self {
            Mutation::SetByte { offset, from, to } => {
                assert!(
                    offset < bytes.len(),
                    "mutation offset {offset} ({what}) is past the {}-byte seed; the seed layout changed",
                    bytes.len()
                );
                assert_eq!(
                    bytes[offset], from,
                    "mutation offset {offset} ({what}) holds 0x{:02x}, expected 0x{from:02x}; the seed layout drifted",
                    bytes[offset]
                );
                bytes[offset] = to;
            }
        }
        bytes
    }
}

/// One `(seed, mutation, expected diagnostic)` row.
struct MutationCase {
    /// Seed vector path relative to `tests/conformance/`.
    seed: &'static str,
    /// Human-readable description of the byte/field being mutated.
    what: &'static str,
    /// The deterministic mutation to apply in memory.
    mutation: Mutation,
    /// The PRIMARY registered error `rule_id` the mutated stream MUST emit. This
    /// is matched by presence, not set-equality: a single low-level mutation can
    /// legitimately cascade into downstream errors (each row's comment notes any
    /// such known secondary diagnostics), so the row pins the anchor diagnostic
    /// and tolerates benign cascades.
    expect_rule_id: &'static str,
}

/// The negative-mutation table. Each row exercises one stable, decidable
/// diagnostic; the layers (IVF container / OBU header / LEB128 framing) span at
/// least three distinct registered diagnostics so the suite is non-vacuous.
///
/// Seed layout (`syn-key-intra-64x64.ivf`, verified with `splot inspect`):
///   - bytes 0..32   IVF file header (`DKIF`, fourcc `AV02`, 64x64, 1 frame)
///   - bytes 32..44  IVF frame header (size = 96, pts = 0)
///   - byte  44      LEB128 obu_size = 1   for OBU #0
///   - byte  45      OBU header 0x08       OBU_TEMPORAL_DELIMITER (type 2)
///   - byte  46      LEB128 obu_size = 12  for OBU #1
///   - byte  47      OBU header 0x04       OBU_SEQUENCE_HEADER (type 1)
///   - byte  59      LEB128 obu_size = 80  for OBU #2
///   - byte  60      OBU header 0x10       OBU_CLOSED_LOOP_KEY (type 4)
const MUTATIONS: &[MutationCase] = &[
    // --- Container layer (IVF) ---
    // IVF is the non-normative byte envelope; bytes 6..8 are the little-endian
    // `header_len` field, which must be at least the 32-byte baseline header
    // (splot-core requires `header_len >= IVF_HEADER_SIZE`). The seed declares
    // 0x0020 = 32; setting byte 6 to 0x1F makes header_len = 31, below the
    // baseline, so the IVF header parser rejects it. Rule: `ivf/invalid-header-length`.
    // (NOTE: the signature itself cannot be exercised here — the validator's
    // container auto-detect only routes to the IVF parser when the input begins
    // with `DKIF`, so a corrupt magic is parsed as raw Annex B instead of raising
    // `ivf/invalid-signature`. The mutation therefore keeps the magic intact and
    // corrupts the declared header length, which IS reachable via auto-detect.)
    MutationCase {
        seed: "vectors/valid/syn-key-intra-64x64.ivf",
        what: "byte 6: shrink IVF header_len below the 32-byte baseline (0x20=32 -> 0x1F=31)",
        mutation: Mutation::SetByte {
            offset: 6,
            from: 0x20,
            to: 0x1F,
        },
        expect_rule_id: "ivf/invalid-header-length",
    },
    // --- LEB128 / OBU-framing layer (AV2 v1.0.0 Annex B § B.2,
    //     docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) ---
    // num_bytes_in_obu is a leb128() length prefix; open_bitstream_unit receives
    // exactly that many bytes. Bumping OBU #2's one-byte obu_size (byte 59) from
    // 80 to 127 makes the declared payload run past the end of the IVF frame /
    // input, so the Annex B parser raises ObuPayloadOutOfRange, surfaced as the
    // generic framing diagnostic `bitstream/parse-error`.
    MutationCase {
        seed: "vectors/valid/syn-key-intra-64x64.ivf",
        what: "byte 59: inflate OBU #2 obu_size LEB128 (0x50=80 -> 0x7F=127) past end of input",
        mutation: Mutation::SetByte {
            offset: 59,
            from: 0x50,
            to: 0x7F,
        },
        expect_rule_id: "bitstream/parse-error",
    },
    // --- OBU-header layer (AV2 v1.0.0 § 6.2.2,
    //     docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) ---
    // OBU_CLOSED_LOOP_KEY (a key-frame type) requires obu_tlayer_id == 0. Byte 60
    // is its 1-byte obu_header() 0x10 = 0b0_00100_00 (ext=0, type=4, tlayer=0);
    // setting the low two bits to 1 (0x11 = 0b0_00100_01) makes obu_tlayer_id = 1,
    // which the validator rejects. Rule: `obu-header/temporal-layer-zero-only-types`.
    MutationCase {
        seed: "vectors/valid/syn-key-intra-64x64.ivf",
        what: "byte 60: set OBU_CLOSED_LOOP_KEY obu_tlayer_id to 1 (0x10 -> 0x11)",
        mutation: Mutation::SetByte {
            offset: 60,
            from: 0x10,
            to: 0x11,
        },
        expect_rule_id: "obu-header/temporal-layer-zero-only-types",
    },
    // --- OBU-header layer (AV2 v1.0.0 § 6.2.2,
    //     docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2), second distinct id ---
    // OBU_SEQUENCE_HEADER is a base-layer-only type (obu_tlayer_id and
    // obu_mlayer_id must be 0). Byte 47 is its 1-byte obu_header() 0x04 =
    // 0b0_00001_00 (ext=0, type=1, tlayer=0); setting the low two bits to 1
    // (0x05 = 0b0_00001_01) makes obu_tlayer_id = 1, which the validator rejects.
    // Rule: `obu-header/base-layer-only-types`.
    MutationCase {
        seed: "vectors/valid/syn-key-intra-64x64.ivf",
        what: "byte 47: set OBU_SEQUENCE_HEADER obu_tlayer_id to 1 (0x04 -> 0x05)",
        mutation: Mutation::SetByte {
            offset: 47,
            from: 0x04,
            to: 0x05,
        },
        expect_rule_id: "obu-header/base-layer-only-types",
    },
];

/// Locates `tests/conformance/` from the crate manifest dir (`crates/splot-cli`),
/// robustly relative to the workspace root (matches `conformance.rs`).
fn conformance_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .canonicalize()
        .expect("tests/conformance/ exists")
}

/// Reads a committed seed's bytes given its `tests/conformance/`-relative path.
fn read_seed(rel_path: &str) -> Vec<u8> {
    let path = conformance_root().join(rel_path);
    let msg = format!("read committed seed {}", path.display());
    std::fs::read(&path).expect(&msg)
}

#[test]
fn negative_mutations_emit_expected_diagnostics() {
    assert!(
        !MUTATIONS.is_empty(),
        "the mutation table must not be empty"
    );

    // The non-strict validator is the CLI default (errors fail, warnings do not),
    // and is stateless across inputs (matches conformance.rs).
    let validator = Validator::new(false);

    // Track the (layer-prefixed) ids actually exercised, to enforce a
    // non-vacuous spread across at least two layers / three distinct ids.
    let mut exercised: BTreeSet<&'static str> = BTreeSet::new();

    for case in MUTATIONS {
        // Anti-vacuity / causation: the UNMUTATED seed must validate clean, so the
        // diagnostic is provably caused by the mutation, not a pre-broken seed.
        let seed = read_seed(case.seed);
        let clean = validator.validate_bytes(&seed);
        let clean_errors: Vec<&str> = clean.errors().map(|d| d.rule_id.as_str()).collect();
        assert!(
            clean_errors.is_empty(),
            "seed {} must validate clean before mutation, but emitted error(s): {clean_errors:?}",
            case.seed
        );

        // Apply the documented, deterministic mutation in memory (the committed
        // file is never written).
        let mutated = case.mutation.apply(&seed, case.what);
        assert_ne!(
            mutated, seed,
            "mutation for {} ({}) did not change the bytes",
            case.seed, case.what
        );

        // Validate the mutated bytes. `validate_bytes` returning at all is itself
        // proof of no-panic (a panic would unwind and fail the test harness).
        let report = validator.validate_bytes(&mutated);
        let got: BTreeSet<&str> = report.errors().map(|d| d.rule_id.as_str()).collect();
        assert!(
            got.contains(case.expect_rule_id),
            "mutated {} ({}) expected error rule id {:?} but the validator emitted: {got:?}",
            case.seed,
            case.what,
            case.expect_rule_id
        );

        exercised.insert(case.expect_rule_id);
    }

    // Non-vacuity: at least three distinct diagnostics across at least two layers
    // (the layer prefix is the segment before the first '/').
    assert!(
        exercised.len() >= 3,
        "the negative mutator must exercise >= 3 distinct diagnostics, got {exercised:?}"
    );
    let layers: BTreeSet<&str> = exercised
        .iter()
        .map(|id| id.split('/').next().unwrap_or(id))
        .collect();
    assert!(
        layers.len() >= 2,
        "the negative mutator must span >= 2 diagnostic layers, got {layers:?}"
    );
}
