## Context

The decoder runtime now supports a narrow `minimal-intra-8bit420-hash-v1`
fixture path. That path parses the committed IVF fixture, derives a
source-backed tile-payload boundary, consumes the root partition and traced flat
block-symbol frontier, validates §8.2.4 `exit_symbol()`, reconstructs the
minimal output, and computes the stable hash.

The tile-payload boundary itself remains crate-private and intentionally
partial. The fuzz crate is outside the workspace and cannot call crate-private
APIs without exposing a fuzz-only hook. A regular public runtime target would
mostly duplicate `decode_runtime_hash_bytes`, so this change adds a
feature-gated, documented fuzzing harness in `splot-decode` that composes the
existing crate-private boundary without changing the default production API.

## Goals / Non-Goals

**Goals:**

- Add `tile_payload_decode_bytes`, a cargo-fuzz target that calls a
  `splot-decode` `fuzzing`-feature harness over bounded arbitrary tile-payload
  bytes and bounded mutations of a known-good two-byte minimal frontier payload.
- Keep all inputs in memory and enforce finite `DecodeLimits` before decoding.
- Assert only stable boundary/frontier invariants on success, while accepting
  typed boundary/frontier errors on malformed or unsupported mutations.
- Record Feature ID `CONF-TILE-PAYLOAD-DECODE-FUZZ` in the implementation matrix
  and as decoder-support/conformance evidence.

**Non-Goals:**

- No default production `splot-decode` API change.
- No direct access to crate-private `tile_payload` modules from the fuzz target;
  it uses the feature-gated harness only.
- No full §5.20 `decode_tile()` implementation, recursive partition/block
  syntax, broad §8.3 CDF selection, or broad Tile/Saved CDF bank coverage.
- No reconstruction expansion, hash/Y4M behavior changes, reference refresh, or
  external decoder evidence.
- No filesystem, network, subprocess, AVM, dav2d, ffmpeg, dependency, or
  scheduler changes.

## Decisions

1. Use a `fuzzing` feature-gated harness instead of the public hash runtime.

   Rationale: the direct tile-payload APIs are crate-private, and fuzzing only
   `DecodeContext::decode_hash_report_bytes` would mostly duplicate
   `decode_runtime_hash_bytes`. A `fuzzing` feature keeps the default API clean
   while letting the fuzz crate exercise the actual boundary/frontier.

   Alternative considered: expose `plan_tile_payload_boundary` publicly.
   Rejected because it would create a production-facing API before diagnostics
   and runtime semantics are stable.

2. Use a compact deterministic fuzz grammar.

   Rationale: the target should cover both arbitrary bounded tile payload bytes
   and successful frontier traces. The grammar carries flags, payload-length and
   limit seeds, optional known-good payload mutation mode, and finite
   tile-size/tile-group seeds, all capped at small limits.

3. Keep the support status partial.

   Rationale: no-panic fuzzing is evidence for robustness, not completion of
   tile syntax. The `tile-payload-decode` row should gain proof references but
   remain `partial` until broad `decode_tile()` behavior is implemented and
   tested.

## Risks / Trade-offs

- [Risk] The feature-gated harness could be mistaken for a supported runtime
  API. -> Mitigation: keep it under a `fuzzing` feature, mark it `#[doc(hidden)]`,
  document that it is test-only, and avoid exporting crate-private types.
- [Risk] The target still covers a minimal one-tile frontier, not arbitrary tile
  group structures. -> Mitigation: document this as minimal-runtime tile
  payload frontier fuzzing and keep broad tile decode partial.
- [Risk] An over-tight success assertion could turn valid future behavior into a
  false fuzz crash. -> Mitigation: assert only local boundary/frontier invariants
  guaranteed by the harness on `Ok`; all typed decode errors remain acceptable.
- [Risk] CI fuzz-smoke time grows with another target. -> Mitigation: the target
  is bounded to one in-memory fixture shape and relies on existing per-target CI
  limits.
