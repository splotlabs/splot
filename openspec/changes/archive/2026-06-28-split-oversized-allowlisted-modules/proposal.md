## Why

Four Rust source files carry hard-cap allowances in `xtask/src/source_lines.rs`
(soft limit 1000, hard cap 2500): `frame/info.rs` (5213), `celu.rs` (3693),
`sequence.rs` (3282), and `wienerns_lr/tx_records.rs` (2615). Each allowance is a
standing IOU. This change records a deliberate, evidence-grounded plan to retire
those IOUs by splitting along the files' *real* responsibility seams — and,
just as important, to **not** split the two files that are under active
development, where a structural move would collide with in-flight bit-exact
decoder work and scatter hard-won correctness history.

This change implements the first, safest slice — the `celu.rs` split — and
records the reviewed contract (which seams, in which order, with which
invariants) for the rest of the campaign. The `sequence.rs` split and the two
deferred decoder-frontier files land as separate follow-up changes under the
same Feature ID and the `module-size-discipline` capability.

## What Changes

- Add Feature ID `INFRA-MODULE-SIZE-REFACTOR` (category/kind `infrastructure`,
  `openspec_change = "split-oversized-allowlisted-modules"`).
- Establish the **real seams** per file, replacing the naive
  responsibility-by-doc-heading split with what the code actually supports:
  - **`crates/splot-validate/src/celu.rs` (do first — cold, 1 commit/90d):**
    move the ~2267-line in-file test module to a `#[path]` sibling
    (`celu/tests.rs`), optionally extract the one genuine logic seam
    `DohTuAccumulator` → `celu/doh.rs`. Do **NOT** perform the proposed 8-way
    split — `observe_frame` is one cohesive state machine whose round-numbered
    invariants (F1/F2/F3/F5, poison-scope ordering) must stay together. Lowers
    the file to ~1200–1400 lines, under the hard cap, removing its allowance.
  - **`crates/splot-core/src/headers/sequence.rs` (do second — warm, sweep-only):**
    convert to a `sequence/` directory and extract `profile.rs` (`ProfileIdc` +
    its custom Eq/Ord/Hash), `layer_dependency.rs` (all three maps together, not
    presence-only), and `child_configs.rs` (the ~860-line §5.4.x struct+parser
    mass). Re-export every moved public item (≈70 files import via
    `crate::headers::sequence::…`). Skip standalone `level.rs`/`chroma.rs`
    (over-fragmentation).
  - **`crates/splot-core/src/headers/frame/info.rs` (DEFER — hot, 25 commits/90d):**
    when the decoder frontier stabilizes, split by real seams: `status` (parse
    vocabulary enums), `show_existing` (the SEF path), and `seq_view` (the
    sequence/MFH input views). Not the proposal's `activation`/`inter_control`/
    `shared_tail`, which already live in sibling modules.
  - **`crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs`
    (DEFER — active ac0ej3 frontier):** when stable, extract `per_block_syntax`
    (`DeltaQState`/`CdefState`, mirroring the existing `ccso.rs`), `tx_partition`
    (the `apply_tx_partition` geometry), and `error` (the error taxonomy) —
    **not** chroma-first (chroma is interleaved inline with the luma walk).
- All splits preserve public APIs via facades/re-exports and keep behavior
  byte-identical (existing tests and goldens unchanged).
- Each in-scope file split lands as its **own** PR; the allowance for a file is
  removed from `xtask/src/source_lines.rs` in the same PR that shrinks it.

Non-goals:

- No AV2 syntax, constant, table, or semantics change; no encoder, conformance,
  or fuzz-coverage work.
- No public API or diagnostic `rule_id` changes; no crate dependency-graph
  changes (all moves stay within their current crate).
- No behavior change: the celu split is a pure relocation; all 71 celu tests are
  preserved and pass unchanged.
- No `sequence.rs` code in this change, and no reduction of the `frame/info.rs`
  or `tx_records.rs` allowances now; those are separate follow-up changes (the
  two decoder files explicitly deferred until the frontier stabilizes).

## Capabilities

### New Capabilities

- `module-size-discipline`: the durable requirement that oversized allowlisted
  modules are retired by responsibility-aligned splits that preserve public APIs
  and byte-identical behavior, sequenced by development activity (cold files
  first, actively-developed files deferred with the deferral recorded in the
  allowlist).

### Modified Capabilities

None — no existing capability's requirements change. The splits are internal
restructuring that preserve every public API and behavior.

## Impact

- `xtask/src/source_lines.rs` (allowances removed/lowered as each in-scope file
  shrinks).
- `crates/splot-validate/src/celu.rs` (+ new `celu/tests.rs`, optional
  `celu/doh.rs`).
- `crates/splot-core/src/headers/sequence.rs` → `sequence/` directory (+
  `profile.rs`, `layer_dependency.rs`, `child_configs.rs`, `mod.rs` facade);
  `crates/splot-core/src/headers.rs` re-exports verified.
- `crates/splot-core/src/headers/frame/info.rs` and
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs`
  (documented as deferred; no edits now).
- `docs/IMPLEMENTATION-MATRIX.toml` (new Feature ID row).
- No runtime, ABI, or dependency-direction impact; no validator diagnostic
  surface change.
