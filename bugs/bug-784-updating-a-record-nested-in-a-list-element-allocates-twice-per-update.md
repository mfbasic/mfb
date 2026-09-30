# bug-784: updating a record nested inside a list element allocates twice per update

Last updated: 2026-09-29
Effort: medium (guess)
Severity: MEDIUM — a silent per-update allocation cost on a common record shape; output correct
Class: Other (performance)

Status: Open
Regression Test: to be added (Phase 1)

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

## Goal / Non-goals

- `nestedsame`, `nestedinner` and `nestedmut` allocate 0 per update. `nested` allocates
  at most 1, the size-change rebuild.
- No language-spec or memory-model change: value semantics, the error behavior and
  every output are unchanged. A record still carries no size word. A size-changing
  replacement still rebuilds; no realloc and no capacity field.

## Phases

- [ ] Phase 1 — RED runtime test (the cases above, N vs 2N `--debug` counts); trace M1
      and M2 in the `.ncode`.
- [ ] Phase 2 — M2: same-granule replacement of a non-last inlined record/`String` field.
- [ ] Phase 3 — M1: an in-place update of a field read out of an element-bound record.
- [ ] Phase 4 — goldens, full suite, benchmark `record nested` row.
