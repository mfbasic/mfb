# bug-784: updating a record nested inside a list element allocates twice per update

Last updated: 2026-10-04
Effort: medium (guess)
Severity: MEDIUM — a silent per-update allocation cost on a common record shape; output correct
Class: Other (performance)

Status: Fixed for the measured shapes (M2, M1 inline and call forms); the benchmark `record nested` row itself is blocked by a separate borrow-analysis limitation (see *Benchmark row*)
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

## M1, call form — destination variants (2026-10-04)

`inner := stepWide(h.inner, i, tag)` used to cost the callee's result block. The
callee already receives `h.inner` as a pointer to the element's own bytes (an
argument is lent), so a second lowering of the callee writes its result there:

- **Analysis — `src/codegen/engine/analysis/into_dest.rs`.** `collect_into_sites`
  finds, in each element-bound binding's update `WITH` (`collect_borrow_gets` with
  "no call writes a global", a superset of the lowering's set), an update
  `f := g(…, p.f, …)` whose callee `g` qualifies for that parameter
  (`into_param_eligible`: the parameter's type is the return type, a flat record,
  never assigned or address-taken, and some `RETURN WITH <param> { … }` exists).
  The same function feeds the builder's sites and the variant demand
  (`demanded_into_variants`), so a called variant always exists.
- **Callee — `<base>$into<j>` (`OwnedVariant::into`, `try_into_return`).**
  `RETURN WITH w { … }` runs `try_inplace_scalar_fields` with `w` as a span owner:
  **read-before-write by construction** — every update value (scalars, the
  replaced `String`'s source pointer and its size) is computed and spilled
  before the first store; the stores then go scalar slots, then the one replaced
  sub-block (whose source may overlap it only as that same span — forward copy),
  then nested scalars. When the result's exact size equals `w`'s, it is written
  into `w` and `w`'s address returned; otherwise nothing is written and a fresh
  block built from the spilled values is returned.
- **Caller — `element_into_call`.** The call is lowered to the variant symbol,
  its result claimed off the pending-temp list, and compared with the
  destination's address (taken right after the call): equal = done in place,
  nothing freed; different = the existing M2 replace / write-back over the fresh
  block, which is then freed. Because the variant writes while the caller's `WITH`
  values are still being evaluated, the caller admits it only when that is
  unobservable: every earlier update is a scalar or nested scalar; every later
  update reads neither `p.f`, `p` whole, the container, nor a binding that may
  alias the element, and calls only infallible built-ins
  (`later_value_misses_destination`); and if a later update can fail, no `TRAP` or
  inline result capture is active and the container is a plain owned local (the
  error leaves the function with it).

Measured (`/tmp/b784/gen.sh`, `DEBUG=1`, `arena.0.alloc_calls`, N = 1000 / 2000):
`nested`, `nestedsame`, `nestedmut` 1482 / 2482 → **546 / 546** (1 → 0 per
update; the 546 includes each element's first, size-changing update). Output
identical. `tests/runtime/rt_record_nested_update.rs` (on the M2-only compiler:
1.00 per update for `nested`, `nestedsame`, `nestedmut`, `not_last_inlined` and
the two overlap cases):

- `overlap_callee_reads_destination` — `stepSelf` reads the old `count` after
  updating it in source order and copies the destination's own `name` into `tag`;
- `overlap_argument_views_destination` — the `tag` argument is `h.inner.name`, a
  view into the destination;
- `overlap_argument_is_replaced_span` — the `tag` argument is the very span the
  callee replaces;
- `overlap_later_update_reads_destination` — `touches := h.touches +
  h.inner.count` must see the OLD count, so the variant is refused (1 per update);
- `a_later_failure_leaves_the_element_unchanged` — an overflowing later update
  under a function-level `TRAP`: the element is unchanged.

The last two were proved to guard something: with both gates disabled the
overlap case printed `809377900` against the contrast's `120552185`, and the
failure case left the element half updated.

## Benchmark row

The benchmark's `test_record_nested` gets none of this — not M2 and not the
destination variant (its `.ncode` still has `bl _mfb_fn_stepWide` then
`record_build_alloc_ok`). The cause is not this bug's code: the function binds
`LET h AS Holder = collections::get(holders, k)` a second time in its checksum
loop, and `borrow_get::collect_borrow_gets` keys its facts by NIR local name
across the whole function (`bind_counts`, `gets`), so a re-bound name is never
element-bound. Renaming the second `h` to `hh` in a copy of
`benchmark/mfb/src/recordbench.mfb` made the build emit and call
`_mfb_fn_stepWide$into0`. Making the analysis scope-aware (or alpha-renaming
re-bound block-local names before it) is a separate change to a shared analysis
and is left as its own ticket; the benchmark source was not edited.

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
- [x] Phase 3 — M1: inline form (`nested_scalar_with`) and call form (destination
      variants, `analysis::into_dest`).
- [ ] Phase 4 — goldens, full suite, benchmark `record nested` row (batch gate).
