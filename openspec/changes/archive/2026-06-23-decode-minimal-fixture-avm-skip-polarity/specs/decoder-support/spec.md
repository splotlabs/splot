## ADDED Requirements

### Requirement: Conformant minimal luma-skip fixture and AVM all_zero skip polarity

The committed `syn-flat-intra-64x64-minimal.ivf` fixture SHALL be an
AVM/dav2d-conformant 8-bit 4:2:0 64x64 intra key frame whose single 64x64 luma
transform block is coded as an all-zero (skipped) transform — AV2 § 5.20.7.27 /
AVM `decodetxb.c` `all_zero == 1` — over a real coded chroma residual, and the
frozen minimal-tier block-symbol trace SHALL assert that same `all_zero == 1` skip
polarity for the luma and V `txb_skip` symbols rather than the inverted `== 0`.
The fixture SHALL decode byte-for-byte identically under `avmdec` and `dav2d`
(recorded as a `reference-output-agreement` entry in
`docs/LOCAL-REFERENCE-EVIDENCE.toml`), and `splot` SHALL reproduce that raw output
byte-for-byte through the general intra path (`DECODE-GENERAL-INTRA-FRAME-RECON`),
not through the frozen `base_q_idx == 255` hand-traced path. The retired
hand-retimed payload that coded the skip symbol with inverted polarity SHALL be
rejected by the corrected frozen trace with a typed symbol-mismatch error. This
requirement SHALL NOT change the general intra decode algorithm, remove the frozen
minimal-tier code, or add new decode tools, partitions, in-loop filters, inter
prediction, tiles, AVM/dav2d source, dependencies, or required CI jobs.

#### Scenario: The conformant luma-skip fixture decodes bit-exact through the general path

- **WHEN** `splot decode` runs on the committed `syn-flat-intra-64x64-minimal.ivf`
  fixture
- **THEN** it routes off the frozen `base_q_idx == 255` path into the general intra
  path and decodes the luma plane as a flat all-zero (skipped) block at the
  no-neighbour DC value 128 over a coded (non-flat) chroma residual
- **AND** the decoded raw output is byte-for-byte identical to the `avmdec` and
  `dav2d` raw output (raw md5 `f618317b…`; `splot-dfh-sha256-v1`
  `92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af`)

#### Scenario: The frozen trace asserts the AVM all_zero skip polarity

- **WHEN** the frozen minimal-tier block-symbol trace consumes a transform block's
  luma or V `txb_skip` symbol for a skipped (all-zero) transform
- **THEN** it asserts the decoded `all_zero` symbol is `1` (the AVM skip value),
  and the retired inverted-polarity payload (which decoded the symbol to `0`) is
  rejected with a typed `expected: 1, actual: 0` symbol mismatch

#### Scenario: The frozen base_q_idx==255 path remains without a committed conformant fixture

- **WHEN** decoder support status is rendered
- **THEN** the minimal-tier rows report that the committed fixture is decoded by
  the general intra path with `avmdec`/`dav2d` agreement
- **AND** the frozen `base_q_idx == 255` hand-traced path stays in code (with the
  corrected skip polarity) but is no longer exercised by any committed conformant
  fixture, because no AVM-producible 64x64 intra frame is an all-planes skip
