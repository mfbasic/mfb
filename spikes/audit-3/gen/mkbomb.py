import struct, sys, zlib

def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

n = int(sys.argv[1])          # inflated byte count
out = sys.argv[2]
ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
idat = zlib.compress(b"\x00" * n, 9)
data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")
open(out, "wb").write(data)
print("wrote", out, len(data), "file bytes ->", n, "inflated bytes; ratio", n / len(data))
