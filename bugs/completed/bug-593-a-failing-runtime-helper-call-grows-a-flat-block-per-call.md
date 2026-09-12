# bug-593: a FAILING runtime-helper call grows a flat, unattributed block per call

Last updated: 2026-09-12
Effort: small (one wrapper-only drop); the attribution was the work
Severity: MEDIUM — unbounded growth in any retry loop over a call that fails
Class: Memory / correctness

Status: **FIXED** — landed on main in `8413676c0` (fix `76b08c1df`); residual closed-record growth is plan-52-B by design, and `List OF net::Address` is bug-599
Regression Test: `tests/runtime/rt_scope_drop_leaks.rs` —
`a_trapped_error_bound_as_a_resource_is_not_retained`,
`a_trapped_error_bound_as_an_address_list_is_not_retained`, and the positive pin
`every_failing_resource_call_still_reports_its_error_and_origin`.

## Why this is filed, and why it is not a duplicate

Two separate bugs measured the same shape, and each explicitly declined to own it:

- **bug-574** (`bugs/completed/bug-574-runtime-helper-string-argument-leaks-its-marshalled-block.md`)
  fixed the argument leak that scaled with length, and recorded that what was left on
  the failure path is *not* that leak:

  | program | before | after bug-574 |
  |---|---:|---:|
  | `net::lookup(<388-char host>)`, resolve failure | 2 801 B/call | 1 089 B/call |
  | `net::lookup(<10-char host>)`, resolve failure | 1 146 B/call | 1 032 B/call |

  > the LENGTH SCALING is gone … and the ~1 KB that remains is flat in the argument and
  > is **NOT attributed here**. It is not the orphaned `ErrorLoc` either … Whatever it is
  > survives both arena fixes.

- **bug-575** fixed the twelve `tls::` C-string marshalling leaks. Its agent then
  measured a failing `tls::connect` still growing **~260 B/call on Linux and ~1.9 KB/call
  on macOS, independent of host length**, and `tcp::connect` — already fixed by bug-574 —
  growing the same way. It concluded "that's the trapped-error path, not this bug" and
  did not file it.

Defect search before filing (not just a number check — see the 587/588/589 duplicate
withdrawal, `fdad98ccc`): `git grep` over `bugs/` and `bugs/completed/` for
failing/trapped connect growth, "flat in the argument", "resolve failure" and the
per-call figures found only bug-574's own "NOT attributed" note. Number verified free
across main, every worktree, and `git log --all --grep=bug-593`.

## The shape, as measured so far

- It appears on the **failure** path of a runtime helper that returns an `Error`
  (`net::lookup` resolve failure, a refused `tcp::connect` / `tls::connect`).
- It is **flat in the argument's length** — so it is not a marshalled argument
  (bug-574 / bug-575 already own those).
- It **survived** bug-573's fix (orphaned `ErrorLoc`) and both arena fixes in bug-574,
  unchanged within noise (1 089 → 1 056, 1 163 → 1 040 B/call).
- It differs by platform (~260 B Linux, ~1.9 KB macOS on `tls::connect`), which suggests
  a per-platform error-construction or error-message block rather than a shared one.
- bug-566's measurements are a useful contrast: `fs::readText` bound with **no** `TRAP`
  grows 129 B/call and that rate is unchanged by bug-566's fix, i.e. a plain successful
  helper call in that table also has a flat residual. Whether that is the same defect is
  unknown.

## Phase 1 — reproduce and attribute (do this before theorising)

1. Reproduce each shape above on the current tree at **>=200k iterations**, RSS with
   `--test-threads=1` (Linux RSS pins are ~4x leaner than macOS — calibrate per host).
2. Separate the variables: failing vs succeeding call; `TRAP`ped vs propagated; bound vs
   unbound error; one helper family (`net`) vs another (`tcp`/`tls`).
3. Only then localize. Candidates to test, not conclusions: the `Error` record built on
   the failure path, its message `String`, a platform error-text lookup, or a per-call
   scratch that the success path frees and the error path skips (the shape bug-575 found
   in the tls helpers).

If the growth does not reproduce, record the conditions tried and keep this open with
that evidence, rather than closing it — two independent measurements saw it.

## Memory gate (required when fixed)

1. the RED RSS pin flips flat;
2. name the contract in `.ai/collections.md` / `mfb spec` §14 the fix realizes, and show
   it only ADDS a free rather than moving a lifetime;
3. artifact-gate delta confined to emitting fixtures, everything else byte-identical,
   zero `.run` goldens moving;
4. a POSITIVE pin that a failing call still reports the correct `ErrorLoc` and message,
   and that a succeeding call is unchanged — the dangerous direction on an error path is
   freeing a block the propagated `Error` still refers to.

---

## Reproduction (base 04c81a605, macOS aarch64, release, peak RSS via `/usr/bin/time -l`)

Scratch projects under `/tmp/b593/proj`, each a `WHILE` loop whose body binds the call
with an inline `TRAP` whose handler `CONTINUE`s.

| shape | 20 000 | 40 000 |
| --- | ---: | ---: |
| `net::lookup(<300-char .invalid host>)` fails, `LET List OF net::Address` | 23.2 MB | 40.4 MB |
| same, handler `RECOVER []` | 27.8 MB | 49.6 MB |
| same, handler reads `e.message` | 23.2 MB | 40.4 MB |
| `net::lookup("127.0.0.1")` SUCCEEDS | 33.5 MB | 65.8 MB |
| `tcp::connect("127.0.0.1", 1, 1000)` refused, `RES` | 22.6 MB | 43.7 MB |
| `tls::connect("127.0.0.1", 1, 1000)` refused, `RES` | 54.2 MB | 101.5 MB |
| `fs::open(<missing dir>)` fails, `RES` | 19.2 MB | 37.4 MB |
| **user `FUNC` that `FAIL`s**, bound `RES fs::File` | 17.8 MB | 34.7 MB |
| `fs::readText(<missing>)` fails, `LET String` | 1.0 MB | 1.0 MB |
| `fs::listDirectory(<missing>)` fails, `LET List OF String` | 1.1 MB | 1.0 MB |
| user `FUNC` fails, bound `List OF Integer` / `List OF String` / `List OF <user record>` / `<user record>` | 1.0 MB | 1.0 MB |
| **user `FUNC` fails, bound `List OF net::Address`** | 14.2 MB | 27.3 MB |
| `net::lookup` failure PROPAGATED out of a `FUNC` (caller traps an `Integer`) | 5.9 MB | 6.0 MB |
| `fs::open` failure PROPAGATED out of a `FUNC` | 1.1 MB | 1.0 MB |

At the gate's count, `tcp::connect` refused: 212.9 MB at 200 000, 424.3 MB at 400 000.
3 000 000 failing lookups reached 2 592 MB — linear, no plateau.

`cargo test --release --test rt_scope_drop_leaks -- --test-threads=1` in a detached
worktree of 04c81a605 carrying only the new test file (first-generation `assert_flat`
pins, 200 000 → 400 000): `fs::open` fails 174 → 348 MB, `tcp::connect` refused
203 → 404 MB, user callee into `RES` 161 → 321 MB, succeeding `fs::open` 247 → 494 MB —
all FAILED; the value pin passed. (Those pins were then replaced — see "What is left".)

## Separating the variables

- **Failing vs succeeding:** both grow on `RES` and on `List OF net::Address`
  (success grows for a different, by-design reason — below).
- **TRAPped vs propagated:** propagated is flat (5.9 / 1.0 MB). Only the inline `TRAP`
  grows.
- **Bound vs unbound error:** identical (23.2 → 40.4 MB either way). Not the `e` copy.
- **`net` vs `tcp`/`tls` vs `fs` vs no helper at all:** all grow when the binding type is
  `RES` or `List OF net::Address`; none grows for `String` / `List OF String`. A user
  `FUNC` with `FAIL error(7, "always")` grows exactly like a helper. **The helper is not
  involved.**
- **libc:** a C loop of 400 000 failing (and 200 000 succeeding) `getaddrinfo` calls is flat
  (5.88 MB / 1.39 MB). `vmmap` on the growing process: `VM_ALLOCATE` 152.5 MB, malloc zones
  small — the growth is the MFB arena.

The discriminator is the binding type: exactly the types `type_is_memcpy_copyable` rejects
(a bare resource nominal; a collection of a pointer-`String` record —
`is_pointer_string_record` is `net.Address`, `udp.Datagram`, `audio.AudioDevice`).

## Root cause

The inline-`TRAP` desugar (`ir::lower::lower_inline_trap`) binds
`$trap_resN : Result OF T = CallResult(..)`. Its value is the `{tag, size, payload}` block
`emit_build_result_inline` allocates in THIS frame (`fresh_trapped_result_value` is its only
constructor). On the error path the payload is the whole flat `Error` — message and
`ErrorLoc` inlined — copied in by `emit_trapped_error_result`.

The `Bind` lowering (`builder_control.rs`, `owns_freeable_value`) registers a scope drop only
when `is_freeable_flat_value(Result OF T)`, which requires `type_is_memcpy_copyable(T)`. For a
non-flat `T` nothing registered, so every failing iteration orphaned the wrapper and the
`Error` inside it. Hence every observation: flat in the argument (the argument is not in the
wrapper), sized by the error MESSAGE (so `tls` on macOS, whose message is longer, costs more),
unchanged by bug-573/574/575 (none of them touches the wrapper), absent when propagated (no
wrapper is built).

**Decisive test — scale the message, not the argument** (user callee `FAIL error(7, msg)`,
20 000 iterations):

| binding | 14-char message | 4 014-char message |
| --- | ---: | ---: |
| `RES fs::File`, base | 17.8 MB | 328.7 MB |
| `List OF net::Address`, base | 14.2 MB | 328.8 MB |
| `RES fs::File`, fixed | 8.8 MB | 8.8 MB |
| `List OF net::Address`, fixed | 5.0 MB | 5.0 MB |

## Docs vs code

`.ai/codegen-invariants.md` said "The `$trap_resN` binding always DID get a scope-drop free
(`ResultOf` is a freeable flat value)". **The doc was wrong** for a non-flat `T`: the
`owns_freeable_value` gate is `is_freeable_flat_value(type_)`, and the emitted `main` of the
`List OF net::Address` probe carries no `_mfb_rt_drop_owned_collection` and no wrapper
`arena_free` at all, while the identical `List OF <user record>` probe carries both. The doc
line is corrected on this branch.

## The fix

`ResultWrapperDrop` on `OwnedValueCleanup` (`src/codegen/engine/builder/mod.rs`), registered in
the `Bind` lowering for a `Result OF T` bound from a `CallResult` when `owns_freeable_value` is
false (and the bind is not a union alias, by-ref capture, runtime-managed or promoted vector).
The drop is the existing generic owned-value free — null guard, size from the wrapper's own
size word at +8, `arena_free`, null the slot — so it frees the one wrapper block and never walks
its payload. Which paths it covers is decided by the SAME predicate
`emit_build_result_inline` and `ResultValue` use, `result_payload_is_block(T)`:

- **`Always`** — payload is a word loaded by value (a resource handle): nothing can alias the
  wrapper on either path.
- **`ErrorOnly`** — payload is a block inlined at +16 that `ResultValue` hands out as
  `wrapper + 16`, and `lower_value_owned` does not deep-copy a non-flat `T`, so the Ok binding
  aliases the wrapper. The drop reads the tag at +0 and skips `RESULT_OK_TAG`. On the error
  path the wrapper holds only the `Error`, which every reader copies out (`ResultError` is an
  aliasing source) exactly as it already does for a flat `T`, whose wrapper was always freed.

### Memory gate

1. RED → GREEN: see the tables above and the test names in the header.
2. **Contract:** `mfb spec language memory-semantics` §14 preamble — "Each live value is owned by
   exactly one binding, container slot, **temporary**, …" — and §14.7, which drops live values
   on `CONTINUE`, `FAIL`, `RETURN` and every other scope edge. The wrapper is the TRAP
   temporary's value and had no owner. The fix **only adds a free**: no existing free moves, no
   payload is released, and the one block whose Ok payload a binding aliases keeps its lifetime
   (`ErrorOnly`).
3. **Golden delta: none.** `bash scripts/artifact-gate.sh target/release/mfb all` →
   `1431 tests, 1597 build(s), 2009 golden(s) checked, 0 diff(s)`, exit 0; zero golden files
   changed on the branch, zero `.run`. The zero is coverage-correct, not a blind pass: no
   fixture that owns a `.ncode`/`.ncodesum` golden contains an inline `TRAP` at all (grep over
   every golden-bearing fixture's `.mfb` finds none), and the gate's binary does emit the new
   drop (4 `owned_value_free_skip` sites in the user-`RES` probe's `main`, against 1 on base).
   The fixtures that DO exercise it — `closed-default-drop-rt`, `closed-default-tls-drop-rt`,
   `inline-trap-producer-float-rt`, `tcp-udp-poll-list-trap-rt`, `tls-poll-list-rt`,
   `inline-trap-union-bind-rt` (a `json::Json` result), `native-link-inline-trap-rt`,
   `func_fs_openWithin_valid`, `native-res-state-inline-trap-valid` — run under
   `scripts/test-accept.sh target/release/mfb …` with their `build.log` output compared:
   9 passed, exit 0. Full suite
   `cargo test --release --no-fail-fast -- --skip artifact_gate_all` (gate run separately
   above): cargo exit 0, 161 test binaries, 5 346 passed, 0 failed, 6 ignored.
4. Positive pin `every_failing_resource_call_still_reports_its_error_and_origin`: `RETURN` from
   the handler reading `e`, a `FAIL inner` re-raise (origin stays line 17, the `fs::open`), the
   hoisted chain form (bug-457), a refused `tcp::connect`, a user `FAIL` into `RES`, a
   successful `fs::open` + `readAll`, a successful `net::lookup` read back after churn (the
   `ErrorOnly` Ok alias), and a failing `net::lookup` — 500 iterations × 10 runs, compared with
   the base compiler's output byte for byte. The scratch version of the same program on the
   fixed compiler printed output identical to base (`diff` empty).

## What is left, and why it is not this bug

After the fix the report's shapes still grow, by less: `tcp::connect` refused
81.4 → 161.3 MB (200 000 → 400 000), `fs::open` fails 8.8 → 16.6 MB, `tls::connect`
38.7 → 69.6 MB, `net::lookup` fails 10.1 → 14.1 MB (20 000 → 40 000). Measured, not guessed:

- **`RES` bindings:** the error path allocates the default CLOSED resource record for
  `$trap_valN` (`emit_closed_resource_record`), and the resource drop
  (`emit_resource_block_reclaim`) frees only the I/O buffers. The record is the tombstone
  every alias reads the closed flag from; plan-52-B's Open Decisions explicitly decline to
  reclaim it ("Should the record itself ever be reclaimed? Not here.", measured residual
  ~1.1 KiB per cycle). A SUCCEEDING `fs::open` + `fs::close` loop grows the same way
  (27.1 → 53.0 MB base, 24.7 → 48.1 MB fixed, 20 000 → 40 000). Freeing it moves a lifetime
  — a product/ABI decision, not an added free. This is why the committed RSS pins compare
  message lengths instead of asserting flatness.
- **`List OF net::Address` bindings — a SEPARATE defect, not filed here (numbers race; lead to
  assign):** a list of a pointer-`String` record is not a flat value, so it has NO drop in
  any position. A succeeding `net::lookup("127.0.0.1")` propagated out of a `FUNC` with no
  `TRAP` grows 18.0 → 34.8 MB (20 000 → 40 000); on the error path the default empty list
  `$trap_valN` leaks the same way. Fixing it needs a deep drop over out-of-line host `String`s
  (or flattening `net.Address`, which bug-483 did not do), and its Ok path aliases the
  wrapper — a layout decision. No existing doc records it
  (`grep -rliE "net::Address|pointer-string" bugs/` finds only bug-483's layout doc, which
  says nothing about drops).
