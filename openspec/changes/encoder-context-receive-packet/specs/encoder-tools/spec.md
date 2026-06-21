## ADDED Requirements

### Requirement: Public Context produces a real packet from input (Milestone A keystone)

The public `splot-encode` `Context::receive_packet` SHALL return a real, decodable AV2 `Packet`
for a sent input frame that is the supported subset — a 64x64 8-bit YUV420 frame whose every
visible Y/U/V sample equals the 128 AV2 § 7.13.2 no-neighbour DC predictor — as one coded access unit (the AV2 Annex B temporal unit of the
minimal skip frame, not an IVF container file), tracked by `ENC-CONTEXT-RECEIVE-PACKET`. Because such a frame has zero
residual, the skip frame's reconstruction SHALL equal the input (lossless). For any other frame
the encoder SHALL NOT emit a packet (no canned output). This is the first real public-API packet
production; forward quantization of arbitrary input is future work.

#### Scenario: An all-128 frame encodes to a packet that decodes back to the input

- **WHEN** an all-128 64x64 YUV420 frame is sent to a `Context` and `receive_packet` is called
  after `flush`
- **THEN** `receive_packet` SHALL return `Packet` carrying non-empty coded access-unit bytes
- **AND** muxing the packet into an IVF and decoding it with `splot-decode` SHALL reconstruct a
  6144-byte frame whose every sample equals the all-128 input (`decode(encode(input)) == input`)
- **AND** the next `receive_packet` SHALL return `Finished`.

#### Scenario: A non-flat frame is retired without a canned packet

- **WHEN** a 64x64 frame with any non-128 visible sample is sent and `receive_packet` is called
  after `flush`
- **THEN** `receive_packet` SHALL NOT return a `Packet` (it returns `Finished`/`NeedMoreData`).
