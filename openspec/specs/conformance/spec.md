# conformance Specification

## Purpose

Proof that the implementation matches the spec: no-panic fuzzing, inspector
snapshots, public vectors, and differential testing against AVM (the oracle).

Tracked by Feature IDs: `CONF-FUZZ-NO-PANIC`, `CONF-INSPECT-SNAPSHOTS`,
`CONF-PUBLIC-VECTORS`, `CONF-AVM-DIFF-HARNESS`.
## Requirements
### Requirement: parsers never panic

Arbitrary input SHALL never panic the parsers. This is covered on stable by the
`parsers_never_panic` proptest and, on nightly, by the `parse_obu` cargo-fuzz target.

#### Scenario: arbitrary bytes

- **WHEN** any byte slice is passed to the LEB128, OBU-header, and Annex B parsers
- **THEN** each returns `Ok`/`Err`, never panicking

### Requirement: AVM as oracle

Differential testing SHALL use AVM as a LOCAL oracle/generator only: AVM
locally produces AV2 streams that `splot validate` must validate clean (or flag
a real defect), and those small generated streams MAY be committed as plain
project fixtures. AVM SHALL NOT be vendored and SHALL NOT be a build or CI
dependency (no committed code path invokes AVM). Proof of the committed
vectors is recorded in the relevant matrix row's `[feature.proof]`.

#### Scenario: AVM-produced stream

- **WHEN** AVM locally encodes a stream and `splot validate` runs on it
- **THEN** the stream validates clean or a real defect is reported

### Requirement: vector licensing

Only redistributable/public vectors SHALL be committed: AVM-generated AV2
bitstreams (AVM is BSD-3-Clause-Clear) MAY be committed as project fixtures,
and samples whose license is unclear SHALL NOT be committed.

#### Scenario: unclear-license sample

- **WHEN** a vector's license is unclear
- **THEN** it is NOT committed to the repository

### Requirement: every untrusted-input surface has fuzz coverage

Every public entry point that consumes arbitrary bytes SHALL be reachable
from at least one cargo-fuzz target: the descriptor and OBU-envelope readers,
the IVF container parser, the container auto-detect, and the `splot-validate`
validator API (which transitively dispatches every OBU payload parser). The
CI fuzz-smoke job SHALL enumerate and run every target rather than a
hardcoded subset.

#### Scenario: a payload parser panics on hostile input

- **WHEN** any OBU payload parser reachable from `Validator::validate_bytes`
  panics, hangs, or exceeds the RSS limit on a fuzzer-generated input
- **THEN** the `validate_bytes` fuzz target crashes and the blocking CI
  fuzz-smoke job fails

#### Scenario: a new fuzz target is added

- **WHEN** a new target is added under `fuzz/fuzz_targets/`
- **THEN** the CI fuzz-smoke job and `cargo xtask fuzz` pick it up without a
  workflow edit (targets are enumerated, not hardcoded)

### Requirement: validator no-panic property tests on stable

`splot-validate` SHALL have property tests asserting that validating arbitrary
bytes under arbitrary validator options returns a report and never panics, so
the no-panic invariant is enforced on the stable toolchain in `cargo test`
where nightly-only fuzzing is unavailable.

#### Scenario: arbitrary input on stable

- **WHEN** `cargo test -p splot-validate` runs the property tests on the
  pinned stable toolchain
- **THEN** `Validator::validate_bytes_with_options` returns a
  `ValidationReport` for every generated input without panicking

### Requirement: committed conformance corpus

The committed conformance corpus under `tests/conformance/` SHALL be
self-contained and validate without AVM: a manifest maps each committed vector
to its expected validation outcome (clean, or a set of expected diagnostic
`rule_id`s), and a CI-reachable runner SHALL validate every manifest vector
with `splot-validate` and assert its expected outcome. The committed runner,
build, and CI SHALL NOT invoke or depend on AVM; AVM is only the local
generator or source seed described by each vector's manifest provenance.
Repository-retimed vectors SHALL identify the retiming in their manifest
description and SHALL NOT claim local reference-decoder evidence unless that
evidence is refreshed.

#### Scenario: committed valid vector validates clean

- **WHEN** the runner validates a committed vector whose manifest entry is
  `clean`
- **THEN** the validator reports no errors and the runner passes

#### Scenario: runner needs no AVM

- **WHEN** CI runs the conformance runner
- **THEN** it validates the committed vectors without invoking AVM or the
  network

### Requirement: targeted negative mutations

The validator SHALL be exercised by a committed, deterministic negative mutator:
for each `(committed valid seed, documented mutation, expected diagnostic)` row,
the mutated stream SHALL produce that registered diagnostic `rule_id` and SHALL
NOT panic. The mutations target stable, decidable diagnostics (IVF container,
OBU header, LEB128 framing); the expected `rule_id`s are existing registered
diagnostics, not new ones, and the mutator runs in CI without AVM or the
network.

#### Scenario: a malformed stream emits its expected diagnostic

- **WHEN** a documented mutation is applied to a committed valid seed and the
  result is validated
- **THEN** the validator emits the row's expected diagnostic `rule_id` and does
  not panic

#### Scenario: a conformant seed without mutation stays clean

- **WHEN** the unmutated seed is validated
- **THEN** the validator reports no errors (the mutation, not the seed, is the
  cause of the diagnostic)

### Requirement: diverse positive-vector coverage

The committed conformance corpus SHALL include valid streams from project-owned
synthetic input, either AVM-generated or explicitly provenance-noted local
retimings, spanning diverse codec feature combinations - at least multiple
resolutions, an 8-bit and a 10-bit stream, intra-only and inter, and an
operating-point-set stream - each validated against the manifest by the
committed runner with no AVM dependency. Streams AVM produces for external-HLS
provision (an absent global LCR, or a QM level with no QM OBU) MAY be committed
with their standalone-validation diagnostic as the expected outcome.

#### Scenario: a diverse clean stream validates clean

- **WHEN** the runner validates a committed self-contained stream (for example
  the 10-bit intra, operating-point-set, or retimed minimal runtime stream)
- **THEN** the validator reports no errors

#### Scenario: an external-HLS-dependent stream emits its availability diagnostic

- **WHEN** the runner validates a committed AVM stream that references a
  resource AVM expects to be provided externally (a global LCR, or a QM level),
  with external HLS disabled
- **THEN** the validator emits exactly that resource's availability diagnostic

### Requirement: inspect output golden snapshots

The conformance suite SHALL include `insta` golden snapshot tests of the `splot inspect
--json` output over a diverse set of committed fixtures, so any change to the inspector's
per-OBU JSON for a committed fixture is surfaced as a reviewable snapshot diff. The
inspector output is deterministic (per-OBU byte offsets, sizes, and parsed fields, with no
paths, timestamps, or filenames), so the snapshots are stable across runs and machines.

#### Scenario: inspector output is frozen

- **WHEN** `splot inspect --json` is run against a committed fixture
- **THEN** its output matches the committed golden snapshot for that fixture

#### Scenario: an output change is surfaced

- **WHEN** the inspector's JSON output for a committed fixture changes
- **THEN** the snapshot test fails with a diff that must be explicitly reviewed and accepted

### Requirement: CLI help-surface snapshots

The test suite SHALL freeze the `splot validate --help` and `splot inspect --help`
output as committed `insta` golden snapshots, so any change to those subcommands'
argument surface (a new, renamed, removed, or reordered flag, or a changed help
string) is surfaced as a reviewable snapshot diff. The snapshots SHALL be
deterministic — no filesystem paths, timestamps, or version strings — and the
top-level `splot --help` SHALL NOT be snapshotted.

#### Scenario: help surface unchanged

- **WHEN** the committed goldens match the current `validate`/`inspect` `--help`
- **THEN** the snapshot tests pass with no pending snapshots

#### Scenario: a flag is added or renamed

- **WHEN** a `validate` or `inspect` flag is added, renamed, or removed
- **THEN** the corresponding help snapshot diffs, requiring an explicit golden
  update in the same change

### Requirement: inspector text-output snapshots

The test suite SHALL freeze the `splot inspect` human (text) output — both the
default per-OBU dump and the `--headers` header-only dump — as committed `insta`
golden snapshots over representative committed fixtures, complementing the existing
`--json` snapshots. The text output is deterministic for a fixed input.

#### Scenario: text dump is stable

- **WHEN** `splot inspect` (default or `--headers`) runs against a committed fixture
- **THEN** its stdout matches the committed golden exactly

### Requirement: fixture manifest integrity gate

The committed `tests/fixtures/*.av2` corpus SHALL have a `tests/fixtures/MANIFEST.toml`
listing every fixture with a `name`, `path`, `sha256`, `description`, `category`
(`valid` | `validation-error` | `parse-error`), and `expect` (`"clean"` or a
`{ diagnostics = [...] }` error-rule-id set). `cargo xtask check-fixtures` SHALL
verify — hermetically, without running the validator, a decoder, or the network —
that every committed `.av2` fixture is listed exactly once, exists, hashes to its
recorded `sha256`, has a unique `name`/`path`, and a `category` consistent with
`expect`, failing with a problem count otherwise. It SHALL be folded into
`cargo xtask ci` and run as a CI step.

#### Scenario: corpus matches the manifest

- **WHEN** every committed fixture is listed with a matching `sha256` and a
  consistent `category`/`expect`
- **THEN** `cargo xtask check-fixtures` exits zero

#### Scenario: a fixture is mutated, missing, or unlisted

- **WHEN** a fixture's bytes change without updating its `sha256`, a manifest path
  is absent, or a committed `.av2` is not in the manifest
- **THEN** `cargo xtask check-fixtures` prints the offending fixture(s) and exits
  non-zero

### Requirement: fixture outcomes verified against the validator

Each fixture's `expect` SHALL be verified against the real validator
(`splot_validate::Validator::validate_bytes`, in-process — the same entry point as
`splot validate`, no external decoder) so a manifest outcome cannot silently drift.
The verification SHALL be anti-vacuous (the corpus exercises both the `clean` and
`diagnostics` arms) and SHALL fail on any committed fixture absent from the manifest.

#### Scenario: expect matches the validator

- **WHEN** the validator runs over a committed fixture
- **THEN** its error rule-id set equals the manifest's `expect` (or is empty for
  `expect = "clean"`)

### Requirement: runtime hash byte fuzz target

The repository SHALL provide a cargo-fuzz target named
`decode_runtime_hash_bytes`, tracked by Feature ID
`CONF-DECODE-RUNTIME-HASH-FUZZ`, that calls
`DecodeContext::decode_hash_report_bytes` with finite decode limits and no
external decoder, filesystem, or network dependency.

#### Scenario: arbitrary bytes return typed results

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it passes fuzz-selected bytes to
  `DecodeContext::decode_hash_report_bytes`
- **AND** success or failure is represented by the public typed return path
  without panicking
- **AND** the target does not invoke AVM, dav2d, ffmpeg, filesystem I/O, or the
  network

#### Scenario: fixture mutations exercise the minimal runtime success path

- **GIVEN** the committed `syn-flat-intra-64x64-minimal.ivf` fixture
- **WHEN** fuzz input selects the fixture-mutation mode
- **THEN** the target applies bounded deterministic mutations before calling
  `DecodeContext::decode_hash_report_bytes`
- **AND** an unmutated or still-supported input that decodes successfully is
  checked for the current minimal hash-report shape without claiming broad AV2
  decode support

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `decode_runtime_hash_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
- **AND** CI corpus seeding MAY include target-specific prefix seeds when a
  target consumes control bytes before the bitstream payload

### Requirement: reconstruction Y4M output serialization fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_y4m_output_bytes`, tracked by Feature ID
`CONF-RECON-Y4M-OUTPUT-FUZZ`, that builds bounded valid
`splot-recon` decoded frames from arbitrary bytes and serializes them through
`Y4mWriter` without filesystem, network, subprocess, AVM, dav2d, or ffmpeg
dependencies.

#### Scenario: structured decoded frames serialize without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into small valid decoded-frame inputs for
  supported Y4M bit-depth and pixel-format combinations
- **AND** it calls `Y4mWriter` to serialize stream headers, frame headers, and
  visible frame payloads
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: output serialization remains bounded

- **WHEN** fuzz input requests larger dimensions, extra frames, stride padding,
  or more sample data than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating sample buffers or serializing output

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_y4m_output_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files

### Requirement: reconstruction intra prediction fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_intra_prediction_bytes`, tracked by Feature ID
`CONF-RECON-INTRA-PREDICTION-FUZZ`, that builds bounded structured inputs for
existing `splot-recon` DC, PAETH, smooth, and current-frame workspace intra
prediction APIs without filesystem, network, subprocess, AVM, dav2d, or ffmpeg
dependencies.

#### Scenario: structured intra prediction inputs return typed results

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into bounded valid or intentionally typed-
  error inputs for the existing intra prediction and workspace APIs
- **AND** it calls the public prediction/workspace APIs for DC, PAETH, smooth,
  and source-backed workspace prediction cases
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: intra fuzzing remains bounded

- **WHEN** fuzz input requests larger block sizes, strides, workspace planes,
  sample buffers, or operation counts than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating buffers or invoking prediction code

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_intra_prediction_bytes` is included in target execution
  without hardcoding the executable target list in CI workflow files

### Requirement: runtime Y4M decode fuzz target
The repository SHALL provide a cargo-fuzz target named
`decode_runtime_y4m_bytes`, tracked by Feature ID
`CONF-DECODE-RUNTIME-Y4M-FUZZ`, that drives the existing
`DecodeContext::decode_y4m_bytes` byte-consuming API with bounded in-memory
inputs and writers without filesystem, network, subprocess, AVM, dav2d, or
ffmpeg dependencies.

#### Scenario: runtime Y4M byte inputs return typed results
- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it feeds either bounded raw input bytes or bounded mutations of the
  committed minimal IVF fixture into `DecodeContext::decode_y4m_bytes`
- **AND** successful decoding writes a complete in-memory Y4M stream for the
  current minimal runtime tier
- **AND** unsupported, malformed, resource-limit, or output failures are
  represented by public typed `DecodeError` returns without panicking

#### Scenario: runtime Y4M fuzzing remains bounded
- **WHEN** fuzz input requests larger raw input, mutation counts, decode work,
  tile payloads, decoded frames, reference storage, or output bytes than the
  target permits
- **THEN** the target clamps those values to fixed CI-safe limits before
  invoking the runtime Y4M API

#### Scenario: runtime Y4M writer behavior is in memory
- **WHEN** the fuzz target exercises successful output or caller-writer failure
  paths
- **THEN** it uses bounded in-memory writers
- **AND** it never creates, opens, renames, fsyncs, or deletes filesystem output
  paths

#### Scenario: smoke automation enumerates the target
- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `decode_runtime_y4m_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files

### Requirement: symbol decoder fuzz target
The repository SHALL provide a cargo-fuzz target named `symbol_decoder_bytes`,
tracked by Feature ID `CONF-SYMBOL-DECODER-FUZZ`, that drives the public AV2
§8.2 `splot_core::symbol::SymbolDecoder` byte-consuming API with bounded
payload bytes, bounded operation streams, and bounded CDF rows.

#### Scenario: arbitrary symbol inputs return typed results
- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it uses a bounded prefix as the tile payload for
  `SymbolDecoder::with_config`
- **AND** it drives a bounded sequence of `read_bool`, `read_literal`,
  `read_symbol`, and `exit_symbol` operations
- **AND** successful operation results satisfy local public-API invariants
- **AND** malformed CDF rows, invalid literal widths, exhausted payload bytes, or
  invalid symbol decoder states are represented by typed `splot_core::Error`
  returns without panicking

#### Scenario: symbol decoder fuzzing remains bounded
- **WHEN** fuzz input requests larger payloads, operation counts, literal
  widths, CDF row arities, CDF values, or CDF mutation counts than the target
  permits
- **THEN** the target clamps those values to fixed CI-safe limits before
  invoking the symbol decoder

#### Scenario: symbol decoder fuzzing does not claim tile decode
- **WHEN** the target is recorded in implementation and support status
- **THEN** the status text states that the target covers only the public §8.2
  symbol decoder primitive
- **AND** it does not claim §8.3 syntax-element CDF selection, default Tile or Saved
  CDF-bank initialization, tile traversal, partition decoding, block syntax,
  reconstruction, runtime hash output, runtime Y4M output, reference refresh,
  AVM evidence, dav2d evidence, filesystem I/O, network I/O, subprocesses, or new
  dependencies

#### Scenario: smoke automation enumerates the target
- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `symbol_decoder_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files

### Requirement: Tile payload runtime byte fuzzing

The conformance suite SHALL include a self-contained cargo-fuzz target named
`tile_payload_decode_bytes` that exercises the current minimal runtime tile
payload boundary through a feature-gated `splot-decode` fuzzing harness. The
target SHALL use bounded in-memory tile-payload bytes, SHALL run without
filesystem output, network access, subprocesses, AVM, dav2d, or ffmpeg, and
SHALL accept typed decode errors for malformed or unsupported mutations.

#### Scenario: Tile payload mutations stay panic-free

- **WHEN** `tile_payload_decode_bytes` mutates bounded tile-payload bytes and
  calls the fuzzing harness with finite limits
- **THEN** the call returns a typed success or typed decode error without
  panicking, hanging, writing files, or invoking external tools

#### Scenario: Successful mutation keeps frontier invariants

- **WHEN** a fuzz-generated tile-payload mutation reaches the boundary or
  minimal block-symbol frontier successfully
- **THEN** the target validates only stable boundary/frontier invariants such as
  single-tile work-unit shape, symbol initialization bounds, typed unsupported
  boundary metadata, and successful frontier summary bounds

### Requirement: Tile payload fuzz target remains self-contained in CI

The CI fuzz-smoke job SHALL enumerate and run `tile_payload_decode_bytes` with
the same bounded cargo-fuzz smoke policy as the other targets. Seed corpus setup
SHALL provide a minimal input for the target without adding large corpora or
external fixtures.

#### Scenario: CI discovers the new target

- **WHEN** CI runs the fuzz-smoke job after this change
- **THEN** `cargo +nightly fuzz list` includes `tile_payload_decode_bytes` and
  the job runs it without a hardcoded target subset

### Requirement: reconstruction frame hash serialization fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_frame_hash_bytes`, tracked by Feature ID
`CONF-RECON-FRAME-HASH-FUZZ`, that builds bounded valid `splot-recon` decoded
frames from arbitrary bytes and exercises `DecodedFrameHashInput` byte
serialization and digest computation without filesystem, network, subprocess,
AVM, dav2d, or ffmpeg dependencies.

#### Scenario: structured decoded frames hash without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into small valid decoded-frame inputs for
  supported bit-depth, sample-storage, pixel-format, crop, padding, and stride
  combinations
- **AND** it calls `DecodedFrameHashInput::byte_len`, `write_to`, and
  `compute_hash`
- **AND** success or writer failure is represented by the public typed return
  path without panicking

#### Scenario: hash input ignores non-visible frame storage and metadata

- **WHEN** two generated decoded frames have identical visible samples but
  different non-visible padding samples and output indices
- **THEN** the fuzz target verifies their emitted hash-input bytes and computed
  digests remain equal

#### Scenario: frame hash fuzzing remains bounded

- **WHEN** fuzz input requests larger dimensions, crop origins, storage padding,
  stride padding, or writer budgets than the target permits
- **THEN** the target clamps those values to fixed CI-safe bounds before
  allocating sample buffers, serializing output, or exercising failing writers

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_frame_hash_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files

### Requirement: reconstruction reference-frame store fuzz target

The repository SHALL provide a cargo-fuzz target named
`recon_reference_frame_store_bytes`, tracked by Feature ID
`CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`, that drives the public `splot-recon`
`ReferenceSlot` and `ReferenceFrameStore<F>` storage APIs with bounded arbitrary
operation sequences and no filesystem, network, subprocess, AVM, dav2d, or
ffmpeg dependencies.

#### Scenario: bounded operation sequences exercise store APIs without panics

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into bounded capacity, slot, payload, and
  operation inputs
- **AND** it calls public `ReferenceSlot` and `ReferenceFrameStore<F>` APIs for
  construction, containment, lookup, insertion, removal, clearing, occupancy,
  and entry iteration
- **AND** success or failure is represented by the public typed return path
  without panicking

#### Scenario: reference store state matches an oracle

- **WHEN** fuzz operations mutate a valid reference-frame store
- **THEN** occupied count, emptiness, slot contents, replacement returns, removal
  returns, and ascending entry order match a bounded oracle model after each
  checkpoint

#### Scenario: reference store fuzzing remains bounded

- **WHEN** fuzz input requests invalid capacities, invalid slots, or longer
  operation streams than the target permits
- **THEN** the target clamps operation count and payload size to fixed CI-safe
  bounds while preserving invalid-capacity and invalid-slot coverage through
  typed errors

#### Scenario: smoke automation enumerates the target

- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `recon_reference_frame_store_bytes` is included in target execution
  without hardcoding the executable target list in CI workflow files

