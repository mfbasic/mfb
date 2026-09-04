import struct, sys, shutil

src = sys.argv[1]
dst = sys.argv[2]
upem = int(sys.argv[3])

b = bytearray(open(src, "rb").read())
n = struct.unpack(">H", b[4:6])[0]
head = None
for i in range(n):
    rec = 12 + i * 16
    tag = bytes(b[rec:rec + 4])
    off = struct.unpack(">I", b[rec + 8:rec + 12])[0]
    if tag == b"head":
        head = off
print("numTables", n, "head at", head)
old = struct.unpack(">H", b[head + 18:head + 20])[0]
struct.pack_into(">H", b, head + 18, upem)
open(dst, "wb").write(bytes(b))
print("unitsPerEm", old, "->", upem, "written", dst, len(b), "bytes")
