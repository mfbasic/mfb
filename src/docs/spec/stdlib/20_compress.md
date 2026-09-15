# In-memory compressed formats and their checksums (compress)

The `compress` package works on whole values held in memory: every member takes a whole
`List OF Byte` and returns its whole result in one call. Called with the `compress::`
qualifier; `IMPORT compress` needs no manifest dependency.
[[src/codegen/builtins/compress/mod.rs:COMPRESS]]

This topic specifies the model behind the package: the implementation guarantee, how its
source is gated into a build, and the CRC-32 model. The per-function API — signatures,
parameters, errors — is owned by `./mfb man compress`.

## Implementation model

Every member is a `Body::Rewrite` onto an injected MFBASIC function named `__compress_*`,
registered as a `RegistryHelper`. There is no `Body::abi_function` or `Body::abi_inline`
member, no runtime-helper family, no `dlopen` and no vendored library.
[[src/codegen/builtins/compress/mod.rs:register]]
[[src/codegen/builtins/compress/func_crc32.rs:register]]

**Guaranteed:** a member's result and its errors depend only on its arguments, so they are
the same on every target. **Not guaranteed:** speed, and the shape of the generated code.

The helper bodies call only `bits` (inline native bit operations) and `collections` (list
reads and in-place appends to function-local lists).

## Source injection and size

Each helper is gated `HelperGate::WhenUsed` on the member names that need it. The gate
opens only when the program imports `compress` **and** references one of those members, so
a program pays only for the members it calls.
[[src/codegen/registry/mod.rs:HelperGate]] [[src/codegen/registry/mod.rs:references_any]]

A `WhenUsed` helper is injected as its own source file, not into the package's shared
companion. Each helper body therefore opens with its own `IMPORT compress`,
`IMPORT bits` and `IMPORT collections`; without them the build fails with
`SYMBOL_UNKNOWN_IMPORT` inside the injected file.
[[src/codegen/builtins/compress/helper_crc32.rs:BODY]]

Measured on macos-aarch64 (2026-09-14) with the `.ai/resources-packages.md` size probe:

| Program | Executable size |
|---|---|
| `IMPORT io`, one `io::print` | 66,600 B |
| the same plus `IMPORT compress`, no `compress` call | 66,604 B |
| `IMPORT io` + `IMPORT compress` + `IMPORT strings`, printing one `compress::crc32` | 83,116 B |

The 4 B difference is the build string that embeds the project name (`mfb.szpcompress`
vs `mfb.szpio` — the only printable string that differs between the two executables);
`IMPORT bits` and `IMPORT term`, which inject no source, measure 66,600 B under
same-length names. Importing `compress` without calling a member adds no code. One
`crc32` call adds 16,516 B — one 16,512 B block of the executable's size quantum plus
the same 4 B name difference — so the CRC-32 source costs at most one block.

## CRC-32

`compress::crc32(data, running)` computes **CRC-32/ISO-HDLC** — the CRC used by gzip,
zip and PNG — with the parameters of the CRC RevEng catalogue entry:

| Parameter | Value |
|---|---|
| width | 32 |
| polynomial | `0x04C11DB7`, processed reflected as `0xEDB88320` |
| init | `0xFFFFFFFF` |
| input and output reflected | yes |
| xorout | `0xFFFFFFFF` |
| check (`"123456789"`) | `0xCBF43926` = 3421780262 |

### `running`

On entry the register is `running XOR 0xFFFFFFFF`; on exit the result is
`register XOR 0xFFFFFFFF`. Because the returned value is a complete CRC and passing it
back as `running` restores the register exactly, the following hold for all byte lists
`a`, `b` and every valid `r`:

- `crc32(b, crc32(a)) = crc32(a` followed by `b)`;
- `crc32([], r) = r`;
- `running` defaults to `0`, which is `crc32([])`, so a default call starts a new CRC.

`running` must be in `0..4294967295`. Any other value raises `ErrInvalidArgument`
(77050002) with the message `compress::crc32: running must be 0..4294967295`. The check
runs before `data` is read, so it applies to empty `data` too.
[[src/codegen/builtins/compress/helper_crc32.rs:BODY]]

### Algorithm: slicing-by-8

The register and every intermediate are `Integer` values in `0..4294967295`; all bit
operations are `bits::bxor`, `bits::band` and `bits::sr`. With `b0..b7` the next eight
input bytes, while at least eight bytes remain:

```
x   = crc XOR (b0 + 256*b1 + 65536*b2 + 16777216*b3)
crc = T7[x AND 255] XOR T6[(x >> 8) AND 255] XOR T5[(x >> 16) AND 255] XOR T4[x >> 24]
      XOR T3[b4] XOR T2[b5] XOR T1[b6] XOR T0[b7]
```

The remaining 0–7 bytes take the byte-at-a-time step:

```
crc = (crc >> 8) XOR T0[(crc XOR b) AND 255]
```

`data` is only read; the loop writes nothing but scalar locals.
[[src/codegen/builtins/compress/helper_crc32.rs:BODY]]

### Tables

`T0..T7` are one module-level `List OF Integer`, `__COMPRESS_CRC32_TABLES`, of 2,048
entries with table `k` at indices `k*256 .. k*256+255`, so every lookup is a single
`collections::get`. `T0[i]` is `i` shifted right eight times, XORing in `0xEDB88320`
after each shift whose outgoing bit was 1; `T_k[i] = (T_(k-1)[i] >> 8) XOR T0[T_(k-1)[i] AND 255]`.
[[src/codegen/builtins/compress/helper_crc32_table.rs:BODY]]

The list is **computed once at program start** by `__compress_crc32Tables`, not written as
a literal. A list literal is lowered as per-element code in the global initializer — a
2,048-element literal took 53,317 of a 59,725-instruction test program, and a probe with the
literal was 379,772 B larger than the same probe with the builder at equal speed
(2026-09-14, macos-aarch64). The builder's one constant is pinned against the catalogue
polynomial by a unit test; every entry it produces is judged against independent CRC-32
implementations (see *Verification*).
[[src/codegen/builtins/compress/mod.rs:crc32_table_builder_uses_the_reflected_polynomial]]

### Verification

Two cross-checks share no code with this implementation:

- `tests/interop/rt_compress_interop.rs` (runs in `cargo test`) compares every length
  0–17 — each slicing-by-8 tail, with and without a preceding eight-byte step — and 200
  seeded random lengths up to 100,000 bytes, in one call and chained across a random split,
  against `flate2::Crc` (`crc32fast`). It also probes `running` at 4294967295, 4294967296
  and -1.
- `tools/oracles/compress` (offline) runs 118 cases up to 1 MiB against both Python's
  `zlib.crc32` and Node's `zlib.crc32`.

Random data over that corpus reaches every table index, so a wrong entry fails both.

## Decoding: `inflate`, `zlibDecode`, `gzipDecode`

The three decoders share one raw DEFLATE (RFC 1951) core; they differ only in the wrapper they read
around it. [[src/codegen/builtins/compress/helper_inflate_core.rs:BODY]]

| Member | Format | Wrapper checks | Trailer |
|---|---|---|---|
| `inflate(data, maxBytes)` | RFC 1951 | none | none |
| `zlibDecode(data, maxBytes, ignoreChecksum)` | RFC 1950 | `(CMF·256 + FLG) MOD 31 = 0`; `CM = 8`; `CINFO ≤ 7`; `FDICT` clear | 4-byte big-endian Adler-32 |
| `gzipDecode(data, maxBytes, ignoreChecksum)` | RFC 1952, every member | `1f 8b`; `CM = 8`; `FLG` bits 5–7 clear; `FEXTRA`/`FNAME`/`FCOMMENT` bounded; `FHCRC` | per member: little-endian `CRC32`, `ISIZE` |

[[src/codegen/builtins/compress/helper_zlib_frame.rs:BODY]] [[src/codegen/builtins/compress/helper_gzip_frame.rs:BODY]]

Every refusal of malformed input raises `ErrInvalidFormat` (`77050003`); output past `maxBytes` raises
`ErrTooLarge` (`77050027`); a negative `maxBytes` raises `ErrInvalidArgument` (`77050002`).
[[src/codegen/builtins/errorcode/mod.rs:register]]

### The core

`__compress_inflateCore(data, start, maxBytes)` holds all decoder state as locals of one function:
the input cursor, an LSB-first bit buffer of at most 56 bits, the final-block flag, the output list and
the current block's decode tables. Helpers are pure — they build tables and read tables; none takes the
output. The bit buffer is refilled a byte at a time to 49–56 bits whenever fewer than 48 remain, which
covers the longest symbol (a 15-bit length code, 5 extra bits, a 15-bit distance code and 13 extra bits),
and is operated with `bits::bor`/`sl`/`band`/`sr`.

- **Stored blocks** skip to a byte boundary, read `LEN` and `NLEN`, require `LEN + NLEN = 65535`, then copy
  `LEN` bytes.
- **Fixed blocks** use the RFC 1951 §3.2.6 lengths: literal/length codes 0–143 are 8 bits, 144–255 are 9,
  256–279 are 7, and 280–287 are 8; the distance table is **32** five-bit codes, as zlib's `fixedtables`,
  with symbols 30 and 31 refused when decoded. Both tables are built once per call.
- **Dynamic blocks** read `HLIT` (≤ 286), `HDIST` (≤ 30) and `HCLEN`, build the code-length table, expand
  the combined length sequence with repeat codes 16/17/18 (a 16 with no previous length, or any repeat
  past `HLIT + HDIST`, is refused), require a length for end-of-block (256), then build the two tables.

Literal and length/distance symbols are decoded until end-of-block. A back-reference copies byte by
byte from the output list itself, so a match that overlaps its own output works by construction. The
length base/extra and distance base/extra tables are built at program start from the RFC 1951 §3.2.5
formula and pinned against the RFC table by a unit test.
[[src/codegen/builtins/compress/helper_deflate_tables.rs:BODY]]
[[src/codegen/builtins/compress/mod.rs:deflate_tables_builder_matches_rfc1951]]

**Bounds.** `maxBytes` is checked before every write — before a literal, before a match copies, before
a stored block copies — so a stream that would exceed it raises `ErrTooLarge` having produced exactly
`maxBytes` bytes, never more. A back-reference to a distance larger than the output so far is refused.
The input cursor may run past the end by the bytes a refill pads with zeros; after every symbol, if any
of those padding bits were consumed, the stream is refused as ending mid-stream. Every loop advances the
input or the output, so no input can loop forever.

**End position.** The core returns its output with the byte position just past the DEFLATE data
appended as 8 little-endian bytes; the wrappers read it (the zlib trailer and the next gzip member start
there) and drop the 8 bytes with one copy. [[src/codegen/builtins/compress/helper_end_position.rs:BODY]]

### Decode tables and their validity rules

A table is built from a list of code lengths as RFC 1951 §3.2.2 canonical codes. It has two levels: a
primary table indexed by the next `root` bits (9 for literal/length, 6 for distance, 7 for code
lengths) whose entries are `symbol · 16 + length`, and, for codes longer than `root`, a sub-table
reached through a primary entry `1048576 + offset · 16 + subBits`. Unused slots hold −1 and are refused
when decoded. [[src/codegen/builtins/compress/helper_huffman_table.rs:BODY]]

The set of lengths is validated as zlib 1.2.12's `inftrees.c` `inflate_table` does, so `compress`
accepts exactly the code sets zlib accepts:

- **over-subscribed** (more codes than the lengths allow) — refused;
- **incomplete** — refused, except that a literal/length or distance set whose only code is **1 bit**
  long is accepted;
- **all lengths zero** — the table is built with only unused slots, so the block is accepted until a
  symbol is decoded from it, which is refused;
- the code-length code must always be complete.

Evidence (`tools/oracles/compress/probe.sh`, 2026-09-14, Python zlib 1.2.12 and Node zlib 1.3.1 agree on
every row): a single 1-bit distance code decodes whether or not it is used; a block with no distance
codes decodes when it uses none and fails with "invalid distance code" when it uses one; a lone 1-bit
end-of-block code decodes to nothing; three 2-bit literal codes fail with "invalid literal/lengths set".

### Checksums and the decided behaviours

- **`ignoreChecksum`** (`zlibDecode`, `gzipDecode`) skips every checksum comparison: the zlib Adler-32,
  each gzip `CRC32` and `ISIZE`, and a gzip header CRC-16 when `FHCRC` is set. With it `TRUE`, the
  checksums are not computed at all. The checksum bytes must still be present, and every structural
  check still applies. Adler-32 sums are reduced modulo 65521 every 5,552 bytes, as zlib's `NMAX`.
  [[src/codegen/builtins/compress/helper_adler32.rs:BODY]]
- **Preset dictionaries** are refused: a zlib header with `FDICT` set raises `ErrInvalidFormat`.
- **Bytes after the data are ignored**: `inflate` stops after the final block and `zlibDecode` after the
  Adler-32. `gzipDecode` decodes another member only while the remaining bytes begin `1f 8b`; anything
  else after the last member is ignored, and bytes that do begin `1f 8b` but are not a valid member are
  refused. This is the one deliberate difference from a judge: Node's `gunzipSync` and Python's
  `gzip.decompress` refuse such trailing bytes, while zlib's own stream decoder
  (`zlib.decompressobj(31)`) leaves them unread, as `compress` does.
- **`maxBytes` bounds the total** across gzip members: each member is decoded with the limit less what
  the earlier members produced.

### Why this shape

Both choices were measured on a 4096×4096 PNG's 67 MiB, literal-heavy zlib stream, in five interleaved
rounds (2026-09-14, macos-aarch64):

- **End position as an appended trailer** costs 1.1–2.2% over returning the output alone; a second,
  output-free pass to find the end costs 61–70%.
- **`bits::` shifts** for the bit buffer beat `*`/`/` and `MOD` by powers of two by 7–19%.

Decode time is linear in the output: 17.2–17.7 MiB/s from 1 MiB to 64 MiB of the same content at `-O1`,
with a constant 1.84–1.87 ms per dynamic block. A stream split into many tiny blocks costs about 36 µs
more per block — the per-block table build.

### Measured throughput

`tools/compress-bench/run.sh target/release/mfb`, 2026-09-14, macos-aarch64: Python zlib 1.2.12 level-6
output of three corpora (pseudo-random, repetitive text, all zeros) at 1, 4 and 16 MiB; the median of three
interleaved rounds of five in-process runs; MiB/s over the decompressed size. Every result equals Python's
and every 16 MiB / 4 MiB time ratio is 3.74–4.03.

| Member | corpus | `-O1` MiB/s (16 MiB) | `-O3` MiB/s (16 MiB) |
|---|---|---|---|
| `inflate` | random | 81.4 | 92.8 |
| `inflate` | text | 49.9 | 62.8 |
| `inflate` | zero | 69.5 | 84.8 |
| `zlibDecode` | random | 55.1 | 60.6 |
| `zlibDecode` | text | 38.6 | 46.1 |
| `zlibDecode` | zero | 48.7 | 56.0 |
| `gzipDecode` | random | 41.1 | 47.6 |
| `gzipDecode` | text | 31.4 | 38.6 |
| `gzipDecode` | zero | 38.3 | 46.1 |

`zlibDecode` and `gzipDecode` do more than `inflate` over the same DEFLATE data: they compute the
checksum of the whole output (Adler-32, or CRC-32 at ≈86 MiB/s) and copy the output once to drop the
core's end-position trailer. With `ignoreChecksum := TRUE` the checksum pass is skipped. Python's `zlib`
decodes the same streams at roughly 2,400–8,900 MiB/s. Speed is not part of the contract; these are
dated measurements.

Against the decoder it replaces: on a 4096×4096 RGBA8 PNG's single IDAT (Python `zlib.compress(raw, 6)`,
38,120,016 B → 67,112,960 B), `compress::zlibDecode` at `-O1` takes 3,847.0 ms (median of three,
16.64 MiB/s), where `canvas::loadImage` with canvas's own bit-at-a-time inflate took 43,691.6 ms
(1.47 MiB/s). The canvas figure covers the whole `loadImage` — unfiltering and RGBA conversion as well as
inflate — because no program outside an `--app` build reaches that inflate directly, so ≈11.4× overstates
the inflate-only difference.

### Verification

- `tools/oracles/compress/run.sh` (offline): `decode-raw`, `decode-zlib`, `decode-gzip` — Python zlib's
  output at every level and strategy over three corpora, multi-member gzip files and every optional gzip
  header field — against Python's and Node's zlib; and `mutate`, 600 seeded 1–3 byte edits of valid
  streams, which fails the run if `compress` accepts what both zlibs refuse, refuses what both accept,
  or produces different output.
- `tests/interop/rt_compress_interop.rs` (in `cargo test`): `flate2`'s output at every level and format;
  80 single-byte flips of zlib and gzip streams, each matching `flate2`'s verdict; the decided behaviours.
- `tests/runtime/rt_compress_bounds.rs`: a 64 MiB + 1 zero bomb refused at the limit within a
  derived memory ceiling; decode time linear in output size.
