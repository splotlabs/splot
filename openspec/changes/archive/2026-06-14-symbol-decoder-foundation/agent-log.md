# Agent Log: symbol-decoder-foundation

## Orchestrator Plan

- Current mission item: implement the next decoder/reconstruction infrastructure
  step after PR #123 (`RECON-Y4M-OUTPUT-WRITER`) merged.
- Candidate change: `symbol-decoder-foundation`.
- Feature ID: `AV2-8.2-SYMBOL-DECODER`.
- Scope decision: `splot-core` AV2 § 8.2 symbol decoder primitive only.
- Explicit non-goals: § 8.3 CDF selection, Tile/Saved CDF banks, default CDF
  initialization, `decode_tile()`, tile syntax traversal, reconstruction,
  runtime hashes, runtime Y4M output, AVM/dav2d invocation, new dependencies,
  and scheduler/concurrency changes.
- Branch discipline: keep this as OpenSpec planning on detached `origin/main`
  until `openspec validate symbol-decoder-foundation --strict` passes.

## Planning Subagents

### @architect — Sagan the 3rd

- Agent ID: `019ec6a6-16cb-7183-986f-7fcd77da9db1`
- Objective: determine whether `symbol-decoder-foundation` is the right next
  PR-sized change and define conservative crate/docs/test boundaries.
- Findings:
  - Yes, this is the correct next PR-sized item because the roadmap places AV2
    § 8 symbol/CDF foundation before constrained intra tile syntax.
  - Use Feature ID `AV2-8.2-SYMBOL-DECODER`.
  - Implement `init_symbol`, `read_bool`, `read_literal`, generic
    caller-supplied-CDF `read_symbol`, CDF update, and `exit_symbol`.
  - Put the implementation in new `crates/splot-core/src/symbol.rs` instead of
    growing `bitio.rs`.
  - Keep `splot-core` dependency-free and scheduler-free; no Rayon/crossbeam,
    worker pools, queues, or decode orchestration in this primitive.
  - Keep `decode_tile()`, § 8.3 CDF selection, full CDF banks, reconstruction,
    runtime output, and AVM/dav2d integration out of scope.

### @spec-reader — Maxwell the 3rd

- Agent ID: `019ec6a6-19ba-7530-a6d7-ff9c4e52639d`
- Objective: read the committed AV2 spec mirror for § 8.2, § 8.3, § 9, and
  direct tile-payload constraints.
- Findings:
  - `init_symbol(sz)` is specified by § 8.2.2:
    `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-2`.
  - `read_bool()` and `read_literal(n)` are specified by § 8.2.3 and § 8.2.5.
  - `read_symbol(cdf)` and CDF update are specified by § 8.2.6.
  - `exit_symbol()` is specified by § 8.2.4 and consumes tile-group trailing
    bits.
  - `read_symbol` depends on `EC_PROB_SHIFT`, `Prob_Inc`, and
    `Para_Adjustment_List` from § 3 and § 9.2.
  - § 8.3 CDF selection is too broad for this first PR and should stay named
    future work.
  - Full default-CDF initialization, Tile/Saved CDF copy/averaging, and
    `decode_tile()` / block syntax remain TODO.

### @api-designer — Chandrasekhar the 3rd

- Agent ID: `019ec6a6-1c78-7771-b0bd-b548a056c503`
- Objective: propose a minimal API shape that helps future tile decode without
  overclaiming runtime decode support.
- Findings:
  - Put the foundation in `splot-core`, not `splot-decode`.
  - Suggested shape: `SymbolDecoder<'a>`, `SymbolDecoderConfig`,
    `CdfUpdateMode`, `Symbol`, `SymbolDecoderSummary`.
  - Use `splot_core::Error` with typed variants/kinds for invalid CDF rows and
    invalid symbol state.
  - Validate CDF length, monotonic cumulative entries, probability range,
    adaptation-rate index, and capped use count before table indexing.
  - Do not add new `DecodeLimitName` in this PR; the primitive is bounded by
    finite tile slices and small CDF rows.
  - Leave `splot decode` behavior unchanged.

### @reference-oracle / @avm-reader-runner / @dav2d-reader-runner — Goodall the 3rd

- Agent ID: `019ec6a6-1f18-7d91-8111-9d64c4e3d93b`
- Objective: decide whether local AVM/dav2d evidence is needed and audit the
  boundary.
- Findings:
  - No local AVM/dav2d evidence is required for this PR if it stays limited to
    spec-derived § 8.2 primitives and generated § 9 table use.
  - Appropriate proof is self-contained Rust tests for init, bool/literal,
    symbol decode, CDF mutation, exit padding, and malformed inputs.
  - AVM/dav2d evidence becomes useful later when claiming decoded tile output,
    frame hashes, Y4M output, reconstruction, or reference-output agreement.
  - No local reference checkouts were run or modified.
  - Boundary rules currently hold; existing AVM/dav2d references are metadata,
    guidance, enforcement code, or negative tests.

### @security-reviewer — Jason the 3rd

- Agent ID: `019ec6a6-21f9-7ff2-828f-98f7aab37b92`
- Objective: threat-model symbol decoding over hostile tile bytes.
- Findings:
  - Treat `SymbolMaxBits` as signed and avoid unsigned underflow.
  - Avoid `sz * 8` overflow with branch-based or widened checked arithmetic.
  - Decode only from a finite tile-data slice produced by tile framing.
  - Validate CDF shape before indexing `Prob_Inc[N - 2][symbol]` or mutating
    `cdf[N]`.
  - Add tests for init boundaries, `read_bool`, `read_literal`, valid/invalid
    CDF rows, CDF update count saturation, `exit_symbol` `-14` vs `-15`, and
    trailing-bit/padding failures.
  - Consider a bounded arbitrary-input or property test to prove no panic.

### @encoder-impact-reviewer — Tesla the 3rd

- Agent ID: `019ec6a6-251a-78b0-86fc-b4dd27cd0e92`
- Objective: ensure the primitive helps future encoder closed-loop/RDO work
  without dead-end APIs.
- Findings:
  - Keep reusable entropy semantics in `splot-core`; do not add dependencies
    from `splot-encode` to `splot-decode`.
  - Future encoder work will need exact CDF mutation semantics, snapshot/restore,
    rate-estimation, and a separate writer path.
  - Keep § 8.3 CDF selection, tile-local CDF banks, saved/averaged CDF state,
    cross-frame CDF loading/blending, `decode_tile()`, and token-stream APIs out
    of this first PR.
  - Add matrix evidence under `AV2-8.2-SYMBOL-DECODER`; no AVM/dav2d evidence is
    needed until real tile decode exists.

## Implementation Log

- Added `crates/splot-core/src/symbol.rs` with `SymbolDecoder<'a>`,
  `SymbolDecoderConfig`, `CdfUpdateMode`, `Symbol`,
  `SymbolBitPosition`, and `SymbolDecoderSummary`.
- Implemented AV2 § 8.2.2 initialization over a finite tile payload byte slice,
  with signed `SymbolMaxBits` and checked payload-size arithmetic.
- Implemented § 8.2.3 `read_bool()`, § 8.2.5 `read_literal(n)`, § 8.2.6
  `read_symbol(&mut [i32])`, optional CDF updates, and § 8.2.4
  `exit_symbol()` / `finish()` validation.
- Added typed `splot-core::Error` variants and kind enums for invalid symbol CDF
  rows and invalid symbol decoder state.
- Exported `pub mod symbol` from `splot-core`; `RangeDecoder` and
  `RangeEncoder` remain unimplemented range-coder stubs.
- Updated the decoder roadmap, decoder support matrix/status, implementation
  matrix, generated feature status, and generated spec coverage.
- Focused local checks run so far:
  - `cargo test -p splot-core symbol --locked`
  - `cargo test -p splot-core tables_spot --locked` (matched no named tests)
  - `cargo test -p splot-core --test tables_spot --locked`
  - `cargo clippy -p splot-core --all-targets --all-features --locked -- -D warnings`
  - `cargo xtask check-feature-status`
  - `cargo xtask check-decoder-support`
  - `git diff --check`
  - `cargo test -p splot-core --locked`

## Review Log

- Encoder/concurrency review subagent `019ec6bc-1715-7611-b60b-310baf986747`
  found no concurrency-boundary or encoder-impact blocker: no Cargo manifests
  changed, `splot-core` stays dependency-neutral, `splot-recon` is untouched,
  no direct Rayon/crossbeam/global pool/thread/channel API was introduced, and
  `RangeEncoder` remains unimplemented.
- That review found two stale OpenSpec prose issues:
  - `design.md` mentioned `read_symbol(&mut [u16])`; resolved by updating the
    design to `read_symbol(&mut [i32])`, matching generated table types.
  - `proposal.md` said the change replaced the entropy-decoder stub; resolved
    by clarifying that `SymbolDecoder` is added alongside existing range-coder
    stubs.
- Security/robustness review subagent `019ec6bc-0abb-7a43-a963-94f89669b933`
  found no actionable issues. It checked CDF validation before generated-table
  indexing, symbol arithmetic bounds and shifts, signed `SymbolMaxBits`,
  `exit_symbol()` bit-position and padding validation, panic resistance,
  external-tool leakage, local paths, and direct thread/Rayon/crossbeam usage.
  It independently ran `cargo test -p splot-core symbol --locked`.
- General reviewer subagent `019ec6bc-05b1-7ca2-a15b-1fcd5af4280b` found one
  P2 code issue and two P3 OpenSpec prose issues.
  - P2: `num_bits_to_read()` narrowed a positive `i64` `SymbolMaxBits` with
    `as u32`; resolved by comparing in signed space before narrowing and adding
    `num_bits_to_read_does_not_truncate_large_symbol_max_bits`.
  - P3: stale `u16` design prose and "replace stub" proposal wording; already
    resolved as described above.
  - Post-fix checks: `cargo test -p splot-core symbol --locked` and
    `cargo clippy -p splot-core --all-targets --all-features --locked -- -D warnings`.
- Spec-conformance review subagent `019ec6bc-10c6-7140-bf8a-26b1ff466ad2`
  found two substantive issues and one duplicate OpenSpec prose nit.
  - Coverage overclaim: the feature row originally marked parse/validate/tests
    done for whole §8.2.2 and §8.2.4 even though Tile/Saved CDF copy and CDF
    averaging remain future work. Resolved by marking parse, validate, and
    tests partial in `AV2-8.2-SYMBOL-DECODER`, explaining the residual CDF-bank
    scope in the row notes, and regenerating `docs/FEATURE-STATUS.md` plus
    `docs/SPEC-COVERAGE.md`. The generated §8.2 and §9.2 lines now show
    partial coverage.
  - Test strength: added exact multi-arity symbol threshold vectors, a
    last-symbol CDF-update vector, count-interval and nonzero adaptation-rate
    update vectors, an `EC_PROB_SHIFT == 7` test, and `tables_spot` mirror checks
    for `Prob_Inc` and `Para_Adjustment_List`.
  - Duplicate stale `u16` design prose was already fixed.
  - Post-fix checks: `cargo test -p splot-core symbol --locked`,
    `cargo test -p splot-core --test tables_spot --locked`, and
    `cargo xtask check-feature-status`.

## Local Reference Evidence

- None used for planning.
- Rationale: the change is a spec-derived § 8.2 primitive with self-contained
  Rust tests. AVM/dav2d evidence is deferred until tile decode or decoded output
  is claimed.

## Boundary Audit

- Planning scope adds no AVM/dav2d source, snippets, binaries, submodules,
  Cargo dependencies, build probes, wrappers, scripts, CI jobs, runtime process
  execution, local absolute paths, or mandatory reference-tool tests.
