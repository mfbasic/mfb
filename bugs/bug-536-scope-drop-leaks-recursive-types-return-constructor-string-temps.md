# bug-536: three scope-drop leaks in codegen — recursive-type values are never freed, `RETURN <constructor>` abandons the fresh block, String call results consumed by an operator are never freed

Last updated: 2026-09-05
Effort: x-large (1d–3d) — three independent shapes; the recursive-drop one is the large half
Severity: HIGH
Class: Memory-safety / Security (denial of service — unbounded memory growth on ordinary programs; the real amplifier behind audit-3 DEC-03)

Status: **shapes A, B, B-2 FIXED** (B-2: 2026-09-06, `b845db0de`). **Shape C remains, and is NOT a bug fix** — see the ruling below.
**Shape B's NATIVE half FIXED** (2026-09-05, `cd8699103`).
**Shape B-2 (the callee half) FIXED** (2026-09-06, branch `bug-536-shape-b2`) —
a `String` returned by a user / `.mfb`-bodied function may now be freed by its
caller, plus two pre-existing memory-safety defects the licence exposed (a
use-after-free through the `toString` identity, and a rodata pointer escaping
`RETURN <constant-folded String local>`). Shape C is
**larger than this document assumed** — see "Shape C is blocked on recursive
copy-insertion" below, which is a finding, not an excuse. Two *separate*
pre-existing leaks found while measuring B-2 are recorded under "Found while
fixing B-2" and are NOT fixed here.
Regression Test: `tests/rt_scope_drop_leaks.rs` (builds each minimal program at
two iteration counts, reads the child's `ru_maxrss` through
`common::run_bounded_with_rss`, and asserts peak RSS does not grow with the count;
plus a positive behaviour pin per shape). Shape B adds
`an_unbound_string_call_result_runs_at_constant_rss`,
`an_unbound_string_concat_runs_at_constant_rss`,
`unbound_native_string_producers_run_at_constant_rss`,
`a_bound_string_call_result_still_runs_at_constant_rss`, and the positive pin
`every_string_producer_still_produces_the_right_value` — which passes on the
pre-fix compiler too, so it pins the fix and not the bug.

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

## USER DECISION (2026-09-06) — shape C leaves the bug backlog

Ruling: **write a plan-NN design doc for shape C.** Stop calling it a bug.

A value of a recursive type is never freed, and the fix needs recursive
copy-insertion, which does not exist in this compiler; the naive fix is a double
free. That is a design pass, and the doc must be reviewable before any code is
written. It should not be dispatched as a bug fix, and this document should not
keep carrying it as one.

Shapes A, B and B-2 are landed and stay here.


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

### Shape B — attributed precisely (2026-09-04)

Measured on this tree (shape A landed), 400 000 vs 800 000 iterations:

| loop body | 400k | 800k | verdict |
| --- | --- | --- | --- |
| `s = "xy"` (reassign a MUT String to a literal) | 0 MB | 0 MB | flat — the assignment frees the old block |
| `s = toString(i)` (reassign to a call result) | 0 MB | 0 MB | flat |
| `LET t AS String = toString(i)` | 0 MB | 0 MB | flat — the binding owns and frees it |
| `acc = acc + len(toString(i))` | **25 MB** | **50 MB** | leaks 64 B per evaluation |

So shape B is exactly and only **an unbound String call result**: reassignment and
binding both already free correctly.

### Shape B — the NATIVE half is FIXED (2026-09-05)

`register_pending_temp`'s blanket `String` exemption is replaced by **fail-closed
freshness provenance**, design 1 of the two below.

**Mechanism.** A producer that has just `_mfb_arena_alloc`ed the block it is about
to return calls `CodeBuilder::mark_fresh_string(<that operand>)`.
`lower_value` clears the mark before lowering a node and `take()`s it after, and
honours it only when it names **that node's own `ValueResult.location`** — an
operand's own `lower_value` frame already consumed its mark, so what survives was
set by this node's emitter, and the identity test rejects an *interior* block a
lowering allocated but did not return. `register_pending_temp` then frees a bare
`String` temp only with that mark.

**Deviation from the doc's design 1, and why.** The flag is on the **builder**,
not on `ValueResult`. `ValueResult` has 333 construction sites here
(`rg -c 'ValueResult\s*\{' src | ...`, up from the 325 recorded above), and the
shared String producers return a bare `VirtualRegister` — the `ValueResult` is
built by the caller, and rebuilt again by most intermediate wrappers. A struct
field would therefore have been dropped at nearly every site, i.e. fail-closed
**and inert**. The mark survives those rebuilds because it never travels in the
value. The safety asymmetry the design exists for is unchanged and is the whole
argument: **an unmarked producer keeps leaking; nothing unmarked can ever be
freed.** The four existing guards (`value_is_aliasing_source`,
`static_string_value`, `call_returns_rodata_string`, `call_returns_param_borrow`)
are untouched.

**Producers opted in**, each by reading its lowering to the `arena_alloc` it
returns: `emit_materialize_string_from_bytes` (the shared materializer — covers
`collections::get` on a `List OF String`, `strings::trim`/`trimChars`/`left`/
`right`/`stripPrefix`/`stripSuffix`/`graphemeAt`, `fs::pathBaseName`/
`pathExtension`, `toString(Scalar)`), `_mfb_rt_string_concat` (pairwise `&`), the
fused concat chain, `_mfb_rt_int_to_string`, `_mfb_rt_float_to_string`, the
`Fixed`/`Money`/`Scalar` out-of-line renderers, the inline `Fixed`/`Money`
renderers, `toString(List OF Byte)`, `toString(AttributedString)` (its
`copy_flat_block`), `strings::repeat`/`upper`/`lower`/`caseFold`/`title`
(`gen_case_map`)/`normalizeNfc`/`join`/`padLeft`/`padRight`/`mid`/`replace`,
`fs::pathJoin`, `fs::pathNormalize`, `json::sciParts`.

**Producers deliberately NOT opted in** (each would be a wild free):

- `toString(String)` — the `String` arm is the IDENTITY: it hands back its own
  argument. This is the trap the whole design exists for.
- `toString(Boolean)`, `typeName`, every constant fold, and the constant-fold
  early return in `strings::upper`/`lower`/`caseFold`/`normalizeNfc` — rodata.
- `fs::pathDirName` — one arm yields a rodata constant pointer.
- The `io::` readers (`abi_function` bodies; their `ValueResult` is the
  synthesized function's, not the call site's).
- Any user / `.mfb`-bodied function's return (see below).

**A second, separate leak found and fixed in the same change.**
`strings::padLeft(s, n)` / `padRight(s, n)` with the default padChar materialize a
one-byte pad String, copy it into the result and never return it. No
result-shaped rule can reach an *interior* block, so it is handed to the
statement-scope free explicitly (`register_fresh_string_temp`). Measured at
200k/400k calls: 13 MB → 25 MB before, 1 MB → 1 MB after.

**Measured, 400 000 vs 800 000 iterations, before → after:**

| loop body | before | after |
| --- | --- | --- |
| `acc = acc + len(toString(i))` | 25 MB → 50 MB | 1 MB → 1 MB |
| `acc = acc + len("x" & toString(i) & "y")` | 93 MB → 191 MB | 1 MB → 1 MB |
| the 8 native producers together (100k/200k) | 224 MB → 447 MB | 1 MB → 1 MB |
| `LET t = toString(i)` + `strings::padLeft` + `s = toString(i) & "-"` | 50 MB → 99 MB | 1 MB → 1 MB |

### csv's per-field cost IS shape B — the CALLEE half, not a separate defect

The previous revision of this document recorded that "csv has a SECOND leak that
is NOT shape B", reasoning that `__csv_decodeRange`'s
`out & __encoding_fromCodepoint(cp)` runs zero times on an empty field. **That
conclusion is wrong.** The site that leaks is one level up, in `__csv_parse`:

```
row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
```

`__csv_fieldValue` returns a `String`; the call result is never bound, `append`
copies its bytes into the row's payload, and nothing frees the block. That is
shape B exactly — the producer is simply a **user-level (`.mfb`-bodied)**
function rather than a native one, and native provenance cannot see through it.

Reproduced standalone (200k vs 400k iterations, post-fix compiler):

```
FUNC mkEmpty(i AS Integer) AS String
  MUT out AS String = ""
  RETURN out
END FUNC
...
xs = collections::append(xs, mkEmpty(j))          ' 14 MB -> 27 MB   (68 B/iter)
LET fv AS String = mkEmpty(j)                      ' 1 MB  ->  1 MB
xs = collections::append(xs, fv)
```

Binding the result makes it flat; leaving it unbound leaks 68 B per empty
String. `csv::parse` of 1.26 MB of empty fields (63 000 rows x 20 fields =
1 260 000 fields) costs +123 MB per repeat call — 98 B per field including
fragmentation — and is **unchanged by this fix** (544 / 667 / 913 MB at 1 / 2 / 4
calls, byte-identical before and after), which is the expected result for a
native-only provenance fix and confirms the attribution.

### Shape B — the callee-side analysis as first written (superseded)

**Kept for the record; two of its four cases were wrong.** See "Shape B-2 —
FIXED" above for the corrections (measured, not argued). The reasoning at the
time was that a `String` returned by a user / `.mfb`-bodied function cannot be
freed by the caller, because a callee may return a non-fresh block on some path:

1. `RETURN "literal"` — a rodata pointer (`arena_free` on it is SIGBUS).
2. `RETURN <param>` — already handled: `function_returns_param_borrow` (plan-86
   K1) classifies such a call as an aliasing source, so it is neither freed nor
   double-copied.
3. `RETURN toString(s)` where `s` is a `String` parameter — the identity arm
   returns the argument, and case 2's predicate does **not** see through the
   call. This is a live counter-example, verified by measurement: a probe with
   `FUNC ident(s AS String) AS String RETURN toString(s) END FUNC` is flat at
   both counts precisely because nothing is allocated — freeing its result would
   free the CALLER's live `String`.
4. `RETURN <global>` — an alias into a global's block.

The predicate's name and shape were right; the "requiring EVERY `NirOp::Return`
to be provably fresh, with a fixpoint … and a seed set of native producers" part
was not needed and is not what landed — the callee DELIVERS the guarantee with one
copy on the unprovable arm, so there is nothing to seed and nothing to iterate.
The original sketch read:

> `RETURN <owned String local>` is the easy arm (every String bind deep-copies an
> aliasing source, so the local owns its own block and `plan_returned_move` moves
> it), and it alone fixes `__csv_decodeRange`; it does NOT fix `csv::parse`, whose
> leaking site calls `__csv_fieldValue`, which itself returns call results — so the
> transitive form is what the decoders need. Getting it wrong is a double free of
> the caller's live String, not a leak, so it wants its own change with its own
> audit rather than being bolted on here.

Two designs were on the table; design 1 is what landed for natives, and design 2
remains rejected:

1. **Fail-closed provenance** — landed (see above).
2. **An audited allowlist of call targets** — rejected: it has the same
   per-member audit cost with none of the structural proof, and it cannot
   express "this block came from the alloc two lines up".

### Shape B-2 — FIXED (2026-09-06, branch `bug-536-shape-b2`)

`function_returns_fresh_string(f)` is the callee half of the same contract
`function_returns_param_borrow` states for the opposite answer, and the two are
disjoint by construction (the fresh predicate checks the borrow one first and
loses). It admits a function whose declared return is a bare `String`, that has at
least one value return, that is not a param-borrow function, and that is not
callback-referenced. Every call site consults the identical predicate over the
identical `functions` map, so caller and callee cannot disagree.

The callback-referenced exclusion is **conservative, not principled**, and it is
the one place this predicate is knowingly weaker than it should be — see finding
3 under "Found while fixing B-2". Where K1 excludes the same set to FORCE a copy,
excluding it here removes the copy obligation, which leaves a pre-existing HOF
SIGSEGV live. It keeps callback lowering byte-identical, which is why it is here.

**The guarantee is DELIVERED, not observed — which is why no fixpoint and no
native seed set are needed.** The document above proposed "a fixpoint over calls
to other user functions and a seed set of native producers (the
`mark_fresh_string` list)". That seed set is design 2 — the audited call-target
allowlist — wearing a different hat, and it turns out to be unnecessary.
`lower_returned_value` already makes three of its four return shapes fresh:

| return shape | what already happens | fresh? |
| --- | --- | --- |
| move-elided owned local | the block moves to the caller (plan-25-C C1) | yes |
| aliasing source / static string | `copy_flat_block` | yes |
| a claimed pending temp | the producer's own `arena_alloc` (marked, or a B-2 callee) | yes |
| anything else (fall-through) | returned as-is | **unprovable** |

so the callee inserts one `copy_flat_block` on the fourth and only the fourth.
`RETURN <call to another such function>` is then fresh *by this same guarantee*,
and mutual recursion bottoms out at a return that is provably fresh or copied.
Measured over the whole builtin corpus: the inserted copy fires **zero** times —
`artifact-gate all` adds no `_mfb_arena_alloc` in any fixture — because every
shipped `.mfb` body already returns one of the three provable shapes.

**Two pre-existing memory-safety defects the licence exposed**, both fixed here
because B-2 would otherwise have widened their reach (a partial fix that
introduces a double free is not a fix):

1. **The `toString` identity broke the pending-temp chain — a use-after-free.**
   `toString`'s `String` arm returns its own argument's block, but spills the
   argument and reloads it, so the block leaves under a NEW operand. That operand
   *is* the `PendingTemp` identity token `claim_pending_temp` compares against, so
   the owning binding's claim missed: the statement-scope free ran anyway and the
   binding was left holding freed memory. On the pre-fix compiler
   `LET a AS String = toString("x" & toString(i))` in a loop printed the wrong
   text and then **SIGSEGVed** (`[exit 139]`); codegen inspection shows it
   emitting **4** `_mfb_rt_drop_owned_string` calls where the unwrapped
   `LET a AS String = "x" & toString(i)` emits 3 — one extra free for the same
   three blocks. `retarget_pending_temp` moves the token forward and emits
   nothing, because the free reads the temp's *slot*, not its location. Pinned by
   `the_tostring_identity_adds_no_owner`.
2. **`RETURN <constant-folded String local>` returned a RODATA pointer.**
   `MUT out AS String = "abc" … RETURN out` copies the literal into the arena at
   the bind, but every *read* of `out` constant-folds back to `adrp _mfb_str_N`.
   `plan_returned_move` therefore "moved" a read-only constant and orphaned the
   arena copy — a 64 B-per-call leak before, and an immediate **SIGBUS** the
   moment a caller frees it (observed the first time B-2 licensed the free).
   `plan_returned_move` now declines on `static_string_value(value).is_some()`,
   routing the return through the existing `copy_flat_block`.

**What this document got wrong, corrected by measurement:**

- "`RETURN "literal"` — a rodata pointer (`arena_free` on it is SIGBUS)" is
  **false**. `static_string_value` classifies the literal as needing an owning
  copy, so `lower_returned_value` has always `copy_flat_block`ed it and the callee
  has always returned a fresh arena block. The proof is that it *leaked*: 25 MB at
  400 000 calls and 50 MB at 800 000, which a rodata pointer cannot do because
  nothing would have been allocated. It is now freed
  (`a_returned_string_literal_runs_at_constant_rss`).
- "`RETURN <param>` — already handled" is right, but for a subtler reason than
  stated: it is handled only while `function_returns_param_borrow` holds for the
  WHOLE function. A function that returns a param on one path and a fresh block on
  another is not a param-borrow function, and its `RETURN <param>` takes the
  ordinary `copy_flat_block` — so it is fresh, not a borrow. B-2 admits it.
- "`RETURN toString(s)` … is a live counter-example" is right about the aliasing
  and wrong about the remedy: it needs no special case. `toString`'s identity arm
  registers no fresh mark, so the return falls through to the unprovable arm and
  the callee copies it — one alloc per call on a shape that previously allocated
  nothing, which is the price of the guarantee.

**Evidence.**

- **RED → GREEN.** `tests/rt_scope_drop_leaks.rs` gains four cases; run against a
  binary built at the base commit (`MFB_TEST_EXE=<pre-fix mfb>`) all four fail,
  and all four pass after:

  | case | pre-fix | post-fix |
  | --- | --- | --- |
  | `an_unbound_user_string_call_result` (`append(xs, mkEmpty(i))`) | 25 → 50 MB | 0 → 0 MB |
  | `a_transitive_user_string_call_chain` (`top → middle → leaf`) | 25 → 50 MB | 0 → 0 MB |
  | `a_returned_string_literal` (`RETURN "even"`) | 25 → 50 MB | 1 → 1 MB |
  | `tostring_of_a_fresh_string` | **SIGSEGV** | 0 → 0 MB |

- **POSITIVE pins, green on BOTH compilers** (so they pin the fix, not the bug):
  `a_param_borrow_string_callee_still_runs_at_constant_rss`,
  `every_string_return_shape_still_produces_the_right_value` (every return shape,
  every consuming position, exact values), and the whole pre-existing shape-A/B
  suite. The structural pin
  `a_param_borrow_string_callee_result_is_never_given_an_owner`
  (`tests/codegen_string_return_freshness.rs`) is the one that decides the design:
  it asserts that introducing a param-borrow call adds NO owner at the call site,
  i.e. that the new permission does not admit the one thing it must not.
- **A 25-shape differential battery** (every `String` producer and consumer,
  natives and `.mfb` callees, identity wrappers, recursion, mutual recursion, a
  `TRAP`ped fallible callee, `MATCH`-free comparisons) is **byte-identical in
  output** between the pre-fix and post-fix compilers, and the churning form runs
  the same values at 20 000 and 40 000 iterations with peak RSS 138/281 MB before
  and 80/164 MB after.
- **Golden containment.** `artifact-gate all`: 1945 goldens checked, 83 diffs
  across 17 fixtures, 0 after `bash scripts/regen-ncodesum.sh`. Attributed at the
  instruction level (pre vs post `-ncode` for every changed fixture): **zero
  functions added or removed, zero `bl` targets removed, zero new stack-slot
  KINDS, and exactly one added `bl` target everywhere —
  `_mfb_rt_drop_owned_string`** — alongside `pending_temp` slots. Per fixture:
  csv +26 drops/+41 slots, json +29/+73, regex +28/+57, encoding +32/+46,
  http +16/+81, tls +18/+52, crypto +18/+30, datetime +15/+46, strings +18/+28,
  resource-xfer-slots +16/+24, vector +123/+123, net +2/+25, tcp/udp/term +1 each.
  No `_mfb_arena_alloc` added anywhere — the callee-side copy never fires in the
  shipped packages. Every fixture with no `.mfb`-bodied `String` producer
  (`collections`, `math`, `money`, `bits`, `os`, `process`, `fs`, `io`, `audio`,
  `thread`, `general`) is byte-identical.
- **Semantics.** `mfb spec language memory-semantics` **§14.3** — *"Returning a
  value moves it into the caller's return slot"* — and **§14.3.1**, whose native
  heap-value contract is the exact sentence this realizes: *"copies are
  independent, returns never point into a shorter-lived frame or arena"*. A
  `RETURN` that hands back a rodata constant or the caller's own argument block
  violates the second clause; `function_returns_fresh_string` is that clause
  written as a predicate. §14.6's *"Reads produce owned values, not aliases into
  the buffer"* is what keeps the `collections::get` borrow and the param-borrow
  callee OUT of it. Every change is an added check, an added copy, or a moved
  identity token; no value's lifetime or identity changes, and
  `retarget_pending_temp` emits no instruction at all.
  (Note: the shape-A entry above cites §14.2 for the same "returning a value
  moves it" sentence — the sentence is in §14.3; §14.2 is assignment and
  initialization.)
- **Decoder movement.** `csv::parse` of 1.26 MB of empty fields (1 260 000
  fields), at 1 / 2 / 4 calls: 853 / 1088 / 1558 MB before, 731 / 842 / 1066 MB
  after — **235 MB → 112 MB per repeat call**. The residual is two separate
  pre-existing leaks (findings 1 and 2 below), not shape B-2.

### Found while fixing B-2 — three SEPARATE pre-existing defects, not fixed here

All three reproduce unchanged on the base commit and are unaffected by B-2. The
first two are the whole of `csv::parse`'s residual 112 MB per repeat call, so
anyone tracking DEC-03 should not attribute that to B-2. Each wants its own
bug number; they are recorded here rather than filed so the numbering does not
race with a peer session.

1. **`s = s & <expr>` on a `MUT String` leaks ~190 B per evaluation.** The
   document's own shape-B table records `s = toString(i)` as "flat — the
   assignment frees the old block"; a **self-append** is not. Minimal repro, flat
   `SUB main`, no functions involved:

   ```
   MUT out AS String = ""
   out = out & "a"          ' 38 MB at 200k, 75 MB at 400k
   ```

   `MUT out AS String = ""` alone is flat at both counts, so it is the assignment.
   This is the in-place string self-append path (`prescan_string_self_appends` /
   `string_capacity_slots`). It is what `__encoding_utf32Decode` and
   `__csv_decodeRange` are built out of (`out = out & __encoding_fromCodepoint(cp)`,
   once per scalar), and `s = s & ch` is the idiom `.ai` recommends for
   performance — so this is the hottest leaking line in the tree.
2. **A `Result OF T` bound through `TRAP` is never freed — type-independent.**
   `LET n AS Integer = fallibleFn(i) TRAP … END TRAP` in a loop leaks **128 B per
   call** (25 MB at 200k, 50 MB at 400k); `String` leaks 64 B, `List OF Integer`
   256 B. The `TRAP` desugar binds `$trap_resN AS Result OF T = callResult …` and
   that binding gets no scope-drop free. Every fallible call in an expression goes
   through it, which is every `__csv_fieldValue` call in `csv::parse`.

3. **A `String`-returning function used as a CALLBACK whose body is the
   `toString` identity SIGSEGVs.** Fourteen lines, identical on the base commit
   and after this change:

   ```
   IMPORT io
   IMPORT collections
   FUNC identish(s AS String) AS String
     RETURN toString(s)
   END FUNC
   SUB main()
     MUT xs AS List OF String = []
     MUT k AS Integer = 0
     WHILE k < 3
       xs = collections::append(xs, "n" & toString(k))
       k = k + 1
     END WHILE
     LET c AS List OF String = collections::transform(xs, identish)   ' [exit 139]
     io::print("c=" & collections::get(c, 0))
   END SUB
   ```

   Mechanism: the `FunctionRef` ABI **owns and frees** the callback's return value
   (that is exactly why plan-86 K1 excludes callback-referenced functions from the
   param-borrow elision — the exclusion FORCES the copy). `identish` is not a
   param-borrow function (it returns a `Call`, not a bare `Local`), so K1's forced
   copy does not apply to it, and `toString`'s identity arm hands the HOF the
   caller's own list-element block, which the HOF then frees. B-2 does not fix it
   because `function_returns_fresh_string` **excludes** callback-referenced
   functions, and that exclusion is deliberately conservative here — it keeps
   callback lowering byte-identical rather than changing a second ABI in this
   change. **The fix is one word:** drop the `callback_referenced` arm from
   `function_returns_fresh_string`, which turns the exclusion from "no obligation"
   into "the callee copies", i.e. exactly what K1's exclusion achieves for the
   borrow shape. It needs its own change with its own callback-ABI audit.

A fourth, already recorded here, also still stands: Phase 2's second checkbox —
`RETURN f(g(i))` clears rather than drops `g`'s interior temp
(`clear_pending_temps_to`), so `RETURN "v" & toString(i)` leaks 64 B per call even
when the caller binds the result. Measured identical pre and post.

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

**Shape B** — `register_pending_temp` returned early for `result.type_ == String`
("a standalone String produced by a call may be a shared rodata constant … or a
non-owned view … String temps therefore leak until scope exit", plan-25). They
did not leak "until scope exit" — nothing tracked them, so they leaked for the
life of the process. FIXED for native producers by the freshness mark above; a
producer with no mark still takes the early return, which is why the user-callee
half (shape B-2) remains. A bound String (`LET s = toString(i)`) is owned by the binding's
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

- [x] Audit and opt in the **native** String producers one at a time, behind
      fail-closed provenance (`mark_fresh_string`). 23 producers opted in, 5
      classes deliberately excluded; `strings::padLeft`/`padRight`'s interior pad
      String freed via `register_fresh_string_temp`.
- [x] **Shape B-2, DONE:** callee-side `function_returns_fresh_string` so a
      `String` returned by a user / `.mfb`-bodied function may be freed by its
      caller. See "Shape B-2 — FIXED" below.

Acceptance: the four shape-B RSS tests pass (they do); `csv::parse` ×2 ≈ ×1 + rows
(HALVED — 235 MB → 112 MB per repeat call; the residual is two *separate*
pre-existing leaks, see "Found while fixing B-2").
Commit: (branches `bug-536-shape-b`, `bug-536-shape-b2`)

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
