# Code review checklist

A concise checklist for humans and agents reviewing changes to `splot`.

## Spec correctness

- [ ] Is the AV2 spec section cited as `§ N.M` plus the mirror path (e.g.
      `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-16`) in the doc comment,
      or marked with `// TODO(spec: <FEATURE-ID>)`?
- [ ] No AV1 leakage (OBU header is § 5.2.2; no AV1 OBU type table, forbidden bit,
      or size-field assumptions)?
- [ ] No invented syntax, constants, or table contents?
- [ ] Nothing under `docs/spec/av2/` was hand-edited
      (`cargo xtask check-spec-mirror`)?

## Encoder reference gate

Only for encoder or encoder-facing syntax PRs.

- [ ] Is the "Encoder research gate" block from
      `.github/PULL_REQUEST_TEMPLATE.md` filled in?
- [ ] Are AV2 spec sections and AVM oracle paths identified for decoder-visible
      behavior?
- [ ] Are rav1e/SVT-AV1 used only as inspiration?
- [ ] No AV1 syntax, code, tables, constants, entropy CDFs, comments, or prose
      copied?
- [ ] Does the feature have a row in `docs/IMPLEMENTATION-MATRIX.toml` when it
      touches syntax, reconstruction, reference state, or layer behavior?
- [ ] Do scalar correctness and deterministic traces exist before performance
      work?

## Error handling

- [ ] No `unwrap` / `expect` / `panic!` / `todo!` / `unimplemented!` reachable in
      library code?
- [ ] Stubs return `Error::Unimplemented { feature }` or a structured `Diagnostic`?
- [ ] Parsers return errors (never panic) on malformed/truncated input?

## Diagnostics

- [ ] Does each validator finding have a stable `rule_id`, `severity`, `spec_section`,
      byte/bit offset (where known), and a clear `message`?
- [ ] Are new or renamed `rule_id` values registered in
      `docs/VALIDATOR-DIAGNOSTICS.md` (`cargo xtask check-diagnostic-registry`)?

## Tests

- [ ] Positive, negative, and EOF cases for parser changes?
- [ ] Property/fuzz coverage where relevant (parsers never panic)?
- [ ] Differential coverage against the AVM oracle where feasible?

## Boundaries

- [ ] Crate dependency graph unchanged (`cargo xtask check-dependency-direction`)?
- [ ] Library-first: no codec/validation logic leaked into `splot-cli`?

## Concurrency

(Policy: [CONCURRENCY.md](./CONCURRENCY.md).)

- [ ] No hidden/global Rayon pool; no `build_global`
      (`cargo xtask check-concurrency-policy`)?
- [ ] No ad-hoc `thread::spawn` outside tests
      (`cargo xtask check-concurrency-policy`)?
- [ ] No unbounded channels / `crossbeam_channel::unbounded`; bounded queues only,
      at coarse pipeline boundaries (never hot per-pixel/block/row loops)
      (`cargo xtask check-concurrency-policy`)?
- [ ] No banned runtime crates (tokio/async-std/futures/threadpool/flume/
      async-channel/…); only `splot-parallel` may depend on
      `rayon`/`crossbeam-channel` (`cargo xtask check-concurrency-policy`)?
- [ ] `splot-core` stays runtime-free; `splot-validate` stays single-threaded
      (`cargo xtask check-concurrency-policy`)?
- [ ] Each encode/decode context owns exactly one `WorkerPool`; nested work via
      `install`, never nested pools?
- [ ] New parallel work reaches Rayon only through `splot_parallel::prelude` (no
      direct `rayon` dep) and runs **inside** `WorkerPool::install` (never at top
      level → global pool) (`cargo xtask check-concurrency-policy`)?
- [ ] Worker threads come only from the context's `WorkerPool` sized by
      `--threads` — no raw `std::thread::spawn` for codec work?
- [ ] Deterministic output preserved across thread counts (indexed iterators /
      ordered merge; commit in presentation/bitstream order)?

## Zero-copy media ownership

(Policy: [ZERO_COPY.md](./ZERO_COPY.md).)

- [ ] No `Clone` derive/impl on media-storage types (frame/plane/reference/
      workspace/sample buffers); borrow a view or share via `SharedFrame`
      (`cargo xtask check-zero-copy-policy`)?
- [ ] View types (`PlaneRef`/`PlaneMut`/`FrameRef`/`FrameMut`) borrow existing
      storage and never allocate or copy on construction?
- [ ] No `Arc::make_mut` / `Rc::make_mut` or copy-on-write on frame storage
      (`cargo xtask check-zero-copy-policy`)?
- [ ] Reference/lookahead stores move or share handles (no `F: Clone`
      requirement, no payload duplication)?
- [ ] Every intentional copy carries a nearby specific `splot-copy-ok: <reason>`
      naming the materialization boundary — no vague markers
      (`cargo xtask check-zero-copy-policy`)?
- [ ] No `unsafe` / `transmute` / `from_raw_parts` to reinterpret bytes as samples
      (`cargo xtask check-zero-copy-policy`)?
- [ ] `zerocopy` only in `splot-core`/`splot-recon`, only for private fixed-layout
      wire structs, never in public APIs or AV2 bit-level/entropy parsing; no
      banned byte/transmute crate added (`cargo xtask check-zero-copy-policy`)?

## Feature tracking

- [ ] Is the Feature ID present in the PR title/body?
- [ ] Is the matrix row added or updated (`docs/IMPLEMENTATION-MATRIX.toml`)?
- [ ] Does an OpenSpec change exist for non-trivial behavior/design changes?
- [ ] Does `cargo xtask check-feature-status` pass?
- [ ] Does every `done` status have proof recorded in `[feature.proof]`?

## AI-slop

- [ ] No banned process-history or fixture-diary phrase in source comments
      (`cargo xtask check-ai-slop`)?
- [ ] Could a new branch be replaced by a generic table, capability, or
      dispatcher instead of a one-off fixture/transform/block-shape case?
- [ ] Does each unsupported diagnostic name the missing capability rather than
      tell an implementation-history story?
- [ ] Did implementation-comment count, duplicate-code budget, or source-line
      pressure rise without justification?

## Hygiene

- [ ] SPDX header on every `.rs` file (`cargo xtask check-license-headers`)?
- [ ] Public items documented?
- [ ] Does each source comment explain an invariant or exception?
- [ ] Could a clearer name, helper, type, or smaller function replace the comment?
- [ ] Does the comment duplicate AV2 spec prose instead of using a short section
      anchor?
- [ ] Does the comment mention history that belongs in an ADR or design doc?
- [ ] Would the comment remain true after a small refactor?
- [ ] Is this Rustdoc required for public API users, or just filler?
- [ ] PR title and commit subjects follow Conventional Commits
      (`cargo xtask check-conventional-title` /
      `cargo xtask check-conventional-commits`)?
- [ ] `cargo xtask ci` passes?
