## Context

`RECON-WIENERNS-FILTER-PRIMITIVE` covers only the luma branch of AV2 §7.20.3.
The same section defines a chroma branch that first applies the UV tap table to
chroma source samples, then adds a luma-derived contribution using
`get_luma_sample`. The repository already has §7.20.2 source-sample selection
and frame-view reads, so this brick can add the scheduler-free chroma arithmetic
without trying to wire runtime loop restoration.

## Goals / Non-Goals

**Goals:**

- Add a panic-free `splot-recon` primitive for AV2 §7.20.3 chroma Wiener NS
  filtering over caller-resolved source callbacks.
- Support the `Wiener_Ns_Config_Uv` chroma tap loop and the luma-tap
  contribution with `Wiener_Filters_420` downsampling for 4:2:0 sources.
- Keep invalid inputs fail-atomic: validation and source-sample range failures
  must not partially mutate caller output.
- Keep the luma primitive behavior and tests unchanged.

**Non-Goals:**

- Full §7.20 loop-restoration traversal, restoration-unit scheduling,
  coefficient selection from frame/unit banks, §7.20.2 frame reads, PC-Wiener
  classification, GDF/BRU, runtime decode wiring, output serialization, or
  successful local decoder mission decode.

## Decisions

- **Separate chroma parameter type.** Add a `WienerNsChromaFilter` with chroma
  block dimensions, output stride, bit depth, chroma coefficients, luma
  coefficients, luma bounds, subsampling, and `cfl_ds_filter_index`. This keeps
  the luma API stable and makes the extra chroma dependencies explicit.
- **Caller-resolved source callbacks.** The primitive receives one callback for
  chroma source samples and one for luma source samples. The caller owns
  frame-coordinate offsets and §7.20.2 source-frame selection; this function owns
  only the §7.20.3 chroma/luma tap arithmetic and luma downsampling.
- **Transcribe only §7.20.3 tables.** `Wiener_Ns_Config_Uv` and
  `Wiener_Filters_420` live next to the existing luma table. Tests pin table
  shape and distinctive rows; no AVM code, AV1 constants, or third-party source
  is copied.
- **Temporary output for fail-atomicity.** Match the luma primitive: validate
  parameters and compute into a temporary vector before copying into the caller's
  strided output.

## Risks / Trade-offs

- **Risk:** The primitive can be called with a source callback that does not
  implement §7.20.2 correctly. **Mitigation:** docs, support rows, and tests keep
  source-frame selection outside this primitive; runtime wiring must prove that
  separately.
- **Risk:** Chroma's luma contribution mixes coordinate spaces. **Mitigation:**
  expose luma bounds/subsampling as explicit parameters and add tests for
  4:2:0 averaging, 4:2:0 vertical filter index 1, non-subsampled direct luma
  reads, and luma clipping.
- **Risk:** This does not advance the live local decoder mission diagnostic by itself.
  **Mitigation:** track it as additive reconstruction infrastructure only; the
  local decoder mission runtime remains fail-closed until frame reconstruction and source reads
  are honestly wired.
