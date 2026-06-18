## ADDED Requirements

### Requirement: Baseline Encoder Profile v1 contract

The encoder program SHALL define Baseline Encoder Profile v1 before production
encoder implementation resumes. The profile SHALL target 8-bit and 10-bit YUV420
Y4M input, raw Annex B and IVF output, deterministic threaded operation, all-intra
legal streams before basic inter streams, closed-loop reconstruction before public
success, and validation/differential evidence before any supported path is marked
done.

#### Scenario: profile is documented before implementation

- **WHEN** the encoder program contract change is complete
- **THEN** the Baseline Encoder Profile v1 scope and non-goals are documented in
  `docs/ENCODER-GOAL.md`
- **AND** the current gaps are documented in `docs/ENCODER-GAP-AUDIT.md`.

### Requirement: First encoder flight is docs only

The `encoder-program-contract` change SHALL NOT change Rust production behavior,
crate manifests, crate dependency direction, public encoder success behavior,
RangeEncoder behavior, rate control, speed presets, or external codec integration.

#### Scenario: first flight has no production behavior change

- **WHEN** the first encoder-program contract PR is reviewed
- **THEN** its changed files are limited to documentation, OpenSpec artifacts, and
  implementation-matrix/generated status metadata
- **AND** `splot encode` still reports the existing unimplemented encoder state.

### Requirement: Encoder PR sequencing and evidence

Every non-trivial encoder PR SHALL name stable Feature IDs, include an OpenSpec
change unless trivial, record proof in `docs/IMPLEMENTATION-MATRIX.toml`, keep CI
self-contained, and avoid overlapping ownership with active PRs. Encoder PRs SHALL
not merge until both GitHub Claude and GitHub Codex have accepted the final HEAD.

#### Scenario: implementation PR declares ownership

- **WHEN** an encoder implementation PR is opened
- **THEN** its PR text names the Feature IDs, OpenSpec change, changed ownership
  surface, proof commands, and any active PR overlap audit.

### Requirement: External reference hygiene

The encoder program SHALL derive AV2 behavior from the AV2 v1.0.0 specification
mirror and AVM. rav1e and SVT-AV1 SHALL be used only as engineering inspiration;
the program SHALL NOT copy AV1 syntax, source code, constants, entropy CDFs,
tables, comments, or prose.

#### Scenario: encoder change cites lawful sources

- **WHEN** an encoder change touches syntax, reconstruction, reference state, or
  layer behavior
- **THEN** the change cites the AV2 spec mirror path and matrix Feature ID
- **AND** any rav1e or SVT-AV1 influence is described as inspiration only.

### Requirement: Recon dependency decision is isolated

After the docs-only contract lands, the next encoder change SHALL be
`encoder-recon-dependency`. That change SHALL explicitly decide whether and how
`splot-encode` depends on `splot-recon`.

#### Scenario: recon dependency is not implicit

- **WHEN** the contract PR is complete
- **THEN** no `splot-encode -> splot-recon` dependency exists yet
- **AND** the dependency decision is reserved for `encoder-recon-dependency`.
