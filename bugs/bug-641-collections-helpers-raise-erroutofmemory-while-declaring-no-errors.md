# bug-641: `collections` helpers raise `ErrOutOfMemory` while their descriptors declare no errors

Last updated: 2026-09-15
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: Open
Regression Test: none yet — see Phase 1

`collections::append` is registered with `errors: vec![]` on both overloads
(`src/codegen/builtins/collections/func_append.rs:107,128`), so its man page
renders no Errors table and its prose says the call is **infallible**, with an
inline `TRAP` on it having "a dead handler". But the lowering can raise:
`src/codegen/collection/list/list_mutate.rs:143-147` and `:622-626` call
`emit_arena_alloc_call()` and then `raise_error_bare("ErrOutOfMemory")` for an
allocation failure or a size overflow.

This is the `collections` instance of the pattern bug-606 recorded for the
`encoding` decoders: a member that raises an error its descriptor does not
declare. The consequence is the same — the Errors table cannot show the
condition, and `mfb`'s own "infallible" prose contradicts the emitted code.

**The single correct behavior a fix produces:** every `collections` member whose
lowering can raise `ErrOutOfMemory` declares it, so the Errors table renders the
row, and no page claims such a call is infallible. Members that genuinely cannot
raise keep `errors: vec![]`.

Found by plan-125-C Phase 4 while triaging the `append` page review. **Filed, not
fixed**, by user instruction during a documentation-only plan ("file all bugs,
make no fixes"). The pages are being corrected to drop the "infallible" claim;
the declaration itself is a code change and belongs here.

## Reproduction

An allocation failure cannot be forced from a test program on a normal host, so
the mismatch is shown from the registry and the lowering:

```
$ grep -n 'errors' src/codegen/builtins/collections/func_append.rs
107:                errors: vec![],
128:                errors: vec![],

$ grep -n 'ErrOutOfMemory' src/codegen/collection/list/list_mutate.rs
145:        self.raise_error_bare("ErrOutOfMemory")?;
147:        self.raise_error_bare("ErrOutOfMemory")?;
624:        self.raise_error_bare("ErrOutOfMemory")?;
626:        self.raise_error_bare("ErrOutOfMemory")?;
```

`./target/release/mfb man collections append` renders no Errors section and says
the member is infallible.

A Phase 1 test can assert the agreement directly: for each registered member,
if its lowering path reaches `raise_error_bare("ErrOutOfMemory")`, its descriptor
must declare `ErrOutOfMemory`. Alternatively, an arena-limit test hook (if one
exists) can drive a real allocation failure through `append` and assert the
raised code is trappable.

## Root cause

The descriptor's `errors` list is written by hand per member, and the allocation
raise lives in shared collection lowering (`list_mutate.rs`,
`gen_mutate.rs:lower_collection_end_insert`) rather than in the member's own
file, so the two were never reconciled. The same shared path backs several
members, which is why this is a class rather than a single page defect.

## Non-goals

- Changing when or whether `ErrOutOfMemory` is raised, or the allocation strategy.
- Adding the error to members that cannot allocate (`contains`, `any`, `all`,
  `hasKey`, the `is*` set predicates): a blanket addition would make every page
  claim a condition it does not have.
- "Fixing" the pages alone. The prose correction is already landing in plan-125-C;
  without the declaration the Errors table still cannot render the row.

## Blast-radius audit

Members whose file references an allocating lowering path
(`lower_collection_end_insert`, `lower_list_insert_collection`, or
`emit_arena_alloc_call`), measured with
`grep -ln` over `src/codegen/builtins/collections/`:

- `append`, `chunks`, `groupBy`, `insert`, `prepend`, `merge`, `set`, `sort`,
  `window`, plus the shared `gen_mutate.rs`.

A second allocation seam reaches more members: `gen_mutate.rs:lower_reserved_list`
also raises `ErrOutOfMemory` when the result list cannot be created. The
`collections::filter` review (plan-125-C Phase 4) found its page claiming it "can
fail only because a failing predicate propagates", while `func_filter.rs:lower_filter`
builds its result through `lower_reserved_list`. Every member that builds a new
list that way — `filter`, `transform`, and the other higher-order helpers — belongs
in the Phase 1 census.

Each needs its own verdict in Phase 1: reaches the raise (declare it), or only
allocates through a path that cannot fail that way (leave as is). Also check
`strings` and other packages that build new collection values through the same
helpers.

## Fix

Phase 1 — RED test asserting descriptor/lowering agreement for the members above,
plus the census with a verdict per member. Commit:

Phase 2 — declare `ErrOutOfMemory` on the members that can raise it, re-render the
affected man pages, and drop any remaining "infallible" prose the plan has not
already corrected (GREEN); full suite and man-page goldens. Commit:
