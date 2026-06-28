## ADDED Requirements

### Requirement: Oversized allowlisted modules are retired by real-seam splits

Each allowlisted oversized module SHALL be retired by reducing it under the
2500-line hard cap (and toward the 1000-line soft limit), extracting cohesive,
low-coupling units along the file's actual responsibility seams, tracked by
`INFRA-MODULE-SIZE-REFACTOR`. A split SHALL NOT scatter a single cohesive state
machine or a shared mutable accumulator across modules merely to satisfy the
line budget; where the production logic is one cohesive unit that is only long,
the test module SHALL be relocated to a sibling file first. When a file is
shrunk under the hard cap, its allowance entry in `xtask/src/source_lines.rs`
SHALL be removed in the same change.

#### Scenario: A retired file drops its allowance and passes the budget gate

- **WHEN** `cargo xtask ci` runs the source-line budget gate after a file has
  been split
- **THEN** the file is at or below the 2500-line hard cap
- **AND** its entry has been removed from `HARD_LINE_ALLOWANCES` in
  `xtask/src/source_lines.rs`
- **AND** the gate reports no allowance problem for a now-compliant path

#### Scenario: Cohesive logic is not scattered to chase the budget

- **WHEN** a split is proposed for a file whose production logic is one cohesive
  state machine (for example `celu.rs`'s `observe_frame`)
- **THEN** the split relocates the in-file test module (and any genuinely
  separable satellite such as `DohTuAccumulator`) rather than fragmenting the
  shared-state logic into per-field modules

### Requirement: Splits preserve public APIs and byte-identical behavior

A module split performed under this capability SHALL preserve every existing
public and crate-public API by re-exporting moved items through a facade module
(for a file becoming a directory, its `mod.rs`), so that no downstream `use`
path changes. The split SHALL be behavior-preserving: existing unit tests,
property tests, conformance vectors, and reconstruction goldens SHALL continue
to pass unchanged, and no validator diagnostic `rule_id` or AV2 bitstream
behavior SHALL change.

#### Scenario: Downstream imports keep compiling after a directory split

- **WHEN** `crates/splot-core/src/headers/sequence.rs` becomes a `sequence/`
  directory and `ProfileIdc`, the layer-dependency maps, and the §5.4.x child
  configs move into submodules
- **THEN** every existing `crate::headers::sequence::…` import resolves through
  re-exports without edits at the call sites
- **AND** `cargo test -p splot-core --locked` passes with the same assertions

#### Scenario: Behavior is unchanged by the move

- **WHEN** the workspace test suite and reconstruction goldens run after a split
- **THEN** all previously passing tests and goldens still pass
- **AND** no diagnostic `rule_id`, spec mapping, or generated table changes

### Requirement: Splits are sequenced by development activity

The campaign SHALL split cold, stable files before files under active
development. A file that is part of an in-flight bit-exact decoder workstream
(for example `frame/info.rs` and `wienerns_lr/tx_records.rs` during the ac0ej3
mission) SHALL be deferred until that frontier stabilizes, and the deferral
SHALL remain recorded in the file's `xtask/src/source_lines.rs` allowance reason
until the split lands. Each in-scope file SHALL be split in its own pull request.

#### Scenario: An actively-developed file is deferred, not split now

- **WHEN** the campaign selects the next file to split
- **THEN** `celu.rs` (cold) and `sequence.rs` (sweep-only) are split before
  `frame/info.rs` and `tx_records.rs`
- **AND** the `frame/info.rs` and `tx_records.rs` allowances retain a reason
  documenting the deferral until their decoder frontier stabilizes

#### Scenario: One file per pull request

- **WHEN** a split PR is opened
- **THEN** it changes exactly one allowlisted file's module structure (plus its
  new sibling files, re-exports, and that file's allowance entry)
- **AND** it does not opportunistically restructure unrelated modules
