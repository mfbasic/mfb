# bug-45: a resource-union bound inside `FOR EACH` fails to compile — `validate_nir`'s `collect_bind_types` is the only NIR collector that skips `ForEach` bodies

Last updated: 2026-07-09
Effort: small (<1h)

A valid MFBASIC program that binds a resource union (`RES s AS Stream = …`) inside a
`FOR EACH` body is **rejected by the compiler** with an internal error:

```
error: NIR declares unused runtime helper 'net'
```

The same bind at top level, or inside `WHILE` / `FOR` / `DO UNTIL` / `IF` / `MATCH` /
`TRAP`, compiles fine. `FOR EACH` alone is broken.

`runtime::required_helpers` descends into `ForEach` and correctly declares the close
helpers for every union variant (`fs` **and** `net` for a `File OR Socket` union).
`validate_nir`'s `collect_bind_types` does **not** descend into `ForEach`, so it never
sees the union bind, never marks `net` as used, and then the "declared but unused"
cross-check fires. The two collectors disagree, and the one that under-counts wins.

The single correct behavior a fix produces: `collect_bind_types` recurses into
`ForEach` bodies, so the program below compiles and links exactly as its `WHILE`
equivalent does.

References:

- `src/target/shared/validate.rs:228-256` (`collect_bind_types` — arms for
  `If`/`Match`/`While`/`For`/`DoUntil`/`Trap`, then `_ => {}` which swallows
  `NirOp::ForEach`).
- `src/target/shared/validate.rs:104-111` (the `declares unused runtime helper` check
  that fires).
- Correct siblings that **do** traverse `ForEach`:
  `src/target/shared/plan/symbols.rs:66-70` (`collect_bind_type_names`),
  `src/target/shared/runtime/usage.rs:225` (`required_helpers`,
  `IrOp::ForEach => push_op_helpers(body, …)`),
  `src/target/shared/validate.rs:368` (`collect_runtime_calls_from_ops`).
- `src/target/shared/nir/mod.rs:197-202` (`NirOp::ForEach { name, type_, iterable, body }`).
- Existing passing test this is derived from:
  `tests/rt-behavior/resources/resource-union-valid/src/main.mfb`.
- Same "collector skips a loop body" class, different file: bug-33
  (`ir/binary.rs:verify_package` skips `For`/`DoUntil` bodies).
- Found during the goal-01 compiler source review of `src/target/shared/`.

## Failing Reproduction

```
mfb init /tmp/feu
cat > /tmp/feu/src/main.mfb <<'EOF'
IMPORT fs
IMPORT net
IMPORT io

UNION Stream
  File
  Socket
END UNION

FUNC main AS Integer
  LET ns AS List OF Integer = [1]
  FOR EACH n IN ns
    RES s AS Stream = fs::createTempFile()
    MATCH s
      CASE File(f)
        fs::writeAll(f, "union-data")
        io::print("file")
      CASE Socket(sock)
        io::print("socket")
    END MATCH
  NEXT
  RETURN 0
END FUNC
EOF
mfb build /tmp/feu
```

- Observed: `error: NIR declares unused runtime helper 'net'` — build fails. Verified
  on macOS/aarch64 with `target/debug/mfb`.
- Expected: `Wrote executable to /tmp/feu/feu.out`.

Contrast cases that build correctly today (both verified, and both become regression
guards):

- The identical union bind at **top level** of `main` (this is
  `tests/rt-behavior/resources/resource-union-valid` verbatim) → builds.
- The identical union bind inside a **`WHILE`** loop → builds.

The error is not platform-specific: `validate_nir` runs on the real build path for
every backend (`write_executable`), not only under a `-nir` dump.

## Root Cause

`validate_nir` cross-checks two independently computed sets and errors if they differ:

- `module.runtime_helpers` — computed by `runtime::required_helpers`
  (`runtime/usage.rs`), which **does** recurse through `IrOp::ForEach` bodies. For the
  program above it sees the `Stream` bind, expands the union's variant close calls
  (`fs.closeFile`, `net.closeSocket`), and declares helpers `{fs, net}`.
- `used_helpers` — computed in `validate.rs:57-94` from the *bind types* returned by
  `collect_bind_types`. That function's `match` has no `NirOp::ForEach` arm, so the
  `_ => {}` catch-all silently drops the entire loop body. The `Stream` bind is never
  collected, its variant closes are never resolved, and `used_helpers` = `{fs}` (from
  the direct `fs::createTempFile` runtime call).

`{fs, net}` declared vs `{fs}` used → `validate.rs:105-111` returns
`NIR declares unused runtime helper 'net'`.

The contrast cases are immune for exactly this reason: `While` has an arm
(`validate.rs:247-253`), so its body is traversed and `net` lands in `used_helpers`; a
top-level bind is matched by the `NirOp::Bind` arm directly.

Note the second, latent half of the same gap: `NirOp::ForEach` carries its **own**
`name`/`type_` (the loop variable's binding). Neither `collect_bind_types` nor
`plan/symbols.rs:collect_bind_type_names` inserts that `type_`. Iterating a
`List OF RES Stream` directly — `FOR EACH s IN streams` with no inner `Bind` — would
therefore under-count helpers in *both* collectors. This is not what the reproduction
above triggers, and it may be unreachable if such a loop always lowers to an inner
`Bind`; Phase 1 must settle it rather than assume.

## Goal

- The reproduction program compiles and produces a working executable.
- The `WHILE` and top-level contrast programs continue to compile, unchanged.
- `collect_bind_types`, `collect_bind_type_names`, and `required_helpers` traverse the
  same set of `NirOp` bodies — verified by a test, not by inspection.

### Non-goals (must NOT change)

- The `validate_nir` cross-check itself. It caught a real inconsistency; do **not**
  weaken it (e.g. downgrade "declares unused runtime helper" to a warning, or drop the
  declared-vs-used comparison). That is the tempting wrong fix and it would hide the
  next collector that drifts.
- `required_helpers` and `module.runtime_helpers` — these are already correct.
- Helper-declaration granularity or the emitted code for any currently-compiling
  program: the fix only makes `used_helpers` see a bind it should always have seen, so
  no `.ncode` output may shift.

## Blast Radius

Every recursive `NirOp`-body walker was enumerated and checked against the same op
set that `required_helpers` traverses. Only `collect_bind_types` under-counted, and
only by skipping the `ForEach` body. The audited walkers:

- `validate.rs:collect_bind_types` — **was broken** (missing `ForEach` body); fixed.
- `validate.rs:collect_runtime_calls_from_ops_with_constants` — already had a full
  `ForEach` arm (line 368); no change.
- `validate.rs:validate_ops` — already had a full `ForEach` arm (line 1045); no change.
- `plan/symbols.rs:collect_bind_type_names` — already traverses the `ForEach` body
  (line 69); no change. (Not edited — outside this bug's scope.)
- `runtime/usage.rs:push_op_helpers` (`required_helpers`) — already correct; no change.

## Phases

- [x] Phase 1 — settle the latent second half (`ForEach`'s own `name`/`type_`).
  Verified from `runtime/usage.rs:push_op_helpers`: `required_helpers` resolves
  resource-union closes **only** from `IrOp::Bind` type strings and never from the
  `ForEach` loop-variable's own `type_`. A borrowed loop variable is never closed, so
  no close helper is ever declared for it. `collect_bind_types` ignoring the loop
  var's `type_` is therefore *symmetric* with the declared set — not an under-count.
  Empirically, a `FOR EACH s IN streams` over a bare-resource-typed list is rejected
  earlier by the type checker (`2-203-0082`/`2-203-0100`), so no bind ever reaches
  `collect_bind_types` with the loop var carrying a resource-union type. No change to
  the loop-variable handling is needed in either collector.
- [x] Phase 2 — teach `collect_bind_types` to recurse into `ForEach` bodies (the one
  fix), matching `collect_bind_type_names` and `required_helpers`.
- [x] Phase 3 — regression tests (unit + acceptance fixture), proven fail-before /
  pass-after.

## Resolution

Fixed in `src/target/shared/validate.rs`: `collect_bind_types` now includes
`NirOp::ForEach { body, .. }` in the loop-body traversal arm (alongside
`While`/`For`/`DoUntil`/`Trap`), so a resource-union `Bind` inside a `FOR EACH` body
is collected and its variant close helpers (`fs`, `net`, …) are counted as used. This
restores parity with `collect_bind_type_names` and `required_helpers`; the
`validate_nir` cross-check was left untouched (per the non-goals).

The latent second half (Phase 1) turned out to be a non-bug: `required_helpers` never
declares a helper for the `ForEach` loop variable's own type, so both collectors
correctly ignore it and stay symmetric.

Verification:

- Repro from this doc now builds and runs: `Wrote executable …`, prints `file`,
  exit 0.
- Contrast cases (top-level bind, `WHILE` bind) still build.
- Unit tests in `validate.rs`: `collects_resource_union_bind_inside_for_each` (new,
  fails before the fix with `NIR declares unused runtime helper 'fs'`, passes after)
  and `collects_resource_union_bind_at_top_level` (guards the unchanged contrast).
  `cargo test --bin mfb target::shared::validate` → 4 passed.
- Acceptance fixture added:
  `tests/rt-behavior/resources/resource-union-foreach-valid/` (the repro verbatim,
  with `golden/{build.log,*.ast,*.ir,*.run}`). Goldens regenerate byte-identically;
  the fixture cannot produce a valid `.run` golden before the fix, so it is a genuine
  guard. (Left for the orchestrator's `scripts/test-accept.sh` run.)