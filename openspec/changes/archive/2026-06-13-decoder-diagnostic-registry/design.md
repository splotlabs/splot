## Context

The validator already has `docs/VALIDATOR-DIAGNOSTICS.md` and an enforced
`cargo xtask check-diagnostic-registry` gate that scans non-test
`crates/splot-validate/src` string literals and compares them with the
marker-delimited registry. The decoder mission added the first CLI diagnostic,
`decode/unsupported-feature`, before any decoder library crate exists. That ID
currently appears in `crates/splot-cli/src/commands/decode.rs` and in the decoder
support matrix, but it is not part of a canonical diagnostic registry.

This change is process and documentation infrastructure. It does not assert new
AV2 decoding semantics beyond the existing `splot decode` unsupported diagnostic
for AV2 §7.1 and the `cli-decode-entrypoint` support-matrix row.

## Goals / Non-Goals

**Goals:**

- Provide a canonical `docs/DECODER-DIAGNOSTICS.md` registry for emitted
  `decode/*` rule IDs.
- Extend the existing diagnostic-registry xtask so `cargo xtask ci` enforces the
  decoder registry alongside the validator registry.
- Keep the first emitted decoder diagnostic, `decode/unsupported-feature`,
  documented with severity, AV2 section, matrix row, message, and remediation.
- Record the work under stable Feature IDs in the implementation matrix and the
  decoder support matrix.

**Non-Goals:**

- No new `splot-decode` or `splot-recon` crate; those require explicit
  dependency-graph approval.
- No new crate dependencies or build-system changes.
- No real pixel decode, symbol decode, reconstruction, Y4M output, deterministic
  hashes, or supported stream-tier expansion.
- No AVM/dav2d source, wrapper, script, CI step, runtime execution, or committed
  local path.
- No validator diagnostic behavior change.

## Decisions

1. Keep the public xtask command name as `check-diagnostic-registry`.

   The existing CI and documentation already refer to one diagnostic-registry
   gate. Extending that command avoids adding a second CI step and keeps the
   acceptance gate obvious: all emitted product diagnostic IDs must be registered.
   Alternative considered: add `check-decoder-diagnostic-registry`. That would be
   clearer at the command level but easier to forget in docs/CI, and it would
   duplicate scanner code.

2. Split registry configurations internally.

   `xtask/src/diagnostic_registry.rs` should define small registry descriptors
   for each owner: validator scans `crates/splot-validate/src` against
   `docs/VALIDATOR-DIAGNOSTICS.md`; decoder scans the current decoder emission
   roots against `docs/DECODER-DIAGNOSTICS.md`. The decoder root is initially
   `crates/splot-cli/src/commands/decode.rs` because no decoder crate exists.
   Future `crates/splot-decode/src` can be added to that descriptor after the
   dependency graph is approved.

3. Scan only emitted decoder source, not every test and doc reference.

   The decoder registry compares emitted IDs to source literals in production
   code paths. Tests, support matrix rows, OpenSpec text, and docs contain the
   same IDs as assertions or references and must not be treated as separate
   emission sites. This matches the existing validator scanner's intent.

4. Keep marker-delimited Markdown as the registry format.

   The validator registry already uses `<!-- diagnostics-registry:begin -->` and
   `<!-- diagnostics-registry:end -->`. Reusing that format gives the same exact
   set comparison with no new parser or data-file dependency.

## Risks / Trade-offs

- Decoder source roots will evolve after `splot-decode` is approved.
  Mitigation: document the current CLI-only root in `docs/DECODER-DIAGNOSTICS.md`
  and keep the descriptor in one place.
- String-literal scanning can over-match unrelated `decode/*` strings in emitted
  source. Mitigation: keep diagnostic IDs plain literals and use the same
  one-slash grammar as the validator registry; unit tests cover drift in both
  directions.
- The registry documents one emitted diagnostic today, which can look like
  overhead. Mitigation: this is mission-critical scaffolding for later decode
  work and prevents unstable diagnostic names from spreading.
