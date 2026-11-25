# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
"""Multiplex atlas metadata into the two committed MSDO seeds.

The 8-bit input has SHA-256 3cab7d7226c08e0e9997fb99e2810cdeb5bf206496fcfc020b5f542559e638e1.
The 10-bit input has SHA-256 ad4458d25b0114f7ce114c2277717e5f813701cac7f014bd025584cf0e08d9cd.
The fixture manifest records the AVM commands and native output hashes.
"""
from pathlib import Path


def fixed(value, width):
    assert 0 <= value < (1 << width)
    return f"{value:0{width}b}"


def uvlc(value):
    code = f"{value + 1:b}"
    return "0" * (len(code) - 1) + code


def aligned(bits):
    return bits + "0" * (-len(bits) % 8)


def hls(obu_type, xlayer, bits):
    # Both the LCR and atlas OBUs end with a zero extension flag and trailing bits.
    bits = aligned(bits + "01")
    return bytes((128 | (obu_type << 2), xlayer)) + int(bits, 2).to_bytes(len(bits) // 8, "big")


def atlas(mode, xlayers, scope=31, labels=(41, 77), rectangles=None):
    bits = fixed(5, 3) + uvlc(mode)
    if mode == 0:
        # Two uniformly sized regions, one region per segment.
        bits += uvlc(1) + uvlc(0) + "1" + uvlc(63) + uvlc(63) + "1"
    elif mode == 2:
        bits += uvlc(63) + uvlc(63)
    else:
        if mode == 1:
            bits += "1"  # Explicit stream IDs.
        bits += uvlc(144) + uvlc(64) + uvlc(1)
        if mode == 4:
            bits += "1"  # The first segment is the alpha plane of the second.
        if mode in (3, 4):
            bits += "0"  # Background values are absent.
        if rectangles is None:
            rectangles = ((0, 0, 64, 64), (0 if mode == 4 else 64, 0, 80, 64))
        for index, (xlayer, rectangle) in enumerate(zip(xlayers, rectangles, strict=True)):
            bits += fixed(xlayer, 5) + "".join(uvlc(value) for value in rectangle)
            if mode == 4 and index == 0:
                bits += "1"
    bits += "1" + "".join(fixed(label, 8) for label in labels)
    return hls(17, scope, bits)


def global_lcr(xlayers, second_segment=77):
    bits = fixed(1, 3) + fixed(sum(1 << x for x in xlayers), 31)
    bits += "00101" + fixed(0, 7) + "10" + fixed(5, 3) + fixed(0, 5)
    for payload in (embedded_info(), fixed(0, 8) + fixed(second_segment, 8) + fixed(0, 16)):
        bits += fixed(len(payload) // 8, 8) + payload
    return hls(16, 31, bits)


def embedded_info():
    bits = aligned("0001") + fixed(1, 8) + fixed(1, 4)  # ML0, TL0
    return aligned(bits + fixed(41, 8) + fixed(0, 32) + "1")


def local_lcr(xlayer):
    bits = fixed(1, 3) + fixed(1, 3) + "01" + fixed(5, 3) + fixed(0, 5)
    return hls(16, xlayer, bits + embedded_info())


def obus(data):
    pos = 0
    while pos < len(data):
        size = 0
        shift = 0
        while True:
            b = data[pos]
            pos += 1
            size |= (b & 127) << shift
            if b < 128:
                break
            shift += 7
        assert size and pos + size <= len(data)
        yield data[pos:pos + size]
        pos += size


def frame(obu):
    size = len(obu)
    prefix = bytearray()
    while size >= 128:
        prefix.append(128 | (size & 127))
        size >>= 7
    prefix.append(size)
    return prefix + obu


def with_lcr(seq, depth):
    # Both seeds use seq_header_id=0 and level 0. The 8-bit seed uses bit_depth_idc=1.
    # Replace only the three seq_lcr_id bits in the known seed header (§ 5.4.1).
    bit_offset = 16 if depth == 8 else 14
    payload = "".join(fixed(b, 8) for b in seq[2:])
    assert int(payload[bit_offset:bit_offset + 3], 2) == (0 if depth == 8 else 1)
    payload = payload[:bit_offset] + fixed(1, 3) + payload[bit_offset + 3:]
    return seq[:2] + int(payload, 2).to_bytes(len(seq) - 2, "big")


def multiplex(seed, depth, local):
    xlayers = (3, 7) if depth == 8 else (2, 9)
    out = bytearray()
    tu = -1
    # Repeat the 10-bit seed to supply a second closed-loop key.
    # The 8-bit seed already contains a second key in its third temporal unit.
    second_key = 2 if depth == 8 else 5
    if depth == 10:
        seed += seed
    first_layer_obu = True
    for obu in obus(seed):
        kind = (obu[0] >> 2) & 31
        if kind == 2:
            tu += 1
            first_layer_obu = True
        if kind == 16:  # Replace the global LCR in the 10-bit seed.
            continue
        if kind == 1:
            obu = with_lcr(obu, depth)
        if kind not in (2, 20) and first_layer_obu:
            if tu in (0, second_key):
                out += frame(global_lcr(xlayers))
                out += frame(atlas(3 if tu == 0 else 4, xlayers))
            if local and tu in (1, second_key):
                out += frame(local_lcr(xlayers[0]))
            if local and tu in (0, second_key):
                out += frame(atlas(2, xlayers, xlayers[0], (41,)))
            first_layer_obu = False
        out += frame(obu)
    return out


def generate(valid, stage):
    valid, stage = Path(valid), Path(stage)
    external = stage / "needs-external-hls"
    clean = stage / "valid"
    clean.mkdir(parents=True, exist_ok=True)
    external.mkdir(parents=True, exist_ok=True)
    for depth in (8, 10):
        seed = (valid / f"syn-msdo-two-streams-{depth}bit.obu").read_bytes()
        (clean / f"syn-atlas-multistream-{depth}bit.obu").write_bytes(multiplex(seed, depth, False))
        xlayers = (3, 7) if depth == 8 else (2, 9)
        (external / f"syn-atlas-local-{depth}bit.annexb").write_bytes(
            frame(atlas(2, xlayers, xlayers[0], (41,)))
        )
        (external / f"syn-atlas-global-local-id-scope-{depth}bit.obu").write_bytes(
            multiplex(seed, depth, True)
        )
    # Each one-TU case carries the MSDO, a global atlas and two decoded pictures.
    seed = (valid / "syn-msdo-two-streams-8bit.obu").read_bytes()
    first_tu = []
    for obu in obus(seed):
        if first_tu and obu == b"\x08":
            break
        if (obu[0] >> 2) & 31 == 1:
            obu = with_lcr(obu, 8)
        first_tu.append(frame(obu))
    for mode in range(5):
        labels = (41,) if mode == 2 else (41, 77)
        layout = frame(atlas(mode, (3, 7), labels=labels))
        record = frame(global_lcr((3, 7), labels[-1]))
        (clean / f"syn-atlas-mode-{mode}.obu").write_bytes(
            b"".join(first_tu[:2] + [record, layout] + first_tu[2:])
        )
