# zip oracle

Checks `packages/zip` against an implementation that is not ours -- Python's
`zipfile` -- over the archives it reads, the archives it writes, and the
archives it extracts.

```
mfb build packages/zip                                       # refresh the package
cp packages/zip/zip.mfp packages/zip/oracle/probe/packages/zip.mfp
cp packages/zip/zip.mfp packages/zip/oracle/writer-probe/packages/zip.mfp
mfb build packages/zip/oracle/probe                          # the reader and extractor probe
mfb build packages/zip/oracle/writer-probe                   # the writer probe

python3 -I packages/zip/oracle/diff.py corpus
python3 -I packages/zip/oracle/diff.py fuzz --count 2000 --seed 139
python3 -I packages/zip/oracle/diff.py roundtrip
python3 -I packages/zip/oracle/diff.py extract
```

Each mode exits 0 when every archive agreed, or the difference is declared in
`divergences.json`. Anything else exits 1 and prints what differed, one `FAIL`
line per archive followed by its reasons, and a summary line of the form
`<mode>: N archive(s), M disagreement(s)`.

**Both probes embed a copy of `zip.mfp`. Refresh both copies after changing the
package, or you are testing the old one.** `build/` and `packages/` inside each
probe directory are gitignored.

## The four modes

| Mode | Archives | Checked |
|---|---|---|
| `corpus` | the fixtures in `corpus/`, an archive made by `zip -r`, and damaged variants of each | reading: entry list, sizes, CRC of every entry's contents, refusals |
| `fuzz --count N --seed S` | the corpus with bytes, offsets and lengths mutated from a fixed seed | the same as `corpus`, on mutated archives |
| `roundtrip` | archives this package writes, produced by `writer-probe` | Python `zipfile` reads each one back and checks it against the expectations in `diff.py` (see below) |
| `extract` | the corpus, the fuzz set, and generated archives with hostile names | what `extractTo` writes to disk, against a Python model of extraction |

### corpus and fuzz

`probe/` is an MFBASIC program that takes a job file of archive paths and writes
one JSON line per archive: the entry list, and a CRC-32 of each entry's contents.
`diff.py` runs Python over the same paths and compares.

Every entry is read, directories included, so a directory's local header and CRC
are compared like any other entry's. Reads use the same 1 TiB cap as extraction
(see the extract section). Every archive is opened **twice** -- once from a
`List OF Byte` and once from an `fs::File` -- and the probe reports `sourcesAgree`. That field is the one thing
`diff.py` will never declare away: the two forms giving different answers is not
a difference of opinion with Python, it is the package contradicting itself, and
that is the property the whole feature exists to provide.

### roundtrip

`writer-probe/` is a separate MFBASIC executable. It takes an output directory and
writes five archives into it, each through the in-memory `Builder` or the
streaming `Writer`:

- `builder-mixed.zip` and `stream-mixed.zip`: the same nine entries and the same
  comment, one built in memory and one streamed. Stored and deflated entries, an
  empty file, a file that deflate cannot shrink, a directory with a non-default
  mode, a non-ASCII name, an odd and an even modification time, a non-default
  file mode, and a non-ASCII comment.
- `builder-empty.zip` and `stream-empty.zip`: no entries.
- `entry-count-65536.zip`: 65536 entries, which needs ZIP64 for the entry count.

`diff.py` holds the expectations (`roundtrip_expectations`, `mixed_entries`,
`entry_count_entries`) and checks each archive with Python `zipfile`:
`testzip()`, the entry names and order, the comment, each entry's size, its
content and CRC, its compression method where one is named, the UTF-8 flag for
non-ASCII names, the DOS date and time (floored to two seconds), the exact time
in the extended-timestamp field (0x5455, absent for an unrecorded time), the
Unix mode in `external_attr`, and the directory attribute on directories.

It also requires each pair in `SAME_BYTES` to be byte-identical, because
`lib.mfb` documents that the Builder and the Writer produce the same bytes for
the same calls.

The writer probe and `diff.py` must agree on the names, contents, times, modes
and comment. They are written twice on purpose: a drift between them fails the
gate, and nothing ever compares against the package's own reading of what it
wrote. A written archive that Python refuses or misreads is a package bug, not a
divergence, so `roundtrip` declares nothing.

`roundtrip` runs the writer probe into a fresh temporary directory and reads
only what it produced. There is no fallback to earlier output.

`roundtrip` takes about a minute, almost all of it the writer probe writing the
65536-entry archive (the writer probe alone took 55.9 s; measured with `time` on
the machine it was last run on).

### extract

`extract` runs the package's `extractTo` on every archive in the corpus and the
fuzz set, plus the hostile-name set, and compares what ends up on disk against
Python's view of the archive. The probe is the same binary as the reader:

```
zipprobe extract <job-file> <work-directory>
```

Each archive gets its own empty directory `<work-directory>/x<index>`, and one
JSON line is printed per archive, in job order:

- `{"path", "written": N, "files": [{"path", "crc", "size"}, ...]}` where
  `files` is walked from the directory after extraction, sorted, with each
  file's CRC-32 and size. It is what is on disk, not what the call reports.
- `{"path", "refused": "<errorCode name>", "stage": "open"|"extract", "leftover": [...]}`
  where `stage` says whether the archive was refused when opened or when
  extracting, and `leftover` lists anything on disk after the refusal. The
  package promises that a failed extraction leaves nothing behind, so a
  non-empty `leftover` always fails the gate and is never declarable.

**The cap.** `extractTo` is called with a 1 TiB `maxTotalBytes` (`EXTRACT_CAP` in
`probe/src/main.mfb`), and the describe mode's reads use the same cap. The package
default is 1 GiB. A declared total over that default is refused by the cap, not by
any property the comparison is about, so the probe does not let it decide. The cap
is therefore not a divergence. The 1 GiB default is not asserted by any test:
`packages/zip/src/test_extract.mfb` exercises `maxTotalBytes` with an explicit small
limit.

The extract probe reports the errorCode number as well as its name (`"code"`), and
the comparison uses the number.

`diff.py` does not call `zipfile.extractall`. That function silently sanitises
the names it is given, which is exactly what the check has to see. Instead the
Python side classifies each raw name itself (`unsafe_classes`, `rel_of`,
`python_refuses_extraction`) and compares:

- **Accepted archives.** The set of files on disk must equal the set of
  non-directory entries in Python's view, after dropping empty and `.`
  components, with the same size and CRC for each. An archive that contains a
  name the policy refuses is a failure even if extraction succeeded.
- **Refused archives.** Python must refuse the archive too, or Python must be
  unable to extract it cleanly (an entry it cannot read, or a file entry with no
  path component), or one of two things must hold:
  - the archive contains a name of a policy class, the refusal is `ErrInvalidPath`
    or `ErrInvalidFormat`, and `policy:refuses-unsafe-names` is declared; or
  - the READ path refuses the same archive with a per-entry refusal of the same
    error code, and `policy:stricter-than-python` is declared. A refusal the read
    path does not make is never accepted under this rule.

  Any other refusal of an archive Python extracts cleanly is a disagreement.

The hostile-name archives are written by Python `zipfile` in `build_hostile`:
`../` and an absolute path, a backslash, a drive prefix, a duplicate name, a
file where a directory is needed, and a control archive whose `./` and empty
components the package must drop.

## What a declared divergence may be

`divergences.json` holds policies, which cover a whole class with one argument,
and per-archive declarations, which are for the dangerous direction only. Every
entry must say why.

- `policy:stricter-than-python` -- we refuse archives Python recovers from, and,
  per entry, four classes the package refuses for reasons its source gives: the
  two copies of a local header disagree, a directory's header is checked too,
  an entry's footprint includes its name and extra field, and flags that change
  the data are unsupported. The policy text names each one.
- `policy:version-needed-is-advisory` -- Python refuses on the advisory "version
  needed to extract" field; we do not.
- `policy:refuses-unsafe-names` -- extraction refuses the name classes an
  independent extractor would sanitise or overwrite. It covers only a refusal
  with `ErrInvalidPath` or `ErrInvalidFormat` on an archive that contains one of
  those names.
- `lenient:<basename>` -- we accepted an archive Python refused. Declarable per
  archive only.
- `name:<ours>:<theirs>` -- a name decoded differently. Declarable per archive.

Two things are never declarable at all: `sourcesAgree: false`, and a crash. A
malformed archive must produce a refusal, not a panic. A leftover file after a
refused extraction is never declarable either.

A refusal that is not covered by one of the policies is a disagreement and is
reported, not declared. That applies to extraction: the policy for name classes
is the only refusal it covers.

## Open disagreements

These are reported, not declared. They are what the gates currently fail on, and
each is a case the package has to resolve.

- **A directory entry with a nonzero recorded CRC is accepted.** `fuzz` at seed 139
  finds 7 archives of this form. Python refuses the read with `BadZipFile: Bad
  CRC-32`. `read` returns `[]` for a directory before any check of its recorded CRC,
  so the package reads it as empty. This is the dangerous direction and is never
  declarable. Reproducer, built with Python `zipfile` by patching the central CRC of
  `d/` from 0 to 1:

  ```
  Python:  BadZipFile: Bad CRC-32 for file 'd/'
  ours:    read d/ -> contentCrc 0, readError null; extractTo writes d/x.txt and succeeds
  ```

- **Same entry, different error code.** `fuzz` at seed 139, archive index 1134 (the
  file `fuzz01134.zip`, which the oracle regenerates from the seed): the read path
  refuses `data/two.txt` with `ErrInvalidFormat`, and extraction refuses the archive
  with `ErrTooLarge`. Python reads the 800 bytes, with the recorded CRC. Both refuse,
  so the archive is not one Python extracts cleanly and only the code differs; the
  rule above does not accept a different code. The extraction inflates each deflated
  entry with `maxBytes` equal to its declared size (`writeEntry` in
  `packages/zip/src/extract.mfb`), so the decompressor reports the overrun. A
  hand-built sync-flushed deflate stream did not reproduce the code difference (both
  paths return `ErrInvalidFormat`), so the fuzzed archive is the reproducer.

## What the fuzzer found

Running 2000 mutated archives at seed 139 found **six** real gaps in the reader,
all now fixed with a regression test each:

| Gap | Why it mattered |
|---|---|
| the two copies of an entry's name were not compared | a tool listing the central directory and a tool walking local headers would report different names for the same entry |
| the directory's declared size was not checked against its entry count | a reader walking by size and one walking by count see different sets of entries |
| general-purpose flag bits 5, 6 and 13 were ignored | patched or strongly-encrypted data was inflated and returned as though it were the entry's contents |
| flags were read only from the local header | the central directory's copy could declare encryption the local copy did not |
| overlapping entry data regions were accepted | the shape of a zip bomb, and of a confusion attack |
| a NUL inside an entry name was kept | Python truncates there and reports a shorter name; either way two tools disagree about what the entry is called |

The fuzz seed is fixed, so any of these can be reproduced.

## The corpus

- the fixture archives from `oracle/corpus/`, written by Python `zipfile` and
  `/usr/bin/zip`
- an archive made by `zip -r` out of real source files
- damaged variants of each: truncated at five points, an end-of-central-directory
  comment length that does not reach the end of the file, a central-directory
  offset past the end, an entry count larger than the directory holds, and a
  ZIP64 locator pointing past the end
