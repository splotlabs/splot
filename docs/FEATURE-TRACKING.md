# Feature tracking

How `splot` tracks AV2 implementation work so the maintainer (and coding agents)
can always answer: *what is mapped, what is implemented, what is proven, and what
to do next.*

## 1. Why this exists

AV2 is far too large for ad-hoc `TODO`s or a GitHub-only board. We need:

- one **machine-readable** record of what exists and how far, and
- **automation** that fails the build when the record drifts from the code.

That record is [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml).
It is canonical. Everything else points at it.

## 2. The five-layer model

```text
OpenSpec change      = design intent and acceptance criteria   (openspec/)
Implementation matrix = canonical source of truth              (docs/IMPLEMENTATION-MATRIX.toml)
Code / tests / diagnostics = the actual work                   (crates/, fuzz/)
xtask                = enforcement and reporting               (cargo xtask ...)
GitHub Issues/Project = execution queue, not canonical truth
```

```text
OpenSpec change
      ↓ defines intent for
Feature ID in docs/IMPLEMENTATION-MATRIX.toml   ← canonical
      ↓ referenced by
code module / diagnostic rule / test / fuzz target / CLI behavior
      ↓ proven by
proof recorded in the matrix row
      ↓ enforced by
cargo xtask check-feature-status   (also in cargo xtask ci and CI)
      ↓ scheduled in
GitHub issue / PR (references the same Feature ID)
```

GitHub Issues/Projects and README checklists are an execution queue
and snapshots. When they disagree with the matrix, **the matrix wins.**

## 3. Feature ID convention

```text
^[A-Z0-9]+(-[A-Z0-9.]+)+$
```

| Kind | Pattern | Real example |
|---|---|---|
| Normative AV2 | `AV2-<SECTION>-<SLUG>` | `AV2-5.2.2-OBU-HEADER` |
| Annex | `AV2-<ANNEX>-<SLUG>` | `AV2-B-ANNEXB-OBU-ENVELOPE` |
| Encoder | `ENC-<SLUG>` | `ENC-BITSTREAM-WRITER` |
| Conformance | `CONF-<SLUG>` | `CONF-FUZZ-NO-PANIC` |
| Tooling | `XTASK-<SLUG>` | `XTASK-FEATURE-STATUS` |
| Docs | `DOC-<SLUG>` | `DOC-FEATURE-TRACKING` |
| CLI | `CLI-<SLUG>` | `CLI-INSPECT` |

A merged ID is **stable** — do not rename it casually. To replace one, add the new
row and list the old id in `replaces` (see the schema). The same ID appears in: the
matrix row, the OpenSpec change, the GitHub issue/PR, the diagnostic rule id (when
applicable), tests, `TODO(spec: <id>)` comments, and docs.

## 4. Status model

Ten stages per feature, each one of: `todo`, `partial`, `done`, `blocked`,
`not-applicable`, `experimental`, `pending`. **Never** percentages.

`mapped`, `types`, `parse`, `validate`, `write`, `encode`, `decode_check`, `tests`,
`avm_diff`, `perf`. Full definitions: [the schema](./IMPLEMENTATION-MATRIX.schema.md).

Good: `OBU header: parse done, validate done, write todo, encode not-applicable`.
Bad: `OBU header: 70% complete`.

## 5. Matrix schema (summary)

Each `[[feature]]` row has `id`, `name`, `category`, `kind`, `spec_sections`,
`sources`, `crate`, `module`, `openspec_change`, `tracking_issue`, `owner`, `risk`,
`notes`, a `[feature.status]` table (the ten stages), and a `[feature.proof]` table
(`tests`, `commands`, `fixtures`, `diagnostics`). The full schema, allowed values,
and proof rules are in [IMPLEMENTATION-MATRIX.schema.md](./IMPLEMENTATION-MATRIX.schema.md).

Render and check it:

```bash
cargo xtask feature-status                 # aligned table
cargo xtask feature-status --format json   # for tooling
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask feature-status --category normative
cargo xtask feature-status --kind bitstream-syntax
cargo xtask check-feature-status           # fail on drift
cargo xtask spec-coverage                  # coverage summary
```

## 6. Workflow: a new AV2 syntax feature

```text
1. Create an OpenSpec change      openspec/changes/<change-id>/ (proposal + tasks)
2. Add a matrix row               docs/IMPLEMENTATION-MATRIX.toml (status todo)
3. Implement strong types         crates/splot-core/src/...
4. Implement the parser           (panic-free; errors, never panics)
5. Add validator diagnostics      crates/splot-validate/... (stable rule ids)
6. Add tests + fuzz/property      positive, negative, EOF
7. Update proof + status          [feature.proof]; bump stages to done only with proof
8. Regenerate the generated docs  cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
                                  cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md
9. Run the checks                 cargo xtask check-feature-status && cargo xtask ci
10. Open a PR                     put the Feature ID in the title/body
```

For any intentionally unmodeled AV2 detail, leave a marker:

```rust
// TODO(spec: AV2-5.4-SEQUENCE-HEADER): parse seq_profile after uvlc support exists.
```

`cargo xtask check-feature-status` rejects a bare spec TODO with no id and rejects
an id that is not in the matrix.

## 7. Workflow: encoder strategy work

Encoder features (`ENC-<SLUG>`) are often not a single spec section. Design first
(`openspec/changes/<change-id>/design.md`), keep bitstream-affecting config separate
from runtime/policy knobs, and never emit syntax the writer cannot produce and the
validator will not accept. Mark `write`/`encode` `done` only when round-trip or AVM
proof exists.

## 8. Workflow: conformance / vector work

Conformance features (`CONF-<SLUG>`) record **proof**. Use AVM as the oracle
(`avm encode` → `splot validate`, later `splot encode` → `avm decode`). Vendor only
redistributable/public vectors. Record the reproduction command and any fixtures in
the row's `[feature.proof]`. See [CONFORMANCE.md](./CONFORMANCE.md).

## 9. When may a stage be `done`?

A **code** stage (`parse`, `validate`, `write`, `encode`, `decode_check`, `tests`,
`avm_diff`, `perf`) may be `done` only when `[feature.proof]` records at least one
of: a test module/path, a reproducible command, a fixture/vector, or a diagnostic
id. `cargo xtask check-feature-status` enforces this. When in doubt, mark `partial`
and explain in `notes`.

## 10. Splitting large features

Some rows (for example `AV2-5.18-FRAME-HEADER`) are too big for one PR. Keep the
umbrella row, add narrower child rows whose ids extend the umbrella id, point each
child `module` at the real code, and do not mark the umbrella `done` until its
children are. See the schema's "splitting" section.

## 11. How an agent chooses the next task

1. Scan the generated [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) (or run
   `cargo xtask spec-coverage`) — blank/partial cells in spec order are the
   open work.
2. Prefer **validator-first** rows: `parse`/`validate` on normative bitstream
   syntax with `risk = high`, in dependency order (LEB128 → OBU header → Annex B →
   headers → ordering).
3. Avoid rows whose dependencies are still `todo` (e.g. tile group before frame
   header).
4. Pick a row with an existing OpenSpec change, or create one.
5. Implement, prove, update the matrix, regenerate the generated docs
   (`FEATURE-STATUS.md`, `SPEC-COVERAGE.md`), run `cargo xtask ci`.

For the current validator expansion plan, start with
[VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) (phases, current focus,
guardrails) and [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) (the
CI-enforced diagnostic registry). The matrix remains canonical. The rationale
for this whole tracking system is recorded in
[DECISIONS/0001-feature-tracking.md](./DECISIONS/0001-feature-tracking.md).

## 12. Diagnostic-ID convention

Validator diagnostics use a kebab/slash namespace with a documented prefix:
`obu-header/`, `obu-reserved/`, `bitstream/`, `trailing-bits/`,
`byte-alignment/`, `sequence-header/`, `sequence-state/`, `obu-order/`, `hls/`,
`msdo/`, `mfh/`, `content-interpretation/`, `frame-header/`, `tile-group/`,
`tile-params/`, `lcr/`, `atlas/`, `ops/`, `brt/`, `qm/`, `film-grain/`,
`padding/`, and `metadata/`.
Example: `obu-header/global-xlayer-required`.

A diagnostic that corresponds directly to a modeled feature MAY instead use the
Feature ID as a base, optionally with a `.SUFFIX` for a narrower rule:

```text
AV2-5.2.2-OBU-HEADER
AV2-5.2.2-OBU-HEADER.MISSING-EXTENSION-BYTE
AV2-B-ANNEXB-OBU-ENVELOPE.ZERO-LENGTH-OBU
```

The base id (before the `.SUFFIX`) must be a known matrix id. New kebab prefixes
must be added to the documented allowlist in `xtask/src/feature_status.rs` and
listed here.
Planned future namespaces are staged in the "Planned / not yet emitted"
section of [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) and should
be moved here when the corresponding xtask allowlist entry lands.
