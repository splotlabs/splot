# Agent Log: decode-byte-stream-planner

## Orchestrator Plan

- Branch: `codex/decode-byte-stream-planner`.
- Feature ID: `DECODE-BYTE-STREAM-PLANNER`.
- Goal-owned PR only; other users' PRs do not block this work.
- Ready PRs only. Do not create a draft PR unless Bartosz explicitly asks for a
  draft.
- Merge gate: request Codex review and wait for explicit approval/thumbs-up or
  final review sign-off. A reaction such as "eyes" is not approval. Do not merge
  before that gate.
- Incorporate the PR #101 concurrency model: raw-byte planning must run through
  `DecodeContext`'s single owned `splot_parallel::WorkerPool`; no direct Rayon,
  crossbeam, global pool, or ad-hoc threads/queues.
- AVM/dav2d boundary: this slice must not locate, build, run, wrap, depend on,
  or commit metadata from AVM/dav2d. Local reference evidence is not required
  for byte traversal without decoded output claims.

## Agents Invoked

| Agent | Role | Objective | Output |
|---|---|---|---|
| @architect | Planning subagent | Design no-new-dependency byte-stream planner architecture and concurrency boundary. | Completed. Recommended `crates/splot-decode/src/byte_stream.rs`, a `DecodeContext` byte-planning method running in `WorkerPool::install`, bounded Annex B/IVF traversal with public `splot-core` primitives, equivalence tests against parsed planning, and fuzz target `decode_plan_bytes`. |
| @spec-reader | Planning sub-subagent | Extract pinned AV2 spec requirements and citation strings. | Completed. Required citations: Annex B.2/B.3, §4.11.6, §5.2.1, §5.2.2, §6.2.1, §6.2.2, §7.1, §7.3/§7.3.6/§7.3.7/§7.3.8; IVF is local container policy, not AV2 normative syntax. |
| @api-designer | Planning sub-subagent | Recommend API/error/test shape for raw byte planning. | Completed. Recommended public `DecodeContext::plan_bytes(&[u8], DecodeOptions) -> Result<DecodeStreamPlan>`, existing error variants, no CLI wiring, and tests for parsed equivalence, limits, malformed sources, unsupported structures, and thread determinism. |
| @reference-oracle | Reference subagent | Decide whether local AVM/dav2d evidence is needed. | No local reference evidence required; do not commit AVM/dav2d material or manifest entries for this planner-only change. |
| @module-implementer | Implementation sub-subagent | Review focused `splot-decode` module implementation. | Signed off with no implementation findings. |
| @integration-implementer | Implementation sub-subagent | Review crate/fuzz/docs integration and dependency direction. | Signed off with no integration findings. |
| @test-writer | Test subagent | Audit unit, limit, malformed, determinism, and fuzz coverage. | Initially found missing unsupported-structure coverage, IVF EOF coverage, error determinism coverage, and `max_frames_to_decode` precedence coverage. Re-review signed off after fixes. |
| @fuzz-author | Test sub-subagent | Review fuzz target shape and external-decoder boundary. | Signed off before the final static-context change; later final reviewer verified the static `DecodeContext` fix and fuzz metadata. |
| @documenter | Documentation subagent | Review docs, matrices, and generated status updates. | Initially found stale `AGENTS.md` fuzz-target count and later stale proposal dependency wording. Re-review signed off after both fixes. |
| @reviewer | Review subagent | Final implementation review. | Initially found per-input worker-pool creation in the fuzz target and stale proposal dependency wording. Re-review signed off after both fixes. |
| @security-reviewer | Review sub-subagent | Review untrusted input, panics, memory, dependencies, and external-tool boundary. | Signed off with no security findings. |
| @spec-conformance-reviewer | Review sub-subagent | Review spec citations and AV2/non-normative IVF boundaries. | Signed off with no spec-conformance findings. |
| @encoder-impact-reviewer | Review sub-subagent | Review future encoder usefulness and concurrency direction. | Signed off with no encoder-impact findings. |

## Local Reference Evidence

Not used. The @reference-oracle pass completed without running external
decoders or editing files. This change is a byte traversal and planning
boundary only: no decoded pixels, tile payload symbols, frame hashes, Y4M,
reconstruction, reference refresh, decoded-output equivalence, or fixture
expectation is derived from reference decoder output.

Do not commit AVM/dav2d source, binaries, submodules, copied snippets,
wrappers, build probes, scripts, `xtask` commands, CI hooks, required tests,
absolute local paths, decoded output files, Y4M files, reference hashes, or
`LOCAL-REFERENCE-EVIDENCE.toml` entries for this change. Revisit reference
evidence at the first milestone that claims decoded output behavior:
deterministic frame-hash verification, pixel reconstruction, Y4M output, or a
portable local-reference evidence manifest.

## Review Notes

- Implementation notes:
  - Added `crates/splot-decode/src/byte_stream.rs` with bounded raw Annex B and
    IVF/DKIF traversal.
  - Added `DecodeContext::plan_bytes(&[u8], DecodeOptions)`.
  - Reused existing `DecodeStreamPlan`, `DecodeError`, and parsed planner
    classification.
  - Added `fuzz/fuzz_targets/decode_plan_bytes.rs` and wired
    `fuzz/Cargo.toml`.
  - Updated decoder roadmap, testing docs, spec mapping, decoder support matrix,
    generated decoder support status, implementation matrix, generated feature
    status, and generated spec coverage.
- Focused verification passed:
  - `openspec validate decode-byte-stream-planner --strict`
  - `cargo test -p splot-decode --locked`
  - `cargo check --manifest-path fuzz/Cargo.toml --bins`
  - `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`
  - `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`
  - `cargo fmt --all -- --check`
  - `cargo xtask check-feature-status`
  - `cargo xtask check-decoder-support`
  - `cargo xtask check-concurrency-policy`
  - `cargo xtask check-dependency-direction`
- Review findings resolved:
  - @test-writer finding: the byte planner did not cover every parsed-planner
    unsupported-structure branch, IVF EOF branch, deterministic error output,
    or `max_frames_to_decode` precedence before later malformed bytes.
    Resolution: added the missing unsupported, EOF, deterministic-error, and
    frame-limit precedence tests; enforced the frame-candidate limit during
    byte traversal before retaining the next accepted CLK candidate.
  - @reviewer finding: the fuzz target created and dropped a worker pool for
    every fuzz input. Resolution: changed the target to reuse a static
    one-thread `DecodeContext` through `OnceLock<Option<DecodeContext>>`.
  - @documenter finding: `AGENTS.md` still listed four fuzz targets. Resolution:
    updated the target list to include `decode_plan_bytes`.
  - @documenter/@reviewer finding: `proposal.md` claimed no new `splot-*`
    dependency edge after `fuzz/Cargo.toml` gained direct local path
    dependencies on `splot-decode` and `splot-parallel`. Resolution: scoped the
    impact statement to no new production/workspace crate edge and explicitly
    called out the fuzz-only local path dependencies.
- Final review status:
  - @test-writer signed off with no remaining test coverage findings.
  - @documenter signed off after the proposal dependency wording correction.
  - @reviewer signed off after the static fuzz context and proposal dependency
    wording corrections.
  - @module-implementer, @integration-implementer, @security-reviewer,
    @spec-conformance-reviewer, and @encoder-impact-reviewer signed off with no
    remaining findings.
- Additional verification after review fixes:
  - `openspec validate decode-byte-stream-planner --strict`
  - `openspec validate --all --no-interactive`
  - `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`
  - `cargo xtask check-fuzz-targets`
  - `cargo xtask check-concurrency-policy`
  - `cargo xtask check-dependency-direction`
