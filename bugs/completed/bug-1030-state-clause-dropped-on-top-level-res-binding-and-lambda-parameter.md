# bug-1030: the `STATE` clause of a top-level `RES` binding and of a `LAMBDA` parameter is dropped, so `.state` on it does not type

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (valid program rejected)

Status: Fixed (bs21-agent-b)
Regression Test: `tests/rt-behavior/resources/bug1030_state_top_level_only_rt` (the reproduction as filed), `tests/rt-behavior/resources/bug1030_state_top_level_rt`, `tests/rt-behavior/resources/bug1030_state_lambda_param_rt`, `tests/syntax/resources/bug1030_state_agreement_invalid`

`RES g AS fs::File STATE Cur = ...` at the top level and `LAMBDA(x AS fs::File STATE Cur) -> ...` lose their `STATE`
clause, so reading `.state` through them is rejected as if the resource had no state.

**Correct behavior:** both programs build; the `STATE Cur` is part of the binding's / parameter's type.

## Failing Reproduction

Measured on 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at 3fa35d543, macOS aarch64),
`cd <dir> && /tmp/main-dbg-target/release/mfb build -ast -ir`.

Top level (`/tmp/bs19-found/a1`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

RES g AS fs::File STATE Cur = fs::createTempFile()
FUNC main AS Integer
  io::print(toString(g.state.n))
  RETURN 0
END FUNC
```

Errors: `main.mfb:9 TYPE_CALL_ARGUMENT_MISMATCH` (`Call to toString has argument type(s) (Unknown), expected Integer,
...`) and `main.mfb:9 TYPE_STATE_INVALID` (`fs.File here has no STATE to read; declare the resource with STATE T. Bare
means "no state" on an owner, and "opaque" on a resource parameter — a bare parameter cannot read the state its caller
attached.`).

Lambda parameter (`/tmp/bs19-found/a2`):

```
IMPORT io
IMPORT fs
TYPE Cur
  n AS Integer
END TYPE

FUNC main AS Integer
  LET j = LAMBDA(x AS fs::File STATE Cur) -> x.state.n
  RES f AS fs::File STATE Cur = fs::createTempFile()
  io::print(toString(j(f)))
  RETURN 0
END FUNC
```

Errors: `main.mfb:8 TYPE_UNKNOWN_VALUE` (`Initializer for binding j does not have a known type.`),
`main.mfb:10 TYPE_CALL_ARGUMENT_MISMATCH`, and `main.mfb:8 TYPE_STATE_INVALID` (same message as above).

## What is known

Stated by the bug-1013 agent, not traced by the coordinator and not verified here: `lower_binding` never reads
`HirTopLevelBinding::state_type`, and a lambda parameter is lowered from its `type_` alone, so the `STATE` clause
never reaches the lowered form.

Found by the bug-1013 agent while fixing bug-1013 (batch 19).

## Resolution (bs21-agent-b, 2026-10-11)

The agent's statement was right: `lower_binding` (and `declared_binding_types`, which types every read of the global)
used `HirTopLevelBinding::type_` alone, and the three lambda sites (`expression_type`'s `Lambda` arm, the lambda's
`IrParam`s and locals, and `ir::shape`'s walk of the body) used `HirParam::type_` alone.

**The clause now rides in the type**, as it does for a local binding and a declared FUNC's parameter
(`top_level_binding_type`, `lambda_param_type` in `src/ir/lower.rs`). Four things had to follow, each found by
running the reproduction or a sibling of it, not by reading:

1. **A package called only from a top-level initializer was not linked.** With the clause carried, the first
   reproduction failed with `NIR runtime call requires undeclared helper 'fs'`. `runtime::required_helpers`
   walked the functions and never the top-level initializers, so the same program without any `STATE`
   (`RES g AS fs::File = fs::createTempFile()` and no other `fs` call) fails that way on the base compiler too
   (`/tmp/bs21-agent-b/src/g1.mfb`, measured). It now walks the initializers.
2. **A top-level state was never allocated.** The reproduction then died with SIGSEGV: only a local `Bind`
   default-initialized the payload. `NirOp::StoreGlobal` now runs `emit_resource_state_init` on a global whose
   declared type carries a `STATE` (a resource that arrives carrying one keeps it, so a stateful return is adopted).
3. **The clause must bind, or `g.state` reads another record's bytes.** `RES g AS fs::File STATE Other =
   openTagged()`, with `openTagged` returning `STATE Cur`, built on the base compiler (harmless there, the clause
   being dropped). `ir::verify` now applies `check_binding_state_agreement` to a top-level binding that declares a
   `STATE`.
4. **A lambda's callable type carries the clause.** A call through a function value is checked against the callable
   type alone, so the type is the only place the rule "a parameter observes a state, it cannot attach or re-type
   one" can be applied: `LAMBDA(x AS fs::File STATE Cur) -> x.state.n` is a `FUNC(fs.File STATE Cur) AS Integer`.
   `check_call_argument_types` applies `check_argument_state_agreement` to a function value's stateful parameter,
   and `ir::shape`'s `compatible` no longer equates `FUNC(File STATE Cur)` with `FUNC(File)` (a `FUNC(File)`'s
   callers may pass any state or none).

The two `ir::shape` rules that existed only because the clause was erased are gone, since `ir::verify` now sees
it: `check_erased_state_type` (bug-1013, both positions) and the top-level arm of `check_stray_state` (bug-998).
The diagnostics are the same text at the same line; they now come from `ir::verify`'s stream, which renders after
`ir::shape`'s (see Golden impact). A side effect measured, not designed: `RES g AS Handle STATE Cur` at the top
level, with `Handle` a resource union, was refused by the shape rule as "not a resource" and is now accepted, as
the identical local binding always was.

### What was measured

Built with the fixed compiler (`target/release/mfb`), macOS aarch64:

* `N=3 /tmp/bs21/bin/probe.sh target/release/mfb a1` prints `0`, exit 0; `a2` prints `0`, exit 0 with
  `alloc_calls 6 free_calls 6 live_bytes 0 double_free_skips 0`.
* State written through a `STATE Cur` parameter is read back through the global and through the lambda parameter's
  owner: the two `.run` goldens (`bump 10 20`, `replaced 7 [seven]`, `update 14 owner 14`, `capture 124 owner 24`).
* The top-level resource is closed once, at instance teardown: `bp _mfb_rt_fs_fs_close bt 3` in `mfb dbg` on
  `/tmp/bs21-agent-b/p/a1c` stops once, under `mfb_module_resource_drain`, and `_close` is entered once with the
  descriptor (`x0 = 3`).
* A worker thread's instance has its own state (`worker 11`, then `plain after worker 7` in the main instance).
* A top-level stateful resource leaves its record and its state block live at exit (`live_bytes 144` against `112`
  for the same program with no `STATE`), which is the module owner's documented contract: teardown closes and
  "reclaims nothing: the instance's arena is released with the instance" (`src/codegen/resource/module_owner.rs`).

### Not changed

`g.state.n = 5` written directly on a top-level binding is still refused (`TYPE_UNKNOWN_VALUE`, "State assignment
target `g` is not a local binding", `ir::shape`). That is an existing rule about the assignment statement, not
about the clause; the state is updated through a `STATE` parameter.

## Checks run

* `scripts/test-accept.sh /tmp/main-dbg-target/release/mfb <out> 'bug1030*'` (base): 10 mismatches in 4 fixtures.
  With the fix: 0.
* `scripts/test-accept.sh target/release/mfb <out> '*resource*' '*lambda*' '*closure*' '*global*' '*state*'
  '*func-value*' '*function-value*' '*bug1030*'`: 199 fixtures, 2 mismatches, both listed under Golden impact.
* `cargo test --bin mfb -- ir:: target::shared::runtime target::shared::validate hir::`: 986 passed.
* `cargo test --test no_type_strings --test architecture_guards --test no_operator_strings
  --test test_targets_registered`: all passed.
* `scripts/artifact-gate.sh target/release/mfb all`: 2344 goldens checked, 0 diffs.
* `scripts/spec-census.sh --citations`: 0 missing; `--render`: 0 failures.

## Golden impact

Two `build.log` goldens change, and only in the ORDER of their diagnostics (`diff <(sort golden) <(sort actual)` is
empty for both; 24 and 14 diagnostics before and after):

* `tests/syntax/resources/bug1013_state_requires_record_invalid/golden/build.log`: the `TYPE_STATE_INVALID` for
  `topBad` (line 52) and for the lambda parameter `x` (line 100) move from the head of the log to its end.
* `tests/syntax/resources/bug998_state_requires_resource_invalid/golden/build.log`: the
  `TYPE_STATE_REQUIRES_RESOURCE` for `top` (line 34) moves from the head of the log to after the function-local
  ones.

Reason: those three were reported by `ir::shape` because lowering erased the clause; the clause is now lowered, so
`ir::verify` reports them, and a build renders `ir::shape`'s stream first.
