## 1. Planning and Agent Log

- [x] 1.1 Create `agent-log.md` with orchestrator plan, scoped non-goals, and
  required subagent roles for this change.
- [x] 1.2 Run planning/spec/API/reference subagents or record a concrete
  BLOCKED entry if required subagents are unavailable.
- [x] 1.3 Validate `decoder-diagnostic-registry` with OpenSpec before code edits.

## 2. Registry Implementation

- [x] 2.1 Add `docs/DECODER-DIAGNOSTICS.md` with a marker-delimited emitted
  decoder diagnostic table for `decode/unsupported-feature`.
- [x] 2.2 Extend `xtask/src/diagnostic_registry.rs` to check validator and
  decoder registry descriptors without changing validator semantics.
- [x] 2.3 Add unit tests for decoder registry success and drift in both
  undocumented and unemitted directions.
- [x] 2.4 Keep `cargo xtask check-diagnostic-registry` wired into `cargo xtask ci`
  with no external decoder or AVM/dav2d dependency.

## 3. Status, Docs, and Specs

- [x] 3.1 Add feature tracking rows/proofs for
  `DOC-DECODER-DIAGNOSTICS` and
  `XTASK-DECODER-DIAGNOSTIC-REGISTRY`.
- [x] 3.2 Add a decoder support matrix row for the decoder diagnostic registry
  and regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 3.3 Update decoder/process docs and mapping references so the registry is
  discoverable.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.

## 4. Verification and Review

- [x] 4.1 Run `openspec validate decoder-diagnostic-registry --strict`.
- [x] 4.2 Run `cargo test -p xtask diagnostic_registry --locked`.
- [x] 4.3 Run `cargo xtask check-diagnostic-registry`,
  `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`.
- [x] 4.4 Run `cargo xtask ci`.
- [x] 4.5 Run required review/security/spec/encoder-impact subagents, address
  findings, and record final acceptance in `agent-log.md`.
