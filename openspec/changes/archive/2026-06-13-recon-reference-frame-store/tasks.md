## 1. Planning and Agent Log

- [x] 1.1 Create `agent-log.md` with orchestrator plan, agent roles, findings,
      and final acceptance notes.
- [x] 1.2 Validate the OpenSpec change before implementation.

## 2. Runtime API

- [x] 2.1 Add `crates/splot-recon/src/reference.rs` with `ReferenceSlot`,
      `ReferenceFrameStore<F>`, `ReferenceFrameEntry<'_, F>`, and slot-order
      `ReferenceFrameEntries<'_, F>` iteration.
- [x] 2.2 Export the new reference-store API from `crates/splot-recon/src/lib.rs`
      and update crate docs/feature tracking comments.
- [x] 2.3 Extend `ReconError` with typed capacity and slot-bound errors plus
      display messages.

## 3. Tests

- [x] 3.1 Add unit tests for valid store construction, `put`, lookup, `take`,
      `clear`, occupancy, and slot-order `entries` iteration.
- [x] 3.2 Add unit tests for zero capacity, excessive capacity, and out-of-range
      slot access without panics.
- [x] 3.3 Run focused `splot-recon` tests and clippy.

## 4. Docs and Status

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and
      `docs/DECODER-SUPPORT-MATRIX.toml` for the source-backed runtime store.
- [x] 4.2 Add `RECON-REFERENCE-FRAME-STORE` to
      `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [x] 4.3 Regenerate generated status docs.

## 5. Final Verification

- [x] 5.1 Run `openspec validate recon-reference-frame-store --strict`.
- [x] 5.2 Run `openspec validate --all --no-interactive`.
- [x] 5.3 Run `cargo xtask feature-status` and
      `cargo xtask check-feature-status`.
- [x] 5.4 Run `cargo xtask check-decoder-support`.
- [x] 5.5 Run `cargo xtask ci`.
- [x] 5.6 Record final reviewer sign-offs and verification in `agent-log.md`.
