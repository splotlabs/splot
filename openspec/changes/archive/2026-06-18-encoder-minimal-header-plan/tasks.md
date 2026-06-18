## 1. Tracking and Scope

- [x] 1.1 Add `ENC-MINIMAL-HEADER-PLAN` to `docs/IMPLEMENTATION-MATRIX.toml` with private planning scope, non-emitting status, and proof placeholders.
- [x] 1.2 Update encoder roadmap and gap-audit docs to place minimal header planning after `ENC-SYNTAX-IR` and before writer integration.

## 2. Private Header Plan

- [x] 2.1 Add a private `splot-encode` header-planning module with typed sequence, frame, and tile-group header intent records.
- [x] 2.2 Validate current-subset format, non-zero config dimensions, frame/config metadata compatibility, and single-tile first-frame intent through typed private errors.
- [x] 2.3 Keep the crate root private: no public re-export and no packet/output queue integration.

## 3. Tests and Evidence

- [x] 3.1 Add positive tests for deterministic valid header-plan construction and stable debug output.
- [x] 3.2 Add negative tests for zero dimensions, unsupported formats, and frame/config mismatches.
- [x] 3.3 Add or preserve a context lifecycle regression proving header planning does not enable `Packet` output.

## 4. Specs and Gates

- [x] 4.1 Run `openspec validate --all --no-interactive`.
- [x] 4.2 Regenerate feature/spec coverage docs and run `cargo xtask check-feature-status`.
- [x] 4.3 Run focused `splot-encode` tests and clippy.
- [x] 4.4 Run `cargo xtask ci`.
- [x] 4.5 Archive `encoder-minimal-header-plan` in the same PR after implementation tasks are complete.
