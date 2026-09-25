# tar oracle

Checks `packages/tar` against an implementation that is not ours — Python's
`tarfile` — over a corpus, a fuzzed set, and the archives this package writes.

```
mfb build packages/tar                                  # refresh the package
cp packages/tar/tar.mfp packages/tar/oracle/probe/packages/tar.mfp
mfb build packages/tar/oracle/probe                     # refresh the probe

python3 packages/tar/oracle/diff.py corpus
python3 packages/tar/oracle/diff.py fuzz --count 2000 --seed 139
python3 packages/tar/oracle/diff.py roundtrip
```

Each exits 0 when every archive agreed, or the difference is declared in
`divergences.json`. **The probe embeds a copy of `tar.mfp`; refresh it after
changing the package, or you are testing the old one.**

## How it works

`probe/` is an MFBASIC program that takes a job file of archive paths and writes
one JSON line per archive: the entry list with every field, and a CRC-32 of each
regular entry's contents. `diff.py` runs Python over the same paths and compares.

Every archive is opened **twice** — from a `List OF Byte` and from an `fs::File`
— and the probe reports `sourcesAgree`. That field is never declarable: the two
forms giving different answers is the package contradicting itself.

## The corpus

- the fixture archives from `oracle/corpus/`: ustar, GNU and PAX written by
  Python `tarfile`, a links archive, and one written by `/usr/bin/bsdtar`
- an archive made by `bsdtar -cf` out of real source files
- damaged variants of each: truncated at five points, a flipped checksum digit,
  a size field that is not octal, a size claiming far more data than the archive
  holds, and a name field with no terminator

## What a declared divergence may be

`divergences.json` holds three policies, each with its argument:

- **we refuse archives Python recovers from.** Tar has no index, so a reader
  meeting a damaged header chooses between stopping and skipping ahead; we stop,
  because tar records no checksum over an entry's *contents* and there is
  nothing left to catch a wrong guess.
- **name encoding is unspecified.** A name whose bytes are not valid UTF-8 has no
  single right presentation; Python uses surrogate escapes, we use Latin-1. The
  bytes agree. Declared only when Python's own name carries a surrogate.
- **Python stops early.** On some damaged archives it lists fewer entries than we
  do, because it stops at the first header it dislikes and returns what it has.
  Verified independently: a third checksum walk belonging to neither
  implementation found 9 valid headers where Python reported 3. Declared in one
  direction only — listing *fewer* than Python is never covered.

## What the fuzzer found

Running 2000 mutated archives at seed 139 found one real bug, now fixed with a
regression test: a GNU long-name record is NUL-**terminated**, and the reader was
stripping NULs from anywhere in the record and keeping the rest — so `a\0bbb`
became the single name `abbb` rather than `a`, and this reader and Python then
disagreed about what the entry was called.

It also confirmed the property that matters most: across 2000 damaged archives,
`sourcesAgree` was true every time.
