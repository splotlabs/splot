## ADDED Requirements

### Requirement: Writer baseline is syntax and framing, not entropy coding

The encoder tool contract SHALL distinguish the current `splot-core` writer
baseline from an encoder. The writer can emit supported parsed syntax structures
and container framing, but it SHALL NOT be treated as able to generate entropy-coded
tile payloads while `RangeEncoder` and the `decode_tile()` body remain unimplemented.

#### Scenario: entropy-coded tiles are not claimed

- **WHEN** encoder documentation describes current writer support
- **THEN** it states that coded tile payload generation is still a gap
- **AND** no public encoder milestone depends on fabricated coded tile bytes.

### Requirement: Closed-loop reconstruction reuse is gated

The encoder program SHALL treat `splot-recon` as available lower-level
reconstruction building blocks, not as an integrated encoder reconstruction loop,
until the `encoder-recon-dependency` change lands.

#### Scenario: recon APIs are not pulled in by the contract PR

- **WHEN** the encoder-program contract PR is reviewed
- **THEN** `splot-encode` still depends only on `splot-core` and `splot-parallel`
- **AND** the recon reuse boundary is documented as future work.

### Requirement: Parked toy intra change is superseded

The parked `toy-intra-encoder-v0` change SHALL NOT be resumed directly. Future
all-intra encoder work SHALL be re-proposed under the Baseline Encoder Profile v1
contract with current writer, reconstruction, validation, and conformance gates.

#### Scenario: toy encoder work restarts under a new proposal

- **WHEN** all-intra encoder implementation resumes
- **THEN** it uses a new or updated OpenSpec change tied to the Baseline Encoder
  Profile v1 contract
- **AND** the parked `toy-intra-encoder-v0` tasks remain unchecked.
