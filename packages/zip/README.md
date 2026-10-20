# zip

Read and write ZIP archives, including ZIP64.

An archive can be read from bytes already in memory or from a file you have open,
through the same `open` call, and both give you the same entries with the same
contents. An archive can be written in memory with a `Builder`, or streamed to a file
with a `Writer` when it is too large to hold in memory. Both produce the same bytes.

```basic
IMPORT zip
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openFile("photos.zip")
  LET archive AS zip::Archive = zip::open(f)
  FOR EACH entry IN zip::entries(archive)
    io::print(entry.name & " (" & toString(entry.size) & " bytes)")
  NEXT
END FUNC
```

## Opening an archive

`zip::open` takes either a `List OF Byte` or an open `fs::File`:

```basic
' From memory: you already have the bytes.
LET raw AS List OF Byte = fs::readBytes("photos.zip")
LET a AS zip::Archive = zip::open(raw)

' From a file: the package reads only the parts it needs.
RES f = fs::openFile("photos.zip")
LET b AS zip::Archive = zip::open(f)
```

The memory form needs a **typed binding**. `zip::open([1, 2, 3])` does not compile,
because an untyped list literal does not tell the compiler which of the two forms you
meant. `LET raw AS List OF Byte = …` does.

Opening a file reads the central directory and nothing else. Reading one entry holds
that entry. So an archive far larger than memory is still readable, one entry at a
time: reading a 10 KB file out of a 40 GB archive costs about 10 KB.

The file stays yours. The package never closes it, and it is left at the read position
it was found at. Closing it while an `Archive` still refers to it makes later reads
fail with `errorCode::ErrResourceClosed`.

## Reading entries

| Call | What you get |
|---|---|
| `zip::entries(archive)` | every entry, in the order the archive lists them |
| `zip::has(archive, name)` | whether an entry with exactly that name is present |
| `zip::find(archive, name)` | that entry, or `errorCode::ErrNotFound` |
| `zip::read(archive, entry)` | the entry's bytes |
| `zip::readText(archive, entry)` | the same, decoded as UTF-8 |
| `zip::comment(archive)` | the archive-level comment |

An `Entry` gives you `name`, `isDirectory`, `size`, `compressedSize`, `method`, `crc`,
`modifiedSeconds`, `mode` and `comment`, all from the archive's index, so they are
available before any contents are read.

- `modifiedSeconds` is seconds since the Unix epoch. An entry written with 0 reads back as 315532800, the earliest time the DOS date and time can name.
- `mode` is the Unix `st_mode`, with the file-type and permission bits, as `unzip` reads
  it. It is 0 when the archive was not written on a Unix-like host.

`read` and `readText` take a `maxBytes` limit, 64 MiB by default. It is checked against
the size the archive records **before** any data is read, so an oversized entry costs
nothing to refuse. That is what makes it safe to open an archive you did not create and
look through its entry list.

Every entry read is checked against the CRC-32 the archive records for it. An entry
whose bytes do not match raises `zip::ErrorChecksum` rather than returning data that
fails its own checksum.

A name that appears twice is allowed. `find` and `read` return the first one in
archive order, which is the one `unzip` lists first. `extractTo` refuses such an
archive outright rather than choosing one.

## Writing an archive in memory

Build with `create` and the `add*` calls, then `finish`:

```basic
IMPORT zip
IMPORT fs

FUNC main() AS Nothing
  MUT b AS zip::Builder = zip::create()
  b = zip::addText(b, "notes.txt", "hello")
  b = zip::addDirectory(b, "images")
  b = zip::addFile(b, "images/logo.png", fs::readBytes("logo.png"))
  fs::writeBytes("out.zip", zip::finish(b))
END FUNC
```

Each `add*` gives you back a new builder; the one you passed in is unchanged.

Every `add*` takes the same optional settings:

- `store = TRUE` skips compression. Otherwise an entry is deflated when that makes it
  smaller and stored when it does not, so an incompressible file is never made larger.
- `modifiedSeconds` is seconds since the Unix epoch. `0` writes 1980-01-01, the earliest DOS time, which reads back as 315532800. Times before
  1980 are written as the earliest DOS time and the exact time is kept in an
  extended-timestamp field; a negative time, or one past the 32-bit range (2106), is
  refused with `errorCode::ErrInvalidArgument`.
- `mode` is the Unix permission bits, `0o0000` to `0o7777`. `0` means `0o644` for a file
  and `0o755` for a directory. Anything else is refused with
  `errorCode::ErrInvalidArgument`.

The times in the DOS fields are UTC, as Go's `archive/zip` writes them. Readers that know
the extended-timestamp field, as this package does, get the exact time back.

Output is deterministic: the same calls in the same order always produce the same
bytes, so you can diff two archives.

## Writing an archive to a file

`Builder` holds the whole archive in memory. To write a large archive, use a `Writer`,
which writes each entry's contents to a file as it is added. The central directory, about
50 bytes plus the name for each entry, is held until `finishWriter` writes it:

```basic
IMPORT zip
IMPORT fs

FUNC main() AS Nothing
  RES f AS fs::File = fs::createTempFile()
  MUT w AS zip::Writer = zip::openWriter(f)
  FOR i = 1 TO 100000
    w = zip::writeText(w, "lines/" & toString(i) & ".txt", "line " & toString(i))
  NEXT
  zip::finishWriter(w, "a hundred thousand files")
  fs::close(f)
END FUNC
```

- `openWriter` takes a file that **must be empty**. Offsets in a zip count from the start
  of the file, so an archive written after existing bytes would point at the wrong places.
  A non-empty file is refused with `errorCode::ErrInvalidArgument`.
- `writeFile`, `writeText` and `writeDirectory` return the updated writer. Use that one.
  The one you passed in refers to the same file at an earlier position, and writing to it
  again would corrupt the archive.
- `finishWriter` writes the central directory and flushes the file. Call it once, and do
  not write to the writer afterwards.
- A `Writer`'s memory grows with the number of entries, not with the size of their
  contents. Each entry costs the same to add however many came before it.
- The file stays yours. This package never closes it, so close it after `finishWriter`.

A refused name or setting is refused before anything is written to the file.

### Names you can write

A name is refused with `errorCode::ErrInvalidPath` when it is empty, absolute (begins
with `/`), contains a backslash or a NUL, begins with a Windows drive (`C:`), contains a
`..` component, or contains an empty or `.` component (so `a//b` and `./a` are refused).
The writer and the extractor apply the same name rules. The writer does not check for two
entries that would extract to one path, nor for a file where another entry needs a
directory: `extractTo` refuses those archives, so give each entry its own path.

A file name may not end in `/`; use `addDirectory` or `writeDirectory`.

### ZIP64

ZIP64 records are written only when a value needs them. A small archive is an ordinary
zip any reader accepts. An archive with 65,535 or more entries, or an entry or offset
of 4 GiB or more, gets the ZIP64 fields it needs.

## Extracting

```basic
LET written AS Integer = zip::extractTo(archive, "out")
```

`extractTo` returns how many files it wrote. It creates `out`, and any missing parents,
when `out` does not exist.

`extractTo` treats the archive as untrusted input:

- **Every entry is checked before any byte is written.** An archive with one unsafe
  entry leaves your directory exactly as it was. A name containing `..`, a backslash, a
  NUL, a drive, or an absolute path is refused. An empty or `.` component is dropped, so
  `./a//b.txt` is written to `out/a/b.txt`.
- **Two entries that extract to the same path** are refused with
  `errorCode::ErrInvalidFormat`. So is a file where another entry needs a directory,
  such as `a` and `a/b.txt`. Comparison is after dropping empty and `.` components.
- **It never overwrites a file.** A path that already exists is refused with
  `errorCode::ErrAlreadyExists`, and nothing is written.
- **Containment is enforced where each file is opened.** A symlink already on disk can
  not redirect a file's contents outside `out`: each file is opened with `fs::openWithin`,
  which refuses any escape at open time, so there is no window between checking a path and
  writing it. Directories are created by path, so a symlink swapped in during the
  extraction could direct the creation of an empty directory outside `out`; no file
  content is written there.
- **A failed extraction leaves nothing behind.** If an entry fails after writing has
  started, such as a CRC mismatch or an I/O error, every file and directory this call
  created is removed, and the original error is raised.
- **Names that differ only in case** are kept apart on a case-sensitive file system. On a
  case-insensitive one (macOS and Windows by default) they are one file: the second is
  refused with `errorCode::ErrAlreadyExists`, and the rollback removes what was written.
- **`maxTotalBytes`** (1 GiB by default) caps the total uncompressed size, checked before
  writing, so a small archive cannot fill a disk.
- Stored entries are copied in bounded chunks, so a very large entry from a file-backed
  archive is never held in memory. A deflated entry is inflated whole, bounded by its
  declared size.

**Permissions and times are not restored.** `fs` cannot set either. Extracted files are
created user-only (0600, less the umask), as `fs` creates every file it opens for writing.
The archive's `mode` and `modifiedSeconds` are still available on each `Entry` if you want
to apply them yourself.

## What this package does not do

- **No encryption.** An encrypted entry raises `errorCode::ErrUnsupported`. It is not
  decrypted, and it is not returned as ciphertext.
- **No compression method other than stored and deflate.** Anything else raises
  `errorCode::ErrUnsupported`.
- **No multi-disk archives.** `errorCode::ErrUnsupported`.
- **No streaming decompression.** A deflated entry is decoded in one piece, bounded by
  `maxBytes`. Stored entries are not held whole by `extractTo`.
- **No `.zip` inside `.zip` handling, no self-extracting archives, and no data
  descriptors.** An archive written with a data descriptor is read through its central
  directory, which records the sizes; the descriptor is not needed.

## Names and character sets

Zip stores an entry name either as UTF-8 or as code page 437, and says which in a flag.
This package reads both. It carries its own CP437 table, because `encoding::Codepage`
has no CP437 member.

Names are always **written** as UTF-8, with the flag set to say so.

A name that cannot be read refuses the **whole archive**, not just the one entry: a name
containing a NUL, or one flagged as UTF-8 that is not valid UTF-8. A listing that dropped
such an entry would hide it, and the entry is identified by its name.

## Limits

| Limit | Value | Refusal |
|---|---|---|
| Entry name | 65,535 bytes | `errorCode::ErrTooLarge` |
| Archive comment | 65,535 bytes | `errorCode::ErrTooLarge` |
| Central directory | 256 MiB | `errorCode::ErrTooLarge` |
| `read` / `readText` | 64 MiB by default, set with `maxBytes` | `errorCode::ErrTooLarge` |
| `extractTo` total | 1 GiB by default, set with `maxTotalBytes` | `errorCode::ErrTooLarge` |

The central directory is read into memory when an archive is opened. An archive of a few
million entries is within the limit; a larger one is refused rather than allocated.

Building with `create` costs more as the archive grows, because each `add` call takes the
whole builder and returns a new one, so its list is copied, and the total cost grows faster
than the number of entries. For many entries, or for an archive that will not fit in
memory, use a `Writer`, whose cost per entry does not grow. Reading has no such cost.

## Errors

Every failure uses a shared `errorCode::` value, except a checksum mismatch, which raises
`zip::ErrorChecksum`. Every message begins `zip: `.

| Condition | Error |
|---|---|
| malformed or truncated archive; fields that contradict each other; overlapping entries; a local header that disagrees with its central record; an undecodable name; two entries extracting to one path | `errorCode::ErrInvalidFormat` (7-705-0003) |
| encryption, an unsupported method, multi-disk, patched or masked data | `errorCode::ErrUnsupported` (7-705-0007) |
| `find` on a name that is not there | `errorCode::ErrNotFound` (7-705-0004) |
| `extractTo` onto a path that exists, or onto a file | `errorCode::ErrAlreadyExists` (7-705-0005) |
| a time or mode that cannot be recorded; a non-empty file given to `openWriter` | `errorCode::ErrInvalidArgument` (7-705-0002) |
| an unsafe entry name | `errorCode::ErrInvalidPath` (7-703-0002) |
| a file that was closed while an archive still refers to it | `errorCode::ErrResourceClosed` (7-703-0004) |
| over `maxBytes`, `maxTotalBytes`, a length limit, or the central directory limit | `errorCode::ErrTooLarge` (7-705-0027) |
| an entry whose bytes do not match its recorded CRC-32 | `zip::ErrorChecksum` (9315-0001) |

## Testing

```
mfb test packages/zip
```

The unit tests cover the reader, both writers, the name rules and the extractor,
including each refusal and the rollback after a failure. The fixtures are archives
written by implementations that are not this one, Python's `zipfile` and `/usr/bin/zip`,
together with the entry metadata those implementations report for them, so the tests
compare this reader against independent readers rather than against its own earlier
output. Regenerate them with:

```
python3 packages/zip/oracle/fixtures.py
python3 packages/zip/oracle/cp437_table.py
```

The oracle in `oracle/` checks the reader and the extractor against Python's `zipfile`
over a corpus, a seeded fuzz set, and the archives this package writes. See
`oracle/README.md`.

The examples in the DOC blocks are built and run by:

```
packages/zip/check-doc-examples.sh
```

## Documentation

```
mfb doc packages/zip --out packages/zip/doc.html
```
