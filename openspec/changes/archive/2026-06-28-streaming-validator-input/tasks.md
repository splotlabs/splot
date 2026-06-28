# Tasks

Sequenced as a PR chain. Each numbered group below is a self-contained, reviewable
PR (`splot-core` reader → `splot-validate` streaming entry → `splot-cli` wiring),
with matrix/docs landing alongside the code that proves them.

## 1. Matrix and docs

- [x] Add `INFRA-STREAMING-TU-READER` to `docs/IMPLEMENTATION-MATRIX.toml`
      (category `infrastructure`, crate `splot-core`,
      `openspec_change = "streaming-validator-input"`, owner `core`).
- [x] Add `INFRA-VALIDATE-STREAMING-READER` (category `infrastructure`, crate
      `splot-validate`, `openspec_change = "streaming-validator-input"`, owner
      `cli`).
- [x] Regenerate `docs/FEATURE-STATUS.md` with
      `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] No `docs/SPEC-MAPPING.md` change (no new AV2 section modeled).

## 2. PR1 — `splot-core` `TemporalUnitReader<R: Read>` (`INFRA-STREAMING-TU-READER`)

- [x] Add `TemporalUnitReader<R: Read>` with a forward-only `next_unit` that yields
      one temporal unit into a reused buffer; container probe via `is_ivf`.
- [x] IVF framing: parse the 32-byte file header once, then per-frame
      `read 12-byte header → frame_size → read_exact`.
- [x] Annex-B framing: `read temporal_unit_size (leb128) → read_exact`; parse OBUs
      within the unit via the existing `AnnexBObuCursor` (no new OBU parser).
- [x] Cross-read-boundary reassembly: accumulate bytes until the length prefix is
      satisfied; correct for a `Read` that returns 1 byte per call.
- [x] Per-unit size cap as a local `TemporalUnitReader` config (byte limit),
      mirroring `decode`'s `max_input_bytes` guard — `splot-core` MUST NOT depend
      on `splot-decode`; typed error (never panic/`unwrap`) on overflow; allocate
      nothing oversized.
- [x] Typed errors for truncation/short-read/malformed length; no `unsafe`.
- [x] Tests: positive IVF + Annex-B; 1-byte-at-a-time `Read`; truncated unit; EOF
      between units; empty input; oversized-unit cap; buffer high-water-mark stays
      bounded by the largest unit.

## 3. PR2 — `splot-validate` `StreamingValidator` + `validate_reader` (`INFRA-VALIDATE-STREAMING-READER`)

- [x] Extract the runner per-OBU loop body into `StreamingValidator { push_unit,
      finish }` over the existing `ValidatorContext` (no change to `observe_obu` /
      `run_checks` / `finish` semantics).
- [x] Add `validate_reader<R: Read>(r, &ValidationOptions) -> ValidationReport`
      driving `TemporalUnitReader` → `push_unit` → `finish`.
- [x] Share the per-OBU engine (`process_obu` = `observe_obu` + the check
      registry) between the in-memory and streaming paths; keep `validate_bytes`
      on the in-memory parser and prove byte-identical equivalence by property
      test, rather than routing the infallible `validate_bytes` through the
      fallible reader. Public signatures preserved.
- [x] Preserve diagnostic ordering vs the in-memory path (see design Diagnostics);
      if unavoidable, re-baseline snapshots explicitly with rationale.
- [x] Tests: **golden equivalence** — `validate_reader(Cursor::new(b)) ==
      validate_bytes(b)` across every fixture (set, order, offsets); malformed
      parity; 1-byte-`Read` parity.

## 4. PR3 — `splot-cli` `validate` wiring (extends `CLI-VALIDATE`)

- [x] `validate` opens the input as a `File` and calls `validate_reader`, bounding
      peak memory (replaces `read_input` for this command).
- [x] Support `-`/stdin via `stdin().lock()`.
- [x] Keep exit codes and report rendering unchanged; update the `validate --help`
      snapshot only if the surface changes.
- [x] Tests: `splot validate <file>` and `splot validate -` agree on a fixture.
- [x] (Out of scope, note only) `inspect` migration is a follow-on change.

## 5. Tests and proof

- [x] Add positive tests (IVF + Annex-B streamed end to end).
- [x] Add malformed/EOF/cap tests and the 1-byte-`Read` reassembly test.
- [x] Add the bounded-memory assertion test.
- [x] Record proof commands and test paths in both matrix rows.

## 6. Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`

## 7. Review discipline

- [x] One PR per group; Conventional Commit subjects.
- [x] Both AI reviewers (claude-review + codex 👍) sign off before merge; reply and
      resolve every thread.
