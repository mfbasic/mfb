# bug-999: a `List OF RES` (or record of resources) returned by a call nested in an expression is never closed — `len(mkList())` leaks every resource it holds

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Fixed (bs22-agent-g)
Regression Test: `rt_resource_aggregate_temporary` (22 tests) and `rt_resource_model::bug_999_*` (4)

bug-842 made a resource produced inline in an expression (a call argument, a member read, a
list or record literal argument) a statement-scoped temporary that is closed at the end of its
statement. It deliberately left out aggregates: a call nested in an expression that RETURNS a
`List OF RES T` (or a record or union holding resources) is not registered. Its resources are
never closed. bug-842's resolution gives the reason: a builtin aggregate can be a view of
resources someone else owns (`canvas::groupItems`) with no owner word to say so, and
registering every aggregate doubled the canvas renderer helpers (`_mfb_ifn_canvas_5FdrawGroup`
477 → 1635 instructions).

**Correct behavior:** language spec 15: "A resource is closed exactly once." An aggregate of
resources that a user call returns and that nothing claims should have its resources closed by
the end of the statement, as the bound form `LET ys = mkList()` does at scope exit. Views
returned by builtins that do not transfer ownership (`canvas::groupItems`) must stay untouched.
The fix probably needs an ownership distinction between "fresh owned aggregate" and "view",
which bug-842 found missing.

## Failing Reproduction

Measured by the coordinator on 2026-10-06, macOS aarch64, with main 468ff0f94
(`/tmp/main-dbg-target/release/mfb`) and with bs17-agent-f (bug-842 fixed). The result is the
same on both.

```
IMPORT fs
IMPORT io
IMPORT collections

FUNC mkList() AS List OF RES fs::File
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  xs = collections::append(xs, f)
  RETURN xs
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  FOR i = 1 TO 2000
    n = n + len(mkList())
  NEXT
  io::print("n=" & toString(n))
  RETURN 0
END FUNC

mfb build -q agg && (ulimit -n 64; ./agg/build/agg.out)
```

- Observed: `Error: 7-702-0002` / `Write or flush operation failed.`, exit 255. The descriptors
  run out.
- Control: the same loop with the list bound first
  (`LET ys AS List OF RES fs::File = mkList()` then `n = n + len(ys)`) prints `n=2000`, exit 0
  (bs17-agent-f).
- Expected: `n=2000`, exit 0.

## Related

- bug-842 (resource temporaries; this is the aggregate case it scoped out).

## Attempt in bug-staging-18 (bs18-agent-k, 2026-10-10): reverted, still open

A fix was committed (`ee8fa14cb`) and reverted (`8982edd27`) in the same batch. What
it found:

- **Who produced it is settled.** A call that runs a function BODY leaves through
  `RETURN`, which releases exactly the records its own scopes owned (spec §15), so an
  unowned record in its result is one it handed over. A natively lowered builtin
  (`canvas::groupItems`) makes no such statement, and neither does a body whose every
  `RETURN` hands back a parameter or a module-level binding.
- **Who keeps it is NOT settled, and that is what broke.** The fix closed, at the end
  of the statement, everything an aggregate from a function body reached unowned.
  That is wrong whenever a value that outlives the statement keeps the aggregate
  without owning what it reaches: `LET back = Layer[items := mkItems()]`,
  `LET items = mkLayer().items`, `FOR EACH h IN mkList()`. Only a call-rooted bind
  adopts, and only a single resource in an element position is hoisted to a name.
  `tests/canvas/rt_canvas_geo_native.rs`
  (`the_native_frame_pass_matches_the_mfbasic_walk_on_a_mixed_moving_scene`) died of
  it with SIGSEGV on `LET back AS canvas::DrawLayer = canvas::DrawLayer[items :=
  shapes(20)]`: a constructor frees its argument's block once it has copied it, so
  the statement-end walk read a freed list (`mfb dbg`: a bad memory access in
  `main+0x5dd4`, on the `ldr x9, [x8]` that loads an element's tag). Passing with
  the base compiler, failing with the fix, three runs each.
- **One class is safe and was measured flat**, but was not verified after being
  separated from the rest, so it is not in the tree: a call to a function taking only
  plain scalars, written straight into an argument of a call that keeps nothing of it,
  takes no keeping callback and can only answer with a plain scalar (`len(mkList())`).
  Nothing can keep such a result, so the statement can adopt what it reaches as a
  binding would (claim at once, drain with the owning frees at the end). With that in
  place the report's program measured `163 / 163 / 0` (alloc / free / live, N=20)
  against the bound control's `163 / 163 / 0` and the base's `143 / 103 / 7040`, and
  printed `n=2000` under `ulimit -n 64`. A result reaching a resource-union box must
  stay out of it (bug-1012).
- **The other positions need a language decision**: either every bind, assignment,
  global store or `RETURN` whose value contains such a call adopts what it handed
  over (then the statement may close the rest), or such a call must be bound first.


## Narrowed fix in bug-staging-21 (bs21-agent-b, 2026-10-11)

Only the class the section above calls safe. Every other position behaves as on the base compiler, measured.

### What is fixed

A call `P(...)` is an aggregate temporary its statement owns outright when ALL of these hold:

1. `P` runs a function BODY and every parameter of it is a plain scalar (Boolean, Byte, Integer, Fixed, Float,
   Money, Nothing, String), and it is a plain call, not the `callResult` of an inline `TRAP`;
2. it is written DIRECTLY as an argument of a call `R(...)`, and `R` does not keep that argument (a natively lowered
   builtin that is not a `collections` member storing it as an element, or a body `callee_keeps` reads and finds not
   to keep it), does not consume it, is not a borrowed-element read, is not called through a function value and is
   handed no callback that keeps a reference;
3. `R` can only answer with a plain scalar (its declared return, or every overload of a builtin);
4. the result reaches a resource, reaches no resource-union box and no function value, and `P` is not a body whose
   every `RETURN` hands back a parameter or a module-level binding.

`CollectionUses` (`record_ownership.rs`) finds 1 to 3 and names the call node; `is_exclusive_aggregate_temp`
(`builder_values.rs`) adds 4. When the call returns, `adopt_exclusive_temp` claims every unowned record the result
reaches for the current scope and lists it, exactly as `LET t = P(...)` adopts
(`emit_owned_list_push_node`, adopting); at the end of the statement, and on its error exit and its `RETURN`,
`emit_exclusive_temp_drop` drains the lists with the owning frees and frees the result's block when the callee built
it and no pending-temporary free already covers it.

The call nodes are known by address, and `lower_runtime_helper_call` lowers a COPY of its arguments, so
`io::print(toString(len(mkList())))` missed the class until `carry_exclusive_temps` carried the entries to the copy.
That is a miss in the safe direction (the statement behaves as on base); any other lowering that clones its
argument nodes has the same effect and is not covered.

### Measured (fixed compiler `target/release/mfb`, macOS aarch64, `--debug` arena 0 counters)

The report's program (`/tmp/bs21/src/b999.mfb`, 2000 iterations) under `ulimit -n 64`: base `exit 255`
(`7-702-0002`); fixed `n=2000`, exit 0, `alloc_calls 16002 free_calls 16002 live_bytes 0 double_free_skips 0`.

The same loop with `N` from the environment, against the bound control `LET ys = mkList()` / `len(ys)`:

| N | base nested | fixed nested | bound control (base and fixed) |
|---|---|---|---|
| 20 | 143 / 103 / 7040 | 163 / 163 / 0 | 163 / 163 / 0 |
| 300 | 2103 / 1503 / 105600 | 2403 / 2403 / 0 | 2403 / 2403 / 0 |

Class statements (`/tmp/bs21-agent-b/m999/c*.mfb`; the program keeps one empty module-level list, the constant
48 live bytes). alloc / free / live at N=10 and N=40, `double_free_skips 0` in every run, and each exits 0 at N=300
under `ulimit -n 64`:

| statement | base | fixed |
|---|---|---|
| `n = n + len(mkList())` | 128/97/4688, 488/367/18608 | 148/147/48, 568/567/48 |
| `n = n + count(mkList())` (a body that iterates and writes) | 128/97/4688, 488/367/18608 | 148/147/48, 568/567/48 |
| `IF len(mkList()) > 1 THEN` | 128/97/4688, 488/367/18608 | 148/147/48, 568/567/48 |
| `RETURN len(mkList())`, early and last | 188/142/7008, 728/547/27888 | 218/217/48, 848/847/48 |
| `len(mkMap())` (`Map OF String TO RES fs::File`) | 78/57/4208, 288/207/16688 | 88/87/48, 328/327/48 |
| `countH(mkHolders())` (`List OF Holder`, a record with a `RES` field) | 88/77/1168, 328/287/4528 | 98/97/48, 368/367/48 |
| `len(mkListN(3, "t"))` | 178/137/5808, 688/527/23088 | 208/207/48, 808/807/48 |
| `len(mkDup())` (one record listed twice) | 78/57/3568, 288/207/14128 | 88/87/48, 328/327/48 |
| a short-circuited operand, a `CONTINUE` past the producer | 116/88/4224, 476/358/18144 | 134/133/48, 554/553/48 |
| `WHILE j < len(mkList())` | 368/277/13968, 1448/1087/55728 | 428/427/48, 1688/1687/48 |
| three such calls in one expression | 318/237/12848, 1248/927/51248 | 368/367/48, 1448/1447/48 |
| `io::print(toString(len(mkList())))` | 138/107/4688, 528/407/18608 | 158/157/48, 608/607/48 |
| the receiving call FAILs, trapped in the function and propagated | 287/226/9328, 1127/886/37168 | 327/326/48, 1287/1286/48 |
| `stash(mkList())`, `stash` appending one element to a module-level list | 141/110/5456, 533/412/21680 | 161/150/1936, 613/572/7600 |
| `len(mkNested())` (`List OF List OF RES fs::File`) | 148/107/7088, 568/407/28208 | 168/157/2448, 648/607/9648 |

The last two equal their bound controls built with the BASE compiler exactly (`LET t = mkList()` / `stash(t)`:
161/150/1936 and 613/572/7600; `LET t = mkNested()` / `len(t)`: 168/157/2448 and 648/607/9648). In the `stash` case
what stays live is what the module-level list holds; the kept files are still writable after the loop. In the nested
case the bound form itself leaves the inner list blocks (240 bytes an iteration), which is not this bug's.

Debugger (`mfb dbg` on the `-O0 --debug` build of the `stash` program, N=2, `bp _mfb_rt_fs_fs_close bt 2` and
`bp _close list reg x0`): each iteration closes the one file `stash` did not keep, from `main` (descriptors 4,
then 5); the two kept files are closed once each by `mfb_module_resource_drain` at exit (4, then 3). No descriptor
is closed twice.

The canvas path: `cargo test --test rt_canvas_geo_native` passes (4 tests, the one the earlier attempt crashed
included). `scripts/artifact-gate.sh target/release/mfb all`: 2346 goldens checked, 0 diffs. The `-app --ncode` dump
of `tests/syntax/app/app-mouse-surface` is byte-identical between the base and the fixed compiler (`cmp`), and
`_mfb_ifn_canvas_5FdrawGroup` has 477 instructions in both.

### Positions outside the class: unchanged, measured equal to base

alloc / free / live at N=10 and N=40, the same numbers from both compilers (`/tmp/bs21-agent-b/m999/o*.mfb`):

| statement | both compilers |
|---|---|
| `LET back AS Layer = Layer[items := mkList()]` | 138/97/4848, 528/367/19248 |
| `LET items = mkLayer().items` | 148/97/5008, 568/367/19888 |
| `FOR EACH h IN mkList()` | 128/97/4688, 488/367/18608 |
| `LET xs = mkList()` | 148/147/48, 568/567/48 |
| `xs = mkList()` | 158/157/48, 608/607/48 |
| `gKeep = mkList()` (a module-level store) | 148/117/4688, 568/447/18608 |
| `len(mkLayer().items)` | 148/97/5008, 568/367/19888 |
| `len(mkHandles())` (`List OF RES Handle`, a resource-union box) | 88/67/3568, 328/247/14128 |
| `len(mkFrom(f))` (the producer takes a resource) | 68/57/2448, 248/207/9648 |
| `len(collections::take(mkList(), 1))` | 138/97/5648, 528/367/22448 |
| `fs::writeAll(collections::get(mkList(), 0), "x")` | 128/97/4688, 488/367/18608 |

A result reaching a resource-union box is LEFT AS ON BASE, not handled (the row above).

### Remaining (each still leaks, one measured repro each, fixed compiler, N=300 under `ulimit -n 64`)

Programs are `/tmp/bs21-agent-b/m999/<name>.mfb`; the prelude is `pre.txt` there.

1. A constructor argument, `LET back AS Layer = Layer[items := mkList()]` (`o01`): `7-702-0002`, exit 255.
2. A member read, `LET items = mkLayer().items` (`o02`) and `len(mkLayer().items)` (`o07`): exit 255.
3. A `FOR EACH` iterable, `FOR EACH h IN mkList()` (`o03`): exit 255.
4. An argument of a call that answers with more than a plain scalar:
   `fs::writeAll(collections::get(mkList(), 0), "x")` (`o12`) and `len(collections::take(mkList(), 1))` (`o11`):
   exit 255.
5. A result reaching a resource-union box, `len(mkHandles())` (`o08`): exit 255.
6. A receiving function that reads a field of a record argument, `useH(mkHolder())` with `RETURN h.n` (`c07`):
   exit 255. `callee_keeps` counts the member read as naming the parameter, so the call is not in the class.
7. An inline `TRAP` on the receiving call, `LET k = countFail(mkList()) TRAP(e) ... END TRAP` (`c05`): the files
   ARE closed (exit 0 at N=300), the memory is not (158/117/7248 at N=10, both compilers). The desugar hoists the
   argument into a binding of `resultValue($trap_res)`, which is a bind position.

Positions 1 to 4 are the ones this document said needed a language decision; the owner has since settled them
(see "Owner ruling" below). 5 waits on which boxes such a result
owns; 6 and 7 are narrower rules of the same class that were not widened here.

## Checks run (bs21-agent-b)

* RED: `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --test rt_resource_aggregate_temporary`: 8 of 9
  fail (`bug999_len N=10: alloc_calls 123 free_calls 93 live_bytes 4640`); the guard for kept aggregates passes.
  GREEN with the fix: 9 passed.
* `cargo test --test rt_canvas_geo_native`: 4 passed. `--test rt_resource_temporary_close`: 1 passed.
  `--test rt_resource_drop_reclaims`: 16 passed. `--test codegen_helper_scratch_release`: 12 passed.
  `--test test_targets_registered`: 4 passed. `--test rt_http_handle_request_serves`,
  `--test rt_native_write_overload_call_argument`, `--test cli_thread_accept_res_bind`: passed.
* `cargo test --bin mfb -- view_return_tests codegen::resource:: target::shared::runtime`: passed.
* `scripts/artifact-gate.sh target/release/mfb all`: 0 diffs.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*canvas*' '*collection*' '*bug842*' '*bug1012*'
  '*bug1000*' '*bug805*'`: 297 fixtures, 2 mismatches, both bug-1030's reorders (its doc), none from this change.
* `scripts/spec-census.sh --citations` / `--render`: clean.

## Golden impact (bs21-agent-b)

None: `artifact-gate all` reports 0 diffs. No fixture in the tree writes a statement of the class.

## Owner ruling (2026-10-11): no language decision is open

The owner's rule, stated on 2026-10-11: a resource put in a list takes the list's scope when that
scope is higher than its own. An aggregate of resources that a call returns unbound therefore has
the scope of whatever takes it, and its resources go with it:

- a record or binding takes it (`Layer[items := mkList()]`, `LET items = mkLayer().items`): the
  resources belong to that record or binding and are released with it;
- a loop takes it (`FOR EACH h IN mkList()`): they are released when the loop ends;
- nothing takes it (`len(mkList())`): they are released at the end of the statement.

So the "either / or" in the bs18 section above is answered: the taker adopts, and the statement
closes only what nothing took. The bs18 attempt crashed because it did the second half without
the first.

**What remains open in this document** is the seven positions listed under "Remaining
positions": each still leaves its resources unreleased with the batch 21 compiler. Position 5
(a result reaching a resource-union box) also needs the box's owner worked out; positions 6 and 7
are inside the class batch 21 fixed as worded, and were not reached.

## Resolution (bs22-agent-g, 2026-10-11)

A resource a callee's `RETURN` released stayed unowned until some binding claimed it, so in every position where nothing bound the call nothing owned it. It is the caller's statement's at once (rule 5: `register_resource_temp`, `adopt_statement_temp`, however the function was reached), every longer-lived taker raises it (rule 4), and the statement closes and frees what it still owns. The descriptors were closed and each 112-byte record kept, because a record was freed only for a holder a static proof accepted. The scope that closes a resource now frees its record, decided by the owner word alone: the cleanup takes the record (`RESOURCE_OWNER_TAKEN`) and the exit frees it after its last cleanup (owned-lists, `RES` bindings, resource-union bindings). The proofs are deleted. No position list: the taker is whatever stores the value.

Regression tests: `rt_resource_aggregate_temporary` (22 tests) and `rt_resource_model::bug_999_*` (4). Each builds the bug's repro (`bugs/repro/`) with `--debug` and requires, at N=10 and N=40, exit 0, the expected total, `double_free_skips 0` and live bytes that do not grow, and exit 0 at N=300 under `ulimit -n 64`.

## Golden impact

Not regenerated. Expect `.ncode` changes wherever a `RES` binding, a resource-union binding or an owned-list is dropped (the take and the reclaim after the exit's last cleanup), wherever a call returns a value reaching resources (the statement's take and drain), and at assignments that float.
