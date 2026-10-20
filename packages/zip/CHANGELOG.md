# Changelog

All notable changes to the `zip` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to the
exported API, to an error code, or to a result for a given input requires a new major
version. A change to which archives are refused, or to which error a refusal carries,
counts as a change to a result.

## 1.0.0

The first stable release. The package was 0.1.0 while its design was settled and was
never published, so the entries below describe the whole 1.0.0 surface.

### Reading

- `zip::open` takes a `List OF Byte` or an open `fs::File` and gives the same entries
  and the same contents either way. Opening a file reads the central directory only.
- Stored and deflated entries are read. Every entry read is checked against its CRC-32.
- Refused with `errorCode::ErrUnsupported`: encryption (including an encryption flag
  set in the central directory alone), any other compression method, multi-disk
  archives, patched data, strong encryption and masked headers.
- Refused with `errorCode::ErrInvalidFormat`: a malformed or truncated archive; a
  central directory whose declared size disagrees with its entry count, including a
  directory with no entries that declares bytes; two entries whose data regions overlap,
  at any offset, including past 2 GiB; an entry whose local header disagrees with its
  central record (name, method, sizes, or a flag that changes the data); a directory that
  records any size or CRC; a deflated entry that inflates past its declared size, refused
  the same way whatever limit the caller passes; and an entry name that contains a NUL or,
  when declared UTF-8, is not valid UTF-8.
- `read` and `extractTo` apply the same local-header checks, so an archive `read`
  refuses is never extracted.

### Writing

- `zip::create`, `zip::addFile`, `zip::addText`, `zip::addDirectory` and `zip::finish`
  build an archive in memory.
- `zip::openWriter`, `zip::writeFile`, `zip::writeText`, `zip::writeDirectory` and
  `zip::finishWriter` write an archive to a file as it is built, so the contents never have
  to fit in memory. The central directory, about 50 bytes plus the name for each entry, is
  held until `finishWriter`. For the same calls they produce the same bytes as the in-memory
  builder.
- Each entry can carry a Unix permission mode (`mode`, 0 for the default) and a
  modification time (`modifiedSeconds`, 0 for none). The time is recorded to the second
  in an extended-timestamp field, which readers that know it prefer, and to the DOS
  two-second grain otherwise.
- A local header that needs ZIP64 carries the ZIP64 extra field, as the format requires.
- Names are validated when they are written: an unsafe name, or one with an empty or `.`
  component, is refused with `errorCode::ErrInvalidPath` before any byte is written.

### Extracting

- `zip::extractTo` checks every entry before it writes any byte.
- It never overwrites a file: a path that already exists is refused with
  `errorCode::ErrAlreadyExists`.
- Two entries that extract to one path, and a file where another entry needs a
  directory, are refused with `errorCode::ErrInvalidFormat`.
- On a case-insensitive file system, two names that differ only in case are one file: the
  second is refused with `errorCode::ErrAlreadyExists`, and the rollback removes what was written.
- Each file is opened with containment enforced, so a symlink already on disk cannot
  redirect a write out of the target directory.
- `extractTo` creates the target directory, with its parents, when it does not exist.
- If an entry fails after writing has started, every file and directory the call created
  is removed and the original error is raised.
- Extracted files are created user-only (0600, less the umask), as `fs` creates every file
  it opens for writing. The archive's recorded mode is not restored, because `fs` cannot set
  permissions. Directories are created by path, so a symlink swapped in during an extraction
  can direct the creation of an empty directory outside the target; file contents are
  always opened with containment enforced.

### Errors

- Every failure uses a shared `errorCode::` value, except a CRC mismatch, which raises
  `zip::ErrorChecksum` (93150001).
- A `Writer` raises `errorCode::ErrInvalidArgument` when its file is not empty.

### Verification

- Unit tests cover the reader, the in-memory and streaming writers, the name rules, and
  the extractor, including each refusal and the rollback.
- The Python `zipfile` oracle (`oracle/`) checks the reader over a corpus, a seeded fuzz
  set, the archives this package writes, and extraction. Declared differences are listed
  in `oracle/divergences.json` with a reason.
- The examples in the DOC blocks are built and run by `check-doc-examples.sh`.
