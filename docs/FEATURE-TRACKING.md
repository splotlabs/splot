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
OpenSpec change (openspec/changes/<change-id>/)
      ↓ defines intent for
Feature ID in docs/IMPLEMENTATION-MATRIX.toml   ← canonical
      ↓ referenced by
code module / diagnostic rule / test / fuzz target / CLI behavior (crates/, fuzz/)
      ↓ proven by
proof recorded in the matrix row
      ↓ enforced by
cargo xtask check-feature-status   (also in cargo xtask ci and CI)
      ↓ scheduled in
GitHub issue / PR (references the same Feature ID)
```

GitHub Issues/Projects are an execution queue and the README status table is a
snapshot. When either disagrees with the matrix, **the matrix wins.**

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

The same ID appears in: the matrix row, the OpenSpec change, the GitHub
issue/PR, the diagnostic rule id (when applicable), tests, `TODO(spec: <id>)`
comments, and docs. ID stability and the `replaces` mechanism are defined in
[IMPLEMENTATION-MATRIX.schema.md](./IMPLEMENTATION-MATRIX.schema.md) (§ 6–§ 7).

## 4. Status model

Ten stages per feature: `mapped`, `types`, `parse`, `validate`, `write`,
`encode`, `decode_check`, `tests`, `avm_diff`, `perf`. Each stage is one of
`todo`, `partial`, `done`, `blocked`, `not-applicable`, `experimental`,
`pending` — **never** percentages. Full definitions and a good/bad status
example: [the schema](./IMPLEMENTATION-MATRIX.schema.md).

## 5. Matrix schema (summary)

Each `[[feature]]` row has `id`, `name`, `category`, `kind`, `spec_sections`,
`sources`, `crate`, `module`, `openspec_change`, `tracking_issue`, `owner`, `risk`,
`notes`, a `[feature.status]` table (the ten stages), and a `[feature.proof]` table
(`tests`, `commands`, `fixtures`, `diagnostics`). The full schema, allowed values,
and proof rules are in [IMPLEMENTATION-MATRIX.schema.md](./IMPLEMENTATION-MATRIX.schema.md).

Render, check, and regenerate (the canonical command list — other sections
point here):

```bash
cargo xtask feature-status                 # aligned table
cargo xtask feature-status --format json   # for tooling
cargo xtask feature-status --category normative
cargo xtask feature-status --kind bitstream-syntax
cargo xtask spec-coverage                  # coverage summary
cargo xtask check-feature-status           # fail on drift

# Regenerate the committed generated docs:
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md
```

## 6. Workflow: a new AV2 syntax feature

1. Create an OpenSpec change under `openspec/changes/<change-id>/`
   (proposal + tasks).
2. Add a row to [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml)
   with stages at `todo`.
3. Implement strong types in `crates/splot-core/src/...`.
4. Implement the parser (panic-free; errors, never panics).
5. Add validator diagnostics in `crates/splot-validate` (stable rule ids).
6. Add tests plus fuzz/property coverage: positive, negative, EOF.
7. Update `[feature.proof]` and status; bump a stage to `done` only with proof.
8. Regenerate the generated docs with the `--output` commands in § 5.
9. Run `cargo xtask check-feature-status && cargo xtask ci`.
10. Open a PR with the Feature ID in the title/body.

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

A code stage may be `done` only when the row's `[feature.proof]` records
evidence; the exact proof rules, and their enforcement by
`cargo xtask check-feature-status`, are defined in
[IMPLEMENTATION-MATRIX.schema.md](./IMPLEMENTATION-MATRIX.schema.md) (§ 5.3).
When in doubt, mark `partial` and explain in `notes`.

## 10. Splitting large features

A row too big for one PR (for example `AV2-5.18-FRAME-HEADER`) stays as an
umbrella with narrower child rows whose ids extend it; the full splitting rules
are in [IMPLEMENTATION-MATRIX.schema.md](./IMPLEMENTATION-MATRIX.schema.md)
(§ 7).

## 11. How an agent chooses the next task

1. Scan the generated [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) — blank/partial
   cells in spec order are the open work.
2. Prefer **validator-first** rows: `parse`/`validate` on normative bitstream
   syntax with `risk = high`, in dependency order (LEB128 → OBU header → Annex B →
   headers → ordering).
3. Avoid rows whose dependencies are still `todo` (e.g. tile group before frame
   header).
4. Pick a row with an existing OpenSpec change, or create one.
5. Implement, prove, update the matrix, regenerate the generated docs
   (commands in § 5), run `cargo xtask ci`.

For the current validator expansion plan, start with
[VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) (phases, current focus,
guardrails) and [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) (the
CI-enforced diagnostic registry). The matrix remains canonical. The rationale
for this whole tracking system is recorded in
[DECISIONS/0001-feature-tracking.md](./DECISIONS/0001-feature-tracking.md).

## 12. Diagnostic-ID convention

Validator diagnostics use a kebab/slash namespace with a documented prefix,
for example `obu-header/`, `sequence-header/`, `frame-header/`, and
`decoder-model/` (decoder-model buffer-delay sum-constancy, §6.4.13 / §6.10.5).
The canonical allowlist is the `DIAGNOSTIC_PREFIXES` constant in
`xtask/src/feature_status.rs`;
[VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md) groups every emitted
rule by namespace. Example: `obu-header/global-xlayer-required`.

A diagnostic that corresponds directly to a modeled feature MAY instead use the
Feature ID as a base, optionally with a `.SUFFIX` for a narrower rule:

```text
AV2-5.2.2-OBU-HEADER
AV2-5.2.2-OBU-HEADER.MISSING-EXTENSION-BYTE
AV2-B-ANNEXB-OBU-ENVELOPE.ZERO-LENGTH-OBU
```

The base id (before the `.SUFFIX`) must be a known matrix id. A new kebab
prefix lands by adding it to `DIAGNOSTIC_PREFIXES` and documenting its rules in
[VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md); planned namespaces stay
in that registry's "Planned / not yet emitted" section until their first rule
is emitted.
