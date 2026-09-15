# plan-137-A: `compress::` builtin package + `crc32`

Last updated: 2026-09-13
Overall Effort: huge (>3d) — the whole plan-137 feature: a library-free, in-memory `compress::` (gzip / zlib / raw DEFLATE, both directions, plus `crc32`), and canvas's PNG decoder moved onto it
Effort: medium (1h–2h)
Depends on: nothing inside plan-137 (see Prerequisites for the whole-feature gate)

plan-137 adds a builtin `compress::` package written entirely in MFBASIC — no system zlib,
no `dlopen`, no vendored C — so it produces the same bytes and the same errors on every
target, Windows included. It is one-shot and in-memory: every function takes a whole
`List OF Byte` and returns a whole `List OF Byte`. It replaces the archived dlopen-zlib
design (`planning/completed/plan-93-A-gzip-primitive.md`, marked "not to be used").

**Correctness over speed, but speed matters.** Every letter is checked against real zlib as
the oracle (zlib must decode what we encode; we must decode what zlib encodes, at every
level; we must refuse what zlib refuses), and every letter records measured throughput.
Our compressed bytes are not required to equal zlib's.

This letter lands the package itself — registration, docs surfaces, test homes, the oracle
and benchmark harnesses — with its first real member, `compress::crc32`. Behavioural
outcome: `compress::crc32(strings::toBytes("123456789"))` returns the CRC-32/ISO-HDLC check
value, and `crc32(b, crc32(a)) = crc32(a & b)` for any split.

The feature, letter by letter (letter order = implementation order):

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | package skeleton, `crc32`, oracle + bench harnesses | medium |
| **B** | table-driven inflate; `inflate`, `zlibDecode`, `gzipDecode`; `ErrTooLarge`; Adler-32 | large |
| **C** | canvas's PNG decoder moves onto `compress::` inflate; `helper_inflate.rs` deleted | medium |
| **D** | `deflate`, `zlibEncode`, `gzipEncode`: level 0 stored, levels 1–9 hash-chain LZ77 + fixed Huffman | large |
| **E** | dynamic Huffman (length-limited codes), lazy matching, per-block type choice; whole-feature validation | large |

References:

- RFC 1950 (zlib format), RFC 1951 (DEFLATE), RFC 1952 (gzip) — fetch the texts; cite
  sections in the spec page.
- The CRC-32/ISO-HDLC entry of the CRC RevEng catalogue (reflected polynomial `0xEDB88320`,
  init and xorout `0xFFFFFFFF`, check value for `"123456789"`) — **fetch it; do not recite
  the check value from memory**.
- `.ai/resources-packages.md` — "New builtin-package registration seams" and "A pure-source
  package's WHOLE companion is compiled into every importing binary".
- `.ai/collections.md` — in-place `append`/`set` only for a local of the function doing the
  write; fixed-width lists are entry-free.
- `.ai/testing-gates.md` — "Where an oracle lives", fixture/golden procedure, gates.
- `.ai/man-content.md`, `.ai/spec-content.md`, `.ai/specifications.md`.
- Precedents to mirror: `src/codegen/builtins/crypto/` (MFBASIC-source builtin with `bits::`
  hot loops, `RegistryHelper`, `Body::Rewrite`), `tools/oracles/crypto/` (differential
  runner), `tests/interop/rt_crypto_aead_interop.rs` (in-CI independent implementation),
  `tests/runtime/rt_json_bounds.rs` (peak-RSS / time bounds).
- `planning/todo.md` "Proposed API: `compress::`" — the surface this plan implements, minus
  `ErrUnavailable` (there is no library to be unavailable).

## Prerequisites

These gate the **whole** plan-137 feature. Letters B–E point here.

| Must be true | Command | Status |
|---|---|---|
| bug-621 fixed (append growth reserves data per element width) — B's decoder and D/E's encoders build multi-MiB outputs by in-place append; with bug-621 open a 64 MiB decode reserves ~19 B per output byte and every RSS bound in this plan would pin the bug | `ls bugs/completed/bug-621-*` → one file | MET (2026-09-14 re-run after merging main into `worktree-P-137` at `55dc9a626`: `bugs/completed/bug-621-append-growth-over-reserves-data-capacity.md`, `Status: Fixed`) |
| Release compiler builds | `cargo build --release --bin mfb` → exit 0 | MET (2026-09-14 re-run in `.claude/worktrees/P-137` at `55dc9a626`: exit 0, `Finished release profile [optimized] target(s) in 1m 28s`) |
| zlib oracles present on the dev host | `python3 -c "import zlib; print(zlib.ZLIB_RUNTIME_VERSION)"` → a version; `node -e "console.log(process.versions.zlib)"` → a version | MET (2026-09-14 re-run: zlib `1.2.12` / Node zlib `1.3.1-470d3a2`) |
| `flate2` resolves offline for the in-CI interop test | `grep -n -A3 '^name = "flate2"' Cargo.lock` → `1.1.9`, deps `crc32fast`, `miniz_oxide 0.8.9` | MET (2026-09-14 re-run: `Cargo.lock:1142 version = "1.1.9"`) |
| No `compress` package exists yet | `ls src/codegen/builtins \| grep -c compress` → 0 | MET (2026-09-14 re-run: `0`; `ls packages \| grep -c '^compress$'` → `0`) |

If bug-621 is not complete, plan-137 cannot start, full stop. This plan does not work around
it (no pre-sized output buffers invented to dodge the growth policy, no "smaller cap until
it lands").

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before you
> decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only the one that
> blocked you.

## 1. Goal

- `IMPORT compress` resolves; `compress::crc32(data AS List OF Byte, running AS Integer = 0) AS Integer`
  returns the CRC-32/ISO-HDLC of `data` continued from `running`, in `0..4294967295`.
- Chaining holds: `crc32(b, crc32(a)) = crc32(collections::append(a, b))`, and
  `crc32([], r) = r`.
- `running` outside `0..4294967295` raises `ErrInvalidArgument`.
- The result agrees with Python `zlib.crc32` and Node `zlib.crc32` on a generated corpus
  (tools oracle) and with `crc32fast` (through `flate2`'s dependency tree, in CI).
- `mfb man compress` and `mfb man compress crc32` render; `mfb spec stdlib compress` exists.
- The oracle harness (`tools/oracles/compress/`) and bench harness (`tools/compress-bench/`)
  exist with a `crc32` mode each, so B–E only add modes.

### Non-goals (explicit constraints)

- **No native code.** No `Body::abi_function`/`abi_inline` member, no runtime-helper family,
  no `dlopen`, no vendored library. Every member is MFBASIC source (`Body::Rewrite`).
- **No streaming API.** No stateful encoder/decoder object in any letter.
- **No new error for "unavailable".** `ErrUnavailable` from `planning/todo.md` is dropped.
- **No change to any other package's surface or codegen.** A new package's injected source
  must not move another package's `.ncodesum` (`scripts/artifact-gate.sh target/release/mfb all`
  shows diffs only under `tests/byte-identity/compress/`).
- **No size cost for programs that do not use a member.** Helpers are gated so `IMPORT compress`
  plus a `crc32` call does not compile inflate/deflate bodies (see §4.3).
- **Out of scope for all of plan-137:** file-backed zip (needs `fs` seek + read-N), `zip::`/`tar::`
  packages, HTTP `Content-Encoding` (consumer, later), brotli/zstd, preset dictionaries.

## 2. Current State

- **No compression code in the compiler's output today except canvas's private inflate.**
  `src/codegen/builtins/canvas/helper_inflate.rs` (460 lines, `wc -l`) defines
  `__canvas_inflate(data, start, limit)` and `__canvas_zlibInflate(data, limit)` in MFBASIC
  source; its only call site is `__canvas_pngDecode` in
  `src/codegen/builtins/canvas/helper_png.rs` (518 lines)
  (`grep -rn "__canvas_zlibInflate(" src` → the definition and one call). Its own header says
  it is deliberately a reference implementation — a bit at a time, `__canvas_pow2` loops, no
  lookup tables. It checks the zlib header but **never verifies Adler-32**, and **accepts
  over-subscribed Huffman trees** (`planning/completed/audit-3-decoders.md`, DEC-57 and DEC-58).
  It never raises; failure returns `[]`, which `__canvas_loadImage`
  (`canvas/func_load_image.rs`) turns into `ErrBadImageFile`. Letter C replaces it.
- **No CRC-32 or Adler-32 in `src/`.** `grep -rni "crc32\|adler" src` hits only a linker-info
  test string in `src/cli/info/tests.rs`. The only implementations in the tree are the Rust
  test helpers `crc32`/`adler32` in `tests/canvas/rt_canvas_image_decode.rs`.
- **The MFBASIC-source builtin precedent is `crypto::`** — 249 `helper_*.rs` files, 8157 lines
  (`ls src/codegen/builtins/crypto/helper_*.rs | wc -l`; `cat … | wc -l`). SHA-2/AES are MFB
  `WHILE` loops over `bits::` and `collections::get`; module-level tables such as
  `__CRYPTO_K256` are read with `collections::get`. Crypto declares
  `pkg.add_imports(vec![...])` (`crypto/mod.rs:register`) and gates big blocks with
  `HelperGate::WhenUsed` (argon2id).
- **`bits::` is inline native code.** All 17 members are `Body::abi_inline`
  (`src/codegen/builtins/bits/func_*.rs`); `band`/`bor`/`bxor` are one instruction; `sl`/`sr`
  add two compare-and-branch pairs on the count even when it is a constant (`bits/func_sl.rs`).
  MFBASIC has **no** integer bitwise operators (`src/docs/spec/language/11_operators.md`:
  "the operator set intentionally omits integer bitwise operations"); `/` on two Integers is
  integer division, `DIV` is Float.
- **Registration seams** (`.ai/resources-packages.md`, "New builtin-package registration seams";
  confirmed against the `big` and `color` commits `ed55277cb`, `b771d7f33`):
  `src/codegen/builtins/compress/mod.rs` `register`; `pub(crate) mod compress;`, `BUILTIN_IMPORTS`
  and `ARGUMENT_CHECKED_PACKAGES` in `src/codegen/builtins/mod.rs`;
  `crate::codegen::builtins::compress::register(&mut r)` in `registry::build()`
  (`src/codegen/registry/mod.rs`); the §18 package sentence in
  `src/docs/spec/language/18_builtin-functions.md` (pinned by
  `spec_section_18_package_list_matches_is_builtin_import`). `compress` exports no types, so
  `src/resolver/mod.rs` `BUILTIN_TYPES` is untouched.
- **Errors are global `errorCode::` constants**, not package symbols: rows in
  `src/codegen/builtins/errorcode/mod.rs` plus the "Constant Registry" table in
  `src/docs/spec/diagnostics/02_error-codes.md` (`table_matches_registry` pins the pair).
  Source raises with a literal: `FAIL error(77050002, "…")` (crypto: `grep -rn "FAIL error(" src/codegen/builtins/crypto`).
  `ErrInvalidArgument` = 77050002 and `ErrInvalidFormat` = 77050003 exist.
- **Docs surfaces.** `mfb man` renders from descriptor fields (`src/cli/man.rs`); spec pages are
  `src/docs/spec/stdlib/NN_<pkg>.md`, discovered from the tree (`build.rs`), highest today
  `19_big.md` (`ls src/docs/spec/stdlib`), with a hand-kept reading-order bullet list in
  `src/docs/spec/stdlib/spec.md`.
- **Oracle homes** (`.ai/testing-gates.md` "Where an oracle lives"): a builtin's differential
  runner goes under `tools/oracles/<area>/` (crypto: `tools/oracles/crypto/{hash,argon2id,mac-kdf,keys}/run.sh`
  on `_lib/harness.sh`, exit 0 agree / 1 disagree / 2 harness failure, not in CI); an
  independent implementation small enough to live in a test file runs in CI
  (`tests/interop/rt_crypto_*_interop.rs`, `[[test]]` entries in `Cargo.toml`, dev-deps
  justified by "already in the lockfile … adds no new compiled code").
- **Mutate mode precedent:** `packages/yaml/oracle/diff.mjs` `mutate`/`modeMutate`,
  `packages/mustache/oracle/diff.mjs`; `packages/jwt/oracle/run.sh`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Existing `compress` package | 0 | `ls src/codegen/builtins \| grep -c compress` |
| Inflate implementations in `src/` | 1 (canvas-private) | `grep -rln "FUNC __canvas_inflate" src` |
| Call sites of `__canvas_zlibInflate` outside its file | 1 | `grep -rn "__canvas_zlibInflate(" src \| grep -v helper_inflate.rs` |
| Canvas decode tests that must stay green (letter C) | 15 | `grep -c "#\[test\]" tests/canvas/rt_canvas_image_decode.rs` |
| CRC-32 / Adler-32 implementations in `src/` | 0 | `grep -rni "crc32\|adler" src` → only `src/cli/info/tests.rs` |
| Highest `7705` error code | 77050026 (`ErrCanvasGroupLimit`) | `grep -o 'constant("Err[A-Za-z]*", "7705[0-9]*"' src/codegen/builtins/errorcode/mod.rs \| sort -t'"' -k4 \| tail -1` |
| Highest stdlib spec page | `19_big.md` | `ls src/docs/spec/stdlib` |
| Byte-identity fixture directories (a new package gets codegen coverage only by adding one) | 27 | `ls -d tests/byte-identity/*/ \| wc -l` (2026-09-13) |
| MFB SHA-256 throughput, `-O1` / `-O3` (the nearest existing MFB byte-crunching number) | 65,536 B in 19.701 ms / 7.910 ms median (≈3.2 / ≈7.9 MiB/s, derived) | `benchmark/baseline/mfb-O1.log`, `mfb-O3.log`, row `crypto sha256`; workload `benchmark/mfb/src/crypto.mfb` |
| MFB per-byte loop (read with `getOr`, split with `/` and `MOD`, append to a local), 16 MiB, `-O1`, macos-aarch64 | +0.74 s user over the build-only program (≈22 MiB/s, derived) | `/tmp/cbench/{a,b}` probes, `/usr/bin/time -l`, 2026-09-13 |
| Canvas inflate throughput | UNMEASURED | measured first in plan-137-B Phase 1 |

### Verified properties

- **Reads of a list parameter are free; writes to anything but a same-function local copy.**
  Verified by `.ai/collections.md` measurements (20,000 writes into a 200,000-byte list: 5 ms
  local vs 1,179 ms through a helper) — every hot loop in this plan owns its output buffer as
  a local and keeps table *writes* local too.
- **A module-level `LET` table is readable from helper source.** Verified by `__CRYPTO_K256`
  (`grep -rn "__CRYPTO_K256" src/codegen/builtins/crypto | head -2`).
- **Helper gating by member exists.** Verified by crypto's argon2id block
  (`grep -n "WhenUsed" src/codegen/builtins/crypto/mod.rs`).
- **A builtin whose injected source imports another builtin needs a late injection pass.**
  Verified by reading `src/ir/lower.rs` (the `color::augmented_project` call with its
  plan-122-B comment) and `synthetic_files` in `src/codegen/registry/mod.rs` (the
  `encoding`/`net`/`http`/`color` skips). `bits`/`collections`/`strings` have empty or
  parse-time companions, so `compress` importing them needs no late pass; **canvas importing
  `compress` does** (letter C).
- **A `Body::Rewrite` member taking `List OF Byte` and returning `Integer` is called with
  no list copy on entry.** Verified 2026-09-14 (Phase 2): `mfb build --ncode` of
  `tests/byte-identity/compress` (macos-aarch64); every `bl _mfb_ifn_compress_5Fcrc32` in
  `_mfb_fn_main` is preceded only by `ldr_u64 x8, [sp, slot]` / `mov x0, x8` (the list
  pointer) and the `running` load into `x1`; the callee's entry stores `x0`/`x1` into its
  frame, and its first `bl` (`_mfb_arena_alloc`) is the `FAIL error(77050002, …)` record on
  the out-of-range-`running` branch, not a copy of `data`.

## 3. Design Overview (whole feature)

Layers, bottom up:

1. **Checksums** — `crc32` (public, A), Adler-32 (private, B).
2. **Bit I/O + Huffman** — an LSB-first bit reader (B) and bit writer (D); canonical-code
   construction and validation shared by decoder (B) and encoder (E).
3. **Inflate core** (B) — one function owns the input cursor, bit buffer, output buffer and
   block state; raw DEFLATE only.
4. **Framing** — zlib (RFC 1950) and gzip (RFC 1952) headers/trailers around the core
   (decode B, encode D).
5. **Deflate core** (D, E) — LZ77 match finder + block emitter.
6. **Consumers** — canvas PNG (C). HTTP and `zip::` later, outside this plan.

**Design uncertainty (schedule first):** *throughput of MFBASIC decoding.* The only MFB
byte-crunching number (SHA-256, ≈3.2 MiB/s at `-O1`, derived) says a naive decoder could make
a 64 MiB default `maxBytes` take tens of seconds. plan-137-B's first phase therefore
measures canvas's inflate and a table-driven prototype on the same stream before building the
public surface, and records the numbers. There is **no speed floor that stops the plan**: the
gates are linear scaling and "faster than the code it replaces" (canvas), with MiB/s recorded
per letter so a later performance plan has a baseline. Correctness never trades for speed.

**Correctness risk (behind tests):**

- Decoder leniency — accepting a stream zlib refuses (over-subscribed or incomplete codes,
  distances before the window start, bad `NLEN`, bad trailers). Guarded by the mutate-mode
  oracle in B and an in-CI tampered-stream test.
- Encoder validity — every emitted stream must decode with zlib at every level, including
  edge sizes (0 bytes, 1 byte, 65,535/65,536 stored boundaries, 258-byte matches, 32,768
  distances). Guarded by zlib-decodes-ours in D/E.
- Canvas regression (C) — the 15 decode tests plus new refusals.

**Byte-identity is not this plan's correctness gate.** It is a new package whose output is
behaviour. Byte-identity is used in one narrow way: no *other* package's `.ncodesum` may move
in A, B, D, E (`scripts/artifact-gate.sh target/release/mfb all` diffs only under
`tests/byte-identity/compress/`). Letter C is **expected** to shift canvas-importing
`.ir`/`.ast` goldens (canvas has no byte-identity fixture: `.ai/testing-gates.md`); a diff
there is the plan working. Any unexpected diff is a bug hunt — inspect one fixture — never
proof the design is dead.

**Rejected alternatives** (do not re-litigate):

- *dlopen system zlib* (plan-93-A) — Windows has no system zlib; Linux minimal images may
  not; output would differ by distro (zlib vs zlib-ng). Rejected by the user 2026-09-13.
- *Hand-written native assembly per backend* — five backends × a decoder and an encoder;
  rejected for cost and for violating "same bytes everywhere" by construction.
- *Streaming API now* — needs decoder state to survive calls; in MFB that is a record threaded
  through calls (copies) or a native resource. Deferred; B keeps state in one place so it can
  be added later.
- *Keep canvas's own inflate* — two decoders, two bug surfaces, and canvas's is the lenient
  one (DEC-57/58). Rejected (`planning/todo.md` "Design question to fold into plan-93-A").

## 4. Detailed Design (this letter)

### 4.1 Package skeleton

`src/codegen/builtins/compress/mod.rs`: `RegistryPackage::new("compress", MODULE_INTRO, MODULE_DESC)`,
`pkg.add_imports(vec!["compress", "bits", "collections"])`, a `COMPRESS` citation anchor like
crypto's, `register` calls per file, and an in-module `#[cfg(test)]` block counting members
(crypto's `assert_eq!(pkg.functions().len(), …)` pattern). Files follow the crypto naming:
`func_crc32.rs`, `helper_crc32.rs`, `helper_crc32_table.rs`.

### 4.2 `crc32`

- Reflected CRC with **slicing-by-8** (decided 2026-09-13). While at least 8 bytes remain, fold
  eight bytes per step: `x = bxor(crc, b0 + 256*b1 + 65536*b2 + 16777216*b3)`, then
  `crc = T7[x & 255] ^ T6[(x >> 8) & 255] ^ T5[(x >> 16) & 255] ^ T4[x >> 24] ^ T3[b4] ^ T2[b5] ^ T1[b6] ^ T0[b7]`
  (written with `bits::bxor` / `band` / `sr`). The remaining 0–7 bytes use the byte step
  `crc = bxor(sr(crc, 8), T0[band(bxor(crc, byte), 255)])`. `crc` starts as
  `bxor(running, 0xFFFFFFFF)`; the result is `bxor(crc, 0xFFFFFFFF)`.
- `T0..T7` live in one module-level `LET __COMPRESS_CRC32_TABLES AS List OF Integer = __compress_crc32Tables()` of
  2,048 entries (`T_k[i]` at index `k*256 + i`), so each lookup is one `collections::get` at a
  computed index. `T0` is the standard table for `0xEDB88320`;
  `T_k[i] = bxor(sr(T_(k-1)[i], 8), T0[band(T_(k-1)[i], 255)])`. **Corrected 2026-09-14:** the
  tables are **computed at program start** by an MFB builder, not written as a 2,048-element
  literal — the literal cost +379,772 B per binary for no speed gain (Corrections). The builder's
  one constant is pinned by a `#[cfg(test)]` test that derives it from the catalogue polynomial;
  every entry is judged against independent CRC-32 implementations by the interop test and the
  oracle, over random data that reaches every index.
- One function owns the loop; the input list is only read. Because slicing-by-8 has two code
  paths (8-byte steps and the tail), every test set includes each length 0–17, so every tail
  length is exercised with and without a preceding 8-byte step.
- `running < 0 OR running > 4294967295` → `FAIL error(77050002, "compress::crc32: running must be 0..4294967295")`.

### 4.3 Size gating

Every `compress` helper is a `RegistryHelper` gated `HelperGate::WhenUsed(&[<members>])` naming
the members that need it (crc32's table/loop: `crc32`, and from B on `gzipDecode`/`gzipEncode`).
Verify with the `.ai/resources-packages.md` size probe: `IMPORT io` + `IMPORT compress` with no
call must equal the `IMPORT io` baseline, and each letter records the delta of a one-member
program.

### 4.4 Oracle and bench harnesses (scaffolding used by every letter)

- `tools/oracles/compress/` — `README.md`, `run.sh [mfb] [modes…]` sourcing
  `tools/oracles/crypto/_lib/harness.sh` (exit 0/1/2, declared expected case counts),
  `mfb/` (a probe program that reads a job file, runs each case, prints one result line per
  case), `python/oracle.py` (stdlib `zlib`/`gzip` only), `node/oracle.mjs` (built-in
  `node:zlib` only, no npm install). The probe and the judges **share no case table**: the
  generator writes a job file, both sides read it, `run.sh` diffs the results. Modes added per
  letter; A adds `crc32` (random lengths 0..1 MiB, random split points for chaining).
- `tools/compress-bench/` — `README.md`, `run.sh <mfb>`: generates a fixed corpus (seeded
  pseudo-random, repetitive text, all-zero; 1 / 4 / 16 MiB), builds an MFB bench program,
  times each op by median of 5 runs with `/usr/bin/time` (**corrected:** in-process `datetime::monotonicNanos` around the op, interleaved rounds — Corrections), and prints MiB/s beside Python
  `zlib` on the same bytes. A adds the `crc32` row.

## Compatibility / Format Impact

- New builtin import name `compress` (a user package named `compress` would now collide the
  way one named `crypto` does — no package in `packages/` uses the name:
  `ls packages | grep -c '^compress$'` → 0 on 2026-09-13; Phase 1 re-checks).
- New public function `compress::crc32`. No existing surface, format or ABI changes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the
> work; `- [~]` for partial with what remains; mark moot tasks `- [x] ~~text~~ — moot: <evidence>`;
> fill `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — re-verify the gate and the numbering

- [x] Re-run every Prerequisites command; update statuses. (2026-09-14: all five rows MET, outputs in the table.)
- [x] `ls packages | grep -c '^compress$'` → 0; `grep -rn '"compress"' src/codegen/builtins/mod.rs` → no hits. (2026-09-14: `0`; grep printed nothing.)
- [x] Record the spec page number: next free `NN` in `ls src/docs/spec/stdlib` (20 at authoring)
      and on all refs (`git log --all --name-only --format= | grep "docs/spec/stdlib/20_"` → none).
      (2026-09-14: highest is `19_big.md`; the `git log` grep printed nothing → **20**, `20_compress.md`.)

Acceptance: every Prerequisites row reads MET with today's output pasted in the table.
  Check: the commands above (est. 3 min, plus the release build if stale).
Commit: f54d3d4b6

### Phase 2 — package skeleton + `crc32`

- [x] `src/codegen/builtins/compress/{mod.rs, func_crc32.rs, helper_crc32.rs, helper_crc32_table.rs}`
      per §4.1–4.3; `Body::Rewrite("__compress_crc32")`; `errors: vec!["ErrInvalidArgument"]`.
      (`cargo build --release --bin mfb` → `Finished`, no warnings; `compress-crc32-valid` built
      and ran: `check=3421780262` = catalogue `0xcbf43926`. See Corrections for the helper
      `IMPORT` lines a `WhenUsed` helper needs.)
- [x] `src/codegen/builtins/mod.rs`: `pub(crate) mod compress;`, `"compress"` in `BUILTIN_IMPORTS`
      (sorted) and in `ARGUMENT_CHECKED_PACKAGES`.
- [x] `src/codegen/registry/mod.rs` `build()`: `crate::codegen::builtins::compress::register(&mut r);`.
- [x] `src/docs/spec/language/18_builtin-functions.md`: add `compress` to the package sentence.
      (`cargo test --bin mfb spec_section_18_package_list_matches_is_builtin_import` → `1 passed`.)
- [x] Table-generation unit test in `compress/mod.rs` (§4.2) and the member-count test. Revised after the
      table rewrite (Corrections): the literal-equality test is replaced by
      `crc32_table_builder_uses_the_reflected_polynomial`. (`cargo test --bin mfb compress` →
      `crc32_table_builder_uses_the_reflected_polynomial ... ok`, `compress_registered_on_the_clean_room_registry ... ok`.)
      (`cargo test --bin mfb compress` → `crc32_tables_literal_matches_the_polynomial`,
      `crc32_tables_produce_the_catalogue_check_value`, `compress_registered_on_the_clean_room_registry` ok.)
- [x] Resolve the §2 UNVERIFIED call-copy property: `mfb build --ncode` the fixture below and
      confirm no block copy of `data` on the `__compress_crc32` call path; write the verdict
      into Verified properties. (Verified: no copy — §2 Verified properties.)

Acceptance: `crc32` is callable, rejects a bad `running`, and its table is machine-checked.
  Check: `cargo test --bin mfb compress` → the new unit tests pass;
  `cargo test --bin mfb spec_section_18_package_list_matches_is_builtin_import` → pass (est. 6 min).
Commit: 33eefae9b

### Phase 3 — tests

- [x] `tests/rt-behavior/compress/compress-crc32-valid/` — prints `crc32` of the fetched catalogue
      check string, of `[]`, of `[]` with `running := 12345`, and a three-way split chain; four
      goldens (`build.log`, `.ast`, `.ir`, `.run`) created by `touch` first, then
      `scripts/sync-goldens.sh target/release/mfb 'tests/rt-behavior/compress/compress-crc32-valid'`,
      then **read** `build.log` and compare the check value to the catalogue by eye.
      (sync: `synced 10 golden file(s) across 4 test(s)`; `build.log` reads `check=3421780262`
      = catalogue `check=0xcbf43926`, `empty=0`, `empty-running=12345`, `split=3421780262`;
      it also chains a 43-byte string split 16/19/8: `long=long-split=1095738169`.)
- [x] `tests/rt-error/compress/compress-crc32-running-invalid/` — `running := -1` raises
      `ErrInvalidArgument` (sibling layout: `tests/rt-error/crypto/crypto-ec-invalid`).
      (`build.log`: `before` / `Error: 7-705-0002` / `compress::crc32: running must be 0..4294967295` / `[exit 255]`.)
- [x] `tests/syntax/compress/compress-crc32-arity-invalid/` — wrong arity gets the argument
      diagnostic (proves `ARGUMENT_CHECKED_PACKAGES`); golden `build.log` only.
      (`build.log`: `TYPE_CALL_ARITY_MISMATCH` "Call to `compress.crc32` has 0 argument(s), expected 1 to 2"
      and "has 3 argument(s)"; `TYPE_CALL_ARGUMENT_MISMATCH` "(String), expected List OF Byte[, Integer]".)
- [x] `tests/byte-identity/compress/` — one program calling `crc32`; eight goldens via
      `scripts/regen-native-goldens.sh target/release/mfb tests/byte-identity/compress`.
      (`build.log`/`.ast`/`.ir` by `sync-goldens.sh`; `bash scripts/regen-native-goldens.sh …` →
      `5 build(s), 5 golden(s) rewritten, 0 failure(s)`; `scripts/artifact-gate.sh target/release/mfb compress`
      → `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`. Re-run after the §4.2 table rewrite, which
      changes the injected source and so every compress `.ir` and `.ncodesum`: `sync-goldens.sh` →
      `synced 10 golden file(s) across 4 test(s)`; regen → `5 golden(s) rewritten, 0 failure(s)`;
      artifact-gate → `7 golden(s) checked, 0 diff(s)`.)
- [x] `tests/interop/rt_compress_interop.rs` + `[[test]] rt_compress_interop` in `Cargo.toml`;
      `flate2 = "1"` in `[dev-dependencies]` with the "already in the lockfile" comment; test
      `crc32_matches_crc32fast_over_a_generated_corpus` (every length 0–17, then seeded LCG lengths 0..100,000, 200 cases,
      one MFB process fed a job file, compared to `crc32fast::hash`/`Hasher` via `flate2`'s
      re-export or a direct `crc32fast` dev-dep if `flate2` does not re-export it — record which).
      (Recorded: `flate2::Crc`, which is `crc32fast::Hasher` because `flate2`'s `zlib-rs` feature is
      off (`flate2-1.1.9/src/crc.rs` `#[cfg(not(feature = "zlib-rs"))] pub use impl_crc32fast::Crc`);
      `cargo tree -i flate2` → `png` → `image` → `mfb`, so the `Cargo.lock` diff is one line adding
      `flate2` to `mfb`'s dependency list. 218 cases (18 + 200) plus `running` probes at
      4294967295 / 4294967296 / -1: `cargo test --test rt_compress_interop` → `1 passed` in 58.88 s;
      re-run after the table rewrite → `1 passed` in 116.29 s under host load.)

Acceptance: fixtures green; the in-CI interop agrees with an independent implementation.
  Check: `scripts/test-accept.sh target/release/mfb /tmp/p137a 'compress' 'compress-*'` → 0 mismatches
  (2026-09-14: `acceptance tests passed (4 test(s) ran)`, and again after the table rewrite; glob corrected, see Corrections);
  `cargo test --test rt_compress_interop` → pass (est. 8 min).
Commit: 0e4fbe738

### Phase 4 — oracle + bench harnesses

- [x] `tools/oracles/compress/` per §4.4 with mode `crc32`; README states it is not in CI.
      (`python/gen.py`, `mfb/`, `python/oracle.py`, `node/oracle.mjs`, `run.sh` on
      `tools/oracles/crypto/_lib/harness.sh`; indexed in `tools/oracles/README.md`. `run.sh target/release/mfb crc32`
      → `crc32: 118/118 agreed with python`, `118/118 agreed with node`, `118 case(s), 0 failure(s)`, exit 0 —
      before and after the §4.2 table rewrite.)
- [x] `tools/compress-bench/` per §4.4 with the `crc32` row. (`run.sh` → `bench.py` + `mfb/`; every row's
      MFB result must equal Python's.)
- [x] Record the bench output (MiB/s at 1/4/16 MiB, `-O1` and `-O3`, macos-aarch64) in this
      file's Corrections section as the `crc32` baseline. (Corrections, "crc32 baseline".)

Acceptance: `tools/oracles/compress/run.sh target/release/mfb crc32` exits 0 with the declared
case count; the bench prints three sizes with linear time (16 MiB ≤ 4.4× the 4 MiB time).
  Check: those two commands (est. 5 min).
  (2026-09-14: oracle exit 0 with 118 declared cases; bench exit 0, every row `ok`, 16 MiB / 4 MiB
  = 3.99–4.03 across three corpora and both levels.)
Commit: f6384ac9e

### Phase 5 — docs

- [x] Descriptor prose: `MODULE_INTRO`/`MODULE_DESC` and `crc32`'s `intro`/`desc`/`example`/`Parameter.desc`
      per `.ai/man-content.md`. Describe only what exists after this letter — the package and
      `crc32`. No mention of decoders or encoders that later letters add; each letter extends the
      intro when its members land.
      (Written in Phase 2's `mod.rs` / `func_crc32.rs`; `mfb man compress crc32` renders. `scripts/man-run-examples.sh
      compress --run` → `examples: 2 built: 2 ran: 2 not run: 0 failed: 0`, both printing `3421780262`;
      `scripts/man-census.sh --fill compress` → INTRO 1, DESC 1, EXAMPLE 1, PARAM-DESC 2/2, PKGDOC 11;
      `--memory-scope compress` → `unclassified memory-vocabulary hits: 0`.)
- [x] `src/docs/spec/stdlib/20_compress.md`: the CRC-32 model (polynomial, reflection, init/xorout,
      `running` semantics and range), the MFBASIC-source/no-native guarantee, the gating rule,
      `[[src/codegen/builtins/compress/mod.rs:COMPRESS]]` provenance; reading-order bullet in
      `src/docs/spec/stdlib/spec.md`.
      (`mfb spec stdlib compress` renders with 0 `[[` markers; `scripts/spec-census.sh --citations stdlib` →
      `MISS-SYMBOL 0`, `--links stdlib` → `TOTAL links=138 unresolved=0`; the one `--render` "leak" is the
      `big::Int[[7, 0], FALSE]` literal in `19_big.md`, not a marker; `touch build.rs && cargo test --bin mfb spec`
      → `43 passed; 0 failed`.)
- [x] Size-gate measurement (§4.3) recorded in the spec page's contributor section. ("Source injection
      and size": 66,600 / 66,604 / 83,116 B and the 4 B build-string explanation.)
- [x] (Added 2026-09-14) Record the two durable lessons this letter hit in `.ai/resources-packages.md`:
      a `WhenUsed` helper needs its own `IMPORT`s, and a list literal costs ≈26 init instructions per
      element. (Two paragraphs before "Writing the native backend".)

Acceptance: pages render and their examples run.
  Check: `scripts/man-run-examples.sh compress --run` → all pass; `scripts/man-census.sh --fill compress`
  and `--memory-scope` → 0 unclassified; `cargo test --bin mfb spec` → pass (est. 6 min).
Commit: —

## Validation Plan

- Tests: the Phase 3 fixtures (valid, rt-error, syntax, byte-identity) and `rt_compress_interop`.
- Coverage check: the new `compress/*.rs` are Rust descriptor files; the behaviour lives in
  injected MFB, which Rust coverage cannot see — the rt/interop tests *are* the coverage. Confirm
  every public member is exercised by at least one executed fixture (`grep -rn "compress::" tests/rt-behavior/compress tests/interop/rt_compress_interop.rs`).
- Runtime proof: the `compress-crc32-valid` program on macos-aarch64 (acceptance) and cross-built
  for `linux-aarch64` on box 2223 via `scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64 glibc` with `FILTER=compress`.
- Doc sync: man descriptors **and** `20_compress.md` + `spec.md` reading order + §18 sentence.
- Final gate: runs once at the end of plan-137 (plan-137-E). This letter's checks stay scoped.

## Decisions (whole feature)

Decided by the user on 2026-09-13. These are settled; no letter re-opens them.

- **Bytes after the end of a stream are ignored** by `inflate`, `zlibDecode` and `gzipDecode`.
  For gzip, another member is decoded only while the remaining bytes begin with the magic
  `1f 8b`; anything else after the last member is ignored. (B §4.3)
- **`ignoreChecksum AS Boolean = FALSE`** on `zlibDecode` and `gzipDecode`. `FALSE`: a mismatched
  zlib Adler-32, gzip header CRC-16 (when `FHCRC` is set), gzip CRC-32 or `ISIZE` raises
  `ErrInvalidFormat`. `TRUE`: every checksum comparison is skipped. Structural checks apply
  either way — the flag never makes a malformed stream decode. Raw `inflate` has no checksum
  and no such parameter. (B §1, §4.3)
- **zlib preset dictionaries are refused for now**: `FDICT` set → `ErrInvalidFormat`. A
  `dictionary AS List OF Byte = []` parameter can be added later without breaking callers. (B §4.3)
- **`crc32` uses slicing-by-8** from the start. (§4.2)

## Open Decisions (whole feature)
- **Canvas strictness after C** — *accept that malformed PNGs (bad Adler-32, over-subscribed
  trees) are now refused* (recommended: they are malformed, and zlib refuses them) vs. a lenient
  canvas mode. (C §4)
- **bug-621 as a whole-feature prerequisite** — *whole feature* (recommended: the plan's
  RSS/time bounds would otherwise pin the bug) vs. letting A (which builds no large list) start
  first. Changing this is the user's call.

## Corrections

- **§4.3 / Phase 2 — a `WhenUsed`-gated helper is injected as its own source file, so its body
  must carry its own `IMPORT` lines.** The plan's `pkg.add_imports(...)` alone is not enough:
  the first build of `compress-crc32-valid` failed with `SYMBOL_UNKNOWN_IMPORT` "Package `bits`
  is used but not imported in this file" at `builtins/compress_crc32.mfb:7`. Crypto's gated
  argon2id helper already does this (`helper_argon2id.rs` BODY opens with `IMPORT crypto` /
  `IMPORT bits` / `IMPORT collections`). Every gated `compress` helper in B–E needs the same
  header. Fixed by adding the header to `helper_crc32.rs` and `helper_crc32_table.rs`.
- **§4.2 — a 2,048-element list literal is the wrong shape for the CRC table; build it at program
  start.** The plan chose a generated literal. Measured after Phase 3: `mfb build --ncode` of
  `tests/byte-identity/compress` has 59,725 instructions, 53,317 of them in the global initializer
  `_mfb_fn__5F_5Fmfb_5Finit_5Fglobals_…` (≈26 instructions per literal element) against 2,031 in
  `__compress_crc32`; the one-call size probe was +396,292 B over `IMPORT io`. A one-off probe
  (`/tmp/p137table.py`: the same user-space slicing-by-8 program, table as a literal vs built by a
  256×8 + 1,792-step MFB loop) gave **677,704 B vs 297,932 B** (−379,772 B), both agreeing with
  `zlib.crc32` on lengths 0–17, 100,000 and 16 MiB, and **195.7 ms vs 187.4 ms** for 16 MiB
  (same within noise). Replaced the literal with `__compress_crc32Tables()`; the literal-parsing
  unit test became `crc32_table_builder_uses_the_reflected_polynomial` (derives `0xEDB88320` as
  `0x04C11DB7.reverse_bits()` and requires the builder to use it). **Other letters:** plan-137-D
  §4.1's 512-entry bit-reverse literal (≈13,000 init instructions by the same rate) and plan-137-B
  §4.2's length/distance base/extra literals (118 elements) are corrected there to prefer a builder.
  The per-element literal cost is a codegen property, not a correctness bug; noted here, not filed.
- **Size gate measured (§4.3), 2026-09-14, macos-aarch64** (`/tmp/p137size.py`, `/tmp/p137size2.py`,
  the `.ai/resources-packages.md` probe): `IMPORT io` 66,600 B; `+ IMPORT compress`, no call,
  66,604 B; with one `crc32` call 83,116 B (+16,516 B, after the table rewrite; +396,292 B
  before it). The 4 B are not code: the only printable string that differs is the embedded
  project name (`mfb.szpcompress` vs `mfb.szpio`), and `IMPORT bits` / `IMPORT term` under
  same-length names both measure 66,600 B.
- **Found, pre-existing: 41 stale `[[ ]]` citations in other stdlib spec topics.**
  `scripts/spec-census.sh --citations stdlib` → `MISS-SYMBOL 41 (stale-by-move 39,
  stale-by-deletion 2)` across `01_regex` (2), `02_datetime` (8), `04_json` (3), `05_http` (21),
  `06_url` (16), `09_vector` (1); none in `20_compress.md`. Not caused by this plan; fixed in its
  own commit on this branch before the merge (the as-is rule: re-point the 39, re-verify the
  claims behind the 2 deletions). **Fixed in `8d17be496`:** 33 plain `__pkg_*` helper moves re-pointed (43 markers);
  the other 8 re-verified before re-citing — datetime's rewrite seam and clock intrinsics
  (`errors` lists, `RESULT_OK_TAG`), http's `__http_dechunkBytes` framing errors, json's registry
  injection and variant-record acceptance (probe `json::stringify(json::JsonStr["hi"])` → `"hi"`),
  regex's script-name helper (reached only from `__regex_canonProp`) and `start` padding
  (`registry::default_argument_padding`, called from `src/ir/lower.rs`). Two sentences were
  rewritten because their mechanism was gone (json's `json_package.mfb` / `uses_package` injection
  and `is_json_value_type`). `--citations stdlib` → `MISS-SYMBOL 0` over 253 citations.
- **crc32 baseline (Phase 4), 2026-09-14, macos-aarch64**, `tools/compress-bench/run.sh target/release/mfb`
  (median of 3 interleaved rounds × 5 runs, in-process), after the §4.2 table rewrite:

  | corpus | -O | 1 MiB | 4 MiB | 16 MiB | Python zlib |
  |---|---|---|---|---|---|
  | random | 1 | 86.8 MiB/s | 86.7 | 86.4 | ≈32,000 MiB/s |
  | random | 3 | 101.6 | 101.8 | 101.6 | |
  | text | 1 | 86.6 | 86.6 | 86.4 | |
  | text | 3 | 101.8 | 101.4 | 101.7 | |
  | zero | 1 | 86.8 | 86.7 | 86.2 | |
  | zero | 3 | 104.4 | 103.9 | 103.1 | |

  16 MiB / 4 MiB: 4.01 / 4.01 / 4.01 / 3.99 / 4.03 / 4.03. Python's `zlib.crc32` is ≈370× faster (native
  SIMD/CRC32 instructions); MFB at −O1 is ≈27× the SHA-256 number in §2.
- **§4.4 bench timing — `/usr/bin/time` replaced by in-process timing, and the rows interleaved.**
  `/usr/bin/time -p` resolves 10 ms, too coarse for a 1 MiB crc32 (≈12 ms), and times file reads and
  start-up too; the bench program times each run with `datetime::monotonicNanos` around the op. The
  first bench run (rows timed back to back, each kind right after generating its corpus) **failed**
  linearity on text and zero (5.12–6.92×) while random passed. A data-independent loop cannot be
  content-dependent, so it was treated as a harness bug: `/tmp/p137lin.py` ran the same binary on zero
  and random files interleaved — 16 MiB medians 182.8 / 184.8 ms, 4 MiB 45.5 / 46.2 ms, ratios 4.02 /
  4.00 — while `uptime` showed load average 19.56 with a QEMU guest at 323% CPU. `bench.py` now writes
  every corpus before timing anything and runs `ROUNDS` (default 3) interleaved rounds; re-run: all
  rows linear.
- **`tools/oracles/crypto/_lib/harness.sh` located the repo root as "up four directories"**, which is
  right only for `tools/oracles/crypto/<name>/run.sh`; `tools/oracles/compress/run.sh` sits one level
  higher. Changed to `git -C "$HERE" rev-parse --show-toplevel`; `git -C tools/oracles/crypto/keys
  rev-parse --show-toplevel` → the worktree root, and the compress oracle builds and runs through it.
- **Phase 3 check glob.** `scripts/test-accept.sh … 'compress'` selects only
  `byte-identity/compress`: a glob matches a test's relative path or its basename, and the other
  three fixtures are `compress-crc32-*`. Corrected to `'compress' 'compress-*'` (4 tests ran). The
  same glob in plan-137-B Phase 3's check is corrected there.
- **`scripts/regen-native-goldens.sh` is not executable** (`permission denied` when run as
  `scripts/regen-native-goldens.sh`); it runs as `bash scripts/regen-native-goldens.sh`. Every later
  letter's byte-identity regen step needs the `bash` prefix.
- **`.run` goldens are empty markers**, not transcripts: `scripts/test-accept.sh` (the
  "A `<pkg>.run` golden forces the full `mfb build`" block) never compares their contents, and
  `sync-goldens.sh` wrote them 0 bytes. The run output is pinned in `build.log`.
- **Prerequisites, 2026-09-14:** bug-621 landed on main (`a3f7cb06a`); `main` merged into
  `worktree-P-137` at `55dc9a626` before the gate re-run.

## Summary

The engineering risk of plan-137 is in B (decoder strictness and speed) and E (length-limited
Huffman construction); this letter is plumbing plus a checksum whose table is machine-checked.
Nothing outside the new package changes; the canvas decoder is untouched until C.
