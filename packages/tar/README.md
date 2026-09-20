# tar

Read and write tar archives — ustar, PAX and GNU.

An archive is opened either from bytes already in memory or from a file you have
open, through the same `open` call. Both forms give you the same entries with the
same contents.

```basic
IMPORT tar
IMPORT fs
IMPORT io

SUB main()
  RES f = fs::openFile("backup.tar")
  LET archive AS tar::Archive = tar::open(f)
  FOR EACH entry IN tar::entries(archive)
    io::print(entry.name & " (" & toString(entry.size) & " bytes)")
  NEXT
END SUB
```

## Listing is cheap, even for a huge archive

A tar archive has no index. Every entry is a 512-byte header followed by its
data, padded to the next 512-byte boundary — which means the only way to list one
is to walk it.

Walking is not the same as reading it. `open` reads each header and steps over the
data between them by arithmetic, so listing a 40 GB archive reads about one header
per entry, not 40 GB. Reading one entry then reads just that entry.

The file stays yours: the package never closes it, and leaves it at the read
position it found. Closing it while an `Archive` still refers to it makes later
reads fail with `errorCode::ErrResourceClosed`.

## The three dialects

Tar's fixed-width header can hold a 100-byte name, or 255 bytes split at a `/`
across its `prefix` and `name` fields. Three conventions exist for saying anything
longer, and this package reads all of them:

| Dialect | How it says it |
|---|---|
| **ustar** | POSIX.1-1988. The `prefix`/`name` split, and nothing more. |
| **GNU** | A pseudo-entry of type `L` whose contents are the long name (`K` for a long link target). |
| **PAX** | POSIX.1-2001. A pseudo-entry of type `x` carrying `key=value` records that override the next header. |

When more than one applies, PAX wins over GNU, which wins over the ustar fields. A
PAX `g` record supplies defaults for every following entry until replaced.

Numbers have two spellings too: normally octal text, but GNU writes a value that
will not fit — a size over 8 GiB, a timestamp past 2242 — as base-256. Both are
read.

Every header's checksum is verified. Implementations historically disagreed about
whether the header bytes are signed, so either answer is accepted, as GNU tar
does.

## Reading entries

| Call | What you get |
|---|---|
| `tar::entries(archive)` | every entry, in archive order |
| `tar::has(archive, name)` | whether that exact name is present |
| `tar::find(archive, name)` | that entry, or `errorCode::ErrNotFound` |
| `tar::read(archive, entry)` | the entry's bytes |
| `tar::readText(archive, entry)` | the same, decoded as UTF-8 |

An `Entry` reports `name`, `kind`, `isDirectory`, `size`, `mode`,
`modifiedSeconds`, `uid`, `gid`, `user`, `group` and `linkTarget`.

`kind` is one of `tar::KindFile`, `tar::KindDirectory`, `tar::KindSymlink`,
`tar::KindHardlink` or `tar::KindOther`. Links and device nodes are **listed** with
their metadata — a symlink's `linkTarget` tells you where it points — but reading
the contents of anything that is not a regular file raises
`errorCode::ErrUnsupported` rather than returning something invented.

`read` and `readText` take a `maxBytes` limit, 64 MiB by default, checked against
the recorded size before any data is read.

Tar records no checksum for an entry's **contents** — only for its header. So
unlike zip, this package cannot tell you whether an entry's data was corrupted.
That is a property of the format worth knowing before trusting an archive.

## Writing archives

```basic
IMPORT tar
IMPORT fs

SUB main()
  MUT b AS tar::Builder = tar::create()
  b = tar::addText(b, "notes.txt", "hello")
  b = tar::addDirectory(b, "data")
  b = tar::addFile(b, "data/blob.bin", fs::readBytes("blob.bin"))
  b = tar::addSymlink(b, "latest", "notes.txt")
  fs::writeBytes("out.tar", tar::finish(b))
END SUB
```

Each `add*` gives you back a new builder; the one you passed in is unchanged.
`mode` is Unix permission bits (`420` = `0o644` for files, `493` = `0o755` for
directories), and `modifiedSeconds` is seconds since the Unix epoch.

Archives are written as **ustar**, with a PAX extended header only where the fixed
fields cannot carry what is needed: a name that will not fit the prefix split, a
link target over 100 bytes, a size over 8 GiB, or a non-ASCII name. Reaching for
PAX every time would produce archives that older tools cannot read, so the writer
does not.

Output is deterministic — the same calls in the same order always produce the same
bytes — and ends with the two zero blocks that mark the end of a tar. The extra
10 KiB of record padding `tar(1)` adds by default is omitted; `bsdtar` and Python's
`tarfile` both read the result.

### How many entries is reasonable

Adding an entry costs more as the archive grows, because `addFile(builder, …)`
hands the whole builder in and gets a new one back. A few thousand entries is
comfortable; tens of thousands takes minutes. Reading has no such limit.

## Extracting

```basic
LET written AS Integer = tar::extractTo(archive, "out")
```

**Every entry is checked before anything is written**, so an archive with one
unsafe entry leaves your directory exactly as it was. A name is refused with
`errorCode::ErrInvalidPath` when it is empty, absolute, contains `..`, contains a
backslash or a NUL byte, names a Windows drive, or resolves outside the target
directory once symlinks already on disk are followed.

An archive containing a **symbolic link, hard link or device node is refused
entirely** with `errorCode::ErrUnsupported`. `fs` has no way to create any of
them, and skipping them would hand you a directory that looks complete and is not
— a program that follows what should be a symlink would find nothing there, and
fail somewhere far from the cause. Refusing says so while the information is still
at hand.

`maxTotalBytes` (1 GiB by default) caps the total size, checked before writing.
Permissions and modification times are not restored, because `fs` cannot set
either; `mode` and `modifiedSeconds` are still on each `Entry` if you want them.

## Compressed archives

A `.tar.gz` is a tar archive inside a gzip stream, so it is composition rather
than a separate API:

```basic
IMPORT tar
IMPORT compress
IMPORT fs

SUB main()
  ' Read one.
  LET packed AS List OF Byte = fs::readBytes("backup.tar.gz")
  LET archive AS tar::Archive = tar::open(compress::gzipDecode(packed, 268435456))

  ' Write one.
  MUT b AS tar::Builder = tar::create()
  b = tar::addText(b, "notes.txt", "hello")
  fs::writeBytes("out.tar.gz", compress::gzipEncode(tar::finish(b)))
END SUB
```

This works in memory. There is no file-backed `.tar.gz` reading, because that
needs streaming decompression and `compress::` decodes whole buffers — so a
compressed archive costs its uncompressed size in memory, and the limit you pass
to `gzipDecode` is what bounds it.

## Errors

| Condition | Error |
|---|---|
| malformed archive, a partial block, a field that is not a number | `errorCode::ErrInvalidFormat` |
| reading a link, a directory or a device node; extracting an archive with links | `errorCode::ErrUnsupported` |
| `find` on a name that is not there | `errorCode::ErrNotFound` |
| over `maxBytes`, `maxTotalBytes`, or a value too large to address | `errorCode::ErrTooLarge` |
| an entry name unsafe to write or extract | `errorCode::ErrInvalidPath` |
| reading through a file that was closed | `errorCode::ErrResourceClosed` |
| a header whose recorded checksum does not match its bytes | `tar::ErrorChecksum` |

Every message begins `tar: `.

## Testing

```
mfb test packages/tar
```

The fixtures are archives written by implementations that are not this one —
Python's `tarfile` in each of its three formats, and `/usr/bin/bsdtar` — together
with the entry metadata those implementations report. The GNU and PAX fixtures
deliberately carry the *same* 200-byte name stored two different ways, and a test
asserts they read back identically.

```
python3 packages/tar/oracle/fixtures.py
```

## Documentation

```
mfb doc packages/tar --out packages/tar/doc.html
```
