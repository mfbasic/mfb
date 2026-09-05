# bug-536: three scope-drop leaks in codegen — recursive-type values are never freed, `RETURN <constructor>` abandons the fresh block, String call results consumed by an operator are never freed

Last updated: 2026-09-04
Effort: x-large (1d–3d) — three independent shapes; the recursive-drop one is the large half
Severity: HIGH
Class: Memory-safety / Security (denial of service — unbounded memory growth on ordinary programs; the real amplifier behind audit-3 DEC-03)

Status: **Shape A FIXED** (2026-09-04). Shapes B and C remain open, and shape C is
**larger than this document assumed** — see "Shape C is blocked on recursive
copy-insertion" below, which is a finding, not an excuse.
Regression Test: `tests/rt_scope_drop_leaks.rs` (added — builds each minimal
program at two iteration counts, reads the child's `ru_maxrss` through
`common::run_bounded_with_rss`, and asserts peak RSS does not grow with the count;
plus a positive behaviour pin per shape)

Three distinct codegen shapes leave arena blocks that are never freed, so a
program that evaluates them in a loop grows without bound. None is an aliasing
or ownership *error* — nothing is freed twice or used after free — every one is a
block that has exactly one owner and that owner never frees it. The single
correct behaviour a fix produces: **a loop that binds, returns or concatenates a
value of any type runs at constant peak RSS**, exactly as a loop over a flat
record literal already does.

Why this is HIGH and not a benchmark nit: the three shapes are exactly what the
untrusted-input decoders are made of. `json::parse` allocates a `json::Json`
(recursive union, shape C) and returns a `__json_Node[...]` literal (shape A) per
value; the regex matcher allocates a `__regex_Cont` (recursive union, shape C) per
step and returns `__regex_Result[...]` literals (shape A); `csv::parse` does
`out = out & __encoding_fromCodepoint(cp)` (shape B) per scalar. Measured with the
`mfb` at `fec4ceddc` (main + bug-509): parsing the same 2 MB JSON array **twice**
adds 194 MB the second time (485 B per element, never reclaimed);
`regex::findAll` over a 100 000-character subject leaks ~200 MB **per call**;
`csv::parse` of a 1.2 MB file leaks ~83 MB **per call**. audit-3 DEC-03 recorded
"1.2 MB → 1.05 GB" and attributed it to per-element collection overhead; the
collections are linear and reusable (a second identical list build adds 8.5 MB,
not 27 MB) — the non-reusable part is these leaks. A server that parses one JSON
body per request leaks every body.

References:

- `planning/completed/plan-02-flat-values.md` §2 (`note-1`): the flat layout is
  what makes the generic `arena_free` sound, and recursive types were left out of
  it — "everything else leaks pointers". bug-391 later added the per-type deep
  *copy* for those types (`thread_copy_symbol`); the deep *free* was never added.
- `planning/completed/plan-25-*` (temp lifetimes): `register_pending_temp`'s
  deliberate `String` exemption is shape B's origin.
- `.ai/collections.md` "get read-only borrow" (plan-86 E): the one class of value
  a fix must keep NOT freeing.
- Found during `bugs/bug-510-text-decoder-dos-cluster.md` (DEC-03); the decoder
  measurements above are its reproduction.

## Failing Reproduction

Each program is complete. Build with `mfb build <dir>` and run at two iteration
counts; the second column is `maximum resident set size` from `/usr/bin/time -l`
(macOS; `\time -v` on Linux). A leak-free loop reads the same at both counts.

Shape C — a value of a recursive type is never freed (128 B per iteration for a
one-word union; 256 B for a record holding `List OF <itself>`):

```
IMPORT json
IMPORT collections
IMPORT os

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

SUB main()
  LET n AS Integer = toInt(collections::get(os::args(), 0))
  MUT i AS Integer = 0
  WHILE i < n
    LET v AS json::Json = json::JsonNull[NOTHING]   ' 51 MB at 400k, 102 MB at 800k
    ' LET nd AS Node = Node[kids := [], tag := i]  ' 103 MB at 400k, 204 MB at 800k
    i = i + 1
  END WHILE
END SUB
```

Shape A — `RETURN <record constructor>` (or `RETURN <call returning a record>`)
leaks the fresh block (64 B per call); `LET r = ...; RETURN r` and
`RETURN [list literal]` do not:

```
TYPE Plain
  value AS Integer
  index AS Integer
END TYPE

FUNC mkLit(i AS Integer) AS Plain
  RETURN Plain[i, i]                  ' leaks: 26 MB at 400k calls, 51 MB at 800k
END FUNC

FUNC mkLocal(i AS Integer) AS Plain
  LET r AS Plain = Plain[i, i]
  RETURN r                            ' does not leak: 1 MB at both counts
END FUNC
```

Shape B — a `String` produced by a call and consumed by an operator (not bound)
is never freed (64 B per evaluation); binding it first does not leak:

```
LET s AS String = "x" & toString(i)   ' leaks: 26 MB at 400k, 51 MB at 800k
LET t AS String = toString(i)         ' does not leak
```

- Observed: RSS grows linearly with the iteration count in every "leaks" line.
- Expected: constant RSS, as the "does not leak" lines already show.

Contrast cases that are immune (measured, 1 MB at both counts): a flat record
literal bound in a loop (`LET r AS Rect = Rect[w := 1.0, h := 2.0]`), a flat
data union bound in a loop (`UNION Shape / Circle / Rect`), a list literal bound
in a loop, `RETURN` of a named local, `RETURN` of a list literal, a fresh call
result consumed *inside* a bound constructor (`LET p = Duo[mkLocal(i), i]` — the
interior temp is dropped at statement end), an Integer-returning call. NOT
immune, and part of shape A: `RETURN mkLocal(i)` — a fresh **call** result of
record type returned directly leaks the same 64 B per call as a constructor
(measured: 26 MB at 400k, 51 MB at 800k), and `LET q = mkLocal(i); RETURN
Duo[q, i]` leaks 128 B per call (the `Duo` block; `q` itself is freed).

Whole-decoder measurements (same binary; `/tmp/spk510` probes):

| Program | Input | RSS, 1 call | RSS, 2 calls | RSS, 4 calls |
| --- | --- | --- | --- | --- |
| `json::parse` | `[true,true,…]` ×400 000 (2 MB) | 359 MB | 553 MB | 942 MB |
| `csv::parse` | 1.2 MB of empty fields | 191 MB | 274 MB | 356 MB |
| `regex::findAll(s, "a")` | 100 000 × `a` | 308 MB | 507 MB | 706 MB |

The per-call increments (194 / 83 / 200 MB) are the leaked part; the first-call
excess over them is geometric-growth garbage the arena does reuse.

## Progress (2026-09-04)

### Shape A — FIXED

`lower_returned_value` (`engine/control/builder_exits.rs`) now claims the
statement's pending temp when the returned value **is** that temp and reports it
`already_standalone = true`. A registered pending temp is by construction a fresh
standalone arena block (`register_pending_temp` requires
`!value_needs_owning_copy`, freeable-flat, not runtime-managed, not a borrowed
`get`, not a bare `String`), so the `materialize_inline_value_in_arena` copy
`store_pending_success_result` used to make was redundant as well as leaky. One
block, one owner. This is plan-25-C C1's `move_elided` reasoning reached for a
fresh temp instead of an owned local.

Evidence:

- **RED → GREEN.** `tests/rt_scope_drop_leaks.rs`'s two shape-A cases fail on the
  pre-fix compiler with exactly the reported numbers — `25 MB → 50 MB` between
  400 000 and 800 000 iterations for both `RETURN Plain[i, i]` and
  `RETURN mkLocal(i)` — and pass after. The two positive pins
  (`RETURN <owned local>` stays flat; every RETURN shape still yields the right
  value) pass on **both** compilers, so they pin the fix rather than the bug.
- **Golden containment**, pre vs post binaries at instruction level: every changed
  fixture removes N × {`bl _mfb_arena_alloc` + its `_mfb_make_error_result` /
  `_mfb_rt_park_error` OOM path} and the `inline_value_source` / `inline_value_size`
  / `inline_value_result` stack-slot triples that drove it, and adds **zero** new
  `bl` targets and zero new slot kinds (json 5 sites, csv 4, term 24). The rest of
  each fixture's diff is the mechanical stack-offset renumbering the removed slots
  cause. `artifact-gate all`: 71 `.ncodesum` diffs across 15 fixtures, all of them
  fixtures that `RETURN` a fresh record/union; 0 after regeneration.
- **Semantics.** `mfb spec language memory-semantics` §14.2 — *"Returning a value
  moves it into the caller's return slot"* — is what this now does. No value's
  identity changes (the caller receives the constructor's own block rather than a
  byte-copy of it), and no user-visible free moves.

### Shape A does NOT move either headline decoder number

Measured on this machine, pre vs post shape A, identical to the megabyte:

| probe | pre | post |
| --- | --- | --- |
| `spikes/audit-3/DEC-03` (800 KB JSON) | 735 MB | 735 MB |
| `csv::parse` of 1.2 MB of empty fields, ×1 / ×2 | 538 / 655 MB | 538 / 655 MB |

So the decoder amplification is shapes **B and C**, not A. Shape A is a real
per-call leak (64 B per `RETURN <constructor>`) and worth having, but anyone
tracking DEC-03 should not expect it to move until C lands.

### Shape C is blocked on recursive COPY-insertion, which does not exist

The Fix Design's Phase 3 ("emit a per-type recursive drop … register `OwnedValue`
cleanups for those types") **cannot be landed on its own**: it would convert the
leak into a double free.

plan-02 note-1's "everything else leaks pointers" is not only about the *drop*
side. There is no recursive **copy** on an owning store either, so every recursive
value in a program is *shared*, and freeing any one owner dangles the others.
Verified by codegen inspection, not inference — this program:

```
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
SUB main()
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = Node[kids := [a], tag := 2]     ' a into a list literal
  MUT xs AS List OF Node = []
  xs = collections::append(xs, a)                  ' a into a growable list
  LET c AS Node = a                                ' a into another binding
  ...
END SUB
```

emits **zero** `_mfb_thread_copy_*` calls in `_mfb_fn_main` (the only two in the
module are inside the emitted copy function itself). The collection payload writer
(`emit_..._payload` in `collection/layout/builder_collection_layout.rs`) copies an
inline record/union payload with `emit_copy_bytes` — the recursive field's pointer
word verbatim — and `lower_value_owned` skips its copy for the class because
`is_freeable_flat_value` is false. So `b.kids[0]`, `xs[0]` and `c` all point at
`a`'s `kids` block.

A correct shape C is therefore: **recursive copy-insertion at every owning store**
(bind, assign, global, return, record-field construction, union wrap, collection
insert/set/literal, closure capture) **and then** the per-type recursive drop, with
the two proven inverse. That is a much larger project than "mirror
`thread_copy_symbol`", it changes the codegen and the performance of every
recursive-typed program, and getting the symmetry wrong is arena corruption rather
than a leak. It should be planned (`write-plan`) rather than attempted as a bug
phase. bug-538's fix is the first piece of it: `collections::get` now deep-copies,
so the READ side of the class is already independent.

### Shape B — remains open; the audit is the work

`register_pending_temp`'s `String` exemption is sound only because a String call
result's provenance is unknown. The four existing guards
(`value_is_aliasing_source`, `static_string_value`, `call_returns_rodata_string`,
`call_returns_param_borrow`) already exclude the known non-fresh producers, so what
is left is the **native** String-returning lowerings: 49 members declare
`return_type: ParameterType::String` with an `abi_inline`/`abi_function` body (26
more are `.mfb`-bodied), plus the generic/overloaded producers (`toString`,
`collections::get` on a `List OF String`). Each has to be read and proven to return
a fresh arena block before it can be opted in; freeing a rodata pointer is a SIGBUS
and freeing a view into an argument corrupts the free list.

Two designs, both sound, neither attempted here:

1. **Fail-closed provenance** — a `fresh` flag on `ValueResult` set by the shared
   String emitters (`emit_materialize_string_from_bytes`, `copy_flat_block`, the
   concat/`toString` runtime helpers), with `register_pending_temp` gating on it. A
   lowering that loses the flag keeps leaking; it can never wild-free. Cost:
   `ValueResult` has **325 construction sites**, so the field addition is the bulk
   of the work.
2. **An audited allowlist of call targets**, extended one member at a time, seeded
   with the decoders' producers. Cheaper, and it is what the Fix Design already
   proposes; the audit is still per-member.

## Root Cause

**Shape C** — `src/codegen/engine/value/builder_values.rs:is_freeable_flat_value`
requires `type_is_memcpy_copyable`, which is false for any type where
`type_participates_in_cycle` holds (`builder_collection_layout.rs`). Both the
`LET` bind (`builder_control.rs:822`, `:1018`, which push the
`ActiveCleanup::OwnedValue`) and `register_pending_temp` gate on that predicate,
so a value of such a type gets **no** scope-drop cleanup and **no** statement-scope
temp free — the comment on `is_freeable_flat_value` says so: "recursive/non-flat
composites … are never freed by the generic owned-value path". There is no other
path: `src/codegen/cleanup/` has no recursive drop, only the flat
`emit_owned_value_drop` and the closure/thread drops. Affected types in the
builtins (found by searching each package's `mod.rs` for a union referenced by
its own variants or a record referenced by its own fields): `json::Json`
(`JsonArr` holds `List OF Json`), `__regex_Node` and `__regex_Cont` (every
`nxt`), `canvas::DrawItem` (a group holds `List OF DrawItem`), `http::Stream`
(candidate — verify), and every user `TYPE` that mentions itself.

**Shape A** — `src/codegen/engine/control/builder_exits.rs:lower_returned_value`
returns `(lowered, already_standalone = false)` for a fresh value that is not an
aliasing source. `emit_return_exit_inner` then treats a record/union result
(`inline_collection_payload_size(..).is_some()`) as possibly-inlined and copies
it into a **second** block with `materialize_inline_value_in_arena`. The original
constructor block was registered as a pending temp by `lower_value`, but the
statement lowering of a control transfer calls `clear_pending_temps_to`
(`builder_control.rs:1517`), which *discards* the pending frees "because a
returned temp is moved to the caller" — true only when the temp *is* what is
returned, which after the re-materialisation it is not. `RETURN r` (a local) is
immune because `plan_returned_move` moves the block (`move_elided`, standalone);
`RETURN [i, i]` is immune because a collection has no inline payload size and is
returned as-is. A fresh **call** result of record type takes the same
re-materialisation path as a constructor and leaks identically (measured above);
the shape is therefore "RETURN of any fresh, non-local record/union value".

**Shape B** — `register_pending_temp` returns early for `result.type_ == String`
("a standalone String produced by a call may be a shared rodata constant … or a
non-owned view … String temps therefore leak until scope exit", plan-25). They
do not leak "until scope exit" — nothing tracks them, so they leak for the life
of the process. A bound String (`LET s = toString(i)`) is owned by the binding's
`OwnedValue` cleanup and freed; an unbound one consumed by `&`, a comparison, a
call argument or a `MATCH` never is. In the decoders: `csv/helper_decode_range.rs`
(`out & __encoding_fromCodepoint(cp)`, once per scalar),
`regex/helper_char_eq.rs` (`strings::caseFold(a) = strings::caseFold(b)`, two
per folded compare), `json/helper_collect_number.rs` (`acc & ch` where `ch` is an
owned `collections::get` copy).

## Goal

- The three minimal programs above run at constant peak RSS at 400 000 and
  800 000 iterations (a new `tests/rt_scope_drop_leaks.rs` asserts it via the
  child's `ru_maxrss`).
- `json::parse` of the same document twice costs no more than once plus the
  document's own tree; `regex::findAll` and `csv::parse` likewise.
- Every existing behavioural test stays green; `artifact-gate all` returns to 0
  diffs after regeneration; the byte-identity delta is confined to the RETURN /
  drop / temp-free sites.

### Non-goals (must NOT change)

- Value semantics: every owner still owns an independent block; a fix must not
  introduce sharing to avoid copies.
- The plan-02 flat layouts, the `.mfp` format, the union `{tag, size, block}`
  layout, the collection header layout.
- The plan-86 E `collections::get` **borrow** (an alias into the container) must
  stay unregistered — freeing it corrupts the free list. Any new "free a String
  call result" rule must prove the result is a fresh block first.
- The plan-25-C C1 move of `RETURN <owned local>` (one free total) must not gain
  a second free.
- Forbidden wrong fixes: making `is_freeable_flat_value` return true for a
  recursive type without a recursive drop (a shallow `arena_free` of the outer
  block leaks the children and mis-sizes the free); "fixing" shape A by copying
  in the caller instead (moves the leak); silencing the regression test by
  raising its threshold.

## Blast Radius

- Shape C, every binding/temp of a cycle-participating type — fixed by this
  bug: `json::Json` (all of `json/`), `__regex_Cont`/`__regex_Node` (the whole
  matcher and compiler in `regex/`), `canvas::DrawItem` (every scene list a
  program presents — latent until measured; `canvas::present` deep-copies the
  scene, so the caller's list is the leaked one), `http::Stream` (verify it is
  recursive at all), user recursive `TYPE`s.
- Shape A, every `RETURN <RecordConstructor>` / `RETURN <UnionConstructor>` —
  fixed by this bug. In the builtin `.mfb` bodies alone: `RETURN __json_Node[…]`
  (×~12), `RETURN __json_StringNode[…]`, `RETURN __regex_Result[…]` (×~8,
  incl. `__regex_fail`), `RETURN __regex_Parse[…]` (×~20), `RETURN
  __canvas_…[…]`, `RETURN CsvRow[…]`, and every user function written this way.
- Shape B, every unbound String call result — fixed by this bug once String
  results carry provenance; until then each site is latent. The three decoder
  sites above are the measured ones; `grep -n '& [a-z_]*::[a-zA-Z]*(' src/codegen/builtins/*/*.rs`
  enumerates ~200 more.
- Unaffected: flat records/unions/collections bound or returned as locals (the
  contrast cases), scalars, resources (their own close path), closures (plan-77
  M6 drop).

## Fix Design

Three independent changes, landed as three commits with their own tests, in
this order (smallest, most local first):

1. **Shape A** (`builder_exits.rs`): in `lower_returned_value`, when the lowered
   value is the most recently registered pending temp (`pending_temp_frees.last()
   .location == lowered.location`), claim it and return `already_standalone = true`
   — a fresh Constructor/Call block is already standalone, so the
   re-materialisation copy was redundant as well as leaky. Then make the RETURN
   path *drop* (free) the remaining interior temps above the statement watermark
   before the branch, after the returned value has been parked in its result slot
   (the arena-free calls clobber caller-saved registers, so the register-path
   variant must park too). Expected output shift: every RETURN of a constructor in
   every fixture — regenerate all `.ncodesum` goldens.
2. **Shape C** (`src/codegen/cleanup/owned/` + `engine/builder/mod.rs`): emit a
   per-type recursive drop function for every member of
   `recursive_transfer_types` (mirroring `thread_copy_symbol`'s emission loop):
   records free their inlined-collection children's element graphs, unions tag-
   dispatch to the variant, collections walk their elements, then the block itself
   is freed with its self-describing size. Register `OwnedValue` cleanups and
   pending temps for those types, dispatching to the symbol (extend
   `OwnedValueCleanup` the way `closure_captures` did). Correctness risk
   concentrates in copy/free symmetry: every owner must hold a *distinct* graph
   (the runtime copy already guarantees this for binds and transfers; audit
   `collections::get` borrows and `MATCH` scrutinee borrows, which must stay
   unfreed).
3. **Shape B** (`builder_values.rs`, `builder_exits.rs`, native String
   lowerings): give String results provenance so the plan-25 exemption can be
   lifted for fresh ones. The narrowest sound version: `lower_returned_value`
   copies a `NirValue::StaticString` return into a fresh block (one small alloc
   per such RETURN), after which every `.mfb`-bodied function returns a fresh
   String and its call results may be registered as temps; native intrinsics
   are then audited one by one (`toString`, `strings::*`, `collections::get`
   on `List OF String`, `encoding::*`) and opted in as their lowering is proven
   to return a fresh block. Rejected: freeing String call results without the
   audit (a rodata or view free is a wild `arena_free`).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt_scope_drop_leaks.rs`: one case per shape (C union, C record,
      A, B) plus the whole-decoder cases (json twice, csv twice, regex twice);
      each builds the program, runs it at n and 2n, and asserts
      `ru_maxrss(2n) - ru_maxrss(n) < 4 MB`. Confirm all fail today.
- [ ] Complete the recursive-type census (is `http::Stream` recursive?) and the
      shape-B site list; record verdicts above.

Acceptance: the new tests fail for the documented reason; the census is complete.
Commit: —

### Phase 2 — shape A

- [x] `lower_returned_value`: claim-and-standalone for a fresh pending temp.
- [ ] RETURN path drops (not clears) interior pending temps before the exit.
      **Deliberately not done.** This is a second, smaller leak (`RETURN f(g(i))`,
      where `g`'s temp is interior) and it needs the returned value parked before
      the frees clobber caller-saved registers, plus a watermark
      `emit_return_exit_inner` does not currently carry. Left for the same change
      that does shape B, where the temp machinery is being touched anyway.
- [x] Regenerated `.ncodesum` goldens under bash; the delta is proven RETURN-site
      only by an instruction-level pre/post attribution (removed `arena_alloc` +
      `inline_value_*` slots, zero added `bl` targets).

Acceptance: the shape-A tests pass; full suite green.
Commit: (see below)

### Phase 3 — shape C

- [ ] Per-type recursive drop emission + cleanup/temp registration for
      cycle-participating types.
- [ ] Decoder measurements: `json::parse` ×2 ≈ ×1 + tree; regex/canvas likewise.

Acceptance: the shape-C tests pass; every json/regex/canvas suite green; no
double-free under `MFB_ARENA_POISON`-style churn fixtures.
Commit: —

### Phase 4 — shape B

- [ ] Static-string return copy; lift the String exemption for `.mfb` call
      results; audit and opt in native String producers one at a time.

Acceptance: the shape-B test passes; `csv::parse` ×2 ≈ ×1 + rows.
Commit: —

### Phase 5 — regenerate expected outputs + full validation

- [ ] `regen-ncodesum.sh` + `regen-outside-ncode.sh` under bash;
      `artifact-gate all` → 0 diffs; full `cargo test --no-fail-fast`.
- [ ] Re-run the decoder measurements and record them here.

Acceptance: full suite green; the reproduction programs are flat at both counts.
Commit: —

## Validation Plan

- Regression tests: `tests/rt_scope_drop_leaks.rs` (RSS-flat assertions per shape).
- Runtime proof: the whole-decoder table above re-measured; `spikes/audit-3/DEC-03`.
- Doc sync: `.ai/collections.md` (the "String temps leak until scope exit"
  sentence becomes false), `.ai/codegen-invariants.md` (recursive drop symmetry
  with `thread_copy_symbol`).
- Full suite: `cargo test --no-fail-fast -- --skip artifact_gate_all` and
  `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Shape B's scope: lift the exemption only for `.mfb`-bodied callees (safe after
  the static-string copy) vs. auditing every native String producer now.
  Recommended: `.mfb` callees first (covers the decoders), natives per audit.
- Whether the arena's non-reuse of geometric grow chains (a first list build
  costs ~8× its final size; later builds ~2.6×) deserves its own bug. It is not
  a leak — a second identical build reuses the garbage — so it is out of scope
  here.

## Summary

The engineering risk is in shape C (a new recursive drop must be the exact
inverse of the existing recursive copy, and must skip borrows) and in the
tree-wide golden regeneration shape A forces; shape B is an audit. Untouched:
layouts, value semantics, every flat type's existing free path, the borrow rule.
