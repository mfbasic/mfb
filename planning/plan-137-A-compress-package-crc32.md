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
| bug-621 fixed (append growth reserves data per element width) — B's decoder and D/E's encoders build multi-MiB outputs by in-place append; with bug-621 open a 64 MiB decode reserves ~19 B per output byte and every RSS bound in this plan would pin the bug | `ls bugs/completed/bug-621-*` → one file | NOT MET |
| Release compiler builds | `cargo build --release --bin mfb` → exit 0 | UNMEASURED — run before starting |
| zlib oracles present on the dev host | `python3 -c "import zlib; print(zlib.ZLIB_RUNTIME_VERSION)"` → a version; `node -e "console.log(process.versions.zlib)"` → a version | MET (2026-09-13: Python 3.14.5 / zlib 1.2.12; Node v24.12.0 / zlib 1.3.1-470d3a2) |
| `flate2` resolves offline for the in-CI interop test | `grep -n -A3 '^name = "flate2"' Cargo.lock` → `1.1.9`, deps `crc32fast`, `miniz_oxide 0.8.9` | MET (2026-09-13) |
| No `compress` package exists yet | `ls src/codegen/builtins \| grep -c compress` → 0 | MET (2026-09-13) |

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
- UNVERIFIED: that a `FUNC` taking `List OF Byte` and returning `Integer` from a
  `Body::Rewrite` member compiles to a call with no list copy on entry. Task in Phase 2
  (inspect `--ncode` of the fixture for a block copy on the call path).

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

- Reflected table-driven CRC: `crc = bxor(sr(crc, 8), T[band(bxor(crc, byte), 255)])`, with
  `crc` initialised to `bxor(running, 0xFFFFFFFF)` and the result `bxor(crc, 0xFFFFFFFF)`.
- `T` is a module-level `LET __COMPRESS_CRC32_TABLE AS List OF Integer = [...]` (256 entries).
  The literal is **generated**, not typed: a `#[cfg(test)]` test in `compress/mod.rs` computes
  the table from `0xEDB88320` and asserts the helper source's literal equals it, so a typo
  cannot survive.
- One byte-at-a-time loop in one function; the input list is only read.
- `running < 0 OR running > 4294967295` → `FAIL error(77050002, "compress::crc32: running must be 0..4294967295")`.
- Slicing-by-4/8 is **not** in this letter; if the bench shows `crc32` dominating `gzipDecode`
  in B, B records it and E's Open Decisions carries it.

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
  times each op by median of 5 runs with `/usr/bin/time`, and prints MiB/s beside Python
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

- [ ] Re-run every Prerequisites command; update statuses.
- [ ] `ls packages | grep -c '^compress$'` → 0; `grep -rn '"compress"' src/codegen/builtins/mod.rs` → no hits.
- [ ] Record the spec page number: next free `NN` in `ls src/docs/spec/stdlib` (20 at authoring)
      and on all refs (`git log --all --name-only --format= | grep "docs/spec/stdlib/20_"` → none).

Acceptance: every Prerequisites row reads MET with today's output pasted in the table.
  Check: the commands above (est. 3 min, plus the release build if stale).
Commit: —

### Phase 2 — package skeleton + `crc32`

- [ ] `src/codegen/builtins/compress/{mod.rs, func_crc32.rs, helper_crc32.rs, helper_crc32_table.rs}`
      per §4.1–4.3; `Body::Rewrite("__compress_crc32")`; `errors: vec!["ErrInvalidArgument"]`.
- [ ] `src/codegen/builtins/mod.rs`: `pub(crate) mod compress;`, `"compress"` in `BUILTIN_IMPORTS`
      (sorted) and in `ARGUMENT_CHECKED_PACKAGES`.
- [ ] `src/codegen/registry/mod.rs` `build()`: `crate::codegen::builtins::compress::register(&mut r);`.
- [ ] `src/docs/spec/language/18_builtin-functions.md`: add `compress` to the package sentence.
- [ ] Table-generation unit test in `compress/mod.rs` (§4.2) and the member-count test.
- [ ] Resolve the §2 UNVERIFIED call-copy property: `mfb build --ncode` the fixture below and
      confirm no block copy of `data` on the `__compress_crc32` call path; write the verdict
      into Verified properties.

Acceptance: `crc32` is callable, rejects a bad `running`, and its table is machine-checked.
  Check: `cargo test --bin mfb compress` → the new unit tests pass;
  `cargo test --bin mfb spec_section_18_package_list_matches_is_builtin_import` → pass (est. 6 min).
Commit: —

### Phase 3 — tests

- [ ] `tests/rt-behavior/compress/compress-crc32-valid/` — prints `crc32` of the fetched catalogue
      check string, of `[]`, of `[]` with `running := 12345`, and a three-way split chain; four
      goldens (`build.log`, `.ast`, `.ir`, `.run`) created by `touch` first, then
      `scripts/sync-goldens.sh target/release/mfb 'tests/rt-behavior/compress/compress-crc32-valid'`,
      then **read** `build.log` and compare the check value to the catalogue by eye.
- [ ] `tests/rt-error/compress/compress-crc32-running-invalid/` — `running := -1` raises
      `ErrInvalidArgument` (sibling layout: `tests/rt-error/crypto/crypto-ec-invalid`).
- [ ] `tests/syntax/compress/compress-crc32-arity-invalid/` — wrong arity gets the argument
      diagnostic (proves `ARGUMENT_CHECKED_PACKAGES`); golden `build.log` only.
- [ ] `tests/byte-identity/compress/` — one program calling `crc32`; eight goldens via
      `scripts/regen-native-goldens.sh target/release/mfb tests/byte-identity/compress`.
- [ ] `tests/interop/rt_compress_interop.rs` + `[[test]] rt_compress_interop` in `Cargo.toml`;
      `flate2 = "1"` in `[dev-dependencies]` with the "already in the lockfile" comment; test
      `crc32_matches_crc32fast_over_a_generated_corpus` (seeded LCG, lengths 0..100,000, 200 cases,
      one MFB process fed a job file, compared to `crc32fast::hash`/`Hasher` via `flate2`'s
      re-export or a direct `crc32fast` dev-dep if `flate2` does not re-export it — record which).

Acceptance: fixtures green; the in-CI interop agrees with an independent implementation.
  Check: `scripts/test-accept.sh target/release/mfb /tmp/p137a 'compress'` → 0 mismatches;
  `cargo test --test rt_compress_interop` → pass (est. 8 min).
Commit: —

### Phase 4 — oracle + bench harnesses

- [ ] `tools/oracles/compress/` per §4.4 with mode `crc32`; README states it is not in CI.
- [ ] `tools/compress-bench/` per §4.4 with the `crc32` row.
- [ ] Record the bench output (MiB/s at 1/4/16 MiB, `-O1` and `-O3`, macos-aarch64) in this
      file's Corrections section as the `crc32` baseline.

Acceptance: `tools/oracles/compress/run.sh target/release/mfb crc32` exits 0 with the declared
case count; the bench prints three sizes with linear time (16 MiB ≤ 4.4× the 4 MiB time).
  Check: those two commands (est. 5 min).
Commit: —

### Phase 5 — docs

- [ ] Descriptor prose: `MODULE_INTRO`/`MODULE_DESC` and `crc32`'s `intro`/`desc`/`example`/`Parameter.desc`
      per `.ai/man-content.md`. Describe only what exists after this letter — the package and
      `crc32`. No mention of decoders or encoders that later letters add; each letter extends the
      intro when its members land.
- [ ] `src/docs/spec/stdlib/20_compress.md`: the CRC-32 model (polynomial, reflection, init/xorout,
      `running` semantics and range), the MFBASIC-source/no-native guarantee, the gating rule,
      `[[src/codegen/builtins/compress/mod.rs:COMPRESS]]` provenance; reading-order bullet in
      `src/docs/spec/stdlib/spec.md`.
- [ ] Size-gate measurement (§4.3) recorded in the spec page's contributor section.

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

## Open Decisions (whole feature)

- **Trailing bytes after a complete stream** (`inflate`, `zlibDecode`; after the last gzip
  member) — *refuse with `ErrInvalidFormat`* (recommended: a decoder that silently ignores
  bytes hides truncation-by-concatenation bugs) vs. ignore. B Phase 1 records what Python and
  Node do and the decision is written into `20_compress.md`. (B §4)
- **gzip `FHCRC`** — *verify the header CRC-16 when the flag is set* (recommended) vs. skip. (B §4)
- **zlib `FDICT`** — *refuse with `ErrInvalidFormat`* (recommended; no dictionary API exists) vs.
  a dictionary parameter. (B §4)
- **Canvas strictness after C** — *accept that malformed PNGs (bad Adler-32, over-subscribed
  trees) are now refused* (recommended: they are malformed, and zlib refuses them) vs. a lenient
  canvas mode. (C §4)
- **`crc32` slicing-by-8** — *byte table first; revisit only if B's bench shows CRC dominating
  `gzipDecode`* (recommended) vs. slicing now. (§4.2)
- **bug-621 as a whole-feature prerequisite** — *whole feature* (recommended: the plan's
  RSS/time bounds would otherwise pin the bug) vs. letting A (which builds no large list) start
  first. Changing this is the user's call.

## Corrections

<Filled in during execution: every place the plan was wrong — the claim, what was true, the
evidence — and whether another letter's scope used the wrong number. Also the recorded bench
baselines.>

## Summary

The engineering risk of plan-137 is in B (decoder strictness and speed) and E (length-limited
Huffman construction); this letter is plumbing plus a checksum whose table is machine-checked.
Nothing outside the new package changes; the canvas decoder is untouched until C.
