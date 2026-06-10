## Context

`splot-core` currently parses raw AV2 Annex B streams: `leb128()` OBU lengths
followed by `open_bitstream_unit(...)` bytes. `splot validate` and `splot inspect`
therefore fail on common `.ivf` files before reaching the AV2 payload.

The crates.io `ivf` crate was evaluated as a possible dependency. It is
BSD-2-Clause, but its public writer helpers panic on I/O errors, it writes an AV1
fourcc by default, it does not provide `splot`'s byte-offset model, and adding it
would also add `bitstream-io`. IVF is small enough that a local, typed,
panic-free implementation is more maintainable than a dependency.

## Goals / Non-Goals

**Goals:**

- Parse IVF headers and frame records in `splot-core` without panics.
- Auto-detect raw Annex B vs IVF at validator/inspector entry points.
- Preserve byte offsets against the original input so diagnostics remain useful.
- Provide writer helpers for future encoder/decoder paths.
- Report malformed IVF as structured `ivf/*` validator diagnostics.

**Non-Goals:**

- Do not implement a decoder or reconstruct IVF frames.
- Do not parse or validate AV2 syntax beyond the existing Annex B payload path.
- Do not add the external `ivf` crate or copy its source/prose.
- Do not reject IVF solely because the fourcc is unfamiliar; the AV2 payload parser
  remains the authority for AV2 bitstream syntax.

## Decisions

1. **Implement IVF locally in `splot-core`.**
   - Rationale: the required container surface is small, must be panic-free, and
     must preserve offsets. Avoiding a dependency also avoids the crate's AV1
     default writer behavior and `bitstream-io` dependency.
   - Alternative considered: add `ivf = "0.1.4"`. Legal under BSD-2-Clause, but
     not a good API fit for `splot`'s validator-first model.

2. **Model IVF as a container around Annex B frame payloads.**
   - Each frame payload is parsed by `parse_annex_b_obus_partial_at(...)`, a new
     offset-aware variant of the existing parser. This avoids rebasing diagnostics
     after parsing.
   - Raw Annex B remains unchanged and continues to use the same parser.

3. **Auto-detect by magic bytes at library entry points.**
   - `DKIF` selects IVF; everything else is treated as raw Annex B. This preserves
     current CLI behavior for raw streams and needs no new command-line flag.
   - IVF records include header metadata and frames for JSON inspection, while
     human inspect output still leads with the OBU list.

4. **Use diagnostics, not CLI errors, for malformed IVF.**
   - The validator converts `IvfError` values to stable `ivf/*` diagnostics. The
     inspector returns exit code 1 after printing any parseable prefix, matching
     the existing malformed-tail behavior.

## Risks / Trade-offs

- **Unknown AV2 fourcc conventions** -> The parser records any fourcc and does not
  reject solely on that field. A future AVM/conformance decision can tighten this
  with a separate diagnostic if needed.
- **Multiple IVF frames with Annex B payloads** -> The initial demuxer concatenates
  parsed OBUs in frame order while retaining original offsets. This is sufficient
  for validator state and inspector output.
- **Container frame count mismatches** -> Header `frame_count` is treated as
  informational for now; truncated records and trailing malformed frame payloads are
  structural diagnostics. A stricter count check can be added once corpus behavior
  is known.
