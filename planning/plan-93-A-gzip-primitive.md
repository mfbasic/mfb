# plan-93-A: Native gzip/deflate primitive (`compress::` package)

Last updated: 2026-08-09
Overall Effort: x-large (1d–3d) — the whole plan-93 feature (gzip on client + server, JS MIME types, cookies on client + server)
Effort: large (3h–1d)
Depends on: nothing

This sub-plan adds the one piece the rest of plan-93's gzip work stands on: a
native primitive that inflates and deflates a `List OF Byte` in the gzip and raw
(HTTP `deflate`) wire formats. The `http::` package deliberately introduces **no**
native intrinsics of its own — "all protocol work is string manipulation; only the
transport branches reach native code" (`src/builtins/http.rs:5`). Real DEFLATE
(dynamic Huffman + a 32 KiB LZ77 window) is neither string manipulation nor
something to reimplement in MFBASIC for 64 MiB bodies, so the primitive lives in a
**sibling package** `compress::` that `http::` calls exactly the way it already
calls `net::`/`tls::`. Runtime helpers in this codebase are emitted as native code
that reaches system libraries by `dlopen`, not by linking a Rust crate — TLS does
this today via `emit_dlopen_libssl` for `libssl.so.3`
(`src/target/shared/code/tls/mod.rs:199`, `TLS_LIB_NAMES` at
`tls/mod.rs:75`). The primitive mirrors that pattern against system **zlib**
(`libz`), which the manifest already knows as a system library
(`src/manifest/libraries.rs:505` → `libz.so.1`).

The single behavioral outcome: a program can `IMPORT compress`, call
`compress::gzipDecode(bytes)` / `compress::gzipEncode(bytes)` (and the raw-deflate
pair), and get back the correct round-tripped bytes on Linux and macOS.

References:

- `src/builtins/http.rs:5` — the "http introduces no new intrinsics" contract this
  design honors by putting zlib in a sibling package.
- `src/target/shared/code/tls/mod.rs:199` (`emit_dlopen_libssl`), `tls/mod.rs:75`
  (`TLS_LIB_NAMES`) — the dlopen-a-system-lib precedent to mirror.
- `src/manifest/libraries.rs:505` — zlib already registered as a Linux system lib
  (`libz.so.1`); `src/target/shared/code/link_locator.rs:614` — zlib locator test.
- `src/builtins/net.rs` — the descriptor pattern (`NET_FUNCTIONS`, `Implementation`,
  `implementation_name`) a new `compress` package descriptor copies.
- `src/builtins/encoding_package.mfb` / `src/builtins/encoding.rs` — the
  byte↔codec package precedent for surface shape and man/spec wiring.
- `.ai/net-tls.md`, `.ai/resources-packages.md`, `.ai/build-tooling.md` — read
  before touching transport/FFI, package authoring, and vendor/link mechanics.

## Prerequisites

**These are the preconditions for the whole plan-93 feature. Every other letter
(B–F) points here.**

| Must be true | Command | Status |
|---|---|---|
| Working tree builds & tests green at start | `cargo test` | UNMEASURED — run before starting |
| System zlib is reachable by `dlopen` on the dev/CI targets | `ls /usr/lib/libz.dylib /lib/x86_64-linux-gnu/libz.so.1 2>/dev/null` | UNMEASURED |
| No half-landed plan-93 letter in flight | `ls planning/plan-93-*` → only this set | MET (this authoring) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update status before continuing and again before
> stopping. Everything below is written against the world where these hold.

## 1. Goal

- A new builtin package `compress` exposes four functions over `List OF Byte`:
  - `compress::gzipDecode(data AS List OF Byte) AS List OF Byte`
  - `compress::gzipEncode(data AS List OF Byte) AS List OF Byte`
  - `compress::inflate(data AS List OF Byte) AS List OF Byte` (raw DEFLATE — HTTP `deflate`)
  - `compress::deflate(data AS List OF Byte) AS List OF Byte`
- `gzipDecode(gzipEncode(x)) == x` and `inflate(deflate(x)) == x` for arbitrary
  binary `x`, verified by a runtime round-trip fixture on Linux and macOS.
- A cross-check fixture decodes a gzip blob produced by the host `gzip(1)` /
  `flate2`, proving wire-format compatibility with real-world encoders (not just
  self-consistency).
- Malformed input fails with a documented error code (a new `compress` error, e.g.
  `ErrInvalidFormat`-class), never a crash or silent truncation.

### Non-goals (explicit constraints)

- **No brotli.** Explicitly out of scope for all of plan-93 (user decision).
- **No new vendored Rust runtime crate.** The runtime reaches zlib by `dlopen` of
  the system library, exactly like TLS reaches openssl; `flate2`/`miniz_oxide`
  stay compiler-host-only (they build the AppImage today, not runtime output).
- **No change to `http::`'s "no new intrinsics" contract.** The intrinsics land in
  `compress::`, and `http::` (in B–C) calls them as a sibling package.
- **Windows gzip is deferred** (see Open Decisions) — this sub-plan targets Linux
  + macOS, where `libz` ships with the OS. Windows is not silently "supported".
- No streaming/one-shot-only API here; `http`'s 64 MiB cap already bounds inputs.

## 2. Current State

`http::` is a pure-MFBASIC HTTP/1.1 client + server; its only native reach is the
`net::`/`tls::` transport branches (spec `src/docs/spec/stdlib/05_http.md:6-11`).
There is no compression anywhere in the package:

### Measured populations

| What | Count | Command |
|---|---|---|
| gzip/deflate/inflate mentions in the http package | 0 | `grep -ciE 'gzip\|deflate\|inflate' src/builtins/http_package.mfb → 0` |
| content-encoding handling in the http package | 0 | `grep -ci 'content-encoding' src/builtins/http_package.mfb → 0` |
| builtin `.mfb` packages today | 23 | `ls src/builtins/*.mfb | wc -l → 23` |
| zlib system-lib entries in the manifest | 1 (linux `libz.so.1`) | `grep -c 'libz.so' src/manifest/libraries.rs` |

### Verified properties

- **The runtime reaches system libs by dlopen, not by linking a Rust crate.** Read
  `src/target/shared/code/tls/mod.rs:199` (`emit_dlopen_libssl`) and `tls/mod.rs:75`
  (`TLS_LIB_NAMES = ["libssl.so.3","libssl.so.1.1"]`): TLS emits code that opens
  the versioned system `.so`/`.dylib` at runtime and resolves symbols. This is the
  template for opening `libz` and resolving `inflateInit2_`/`inflate`/`inflateEnd`
  and the deflate trio. **VERIFIED** by reading the tls lowering.
- **`flate2` is host-only.** `grep -rn flate2 src --include=*.rs` shows use only in
  the AppImage/squashfs build path (`src/os/linux/appimage/…`), never in emitted
  runtime code. So the crate cannot be called from an mfb output binary — zlib
  FFI is the only native route. **VERIFIED** by grep + reading the appimage use.
- **zlib is a known system library, not yet wired to a builtin.** `libraries.rs`
  and `link_locator.rs` reference `zlib`/`libz.so.1` for user `LINK "zlib"`
  programs, but no builtin package resolves it. **VERIFIED** by reading
  `libraries.rs:505` and the `link_locator.rs:614` test.
- **zlib window-bits select the format.** `inflateInit2(strm, 15+16)` = gzip,
  `+32` = auto gzip/zlib, `-15` = raw DEFLATE; `deflateInit2` symmetric. This is
  how one primitive serves both `gzipDecode` and raw `inflate`. **VERIFIED**
  against the zlib manual (cite in the man page).

## 3. Design Overview

Three layers, mirroring an existing native package (`net`):

1. **Descriptor** — new `src/builtins/compress.rs` declaring `COMPRESS_FUNCTIONS`
   (four functions, all `List OF Byte -> List OF Byte`) and their
   `implementation_name` mapping to runtime helpers `compress.gzipDecode` etc.,
   copied structurally from `NET`/`net.rs`. Register the package so `IMPORT
   compress` resolves. **This is the low-risk half.**
2. **Runtime lowering** — under `src/target/shared/code/`, a `compress` module that
   emits, per architecture, the `dlopen("libz…")` + symbol-resolve + call sequence
   for `inflate`/`deflate` (bounded one-shot: init2 → loop inflate/deflate over the
   `List OF Byte` buffer → end), returning a freshly allocated `List OF Byte`. This
   reuses the tls dlopen scaffolding shape. **This is where the correctness and
   portability risk concentrates** — FFI struct layout of `z_stream`, per-arch ABI,
   and error/EOF handling.
3. **Package glue** — a thin `src/builtins/compress_package.mfb` only if any part
   of the surface is expressible in MFBASIC (e.g. argument validation); the four
   codec calls themselves are native, so this may be empty/minimal.

**Design uncertainty (schedule FIRST):** does the dlopen-zlib approach round-trip
correctly across Linux and macOS with the emitted `z_stream` FFI? Phase 1 proves
exactly that with the smallest possible native call before any surface polish.

**Correctness risk (schedule behind tests):** the `z_stream` struct layout and the
inflate/deflate loop (handling `Z_BUF_ERROR`, `Z_STREAM_END`, output-buffer growth).

This plan's gate is **runtime round-trip behavior**, NOT byte-identity — the whole
point is new observable output, so byte-identity is the wrong gate here. The
`.ncode`/objdump of unrelated fixtures MUST stay byte-identical (no unintended
codegen change); any diff there is a bug to root-cause, not the design working.

### Rejected alternatives

- **flate2 linked into runtime** — impossible without a Rust runtime crate this
  compiler doesn't have; runtime reaches libs by dlopen only. Rejected on §2 fact.
- **Pure-MFBASIC inflate/deflate in `http::`** — keeps zero native deps and matches
  http's ethos, but a correct dynamic-Huffman inflater + a valid DEFLATE encoder in
  MFBASIC is large and slow at 64 MiB, and encode-correctness (valid framing) is a
  real hazard. Kept as the Windows fallback candidate (Open Decisions), rejected as
  the primary path on effort/perf grounds.
- **Extend `encoding::` instead of a new package** — `encoding` is deliberately
  pure-MFBASIC (`encoding_package.mfb` header); adding dlopen intrinsics there
  changes its nature and its docs' promise. A dedicated `compress::` is more
  discoverable and keeps `encoding` pure. Rejected.

## Compatibility / Format Impact

- **New** public package `compress` (new surface; nothing existing changes).
- **New** runtime link dependency on system `libz` — resolved by dlopen at run
  time, so no build-time link requirement on programs that never import
  `compress`. Programs that do import it gain a soft runtime dependency on the OS
  zlib (present by default on Linux/macOS).
- No change to `http::`, `net::`, `tls::`, or any wire format in this sub-plan.

## Phases

> Keep checkboxes current in the same commit as the work; fill `Commit:` when each
> lands. An unticked box means NOT DONE.

### Phase 1 — Prove dlopen-zlib round-trips (falsify the premise cheaply)

Smallest end-to-end native call before any surface work.

- [ ] Add a runtime lowering module `src/target/shared/code/compress/mod.rs` that
      emits `dlopen`+resolve+call for zlib `inflateInit2_`/`inflate`/`inflateEnd`
      and the deflate trio, modeled on `emit_dlopen_libssl`
      (`src/target/shared/code/tls/mod.rs:199`). One helper each: `compress.gzipEncode`,
      `compress.gzipDecode` (window-bits 15+16 / 15+32).
- [ ] Wire a temporary/internal call path (or a minimal descriptor entry) so a
      test program can invoke `gzipEncode`→`gzipDecode`.
- [ ] Tests: an rt-behavior fixture `tests/rt-behavior/compress/gzip-roundtrip-rt`
      that encodes then decodes a known buffer and asserts equality; run on Linux
      and macOS per the acceptance harness.

Acceptance: the round-trip fixture returns the original bytes on both Linux and
macOS. If it does not, root-cause the `z_stream` FFI (objdump one fixture) — this
is the premise-proving phase, not a stop.
Commit: —

### Phase 2 — `compress` package descriptor + full surface

- [ ] Add `src/builtins/compress.rs` with `COMPRESS_FUNCTIONS` (all four
      functions) and `implementation_name` mapping, copied structurally from
      `src/builtins/net.rs`; register the module so `IMPORT compress` resolves.
- [ ] Add raw `inflate`/`deflate` (window-bits ∓15) lowering to the Phase-1 module.
- [ ] Add a `compress` error code (ErrInvalidFormat-class) for malformed input;
      return it from the inflate path on `Z_DATA_ERROR`.
- [ ] Tests: extend the fixture set with `inflate(deflate(x))==x`, an
      empty-input case, and a malformed-input case asserting the error code.

Acceptance: all four functions round-trip via `IMPORT compress` from a normal
program; malformed input yields the documented error, not a crash.
Commit: —

### Phase 3 — Wire-format cross-check + docs

- [ ] Tests: a fixture that decodes a gzip blob produced by the host (`gzip(1)` or
      `flate2` in a build step / checked-in golden) and asserts the plaintext —
      proves compatibility with third-party encoders, not just self-consistency.
- [ ] Man pages: `src/docs/man/builtins/compress/{gzipEncode,gzipDecode,inflate,deflate,package,types}.md`
      per `.ai/man_template.md` / `.ai/man_package_template.md`, driven by
      `scripts/update_man.sh` / `scripts/update_man_package.sh`.
- [ ] Spec: new `src/docs/spec/stdlib/NN_compress.md` (window-bits/format table,
      error model, the dlopen-zlib note) per `.ai/specifications.md`; link it from
      the stdlib index and from the http spec's forthcoming encoding section.

Acceptance: `mfb man compress gzipDecode` and `mfb spec stdlib compress` render;
the host-encoded cross-check fixture decodes correctly; `cargo test` green.
Commit: —

## Validation Plan

- Tests: rt-behavior round-trip + cross-check fixtures under
  `tests/rt-behavior/compress/`; descriptor/parity unit tests in `compress.rs`
  mirroring `net.rs`'s `parity_matches_descriptor` and dispatch tests.
- Coverage check: confirm the new `compress` fixtures are in the acceptance
  harness denominator (the harness picks up `tests/rt-behavior/**`); a green run
  must include them, not skip on a missing-lib gate.
- Runtime proof: a small program that gzip-encodes a string, prints the byte
  length, decodes, and prints the recovered string — run on Linux and macOS.
- Doc sync: new man pages + new stdlib spec page + stdlib index link.
- Acceptance: full `cargo test`; the acceptance golden harness on Linux + macOS.

## Open Decisions

- **Windows gzip** — recommend: **defer** (document `compress::` as Linux+macOS in
  this sub-plan; add Windows later via bundled zlib or a pure-MFBASIC inflate
  fallback). Alternative: block plan-93 on a Windows zlib story now. The client
  (plan-93-C) must therefore treat "no decoder available" as *keep the encoded
  body + leave `content-encoding` intact*, never a crash — so Windows degrades
  gracefully rather than failing requests. (§3, §Non-goals)
- **Package home** — recommend a dedicated `compress` package (chosen above) over
  extending `encoding`. (§Rejected alternatives)

## Corrections

<Filled in during execution.>

## Summary

The engineering risk is entirely in Phase 1: proving the emitted `z_stream` dlopen
FFI round-trips across Linux and macOS. Everything after it (descriptor, docs,
cross-check) is low-risk package plumbing. Windows and pure-MFBASIC fallback are
consciously deferred, not silently assumed. `http::` is untouched here; B and C
consume this package.
