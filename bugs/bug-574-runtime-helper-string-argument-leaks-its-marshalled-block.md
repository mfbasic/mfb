# bug-574: every runtime-helper call leaks its marshalled `String` ARGUMENT, and the leak scales with the argument's length

Last updated: 2026-09-07
Effort: small–medium
Severity: **HIGH** (unbounded leak on every `fs::`/`os::`/`net::`/`process::` call that takes a path or a name — proportional to the path's length)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/runtime/rt_scope_drop_leaks.rs` RSS case, to be added)

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
