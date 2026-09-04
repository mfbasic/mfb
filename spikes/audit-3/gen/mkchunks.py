import struct, sys, zlib

def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

n = int(sys.argv[1])       # number of IDAT chunks
sz = int(sys.argv[2])      # bytes per IDAT chunk
out = sys.argv[3]
ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
body = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
payload = zlib.compress(b"\x00" * 8)
body += chunk(b"IDAT", payload)
for i in range(n):
    body += chunk(b"IDAT", b"\xAB" * sz)
body += chunk(b"IEND", b"")
open(out, "wb").write(body)
print("wrote", out, len(body), "bytes;", n, "IDAT chunks of", sz)
