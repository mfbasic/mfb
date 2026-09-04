import struct, sys

src, dst = sys.argv[1], sys.argv[2]
groups = int(sys.argv[3], 0)
b = bytearray(open(src, "rb").read())
n = struct.unpack(">H", b[4:6])[0]
cmap = None
for i in range(n):
    rec = 12 + i * 16
    if bytes(b[rec:rec + 4]) == b"cmap":
        cmap = struct.unpack(">I", b[rec + 8:rec + 12])[0]

end = len(b)
blob = struct.pack(">HHIII", 12, 0, 16, 0, groups)   # format 12, numGroups
b += blob
# repoint cmap subtable record 0 at the new blob
struct.pack_into(">I", b, cmap + 4 + 0 * 8 + 4, end - cmap)
open(dst, "wb").write(bytes(b))
print("cmap", cmap, "blob at file offset", end, "subtable offset", end - cmap,
      "numGroups", hex(groups), "->", dst)
