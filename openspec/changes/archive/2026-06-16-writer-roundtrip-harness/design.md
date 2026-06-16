# Design: writer-roundtrip-harness

## The round-trip contract

For a parsed OBU `x = parse(bytes)`, the writer is the inverse of the parser iff
`parse(write(x)) == x` (semantic round-trip). The harness checks exactly this over the complete-OBU
dispatch: `parse → write_complete_obu → reparse → assert model equality`.

## Passthrough recovery — the key idea

`ParsedObu` does not hold opaque bytes; `write_complete_obu` takes them as a `passthrough: &[u8]`.
To re-write a parsed OBU the harness must reconstruct that passthrough from the original payload:

- **Padding** (`§ 5.16`): the parser splits at the *last non-zero byte*, so the `obu_padding_byte`
  run is exactly `payload[..padding_len]`. Its byte values determine the split on reparse, so they
  must be recovered exactly. Recovered as a real slice.
- **Metadata blobs** (`§ 5.17.9–§ 5.17.13`: ITU-T T.35, ICC, user-data, unknown-raw): the model
  stores only the blob *length* (`payload_len` / `raw_len`), not its bytes. Therefore **any** bytes
  of that length reparse to the same model. The harness returns a **zero-fill of the modeled length**
  — no fragile re-derivation of per-unit byte offsets is needed, and the *semantic* round-trip holds.
  (Byte-exactness does not hold for a non-zero blob; that is documented and out of scope here, since
  the model genuinely cannot represent the blob bytes.)
- **Everything else** (temporal delimiter, sequence header, fully-modeled / cancelled metadata
  units): empty passthrough.

The metadata-group flat passthrough length is the sum of each unit's modeled blob length (the same
`metadata_group_unit_passthrough_len` the dispatch uses to split it). The recovery allocates at most
`payload.len()` bytes (a parsed model's blob lengths are sub-slices of the payload), and rejects a
constructed model whose lengths exceed the payload — both correctness and an OOM guard.

## Why `write_complete_obu`, not `write_obu_payload`

A metadata group on the **global** layer-map branch (`obu_xlayer_id == 31`, `§ 6.16.3`) encodes its
layer maps differently from the local branch. `write_obu_payload` has no OBU header so it always
writes the *local* branch; `write_complete_obu` threads `header.extended_layer_id`, so it round-trips
both branches. The harness therefore writes the **complete** OBU (header + payload) and reframes it
with the Annex B `leb128(num_bytes_in_obu)` size prefix (`write_complete_obu` output is exactly the
`header ++ payload` that the size prefix wraps), then reparses via `parse_annex_b_obus`.

## Outcome model (no panic in the library)

`roundtrip_obu` returns a `RoundtripOutcome` rather than asserting, honoring the splot-core no-panic
policy:

- `RoundTripped` — wrote and reparsed to an equal model.
- `Unwritable { feature }` — `write_complete_obu` returned `Unimplemented` (the nine OBU types with no
  body writer yet); the harness skips them, like the parser fuzz target skips unparsed payloads.
- `Failed { reason }` — a writer reject of a parsed model, an unrecoverable passthrough, a reparse
  failure, or a header / model mismatch. For a *parser-produced* model this is always a defect, so
  the fuzz target and the unit tests treat any `Failed` as a finding.

The fuzz target (outside the workspace, where panics are the libFuzzer signal) asserts the outcome is
`RoundTripped` or `Unwritable`; a `Failed` or a panic is a crash libFuzzer records with the input.

## Scope

This slice is the round-trip property + its fuzz target only. The cross-tool
`writer stream → splot validate clean` assertion is the next slice (it cannot live in `splot-core`,
which must not depend on `splot-validate`). No new OBU-type body writers; no public `encode` command.
