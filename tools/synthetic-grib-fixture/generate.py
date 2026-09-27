#!/usr/bin/env python3
"""Generate a minimal, spec-legal GRIB2 fixture for bug-711's regression test.

Builds four GRIB2 messages, each a 1x1 grid carrying one of the four fields
examples/wind/src/fetch.mfb:windsFrom requires (UGRD 10 m, VGRD 10 m, TMP
2 m, PRATE surface), using data representation template 5.0 at bit width 0
(a constant field, so section 7 needs no packed data). Decodes correctly
with the real, unmodified examples/wind/src/grib.mfb. See README.md for why
this replaced a captured real (926 KB) GFS analysis file.

Usage: generate.py [output-path]  (default: /tmp/synthetic-valid-analysis.grb2)
"""

import struct

def be(n, nbytes):
    return n.to_bytes(nbytes, 'big', signed=False)

def sign_mag(value, nbytes):
    # value may be negative; encode as sign-bit magnitude in nbytes*8 bits
    nbits = nbytes * 8
    mag = abs(value)
    if value < 0:
        return (mag | (1 << (nbits - 1))).to_bytes(nbytes, 'big')
    return mag.to_bytes(nbytes, 'big')

def section(number, body):
    # body excludes the 4-byte length + 1-byte number header
    total_len = 4 + 1 + len(body)
    return be(total_len, 4) + bytes([number]) + body

def section3_grid():
    # Grid template 3.0, regular lat/lon, 1x1 grid, scanning mode 0.
    # gridOf reads: template@12(2), ni@30(4), nj@34(4), lat1@46(4), lon1@50(4),
    # dlon@63(4), dlat@67(4), scanning@71(1). All offsets within the FULL
    # section byte list (header included), 0-based.
    body = bytearray(72 - 5)  # section body = total(72) - header(5)
    # Build the whole 72-byte section directly for clarity, then strip header.
    full = bytearray(72)
    full[0:4] = be(72, 4)
    full[4] = 3
    full[12:14] = be(0, 2)          # grid template 3.0
    full[30:34] = be(1, 4)          # ni = 1
    full[34:38] = be(1, 4)          # nj = 1
    full[46:50] = struct.pack('>i', int(0.0 * 1_000_000))   # lat1 = 0
    full[50:54] = struct.pack('>i', int(0.0 * 1_000_000))   # lon1 = 0
    full[63:67] = struct.pack('>i', int(1.0 * 1_000_000))   # dlon
    full[67:71] = struct.pack('>i', int(1.0 * 1_000_000))   # dlat
    full[71] = 0                     # scanning mode 0
    return bytes(full)

def section4(category, parameter, surface, height):
    # describedBy reads: template@7(2, unused value here - 0 = instant),
    # category@9(1), parameter@10(1), surface@22(1), height@24(4). Needs
    # length >= 28.
    full = bytearray(28)
    full[0:4] = be(28, 4)
    full[4] = 4
    full[7:9] = be(0, 2)             # product template 4.0 (instant)
    full[9] = category
    full[10] = parameter
    full[22] = surface
    full[24:28] = be(height, 4)
    return bytes(full)

def ieee32_bits(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]

def section5_simple(count, reference_value):
    # scalingOf reads reference@11(4, IEEE32), binaryScale@15(2, signed),
    # decimalScale@17(2, signed); unpackSimple reads width@19(1) and count
    # from beInt(s5,5,4) with template@9(2). Width 0 -> every value equals
    # the reference exactly (readBits(...,0) always returns 0).
    full = bytearray(20)
    full[0:4] = be(20, 4)
    full[4] = 5
    full[5:9] = be(count, 4)
    full[9:11] = be(0, 2)             # data representation template 5.0
    full[11:15] = be(ieee32_bits(reference_value), 4)
    full[15:17] = sign_mag(0, 2)      # binary scale factor = 0
    full[17:19] = sign_mag(0, 2)      # decimal scale factor = 0
    full[19] = 0                      # bit width = 0 (constant field)
    return bytes(full)

def section6_bitmap():
    # indicator@5 = 255 means every point present.
    full = bytearray(6)
    full[0:4] = be(6, 4)
    full[4] = 6
    full[5] = 255
    return bytes(full)

def section7_empty():
    # Header only: width 0 means unpackSimple's bit reader never advances,
    # so no packed data bytes are needed at all.
    full = bytearray(5)
    full[0:4] = be(5, 4)
    full[4] = 7
    return bytes(full)

def section1_minimal():
    # decodeMessage's MATCH does nothing with section 1 (CASE ELSE) - any
    # correctly-length-prefixed section is enough for the byte walk to
    # step over it. Minimum legal section length is 5 (the length+number
    # header alone, sectionLength < 5 is rejected).
    return be(5, 4) + bytes([1])

def grib_message(category, parameter, surface, height, value):
    s1 = section1_minimal()
    s3 = section3_grid()
    s4 = section4(category, parameter, surface, height)
    s5 = section5_simple(1, value)
    s6 = section6_bitmap()
    s7 = section7_empty()
    body = s1 + s3 + s4 + s5 + s6 + s7
    total_length = 16 + len(body) + 4  # section0 header + body + "7777"
    section0 = b'GRIB' + bytes([0, 0]) + bytes([0]) + bytes([2]) + be(total_length, 8)
    return section0 + body + b'7777'

def build_fixture():
    # The four fields wind.mfb:windsFrom's pick() calls require:
    #   east         cat 2 param 2 surface 103 height 10   (UGRD 10 m)
    #   north        cat 2 param 3 surface 103 height 10   (VGRD 10 m)
    #   temperature  cat 0 param 0 surface 103 height 2     (TMP 2 m)
    #   precipitation cat 1 param 7 surface 1 height 0      (PRATE surface)
    msgs = [
        grib_message(2, 2, 103, 10, 3.5),
        grib_message(2, 3, 103, 10, -1.25),
        grib_message(0, 0, 103, 2, 288.15),
        grib_message(1, 7, 1, 0, 0.0002),
    ]
    return b''.join(msgs)

if __name__ == '__main__':
    import sys
    out = build_fixture()
    path = sys.argv[1] if len(sys.argv) > 1 else '/tmp/synthetic-valid-analysis.grb2'
    with open(path, 'wb') as f:
        f.write(out)
    print(f"wrote {len(out)} bytes to {path}")
