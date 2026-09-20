# zip oracle

Checks `packages/zip` against an implementation that is not ours — Python's
`zipfile` — over a corpus, a fuzzed set, and the archives this package writes.

```
mfb build packages/zip                                  # refresh the package
cp packages/zip/zip.mfp packages/zip/oracle/probe/packages/zip.mfp
mfb build packages/zip/oracle/probe                     # refresh the probe

python3 packages/zip/oracle/diff.py corpus
python3 packages/zip/oracle/diff.py fuzz --count 2000 --seed 139
python3 packages/zip/oracle/diff.py roundtrip
```

Each exits 0 when every archive agreed, or the difference is declared in
`divergences.json`. **The probe embeds a copy of `zip.mfp`; refresh it after
changing the package, or you are testing the old one.**

## How it works

`probe/` is an MFBASIC program that takes a job file of archive paths and writes
one JSON line per archive: the entry list, and a CRC-32 of each entry's contents.
`diff.py` runs Python over the same paths and compares.

Every archive is opened **twice** — once from a `List OF Byte` and once from an
`fs::File` — and the probe reports `sourcesAgree`. That field is the one thing
`diff.py` will never declare away: the two forms giving different answers is not
a difference of opinion with Python, it is the package contradicting itself, and
that is the property the whole feature exists to provide.

## The corpus

- the fixture archives from `oracle/corpus/`, written by Python `zipfile` and
  `/usr/bin/zip`
- an archive made by `zip -r` out of real source files
- damaged variants of each: truncated at five points, an end-of-central-directory
  comment length that does not reach the end of the file, a central-directory
  offset past the end, an entry count larger than the directory holds, and a
  ZIP64 locator pointing past the end

## What a declared divergence may be

`divergences.json` holds two kinds of entry, and both must give a reason:

- a **policy**, which covers a whole class with one argument. There are two:
  we refuse archives Python recovers from, and we ignore the advisory "version
  needed to extract" field that Python refuses on.
- a **per-archive** declaration, for the dangerous direction — us accepting
  something Python refuses. That is never covered by a policy, because it is the
  direction in which we might be the one getting it wrong.

Two things are never declarable at all: `sourcesAgree: false`, and a crash. A
malformed archive must produce a refusal, not a panic.

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
