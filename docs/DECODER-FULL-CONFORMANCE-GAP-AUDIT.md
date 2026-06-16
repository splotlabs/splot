# Decoder Full Conformance Gap Audit

`status: step-0-audit`
`created: 2026-06-15`
`owner: decoder`

This audit is the read-only kickoff artifact for the full AV2 v1.0.0 decoder
conformance program. It records the local checkout state before any feature
implementation and uses the committed AV2 spec mirror as the section index:
`docs/spec/av2/1.0.0/index.md`.

The current conclusion is deliberately narrow: the baseline is green, but
`splot decode` is still a plan-only, intentionally unsupported runtime entry
point. Full decoder conformance is not currently implemented or claimed.

## Step 0 Baseline

Baseline commands were run from the repository root on 2026-06-15.

| Command | Result |
|---|---|
| `cargo xtask audit-scope --format json` | Passed. Wide audit scope selected 842 candidates at commit `7e6d9764a8f20943bef43b8c41891bc8e21fa1c2`. |
| `git fetch --all --prune` | Passed after sandbox escalation, because this worktree stores git metadata outside the writable sandbox. |
| `git status --short` | Clean. |
| `gh pr list --state open` | No open pull requests returned. |
| `openspec list` | Active changes: `validator-context-split` complete, `avm-differential-harness` 0/5 tasks, `toy-intra-encoder-v0` 0/6 tasks. |
| `cargo xtask ci` | Passed: fmt, clippy, build, tests, doctests, rustdoc, typos, machete, deny, OpenSpec, license headers, source-line check, dependency/concurrency/spec/fuzz/table/feature/reference/decoder/diagnostic/fixture checks. |
| `openspec validate --all --no-interactive` | Passed: 14 items. |
| `cargo xtask check-decoder-support` | Passed: 32 decoder support rows and 2 reference evidence entries. |
| `cargo xtask check-diagnostic-registry` | Passed: validator 249 ids, decoder 3 ids. |
| `cargo xtask check-dependency-direction` | Passed. |
| `rg -n 'todo!\\(\|unimplemented!\\(\|unreachable!\\(\|panic!\\(\|expect\\(\|unwrap\\(\|Unsupported\|unsupported-feature\|partial\|todo' crates docs openspec fuzz tests xtask` | Produced expected hits in tests, docs, matrices, and supported diagnostic/unsupported-feature paths. No current baseline failure. |

The baseline is not red, so the first backlog item is not
`fix-decoder-baseline-gate`.

## Current Crate And Module Map

| Area | Current responsibility | Decoder conformance impact |
|---|---|---|
| `crates/splot-core` | AV2 bitstream/container model, Annex B/IVF, OBU headers, syntax parsers, generated tables, and generic symbol primitives. | Parser and table source of truth, but parser ownership alone is not runtime decode ownership. |
| `crates/splot-parallel` | Approved worker-pool and bounded-queue primitives. | Runtime decoder work must route parallelism through this crate. |
| `crates/splot-recon` | Decoded frame/plane model, reference-slot container, deterministic frame-hash input/digest, Y4M writer for caller-supplied frames, current-frame workspace, and a few intra prediction primitives. | Reusable reconstruction/output primitives exist, but they are not wired into `splot decode`. |
| `crates/splot-decode` | Decode diagnostics, limits API, runtime context, byte/parsed stream planning, and crate-private tile-payload boundary planning. | It is plan-only and has no dependency on `splot-recon` yet. |
| `crates/splot-validate` | Parser-driven conformance diagnostics. | Validator remains separate from runtime decode. |
| `crates/splot-cli` | Thin command entry points and file I/O. | `decode` reads input and renders diagnostics, but intentionally does not write output. |
| `fuzz` | Bounded fuzz targets for parser/validator/decode planning surfaces. | `decode_plan_bytes` covers the current byte planner only. |
| `xtask` | Repository gates, generated docs, feature/matrix/evidence checks. | Current gates keep status honest but do not yet enforce full decoder spec coverage. |

Current `splot-decode` modules:

- `byte_stream.rs`: raw Annex B / IVF byte planning.
- `context.rs`: context-owned worker pool and plan entry points.
- `diagnostic.rs`: decoder diagnostic adaptation.
- `error.rs`: decode error model.
- `limits.rs`: decode resource limit API.
- `runtime.rs`: runtime configuration.
- `stream_plan.rs`: parsed stream planning and layer/OBU selection.
- `tile_payload.rs`, `tile_payload/cdf.rs`, `tile_payload/input.rs`: crate-private tile payload boundary and partition CDF subset.

Current `splot-recon` modules:

- `format.rs`, `geometry.rs`, `plane.rs`, `frame.rs`: frame and plane model.
- `hash_input.rs`: deterministic decoded-frame hash input and digest.
- `reference.rs`: reference-slot container.
- `workspace.rs`: current-frame workspace.
- `intra.rs`, `intra_basic.rs`, `intra_smooth.rs`: partial intra prediction primitives.
- `y4m.rs`: Y4M writer for caller-supplied decoded frames.

## Public CLI Behavior

`splot decode` is plan-only. `crates/splot-cli/src/commands/decode.rs` resolves
the future output target, intentionally does not write to it, reads bounded input
bytes, constructs `DecodeContext`, calls `DecodeContext::plan_bytes`, renders one
diagnostic, and returns exit code 1.

Observed local behavior:

| Command | Result |
|---|---|
| `cargo run -p splot-cli -- decode tests/conformance/vectors/valid/syn-key-intra-64x64.ivf --output-format hash --json` | Exit 1 with `decode/unsupported-feature`, `detail_kind = "runtime_unsupported"`, `spec_section = "7.1"`, `matrix_row = "cli-decode-entrypoint"`, `input_len_bytes = 140`, `obu_count = 3`, `frame_candidate_count = 1`. |
| `cargo run -p splot-cli -- decode tests/conformance/vectors/invalid/syn-key-intra-64x64-truncated.ivf --output-format hash --json` | Exit 1 with `decode/malformed-source`, `detail_kind = "malformed_source"`, `parser_rule_id = "ivf/truncated-frame-payload"`, `byte_offset = 100`, `ivf_frame_index = 0`. |
| `cargo run -p splot-cli -- decode --help` | Lists `--output-format y4m|hash`, `-o/--output`, `--json`, and `--threads`; descriptions still identify output formats as future artifacts. |

There is no successful runtime decode output path yet.

## Decoder Support Rows

`docs/DECODER-SUPPORT-STATUS.md` reports 32 rows: 22 `supported`, 9 `partial`,
1 `unsupported-intentional`, 0 `todo`, and 0 `blocked`.

| Row | Tier | Status |
|---|---|---|
| `decoder-roadmap` | `foundation` | `supported` |
| `decoder-support-matrix` | `foundation` | `supported` |
| `decoder-status-drift-gate` | `foundation` | `supported` |
| `local-reference-evidence-manifest` | `foundation` | `supported` |
| `decoder-crate-scaffolding` | `foundation` | `supported` |
| `decode-runtime-context` | `foundation` | `supported` |
| `decoded-frame-plane-runtime-types` | `foundation` | `supported` |
| `decode-unsupported-diagnostic-api` | `foundation` | `supported` |
| `decoder-diagnostic-registry` | `foundation` | `supported` |
| `cli-decode-entrypoint` | `foundation` | `unsupported-intentional` |
| `cli-decode-hash-output-contract` | `foundation` | `partial` |
| `decode-limits-budget` | `foundation` | `partial` |
| `decode-limits-runtime-api` | `foundation` | `supported` |
| `decoded-frame-plane-model` | `foundation` | `supported` |
| `deterministic-frame-hash` | `foundation` | `supported` |
| `minimal-decode-tier-contract` | `foundation` | `partial` |
| `decode-stream-state` | `tier0-plan` | `partial` |
| `decode-byte-stream-planner` | `tier0-plan` | `supported` |
| `symbol-decoder` | `tier1-intra` | `partial` |
| `tile-payload-decode` | `tier1-intra` | `partial` |
| `tile-cdf-selection-boundary` | `tier1-intra` | `partial` |
| `decode-context-tile-payload-handoff` | `tier1-intra` | `supported` |
| `tile-payload-input-derivation` | `tier1-intra` | `supported` |
| `intra-dc-square-prediction` | `tier1-intra` | `supported` |
| `intra-dc-rectangular-prediction` | `tier1-intra` | `supported` |
| `intra-basic-paeth-prediction` | `tier1-intra` | `supported` |
| `intra-smooth-prediction` | `tier1-intra` | `supported` |
| `current-frame-workspace` | `tier1-intra` | `supported` |
| `intra-reconstruction` | `tier1-intra` | `partial` |
| `output-y4m` | `tier1-intra` | `partial` |
| `reference-frame-store` | `encoder-reuse` | `supported` |
| `decode-fuzz-entrypoint` | `foundation` | `supported` |

Important support-matrix observations:

- `cli-decode-entrypoint` is intentionally unsupported.
- `intra-reconstruction` is `partial`, has no Feature ID in the decoder support
  matrix, and has no self-contained tests yet.
- `deterministic-frame-hash` is supported only for caller-supplied decoded
  frames, not runtime decode.
- `output-y4m` is a library writer for caller-supplied frames, not runtime
  `splot decode -o` output.
- There are no `todo` rows, but many runtime decoder sections have no row at all.

## Current `decode/unsupported-feature` Sources

The decoder diagnostic registry currently enforces three decoder rule IDs:
`decode/malformed-source`, `decode/resource-limit`, and
`decode/unsupported-feature`.

Reachable or planned `decode/unsupported-feature` sources:

| Source | Scope | Current exposure |
|---|---|---|
| `UNSUPPORTED_FEATURE_DIAGNOSTIC` in `splot-decode` | Runtime output deferral after byte planning succeeds. | Public through `splot decode`; cites section 7.1, row `cli-decode-entrypoint`, feature `CLI-DECODE`. |
| `DecodeUnsupportedStructure` in `stream_plan.rs` | Planner-level unsupported structures: invalid global/local layer scope, non-base temporal/embedded/extended layers, multistream/OPS/atlas/LCR/MSDO selection, non-CLK frame OBUs, reserved OBUs, output-effect OBUs. | Public through `splot decode` after byte planning hits one of those structures. |
| Tile-payload boundary metadata | Crate-private unsupported `decode_tile()` boundary and minimal-tier gates. | Not public yet; docs say this remains crate-private until runtime decode surfaces stable public diagnostics. |
| Decoder support rows | `minimal-decode-tier-contract` and `intra-reconstruction` name planned future unsupported-feature diagnostics. | Planned, not runtime behavior. |

## Spec Sections With No Runtime Decode Owner

This audit treats a parser or validator row as evidence, not as runtime decoder
ownership. The current decoder support matrix names a small subset of the AV2
spec. The following ranges need explicit decoder support rows and Feature IDs
before implementation can claim full decoder coverage.

| AV2 range | Current local state | Missing runtime decoder owner |
|---|---|---|
| Section 4 descriptors beyond the current byte-planner subset | Core descriptor parsers exist in `splot-core`; decoder support rows mainly cite `4.11.6`. | Full decode-relevant descriptor coverage in a generated decoder spec coverage matrix. |
| Sections 5.4-5.17 and 6.4-6.16 | Parser/validator rows exist for many HLS, metadata, film-grain, QM, content-interpretation, OPS/LCR/MSDO/Atlas paths. | Runtime decoder state consumption, output-affecting metadata handling, film-grain state, external-HLS/operating-point state. |
| Sections 5.18 and 6.17 | Core frame-header parsing is partial; decode derives minimal tile facts only. | Full frame header state, inter paths, filters, global motion, film grain config, segmentation, quant, and tool gating as runtime decode state. |
| Sections 5.19-5.20 and 6.18-6.19 | Tile payload boundary and partition CDF subset are partial. | `decode_tile()`, partition/block traversal, mode syntax, residuals, multi-tile/multi-tile-group, BRU/bridge/TIP paths, CDF bank mutation. |
| Sections 7.1-7.4 | Planner supports some ordering/layer selection; CLI runtime remains unsupported. | General decode process, frame wrapup, output unit behavior, random access decode behavior, sub-bitstream extraction behavior. |
| Sections 7.5-7.12 | No runtime decoder support row. | Frame-end CDF update, extended layer context management, reference list construction, motion-field estimation, TIP motion fields, MV contexts, MV prediction. |
| Section 7.13 | DC, subsampled DC, IBP DC, PAETH, smooth, H/V cardinal directional, and workspace subsets exist. | General directional prediction, data-driven prediction, general directional-angle IBP, full CfL/MHCCP, palette, all inter prediction and mask/blend paths. |
| Sections 7.14-7.20 | Only broad `intra-reconstruction` cites 7.14-7.15 as partial/planned. | Dequantization, inverse transforms, residual add, deblocking, CDEF, CCSO, loop restoration, GDF. |
| Sections 7.21-7.23 | Frame model, hash input, Y4M writer, and reference container exist. | Runtime output order, implicit/show-existing/flush behavior, film-grain output variants, motion-field storage, exact AV2 reference refresh/update semantics. |
| Sections 8.2-8.3 | Generic symbol foundation and a partition CDF subset exist. | Full section 8.3 CDF selection and lifecycle, saved CDF copy/average/update policy, tile/frame CDF state. |
| Section 9 | Section 9.2 and a first section 9.3 CDF subset are represented. | Full section 9.3 CDF banks and section 9.4-9.8 decode table consumers for quant matrices, warp filters, transforms, secondary transforms, restoration. |
| Annex A | Minimal tier cites Annex A.2/A.5 as partial. | Profiles, levels, tiers, level limits, multi-sequence configurations, decoder conformance checks. |
| Annex B | Byte planner supports length-delimited traversal. | Runtime decode success over Annex B streams and complete malformed-source diagnostics at later stages. |
| Annex E | No decoder support row. | Decoder model timing, buffer, deadline, presentation, and conformance checks when signaled. |

## Panic, Allocation, And Output Safety

Current planner-only behavior has no known arbitrary-input panic blocker.

- The audited decoder/reconstruction/output paths have no reachable production
  `unwrap()`, `expect()`, `panic!`, `todo!`, or `unimplemented!` in the current
  CLI decode path. Matches from the Step 0 scan are test-only, docs/matrix text,
  or intentional structured unsupported diagnostics.
- Current byte planning checks input bytes, OBU count, IVF frame records, and
  frame candidate counts before retaining plan data.
- Tile-payload boundary code checks tile count and tile payload byte limits
  before slicing/decoding its bounded plan-only input.
- `splot-recon` allocation surfaces use typed errors and checked arithmetic, but
  runtime decode limits are not wired through future workspace/frame/reference
  allocations yet.
- Output-file atomicity is not implemented because current `splot decode` does
  not write output. Existing hash/Y4M primitives are writer APIs and can leave a
  caller-owned writer partially written on I/O error; CLI atomic temp-file and
  rename semantics remain future work.

Required future safety backlog:

1. Thread `DecodeLimits` through frame dimensions, luma samples, decoded-frame
   bytes, reference store bytes, tile allocations, transform buffers, and output
   bytes before runtime decode allocations.
2. Add atomic output-file semantics before successful `-o` output: temp file,
   flush/fsync policy, rename only after success, no final-path partial output
   on failure, and cleanup tests.
3. Keep fuzz coverage aligned with every new byte-consuming boundary.

## Fixtures And Reference Evidence

The committed conformance corpus currently validates parser/validator behavior,
not runtime decode output. The runner uses `splot_validate::Validator` over
committed bytes and has no AVM dependency.

| Fixture | Classification | Current expectation |
|---|---|---|
| `tests/conformance/vectors/valid/syn-key-intra-64x64.ivf` | Parser-only validator fixture; also used by one local reference evidence row. | Validator clean. |
| `tests/conformance/vectors/valid/syn-key-inter-64x64.ivf` | Parser-only validator fixture. | Validator clean. |
| `tests/conformance/vectors/invalid/syn-key-intra-64x64-truncated.ivf` | Parser-only negative validator fixture. | `ivf/truncated-frame-payload`. |
| `tests/conformance/vectors/valid/syn-intra-128x128.ivf` | Parser-only validator fixture. | Validator clean. |
| `tests/conformance/vectors/valid/syn-inter-96x64.ivf` | Parser-only validator fixture. | Validator clean. |
| `tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf` | Parser-only validator fixture; also used by one local reference evidence row. | Validator clean. |
| `tests/conformance/vectors/valid/syn-ops-64x64.ivf` | Parser-only validator fixture. | Validator clean. |
| `tests/conformance/vectors/needs-external-hls/syn-lcr-64x64.ivf` | Parser-only validator fixture. | `lcr/global-lcr-unavailable`. |
| `tests/conformance/vectors/needs-external-hls/syn-qm-64x64.ivf` | Parser-only validator fixture. | `frame-header/qm-level-unavailable`. |

Local reference evidence rows:

| Evidence row | Classification | Scope |
|---|---|---|
| `lref-avm-dav2d-syn-key-intra-64x64` | Hash-only reference evidence metadata. | Records AVM/dav2d raw decoder output MD5 equality for the 8-bit 64x64 fixture. It is not runtime `splot decode`, not `splot-dfh-sha256-v1`, and not output-image parity proof. |
| `lref-avm-dav2d-syn-intra-64x64-10bit` | Hash-only reference evidence metadata. | Records AVM/dav2d raw decoder output MD5 equality for the 10-bit 64x64 fixture. It is not runtime `splot decode`, not `splot-dfh-sha256-v1`, and not output-image parity proof. |

No current row covers runtime decode success, runtime hash output, runtime Y4M
output, raw output, post-film-grain output, or decoded-output-image parity.

The repo remains compliant with the no-integration rule: AVM/dav2d appear in
docs, evidence metadata, and negative validator tests for manifest portability,
but no checked-in decoder code, CI job, wrapper, dependency, or required command
invokes AVM or dav2d.

## Ordered OpenSpec Backlog

This backlog converts the mission into PR-sized changes. Existing active
OpenSpec changes should be resolved before opening a new implementation branch,
because the mission requires one OpenSpec change and one implementation branch at
a time.

0. `resolve-existing-active-openspec-state`
   - Decide whether to archive, complete, or explicitly leave alone
     `validator-context-split`, `avm-differential-harness`, and
     `toy-intra-encoder-v0` before starting decoder full-conformance work.

1. `decoder-full-conformance-contract`
   - Add `docs/DECODER-FULL-CONFORMANCE.md`.
   - Expand `docs/DECODER-SUPPORT-MATRIX.toml` so every normative
     decode-relevant section has an owner row.
   - Add or generate `docs/DECODER-SPEC-COVERAGE.md`.
   - Add `cargo xtask check-decoder-conformance-coverage`.
   - Do not implement codec features except what is needed for the coverage gate.

2. `decoder-output-equivalence-contract`
   - Formalize raw intermediate versus post-film-grain output variants, output
     order, flush/show-existing behavior, hash JSON schema, raw/Y4M behavior,
     and atomic output-file semantics.

3. `decode-minimal-tier-runtime-success`
   - Wire the first runtime success path for `--output-format hash --json` over
     the existing minimal intra tier, with deterministic hashes across thread
     policies and structured diagnostics on failures.

4. `decode-y4m-runtime-output`
   - Wire runtime Y4M output through atomic output-file handling after hash output
     is trustworthy.

5. `symbol-decoder-complete` and `cdf-lifecycle-complete`
   - Finish full section 8 symbol/CDF decode and lifecycle before broad tile
     syntax traversal.

6. `tile-payload-decode-complete`
   - Implement complete tile group/tile/block syntax traversal, including
     multi-tile and byte-span accounting.

7. `partition-and-block-mode-decode`
   - Add partition tree and mode-info decoding with edge/tile/subsampling tests.

8. `intra-prediction-complete`
   - Complete all section 7.13 intra prediction paths beyond the existing DC,
     PAETH, and smooth subsets.

9. `transform-quant-residual-complete`
   - Implement coefficient syntax, inverse quantization, inverse transforms,
     transform selection, residual addition, and exact clipping.

10. `reference-frame-state-complete`
    - Implement full AV2 reference slot validity, refresh, show-existing, output
      order, film grain metadata, segmentation maps, motion fields, and side data.

11. `motion-vector-and-inter-mode-decode`
    - Implement inter mode syntax and motion-vector parsing/prediction.

12. `inter-prediction-reconstruction-complete`
    - Implement inter prediction sampling, scaling, compound masks, warped/global
      prediction, TIP/BRU/RAS/switch behavior, and bit-exact fixtures.

13. `loop-filtering-complete`
    - Implement deblocking, CDEF, CCSO, loop restoration, GDF, super-resolution
      where applicable, and edge/tile/restoration-unit exactness.

14. `film-grain-complete`
    - Implement film-grain state, random process, synthesis, output variants, and
      metadata hash validation.

15. `profiles-levels-tiers-complete`
    - Implement Annex A profiles, levels, tiers, level constraints, and
      conformance diagnostics separate from local resource limits.

16. `layering-and-operating-points-complete`
    - Implement temporal/extended/embedded layers, operating-point selection, and
      sub-bitstream extraction behavior.

17. `multistream-and-random-access-complete`
    - Implement closed/open random access, RAS, multistream random access, LCR,
      MSDO, OPS, bridge, and long-term-reference interactions.

18. `decoder-model-constraints-complete`
    - Implement Annex E timing, buffer, presentation, deadline, and level-imposed
      constraints where signaled.

19. `metadata-decode-effects-complete`
    - Support metadata that affects decoding, output verification, or state,
      including decoded-frame-hash and film-grain connections.

20. `raw-output-and-test-hash-suite`
    - Add raw output and stable conformance hash fixtures for supported streams.

21. `decoder-conformance-corpus-v1`
    - Build a self-contained committed corpus manifest for runtime decode
      fixtures and expected hashes/diagnostics.

22. `decoder-local-reference-evidence-v1`
    - Expand local AVM/dav2d evidence metadata per fixture without adding repo or
      CI integration.

23. `decode-fuzz-complete`
    - Add fuzz coverage for every byte-consuming decode, reconstruction, and
      output boundary.

24. `decoder-mutation-and-negative-suite`
    - Add deterministic negative coverage for malformed decode paths and output
      failure safety.

25. `decoder-thread-determinism-complete`
    - Prove identical hashes, diagnostics ordering, CDF state, output order, and
      file bytes across supported thread policies.

26. `decoder-performance-baseline`
    - Add local benchmarks only after correctness is established for the relevant
      path.

## Reviewer Lane Results

Read-only subagents reviewed the Step 0 audit inputs:

| Lane | Decision |
|---|---|
| `decoder-architect` | Pass with gaps: runtime decode is absent at the public CLI boundary; backlog must start with orchestration into tile traversal and `splot-recon`. |
| `spec-mapper` | Pass with gaps: many runtime decoder sections have no owner row; `decoder-full-conformance-contract` should create the generated section-to-owner map. |
| `reference-auditor` | Pass with gaps: current evidence is parser-only or raw MD5 metadata, not runtime decode parity. No AVM/dav2d integration issue found. |
| `security` | Pass for current planner-only surface. Future runtime decode must wire limits through reconstruction allocations and implement atomic output files before success paths. |

## Next Action

Start with OpenSpec change `decoder-full-conformance-contract`, after resolving
the existing active OpenSpec state. The first implementation PR should be a
measurement/coverage PR, not a codec feature PR.
