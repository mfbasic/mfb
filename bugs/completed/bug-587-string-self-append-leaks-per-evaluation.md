# bug-587: `s = s & <expr>` on a `MUT String` leaks ~190 B per evaluation

> **WITHDRAWN.** This is the same defect as **bug-560** (`bugs/completed/bug-560-mut-string-self-append-leaks.md`),
> which was filed on 2026-09-06 and FIXED in `29f3d5885` (2026-09-07).
> It should never have been filed. See "Why this was filed twice" at the end.


Last updated: 2026-09-12
Effort: unknown (the in-place self-append path is the suspect; measure first)
Severity: HIGH — unbounded leak in the idiom the performance docs recommend  
*(severity as filed; moot — the defect is fixed)*
Class: Memory / correctness

Status: **CLOSED — DUPLICATE of bug-560, which was fixed before this was filed.**
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

## Why this was filed twice — and how to not do it again

bug-536 recorded three defects found while fixing its shape B-2, with the note
"they are recorded here rather than filed so the numbering does not race with a
peer session". **They were filed the next day**, as bugs 560, 561 and 562
(`8c57683f3`, 2026-09-06) — but nobody went back and updated bug-536's item
list. Six days later a session read that unchanged list, took the note at face
value, and filed all three a second time as 587/588/589.

The number checks that were run (`ls bugs/ bugs/completed/` and
`git log --all --grep=bug-NNN`) all passed, because they answer "is this NUMBER
free?" — which it was. Nobody asked "is this DEFECT already tracked?", and that
is the question that mattered.

**The rule: before filing, search for the DEFECT, not the number.** Grep
`bugs/completed/` for the symptom, the function name, and the idiom — here,
`git grep -il "self.append" bugs/` would have surfaced bug-560 immediately.
A "recorded but not filed" note in an older document is a claim about the past
with a timestamp on it; check whether it is still true before acting on it.

Verified before closing (not assumed): bug-560 is marked FIXED with a named
commit, and its regression pins are live at HEAD —
`a_reassigned_string_self_append_runs_at_constant_rss`,
`a_returned_string_self_append_runs_at_constant_rss`,
`a_plain_string_self_append_still_runs_at_constant_rss` and
`every_string_self_append_shape_still_produces_the_right_value` in
`tests/runtime/rt_scope_drop_leaks.rs`.
