## 1. Planning and Agent Log

- [x] 1.1 Maintain `agent-log.md` with orchestrator plan, every required
      subagent and sub-subagent role, findings, implementation notes, review
      fixes, and final acceptance.
- [x] 1.2 Validate `openspec validate recon-y4m-output-writer --strict` before
      implementation.

## 2. Y4M API

- [x] 2.1 Add `crates/splot-recon/src/y4m.rs` with typed frame-rate, frame
      format, chroma tag, stream header, frame header, writer, result, and
      error APIs.
- [x] 2.2 Serialize stream headers and frame headers using visible dimensions,
      progressive output, caller-supplied frame rate, and pinned Y4M chroma
      tags for modeled AV2 formats.
- [x] 2.3 Serialize frame payloads from `DecodedFrame<T>` visible rows in Y/U/V
      order, with 8-bit one-byte samples and 10-bit little-endian sample pairs.
- [x] 2.4 Reject invalid frame rates, unsupported formats, and stream/frame
      mismatches with typed errors before writing frame payload bytes.
- [x] 2.5 Export the Y4M API from `splot-recon` and update crate/module docs
      without changing `splot-decode` or `splot-cli` runtime behavior.

## 3. Tests

- [x] 3.1 Add exact stream-header tests for monochrome, YUV420, YUV422, and
      YUV444 in 8-bit and 10-bit forms.
- [x] 3.2 Add frame payload tests for visible crop/stride exclusion, Y/U/V
      order, monochrome Y-only output, odd-size 4:2:0 chroma, 8-bit `u16`
      storage, and 10-bit little-endian samples.
- [x] 3.3 Add multi-frame tests proving the stream header is written once and
      each accepted frame gets one `FRAME\n` header.
- [x] 3.4 Add negative tests for invalid frame rates, stream/frame format
      mismatch without payload writes, and propagated writer I/O errors.

## 4. Docs and Status

- [x] 4.1 Add `RECON-Y4M-OUTPUT-WRITER` to
      `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [x] 4.2 Update `docs/DECODER-ROADMAP.md` for source-backed library Y4M
      writing while keeping runtime decode/Y4M/AVM non-goals explicit.
- [x] 4.3 Update `docs/DECODER-SUPPORT-MATRIX.toml` `output-y4m` row status,
      Feature ID, tests, source module, and non-overclaiming notes.
- [x] 4.4 Regenerate generated decoder/feature/spec status docs as required by
      repo tooling.

## 5. Verification

- [x] 5.1 Run focused `splot-recon` tests and clippy.
- [x] 5.2 Run `cargo xtask check-dependency-direction` and
      `cargo xtask check-concurrency-policy`.
- [x] 5.3 Run `cargo xtask check-feature-status`,
      `cargo xtask check-decoder-support`, and
      `openspec validate --all --no-interactive`.
- [x] 5.4 Run `cargo xtask ci`.
- [x] 5.5 Run an AVM/dav2d boundary scan proving no reference source, snippets,
      binaries, wrappers, build probes, CI jobs, runtime process execution,
      mandatory tests, or local paths were added.

## 6. Review, Archive, and PR

- [x] 6.1 Run final review subagents: reviewer, security-reviewer,
      spec-conformance-reviewer, encoder-impact-reviewer, and
      dependency/concurrency review; fix or explicitly close all findings in
      `agent-log.md`.
- [x] 6.2 Confirm tasks/proposal/design/spec match implementation reality, then
      run `openspec validate recon-y4m-output-writer --strict`.
- [x] 6.3 Archive the OpenSpec change and verify the active
      `openspec/specs/decoder-support/spec.md` delta.
- [x] 6.4 Move post-archive PR, CI, latest-head Codex review, and merge gating
      into the PR process for the implementation branch.
