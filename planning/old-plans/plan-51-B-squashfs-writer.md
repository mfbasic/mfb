# plan-51-B: SquashFS 4.0 writer

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: nothing

An AppImage is `[runtime ELF][squashfs image]` concatenated. plan-51-C needs the
second half, and nothing in the tree can produce it. This sub-plan adds
`src/os/linux/squashfs.rs`: a SquashFS 4.0 writer that serializes an in-memory tree
of directories, regular files, and symlinks to bytes.

The writer emits a **fully uncompressed** image and contains **zero compressor
code**. This is not a shortcut — it is a supported configuration that every SquashFS
reader handles, because decompression is decided per block by a flag bit in each
block's own header, and an uncompressed block never reaches a decompressor. The one
catch, verified empirically against `unsquashfs` and read out of both the kernel and
squashfuse sources, is that the superblock must still **name** a compressor that the
reader has compiled in, even though it is never invoked (§4.2).

The behavioral outcome: `squashfs::write(&tree)` returns bytes that `unsquashfs`
extracts to exactly `tree`, that the Linux kernel loop-mounts, and that the
AppImage runtime's bundled squashfuse mounts via FUSE.

This sub-plan adds a primitive with **no callers**. It lands independently and
changes no artifact.

References (read first):

- https://dr-emann.github.io/squashfs/squashfs.html — the format reference. Accurate,
  but the headers below are the ground truth where they disagree.
- https://raw.githubusercontent.com/torvalds/linux/master/fs/squashfs/squashfs_fs.h —
  struct definitions and the flag bit positions.
- https://raw.githubusercontent.com/torvalds/linux/master/fs/squashfs/super.c —
  every mount-time invariant in §4.9.
- https://raw.githubusercontent.com/torvalds/linux/master/fs/squashfs/dir.c — the
  three off-by-one biases and the `+3` rule in §4.6.
- https://raw.githubusercontent.com/torvalds/linux/master/fs/squashfs/id.c — why the
  ID table is mandatory (§4.7).
- https://raw.githubusercontent.com/vasi/squashfuse/master/fs.c — the AppImage
  runtime's reader; `sqfs_decompressor_get` is why §4.2 exists.
- https://raw.githubusercontent.com/plougher/squashfs-tools/master/squashfs-tools/squashfs_fs.h —
  `SQUASHFS_MKFLAGS`, which pins the bit positions the kernel header omits.
- `src/os/linux/link/elf.rs` — the house precedent for a from-scratch binary format
  writer: little-endian field packing, no third-party encoder.

## 1. Goal

- `squashfs::write(&SquashTree) -> Result<Vec<u8>, String>` produces a valid
  SquashFS 4.0 image containing directories, regular files, and symlinks with
  per-entry permissions.
- The image mounts under the kernel's squashfs driver and under squashfuse, and
  `unsquashfs` extracts it byte-for-byte.
- No compression code, no third-party crate, no external tool. The writer is pure
  byte layout over a `&SquashTree` and touches no filesystem.
- Every format invariant in §4.9 that a reader enforces is asserted by the writer
  rather than discovered at mount time.

### Non-goals (explicit constraints)

- **No compression.** Not gzip, not zstd. §3.1 records why, and §4.2 records the
  one place the decision is visible in the output.
- **No reader.** Tests verify against real `unsquashfs`/`mksquashfs` where
  available and against fixed golden bytes otherwise. Writing a parser to test the
  writer proves only that they agree with each other.
- **No fragments.** Tail blocks are written as short data blocks. Fragments exist to
  pack sub-block tails from many files into shared blocks — a space optimization
  that buys nothing at our scale and adds a whole table plus an inode field.
- **No extended inodes.** Files ≥ 4 GiB (LREG, type 9) and directory listings
  ≥ 64 KiB (LDIR, type 8) are **rejected with an error, never silently truncated**.
  An AppDir hits neither; §4.9 asserts rather than assumes.
- **No xattrs, no export table.** Both are omitted via their sentinels (§4.8).
- **No hard links, no device nodes, no FIFOs, no sockets.** An AppDir has none.
  `SquashNode` cannot represent them, so this is a type-level guarantee.
- **No uid/gid support.** Everything is root-owned (uid 0, gid 0), which is what
  `mksquashfs -root-owned` does for AppImages and what a read-only image wants.

## 2. Current State

Nothing in the tree writes, reads, or references SquashFS. There is no `squashfs`,
`mksquashfs`, or `AppImage` string anywhere in `src/`, `scripts/`, or `tests/`.

The relevant precedent is not a filesystem but the linker. `src/os/linux/link/elf.rs`
writes ELF from first principles — little-endian field packing into a `Vec<u8>`, no
encoder crate, offsets computed and then asserted. `src/os/macos/link/macho.rs` does
the same for Mach-O. A from-scratch SquashFS writer is the same kind of object and
belongs beside them.

Two lessons from that code carry directly:

1. **plan-46-D §1's `.dynstr` bug** — two independent computations derived from one
   length, one was updated, the other was not, and every import stub segfaulted while
   unit tests and `readelf` both passed. SquashFS has the identical hazard shape in
   §4.7: `id_table_start` must land such that the table ends at *exactly*
   `bytes_used`, a relationship between two separately-computed numbers.
2. **`src/os/linux/link/elf.rs:849-855`'s `runpath_string`** — one owner for a value
   two consumers derive from. §4.4's metadata-reference encoding needs the same
   treatment for the same reason.

## 3. Design Overview

Three layers, bottom-up:

1. **`MetadataWriter`** (§4.3) — an append-only stream that chunks into 8 KiB
   metadata blocks and hands back the `(stream_offset)` of each write. Owns the
   block-header format so nothing else can get the inverted compressed bit wrong.
2. **`SquashTree` / `SquashNode`** (§4.1) — the in-memory input. A plain tree,
   constructed by the caller, holding no file handles.
3. **`write`** (§4.10) — the serializer: walk the tree, emit data blocks, build
   inodes and directory listings, then the ID table, then the superblock.

The correctness risk concentrates in **§4.4, the metadata reference encoding**. Its
upper 48 bits are a *byte offset*, not a block index. Those are the same number for
any tree small enough to fit in one 8 KiB metadata block — which is every tree a
test is likely to hand-write, and every small AppDir. The bug is invisible until the
tree crosses 8 KiB, and then it fails as a mount error with no useful diagnostic.
§5's test plan exists primarily to force that boundary.

### 3.1 Why uncompressed

**Zero compressor code, and no dependency.** The alternative is zstd or gzip (the
only two the AppImage runtime's squashfuse has compiled in — its `Makefile` links
`-lzstd -lz` and nothing else; there is no xz, lz4, or lzo, regardless of what
squashfuse's `sqfs_compression_names[]` table lists). Adding either means a new
crate in a compiler that currently has five dependencies, or a compressor written
in-tree.

The cost is file size, and at our scale it is small. An AppImage is dominated by the
~920 KiB runtime, which is already compressed and which we do not touch. The payload
is one ELF (a few hundred KiB) plus seven PNGs and two text files. Uncompressed
costs perhaps a few hundred KiB on a ~1.5 MB artifact.

The benefit is not just less code — it is a **writer with no failure modes of its
own**. A compressed image can be wrong because the compressor is wrong. An
uncompressed image is wrong only if the layout is wrong, and the layout is what the
tests check.

Rejected: **zstd via a crate.** Genuinely attractive — it is what modern
appimagetool defaults to (`-comp zstd -b 128K`), it would shrink the payload
several-fold, and `zstd` is a well-maintained crate. Rejected because it buys size on
an artifact whose size is dominated by a blob we do not control, and because it adds
a dependency plus a class of bug that uncompressed cannot have. If AppImage size ever
becomes a real complaint, this is the first thing to revisit: `MetadataWriter` and
the block-list encoder are the only two places that would change, both already
isolated behind the uncompressed bit.

### 3.2 Why no reader, and how it gets tested

The temptation is to write a parser and round-trip. That proves the writer and the
parser agree, which is worth nothing — both could share a misreading, and §4.4's
byte-offset trap is exactly the kind of misreading a single author makes twice.

Instead, §5: **differential-test against real `mksquashfs`**, which we do not
control and which cannot share our bugs. `mksquashfs -noI -noD -noF -noId
-no-fragments` produces the same configuration we do, and its output is
byte-comparable modulo `mkfs_time` and two cosmetic flag bits. Where the tools are
unavailable, fall back to fixed golden bytes checked into the tree.

## 4. Detailed Design

### 4.1 The input tree

```rust
/// A node in the tree a SquashFS image is built from (plan-51-B §4.1).
///
/// Deliberately cannot represent a hard link, device node, FIFO, or socket: an
/// AppDir has none, and a type that cannot express them cannot emit them wrong.
pub(crate) enum SquashNode {
    Dir { entries: BTreeMap<String, SquashNode>, mode: u16 },
    File { data: Vec<u8>, mode: u16 },
    Symlink { target: String },
}

pub(crate) struct SquashTree {
    pub root: SquashNode, // must be Dir
}
```

`BTreeMap` is not incidental: §4.6 requires directory entries sorted
ASCIIbetically, and a `BTreeMap<String, _>` iterates in exactly that order. Sorting
is therefore a property of the type rather than a step that can be forgotten.

Symlinks carry no mode: SquashFS stores one for them, but every reader ignores it
and `mksquashfs` writes `0777`. We match that.

### 4.2 The superblock

96 bytes at offset 0, all little-endian.

| Off | Sz | Field | Value we write |
| --- | --- | --- | --- |
| 0 | u32 | `s_magic` | `0x73717368` (`hsqs`) |
| 4 | u32 | `inodes` | count |
| 8 | u32 | `mkfs_time` | **0** — see below |
| 12 | u32 | `block_size` | `131072` |
| 16 | u32 | `fragments` | `0` |
| 20 | u16 | `compression` | **`1` (ZLIB)** — see below |
| 22 | u16 | `block_log` | `17` |
| 24 | u16 | `flags` | `0x0A1B` |
| 26 | u16 | `no_ids` | `1` |
| 28 | u16 | `s_major` | `4` |
| 30 | u16 | `s_minor` | `0` |
| 32 | u64 | `root_inode` | metadata ref (§4.4) |
| 40 | u64 | `bytes_used` | archive size **excluding** padding |
| 48 | u64 | `id_table_start` | §4.7 |
| 56 | u64 | `xattr_id_table_start` | `0xFFFF_FFFF_FFFF_FFFF` |
| 64 | u64 | `inode_table_start` | |
| 72 | u64 | `directory_table_start` | |
| 80 | u64 | `fragment_table_start` | `0xFFFF_FFFF_FFFF_FFFF` |
| 88 | u64 | `lookup_table_start` | `0xFFFF_FFFF_FFFF_FFFF` |

⚠️ **`compression` must be `1`, not `0`.** This is the single most counter-intuitive
requirement in the format and the one most likely to be "fixed" by a future reader of
this code. Both readers validate the compressor id **unconditionally, before any flag
is consulted and before any block is read**:

```c
/* squashfuse fs.c — the AppImage runtime's reader */
if (!(fs->decompressor = sqfs_decompressor_get(fs->sb.compression))) return SQFS_BADCOMP;

/* linux fs/squashfs/super.c — runs before any table read, no flag check */
msblk->decompressor = supported_squashfs_filesystem(fc, major, minor, le16_to_cpu(sblk->compression));
if (msblk->decompressor == NULL) goto failed_mount;
```

Verified empirically on a fully-uncompressed image: `compression` of `0`, `2`, `4`,
`6`, or `99` all fail to mount; `1` succeeds. We declare gzip and never emit a
compressed block, so the decompressor is resolved, allocated, and never called.
`1` (ZLIB) is the safest of the six ids: `CONFIG_SQUASHFS_ZLIB` is universally
enabled and it is `mksquashfs`'s own default.

⚠️ **`block_size` must be `131072`, not `4096`.** The kernel rejects a mount where
`PAGE_SIZE > block_size`. A 4 KiB-block image will not mount on a 16 KiB-page kernel
(Apple Silicon under Asahi) or a 64 KiB-page arm64 kernel. `131072` is
`mksquashfs`'s default and clears every page size in use. `block_log` must equal
`log2(block_size)` — the kernel checks this explicitly.

`mkfs_time = 0` for reproducibility: two builds of the same tree must produce
identical bytes, which is a standing property of every other artifact this compiler
emits. `mksquashfs` itself supports this via `-mkfs-time 0`, which appimagetool
passes.

### 4.3 Flags

⚠️ **These are bit positions, and bit 2 is a trap.** Bit 2 is `SQUASHFS_CHECK`,
unused since 4.0 — it is *not* `UNCOMPRESSED_FRAGMENTS`. Every flag from bit 3 up is
one position higher than a naive reading suggests. `SQUASHFS_MKFLAGS` in
squashfs-tools pins them:

| Value | Bit | Name | Reader behavior |
| --- | --- | --- | --- |
| `0x0001` | 0 | `UNCOMPRESSED_INODES` | informational |
| `0x0002` | 1 | `UNCOMPRESSED_DATA` | informational |
| `0x0004` | 2 | **`CHECK` — unused; leave clear** | — |
| `0x0008` | 3 | `UNCOMPRESSED_FRAGMENTS` | informational |
| `0x0010` | 4 | `NO_FRAGMENTS` | informational |
| `0x0020` | 5 | `ALWAYS_FRAGMENTS` | informational |
| `0x0040` | 6 | `DUPLICATES` | informational |
| `0x0080` | 7 | `EXPORTABLE` | informational — the sentinel governs (§4.8) |
| `0x0100` | 8 | `UNCOMPRESSED_XATTRS` | informational |
| `0x0200` | 9 | `NO_XATTRS` | informational |
| `0x0400` | 10 | **`COMPRESSOR_OPTIONS`** | **the only functional flag** |
| `0x0800` | 11 | `UNCOMPRESSED_IDS` | informational |

We write `0x0A1B` = `NOI | NOD | NOF | NO_FRAG | NO_XATTR | NOID`.

Two facts worth stating plainly, because they are the opposite of what the names
suggest:

- **The `UNCOMPRESSED_*` flags are documentation.** The kernel tests exactly one
  flag ever — `SQUASHFS_COMP_OPTS` in `decompressor.c:97`. squashfuse tests none.
  Compression is decided per block by that block's own header bit. An image with
  `flags = 0` (claiming everything is compressed) and every block marked
  uncompressed still extracts perfectly. We set them honestly for `unsquashfs -s`
  and for humans, not because a reader needs them.
- **`COMPRESSOR_OPTIONS` (`0x0400`) must be clear.** It is the one flag with teeth:
  set it and the reader tries to parse a compressor-options block after the
  superblock, finds our data, and fails the mount. This is the concrete reason the
  bit-2 trap matters — a naive flag list shifts `COMPRESSOR_OPTIONS` onto `0x0200`,
  and then every image ever written claims options follow the superblock.

### 4.4 Metadata blocks and references

**Block format:** a 2-byte little-endian header, then ≤ 8192 bytes of payload.
**Bit 15 set means UNCOMPRESSED** — the sense is inverted:

```c
#define SQUASHFS_COMPRESSED_BIT     (1 << 15)
#define SQUASHFS_COMPRESSED(B)      (!((B) & SQUASHFS_COMPRESSED_BIT))
#define SQUASHFS_COMPRESSED_SIZE(B) (((B) & ~SQUASHFS_COMPRESSED_BIT) ? (B) & ~SQUASHFS_COMPRESSED_BIT : SQUASHFS_COMPRESSED_BIT)
```

Size mask `0x7FFF`. Constraints the kernel's `block.c` enforces:

- On-disk size ≤ 8192 (`output->length` is 8192 for metadata; larger → `-EIO`).
- Exactly 8192 uncompressed is legal: header `0xA000` = `0x8000 | 0x2000`.
- **Never emit header `0x0000` or `0x8000`.** `SQUASHFS_COMPRESSED_SIZE` maps a zero
  size field to 32768, which exceeds 8192 → `-EIO`. A zero-length metadata block is
  unrepresentable; do not write one.
- Every block must lie entirely within `bytes_used`.

Uncompressed metadata blocks therefore have a fixed on-disk stride of **8194**
bytes (2 header + 8192 payload).

⚠️ **The reference encoding is the trap in this sub-plan.**

```c
#define SQUASHFS_INODE_BLK(A)    ((unsigned int) ((A) >> 16))
#define SQUASHFS_INODE_OFFSET(A) ((unsigned int) ((A) & 0xffff))
```

- bits 0–15: offset into the **uncompressed** 8 KiB payload (0…8191).
- bits 16–63: the on-disk **byte offset** of that block's 2-byte header, relative to
  the table start.

The upper field is a **byte offset, not a block index**. For all-uncompressed
metadata:

```rust
/// Encode a metadata reference (plan-51-B §4.4) from a logical stream offset.
///
/// The upper 48 bits are the **on-disk byte offset** of the containing block's
/// 2-byte header relative to the table start — NOT a block index. The two are
/// identical for any stream that fits in one 8 KiB block, which is every small
/// tree and every hand-written test fixture, so a block-index encoding passes
/// every cheap test and then fails to mount the moment the stream crosses 8 KiB.
/// One owner; both `root_inode` and every directory-header `start_block` derive
/// from it.
fn metadata_ref(stream_offset: u64) -> u64 {
    let block = (stream_offset / 8192) * 8194; // on-disk stride, not block index
    let offset = stream_offset % 8192;
    (block << 16) | offset
}
```

This exact bug was hit and diagnosed during research: a writer using a block index
produced a perfect 3-file image (single block, index 0 == byte offset 0) and failed
at 1000 files with `read_inode: failed to read inode 19997:3556`. §5's stress test
exists to catch it.

`MetadataWriter` owns this. It exposes `position() -> u64` (a logical stream offset)
and `finish() -> Vec<u8>`, and callers never see block boundaries.

### 4.5 Inodes

Common 16-byte header, then a per-type tail:

```c
struct squashfs_base_inode { __le16 inode_type, mode, uid, guid; __le32 mtime, inode_number; };
```

- `mode` is **permission bits only** — the type comes from `inode_type`, and readers
  OR it back in themselves.
- `uid`/`guid` are **indices into the ID table**, not raw ids. Both are `0` for us,
  and `squashfs_get_id` rejects `index >= no_ids`.
- `inode_number` ∈ `[1, inodes]`, assigned in tree walk order.
- `mtime = 0`, for the same reproducibility reason as `mkfs_time`.

**Basic directory (type 1)** — tail: `__le32 start_block; __le32 nlink; __le16
file_size; __le16 offset; __le32 parent_inode;`

- `start_block` / `offset` — the directory table location, split from a §4.4 ref.
- `file_size` — **`listing_bytes + 3`** (§4.6). It is `u16`; a listing ≥ 64 KiB
  needs an LDIR and we reject instead (§4.9).
- `nlink` — 2 + subdirectory count.
- `parent_inode` — the root's parent is its own inode number.

**Basic file (type 2)** — tail: `__le32 start_block, fragment, offset, file_size;`
then `__le32 block_list[]`.

- `start_block` — the **absolute byte offset** of the file's data in the image. It is
  `u32`; data past 4 GiB needs an LREG and we reject (§4.9).
- `fragment = 0xFFFF_FFFF` (`SQUASHFS_INVALID_FRAG`) ⇒ block count is
  `ceil(file_size / block_size)`, tail included as a short block.
- `offset = 0` (meaningless without a fragment).
- `block_list[i]` — **bit 24 set = uncompressed**; bits 0–23 are the on-disk size.
  Bits ≥ 25 must be zero (`squashfs_block_size()` returns `-EIO` on `size >> 25`).
  An entry of **`0` means a sparse block** — a whole block of zeros with no I/O. We
  never emit one: a file of zeros should round-trip as itself, and sparseness is an
  optimization with a correctness question attached.

**Basic symlink (type 3)** — tail: `__le32 nlink; __le32 symlink_size; char
symlink[];`. **Not NUL-terminated.**

### 4.6 The directory table

```c
struct squashfs_dir_header { __le32 count, start_block, inode_number; };
struct squashfs_dir_entry  { __le16 offset; __le16 inode_number; __le16 type; __le16 size; char name[]; };
```

⚠️ **Three off-by-one biases, all mandatory:**

- **`header.count` is stored `N - 1`.** `dir_count = le32_to_cpu(dirh.count) + 1;`
  and `if (dir_count > SQUASHFS_DIR_COUNT) goto failed_read;` → **max 256 entries per
  header**.
- **`entry.size` is stored `name_len - 1`.** `size = le16_to_cpu(dire->size) + 1;`
  → max name 256.
- **`entry.inode_number` is a signed 16-bit delta** from the header's base, despite
  the `__le16` type:
  `inode_number = le32_to_cpu(dirh.inode_number) + ((short) le16_to_cpu(dire->inode_number));`

`entry.offset` is the inode's offset **within its metadata block**; the header's
`start_block` carries the block, shared across the whole run. That sharing is what
forces the third break condition below.

**A new header must start when any of:**

1. 256 entries have been written under the current one;
2. **the next entry's inode lands in a different metadata block** — `start_block` is
   per-header, so a run cannot span blocks;
3. the inode-number delta would overflow `i16`.

Breaking only at 256 corrupts any directory whose inodes straddle an 8 KiB boundary.
This was hit during research on a 1000-file tree.

**Entries must be sorted ASCIIbetically** — readers binary-search the listing. §4.1's
`BTreeMap` gives this for free.

`type` stores the **basic** type (1/2/3) even when the inode is extended.

⚠️ **The `+3` rule.** A directory inode's `file_size` is `listing_bytes + 3`, always.
The kernel synthesizes `.` and `..` at f_pos 0–2, so the external position is offset
by three:

```c
if (f_pos <= 3) return f_pos;   f_pos -= 3;   ...   return length + 3;
...
while (length < i_size_read(inode)) { /* read header; dir_count = count + 1; ... */ }
```

`length` starts at 3, so `i_size` must be `listing + 3`. `mksquashfs` literally calls
`create_inode(..., dir_size + 3, ...)`. **An empty directory has `file_size == 3`**,
and `file_size < 4` means empty.

Measured tolerance is `listing+0`…`listing+3` for a single-run directory — but only
because the loop reads whole runs and stops at the header check. With multiple runs a
short `file_size` **silently truncates the listing**. Write exactly `listing + 3`.

### 4.7 The ID table — mandatory

```c
/* fs/squashfs/id.c */
if (no_ids == 0) return ERR_PTR(-EINVAL);  /* "there should always be at least one id" */
if (length != (next_table - id_table_start)) return ERR_PTR(-EINVAL);
```

`no_ids = 0` fails with *"Bad id count in super block"*. The table is two levels:
`id_table_start` points at an array of `u64` **metadata block locations**, not at the
ids. Our minimum:

- one metadata block: header `0x8004` + `u32 0` — 6 bytes on disk;
- one `u64` location list entry pointing at it — 8 bytes;
- `no_ids = 1`; every inode uses `uid = 0, gid = 0`.

⚠️ **The adjacency constraint, and this sub-plan's plan-46-D §1 analogue.** With no
xattr table, `next_table == bytes_used`. The kernel enforces
`length == next_table - id_table_start` exactly, where `length` is
`SQUASHFS_ID_BLOCK_BYTES(no_ids)`. **The ID location list must therefore be the last
thing in the archive and end precisely at `bytes_used`.** Two separately-computed
numbers that must agree exactly — the same shape as the `.dynstr` bug that cost a
day in plan-46-D. Assert it in the writer (§4.9) rather than discovering it as
`-EINVAL` on a box.

Also: `id_table[last] < id_table_start`, and the gap ≤ 8194.

### 4.8 Omitted tables

| table | how to omit | who checks |
| --- | --- | --- |
| fragment | `fragments = 0`; `fragment_table_start = 0xFFFF…FF` | kernel `super.c`: `if (fragments == 0) goto check_directory_table;`; squashfuse `table.c`: `if (count == 0) return SQFS_OK;` |
| export/lookup | `lookup_table_start = 0xFFFF…FF` | kernel checks the **sentinel**, not the `EXPORTABLE` flag; `sqfs_export_ok` likewise |
| xattr | `xattr_id_table_start = 0xFFFF…FF` | `if (start == SQUASHFS_INVALID_BLK) return SQFS_OK;` |

`SQUASHFS_INVALID_BLK` is `-1LL` = `0xFFFF_FFFF_FFFF_FFFF`. It is valid **only** for
`xattr_id_table_start` and `lookup_table_start` — those are the only two fields the
kernel sentinel-checks. `fragment_table_start` is skipped via the count instead, so
any value works there; the sentinel is simply the clearest.

Worth noting for anyone diffing against `mksquashfs`: with `-no-fragments` it writes
`fragment_table_start` = the *next* table's position (its `generic_write_table` with
length 0 returns the current position), not a sentinel. Both read identically.

### 4.9 Invariants the writer asserts

Every one of these is enforced by a real reader. Asserting them here converts a
mount failure with no diagnostic into a build error naming the cause.

- `block_size == 1 << block_log`, `block_size == 131072`.
- `compression != 0` and resolves to a real id (§4.2).
- `flags & 0x0400 == 0` (`COMPRESSOR_OPTIONS` clear).
- `no_ids >= 1`.
- `inode_table_start < directory_table_start` — **strict**, kernel-enforced.
- `directory_table_start <= next_table`.
- `SQUASHFS_INODE_OFFSET(root_inode) <= 8192`.
- `id_table_start + SQUASHFS_ID_BLOCK_BYTES(no_ids) == bytes_used` (§4.7).
- every metadata block header ∉ {`0x0000`, `0x8000`} and payload ≤ 8192 (§4.4).
- every directory `file_size == listing + 3` and `< 65536` — else the tree needs an
  LDIR: **error, do not truncate**.
- every file `start_block + total_data < 4 GiB` and `file_size < 4 GiB` — else LREG:
  **error**.
- every directory-entry name ≤ 256 bytes; every header run ≤ 256 entries.
- `inodes` equals the number of inodes actually written; every `inode_number` ∈
  `[1, inodes]`.

### 4.10 Layout

The kernel walks the tables backwards validating adjacency, which pins the order.
`mksquashfs` writes exactly this and so do we:

```text
0            superblock (96 B)
96           data blocks          (uncompressed, byte-aligned, no gaps within a file)
             inode table          (metadata blocks)   -> inode_table_start
             directory table      (metadata blocks)   -> directory_table_start  (> inode_table_start)
             ID metadata block    (0x8004 + u32 0)    -> id_table[0]
             ID location list     (u64)               -> id_table_start; ends EXACTLY at bytes_used
bytes_used
             zero padding to 4096
```

**Padding to 4096.** `bytes_used` excludes it and no reader requires it — an image
truncated to exactly `bytes_used` extracts fine. But squashfs is `FS_REQUIRES_DEV`
and uses `sb_min_blocksize(sb, SQUASHFS_DEVBLK_SIZE)`; a loop device rounds the
backing file **down** to whole sectors, so an unpadded tail can put the ID location
list past the end of the device. `mksquashfs` pads unless `-nopad`. We pad.

## Compatibility / Format Impact

None. This sub-plan adds a module with no callers, changes no artifact, and touches
no existing file except to declare `mod squashfs;`.

## Phases

### Phase 1 — Metadata stream and the reference encoding

The riskiest primitive, isolated and landed first with no dependents.

- [ ] Add `src/os/linux/squashfs.rs` with `MetadataWriter`: `write(&[u8])`,
      `position() -> u64`, `finish() -> Vec<u8>`.
- [ ] Implement `metadata_ref` per §4.4 as the single owner of the encoding.
- [ ] Tests: a stream crossing 8192 has its second block at on-disk offset 8194;
      `metadata_ref(8192) == (8194 << 16)`; `metadata_ref(8191) == 8191`;
      `metadata_ref(16384) == (16388 << 16)`; an exactly-8192 payload emits header
      `0xA000`; no emitted header is ever `0x0000` or `0x8000`.

Acceptance: the reference-encoding tests above pass, including at least one asserting
a value that a block-index encoding would get wrong.
Commit: —

### Phase 2 — Inodes, directories, and the serializer

- [ ] Add `SquashNode`/`SquashTree` per §4.1.
- [ ] Emit data blocks + block lists (§4.5), inodes (§4.5), the directory table with
      all three biases and all three header-break conditions (§4.6).
- [ ] Emit the ID table (§4.7) and the superblock (§4.2/§4.3).
- [ ] Add every §4.9 assertion, each with an error message naming the invariant.
- [ ] Tests: a directory whose entries straddle an 8 KiB inode-block boundary emits
      more than one header for that directory; an empty directory has `file_size == 3`;
      a 257-entry directory emits two headers; a 4 GiB file and a 64 KiB listing each
      error rather than truncate; `write` is deterministic (two calls, identical bytes).

Acceptance: `unsquashfs -s` on the output reports `4.0`, `gzip`, `131072`, and the
expected flags; `write` of a fixed tree is byte-stable across runs.
Commit: —

### Phase 3 — Differential validation

The phase that proves the writer against something that cannot share its bugs.

- [ ] Add a test that shells out to `mksquashfs -noI -noD -noF -noId -no-fragments`
      and compares byte-for-byte with our output for the same tree, normalizing
      `mkfs_time` and the two cosmetic flag bits `mksquashfs` differs on (it clears
      `NOID` — `if (noI && noId) noId = FALSE;` "for backwards compatibility" — and
      sets `DUPLICATE`). `#[ignore]` when the tool is absent, so the dev box without
      squashfs-tools stays green.
- [ ] Add a stress test: 1000 files across 8 nesting levels, forcing multiple inode
      metadata blocks and multi-run directories. Extract with `unsquashfs` and compare
      every file's content and mode.
- [ ] Add a checked-in golden: the exact bytes for a small fixed tree (file +
      symlink + nested dir), so the format is pinned even where no tool exists.

Acceptance: byte-identical to `mksquashfs` modulo the documented normalizations;
1000/1000 files extract with zero content mismatches; the golden matches.
Commit: —

## Validation Plan

- **Tests:** all in `src/os/linux/squashfs.rs` per the house convention for
  format writers (`src/os/linux/link/tests.rs` is the precedent). Negative cases are
  first-class: `0x0000`/`0x8000` headers, a 4 GiB file, a 64 KiB listing, a
  257-entry run, a boundary-straddling directory. Determinism is a test, not an
  assumption.
- **Runtime proof:** the image must *mount*, not merely extract — `unsquashfs`
  tolerates things the kernel rejects. On the Ubuntu x86_64 box (`ssh -p 2228`):
  `mount -o loop,ro image.sqfs /mnt` succeeds, the tree reads back correctly, and
  `dmesg` is clean. This is the only way to exercise the three kernel-only
  invariants — `PAGE_SIZE > block_size`, `bytes_used <= bdev_nr_bytes`, and the
  strict ID-table adjacency rule — none of which `unsquashfs` checks. FUSE mounting
  is proved in plan-51-C, where the runtime exists.
- **Doc sync:** none. The module is internal and documents itself; no spec page
  describes an artifact that does not exist yet. plan-51-C owes the artifact-table
  entry.
- **Acceptance:** `cargo test`, `cargo fmt` (second pass in `repository/`).
  `scripts/test-accept.sh` and `scripts/artifact-gate.sh` must be unchanged — this
  sub-plan emits nothing and any golden movement means something leaked.

## Open Decisions

- **Uncompressed vs. zstd** — recommend uncompressed, per §3.1. Revisit only if
  AppImage size becomes a real complaint; `MetadataWriter` and the block-list
  encoder are the only two sites that would change.
- **`mksquashfs` in CI** — recommend `#[ignore]`-when-absent plus the checked-in
  golden, rather than making squashfs-tools a hard test dependency. The golden pins
  the format; the differential test catches drift when a developer has the tool. Making
  it mandatory would break `cargo test` on a clean macOS box for a test that
  duplicates the golden's coverage.
- **Emit sparse blocks for all-zero blocks** — recommend no. It is a real format
  feature (`block_list[i] == 0`) and it would shrink zero-padded payloads, but an
  AppDir has no such files and it adds a correctness question to a writer whose main
  virtue is having none.

## Summary

The engineering risk is concentrated almost entirely in **§4.4's metadata
reference**, and it has an unusually nasty shape: the wrong encoding (block index
instead of on-disk byte offset) is *correct for every tree under 8 KiB*. That covers
every hand-written test fixture and most small AppDirs. It passes review, passes
tests, ships, and then fails as an undiagnosable mount error on the first project
big enough to cross the boundary. Phase 1 exists to force that boundary before any
caller exists. The secondary risks are §4.6's three off-by-one biases and the
inode-block header-break condition — all of which are also invisible at small sizes —
and §4.7's exact-adjacency rule, which is structurally the same trap that cost a day
in plan-46-D.

Against that, the decision to skip compression removes an entire category of failure:
this writer can only be wrong about layout, and layout is what the tests check.

Left untouched: everything. No caller, no artifact, no golden, no spec. The module
is inert until plan-51-C calls it.
