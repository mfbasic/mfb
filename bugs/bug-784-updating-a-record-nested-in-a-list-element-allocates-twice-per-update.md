# bug-784: updating a record nested inside a list element allocates twice per update

Last updated: 2026-10-03
Effort: medium (guess)
Severity: MEDIUM — a silent per-update allocation cost on a common record shape; output correct
Class: Other (performance)

Status: Partially fixed — M2 and the inline form of M1 landed; the call form of M1 (`inner := step(h.inner, …)`) is open (see *Remaining work*)
Regression Test: `tests/runtime/rt_record_nested_update.rs`

Split out of bug-766, which removed the allocations for most record-update spellings
but did not design this one. The benchmark row `record nested`
(`benchmark/mfb/src/recordbench.mfb:test_record_nested`) updates a `Wide` held in a
`Holder` held in a `List OF Holder`:

```basic
LET h AS Holder = collections::get(holders, k)
holders = collections::set(holders, k, WITH h { inner := stepWide(h.inner, i, tag), touches := h.touches + 1 })
```

Each update costs **2 arena allocations**. Measured with `bash /tmp/b784-measure.sh`
(`--debug` `arena.0.alloc_calls`, N = 100 000 vs 200 000, on main after bug-766):
200 407 vs 400 407. This is the same whether `tag` alternates sizes (`nested`), stays
the same (`nestedsame`), or `h` is a `MUT` rebound before the `set` (`nestedmut`).

**The single correct behavior a fix produces:** an update whose new `inner` has the same
rounded size as the old one allocates nothing. One that changes the size costs at most
the one rebuild bug-766 D keeps for a size change (owner decision, 2026-09-29: "keep
the rebuild"). Output is unchanged.

## Measurements

`/tmp/dbA-wide/gen784.sh <case> <N>` with `DEBUG=1`; types as in `recordbench.mfb`
(`Wide` has 10 fields including two `String`s; `Holder` is `id`, `label AS String`,
`inner AS Wide`, `touches`):

| Case | Loop body (after `LET h = get(holders, k)`) | allocs / update |
|---|---|---|
| `nested` | `set(holders, k, WITH h { inner := stepWide(h.inner, i, tagEven\|tagOdd), touches := … })` | **2** |
| `nestedsame` | the same, always `tagOdd` | **2** |
| `nestedcall` | `LET nw AS Wide = stepWide(h.inner, i, tagOdd)` only | **1** |
| `nestedinner` | `set(holders, k, WITH h { inner := WITH h.inner { count := h.inner.count + 1 }, touches := … })` | **2** |
| `nestedtouch` | `set(holders, k, WITH h { touches := h.touches + 1 })` | **0** |

So there are two independent sources, 1 allocation each:

- **M1 — the new inner record.** `stepWide(h.inner, …)` (and an inline
  `WITH h.inner { … }`) builds a fresh `Wide`. Nothing hands `h.inner` over, so the
  callee has no owned variant (`handover.rs:consuming_use` accepts a local, not a
  field read). `h` is itself a copy out of `holders` that is written back, so its
  `inner` block is a candidate for an in-place update. **Guess:** this needs the
  element-bound analysis (`borrow_get.rs`) to treat `h.inner` as owned for the
  duration of the write-back.
- **M2 — replacing `inner` in the write-back.** `WITH h { inner := <new Wide> }` inside
  `set` rebuilds the element even when the rounded size is unchanged.
  `builder_control.rs:try_inplace_scalar_fields` replaces an inlined `String`/record
  field in place only when it is the record's *last* inlined field (bug-766 D); `inner`
  is followed by `touches`. **Guess:** the fix is the same-granule replacement for a
  non-last inlined field, where the later fields' offsets are unchanged when the rounded
  size is unchanged.

## Findings (Phase 1 trace, 2026-10-03)

Traced in the `.ncode` of `nestedsame` (`mfb build -O 1 --ncode`, `main`'s loop):
`list_get_done` (the borrowed `get`, no copy), then `bl _mfb_fn_stepWide` (M1: the
callee's `RETURN WITH w { … }` builds a fresh `Wide`), then `record_build_alloc_ok`
(M2: a fresh `Holder`), then `set_inplace` taking its same-size overwrite.

**M2's guess was wrong.** `inner` IS `Holder`'s last inlined field — `touches` is a
scalar slot, not an inlined sub-block — so bug-766 D's last-field rule was not what
declined. `replace_field_admitted` refuses every element-bound owner
(`element_updates.field_owner`, bug-689), so `lower_element_update` fell back to
lowering `xs = set(xs, k, WITH h { … })` as written, and that `WITH` built the
element anew.

## Fix (2026-10-03)

- **M2 — `element_replace_admitted` / `emit_element_replace_or_write_back`.** Over an
  element-bound owner, one `String` or variable-size record field at ANY inlined
  position takes `FieldStoreKind::Replace`. The element is a span of the list's data
  region whose entry records its exact byte length (the `set` arm overwrites in
  place only at that exact length), so the granule rule becomes an exact-size rule:
  equal sizes write the new bytes over the old sub-block (no offset, no length
  changes); otherwise the element's own write-back runs, with every update value
  replaced by a hidden local bound to the slot this statement already spilled it
  to (`element_write_back_item`), so nothing is evaluated twice. That branch is the
  copying `set` of a `WITH` the bug-689 fallback already ran.
- **M1, inline form — `nested_scalar_with`, `FieldStoreKind::Nested`.**
  `inner := WITH h.inner { <scalars> }` beside other updates stores those scalars
  into `inner`'s sub-block (a scalar slot never changes size), over any owner and at
  any inlined position. `peel_field_path` already served the single-update form;
  this is the multi-update one. The nested stores run last in `emit_field_stores`,
  after every copy that may read the sub-block.

Measured with `/tmp/b784/gen.sh <mfb> <case> N` (`DEBUG=1`, `arena.0.alloc_calls`,
N = 1000 vs 2000; the generator is the doc's `gen784.sh` with a valid
`project.json`), base = `bug-staging-7` at `4073c70e2`:

| Case | base N / 2N | fixed N / 2N | allocs / update |
|---|---|---|---|
| `nested` | 2418 / 4418 | 1482 / 2482 | 2 → **1** |
| `nestedsame` | 2418 / 4418 | 1482 / 2482 | 2 → **1** |
| `nestedmut` | 2418 / 4418 | 1482 / 2482 | 2 → **1** |
| `nestedinner` | 2417 / 4417 | 417 / 417 | 2 → **0** |
| `nestedcall` | 1417 / 2417 | 1417 / 2417 | 1 → 1 (the call alone) |
| `nestedtouch` | 417 / 417 | 417 / 417 | 0 → 0 |

Every printed result is identical before and after. Timing (N = 200 000, three runs
each, loaded machine): `nested` 155/177/103 → 82/95/86 ns/op, `nestedinner`
146/127/229 → 4/5/4 ns/op.

## Remaining work — M1, call form

`inner := stepWide(h.inner, i, tag)` still costs the callee's result block, 1 per
update (`nestedcall` alone measures the same 1). No hand-over can remove it as the
calling convention stands: plan-147's owned variant takes ownership of an arena
block, may free it (bug-766 D's rebuild on a granule change) and returns a block,
while `h.inner` is a span inside the list's data region — handing it over would
free a pointer into a live allocation. Removing the allocation needs one of:

1. a per-call-site recycled block: copy `h.inner` into a function-scoped arena
   block (allocated once, re-copied while its walked size keeps the granule), hand
   THAT over to the `$own` variant, write the result into the element (the M2
   store), and keep the returned block as the next iteration's instead of freeing
   it; freed (walk-sized) on every exit, error and `TRAP` paths included. New
   frame-scoped state with exit cleanup plus a `handover.rs:consuming_use` arm for
   a field-read argument; or
2. a destination-passing variant of the callee that writes its `WITH` result into
   a caller-supplied span when the size is unchanged — an ABI addition.

Both are new mechanisms rather than a fix of an existing route, so they are left
for a follow-up. Until then the goal row "`nestedsame` and `nestedmut` allocate 0"
is not met (1 each); `nestedinner` is (0), and `nested` meets "at most 1".

## Goal / Non-goals

- `nestedsame`, `nestedinner` and `nestedmut` allocate 0 per update. `nested` allocates
  at most 1, the size-change rebuild.
- No language-spec or memory-model change: value semantics, the error behavior and
  every output are unchanged. A record still carries no size word. A size-changing
  replacement still rebuilds; no realloc and no capacity field.

## Phases

- [x] Phase 1 — RED runtime test (the cases above, N vs 2N `--debug` counts); trace M1
      and M2 in the `.ncode` (see *Findings*).
- [x] Phase 2 — M2: exact-size replacement of a variable-width field of an
      element-bound record (the actual decline; see *Findings*).
- [ ] Phase 3 — M1: inline form done (`nested_scalar_with`); call form open (see
      *Remaining work*).
- [ ] Phase 4 — goldens, full suite, benchmark `record nested` row (batch gate).
