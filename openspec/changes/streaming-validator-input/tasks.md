# Tasks

Sequenced as a PR chain. Each numbered group below is a self-contained, reviewable
PR (`splot-core` reader → `splot-validate` streaming entry → `splot-cli` wiring),
with matrix/docs landing alongside the code that proves them.

## 1. Matrix and docs

- [ ] Add `INFRA-STREAMING-TU-READER` to `docs/IMPLEMENTATION-MATRIX.toml`
      (category `infrastructure`, crate `splot-core`,
      `openspec_change = "streaming-validator-input"`, owner `core`).
- [ ] Add `INFRA-VALIDATE-STREAMING-READER` (category `infrastructure`, crate
      `splot-validate`, `openspec_change = "streaming-validator-input"`, owner
      `cli`).
- [ ] Regenerate `docs/FEATURE-STATUS.md` with
      `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [ ] No `docs/SPEC-MAPPING.md` change (no new AV2 section modeled).

## 2. PR1 — `splot-core` `TemporalUnitReader<R: Read>` (`INFRA-STREAMING-TU-READER`)

- [ ] Add `TemporalUnitReader<R: Read>` with a forward-only `next_unit` that yields
      one temporal unit into a reused buffer; container probe via `is_ivf`.
- [ ] IVF framing: parse the 32-byte file header once, then per-frame
      `read 12-byte header → frame_size → read_exact`.
- [ ] Annex-B framing: `read temporal_unit_size (leb128) → read_exact`; parse OBUs
      within the unit via the existing `AnnexBObuCursor` (no new OBU parser).
- [ ] Cross-read-boundary reassembly: accumulate bytes until the length prefix is
      satisfied; correct for a `Read` that returns 1 byte per call.
- [ ] Per-unit size cap as a local `TemporalUnitReader` config (byte limit),
      mirroring `decode`'s `max_input_bytes` guard — `splot-core` MUST NOT depend
      on `splot-decode`; typed error (never panic/`unwrap`) on overflow; allocate
      nothing oversized.
- [ ] Typed errors for truncation/short-read/malformed length; no `unsafe`.
- [ ] Tests: positive IVF + Annex-B; 1-byte-at-a-time `Read`; truncated unit; EOF
      between units; empty input; oversized-unit cap; buffer high-water-mark stays
      bounded by the largest unit.

## 3. PR2 — `splot-validate` `StreamingValidator` + `validate_reader` (`INFRA-VALIDATE-STREAMING-READER`)

- [ ] Extract the runner per-OBU loop body into `StreamingValidator { push_unit,
      finish }` over the existing `ValidatorContext` (no change to `observe_obu` /
      `run_checks` / `finish` semantics).
- [ ] Add `validate_reader<R: Read>(r, &ValidationOptions) -> ValidationReport`
      driving `TemporalUnitReader` → `push_unit` → `finish`.
- [ ] Re-express `validate_bytes` / `validate_bytes_with_options` over the same
      engine; preserve the public signatures.
- [ ] Preserve diagnostic ordering vs the in-memory path (see design Diagnostics);
      if unavoidable, re-baseline snapshots explicitly with rationale.
- [ ] Tests: **golden equivalence** — `validate_reader(Cursor::new(b)) ==
      validate_bytes(b)` across every fixture (set, order, offsets); malformed
      parity; 1-byte-`Read` parity.

## 4. PR3 — `splot-cli` `validate` wiring (extends `CLI-VALIDATE`)

- [ ] `validate` opens the input as a `File` and calls `validate_reader`, bounding
      peak memory (replaces `read_input` for this command).
- [ ] Support `-`/stdin via `stdin().lock()`.
- [ ] Keep exit codes and report rendering unchanged; update the `validate --help`
      snapshot only if the surface changes.
- [ ] Tests: `splot validate <file>` and `splot validate -` agree on a fixture.
- [ ] (Out of scope, note only) `inspect` migration is a follow-on change.

## 5. Tests and proof

- [ ] Add positive tests (IVF + Annex-B streamed end to end).
- [ ] Add malformed/EOF/cap tests and the 1-byte-`Read` reassembly test.
- [ ] Add the bounded-memory assertion test.
- [ ] Record proof commands and test paths in both matrix rows.

## 6. Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`

## 7. Review discipline

- [ ] One PR per group; Conventional Commit subjects.
- [ ] Both AI reviewers (claude-review + codex 👍) sign off before merge; reply and
      resolve every thread.
