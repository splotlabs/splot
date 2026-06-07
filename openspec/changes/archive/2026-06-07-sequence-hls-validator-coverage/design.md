# Design: sequence and HLS validator coverage

## Architecture

The change keeps the existing crate split:

```text
splot-cli -> splot-validate -> splot-core
splot-cli -> splot-core
```

No parser or validator logic moves into the CLI.

## Parser layering

`open_bitstream_unit(sz)` dispatches by OBU type. For `OBU_SEQUENCE_HEADER`, it calls the sequence parser. The sequence parser should be split into child parsers that map to AV2 §5.4 child rows:

```text
headers/sequence.rs
  read_sequence_header_obu()
  read_sequence_partition_config()
  read_sequence_segment_config()
  read_sequence_intra_config()
  read_sequence_inter_config()
  read_sequence_scc_config()
  read_sequence_tq_entropy_config()
  read_sequence_filter_config()
  read_sequence_tile_config()
  read_timing_info()
  read_seq_decoder_model_info()
```

A child parser must either:

1. fully parse its syntax and return a typed struct, or
2. stop at a feature-boundary with `Unimplemented { feature: "..." }` / `PayloadStatus::Unimplemented`, preserving the bounded payload slice.

It must never silently advance over unknown payload syntax.

## State model

`ValidatorContext` owns sequence/HLS state:

- `SequenceHeaderStore`: parsed sequence headers, payload fingerprints, active header per xlayer.
- `TemporalUnitState`: delimiter presence, HLS prefix state, xlayer ordering.
- `HlsAvailabilityStore`: in-band and optional external HLS objects.

The state is per validation run and deterministic.

## Strictness model

Default validation can report warnings for bounded unimplemented payload children if the repo already treats partial coverage that way. Strict mode should be allowed to upgrade unsupported normative payload syntax to an error when the relevant feature row is expected by the caller.

The mode must be explicit. Do not surprise normal users by making every todo child fatal unless that is already the validator policy.

## Sequence payload fingerprints

Use a simple stable hash/fingerprint over the sequence-header OBU payload bytes, not over a debug representation. The repeated-identical rule is about bitstream contents, and comparing only parsed fields can miss syntax differences with the same inferred values.

## Inspector output

`inspect --json` should distinguish:

```json
{
  "payload_status": {
    "status": "parsed",
    "feature_id": "AV2-5.4-SEQUENCE-HEADER"
  }
}
```

from:

```json
{
  "payload_status": {
    "status": "unimplemented",
    "feature_id": "AV2-5.4.11-USER-QM"
  }
}
```

The exact schema can follow existing code, but must remain machine-readable.

## Testing strategy

For every new parser:

- one positive minimal parse test;
- one branch/inferred-value test;
- EOF tests at field groups;
- validator diagnostic test where semantics exist;
- no-panic proptest if the parser is exposed to arbitrary bytes.

For state:

- sequence followed by OBU exceeding max tlayer/mlayer;
- repeated identical sequence header accepted;
- repeated non-identical sequence header rejected;
- duplicate temporal delimiter rejected;
- HLS after coded xlayer rejected;
- MSDO non-global rejected;
- MSDO `num_streams_minus_2 > 2` rejected.

## Documentation updates

Update:

- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/FEATURE-STATUS.md` (generated)
- `docs/SPEC-MAPPING.md` if module boundaries change
- `docs/VALIDATOR-ROADMAP.md` if phases are reclassified
- `STATUS.md` after implementation
