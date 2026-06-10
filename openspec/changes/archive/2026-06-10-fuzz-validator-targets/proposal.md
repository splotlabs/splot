# Proposal: Fuzz every untrusted-input surface (validator, IVF, container dispatch)

## Feature IDs

- `CONF-FUZZ-NO-PANIC`

## Why

The "parsers never panic" conformance requirement is enforced by exactly one
fuzz target. `fuzz/fuzz_targets/parse_obu.rs` exercises `read_leb128`,
`read_obu_header`, and `parse_annex_b_obus` — three of the ~25 public entry
points that consume arbitrary bytes. Not one OBU payload parser
(sequence/frame headers, HLS OBUs, metadata, film grain, quantizer matrix,
tile group), neither IVF entry point (`parse_ivf_header`,
`parse_ivf_partial`), the container auto-detect (`parse_bitstream_partial`),
nor the validator API (`Validator::validate_bytes`, the largest
attacker-reachable surface) is exercised by any coverage-guided/ASan target.
The CI fuzz-smoke job hardcodes the single `parse_obu` target. And
`splot-validate` has zero property tests and no `proptest` dev-dependency, so
the stable-toolchain half of the no-panic invariant stops at `splot-core`.

## What Changes

- Three new fuzz targets in `fuzz/fuzz_targets/` (the existing `parse_obu`
  stays as the fast descriptor/envelope target):
  - `validate_bytes` — drives `Validator::validate_bytes_with_options` with
    options derived deterministically from the first input byte and the rest
    as the bitstream. This transitively reaches every OBU payload parser,
    both container formats, and every validator check — the single
    highest-coverage target.
  - `parse_ivf` — `is_ivf` + `parse_ivf_header` + `parse_ivf_partial`.
  - `parse_bitstream` — `parse_bitstream_partial` (container auto-detect and
    OBU payload dispatch on both raw Annex B and IVF-wrapped inputs).
- `fuzz/Cargo.toml` gains a `splot-validate` path dependency and the three
  `[[bin]]` entries.
- The CI `fuzz-smoke` job and `cargo xtask fuzz` enumerate targets via
  `cargo +nightly fuzz list` and run each for a per-target time slice instead
  of hardcoding `parse_obu`; corpus seeding from `tests/fixtures/*.av2`
  applies to every target.
- `splot-validate` gains `proptest` as a dev-dependency (already a workspace
  dependency used by `splot-core` — no new third-party crate) and a
  `validator_never_panics` property test over arbitrary bytes and arbitrary
  option bytes, mirroring the `parsers_never_panic` pattern in `splot-core`.
- Matrix row `CONF-FUZZ-NO-PANIC` records the target-to-surface mapping in
  its notes and advances stages only with proof.
- Docs: `AGENTS.md` § 4 fuzz command line and `docs/TESTING.md` reflect the
  multi-target reality; generated docs and audit ledger refreshed.

## Scope

- Spec sections: none (conformance tooling; no AV2 syntax change).
- Crates/modules: `fuzz/` (new targets + manifest), `crates/splot-validate`
  (dev-dependency + property test only — no library code change),
  `xtask/src/main.rs` (`run_fuzz`), `.github/workflows/ci.yml` (fuzz-smoke
  job).
- CLI/docs/tests: `AGENTS.md`, `docs/TESTING.md`,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated docs, audit ledger.

## Non-goals

- New conformance vectors or corpus tooling (`conformance-corpus-scaffold`
  and later backlog items).
- Fixing any panic the new targets might find — a found panic is a bug to fix
  in its own change with a regression fixture (if smoke finds one during this
  change's CI, that bug becomes an immediate follow-up fix in this PR only if
  trivially small; otherwise the target ships with the crashing input
  excluded and a filed matrix TODO).
- Per-payload-type narrow fuzz targets beyond the four (the validator target
  reaches all payload parsers transitively; narrow targets can be added later
  if coverage data shows gaps).
- Structured/arbitrary-based fuzzing (`arbitrary` derive) — raw-bytes targets
  only.

## Acceptance criteria

- [ ] `cargo +nightly fuzz list` shows `parse_obu`, `validate_bytes`,
  `parse_ivf`, `parse_bitstream`; each builds and runs locally for a short
  smoke (`cargo xtask fuzz --time 10`).
- [ ] The CI fuzz-smoke job runs every listed target and stays blocking.
- [ ] `splot-validate` has a no-panic property test; `cargo test -p
  splot-validate` passes on stable.
- [ ] Every public byte-consuming entry point in `splot-core` and
  `splot-validate` is reachable from at least one fuzz target (mapping
  documented in the matrix row notes).
- [ ] Matrix stages advance only with recorded proof;
  `cargo xtask check-feature-status` and `cargo xtask ci` pass.
