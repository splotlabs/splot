# Agent Log: decode-stream-state-planner

## Orchestrator Plan

- Current branch state: detached at `origin/main` commit
  `0ef2be79742074bb0d79fa3ab866fc1d7e26db01`, clean before the change
  scaffold.
- Mission slice: implement the existing `decode-stream-state` support-matrix
  row as a PR-sized parsed stream planner in `splot-decode`.
- Branch rule: create and validate OpenSpec artifacts first, then create the
  `codex/decode-stream-state-planner` branch.
- PR rule: open a ready PR by default. Do not create a draft PR unless the user
  explicitly asks for draft.
- Merge rule: wait for Codex review/thumbs-up and green checks before merging.

## Planning Agents

### @architect

- Agent id: `019ec4ec-a4ad-7313-84fc-3af18b4c616a`
- Objective: propose the smallest PR-sized architecture and concurrency model.
- Findings:
  - Add Feature ID `DECODE-STREAM-STATE-PLANNER`.
  - Implement a source-backed planner in `crates/splot-decode/src/stream_plan.rs`.
  - Add `splot-core` to `splot-decode`; do not add `splot-recon` or
    `splot-validate`.
  - Root the API in `DecodeContext` so the PR #101 worker-pool model owns future
    scheduling.
  - Keep the first planner serial and deterministic across thread policies.
  - Keep `splot decode` CLI unchanged for this PR.

### @spec-reader

- Agent id: `019ec4ec-dc39-7b71-ac2b-14da3748b6f4`
- Objective: extract pinned AV2 stream traversal requirements.
- Findings:
  - Use only the repo-supported raw Annex B and IVF-wrapped Annex B container
    model.
  - Preserve AV2 bitstream order per AV2 Section 7.1.
  - Keep the first planner base-only and reject multistream/MSDO/LCR/OPS/Atlas
    paths as unsupported.
  - Treat temporal-unit and random-access behavior conservatively. OLK/RAS and
    other stateful paths remain blocked until later decode state exists.
  - Reconstruction, hashes, Y4M, tile payload decode, and reference refresh stay
    unsupported.

### @api-designer

- Agent id: `019ec4ec-e0df-7a90-9750-726c5ebdcd87`
- Objective: propose the Rust API surface.
- Findings:
  - Expose a parsed-input API:
    `DecodeContext::plan_stream(DecodeStreamInput, DecodeOptions)`.
  - Return `DecodeStreamPlan` with private fields and metadata getters.
  - Add typed library errors for limits, malformed source, and unsupported
    structure.
  - Check only `max_input_bytes`, `max_obus`, `max_ivf_frame_records`, and
    `max_frames_to_decode` in this slice.
  - Keep raw payload slices private or absent from public plan records.
  - Keep CLI diagnostics unchanged until a future diagnostic adapter PR.

### @security-reviewer

- Agent id: `019ec4ec-e576-7521-9a3c-957527228164`
- Objective: threat-model untrusted input and parser/planner limits.
- Findings:
  - Do not implement raw-byte planning by calling `parse_bitstream_partial` and
    then checking limits; current partial parsers allocate vectors before
    `splot-decode` can enforce `max_obus`.
  - For this PR, keep the planner parsed-input only and document that it is not a
    byte-consuming decode boundary.
  - Reject malformed parser output transactionally.
  - Do not blanket-call `payload_status()` on every OBU.
  - Store only offsets, sizes, layer IDs, roles, and source context in plan
    records.
  - Add fuzz coverage when the first raw byte-consuming decode planner is added,
    not in this parsed-input PR.

### @encoder-impact-reviewer

- Agent id: `019ec4ec-ea8c-7d73-a1d7-1567d0dc4537`
- Objective: ensure the planner helps future encoder closed-loop work.
- Findings:
  - The planner should be an ordered work-plan boundary, not playback plumbing.
  - Avoid IVF timestamp/timebase scheduling, Y4M policy, display timing, and
    "best stream" heuristics.
  - Keep scheduler/state out of `splot-recon`.
  - Test identical plans across `--threads 1`, `auto`, and a fixed positive
    count.

### @reference-oracle

- Agent id: `019ec4ec-efed-7fe1-81f5-ae43d7f77066`
- Objective: decide whether local AVM/dav2d evidence is needed.
- Findings:
  - No AVM/dav2d evidence is required for this slice because it does not claim
    decoded output bytes, hashes, Y4M, reconstruction, or reference refresh.
  - Proof should be self-contained `splot` tests over `splot-core` parse output
    and decoder planner metadata.
  - Do not locate, build, run, wrap, or commit metadata from AVM/dav2d in this
    slice.

## Local Reference Evidence

None. AVM and dav2d were not run and are not needed for this parsed planner
slice.

## Boundary Statement

No AVM/dav2d source, snippets, binaries, submodules, dependencies, build probes,
wrappers, CI jobs, required scripts, required `xtask` commands, or mandatory
tests are added by this change.

## Review Log

- General reviewer `019ec501-e5d1-72a1-b043-1f66386bc2fe`: sign-off, no
  findings. Verified the split `stream_plan.rs` / `stream_plan/tests.rs`
  planner shape, typed errors, context-owned worker-pool boundary,
  deterministic thread-policy coverage, and source-line budget.
- Encoder-impact reviewer `019ec501-f36a-7b73-a42b-644e2a121267`: sign-off, no
  findings. Confirmed the planner is an ordered deterministic decode work-plan
  boundary useful for future encoder closed-loop work and that PR #101
  concurrency policy is incorporated through `DecodeContext` / `WorkerPool`.
- Security reviewer `019ec501-e9bf-79e3-abba-e036326f5c69`: initial P3 found
  unbounded parsed IVF frame-record traversal when many empty frame payloads
  precede any OBU. Resolution: added first-class
  `DecodeLimitName::MaxIvfFrameRecords` / `DecodeLimits` API, enforced it before
  each parsed IVF frame record, added an empty-IVF-frame regression, and updated
  OpenSpec/docs/matrix/generated status. Focused re-review signed off with no
  blocking findings.
- Spec-conformance reviewer `019ec501-ee97-77d1-8db3-39405762eb22`: initial
  sign-off withheld because the planner accepted invalid § 6.2.2 xlayer scopes
  and roadmap text overclaimed operating-point selection. Resolution: enforce
  `ObuType::{requires_global_xlayer, permits_global_xlayer}` before role
  classification, add invalid-scope tests for non-global TD and global
  CLK/sequence-header, reword Stage 4 as base-layer parsed traversal with
  operating-point selection rejected/planned, add § 6.2.2 to the decoder support
  row, and regenerate generated docs. Focused re-review signed off.
- Verification before archive: `cargo xtask ci` passed, including fmt, clippy,
  build, workspace tests, doctests, rustdoc with warnings denied, typos,
  machete, deny, OpenSpec, source-line, dependency-direction, concurrency
  policy, feature-status, decoder-support, reference-evidence, and diagnostic
  registry checks. Existing cargo-deny unmatched-license warnings and existing
  source-line advisories remain non-blocking.
