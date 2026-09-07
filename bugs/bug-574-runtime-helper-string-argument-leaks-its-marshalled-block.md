# bug-574: every runtime-helper call leaks its marshalled `String` ARGUMENT, and the leak scales with the argument's length

Last updated: 2026-09-07
Effort: small–medium
Severity: **HIGH** (unbounded leak on every `fs::`/`os::`/`net::`/`process::` call that takes a path or a name — proportional to the path's length)
Class: Memory / correctness

Status: **Fixed** (this branch)
Regression Test:
`tests/runtime/rt_scope_drop_leaks.rs` — seven RSS cases (`b574_short_path`,
`b574_long_path`, `b574_local_path`, `b574_env_name`, `b574_two_args`,
`b574_no_argument`, `b574_arch`) plus
`every_helper_whose_scratch_is_released_still_produces_the_right_value`;
`tests/codegen/codegen_helper_scratch_release.rs` — the per-symbol enumeration.

Found while measuring bug-566, whose contrast programs would not go flat after
that fix. It is a **different defect**, independent of `TRAP`, of the result type,
and of bug-566's own fix: it reproduces on the base compiler `ac421788a` and
identically after bug-566.

## The finding

```
LET b AS Boolean = fs::exists("/tmp/x.txt")      ' no TRAP, Boolean result
```
in a loop leaks per call. The result type is irrelevant — this call returns a
`Boolean`, which carries no block at all. What leaks is the **argument**.

macOS arm64, peak RSS via `/usr/bin/time -l`, base `ac421788a`:

| program | 20 000 | 40 000 | per call |
| --- | --- | --- | --- |
| `fs::exists("/tmp/x.txt")` — 10-char path | 2.3 MB | 3.6 MB | **~65 B** |
| `fs::exists("/tmp/leakprobe/zzz…zzz.txt")` — 415-char path | 35.7 MB | 70.4 MB | **~1 819 B** |
| `LET p AS String = <the 415-char path>` once, then `fs::exists(p)` | 35.7 MB | 70.4 MB | **~1 819 B** |
| `os::getEnvOr("MFB_PROBE_NOT_SET", "d")` | 3.5 MB | 5.9 MB | ~129 B |
| contrast: `os::arch()` — a runtime helper with **no arguments** | 1.0 MB | 1.0 MB | **flat** |
| contrast: `strings::upper("hello world!")` — inline builtin, not a helper | 1.0 MB | 1.0 MB | **flat** |

Three things that row set establishes:

* It is **not the result**: `fs::exists` returns a `Boolean`, and `os::arch()`
  returns a `String` and is flat.
* It is **not the literal**: hoisting the path into a `LET` outside the loop
  changes nothing, so it is not a per-iteration copy of a rodata constant.
* It **scales with the argument's byte length** — 10 bytes costs ~65, 415 bytes
  costs ~1 819. A constant-size leak would not.

Zero-argument helpers are flat, so the site is argument marshalling.

## Where to look

`emit_raw_call(symbol, args, "runtime_call_arg")` in
`src/codegen/engine/builder/builder_emit_helpers.rs` is the one place every
runtime-helper call stages its arguments. A `String` argument has to reach the
host as a NUL-terminated buffer, and that buffer is an arena allocation with no
owner: nothing in `lower_ops_inner`'s statement-scope drop knows about it, because
it is not a `ValueResult` any node yielded — it is interior to the call sequence,
the same shape `register_fresh_string_temp` exists for (bug-536 shape B's
`strings::padLeft` pad character).

Check the per-call staging for a `String`/`List OF Byte` parameter and whether the
staged block is registered anywhere. `register_fresh_string_temp` is the existing
mechanism for exactly "a lowering allocates a block for its own use and never
returns it".

## Why it was invisible

Every leak in the 560–572 cluster was measured on shapes with no runtime-helper
call in them (`toString`, `&`, `collections::*`, a user `FUNC`) — all inline
builtins, which stage nothing. The fs/net/process families were never in a
constant-RSS test, so the whole argument-marshalling path had no leak coverage.
bug-566's RSS pins are comparative (`assert_no_extra_growth`) for precisely this
reason: they could not be flatness assertions while this is open.

## What a fix must produce

`fs::exists(p)` in a loop runs at constant RSS at N and 2N for a long `p` as well
as a short one, and `assert_no_extra_growth` in `rt_scope_drop_leaks.rs` can be
strengthened to `assert_flat` for bug-566's four shapes.

Measure as peak RSS at N and 2N with a LONG argument — a short path's ~65 B is
easy to lose in chunk-growth noise, while a 415-byte path's ~1.8 KB is not.

**The failure direction is a double free**: an argument block that a
`register_fresh_string_temp`-style registration frees at statement scope must not
also be the block a `String` VALUE argument already owns. `emit_raw_call` stages
some arguments by copying and some by passing an existing pointer through; only
the copies may be registered, and the distinction has to be read off the staging
code rather than assumed.


---

## What the fix is

`_mfb_rt_fs_fs_exists` allocates `len(path) + 1` bytes from the arena, copies the
path in NUL-terminated, calls `access`, and returns. Nothing frees the copy, on
any path. Every sibling does the same thing.

The report's "where to look" was wrong: `emit_raw_call` /
`emit_prepared_call_args` copy nothing — they lower each argument, spill it to a
stack slot and move it into an ABI register. The marshalling is INSIDE the
callee, in the hand-written `gen_*` body of each member, which is why no
caller-side ownership analysis could ever have seen it and why
`register_fresh_string_temp` was not the mechanism. The fix is therefore per
helper, at the helper's single `ret`:

```
<symbol>_done:
    mov  save0..save3, RESULT_TAG/VALUE/MESSAGE/ERROR_SOURCE
    cmp  scratch, 0
    b.eq <symbol>_scratch_kept_0
    mov  ARG[0], scratch ; mov ARG[1], scratch_size ; bl _mfb_arena_free
<symbol>_scratch_kept_0:
    mov  RESULT_TAG/VALUE/MESSAGE/ERROR_SOURCE, save0..save3
    ret
```

`emit_helper_scratch_release` in `src/codegen/memory/arena/native_arena.rs`.
Three things make it safe:

* **A runtime pointer guard, not a whole-program proof.** `HelperScratch::declare`
  nulls the pointer at the TOP of the body, ahead of every branch that can reach
  `done` — the `ErrOutOfMemory` tail that never allocated, the empty-path
  rejection, `net::listen`'s bind-all host that jumps straight past its
  `emit_cstring`. Nulling at the allocation site instead (the first shape tried)
  is wrong for exactly those paths.
* **The size is the allocation's own.** `_mfb_arena_free` is size-taking and
  re-normalizes exactly as `_mfb_arena_alloc` does, so a size that is not the one
  the allocation was given returns the block to the wrong bin (bug-560's
  mechanism). The size lives in its own vreg, written where the allocation size
  is computed.
* **The result registers survive.** `_mfb_arena_free` is a PCS call taking its
  arguments in `c_arg(0..=1)`, which are the SAME physical registers as
  `mfb_return(0..=1)` on every backend (the `Mfb`/`C` banks are aligned —
  `realize_abi_operand`). All four fallible-ABI outputs are saved into vregs
  across the frees and restored.

## The doc's mechanism survived, and is NARROWER than the defect

Every measured claim reproduced on `4ef3bce0c` to within a percent: `fs::exists`
with a 10-character path 2.3 → 3.6 MB (66 B/call), with a 415-character path
35.7 → 70.4 MB (1 820 B/call), the same path in a `LET` identical, `os::arch()`
flat. The slope is linear in the argument's length at ~5.1 B per byte (measured
at 15/50/100/200/300/415/800 characters — the multiplier over the 1 B/byte the
copy itself needs is the arena's `ARENA_MIN_CHUNK` rounding and chunk geometry).

What is too narrow is the framing "argument marshalling". Two of the biggest
rows have no `String` argument at all:

| call | baseline | after |
| --- | --- | --- |
| `fs::isWithin("/tmp", "/tmp")` — 4 scratch blocks, `Boolean` result | 32 768 B/call | **0** |
| `fs::currentDirectory()` — no argument, 4 KiB `getcwd` buffer | 16 384 B/call | **0** |
| `fs::exists(<415-char path>)` | 1 820 B/call | **0** |
| `os::getEnvOr(<400-char name>, "d")` | 1 821 B/call | **0** |
| `fs::exists("/tmp/x.txt")` | 66 B/call | **0** |
| `net::lookup(<388-char host>)`, resolve failure | 2 801 B/call | 1 089 B/call |
| `net::lookup(<10-char host>)`, resolve failure | 1 146 B/call | 1 032 B/call |
| `os::arch()` (contrast, bound) | 0 B/call | **0** |

The generalisation the fix is built on is "a fixed runtime helper allocates a
block for its own use and never hands it back", which covers the `getcwd`
buffer, `fs::readLine`'s growing line accumulator (and the block its regrow
abandoned), and `fs::readBytes(path)`'s internal `File` record as well as the
marshalled arguments.

The two `net::lookup` rows are the honest ones: the LENGTH SCALING is gone
(4.4 B per host byte → 0.15), and the ~1 KB that remains is flat in the argument
and is NOT attributed here. It is not the orphaned `ErrorLoc` either: the same
two programs measure 1 089 → 1 056 B/call and 1 163 → 1 040 B/call across
bug-573's fix, i.e. unchanged. Whatever it is survives both arena fixes.

## bug-566's pins are flat now

`assert_no_extra_growth` was comparative for exactly this reason. All three
bug-566 shapes (`fs::readText` trapped/plain, `fs::readBytes` trapped/plain,
`fs::exists` trapped/plain) now measure 0 B growth between 20 000 and 40 000
iterations, and the helper asserts flatness in addition to the comparison.
`fs::readBytes` needed one more free than the argument to get there — its
internal `File` record.

## The enumeration, and its totality

`tests/codegen/codegen_helper_scratch_release.rs` builds each package's own
`codegen_cover` byte-identity fixture and asserts a per-symbol table of
`(arena_alloc, arena_free, guarded scratch release)` counts for
`fs` (38 helpers), `os` (15), `net` (3), `tcp` (14) and `udp` (10) — and asserts
the table's KEY SET **is** the set of that package's runtime helpers. A new
member, or a new allocation inside an existing one, changes a triple or adds a
key and reds the file; the answer cannot be inherited. Each `_scratch_kept_N`
label is separately checked to be the tail of a complete guard-and-free sequence,
so a refactor that dropped the null compare — turning the release into an
unguarded free of a pointer the OOM path never allocated — reds too.

`marshal_cstring` (`os`) and `emit_cstring` (`os/socket`) now take the declared
`HelperScratch` rather than minting it, so the null-init cannot be misplaced, and
`HelperScratch` is `#[must_use]`.

## Golden attribution

40 `.ncodesum` goldens across 8 packages × 5 targets. Per function, on
`linux-x86_64`:

| package | changed / total | gained ≥1 free | gained an alloc | `dataObjects` |
| --- | --- | --- | --- | --- |
| fs | 27 / 61 | 27 | 0 | identical |
| os | 5 / 36 | 5 | 0 | identical |
| net | 3 / 44 | 3 | 0 | identical |
| tcp | 3 / 54 | 3 | 0 | identical |
| udp | 3 / 50 | 3 | 0 | identical |
| tls | 1 / 115 | 1 | 0 | identical |
| thread | 1 / 48 | 1 | 0 | identical |
| http | 6 / 157 | 6 | 0 | identical |

**49 functions changed, every one gained at least one `_mfb_arena_free`, not one
gained an `_mfb_arena_alloc`, `dataObjects` byte-identical in all eight, and no
function changed without gaining an owner.** The `tls`/`thread`/`http` rows are
the `fs`/`net`/`tcp` helpers those packages link, not backend changes.

## Residue, filed rather than assumed

* **`tls::` marshalling — 12 sites, bug-575.** `tls` has its OWN `emit_cstring`
  (`builtins/tls/gen_shared.rs`) with the same defect, across the OpenSSL, Secure
  Transport and Schannel backends. Two of the twelve sit in shared sub-emitters
  (`emit_read_whole_file`, `socket_connect`) whose `done` belongs to a caller, and
  two of the three backends cannot be exercised on this host — a scratch release
  placed after a branch that reaches `done` frees an undefined vreg, so it wants
  its own change. `the_tls_marshalling_sites_are_the_known_residue` pins the count
  at 12 so a new one is not silently added.
* **An UNBOUND runtime-helper `String` result has no owner.**
  `n = n + len(os::hostName())` grows 128 B per call, `fs::tempDirectory()` 265,
  `os::arch()` 63 — identically on `4ef3bce0c` and after this change, and 0 for
  all three when the result is bound to a `LET`. Pre-existing, unrelated to
  marshalling, and not touched here.
* **`udp::receive` (6 allocations, 0 frees) and `io::readLine`'s error paths**
  are in the enumeration's tables with their current counts, so any change to
  them is a decision rather than a drift.

## Gates

`cargo test --release --no-fail-fast`, `scripts/artifact-gate.sh` (1 412 tests,
1 578 builds, 1 973 goldens, 0 diffs), `scripts/test-accept.sh` (1 434 ran),
`rustup run 1.96.0 cargo fmt --all --check`, `cargo check --all-targets` — all
green on a detached worktree at `4ef3bce0c`.
