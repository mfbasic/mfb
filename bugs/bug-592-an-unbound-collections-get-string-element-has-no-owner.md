# bug-592: an UNBOUND `collections::get`/`getOr` `String` element is never freed

Last updated: 2026-09-12
Effort: small–medium (one more producer to opt into bug-536 shape B; the audit is which collection members hand back a FRESH element and which hand back an alias)
Severity: MEDIUM (unbounded leak in any loop that reads a `String` element without binding it)
Class: Memory / correctness

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`, to be added)

Found while fixing bug-576, and measured to be a **different defect**: it needs no
runtime helper in the program at all, and it survives bug-576's fix.

## The finding

Measured on the bug-576 branch's release binary (so, WITH bug-576 fixed),
200 000 → 400 000 iterations, peak RSS via `/usr/bin/time -l`:

| shape | per call |
| --- | --- |
| `n = n + len(collections::getOr(names, 0, ""))`, `names` from a **literal** | **64 B** |
| the same read into `LET e AS String = collections::getOr(names, 0, "")` | 0 B |
| `names = collections::append(names, os::arch())` with no `getOr` at all | 0 B |

The third row is the control that rules bug-576 out: the helper's own block is
flat now. The first row has no helper in it.

Repro (the leaking one):

```
IMPORT io
IMPORT collections
SUB main()
  MUT n AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 400000
    MUT names AS List OF String = ["aarch64"]
    n = n + len(collections::getOr(names, 0, ""))
    i = i + 1
  END WHILE
  io::print("n=" & toString(n))
END SUB
```

## Where to look

Same mechanism as bug-536 shape B and bug-576: `register_pending_temp`
(`src/codegen/engine/value/builder_values.rs`) frees a bare `String` temp only
with freshness provenance, and the collection `get`/`getOr` lowering never calls
`mark_fresh_string`. A `LET` binding is flat because the bind owns and frees the
block; an unbound read has no owner at all.

`materialize_owned_element` (`src/codegen/memory/owned.rs`) is the obvious site
and is NOT it: its copy arm explicitly excludes `String`
(`is_freeable_flat_value(..) && result.type_ != ParameterType::String`), because a
`String` element is already handed back as an owned fresh block by the `get`
lowering itself. That producer is the one to mark.

## Why it is not a one-liner

The mark is only sound for a member that returns a FRESH block, and this family
has both kinds in it:

* `borrow_get_result` (plan-86 E) makes a read-only `get` return an ALIAS into the
  container's data region — marking one of those is an `arena_free` INTO the
  container. `register_pending_temp` already early-returns while that flag is set,
  so the guard exists, but the audit has to confirm it covers every path.
* the whole surface has to be enumerated, not just `getOr`: `get`, `getOr`,
  `first`, `last`, `pop`, `keys`, `values`, `find`, map iteration — each is either
  fresh or an alias, and a wrong verdict is a wild free rather than a leak.

That audit is the work, exactly as bug-566's per-helper audit was for the runtime
helpers, which is why this is filed rather than folded into bug-576.

## What a fix must produce

The first row above flat at N and 2N; the `LET`-bound and append-only rows still
flat (no double free); and a value probe that reads every element shape back in a
churning loop — a block freed while the container still points into it shows up as
a wrong value or a later allocation failure, never as a failing free.
