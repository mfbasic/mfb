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
