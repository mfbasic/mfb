# bug-587: `s = s & <expr>` on a `MUT String` leaks ~190 B per evaluation

Last updated: 2026-09-12
Effort: unknown (the in-place self-append path is the suspect; measure first)
Severity: HIGH — unbounded leak in the idiom the performance docs recommend
Class: Memory / correctness

Status: Open
Regression Test: an RSS pin in `tests/runtime/rt_scope_drop_leaks.rs` (the
87-case set this cluster already owns).

Split out of **bug-536**, where it was found while fixing shape B-2 and recorded
rather than filed so the numbering would not race a peer session. It is NOT
shape B-2 and is NOT fixed by it — it reproduces unchanged on that fix's base
commit.

## Reproduction

Flat `SUB main`, no functions involved:

```basic
MUT out AS String = ""
out = out & "a"          ' 38 MB at 200k iterations, 75 MB at 400k
```

- `MUT out AS String = ""` alone is **flat** at both counts, so the growth is
  the assignment, not the binding.
- Scaling is linear in the iteration count, i.e. a per-evaluation leak of
  roughly 190 B.

Calibrate at **>=200k iterations** and measure RSS with `--test-threads=1`;
RSS leak pins are ~4x leaner on Linux than macOS.

## Why this is the one to fix first

`s = s & ch` is the idiom `.ai` recommends for string building — it beats
`List`+`join` by ~3x — so this is the **hottest leaking line in the tree**. It
is also what two builtin package bodies are built out of, once per scalar:

- `__encoding_utf32Decode` — `out = out & __encoding_fromCodepoint(cp)`
- `__csv_decodeRange` — same shape

Together with bug-588 this is the whole of `csv::parse`'s residual ~112 MB per
repeat call. Anyone tracking DEC-03 should attribute it here, not to bug-536
shape B-2.

## Root Cause (suspected — VERIFY BEFORE FIXING)

The in-place string self-append path: `prescan_string_self_appends` /
`string_capacity_slots`. bug-536's own shape-B table records `s = toString(i)`
as "flat — the assignment frees the old block"; a **self-append** is not, and
the table did not distinguish the two.

This attribution is inherited from bug-536's measurements and has **not** been
independently confirmed. Reproduce and localize before changing anything.

## Goal

- A `MUT String` self-append in a loop is flat in RSS.

### Non-goals (must NOT change)

- Do not regress the ~3x advantage `s = s & ch` holds over `List`+`join`; the
  in-place path exists for that reason.
- Do not free a block the assignment does not own (a double free is the failure
  mode this whole cluster has to avoid).

## Memory gate (required)

Per the standing rule for allocation/aliasing/ownership/drop bugs:
1. the RED RSS pin flips flat;
2. name the documented contract in `.ai/collections.md` / `mfb spec` §14 the fix
   realizes, and show it only ADDS a free rather than moving a lifetime;
3. the artifact-gate golden delta is confined to the emitting fixtures,
   everything else byte-identical — zero `.run` goldens moving is the strongest
   signal;
4. a POSITIVE pin that ordinary correct usage is unchanged, and a performance
   pin that the in-place fast path still fires.
