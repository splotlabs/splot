# Design: bit-writer-primitives

## Context

`splot-core::bitio::BitReader` reads AV2 descriptors MSB-first and is panic-free,
returning typed `Error`s on malformed input. The writer must be its exact inverse:
for every value the reader could have produced, the writer emits bits the reader
reads back to the same value. The round-trip contract is the spec of correctness:

```text
read(write(x)) == x        (for every value the writer accepts)
```

## Crate placement

Per the mission, the writer starts in `crates/splot-core/src/write/`, beside the
model and parsers it inverts. `splot-core` already owns both, so no new crate and no
new dependency. Extraction to a `splot-bitstream` crate is explicitly deferred until
a documented trigger fires (extra deps, build-time regression, or another crate
needing the writer without the full `splot-core` surface).

Module layout:

- `write/bit_writer.rs` — `BitWriter` and all primitive inverses.
- `write/error.rs` — `WriteError` (self-contained) + `WriteResult`.
- `write/mod.rs` — re-exports `BitWriter`, `WriteError`, `WriteResult`.

## Decision: a self-contained `WriteError`, not the parser `Error`

The parser's `Error` models *bitstream conformance / EOF* failures (untrusted input).
Writer failures are *encoder-side* programming errors: the caller asked for an
encoding the descriptor cannot represent (a value too wide for a field, a width
outside the descriptor's domain). Mixing the two would overload the parser error
model and couple the additive writer to it. The writer therefore carries its own
`WriteError` and touches no parser/model/error code. Every variant maps to a
precondition of the matching reader descriptor, so the writer rejects exactly the
values the reader could never have produced.

## Buffer model (MSB-first)

`BitWriter` accumulates bits into an in-progress byte (`current`, `nbits`) and
flushes a completed byte to a `Vec<u8>` every eight bits. The first bit written
becomes the most-significant bit of the byte, matching `f(n)` and every reader
primitive. `current << 1` before the eighth bit never exceeds a `u8`, and
`align_to_byte` left-justifies a partial byte and zero-pads the low bits — the exact
inverse of `byte_align_zero`, which requires the padding bits to be zero.

## Inverse mappings (the non-trivial descriptors)

- **`su(n)`** — encode the `n`-bit two's-complement field: `coded = value & ((1<<n)-1)`,
  computed in `i64` so `n == 32` is panic-free. Reject values outside
  `-(2^(n-1)) ..= 2^(n-1) - 1`.
- **`uvlc()`** — the reader returns `suffix + (1<<lz) - 1`, so the magnitude is
  `m = value + 1`. Emit `lz = floor(log2 m)` zero bits, a `1`, then the `lz`-bit
  suffix `m - 2^lz`. AV2 requires `lz < 32`; `value == u32::MAX` (which needs
  `lz == 32`) is rejected. `m` is computed in `u64` to avoid the `+1` overflow.
- **`svlc()`** — inverse of `half = (v+1)>>1; svlc = (v&1)?half:-half`:
  `value>0 -> v = 2*value-1`, `value<0 -> v = -2*value`, `0 -> 0`; then `write_uvlc(v)`.
  Only `i32::MIN` (whose `v == 2^32`) is unencodable; rejected before the `u32` cast.
- **`ns(n)`** — recompute the reader's `w` and `m`. For `value < m`, write the
  `w-1`-bit short form. Otherwise invert `value = (v<<1) - m + extra`: `t = value + m`,
  `v = t >> 1` (`w-1` bits), `extra = t & 1`.
- **`rg(n)`** — `quotient = value >> n`, `remainder = value & ((1<<n)-1)` (special-cased
  for `n == 32`, where the shift is undefined and the quotient is always zero). Emit
  `quotient` one bits, a terminating zero, then the `n`-bit remainder. Reject
  `quotient >= 32` (the reader's unary prefix must terminate within 32 bits).
- **`leb128()`** — canonical minimal-length encoding (1–5 bytes for any `u32`); always
  succeeds.

## Round-trip guarantees: semantic vs. byte-exact

- **Semantic round-trip** (`read(write(x)) == x`) holds for every value the writer
  accepts — this is the property-tested contract.
- **Byte-exact round-trip** (`parse(bytes) -> write -> bytes` identical) holds only
  for *canonically* encoded inputs. The one primitive with alternate encodings is
  `leb128()`, which permits non-minimal byte sequences; the writer always emits the
  minimal form, so byte-exactness is guaranteed only when the source used the minimal
  form. This is documented on `write_leb128` and will be reflected per-structure in
  the writer coverage matrix.

## Testing strategy

- Property tests establish `read(write(x)) == x` for `f(n)`, `su`, `uvlc`, `svlc`,
  `le(n)->u64`, `leb128`, `ns`, and `rg` across their valid value spaces, including
  the boundary widths (`n == 32`, `n == 8`) and the `ns`/`rg` non-power-of-two paths.
- Unit tests cover every error variant (`BitWidthTooLarge`, `ByteWidthTooLarge`,
  `ZeroWidth`, `ValueTooWide`, `ValueOutOfRange`) and assert canonical byte output
  for known `leb128`/`rg`/alignment cases that mirror the existing reader tests.
- A "never panics" property test drives every primitive with arbitrary value/width,
  proving the writer always returns `Result` rather than panicking.
- The `cargo fuzz` primitive target is deferred to `roundtrip-and-fuzz-harness`; the
  proptest suite already covers the contract over the full value space.
