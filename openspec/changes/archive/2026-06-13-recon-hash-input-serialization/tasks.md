## 1. Planning and Agent Log

- [x] 1.1 Create `agent-log.md` with orchestrator plan, agent roles, findings,
      implementation notes, and final acceptance notes.
- [x] 1.2 Validate the OpenSpec change before implementation.

## 2. Runtime API

- [x] 2.1 Add `crates/splot-recon/src/hash_input.rs` with
      `DecodedFrameHashInput<'_, T>`, byte-stream/variant identifiers,
      checked `byte_len`, and writer-based serialization.
- [x] 2.2 Export the new hash-input API from `crates/splot-recon/src/lib.rs`
      and update crate docs/feature tracking comments.
- [x] 2.3 Keep `crates/splot-recon/Cargo.toml` unchanged: no digest dependency
      and no crate dependency graph change.

## 3. Tests

- [x] 3.1 Add unit tests for visible-row serialization excluding stride/padding.
- [x] 3.2 Add unit tests for monochrome, YUV420, YUV422, and YUV444 byte order
      and byte-length behavior.
- [x] 3.3 Add unit tests for 8-bit `u8`, 8-bit `u16`, and 10-bit little-endian
      sample serialization.
- [x] 3.4 Add unit tests for metadata exclusion and writer error propagation.
- [x] 3.5 Run focused `splot-recon` tests and clippy.

## 4. Docs and Status

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and
      `docs/DECODER-SUPPORT-MATRIX.toml` for source-backed hash-input
      serialization with digest computation still future work.
- [x] 4.2 Add `RECON-HASH-INPUT-SERIALIZATION` to
      `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [x] 4.3 Regenerate generated status docs.

## 5. Final Verification

- [x] 5.1 Run `openspec validate recon-hash-input-serialization --strict`.
- [x] 5.2 Run `openspec validate --all --no-interactive`.
- [x] 5.3 Run `cargo xtask feature-status` and
      `cargo xtask check-feature-status`.
- [x] 5.4 Run `cargo xtask check-decoder-support`.
- [x] 5.5 Run `cargo xtask ci`.
- [x] 5.6 Record final reviewer sign-offs and verification in `agent-log.md`.
