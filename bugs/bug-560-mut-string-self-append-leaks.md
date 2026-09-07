# bug-560: `s = s & <expr>` on a `MUT String` leaks ~190 B per evaluation

Last updated: 2026-09-06
Effort: medium
Severity: **HIGH** (unbounded leak on the idiom the performance docs recommend)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/rt_scope_drop_leaks.rs` RSS case, to be added)

Found while fixing bug-536 shape B-2. **Reproduces unchanged on the base commit
and is unaffected by that fix** — do not attribute it to B-2.

## The finding

A `MUT String` self-append leaks about 190 bytes per evaluation. Minimal repro:
a flat `SUB main`, no functions involved at all.

```
MUT out AS String = ""
out = out & "a"          ' 38 MB at 200k iterations, 75 MB at 400k
```

`MUT out AS String = ""` on its own is **flat** at both counts, so the leak is
the assignment, not the binding.

## Why this one matters more than its size suggests

1. **It is the idiom the docs recommend.** `s = s & ch` is measured ~3× faster
   than `List OF String` + `join`, and that advice is recorded and followed. The
   fastest spelling is the leaking one.
2. **The builtins are built out of it.** `__encoding_utf32Decode` and
   `__csv_decodeRange` are both `out = out & __encoding_fromCodepoint(cp)`, once
   per scalar. So this is the hottest leaking line in the tree, and it scales with
   input length.
3. **bug-536's own shape-B table gets it wrong.** It records `s = toString(i)` as
   "flat — the assignment frees the old block". That is true for a plain
   assignment and **false for a self-append**, which is a different lowering path.

## Where to look

The in-place string self-append path: `prescan_string_self_appends` and
`string_capacity_slots`. The plain-assignment path frees the old block; this one
appears not to.

## What a fix must produce

`out = out & x` in a loop runs at constant RSS, and `__encoding_utf32Decode` /
`__csv_decodeRange` inherit that without source changes.

Measure it as a leak: **peak RSS at N and 2N iterations**, not a one-shot. A
one-shot cannot distinguish a leak from an allocator high-water mark.

## Non-goals

- Do not "fix" this by recommending `List OF String` + `join` instead. The
  self-append is the faster shape and should also be the correct one.
