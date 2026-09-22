# plan-149-A: Audit and the owned-local forward

Last updated: 2026-09-22
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing (the Prerequisites below gate the whole plan)

## What plan-149 builds

plan-149 adds a new Level-1 (`-O1`, the default) Opt1 catalog row, **place copy forwarding**. It removes the deep copy
made by `LET b AS <aggregate> = <place>`, where `<place>` is a local, a global, or a field path rooted at one of those
or at a resource's `STATE`. It removes the copy only when every read of `b` provably happens before the next write of
the place's owner. In that case the pass points `b`'s reads at the place itself and drops the binding.

The payoff is the self-update seam plan-145 built. The seam runs `rec = WITH rec { xs := OP(rec.xs, …) }` and
`f.state.xs = OP(f.state.xs, …)` in place, but only when the call's argument *is* the field expression. The benchmark,
and ordinary code, reads the field into a local first:

```
LET cur AS List OF Integer = rec.xs
LET v AS Integer = collections::get(cur, j)
rec = WITH rec { xs := collections::set(cur, j, v + 1) }
```

That shape copies the field, never matches the seam, and rebuilds the record every time. After the forward, the NIR
is exactly the direct form, and the existing seam takes it. No codegen changes.

**The single behavioural outcome:** at the default level, a `LET b = <place>` whose reads all precede the next owner
write compiles to the same in-place code as the direct form. Output is identical at `-O0` and `-O1`, and `-O0` keeps
the copy.

References:

- `mfb man optimizations`: the dial contract. Level 0 is only for rewrites the language requires; Level 1 is
  "transparent local rewrites — same operations, less waste".
- `planning/optimizations.md`: the catalog table and the level-sorting rule (line 24: "Sort by 'is the
  un-transformed program still correct?'").
- `src/optimizer/opt1/aggcopy.rs`: the precedent row this one mirrors (Level 3; forwards `LET b = a` only when
  neither name is ever written).
- `planning/completed/plan-145-*`: the in-place field seam this row feeds. `src/codegen/collection/assign/inplace_dest.rs`
  and `src/codegen/engine/control/builder_control.rs:552` (`try_inplace_mixed_with`).
- `.ai/testing-gates.md`, `.ai/compiler.md`: the gates.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-145 complete and archived | `ls planning/completed/plan-145-* \| wc -l` → 9, and `ls planning/plan-145*` → no match | MET (2026-09-22: 9 / no match) |
| The default level is 1 | `grep -n 'OptLevel(1)' src/optimizer/mod.rs` → the `Default` impl (line 73) | MET (2026-09-22) |
| The direct form is correct today, including a sibling that reads the owner | build and run the Appendix A program → `4 3` / `3 4` / `9 2` / `9 77` | MET (2026-09-22, `target/release/mfb build`, output as stated) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command and update every
> status before you continue, and again before you decide to stop. If you stop, report the current status of *all*
> prerequisites.

## 1. Goal

- Two things hold at `-O1`, with output identical to `-O0`:
  - `LET cur = rec.xs` followed by `rec = WITH rec { xs := OP(cur, …) }` takes the plan-145 in-place path.
    `codegen_place_forward.rs` checks for the `inplace_*` slots and no `with_target` slot.
  - `LET snap = nums` followed by `nums = OP(snap, …)` takes the plain-local in-place path.
- Global and `STATE` owners come in plan-149-B. The catalog, docs and benchmark come in plan-149-C.

### Non-goals (explicit constraints)

- **No codegen change.** The row is NIR-to-NIR only. If a forwarded shape does not go in place, that is the seam's
  decline and stays out of this plan. The fallback (a rebuild with the field as the argument) is no worse than today's
  copy plus rebuild.
- **No change to any program's output, errors, or error locations at any level.** This is the dial contract.
- **Not Level 0.** Keeping the copy is still correct, so by the catalog's own sorting rule this is a Level-1 row.
- **Not the `String`-element rows.** The 8 "Dynamic" benchmark rows (`List OF String` and friends) are slow even in the
  direct form: 9,294 ns per statement direct versus 11,820 with `LET cur` (probe `/tmp/getprobe/d`, `dynDirect` /
  `dynCur`). The forward removes their copy, but their rebuild is a separate seam gap, to be filed separately (see
  plan-149-C, Open Decisions).
- **`aggcopy.rs` is not changed or removed.** Its Level-3 row keeps its own gates. Where both could fire, the new row
  runs first, and aggcopy then finds nothing.

## 2. Current State

**NIR shapes** (from `mfb build --nir` on the probe `/tmp/getprobe/c`, 2026-09-22):

| Source | Binding NIR | Owner write NIR |
|---|---|---|
| `LET cur = rec.xs` (local) | `Bind{mutable:false, value: MemberAccess{target: Local("rec"), member:"xs"}}` | `Assign{name:"rec", value: WithUpdate{target: Local("rec"), …}}` |
| `LET cur = g.xs` (global) | `MemberAccess{target: Global{name:"g"}, member:"xs"}` | `StoreGlobal{name:"g", value: WithUpdate{…}}` |
| `LET cur = f.state.xs` | `MemberAccess{target: MemberAccess{target: Local("f"), member:"state"}, member:"xs"}` | `StateAssign{resource:"f", value: WithUpdate{target: MemberAccess{Local f, "state"}, …}}` |
| `LET snap = nums` | `Local("nums")` | `Assign{name:"nums", value: Call{…}}` |

- NIR definitions: `src/target/shared/nir/mod.rs:170` (`NirOp`) and `:264` (`NirValue`).
- User calls are `Call{target:"recrow"}`, a bare function name that appears in `module.functions`. Builtins are
  package-qualified, e.g. `Call{target:"collections.get"}`.
- **The precedent** (`src/optimizer/opt1/aggcopy.rs`, read in full):
  - A scope-blind `Facts` census counts binds, writes (`Assign`, `StateAssign`), `LocalRef` (address-taken) and closure
    captures.
  - `pick` finds `Bind{mutable:false, value: Local(src)}` on an aggregate type (`is_aggregate`).
  - `rewrite_reads` repoints the reads, and `drop_bind` removes the binding. The function loops to a fixpoint and
    counts with `stats::count_aggregate_copies_forwarded`.
  - Its gates: both names stable (never written); neither address-taken, captured, or a resource owner; the source is
    not a parameter, because `RETURN b` → `RETURN a` must still see a local that `plan_returned_move` may move.
- **Pipeline** (`src/optimizer/opt1/mod.rs:optimize_nir`): `local_rewrites` (Level 1), branches, globals, lencache,
  the loop rows, aggcopy, recovery, UCE, DCE. Each row self-guards with `crate::optimizer::level_enabled(n)`.
- **Helpers to reuse:**
  - `plans::shape::{nested_bodies, nested_bodies_mut, own_values_mut}`.
  - The `NirVisitor` / `walk_op` / `walk_value` visitors.
  - `aggcopy::children_mut` (private today; A moves it to `plans::shape` so both rows share it).
  - Unit tests reuse `local_rewrites::testutil::*`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Benchmark rows with `LET cur = <owner>.<field>` then a write to that owner using `cur` | 16 (list 8, mapmatrix 4, setops 4); 8 Fixed, 8 Dynamic | the census script in this plan's commit message, run over `benchmark/mfb/src/*.mfb` |
| `LET <T> = <owner>.<field>` lines in the benchmarks (write-back plus read-only rows) | 210 (list 134, mapmatrix 32, setops 44) | `grep -rEc '^\s*LET \w+ AS [^=]+= [A-Za-z_]\w*(\.\w+)+\s*$' benchmark/mfb/src/*.mfb` |
| Fixture/example dirs containing a `LET <T> = owner.field` or `LET <agg> = local` line (an upper bound on the goldens that can diff) | 30 | the two `grep -rlE` commands recorded in plan-149-C §Validation |

### Verified properties

- **The direct form is fast, and the `LET cur` form is not** (probe `/tmp/getprobe/a` vs `b`, 1,000,000 statements,
  `target/release/mfb`):
  - `list (Record-Fixed) set` shape: 1,443 / 1,416 ns per statement with `LET cur`, versus 7 / 10 direct.
  - `STATE` `set`: 1,566 versus 87. `STATE` `removeAt`: 986 versus 124 (probe `/tmp/getprobe/d`).
- **Reading `rec.xs` as the argument of a builtin makes no copy.** The `.ncode` for probe `b` has no extra
  `copy_source` or `copy_result`, and no `with_target`, relative to `a`. It does have `inplace_recfield_set_*`.
- **The direct form is correct with sibling reads of the owner** (probe `/tmp/getprobe/e`, output
  `4 3 / 3 4 / 9 2 / 9 77`). The last pair shows that `RETURN rec.xs` from a parameter returns an independent copy:
  `WITH` evaluates every right-hand side against the old record, and a member-access return copies.
- **UNVERIFIED: a member-access read never costs more than a `Local` read in any consuming context.** If some context
  borrows `Local(b)` but copies `MemberAccess`, forwarding a `b` read many times in a loop could add copies. Task A1
  measures this.
- **UNVERIFIED: how an indirect call (through a function value) appears in NIR.** It matters for B's call gate. Task A1
  records it.

## 3. Design Overview

The row lives in one module, `src/optimizer/opt1/placefwd.rs`, called `placefwd::forward`. It runs at
`level_enabled(1)`, right after `local_rewrites` in `optimize_nir`, so that every later row sees the direct form.

**A candidate** is `Bind{mutable:false, name:b, type_ aggregate, value: Some(place)}`, where `place` is
`Local(o)`, `Global{o}`, or a `MemberAccess` chain whose root is one of those. Nothing else qualifies: no
`UnionExtract` and no call in the chain, so evaluating the place can never trap.

**Function-wide gates** (a scope-blind census, as in aggcopy):

- `b` is bound once, never assigned, not a `LocalRef`, never in closure captures, and not a resource owner.
- A local owner `o` is not a `LocalRef` and not captured (a by-ref capture is B's owner class). It is not rebound by a
  `For`, `ForEach` or `Trap` binder, and it has exactly one `Bind`.
- A bare `Local(o)` source (no members) must not be a parameter. This is aggcopy's return-move reason.

**The window rule.** This is where the correctness risk concentrates. Let the `Bind` be `ops[i]` in its block. Scope
confines every read of `b` to `ops[i+1..]` of that block, possibly nested. Let `L` be the last op containing a read.

- **W1.** No op in `i+1..L-1` contains, at any depth, a write of the owner.
- **W2.** Op `L` contains no owner write at any depth, **or** `L` is itself a top-level owner write (`Assign{o}` in A;
  `StoreGlobal`/`StateAssign` in B) whose reads of `b` are in its value, with no deeper owner write. A value is fully
  evaluated before its store, and plan-145's seam already handles the direct form's sibling reads (Prerequisites row 3).
- **W3.** No read of `b` sits inside a `Trap` body (a handler runs after the failing statement) or in a closure capture
  list.
- **Loop placement needs no extra rule.** A `Bind` inside a loop body is re-evaluated each iteration, and its window
  ends inside that body.

**The rewrite:** replace each `Local(b)` in `ops[i+1..=L]` with a clone of `place`, delete `ops[i]`, count it once,
and repeat to a fixpoint.

**Owner classes.** A covers the **owned local** (record field paths, and the bare local). The only thing that can write
it is `Assign{o}`, so no call can break the window. B adds global, `STATE` and by-ref-capture owners, whose windows
must also be call-safe.

**Design uncertainty, scheduled first (A1):** whether some consuming context makes a forwarded read cost more than the
old `Local` read. If A1 finds such a context, the row forwards only when every read of `b` is in a context A1 marks as
safe. That is an allow-list, and it is decided by measurement, not guessed here.

**Correctness gate class.** Behaviour legitimately changes (fewer copies), so **byte-identity is NOT the gate.** The
gates are:

- run-time output identical at `-O0` and `-O1`;
- the `.ncode` markers (in-place slots present, `with_target` absent);
- the full suite.

Goldens are **expected** to diff, but only for fixtures containing a forwardable `LET`. Each such diff must show a
dropped copy, and nothing else. Any other golden diff is a bug to localize (objdump ONE fixture), not a stop.

**Rejected alternatives:**

- *Level 0.* This breaks the dial contract: keeping the copy is still correct.
- *Special-case the `LET cur` shape in codegen's in-place seam.* That duplicates the seam's matching for every site
  (plan-145 has 19) and still leaves the copy in read-only rows. A NIR rewrite reaches every site through the one
  existing seam.
- *Extend aggcopy.* Its "never written" gate is what makes it simple and whole-function. The window rule is a
  different proof, and aggcopy is Level 3.
- *A general last-use or move analysis.* That is far larger, and nothing in the benchmark or the fixtures needs more
  than one straight-line window.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the work, use `- [~]` for
> partial work, mark a moot task `- [x] ~~text~~ — moot: <evidence>`, fill `Commit:` the moment it lands (in the next
> commit), and add any task you discover. **An unticked box means NOT DONE.**

### Phase A1 — Cost audit (uncertainty first; no code change)

- [ ] For each consuming context of a `Local(b)` read, build one probe at `-O1` in two versions: the argument as
      `Local(cur)` bound by `LET cur = rec.xs`, and the argument as `rec.xs` directly. Compare the `.ncode`
      `copy_source` / `copy_result` / `with_target` counts **per function**, isolating the function's JSON object
      rather than the whole file (plan-145's follow-up probe did not isolate it). The contexts:
      - a builtin call argument;
      - a user call argument;
      - a `LET` initializer;
      - `RETURN`;
      - a constructor argument;
      - a `FOR EACH` iterable (with and without an owner write in the body);
      - a `WITH` update value;
      - a list-literal element;
      - a `String &` operand;
      - `len()`.

      Record a table: context → copies with `Local`, copies with the member access.
- [ ] Record how a call through a function value (`LET f AS FUNC… = …; f(x)`) and a `collections::` call with a lambda
      argument appear in NIR (`mfb build --nir`). Write the result into §2 Verified properties.
- [ ] Update this plan: any context where the member access costs more becomes a **read-context allow-list** in §3,
      with the table as evidence. If none does, record "no allow-list needed" with the table.

Acceptance: the cost table and the indirect-call shape are recorded in §2, and the allow-list decision is written in
§3. Check: the table has one row per context above, each with both counts (est. 30 min).
Commit: —

### Phase A2 — The row for owned-local owners

- [ ] Move `children_mut` from `src/optimizer/opt1/aggcopy.rs` to `src/optimizer/opt1/plans/shape.rs` as
      `pub(crate) fn value_children_mut`, and repoint aggcopy at it. That part is a pure move (Check: aggcopy's 6 unit
      tests still pass).
- [ ] Add `src/optimizer/opt1/placefwd.rs` with `pub(crate) fn forward(module: &mut NirModule)`, self-guarded by
      `level_enabled(1)`. It contains:
      - the census (§3 function-wide gates);
      - the candidate test (the place is `Local`, or a `MemberAccess` chain rooted at a `Local` owner);
      - the window rule W1–W3, with `Assign{o}` as the only owner write;
      - the A1 allow-list, if any;
      - the rewrite, run to a fixpoint.

      Declare it in `src/optimizer/opt1/mod.rs`, and call it with `timed("place copy forwarding", …)` right after
      `local_rewrites::apply`. Add a comment there saying why it runs first: every later row must see the direct form.
- [ ] Add the counter `PLACE_COPIES_FORWARDED` and `count_place_copies_forwarded` to `src/optimizer/stats.rs`,
      mirroring `AGGREGATE_COPIES_FORWARDED` (`stats.rs:122`, `:410`). The catalog row that makes it visible lands in C.
- [ ] Unit tests in `placefwd.rs`, in aggcopy's style (built NIR, `with_opt_level`). Each asserts the body after the
      pass. These **forward**:
      - the benchmark `set` shape (`get(cur)`, then `WITH` rewriting `cur`);
      - the `removeAt` shape;
      - a nested path `rec.a.xs`;
      - a bare local `snap = nums`;
      - a read-only window reaching the block's end;
      - a read inside a nested `FOR` in the window.

      These **keep the copy**:
      - the owner assigned between the bind and a read (W1);
      - a read after the owner write;
      - the owner write nested inside op `L` (W2: `FOR k … get(cur) … rec = … NEXT`);
      - a read in a `Trap` body (W3);
      - `b` captured;
      - the owner a `LocalRef`;
      - the owner captured;
      - a bare-local source that is a parameter;
      - a scalar `b`;
      - `-O0`.

Acceptance: the owned-local shapes are forwarded, and every negative case keeps its copy. Check:
`cargo test --lib optimizer::opt1::placefwd` → all pass; `cargo test --lib optimizer::opt1::aggcopy` → 6 pass (est. 5 min).
Commit: —

### Phase A3 — Codegen and run-time proof for owned locals

- [ ] Add `pub fn build_ncode_opt(project, target, name, level: &str)` to `tests/common/mod.rs`. It is the same as
      `build_ncode`, plus `-O <level>`. Leave `build_ncode` unchanged.
- [ ] Add `tests/codegen/codegen_place_forward.rs` and register it in `Cargo.toml`. For the benchmark
      `list (Record-Fixed) set` shape, the `removeAt` shape, and the bare-local `snap` shape, it asserts:
      - at `-O1`, the in-place slot (`inplace_recfield_set_index` / `inplace_inlined_subblock` / the local arm's slot)
        is present and `with_target` is 0;
      - at `-O0`, `with_target` is at least 1 (the row is the difference).
- [ ] Add `tests/runtime/rt_place_forward.rs` and register it in `Cargo.toml`. Each program in it is built at `-O0` and
      at `-O1`, and the test asserts identical stdout: the six forwarding unit-test shapes, plus the W1 and W2 negatives
      as programs.

Acceptance: at `-O1` each shape takes the in-place path, with output identical to `-O0`. Check:
`cargo test --test codegen_place_forward --test rt_place_forward` → all pass (est. 5 min).
Commit: —

## Appendix A — direct-form sibling-read probe

Build it as an executable project (`mfb build <dir>`) and run it. Expected stdout: `4 3`, `3 4`, `9 2`, `9 77`.

```
IMPORT collections
IMPORT io
TYPE R
  before AS Integer
  xs AS List OF Integer
  after AS Integer
END TYPE
FUNC give(rec AS R) AS List OF Integer
  RETURN rec.xs
END FUNC
FUNC main AS Integer
  MUT rec AS R = R[1, [1, 2, 3], 0]
  rec = WITH rec { xs := collections::append(rec.xs, 4), after := len(rec.xs) }
  io::print(toString(len(rec.xs)) & " " & toString(rec.after))
  rec = WITH rec { after := len(rec.xs), xs := collections::removeAt(rec.xs, 0) }
  io::print(toString(len(rec.xs)) & " " & toString(rec.after))
  rec = WITH rec { xs := collections::set(rec.xs, 0, 9), before := collections::get(rec.xs, 0) }
  io::print(toString(collections::get(rec.xs, 0)) & " " & toString(rec.before))
  MUT out AS List OF Integer = give(rec)
  out = collections::set(out, 0, 77)
  io::print(toString(collections::get(rec.xs, 0)) & " " & toString(collections::get(out, 0)))
  RETURN 0
END FUNC
```

## Corrections

## Summary

The risk is entirely in the window rule. A forward across an owner write would make `b` read the new value instead of
the old one: a silent wrong answer. So W1–W3 decline anything they cannot prove, and the unit tests pin every decline.
A's owner class (the owned local) can only be written by `Assign`, which keeps its proof local. B takes on the classes
that calls can write.
