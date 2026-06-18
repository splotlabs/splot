## Why

`Context::send_frame`, `Context::receive_packet`, and `Context::flush` still use
`splot_core::Error::Unimplemented` as lifecycle flow control. The frame input
boundary is now real, so the encoder needs a deterministic state machine that can
be tested before any legal AV2 packet generation exists.

## What Changes

- Add a new `ENC-CONTEXT-STATE-MACHINE` Feature ID for non-normative encoder
  lifecycle API behavior.
- Replace lifecycle `Unimplemented` results with explicit send/receive/flush
  statuses and typed encoder state errors.
- Track accepting, draining, finished, and failed terminal behavior in
  `splot-encode::Context`.
- Add bounded input/output queue accounting without emitting fake coded packets.
- Add deterministic unit/property coverage and a bounded fuzz target for lifecycle
  command sequences.
- Update encoder API specs, implementation-matrix proof, and encoder status docs.

Non-goals:

- No AV2 syntax emission, range/entropy coding, coded tile body, or container
  packet output.
- No Y4M reader, CLI public success path, output file publication, or packet
  muxing.
- No dependency graph change and no new third-party dependency.
- No reconstruction, reference state, RDO, rate control, or threading behavior
  beyond preserving deterministic lifecycle semantics.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-api`: replaces the temporary "all lifecycle calls are
  unimplemented" API contract with explicit no-packet lifecycle statuses and
  state-error behavior while keeping public encode success unavailable.

## Impact

- Affected code: `crates/splot-encode/src/context.rs`,
  `crates/splot-encode/src/error.rs`, `crates/splot-encode/src/lib.rs`, the
  encoder CLI call site if required by the new `send_frame` result type, and a
  new encoder state-machine fuzz target.
- Affected docs/specs: `openspec/specs/encoder-api/spec.md` delta,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated feature/status views, and encoder
  roadmap/gap/goal wording that currently says lifecycle operations return
  `Unimplemented`.
- Dependencies: none.
- Validator impact: none; no bitstream is emitted.
