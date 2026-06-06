# Validator implementation matrix expansion

`status: draft`  
`owner: validator/core`  
`canonical target: docs/IMPLEMENTATION-MATRIX.toml`

This document is a copy/paste aid for expanding the canonical matrix. It is **not** canonical. After editing the matrix, run:

```bash
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask check-feature-status
cargo xtask spec-coverage
cargo xtask ci
```

## 1. Matrix principles for validator work

- Keep existing completed rows stable. Do not rename `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`, `AV2-5.2.1-OBU-TYPE`, `AV2-B-ANNEXB-OBU-ENVELOPE`, or `AV2-5.3-RESERVED-OBU`.
- Split large rows before implementation. Sequence header, frame header, tile group, metadata, LCR, and OPS are too large for one row.
- The umbrella row remains `partial` until all child rows are done.
- `mapped = done` is allowed when the row is identified and linked. `parse = done`, `validate = done`, `tests = done`, and `decode_check = done` need proof entries.
- Use OpenSpec change IDs that match the change folder, e.g. `parse-sequence-header` or `validator-coverage-roadmap`.
- Use `pending` for `avm_diff` until AVM/public-vector proof is recorded.

## 2. Existing rows to revise, not replace

### `AV2-5.2.3-TRAILING-BITS`

Current notes say the reserved-OBU all-zero-payload check partially exercises trailing semantics. Revise when implementing the actual parser:

```toml
[[feature]]
id = "AV2-5.2.3-TRAILING-BITS"
name = "Trailing bits syntax and semantics"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.2.3", "6.2.3"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/obu.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "medium"
notes = "Parse trailing_bits(nbBits), enforce trailing_one_bit and zero padding, and share diagnostics with payload parsers."
[feature.status]
mapped = "done"
types = "partial"
parse = "todo"
validate = "todo"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

### `AV2-5.2.4-BYTE-ALIGNMENT`

```toml
[[feature]]
id = "AV2-5.2.4-BYTE-ALIGNMENT"
name = "Byte alignment syntax and semantics"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.2.4", "6.2.4"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/bitio.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "medium"
notes = "Model byte_alignment() as a syntax structure, not only as a reader helper. Zero bits must be validated."
[feature.status]
mapped = "done"
types = "partial"
parse = "todo"
validate = "todo"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 3. Descriptor rows to add

### `AV2-4.11.3-UVLC`

```toml
[[feature]]
id = "AV2-4.11.3-UVLC"
name = "Unsigned variable-length code descriptor"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["4.11.3"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/bitio.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Needed by sequence_header_obu(), metadata, LCR, OPS, frame header, and many later payload parsers. Must be bounded and panic-free."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "not-applicable"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

### `AV2-4.11.5-LE`

```toml
[[feature]]
id = "AV2-4.11.5-LE"
name = "Little-endian fixed-width descriptor"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["4.11.5"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/bitio.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "medium"
notes = "Needed by payload syntax that uses le(n). Implement only from AV2 descriptor semantics."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "not-applicable"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

### `AV2-4.11.8-NS`

```toml
[[feature]]
id = "AV2-4.11.8-NS"
name = "Non-symmetric integer descriptor"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["4.11.8"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/bitio.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Needed by sequence and frame syntax. Implement with tests for powers of two, non-powers, n=1, EOF, and malformed boundaries."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "not-applicable"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 4. OBU dispatch row to add

```toml
[[feature]]
id = "AV2-5.2.1-OBU-DISPATCH"
name = "open_bitstream_unit payload dispatch"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.2.1"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/obu.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Dispatch OBU payloads by AV2 obu_type after parsing obu_header(). Initially partial: variants are added as child syntax parsers land."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "not-applicable"
write = "todo"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 5. Sequence header split rows

Keep the existing `AV2-5.4-SEQUENCE-HEADER` umbrella, but add child rows. Use `crates/splot-core/src/headers/sequence.rs` or `crates/splot-core/src/headers.rs` depending on the module split selected in the PR.

### Recommended child row template

Copy this and replace the placeholders.

```toml
[[feature]]
id = "AV2-<5.4 child section>-<SEQUENCE-CHILD-SLUG>"
name = "Sequence header child syntax: human-readable name"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.4.X", "6.4.X"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/headers/sequence.rs"
openspec_change = "parse-sequence-header"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Child row of AV2-5.4-SEQUENCE-HEADER. Do not mark umbrella done until this is done."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "todo"
write = "todo"
encode = "todo"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

### Child IDs

```text
AV2-5.4.1-SEQUENCE-HEADER-GENERAL
AV2-5.4.2-SEQUENCE-TILE-CONFIG
AV2-5.4.3-SEQUENCE-PARTITION-CONFIG
AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG
AV2-5.4.5-SEQUENCE-INTRA-CONFIG
AV2-5.4.6-SEQUENCE-INTER-CONFIG
AV2-5.4.7-SEQUENCE-SCC-CONFIG
AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG
AV2-5.4.9-SEGMENT-INFO
AV2-5.4.10-SEQUENCE-FILTER-CONFIG
AV2-5.4.11-USER-QM
AV2-5.4.12-TIMING-INFO
AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO
AV2-6.4-SEQUENCE-HEADER-SEMANTICS
```

### First concrete sequence row: §5.4.1

```toml
[[feature]]
id = "AV2-5.4.1-SEQUENCE-HEADER-GENERAL"
name = "General sequence header syntax"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.4.1", "6.4.1"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/headers/sequence.rs"
openspec_change = "parse-sequence-header"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Parse sequence_header_obu() through general fields, dimensions, cropping, timing flags, layer dependency maps, and calls into child config parsers as implemented."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "todo"
write = "todo"
encode = "todo"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

### Sequence semantics row

```toml
[[feature]]
id = "AV2-6.4-SEQUENCE-HEADER-SEMANTICS"
name = "Sequence header OBU semantics"
category = "normative"
kind = "validator-check"
spec_sections = ["6.4"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-validate"
module = "crates/splot-validate/src/checks/sequence.rs"
openspec_change = "parse-sequence-header"
tracking_issue = ""
owner = "validator"
risk = "high"
notes = "Validator checks for locally decidable sequence header semantics. Split into child rows if the file becomes too broad."
[feature.status]
mapped = "done"
types = "todo"
parse = "not-applicable"
validate = "todo"
write = "not-applicable"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 6. Activated sequence header limits row

```toml
[[feature]]
id = "AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS"
name = "OBU header layer ids constrained by activated sequence header"
category = "normative"
kind = "validator-check"
spec_sections = ["6.2.2", "6.4", "7.3.8"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-validate"
module = "crates/splot-validate/src/validator.rs"
openspec_change = "parse-sequence-header"
tracking_issue = ""
owner = "validator"
risk = "high"
notes = "Completes the §6.2.2 checks that require max_tlayer_id and max_mlayer_id from an activated sequence header. Depends on sequence-header parsing and stateful validation context."
[feature.status]
mapped = "done"
types = "todo"
parse = "not-applicable"
validate = "todo"
write = "not-applicable"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

Suggested diagnostics:

```text
obu-header/tlayer-exceeds-sequence-max
obu-header/mlayer-exceeds-sequence-max
obu-header/no-active-sequence-header
```

## 7. OBU ordering and HLS rows

The existing `AV2-7.3-OBU-ORDERING` row should become an umbrella. Add child rows before coding.

```text
AV2-7.3.2-CMVS-BOUNDARIES
AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT
AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT
AV2-7.3.5-CODED-FRAME-UNIT
AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT
AV2-7.3.7-TEMPORAL-UNIT-ORDER
AV2-7.3.8-HLS-AVAILABILITY
AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY
```

Use this template:

```toml
[[feature]]
id = "AV2-7.3.7-TEMPORAL-UNIT-ORDER"
name = "Temporal unit OBU order"
category = "normative"
kind = "bitstream-semantics"
spec_sections = ["7.3.7"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-validate"
module = "crates/splot-validate/src/checks/ordering.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "validator"
risk = "high"
notes = "Temporal unit state machine: global temporal delimiter, HLS prefix OBUs, coded extended layer units in ascending xlayer order, and padding exceptions."
[feature.status]
mapped = "done"
types = "todo"
parse = "not-applicable"
validate = "todo"
write = "not-applicable"
encode = "not-applicable"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 8. Missing top-level §5 OBU rows to add or split

Some rows already exist as umbrellas. Add missing ones and split them as implementation starts.

```text
AV2-5.5-TEMPORAL-DELIMITER
AV2-5.6-MSDO
AV2-5.7-MULTI-FRAME-HEADER
AV2-5.9-ATLAS-SEGMENT
AV2-5.11-OPERATING-POINT-PAYLOAD
AV2-5.12-BUFFER-REMOVAL-TIMING
AV2-5.13-QUANTIZATION-MATRIX
AV2-5.14-FILM-GRAIN
AV2-5.15-CONTENT-INTERPRETATION
AV2-5.16-PADDING
AV2-5.17-METADATA
```

Existing umbrellas to split:

```text
AV2-5.8-LAYER-CONFIG-RECORD
AV2-5.10-OPERATING-POINT-SET
AV2-5.18-FRAME-HEADER
AV2-5.19-TILE-GROUP
AV2-5.20-TILE-GROUP-PAYLOAD
AV2-9-ADDITIONAL-TABLES
```

Top-level template:

```toml
[[feature]]
id = "AV2-<5.x section>-<SLUG>"
name = "Human-readable syntax name"
category = "normative"
kind = "bitstream-syntax"
spec_sections = ["5.X", "6.Y"]
sources = ["https://av2.aomedia.org/v1.0.0/index.html"]
crate = "splot-core"
module = "crates/splot-core/src/headers.rs"
openspec_change = "validator-coverage-roadmap"
tracking_issue = ""
owner = "core"
risk = "high"
notes = "Payload parser and validator hooks. Split before implementation if broad."
[feature.status]
mapped = "done"
types = "todo"
parse = "todo"
validate = "todo"
write = "todo"
encode = "todo"
decode_check = "todo"
tests = "todo"
avm_diff = "pending"
perf = "not-applicable"
[feature.proof]
tests = []
commands = []
fixtures = []
diagnostics = []
```

## 9. Frame header child rows

Do not implement `AV2-5.18-FRAME-HEADER` as one row. Add:

```text
AV2-5.18.1-FRAME-HEADER-GENERAL
AV2-5.18.2-FRAME-HEADER-INFO
AV2-5.18.3-FRAME-CONFIGURATION
AV2-5.18.4-FRAME-SIZE
AV2-5.18.5-FILTERING
AV2-5.18.6-QUANTIZATION
AV2-5.18.7-SEGMENTATION-TILING
AV2-5.18.8-TRANSFORM-CODING-MODES
AV2-5.18.9-GLOBAL-MOTION
AV2-5.18.10-FILM-GRAIN-STRUCTURES
```

Matching semantics rows can use `AV2-<6.17 child section>-<SLUG>` IDs when implementation begins.

## 10. Metadata child rows

Add these before implementing metadata:

```text
AV2-5.17.1-METADATA-UNIT
AV2-5.17.2-METADATA-SHORT
AV2-5.17.3-METADATA-GROUP
AV2-5.17.4-METADATA-ITUT-T35
AV2-5.17.5-METADATA-HDR-CLL
AV2-5.17.6-METADATA-HDR-MDCV
AV2-5.17.7-METADATA-TIMECODE
AV2-5.17.8-METADATA-BANDING-HINTS
AV2-5.17.9-METADATA-ICC-PROFILE
AV2-5.17.10-METADATA-SCAN-TYPE
AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO
AV2-5.17.12-METADATA-DECODED-FRAME-HASH
AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED
```

## 11. Annex and conformance rows

Add these after enough syntax exists to make them actionable:

```text
AV2-A-PROFILES
AV2-A-LEVELS-TIERS
AV2-E-DECODER-MODEL
CONF-AVM-PARSER-TRACES
CONF-AVM-VALID-STREAMS
CONF-AVM-INVALID-STREAMS
CONF-PUBLIC-VECTOR-LICENSE-REVIEW
```

## 12. Suggested matrix proof pattern

When a parser/check is done, update proof like this:

```toml
[feature.proof]
tests = [
  "crates/splot-core/src/headers/sequence.rs::tests",
  "crates/splot-validate/src/checks/sequence.rs::tests",
]
commands = [
  "cargo test -p splot-core sequence_header",
  "cargo test -p splot-validate sequence_header",
]
fixtures = [
  "tests/fixtures/sequence-header-minimal.av2",
  "tests/fixtures/sequence-header-bad-crop.av2",
]
diagnostics = [
  "sequence-header/chroma-format-out-of-range",
  "sequence-header/bit-depth-out-of-range",
]
```
