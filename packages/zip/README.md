# zip

Read and write ZIP archives, including ZIP64.

An archive is opened either from bytes already in memory or from a file you have
open, through the same `open` call. Both forms give you the same entries with the
same contents — the package reads through one internal seam, so there is no
second code path that could drift.

```basic
IMPORT zip
IMPORT fs
IMPORT io

SUB main()
  RES f = fs::openFile("photos.zip")
  LET archive AS zip::Archive = zip::open(f)
  FOR EACH entry IN zip::entries(archive)
    io::print(entry.name & " (" & toString(entry.size) & " bytes)")
  NEXT
END SUB
```

## The two ways to open an archive

`zip::open` takes either a `List OF Byte` or an open `fs::File`:

```basic
' From memory — you already have the bytes.
LET raw AS List OF Byte = fs::readBytes("photos.zip")
LET a AS zip::Archive = zip::open(raw)

' From a file — the package reads only the parts it needs.
RES f = fs::openFile("photos.zip")
LET b AS zip::Archive = zip::open(f)
```

The memory form needs a **typed binding**. `zip::open([1, 2, 3])` does not
compile, because an untyped list literal does not tell the compiler which of the
two forms you meant; `LET raw AS List OF Byte = …` does.

### What the file form keeps in memory

Opening a file-backed archive reads the list of entries and nothing else. Reading
one entry holds that one entry. So an archive far larger than memory is still
readable, one entry at a time — reading a 10 KB file out of a 40 GB archive costs
about 10 KB, not 40 GB.

The file stays yours. The package never closes it, and it is still open and at the
same read position after any call here. Close it when you are done, as usual — and
note that closing it while an `Archive` still refers to it makes later reads
through that archive fail with `errorCode::ErrResourceClosed`.

## Reading entries

| Call | What you get |
|---|---|
| `zip::entries(archive)` | every entry, in the order the archive lists them |
| `zip::has(archive, name)` | whether an entry with exactly that name is present |
| `zip::find(archive, name)` | that entry, or `errorCode::ErrNotFound` |
| `zip::read(archive, entry)` | the entry's bytes |
| `zip::readText(archive, entry)` | the same, decoded as UTF-8 |
| `zip::comment(archive)` | the archive-level comment |

An `Entry` tells you `name`, `isDirectory`, `size`, `compressedSize`, `method`,
`crc`, `mode`, `modifiedSeconds` and `comment` — all read from the archive's index,
so they are available before you read any contents.

`read` and `readText` take a `maxBytes` limit, 64 MiB by default:

```basic
LET small AS List OF Byte = zip::read(archive, entry, 1048576)   ' at most 1 MiB
```

The limit is checked against the size the archive records **before** any data is
read, so an oversized entry costs nothing to refuse. That is what makes it safe to
open an archive you did not create and look through its entry list.

Every entry you read is checked against the CRC-32 the archive records for it. An
entry whose bytes do not match raises `zip::ErrorChecksum` rather than handing you
data that fails its own checksum.

## Writing archives

Build with `create` and the `add*` calls, then `finish`:

```basic
IMPORT zip
IMPORT fs

SUB main()
  MUT b AS zip::Builder = zip::create()
  b = zip::addText(b, "notes.txt", "hello")
  b = zip::addDirectory(b, "images")
  b = zip::addFile(b, "images/logo.png", fs::readBytes("logo.png"))
  fs::writeBytes("out.zip", zip::finish(b))
END SUB
```

Each `add*` gives you back a new builder; the one you passed in is unchanged.

Entries are deflated when that makes them smaller and stored when it does not, so
an incompressible file is never made bigger by compressing it. Pass `store = TRUE`
to skip compression. `modifiedSeconds` is seconds since the Unix epoch; leave it 0
and the archive records the earliest time the format can express. Zip timestamps
tick every two seconds, so an odd second rounds down.

Output is deterministic: the same calls in the same order always produce the same
bytes, which means you can diff two archives and learn something. ZIP64 records are
written only when the archive actually needs them, so an ordinary archive is an
ordinary zip that any reader accepts.

### How many entries is reasonable

Adding an entry costs more as the archive grows, because `addFile(builder, …)`
hands the whole builder in and gets a new one back. A few thousand entries is
comfortable; tens of thousands takes minutes and a lot of memory. If you are
writing an archive with very many entries, write it in pieces or use another tool.
Reading an archive with very many entries has no such limit.

## Extracting

```basic
LET written AS Integer = zip::extractTo(archive, "out")
```

`extractTo` creates directories, writes files and returns how many files it wrote.

**It checks every entry before it writes anything.** An archive with one unsafe
entry leaves your directory exactly as it was, rather than extracting part of
itself and then failing. An entry name is refused with `errorCode::ErrInvalidPath`
when it is empty, absolute, contains `..`, contains a backslash or a NUL byte,
names a Windows drive, or resolves outside the target directory once any symlinks
already on disk are followed — the last of which is the case that string
inspection alone cannot catch.

`maxTotalBytes` (1 GiB by default) caps the total size, checked before writing, so
a small archive cannot be used to fill a disk. Each entry's checksum is verified as
it is written; a mismatch deletes the partial file and raises `zip::ErrorChecksum`.

File permissions and modification times are **not** restored — `fs` has no way to
set either — so extracted files get the host's defaults. The `mode` and
`modifiedSeconds` an `Entry` reports are still there if you want them.

## What this package does not do

- **No encryption.** An encrypted entry raises `errorCode::ErrUnsupported`. It is
  not decrypted, and it is not silently returned as ciphertext.
- **No compression method other than stored and deflate.** Anything else raises
  `errorCode::ErrUnsupported` rather than being guessed at.
- **No multi-disk archives.** `errorCode::ErrUnsupported`.
- **No streaming decompression.** A deflated entry is decoded in one piece, bounded
  by `maxBytes`. Stored entries are not: `extractTo` copies those in fixed-size
  pieces, so a huge stored entry extracts without being held whole.
- **No `.zip` inside `.zip` handling, no self-extracting archives.**

## Errors

Everything but the checksum uses a shared `errorCode::` value:

| Condition | Error |
|---|---|
| malformed archive, truncation, a field that contradicts another | `errorCode::ErrInvalidFormat` |
| encryption, an unsupported method, multi-disk | `errorCode::ErrUnsupported` |
| `find` on a name that is not there | `errorCode::ErrNotFound` |
| over `maxBytes`, `maxTotalBytes`, or an internal limit | `errorCode::ErrTooLarge` |
| an entry name that is unsafe to write or extract | `errorCode::ErrInvalidPath` |
| reading through a file that was closed | `errorCode::ErrResourceClosed` |
| an entry whose bytes do not match its recorded CRC-32 | `zip::ErrorChecksum` |

Every message begins `zip: `.

## Names and character sets

Zip stores an entry name either as UTF-8 or as code page 437, and says which in a
flag. This package reads both — it carries its own CP437 table, because
`encoding::Codepage` has no CP437 member — so a name written by a DOS-era or
Windows-default tool comes back as the text it was meant to be rather than as
mangled bytes.

Names are always **written** as UTF-8, with the flag set to say so.

## Testing

```
mfb test packages/zip
```

The fixtures are archives written by implementations that are not this one —
Python's `zipfile`, and `/usr/bin/zip` — together with the entry metadata those
implementations report for them, so the tests compare this reader against
independent readers rather than against its own earlier output. Regenerate them
with:

```
python3 packages/zip/oracle/fixtures.py
python3 packages/zip/oracle/cp437_table.py
```

## Documentation

```
mfb doc packages/zip --out packages/zip/doc.html
```
