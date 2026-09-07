# bug-571: `FOR EACH` over a `List OF String` / `Map OF String TO …` leaks one block per element per pass

Last updated: 2026-09-07
Effort: small-to-medium (one materialisation site; the ownership proof is the work)
Severity: **HIGH** (unbounded leak on the most ordinary loop in the language)
Class: Memory / collection iteration

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`)

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
