# Agent Log: recon-frame-hash-digest

## Orchestrator Plan

- Objective: implement source-backed `splot-dfh-sha256-v1` digest computation
  in `splot-recon` over the existing `DecodedFrameHashInput` byte stream.
- Baseline: detached `origin/main` at `a9cb9c8`; `cargo xtask ci` passed before
  planning.
- Scope: `splot-recon` API/tests plus docs, matrices, OpenSpec sync, and
  generated status.
- Non-goals: byte-consuming decode, tile payload parsing, reconstruction
  algorithms, output ordering, film grain, AV2 metadata MD5 verification, Y4M,
  CLI hash output success, AVM/dav2d execution, or reference-tool integration.
- Dependency decision: use existing workspace `sha2` as a direct `splot-recon`
  dependency. This is not a new third-party package/version, but it is a crate
  dependency edge and must be visible in this log, docs, and PR.

## Planning Agents

### @architect / Peirce the 3rd

- Agent id: `019ec63f-8f0d-7fa0-8295-15e0204ec2a7`
- Prompt: evaluate architecture, crate boundaries, `sha2` dependency edge,
  AVM/dav2d boundary, and PR #101 concurrency model.
- Findings:
  - Architecture approved with condition that `sha2.workspace = true` for
    `splot-recon` is explicitly called out for maintainer review.
  - Keep the change narrow: `splot-recon` digest over existing
    `DecodedFrameHashInput`; no `splot-decode`, CLI, AVM/dav2d, Y4M,
    metadata-MD5, output ordering, film grain, or scheduler behavior.
  - `splot-recon` stays scheduler-free: no `splot-parallel`, no `WorkerPool`,
    no Rayon, no queues. Future multi-frame parallel hash orchestration belongs
    above it through `DecodeContext` and `splot_parallel::WorkerPool`.
  - Add Feature ID `RECON-FRAME-HASH-DIGEST`; update the deterministic-frame-hash
    support row without claiming runtime decode.

### @spec-reader / Aristotle the 3rd

- Agent id: `019ec63f-a8f9-7cc2-8b44-eac59762fab1`
- Prompt: read pinned AV2 spec mirror sections for decoded-frame-hash sample
  serialization and output samples.
- Findings:
  - `splot-dfh-sha256-v1` is SHA-256 over exactly
    `av2-output-samples-v1`, with no contract metadata, frame index,
    dimensions, OBU bytes, container timestamps, or AV2 decoded-frame-hash
    metadata in the digest input.
  - AV2 § 6.16.13 supplies sample-byte serialization: cropped visible output
    dimensions, Y/U/V plane order for non-monochrome, Y only for monochrome,
    raster order, 8-bit as one byte, greater-than-8-bit as little-endian
    two-byte samples.
  - Default variant remains `raw_intermediate_output`: pre-film-grain § 7.21.2
    `OutY`/`OutU`/`OutV`, corresponding to AV2 decoded-frame-hash `has_grain = 0`.
  - Non-goals: AV2 metadata MD5 verification, post-film-grain hash variant,
    output ordering, show-existing/flush handling, or runtime decode.
  - Supplied known SHA-256 vectors for tests, including monochrome visible rows,
    8-bit `u16`, 10-bit little-endian, 10-bit YUV420, and odd-dimension YUV420.

### @api-designer / Cicero the 3rd

- Agent id: `019ec63f-c143-7982-a0f4-bf580bbbc802`
- Prompt: propose minimal public/internal API, error propagation, docs, and
  tests for the digest.
- Findings:
  - Add a typed digest value with 32 raw bytes, stable algorithm identifier,
    `as_bytes`, lowercase hex formatting, `Display`, and `AsRef<[u8]>`.
  - Make digest computation infallible for already validated frame models.
    Do not call `write_to()` internally because it exposes writer and row-buffer
    allocation errors; feed visible samples directly into SHA-256.
  - Keep `byte_len()` and `write_to()` behavior unchanged; no new `ReconError`
    variant is needed.
  - Update crate/module docs that currently say digest computation is missing.
  - Do not touch `splot-cli` or `splot-decode`.

## Reference Agents

### @reference-oracle / Lorentz the 3rd

- Agent id: `019ec640-b035-73f2-a110-4d0210f5555f`
- Prompt: determine whether local AVM/dav2d runs/evidence are needed.
- Findings:
  - Reference evidence is not required and no AVM/dav2d run was performed.
  - This slice hashes already validated caller-supplied `DecodedFrame<T>` values
    and is separate from raw reference-output MD5 metadata.
  - Proof should be self-contained Rust tests; do not add
    `docs/LOCAL-REFERENCE-EVIDENCE.toml` entries unless real reference runs are
    performed.
  - Boundary check before merge must prove no AVM/dav2d source, snippets,
    binaries, submodules, wrappers, build probes, `xtask` commands, CI jobs,
    runtime `Command` execution, mandatory tests, local paths, or reference
    overclaims were added.

### @avm-reader-runner / Dalton the 3rd

- Agent id: `019ec640-ceb5-7953-843f-9e81c7b3df81`
- Prompt: assess whether local AVM source/runs are needed.
- Findings:
  - No local AVM source inspection or AVM run is needed.
  - A local AVM checkout existed outside the repository, but the agent did not
    inspect source or run binaries.
  - AVM becomes useful when proving reconstructed pixels from real AV2
    bitstreams, not for a repo-owned SHA-256 transform over modeled samples.
  - Do not claim AVM differential proof or label existing raw MD5 evidence as
    proof of `splot-dfh-sha256-v1`.

### @dav2d-reader-runner / Mendel the 3rd

- Agent id: `019ec640-e721-75a2-b0b5-a4dcd91b976e`
- Prompt: assess whether local dav2d source/runs are needed.
- Findings:
  - No local dav2d source read or dav2d run is needed.
  - The planned API is a repo-owned serialization/digest layer, not a decoder
    output oracle.
  - Existing AVM/dav2d manifest entries remain non-executable raw MD5
    background metadata and are not proof of this digest implementation.

## Implementation Notes

- Added `sha2.workspace = true` to `crates/splot-recon`; this is an explicit
  direct dependency edge to the already-approved workspace `sha2` package, not a
  new third-party package/version.
- Added public `DecodedFrameHash` with stable identifiers:
  `splot.decoded_frame_hash`, contract version `1`,
  `splot-dfh-sha256-v1`, `av2-output-samples-v1`, and
  `raw_intermediate_output`.
- Added `DecodedFrameHashInput::compute_hash()`, hashing the same cropped
  visible sample stream as `write_to()` directly into SHA-256 without allocation
  and without changing `write_to()` / `byte_len()` error behavior.
- Exported `DecodedFrameHash` from `splot-recon` and updated crate/module docs
  so the crate no longer describes digest computation as missing.
- Updated `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  and `docs/DECODER-ROADMAP.md` for Feature ID `RECON-FRAME-HASH-DIGEST`,
  while keeping runtime decode, Y4M output, AV2 metadata MD5 verification,
  AVM/dav2d execution, and reference-tool integration as non-goals.
- Inspected PR #113 Codex review
  `https://github.com/splotlabs/splot/pull/113#pullrequestreview-4492663492`.
  Current `main` / this branch already carries all four fixes: unsupported
  prefix precedence, IVF retry-stable truncated frame-header errors,
  `decode_plan_bytes` prefixed fuzz seeds, and updated `DecodeContext` raw-byte
  docs. This PR must mention that carry-forward review evidence explicitly.

## Test and Verification Notes

- Implementation worker `Laplace the 3rd`
  (`019ec645-64c8-7762-af16-093e5266dc00`) ran:
  `cargo test -p splot-recon hash_input --locked` and
  `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`; both
  passed.
- PR #113 Codex-review carry-forward checks run on this branch:
  `cargo test -p splot-decode unsupported_prefix --locked`,
  `cargo test -p splot-decode malformed_suffix --locked`,
  `cargo test -p splot-core frame_cursor_retry_preserves_truncated_initial_frame_header_error --locked`,
  and `cargo xtask check-concurrency-policy`; all valid invocations passed.
- `wc -l crates/splot-recon/src/hash_input.rs` reported 696 physical lines,
  below the 1000-line advisory budget.
- A token scan of changed recon code found no `unsafe`, Rayon/crossbeam/thread
  spawning, runtime process execution, or AVM/dav2d integration.
- Required verification gates run by the orchestrator:
  `cargo test -p splot-recon --locked`,
  `cargo xtask check-dependency-direction`,
  `cargo xtask check-concurrency-policy`,
  `cargo xtask check-feature-status`,
  `cargo xtask check-decoder-support`,
  `openspec validate recon-frame-hash-digest --strict`,
  `openspec validate --all --no-interactive`, and `git diff --check`; all
  passed.
- Generated status docs were refreshed with
  `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`,
  `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`,
  and
  `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`.
- Full acceptance gate `cargo xtask ci` passed after implementation and initial
  generated-doc refresh.
- AVM/dav2d boundary scans found no new reference source, snippets, binaries,
  wrappers, build probes, CI jobs, runtime process execution, mandatory tests,
  or local absolute paths. Broad hits were existing workspace dependency
  entries or explicit no-AVM/dav2d documentation; the changed dependency diff is
  only `splot-recon -> sha2.workspace`.

## Review Notes

- Final read-only code/API reviewer `Ramanujan the 3rd`
  (`019ec64f-cd65-7613-84c5-46475df5d7ff`) found no code/API bugs,
  regressions, or missing tests. The reviewer also independently checked the
  PR #113 Codex review `4492663492` carry-forward items and found the current
  branch preserves unsupported-prefix precedence, IVF retry-stable
  frame-header errors, `decode_plan_bytes` prefixed fuzz seeds, and updated
  `DecodeContext` raw-byte docs.
- Final security/supply-chain reviewer `Boole the 3rd`
  (`019ec650-8f1d-7741-b592-53fb3f589515`) found one issue: this log recorded
  an absolute local AVM checkout path. Fixed by replacing it with path-free
  wording. Follow-up local-path scan over this change returned no matches, and
  `cargo xtask check-reference-evidence` plus `cargo xtask check-decoder-support`
  passed.
- Final encoder/dependency/concurrency reviewer `Kant the 3rd`
  (`019ec650-a591-7113-8025-dc599395dc9c`) found no issues. The reviewer
  confirmed no encoder behavior changes, dependency direction and concurrency
  policy remain clean, `splot-recon` adds only the existing workspace `sha2`
  dependency, and PR #101 concurrency boundaries are preserved by keeping
  `splot-recon` scheduler-free with future orchestration through
  `DecodeContext` / `splot_parallel::WorkerPool`.
- Final AV2/spec/status reviewer `Hypatia the 3rd`
  (`019ec650-7a07-7722-914d-c3fe0a02802b`) found two issues:
  1. `docs/DECODER-SUPPORT-MATRIX.toml` marked `deterministic-frame-hash`
     supported while listing broader sections `5.17.12`, `7.21.1`, and
     `7.21.7`. Fixed by narrowing the row to the implemented and proven
     `6.16.13` and `7.21.2` sections, changing its source pointer away from
     metadata parsing to `crates/splot-recon/src/hash_input.rs`, and
     regenerating `docs/DECODER-SUPPORT-STATUS.md`.
  2. `design.md` still used a stale `digest` method name. Fixed to
     `DecodedFrameHashInput::compute_hash()`.
  Follow-up `cargo xtask check-decoder-support`,
  `cargo xtask check-feature-status`,
  `openspec validate recon-frame-hash-digest --strict`, and
  `openspec validate --all --no-interactive` passed.
- Final docs/OpenSpec consistency reviewer `Herschel the 3rd`
  (`019ec650-bb18-7991-b07e-89cf7d4ee533`) found two issues:
  1. Verification tasks were marked complete without enough matching evidence in
     this log. Fixed by adding the required gate list and boundary-scan summary
     above.
  2. `design.md` still used a stale `digest` method name. This had already
     been fixed to `DecodedFrameHashInput::compute_hash()` after the
     AV2/spec/status review.
  The reviewer did not complete remote PR #113 verification before being
  interrupted for status; the orchestrator and code/API reviewer did verify PR
  #113 review `4492663492` and its four local carry-forward fixes.

## Final Acceptance

- Archived successfully with
  `openspec archive recon-frame-hash-digest --yes`. The command updated
  `openspec/specs/decoder-support/spec.md` and moved the change to
  `openspec/changes/archive/2026-06-14-recon-frame-hash-digest/`.
- Post-archive active-spec verification confirmed the
  `Decoded frame hash input serialization` requirement now includes
  `splot-dfh-sha256-v1` digest computation, stable digest identifiers, 32-byte
  raw digest access, lowercase hex formatting, and the explicit non-goals for
  AV2 metadata MD5 verification, runtime decode, output ordering, film grain,
  Y4M, AVM/dav2d invocation, and CI reference-tool requirements.
- Post-archive `openspec validate --all --no-interactive` passed.
- Post-archive full gate `cargo xtask ci` passed.
- Remaining PR, latest-head Codex review, CI, and merge gating is tracked by
  the implementation branch process, not by this archived OpenSpec change.
