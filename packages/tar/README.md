# tar

Read and write tar archives — ustar, PAX and GNU.

An archive is opened either from bytes already in memory or from a file you have
open, through the same `open` call. Both forms give you the same entries with the
same contents.

```basic
IMPORT tar
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openFile("backup.tar")
  LET archive AS tar::Archive = tar::open(f)
  FOR EACH entry IN tar::entries(archive)
    io::print(entry.name & " (" & toString(entry.size) & " bytes)")
  NEXT
END FUNC
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
PAX `g` record supplies defaults for every following entry until replaced. The
`size` a PAX record gives is the size the walk steps over, so an archive whose
ustar size field is zero (as writers emit for a file over 8 GiB) still lists
correctly.

Numbers have two spellings: normally octal text, but GNU writes a value that will
not fit — a size over 8 GiB, a timestamp past 2242, a timestamp before 1970 — as
base-256. Both are read. A PAX numeric value must be decimal text; a value that is
not is refused rather than replaced by the ustar field it overrides.

Every header's checksum is verified. Implementations historically disagreed about
whether the header bytes are signed, so either answer is accepted, as GNU tar does.

Extended headers are bounded: a PAX or GNU record larger than 1 MiB is refused
before any of its bytes are read.

## Reading entries

| Call | What you get |
|---|---|
| `tar::entries(archive)` | every entry, in archive order |
| `tar::has(archive, name)` | whether that exact name is present |
| `tar::find(archive, name)` | that entry — the last one with the name — or `errorCode::ErrNotFound` |
| `tar::read(archive, entry)` | the entry's bytes |
| `tar::readText(archive, entry)` | the same, decoded as UTF-8 |

An `Entry` reports `name`, `kind`, `isDirectory`, `size`, `mode`,
`modifiedSeconds`, `uid`, `gid`, `user`, `group` and `linkTarget`.

A directory's `name` has no trailing `/`, so `tar::find(archive, "docs")` finds the
entry `docs/` — the form Python's `tarfile` reports too.

Tar permits the same name more than once, which is how an archive records a file
being replaced. `find` and `has` see the last occurrence, because that is the
version the archive ends up with; `entries` returns all of them in order.

`kind` is one of `tar::KindFile`, `tar::KindDirectory`, `tar::KindSymlink`,
`tar::KindHardlink` or `tar::KindOther`. Links and device nodes are **listed** with
their metadata — a symlink's `linkTarget` tells you where it points — but reading
the contents of anything that is not a regular file raises
`errorCode::ErrUnsupported` rather than returning something invented.

`read` and `readText` take a `maxBytes` limit, 64 MiB by default, checked against
the recorded size before any data is read. An `Entry` is only meaningful with the
archive it came from: `read` checks the entry's extent against that archive, so a
mismatched pair fails rather than reading past the end.

Tar records no checksum for an entry's **contents** — only for its header. So
unlike zip, this package cannot tell you whether an entry's data was corrupted.
That is a property of the format worth knowing before trusting an archive.

## Writing archives

```basic
IMPORT tar
IMPORT fs

FUNC main() AS Nothing
  MUT b AS tar::Builder = tar::create()
  b = tar::addText(b, "notes.txt", "hello")
  b = tar::addDirectory(b, "data")
  b = tar::addFile(b, "data/blob.bin", fs::readBytes("blob.bin"))
  b = tar::addSymlink(b, "latest", "notes.txt")
  fs::writeBytes("out.tar", tar::finish(b))
END FUNC
```

Each `add*` gives you back a new builder; the one you passed in is unchanged.
`mode` is Unix permission bits (`420` = `0o644` for files, `493` = `0o755` for
directories), and `modifiedSeconds` is seconds since the Unix epoch, which may be
negative.

Archives are written as **ustar**, with a PAX extended header only where the fixed
fields cannot carry what is needed: a name that will not fit the prefix split, a
link target over 100 bytes, a size over 8 GiB, or a non-ASCII name. Reaching for
PAX every time would produce archives that older tools cannot read, so the writer
does not. A file over 8 GiB has its real size in a PAX `size` record and zero in
the ustar size field. A timestamp the octal field cannot carry is written in
base-256.

Output is deterministic — the same calls in the same order always produce the same
bytes — and ends with the two zero blocks that mark the end of a tar. The extra
10 KiB of record padding `tar(1)` adds by default is omitted; `bsdtar` and Python's
`tarfile` both read the result.

A name the extractor would refuse is refused when it is added, so this package
never writes an archive it cannot read back out.


## Extracting

```basic
LET written AS Integer = tar::extractTo(archive, "out")
```

`extractTo` checks every entry before it writes one. The archive is refused, and
the directory left as it was, when:

- an entry's name is empty, absolute, contains `..` anywhere, a backslash, a NUL
  byte, or names a Windows drive (`errorCode::ErrInvalidPath`);
- the archive contains a file and a directory at one path, or a file beneath a
  file (`errorCode::ErrInvalidPath`);
- something already on disk is in the way: a symbolic link anywhere along an
  entry's path, a file where a directory must go, or a directory where a file
  must go (`errorCode::ErrInvalidPath`);
- the archive contains a symbolic link, a hard link or a device node
  (`errorCode::ErrUnsupported`), because `fs` cannot create any of them. Skipping
  them would hand back a directory that looks complete and is not;
- the total size is over `maxTotalBytes` (`errorCode::ErrTooLarge`).

An entry for the archive root (`./`) is accepted and writes nothing.

**Nothing is written through a symbolic link.** Files are opened beneath the
directory with `fs::openWithin`, which refuses a link at any component when it
opens the file, so a link that appears after the check still cannot redirect a
write. Directories are created one component at a time.

A write that fails part way — an I/O error, or a file that changes under the
extraction — removes the partial file and raises the error. Entries written before
it, and directories created, are left in place. On Windows a file still open
cannot be removed, so the partial file can remain there.

File permissions and modification times are not restored, because `fs` has no way
to set either. `mode` and `modifiedSeconds` are on each `Entry` if you want them.

`maxTotalBytes` (1 GiB by default) caps the total size, checked before writing.

## Compressed archives

A `.tar.gz` is a tar archive inside a gzip stream, so it is composition rather
than a separate API:

```basic
IMPORT tar
IMPORT compress
IMPORT fs

FUNC main() AS Nothing
  ' Read one.
  LET packed AS List OF Byte = fs::readBytes("backup.tar.gz")
  LET archive AS tar::Archive = tar::open(compress::gzipDecode(packed, 268435456))

  ' Write one.
  MUT b AS tar::Builder = tar::create()
  b = tar::addText(b, "notes.txt", "hello")
  fs::writeBytes("out.tar.gz", compress::gzipEncode(tar::finish(b)))
END FUNC
```

This works in memory. There is no file-backed `.tar.gz` reading, because that
needs streaming decompression and `compress::` decodes whole buffers — so a
compressed archive costs its uncompressed size in memory, and the limit you pass
to `gzipDecode` is what bounds it.

## Errors

| Condition | Error |
|---|---|
| malformed archive, a partial block, a field that is not a number, a PAX value that is not a number | `errorCode::ErrInvalidFormat` |
| reading a link, a directory or a device node; extracting an archive with links | `errorCode::ErrUnsupported` |
| `find` on a name that is not there | `errorCode::ErrNotFound` |
| over `maxBytes`, `maxTotalBytes`, an extended header over 1 MiB, or a value too large to address | `errorCode::ErrTooLarge` |
| an entry name unsafe to write or extract, a conflict in the archive or on disk, or a link on the path | `errorCode::ErrInvalidPath` |
| reading through a file that was closed | `errorCode::ErrResourceClosed` |
| a header whose recorded checksum does not match its bytes | `tar::ErrorChecksum` |

Every message begins `tar: `.

## Testing

```
mfb test packages/tar
```

The fixtures are archives written by implementations that are not this one —
Python's `tarfile` in each of its three formats, `/usr/bin/bsdtar`, and a few laid
out by hand for cases no writer emits (a PAX size override on a small file, for
one) — together with the entry metadata Python reports for each. The GNU and PAX
fixtures deliberately carry the *same* 200-byte name stored two different ways,
and a test asserts they read back identically.

Tests that place a symbolic link on disk create it with `ln -s`, so they need a
POSIX shell.

```
python3 packages/tar/oracle/fixtures.py
```

## Documentation

```
mfb doc packages/tar --out packages/tar/doc.html
```
