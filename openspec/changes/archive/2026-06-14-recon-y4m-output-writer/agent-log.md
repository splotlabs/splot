# Agent Log: recon-y4m-output-writer

## Orchestrator Plan

- Objective: implement a source-backed `splot-recon` Y4M writer for
  caller-supplied `DecodedFrame<T>` values.
- Baseline: detached `origin/main` at `25aef58`; `cargo xtask ci` passed before
  this change was started.
- Branch: `codex/recon-y4m-output-writer`.
- Scope: `splot-recon` API/tests plus docs, matrices, OpenSpec sync, generated
  status, and review evidence.
- Feature ID: `RECON-Y4M-OUTPUT-WRITER`.
- Non-goals: `splot decode -o` runtime success, byte-consuming decode, tile
  payload parsing, reconstruction algorithms, output scheduling, film grain,
  reference refresh, AVM/dav2d execution, and reference-tool integration.
- PR #101 carry-forward: `splot-recon` stays scheduler-free; future decode/Y4M
  orchestration belongs in `splot-decode` through `DecodeContext` and
  `splot_parallel::WorkerPool`.
- PR #113 carry-forward: current main already preserves unsupported-prefix
  precedence, retry-stable IVF truncated-header errors, prefixed
  `decode_plan_bytes` fuzz seeds, and updated `DecodeContext` raw-byte docs.
  This change must not touch or duplicate those parser paths.

## Planning Agents

### @architect / Rawls the 3rd

- Agent id: `019ec671-a2f9-7011-bf8a-0e2e56aaedfe`
- Prompt: evaluate architecture, crate boundaries, dependency flow, PR #101
  concurrency, and PR #113 carry-forward for a recon-owned Y4M writer.
- Findings:
  - Add a small `crates/splot-recon/src/y4m.rs` writer over caller-supplied
    `DecodedFrame<T>`.
  - Do not wire `splot decode` runtime output in this slice.
  - No new dependency is needed; use `std::io::Write`.
  - Update Feature ID `RECON-Y4M-OUTPUT-WRITER`, the `output-y4m` decoder
    support row, the implementation matrix, roadmap, and generated status.
  - Marking `output-y4m` supported can overclaim unless notes explicitly limit
    support to the library writer and leave CLI/runtime output unsupported.
  - Future parallel decode must commit frames in emission order before calling
    the writer; `splot-recon` must not sort or schedule.

### @spec-reader / Schrodinger the 3rd

- Agent id: `019ec671-a614-7891-9228-555fef71c81c`
- Prompt: read pinned AV2 spec mirror sections for decoded output, sample bytes,
  reference storage, and Y4M boundary.
- Findings:
  - Cite AV2 § 6.4.1 for bit depth, chroma format, subsampling, monochrome, and
    plane counts.
  - Cite § 6.17.4.4 only for cropped output geometry and chroma crop alignment.
  - Cite § 7.21.1 and § 7.21.2 for `OutY`/`OutU`/`OutV`, visible output
    dimensions, bit depth, and chroma sizes.
  - Cite § 6.16.13 for sample byte traversal when mirroring existing
    hash-input policy: Y/U/V order, raster scan, one byte for 8-bit, and
    little-endian two-byte samples above 8-bit.
  - Cite § 7.23 only negatively: Y4M serializes cropped output frames, not
    padded reference `FrameStore` storage.
  - Y4M itself is outside the AV2 spec and must be documented as
    repository-owned output policy, not AV2 semantics.

### @api-designer / Lovelace the 3rd

- Agent id: `019ec671-a8d0-7c30-994f-faa82763f959`
- Prompt: propose public API, error type, generic sample handling, tests, and
  docs naming.
- Findings:
  - Add `Y4mFrameRate`, `Y4mColorSpace`, `Y4mStreamHeader`,
    `Y4mFrameHeader`, `Y4mWriter<W>`, `Y4mError`, and `Y4mResult<T>`.
  - Keep `Y4mError` separate from `ReconError` because I/O errors do not fit
    the current clone/equality model.
  - Active `BitDepth`, not Rust sample storage type, decides byte width.
    `DecodedFrame<u16>` with 8-bit output must emit one byte per sample.
  - Stream header should use visible luma size, progressive `Ip`,
    caller-supplied frame rate, and color tags derived from `BitDepth` plus
    `PixelFormat`.
  - Validate stream/frame format before writing `FRAME\n`.

## Reference Agents

### @reference-oracle / Parfit the 3rd

- Agent id: `019ec671-ab69-7081-92ef-b7e43ef23d01`
- Prompt: determine whether local AVM/dav2d evidence is needed for this change.
- Findings:
  - No local AVM/dav2d evidence is needed if scope remains
    `DecodedFrame<T> -> Y4M bytes`.
  - This is a serialization contract over already materialized frames, not
    proof of AV2 bitstream decode, reconstruction, output scheduling, reference
    state, film grain, or decoder parity.
  - Do not add a `docs/LOCAL-REFERENCE-EVIDENCE.toml` entry unless a later
    change becomes an end-to-end bitstream decode comparison.
  - Agent log and PR should record self-contained tests and the no-runtime-decode
    boundary.

### @avm-reader-runner / Nash the 3rd

- Agent id: `019ec677-27a9-78f1-94a7-e64b8cc3647e`
- Prompt: decide whether local AVM source inspection or runs are needed.
- Findings:
  - No local AVM source inspection or AVM run is needed for this change as
    scoped.
  - A Y4M writer over caller-supplied `DecodedFrame<T>` serializes an already
    validated frame model, not AV2 decode/reconstruction behavior.
  - AVM is useful for decoded pixel/output-order correctness, not for proving a
    container writer fed by caller-owned pixels.
  - Do not reuse existing AVM/dav2d raw MD5 entries as Y4M proof, and do not add
    AVM wrappers, runners, CI, checked-in local paths, or source snippets.
  - Prove this writer with self-contained known-byte tests for headers,
    `FRAME\n`, visible-row-only output, stride/padding exclusion, chroma shapes,
    8-bit/10-bit byte policy, writer errors, and stream-format rejection.

### @dav2d-reader-runner / Averroes the 3rd

- Agent id: `019ec677-2af4-72e1-bc43-7786626c6380`
- Prompt: decide whether local dav2d source inspection or runs are needed.
- Findings:
  - No local dav2d source inspection or dav2d run is needed for this change.
  - The proof boundary is the existing in-memory frame contract, not decoder
    agreement.
  - Do not record dav2d revision/version/command/digest evidence for this Y4M
    writer and do not reuse existing AVM/dav2d raw MD5 entries as Y4M proof.
  - Test Y4M header/chroma/byte decisions with synthetic `DecodedFrame<T>`
    fixtures and document them as writer format behavior, not dav2d-derived
    behavior.

## Implementation Notes

- Implementation worker `Kuhn the 3rd`
  (`019ec679-e8c7-7840-a99c-95493daa518b`) edited only
  `crates/splot-recon/src/y4m.rs` and `crates/splot-recon/src/lib.rs`.
- Added public Y4M API: `Y4mFrameRate`, `Y4mChromaTag`, `Y4mFrameFormat`,
  `Y4mStreamHeader`, `Y4mFrameHeader`, `Y4mWriter<W>`, `Y4mError`, and
  `Y4mResult<T>`.
- The writer uses `std::io::Write` only, writes the stream header once, writes
  `FRAME\n` per accepted frame, and serializes visible decoded samples only.
- Byte policy: non-monochrome writes Y/U/V plane order; monochrome writes Y
  only; 8-bit writes one byte per visible sample; 10-bit writes little-endian
  `u16` bytes without scaling.
- Mismatch policy: stream/frame visible size, bit depth, and pixel format are
  validated before writing `FRAME\n` or payload bytes for the attempted frame.
- Added `Y4mWriter::flush()` during orchestrator review so the writer exposes a
  complete standard I/O surface.
- Updated `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md`. The
  decoder support `output-y4m` row is `partial`, not `supported`, because this
  is source-backed library writing over caller-supplied frames while runtime
  `splot decode -o` Y4M output remains unsupported.
- Regenerated `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.

## Test and Verification Notes

- Focused PR #113 carry-forward checks on current `origin/main`:
  `cargo test -p splot-decode unsupported_prefix --locked`,
  `cargo test -p splot-decode malformed_suffix --locked`, and
  `cargo test -p splot-core frame_cursor_retry_preserves_truncated_initial_frame_header_error --locked`
  passed before this change's implementation.
- Implementation worker reported passing:
  `cargo test -p splot-recon y4m --locked` and
  `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`.
- Orchestrator reran focused verification after adding `flush()`:
  `cargo test -p splot-recon y4m --locked` passed with 14 Y4M tests, and
  `cargo clippy -p splot-recon --all-targets --locked -- -D warnings` passed.
- Test review found denominator-zero frame-rate coverage missing. Fixed by
  table-driving `invalid_frame_rate_is_rejected_before_stream_header_write`
  across `(0, 1)` and `(1, 0)`. Follow-up
  `cargo test -p splot-recon y4m --locked` and
  `cargo clippy -p splot-recon --all-targets --locked -- -D warnings` passed.
- `wc -l crates/splot-recon/src/y4m.rs` reported 995 physical lines, below the
  repository's 1000-line advisory budget.
- `cargo test -p splot-recon --locked` passed with 58 unit tests and doctests.
- `cargo xtask check-dependency-direction` and
  `cargo xtask check-concurrency-policy` passed.
- `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`,
  `openspec validate recon-y4m-output-writer --strict`, and
  `openspec validate --all --no-interactive` passed.
- `git diff --check` passed.
- Full acceptance gate `cargo xtask ci` passed.
- AVM/dav2d boundary scan over changed code/docs/OpenSpec found no local
  absolute paths, runtime `Command` execution, or committed reference-tool
  integration. Broad AVM/dav2d hits are existing policy/docs language and the
  new explicit no-reference-tool boundary statements, not executable
  integration.

## Review Notes

- Test reviewer `Ohm the 3rd` (`019ec68c-f937-76f3-92d9-c7a0b3cb9ce6`)
  found one P3 test gap: the invalid frame-rate scenario required both zero
  numerator and zero denominator, but the test only covered `(0, 1)`. Fixed as
  recorded above; no other test gaps and no AVM/dav2d test requirement found.
- Fixture reviewer `Turing the 3rd` (`019ec68c-fbf8-7cc0-82c1-63ace84c1a12`)
  found no fixture issues. Inline synthetic byte vectors are adequate; committed
  `.y4m` fixtures or local-reference manifests would overclaim this slice.
- Fuzz reviewer `Darwin the 3rd` (`019ec68c-fecc-75c3-8b73-b0e929c93e9e`)
  found no fuzz requirement. The API is not byte-consuming; fuzz should wait for
  future arbitrary AV2/IVF decode-to-Y4M paths or any Y4M parser/input surface.
- Documentation reviewer `Russell the 3rd` (`019ec68d-01df-70e3-8910-15a4049224f7`)
  found no issues. The roadmap, matrices, generated status, and OpenSpec all
  keep `output-y4m` partial and runtime `splot decode -o` unsupported.
- Code/API reviewer `Popper the 3rd` (`019ec68d-04d7-7321-80e3-a3ab56030ceb`)
  found no code findings. The reviewer noted `y4m.rs` is under but close to the
  1000-line advisory budget and confirmed PR #113 parser areas were untouched.
- Security reviewer `Bacon the 3rd` (`019ec68d-434b-7351-bcf5-0b5a7fe38bab`)
  found no security, panic, allocation/overflow, I/O propagation, dependency,
  AVM/dav2d boundary, local-path, process-execution, or scheduler issues.
- Spec-conformance reviewer `Bernoulli the 3rd`
  (`019ec68d-4645-7930-b42e-deb984ee235c`) found no issues. Y4M is consistently
  documented as repository-owned and outside AV2; runtime decode/Y4M remains
  unsupported.
- Encoder-impact reviewer `Dewey the 3rd` (`019ec68d-4976-73b0-8283-9388d88af039`)
  found no issues. The API remains reusable for future encoder/recon roundtrips,
  does not change encoder behavior, and avoids AVM/dav2d integration.
- Dependency/concurrency reviewer `Hooke the 3rd`
  (`019ec68d-4c25-7331-9796-60a5881ae09f`) found no dependency or concurrency
  issues. No manifests or lockfile changed, and `splot-recon` has no WorkerPool,
  Rayon, crossbeam, threads, channels, queues, scheduler, or parallel iterator
  use.

## Final Acceptance

- Archived with `openspec archive recon-y4m-output-writer --yes`. The command
  updated `openspec/specs/decoder-support/spec.md` with one added requirement
  and one modified requirement, then moved the change to
  `openspec/changes/archive/2026-06-14-recon-y4m-output-writer/`.
- Active-spec verification confirmed the `Reconstruction Y4M output writer`
  requirement is present and the minimal decode tier contract now separates
  source-backed library Y4M writing from runtime `splot decode -o` Y4M output.
- Post-archive PR, CI, latest-head Codex review, and merge gating now belong to
  the implementation branch process.
