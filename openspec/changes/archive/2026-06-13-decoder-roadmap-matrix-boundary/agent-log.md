# Agent Log

## Orchestrator Plan

Objective: execute the first PR-sized decoder mission item,
`decoder-roadmap-matrix-boundary`, before any decoder crate, dependency graph
change, or pixel reconstruction work. This change establishes docs, a canonical
decoder support matrix, generated status docs, and CI drift checks. It must not
invoke, locate, require, or integrate AVM/dav2d.

Baseline:

- Worktree started clean on detached `HEAD` at `44ce7bda6589baeb66ef2e05117c589ea43b469c`.
- `git fetch` required escalation because this linked worktree writes FETCH_HEAD
  outside the sandbox; rerun succeeded.
- `cargo xtask ci` passed before changes.
- `openspec list` showed active changes: `validator-context-split` complete,
  `avm-differential-harness`, `toy-intra-encoder-v0`, and
  `add-bitstream-writer`.
- `gh pr list --state open --limit 30` returned no open PRs.
- `cargo xtask audit-scope --format json` selected 462 candidates because
  `docs/IMPLEMENTATION-MATRIX.toml` is a wide-review trigger.

## Subagents and Sub-subagents

### @architect with @spec-reader and @api-designer

Agent id: `019ec076-bb7b-7340-975a-fcfab5cacda3`

Objective: map current architecture and propose a decoder/reconstruction
backlog.

Findings:

- `splot-cli/src/commands/decode.rs` is a stub.
- No decoder or reconstruction crate exists.
- Existing parser rows that future decoder work can reuse include
  `AV2-5.18.2-FRAME-HEADER-INFO`, `AV2-5.19-TILE-GROUP`,
  `AV2-5.20-TILE-GROUP-PAYLOAD`, `AV2-7.23-REFERENCE-FRAME-UPDATE`,
  `AV2-9-ADDITIONAL-TABLES`, and `AV2-IVF-CONTAINER`.
- Minimal decoder work is anchored in AV2 § 7.1, § 5.20, § 8.2/§ 8.3,
  § 7.13-§ 7.15, § 7.17-§ 7.21, and § 7.23.
- Recommended later crate split: `splot-decode -> splot-core`, with
  `splot-cli -> splot-decode`; this requires explicit dependency-graph approval.

### @reference-oracle with @avm-reader-runner and @dav2d-reader-runner

Agent id: `019ec076-f351-7e20-95dc-3cc62a21be71`

Objective: inspect local AVM/dav2d evidence without editing, building, or
downloading.

Findings:

- AVM commit: `f6f0b9c8914f38be39a953c0a9aa6a2e4050717c`; local status clean.
- dav2d commit: `f4f96cb06bb3cd3f31e29e1f190f1c0e373ab352`; local status has
  untracked `subprojects/.wraplock` and `subprojects/checkasm/`.
- Built AVM binaries observed locally: `avmenc`, `avmdec`, `dump_obu`,
  `decode_to_md5`.
- Built dav2d binary observed locally: `dav2d`.
- Local raw MD5 evidence matched for:
  - `tests/conformance/vectors/valid/syn-key-intra-64x64.ivf`:
    `f2d45ae552bebe211f3156daf0a7fcf6`
  - `tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf`:
    `6c9c31585f56bcc7ca40cfbb319f7bb5`
- This is local evidence only. It must not become a repo command, CI
  requirement, dependency, wrapper, or absolute-path manifest.

### @security-reviewer

Agent id: `019ec077-0feb-77f2-89da-638ad1d42ad8`

Objective: threat-model future decode/reconstruction paths.

Findings:

- Decode must start with explicit limits before any pixel allocation.
- Geometry, stride, frame buffer size, tile scratch size, and reference buffer
  sizes need checked arithmetic and typed resource-limit errors.
- Decode state must be transactional: malformed frames must not update reference
  state or emit partial output.
- First byte-consuming decode API must get property/fuzz coverage.
- No repo code may invoke AVM, dav2d, ffmpeg, or other external decoders.

### @encoder-impact-reviewer

Agent id: `019ec077-2ab7-7741-a251-3a5c3a7eb6bc`

Objective: identify only decoder/reconstruction pieces useful for the future
encoder.

Findings:

- Shared decoded frame and plane model should precede encoder growth.
- Decoded-frame hash verification is useful once pixel output exists.
- Validator-private reference state is not enough; future decoder needs pixel
  reference slots.
- Tile payload decode boundary and scalar reconstruction primitives are the
  encoder-useful path.
- Do not start RDO, motion estimation, rate control, or filter search before
  scalar reconstruction and reference state exist.

## Implementation Log

- OpenSpec proposal, design, task list, and delta specs were created under
  `openspec/changes/decoder-roadmap-matrix-boundary/`.
- Backlog item #1 was printed before implementation:
  `decoder-roadmap-matrix-boundary`, followed by future decoder API, decoded
  frame/hash, CLI, stream-state, fuzz, symbol decoder, intra, output/reference,
  local-reference evidence, and encoder-recon API items.
- Branch created from fetched `origin/main`:
  `feat/decoder-roadmap-matrix-boundary`.
- Added docs:
  - `docs/DECODER-ROADMAP.md`
  - `docs/DECODER-SUPPORT-MATRIX.toml`
  - generated `docs/DECODER-SUPPORT-STATUS.md`
- Updated docs:
  - `README.md`
  - `docs/SPEC-MAPPING.md`
  - `docs/TESTING.md`
- Added implementation-matrix rows:
  - `DOC-DECODER-ROADMAP`
  - `DOC-DECODER-SUPPORT-MATRIX`
  - `XTASK-DECODER-SUPPORT-STATUS`
  - `CLI-DECODE`
- Regenerated:
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
- Implemented xtask automation:
  - `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`
  - `cargo xtask check-decoder-support`
  - `cargo xtask ci` now runs the decoder-support drift check.
- `xtask/src/decoder_support.rs` validates required fields, allowed statuses,
  duplicate row ids, supported-row proof, local absolute path leaks,
  self-contained proof entries that mention external decoder tools, portable
  local-reference evidence, and generated status drift. It never locates,
  builds, invokes, or requires AVM/dav2d.

### @implementer / @integration-implementer

Agent id: `019ec082-6fcf-7c10-abfe-417cf3555b44`

Objective: implement the xtask automation slice.

Changed:

- `xtask/src/main.rs`
- `xtask/src/decoder_support.rs`

Verification reported by agent:

- `cargo test -p xtask decoder_support --locked`
- `cargo test -p xtask --locked`
- `cargo clippy -p xtask --all-targets --locked -- -D warnings`
- `cargo xtask check-decoder-support`
- `cargo xtask ci`

## Review Log

- `@reviewer` agent `019ec08d-900b-7372-b893-61b26a39337f` found:
  - stale support-matrix proof test name;
  - stale `xtask/src/decoder_support.rs` module doc saying the command renders
    the TOML matrix rather than the generated status document.
  Fixes:
  - proof test updated to
    `decoder_support::tests::supported_row_requires_test_or_fixture`;
  - module doc corrected;
  - `docs/DECODER-SUPPORT-STATUS.md` regenerated.
- `@spec-conformance-reviewer` agent `019ec08d-95b1-79e2-880e-b13502a95102`
  found:
  - `CLI-DECODE` overclaimed `decode_check = "partial"` while `splot decode`
    remains a stub.
  Fix:
  - changed `CLI-DECODE` to `decode_check = "todo"`;
  - regenerated `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- `@encoder-impact-reviewer` agent `019ec08d-989f-7b42-8dcc-bd6d4314975d`
  found:
  - stale support-matrix proof test name;
  - future `reference-frame-store` row pointed at validator-private
    `crates/splot-validate/src/reference_state.rs`;
  - supported-row proof validation blocked AVM/dav2d but not other external
    decoders such as ffmpeg or dav1d.
  Fixes:
  - proof test updated;
  - `reference-frame-store.parser_source` changed to `planned`;
  - external decoder denylist added for self-contained tests/fixtures, with
    `supported_proof_rejects_external_decoders` unit coverage.
- `@security-reviewer` agent `019ec08d-9323-7c52-be06-5c3bb79ab8d8` found:
  - preventive path-leak gap: only `local_reference_evidence` rejected local
    absolute paths.
  Fix:
  - all rendered matrix fields now reject local absolute paths and `file://`
    local paths, including `tier` and `last_reviewed`, with
    `rendered_fields_reject_local_absolute_paths` and
    `metadata_rejects_local_absolute_paths` unit coverage.

Final review sign-offs:

- `@reviewer`: no findings after fixes; signed off.
- `@security-reviewer`: no findings after final path-leak fix; signed off.
- `@spec-conformance-reviewer`: no findings after fixes; signed off.
- `@encoder-impact-reviewer`: no findings after fixes; signed off.
- `@test-writer` agent `019ec096-fc94-75f1-87ea-16e7bc4f3fb1` found:
  - `decoder-status-drift-gate` support-matrix proof cited the supported-row
    policy test instead of the drift test.
  Fix:
  - row proof changed to
    `decoder_support::tests::check_decoder_support_detects_drift`;
  - `docs/DECODER-SUPPORT-STATUS.md` regenerated.
- `@documenter` agent `019ec096-ffbf-72c0-becf-a44d73f9bdbd` found:
  - `decoder-status-drift-gate` support-matrix source pointed at
    `xtask/src/main.rs`, while the implementation matrix pointed at
    `xtask/src/decoder_support.rs`.
  Fix:
  - row source changed to `xtask/src/decoder_support.rs`;
  - `docs/DECODER-SUPPORT-STATUS.md` regenerated.
- `@test-writer`: no findings after proof-row fix; signed off.
- `@documenter`: no findings after source-trail fix; signed off.

Post-fix verification:

- `cargo test -p xtask decoder_support --locked`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `openspec validate decoder-roadmap-matrix-boundary --strict`
- `cargo xtask ci`
- After the final test-writer/documenter fixes, `cargo xtask ci` passed again.

Boundary review result:

No AVM/dav2d source, snippets, binaries, submodules, dependencies, build probes,
wrappers, CI jobs, required scripts, required xtask commands, or mandatory tests
were added. Local AVM/dav2d evidence remains metadata in this log and the
decoder support matrix/status; committed tests and CI do not require those
tools.

## Final Acceptance

- Implementation and review are complete for
  `decoder-roadmap-matrix-boundary`. The change is ready for OpenSpec archive.
