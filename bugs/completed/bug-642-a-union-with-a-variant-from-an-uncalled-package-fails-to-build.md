# bug-642: a resource union with a variant from a package the program never calls fails to build

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: **HIGH** (raised — see Widened scope: the same defect silently miscompiles
`MATCH` on any union bound through an inline `TRAP`)
Class: Correctness (build failure on a valid program; silent wrong behavior)

Status: Fixed
Regression Test: `tests/cli/cli_thread_accept_res_bind.rs`
(`a_trap_bound_union_with_an_uncalled_variant_package_builds`, the build half);
`tests/runtime/rt_inline_trap_union_bind.rs` (the runtime half, 4 cases)

> **STATUS: FIXED (65a4e8e36)** — `ir::lower::lower_inline_trap` never applied the
> variant→union coercion at its delivery, so a union-typed target initialized from a
> variant-typed fallible producer held an untagged variant. Fixed by
> `wrap_trap_slot_value`, applied in the `Bind` and `Assign` arms. bug-642's own
> reproduction now builds and prints `ok=1`. **Deviation from the plan below:** the Root
> Cause hypothesis was wrong in a way that mattered — see Root Cause. **Addition:** the
> same one-line defect silently miscompiled `MATCH` on every inline-`TRAP`-bound union,
> data unions included; folded in here rather than filed separately (Widened scope).

A program that declares a resource union with a variant from a package it never calls
directly (here `fs::File`), and binds that union through an inline `TRAP`, fails to build
with an internal NIR error. The program is valid; the failure is in the compiler.

**The single correct behavior a fix produces:** the program below builds and prints `ok=1`.

References:

- `src/docs/spec/language/` resource unions and inline TRAP.
- Found by bug-623's union-alias fix work (subagent report), confirmed on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT fs

UNION Chan
  udp::Socket
  fs::File
END UNION

FUNC openOne(port AS Integer) AS Integer
  RES c AS Chan = udp::bind("127.0.0.1", port) TRAP(e)
    RETURN 0
  END TRAP
  RETURN 1
END FUNC

SUB main()
  io::print("ok=" & toString(openOne(0)))
END SUB
```

`mfb build <project>`:

- Observed: `error: NIR runtime call requires undeclared helper 'fs'`, build fails.
- Expected: builds; prints `ok=1`.

| Binary | Result |
| --- | --- |
| main `9b5e5b55f` | fails ✗ |
| integration `38e620ddb` | fails ✗ |

Contrast (subagent-reported): adding any real `fs::` call elsewhere in the program (or a
`tcp::` call for a `tcp` variant) makes it build.

## Root Cause

**Confirmed (and the hypothesis below was wrong — read the correction first).**

`ir::lower::lower_inline_trap` stages the trapped expression's value in a `$trap_valN` temp
typed with the **producer's** type, then delivers it to the target with a bare
`Bind c : Chan = local $trap_valN`. Every other binding and assignment path runs its value
through `wrap_union_value` first, which inserts the `UnionWrap` that gives a variant value the
union's `{tag@0, payload@8}` representation. The `TRAP` path did not.

So the bind reaches NIR as `NirValue::Local` — which is the **aliasing** shape.
`runtime::usage::push_op_helpers` correctly skips an aliasing bind (bug-375: an alias emits no
close), so it declared none of `Chan`'s variant close helpers; meanwhile
`validate::capabilities::collect_bind_types` has no aliasing gate and counted every variant of
every bound union as used. The two arms disagreed, and the build died on a
compiler-internal name with no code and no location. Verified against the contrast: a plain
`RES c AS Chan = u` lowers to `unionWrap`, not `local`, which is why that shape always built
and why adding any real `fs::` call hid the failure.

### Correction to the original hypothesis

The hypothesis said the drop "emits a tag-dispatched close for EVERY variant", i.e. that the
helper was genuinely needed and the DECLARER was at fault. It is the other way round: for an
aliasing union bind codegen emits **no** tag dispatch at all (`builder_control.rs` takes the
"Non-owning — no cleanup" branch when `aliases_live_resource && resource_union_cleanup(..)`),
so the declarer was right and the used side over-counted.

This mattered. Acting on the hypothesis — gating the used side to match the declarer — would
have made the build error go away while leaving the program **silently miscompiled**, because
the missing `UnionWrap` is a wrong *value*, not just a wrong helper set. See Widened scope.

## Goal

- The reproduction builds and runs; the same with the variant order swapped and with a
  `tcp::Socket` variant in a program that never calls `tcp::`.

### Non-goals (must NOT change)

- Programs that already build keep their helper sets.
- **Tempting wrong fix:** requiring an `IMPORT`-side dummy call, or pruning the union
  variant's close from the drop.

## Blast Radius

- Every drop that dispatches a union variant close: the plain union binding drop, the TRAP
  closed default, the owned-list drain — audit in Phase 1.

## Widened scope (2026-09-15): the same defect silently miscompiles `MATCH`

Found while confirming the root cause, and worse than the filed symptom. Because the
inline-`TRAP` binding holds an **untagged variant**, `MATCH` on it reads a tag that was never
written and falls through **every case** — no diagnostic, clean exit, the block simply
skipped.

Not resource-specific. A plain data union does it too:

```
TYPE Num
  v AS Integer
END TYPE
UNION Val
  Num
  Txt
END UNION

FUNC makeNum(n AS Integer) AS Num       ' returns the VARIANT, not the union
  IF n < 0 THEN FAIL error(77050004, "bad " & toString(n))
  RETURN Num[n]
END FUNC

LET v AS Val = makeNum(7) TRAP(e) … END TRAP
MATCH v
  CASE Num(m)  io::print("matched-num " & toString(m.v))
  CASE Txt(t)  io::print("matched-txt")
END MATCH
```

- Observed: prints nothing, exits 0. The same program with `LET v AS Val = makeNum(7)` (no
  `TRAP`) prints `matched-num 7`.
- In **assignment** position it is worse still: `v = makeNum(7) TRAP …` over a `MUT v AS Val`
  that already held a valid `Txt["start"]` replaces it with an untagged variant, so a
  previously-matching value stops matching.
- `StateAssign` position needs no fix: a STATE type must be "a copyable, defaultable data
  type" (`TYPE_STATE_INVALID` 2-203-0085), which rejects a union outright — verified by probe.

Why no existing test caught it: `tests/rt-behavior/control-flow/inline-trap-positions-rt`
does cover a union inside a trapped expression, but its producer (`shapeOf`) already returns
the **union** type, so no coercion is required. A variant-typed producer into a union-typed
target was the untested gap.

## Phases

### Phase 1 — failing test + audit

- [x] A build test for the reproduction (source form); confirm RED.
- [x] Locate the declared-helper computation; record the root cause here.
- [x] (added) Runtime tests for the silent-`MATCH` half, data and resource unions.

Commit: `60aac1623` (tests), `65a4e8e36` (audit recorded here)

### Phase 2 — the fix

- [x] Apply the variant→union coercion at the inline-`TRAP` delivery
      (`wrap_trap_slot_value`, `Bind` and `Assign` arms). This subsumes the originally
      planned "declare the close helpers of every variant": with the `UnionWrap` present the
      bind is no longer the aliasing shape, so the declarer and the used side agree.

Commit: `65a4e8e36`

### Phase 3 — full validation

- [x] Goldens: `scripts/artifact-gate.sh target/release/mfb all` → 1460 tests, 2050 goldens,
      **0 diffs**. No fixture covered this shape, which is exactly why the bug survived; the
      green gate here is a drift sentinel, not coverage.
- [x] Full suite — see the integration commit.

Commit: `65a4e8e36`

## Summary

Not a declaration-set gap after all: a missing value coercion in the inline-`TRAP` desugar,
whose *visible* symptom was a declaration-set disagreement and whose *invisible* symptom was a
silently skipped `MATCH`.
