## Context

The finite-q golomb tail (`ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE`) covers luma DC
magnitude 8..=17 (`q < cMax`). Magnitude 18+ uses the §5.20.7.28 `read_quant`
golomb-*prefix* path (`q == cMax`), verified against the spec and the decoder
`read_quant`:

For the first/only DC coefficient `hrLevelAvg = 0` → `predLevel = 0`,
`m = Clip3(1, 6, GetMsb(0)) = 1`, `k = m + 1 = 2`, `cMax = Min(m + 4, 6) = 5`.

- The `q_length` loop reads `cMax = 5` zeros (no terminating 1), so `q == cMax`.
- `length = -1; do { length++; golomb_length_bit L(1) } while (!golomb_length_bit)`
  reads `golomb_zeros` zeros then a terminating 1 (`golomb_zeros + 1` bits); then
  `length += k` → `length = golomb_zeros + 2`.
- `xBase = (q << m) + (1 << length) - (1 << k) = 10 + 2^length - 4 = 6 + 2^length`.
- `coeff_rem L(length)`; `x = xBase + coeff_rem`; `magnitude = maxLevel + x = 8 + x`.

Encoding (inverting the decode), given `x = magnitude - 8` with `x >= 10`:

- `length = GetMsb(x - 6) = (x - 6).ilog2()` (valid since `x - 6 >= 4`).
- `golomb_zeros = length - k = length - 2`.
- `coeff_rem = (x - 6) - (1 << length)` (and `coeff_rem < 2^length`).

So the bypass bits are: `cMax` (5) `q_length` zeros, then `golomb_zeros` zeros and
a terminating 1 (`golomb_length`), then `coeff_rem` as one `L(length)` literal.
The `dc_sign` CDF token precedes all of this (§5.20.7.27's sign+quant pass reads the
sign before calling `read_quant`).

Per golomb `length`, the magnitude span is `14 + 2^length .. 13 + 2^(length+1)`:

- length 2 → 18..21, length 3 → 22..29, length 4 → 30..45, length 5 → 46..77,
  length 6 → 78..141, length 7 → 142..269, length 8 → 270..525.

This change supports `length` 2..=8, i.e. magnitude 18..=525 (`coeff_rem` ≤ 255,
`golomb_length` ≤ 7 bits) — a bounded, fully-tested range. Larger magnitudes are a
trivial extension of the same mechanism (a wider `coeff_rem`) and are rejected with
the typed `BlockSymbolTraceGolombMagnitudeOutOfRange` error until a later brick
needs them.

For magnitude 18 (the canonical minimal): `x = 10`, `length = GetMsb(4) = 2`,
`golomb_zeros = 0`, `coeff_rem = 4 - 4 = 0`. Bypass bits: `0,0,0,0,0` (q_length),
`1` (golomb_length, 0 zeros + the 1), `coeff_rem = L(2) = 0`. The 17-token trace is
`[0,0,0, 0,0,4,3, 0, 0,0,0,0,0, 1, 0, 1,1]`.

Normative AV2 v1.0.0 sections:

- §5.20.7.27 sign+quant pass (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §5.20.7.28 `read_quant()` golomb-prefix (`#s-5-20-7-28`).
- §8.2.5 bypass `L(n)` literals.

## Goals / Non-Goals

**Goals:**

- Add `compose_intra_dc_golomb_prefix_block_trace` for magnitude 18..=525.
- Prove the trace through one §8.2 coder and that the decoded golomb-prefix bits
  reconstruct each encoded magnitude via the `read_quant` golomb-prefix arithmetic.
- Reject out-of-range magnitudes at runtime; preserve the no-packet invariant.

**Non-Goals:**

- No magnitude beyond 525 (a wider `coeff_rem` is a trivial later extension), no
  multi-coefficient blocks, higher-frequency coefficients, chroma golomb, partition
  syntax, tile CDF lifecycle, tile-body emission, packet output, CLI success, or
  Baseline Encoder Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **`coeff_rem` is one `L(length)` bypass token.** The decoder reads `coeff_rem`
   as a single `L(length)` literal, so the trace emits one `bypass(length,
   coeff_rem)` token (the unary `q_length`/`golomb_length` bits stay individual
   1-bit literals). For `length` ≤ 8 the `coeff_rem` value ≤ 255 is exact in the
   `decoded_symbols` u8 view, so the conformance test reconstructs it directly.

2. **Runtime range rejection (not a debug assert).** Mirroring the finite-q P2,
   the parameterized helper returns `BlockSymbolTraceGolombMagnitudeOutOfRange` for
   any magnitude outside 18..=525 so a release build cannot emit a non-conformant
   trace.

3. **Bounded supported range.** `length` ≤ 8 keeps the trace and `coeff_rem` width
   small and fully testable. The mechanism is length-agnostic; the cap is a
   brick-scope decision, documented and enforced, not a spec limit.

## Flight Manifest

- Change ID: `encoder-block-trace-golomb-prefix`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX`
- Base commit: `6d177911` (`feat(encode): finite-q golomb tail for a larger coded luma DC (#325)`)
- Depends on merged changes: `encoder-block-trace-golomb-finite`,
  `encoder-block-trace-bypass-literal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/block_symbol_trace_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-golomb-prefix/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-golomb-prefix/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (reuses the existing
    `BlockSymbolTraceGolombMagnitudeOutOfRange`); `crates/splot-encode/src/coefficient_tokenization.rs`;
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `6d177911`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs (the merge changes the
  feature count), and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The golomb-prefix arithmetic is intricate (two unary codes + a sized
  remainder). -> Mitigation: the derivation is verified against the decoder's
  `read_quant`, and a range test reconstructs every supported magnitude back to
  its value.
- [Risk] The §8.2 roundtrip only proves the bits are self-consistent. ->
  Mitigation: the reconstruction test runs the decoder's golomb-prefix arithmetic
  on the decoded bits and asserts the encoded magnitude.
