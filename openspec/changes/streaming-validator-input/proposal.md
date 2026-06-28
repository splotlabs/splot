# Change: streaming-validator-input

## Feature IDs

- `INFRA-STREAMING-TU-READER`
- `INFRA-VALIDATE-STREAMING-READER`

## Why

`read_input()` (`crates/splot-cli/src/commands/mod.rs:42`) loads each whole input
file into a `Vec<u8>` via `fs::read` for the `validate` and `inspect` commands.
Peak memory therefore scales with file size, which becomes a hotspot for large
IVF/Annex-B streams, corpus sweeps, and CI validation of many files. `decode`
already bounds its input via `DecodeOptions::limits().max_input_bytes()` and a
`take`-limited reader, but `validate`/`inspect` have **no guard at all** and can
exhaust memory on a large or adversarial file. That asymmetry — a bounded
`decode` next to an unbounded `validate` — is the concrete present-tense gap.

The validator is already shaped for incremental processing:

- `ValidatorContext` (`crates/splot-validate/src/context/mod.rs:139`) is an owned,
  lifetime-free state machine driven one OBU at a time via `observe_obu`, with an
  end-of-stream `finish()` (`context/mod.rs:726`) that already isolates every
  whole-stream / EOF check.
- `Diagnostic` / `ValidationReport` (`crates/splot-validate/src/diagnostic.rs:34`)
  are **fully owned** (`String` + numeric offsets, no borrows), so findings
  outlive the bytes that produced them.
- `splot-core` already exposes per-unit cursors: `AnnexBObuCursor::next_obu`
  (`crates/splot-core/src/annexb.rs:104`) and `IvfFrameCursor::next_frame_record`
  (`crates/splot-core/src/ivf.rs:174`).

Only the eager whole-buffer collection in `parse_bitstream_partial`
(`crates/splot-core/src/stream.rs:71`) stands between the current design and a
streaming one. This change adds a forward-only, `Read`-based streaming input path
so peak input memory is bounded to the largest temporal unit rather than the whole
file.

The design mirrors the universal three-tier shape used by the AV2 reference
tools and production media frameworks (engineering precedent only — AV2 behavior
still comes from the spec and AVM):

- **AVM / avmdec** — `obudec_read_temporal_unit` / `obudec_read_frame_unit` read
  one decode unit at a time into a single reused, grown-on-demand buffer; peak
  input ≈ largest unit, never the whole file.
- **dav2d** (the AV2 fork of dav1d) — `Dav2dData` is exactly one temporal unit;
  the Annex-B and Section-5 demuxers set `.seek = NULL` and stream pure-forward.
- **FFmpeg** — `AVIOContext.read_packet` pull callback → `AVPacket` (one access
  unit); `seekable == 0` is fully supported.

The common contract: a pull byte source → a demuxer that frames one temporal unit
at a time → a consumer that accumulates state and finalizes at EOF, with seek
strictly optional. splot's parser is already forward-only, so the byte source is
plain `std::io::Read` (no `Seek`), which is strictly simpler than AVM's local-seek
approach and gains free stdin/pipe support.

## Scope

- Spec sections: none newly normative. Reuses existing container framing —
  leb128 (`AV2-4.11.6-LEB128`), the Annex-B OBU envelope
  (`AV2-B-ANNEXB-OBU-ENVELOPE`), and IVF (`AV2-IVF-CONTAINER`).
- Crates/modules:
  - `splot-core`: new `TemporalUnitReader<R: Read>` — a forward-only container
    demuxer over `Read` that yields one temporal unit at a time into a reused
    buffer, reusing `AnnexBObuCursor` / `IvfFrameCursor`; enforces a per-unit
    byte cap.
  - `splot-validate`: new `StreamingValidator` wrapping `ValidatorContext`
    (`push_unit` / `finish`); new `validate_reader<R: Read>`; `validate_bytes`
    re-expressed over the same per-OBU engine.
  - `splot-cli`: `validate` reads via `validate_reader` from a `File` (and stdin),
    bounding peak memory; extends the existing `CLI-VALIDATE` surface.
- CLI/docs/tests: golden-equivalence tests (`validate_reader` ≡ `validate_bytes`
  on all fixtures), chunked-`Read` reassembly tests, per-unit size-cap tests,
  updated `validate --help` snapshot if stdin support changes it.

## Non-goals

- No streaming refactor of `inspect`. It is a separate, more-entangled consumer
  (`collect_obus` and `--json` pretty-printing assume the full `Vec`). Deferred to
  a follow-on change.
- No `memmap2` / memory-mapped input. That would add a third-party dependency and
  a crate-graph change (both §10 human sign-off triggers), carries a
  SIGBUS-on-truncation caveat, and is not actually streaming.
- No `Seek` requirement. The reader is strictly forward-only.
- No generic pluggable `Demuxer` trait or `Packet` type. YAGNI for two container
  formats and a forward-only parser; revisit only under the rule of three (a third
  format, or a unified decode/inspect path).
- No change to which diagnostics are computed, to severities, or to exit codes.
  Output stays accumulate-then-return; streaming *diagnostic output* is out of
  scope for v1.
- No changes to the `decode` path (already bounded independently).

## Acceptance criteria

- [ ] Matrix rows `INFRA-STREAMING-TU-READER` and `INFRA-VALIDATE-STREAMING-READER`
      exist in `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [ ] Public API documented: `TemporalUnitReader<R: Read>`, `StreamingValidator`,
      `validate_reader<R: Read>`.
- [ ] Streaming validation implemented; `validate_bytes(&[u8])` preserved as the
      stable in-memory API.
- [ ] `validate_reader` produces a `ValidationReport` byte-identical to
      `validate_bytes` on **every** existing fixture (diagnostic set, order, and
      offsets), proven by test.
- [ ] Positive tests: IVF and Annex-B streams validated end to end via a reader.
- [ ] Negative/EOF/malformed tests: truncated unit mid-stream; a `Read` that
      returns one byte per call (cross-boundary reassembly); a declared unit size
      over the cap.
- [ ] Peak input memory is bounded to one temporal unit (no whole-file
      allocation), demonstrated by test.
- [ ] No reachable `unwrap`/`expect`/`panic!`; no `unsafe`.
- [ ] `cargo xtask check-feature-status` and `cargo xtask ci` pass.
