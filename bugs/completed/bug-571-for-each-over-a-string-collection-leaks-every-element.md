# bug-571: `FOR EACH` over a `List OF String` / `Map OF String TO …` leaks one block per element per pass

Last updated: 2026-09-07
Effort: small-to-medium (one materialisation site; the ownership proof is the work)
Severity: **HIGH** (unbounded leak on the most ordinary loop in the language)
Class: Memory / collection iteration

Status: **FIXED** (2026-09-07, `4bd89ee76`)
Regression Test:
- `tests/runtime/rt_scope_drop_leaks.rs` — seven RSS cases at N and 2N
  (`a_for_each_over_a_list_of_string_does_not_leak_its_element`,
  `…a_map_of_string…`, `…a_set_of_string…`,
  `a_for_each_over_fixed_width_elements_stays_flat` (the contrast),
  `an_early_exit_from_a_for_each_does_not_leak_the_item`,
  `returning_the_loop_item_neither_leaks_nor_double_frees`,
  `a_for_each_body_that_uses_the_item_does_not_leak_it`), plus
  `every_for_each_body_shape_still_produces_the_right_value` — 25 runs over
  thirteen body shapes, which is the half a leak test cannot see.
- `tests/codegen/codegen_for_each_item_drop.rs` — five comparative owner counts.
  Four were RED before the fix; the fifth
  (`an_aliasing_element_is_never_given_an_owner`) is the positive pin and was
  green both ways.

Found while fixing bug-569, as the residual under `collections::mapValues`. It is
NOT that bug: it has no callback in it at all.

## Failing reproduction — twelve lines, no callback, no HOF

```
IMPORT io

SUB main()
  LET xs AS List OF String = ["n0", "n1", "n2", "n3", "n4", "n5", "n6", "n7"]
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 50000
    FOR EACH e IN xs
      acc = acc + len(e)
    NEXT
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`), macos-aarch64, release:

| N (outer passes) | peak RSS |
|---|---|
| 50 000 | 25 MB |
| 100 000 | 50 MB |

It doubles with the count: one leaked block per element per pass, and the loop
body only *reads* `e`. The same loop over a `Map OF String TO String` reading
`e.key`/`e.value` is 50 MB -> 99 MB.

The contrast that says it is the `String`, not the loop: `FOR EACH` over a
`Map OF Integer TO Integer` reading `e.key` and `e.value` is **1.0 MB flat** at
both counts.

## Root cause (suspected — reproduce before trusting it)

A packed `String` element has no standalone header to point at, so iteration
materialises a fresh arena block per element
(`emit_load_collection_payload`'s `String` arm, via
`emit_materialize_string_from_bytes`). The HOF loops free that block after the
callback returns — `free_collection_loop_item`, whose whole reason to exist is
this (bug-307) — but the `FOR EACH` lowering has no equivalent: the loop variable
binding goes out of scope each iteration and nothing drops the block it holds.

That the HOF loops are flat is the evidence: `collections::filter(xs, short)` with
a named `FUNC(String) AS Boolean` over the same list is 1.0 MB at both counts, and
it walks exactly the same data with exactly the same materialisation.

## What a fix must produce

The loop above runs at constant RSS, `FOR EACH` over a `Map OF Integer TO Integer`
stays flat, and — the direction of danger, since the fix ADDS a free — the loop
variable is not freed on any path that still owns it: `FOR EACH` bodies that
`EXIT FOR`, `CONTINUE FOR`, `RETURN`, `FAIL`, or auto-propagate, and a body that
stores `e` into a collection or returns it (the store copies, so the block is
still the loop's to drop, but that has to be measured, not assumed).

Read bug-569's soundness section first: it is the same class of change (adding a
free), and `collections::reduce`'s runtime pointer-identity guard is the model for
the shapes where the block might have another owner.

## The fix (2026-09-07)

`lower_for_each` (`src/codegen/engine/control/builder_control.rs`) now registers
the materialised payload as an ordinary `ActiveCleanup::OwnedValue` obligation of
the loop BODY's own cleanup scope, which it opens itself instead of letting
`lower_loop_body` open it. That is the whole fix: §14.7 already says *"At normal
scope exit, `RETURN`, `EXIT FOR`/…, `CONTINUE FOR`/…, `FAIL`, `PROPAGATE`, or
auto-propagated errors, live bindings are dropped in reverse declaration order
within each scope"*, and every one of those edges already has an emitter
(`lower_ops_inner`'s tail, `emit_cleanup_branch_to_depth`,
`emit_current_result_exit`). The loop variable was simply never enrolled — not
excluded by a rule, just never registered, the same gap `NirOp::Trap`'s caught
`Error` had before bug-151.

### The shape enumeration

`lower_for_each` has FOUR materialisation sites, not one, and only the `String`
arm of `emit_load_payload_with_stride` allocates:

| # | Arm | Slot | Frees when |
|---|---|---|---|
| A | Map KEY (`emit_load_map_payload`) | `for_each_map_entry + 0` | key type is `String` |
| B | Map VALUE | `for_each_map_entry + 8` | value type is `String` |
| C | `Set OF T` element (entry key payload) | the loop local | element is `String` |
| D | `List OF T` element (kind-2 packed AND entry-table) | the loop local | element is `String` |

A `Map OF String TO String` therefore owns TWO blocks per entry, which is why it
leaked twice as fast (50 -> 99 MB). The `Set` arm is the one a `List`-only
enumeration omits; it leaked identically and has its own case.

Every other arm hands back a scalar, a shared closure pointer, or an ALIAS into
the container's own block — so none of them registers anything and their codegen
is byte-for-byte unchanged.

### The guard

Adding a free is the double-free direction, and the hazard specific to a loop
item is the alias arms above: freeing one would `arena_free` into the middle of
the collection. So the free does not rest on a second copy of that enumeration
living in the drop. `emit_load_collection_payload_with_alias_base` returns the
`dataBase + offset` pointer **the materialising emitter itself computed**, the
loop spills it per iteration, and `emit_loop_item_drop` skips the free when the
item IS it. One compare, exact by construction: every aliasing arm returns that
register verbatim. It is `collections::reduce`'s model — compare the produced
pointer against the value you do not own — applied to the one value a `FOR EACH`
provably does not own.

The ESCAPE direction is handled by two gates that were already there, and both
answer correctly *because* the item is registered as an ordinary `OwnedValue`
keyed on its stack offset:

* `plan_returned_move` finds it for `RETURN e` and removes the cleanup on that
  path, moving the block to the caller instead of freeing it.
* `lower_returned_value`'s param-borrow gate asks "does this local own a block"
  the same way, so a bare `RETURN <local>` in a param-borrow function copies
  rather than handing out the item pointer.

A distinct cleanup variant would have made both read "owns no block" — the
second one silently, and only in a function whose loop variable shadows a
parameter name. That shape is rejected outright (`SYMBOL_DUPLICATE_LOCAL`:
`FOR EACH s IN xs` inside `FUNC f(s AS String, …)` does not compile), so the
name-shadowing hazard cannot arise today; the slot-keyed registration is what
keeps it from mattering if it ever can.

Every owning consumer of `e` copies before the drop runs — `value_needs_owning_copy`
classes a `Local`/`MemberAccess` as an aliasing source, so `LET t = e`,
`append(out, e)`, `s = s & e` and `RETURN e.value` all take an independent block
(§14.6: a container "never stores a non-owning alias"). Measured, not assumed:
`a_for_each_body_that_uses_the_item_does_not_leak_it` went 27 -> 54 MB before and
is flat after, with the source list read back intact.

### Measured (macos-aarch64, release, `/usr/bin/time -l`, peak RSS)

| Shape | before (N / 2N) | after (N / 2N) |
|---|---|---|
| `List OF String`, `len(e)` only | 25 / 50 MB | 0 / 0 MB |
| `Map OF String TO String` | 50 / 99 MB | 1 / 1 MB |
| `Set OF String` | 25 / 50 MB | 1 / 0 MB |
| `EXIT FOR` out of the body | 16 / 31 MB | 0 / 0 MB |
| `CONTINUE FOR` | 25 / 50 MB | 1 / 0 MB |
| `RETURN e` | 16 / 31 MB | 0 / 0 MB |
| `append(out, e)` + `s = s & e` | 27 / 54 MB | 1 / 1 MB |
| **contrast** `Map OF Integer TO Integer` | 0 / 1 MB | 0 / 0 MB |
| **contrast** `List OF Integer` | 0 / 0 MB | 0 / 0 MB |

N = 50 000 outer passes over an 8-element collection, 2N = 100 000. Every
program printed the identical value before and after.

### Golden delta: 93, and why it is contained

`artifact-gate.sh <exe> all` reported **0 diff(s) over 1973 goldens** on a
detached worktree at the same base commit, and **93** on this branch. Unlike
bug-562 and bug-569, this shape IS exercised by committed fixtures — `FOR EACH`
over a `List OF String` is ordinary source, and the builtin `.mfb` package bodies
are full of it.

The delta was localized per function by diffing the `-ncode` dumps of twelve
fixtures against the pre-change compiler: **every one of the 44 changed functions
gained at least one alias witness and at least one `_mfb_rt_drop_owned_string`
call, and not one function changed without gaining an owner.** The changed set is
exactly the functions containing a `FOR EACH` over a `String` collection —
`encoding::punycodeEncode`/`punycodeDecode`/`codepageEncode`,
`http::buildRequest`/`headerValue`/`serializeHead`/`checkResponse`/`partHeader`/
`dispositionParam`, `json::stringify`/`stringifyIndent`/`get`/`getOr`/`revive`,
`csv::stringifyRow`, `audio::mmlParse`/`mmlTokens`/`mmlHasOpen`/`playTracks`,
`net::parseQuery`, `term::color::nameOf`, `regex::allDigits`,
`collections::mapValues`/`merge`. Each of those was leaking a block per element
per call in the standard library.

The 93 goldens regenerated are exactly the 93 the gate flagged (the two sets were
diffed): 144 `.ncodesum` under `tests/byte-identity/` via
`scripts/regen-ncodesum.sh` and 17 outside it via `scripts/regen-outside-ncode.sh`
were re-summed, of which 93 actually changed.

### Not fixed here, and measured to be someone else's

The error paths out of a `FOR EACH` body still grow, and the growth is **not** the
loop item. The same `FAIL`-through-`TRAP` program with the `FOR EACH` replaced by
a `WHILE` + `collections::get` walk leaks the identical 2 -> 3 MB at 20k/40k on
BOTH the pre- and post-fix compilers, and an auto-propagated error out of the
middle of the body leaks the identical 6 -> 11 MB in all four combinations. That
residual is **bug-565** (the inline-`TRAP` error path, Open). With the `FOR EACH`
version the fix took the `FAIL` case from 6 -> 11 MB down to 2 -> 3 MB, i.e. down
to exactly what the loop-free control costs.

