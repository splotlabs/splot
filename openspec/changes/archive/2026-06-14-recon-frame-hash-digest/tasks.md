## 1. Planning and Agent Log

- [x] 1.1 Maintain `agent-log.md` with orchestrator plan, every subagent and
      sub-subagent role, findings, implementation notes, review fixes, and final
      acceptance.
- [x] 1.2 Validate `openspec validate recon-frame-hash-digest --strict` before
      creating the implementation branch.

## 2. Runtime API

- [x] 2.1 Add the existing workspace `sha2` dependency to `splot-recon` and
      keep the decision visible in docs/agent-log/PR evidence.
- [x] 2.2 Add a typed `DecodedFrameHash` value with stable contract,
      algorithm, byte-stream, variant, byte length, raw-byte, lowercase-hex, and
      display behavior.
- [x] 2.3 Add an infallible `DecodedFrameHashInput::compute_hash` or equivalent
      digest method that hashes the same visible sample byte stream without
      allocating or changing `write_to` / `byte_len` error behavior.
- [x] 2.4 Export the digest API from `splot-recon` and update crate/module docs
      so they no longer say digest computation is unimplemented.

## 3. Tests

- [x] 3.1 Add self-contained `splot-recon` tests for stable identifiers, raw
      bytes, lowercase hex, and `Display` formatting.
- [x] 3.2 Add SHA-256 vector tests for monochrome visible rows, 8-bit `u16`
      storage, 10-bit little-endian samples, 10-bit YUV420 order, and odd-size
      YUV420 chroma dimensions.
- [x] 3.3 Add a drift guard proving digest computation matches SHA-256 over
      bytes emitted by `DecodedFrameHashInput::write_to`.
- [x] 3.4 Add or update tests proving output metadata, stride, and padding do
      not affect the digest when visible bytes match.

## 4. Docs and Status

- [x] 4.1 Add `RECON-FRAME-HASH-DIGEST` to
      `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [x] 4.2 Update `docs/DECODER-ROADMAP.md` hash policy/current-state text for
      source-backed digest computation while keeping runtime decode/Y4M/AVM
      non-goals explicit.
- [x] 4.3 Update `docs/DECODER-SUPPORT-MATRIX.toml` deterministic-frame-hash row
      status, Feature ID, tests, module notes, dependency boundary, and local
      evidence language without overclaiming runtime decode.
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
      spec-conformance-reviewer, encoder-impact-reviewer, and dependency/boundary
      review; fix or explicitly close all findings in `agent-log.md`.
- [x] 6.2 Confirm tasks/proposal/design/spec match implementation reality, then
      run `openspec validate recon-frame-hash-digest --strict`.
- [x] 6.3 Archive the OpenSpec change and verify the active
      `openspec/specs/decoder-support/spec.md` delta.
- [x] 6.4 Move post-archive PR, CI, latest-head Codex review, and merge gating
      into the PR process for the implementation branch.
