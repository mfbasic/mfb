# plan-139-E: `fs::size` and `fs::readBytesAt` — the two builtins plan-139 is gated on

Last updated: 2026-09-19
Effort: medium (1h–2h)
Depends on: nothing. **This letter gates plan-139-A**, and through it B, C and D.

plan-139-A § Prerequisites names two `fs` builtins that do not exist and records that they
"belong to their own plan". No such plan was ever written — `grep -rln 'readBytesAt' planning/`
matched only plan-139-A itself (2026-09-19) — so the feature sat blocked indefinitely. This letter
absorbs that work into plan-139 as an append-only new letter, on the user's explicit instruction
(2026-09-19: "add the 2 prereqs to the plan or they will never get done").

Outcome: **`fs::size` reports an open handle's byte length and `fs::readBytesAt` reads `count`
bytes from an absolute offset without disturbing the handle's read position, returning fewer than
`count` bytes only at end of file; both raise `ErrResourceClosed` on a closed handle; neither reads
the whole file.**

| Letter | Delivers | Effort |
|---|---|---|
| **E** (this) | `fs::size`, `fs::readBytesAt` — registry descriptors, codegen, man, spec, fixtures | medium |
| A | package skeleton, source layer, zip reader | large |
| B | zip writer, `zip::extractTo`, README/doc.html | large |
| C | tar reader + writer + `tar::extractTo` | large |
| D | oracle probes, corpus, fuzz, differential, RSS proof, final gate, archive | medium |

Dependency graph (topological order — **E first**):

```
E ──> A ──> B ──> C ──> D
```

References:

- `src/codegen/builtins/fs/func_read_all_bytes.rs` — the registry-descriptor template (INTRO/DESC/
  EX, `Body::abi_function`, `ParameterType::named(super::FILE_TYPE_ID)`).
- `src/codegen/builtins/fs/gen_read_write.rs:448` `lower_fs_read_all_bytes_helper` — the byte-List
  allocation + read-loop template; `:667` `lower_fs_eof_helper` — the save/measure/restore seek
  triple this letter's `size` is built from.
- `src/codegen/engine/types/types.rs:760,781` — `CodegenPlatform::emit_read_file` /
  `emit_seek_file`, implemented for all three real targets
  (`src/target/macos_aarch64/code.rs:660,690`, `src/target/linux_common/code.rs:969,1027`,
  `src/target/win_x86_64/code.rs:2112,2262`).
- `src/codegen/builtins/fs/gen_open.rs:605` — `abi::mfb_return(2)`, the third-argument register.
- `.ai/testing-gates.md` — artifact gate, `test-accept.sh`/`sync-goldens.sh` mechanics, the
  new-fixture placeholder-golden rule, and the 4 known-baseline acceptance mismatches.
- `tests/rt-behavior/fs/func_fs_readAllBytes_valid/` — rt-behavior fixture layout.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Release compiler builds at HEAD | `cargo build --release` → `Finished` | MET (2026-09-19 in `.claude/worktrees/P-139` at `f204b84e2`: `Finished \`release\` profile [optimized] target(s) in 1m 51s` at the earlier tip `23c06f46b`; re-measure at the worktree tip before Phase 1) |
| `emit_seek_file` and `emit_read_file` exist on `CodegenPlatform` and are implemented by every real target | `grep -rn 'fn emit_seek_file\|fn emit_read_file' src/target/*/code.rs src/codegen/engine/types/types.rs` → trait decl + macos_aarch64 + linux_common + win_x86_64 | MET (2026-09-19: 2 in `types.rs`, 2 in each of `macos_aarch64/code.rs`, `linux_common/code.rs`, `win_x86_64/code.rs`) |
| `fs::size` / `fs::readBytesAt` do not already exist | `target/release/mfb man fs size`, `… readBytesAt` → `unknown fs function` ×2 | MET (2026-09-19: both report `error: unknown fs function`) |
| A builtin may take three arguments | `grep -rn 'mfb_return(2)' src/codegen/builtins/` → at least one site | MET (2026-09-19: `src/codegen/builtins/fs/gen_open.rs:605`, `fs::openWithin`'s `mode`) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command
> before you continue and before you decide to stop; if you stop, report the status of *all* rows.

## 1. Goal

- `fs::size(file AS fs::File) AS Integer` returns the file's byte length and leaves the read
  position exactly where it found it.
- `fs::readBytesAt(file AS fs::File, offset AS Integer, count AS Integer) AS List OF Byte` returns
  the bytes at `[offset, offset + count)`, truncated only by end of file, and leaves the read
  position exactly where it found it.

### Non-goals (explicit constraints)

- **No new syscall plumbing.** Both functions are compositions of the existing
  `emit_seek_file`/`emit_read_file` platform hooks. plan-139-A's Prerequisites text predicted
  "per-target codegen for `pread`/`lseek`+`read`/`ReadFile` with `OVERLAPPED`"; that prediction is
  **wrong** and is corrected in plan-139-A's Corrections — the per-target work is already done and
  abstracted, which is why this letter is medium and not large.
- **No public seek.** `fs::seek` is not added: nothing in plan-139 needs a mutable position, and a
  public seek would make every `fs::File` alias position-dependent. Positional reads are
  position-neutral by construction, which is what letter A's `readAt` seam requires.
- **No change to any existing `fs` function**, its behavior, or its man page.
- **No streaming read into a caller-supplied buffer.** `readBytesAt` allocates and returns a fresh
  `List OF Byte`, like every other `fs` read.
- **No `.ncode` churn expected.** Builtin bodies are emitted on demand, so adding two uncalled
  registry functions should leave every existing fixture byte-identical. This is a **prediction**,
  not a licence: if the gate reports a diff, Phase 3 root-causes ONE fixture before concluding
  anything (see Phases).

## 2. Current State

- `fs` has 41 registry function files (`ls src/codegen/builtins/fs/func_*.rs | wc -l` → 41) and no
  positional read, no seek and no size: `grep -h -o 'name: "[a-zA-Z]*"'
  src/codegen/builtins/fs/func_*.rs | grep -i -E 'size|seek|At"'` → exit 1, no matches.
- The handle reads are `readLine`, `readAll`, `readAllBytes`, `eof`. `readBytes`/`readText` are
  path-based whole-file reads (`mfb man fs`: "read the entire file in one call").
- **Seeking already happens inside `fs`.** `fs::readAllBytes`, `fs::readAll` and `fs::eof` each
  emit the same triple — `emit_seek_file(fd, 0, SEEK_CUR)` to save the position,
  `emit_seek_file(fd, 0, SEEK_END)` to measure the length, `emit_seek_file(fd, saved, SEEK_SET)` to
  restore — and branch to a shared `seek_error` label raising `ErrReadFailed`
  (`gen_read_write.rs:509-547` and `:703-745`). `fs::size` is that triple returning `end`.
- A buffered handle keeps `FILE_OFFSET_READ_POS`/`FILE_OFFSET_READ_FILL` read-ahead state
  (`gen_read_write.rs`, `emit_reconcile_read_buffer`). `readAllBytes` reconciles it because it
  *consumes* from the logical position; `eof` instead compares the buffer indices directly.
- Arguments arrive in `abi::return_register()` (arg 0), `abi::mfb_return(1)`, `abi::mfb_return(2)`.
- `src/docs/spec/language/18_builtin-functions.md:83` is the one spec line listing `fs::` members.
  There is no `src/docs/spec/stdlib/*` page for `fs` (`ls src/docs/spec/stdlib/` → 20 pages, none
  `fs`), so that single line plus the generated man pages are the whole doc surface.

### Measured populations

| What | Count | Command |
|---|---|---|
| `fs` registry function files | 41 | `ls src/codegen/builtins/fs/func_*.rs \| wc -l` → 41 |
| existing `fs` rt-behavior fixtures | 24 | `ls -d tests/rt-behavior/fs/*/ \| wc -l` → measure in Phase 1 and record |
| sites emitting the save/measure/restore seek triple | 3 (`readAll`, `readAllBytes`, `eof`) | `grep -c 'emit_seek_file' src/codegen/builtins/fs/gen_read_write.rs` → 9 calls = 3 triples |
| spec lines listing `fs::` members | 1 | `grep -c 'fs::readAllBytes' src/docs/spec/language/18_builtin-functions.md` → 1 |

### Verified properties

- **The seek/read platform hooks are target-complete.** Verified 2026-09-19:
  `emit_read_file`/`emit_seek_file` are declared on `CodegenPlatform`
  (`src/codegen/engine/types/types.rs:760,781`) and implemented by `macos_aarch64`,
  `linux_common` (covering linux-x86_64/aarch64/riscv64) and `win_x86_64`. No target needs new
  syscall code for this letter.
- **Three-argument builtins work.** `fs::openWithin(dir, path, mode)` reads its third argument from
  `abi::mfb_return(2)` (`gen_open.rs:605`).
- **`ErrInvalidArgument` is an existing shared code**, so negative `offset`/`count` need no new
  error (`grep -o -E '\`Err[A-Za-z]+\`' src/docs/spec/diagnostics/02_error-codes.md | sort -u` →
  includes `ErrInvalidArgument`).
- **UNVERIFIED — whether a seek-away-and-back leaves a buffered handle consistent.** The design
  argues it must (the raw fd position is restored exactly and the buffer's bytes are untouched), but
  a `readLine`-then-`readBytesAt`-then-`readLine` sequence is the case that would expose a mistake.
  Phase 2 pins it with a fixture rather than asserting it.
- **UNVERIFIED — whether adding two uncalled registry functions is codegen-neutral.** Phase 3
  measures it with the full artifact gate; a diff is a root-cause trigger, not a stop.

## 3. Design Overview

Both functions are **position-neutral**: they save the raw descriptor position, do their work, and
restore it. That is the property letter A's `readAt` seam depends on — an `Archive` aliasing a
caller's `fs::File` must not move the caller's read position out from under them, and two
`readAt`s must not interfere.

`fs::size` is `lower_fs_eof_helper`'s measurement with the comparison removed: save `SEEK_CUR`,
measure `SEEK_END`, restore `SEEK_SET`, return the measured end. It deliberately does **not**
reconcile the read buffer: the length of the file does not depend on the read position, and the
raw position is restored exactly, so reconciling would only discard useful read-ahead.

`fs::readBytesAt` is `lower_fs_read_all_bytes_helper` with the length arithmetic replaced. It also
does not reconcile the read buffer, for the same reason plus one more: reconciling would make a
positional read *destroy* a concurrent `readLine`'s read-ahead, turning a read-only query into a
side effect. The correctness argument is that the buffer describes bytes at a fixed file region and
the raw fd position is returned to its exact prior value, so nothing the buffer records becomes
stale. Phase 2's interleaving fixture is what makes that an observation rather than an assertion.

**Correctness risk** concentrates in the clamp (`count` vs. bytes actually available) and in the
restore-on-error paths: every failure exit must still restore the position, or a failed
`readBytesAt` silently corrupts the caller's handle. The design routes all error exits through a
single restore-then-raise tail.

**Gate class:** builtin-adding, behavior-adding, expected codegen-neutral for existing programs.
Gates are `cargo test --bin mfb`, the two new rt-behavior fixtures, `scripts/test-accept.sh`, and
one full `scripts/artifact-gate.sh <exe> all`.

## 4. Detailed Design

### 4.1 `fs::size(file AS fs::File) AS Integer`

New files: `src/codegen/builtins/fs/func_size.rs` (descriptor, modeled on `func_read_all_bytes.rs`)
and `lower_fs_size_helper` in `gen_read_write.rs`. Registered in `mod.rs` beside `func_eof`.

1. `file.closed ≠ 0` → `ErrResourceClosed`.
2. `fd = file.fd`; `saved = seek(fd, 0, SEEK_CUR)`; `< 0` → `ErrReadFailed`.
3. `end = seek(fd, 0, SEEK_END)`; `< 0` → `ErrReadFailed`.
4. `seek(fd, saved, SEEK_SET)`; `< 0` → `ErrReadFailed`.
5. Return `end` as `Integer`.

A non-seekable handle (pipe, socket) fails at step 2 with `ErrReadFailed`, exactly as `fs::eof`
already does; the man page says so.

### 4.2 `fs::readBytesAt(file AS fs::File, offset AS Integer, count AS Integer) AS List OF Byte`

New files: `src/codegen/builtins/fs/func_read_bytes_at.rs` and `lower_fs_read_bytes_at_helper` in
`gen_read_write.rs`.

1. `file.closed ≠ 0` → `ErrResourceClosed`.
2. `offset < 0` or `count < 0` → `ErrInvalidArgument`.
3. `saved = seek(fd, 0, SEEK_CUR)`; `end = seek(fd, 0, SEEK_END)`.
4. Clamp: `offset >= end` → `available = 0`; else `available = end - offset`;
   `n = min(count, available)`.
5. `seek(fd, offset, SEEK_SET)`.
6. Allocate a `List OF Byte` of exactly `n` (the `read_all_bytes` allocation block verbatim, with
   `n` for `length`); `n = 0` allocates the empty list and skips the read loop.
7. Read loop into the data region until `n` bytes are in hand (the `emit_transfer_loop_tail` the
   sibling helpers use); a short read before `n` → `ErrReadFailed`, since step 4 already proved
   those bytes exist.
8. `seek(fd, saved, SEEK_SET)` — **on every path, success or failure**, before returning or
   raising. Allocation failure → `ErrOutOfMemory` after the restore.
9. Return the list.

Step 4 is what makes the prerequisite's contract — "returns fewer than `count` bytes only at end of
file" — true by construction rather than by a short-read accident.

### 4.3 Errors

`ErrResourceClosed` (closed handle), `ErrInvalidArgument` (negative `offset`/`count`),
`ErrReadFailed` (seek failed, non-seekable handle, short read, host read error), `ErrOutOfMemory`
(allocation). All already exist in `src/docs/spec/diagnostics/02_error-codes.md`; no new code.

### 4.4 Docs

Man pages are generated from the registry INTRO/DESC/EX, so writing the descriptor writes the page.
Both DESCs must satisfy `.ai/man-content.md` vocabulary rules (no borrow/ownership/heap/lifetime/
allocate/refcount/dangling) and state: position-neutrality, the short-read-only-at-EOF contract,
`ErrResourceClosed` on a closed handle, and non-seekable handles failing. `fs::readBytesAt`'s DESC
cross-references `fs::size` and contrasts with `fs::readBytes` (path-based, whole file).

Spec: append `fs::size` and `fs::readBytesAt` to the `fs::` list at
`src/docs/spec/language/18_builtin-functions.md:83`, after `fs::readAllBytes`.

## Compatibility / Format Impact

Two added builtins. No existing function, signature, error code or man page changes. Programs that
do not call the new functions are expected to compile to byte-identical native code (Non-goals;
measured in Phase 3).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the work;
> `- [~]` for partial with what remains; moot tasks struck through with evidence, never deleted;
> fill `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — `fs::size`

The smaller function first: it proves the descriptor → codegen → man → fixture → golden path end to
end before the harder allocation-carrying one is written.

- [ ] Record the measured-population row for `fs` rt-behavior fixtures (`ls -d
      tests/rt-behavior/fs/*/ | wc -l`) and re-run the Prerequisites table at the worktree tip.
- [ ] `src/codegen/builtins/fs/func_size.rs` — descriptor per §4.1/§4.4, modeled on
      `func_read_all_bytes.rs`; `mod func_size;` + `func_size::register(&mut pkg);` in `mod.rs`.
- [ ] `lower_fs_size_helper` in `gen_read_write.rs` per §4.1.
- [ ] `target/release/mfb man fs size` → a page whose Declaration is
      `fs::size(file AS fs::File) AS Integer`, with no banned vocabulary
      (`target/release/mfb man fs size | grep -n -i -E 'borrow|ownership|heap|lifetime|allocate|refcount|dangling'`
      → no matches).
- [ ] Spec line updated (`src/docs/spec/language/18_builtin-functions.md:83`).
- [ ] New fixture `tests/rt-behavior/fs/func_fs_size_valid/` (layout copied from
      `func_fs_readAllBytes_valid`): writes a known-length file, opens it, prints `fs::size`, then
      prints `fs::readLine` **after** the size call to show the position was not moved, then
      `fs::size` after `fs::close` raising `ErrResourceClosed`. Pre-create the `golden/`
      placeholders (`build.log`, `.ast`, `.ir`, `.run`) before syncing — `sync-goldens.sh` only
      refreshes files that already exist.
- [ ] `scripts/sync-goldens.sh target/release/mfb 'func_fs_size_valid'` then
      `scripts/test-accept.sh target/release/mfb "$(mktemp -d)" 'func_fs_size_valid'` → pass.

Acceptance: `fs::size` reports the right length and does not move the read position.
  Check: `cargo test --bin mfb` → pass; the fixture's `build.log` shows the file's true length and
  the post-size `readLine` still returns the FIRST line (est. 4 min).
Commit: —

### Phase 2 — `fs::readBytesAt`

- [ ] `src/codegen/builtins/fs/func_read_bytes_at.rs` — descriptor per §4.2/§4.4; registered in
      `mod.rs`.
- [ ] `lower_fs_read_bytes_at_helper` in `gen_read_write.rs` per §4.2, including the
      restore-on-every-path tail (§4.2 step 8).
- [ ] `target/release/mfb man fs readBytesAt` → Declaration
      `fs::readBytesAt(file AS fs::File, offset AS Integer, count AS Integer) AS List OF Byte`;
      same banned-vocabulary grep → no matches.
- [ ] Spec line updated.
- [ ] New fixture `tests/rt-behavior/fs/func_fs_readBytesAt_valid/` covering, in one program:
      a read at offset 0; a read in the middle; a read whose `count` runs past EOF (fewer bytes,
      no error); `offset` at/past EOF → empty list; **position-neutrality** — `fs::readLine`,
      then `fs::readBytesAt`, then `fs::readLine` returning the SECOND line, proving the positional
      read did not disturb the buffered handle (the UNVERIFIED property in §2); a negative `count`
      → `ErrInvalidArgument`; a read after `fs::close` → `ErrResourceClosed`. Pre-create the
      `golden/` placeholders as in Phase 1.
- [ ] `scripts/sync-goldens.sh` + `scripts/test-accept.sh` on the new fixture → pass.
- [ ] Both `fs` rt-behavior fixtures still pass together:
      `scripts/test-accept.sh target/release/mfb "$(mktemp -d)" 'func_fs_*'` → no mismatch.

Acceptance: positional reads are correct, clamp only at EOF, and leave the handle's position and
read-ahead untouched.
  Check: `cargo test --bin mfb` → pass; the fixture's `build.log` shows the interleaved
  `readLine`/`readBytesAt`/`readLine` sequence returning line 1, the offset bytes, then line 2
  (est. 5 min).
Commit: —

### Phase 3 — gate, and release the plan-139-A prerequisite

- [ ] `cargo build --bin mfb --tests` → clean (catches cross-module breakage `--bin mfb` alone
      misses, per `.ai/testing-gates.md`).
- [ ] Full `scripts/artifact-gate.sh target/release/mfb all` (~15–20 min — start it in the
      background and do the next task while it runs). Expected `diffs=0` per §1 Non-goals.
      **If it is not 0:** objdump/diff ONE flagged fixture to localize before concluding anything.
      A diff here means either a bug in the new helpers (most likely: a clobbered register or a
      changed shared emitter) or a wrong neutrality prediction. Fix the bug, or correct the
      prediction in Corrections with the fixture-level evidence, and regenerate what the gate
      flags. It is not a stop.
- [ ] `scripts/test-accept.sh target/release/mfb "$(mktemp -d)"` → only the 4 known-baseline
      mismatches (`rt-behavior/native/libsnd-load-sound-rt`, `…/libsnd-playback-rt`,
      `…/native-link-inline-trap-rt`, `rt-behavior/tls/tls-connect-google-rt`). Any fifth is
      proven against a clean base checkout before it is treated as this letter's regression.
- [ ] Update plan-139-A § Prerequisites: both `fs` rows re-measured and, if green, flipped to MET
      with the `mfb man` output as evidence, and their note rewritten to point at letter E.
- [ ] Update plan-139-D Phase 4's "No `src/` change across plan-139" criterion, which this letter
      falsifies (see Corrections) — it becomes "no `src/` change in letters A–D", checked from
      letter A's first commit.

Acceptance: the tree is green and plan-139-A's gate is open.
  Check: `target/release/mfb man fs size` and `… man fs readBytesAt` both print a page with the
  §1 Goal declarations; artifact gate `diffs=0`; test-accept at baseline (est. 25 min).
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/fs/func_fs_size_valid/`, `tests/rt-behavior/fs/func_fs_readBytesAt_valid/`;
  `cargo test --bin mfb`.
- Coverage check: both new functions are called from an rt-behavior fixture that owns a `golden/`
  dir — required, because `.ai/testing-gates.md` records that a function reachable only from
  `tests/acceptance/**` is invisible to the artifact gate (the `strings::graphemeAt` false
  negative).
- Runtime proof: the fixtures' `build.log` program output, and letter D's RSS measurement, which is
  the end-to-end proof that `readBytesAt` does not read the whole file.
- Doc sync: generated man pages; `src/docs/spec/language/18_builtin-functions.md`.
- Final gate: Phase 3's artifact gate + test-accept; plan-139-D Phase 4 runs the feature-wide gate
  once at the end.

## Open Decisions

- `fs::size` on a non-seekable handle — `ErrReadFailed` (recommended; identical to `fs::eof`'s
  existing behavior on a pipe, so the two handle-measuring functions agree) vs. a distinct
  `ErrUnsupported`, which would be a better diagnostic but makes `size` and `eof` disagree about
  the same handle.
- Whether to also expose `fs::seek` — deliberately not (see §1 Non-goals). Revisit only if a caller
  outside plan-139 needs a mutable position.

## Corrections

*(none yet — this letter was authored 2026-09-19)*

## Summary

The work plan-139 was blocked on turns out to be materially smaller than plan-139-A predicted: the
per-target syscall plumbing it expected to need already exists as `emit_seek_file`/`emit_read_file`,
and three `fs` functions already emit the exact save/measure/restore seek triple `fs::size` needs.
The real risk is position-neutrality across a buffered handle and restoring the position on error
paths — both pinned by fixtures rather than argued.
