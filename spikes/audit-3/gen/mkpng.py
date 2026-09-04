import struct, sys, zlib

def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

w = int(sys.argv[1]); h = int(sys.argv[2])
out = sys.argv[3] if len(sys.argv) > 3 else "/tmp/dec/in.png"
crc_bad = len(sys.argv) > 4 and sys.argv[4] == "badcrc"

ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
idat = zlib.compress(b"\x00" * 64)
data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")
if crc_bad:
    data = bytearray(data)
    data[29] ^= 0xFF   # corrupt IHDR CRC
    data = bytes(data)
open(out, "wb").write(data)
print("wrote", out, len(data), "bytes, declares", w, "x", h)
