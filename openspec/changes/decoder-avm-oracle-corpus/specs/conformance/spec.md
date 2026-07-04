# conformance delta: decoder-avm-oracle-corpus

## ADDED Requirements

### Requirement: Decoder-output oracle differential

The committed runner SHALL compare `splot` decoded output against recorded AVM
decoder oracle hashes over the committed corpus, without invoking AVM, requiring
an AVM checkout, or touching the network.

Each committed valid `.ivf` has a `tests/conformance/decoder-oracle.toml` entry
recording the AVM raw-output SHA-256 (`avmdec --i420 --rawvideo`, visible I420
samples) and an expected `splot` outcome: `must_pass` (decode output equals the
oracle) or `xfail_splot` (decode fails closed with a recorded
`decode/unsupported-feature` diagnostic).

#### Scenario: must_pass fixture matches the AVM oracle

- **WHEN** the runner decodes a `must_pass` fixture with `splot`
- **THEN** the raw I420 output SHA-256 equals the recorded AVM oracle hash

#### Scenario: xfail fixture fails closed

- **WHEN** the runner decodes an `xfail_splot` fixture with `splot`
- **THEN** decode fails with the recorded `decode/unsupported-feature` rule id
  and unsupported reason, and normal CI is not blocked

#### Scenario: no decodable stream hides

- **WHEN** a committed valid `.ivf` has no `decoder-oracle.toml` entry
- **THEN** `cargo xtask decoder-fixtures verify` and the CI runner both fail

### Requirement: Decoder-oracle coverage report

`cargo xtask decoder-fixtures coverage` SHALL generate a coverage report that
lists every decoder-relevant capability in the taxonomy and its fixture backing,
and `cargo xtask ci` SHALL fail when the committed report drifts.

#### Scenario: non-emittable capability is marked

- **WHEN** a taxonomy capability cannot be produced by the local AVM encoder
- **THEN** the report marks it `not_fixtureable_with_avm_encoder` rather than
  claiming coverage
