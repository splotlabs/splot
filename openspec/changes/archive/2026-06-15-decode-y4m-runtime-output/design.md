## Context

The current minimal runtime path lives in `splot-decode/src/runtime_hash.rs`: it reuses `DecodeContext::plan_bytes`, validates the committed 64x64 AV02 IVF minimal tier, verifies the traced six-symbol §8.2 tile payload, constructs a flat 8-bit 4:2:0 `DecodedFrame<u8>`, and emits a `splot.decode.hash_report` success artifact. The CLI Y4M branch still only plans bytes and returns `decode/unsupported-feature`.

The source-backed Y4M writer already exists in `splot-recon`, and `splot-decode` already has the approved dependency edge to `splot-recon`. `splot-cli` must remain a thin file-I/O layer and must not gain a direct `splot-recon` dependency. `runtime_hash.rs` and `splot-recon/src/y4m.rs` are both at the 1000-line soft budget, so this change should split rather than grow those modules.

## Goals / Non-Goals

**Goals:**
- Add `DECODE-Y4M-RUNTIME-OUTPUT` as a narrow runtime output feature for the same minimal IVF tier already validated by `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.
- Add a `DecodeContext::decode_y4m_bytes` API that writes a complete Y4M stream to a caller-provided writer without filesystem access.
- Publish CLI Y4M output atomically through a same-directory temporary file and rename after full success.
- Count complete Y4M stream bytes against `DecodeLimitName::MaxOutputBytes` before publication.
- Preserve structured diagnostics and no-touch behavior for malformed, resource-limit, out-of-tier, and hash-mode paths.

**Non-Goals:**
- Broad AV2 tile traversal, prediction, transform, filtering, film grain, reference refresh, show-existing/flush scheduling, raw output, or multi-frame output.
- Any AVM/dav2d integration, repository wrapper, CI job, cache, or executable dependency.
- New third-party crates.
- Direct `splot-cli -> splot-recon` dependency.
- Validator behavior changes.

## Decisions

1. Split the minimal runtime validation/build surface from hash serialization.
   - Move the shared minimal-tier validation and flat-frame construction into a crate-private module such as `runtime_minimal.rs`.
   - `runtime_hash.rs` becomes a hash-report adapter over that shared output.
   - A new `runtime_y4m.rs` becomes the Y4M adapter over the same output.
   - Alternative considered: add Y4M serialization directly to `runtime_hash.rs`. Rejected because the file is already at the soft source-line budget and the shared tier would become harder to review.

2. Keep decoded frames private and expose only a writer API.
   - Add `DecodeContext::decode_y4m_bytes<W: std::io::Write>(&self, bytes, options, writer) -> Result<()>`.
   - This avoids a premature public decoded-frame streaming API while still keeping codec/Y4M serialization inside `splot-decode`.
   - Alternative considered: expose `DecodedFrame<u8>` from `splot-decode` and let `splot-cli` call `Y4mWriter`. Rejected because it would either leak a narrow internal tier shape or require `splot-cli` to depend on `splot-recon`.

3. Use IVF container timing for the Y4M frame-rate policy.
   - For the committed minimal fixture, the Y4M header uses the IVF timebase as a repository-owned output-container policy. The minimal fixture is expected to serialize as `F30:1` when its IVF header carries 30/1.
   - This does not claim AV2 normative timing support; future output-order/timing work remains separate.
   - Alternative considered: fixed `F30:1` for all supported streams. Rejected because using the validated IVF container facts avoids inventing an unrelated constant for future minimal fixtures.

4. CLI owns atomic publication.
   - `splot-cli` creates a unique temp file in the output path's parent directory with exclusive create, asks `splot-decode` to write the complete Y4M stream into it, flushes and syncs, renames over the final path, and attempts parent-directory sync.
   - Any failure before rename cleans up the temp file and leaves the final path absent or unchanged. Hash mode continues to ignore `-o`.
   - Alternative considered: write directly to final output. Rejected because it violates the mission's no-partial-output rule.

5. Add `decode/output-error` for publication and serialization failures.
   - Decode/malformed-source, decode/resource-limit, and decode/unsupported-feature keep their existing meanings.
   - Filesystem publication failures and Y4M writer I/O errors map to `decode/output-error` with stable operation metadata.
   - Pure filesystem failures do not cite an AV2 spec section; they are output publication contract failures.

## Risks / Trade-offs

- [Risk] Y4M output overclaims full decoder support. -> Mitigation: add a dedicated `decode-y4m-runtime-output` row, keep broad rows partial, and state that this only covers the existing minimal IVF tier.
- [Risk] Output byte limits count sample payload but not Y4M container bytes. -> Mitigation: the Y4M runtime precomputes or buffers the full Y4M byte stream and checks `MaxOutputBytes` against the complete byte length before CLI publication.
- [Risk] Atomic publication tests become OS-specific. -> Mitigation: test success/no-touch/cleanup with temp directories and stable file states; avoid relying on platform-specific directory sync failures.
- [Risk] Y4M temp filenames leak nondeterminism into diagnostics. -> Mitigation: diagnostics carry operation/kind/message only, not temp path suffixes.
- [Risk] Existing hash no-touch behavior regresses. -> Mitigation: keep and extend hash-mode tests that pass `-o` and assert the file remains unchanged.
- [Risk] Module split changes behavior. -> Mitigation: keep existing hash tests, add Y4M tests, and run `cargo xtask ci` plus targeted decoder/CLI tests before PR.
