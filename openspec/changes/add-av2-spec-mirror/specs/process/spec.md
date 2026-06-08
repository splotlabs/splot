# process delta: add-av2-spec-mirror

## ADDED Requirements

### Requirement: AV2 spec grounding via the committed mirror

Development that asserts AV2 syntax, constants, tables, or semantics SHALL ground
those claims in the committed AV2 specification mirror under `docs/spec/av2/<version>/`,
treating it as the canonical offline source of truth alongside the upstream AOM
PDF/HTML. Contributors and agents SHALL NOT invent spec behavior; where a detail
is intentionally unmodeled, the existing `TODO(spec: <FEATURE-ID>)` convention
applies. Tracked by `DOC-AV2-SPEC-MIRROR`.

#### Scenario: a change cites AV2 behavior

- **WHEN** a code comment, diagnostic, or document states an AV2 syntax element,
  constant, table, or semantic rule
- **THEN** it is traceable to a `§` section resolvable in the committed mirror
  (via `index.md`), not to memory or an uncited external source

#### Scenario: spec text is needed offline

- **WHEN** an agent or reviewer needs the exact normative wording of an AV2
  section while working in the repository
- **THEN** the text is available from the committed mirror without network access
