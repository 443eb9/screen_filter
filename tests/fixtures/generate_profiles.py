"""Create deterministic ICC v4 matrix/TRC and CLUT profiles for color tests.

ICC binary layout: https://www.color.org/specification/ICC.1-2022-05.pdf
Regenerate with: python tests/fixtures/generate_profiles.py
"""

import struct
from itertools import product
from pathlib import Path


def fixed(value):
    return struct.pack(">i", round(value * 65536))


def xyz(values):
    return b"XYZ \0\0\0\0" + b"".join(map(fixed, values))


def text(value):
    encoded = value.encode("utf-16-be")
    return b"mluc\0\0\0\0" + struct.pack(">II2s2sII", 1, 12, b"en", b"US", len(encoded), 28) + encoded


def curve(gamma=None):
    if gamma is not None:
        return b"para\0\0\0\0" + struct.pack(">HH", 0, 0) + fixed(gamma)
    parameters = (2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045, 0, 0)
    return b"para\0\0\0\0" + struct.pack(">HH", 4, 0) + b"".join(map(fixed, parameters))


def lut(matrix, grid, inverse):
    header = b"mft2\0\0\0\0" + bytes((3, 3, grid, 0))
    header += b"".join(map(fixed, (1, 0, 0, 0, 1, 0, 0, 0, 1)))
    header += struct.pack(">HH", 256, 256)
    input_curve = [i / 255 if inverse else (i / 255) ** 2.2 for i in range(256)]
    output_curve = [(i / 255) ** (1 / 2.2) if inverse else i / 255 for i in range(256)]
    def tables(values):
        return b"".join(struct.pack(">H", round(value * 65535)) for value in values) * 3

    data = bytearray()
    for point in product(range(grid), repeat=3):
        rgb = [value / (grid - 1) for value in point]
        scale = 65535 / 32768 if inverse else 32768 / 65535
        for row in matrix:
            value = sum(a * b for a, b in zip(row, rgb, strict=True)) * scale
            data.extend(struct.pack(">H", round(max(0, min(1, value)) * 65535)))
    return header + tables(input_curve) + data + tables(output_curve)


def profile(name, columns, gammas, use_lut=False):
    adaptation = (1.0478112, 0.0228866, -0.0501270,
                  0.0295424, 0.9904844, -0.0170491,
                  -0.0092345, 0.0150436, 0.7521316)
    tags = [
        (b"desc", text(name)),
        (b"cprt", text("Screen Filter test data, CC0-1.0")),
        (b"wtpt", xyz((0.9642, 1, 0.8249))),
        (b"chad", b"sf32\0\0\0\0" + b"".join(map(fixed, adaptation))),
    ]
    if use_lut:
        forward = lut(tuple(zip(*columns, strict=True)), 2, False)
        inverse = lut(((3.1338561, -1.6168667, -0.4906146),
                       (-0.9787684, 1.9161415, 0.0334540),
                       (0.0719453, -0.2289914, 1.4052427)), 17, True)
        tags.extend(((b"A2B0", forward), (b"A2B1", forward),
                     (b"B2A0", inverse), (b"B2A1", inverse)))
    else:
        for channel, column, gamma in zip((b"r", b"g", b"b"), columns, gammas, strict=True):
            tags.extend(((channel + b"XYZ", xyz(column)), (channel + b"TRC", curve(gamma))))

    offset = 128 + 4 + len(tags) * 12
    table = bytearray(struct.pack(">I", len(tags)))
    data = bytearray()
    for signature, content in tags:
        table.extend(signature + struct.pack(">II", offset + len(data), len(content)))
        data.extend(content)
        data.extend(b"\0" * (-len(data) % 4))

    header = bytearray(128)
    struct.pack_into(">I", header, 0, 128 + len(table) + len(data))
    struct.pack_into(">I", header, 8, 0x04300000)
    header[12:24] = b"mntrRGB XYZ "
    struct.pack_into(">6H", header, 24, 2024, 1, 1, 0, 0, 0)
    header[36:44] = b"acspMSFT"
    struct.pack_into(">I", header, 64, 1)
    header[68:80] = b"".join(map(fixed, (0.9642, 1, 0.8249)))
    return header + table + data


if __name__ == "__main__":
    directory = Path(__file__).resolve().parent
    srgb = ((0.4360747, 0.2225045, 0.0139322),
            (0.3850649, 0.7168786, 0.0971045),
            (0.1430804, 0.0606169, 0.7141733))
    display_p3 = ((0.515102, 0.241182, -0.00104941),
                  (0.291965, 0.692236, 0.0418818),
                  (0.157153, 0.0665819, 0.784378))
    for filename, name, columns, gammas in (
        ("srgb.icc", "sRGB test profile", srgb, (None, None, None)),
        ("display_p3.icc", "Display P3 test profile", display_p3, (None, None, None)),
        ("channel_gamma.icc", "Channel gamma test profile", srgb, (1.8, 2.2, 2.6)),
    ):
        (directory / filename).write_bytes(profile(name, columns, gammas))
    (directory / "lut.icc").write_bytes(profile("CLUT gamma 2.2 test profile", srgb, (), use_lut=True))
