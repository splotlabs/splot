## 1. OpenSpec And Feature Scope

- [x] 1.1 Validate `recon-reference-frame-store-refresh-flags` with strict OpenSpec checks.
- [x] 1.2 Keep Feature ID `RECON-REFERENCE-FRAME-STORE` and document the storage-only exclusions in OpenSpec, matrix rows, and roadmap text.

## 2. Reference Store API

- [x] 2.1 Add `ReferenceRefreshMask`, selected-slot iteration, and typed refresh-mask errors without adding dependencies or `F: Clone` bounds.
- [x] 2.2 Add `ReferenceFrameStore<F>::refresh_slots_with` with preflight capacity validation, ascending slot visitation, replacement returns, and zero-mask no-op behavior.
- [x] 2.3 Export the new public reference-store types from `splot-recon`.

## 3. Tests And Fuzzing

- [x] 3.1 Add focused `splot-recon` unit tests for valid masks, bit-16 rejection, zero-mask no-op, single- and multi-slot refresh, replacement returns, out-of-capacity no-mutation behavior, non-Clone payloads, and explicit `SharedFrame::share()` use.
- [x] 3.2 Extend `recon_reference_frame_store_bytes` to fuzz mask construction, selected-slot iteration, refresh application, and typed invalid/no-mutation paths against its oracle.
- [x] 3.3 Run targeted Rust and fuzz-crate checks for the reference store.

## 4. Status Docs And Gates

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/TESTING.md` with the scoped helper and exclusions.
- [x] 4.2 Run feature-status, decoder-support, and decoder-conformance coverage
      checks; generated status/coverage renders remain on demand.
- [x] 4.3 Run `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, `cargo xtask ci`, and `git diff --check`.

## 5. Review And PR

- [x] 5.1 Run independent review agents for API scope, spec honesty, fuzz/oracle behavior, and docs/status coverage.
- [x] 5.2 Commit, push, and open a ready non-draft PR with the Feature ID, proof commands, and explicit non-goals.
- [ ] 5.3 Wait for current-head green CI, fresh approval, and zero unresolved live review threads before merge.
