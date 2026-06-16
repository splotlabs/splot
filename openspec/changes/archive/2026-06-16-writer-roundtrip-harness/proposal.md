# Change: writer-roundtrip-harness

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)

## Why

The unified complete-OBU writer dispatch (`write_complete_obu`) now exists. The next backlog item is
the harness it was built for: a **`parse → write → reparse` round-trip** that proves, over arbitrary
OBU bytes, that writing a parsed OBU back out and reparsing yields the *same* model. This is the
writer's real specification — the inverse of the parser is only correct if `reparse(write(parse(x)))`
equals `parse(x)`. It also pins the no-panic invariant for the writer on adversarial input, mirroring
the parser fuzz targets, and surfaces over-strict writer guards (a writer that rejects a
*parser-produced* model is a bug the round-trip catches).

## What changes

- **Round-trip module** (`crates/splot-core/src/write/roundtrip.rs`, additive; no model change):
  - `recover_roundtrip_passthrough(payload: &[u8], parsed: &ParsedObu) -> WriteResult<Vec<u8>>` —
    recovers the opaque `passthrough` bytes a parsed OBU needs to be re-written. For padding it
    returns the real `obu_padding_byte` run (`payload[..padding_len]`; its byte values drive the
    parser's last-non-zero split, so they must be exact). For the metadata blobs (ITU-T T.35 / ICC /
    user-data / unknown-raw) it returns a **zero-fill of the modeled blob length** — the blob *values*
    are not modeled (only their length), so any bytes of the right length reparse to the same model,
    which is sufficient for the *semantic* round-trip. Allocations are bounded by `payload.len()` (you
    cannot recover more passthrough than the source payload holds), so a constructed model cannot
    drive an unbounded allocation.
  - `roundtrip_obu(header: &ObuHeader, payload: &[u8], parsed: &ParsedObu) -> RoundtripOutcome` —
    recovers the passthrough, writes the complete OBU via `write_complete_obu`, frames it with the
    Annex B `leb128(num_bytes_in_obu)` size prefix, reparses, and reports the outcome:
    `RoundTripped` (reparsed model equals the input), `Unwritable { feature }` (the OBU type has no
    body writer yet — `write_complete_obu` returned `Unimplemented`), or `Failed { reason }` (a
    writer reject of a parsed model, an unrecoverable passthrough, a reparse failure, or a model /
    header mismatch — each a round-trip defect). The function never panics (splot-core library
    policy); the caller decides what is a finding.
  - It uses `write_complete_obu` (not `write_obu_payload`) so a metadata group on the **global**
    `obu_xlayer_id == 31` layer-map branch round-trips (the header threads the real xlayer).

- **Fuzz target** (`fuzz/fuzz_targets/roundtrip_obu_bytes.rs`, registered in `fuzz/Cargo.toml`):
  partial-parses arbitrary bytes as an Annex B stream and, for every OBU whose payload parses to a
  `ParsedObu`, asserts `roundtrip_obu` returns `RoundTripped` or `Unwritable` (never `Failed`, never a
  panic).

## Validator impact

None.

## Non-goals

- **No** cross-tool `writer stream → splot validate clean` assertion yet (the next slice;
  `splot-core` cannot depend on `splot-validate`, so that test lives elsewhere).
- **No** new OBU-type body writers (the nine `Unimplemented` types stay `Unwritable`; each is a future
  slice).
- **No** byte-exact guarantee for metadata with non-zero opaque blobs (the zero-fill recovery is
  semantic-only; byte-exactness holds for the no-blob types and for padding, which recovers its real
  bytes). Documented in the module.
- **No** public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::roundtrip` surface).
- Fuzz: one new target (`roundtrip_obu_bytes`); `cargo xtask check-fuzz-targets` count 14 → 15; the CI
  per-target fuzz smoke covers it.
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (a WRITER note + proof on `ENC-BITSTREAM-WRITER`) +
  regenerated `docs/FEATURE-STATUS.md`; `AGENTS.md` fuzz-target list.
